---
icon: lucide/sparkles
---

# Components

Components are CLI code blocks that run when Markdown pages load.

## Syntax

Add `component` code blocks to your Markdown pages:

````markdown title="page.md" hl_lines="4-6"
# Dashboard
- Use today's date

```component
echo "App time: $(date)"
```
````

When a page loads, components are replaced with their output:

```markdown title="page.md" hl_lines="4"
# Dashboard
- Use today's date

App time: Mon Jan 27 10:42:03 PST 2026
```

## Multiple components

Use multiple components on a single page:

````markdown title="page.md"
# Dashboard

## Server status

```component
uptime
```

## Database

```component
psql -c "SELECT COUNT(*) FROM orders"
```

## Recent logs

```component
tail -5 /var/log/app.log
```
````

## Environment variables

Reference environment `$VARIABLES` in your components to hide secrets from agents

````markdown title="page.md"
# Dashboard

```component
echo "User ID: $USER"
```
````

You can set secrets for a deployed app with the [CLI](../reference/cli.md):

```console
$ statespace secrets set --app <APP> USER=admin
```

Or pass them as query parameters when [fetching pages](../reference/api.md#get):


```bash
curl "https://example.statespace.app/page.md?USER=admin"
```

!!! warning

    Query parameters can be exposed iduring transit. For sensitive values, set secrets with the CLI, or use tools.