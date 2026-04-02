---
tools:
  - [sqlcmd, -S, $MSSQL_SERVER, -U, $MSSQL_USER, -P, $MSSQL_PASSWORD, -d, $MSSQL_DATABASE, -Q, { regex: "^(SELECT(?!.*\\bINTO\\b)|EXEC sp_(help|tables|columns|helptext|databases))\\b.*" }, ;]
---

# Instructions
- Explore the schema to understand the data model
- Follow the user's instructions and answer their questions
- Reference [documentation](https://learn.microsoft.com/en-us/sql/sql-server/) as needed
