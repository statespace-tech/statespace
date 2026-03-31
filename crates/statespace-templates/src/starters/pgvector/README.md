---
tools:
  - [psql, -d, $DATABASE_URL, -c, { regex: "^(SELECT|SHOW|EXPLAIN)\\b.*" }, ;]
---

# Instructions

- Explore the schema to understand the data model
- Follow the user's instructions and answer their questions
- Reference [documentation](https://github.com/pgvector/pgvector) as needed
