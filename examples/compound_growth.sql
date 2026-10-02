FROM sales
|> WHERE amount > 0
|> ORDER BY amount
|> SELECT amount AS rate_percent, 1000*POWER(1+amount/100, 3) AS compounded;
