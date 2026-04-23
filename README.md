# Statespace

[![npm](https://img.shields.io/npm/v/statespace?style=flat-square)](https://www.npmjs.com/package/statespace)
[![License](https://img.shields.io/badge/license-MIT-007ec6?style=flat-square)](LICENSE)
[![Discord](https://img.shields.io/discord/1323415085011701870?label=Discord&logo=discord&logoColor=white&color=5865F2&style=flat-square)](https://discord.gg/rRyM7zkZTf)
[![X](https://img.shields.io/badge/Statespace-black?style=flat-square&logo=x&logoColor=white)](https://x.com/statespace_tech)

Search Markdown documentation indexed from [llms.txt](https://llmstxt.org/) sites.

## Query syntax

Plain queries:
```
> redis connection pooling
> vector database embedding
> rate limiting middleware

```

Queries within a site:

```
> aws: ec2 setup
> vercel: edge middleware
> supabase: security login auth
```


## CLI

```bash
npx statespace search "redis connection pooling"
npx statespace search "aws: ec2 setup" --limit 5
```

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--limit <n>` | `-l` | `10` | Max results |

## SDK

```typescript
import { search } from 'statespace';

const { results } = await search("redis connection pooling");
const { results } = await search("aws: ec2 setup", { limit: 5 });
```

Each result has `url`, `site`, `title`, and `snippet`.

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

| Parameter | Required | Default | Description |
|-----------|----------|---------|-------------|
| `q` | yes | — | Search query |
| `limit` | no | `10` | Max results |

No MCP? See [SKILL.md](SKILL.md) for using the CLI as an agent skill.

## Requirements

Node.js 18+

## Community

- **Discord**: [discord.gg/rRyM7zkZTf](https://discord.gg/rRyM7zkZTf)
- **X**: [@statespace_tech](https://x.com/statespace_tech)
- **Issues**: [GitHub Issues](https://github.com/statespace-tech/statespace/issues)

## License

MIT
