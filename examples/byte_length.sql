FROM sales
|> SELECT BYTE_LENGTH(region) AS region_bytes, BYTE_LENGTH('雪') AS label_bytes
|> AGGREGATE MIN(region_bytes) AS shortest, MAX(region_bytes) AS longest,
             SUM(label_bytes) AS label_total;
