# API instructions

This Statespace application exposes content and commands over HTTP.

## Quick start

1. **`GET /`** — read the API instructions and discover what this application does.
2. **Follow links** — `GET /{path}` to read any file.
3. **Execute a command** — `POST /{path}` with `{"command": [...]}` where the command is declared in that page's frontmatter.

## URL resolution

- The same resolution rules apply to both `GET` and `POST`.
- `POST` must resolve to a Markdown file - only Markdown pages declare tools in their frontmatter.

| Path             | Resolves to                                    |
|------------------|------------------------------------------------|
| `/`              | `API.md`                                       |
| `/page`          | `page`, then `page/README.md`, then `page.md`  |
| `/dir/`          | `dir/README.md`                                |
| `/page.md`       | `page.md`                                      |
| `/file.txt`      | `file.txt`                                     |

## Command Rules

- Commands are declared in YAML frontmatter of Markdown pages.
- To execute a command, `POST` to the page path with the command array.
- For example, if `page.md` declares `[grep]`, POST `{"command": ["grep", ...]}` to `/page.md`.

**Extra arguments are allowed by default**

```
Tool:       [ls]
CORRECT:    POST {"command": ["ls", "."]}
CORRECT:    POST {"command": ["ls", "--help"]}
```

**`{ }` accepts exactly one argument**

```
Tool:       [ls, { }]
CORRECT:    POST {"command": ["ls", "src"]}
CORRECT:    POST {"command": ["ls", "src", "lib"]}
INCORRECT:  POST {"command": ["ls"]}
```

**`{ regex: "pattern" }` accepts one argument matching the pattern**

```
Tool:       [cat, { regex: ".*\\.txt$" }]
CORRECT:    POST {"command": ["cat", "notes.txt"]}
CORRECT:    POST {"command": ["cat", "notes.txt", "logs.csv"]}
INCORRECT:  POST {"command": ["cat", "notes.csv"]}
```

**Fixed elements are immutable**

```
Tool:       [grep, -r, -i, { }, ./data]
CORRECT:    POST {"command": ["grep", "-r", "-i", "error", "./data"]}
CORRECT:    POST {"command": ["grep", "-r", "-i", "error", "./data", "-l"]}
INCORRECT:  POST {"command": ["grep", "-r", "error", "./data"]}
```

**Trailing `;` locks the argument list**

```
Tool:       [rm, { }, ;]
CORRECT:    POST {"command": ["rm", "file.txt"]}
INCORRECT:  POST {"command": ["rm", "-f", "file.txt"]}
```

**Never substitute the value of `$VAR`**

```
Tool:       ["psql", "$DB"]
CORRECT:    POST {"command": ["psql", "$DB", "-c", "SELECT 1"]}
INCORRECT:  POST {"command": ["psql", "postgres://localhost/mydb", "-c", "SELECT 1"]}
```
