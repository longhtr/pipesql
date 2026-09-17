//! Challenge build-artifact selection, frozen inputs and recorded command settings.
//!
//! Synthetic Cargo output must identify exactly one artifact of the requested kind;
//! wrong profiles, malformed records, failed builds and truncation must be refused.
//! Small source exports test replay without Git, including changed bytes, duplicate
//! paths, escape attempts and false identities. A real child verifies that explicit
//! environment changes and its working directory also appear in the command record.

use super::*;

#[test]
fn command_record_preserves_explicit_environment_and_directory() {
    let directory = Directory::new();
    let mut run = Run::new(directory.0.clone(), "command-settings").unwrap();
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "test \"$PIPESQL_RECORD_CONTROL\" = chosen"])
        .env("PIPESQL_RECORD_CONTROL", "chosen")
        .env_remove("PIPESQL_REMOVED_CONTROL");
    run.command(&mut command, None, Duration::from_secs(5))
        .unwrap()
        .require_success()
        .unwrap();
    let records: Vec<Value> = fs::read_to_string(run.directory.join("commands.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records[1]["directory"], json!(directory.0));
    assert_eq!(
        records[1]["environment"],
        json!({
            "PIPESQL_RECORD_CONTROL": "chosen", "PIPESQL_REMOVED_CONTROL": null
        })
    );
    let output = run.directory.clone();
    run.finish().unwrap();
    fs::remove_dir_all(output).unwrap();
}

#[test]
fn cargo_selection_rejects_ambiguous_incomplete_and_wrong_profile_outputs() {
    use crate::process::{Capture, Completion, Output};
    use std::os::unix::process::ExitStatusExt;

    let executable = json!({"reason": "compiler-artifact", "target": {"name": "probe"},
        "profile": {"test": true}, "executable": "/target/probe"});
    let make = |text: String| Output {
        completion: Completion::Exited(std::process::ExitStatus::from_raw(0)),
        stdout: Capture {
            bytes: text.into_bytes(),
            omitted: 0,
        },
        stderr: Capture::default(),
    };
    let select =
        |output: &Output| cargo_artifact(output, "probe", Artifact::Executable { test: true });
    assert_eq!(
        select(&make(executable.to_string())).unwrap(),
        Path::new("/target/probe")
    );
    for text in [
        String::new(),
        format!("{executable}\n{executable}"),
        format!("{executable}\nmalformed"),
        executable.to_string().replace("true", "false"),
        executable.to_string().replace("/target/probe", ""),
    ] {
        assert!(select(&make(text)).is_err());
    }
    let mut truncated = make(executable.to_string());
    truncated.stdout.omitted = 1;
    assert!(select(&truncated).is_err());
    let mut failed = make(executable.to_string());
    failed.completion = Completion::Exited(std::process::ExitStatus::from_raw(256));
    assert!(select(&failed).is_err());

    let library = json!({"reason": "compiler-artifact", "target": {"name": "native"},
        "filenames": ["/target/libnative.rlib", "/target/libnative.so"]});
    let select = |text: String| cargo_artifact(&make(text), "native", Artifact::Library("so"));
    assert_eq!(
        select(library.to_string()).unwrap(),
        Path::new("/target/libnative.so")
    );
    let mut ambiguous = library.clone();
    ambiguous["filenames"][0] = json!("/other/libnative.so");
    assert!(select(ambiguous.to_string()).is_err());
    let mut malformed = library;
    malformed["filenames"][0] = json!(42);
    assert!(select(malformed.to_string()).is_err());
}

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "pipesql-source-test-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn exported(root: &Path) -> Value {
    fs::create_dir_all(root.join("dev/src")).unwrap();
    fs::write(root.join("Cargo.toml"), b"example cargo input\n").unwrap();
    fs::write(root.join("dev/src/main.rs"), b"fn main() {}\n").unwrap();
    let files: Vec<_> = ["Cargo.toml", "dev/src/main.rs"]
        .iter()
        .map(|path| json!({"path": path, "sha256": hash(&root.join(path)).unwrap()}))
        .collect();
    let identity = format!("{:x}", Sha256::digest(serde_json::to_vec(&files).unwrap()));
    let manifest = json!({"sha256": identity, "files": files});
    fs::write(
        root.join(".pipesql-source.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    manifest
}

#[test]
fn export_replay_checks_every_input_without_git() {
    let directory = Directory::new();
    let root = directory.0.join("input");
    let manifest = exported(&root);
    fs::write(root.join("unrelated.txt"), b"not a build input").unwrap();
    let destination = directory.0.join("copy");
    assert_eq!(freeze(&root, &destination).unwrap(), manifest["sha256"]);
    assert!(!destination.join("unrelated.txt").exists());
    assert_eq!(
        freeze(&destination, &directory.0.join("second-copy")).unwrap(),
        manifest["sha256"]
    );

    fs::write(root.join("dev/src/main.rs"), b"fn changed() {}\n").unwrap();
    let error = freeze(&root, &directory.0.join("changed")).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("differs from its recorded identity")
    );
}

#[test]
fn export_rejects_escape_duplicate_and_false_identity() {
    let directory = Directory::new();
    let root = directory.0.join("input");
    let valid = exported(&root);
    for (index, path) in ["../outside", "/outside", "", "Cargo.toml"]
        .iter()
        .enumerate()
    {
        let mut invalid = valid.clone();
        invalid["files"][1]["path"] = json!(path);
        fs::write(
            root.join(".pipesql-source.json"),
            serde_json::to_vec(&invalid).unwrap(),
        )
        .unwrap();
        assert!(freeze(&root, &directory.0.join(format!("invalid-{index}"))).is_err());
    }
    let mut invalid = valid;
    invalid["sha256"] = json!("wrong");
    fs::write(
        root.join(".pipesql-source.json"),
        serde_json::to_vec(&invalid).unwrap(),
    )
    .unwrap();
    assert!(
        freeze(&root, &directory.0.join("wrong-identity"))
            .unwrap_err()
            .to_string()
            .contains("differs from its recorded identity")
    );
}
