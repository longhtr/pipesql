FROM facts AS f
|> LEFT JOIN regions AS r ON f.region = r.id
|> SELECT COALESCE(r.id, 0) AS region, f.amount
|> AGGREGATE SUM(amount) AS total, COUNT(*) AS n GROUP BY region
|> ORDER BY region;
