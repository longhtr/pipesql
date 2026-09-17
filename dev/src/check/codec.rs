//! Compare independent format vectors with retained files and reject old prototypes.
//!
//! Catalog and snapshot encoders supply exact expected filenames and bytes; the
//! comparison rejects missing, extra or changed records. The rejected-format case
//! opens private copies with the stock CLI and requires refusal without file changes.
//! A separate rounding selection checks numeric conversion against exact arithmetic.
//! Unknown selections fail, and comparison controls deliberately alter inputs so a
//! broken byte checker cannot quietly certify its own output.

use crate::{
    Result,
    oracle::catalog_vectors,
    workspace::{self, Run},
};
use std::{collections::BTreeMap, fs, path::Path, process::Command, time::Duration};

fn rejected(run: &mut Run) -> Result<()> {
    let cli = run.build("pipesql", "--bin", "pipesql")?;
    let data = run.root.join("test/data");
    for (version, directory) in [
        (1_u32, "rejected-single-table-format"),
        (2, "rejected-attempt-reuse-format"),
        (3, "rejected-multi-table-format"),
        (5, "candidate-multi-table-format"),
    ] {
        for literal in [true, false] {
            let path = run.directory.join(format!("version-{version}-{literal}"));
            crate::oracle::snapshot::write(&path, &data.join("current-single-table-format"), &[])?;
            let control = if literal {
                fs::read(data.join(directory).join("CONTROL"))?
            } else {
                let mut bytes = fs::read(path.join("CONTROL"))?;
                bytes[8..12].copy_from_slice(&version.to_le_bytes());
                bytes[36..40].fill(0);
                let crc = crate::oracle::catalog::crc32c(&bytes);
                bytes[36..40].copy_from_slice(&crc.to_le_bytes());
                bytes
            };
            fs::write(path.join("CONTROL"), control)?;
            let before = workspace::tree_contents(&path)?;
            let mut command = Command::new(&cli);
            command.arg("open").arg("--database").arg(&path).args([
                "--memory-limit-bytes",
                "2000000",
                "--temp-limit-bytes",
                "1000000",
            ]);
            let output = run.command(&mut command, None, Duration::from_secs(30))?;
            if !matches!(output.completion, crate::process::Completion::Exited(status) if status.code() == Some(1))
                || output.stdout.omitted != 0
                || output.stderr.omitted != 0
                || !output.stdout.bytes.is_empty()
                || output.stderr.bytes
                    != format!("database error: unsupported database format version {version}\n")
                        .as_bytes()
                || workspace::tree_contents(&path)? != before
            {
                return Err(format!("prototype format {version} was not rejected without changes (literal={literal})").into());
            }
        }
    }
    println!("Codec rejected: 8 literal and mutated version checks passed");
    Ok(())
}

fn compare(directory: &Path, expected: &BTreeMap<&str, Vec<u8>>) -> Result<usize> {
    let mut actual = BTreeMap::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Err(format!(
                "codec sample is not an ordinary file: {}",
                entry.path().display()
            )
            .into());
        }
        actual.insert(
            entry
                .file_name()
                .into_string()
                .map_err(|_| "non-UTF-8 codec sample name")?,
            fs::read(entry.path())?,
        );
    }
    if actual.len() != expected.len() {
        return Err(format!("codec sample file set differs: {}", directory.display()).into());
    }
    for (name, bytes) in expected {
        if actual.get(*name) != Some(bytes) {
            return Err(
                format!("codec sample bytes differ: {}/{name}", directory.display()).into(),
            );
        }
    }
    Ok(expected.len())
}

pub fn run(case: &str) -> Result<()> {
    if !["catalog", "snapshot", "rejected", "rounding"].contains(&case) {
        return Err("unknown codec case".into());
    }
    let mut run = Run::new(workspace::root()?, "codec")?;
    if case == "rounding" {
        let text = fs::read_to_string(run.root.join("test/data/aggregate-semantics/rounding.txt"))?;
        let count = crate::oracle::rounding::check(&text)?;
        if count != 600 {
            return Err("retained rounding input set is incomplete".into());
        }
        println!("Codec rounding: {count} exact SUM and AVG references passed");
        return run.finish();
    }
    if case == "rejected" {
        rejected(&mut run)?;
        return run.finish();
    }
    let mut count = 0;
    let vectors = match case {
        "catalog" => catalog_vectors::vectors(),
        "snapshot" => BTreeMap::from([(
            "current-single-table-format",
            crate::oracle::snapshot::vectors()?,
        )]),
        _ => unreachable!(),
    };
    for (directory, files) in vectors {
        count += compare(&run.root.join("test/data").join(directory), &files)?;
    }
    println!("Codec {case}: {count} complete independent encodings passed");
    run.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn comparison_rejects_missing_extra_changed_and_linked_files() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let run = Run::new(root, "codec-controls").unwrap();
        let directory = run.directory.join("samples");
        fs::create_dir(&directory).unwrap();
        let expected = BTreeMap::from([("record", vec![1, 2, 3])]);
        assert!(compare(&directory, &expected).is_err());
        fs::write(directory.join("record"), [1, 2, 3]).unwrap();
        assert_eq!(compare(&directory, &expected).unwrap(), 1);
        fs::write(directory.join("extra"), []).unwrap();
        assert!(compare(&directory, &expected).is_err());
        fs::remove_file(directory.join("extra")).unwrap();
        fs::write(directory.join("record"), [1, 2, 4]).unwrap();
        assert!(compare(&directory, &expected).is_err());
        fs::remove_file(directory.join("record")).unwrap();
        let target = run.directory.join("target");
        fs::write(&target, [1, 2, 3]).unwrap();
        symlink(&target, directory.join("record")).unwrap();
        assert!(compare(&directory, &expected).is_err());
        fs::remove_file(directory.join("record")).unwrap();
        fs::remove_file(target).unwrap();
        run.finish().unwrap();
    }
}
