FROM sales
|> WHERE amount > 0
|> AGGREGATE AVG(LN(amount)) AS mean_log;
