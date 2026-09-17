//! Own diagnostic dependency copies and identify the rebuilt standard library.
//!
//! Preparation records the installed standard-library source and creates a private
//! dependency copy, retaining both file inventories for later comparison.
//! Cargo output must identify one standard-library artifact inside the expected
//! target directory. Missing, ambiguous, truncated or escaping paths are errors;
//! merely requesting a rebuild does not prove that the test used it. The dependency
//! merge preserves the engine's pinned packages while supplying compiler needs.

use super::*;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(super) struct StandardLibrary {
    source: PathBuf,
    source_before: BTreeMap<PathBuf, workspace::EntryContent>,
    vendor: PathBuf,
    vendor_before: BTreeMap<PathBuf, workspace::EntryContent>,
}

impl StandardLibrary {
    pub(super) fn control(
        &self,
        run: &mut Run,
        toolchain: &str,
        host: &str,
        directory: &Path,
        kind: &str,
    ) -> Result<PathBuf> {
        let manifest = directory.join("Cargo.toml");
        let source = run.root.join(if kind == "thread" {
            "dev/driver/src/thread_control.rs"
        } else {
            "dev/driver/src/address_control.rs"
        });
        fs::write(
            &manifest,
            format!(
                "[package]\nname = \"sanitizer-control\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n[[bin]]\nname = \"sanitizer-control\"\npath = {}\n[profile.release]\ndebug = true\n",
                serde_json::to_string(&source)?
            ),
        )?;
        let target = run.directory.join(kind);
        let configure = |command: &mut Command| {
            environment(command);
            command
                .env("CARGO_TARGET_DIR", &target)
                .env("RUSTFLAGS", format!("-Dwarnings -Zsanitizer={kind}"));
        };
        let mut lock = Command::new("cargo");
        lock.args([
            &format!("+{toolchain}"),
            "generate-lockfile",
            "--offline",
            "--manifest-path",
        ])
        .arg(&manifest);
        configure(&mut lock);
        run.command(&mut lock, None, Duration::from_secs(30))?
            .require_success()?;
        let mut build = Command::new("cargo");
        build
            .args([
                &format!("+{toolchain}"),
                "build",
                "--release",
                "--offline",
                "--locked",
                "-j",
                "1",
                "--target",
                host,
                "--manifest-path",
            ])
            .arg(&manifest)
            .arg("--message-format=json");
        configure(&mut build);
        self.configure(&mut build)?;
        let output = run.command(&mut build, None, Duration::from_secs(600))?;
        let library = rebuilt(&output, &target)?;
        fs::write(
            run.directory.join("control-standard-library.sha256"),
            workspace::hash(&library)?,
        )?;
        let binary = workspace::cargo_artifact(
            &output,
            "sanitizer-control",
            workspace::Artifact::Executable { test: false },
        )?
        .canonicalize()?;
        if !binary.starts_with(target.canonicalize()?) {
            return Err("rebuilt control is outside its owned target".into());
        }
        Ok(binary)
    }

    pub(super) fn prepare(run: &mut Run, toolchain: &str, supplied: &Path) -> Result<Self> {
        let mut command = Command::new("rustc");
        command.args([&format!("+{toolchain}"), "--print", "sysroot"]);
        environment(&mut command);
        let output = run.command(&mut command, None, Duration::from_secs(30))?;
        output.require_success()?;
        let source = Path::new(std::str::from_utf8(&output.stdout.bytes)?.trim())
            .join("lib/rustlib/src/rust/library");
        let source_before = workspace::tree_contents(&source)?;
        if !source_before.contains_key(Path::new("Cargo.lock"))
            || !source_before.contains_key(Path::new("std/Cargo.toml"))
        {
            return Err("diagnostic toolchain requires matching rust-src".into());
        }
        let vendor = run.directory.join("standard-vendor");
        vendor::merge(&run.root.join("vendor"), supplied, &vendor)?;
        let vendor_before = workspace::tree_contents(&vendor)?;
        for (name, contents) in [
            ("standard-library-inputs.json", &source_before),
            ("standard-vendor-inputs.json", &vendor_before),
        ] {
            let files: BTreeMap<_, _> = contents
                .iter()
                .map(|(path, value)| {
                    let value = match value {
                        workspace::EntryContent::Directory => serde_json::Value::Null,
                        workspace::EntryContent::File(hash) => serde_json::json!(hash),
                    };
                    (path.to_string_lossy().into_owned(), value)
                })
                .collect();
            fs::write(run.directory.join(name), serde_json::to_vec_pretty(&files)?)?;
        }
        Ok(Self {
            source,
            source_before,
            vendor,
            vendor_before,
        })
    }

    pub(super) fn configure(&self, command: &mut Command) -> Result<()> {
        command.arg("-Zbuild-std").arg("--config").arg(format!(
            "source.vendored-sources.directory={}",
            serde_json::to_string(&self.vendor)?
        ));
        Ok(())
    }

    pub(super) fn unchanged(&self) -> Result<()> {
        if workspace::tree_contents(&self.source)? != self.source_before
            || workspace::tree_contents(&self.vendor)? != self.vendor_before
        {
            return Err("standard-library inputs changed during instrumentation".into());
        }
        Ok(())
    }
}

pub(super) fn rebuilt(output: &Output, target: &Path) -> Result<PathBuf> {
    let library = workspace::cargo_artifact(output, "std", workspace::Artifact::Library("rlib"))?
        .canonicalize()?;
    if !library.starts_with(target.canonicalize()?) {
        return Err("rebuilt standard library is outside the owned target".into());
    }
    Ok(library)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{Capture, Completion};
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn rebuilt_library_must_be_unique_and_inside_the_owned_target() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_owned();
        let run = Run::new(root, "standard-artifact-controls").unwrap();
        let target = run.directory.join("target");
        fs::create_dir(&target).unwrap();
        let path = target.join("libstd-test.rlib");
        fs::write(&path, []).unwrap();
        let event = serde_json::json!({"reason": "compiler-artifact", "target": {"name": "std"}, "filenames": [path]});
        let make = |text: String| Output {
            completion: Completion::Exited(std::process::ExitStatus::from_raw(0)),
            stdout: Capture {
                bytes: text.into_bytes(),
                omitted: 0,
            },
            stderr: Capture::default(),
        };
        assert_eq!(
            rebuilt(&make(event.to_string()), &target).unwrap(),
            path.canonicalize().unwrap()
        );
        assert!(rebuilt(&make(String::new()), &target).is_err());
        assert!(rebuilt(&make(format!("{event}\n{event}\n")), &target).is_err());
        let other = run.directory.join("other");
        fs::create_dir(&other).unwrap();
        assert!(rebuilt(&make(event.to_string()), &other).is_err());
        let mut truncated = make(event.to_string());
        truncated.stdout.omitted = 1;
        assert!(rebuilt(&truncated, &target).is_err());
        run.finish().unwrap();
    }
}
