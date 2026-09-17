FROM facts
|> SELECT region
|> EXCEPT DISTINCT (FROM regions |> SELECT id)
|> ORDER BY region NULLS FIRST;
