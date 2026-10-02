FROM sales
|> EXTEND ROUND(amount/15) AS bucket
|> AGGREGATE SUM(amount) AS total, COUNT(*) AS n GROUP AND ORDER BY bucket;
