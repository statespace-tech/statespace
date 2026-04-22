# statespace

[![npm](https://img.shields.io/npm/v/statespace?style=flat-square)](https://www.npmjs.com/package/statespace)
[![License](https://img.shields.io/badge/license-MIT-007ec6?style=flat-square)](LICENSE)
[![Discord](https://img.shields.io/discord/1323415085011701870?label=Discord&logo=discord&logoColor=white&color=5865F2&style=flat-square)](https://discord.gg/rRyM7zkZTf)
[![X](https://img.shields.io/badge/Statespace-black?style=flat-square&logo=x&logoColor=white)](https://x.com/statespace_tech)

Search documentation indexed from [llms.txt](https://llmstxt.org/) sites — from the terminal or your AI assistant.

## Query syntax

```
redis connection pooling
supabase: auth
```

Plain queries search across all sites. Prefix with a site name and `:` to search within it.

## CLI

```bash
npx statespace search "redis connection pooling"
npx statespace search "supabase: auth"
```

> **Note** `--limit <n>` / `-l` sets the number of results (default: 10).

## MCP

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

> **Note** The `search` tool accepts `q` (required) and `limit` (default: 10).

## Requirements

Node.js 18+

## Community

- **Discord**: [discord.gg/rRyM7zkZTf](https://discord.gg/rRyM7zkZTf)
- **X**: [@statespace_tech](https://x.com/statespace_tech)
- **Issues**: [GitHub Issues](https://github.com/statespace-tech/statespace/issues)

## License

MIT
