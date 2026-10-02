FROM sales
|> EXTEND DIV(amount, 15) AS bucket
|> AGGREGATE COUNT(*) AS n, SUM(amount) AS total GROUP BY bucket
|> ORDER BY bucket NULLS FIRST;
