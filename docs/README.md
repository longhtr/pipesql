# Documentation

Start with the guide for the task you need to do. [README](../README.md) owns
the product constraints; the guides below own their stated technical concerns.
[The work plan](../notes/plan.md) records current work and publication
restrictions.

## Use PipeSQL

- [Create and query a declared table](getting-started.md): a complete Rust example.
- [Explore queries on a declared table](query-examples.md): runnable queries,
  expected results and implementation reading paths.
- [Language](language.md#current-public-query-manifest): supported queries and limits.
- [Public interfaces](interfaces.md): API ownership, CLI commands, errors, and recovery.

## Learn the implementation

Run the declared-table example first, then [trace a query through
preparation](frontend.md#trace-a-query-through-preparation). Keep the
corresponding source open as you read: follow `bind_plan` into
`Binder::bind_stage`, then inspect one projection or aggregate operation. The
example connects SQL names to typed column identities before introducing the
compact plan representation. Try the [EXTEND example](query-examples.md#transform-columns-while-retaining-the-original-values)
to follow preserved columns, new definitions, and alias scope.

Continue with [Planning](planning.md) for demanded inputs and the producer
graph, then [trace the same query through execution](execution.md#trace-a-query-through-execution)
for input requests and controller steps. Compare
[computed projection costs](getting-started.md#compare-computed-projection-costs)
to distinguish producer fusion from scalar evaluation and measure both complete
queries. Read
[result ownership](execution.md#result-ownership) for completion and cleanup.
Use the [joined workload](getting-started.md#follow-a-join-through-grouping-and-sorting)
to trace several blocking operators sharing one memory budget.
For the write path, read [Transactions](transactions.md) alongside its owners in
the [source map](source-map.md#persistence-and-native-effects). Use the [catalog
graph](storage.md#declared-table-catalog-graph) when you reach a stored
reference and the [admission protocol](resources.md#admission-protocol) when you
reach an allocation or spill boundary. For failures, follow
[recovery](transactions.md#recovery) and
[cancellation](concurrency.md#cancellation) back to the operation that owns
cleanup.

Try [LEFT JOIN with missing dimensions](getting-started.md#retain-facts-with-missing-dimensions)
to follow nullable output identities into matching, grouping and ordering.

Run the [snapshot lifetime example](getting-started.md#keep-an-old-snapshot-readable)
to connect preparation pins, append publication, aborted attempts and reclamation
in one public-library flow. It checks old/new rows and retained outcomes across
reopen. Follow [their distinct owners](transactions.md#follow-snapshot-pins-and-retained-outcomes)
to see why reader data pins and transaction history have separate lifetimes.

## Work on the repository

- [Engineering](engineering.md): how to choose, implement, review, and finish changes.
- [Testing](testing.md): setup, focused checks, the complete gate, and coverage locations.
- [Source map](source-map.md): implementation owners and their relationships.
- [Tools](../tools/README.md): maintained commands and their inputs and outputs.
- [Tests](../tests/README.md): suite organization and fixture ownership.

## Understand the contracts

| Concern | Authoritative guide |
| --- | --- |
| Components, dependencies, and cross-system invariants | [Architecture](architecture.md) |
| Accepted syntax and user-visible semantics | [Language](language.md) |
| Parsing, binding, scopes, and semantic identities | [Frontend](frontend.md) |
| Logical inputs, lowering, demand, and independent plan validation | [Planning](planning.md) |
| Producers, kernels, replay, and result lifetime | [Execution](execution.md) |
| Memory, temporary storage, admission, and release | [Resources](resources.md) |
| Native bytes, validation, construction, and format lifecycle | [Storage](storage.md) |
| Commit outcomes, publication, and recovery | [Transactions](transactions.md) |
| Reader/writer authority, locks, and cancellation | [Concurrency](concurrency.md) |
| Required evidence and permitted release claims | [Verification](verification.md) |

The language guide distinguishes current accepted forms from future directions.
Other contracts likewise label targets that have not been implemented. A target
is not permission to accept a partial feature or advertise a capability.

## Consult evidence

[Retained evidence](../notes/evidence.md) preserves current observations, consequential limitations,
and commands using maintained inputs. A result applies to its recorded source, inputs,
and platform. Consult the current plan before treating an old result as current
verification. Historical notes cannot override these contracts.
