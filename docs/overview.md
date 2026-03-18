---
icon: lucide/compass
---

# Overview

Learn how to build RESTful APIs for agents in Markdown.

## Pages

Each Markdown file is a self-contained page with [tools](pages/develop/tools.md), [components](pages/develop/components.md), and [instructions](pages/develop/instructions.md):

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

```bash
curl https://demo.statespace.app/page.md
```

And call tools with `POST` requests:

```bash
curl -X POST https://demo.statespace.app/page.md -d '{"command": ["grep", "error"]}'
```

## Base structure

All apps have two [base pages](pages/develop/instructions.md#base-pages):

- **AGENTS.md** - ^^Auto-generated^^ page served at your app's root URL to onboard agents.
- **README.md** - Your app's ^^required^^ main page with tools, components, and instructions.

```
app/
├── AGENTS.md           # auto-generated
├── README.md           # you write this
└── ...
```

## Multi-page apps

Split large apps into multiple pages to reduce token usage through [progressive disclosure](https://en.wikipedia.org/wiki/Progressive_disclosure):

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

Your application can also include data files and scripts used by tools and components:
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

[Run apps locally](pages/deploy/local_development.md) for development or self-hosting:

```bash
statespace serve app/
```

Or [deploy to the cloud](pages/deploy/cloud_deployment.md) for a public URL:

```bash
statespace deploy app/ --name demo
```

## Next steps

- Learn more about [tools](pages/develop/tools.md), [components](pages/develop/components.md), and [instructions](pages/develop/instructions.md)
- [Run your apps](pages/deploy/local_development.md) locally or [deploy them](pages/deploy/cloud_deployment.md) to the cloud
- [Secure](pages/deploy/security.md) your apps with token-based authentication
- [Connect your agents](pages/connect/agents.md) to applications
- Explore Statespace's [CLI](pages/reference/cli.md) and [REST API](pages/reference/api.md)
