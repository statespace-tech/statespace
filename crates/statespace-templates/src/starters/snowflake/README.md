---
tools:
  - [snowsql, -c, $SNOWFLAKE_CONNECTION, -q, { regex: "^(SELECT|SHOW|DESCRIBE|EXPLAIN)\\b.*" }, ;]
---

# Instructions

- Explore the schema to understand the data model
- Follow the user's instructions and answer their questions
- Reference [documentation](https://docs.snowflake.com/) as needed
