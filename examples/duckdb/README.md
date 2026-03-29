---
tools:
  - [duckdb, -c, { regex: "^(SELECT|DESCRIBE|EXPLAIN)\\b.*" }]
---

# Instructions
- Query files directly with `SELECT * FROM 'path/to/file.parquet'` or `read_csv('path/to/file.csv')`
- Inspect columns with `DESCRIBE SELECT * FROM '<file>'`
- See [DuckDB documentation](https://duckdb.org/docs/) for reference
