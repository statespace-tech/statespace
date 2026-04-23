---
name: statespace
description: Search documentation indexed from llms.txt sites using the Statespace CLI.
---

# Statespace CLI Skill

Statespace searches Markdown documentation indexed from [llms.txt](https://llmstxt.org/) sites.

## Usage

```bash
npx statespace search "<query>"
npx statespace search "<site>: <query>"
npx statespace search "<query>" --limit 5
```

## Examples

```bash
npx statespace search "redis connection pooling"
npx statespace search "aws: ec2 setup"
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

Use `snippet` to decide relevance. Use `url` to fetch the full page.
