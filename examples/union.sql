FROM sales
|> WHERE region = 'north'
|> SELECT region,amount
|> UNION DISTINCT (
  FROM sales
  |> WHERE amount >= 10
  |> SELECT region AS area,amount AS value
)
|> AGGREGATE SUM(amount) AS total,COUNT(*) AS n GROUP AND ORDER BY region;
