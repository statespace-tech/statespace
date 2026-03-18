---
icon: lucide/terminal
---

# CLI reference

## `statespace`

Run, deploy, and manage Statespace apps.

**Usage**

```
statespace [OPTIONS] <COMMAND>
```

**Commands**

[`statespace serve`](#statespace-serve)
: Run an app locally (no account required)

[`statespace deploy`](#statespace-deploy)
: Deploy an app (create or update)

[`statespace app`](#statespace-app)
: Application commands

[`statespace auth`](#statespace-auth)
: Authentication commands

[`statespace tokens`](#statespace-tokens)
: Token management commands

[`statespace docs`](#statespace-docs)
: Open the Statespace documentation in your browser

[`statespace update`](#statespace-update)
: Update this CLI to the latest version

**Global options**

`--api-key`
: API key override

`--org-id`
: Organization ID override

`--config`
: Path to configuration

## `statespace serve`

Run an app locally (no account required)

**Usage**

```
statespace serve [OPTIONS] [PATH]
```

**Arguments**

`PATH`
: Directory to serve (default: current directory)

**Options**

`--host`
: Host to bind the server to

`--port`
: Port to bind the server to

`--env, -e`
: Environment variables for component blocks (KEY=VALUE)

`--env-file`
: Load environment variables from a file

## `statespace deploy`

Deploy an app (create or update)

**Usage**

```
statespace deploy [OPTIONS] [PATH]
```

**Arguments**

`PATH`
: Directory to deploy. If omitted, creates an empty application

**Options**

`--visibility`
: Application visibility (default: public on free-tier, otherwise private)

`--name, -n`
: Application name. Creates a new app with a random name if omitted

`--env, -e`
: Environment variables for deployed app secrets (KEY=VALUE)

`--env-file`
: Load deployed app secrets from a file

## `statespace app`

Application commands

**Usage**

```
statespace app <COMMAND>
```

**Commands**

[`statespace app list`](#statespace-app-list)
: List all applications

[`statespace app get`](#statespace-app-get)
: Show details for an application

[`statespace app delete`](#statespace-app-delete)
: Delete an application

[`statespace app restart`](#statespace-app-restart)
: Restart an application (pulls latest runtime image)

### `statespace app list`

List all applications

**Usage**

```
statespace app list
```

### `statespace app get`

Show details for an application

**Usage**

```
statespace app get <APP>
```

**Arguments**

`APP`
: Application name, ID, or URL

### `statespace app delete`

Delete an application

**Usage**

```
statespace app delete [OPTIONS] <APP>
```

**Arguments**

`APP`
: Application name, ID, or URL

**Options**

`--yes, -y`
: Skip confirmation prompt

### `statespace app restart`

Restart an application (pulls latest runtime image)

**Usage**

```
statespace app restart <APP>
```

**Arguments**

`APP`
: Application name, ID, or URL

## `statespace auth`

Authentication commands

**Usage**

```
statespace auth <COMMAND>
```

**Commands**

[`statespace auth login`](#statespace-auth-login)
: Log in via browser (device auth flow)

[`statespace auth logout`](#statespace-auth-logout)
: Log out and clear stored credentials

[`statespace auth status`](#statespace-auth-status)
: Show current authentication status

[`statespace auth token`](#statespace-auth-token)
: Print the current API token

### `statespace auth login`

Log in via browser (device auth flow)

**Usage**

```
statespace auth login
```

### `statespace auth logout`

Log out and clear stored credentials

**Usage**

```
statespace auth logout
```

### `statespace auth status`

Show current authentication status

**Usage**

```
statespace auth status
```

### `statespace auth token`

Print the current API token

**Usage**

```
statespace auth token [OPTIONS]
```

**Options**

`--format, -f`
: Output format

## `statespace tokens`

Token management commands

**Usage**

```
statespace tokens <COMMAND>
```

**Commands**

[`statespace tokens create`](#statespace-tokens-create)
: Create a new personal access token

[`statespace tokens list`](#statespace-tokens-list)
: List personal access tokens

[`statespace tokens get`](#statespace-tokens-get)
: Show details for a token

[`statespace tokens rotate`](#statespace-tokens-rotate)
: Rotate a token (revoke old, issue new)

[`statespace tokens revoke`](#statespace-tokens-revoke)
: Revoke a token

### `statespace tokens create`

Create a new personal access token

**Usage**

```
statespace tokens create [OPTIONS] <NAME>
```

**Arguments**

`NAME`
: Token name

**Options**

`--scope, -s`
: Token scope (read or admin)

`--app-id`
: Restrict token to specific application IDs

`--expires`
: Expiration (ISO 8601 datetime, e.g. 2026-12-31T00:00:00Z)

### `statespace tokens list`

List personal access tokens

**Usage**

```
statespace tokens list [OPTIONS]
```

**Options**

`--all, -a`
: Show all tokens including revoked

`--limit, -l`
: Maximum number of tokens to return

### `statespace tokens get`

Show details for a token

**Usage**

```
statespace tokens get <TOKEN_ID>
```

**Arguments**

`TOKEN_ID`
: Token ID

### `statespace tokens rotate`

Rotate a token (revoke old, issue new)

**Usage**

```
statespace tokens rotate [OPTIONS] <TOKEN_ID>
```

**Arguments**

`TOKEN_ID`
: Token ID to rotate

**Options**

`--name`
: New name

`--scope`
: New scope (read or admin)

`--app-id`
: Restrict to specific application IDs

`--expires`
: New expiration (ISO 8601 datetime)

### `statespace tokens revoke`

Revoke a token

**Usage**

```
statespace tokens revoke [OPTIONS] <TOKEN_ID>
```

**Arguments**

`TOKEN_ID`
: Token ID to revoke

**Options**

`--reason, -r`
: Revocation reason

`--yes, -y`
: Skip confirmation prompt

## `statespace docs`

Open the Statespace documentation in your browser

**Usage**

```
statespace docs
```

## `statespace update`

Update this CLI to the latest version

**Usage**

```
statespace update
```

