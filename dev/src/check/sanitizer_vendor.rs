//! Merge diagnostic dependencies without replacing the engine's pinned packages.
//!
//! The destination is an owned copy, separate from both input directories. Package
//! names and versions are read from normalized vendored manifests; unfamiliar or
//! conflicting identities fail rather than being guessed. Additional standard-library
//! dependencies can then coexist with the engine inventory. Tests exercise conflicts
//! and overlapping paths and verify that the original engine vendor tree stays
//! unchanged when a merge is rejected.

use crate::{Result, workspace};
use std::{collections::BTreeMap, fs, path::Path};

// Cargo vendor writes normalized manifests. Accept literal package identities
// only; an unfamiliar representation must fail instead of guessing its meaning.
fn package(path: &Path) -> Result<(String, String)> {
    let text = fs::read_to_string(path.join("Cargo.toml"))?;
    let mut inside = false;
    let mut name = None;
    let mut version = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == "[package]";
            continue;
        }
        if !inside {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let target = match key.trim() {
            "name" => &mut name,
            "version" => &mut version,
            _ => continue,
        };
        let value = value
            .trim()
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .ok_or("vendor package identity is not a literal string")?;
        if value.is_empty()
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b))
            || target.replace(value.to_owned()).is_some()
        {
            return Err("invalid or repeated vendor package identity".into());
        }
    }
    Ok((
        name.ok_or("vendor package name is missing")?,
        version.ok_or("vendor package version is missing")?,
    ))
}

pub(super) fn merge(engine: &Path, supplied: &Path, destination: &Path) -> Result<()> {
    let engine = engine.canonicalize()?;
    let supplied = supplied.canonicalize()?;
    if !destination.is_absolute()
        || destination.exists()
        || destination.starts_with(&engine)
        || destination.starts_with(&supplied)
        || engine.starts_with(destination)
        || supplied.starts_with(destination)
    {
        return Err(
            "diagnostic vendor destination must be fresh and separate from its inputs".into(),
        );
    }
    let before_engine = workspace::tree_contents(&engine)?;
    let before_supplied = workspace::tree_contents(&supplied)?;
    if before_supplied.is_empty() {
        return Err("diagnostic dependency directory is empty".into());
    }
    workspace::copy_tree(&engine, destination)?;
    if workspace::tree_contents(destination)? != before_engine {
        return Err("engine dependencies changed while copying".into());
    }
    let mut packages = BTreeMap::new();
    for entry in fs::read_dir(destination)? {
        let entry = entry?;
        if entry.file_name() == "README.md" && entry.file_type()?.is_file() {
            continue;
        }
        if !entry.file_type()?.is_dir() {
            return Err("expected vendor package directory".into());
        }
        if packages
            .insert(package(&entry.path())?, entry.path())
            .is_some()
        {
            return Err("duplicate engine vendor package".into());
        }
    }
    let mut entries = fs::read_dir(&supplied)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for (index, entry) in entries.into_iter().enumerate() {
        if !entry.file_type()?.is_dir() {
            return Err("expected diagnostic package directory".into());
        }
        let identity = package(&entry.path())?;
        let content = workspace::tree_contents(&entry.path())?;
        if let Some(existing) = packages.get(&identity) {
            if workspace::tree_contents(existing)? != content {
                return Err(format!(
                    "diagnostic dependency would replace engine package {} {}",
                    identity.0, identity.1
                )
                .into());
            }
        } else {
            let path = destination.join(format!("diagnostic-{index}"));
            workspace::copy_tree(&entry.path(), &path)?;
            if workspace::tree_contents(&path)? != content {
                return Err("diagnostic package changed while copying".into());
            }
            packages.insert(identity, path);
        }
    }
    if workspace::tree_contents(&engine)? != before_engine
        || workspace::tree_contents(&supplied)? != before_supplied
    {
        return Err("dependency inputs changed during assembly".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Run;
    fn seed(root: &Path, directory: &str, name: &str, version: &str, body: &str) {
        let path = root.join(directory);
        fs::create_dir(&path).unwrap();
        fs::write(
            path.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\n"),
        )
        .unwrap();
        fs::write(path.join("lib.rs"), body).unwrap();
    }
    #[test]
    fn merge_preserves_engine_packages_and_refuses_replacement() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_owned();
        let run = Run::new(root, "sanitizer-vendor-controls").unwrap();
        let engine = run.directory.join("engine");
        let supplied = run.directory.join("supplied");
        fs::create_dir(&engine).unwrap();
        fs::create_dir(&supplied).unwrap();
        seed(&engine, "core", "sample", "1.0.0", "original");
        fs::write(engine.join("README.md"), "Dependency guide\n").unwrap();
        seed(&supplied, "sample-1", "sample", "1.0.0", "original");
        seed(&supplied, "sample-2", "sample", "2.0.0", "new version");
        let before = workspace::tree_contents(&engine).unwrap();
        let merged = run.directory.join("merged");
        merge(&engine, &supplied, &merged).unwrap();
        assert_eq!(fs::read_dir(&merged).unwrap().count(), 3);
        assert_eq!(
            fs::read(merged.join("README.md")).unwrap(),
            b"Dependency guide\n"
        );
        assert_eq!(workspace::tree_contents(&engine).unwrap(), before);
        fs::write(engine.join("unexpected.txt"), "not a package").unwrap();
        let error = merge(&engine, &supplied, &run.directory.join("unexpected")).unwrap_err();
        assert_eq!(error.to_string(), "expected vendor package directory");
        fs::remove_file(engine.join("unexpected.txt")).unwrap();
        fs::write(supplied.join("sample-1/lib.rs"), "changed").unwrap();
        assert!(merge(&engine, &supplied, &run.directory.join("conflict")).is_err());
        assert_eq!(workspace::tree_contents(&engine).unwrap(), before);
        assert!(merge(&engine, &supplied, &engine.join("overlap")).is_err());
        run.finish().unwrap();
    }
}
