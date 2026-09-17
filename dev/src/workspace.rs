//! Own build artifacts, frozen source inputs and the diagnostics for one run.
//!
//! Run records command arguments and explicit environment changes before launch,
//! then retains output, completion and artifact hashes. Cargo supplies artifact
//! paths; the selector rejects failed, truncated or ambiguous build output.
//! Freezing copies the intended Git inventory, or verifies a prior export for replay,
//! and detects changed inputs. Successful finish removes owned working directories
//! while retaining logs; an unfinished run keeps its files for investigation.

use crate::{Result, process};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, PartialEq, Eq)]
pub enum EntryContent {
    Directory,
    File(String),
}

/// Record directory names and file bytes without following links or special files.
pub fn tree_contents(path: &Path) -> Result<std::collections::BTreeMap<PathBuf, EntryContent>> {
    let mut output = std::collections::BTreeMap::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let relative = PathBuf::from(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            output.insert(relative.clone(), EntryContent::Directory);
            for (child, content) in tree_contents(&entry.path())? {
                output.insert(relative.join(child), content);
            }
        } else if kind.is_file() {
            output.insert(relative, EntryContent::File(hash(&entry.path())?));
        } else {
            return Err("unexpected file type while recording database contents".into());
        }
    }
    Ok(output)
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;

pub fn root() -> Result<PathBuf> {
    let root = std::env::current_dir()?;
    if !root.join("dev/Cargo.toml").is_file() || !root.join("Cargo.toml").is_file() {
        return Err("run cargo dev from the repository root".into());
    }
    Ok(root)
}

pub fn hash(path: &Path) -> Result<String> {
    let mut input = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Copy an owned database seed; links and special files are never followed.
pub fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            return Err("unexpected file type in database seed".into());
        }
    }
    Ok(())
}

pub enum Artifact<'a> {
    Executable { test: bool },
    Library(&'a str),
}

/// Select exactly one Cargo output after checking completion and capture limits.
/// Callers that own a target directory must separately check the resolved path.
pub fn cargo_artifact(output: &process::Output, name: &str, kind: Artifact<'_>) -> Result<PathBuf> {
    output.require_success()?;
    let mut selected = None;
    let mut accept = |path: &str| -> Result<()> {
        if path.is_empty() || selected.replace(PathBuf::from(path)).is_some() {
            return Err(format!("Cargo reported empty or multiple artifacts for {name}").into());
        }
        Ok(())
    };
    for line in output
        .stdout
        .bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let event: Value = serde_json::from_slice(line)?;
        if event["reason"] != "compiler-artifact" || event["target"]["name"] != name {
            continue;
        }
        match kind {
            Artifact::Executable { test } => {
                if event["profile"]["test"] == test
                    && let Some(path) = event["executable"].as_str()
                {
                    accept(path)?;
                }
            }
            Artifact::Library(extension) => {
                for path in event["filenames"]
                    .as_array()
                    .ok_or("Cargo artifact has no filenames")?
                {
                    let path = path.as_str().ok_or("Cargo artifact filename is not text")?;
                    if Path::new(path)
                        .extension()
                        .is_some_and(|value| value == extension)
                    {
                        accept(path)?;
                    }
                }
            }
        }
    }
    selected.ok_or_else(|| format!("Cargo did not report artifact {name}").into())
}

pub struct Run {
    pub root: PathBuf,
    pub directory: PathBuf,
    commands: File,
    sequence: usize,
}

impl Run {
    pub fn new(root: PathBuf, name: &str) -> Result<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let parent =
            std::env::var_os("PIPESQL_RUN_ROOT").map_or_else(std::env::temp_dir, PathBuf::from);
        let directory = parent.join(format!("pipesql-{name}-{}-{stamp}", std::process::id()));
        fs::create_dir(&directory)?;
        eprintln!("run output: {}", directory.display());
        let commands = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("commands.jsonl"))?;
        let mut run = Self {
            root,
            directory,
            commands,
            sequence: 0,
        };
        writeln!(
            run.commands,
            "{}",
            json!({"case": name, "os": std::env::consts::OS, "architecture": std::env::consts::ARCH, "source": run.root})
        )?;
        Ok(run)
    }

    /// Write the invocation before starting it. Failed runs retain their directory;
    /// a caller explicitly finalizes a successful run after checking all artifacts.
    pub fn command(
        &mut self,
        command: &mut Command,
        output: Option<&Path>,
        timeout: Duration,
    ) -> Result<process::Output> {
        self.execute(command, output, timeout, false)
    }

    /// Finish owned work after cancellation, retaining the same command logs.
    pub fn cleanup_command(
        &mut self,
        command: &mut Command,
        timeout: Duration,
    ) -> Result<process::Output> {
        self.execute(command, None, timeout, true)
    }

    fn execute(
        &mut self,
        command: &mut Command,
        output: Option<&Path>,
        timeout: Duration,
        cleanup: bool,
    ) -> Result<process::Output> {
        self.sequence += 1;
        let number = self.sequence;
        let started = Instant::now();
        // Preserve deliberate overrides and removals, not the inherited host
        // environment. Allocator and sanitizer settings can change the experiment.
        let environment: std::collections::BTreeMap<_, _> = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy(),
                    value.map(|value| value.to_string_lossy()),
                )
            })
            .collect();
        writeln!(
            self.commands,
            "{}",
            json!({"command": number, "program": command.get_program().to_string_lossy(), "arguments": command.get_args().map(|s| s.to_string_lossy()).collect::<Vec<_>>(), "directory": self.root, "environment": environment, "timeout_seconds": timeout.as_secs_f64(), "output": output})
        )?;
        self.commands.flush()?;
        command.current_dir(&self.root);
        let result = if cleanup {
            process::run_cleanup(command, timeout)
        } else if let Some(path) = output {
            let file = OpenOptions::new().write(true).create_new(true).open(path)?;
            process::run_to_file(command, timeout, file)
        } else {
            process::run_supervisor(command, timeout)
        };
        match result {
            Ok(result) => {
                fs::write(
                    self.directory.join(format!("{number}.stdout")),
                    &result.stdout.bytes,
                )?;
                fs::write(
                    self.directory.join(format!("{number}.stderr")),
                    &result.stderr.bytes,
                )?;
                writeln!(
                    self.commands,
                    "{}",
                    json!({"command": number, "completion": format!("{:?}", result.completion), "seconds": started.elapsed().as_secs_f64(), "stdout_omitted": result.stdout.omitted, "stderr_omitted": result.stderr.omitted})
                )?;
                Ok(result)
            }
            Err(error) => {
                writeln!(
                    self.commands,
                    "{}",
                    json!({"command": number, "error": error.to_string(), "seconds": started.elapsed().as_secs_f64()})
                )?;
                Err(error.into())
            }
        }
    }

    pub fn build(&mut self, package: &str, selector: &str, name: &str) -> Result<PathBuf> {
        let mut command = Command::new("cargo");
        command.args([
            "build",
            "--offline",
            "--locked",
            "--release",
            "-j",
            "1",
            "-p",
            package,
            "--message-format=json-render-diagnostics",
        ]);
        command.arg(selector);
        if selector != "--lib" {
            command.arg(name);
        }
        let output = self.command(&mut command, None, Duration::from_secs(600))?;
        output.require_success()?;
        let kind = if selector == "--lib" {
            Artifact::Library(if cfg!(target_os = "macos") {
                "dylib"
            } else {
                "so"
            })
        } else {
            Artifact::Executable { test: false }
        };
        let executable = cargo_artifact(&output, name, kind)?;
        writeln!(
            self.commands,
            "{}",
            json!({"artifact": executable, "sha256": hash(&executable)?, "profile": "release", "package": package})
        )?;
        Ok(executable)
    }

    pub fn finish(mut self) -> Result<()> {
        self.commands.flush()?;
        drop(self.commands);
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                fs::remove_dir_all(entry.path())?;
            }
        }
        fs::write(
            self.directory.join("complete.json"),
            serde_json::to_vec_pretty(&json!({"success": true, "commands": self.sequence}))?,
        )?;
        Ok(())
    }
}

/// Copy the intended Git inventory, then verify every input again. Newly authored
/// files must be staged (intent-to-add is enough); unrelated untracked files are
/// never silently folded into a long check. An exported tree needs no Git to run.
pub fn freeze(root: &Path, destination: &Path) -> Result<String> {
    use std::os::unix::ffi::OsStringExt;
    fn inventory(root: &Path) -> Result<Vec<PathBuf>> {
        let mut command = Command::new("git");
        command
            .current_dir(root)
            .args(["ls-files", "--cached", "--deduplicate", "-z"]);
        let output = process::run(&mut command, Duration::from_secs(10))?;
        output.require_success()?;
        let mut paths = Vec::new();
        for bytes in output
            .stdout
            .bytes
            .split(|b| *b == 0)
            .filter(|b| !b.is_empty())
        {
            let path = PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec()));
            // A removed tracked file is an intended deletion in this checkout.
            if root.join(&path).try_exists()? {
                paths.push(path);
            }
        }
        paths.sort();
        Ok(paths)
    }
    fs::create_dir(destination)?;
    // An exported source tree carries its own exact inventory. Replaying it must
    // neither require Git nor silently accept edits under the old identity.
    let export_path = root.join(".pipesql-source.json");
    let exported: Option<Value> = if export_path.try_exists()? {
        Some(serde_json::from_slice(&fs::read(&export_path)?)?)
    } else {
        None
    };
    let paths = if let Some(exported) = &exported {
        exported["files"]
            .as_array()
            .ok_or("invalid exported source inventory")?
            .iter()
            .map(|entry| {
                entry["path"]
                    .as_str()
                    .map(PathBuf::from)
                    .ok_or("invalid exported source path")
            })
            .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        inventory(root)?
    };
    for path in &paths {
        if path.as_os_str().is_empty()
            || path == Path::new(".pipesql-source.json")
            || path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err("invalid source inventory path".into());
        }
    }
    if paths.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("source inventory is not ordered and unique".into());
    }
    if !paths.iter().any(|p| p == Path::new("dev/src/main.rs")) {
        return Err("stage new development sources before freezing".into());
    }
    let mut manifest = Vec::new();
    for path in &paths {
        let source = root.join(path);
        if !fs::symlink_metadata(&source)?.is_file() {
            return Err(format!("source input is not a regular file: {}", source.display()).into());
        }
        let before = hash(&source)?;
        let target = destination.join(path);
        fs::create_dir_all(target.parent().unwrap())?;
        fs::copy(&source, &target)?;
        if hash(&target)? != before || hash(&source)? != before {
            return Err(format!("source changed while copying: {}", path.display()).into());
        }
        let mut permissions = fs::metadata(&target)?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&target, permissions)?;
        manifest.push(json!({"path": path, "sha256": before}));
    }
    if exported.is_none() && inventory(root)? != paths {
        return Err("source inventory changed while copying".into());
    }
    for (path, entry) in paths.iter().zip(&manifest) {
        if hash(&root.join(path))? != entry["sha256"] {
            return Err(
                format!("source changed before freeze completed: {}", path.display()).into(),
            );
        }
    }
    let bytes = serde_json::to_vec(&manifest)?;
    let identity = format!("{:x}", Sha256::digest(&bytes));
    if let Some(exported) = exported {
        if exported["files"] != json!(manifest) || exported["sha256"] != identity {
            return Err("exported source differs from its recorded identity".into());
        }
        if serde_json::from_slice::<Value>(&fs::read(export_path)?)? != exported {
            return Err("exported source inventory changed while copying".into());
        }
    }
    fs::write(
        destination.join(".pipesql-source.json"),
        serde_json::to_vec_pretty(&json!({"sha256": identity, "files": manifest}))?,
    )?;
    Ok(identity)
}
