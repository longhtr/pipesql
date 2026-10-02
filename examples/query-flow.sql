FROM sales
|> SELECT amount AS subtotal
|> SELECT subtotal + 1 AS adjusted
|> AGGREGATE SUM(adjusted) AS total;
