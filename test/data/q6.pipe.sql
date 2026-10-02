# Extracted from google/zetasql at commit
# 0e7d7073ed0360be587a5efa0fa78abeee00f17b.
# Upstream path: googlesql/examples/tpch/pipe_queries/6.sql
# Upstream file SHA-256:
# 774103dc043f2d9723742f29e8229cab9e4781a95b374f233d41d8aa96ca6c46
# Copyright 2019 Google LLC. Licensed under Apache-2.0; see the upstream
# LICENSE.google-zetasql.txt and provenance in upstream/README.md.

FROM
  lineitem
|> WHERE
    l_shipdate >= date '1994-01-01'
    AND l_shipdate < date_add(date '1994-01-01', INTERVAL 1 year)
    AND l_discount BETWEEN 0.08 - 0.01 AND 0.08 + 0.01
    AND l_quantity < 25
|> AGGREGATE
    sum(l_extendedprice * l_discount) AS revenue;
