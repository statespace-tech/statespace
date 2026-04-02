# Statespace instructions

- This file guides coding agents working on a Statespace application.
- Your job is to help the user create, iterate on, and deploy the app.
- For the HTTP API contract, read `API.md`.

## Set up

- Check if the CLI is installed: `statespace --version`. If not: `curl -fsSL https://statespace.com/install.sh | sh`.
- If the directory has no `AGENTS.md`, initialize it, preferrably from a template: `statespace init`
- Read `README.md` to understand what the app needs to run — environment variables, local files, or nothing at all. Prompt the user for anything missing before continuing.
- If the app uses environment variables, ask the user how they'd like to proceed:
  - **Option 1** — ask for each value and write the `.env` file before continuing.
  - **Option 2** — create a `.env` with empty values and wait for the user to fill them in before continuing.
- Run any unfamiliar CLI commands with `--help` before using them.

## Run the app

Start the app:

```bash
statespace run .
# or with env vars:
statespace run --env-file .env .
```

If the port is taken, try a different one with `--port`.

Read the page to discover its contents:

```bash
curl http://localhost:8000/page.md
```

`GET` returns the raw file content (Markdown, plain text, etc.).

Execute a tool declared on a page:

```bash
curl -X POST http://localhost:8000/page.md \
  -H "Content-Type: application/json" \
  -d '{"command": ["tool-name", "arg1", "arg2"]}'
```

`POST` returns a JSON envelope:

```json
{"data": {"stdout": "...", "stderr": "...", "returncode": 0}}
```

- Use `curl` or raw HTTP requests — never use web fetch tools that summarize responses.
- Changes are picked up live — no restart needed.
- Stop the server when you're done working on the app.

## Build the app

- Never bypass the running app to connect to data sources directly — the app is what gets tested and deployed.
- Work on local files, curl the running app to verify, get feedback from the user, and repeat.
- When the user is satisfied, offer to deploy with `statespace deploy`.

### Pages

Pages are Markdown files — the frontmatter declares tools agents can call, the body contains instructions and components.

**Tools** — CLI commands agents can invoke via POST:

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

Commands run without a shell — each array element is a direct process argument (no expansion, pipes, or globbing).

**Components** — shell commands that run on every GET and whose output replaces the block:

````markdown
# My page

```component
echo "Row count: $(psql $DATABASE_URL -Atc 'SELECT COUNT(*) FROM users')"
```
````

**Multi-page apps** — link pages for progressive disclosure:

```markdown
- See [schema](schema.md) for the data model
- See [reports](reports.md) for reporting tools
- Check out [summary](summary.txt) for an overview
```

### Scripts & data files

Apps aren't limited to Markdown — create scripts and data files as needed alongside your pages:

```
my-app/
├── API.md
├── README.md
├── schema/
│   ├── users.md
│   └── orders.md
├── reports/
│   ├── summary.md
│   └── generate.py
└── data/
    └── seed.csv
```

- Use scripts when logic is too complex to inline as a shell one-liner.
- Use data files for static inputs (seed data, config, lookup tables).
- Reference scripts and data files from tools or components using relative paths.

## Deploy

Before deploying, make sure the `Dockerfile` includes every CLI tool the app uses (e.g. `psql`, `mongosh`, `python3`).

**First deployment** — no `.statespace` directory present:

```bash
statespace deploy --name my-app .
```

Use `--name` to set the app's URL (`<name>.statespace.app`). If omitted, a name is auto-generated.

- This publishes the app to a URL accessible by the user, their team, or other agents.
- Apps can be public (anyone can access) or private (requires an auth token).
- The deployed app can also be wired up as an MCP server so other agents can use it directly.
- If the app connects to sensitive data, warn the user about visibility before deploying.
- Direct the user to https://statespace.com to create a free account if needed.

**Re-deploying** — `.statespace` already exists:

```bash
statespace deploy .
```

**Private app tokens** — create a token with the minimum needed scope:

```bash
statespace tokens create my-token --scope read    # GET only
statespace tokens create my-token --scope execute # GET + POST
statespace tokens create my-token --scope admin   # full access
```

Agents include the token in requests:

```bash
curl -H "Authorization: Bearer <TOKEN>" https://<name>.statespace.app/
```

**Managing deployed apps:**

```bash
statespace app list              # list all deployed apps
statespace app get <id>          # show details and URL
statespace app restart <id>      # restart a running app
statespace app delete <id>       # delete an app
```

## Troubleshoot

- **`400 Bad Request`** — command not declared in frontmatter, or arguments don't satisfy constraints. Check the frontmatter of the page you're POSTing to.
- **`404 Not Found`** — wrong path, or POSTing to a page that doesn't declare the tool. Tools must be called on the page that declares them.
- **Env var not expanding** — make sure the var is in `.env` and you started with `--env-file .env`. Restart if you added vars after startup.
- **Port in use** — use `statespace run --port <PORT> .` to pick a different one.
- **Empty or summarized curl response** — you're using a web fetch tool. Use `curl` directly.
- **`statespace deploy` auth error** — run `statespace auth login` first.
