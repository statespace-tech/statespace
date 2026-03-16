---
name: statespace
description: "Build and deploy Statespace apps — Markdown-based web applications that agents interact with over HTTP. TRIGGER when: user asks to create a Statespace app, add tools/components to markdown, deploy with `statespace` CLI, or connect agents to a Statespace endpoint. DO NOT TRIGGER when: general markdown editing, static site generators, or unrelated CLI tools."
---

# Building Statespace Apps

Statespace apps are Markdown files served over HTTP. Agents read pages with GET and call tools with POST. No frameworks, no dependencies — just Markdown.

## App Structure

Every app needs a `README.md`. That's it. An `AGENTS.md` is auto-generated on first serve/deploy.

```
my-app/
├── AGENTS.md          # auto-generated (onboards agents)
├── README.md          # you write this
├── data/              # optional data files
└── pages/             # optional sub-pages
```

## Three Building Blocks

### 1. Tools — CLI commands agents can call

Declare tools in YAML frontmatter. Each tool is an array of command parts:

```yaml
---
tools:
  - [grep]                                     # bare command, extra args allowed
  - [cat, { }]                                 # placeholder: agent must pass one arg
  - [psql, -c, { regex: "^SELECT\\b.*" }]     # regex: restrict to SELECT queries
  - [curl, -X, GET, https://api.example.com, ;]  # semicolon: no extra args allowed
  - [psql, -U, $USER, -d, $DB, -c, { }]       # $ENV: expanded at runtime, hidden from agent
---
```

**Rules:**
- `{ }` — agent fills in one argument (extra trailing args still allowed)
- `{ regex: "pattern" }` — argument must match the pattern
- `;` at end — locks the command, no extra arguments
- `$VAR` — environment variable, expanded server-side

### 2. Components — run on page load

Shell code blocks tagged `component` execute when the page is fetched and render their stdout as Markdown:

````markdown
```component
echo "Server time: $(date)"
echo "$(wc -l < data/users.csv) users in database"
```
````

Use `$VARIABLES` in components — pass them via `--env`, `--env-file`, or query params (`?USER=admin`).

### 3. Instructions — plain Markdown

Everything else is instructions. Use links for multi-page progressive disclosure:

```markdown
Browse the available tools:
- [Search](pages/search.md) — full-text search
- [Admin](pages/admin.md) — management tools
```

## Installation

```bash
curl -fsSL https://statespace.com/install.sh | bash
```

This installs the `statespace` binary (and `ssp` alias) to `~/.statespace/bin/`. Supports macOS (Apple Silicon, Intel) and Linux (x86_64, ARM64).

## CLI

### Authentication

```bash
statespace auth login       # log in via browser (device authorization flow)
statespace auth logout      # clear stored credentials
statespace auth status      # show current auth info (email, user ID, token expiration)
statespace auth token       # print API token (for CI/CD scripts)
```

### Serving locally (no account needed)

```bash
statespace serve ./my-app
statespace serve ./my-app --port 3000 --env API_KEY=abc123
statespace serve ./my-app --env-file .env
```

### Cloud deployment

```bash
statespace deploy ./my-app --name my-app
statespace deploy ./my-app --name my-app --visibility private --env-file .env
statespace deploy --name scratch-env              # empty environment, no files
```

Omitting `--name` creates a random name. App names: 3–63 chars, lowercase, digits, hyphens only. No leading/trailing/consecutive hyphens.

### App management

```bash
statespace app list              # list all apps in your org
statespace app get <APP>         # show app details (accepts name, ID, or URL)
statespace app delete <APP>      # delete an app (--yes to skip confirmation)
```

## How Agents Interact

**Read pages:**

```bash
curl https://my-app.statespace.app/README.md
curl "https://my-app.statespace.app/page.md?USER_ID=42"
```

**Call tools:**

```bash
curl -X POST https://my-app.statespace.app \
  -H "Content-Type: application/json" \
  -d '{"command": ["grep", "-r", "error", "logs/"]}'
```

Response: `{"stdout": "...", "stderr": "...", "returncode": 0}`

**Private apps** — pass a token:

```bash
curl -H "Authorization: Bearer <TOKEN>" https://my-app.statespace.app/README.md
```

## Complete Example

A text-to-SQL app with a SQLite database:

````yaml
---
tools:
  - [sqlite3, store.db, { regex: "^SELECT\\b.*" }]
---

# E-Commerce Store

```component
sqlite3 store.db "SELECT count(*) FROM orders" | xargs -I{} echo "{} orders"
```

Use `sqlite3` to query the database. Only SELECT queries are allowed.

## Schema

**customers** — id, name, email, city, country, joined
**orders** — id, customer_id, product_id, quantity, ordered_at
````

## Token Management

```bash
statespace tokens create ci-readonly --scope read
statespace tokens create deploy-token --scope admin --app-id APP_ID
statespace tokens list
statespace tokens rotate TOKEN_ID
statespace tokens revoke TOKEN_ID
```

Scopes: `read` (pages only), `execute` (pages + tools), `admin` (full access).

## Checklist

When building a new Statespace app:

1. Create `README.md` with tools in frontmatter
2. Add components for dynamic data shown on page load
3. Write clear instructions — agents follow them literally
4. Use `--env` or `--env-file` for secrets, never hardcode them
5. Test locally with `statespace serve`, then deploy with `statespace deploy`
