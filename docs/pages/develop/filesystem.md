---
icon: lucide/folder
---

# Filesystem

Statespace serves files over HTTP so agents can read and search with `curl`.

## Overview

Run `statespace init` to scaffold a new app in any directory:

```bash
$ statespace init
```

This creates the following files:

- **`README.md`** — the main page of your app. Add general tools, instructions, and links here.
- **`AGENTS.md`** — instructions that teach coding agents how to build and serve on the app.
- **`CLAUDE.md`** — same content as `AGENTS.md`, picked up automatically by Claude Code.
- **`API.md`** — HTTP contract instructions for the agent. Served at the root URL (`/`).
- **`.gitignore`** — pre-configured to exclude secrets and build artifacts.

Agents can read any file with a plain `GET`:

```bash
$ curl http://localhost:8000/README.md
```

## Markdown pages

Markdown pages can declare [CLI tools](cli_tools.md) that are callable over HTTP.

```yaml title="schema/orders.md"
---
tools:
  - [grep]
  - [python, scripts/summarize.py]
  - [sqlite3, data/app.db, { regex: "^(SELECT|EXPLAIN)\\b.*" }]
---

# Instructions
- Only run read-only queries against the database
- Use `summarize.py` for aggregations and report generation
- Use `grep` to search across local files and logs
```

## Multi-page apps

Split your context and tools across pages so agents load only what they need:

```markdown title="README.md"
...
- See [reports](pages/revenue.md) for reporting tools
```

Agents follow links progressively, keeping their context window small:

```text
app/
├── README.md               # start here
├── pages/
│   └── revenue.md          # add more tools here
├── scripts/
│   └── summarize.py
└── data/
    └── app.db
```
