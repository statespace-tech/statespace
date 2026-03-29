# AGENTS.md — Statespace

Statespace lets you build self-documenting data apps that describe themselves to agents over HTTP. Apps expose content and tools that any agent can discover and use without prior knowledge.

## Step 1: Install the CLI

Check if the CLI is already installed:

```bash
statespace --version
```

If not, install it:

```bash
curl -fsSL https://statespace.com/install.sh | sh
```

## Step 2: Set up the project

If the current directory already contains an `AGENTS.md`, the project is initialized — skip to Step 3.

Otherwise, initialize the app in the current directory:

```bash
statespace init --from <template>
```

`--from` pulls the `README.md` from a built-in template (e.g. `postgresql`, `vectorless-rag`). Built-in templates are available at [github.com/statespace-tech/statespace/examples](https://github.com/statespace-tech/statespace/tree/main/examples). If omitted, a blank `README.md` is created.

## Step 3: Spin up the server

Read the app's `README.md` to understand which environment variables it needs.

Check if a `.env` file already exists in the project directory. If it does, use it directly:

```bash
statespace serve . --env-file .env
```

If not, identify the required variables from `README.md` and either write the `.env` file yourself or ask the user to provide the values. A `.env` file persists across sessions and server restarts — prefer it over `--env KEY=VALUE`, which has to be re-passed every time. Once the file is written, run:

```bash
statespace serve . --env-file .env
```

The app runs at `http://localhost:8000`.

## Step 4: Iterate on the app

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

Follow links to load additional pages only as needed. Edit app files, verify the result with `curl`, get feedback from the user, and repeat. Changes are picked up live — no restart needed.

Always interact with the data source through the running app — never connect to it directly (e.g. do not run `psql` or `redis-cli` yourself). The whole point of the app is to define and test the tools the agent will use in production. Bypassing the app means you're not actually testing what will be deployed.

## Step 5: Deploy the app

Only suggest this step once you believe the user is satisfied with the app after iterating on it.

```bash
statespace deploy . --name <name>
```

Requires a free [Statespace account](https://statespace.com). Returns a public URL and an access token. Pass the URL to other agents or wire it up as an MCP server.

For private apps, agents must include the token in requests:

```bash
curl -H "Authorization: Bearer <TOKEN>" https://<name>.statespace.app/README.md
```

## App protocol

A Statespace app is a directory of Markdown files served over HTTP. Each file is a page. Pages have two parts:

- **Frontmatter:** declares tools agents can call via POST
- **Body:** instructions and components agents read via GET

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

**Extra arguments are allowed by default:**

```text
Tool:       [ls]
CORRECT:    {"command": ["ls", "."]}
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
INCORRECT:  {"command": ["cat", "notes.py"]}                  ← doesn't match pattern
```

**Fixed elements are immutable.** Only replace placeholders — never modify, remove, or reorder fixed elements.

```text
Tool:       [grep, -r, -i, { }, ../data/]
CORRECT:    {"command": ["grep", "-r", "-i", "error", "../data/"]}
CORRECT:    {"command": ["grep", "-r", "-i", "error", "../data/", "-l"]}    ← extra arguments are fine
INCORRECT:  {"command": ["grep", "-r", "-i", "error", "../data/file.txt"]}  ← changed fixed path
INCORRECT:  {"command": ["grep", "-r", "error", "../data/"]}                ← removed fixed flag
```

**Trailing `;` locks the argument list:**

```text
Tool:       [rm, { }, ;]
CORRECT:    {"command": ["rm", "file.txt"]}
INCORRECT:  {"command": ["rm", "-f", "file.txt"]}  ← no extra arguments allowed
```

**Write environment variables literally** — the server expands them at execution time:

```text
Tool:       [psql, $DATABASE_URL, -c, { }]
CORRECT:    {"command": ["psql", "$DATABASE_URL", "-c", "SELECT 1"]}
INCORRECT:  {"command": ["psql", "postgres://localhost/mydb", "-c", "SELECT 1"]}  ← substituted value
```

### Components

Components are `component` code blocks in the page body that run when the page is fetched. Their output replaces the block in the response:

````markdown
```component
echo "Server time: $(date)"
```
````

Agents see the output, not the command. Use components for live data — current time, row counts, recent logs — that should be fresh every time the page loads.

### Multi-page apps

Large apps can be split across multiple pages. Link them from `README.md`:

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

**When in doubt, use `--help`** — every CLI command and subcommand supports it:

```bash
statespace --help
statespace serve --help
statespace deploy --help
statespace init --help
```

**`400 Bad Request` on a tool call** — the command isn't declared in that page's frontmatter, or the arguments don't satisfy the constraints (missing placeholder, regex mismatch, extra args blocked by `;`). Check the frontmatter of the page you're POSTing to.

**`404 Not Found`** — the page path is wrong, or you're POSTing to a page that doesn't declare the tool you're trying to run. Tools must be called on the page that declares them.

**Environment variable not expanding** — make sure the variable is present in `.env` and that you started the server with `--env-file .env`. Restart the server if you added variables after it started.

**Server won't start (port in use)** — another process is on port 8000. Use `--port` to pick a different one:

```bash
statespace serve . --env-file .env --port 8080
```

**`Unknown template` error from `statespace init --from`** — the slug doesn't match any built-in template. Check available templates at [github.com/statespace-tech/statespace/examples](https://github.com/statespace-tech/statespace/tree/main/examples).

**curl returns unexpected results / empty response** — you may be using a web fetch tool that summarizes responses. Use `curl` directly; Statespace apps require unfiltered HTTP responses.

**`statespace deploy` fails with auth error** — run `statespace auth login` first, then retry.
