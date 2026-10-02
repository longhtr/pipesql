FROM facts AS f
|> LEFT JOIN regions AS r ON f.region = r.id
|> AGGREGATE SUM(f.amount) AS total, COUNT(*) AS n GROUP BY r.name
|> ORDER BY name NULLS FIRST;
