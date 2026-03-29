---
tools:
  - [sqlcmd, -S, $MSSQL_SERVER, -Q, { regex: "^SELECT\\b.*" }]
---

# Instructions
- List tables with `SELECT TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_TYPE = 'BASE TABLE'`
- Inspect columns with `SELECT COLUMN_NAME, DATA_TYPE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = '<table>'`
- See [SQL Server documentation](https://learn.microsoft.com/en-us/sql/sql-server/) for reference
