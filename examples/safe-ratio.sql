FROM sales
|> SELECT region,amount AS denominator
|> EXTEND SAFE_DIVIDE(10,denominator) AS ratio
|> ORDER BY region,denominator NULLS FIRST;
