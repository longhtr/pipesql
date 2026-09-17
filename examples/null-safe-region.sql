FROM facts
|> WHERE region IS DISTINCT FROM 3
|> AGGREGATE SUM(amount) AS total, COUNT(*) AS nrows;
