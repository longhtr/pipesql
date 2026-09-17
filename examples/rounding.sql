FROM sales
|> EXTEND FLOOR(amount/15) AS bucket
|> AGGREGATE SUM(amount) AS total, COUNT(*) AS n GROUP AND ORDER BY bucket;
