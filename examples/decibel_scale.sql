FROM sales
|> WHERE amount > 0
|> ORDER BY amount
|> SELECT amount, 10*LOG10(amount/10) AS relative_db;
