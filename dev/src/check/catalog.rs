//! Damage engine-created catalogs and require independent and public rejection.
//!
//! Expected typed rows are literals. Structural mutations repair referring
//! checksums, so rejection must identify the intended field rather than stop at
//! the checksum guard. Valid damaged-root recovery must preserve the same rows.
//!
//! The campaign also checks inspection limits, lease exclusion and retained format
//! samples. It builds a separate inspector artifact for child invocations and
//! requires both the tool and public-library driver to reach their expected outcome.
//! Inspection itself must not repair files; recovery authority belongs to open.

use crate::{
    Result,
    oracle::catalog,
    workspace::{self, Run, copy_tree},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[path = "catalog_mutation.rs"]
mod mutation;
#[path = "catalog_report.rs"]
mod report;
use mutation::{Mutation, get, put};

fn expected(graph: &Value) -> Result<()> {
    let rows = json!([
        [i64::MIN, {"double_bits":"7ff0000000000001"}, "雪\u{0}", -719162],
        [-1, {"double_bits":"8000000000000000"}, "", -719162],
        [0, {"double_bits":"7ff0000000000000"}, "é", -719162],
        [1, {"double_bits":"fff0000000000000"}, "🙂", -719162],
        [2, {"double_bits":"3ff0000000000000"}, "same", -719162],
        [3, {"double_bits":"4000000000000000"}, "same", -719162],
        [4, {"double_bits":"4008000000000000"}, "a", -719162],
        [5, null, null, null],
        [i64::MAX, {"double_bits":"fff8000000001234"}, "end", 2932896]
    ]);
    let rows: Vec<_> = rows
        .as_array()
        .unwrap()
        .iter()
        .cycle()
        .take(18)
        .cloned()
        .collect();
    let tables = json!([
        {"id":1, "name":"facts", "rows":rows, "columns":[
            {"id":1, "name":"k", "type":1, "nullable":false},
            {"id":2, "name":"d", "type":2, "nullable":true},
            {"id":3, "name":"s", "type":3, "nullable":true},
            {"id":4, "name":"day", "type":4, "nullable":true}
        ]},
        {"id":2, "name":"empty", "rows":[], "columns":[{"id":1, "name":"x", "type":3, "nullable":true}]}
    ]);
    if graph["issued"] != 6 || graph["generation"] != 4 || graph["successes"] != json!([1, 2, 4, 5])
    {
        return Err("independent receipt history differs".into());
    }
    if graph["tables"] != tables {
        return Err("independent typed rows or schema differ".into());
    }
    Ok(())
}

struct Campaign {
    run: Run,
    driver: PathBuf,
    inspector: PathBuf,
    seed: PathBuf,
    records: fs::File,
    cases: usize,
}

impl Campaign {
    fn driver(&mut self, path: &Path, mode: &str) -> Result<()> {
        let mut command = Command::new(&self.driver);
        command.arg(path).arg(mode);
        if mode == "hold" {
            command.env("PIPESQL_CATALOG_INSPECTOR", &self.inspector);
        }
        let output = self
            .run
            .command(&mut command, None, Duration::from_secs(30))?;
        if let Err(error) = output.require_success() {
            return Err(format!(
                "driver {mode}: {error}: {}",
                String::from_utf8_lossy(&output.stderr.bytes)
            )
            .into());
        }
        Ok(())
    }

    fn case(
        &mut self,
        label: &str,
        error: Option<&str>,
        public: &str,
        edit: impl FnOnce(&mut Mutation) -> Result<()>,
    ) -> Result<()> {
        writeln!(
            self.records,
            "{}",
            json!({"case":label, "expected_error":error, "public":public, "complete":false})
        )?;
        self.records.flush()?;
        let path = self.run.directory.join(label);
        copy_tree(&self.seed, &path)?;
        let result = (|| -> Result<()> {
            edit(&mut Mutation::new(&path)?)?;
            match (catalog::inspect(&path), error) {
                (Ok(graph), None) => expected(&graph)?,
                (Err(actual), Some(reason)) if actual.to_string().contains(reason) => (),
                (Ok(_), Some(reason)) => {
                    return Err(format!("accepted corruption: expected {reason}").into());
                }
                (Err(actual), _) => return Err(actual),
            }
            if !public.is_empty() {
                self.driver(&path, public)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            return Err(format!("catalog/{label}: {error}").into());
        }
        writeln!(self.records, "{}", json!({"case":label, "complete":true}))?;
        self.cases += 1;
        Ok(())
    }
}

type FieldCase = (
    &'static str,
    &'static str,
    fn(&mut [u8]),
    &'static str,
    &'static str,
);

fn fields(campaign: &mut Campaign) -> Result<()> {
    let cases: &[FieldCase] = &[
        (
            "wrong-schema",
            "catalog",
            |b| b.copy_within(240..264, 112),
            "schema table identity",
            "reject",
        ),
        (
            "future-schema",
            "catalog",
            |b| put(b, 112, 7, 8),
            "future schema reference",
            "reject",
        ),
        (
            "duplicate-table",
            "catalog",
            |b| put(b, 192, 1, 8),
            "table identity/name length",
            "reject",
        ),
        (
            "row-count",
            "catalog",
            |b| put(b, 160, 17, 8),
            "table index header",
            "reject",
        ),
        (
            "schema-count",
            "schema",
            |b| put(b, 12, 65, 4),
            "schema count/extent",
            "reject",
        ),
        (
            "catalog-count",
            "catalog",
            |b| put(b, 12, 65, 4),
            "catalog count/extent",
            "reject",
        ),
        (
            "schema-padding",
            "schema",
            |b| put(b, 40, 1, 4),
            "nonzero reserved bytes",
            "reject",
        ),
        (
            "column-identity",
            "schema",
            |b| put(b, 112, 1, 4),
            "duplicate column identity/name",
            "reject",
        ),
        (
            "catalog-identity",
            "catalog",
            |b| put(b, 32, 4, 8),
            "catalog identity",
            "reject",
        ),
        (
            "repeated-unit",
            "index",
            |b| b.copy_within(64..88, 112),
            "unit order/creator",
            "reject",
        ),
        (
            "row-gap",
            "index",
            |b| put(b, 136, 9, 8),
            "unit row coverage",
            "reject",
        ),
        (
            "payload-overlap",
            "unit",
            |b| put(b, 104, 192, 8),
            "column type/coverage",
            "reject",
        ),
        (
            "unit-column-alias",
            "unit",
            |b| put(b, 96, 1, 4),
            "duplicate unit column identity",
            "reject",
        ),
        (
            "unit-type",
            "unit",
            |b| put(b, 68, 4, 1),
            "column type/coverage",
            "reject",
        ),
        (
            "history-order",
            "history",
            |b| put(b, 72, 1, 8),
            "history order",
            "reject",
        ),
        (
            "history-last",
            "history",
            |b| put(b, 56, 4, 8),
            "history header",
            "reject",
        ),
        (
            "null-number",
            "payload",
            |b| put(b, get(b, 104, 8) as usize + 1 + 7 * 8, 1, 8),
            "nonzero reserved bytes",
            "query-reject",
        ),
        (
            "date-domain",
            "payload",
            |b| put(b, get(b, 168, 8) as usize + 1, 2932897, 4),
            "DATE domain",
            "query-reject",
        ),
        (
            "invalid-utf8",
            "payload",
            |b| put(b, get(b, 136, 8) as usize + 1 + 9 * 4, 255, 1),
            "invalid utf-8",
            "query-reject",
        ),
        (
            "string-offset",
            "payload",
            |b| put(b, get(b, 136, 8) as usize + 1, 1, 4),
            "initial string offset",
            "query-reject",
        ),
        (
            "validity-padding",
            "last-payload",
            |b| put(b, 192, 129, 1),
            "validity padding",
            "query-reject",
        ),
    ];
    for &(label, kind, edit, reason, public) in cases {
        campaign.case(label, Some(reason), public, |m| m.change(kind, edit))?;
    }
    campaign.case("cycle", Some("object alias or cycle"), "reject", |m| {
        let root = fs::read(m.path.join("ROOT.A"))?;
        m.change("catalog", |b| b[112..136].copy_from_slice(&root[128..152]))
    })?;
    Ok(())
}

fn damage(path: &Path, at: usize) -> Result<()> {
    let mut bytes = fs::read(path)?;
    bytes[at] ^= 1;
    fs::write(path, bytes)?;
    Ok(())
}

fn namespace(c: &mut Campaign) -> Result<()> {
    c.case(
        "newer-corrupt-older-valid",
        Some("object checksum"),
        "reject",
        |m| {
            m.roots(|b| put(b, 112, 5, 8))?;
            let old_catalog = fs::read(m.path.join("units/0000000000000004-00000004.obj"))?;
            let old_successes = fs::read(m.path.join("units/0000000000000004-00000005.obj"))?;
            mutation::root(&m.path.join("ROOT.B"), |b| {
                put(b, 40, 3, 8);
                put(b, 72, 4, 8);
                for (at, ordinal, bytes) in [(128, 4, &old_catalog), (152, 5, &old_successes)] {
                    put(b, at, 4, 8);
                    put(b, at + 8, ordinal, 4);
                    put(b, at + 12, bytes.len() as u64, 4);
                    put(b, at + 16, u64::from(catalog::crc32c(bytes)), 4);
                }
            })?;
            damage(&m.path.join("units/0000000000000005-00000004.obj"), 64)
        },
    )?;
    c.case(
        "missing-schema",
        Some("missing referenced object"),
        "reject-missing",
        |m| Ok(fs::remove_file(m.path.join("units").join(&m.schema))?),
    )?;
    c.case("unknown-name", Some("namespace names"), "reject", |m| {
        Ok(fs::write(m.path.join("unexpected"), [])?)
    })?;
    c.case(
        "foreign-root",
        Some("foreign snapshot identity"),
        "reject",
        |m| m.roots(|b| b[16] ^= 1),
    )?;
    c.case(
        "unknown-version",
        Some("unsupported authoritative version"),
        "reject-version",
        |m| m.roots(|b| put(b, 8, 8, 4)),
    )?;
    c.case(
        "overlong-reference",
        Some("root reference extent"),
        "reject",
        |m| m.roots(|b| put(b, 140, 8257, 4)),
    )?;
    c.case(
        "damaged-payload",
        Some("payload checksum"),
        "query-reject",
        |m| damage(&m.path.join("units/0000000000000004-00000001.obj"), 192),
    )?;
    c.case("damaged-root", None, "verify", |m| {
        damage(&m.path.join("ROOT.A"), 108)?;
        if catalog::inspect(&m.path)?["roots_settled"] != false {
            return Err("damaged root was reported as settled".into());
        }
        Ok(())
    })?;
    c.case("missing-root", None, "verify", |m| {
        Ok(fs::remove_file(m.path.join("ROOT.B"))?)
    })?;
    c.case(
        "both-roots-damaged",
        Some("insufficient root authority"),
        "reject",
        |m| {
            damage(&m.path.join("ROOT.A"), 108)?;
            damage(&m.path.join("ROOT.B"), 108)
        },
    )?;
    c.case(
        "nonadjacent-roots",
        Some("nonadjacent roots"),
        "reject",
        |m| mutation::root(&m.path.join("ROOT.A"), |b| put(b, 112, 8, 8)),
    )?;
    c.case("ahead-fence", None, "verify", |m| {
        mutation::root(&m.path.join("WAL"), |b| put(b, 112, 7, 8))
    })?;
    c.case(
        "ahead-fence-missing-root",
        Some("insufficient root authority"),
        "reject",
        |m| {
            fs::remove_file(m.path.join("ROOT.B"))?;
            mutation::root(&m.path.join("WAL"), |b| put(b, 112, 7, 8))
        },
    )?;
    let extra = "units/0000000000000003-00000001.obj";
    c.case("unreferenced-object", None, "verify", |m| {
        fs::write(m.path.join(extra), [])?;
        let graph = catalog::inspect(&m.path)?;
        if !graph["unreferenced"]
            .as_array()
            .is_some_and(|names| names.contains(&json!("0000000000000003-00000001.obj")))
        {
            return Err("inspector omitted the unreferenced object".into());
        }
        Ok(())
    })?;
    c.case(
        "symlink-object",
        Some("object ownership/extent"),
        "reject",
        |m| Ok(std::os::unix::fs::symlink(&m.schema, m.path.join(extra))?),
    )?;
    c.case(
        "aliased-object",
        Some("object ownership/extent"),
        "reject",
        |m| {
            Ok(fs::hard_link(
                m.path.join("units").join(&m.schema),
                m.path.join(extra),
            )?)
        },
    )?;
    c.case(
        "oversize-object",
        Some("object ownership/extent"),
        "reject",
        |m| Ok(fs::File::create(m.path.join(extra))?.set_len(33_556_545)?),
    )?;
    Ok(())
}

fn limits_and_controls(c: &mut Campaign, baseline: &Value) -> Result<()> {
    if locked(&c.seed).is_ok() {
        return Err("lease check treated an idle database as busy".into());
    }
    c.driver(&c.seed.clone(), "hold")?;
    let exact = catalog::Limits {
        objects: fs::read_dir(c.seed.join("units"))?.count(),
        read_bytes: baseline["read_bytes"]
            .as_u64()
            .ok_or("missing read count")?,
        values: baseline["decoded_values"]
            .as_u64()
            .ok_or("missing decoded count")? as usize,
    };
    expected(&catalog::inspect_with_limits(&c.seed, exact)?)?;
    for (limit, reason) in [
        (
            catalog::Limits {
                objects: exact.objects - 1,
                ..exact
            },
            "directory entry budget",
        ),
        (
            catalog::Limits {
                read_bytes: exact.read_bytes - 1,
                ..exact
            },
            "read byte budget",
        ),
        (
            catalog::Limits {
                values: exact.values - 1,
                ..exact
            },
            "decoded value budget",
        ),
    ] {
        let error =
            catalog::inspect_with_limits(&c.seed, limit).expect_err("inspector ignored limit");
        if !error.to_string().contains(reason) {
            return Err(format!("wrong limit failure: expected {reason}: {error}").into());
        }
    }
    let mut wrong = baseline.clone();
    wrong["tables"][0]["rows"][0][0] = json!(0);
    if expected(&wrong).is_ok() {
        return Err("wrong rows passed comparison".into());
    }
    let mut wrong = baseline.clone();
    wrong["successes"] = json!([1, 2, 3, 5]);
    if expected(&wrong).is_ok() {
        return Err("wrong receipt history passed comparison".into());
    }
    for (mode, reason) in [
        ("reject", "public open accepted malformed graph"),
        ("query-reject", "public query accepted corrupt payload"),
    ] {
        let mut command = Command::new(&c.driver);
        command.arg(&c.seed).arg(mode);
        let output = c.run.command(&mut command, None, Duration::from_secs(30))?;
        if !matches!(output.completion, crate::process::Completion::Exited(status) if !status.success())
            || !std::str::from_utf8(&output.stderr.bytes)?.contains(reason)
        {
            return Err(format!("{mode} control failed to reject a healthy database").into());
        }
    }
    Ok(())
}

/// The driver invokes this process while its live Database owns the lease.
pub fn locked(path: &Path) -> Result<()> {
    match catalog::inspect(path) {
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::WouldBlock) =>
        {
            Ok(())
        }
        Err(error) => Err(format!("expected busy lease, got {error}").into()),
        Ok(_) => Err("inspector ignored live engine lease".into()),
    }
}

fn retained_bytes(c: &mut Campaign) -> Result<()> {
    let genesis = c.run.directory.join("genesis");
    c.driver(&genesis, "genesis")?;
    let empty = catalog::inspect(&genesis)?;
    if empty["issued"] != 0
        || empty["generation"] != 0
        || ["tables", "successes", "reachable"]
            .iter()
            .any(|name| empty[name] != json!([]))
    {
        return Err("wrong empty catalog".into());
    }
    let path = c.run.directory.join("retained-bytes");
    crate::oracle::catalog_vectors::database(&path, &c.run.root.join("test/data"))?;
    let graph = catalog::inspect(&path)?;
    // These bytes store columns in the opposite order to their schema. Values
    // must follow column identity, including signed zero and the NaN payload.
    if graph["successes"] != json!([3, 5])
        || graph["issued"] != 6
        || graph["tables"][0]["columns"][0]["id"] != 29
        || graph["tables"][0]["columns"][1]["id"] != 3
        || graph["tables"][0]["rows"]
            != json!([
                [null, {"double_bits":"8000000000000000"}],
                ["", {"double_bits":"7ff0000000000001"}],
                ["雪", {"double_bits":"7ff0000000000000"}],
                ["é\u{0}🙂", {"double_bits":"ffefffffffffffff"}]
            ])
    {
        return Err("retained bytes decoded with wrong column identities or values".into());
    }
    Ok(())
}

pub fn run() -> Result<()> {
    let mut run = Run::new(workspace::root()?, "catalog")?;
    let driver = run.build("pipesql-driver", "--bin", "catalog")?;
    // Select and hash the child through Cargo. current_exe can describe an
    // unlinked image on Linux even while this supervisor continues running.
    let inspector = run.build("pipesql-dev", "--bin", "pipesql-dev")?;
    let seed = run.directory.join("seed");
    let records = fs::File::create(run.directory.join("cases.jsonl"))?;
    let mut c = Campaign {
        run,
        driver,
        inspector,
        seed: seed.clone(),
        records,
        cases: 0,
    };
    c.driver(&seed, "setup")?;
    let baseline = catalog::inspect(&seed)?;
    expected(&baseline)?;
    let output = c.run.command(
        Command::new(&c.inspector).arg("inspect").arg(&seed),
        None,
        Duration::from_secs(30),
    )?;
    output.require_success()?;
    expected(&serde_json::from_slice(&output.stdout.bytes)?)?;
    let refused = c.run.command(
        Command::new(&c.inspector)
            .arg("inspect")
            .arg(&seed)
            .args(["--max-values", "1"]),
        None,
        Duration::from_secs(30),
    )?;
    if !matches!(refused.completion, crate::process::Completion::Exited(status) if !status.success())
        || !refused.stdout.bytes.is_empty()
        || !String::from_utf8_lossy(&refused.stderr.bytes).contains("value budget")
    {
        return Err("inspection command did not reject its value budget before output".into());
    }
    limits_and_controls(&mut c, &baseline)?;
    retained_bytes(&mut c)?;
    fs::write(
        c.run.directory.join("graph.json"),
        serde_json::to_vec_pretty(&baseline)?,
    )?;
    fields(&mut c)?;
    namespace(&mut c)?;
    report::run(&mut c)?;
    c.run.finish()?;
    println!("catalog: {} corruption and recovery cases passed", c.cases);
    Ok(())
}
