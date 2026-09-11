FROM sales AS s
|> EXTEND amount * 2 AS doubled
|> SET amount=doubled
|> RENAME amount AS subtotal
|> DROP doubled
|> EXTEND subtotal + 1 AS adjusted
|> WHERE adjusted IS NOT NULL
|> ORDER BY adjusted
|> SELECT s.region, s.amount, subtotal, adjusted
