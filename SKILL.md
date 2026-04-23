---
name: statespace
description: Search documentation indexed from llms.txt sites using the Statespace CLI.
---

# Statespace CLI Skill

Statespace searches Markdown documentation indexed from [llms.txt](https://llmstxt.org/) sites.

## Usage

```bash
npx statespace search "<query>"
```

## Examples

Search across all sites:

```bash
npx statespace search "redis connection pooling"
```

Search within a specific site:

```bash
npx statespace search "aws: ec2 setup"
npx statespace search "supabase: row level security"
```

Limit results:

```bash
npx statespace search "rate limiting" --limit 5
```

## Output

Results are returned as JSON:

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

Use `url` to fetch the full page, `snippet` for a preview.
