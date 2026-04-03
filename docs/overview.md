---
icon: lucide/compass
---

# Overview

A complete walkthrough of the Statespace app lifecycle.

## 1. Initialize

Create a new project from a template:

```bash
statespace init --template postgresql
```

Templates pre-populate your API with starter tools, instructions, and guardrails:

````yaml title="README.md"
---
tools:
  - [psql, -d, $DATABASE_URL, -c, { regex: "^(SELECT|SHOW|EXPLAIN)\\b.*" }, ;]
---

# Instructions
- Explore the schema to understand the data model
- Follow the user's instructions and answer their questions
- Reference [documentation](https://www.postgresql.org/docs/) as needed
````

This is what an initialized project looks like:

```text
app/
├── README.md    # starter tools & instructions
├── AGENTS.md    # coding agent instructions 
├── CLAUDE.md
├── API.md       # HTTP contract for agents consuming the app
└── .gitignore
```


> Note: Run `statespace init --help` to see all available templates.

## 2. Run

[Run your app locally](pages/deploy/local_development.md) so your agent can build and test it:

```console
statespace run app/ --port 8000
```

Your coding agent will interact with the running app [over HTTP](pages/reference/api.md):

```bash
curl http://localhost:8000/README.md

curl -X POST http://localhost:8000/README.md \
  -H "Content-Type: application/json" \
  -d '{"command": ["grep", "error", "./data"]}'
```

## 3. Build

Tell your coding agent what you know about your data:

```bash
claude "Help me document my database's schema, business rules, and context"
```

Your agent will build the app page by page based on what you share:

```text
app/
├── README.md
├── schema/
│   ├── orders.md
│   ├── customers.md
│   └── products.md
├── reports/
│   ├── revenue/
│   │   ├── monthly.md
│   │   └── by_region.md
│   ├── churn.md
│   └── summarize.py
├── queries/
│   └── funnel.sql
└── data/
    ├── metrics.csv
    └── segments.csv
```

Markdown pages can declare [tools](pages/develop/tools.md), [components](pages/develop/components.md), and [instructions](pages/develop/instructions.md):

=== ":lucide-wrench: Tools"

    `````yaml title="schema/orders.md" hl_lines="1-4"
    ---
    tools:
      - [psql, -d, $DATABASE_URL, -c, { regex: "^(SELECT|EXPLAIN)\\b.*" }]
    ---

    ```component
    psql -d $DATABASE_URL -c "\d orders"
    ```

    # Orders
    - `order_id` — primary key
    - `customer_id` — foreign key to customers
    - `status` — one of `pending`, `fulfilled`, `cancelled`
    - Revenue is `quantity * unit_price`, excluding cancelled orders
    `````

=== ":lucide-sparkles: Components"

    `````yaml title="schema/orders.md" hl_lines="6-8"
    ---
    tools:
      - [psql, -d, $DATABASE_URL, -c, { regex: "^(SELECT|EXPLAIN)\\b.*" }]
    ---

    ```component
    psql -d $DATABASE_URL -c "\d orders"
    ```

    # Orders
    - `order_id` — primary key
    - `customer_id` — foreign key to customers
    - `status` — one of `pending`, `fulfilled`, `cancelled`
    - Revenue is `quantity * unit_price`, excluding cancelled orders
    `````

=== ":lucide-file-text: Instructions"

    `````yaml title="schema/orders.md" hl_lines="10-14"
    ---
    tools:
      - [psql, -d, $DATABASE_URL, -c, { regex: "^(SELECT|EXPLAIN)\\b.*" }]
    ---

    ```component
    psql -d $DATABASE_URL -c "\d orders"
    ```

    # Orders
    - `order_id` — primary key
    - `customer_id` — foreign key to customers
    - `status` — one of `pending`, `fulfilled`, `cancelled`
    - Revenue is `quantity * unit_price`, excluding cancelled orders
    `````

## 4. Deploy

[Deploy to the cloud](pages/deploy/cloud_deployment.md) to get a shareable URL:

```bash
statespace deploy app/ --name my-app
```

Apps can be public (anyone can use) or private ([access token](pages/deploy/security.md#access-tokens) required):

```console
statespace deploy app/ --name my-private-app --visibility private
```

## 5. Connect

Share your app URL with agents directly:

```bash
claude "Use the API at https://my-app.statespace.app to summarize recent orders"
```

Or [wire it up as an MCP server](pages/connect/mcp.md):

```json
"mcpServers": {
  "statespace": {
    "command": "npx",
    "args": ["-y", "statespace-mcp", "https://my-app.statespace.app"]
  }
}
```

## Next steps

- Learn more about [tools](pages/develop/tools.md), [components](pages/develop/components.md), and [instructions](pages/develop/instructions.md)
- [Secure](pages/deploy/security.md) your apps with token-based authentication
- Connect your agents directly to the [API](pages/connect/api.md) or through an [MCP server](pages/connect/mcp.md)
- Explore Statespace's [CLI](pages/reference/cli.md) and [REST API](pages/reference/api.md)
