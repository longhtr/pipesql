FROM sales
|> EXTEND ABS(amount-10) AS deviation
|> SELECT region, amount, deviation
|> ORDER BY region, amount NULLS FIRST;
