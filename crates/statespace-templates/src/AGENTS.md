# Statespace instructions

This file guides local coding agents working on the app source tree. It is not the deployed HTTP API contract; see `API.md` for that.

Statespace lets you build self-documenting data apps that describe themselves to agents over HTTP. Apps expose content and tools that any agent can discover and use without prior knowledge.

Before running any CLI command for the first time, run it with `--help` to see all available options and flags.

## Step 1: Install the CLI

Check if the CLI is already installed:

```bash
statespace --version
```

If not, install it:

```bash
curl -fsSL https://statespace.com/install.sh | sh
```

## Step 2: Initialize the project

If the working directory already contains an `AGENTS.md` and `README.md`, the project is initialized — skip to Step 3. Otherwise, use `statespace init` to initialize the project.

## Step 3: Set up the app

Read the project's `README.md` to understand what the app needs to run. This could be environment variables, local files, or nothing at all. Prompt the user for anything that's missing before continuing.

For environment variables, ask the user how they'd like to proceed:

**Option 1 — Ask the user for each value** and write the `.env` file yourself before continuing.

**Option 2 — Create a `.env` with empty values** and wait for the user to fill them in before continuing.

If the app uses a `.env` file, start it with `statespace run --env-file .env` so those values are loaded explicitly.

## Step 4: Iterate on the app

Always interact with the user's data through the running app — never connect to it directly. The whole point of the app is to define and test the tools that will be used in production. Bypassing the app means you're not testing what will be deployed.

Always use `curl` (or raw HTTP requests) to interact with Statespace apps. Web fetch tools that summarize pages will not work — you need unfiltered HTTP responses.

Start by reading `README.md` to discover what the app does, its tools, and where to navigate:

```bash
curl http://localhost:8000/README.md
```

Execute a tool declared on a page:

```bash
curl -X POST http://localhost:8000/README.md \
  -H "Content-Type: application/json" \
  -d '{"command": ["tool-name", "arg1", "arg2"]}'
```

The response includes stdout, stderr, and the exit code:

```json
{"data": {"stdout": "...", "stderr": "...", "returncode": 0}}
```

Follow links to load additional pages only as needed as if you were an agent navigating the app for the first time. Edit app files, verify the results by curling the running app, get feedback from the user, and repeat. Changes are picked up live — no restart needed.

## Step 5: Deploy the app

Once you believe the user is satisfied, suggest deploying with `statespace deploy`, but don't be pushy about it. Check whether a `.statespace` directory exists in the project:

- **No `.statespace`:** this is a first deployment. Explain what deploying means: it publishes the app to a URL so it can be used without running anything locally — by the user, their team, or other agents — and it can be wired up as an MCP server. Apps can be public (accessible to anyone with the URL) or private (require a token). Make sure the user understands this before deploying, especially if the app connects to sensitive data. Share https://statespace.com so the user can create a free account if they don't have one.
- **`.statespace` exists:** the app has been deployed before. Suggest re-deploying to push the latest changes.

Before (re)deploying, make sure the `Dockerfile` is up to date. It defines the runtime image and must include every CLI tool the app relies on (e.g. `psql`, `mongosh`). If you added tools during iteration that aren't in the `Dockerfile`, add them before deploying.


For private apps, access requires a token. Tokens can be shared with teammates or other agents. Agents must include the token in requests:

```bash
curl -H "Authorization: Bearer <TOKEN>" https://<name>.statespace.app/README.md
```

Tokens have three scopes — choose the minimum needed:

- `read` — fetch pages (GET only)
- `execute` — fetch pages and call tools (GET + POST)
- `admin` — full access including deployment

Use `statespace tokens create` to create a token with the appropriate scope.

## App protocol

A Statespace app is a directory of files served over HTTP. Each Markdown file is a page. Pages have two parts:

- **Frontmatter:** declares tools agents can call via POST
- **Body:** instructions and components agents read via GET

### URL resolution

| Request | Resolves to |
|---|---|
| `GET /` | `API.md` |
| `GET /page` | `page`, then `page/README.md`, then `page.md` |
| `GET /dir/` | `dir/README.md` |
| `GET /page.md` | `page.md` |
| `GET /file.txt` | `file.txt` |

### Tools

Tools are CLI commands declared in the YAML frontmatter of a page:

```markdown
---
tools:
  - [ls]
  - [grep, -r, -i, { }, ../data/]
  - [cat, { regex: ".*\\.txt$" }]
---

# My page
...
```

To execute a tool, POST `{"command": [...]}` to the path of the page that declares it. Commands run without a shell — each array element is a direct process argument (no expansion, pipes, or globbing).

#### Tool rules

**Extra arguments are allowed by default**

```
Tool:       [ls]
CORRECT:    {"command": ["ls", "."]}
CORRECT:    {"command": ["ls", "-la", "."]}
```

**`{ }` accepts exactly one argument**

```
Tool:       [ls, { }]
CORRECT:    {"command": ["ls", "src"]}
CORRECT:    {"command": ["ls", "src", "lib"]}  ← extra arguments are fine
INCORRECT:  {"command": ["ls"]}                ← missing argument
```

**`{ regex: "pattern" }` accepts one argument matching the pattern**

```
Tool:       [cat, { regex: ".*\\.txt$" }]
CORRECT:    {"command": ["cat", "notes.txt"]}
CORRECT:    {"command": ["cat", "notes.txt", "logs.csv"]}     ← extra arguments are fine
INCORRECT:  {"command": ["cat", "notes.py"]}                  ← doesn't match pattern
```

**Fixed elements are immutable**

```
Tool:       [grep, -r, -i, { }, ../data/]
CORRECT:    {"command": ["grep", "-r", "-i", "error", "../data/"]}
CORRECT:    {"command": ["grep", "-r", "-i", "error", "../data/", "-l"]}    ← extra arguments are fine
INCORRECT:  {"command": ["grep", "-r", "-i", "error", "../data/file.txt"]}  ← changed fixed path
INCORRECT:  {"command": ["grep", "-r", "error", "../data/"]}                ← removed fixed flag
```

**Trailing `;` locks the argument list**

```
Tool:       [rm, { }, ;]
CORRECT:    {"command": ["rm", "file.txt"]}
INCORRECT:  {"command": ["rm", "-f", "file.txt"]}  ← no extra arguments allowed
```

**Write environment variables literally.** The server expands them at execution time.

```
Tool:       [psql, $DATABASE_URL, -c, { }]
CORRECT:    {"command": ["psql", "$DATABASE_URL", "-c", "SELECT 1"]}
INCORRECT:  {"command": ["psql", "postgres://localhost/mydb", "-c", "SELECT 1"]}  ← substituted value
```

### Components

Component code blocks run when the page is fetched. Their output replaces the block in the response:

````markdown
# This is an app

```component
echo "Server time: $(date)"
```
````

Agents see the output, not the command. Use components for live data that should be fresh every time the page loads (e.g. current time, row counts, recent logs).

### Multi-page apps

Large apps can be split across multiple pages. Link them from `README.md` or between pages:

```markdown
# My App

- See [search](pages/search.md) for search capabilities
- See [analytics](pages/analytics.md) for reporting
```

Load pages progressively — only fetch pages relevant to the current task.

### Constraints

- Only declared tools can be executed.
- Commands run relative to the app's root directory.
- All interaction is over HTTP.

## Troubleshooting

**`400 Bad Request` on a tool call** — the command isn't declared in that page's frontmatter, or the arguments don't satisfy the constraints (missing placeholder, regex mismatch, extra args blocked by `;`). Check the frontmatter of the page you're POSTing to.

**`404 Not Found`** — the page path is wrong, or you're POSTing to a page that doesn't declare the tool you're trying to run. Tools must be called on the page that declares them.

**Environment variable not expanding** — make sure the variable is present in `.env` and that you started the server with `--env-file .env`. Restart the server if you added variables after it started.

**Server won't start (port in use)** — another process is on port 8000. Run `statespace run --help` and use `--port` to pick a different one.

**curl returns unexpected results or an empty response** — you may be using a web fetch tool that summarizes responses. Use `curl` directly — Statespace apps require unfiltered HTTP responses.

**`statespace deploy` fails with auth error** — run `statespace auth login` first, then retry.
