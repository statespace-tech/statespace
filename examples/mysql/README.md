---
tools:
  - [mysql, -h, $DB_HOST, -u, $DB_USER, "-p$DB_PASS", $DB_NAME, -e, { regex: "^(SELECT|SHOW|DESCRIBE|EXPLAIN)\\b.*" }]
---

# Instructions
- Explore the schema with `SHOW TABLES` and `DESCRIBE <table>`
- See [MySQL documentation](https://dev.mysql.com/doc/) for reference
