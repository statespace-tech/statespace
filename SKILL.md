---
name: statespace
description: Search sites and technical docs indexed from llms.txt sites with the Statespace CLI.
---

# Statespace CLI Skill

Statespace searches Markdown pages indexed from [llms.txt](https://llmstxt.org/) sites.

## Usage

```bash
npx statespace search "<query>"
npx statespace search "<site>: <query>"
npx statespace search "<query>" --limit 5
```

## Examples

```bash
npx statespace search "mcp server setup"
npx statespace search "database connection pooling" 
npx statespace search "rate limiting middleware setup" --limit 10
npx statespace search "stripe: webhook verification"
npx statespace search "supabase: edge functions auth" --limit 15
```

## Output

Results are returned as a JSON array:

```json
[
  {
    "url": "https://upstash.com/docs/redis/quickstart",
    "site": "Upstash",
    "title": "Redis Quickstart",
    "snippet": "Connect to your Upstash Redis database using the REST API or a compatible client."
  }
]
```

Use `site`, `title`, and `snippet` to decide relevance. Use `url` to fetch the full page.
