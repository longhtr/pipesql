# Expressions

Expressions calculate values within a row. They do not choose how many rows an
operator returns. A numeric expression can appear in SELECT, EXTEND, SET, or an
aggregate argument; a predicate decides whether WHERE keeps a row. Functions have
only the signatures documented here.

## Numbers and NULL

Numeric operands can be visible numeric columns, INT64 or finite DOUBLE literals,
parenthesized expressions, and the calls below. Unary signs and `+`, `-`, `*`, `/`
are supported. Multiplication and division have equal precedence and associate
left to right. Predicate constants use the same arithmetic without column references.

INT64 arithmetic is checked before any surrounding DOUBLE conversion. For example,
adding a DOUBLE later cannot rescue an overflowing integer multiplication.
Division always returns DOUBLE, including INT64 divided by INT64. Other mixed
numeric operations convert at the operation that needs a common type.

NULL begins as nullable INT64. Its enclosing operation may convert that type;
a standalone NULL projection remains INT64. Except for the conditional calls
specified below, arguments evaluate before the operation handles NULL. A NULL
argument is not a way to hide a failure calculating another argument.

For `/`, NULL produces NULL. Otherwise either signed-zero denominator produces
`DivisionByZero`, even with an infinite or NaN numerator. Finite inputs producing
infinity cause `ArithmeticOverflow`; representable subnormals remain valid and
underflow can become signed zero. Stored nonfinite values follow each operation's
specified rules, although NaN and infinity literals are not SQL syntax.

## Choose a result

### COALESCE

`COALESCE(value, fallback)` returns the first non-NULL numeric value. It evaluates
`value` first and evaluates `fallback` only if needed. Thus `COALESCE(1, 1/0)`
returns DOUBLE 1 without division by zero. An error in the first argument is still
an error, not a missing value.

Two INT64 arguments give INT64; either DOUBLE argument gives DOUBLE. Only the
selected value is converted. The result permits NULL only when both arguments do.
Exactly two arguments are supported.

### NULLIF

`NULLIF(value, sentinel)` evaluates both numeric arguments, in order, and returns
NULL when their converted values compare equal. Otherwise it returns the first
converted value. Two INT64 arguments stay exact; either DOUBLE argument makes both
the comparison and result DOUBLE. The result is nullable.

Unlike COALESCE, a NULL first argument does not skip the second. NaNs compare
unequal, signed zeros compare equal, and a returned DOUBLE keeps its bits.
`NULLIF(amount, 0)` can exclude zero from an aggregate without removing its row.

### Searched CASE

```sql
CASE WHEN amount IS NULL THEN 0
     WHEN amount < 10 THEN 1
     ELSE 2 END
```

CASE tries conditions in written order and evaluates only the first selected
result. A comparison yielding NULL does not select its branch. If nothing matches,
ELSE supplies the result; without ELSE the result is NULL. Later conditions and
unselected results are not evaluated.

Conditions compare numeric expressions using `<`, `<=`, `=`, `!=`, `>=`, or `>`;
or test one numeric expression with IS NULL or IS NOT NULL. Comparison operands
evaluate left to right. Results are numeric expressions, including nested CASE.
All-INT64 results give INT64; any DOUBLE result makes the common type DOUBLE.
Conditions do not affect that type. A selected DOUBLE keeps its bits. The result
is conservatively nullable if any result is nullable or ELSE is absent. An
all-NULL CASE has type INT64.

Every branch still binds its names, types, and literal ranges. The 32-operation
budget includes condition operands, result expressions, selection operations, and
the final fallback. IS NULL includes a NULL operand; omitted ELSE includes a NULL
fallback. Nesting shares that budget.

Simple `CASE value WHEN ...`, STRING/DATE conditions or results, Boolean literals,
NOT/AND/OR conditions, IN, BETWEEN, and parentheses around a complete condition are
unsupported. Parentheses around numeric operands are allowed. For text or date
classification, project a numeric measure such as length or year in an earlier stage.

## Arithmetic functions

Numeric calls may nest within the ordinary 32-operation and parser-stack limits.
A unary or binary call adds one operation beyond its operands. Unlisted arities,
types, and the generic `SAFE.` prefix are unsupported.

| Call | Result type | Non-NULL behavior |
| --- | --- | --- |
| `DIV(a, b)` | INT64; both arguments must be INT64. | Truncate quotient toward zero. Zero divisor fails; INT64_MIN / -1 overflows. |
| `MOD(a, b)` | INT64; both arguments must be INT64. | Remainder has the dividend's sign. Zero divisor fails; INT64_MIN modulo -1 is zero. |
| `ABS(x)` | Argument type. | Absolute value; INT64_MIN overflows. DOUBLE gives positive zero for either zero and positive infinity for either infinity. NaN remains NaN. |
| `SIGN(x)` | Argument type. | -1, 0, or 1 by sign, including infinities. DOUBLE zeros produce positive zero; NaN bits are retained. |
| `CAST(x AS DOUBLE)` | DOUBLE. | Convert INT64 to nearest binary64, ties to even; DOUBLE retains its bits. `FLOAT64` is an alias for this target. |
| `FLOOR(x)` | DOUBLE. | Largest integral value no greater than x. |
| `CEIL(x)`, `CEILING(x)` | DOUBLE. | Smallest integral value no less than x. |
| `ROUND(x)` | DOUBLE. | Nearest integral value; halfway values go away from zero. |
| `SQRT(x)` | DOUBLE. | Correctly rounded square root; negative input, including negative infinity, is a domain error. |
| `LN(x)` | DOUBLE. | Natural logarithm, with the exceptions below. |
| `LOG10(x)` | DOUBLE. | Base-ten logarithm, with the exceptions below. |
| `EXP(x)` | DOUBLE. | e to the power x, with the exceptions below. |
| `POW(x, y)`, `POWER(x, y)` | DOUBLE. | Power, with the exceptional-value table below. |
| `SAFE_DIVIDE(a, b)` | Nullable DOUBLE. | NULL for its own zero-divisor or finite-overflow failure; ordinary division otherwise. |

Calls returning DOUBLE convert INT64 arguments before the function runs.
All these calls propagate NULL. Except SAFE_DIVIDE, their results are nullable
exactly when an argument is nullable. DIV and MOD handle NULL before checking the divisor,
but their argument expressions have already evaluated. SAFE_DIVIDE does not catch
an error calculating its arguments. Its folded NULL is still DOUBLE, so comparing
it with a STRING is a type error.

CAST accepts only numeric input and these two target spellings. It does not support
STRING/DATE conversion, formatting clauses, or SAFE_CAST. Conversion loses integer
precision when binary64 cannot represent the value: 9007199254740993 becomes
9007199254740992, and INT64_MAX becomes 9223372036854775808. Earlier integer
arithmetic remains exact and checked. CAST itself cannot overflow: every INT64
is within DOUBLE's finite range, even when it cannot be represented exactly.

FLOOR, CEIL, and ROUND convert INT64 to DOUBLE *before* rounding. They preserve
infinities, input NaN bits, and zero signs. Rounding a small negative value can
produce negative zero. ROUND accepts neither a decimal-place argument nor a
rounding-mode argument. SQRT preserves both zero signs and input NaN bits; positive
infinity stays positive infinity. Its domain error names `square root`.

### Logarithms and exponential

| Input | LN / LOG10 | EXP |
| --- | --- | --- |
| Either zero | Domain error. | Exactly 1. |
| Finite negative | Domain error. | Positive finite result, possibly underflowing to positive zero. |
| Negative infinity | Quiet NaN `7ff8000000000000`. | Positive zero. |
| Positive infinity | Positive infinity. | Positive infinity. |
| NaN | Preserve its bits. | Preserve its bits. |

LN(1) and LOG10(1) produce positive zero. Their domain errors name
`natural logarithm` and `base-ten logarithm`. EXP of a finite value overflows if
the result becomes infinity, naming `exponentiation`. Subnormal results are retained.

The negative-infinity logarithm result follows the pinned
[GoogleSQL compliance case](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/googlesql/compliance/functions_testlib_math.cc#L1515),
which differs from its prose description of nonpositive inputs. The generated
NaN bits are PipeSQL's explicit choice.

These functions use Rust's floating-point logarithm and exponential operations.
Finite precision may vary by platform, compiler, and invocation; correct rounding
or repeated bit identity is not promised. LOG, LOG2, LOG1P, EXP2, and EXPM1 are
unsupported.

### Power

POW converts each INT64 operand before testing exponent properties. In particular,
`POW(-1, 9007199254740993)` returns 1: conversion rounds the exponent to the even
integer 9007199254740992.

NULL wins before power identities. Otherwise exponent zero or base one returns
exactly 1, even when the other value is NaN. Without either identity, an input NaN
is returned with its bits; the base is checked first. Remaining cases are:

| Base | Exponent | Result |
| --- | --- | --- |
| Absolute value 1 | Either infinity | 1. |
| Absolute value below 1 | Positive / negative infinity | Positive zero / positive infinity. |
| Absolute value above 1 | Positive / negative infinity | Positive infinity / positive zero. |
| Either infinity | Finite positive | Infinity, negative only for a negative base and odd integral exponent. |
| Either infinity | Finite negative | Zero, negative only for a negative base and odd integral exponent. |
| Either zero | Finite positive | Zero, negative only for negative zero and an odd integral exponent. |
| Either zero | Finite negative | Domain error. |
| Finite negative | Finite nonintegral | Domain error. |

Other finite inputs use `f64::powf`. Overflow and domain failures name `power`.
Subnormals are retained; smaller results can underflow to zero. No universal ULP
bound, correct rounding, or cross-platform bit identity is promised. The identities
above do not suppress errors while evaluating either argument.

## Text and dates

STRING and DATE constants may be whole expressions in SELECT, EXTEND, and SET,
including parentheses. They own their values after query preparation and are
nonnullable. Bare NULL is not an untyped STRING or DATE constant.

`BYTE_LENGTH(s)` counts UTF-8 bytes. `CHAR_LENGTH(s)` counts Unicode scalar values,
not graphemes: `é` has 2 bytes and 1 scalar; its decomposed spelling has 3 bytes
and 2 scalars. Neither normalizes text. Each call accepts one visible STRING column
or STRING literal and returns INT64 with the input's nullability.

These length calls must be whole projection expressions. Parentheses around the
call or argument are allowed; nested calls, direct arithmetic around them,
`SUM(BYTE_LENGTH(s))`, and untyped NULL are not. Name the result in one stage and
use it numerically in the next. OCTET_LENGTH, LENGTH, and CHARACTER_LENGTH aliases
are unsupported. Literal lengths are calculated during preparation.

DATE literals use Gregorian years 0001–9999, for example `DATE '2026-01-01'`.
A shift has the form `DATE_ADD(date_constant, INTERVAL integer_constant MONTH)`
or DATE_SUB, with DAY and YEAR also accepted. Constant DATE_ADD and DATE_SUB accept
checked INT64 intervals in DAY, MONTH, or YEAR units, with at most eight nested
calls. Month/year shifts clamp to the destination month's last day. Every
intermediate date must remain in range; column-valued shifts are unsupported.

`EXTRACT(YEAR FROM d)` accepts a visible DATE column or a supported DATE constant
expression. It returns the Gregorian year, 1–9999, as INT64 with the input's
nullability. It too must be a whole SELECT, EXTEND, or SET expression. Project it
before arithmetic or aggregation. Other date parts, nested extraction, untyped
NULL, and `SUM(EXTRACT(...))` are unsupported. Constant dates and shifts fold during
preparation; parentheses around the call and input are allowed.

### Quoted literals

Use one single- or double-quoted token. Both the source payload, excluding quotes,
and decoded UTF-8 are limited to 32 bytes. Empty text and Unicode are supported.
Recognized escapes are:

- `\a`, `\b`, `\f`, `\n`, `\r`, `\t`, `\v`, `\\`, `\?`, `\"`, `\'`, and `` \` ``;
- three-digit octal `\000`–`\377`;
- two-digit hexadecimal `\xhh` or `\Xhh`;
- four-digit `\uhhhh` and eight-digit `\Uhhhhhhhh` Unicode values.

Numeric escapes encode Unicode scalars. Surrogates and values above U+10FFFF fail.
Raw prefixes, triple quoting, adjacent literal concatenation, unknown escapes,
and unescaped newlines are unsupported. DATE uses this decoder before validating
the calendar value.

## Predicates

WHERE compares a visible column with a compatible constant using `<`, `<=`, `=`,
`!=`, `>=`, or `>`. INT64 comparison is exact until a DOUBLE operand requires
conversion. STRING compares UTF-8 bytes without collation or normalization;
DATE compares dates. Column-to-column equality belongs to JOIN; CASE has its own
numeric condition rules.

An ordinary comparison with NULL produces UNKNOWN, and WHERE keeps only TRUE.
NaN is unordered under numeric comparisons; both zero signs compare equal.
Use `name IS NULL` or `name IS NOT NULL` to test missingness. Those tests always
produce TRUE or FALSE and demand the column even when declared nullability could
predict the answer. They do not hide a computed error or corrupt payload.

`name IS [NOT] DISTINCT FROM constant` never produces UNKNOWN. It treats NULLs as
equal, all NaNs as equal, and both zero signs as equal. Bare NULL is accepted with
any supported column type; a folded numeric NULL retains its numeric type.
For example, `amount IS DISTINCT FROM 3` retains missing amounts, while
`amount != 3` rejects them.

`name [NOT] IN (constant, ...)` tests a nonempty literal list in written order.
A match gives TRUE. Without a match, a NULL search value or NULL candidate gives
UNKNOWN; otherwise FALSE. NOT preserves UNKNOWN. Thus `amount IN (5, 20, NULL)`
keeps 5 and 20, while its negation keeps nothing. Duplicate candidates do not
duplicate rows. Each candidate consumes one predicate stage. All bind, even in
a skipped branch. This does not introduce ordinary `name = NULL` syntax.

BETWEEN includes both endpoints; NOT BETWEEN negates that complete test. With
finite bounds, a NaN search value makes NOT BETWEEN TRUE. Each range uses two
predicate stages. Expression lists, membership subqueries, IS TRUE/FALSE/UNKNOWN,
and Boolean scalar values are unsupported.

## Evaluation

### Boolean filters

NOT, AND, OR, and parentheses combine predicates. Comparisons bind first, then
NOT, AND, and OR. Infix NOT is accepted before IN and BETWEEN. Parentheses and NOT
add no predicate stages.

A filter visits operands in order and stops when it knows whether the row can be
kept. UNKNOWN is neither TRUE nor FALSE, so negation changes what must be tested:

| Operand values | Must a filter inspect `bad`? | Reason |
| --- | --- | --- |
| `NULL AND bad` | No. | It cannot be TRUE. |
| `NOT (NULL AND bad)` | Yes. | A FALSE `bad` would make the result TRUE. |
| `NOT (NULL OR bad)` | No. | It cannot be TRUE. |

Here NULL describes an operand's value; a bare NULL predicate is not syntax.
Negation acts on truth, not by reversing comparison operators. For NaN,
`NOT (n < 0)` is TRUE but `n >= 0` is FALSE.

Skipped branches do not evaluate their computed values or demand their source
payloads. Reading a storage block still requires validating the whole consumed
block. COALESCE's scalar laziness also does not guarantee that a scan never loaded
one of the expression's potential dependencies.

### Projections and operator boundaries

Defining a computed column does not necessarily evaluate it. Across consecutive
nonanalytic SELECT, EXTEND, SET, DROP, RENAME, WHERE, and AS stages, filters run in
written order. A rejected row does not demand later filters or final projected
values. Remaining rows demand the output and the next consuming operator's inputs.

JOIN, AGGREGATE, ORDER BY, and LIMIT retain their input boundaries. Required keys,
arguments, and downstream payloads can be computed before a following filter sees
the row. Moving a filter across such a boundary can change both values and errors.

Aggregate final values remain demand-driven through subsequent ordinary projections
and filters. Replacing SUM's alias `s` with `s + 0` cannot force overflow for a
group already rejected by a predicate that does not need `s`. Its required input
arguments still evaluate. CASE and COALESCE may skip aggregate finalization in
the same producer, but cannot undo an earlier materialization.

Every expression binds its syntax, names, types, and literal ranges. Projection
arithmetic is runtime work, including constant arithmetic; empty input or an
unused output can leave it unevaluated. WHERE and LIMIT constants instead evaluate
at preparation. An unchosen branch may avoid runtime failure, not a binding error.

### Errors and spans

Parse, binding, arithmetic, conversion, resource, cancellation, I/O, and corruption
errors remain distinct. A report can fail after earlier batches; receiving a prefix
is not successful completion.

A source span is a half-open UTF-8 byte range in the submitted query. Preparation
arithmetic identifies the complete numeric constant expression. Runtime aggregate
argument or finalization errors identify the aggregate call; projection errors
identify the complete computed expression. Aliases are excluded. An aggregate
supplying a projection retains its own call span.

If calls share argument evaluation, an argument failure identifies a demanded
call using that program. Final SUM overflow identifies SUM even when it shares
state with AVG. No expression is evaluated merely to attach a nicer error location.
When several rows could fail, the particular row encountered first is unspecified;
the category and a valid demanded source span still matter.

The [pinned semantic source](README.md#semantic-source) supplies the upstream
[mathematical](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/mathematical_functions.md),
[conditional](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/conditional_expressions.md),
[conversion](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/conversion_rules.md),
[text](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/string_functions.md),
and [date](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/date_functions.md)
contracts. PipeSQL's accepted signatures and explicit raw-bit choices are the
rules on this page, not every form available upstream.
