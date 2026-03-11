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

## Secrets

Reference environment `$VARIABLES` in [tools](../develop/tools.md) and [components](../develop/components.md) in your apps:

````markdown title="page.md"
---
tools:
  - [psql, -U, $DB_USER, -d, $DB_NAME, -c, { }]
---

# Dashboard

```component
echo "You are answering questions to: $DB_USER"
```
````

Pass them with the CLI when serving or deploying:

```console
$ statespace serve|deploy --env DB_USER=admin --env DB_NAME=mydb
$ statespace serve|deploy --env-file .env
```

For [tool calls](../reference/api.md#post), you can pass them in the request body:

```bash
curl -X POST \
  -H "Content-Type: application/json" \
  "https://example.statespace.app/page.md" \
  -d '{
    "command": ["psql", "-U", "$DB_USER", "-d", "$DB_NAME", "-c", "SELECT 1"],
    "env": {"DB_USER": "admin", "DB_NAME": "mydb"}
  }'
```

For [page fetches](../reference/api.md#get), you can also pass them as query parameters:

```bash
curl "https://example.statespace.app/page.md?DB_USER=admin"
```

!!! warning

    Follow these best practices for managing secrets across apps:

    - Use `--env` or `--env-file` for static secrets that persist across requests (API keys, credentials, etc.)
    - Use body/query params for non-sensitive values that change with each call (user ID, session context, etc.)
    - Don't forget to add `.env` to your `.gitignore`!

!!! abstract "Work in progress"

    We're working on native support for secret managers like HashiCorp Vault, AWS Secrets Manager, and 1Password. [Contact us](https://statespace.com/contact) if you're interested in early access.
