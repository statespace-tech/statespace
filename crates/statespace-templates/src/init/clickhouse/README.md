---
tools:
  - [clickhouse-client, --host, $CLICKHOUSE_HOST, --port, $CLICKHOUSE_PORT, --user, $CLICKHOUSE_USER, --password, $CLICKHOUSE_PASSWORD, --query, { regex: "^(SELECT|SHOW|DESCRIBE|EXPLAIN)\\b.*" }]
---

# Instructions
- Explore the schema to understand the data model
- Follow the user's instructions and answer their questions
- Reference [documentation](https://clickhouse.com/docs/) as needed
