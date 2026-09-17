FROM sales
|> WHERE amount IN (5, 20, NULL)
|> ORDER BY amount
|> SELECT region, amount;
