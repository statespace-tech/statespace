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
  <p style="font-style: italic; font-size: 1.1em; margin-top: 0.75rem; color: var(--md-default-fg-color--light);">Database APIs for AI Agents</p>
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

Databases are a mess: schema names don't make sense, foreign keys are missing, and business context lives everywhere.
Statespace lets you and your coding agent quickly turn that domain knowledge into APIs that any AI agent can query.
Once you've created an API, you can deploy and monitor it with our [cloud platform](https://statespace.com/).

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

Templates give your coding agent the tools and guardrails it needs to start exploring your data:

```yaml title="README.md"
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

Tell your coding agent what you know about your data:

```bash
claude "Help me document my database's schema, business rules, and context"
```

Your agent will build, run, and test your API locally based on what you share:

```text
my-app/
├── README.md
├── schema/
│   ├── orders.md
│   ├── customers.md
│   └── products.md
├── reports/
│   ├── revenue/
│   │   ├── monthly.md
│   │   └── by_region.md
│   ├── churn.md
│   └── summarize.py
├── queries/
│   └── funnel.sql
└── data/
    ├── metrics.csv
    └── segments.csv
```

### 3. Ship it

Deploy your API to the cloud with a free [Statespace account](https://statespace.com/auth/login):

```bash
statespace deploy my-app/
```

Then share the API URL with other agents:

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

- 🔌 **Pluggable** — works with virtually any database that has a CLI or SDK
- 🔒 **Safe** — tool constraints like regex mean agents can never run destructive queries
- 🧠 **Self-describing** — APIs are both the documentation and the interface for your databases
- 📖 **Composable** — split your app across pages so agents load only what they need and save tokens
- 🚀 **Shareable** — publish your API to a URL, wire it up as an MCP server, or share with teammates

## Use cases

<div class="grid cards" markdown style="grid-template-columns: repeat(3, 1fr);">

-   :lucide-database:{ .md .middle .jade } &nbsp; **Text-to-SQL**

    ---

    Query a database with natural language.


-   :lucide-file-stack:{ .md .middle .jade } &nbsp; **RAG**

    ---

    Search and analyze files with `grep`.

-   :lucide-library:{ .md .middle .jade } &nbsp; **Knowledge bases**

    ---

    Navigate a multi-page documentation tree.

-   :lucide-workflow:{ .md .middle .jade } &nbsp; **AI Workflows**

    ---

    Chain API calls to build complex workflows.

-   :lucide-sprout:{ .md .middle .jade } &nbsp; **Agent skills**

    ---

    An agent skill for using the Statespace CLI.

-   :lucide-toolbox:{ .md .middle .jade } &nbsp; **Toolkits**

    ---

    Python scripts for querying Reddit.

</div>
