//! Literal public diagnostics, sink failure and prepared ownership.
use super::*;
use std::fmt::{self, Write};

#[test]
fn logical_plan_distinguishes_relations_identities_and_output_positions() {
    let (_directory, db) = join_fixture();
    let resident = db.reserved_memory_bytes();
    let query = db.prepare(
        "FROM facts |> SELECT k AS group_key, v + 1 AS adjusted |> AGGREGATE SUM(adjusted) AS total, COUNT(*) AS entries GROUP BY group_key |> ORDER BY group_key |> SELECT group_key, total, total AS again",
    ).unwrap();
    let prepared = db.reserved_memory_bytes();
    let report = query.logical_plan().to_string();
    assert_eq!(
        report,
        concat!(
            "logical plan\n",
            "r0 = source occurrence=0 columns=[c1, c2]\n",
            "r1 = select input=r0 columns=[c1, c3]\n",
            "r2 = aggregate input=r1 ordered=false groups=[c1] values=[sum([c3]), count(*)] columns=[c4, c5, c6]\n",
            "r3 = order input=r2 keys=[c4 ASC NULLS FIRST] columns=[c4, c5, c6]\n",
            "r4 = select input=r3 columns=[c4, c5, c5]\n",
            "c3 = numeric [c2, 1, add] input=r0 type=INT64 required\n",
            "result = r4\n",
            "output[0] = c4 name=Some(\"group_key\") type=INT64 nullable\n",
            "output[1] = c5 name=Some(\"total\") type=INT64 nullable\n",
            "output[2] = c5 name=Some(\"again\") type=INT64 nullable\n",
        )
    );
    assert_eq!(db.reserved_memory_bytes(), prepared);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert_eq!(
        collect_unordered(&mut db.execute(&query, &CancellationToken::new()).unwrap()),
        [
            vec![Cell::Null, Cell::Integer(41), Cell::Integer(41)],
            vec![Cell::Integer(1), Cell::Integer(32), Cell::Integer(32)],
            vec![Cell::Integer(2), Cell::Integer(31), Cell::Integer(31)],
        ]
    );
    drop(query);
    assert_eq!(db.reserved_memory_bytes(), resident);
    db.close().unwrap();
}

#[test]
fn logical_plan_shows_both_inputs_without_inventing_a_source_dependency() {
    let (_directory, db) = join_fixture();
    let query = db
        .prepare("FROM facts AS f |> JOIN dimensions AS d ON f.k = d.k |> SELECT f.v, d.label")
        .unwrap();
    assert_eq!(
        query.logical_plan().to_string(),
        concat!(
            "logical plan\n",
            "r0 = source occurrence=0 columns=[c1, c2]\n",
            "r1 = source occurrence=1 columns=[c3, c4]\n",
            "r2 = inner-join left=r0 right=r1 on=c1 = c3 columns=[c1, c2, c3, c4]\n",
            "r3 = select input=r2 columns=[c2, c4]\n",
            "result = r3\n",
            "output[0] = c2 name=Some(\"v\") type=INT64 required\n",
            "output[1] = c4 name=Some(\"label\") type=STRING required\n",
        )
    );
    drop(query);
    let query = db
        .prepare("FROM facts |> SELECT k |> UNION ALL (FROM dimensions |> SELECT k)")
        .unwrap();
    assert_eq!(
        query.logical_plan().to_string(),
        concat!(
            "logical plan\n",
            "r0 = source occurrence=0 columns=[c1, c2]\n",
            "r1 = select input=r0 columns=[c1]\n",
            "r2 = source occurrence=1 columns=[c3, c4]\n",
            "r3 = select input=r2 columns=[c3]\n",
            "r4 = union-all left=r1 right=r3 columns=[c5]\n",
            "result = r4\n",
            "output[0] = c5 name=Some(\"k\") type=INT64 nullable\n",
        )
    );
    drop(query);
    db.close().unwrap();
}

#[test]
fn logical_plan_propagates_sink_failure_and_keeps_the_prepared_snapshot() {
    let (_directory, db) = join_fixture();
    let resident = db.reserved_memory_bytes();
    let query = db.prepare("FROM facts |> SELECT v").unwrap();
    let prepared = db.reserved_memory_bytes();
    let view = query.logical_plan();
    let before = view.to_string();
    // Fail at every byte position, including inside a formatted number or name.
    // No write after the first failure is permitted; a complete sink is the control.
    for capacity in 0..=before.len() {
        let mut sink = LimitedSink {
            remaining: capacity,
            bytes: 0,
            failed: false,
        };
        let result = write!(sink, "{view}");
        assert_eq!(result.is_ok(), capacity == before.len());
        assert_eq!(sink.bytes, capacity);
        assert_eq!(db.reserved_memory_bytes(), prepared);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    let cancel = CancellationToken::new();
    let mut append = db.begin_append("facts", limits(), &cancel).unwrap();
    append
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&[3]),
                    validity: &[1],
                },
                ColumnInput {
                    values: ColumnValues::Int64(&[99]),
                    validity: &[1],
                },
            ],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();
    assert_eq!(view.to_string(), before);
    assert_eq!(
        collect_unordered(&mut db.execute(&query, &cancel).unwrap()),
        [
            vec![Cell::Integer(10)],
            vec![Cell::Integer(20)],
            vec![Cell::Integer(30)],
            vec![Cell::Integer(40)],
        ]
    );
    drop(query);
    let fresh = db.prepare("FROM facts |> SELECT v").unwrap();
    assert_eq!(
        collect_unordered(&mut db.execute(&fresh, &cancel).unwrap()).len(),
        5
    );
    drop(fresh);
    assert_eq!(db.reserved_memory_bytes(), resident);
    assert_eq!(db.reserved_temp_bytes(), 0);
    db.close().unwrap();
}

struct LimitedSink {
    remaining: usize,
    bytes: usize,
    failed: bool,
}

#[test]
fn logical_plan_keeps_scalar_programs_and_nullable_join_outputs_explicit() {
    let (_directory, db) = join_fixture();
    for (sql, expected) in [
        (
            "FROM facts |> SELECT CAST(EXP(LOG10(LN(SQRT(ROUND(CEIL(FLOOR(SIGN(ABS(-v))))))))) AS DOUBLE) AS n",
            "c3 = numeric [c2, negate, abs, sign, floor, ceil, round, sqrt, ln, log10, exp, to_double] input=r0 type=DOUBLE required\n",
        ),
        (
            "FROM facts |> SELECT SAFE_DIVIDE(POWER(v + 1, 2) * MOD(v, 3) - DIV(v, 4), COALESCE(NULLIF(v, 0), 1.5)) / 2 AS n",
            "c3 = numeric [c2, 1, add, 2, power, c2, 3, mod, multiply, c2, 4, div, subtract, c2, 0, nullif, double_bits(0x3ff8000000000000), coalesce, safe_divide, 2, divide] input=r0 type=DOUBLE nullable\n",
        ),
        (
            "FROM facts |> SET v = k",
            "c3 = copy c1 input=r0 type=INT64 nullable\n",
        ),
        (
            "FROM dimensions |> SELECT BYTE_LENGTH(label) AS n, CHAR_LENGTH(label) AS m",
            "c3 = byte_length c2 input=r0 type=INT64 required\nc4 = char_length c2 input=r0 type=INT64 required\n",
        ),
        (
            "FROM facts |> SELECT 'hello' AS label, DATE '1970-01-02' AS day, COUNT(*) OVER () AS n",
            "c3 = string \"hello\" input=r0 type=STRING required\nc4 = date days=1 input=r0 type=DATE required\nc5 = count(*) over () input=r0 type=INT64 required\n",
        ),
        (
            "FROM facts AS f |> LEFT JOIN dimensions AS d ON f.k = d.k |> SELECT d.label",
            "r2 = left-join left=r0 right=r1 on=c1 = c3 columns=[c1, c2, c5, c6]\nr3 = select input=r2 columns=[c6]\nresult = r3\noutput[0] = c6 name=Some(\"label\") type=STRING nullable\n",
        ),
    ] {
        let query = db.prepare(sql).unwrap();
        let prepared = db.reserved_memory_bytes();
        let report = query.logical_plan().to_string();
        assert!(
            report.contains(expected),
            "{sql}\n{report}\nexpected:\n{expected}"
        );
        assert_eq!(db.reserved_memory_bytes(), prepared);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    db.close().unwrap();
}

impl fmt::Write for LimitedSink {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        assert!(!self.failed, "formatter wrote after sink failure");
        let written = self.remaining.min(text.len());
        self.remaining -= written;
        self.bytes += written;
        if written != text.len() {
            self.failed = true;
            Err(fmt::Error)
        } else {
            Ok(())
        }
    }
}

#[test]
fn logical_plan_reports_structural_operators_and_filter_decisions() {
    let (_directory, db) = join_fixture();
    for (sql, expected) in [
        (
            "FROM dimensions |> WHERE label != 'x' |> EXTEND CHAR_LENGTH(label) AS n |> DROP k |> RENAME label AS text |> AS named |> SELECT named.n |> DISTINCT |> LIMIT 2 OFFSET 1",
            &[
                "r1 = filter input=r0 c2 != \"x\" negated=false matched=+1 other=+0 end=+1 columns=[c1, c2]\n",
                "r2 = extend input=r1 columns=[c1, c2, c3]\n",
                "r3 = drop input=r2 columns=[c2, c3]\n",
                "r4 = rename input=r3 columns=[c2, c3]\n",
                "r5 = alias input=r4 columns=[c2, c3]\n",
                "r7 = distinct input=r6 columns=[c4]\n",
                "r8 = limit input=r7 count=2 offset=1 columns=[c4]\n",
            ][..],
        ),
        (
            "FROM (FROM facts |> SELECT k, v) AS f |> ORDER BY k DESC NULLS LAST, v ASC |> SELECT v + 1",
            &[
                "r2 = derived input=r1 columns=[c1, c2]\n",
                "r3 = order input=r2 keys=[c1 DESC NULLS LAST, c2 ASC NULLS FIRST] columns=[c1, c2]\n",
                "output[0] = c3 name=None type=INT64 required\n",
            ][..],
        ),
    ] {
        let query = db.prepare(sql).unwrap();
        let report = query.logical_plan().to_string();
        for line in expected {
            assert!(report.contains(line), "{sql}\n{report}\nexpected:\n{line}");
        }
    }
    for (syntax, kind) in [
        ("EXCEPT DISTINCT", "except-distinct"),
        ("EXCEPT ALL", "except-all"),
        ("INTERSECT DISTINCT", "intersect-distinct"),
        ("INTERSECT ALL", "intersect-all"),
    ] {
        let query = db
            .prepare(&format!(
                "FROM facts |> SELECT k |> {syntax} (FROM dimensions |> SELECT k)"
            ))
            .unwrap();
        let report = query.logical_plan().to_string();
        assert!(
            report.contains(&format!("r4 = {kind} left=r1 right=r3 columns=[c5]\n")),
            "{report}"
        );
    }
    db.close().unwrap();
}

#[test]
fn logical_plan_also_formats_the_legacy_source_without_execution() {
    let directory = Directory::new();
    let db = Database::create(&directory.database(), config()).unwrap();
    let query = db
        .prepare("FROM lineitem |> SELECT l_quantity AS quantity")
        .unwrap();
    assert_eq!(
        query.logical_plan().to_string(),
        concat!(
            "logical plan\n",
            "r0 = source occurrence=0 columns=[c1, c2, c3, c4, c5, c6, c7]\n",
            "r1 = select input=r0 columns=[c1]\n",
            "result = r1\n",
            "output[0] = c1 name=Some(\"quantity\") type=DOUBLE required\n",
        )
    );
    drop(query);
    db.close().unwrap();
}
