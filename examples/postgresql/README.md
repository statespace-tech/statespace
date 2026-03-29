---
tools:
  - [psql, -d, $DATABASE_URL, -c, { regex: "^(SELECT|SHOW|EXPLAIN)\\b.*" }]
---

# Instructions
- Explore the schema with `SELECT table_name FROM information_schema.tables WHERE table_schema = 'public'`
- Inspect columns with `SELECT column_name, data_type FROM information_schema.columns WHERE table_name = '<table>'`
- See [PostgreSQL documentation](https://www.postgresql.org/docs/) for reference
