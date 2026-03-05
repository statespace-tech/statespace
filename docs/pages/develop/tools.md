---
icon: lucide/wrench
---

# Tools

Tools are CLI commands that agents can call with the [REST API](../reference/api.md#post-path).

## Syntax

List tools in the YAML frontmatter of Markdown pages:

```yaml title="page.md" hl_lines="1-6"
---
tools:
  - [grep]
  - [curl, -X, GET, { }]
  - [psql, -c, { regex: "^SELECT\\b.*" }]
---

# Instructions
- Use the provided tools
```

By default, agents can append additional arguments to tool calls:

```bash
Tool:    [grep]
CORRECT: {"command": ["grep", "--help"]}
CORRECT: {"command": ["grep", "-r", "error", "logs/"]}
```

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

Tools run without shell interpretation, so placeholders are safe from command injections:

```bash
Tool:    [cat, { }]
CORRECT: {"command": ["cat", "data.txt"]}
ERROR:   {"command": ["cat", "data.txt; rm -rf /"]}   ← treated as a literal filename
```

## Regex constraints

Restrict tool arguments with `{ regex: ... }` patterns:

```yaml
---
tools:
  - [rm, { regex: ".*\\.(txt|md|json)$" }]                  # file type restrictions
  - [curl, { regex: "^https://(api\\.company\\.com)/.+" }]  # URL restrictions
  - [psql, -c, { regex: "^SELECT\\b.*" }]                   # SQL safety (read-only)
  - [ls, { regex: "^/home/user/.*" }]                       # path restrictions
  - [git, checkout, { regex: "^[a-z0-9-]+$" }]              # valid branch names
---
```

## Options control

Append `;` to prevent agents from adding extra flags and arguments:

```yaml
---
tools:
  - [cat, { }, ;]                                # only allows placeholder argument
  - [curl, -X, GET, https://api.example.com, ;]  # no additional arguments allowed
  - [python3, scripts/report.py, { }, ;]         # agent passes one arg, nothing else
---
```

## Environment variables

Reference environment `$VARIABLES` in your tools to hide secrets from agents:

```yaml
---
tools:
  - [curl, -H, "Authorization: Bearer $API_KEY", https://api.example.com]
  - [psql, -U, $DB_USER, -d, $DB_NAME, -c, { }]
  - [python3, scripts/upload.py, --token, $UPLOAD_TOKEN, { }]
---
```

!!! tip
    You can set your app's environment variables with the [CLI](../reference/cli.md), or inject them at runtime through the [REST API](../reference/api.md#post-path).