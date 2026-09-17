//! Check Parquet CLI transfers against retained external-reader samples.
//!
//! Import and export use explicit limits and compare complete output with bytes
//! separately checked by PyArrow. This campaign does not invoke PyArrow itself.
//! Allocation-prefix sweeps verify that the selected refusal occurred, then stock
//! commands inspect rows and transaction outcomes. Limit and stream failures must
//! preserve database files where required. A successful census or process exit alone
//! cannot establish that the exported Parquet file is complete and correct.

use super::database::{add, command, line, stock, valid_token};
use super::{Campaign, Result, census, payload, status};
use crate::process::{Completion, Output};
use crate::workspace::{copy_tree, tree_contents};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

fn configured(verb: &str, database: &Path) -> Vec<OsString> {
    let mut args = command(verb, database);
    for flag in ["--memory-limit-bytes", "--temp-limit-bytes"] {
        let index = args.iter().position(|arg| arg == flag).unwrap();
        args[index + 1] = "8000000".into();
    }
    args
}

fn import_args(database: &Path, source: &Path) -> Vec<OsString> {
    let mut args = configured("import", database);
    add(&mut args, "--input", source);
    for (flag, value) in [
        ("--format", "parquet"),
        ("--table", "facts"),
        ("--input-limit-bytes", "100000"),
        ("--row-limit", "8"),
        ("--metadata-limit-bytes", "16384"),
        ("--row-group-limit", "8"),
        ("--row-group-rows", "3"),
        ("--row-group-limit-bytes", "70000"),
        ("--page-limit-bytes", "70000"),
        ("--batch-limit", "8"),
        ("--encoded-limit-bytes", "200000"),
    ] {
        add(&mut args, flag, value);
    }
    args
}

fn export_args(database: &Path, query: &Path) -> Vec<OsString> {
    let mut args = configured("export", database);
    add(&mut args, "--query-file", query);
    for (flag, value) in [
        ("--format", "parquet"),
        ("--row-limit", "8"),
        ("--output-limit-bytes", "100000"),
        ("--metadata-limit-bytes", "16384"),
        ("--row-group-limit", "8"),
        ("--row-group-rows", "3"),
        ("--row-group-text-bytes", "65536"),
    ] {
        add(&mut args, flag, value);
    }
    args
}

struct Inputs {
    seed: PathBuf,
    source: PathBuf,
    query: PathBuf,
    expected: Vec<u8>,
    empty: Vec<u8>,
}

impl Inputs {
    fn create(campaign: &mut Campaign) -> Result<Self> {
        let data = campaign.run.root.join("test/data/parquet");
        let seed = campaign.run.directory.join("parquet-seed");
        let schema = campaign.run.directory.join("parquet.schema");
        let query = campaign.run.directory.join("parquet.sql");
        fs::write(
            &schema,
            "table facts\nid int64 required\namount int64 nullable\nnumber double nullable\nday date nullable\nnote string nullable\n",
        )?;
        fs::write(&query, "FROM facts |> ORDER BY id")?;
        stock(campaign, &configured("create-declared", &seed))?;
        let mut declare = configured("declare", &seed);
        add(&mut declare, "--schema-file", &schema);
        stock(campaign, &declare)?;
        let empty = stock(campaign, &export_args(&seed, &query))?.stdout.bytes;
        if !empty.starts_with(b"PAR1") || !empty.ends_with(b"PAR1") {
            return Err("empty Parquet export lacks file markers".into());
        }
        Ok(Self {
            seed,
            source: data.join("plain-v2.parquet"),
            query,
            expected: fs::read(data.join("pipesql-v1.parquet"))?,
            empty,
        })
    }

    fn check_rows(&self, campaign: &mut Campaign, database: &Path, durable: bool) -> Result<()> {
        let output = stock(campaign, &export_args(database, &self.query))?;
        if output.stdout.bytes != if durable { &self.expected } else { &self.empty }.as_slice() {
            return Err("Parquet transfer changed complete expected bytes".into());
        }
        Ok(())
    }

    fn settle(&self, campaign: &mut Campaign, database: &Path, result: &Output) -> Result<bool> {
        stock(campaign, &configured("open", database))?;
        let text = std::str::from_utf8(&result.stdout.bytes)?;
        let receipt = if text.lines().any(|line| line.starts_with("transaction=")) {
            Some(line(text, "transaction=")?)
        } else {
            None
        };
        let mut resolve = configured("resolve", database);
        let durable = if let Some(token) = receipt {
            if !valid_token(token) {
                return Err("invalid Parquet receipt".into());
            }
            add(&mut resolve, "--transaction", token);
            let output = stock(campaign, &resolve)?;
            let text = std::str::from_utf8(&output.stdout.bytes)?;
            if line(text, "transaction=")? != token {
                return Err("wrong Parquet receipt resolved".into());
            }
            match line(text, "resolution=")? {
                "durable" => true,
                "aborted" => false,
                _ => return Err("Parquet receipt did not settle".into()),
            }
        } else {
            false
        };
        if matches!(result.completion, Completion::Exited(status) if status.success())
            && (!durable || !text.contains("status=imported\ngeneration=2\n"))
        {
            return Err("successful Parquet import lacks a durable receipt".into());
        }
        self.check_rows(campaign, database, durable)?;
        if !durable {
            stock(campaign, &import_args(database, &self.source))?;
            self.check_rows(campaign, database, true)?;
            if receipt.is_some() {
                let again = stock(campaign, &resolve)?;
                if line(std::str::from_utf8(&again.stdout.bytes)?, "resolution=")? != "aborted" {
                    return Err("Parquet retry changed old receipt".into());
                }
            }
        }
        Ok(durable)
    }

    fn import_cell(&self, campaign: &mut Campaign, mode: &str) -> Result<Output> {
        let database = campaign
            .run
            .directory
            .join(format!("parquet-import-{mode}"));
        copy_tree(&self.seed, &database)?;
        let result = campaign.execute(&import_args(&database, &self.source), Some(mode), None)?;
        census(&result)?;
        self.settle(campaign, &database, &result)?;
        fs::remove_dir_all(database)?;
        Ok(result)
    }
}

pub(super) fn run(campaign: &mut Campaign) -> Result<()> {
    let inputs = Inputs::create(campaign)?;
    let observer = campaign
        .run
        .build("pipesql-native", "--lib", "pipesql_native")?;
    for cut in [0, 3, 4] {
        let database = campaign.run.directory.join(format!("parquet-cut-{cut}"));
        copy_tree(&inputs.seed, &database)?;
        let mut command = campaign.command(
            &import_args(&database, &inputs.source),
            Some(&format!("publication-{cut}")),
            None,
        );
        command
            .env_remove("LD_PRELOAD")
            .env_remove("DYLD_INSERT_LIBRARIES");
        command.env(
            if cfg!(target_os = "macos") {
                "DYLD_INSERT_LIBRARIES"
            } else {
                "LD_PRELOAD"
            },
            &observer,
        );
        let result = campaign
            .run
            .command(&mut command, None, Duration::from_secs(20))?;
        status(&result, if cut == 0 { 0 } else { 1 })?;
        if line(
            std::str::from_utf8(&result.stderr.bytes)?,
            "observed_renames=",
        )? != if cut == 3 { "3" } else { "4" }
            || !std::str::from_utf8(&result.stdout.bytes)?
                .lines()
                .any(|line| line.starts_with("transaction="))
        {
            return Err("Parquet publication missed its cut or receipt".into());
        }
        if inputs.settle(campaign, &database, &result)? != (cut != 3) {
            return Err("Parquet publication outcome disagrees with cut".into());
        }
        fs::remove_dir_all(database)?;
    }
    let control = inputs.import_cell(campaign, "entry-control")?;
    status(&control, 0)?;
    let (imports, refused) = census(&control)?;
    if imports == 0 || refused != 0 {
        return Err("invalid Parquet import census".into());
    }
    for prefix in std::iter::once(imports).chain(0..imports) {
        let result = inputs.import_cell(campaign, &format!("entry-after-{prefix}"))?;
        check_prefix(&result, prefix, imports)?;
    }
    let database = campaign.run.directory.join("parquet-export");
    copy_tree(&inputs.seed, &database)?;
    stock(campaign, &import_args(&database, &inputs.source))?;
    let args = export_args(&database, &inputs.query);
    let before = tree_contents(&database)?;
    let control = campaign.execute(&args, Some("entry-control"), None)?;
    status(&control, 0)?;
    let (exports, refused) = census(&control)?;
    if exports == 0 || refused != 0 || payload(&control)? != inputs.expected {
        return Err("invalid Parquet export control".into());
    }
    inputs.check_rows(campaign, &database, true)?;
    for prefix in std::iter::once(exports).chain(0..exports) {
        let result = campaign.execute(&args, Some(&format!("entry-after-{prefix}")), None)?;
        check_prefix(&result, prefix, exports)?;
        if (payload(&result)? == inputs.expected) != (prefix == exports) {
            return Err("Parquet export completion disagrees with allocation prefix".into());
        }
        if tree_contents(&database)? != before {
            return Err("Parquet export changed database".into());
        }
    }
    for (flag, value, succeeds) in [
        ("--output-limit-bytes", inputs.expected.len(), true),
        ("--output-limit-bytes", inputs.expected.len() - 1, false),
        ("--row-limit", 7, false),
        ("--row-group-limit", 1, false),
        ("--metadata-limit-bytes", 1, false),
        ("--row-group-text-bytes", 65535, false),
    ] {
        let mut bounded = args.clone();
        let index = bounded.iter().position(|arg| arg == flag).unwrap();
        bounded[index + 1] = value.to_string().into();
        let result = campaign.execute(&bounded, None, None)?;
        status(&result, if succeeds { 0 } else { 1 })?;
        if (result.stdout.bytes == inputs.expected) != succeeds {
            return Err("Parquet bound accepted incorrect output".into());
        }
    }
    let produced = campaign.run.directory.join("round-trip.parquet");
    // Use freshly produced output, not merely the retained expected file.
    fs::write(&produced, payload(&control)?)?;
    let restored = campaign.run.directory.join("parquet-restored");
    copy_tree(&inputs.seed, &restored)?;
    stock(campaign, &import_args(&restored, &produced))?;
    inputs.check_rows(campaign, &restored, true)?;
    let seed_before = tree_contents(&inputs.seed)?;
    for name in [
        "unsupported-snappy",
        "unsupported-dictionary",
        "unsupported-nested",
    ] {
        let path = inputs
            .source
            .parent()
            .unwrap()
            .join(format!("{name}.parquet"));
        let result = campaign.execute(&import_args(&inputs.seed, &path), None, None)?;
        status(&result, 1)?;
        if std::str::from_utf8(&result.stdout.bytes)?.contains("transaction=")
            || tree_contents(&inputs.seed)? != seed_before
        {
            return Err("unsupported Parquet input issued a transaction or changed files".into());
        }
    }
    let closed = campaign.execute(
        &import_args(&inputs.seed, Path::new("-")),
        Some("entry-closed-stdin"),
        None,
    )?;
    status(&closed, 1)?;
    if !std::str::from_utf8(&closed.stderr.bytes)?.contains("seekable regular file")
        || tree_contents(&inputs.seed)? != seed_before
    {
        return Err("Parquet stdin refusal changed files or missed diagnostic".into());
    }
    for (mode, code) in [("entry-closed-stdout", 1), ("entry-closed-stderr", 0)] {
        status(&campaign.execute(&args, Some(mode), None)?, code)?;
    }
    super::broken_output(campaign, &args)?;
    if tree_contents(&database)? != before {
        return Err("Parquet limits or stream failures changed files".into());
    }
    println!(
        "Parquet CLI: {} import and {} export prefixes, publication cuts, independent bytes, bounds, round-trip and stream failures passed",
        imports + 1,
        exports + 1
    );
    Ok(())
}

fn check_prefix(result: &Output, prefix: usize, total: usize) -> Result<()> {
    let (calls, refused) = census(result)?;
    if prefix == total {
        status(result, 0)?;
        if calls != total || refused != 0 {
            return Err("Parquet full prefix changed census".into());
        }
    } else {
        if status(result, 1).is_err() {
            status(result, 2)?;
        }
        if calls <= prefix || refused == 0 {
            return Err("Parquet allocation refusal missed prefix".into());
        }
    }
    Ok(())
}
