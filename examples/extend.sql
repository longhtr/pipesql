FROM sales AS s
|> EXTEND amount * 2 AS doubled
|> EXTEND doubled + 1 AS adjusted
|> WHERE adjusted IS NOT NULL
|> ORDER BY adjusted
|> SELECT s.region, s.amount, doubled, adjusted
