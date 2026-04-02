<br>

<div align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/statespace-tech/statespace/main/docs/assets/images/header_light.png" />
    <img src="https://raw.githubusercontent.com/statespace-tech/statespace/main/docs/assets/images/header_dark.png" alt="Statespace" width="375" />
  </picture>
</div>

<div align="center">

<br>

*Database APIs for AI Agents*

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

Databases are a mess: schema names don't make sense, foreign keys are missing, and business context lives everywhere.
Statespace lets you quickly turn that domain knowledge into APIs that AI agents can use to understand and query your databases.
Once you’ve created an app, you can deploy, monitor, and share it with our [cloud platform](https://statespace.com/).

## Quickstart

Install the CLI: 

```bash
curl -fsSL https://statespace.com/install.sh | sh
```

Then, pass the Statespace guide to your coding agent:

```bash
statespace guide | claude
```

## Example

### 1. Create it

Initialize a new project in the current directory:

```bash
statespace init --template postgresql
```

Templates define just enough tools and instructions for your agent to start exploring your data:

```yaml
---
tools:
  - [psql, -d, $DATABASE_URL, -c, { regex: "^(SELECT|SHOW|EXPLAIN)\\b.*" }, ;]
---

# Instructions
- Explore the schema to understand the data model
- Follow the user's instructions and answer their questions
- Reference [documentation](https://www.postgresql.org/docs/) as needed
```

### 2. Build it

Work with your coding agent to document your schema, rules, and context: 

```bash
claude "Document my database's schema and add summarize script"
```

Your agent will run your app locally and iterate on it until it looks something like this:

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

Then give other agents the API URL:

```bash
claude "Use the API at https://my-app.statespace.app to find out the number of users"
```

Or wire it up as an MCP server:

```json
"statespace": {
  "command": "npx",
  "args": ["-y", "statespace-mcp", "https://my-app.statespace.app"]
}
```

## Community & Contributing

- **Discord**: Join our [community server](https://discord.gg/rRyM7zkZTf) for real-time help and discussions
- **X**: Follow us [@statespace_tech](https://x.com/statespace_tech) for updates and news
- **Issues**: Report bugs or request features on [GitHub Issues](https://github.com/statespace-tech/statespace/issues)

## License

This project is licensed under the terms of the MIT license.
