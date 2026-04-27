# Statespace

[![npm](https://img.shields.io/npm/v/statespace?style=flat-square)](https://www.npmjs.com/package/statespace)
[![License](https://img.shields.io/badge/license-MIT-007ec6?style=flat-square)](LICENSE)
[![Discord](https://img.shields.io/discord/1323415085011701870?label=Discord&logo=discord&logoColor=white&color=5865F2&style=flat-square)](https://discord.gg/rRyM7zkZTf)
[![X](https://img.shields.io/badge/Statespace-black?style=flat-square&logo=x&logoColor=white)](https://x.com/statespace_tech)

Search Markdown documentation indexed from [llms.txt](https://llmstxt.org/) sites. Available as a CLI, SDK, MCP server, and agent skill.

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

| Flag           | Short | Default | Max  | Description                           |
| -------------- | ----- | ------- | ---- | ------------------------------------- |
| `--limit <n>`  | `-l`  | `10`    | `50` | Max results to return                 |
| `--offset <n>` | `-o`  | `0`     | —    | Results to skip (for pagination)      |
| `--human`      | —     | —       | —    | Human-readable output instead of JSON |

## SDK

Import and call `search()` directly from TypeScript or JavaScript.

```typescript
import { search } from 'statespace';

const results = await search("mcp server setup");
const results = await search("stripe: webhook verification", { limit: 5 });
const results = await search("redis connection pooling", { limit: 10, offset: 3 });
```

Each result has `url`, `site`, `title`, and `snippet`.

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

Exposes one tool: `search`

| Parameter | Required | Default | Max  | Description                      |
| --------- | -------- | ------- | ---- | -------------------------------- |
| `q`       | yes      | —       | —    | Search query                     |
| `limit`   | no       | `10`    | `50` | Max results to return            |
| `offset`  | no       | `0`     | —    | Results to skip (for pagination) |

## Agent skill

See [SKILL.md](SKILL.md)

## Requirements

Node.js 18+

## Community & Contributing

- **Discord**: Join our [community server](https://discord.gg/rRyM7zkZTf) for real-time help and discussions
- **X**: Follow us [@statespace_tech](https://x.com/statespace_tech) for updates and news
- **Issues**: Report bugs or request features on [GitHub Issues](https://github.com/statespace-tech/statespace/issues)

## License

MIT
