---
tools:
  - [clickhouse-client, --host, $CLICKHOUSE_HOST, --port, $CLICKHOUSE_PORT, --user, $CLICKHOUSE_USER, --password, $CLICKHOUSE_PASSWORD, --query, { regex: "^(SELECT|SHOW|DESCRIBE|EXPLAIN)\\b.*" }]
---

# Instructions
- Explore with `SHOW DATABASES`, `SHOW TABLES FROM <database>`, and `DESCRIBE TABLE <database>.<table>`
- ClickHouse is columnar — prefer aggregations over row lookups
- See [ClickHouse documentation](https://clickhouse.com/docs/) for reference
