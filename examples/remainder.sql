FROM sales
|> EXTEND MOD(amount,10) AS remainder
|> AGGREGATE COUNT(*) AS n,SUM(amount) AS total GROUP BY remainder
|> ORDER BY remainder NULLS FIRST;
