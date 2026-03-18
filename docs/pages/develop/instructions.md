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

## Base pages

Every app requires a `README.md` and auto-generates an `AGENTS.md`:

```
app/
├── AGENTS.md           # auto-generated, modify with caution!
├── README.md           # required, you write this
└── ...
```

Add general instructions and tools to your `README.md`:

```yaml title="README.md"
---
tools:
  - [echo]
---

# My app's README

- You are a data analyst
- Answer the user's prompt
- Do not make assumptions
```

`AGENTS.md` teaches agents how to interact with your app through its [REST API](../reference/api.md):

```markdown title="AGENTS.md"
# App instructions

This Statespace web application exposes content and tools over HTTP. Follow these instructions exactly.

## Quick start

1. **GET `/README.md`** — discover what this application does, root-level tools, and where to navigate.
2. **Follow links** — GET any path to read content (Markdown, data files, etc.).
3. **Execute tools** — POST to the page where the tool is declared with `{"command": ["tool-name", "arg1", "arg2"]}`.

...
```

While `AGENTS.md` is served at the root URL for onboarding, `README.md` is served normally:

```bash
curl https://demo.statespace.app/            # returns AGENTS.md
curl https://demo.statespace.app/README.md   # returns README.md
```

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
