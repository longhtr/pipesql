FROM facts
|> SELECT region
|> EXCEPT ALL (FROM regions |> SELECT id)
|> ORDER BY region NULLS FIRST;
