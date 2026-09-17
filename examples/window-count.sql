FROM sales
|> WHERE amount IS NOT NULL
|> EXTEND COUNT(*) OVER () AS total_rows
|> EXTEND COUNT(*) OVER (PARTITION BY region) AS regional_rows
|> ORDER BY amount
|> SELECT region, amount, total_rows, regional_rows;
