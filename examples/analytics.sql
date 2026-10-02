FROM events AS f
|> LEFT JOIN dimensions AS d ON f.dimension_id = d.id
|> EXTEND EXTRACT(YEAR FROM f.happened) AS calendar_year,
          CASE WHEN f.amount IS NULL THEN 0 WHEN f.amount < 0 THEN -1 ELSE 1 END AS amount_class
|> AGGREGATE COUNT(*) AS entries, COUNT(f.amount) AS present, SUM(f.amount) AS total
   GROUP BY calendar_year, d.label, amount_class
|> EXTEND COUNT(*) OVER (PARTITION BY label) AS label_groups
|> EXTEND SUM(total) OVER (PARTITION BY label ORDER BY calendar_year) AS through_year
|> ORDER BY calendar_year NULLS FIRST, label NULLS FIRST, amount_class;
