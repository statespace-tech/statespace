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

Each result is printed as `[Site] Title — URL`:

```
[Upstash] Redis Quickstart — https://upstash.com/docs/redis/quickstart
[Supabase] Row Level Security — https://supabase.com/docs/guides/auth/row-level-security
```

Use the URLs to fetch the full documentation page.
