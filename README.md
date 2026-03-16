<p align="center">
  <a href="https://github.com/statespace-tech/statespace">
    <img src="https://raw.githubusercontent.com/statespace-tech/statespace/main/docs/assets/images/favicon.svg" width="150" alt="Statespace">
  </a>
</p>

<div align="center">

# Statespace

**Build interactive web apps for AI agents in Markdown.**

[![Test Suite](https://github.com/statespace-tech/statespace/actions/workflows/test.yml/badge.svg)](https://github.com/statespace-tech/statespace/actions/workflows/test.yml)
[![License](https://img.shields.io/badge/license-MIT-007ec6?style=flat-square)](https://github.com/statespace-tech/statespace/blob/main/LICENSE)
[![Discord](https://img.shields.io/discord/1323415085011701870?label=Discord&logo=discord&logoColor=white&color=5865F2&style=flat-square)](https://discord.gg/rRyM7zkZTf)
[![X](https://img.shields.io/badge/Statespace-black?style=flat-square&logo=x&logoColor=white)](https://x.com/statespace_tech)

</div>

---

**Website: [https://statespace.com](https://statespace.com/)**

**Documentation: [https://docs.statespace.com](https://docs.statespace.com/)**

---

_A declarative framework for building AI-friendly web applications that agents can navigate and interact with._

## Installation

Install the CLI:

```bash
curl -fsSL https://statespace.com/install.sh | bash
```

## Example

### 1. Create it

Write a `README.md` with tools, components, and instructions:

````yaml
---
tools:
  - [sqlite3, store.db, { regex: "^SELECT\\b.*" }]
  - [grep, -r, { }, logs/]
---

# Support Dashboard

Query the database for customer and order data, or search the logs for errors.

**customers** — id, name, email, city, country, joined
**orders** — id, customer_id, product_id, quantity, ordered_at
````

### 2. Add your data

Populate the directory with your app's files:

```text
demo/
├── README.md
├── store.db
└── logs/
    ├── api-gateway.log
    ├── auth-service.log
    └── database.log
```

### 3. Serve it

```bash
statespace serve demo/
```

### 4. Ask it

Pass the URL to your agents:

```bash
claude "Find all database connection failures at http://127.0.0.1:8000"
```

### 5. Deploy it

Create a free [Statespace account](https://statespace.com/auth/login) and deploy your app to the cloud:

```bash
$ statespace deploy demo/
Deployed to https://example.statespace.app
```

### More examples

See the [`examples/`](examples/) directory for ready-to-run apps:

- **[rag](examples/rag)** — Search and analyze log files with `grep`
- **[text_to_sql](examples/text_to_sql)** — Query a SQLite database with natural language
- **[knowledge_base](examples/knowledge_base)** — Navigate a multi-page documentation tree
- **[workflow](examples/workflow)** — Chain API calls to track the ISS
- **[toolkit](examples/toolkit)** — Python scripts for querying Reddit
- **[agent_skill](examples/agent_skill)** — An agent skill for using the Statespace CLI

## Concepts

<details open>
<summary><b>Tools</b> — Give agents controlled access to CLI commands over HTTP.</summary>

```yaml
---
tools:
  - [grep]
  - [curl, -X, GET, { }]
  - [psql, -c, { regex: "^SELECT\\b.*" }]
---
```

</details>

<details>
<summary><b>Components</b> — Render live data inside <code>component</code> code blocks.</summary>

````yaml
```component
echo "Server time: $(date)"
```
````

</details>

<details>
<summary><b>Instructions</b> — Guide agents through your data, workflows, and pages.</summary>

```markdown
# Instructions
- Use grep to search for logs in ./data
- Query the database for recent users
- See [analyze](src/analyze.md) for more workflows
```

</details>

## Features

✅ **Simple** — It's just Markdown. Easy to learn, easy to use, easy to maintain.

⚡ **Lightweight** — Install a single Rust binary. No dependencies.

🌐 **Universal** — Works immediately with any agent that can make HTTP requests.

📦 **Portable** — Deploy apps to the cloud for a public URL, or run them locally.

🔒 **Secure** — Restrict access to your private apps with token-based authentication.

## Community & Contributing

- **Discord**: Join our [community server](https://discord.gg/rRyM7zkZTf) for real-time help and discussions
- **X**: Follow us [@statespace_tech](https://x.com/statespace_tech) for updates and news
- **Issues**: Report bugs or request features on [GitHub Issues](https://github.com/statespace-tech/statespace/issues)

## License

This project is licensed under the terms of the MIT license.
