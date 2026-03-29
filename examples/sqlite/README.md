---
tools:
  - [sqlite3, data.db, { regex: "^(SELECT|PRAGMA|EXPLAIN)\\b.*" }]
---

# Instructions
- List tables with `SELECT name FROM sqlite_master WHERE type = 'table'`
- Inspect columns with `PRAGMA table_info(<table>)`
- See [SQLite documentation](https://www.sqlite.org/docs.html) for reference
