---
icon: lucide/settings
---

# Configuration

Running `statespace auth login` creates a config file.

```toml title="~/.config/statespace/config.toml"
[auth]
api_key = "sk_prod_abc123"
org_id = "org_789"

[profile]
email = "you@example.com"
name = "Your Name"
user_id = "user_123"

```

Override the default config for any command with `--config`:

```bash
statespace deploy ./myapp --config custom.toml
```

CLI flags take precedence over config file values:

```bash
statespace deploy ./myapp --config custom.toml --api-key sk_other_key
```

## `[auth]`

API credentials for cloud deployment:

```toml
[auth]
api_key = "sk_prod_abc123"
org_id = "org_789"
api_url = "https://api.statespace.com"  # optional
```

## `[profile]`

User info, auto-populated by running `statespace auth login`:

```toml
[profile]
email = "you@example.com"
name = "Your Name"
user_id = "user_123"
org_name = "Acme Corp"
```

