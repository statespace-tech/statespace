---
icon: lucide/terminal
---

# CLI tools

CLI tools let agents interact with your filesystem over HTTP.

## Overview

List tools in the YAML frontmatter of any Markdown page:

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

Agents invoke them via `POST`:

```bash
$ curl -X POST http://localhost:8000/README.md \
  -H "Content-Type: application/json" \
  -d '{"command": ["grep", "-r", "revenue", "."]}'
```

By default, agents can append additional arguments to tool calls:

```bash
Tool:    [ls]
CORRECT: {"command": ["ls", "--help"]}
CORRECT: {"command": ["ls", "-la", "."]}
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

Each placeholder accepts exactly one argument:

```bash
Tool:       [ls, { }]
CORRECT:    {"command": ["ls", "src"]}
CORRECT:    {"command": ["ls", "src", "lib"]}  ← extra arguments are fine
INCORRECT:  {"command": ["ls"]}                ← missing argument
```

Tools run without shell interpretation, so placeholders are safe from command injection:

```bash
Tool:       [cat, { }]
CORRECT:    {"command": ["cat", "data.txt"]}
INCORRECT:  {"command": ["cat", "data.txt; rm -rf /"]}   ← treated as literal filename
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

Arguments that don't match the pattern are rejected:

```bash
Tool:       [cat, { regex: ".*\\.txt$" }]
CORRECT:    {"command": ["cat", "note.txt"]}
INCORRECT:  {"command": ["cat", "note.py"]}   ← doesn't match pattern
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

Only the defined arguments are accepted — extra flags are rejected:

```bash
Tool:       [rm, { }, ;]
CORRECT:    {"command": ["rm", "file.txt"]}
INCORRECT:  {"command": ["rm", "-f", "file.txt"]}  ← no extra arguments allowed
```

## Components

Components are shell commands that run on every `GET` and render their output inline:

````markdown title="page.md"
# Dashboard

```component
psql -c "SELECT COUNT(*) FROM orders"
```
````

When a page loads, components are replaced with their output:

```markdown title="page.md"
# Dashboard

42
```

## Environment variables

Reference environment `$VARIABLES` in tools and components to hide secrets from agents:

````markdown title="page.md"
---
tools:
  - [psql, -U, $USER, -d, $DB, -c, { }]
---

```component
echo "Connected as: $USER"
```

# This is an example
````

[Configure them](../deploy/security.md#secrets) when serving or deploying apps:

```bash
$ statespace {serve,deploy} --env USER=admin --env DB=mydb
$ statespace {serve,deploy} --env-file .env
```

For tools, you can also pass them directly in the request body of [`POST` requests](../reference/api.md#post):

```bash
$ curl -X POST \
  -H "Content-Type: application/json" \
  "https://demo.statespace.app/page.md" \
  -d '{
    "command": ["psql", "-U", "$USER", "-d", "$DB", "-c", "SELECT * FROM users"],
    "env": {"USER": "admin", "DB": "mydb"}
  }'
```

And for components, you can pass them as query parameters in [`GET` requests](../reference/api.md#get):

```bash
$ curl "https://demo.statespace.app/page.md?USER=admin"
```
