---
icon: lucide/lock
---

# Security

Manage authentication and access control for your apps.

!!! info "First time?"

    You'll need a Statespace account to manage and secure your apps. [Create a free account](https://statespace.com/auth/login) to get started.

## API keys

Run `statespace auth login` to authenticate and save your API key locally:

```console
$ statespace auth login
Requesting authorization...

Open this URL in your browser:

  https://statespace.com/auth/device?code=QK8F-9FTJ

And enter code: QK8F-9FTJ
```

Once logged in, all CLI commands use the saved credentials automatically:

```console
$ statespace deploy ./myapp
$ statespace app list
$ statespace app delete <app-id>
```

Alternatively, set the `STATESPACE_API_KEY` environment variable:

```console
$ export STATESPACE_API_KEY=<api-key>
$ statespace deploy <path>
```

!!! warning "Avoid passing API keys as command-line arguments"

    CLI arguments are visible in process listings and shell history. Use `statespace auth login` or the `STATESPACE_API_KEY` environment variable instead.

## Access tokens

Access tokens control access to your private apps:

```console
$ statespace tokens create <name>
Token created: <your-access-token>
```

Restrict what a token can do with `--scope`:

```console
$ statespace tokens create <name> --scope <scope>
```

| Scope     | Description                          |
|-----------|--------------------------------------|
| `read`    | Read pages only (default)            |
| `execute` | Read pages and call tools            |
| `admin`   | Full access                          |


Include the token in the `Authorization` header:

```console
$ curl -H "Authorization: Bearer <token>" https://myapp.statespace.app
```

You can list, rotate, and revoke tokens:

```console
$ statespace tokens list
$ statespace tokens rotate <token>
$ statespace tokens revoke <token>
```