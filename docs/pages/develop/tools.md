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

Tools run without shell interpretation, so placeholders are safe from command injections:

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
CORRECT:    {"command": ["cat", "note.txt", "logs.csv"]}     ← extra arguments are fine
INCORRECT:  {"command": ["cat", "note.py"]}                  ← doesn't match pattern
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

## Environment variables

Reference environment `$VARIABLES` in your tools to hide secrets from agents:

```yaml
---
tools:
  - [psql, -U, $USER, -d, $DB, -c, { }]
---
```

You can set secrets for a deployed app with the [CLI](../reference/cli.md):

```console
$ statespace secrets set --app <APP> USER=admin DB=mydb
```

Or pass them in the request body of [tool calls](../reference/api.md#post):

```bash
curl -X POST \
  -H "Content-Type: application/json" \
  "https://example.statespace.app/page.md" \
  -d '{
    "command": ["psql", "-U", "$USER", "-d", "$DB", "-c", "SELECT * FROM users"],
    "env": {"USER": "admin", "DB": "mydb"}
  }'
```
