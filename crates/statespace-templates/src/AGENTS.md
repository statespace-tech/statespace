# App instructions

This Statespace web application exposes content and tools over HTTP. Follow these instructions exactly.

## Quick start

1. **GET `/README.md`** — discover what this application does, root-level tools, and where to navigate.
2. **Follow links** — GET any path to read content (Markdown, data files, etc.).
3. **Execute tools** — POST to the page where the tool is declared with `{"command": ["tool-name", "arg1", "arg2"]}`.

## Tools

Tools are declared in YAML frontmatter of Markdown pages:

```markdown
---
tools:
  - [ls]
  - [grep, -r, -i, { }, ../data/]
  - [cat, { regex: ".*\\.txt$" }]
---

# My Markdown page
...
```

To execute a tool, POST `{"command": [...]}` to the path of the page that declares it.
For example, if `/page.md` declares `[grep]`, POST to `/page.md`.
Commands run without a shell — each array element becomes a process argument directly (no expansion, pipes, or globbing).

### Rules

**Extra arguments are allowed by default.** You can append additional flags after the defined elements.

```text
Tool:       [ls]
CORRECT:    {"command": ["ls", "."]}
CORRECT:    {"command": ["ls", "--help"]}
CORRECT:    {"command": ["ls", "-la", "."]}
```

**`{ }` accepts exactly one argument:**

```text
Tool:       [ls, { }]
CORRECT:    {"command": ["ls", "src"]}
CORRECT:    {"command": ["ls", "src", "lib"]}  ← extra arguments are fine
INCORRECT:  {"command": ["ls"]}                ← missing argument
```

**`{ regex: "pattern" }` accepts one argument matching the pattern:**

```text
Tool:       [cat, { regex: ".*\\.txt$" }]
CORRECT:    {"command": ["cat", "notes.txt"]}
CORRECT:    {"command": ["cat", "notes.txt", "logs.csv"]}     ← extra arguments are fine
INCORRECT:  {"command": ["cat", "notes.py"]}                  ← doesn't match regex pattern
```

**Fixed elements are immutable.** Only replace placeholders — never modify, remove, or add to fixed elements.

```text
Tool:       [grep, -r, -i, { }, ../data/]
CORRECT:    {"command": ["grep", "-r", "-i", "error", "../data/"]}
CORRECT:    {"command": ["grep", "-r", "-i", "error", "../data/", "-l"]}    ← extra arguments are fine
INCORRECT:  {"command": ["grep", "-r", "-i", "error", "../data/file.txt"]}  ← changed fixed path
INCORRECT:  {"command": ["grep", "-r", "error", "../data/"]}                ← removed fixed flag
```

**Trailing `;` locks the argument list.** The command accepts only what is defined.

```text
Tool:       [rm, { }, ;]
CORRECT:    {"command": ["rm", "file.txt"]}
INCORRECT:  {"command": ["rm", "-f", "file.txt"]}  ← no extra arguments allowed
```

**Write environment variables literally** — the server expands them at execution time.

```text
Tool:       [psql, $DATABASE_URL, -c, { }]
CORRECT:    {"command": ["psql", "$DATABASE_URL", "-c", "SELECT 1"]}
INCORRECT:  {"command": ["psql", "postgres://localhost/mydb", "-c", "SELECT 1"]}  ← substituted value
```

## Constraints

- Only declared tools can be executed.
- Commands run relative to the app's root directory.
- All interaction is over HTTP.
