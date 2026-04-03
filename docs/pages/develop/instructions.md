---
icon: lucide/file-text
---

# Instructions

Instructions are static content in your Markdown pages.

## Syntax

Write instructions in the body of Markdown pages:

````yaml title="page.md" hl_lines="6-9"
---
tools:
  - [grep]
---

# Instructions
- Use `grep` to search for logs in ./data
- Query the database for recent users
- See [analyze](src/analyze.md) for more workflows
````

## App structure

Every app is initialized with five base files:

- **`README.md`** — the main page of your app. Add tools, components, and instructions here for agents consuming your API. You write and maintain this.
- **`AGENTS.md`** — auto-generated instructions that teach coding agents how to build and work on the app via its [REST API](../reference/api.md). Served at the root URL (`/`).
- **`CLAUDE.md`** — same content as `AGENTS.md`, picked up automatically by Claude Code when working inside the project directory.
- **`API.md`** — the HTTP contract for the app. Documents URL resolution, request format, and response format for agents consuming the API.
- **`.gitignore`** — pre-configured to exclude secrets, build artifacts, and editor files.

## Links

Connect pages with links to help agents navigate multi-page apps:

```markdown title="README.md"
# My Multi-page app

- Start with [search](pages/search.md)
- Check out [analytics](pages/analytics.md) for analytics
- In case of admin stuff, see [admin](pages/admin.md)
```

Agents will progressively load pages as needed, keeping your context minimal:

```text
app/
├── AGENTS.md
├── README.md           # first here...
└── pages/
    ├── search.md       # ...then here...
    ├── analytics.md    # ...and lastly here
    └── admin.md        # but likely not here!
```
