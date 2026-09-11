# Upstream query inputs

These fixtures come from `google/zetasql` revision
`0e7d7073ed0360be587a5efa0fa78abeee00f17b`, under
`googlesql/examples/tpch/pipe_queries/`. They test the accepted Q1/Q6 surface,
not the entire upstream language.

| Fixture | Upstream file | Attribution and byte identity |
| --- | --- | --- |
| [Q1](q1-upstream.pipe.sql) | `1.sql` | Unchanged 1,204-byte source, including its Apache-2.0 header. SHA-256: `09377d7309b54e3d60522a4ec297baaaa75d5a6f44c3dea2d32ac93438be8f8a`. |
| [Q6](../q6.pipe.sql) | `6.sql` | Query body unchanged; attribution header shortened. Original upstream SHA-256: `774103dc043f2d9723742f29e8229cab9e4781a95b374f233d41d8aa96ca6c46`. |

The current Q6 fixture's full-file SHA-256 is
`25750dbadd3c60b946ffa4220da805aa8d0a627628be5aad79a97e22e5e4b0ee`.
Its comment points here; moving provenance does not change its SQL body.

The complete [Apache-2.0 license](../LICENSE.google-zetasql.txt) is retained from
that upstream revision. Its SHA-256 is
`cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30`.
