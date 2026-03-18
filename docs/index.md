---
icon: lucide/home
title: Get started
---

<style>.md-content__inner > h1 { display: none; }</style>


<div style="text-align: center; margin: 2rem 0 1.5rem;">
  <div style="display: flex; align-items: center; justify-content: center; gap: 0.75rem;">
    <img src="assets/images/favicon.svg" alt="Statespace" style="width: 56px; height: 56px;" />
    <span style="font-family: Montserrat, sans-serif; letter-spacing: 0.25em; font-weight: 600; font-size: 2.2em;">STATESPACE</span>
  </div>
  <p style="font-style: italic; font-size: 1.1em; margin-top: 0.75rem; color: var(--md-default-fg-color--light);">Build APIs that agents can directly interact with.</p>
  <div style="margin-top: 1rem; display: flex; gap: 0.4rem; justify-content: center; flex-wrap: wrap;">
    <a href="https://github.com/statespace-tech/statespace/actions/workflows/test.yml"><img src="https://github.com/statespace-tech/statespace/actions/workflows/test.yml/badge.svg" alt="Test Suite" /></a>
    <a href="https://github.com/statespace-tech/statespace/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-007ec6?style=flat-square" alt="License" /></a>
    <a href="https://discord.gg/rRyM7zkZTf"><img src="https://img.shields.io/discord/1323415085011701870?label=Discord&logo=discord&logoColor=white&color=5865F2&style=flat-square" alt="Discord" /></a>
    <a href="https://x.com/statespace_tech"><img src="https://img.shields.io/badge/Statespace-black?style=flat-square&logo=x&logoColor=white" alt="X" /></a>
  </div>
</div>

---

**Website**: [https://statespace.com](https://statespace.com/)

**Source code**: [https://github.com/statespace-tech/statespace](https://github.com/statespace-tech/statespace)

---

Statespace is a declarative, Markdown-based web framework for building APIs that AI agents can directly interact with.
Build apps for RAG, knowledge bases, text-to-SQL, and more.



## Example

The following example is live at [https://hello-world.statespace.app](https://hello-world.statespace.app).

### 1. Create it

Create a file `README.md` with:

````yaml title="README.md"
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

Run the server with:

```bash
statespace serve .
```

### 3. Ask it

Pass the URL to your agents:

=== ":simple-claude: &nbsp; Claude Code"

    ```bash
    claude "What can I do with the API at http://127.0.0.1:8000?"
    ```

=== ":simple-cursor: &nbsp; Cursor"

    ```bash
    agent "What can I do with the API at http://127.0.0.1:8000?"
    ```

=== ":simple-githubcopilot: &nbsp; GitHub Copilot"

    ```bash
    copilot -p "What can I do with the API at http://127.0.0.1:8000?"
    ```

### 4. Update it

Add files to your application:

```text
app/
├── README.md
├── script.py
└── data/
    ├── notes.txt
    └── logs.txt
```

Then update `README.md` with tools and instructions for using them:

````yaml title="README.md" hl_lines="4-5 14-16"
---
tools:
  - [date]
  - [grep, -r, -i, { }, ./data/]
  - [python3, script.py]
---

```component
echo "Hello, world!"
```

This is an example API.

## Instructions
- Use grep to search through files in ./data/
- Run python3 script.py
````

### 5. Deploy it

Optionally, create a free [Statespace account](https://statespace.com/auth/login) and deploy your app to the cloud:

```bash
statespace deploy . --public
```

## Concepts

=== ":lucide-wrench: &nbsp; Tools"
    
    Give agents controlled access to CLI commands over HTTP. [Learn more](pages/develop/tools.md)

    ````yaml title="example.md" hl_lines="1-6"
    ---
    tools:
      - [grep]
      - [curl, -X, GET, { }]
      - [psql, -c, { regex: "^SELECT\\b.*" }]
    ---

    ```component
    echo "Server time: $(date)"
    ```

    # Instructions
    - Use grep to search for logs in ./data
    - Query the database for recent users
    - See [analyze](src/analyze.md) for more workflows
    ````

=== ":lucide-sparkles: &nbsp; Components"

    Render live data inside pages with `component` code blocks. [Learn more](pages/develop/components.md)

    ````yaml title="example.md" hl_lines="8-10"
    ---
    tools:
      - [grep]
      - [curl, -X, GET, { }]
      - [psql, -c, { regex: "^SELECT\\b.*" }]
    ---

    ```component
    echo "Server time: $(date)"
    ```

    # Instructions
    - Use grep to search for logs in ./data
    - Query the database for recent users
    - See [analyze](src/analyze.md) for more workflows
    ````

=== ":lucide-file-text: &nbsp; Instructions"

    Guide agents through your data, workflows, and pages. [Learn more](pages/develop/instructions.md)

    ````yaml title="example.md" hl_lines="12-15"
    ---
    tools:
      - [grep]
      - [curl, -X, GET, { }]
      - [psql, -c, { regex: "^SELECT\\b.*" }]
    ---

    ```component
    echo "Server time: $(date)"
    ```

    # Instructions
    - Use grep to search for logs in ./data
    - Query the database for recent users
    - See [analyze](src/analyze.md) for more workflows
    ````

## Features

**Simple** - It's just Markdown. Easy to learn, easy to use, easy to maintain.

**Lightweight** - [Install](install.md) a single, lightning-fast Rust binary. No dependencies.

**Universal** - Works directly with [any agent](pages/connect/agents.md) that can make HTTP requests.

**Portable** - [Run](pages/deploy/local_development.md) or [deploy](pages/deploy/cloud_deployment.md) your apps with a single CLI command.

**Secure** - Restrict access to private apps with [token-based authentication](pages/deploy/security.md).

## Use cases

<div class="grid cards" markdown style="grid-template-columns: repeat(3, 1fr);">

-   :lucide-file-stack:{ .md .middle .jade } &nbsp; **RAG**

    ---

    Search and analyze log files with `grep`.

    [See example](https://github.com/statespace-tech/statespace/tree/main/examples/rag)

-   :lucide-library:{ .md .middle .jade } &nbsp; **Knowledge bases**

    ---

    Navigate a multi-page documentation tree.

    [See example](https://github.com/statespace-tech/statespace/tree/main/examples/knowledge_base)

-   :lucide-sprout:{ .md .middle .jade } &nbsp; **Agent skills**

    ---

    An agent skill for using the Statespace CLI.

    [See example](https://github.com/statespace-tech/statespace/tree/main/examples/agent_skill)

-   :lucide-database:{ .md .middle .jade } &nbsp; **Text-to-SQL**

    ---

    Query a SQLite database with natural language.

    [See example](https://github.com/statespace-tech/statespace/tree/main/examples/text_to_sql)

-   :lucide-workflow:{ .md .middle .jade } &nbsp; **AI Workflows**

    ---

    Chain API calls to track the ISS and its trajectory.

    [See example](https://github.com/statespace-tech/statespace/tree/main/examples/workflow)

-   :lucide-toolbox:{ .md .middle .jade } &nbsp; **Toolkit**

    ---

    Python scripts for querying Reddit.

    [See example](https://github.com/statespace-tech/statespace/tree/main/examples/toolkit)

</div>

