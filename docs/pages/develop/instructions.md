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

Add your app's general tools and instructions to `README.md`:

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

`AGENTS.md` teaches agents how to interact with your app through the [REST API](../reference/api.md):

```markdown title="AGENTS.md"
# Statespace Application Instructions

This web application exposes content and tools over HTTP.

## Quick Start

1. GET `/README.md` to discover what this app does
2. Follow links to read content
3. POST to `/` with {"command": [...]} to execute tools

...
```

While `AGENTS.md` is served at the root URL for onboarding, `README.md` is served normally:

```console
$ curl https://example.statespace.app/            # returns AGENTS.md
$ curl https://example.statespace.app/README.md   # returns README.md
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
