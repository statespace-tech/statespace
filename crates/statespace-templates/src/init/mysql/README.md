---
tools:
  - [mysql, -h, $DB_HOST, -u, $DB_USER, "-p$DB_PASS", $DB_NAME, -e, { regex: "^(SELECT|SHOW|DESCRIBE|EXPLAIN)\\b.*" }]
---

# Instructions
- Explore the schema to understand the data model
- Follow the user's instructions and answer their questions
- Reference [documentation](https://dev.mysql.com/doc/) as needed
