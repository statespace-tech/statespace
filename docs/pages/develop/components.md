---
icon: lucide/sparkles
---

# Components

Components are shell commands embedded in pages that run when the page loads.

## Overview

Add `component` code blocks to your pages:

````markdown title="page.md" hl_lines="3-5"
# Dashboard

```component
echo "Server time: $(date)"
```
````

When the page loads, the output replaces the code block:

```markdown title="page.md" hl_lines="3"
# Dashboard

Server time: Mon Jan 27 10:42:03 PST 2026
```

## Multiple components

Use multiple components on a single page:

````markdown
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

> **Note:** Each component executes independently when the page loads.

## Environment variables

Components can reference environment variables:

````markdown
# Dashboard

```component
echo "User ID: $USER"
```
````