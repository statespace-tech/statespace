---
icon: lucide/file
---

# Pages

Pages are Markdown files served over HTTP.

## Overview

Add content to a Markdown file to create a page:

````yaml title="README.md"
---
tools:
  - [expr, { }]
---

```component
echo "Random number: $RANDOM"
```

# Instructions
- The component loads a random number when the page loads
- Use the `expr` tool to multiply it
````

> **Note**: Pages can also define interactive [tools](tools.md) and dynamically rendered [components](components.md).

## Base structure

Every app has two base pages:

- **README.md** - Your app's entry point with tools, components, and instructions.
- **AGENTS.md** - Auto-generated file that teaches agents how to use your app.

!!! question "How does it work?"

    When an agent first makes an HTTP GET request to your app's base URL, it receives `AGENTS.md`. This page teaches the agent how to interact with your app over HTTP. You shouldn't modify this file.

## Multi-page apps

Split larger applications into multiple pages:

```text
app/
├── AGENTS.md
├── README.md
└── pages/
    ├── search.md        # search capabilities
    ├── analytics.md     # analytics capabilities
    └── admin.md         # admin tools
```

Reference other pages in your content to help agents discover them:

```markdown title="README.md"
# My App

Start here.

- For search, see [search](pages/search.md)
- For analytics, see [analytics](pages/analytics.md)
- For admin, see [admin](pages/admin.md)
```

> **Note**: Multi-page apps reduce token usage by letting agents load context as needed. This is called [progressive disclosure](https://en.wikipedia.org/wiki/Progressive_disclosure).

## Custom files

Include data files and scripts used by [tools](tools.md) and [components](components.md) in your app directory:

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

Reference them using relative paths:

````yaml title="README.md"
---
tools:
  - [sqlite3, data/sales.db, { }]
  - [python3, scripts/query.py, { }]
---

Check out data/customers.csv

```component
bash scripts/report.sh
```
````
