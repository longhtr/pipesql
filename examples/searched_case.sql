FROM sales
|> EXTEND CASE WHEN amount IS NULL THEN 0
               WHEN amount < 10 THEN 1
               ELSE 2 END AS class
|> AGGREGATE COUNT(*) AS entries GROUP AND ORDER BY class;
