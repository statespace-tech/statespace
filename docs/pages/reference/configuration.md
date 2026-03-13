---
icon: lucide/settings
---

# Configuration

Running `statespace auth login` creates a config file. The CLI resolves the config path in this order:

1. If `XDG_CONFIG_HOME` is set: `$XDG_CONFIG_HOME/statespace/config.toml`
2. On Windows: `%USERPROFILE%\AppData\Roaming\statespace\config.toml`
3. Otherwise: `~/.config/statespace/config.toml`

```toml title="~/.config/statespace/config.toml"
[auth]
api_key = "sk_prod_abc123"
org_id = "org_789"

[profile]
email = "you@example.com"
name = "Your Name"
user_id = "user_123"

[env]
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

User info, auto-populated by `auth login`:

```toml
[profile]
email = "you@example.com"
name = "Your Name"
user_id = "user_123"
org_name = "Acme Corp"
```

## `[env]`

Environment variables set when serving or deploying apps:

```toml
[env]
USER_NAME = "admin"
LOG_LEVEL = "debug"
```
