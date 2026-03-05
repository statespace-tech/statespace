---
icon: lucide/blocks
---

# Overview

Apps are built in Markdown and served to agents over a [REST API](pages/reference/api.md).

## Pages

Markdown pages can include [tools](pages/develop/tools.md), [components](pages/develop/components.md), and [instructions](pages/develop/instructions.md):

````yaml title="page.md"
---
tools:
  - [grep]
  - [curl, -X, GET, { }]
  - [psql, -c, { regex: "^SELECT\\b.*" }]
---

```component
echo "Server time: $(date)"
```

# Instructions
- Use grep to search for logs in ./data
- Query the database for recent users
- See [analyze](src/analyze.md) for more workflows
````

## Interactions

Agents can read pages and files with `GET` requests:

```console
$ curl https://example.statespace.app/page.md
```

And call tools with `POST` requests:

```console
$ curl -X POST https://example.statespace.app -d '{"command": ["grep", "error"]}'
```

## Base structure

All apps have two base pages:

- **AGENTS.md** - ^^Auto-generated^^ page served at your app's root URL to onboard agents.
- **README.md** - Your app's ^^required^^ main page with tools, components, and instructions.

```
app/
├── AGENTS.md           # auto-generated
├── README.md           # you write this
└── ...
```

## Multi-page apps

Split large apps into multiple pages to reduce token usage through progressive context disclosure:

```text
app/
├── AGENTS.md
├── README.md            # links to pages/
└── pages/
    ├── search.md        # search capabilities
    ├── analytics.md     # analytics capabilities
    └── admin.md         # admin tools
```

## Custom files

Your app directory can also include data files and scripts used by tools and components:
```
app/
├── AGENTS.md
├── README.md
├── data/
│   ├── customers.csv
│   └── sales.db
└── scripts/
    ├── query.py
    └── report.sh
```

## Running apps

[Serve apps locally](pages/deploy/self_hosting.md) for development:

```console
$ statespace serve app/
```

Or [deploy to the cloud](pages/deploy/cloud.md) for a public URL:

```console
$ statespace deploy app/
```

## Next steps

- Learn more about [tools](pages/develop/tools.md), [components](pages/develop/components.md), and [instructions](pages/develop/instructions.md)
- [Deploy to the cloud](pages/deploy/cloud.md) or [self-host](pages/deploy/self_hosting.md) your apps
- [Secure](pages/deploy/security.md) your apps with token-based authentication
- [Connect your agents](pages/connect/agents.md) to running applications
- [Tunnel via SSH](pages/connect/ssh.md) to debug and patch deployed apps live
- Explore Statespace's [CLI](pages/reference/cli.md) and [REST API](pages/reference/api.md)
