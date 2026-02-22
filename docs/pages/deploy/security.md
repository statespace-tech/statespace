---
icon: lucide/lock
---

# Security

Authenticate with the Statespace platform and restrict access to your deployed apps.

!!! abstract "First time?"

    Create a free [Statespace](https://statespace.com) account.

## API keys

API keys authenticate you with the Statespace platform for deploying and managing apps.

### Log in

Authenticate via the device authorization flow:

```console
$ statespace auth login
Opening browser to authenticate...
```

### Usage

Include the API key when using the CLI:

```console
$ statespace deploy <path> --api-key <api-key>
```

## Visibility

Apps are **public** by default — anyone with the URL can access them. Set an app to **private** to require an access token for all requests:

```console
$ statespace deploy ./myapp --visibility private
```

> **Note**: Private apps require a [Pro plan](https://statespace.com/pricing). Free-tier apps are always public.

## Access tokens

Access tokens restrict agent access to your deployed apps.

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
| `read`    | Read pages only (default)            |
| `execute` | Read pages and invoke tools          |
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
