---
icon: lucide/lock
---

# Security

Authenticate with the Statespace platform and restrict access to your deployed apps.

!!! abstract "First time?"

    Create a free [Statespace](https://statespace.com) account.

## API keys

API keys authenticate you with the Statespace platform for deploying and managing apps.

### Create an API key

Generate an API key from your account:

```console
$ statespace auth
API key: ss_key_abc123
```

### Usage

Include the API key when using the CLI:

```console
$ statespace deploy <path> --api-key <api-key>
```

Or set it as an environment variable:

```console
$ export STATESPACE_API_KEY=<api-key>
$ statespace deploy <path>
```

## Access tokens

PATs restrict agent access to your deployed apps.

### Create a token

Generate a PAT for your app:

```console
$ statespace tokens create <name>
Token created: <your-access-token>
```

### Scopes

Restrict what a token can do with `--scope`:

```console
$ statespace tokens create <name> --scope <scope>
```

| Scope     | Description                          |
|-----------|--------------------------------------|
| `read`    | Read pages only                      |
| `execute` | Read pages and invoke tools (default)|
| `admin`   | Full access                          |

### Usage

Include the token in the `Authorization` header when making requests:

```console
$ curl -H "Authorization: Bearer <token>" https://myapp.statespace.app
```

> **Note**: Requests with invalid tokens receive a `401 Unauthorized` response.

### List tokens

View all tokens associated with your account:

```console
$ statespace tokens list
```

### Revoke a token

Remove a token to prevent further access:

```console
$ statespace tokens revoke <token>
```

!!! info "Learn more"

    See the [CLI reference](../reference/cli.md) for all available commands and options.
