FROM sales
|> WHERE amount IS NOT NULL
|> EXTEND COUNT(*) OVER () AS total_rows
|> ORDER BY amount
|> SELECT region,amount,total_rows;
