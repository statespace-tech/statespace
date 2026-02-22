---
icon: lucide/wrench
---

# Tools

Tools are CLI commands that agents can call via [HTTP POST requests](../reference/api.md#post-path).

Every environment includes standard Unix utilities (`ls`, `cat`, `grep`, `sed`, `awk`, `jq`, etc.). To make a command available to agents, declare it in your page's frontmatter. Need something beyond the basics? Add a [Dockerfile](../deploy/cloud.md#dependencies) to install additional packages.

## Overview

List tools in the YAML frontmatter of Markdown pages:

```yaml hl_lines="1-8"
---
tools:
  - [ls]
  - [cat]
  - [grep, -r, "error", "logs/"]
  - [curl, -X, GET, "https://api.com/v1"]
  - [python3, scripts/analyze.py]
---

# Instructions
- Use the provided tools to explore and analyze
```

> **Note**: By default, agents can append additional arguments to tool calls (e.g., `grep --help`).

## Placeholders

Use `{ }` to mark where agents can provide arguments:

```yaml
---
tools:
  - [cat, { }]                      # agent passes file name
  - [grep, -r, { }, logs/]          # agent passes search term
  - [curl, -X, POST, { }, -d, { }]  # agent passes URL and data
---
```

> **Note**: Tools run directly without shell interpretation, preventing command injection attacks.

## Regex constraints

Restrict tool arguments with `{ regex: ... }` patterns:

```yaml
---
tools:
  - [rm, { regex: ".*\.(txt|md|json)$" }]                 # file type restrictions
  - [curl, { regex: "^https://(api\.company\.com)/.+" }]  # URL restrictions
  - [psql, -c, { regex: "^SELECT\b.*" }]                  # SQL safety (read-only)
  - [ls, { regex: "^/home/user/.*" }]                     # path restrictions
  - [git, checkout, { regex: "^[a-z0-9-]+$" }]            # valid branch names
---
```

## Options control

Append `;` to prevent agents from adding extra flags:

```yaml
---
tools:
  - [cat, { }, ;]                                # only allows placeholder argument
  - [curl, -X, GET, https://api.example.com, ;]  # no additional arguments allowed
---
```


## Environment variables

Reference environment `$VARIABLES` to hide secrets from agents and inject them at runtime:

```yaml
---
tools:
  - [curl, -H, "Authorization: Bearer $API_KEY", https://api.example.com]
  - [psql, -U, $DB_USER, -d, $DB_NAME, -c, { }]
---
```