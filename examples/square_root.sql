FROM sales
|> AGGREGATE AVG(amount*amount) AS mean_square
|> SELECT SQRT(mean_square) AS rms;
