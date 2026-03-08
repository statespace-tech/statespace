---
icon: lucide/lock
---

# Security

Manage authentication, access control, and secrets for your apps.

!!! info "First time?"

    You'll need a Statespace account to manage and secure your apps. [Create a free account](https://statespace.com/auth/login) to get started.

## API keys

Run `statespace auth login` to authenticate and save an API key locally:

```console
$ statespace auth login
Requesting authorization...

Open this URL in your browser:

  https://statespace.com/auth/device?code=QK8F-9FTJ

And enter code: QK8F-9FTJ
```

Once logged in, all relevant [CLI commands](../reference/cli.md) use the saved credentials automatically:

```console
$ statespace deploy <PATH>
$ statespace app list
$ statespace app delete <APP>
```

Alternatively, pass an API key directly with `--api-key`:

```console
$ statespace app list --api-key <API_KEY>
```

## Access tokens

Use tokens to control access to your private apps:

```console
$ statespace tokens create <NAME> --scope <SCOPE>
```

Tokens can be configured with three scopes:

| Scope     | Description                          |
|-----------|--------------------------------------|
| `read`    | Read pages only (default)            |
| `execute` | Read pages and call tools            |
| `admin`   | Full access                          |

Include the token in the `Authorization` header:

```console
$ curl -H "Authorization: Bearer <TOKEN>" https://myapp.statespace.app
```

You can list, rotate, and revoke tokens:

```console
$ statespace tokens list
$ statespace tokens rotate <TOKEN>
$ statespace tokens revoke <TOKEN>
```
