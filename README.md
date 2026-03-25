<br>

<div align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/statespace-tech/statespace/main/docs/assets/images/header_light.png" />
    <img src="https://raw.githubusercontent.com/statespace-tech/statespace/main/docs/assets/images/header_dark.png" alt="Statespace" width="375" />
  </picture>
</div>

<div align="center">

<br>

*Build web APIs that agents can directly interact with.*

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

Statespace is a Markdown framework for building web APIs that agents can directly connect to and interact with.

## Installation

Install the CLI:

```bash
curl -fsSL https://statespace.com/install.sh | bash
```

## Example

### 1. Create it

Create a file `README.md` with:

````yaml
---
tools:
    - [date]
---

```component
echo "Hello, world!"
```

This is an example application.
````

### 2. Run it

```bash
statespace serve .
```

### 3. Ask it

Pass the URL to your agents:

```bash
claude "What can I do with the API at http://127.0.0.1:8000?"
```

### 4. Update it

Add files to your app directory:

```text
app/
├── README.md
├── script.py
└── data/
    ├── notes.txt
    └── logs.txt
```

Then update `README.md` to add tools and instructions:

````yaml
---
tools:
    - [date]
    - [python3, script.py]
    - [grep, -r, -i, { }, ./data/]
---

```component
echo "Hello, world!"
```

This is an example app.

## Instructions
- Run the python script
- Use grep to search through files
````

### 5. Deploy it

Optionally, create a free [Statespace account](https://statespace.com/auth/login) and deploy your app to the cloud:

```bash
statespace deploy . --public
```

## Concepts

<details open>
<summary><b>Tools</b> — Give agents controlled access to CLI commands over HTTP.</summary>

```yaml
---
tools:
    - [date]
    - [python3, script.py]
    - [grep, -r, -i, { }, ./data/]
---
```

</details>

<details>
<summary><b>Components</b> — Render live data inside pages with <code>component</code> code blocks.</summary>

````yaml
```component
echo "Hello, world!"
```
````

</details>

<details>
<summary><b>Instructions</b> — Guide agents through your data, workflows, and pages.</summary>

```markdown
## Instructions
- Run the python script
- Use grep to search through files
```

</details>

## Use cases

- **[rag](examples/rag)** — Search and analyze log files with `grep`
- **[knowledge_base](examples/knowledge_base)** — Navigate a multi-page documentation tree
- **[text_to_sql](examples/text_to_sql)** — Query a SQLite database with natural language
- **[workflow](examples/workflow)** — Chain API calls to track the ISS and its trajectory
- **[agent_skill](examples/agent_skill)** — An agent skill for using the Statespace CLI
- **[toolkit](examples/toolkit)** — Python scripts for querying Reddit

## Features

✅ **Simple** — It's just Markdown. Easy to learn, easy to use, easy to maintain.

⚡ **Lightweight** — Install a single, lightning-fast Rust binary. No dependencies.

🌐 **Universal** — Works directly with any agent that can make HTTP requests.

📦 **Portable** — Run or deploy your apps with a single CLI command.

🔒 **Secure** — Restrict access to your private apps with token-based authentication.

## Community & Contributing

- **Discord**: Join our [community server](https://discord.gg/rRyM7zkZTf) for real-time help and discussions
- **X**: Follow us [@statespace_tech](https://x.com/statespace_tech) for updates and news
- **Issues**: Report bugs or request features on [GitHub Issues](https://github.com/statespace-tech/statespace/issues)

## License

This project is licensed under the terms of the MIT license.
