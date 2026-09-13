FROM sales
|> AGGREGATE SUM(amount) AS total,COUNT(amount) AS measured GROUP AND ORDER BY region
|> SELECT region,total/measured AS mean_amount;
