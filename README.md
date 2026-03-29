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

## Quickstart

If you're a human, point your agent to this repo:

```bash
claude "Help me create an app with Statespace: https://github.com/statespace-tech/statespace"
```

If you're an agent, check out this repo's `AGENTS.md`.


## Install

Install the Statespace CLI with:

```
curl -fsSL https://statespace.com/install.sh | sh
```

## Example

### 1. Create it

Create a new text-to-SQL project:

```
statespace init --from postgresql
````

The skeleton defines just enough tools and instructions for your agent to start exploring your data:

```yaml
---
tools:
  - [psql, -d, $DB, -c, { regex: "^SELECT\\b.*" }]
---

# Instructions
- Learn the schema by exploring tables, columns, and relationships
- Translate the user's question into a query that answers it
```

### 2. Build it

Iterate with your coding agent:

```
claude "Document my database's schema and add a script to summarize them"
```

Your agent will run the app locally and iterate on it until it looks something like this:

```text
.demo/
├── README.md         # from above
├── summarize.py
└── schema/
    ├── users.md
    └── products.md
```

### 3. Ship it

Optionally, deploy your app to the cloud with a free [Statespace account](https://statespace.com/auth/login):

```bash
statespace deploy .demo/
```

Then give your agent the public API URL:

```bash
claude "Use the API at https://demo.statespace.app to find out the number of users"
```

Or wire it up as an MCP server:

```json
"statespace": {
  "command": "uvx",
  "args": ["statespace-mcp", "https://demo.statespace.app"]
}
```
</details>

### Example skeletons

- **[vectorless rag](examples/vectorless_rag)**
- **[postgresql](examples/postgresql)**
- **[pgvector](examples/pgvector)**
- **[mysql](examples/mysql)**
- **[sqlite](examples/sqlite)**
- **[duckdb](examples/duckdb)**
- **[snowflake](examples/snowflake)**
- **[mssql](examples/mssql)**
- **[mongodb](examples/mongodb)**
- **[clickhouse](examples/clickhouse)**
- **[redis](examples/redis)**
- **[elasticsearch](examples/elasticsearch)**
- **[qdrant](examples/qdrant)**
- **[weaviate](examples/weaviate)**

## Community & Contributing

- **Discord**: Join our [community server](https://discord.gg/rRyM7zkZTf) for real-time help and discussions
- **X**: Follow us [@statespace_tech](https://x.com/statespace_tech) for updates and news
- **Issues**: Report bugs or request features on [GitHub Issues](https://github.com/statespace-tech/statespace/issues)

## License

This project is licensed under the terms of the MIT license.
