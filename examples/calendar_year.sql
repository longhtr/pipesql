FROM events
|> EXTEND EXTRACT(YEAR FROM happened) AS calendar_year
|> AGGREGATE SUM(amount) AS total, COUNT(*) AS entries GROUP AND ORDER BY calendar_year;
