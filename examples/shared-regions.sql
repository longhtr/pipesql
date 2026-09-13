FROM facts
|> SELECT region
|> INTERSECT DISTINCT (FROM regions |> SELECT id)
|> ORDER BY region;
