FROM sales
|> LIMIT 1
|> SELECT BYTE_LENGTH('é') AS composed_bytes, CHAR_LENGTH('é') AS composed_scalars,
          BYTE_LENGTH('e\u0301') AS decomposed_bytes, CHAR_LENGTH('e\u0301') AS decomposed_scalars,
          BYTE_LENGTH('😀') AS emoji_bytes, CHAR_LENGTH('😀') AS emoji_scalars,
          BYTE_LENGTH('👩‍💻') AS joined_bytes, CHAR_LENGTH('👩‍💻') AS joined_scalars;
