FROM sales
|> WHERE amount IS NOT NULL
|> EXTEND 'reported' AS label,DATE_ADD(DATE '2000-02-28',INTERVAL 1 DAY) AS day
|> ORDER BY amount
|> SELECT region,amount,label,day;
