//! Construct persisted-byte faults independently of the engine's recovery decision.
//!
//! Lost directory updates follow the publication order: A is synchronized before
//! replacing B. Mixed fence writes and damaged roots are separate fault classes.
//! These constructed images test recovery rules, not a storage device's durability.
//!
//! Each image states either an expected recovered state or a specific refusal.
//! Refusal must preserve persistent files; successful recovery must settle roots and
//! retain complete expected rows. Further cuts during recovery test that the repair
//! can be repeated after interruption, not merely applied once to a damaged image.

use super::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
};

#[derive(Clone)]
struct Roots {
    a: Vec<u8>,
    b: Vec<u8>,
    fence: Vec<u8>,
}

impl Roots {
    fn read(path: &Path) -> Result<Self> {
        let roots = Self {
            a: fs::read(path.join("ROOT.A"))?,
            b: fs::read(path.join("ROOT.B"))?,
            fence: fs::read(path.join("WAL"))?,
        };
        if roots.a.len() != 4096 || roots.b.len() != 4096 || roots.fence.len() != 512 {
            return Err("unexpected root or fence extent in image source".into());
        }
        Ok(roots)
    }
}

#[derive(Clone, Copy)]
enum Refusal {
    Authority,
    MissingCatalog,
}

struct Image {
    label: String,
    source: PathBuf,
    replacements: BTreeMap<String, Option<Vec<u8>>>,
    state: u32,
    refusal: Option<Refusal>,
    repeat_recovery: bool,
}

impl Image {
    fn new(label: String, source: &Path, roots: Roots, state: u32) -> Self {
        Self {
            label,
            source: source.to_owned(),
            replacements: [
                ("ROOT.A".into(), Some(roots.a)),
                ("ROOT.B".into(), Some(roots.b)),
                ("WAL".into(), Some(roots.fence)),
            ]
            .into(),
            state,
            refusal: None,
            repeat_recovery: false,
        }
    }
}

fn mixed_fences(old: &[u8], new: &[u8]) -> Result<BTreeSet<Vec<u8>>> {
    let mut mixed = BTreeSet::new();
    for split in 0..=512 {
        mixed.insert([&new[..split], &old[split..]].concat());
        mixed.insert([&old[..split], &new[split..]].concat());
    }
    mixed.remove(old);
    mixed.remove(new);
    if mixed.is_empty() {
        return Err("image sources did not change the fence".into());
    }
    for bytes in &mixed {
        let expected = u32::from_le_bytes(bytes[108..112].try_into()?);
        let mut copy = bytes.clone();
        copy[108..112].fill(0);
        if catalog::crc32c(&copy) == expected {
            return Err("mixed fence unexpectedly has a valid checksum".into());
        }
    }
    Ok(mixed)
}

fn images(seed: &Path, issued: &Path, complete: &Path) -> Result<Vec<Image>> {
    let sources = [seed, issued, complete];
    let roots = [
        Roots::read(seed)?,
        Roots::read(issued)?,
        Roots::read(complete)?,
    ];
    let mut images = Vec::new();
    for (transition, prefix) in ["issuance", "commit"].iter().enumerate() {
        let old = &roots[transition];
        let new = &roots[transition + 1];
        let source = sources[transition + 1];
        let before = transition as u32;
        let after = before + 1;
        for (label, a, b, fence, state) in [
            ("fence-write-lost", old, old, old, before),
            ("first-name-lost", old, old, new, before),
            ("second-name-lost", new, old, new, after),
            ("both-names-retained", new, new, new, after),
        ] {
            images.push(Image::new(
                format!("{prefix}-{label}"),
                source,
                Roots {
                    a: a.a.clone(),
                    b: b.b.clone(),
                    fence: fence.fence.clone(),
                },
                state,
            ));
        }
        let mut pending = Image::new(
            format!("{prefix}-staging-name-retained"),
            source,
            Roots {
                fence: new.fence.clone(),
                ..old.clone()
            },
            before,
        );
        pending
            .replacements
            .insert("ROOT.A.next".into(), Some(new.a.clone()));
        images.push(pending);
        let mut pending = Image::new(
            format!("{prefix}-second-staging-name-retained"),
            source,
            Roots {
                b: old.b.clone(),
                ..new.clone()
            },
            after,
        );
        pending
            .replacements
            .insert("ROOT.B.next".into(), Some(new.b.clone()));
        images.push(pending);

        let mixed = mixed_fences(&old.fence, &new.fence)?;
        for (index, fence) in mixed.iter().enumerate() {
            let mut image = Image::new(
                format!("{prefix}-torn-fence-{index}"),
                source,
                Roots {
                    fence: fence.clone(),
                    ..old.clone()
                },
                before,
            );
            image.repeat_recovery = index == 0;
            images.push(image);
        }
        let mut damaged_a = new.a.clone();
        damaged_a[108] ^= 1;
        let mut damaged_b = new.b.clone();
        damaged_b[108] ^= 1;
        let mut image = Image::new(
            format!("{prefix}-one-root-damaged"),
            source,
            Roots {
                a: damaged_a.clone(),
                ..new.clone()
            },
            after,
        );
        image.repeat_recovery = true;
        images.push(image);
        for (label, b, fence, state) in [
            ("newer-root-damaged", &old.b, &new.fence, before),
            ("both-roots-damaged", &damaged_b, &new.fence, after),
            (
                "root-and-fence-damaged",
                &new.b,
                mixed.first().unwrap(),
                after,
            ),
        ] {
            let mut image = Image::new(
                format!("{prefix}-{label}"),
                source,
                Roots {
                    a: damaged_a.clone(),
                    b: b.clone(),
                    fence: fence.clone(),
                },
                state,
            );
            image.refusal = Some(Refusal::Authority);
            images.push(image);
        }
        let attempt = u64::from_le_bytes(new.a[128..136].try_into()?);
        let ordinal = u32::from_le_bytes(new.a[136..140].try_into()?);
        let mut image = Image::new(
            format!("{prefix}-missing-catalog"),
            source,
            new.clone(),
            after,
        );
        image
            .replacements
            .insert(format!("units/{attempt:016x}-{ordinal:08x}.obj"), None);
        image.refusal = Some(Refusal::MissingCatalog);
        images.push(image);
    }
    Ok(images)
}

pub(super) fn run(
    recovery: &mut Recovery<'_>,
    seed: &Path,
    issued: &Path,
    complete: &Path,
) -> Result<()> {
    let images = images(seed, issued, complete)?;
    let count = images.len();
    let mut refused = 0;
    let mut cuts = 0;
    let mut records = fs::File::create(recovery.run.directory.join("images.jsonl"))?;
    for image in images {
        let label = format!("persisted-{}", image.label);
        let db = recovery.run.directory.join(&label);
        writeln!(
            records,
            "{}",
            json!({"case": label, "source": image.source, "state": image.state, "repeat_recovery": image.repeat_recovery, "complete": false})
        )?;
        records.flush()?;
        copy_tree(&image.source, &db)?;
        for name in ["ROOT.A.next", "ROOT.B.next"] {
            match fs::remove_file(db.join(name)) {
                Ok(()) => (),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => return Err(error.into()),
            }
        }
        for (name, bytes) in image.replacements {
            if let Some(bytes) = bytes {
                fs::write(db.join(name), bytes)?;
            } else {
                fs::remove_file(db.join(name))?;
            }
        }
        if let Some(refusal) = image.refusal {
            let (reason, mode) = match refusal {
                Refusal::Authority => ("insufficient root authority", "refuse"),
                Refusal::MissingCatalog => ("missing referenced object", "refuse-missing"),
            };
            let error = catalog::inspect(&db).expect_err("reader accepted damaged authority");
            if !error.to_string().contains(reason) {
                return Err(format!("{label}: expected {reason}, got {error}").into());
            }
            let before = workspace::tree_contents(&db)?;
            recovery.healthy(&db, mode, image.state)?;
            if workspace::tree_contents(&db)? != before {
                return Err(format!("{label}: refusal changed persistent files").into());
            }
            refused += 1;
        } else {
            recovery.check_graph(&catalog::inspect(&db)?, image.state, false)?;
            if image.repeat_recovery {
                cuts += recovery.check_recovery_cuts(&db, image.state)?;
            }
            recovery.healthy(&db, "recover", image.state)?;
            recovery.check_graph(&catalog::inspect(&db)?, image.state, false)?;
            recovery.healthy(&db, "verify", image.state)?;
            let graph = catalog::inspect(&db)?;
            recovery.check_graph(&graph, image.state, true)?;
            if graph["roots_settled"] != true {
                return Err(format!("{label}: recovery left unsettled roots").into());
            }
        }
        writeln!(records, "{}", json!({"case": label, "complete": true}))?;
    }
    println!("persisted images: {count} checked, {refused} refused, {cuts} recovery cuts");
    Ok(())
}
