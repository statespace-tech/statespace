---
tools:
  - [psql, -d, $DATABASE_URL, -c, { regex: "^(SELECT|SHOW|EXPLAIN)\\b.*" }]
---

# Instructions
- Find tables with vector columns with `SELECT table_name, column_name FROM information_schema.columns WHERE udt_name = 'vector'`
- Choose the distance operator that matches the index on the column: `<->` L2, `<=>` cosine, `<#>` inner product, `<+>` L1
- Always include a `LIMIT` clause on similarity searches
- See [pgvector documentation](https://github.com/pgvector/pgvector) for reference
