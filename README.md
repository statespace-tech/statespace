# statespace

[![npm](https://img.shields.io/npm/v/statespace?style=flat-square)](https://www.npmjs.com/package/statespace)
[![License](https://img.shields.io/badge/license-MIT-007ec6?style=flat-square)](LICENSE)
[![Discord](https://img.shields.io/discord/1323415085011701870?label=Discord&logo=discord&logoColor=white&color=5865F2&style=flat-square)](https://discord.gg/rRyM7zkZTf)
[![X](https://img.shields.io/badge/Statespace-black?style=flat-square&logo=x&logoColor=white)](https://x.com/statespace_tech)

Search documentation indexed from [llms.txt](https://llmstxt.org/) sites — from the terminal or your AI assistant.

## CLI

```bash
npx statespace search "redis connection pooling"
npx statespace search "supabase: edge functions"
npx statespace search "rate limiting" --limit 20
```

**Query syntax**

| Syntax | Description |
|--------|-------------|
| `<query>` | Search all indexed pages across all sites |
| `<site>: <query>` | Match site by name, search within it (e.g. `supabase: auth`) |

**Options**

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--limit <n>` | `-l` | 10 | Max results |

## MCP

Add to your MCP client config (Claude Desktop, Cursor, etc.):

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

**Tool: `search`**

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `q` | string | yes | — | Search query. Use `site: query` syntax to target a specific site. |
| `limit` | integer | no | 10 | Max results |

## Requirements

Node.js 18+

## Community

- **Discord**: [discord.gg/rRyM7zkZTf](https://discord.gg/rRyM7zkZTf)
- **X**: [@statespace_tech](https://x.com/statespace_tech)
- **Issues**: [GitHub Issues](https://github.com/statespace-tech/statespace/issues)

## License

MIT
