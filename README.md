<br>

<div align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/statespace-tech/statespace/main/docs/assets/images/header_light.png" />
    <img src="https://raw.githubusercontent.com/statespace-tech/statespace/main/docs/assets/images/header_dark.png" alt="Statespace" width="375" />
  </picture>
</div>

<div align="center">

<br>

*Self-documenting AI applications*

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

AI doesn't know your data. Statespace helps you build self-documenting data applications that describe themselves to agents. Build RAG, text-to-SQL, and knowledge bases that agents can maintain and improve on their own. Once you’ve created an app, you can deploy, manage, and share it from our [cloud platform](https://statespace.com/).

System that writes and maintains its own context.

## Install

```bash
curl -fsSL https://statespace.com/install.sh | sh
```

## Quickstart

**If you're a human**: point your agent at this repo:

```bash
claude "Help me build a Statespace app: https://github.com/statespace-tech/statespace"
```

**If you're an agent**: install the CLI and read the agent guide:

```bash
curl -fsSL https://statespace.com/install.sh | sh && statespace guide
```

## Example

### 1. Create it

Initialize a new project in the current directory:

```bash
statespace init --template postgresql
```

The template defines just enough tools and instructions for your agent to start exploring your data:

```yaml
---
tools:
  - [psql, -d, $DATABASE_URL, -c, { regex: "^(SELECT|SHOW|EXPLAIN)\\b.*" }]
---

# Instructions
- Explore the schema with `SELECT table_name FROM information_schema.tables WHERE table_schema = 'public'`
- Inspect columns with `SELECT column_name, data_type FROM information_schema.columns WHERE table_name = '<table>'`
- See [PostgreSQL documentation](https://www.postgresql.org/docs/) for reference
```

### 2. Build it

Iterate with your coding agent:

```bash
claude "Document my database's schema and add summarize script"
```

Your agent will run the app locally and iterate on it until it looks something like this:

```text
my-app/
├── README.md
├── summarize.py
└── schema/
    ├── users.md
    └── products.md
```

### 3. Ship it

Optionally, deploy your app to the cloud with a free [Statespace account](https://statespace.com/auth/login):

```bash
statespace deploy my-app/
```

Then give your agent the public API URL:

```bash
claude "Use the API at https://my-app.statespace.app to find out the number of users"
```

Or wire it up as an MCP server:

```json
"statespace": {
  "command": "uvx",
  "args": ["statespace-mcp", "https://my-app.statespace.app"]
}
```

### App templates

- **[vectorless rag](crates/statespace-templates/app/vectorless_rag)**
- **[postgresql](crates/statespace-templates/app/postgresql)**
- **[pgvector](crates/statespace-templates/app/pgvector)**
- **[mysql](crates/statespace-templates/app/mysql)**
- **[sqlite](crates/statespace-templates/app/sqlite)**
- **[duckdb](crates/statespace-templates/app/duckdb)**
- **[snowflake](crates/statespace-templates/app/snowflake)**
- **[mssql](crates/statespace-templates/app/mssql)**
- **[mongodb](crates/statespace-templates/app/mongodb)**
- **[clickhouse](crates/statespace-templates/app/clickhouse)**
- **[redis](crates/statespace-templates/app/redis)**
- **[elasticsearch](crates/statespace-templates/app/elasticsearch)**
- **[qdrant](crates/statespace-templates/app/qdrant)**
- **[weaviate](crates/statespace-templates/app/weaviate)**

## Community & Contributing

- **Discord**: Join our [community server](https://discord.gg/rRyM7zkZTf) for real-time help and discussions
- **X**: Follow us [@statespace_tech](https://x.com/statespace_tech) for updates and news
- **Issues**: Report bugs or request features on [GitHub Issues](https://github.com/statespace-tech/statespace/issues)

## License

This project is licensed under the terms of the MIT license.
