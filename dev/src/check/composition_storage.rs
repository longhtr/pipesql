//! Check how stored values and set composition reach the query result.
//!
//! Controlled legacy files contain invalid key bytes that must fail when read,
//! while valid DOUBLE cases preserve the intended bits. The separate unions
//! selection exercises declared-table column positions, aliases and duplicate
//! handling with complete expected answers. These cases distinguish storage or
//! schema errors from valid transformations; a printed prefix is never accepted
//! as a completed result.

use super::*;

pub(super) fn run(queries: &mut Queries) -> Result<()> {
    let retained = queries
        .run
        .root
        .join("test/data/current-single-table-format");
    for (column, key) in ["l_returnflag", "l_linestatus"].into_iter().enumerate() {
        for bad in [0, 31, 124, 127, 255] {
            let name = format!("invalid-key-{column}-{bad}");
            let mut row = Row {
                numbers: [0, 1.0_f64.to_bits(), 0, 0],
                keys: *b"!!",
                day: day_offset(1, 1, 1),
            };
            row.keys[column] = bad;
            // Recompute checksums so value validation, rather than damaged bytes,
            // must reject the key.
            snapshot::write(&queries.run.directory.join(&name), &retained, &[row])?;
            for shape in [
                format!("SELECT {key}"),
                format!("AGGREGATE COUNT(*) AS n GROUP BY {key}"),
                format!("WHERE {key} IS NULL"),
                format!("WHERE {key} IS NOT NULL"),
            ] {
                queries.reject(
                    &format!("FROM lineitem |> {shape}"),
                    &name,
                    Failure::StoredKey,
                )?;
            }
            queries.check(
                "undemanded-invalid-key",
                "FROM lineitem |> AGGREGATE COUNT(*) AS n",
                vec![vec![integer(1)]],
                &name,
                true,
            )?;
        }
    }
    queries.seed("price-corrupt", &[(0.0, b'A')])?;
    let unit = queries
        .run
        .directory
        .join("price-corrupt/units/0000000000000001.unit");
    let mut bytes = fs::read(&unit)?;
    // The one-row quantity precedes the price. Leave its checksum unchanged.
    bytes[28672 + 8] ^= 1;
    fs::write(&unit, bytes)?;
    for sql in [
        "FROM lineitem |> AGGREGATE SUM(l_extendedprice) AS total",
        "FROM lineitem |> WHERE l_extendedprice IS NULL |> AGGREGATE COUNT(*) AS n",
        "FROM lineitem |> WHERE l_extendedprice IS NOT NULL |> AGGREGATE COUNT(*) AS n",
    ] {
        queries.reject(sql, "price-corrupt", Failure::PayloadChecksum)?;
    }
    queries.check(
        "undemanded-price-checksum",
        "FROM lineitem |> WHERE l_quantity < 0 |> AGGREGATE SUM(l_extendedprice) AS total",
        vec![vec!["null".into()]],
        "price-corrupt",
        true,
    )?;
    for (name, bits) in [("subnormal", 1_u64), ("negative-zero", 0x8000000000000000)] {
        queries.seed(name, &[(f64::from_bits(bits), b'A')])?;
        for sql in [
            "FROM lineitem |> SELECT l_quantity",
            "FROM lineitem |> AGGREGATE SUM(l_quantity) AS total",
        ] {
            queries.check(name, sql, vec![vec![format!("{bits:016x}")]], name, true)?;
        }
    }
    Ok(())
}

pub(super) fn unions(queries: &mut Queries) -> Result<()> {
    let path = queries.run.directory.join("declared");
    crate::oracle::catalog_vectors::database(&path, &queries.run.root.join("test/data"))?;
    // Retained rows distinguish NULL, empty and Unicode text, signed zero,
    // NaN and infinity. The right-hand names do not determine column pairing.
    for (mode, count) in [("ALL", 8), ("DISTINCT", 4)] {
        queries.check(mode, &format!("FROM facts |> UNION {mode} (FROM facts |> SELECT note AS text, amount AS value) |> AGGREGATE COUNT(*) AS n"), vec![vec![integer(count)]], "declared", true)?;
    }
    for (label, sql, expected) in [
        (
            "union-distinct-complete-row",
            "FROM facts |> SELECT note, 1 AS n |> UNION DISTINCT (FROM facts |> SELECT note, 1 AS n) |> SELECT n",
            vec![vec![integer(1)]; 4],
        ),
        (
            "union-distinct-null-class",
            "FROM facts |> WHERE note IS NULL |> UNION DISTINCT (FROM facts |> WHERE note IS NULL) |> SELECT amount",
            vec![vec!["8000000000000000".into()]],
        ),
        (
            "union-distinct-nested-mode",
            "FROM facts |> SELECT 1 AS n |> UNION DISTINCT (FROM facts |> SELECT 1 AS n |> UNION ALL (FROM facts |> SELECT 2 AS n)) |> ORDER BY n",
            vec![vec![integer(1)], vec![integer(2)]],
        ),
        (
            "union-distinct-empty",
            "FROM facts |> WHERE amount < -1e308 AND amount > 0 |> UNION DISTINCT (FROM facts |> WHERE amount < -1e308 AND amount > 0) |> AGGREGATE COUNT(*) AS n",
            vec![vec![integer(0)]],
        ),
    ] {
        queries.check(label, sql, expected, "declared", true)?;
    }
    Ok(())
}
