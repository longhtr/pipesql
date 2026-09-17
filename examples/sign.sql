FROM sales
|> EXTEND SIGN(amount-10) AS direction
|> AGGREGATE SUM(amount) AS total, COUNT(*) AS n GROUP AND ORDER BY direction;
