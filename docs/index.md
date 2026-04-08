---
icon: lucide/home
title: Get started
---

<style>
.md-content .md-typeset h1 { display: none; }
</style>

<div style="text-align: center; margin: 2rem 0 1.5rem;">
  <div style="display: flex; align-items: center; justify-content: center; gap: 1rem;">
    <img src="assets/images/favicon.svg" alt="Statespace" style="width: 56px; height: 56px;" />
    <span style="font-family: Montserrat, sans-serif; letter-spacing: 0.25em; font-weight: 600; font-size: 2.2em;">STATESPACE</span>
  </div>
  <p style="font-style: italic; font-size: 1.1em; margin-top: 0.75rem; color: var(--md-default-fg-color--light);">Shareable data apps for AI agents
  <div style="margin-top: 1rem; display: flex; gap: 0.2rem; justify-content: center; flex-wrap: wrap;">
    <a href="https://github.com/statespace-tech/statespace/actions/workflows/test.yml"><img src="https://github.com/statespace-tech/statespace/actions/workflows/test.yml/badge.svg" alt="Test Suite" /></a>
    <a href="https://github.com/statespace-tech/statespace/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-007ec6?style=flat-square" alt="License" /></a>
    <a href="https://crates.io/crates/statespace"><img src="https://img.shields.io/crates/v/statespace?style=flat-square" alt="crates.io" /></a>
    <a href="https://discord.gg/rRyM7zkZTf"><img src="https://img.shields.io/discord/1323415085011701870?label=Discord&logo=discord&logoColor=white&color=5865F2&style=flat-square" alt="Discord" /></a>
    <a href="https://x.com/statespace_tech"><img src="https://img.shields.io/badge/Statespace-black?style=flat-square&logo=x&logoColor=white" alt="X" /></a>
  </div>
</div>

---

**Website**: [https://statespace.com](https://statespace.com/)

**Source code**: [https://github.com/statespace-tech/statespace](https://github.com/statespace-tech/statespace)

---

AI doesn't know your data, but it knows Unix and filesystems. Statespace lets you transform your files and CLI tools into shareable data apps that any agent can discover and use. Build database explorers, share business rules, or document legacy APIs. Once you’ve created an app, deploy and monitor it with our [cloud platform](https://statespace.com/).

## Installation

```bash
$ curl -fsSL https://statespace.com/install.sh | bash
```

## Quickstart

### 1. Create it

Run `statespace init` in the current directory:

```bash
$ statespace init
```

### 2. Build it

Add constrained CLI tools to `README.md` or any other Markdown file:

```yaml title="README.md"
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

Alternatively, let your coding agent build it out for you:

```bash
$ claude "Document my database schema and add tools to query it"
```

### 3. Run it

Run your app locally:

```bash
$ statespace run --port 8000
```

Agents and HTTP clients can now read pages and execute tools:

```bash
# Read a page
$ curl http://localhost:8000/README.md

# Execute a CLI tool
$ curl -X POST http://localhost:8000/README.md \
  -H "Content-Type: application/json" \
  -d '{"command": ["grep", "-r", "revenue", "."]}'
```

### 4. Deploy it

Deploy your app to the cloud:

```bash
$ statespace deploy --name demo
```

Your filesystem and CLI tools are now live at a public URL:

```bash
$ curl https://demo.statespace.app/README.md
```

### 5. Share it

Point any agent at the URL directly:

```bash
$ claude "Use the API at https://demo.statespace.app to break down revenue by region"
```

Or wire it up as an MCP server:

```json
"mcpServers": {
  "statespace": {
    "command": "npx",
    "args": ["-y", "statespace-mcp", "https://demo.statespace.app"]
  }
}
```

## Features

- 🔌 **Any CLI tool** — `psql`, `sqlite3`, `grep`, `python` — if it runs in a shell, it works
- 🔒 **Safe by default** — regex constraints mean agents can only run what you explicitly allow
- 🧠 **Self-describing** — Markdown pages are both the documentation and the interface
- 📖 **Composable** — split across pages so agents load only what they need and save tokens
- 🚀 **Shareable** — deploy to a URL, wire up as an MCP server, or share with teammates

## Next steps

- Learn more about [filesystem](pages/develop/filesystem.md) and [CLI tools](pages/develop/cli_tools.md)
- Run your app [locally](pages/deploy/local_development.md) or [deploy to the cloud](pages/deploy/cloud_deployment.md)
- [Secure](pages/deploy/security.md) your apps with token-based authentication
- Connect your agents directly to the [API](pages/connect/api.md) or through an [MCP server](pages/connect/mcp.md)
- Explore Statespace's [commands](pages/reference/cli.md) and [HTTP API](pages/reference/api.md)
