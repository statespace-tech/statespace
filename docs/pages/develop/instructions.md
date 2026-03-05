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
- Use grep to search for logs in ./data
- Query the database for recent users
- See [analyze](src/analyze.md) for more workflows
````

## README.md

Every app requires a `README.md` with general instructions:

```markdown title="README.md"

# My App's README

Start here.

- You are a data analyst
- Answer the user's prompt without making assumptions
- See [analytics](pages/analytics.md) for usage stats
```

## Links

Connect pages with links to help agents navigate multi-page apps:

```markdown title="page.md"
# My Multi-page App

- For search, see [search](pages/search.md)
- For analytics, see [analytics](pages/analytics.md)
- For admin, see [admin](pages/admin.md)
```

