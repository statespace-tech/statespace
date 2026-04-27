---
name: statespace
description: Search sites and technical docs indexed from llms.txt sites with Statespace.
---

# Statespace Search Skill

Statespace searches Markdown and .txt pages indexed from [llms.txt](https://llmstxt.org/) sites.

## Query syntax

Plain queries search across all indexed sites:

```
mcp server setup
vector database embeddings
rate limiting middleware
oauth2 token refresh
websocket reconnection strategy
```

Scope a query to a specific site with `site: query`:

```
stripe: webhook verification
supabase: edge functions auth
aws: s3 presigned urls
vercel: edge middleware caching
anthropic: tool use function calling
```

## CLI

Search indexed documentation from the command line: `npx statespace search <query> [options]`

```bash
npx statespace search "mcp server setup"
npx statespace search "stripe: webhook verification" --limit 5
npx statespace search "redis connection pooling" --limit 10 --offset 3
npx statespace search "anthropic: tool use function calling" --limit 5 --human
```

| Flag              | Short | Default | Max  | Description                            |
| ----------------- | ----- | ------- | ---- | -------------------------------------- |
| `--limit <n>`     | `-l`  | `10`    | `50` | Max results to return                  |
| `--offset <n>`    | `-o`  | `0`     | —    | Results to skip (for pagination)       |
| `--human`         | —     | —       | —    | Human-readable output instead of JSON  |

## SDK

Import and call `search()` directly from TypeScript or JavaScript.

```typescript
import { search } from 'statespace';

const results = await search("mcp server setup");
const results = await search("stripe: webhook verification", { limit: 5 });
const results = await search("redis connection pooling", { limit: 10, offset: 3 });
```

| Option   | Type     | Default | Max  | Description                      |
| -------- | -------- | ------- | ---- | -------------------------------- |
| `limit`  | `number` | `10`    | `50` | Max results to return            |
| `offset` | `number` | `0`     | —    | Results to skip (for pagination) |

## MCP

Add to your MCP config to expose a `search` tool to your agents:

```json
{
  "mcpServers": {
    "statespace": {
      "command": "npx",
      "args": ["statespace", "mcp"]
    }
  }
}
```

| Parameter | Required | Default | Max  | Description                      |
| --------- | -------- | ------- | ---- | -------------------------------- |
| `q`       | yes      | —       | —    | Search query                     |
| `limit`   | no       | `10`    | `50` | Max results to return            |
| `offset`  | no       | `0`     | —    | Results to skip (for pagination) |

## API

`GET https://search.statespace.com/search` — returns a JSON object with `results` and `total`.

```bash
curl "https://search.statespace.com/search?q=mcp+server+setup"
curl "https://search.statespace.com/search?q=stripe%3A+webhook+verification&limit=5"
curl "https://search.statespace.com/search?q=redis+connection+pooling&limit=10&offset=3"
```

| Parameter | Required | Default | Max  | Description                      |
| --------- | -------- | ------- | ---- | -------------------------------- |
| `q`       | yes      | —       | —    | Search query                     |
| `limit`   | no       | `10`    | `50` | Max results to return            |
| `offset`  | no       | `0`     | —    | Results to skip (for pagination) |

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
