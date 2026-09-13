FROM sales
|> WHERE amount NOT IN (5) AND amount NOT BETWEEN 0 AND 10
|> SELECT region, amount;
