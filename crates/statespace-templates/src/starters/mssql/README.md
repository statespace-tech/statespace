---
tools:
  - [sqlcmd, -S, $MSSQL_SERVER, -Q, { regex: "^\\s*SELECT\\b[^;]*$" }, ;]
---

# Instructions

- Explore the schema to understand the data model
- Follow the user's instructions and answer their questions
- Reference [documentation](https://learn.microsoft.com/en-us/sql/sql-server/) as needed
