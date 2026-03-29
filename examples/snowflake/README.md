---
tools:
  - [snowsql, -c, $SNOWFLAKE_CONNECTION, -q, { regex: "^(SELECT|SHOW|DESCRIBE|EXPLAIN)\\b.*" }]
---

# Instructions
- Explore with `SHOW DATABASES`, `SHOW SCHEMAS IN DATABASE <db>`, `SHOW TABLES IN SCHEMA <db>.<schema>`
- Inspect a table with `DESCRIBE TABLE <db>.<schema>.<table>`
- See [Snowflake documentation](https://docs.snowflake.com/) for reference
