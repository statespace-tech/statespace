<br>

<div align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/statespace-tech/statespace/main/docs/assets/images/header_light.png" />
    <img src="https://raw.githubusercontent.com/statespace-tech/statespace/main/docs/assets/images/header_dark.png" alt="Statespace" width="375" />
  </picture>
</div>

<div align="center">

<br>

*`curl` your filesystem and CLI tools*

[![Test Suite](https://github.com/statespace-tech/statespace/actions/workflows/test.yml/badge.svg)](https://github.com/statespace-tech/statespace/actions/workflows/test.yml)
[![License](https://img.shields.io/badge/license-MIT-007ec6?style=flat-square)](https://github.com/statespace-tech/statespace/blob/main/LICENSE)
[![crates.io](https://img.shields.io/crates/v/statespace?style=flat-square)](https://crates.io/crates/statespace)
[![Discord](https://img.shields.io/discord/1323415085011701870?label=Discord&logo=discord&logoColor=white&color=5865F2&style=flat-square)](https://discord.gg/rRyM7zkZTf)
[![X](https://img.shields.io/badge/Statespace-black?style=flat-square&logo=x&logoColor=white)](https://x.com/statespace_tech)

</div>

---

**Website: [https://statespace.com](https://statespace.com/)**

**Documentation: [https://docs.statespace.com](https://docs.statespace.com/)**

---

Agents were trained on Unix and filesystems, not your APIs and schemas. Statespace serves your files and CLI tools over HTTP, so agents can discover and run them with nothing but `curl`.

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

Add a `tools` block to `README.md` or any other Markdown file:

```yaml
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
$ statespace serve --port 8000
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

## Community & Contributing

- **Discord**: Join our [community server](https://discord.gg/rRyM7zkZTf) for real-time help and discussions
- **X**: Follow us [@statespace_tech](https://x.com/statespace_tech) for updates and news
- **Issues**: Report bugs or request features on [GitHub Issues](https://github.com/statespace-tech/statespace/issues)

## License

This project is licensed under the terms of the MIT license.
