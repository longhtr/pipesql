FROM sales
|> EXTEND SUM(amount) OVER (ORDER BY region) AS through_region
|> EXTEND SUM(amount) OVER (PARTITION BY region ORDER BY amount) AS regional_total
|> ORDER BY region, amount NULLS FIRST
|> SELECT region, amount, through_region, regional_total;
