# Statespace Application Instructions

This web application exposes content and tools over HTTP. Follow these instructions exactly.

## Quick Start

1. **GET `/README.md`** — discover what this application does, its tools, and where to navigate.
2. **Follow links** — GET any path to read content (Markdown, data files, etc.).
3. **Execute tools** — POST to `/` with `{"command": ["tool-name", "arg1", "arg2"]}`.

## Tools

Tools are declared in YAML frontmatter on Markdown files:

```yaml
---
tools:
  - [grep, -r, -i, { }, ../data/]
  - [cat, { regex: ".*\\.txt$" }]
  - [ls]
---
```

Execute any declared tool by POSTing `{"command": [...]}` to `/`. Commands run without a shell — each array element becomes a process argument directly (no expansion, pipes, or globbing).

### Rules

**Extra arguments are allowed by default.** You can append additional flags after the defined elements.

```text
Tool:    [ls]
CORRECT: {"command": ["ls", "-la"]}
CORRECT: {"command": ["ls", "--color", "-h"]}
```

**`{ }` accepts exactly one argument:**

```text
Tool:    [ls, { }]
CORRECT: {"command": ["ls", "src"]}
WRONG:   {"command": ["ls"]}                ← missing argument
WRONG:   {"command": ["ls", "src", "lib"]}  ← too many arguments
```

**`{ regex: "pattern" }` accepts one argument matching the pattern:**

```text
Tool:    [cat, { regex: ".*\\.txt$" }]
CORRECT: {"command": ["cat", "notes.txt"]}
WRONG:   {"command": ["cat", "notes.py"]}   ← doesn't match
```

**Fixed elements are immutable.** Only replace placeholders — never modify, remove, or add to fixed elements.

```text
Tool:    [grep, -r, -i, { }, ../data/]
CORRECT: {"command": ["grep", "-r", "-i", "error", "../data/"]}
CORRECT: {"command": ["grep", "-r", "-i", "error", "../data/", "-l"]}    ← extra flag is fine
WRONG:   {"command": ["grep", "-r", "-i", "error", "../data/file.txt"]}  ← changed fixed path
WRONG:   {"command": ["grep", "-r", "error", "../data/"]}                ← removed fixed flag
```

**Trailing `;` locks the argument list.** The command accepts only what is defined.

```text
Tool:    [rm, { }, ;]
CORRECT: {"command": ["rm", "file.txt"]}
WRONG:   {"command": ["rm", "-f", "file.txt"]}  ← no extra arguments allowed
```

**Write environment variables literally** — the server expands them at execution time.

```text
Tool:    [psql, $DATABASE_URL, -c, { }]
CORRECT: {"command": ["psql", "$DATABASE_URL", "-c", "SELECT 1"]}
WRONG:   {"command": ["psql", "postgres://localhost/mydb", "-c", "SELECT 1"]}  ← substituted value
```

## Constraints

- Only declared tools can be executed.
- Commands run relative to the app's root directory.
- All interaction is over HTTP.
