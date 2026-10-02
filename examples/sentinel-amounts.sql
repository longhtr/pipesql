FROM facts
|> AGGREGATE SUM(NULLIF(amount, 20)) AS total,
    COUNT(NULLIF(amount, 20)) AS measured, COUNT(*) AS nrows;
