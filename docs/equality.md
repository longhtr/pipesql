# Compare equality with stored floating-point bits

Run this example to inspect eight stored DOUBLE values, group them, remove
duplicates and join the table to itself. The program checks every result against
literal expectations before printing its report.

Complete [the declared-table tutorial](getting-started.md) first. Use the same
[build prerequisites](testing.md#prerequisites) and run commands from the
repository root. The database path must be absolute and must not already exist.

```sh
cargo build --release --offline --locked --example equality
equality_root=$(mktemp -d)
target/release/examples/equality "$equality_root/database"
```

## Inspect the reopened values

The example creates a table with an INT64 `id` and a nullable DOUBLE `v`, appends
one batch, closes the database and reopens it. Its first query orders by `id`.
The first eight output lines are:

```text
input id=0 bits=8000000000000000
input id=1 bits=0000000000000000
input id=2 bits=7ff8000000000042
input id=3 bits=fff8000000001234
input id=4 value=NULL
input id=5 value=NULL
input id=6 bits=3ff0000000000000
input id=7 bits=3ff0000000000000
```

Rows 0 and 1 contain negative and positive zero. Rows 2 and 3 contain distinct
NaN representations. Rows 6 and 7 contain `1.0`. The hexadecimal output records
each non-NULL value's original bits.

NULL is represented by validity metadata. The append supplies different payloads
for rows 4 and 5, but their validity bits mark both values absent. The count query
reports `counts rows=8 present=6`: NaNs are present values and contribute to
`COUNT(v)`; NULLs do not.

## Compare the result classes

The example runs these queries:

```sql
FROM samples
|> AGGREGATE COUNT(*) AS entries GROUP BY v;

FROM samples
|> SELECT v
|> DISTINCT;
```

The report contains four groups of two rows and four DISTINCT representatives:

```text
group class=null rows=2
group class=nan rows=2
group class=zero rows=2
group class=one rows=2
distinct classes=4 representatives=original-input
```

GROUP BY and DISTINCT use the [not-distinct relation](language.md#values-null-and-equality).
Both zeros belong to one class, both NaNs belong to another, and both NULLs belong
to a third. DISTINCT retains an actual input representative without promising
which zero or NaN bits it chooses. The example accepts either original
representation and rejects invented bits, missing classes and duplicate classes.
The report's class order comes from the example, not a query ordering guarantee.

Next, the example checks the exact row IDs retained by four filters:

| Predicate | Retained IDs | Reported rows |
| --- | --- | --- |
| `v = 0` | 0, 1 | 2 |
| `v != 0` | 2, 3, 6, 7 | 4 |
| `v IS NULL` | 4, 5 | 2 |
| `v IS DISTINCT FROM 0` | 2, 3, 4, 5, 6, 7 | 6 |

Ordinary equality treats the two zeros as equal. NaN is unequal to zero and to
itself. Ordinary comparisons with NULL yield UNKNOWN, which WHERE does not
retain. `IS DISTINCT FROM` produces a Boolean result even for NULL.

## Check the join pairs

```sql
FROM samples AS l
|> JOIN samples AS r ON l.v = r.v
|> ORDER BY l.id, r.id
|> SELECT l.id AS left_id, r.id AS right_id;
```

Each zero matches both zero rows, and each `1.0` matches both `1.0` rows.
The eight reported pairs are `(0, 0)`, `(0, 1)`, `(1, 0)`, `(1, 1)`,
`(6, 6)`, `(6, 7)`, `(7, 6)` and `(7, 7)`. NULL and NaN never match, even though
grouping collected them into classes. Successful execution ends with
`status=equality-checked` and exit status zero.

## Follow the implementation and clean up

Open [equality.rs](../examples/equality.rs). `create_samples` supplies the literal
bits and validity mask. `read_numeric` checks result metadata, copies numeric
cells from borrowed batches and requires Finished. It then drops execution and
preparation owners and checks their logical reservation release. The example's
owned copies and these logical counters do not establish a process memory bound.

Follow [predicate evaluation](execution.md#predicate-evaluation) into the
comparison decision, then [duplicate removal](execution.md#duplicate-removal)
into key comparison and retained representatives. Finally, inspect
[equality joins](execution.md#equality-joins): sorting places equivalent keys
together, but the join separately requires ordinary equality before emitting
a pair.

After the example exits, remove its database and temporary directory:

```sh
rm -r "$equality_root"
```
