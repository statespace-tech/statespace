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

Statespace lets you deploy and share filesystems and CLI tools HTTP, so any agent can `curl` them directly.

## Installation

```bash
curl -fsSL https://statespace.com/install.sh | bash
```

## Example

### 1. Create it

Initialize a project from a template in the current directory:

```bash
statespace init --template postgresql
```

Each Markdown page in your app can expose CLI tools over HTTP.

```yaml
---
tools:
  - [psql, -d, $DATABASE_URL, -c, { regex: "^(SELECT|SHOW|EXPLAIN)\\b.*" }, ;]
---

# Orders
- `order_id` — primary key
- `customer_id` — foreign key to customers
- `status` — one of `pending`, `fulfilled`, `cancelled`
- Revenue is `quantity * unit_price`, excluding cancelled orders
```

Any CLI tool works — `psql`, `curl`, `grep`, `python`, `gh`. The regex constraint means agents can only run what you allow.

### 2. Run it

Start the app locally:

```bash
statespace run my-app/ --port 8000
```

Any agent (or HTTP client) can now read pages and execute tools directly:

```bash
# Read a page
curl http://localhost:8000/schema/orders.md

# Execute a CLI tool
curl -X POST http://localhost:8000/schema/orders.md \
  -H "Content-Type: application/json" \
  -d '{"command": ["psql", "-d", "$DATABASE_URL", "-c", "SELECT * FROM orders LIMIT 5"]}'
```

### 3. Build it

Tell your coding agent what you want to share:

```bash
claude "Help me document my database's schema, business rules, and context"
```

Your agent will build out the filesystem and tools based on what tell it:

```text
my-app/
├── README.md
├── schema/
│   ├── orders.md
│   ├── customers.md
│   └── products.md
├── reports/
│   ├── monthly.md
│   └── churn.md
└── queries/
    └── funnel.sql
```

### 4. Deploy it

Deploy to the cloud with a free [Statespace account](https://statespace.com/auth/login):

```bash
statespace deploy my-app/
```

Your filesystem and CLI tools are now live at a public URL:

```bash
curl https://my-app.statespace.app/schema/orders.md
```

### 5. Share it

Point any agent at the URL directly:

```bash
claude "Use the API at https://my-app.statespace.app to break down revenue by region"
```

Or wire it up as an MCP server:

```json
"mcpServers": {
  "statespace": {
    "command": "npx",
    "args": ["-y", "statespace-mcp", "https://my-app.statespace.app"]
  }
}
```

## Features

- 🔌 **Pluggable** — works with virtually any CLI or SDK, including databases, search backends, and observability tools
- 🔒 **Safe** — tool constraints like regex mean agents can never run destructive queries
- 🧠 **Self-describing** — APIs are both the documentation and the interface for your databases
- 📖 **Composable** — split your app across pages so agents load only what they need and save tokens
- 🚀 **Shareable** — publish your API to a URL, wire it up as an MCP server, or share with teammates

## Community & Contributing

- **Discord**: Join our [community server](https://discord.gg/rRyM7zkZTf) for real-time help and discussions
- **X**: Follow us [@statespace_tech](https://x.com/statespace_tech) for updates and news
- **Issues**: Report bugs or request features on [GitHub Issues](https://github.com/statespace-tech/statespace/issues)

## License

This project is licensed under the terms of the MIT license.
