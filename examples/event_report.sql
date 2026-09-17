FROM events AS f
|> LEFT JOIN dimensions AS d ON f.dimension_id = d.id
|> EXTEND EXTRACT(YEAR FROM f.happened) AS calendar_year
|> AGGREGATE COUNT(*) AS entries, COUNT(f.amount) AS present, SUM(f.amount) AS total
   GROUP BY calendar_year, d.label
|> ORDER BY calendar_year NULLS FIRST, label NULLS FIRST;
