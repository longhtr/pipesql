//! Check report reads against admission refusal, damaged storage and wrong answers.
//!
//! Private copies of a valid multi-table report isolate each change. The independent
//! reader and stock driver must reject damaged selected columns with the expected
//! reason; read-only failures must leave files unchanged where required. Other
//! cases restore permissions or reopen unsettled roots and require the same rows.
//! A validly encoded wrong amount is a control: decoding success must still fail
//! the literal report comparison.

use super::*;
use std::os::unix::fs::PermissionsExt;
use workspace::tree_contents;

fn rejected(
    c: &mut Campaign,
    path: &Path,
    reason: &str,
    mode: &str,
    unchanged: bool,
) -> Result<()> {
    let error = catalog::inspect(path).expect_err("independent reader accepted damaged report");
    if !error.to_string().contains(reason) {
        return Err(format!("report: expected {reason}, got {error}").into());
    }
    let before = unchanged.then(|| tree_contents(path)).transpose()?;
    c.driver(path, mode)?;
    if let Some(before) = before
        && tree_contents(path)? != before
    {
        return Err("refused admission changed stored files".into());
    }
    Ok(())
}

struct Permissions {
    path: PathBuf,
    original: fs::Permissions,
}

impl Drop for Permissions {
    fn drop(&mut self) {
        if let Err(error) = fs::set_permissions(&self.path, self.original.clone()) {
            eprintln!("restore directory permissions: {error}");
        }
    }
}

pub(super) fn run(c: &mut Campaign) -> Result<()> {
    let seed = c.run.directory.join("report-seed");
    c.driver(&seed, "report-setup")?;
    let baseline = catalog::inspect(&seed)?;
    if baseline["tables"][0]["name"] != "events"
        || baseline["tables"][0]["rows"].as_array().map(Vec::len) != Some(16)
        || baseline["tables"][1]["name"] != "dimensions"
        || baseline["tables"][1]["rows"].as_array().map(Vec::len) != Some(4)
    {
        return Err("wrong report setup".into());
    }
    for label in ["measurement", "version"] {
        let path = c.run.directory.join(format!("report-{label}"));
        copy_tree(&seed, &path)?;
        let mut mutation = Mutation::new(&path)?;
        if label == "measurement" {
            mutation.payload(5, false, |data, at| data[at] ^= 1)?;
            rejected(
                c,
                &path,
                "payload checksum",
                "report-measurement-reject",
                false,
            )?;
        } else {
            mutation.roots(|data| put(data, 8, 8, 4))?;
            rejected(
                c,
                &path,
                "unsupported authoritative version",
                "report-version-reject",
                true,
            )?;
        }
    }
    let path = c.run.directory.join("report-newer-corrupt");
    copy_tree(&seed, &path)?;
    let older_root = fs::read(seed.with_extension("older-root"))?;
    fs::write(path.join("ROOT.B"), &older_root)?;
    mutation::root(&path.join("ROOT.B"), |data| data[32] = 1)?;
    if catalog::inspect(&path)?["tables"] != baseline["tables"] {
        return Err("newer report root was not selected".into());
    }
    let older = c.run.directory.join("report-older-valid");
    copy_tree(&seed, &older)?;
    Mutation::new(&older)?.roots(|data| {
        data[40..80].copy_from_slice(&older_root[40..80]);
        data[112..176].copy_from_slice(&older_root[112..176]);
    })?;
    let graph = catalog::inspect(&older)?;
    if graph["tables"][0]["rows"] != json!(baseline["tables"][0]["rows"].as_array().unwrap()[..8])
        || graph["tables"][1] != baseline["tables"][1]
    {
        return Err("older report root does not retain the original rows".into());
    }
    let catalog = Mutation::new(&path)?.catalog_path();
    let original = fs::read(&catalog)?;
    damage(&catalog, 64)?;
    rejected(c, &path, "object checksum", "report-corrupt-reject", true)?;
    fs::write(catalog, original)?;
    c.driver(&path, "report-healthy")?;
    if catalog::inspect(&path)?["tables"] != baseline["tables"] {
        return Err("restored report changed rows".into());
    }

    let path = c.run.directory.join("report-failed-recovery");
    copy_tree(&seed, &path)?;
    damage(&path.join("ROOT.A"), 108)?;
    if catalog::inspect(&path)?["roots_settled"] != false {
        return Err("damaged report did not require recovery".into());
    }
    let permissions = Permissions {
        path: path.clone(),
        original: fs::metadata(&path)?.permissions(),
    };
    fs::set_permissions(&path, fs::Permissions::from_mode(0o555))?;
    let result = c.driver(&path, "report-recovery-reject");
    fs::set_permissions(&path, permissions.original.clone())?;
    drop(permissions);
    result?;
    c.driver(&path, "report-healthy")?;
    let healed = catalog::inspect(&path)?;
    if healed["roots_settled"] != true || healed["tables"] != baseline["tables"] {
        return Err("report recovery did not preserve data and settle roots".into());
    }

    let path = c.run.directory.join("report-wrong-answer");
    copy_tree(&seed, &path)?;
    Mutation::new(&path)?.payload(4, true, |data, at| put(data, at + 1, 11, 8))?;
    if catalog::inspect(&path)?["tables"][0]["rows"][0][3] != 11 {
        return Err("wrong-answer control did not change the intended amount".into());
    }
    let mut command = Command::new(&c.driver);
    command.arg(&path).arg("report-healthy");
    let output = c.run.command(&mut command, None, Duration::from_secs(30))?;
    if !matches!(output.completion, crate::process::Completion::Exited(status) if !status.success())
        || !std::str::from_utf8(&output.stderr.bytes)?.contains("report group history")
    {
        return Err("wrong valid report answer passed comparison".into());
    }
    c.driver(&seed, "report-healthy")?;
    writeln!(
        c.records,
        "{}",
        json!({"case":"report-corruption", "complete":true, "controls":["measurement", "version", "newer-catalog", "failed-recovery", "wrong-answer"]})
    )?;
    Ok(())
}
