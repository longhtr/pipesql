//! Compare expression placement with arithmetic over independent input rows.
//!
//! Cases place computations before and inside aggregates and filters, then compare
//! complete results with the corresponding row calculations. Q1 also checks its
//! full typed schema; Q6 requires a corpus that actually selects rows, so an empty
//! input cannot make its predicates appear correct. Shared process and grouping
//! helpers supply mechanics while the expressions and expected calculations remain
//! explicit in each case.

use super::*;
use Aggregate::{Average, Count, Sum};

const Q1_SCHEMA: &str = "l_returnflag:string:required|l_linestatus:string:required|sum_qty:double:nullable|sum_base_price:double:nullable|sum_disc_price:double:nullable|sum_charge:double:nullable|avg_qty:double:nullable|avg_price:double:nullable|avg_disc:double:nullable|count_order:int64:required";

fn check_q1(queries: &mut Queries, sql: &str, expected: Vec<Vec<String>>) -> Result<()> {
    let output = queries.execute(sql, "data")?;
    verify(&output, &expected, true, Some(Q1_SCHEMA))?;
    queries.count += 1;
    println!("composition Q1: {} typed rows passed", expected.len());
    Ok(())
}

pub(super) fn run(queries: &mut Queries, rows: &[Row]) -> Result<()> {
    let all: Vec<_> = rows.iter().collect();
    let q1 = "FROM lineitem |> AGGREGATE SUM(l_quantity) AS sum_qty, SUM(l_extendedprice) AS sum_base_price, SUM(l_extendedprice*(1-l_discount)) AS sum_disc_price, SUM(l_extendedprice*(1-l_discount)*(1+l_tax)) AS sum_charge, AVG(l_quantity) AS avg_qty, AVG(l_extendedprice) AS avg_price, AVG(l_discount) AS avg_disc, COUNT(*) AS count_order GROUP AND ORDER BY l_returnflag, l_linestatus";
    let entries = [
        Sum(quantity),
        Sum(price),
        Sum(|r| price(r) * (1.0 - discount(r))),
        Sum(|r| price(r) * (1.0 - discount(r)) * (1.0 + tax(r))),
        Average(quantity),
        Average(price),
        Average(discount),
        Count,
    ];
    let deep = format!("l_quantity{}{}", "+(1".repeat(15), ")".repeat(15));
    queries.check("deep-expression-under-cap", &format!("FROM lineitem |> AGGREGATE SUM({deep}) AS complex, SUM(l_quantity) AS q, SUM(l_extendedprice) AS p, SUM(l_discount) AS d, SUM(l_tax) AS t GROUP AND ORDER BY l_returnflag, l_linestatus"), groups(&all, &[0,1], &[Sum(|r| quantity(r) + 15.0), Sum(quantity), Sum(price), Sum(discount), Sum(tax)]), "data", true)?;
    check_q1(queries, q1, groups(&all, &[0, 1], &entries))?;
    type Model = fn(&Row) -> f64;
    let cases: [(&str, String, Model); 8] = [
        ("precedence", "l_quantity+2*3".into(), |r| quantity(r) + 6.0),
        ("parentheses", "(l_quantity+2)*3".into(), |r| {
            (quantity(r) + 2.0) * 3.0
        }),
        ("unary", "-(l_quantity-2)*-3".into(), |r| {
            -(quantity(r) - 2.0) * -3.0
        }),
        (
            "integer-intermediate",
            "l_extendedprice*(9007199254740993-9007199254740992)".into(),
            price,
        ),
        (
            "int64-min",
            "l_quantity+(-9223372036854775808+9223372036854775807)".into(),
            |r| quantity(r) - 1.0,
        ),
        ("signed-zero", "l_quantity*-0.0".into(), |r| {
            quantity(r) * -0.0
        }),
        (
            "exact-operation-bound",
            format!("-(l_quantity{})", "+1".repeat(15)),
            |r| -(quantity(r) + 15.0),
        ),
        (
            "exact-nesting-bound",
            format!("{}l_quantity{}", "(".repeat(32), ")".repeat(32)),
            quantity,
        ),
    ];
    for (label, expression, model) in cases {
        let expected = groups(&all, &[], &[Sum(model)]);
        queries.check(
            label,
            &format!("FROM lineitem |> AGGREGATE SUM({expression}) AS value"),
            expected.clone(),
            "data",
            false,
        )?;
        queries.check(
            &format!("computed-{label}"),
            &format!("FROM lineitem |> SELECT {expression} AS x |> AGGREGATE SUM(x) AS value"),
            expected,
            "data",
            false,
        )?;
        let filtered: Vec<_> = rows.iter().filter(|r| model(r) < 20.0).collect();
        queries.check(&format!("computed-filter-{label}"), &format!("FROM lineitem |> SELECT {expression} AS x |> WHERE x < 20 |> AGGREGATE SUM(x) AS value"), groups(&filtered, &[], &[Sum(model)]), "data", false)?;
    }
    let cutoff = day_offset(1998, 12, 1) - 74;
    let selected: Vec<_> = rows.iter().filter(|r| r.day <= cutoff).collect();
    let query = fs::read_to_string(
        queries
            .run
            .root
            .join("test/data/upstream/q1-upstream.pipe.sql"),
    )?;
    check_q1(queries, &query, groups(&selected, &[0, 1], &entries))?;
    let selected: Vec<_> = rows
        .iter()
        .filter(|r| {
            r.day >= day_offset(1994, 1, 1)
                && r.day < day_offset(1995, 1, 1)
                && (0.08 - 0.01..=0.08 + 0.01).contains(&discount(r))
                && quantity(r) < 25.0
        })
        .collect();
    if selected.is_empty() {
        return Err("Q6 corpus no longer selects any rows".into());
    }
    let query = fs::read_to_string(queries.run.root.join("test/data/q6.pipe.sql"))?;
    queries.check(
        "actual-q6-source",
        &query,
        groups(&selected, &[], &[Sum(|r| price(r) * discount(r))]),
        "data",
        false,
    )?;
    Ok(())
}
