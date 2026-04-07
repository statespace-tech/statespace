---
icon: lucide/chevrons-right
---

# CLI reference

## `statespace`

Run, deploy, and manage Statespace apps.

**Usage**

```console
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

<a id="--api-key"></a>[`--api-key`](#--api-key)
: API key override

<a id="--org-id"></a>[`--org-id`](#--org-id)
: Organization ID override

<a id="--config"></a>[`--config`](#--config)
: Path to configuration

## `statespace serve`

Run an app locally (no account required)

**Usage**

```console
statespace serve [OPTIONS] [PATH]
```

**Arguments**

<a id="path"></a>[`PATH`](#path)
: Directory to serve (default: current directory)

**Options**

<a id="--host"></a>[`--host`](#--host)
: Host to bind the server to

<a id="--port"></a>[`--port`](#--port)
: Port to bind the server to

<a id="--env"></a>[`--env`](#--env), `-e`
: Environment variables for component blocks (KEY=VALUE)

<a id="--env-file"></a>[`--env-file`](#--env-file)
: Load environment variables from a file

## `statespace deploy`

Deploy an app (create or update)

**Usage**

```console
statespace deploy [OPTIONS] [PATH]
```

**Arguments**

<a id="path"></a>[`PATH`](#path)
: Directory to deploy. If omitted, creates an empty application

**Options**

<a id="--visibility"></a>[`--visibility`](#--visibility)
: Application visibility (default: public on free-tier, otherwise private)

<a id="--name"></a>[`--name`](#--name), `-n`
: Application name. Creates a new app with a random name if omitted

<a id="--env"></a>[`--env`](#--env), `-e`
: Environment variables for deployed app secrets (KEY=VALUE)

<a id="--env-file"></a>[`--env-file`](#--env-file)
: Load deployed app secrets from a file

## `statespace app`

Application commands

**Usage**

```console
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

```console
statespace app list
```

### `statespace app get`

Show details for an application

**Usage**

```console
statespace app get <APP>
```

**Arguments**

<a id="app"></a>[`APP`](#app)
: Application name, ID, or URL

### `statespace app delete`

Delete an application

**Usage**

```console
statespace app delete [OPTIONS] <APP>
```

**Arguments**

<a id="app"></a>[`APP`](#app)
: Application name, ID, or URL

**Options**

<a id="--yes"></a>[`--yes`](#--yes), `-y`
: Skip confirmation prompt

### `statespace app restart`

Restart an application (pulls latest runtime image)

**Usage**

```console
statespace app restart <APP>
```

**Arguments**

<a id="app"></a>[`APP`](#app)
: Application name, ID, or URL

## `statespace auth`

Authentication commands

**Usage**

```console
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

```console
statespace auth login
```

### `statespace auth logout`

Log out and clear stored credentials

**Usage**

```console
statespace auth logout
```

### `statespace auth status`

Show current authentication status

**Usage**

```console
statespace auth status
```

### `statespace auth token`

Print the current API token

**Usage**

```console
statespace auth token [OPTIONS]
```

**Options**

<a id="--format"></a>[`--format`](#--format), `-f`
: Output format

## `statespace tokens`

Token management commands

**Usage**

```console
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

```console
statespace tokens create [OPTIONS] <NAME>
```

**Arguments**

<a id="name"></a>[`NAME`](#name)
: Token name

**Options**

<a id="--scope"></a>[`--scope`](#--scope), `-s`
: Token scope (read or admin)

<a id="--app-id"></a>[`--app-id`](#--app-id)
: Restrict token to specific application IDs

<a id="--expires"></a>[`--expires`](#--expires)
: Expiration (ISO 8601 datetime, e.g. 2026-12-31T00:00:00Z)

### `statespace tokens list`

List personal access tokens

**Usage**

```console
statespace tokens list [OPTIONS]
```

**Options**

<a id="--all"></a>[`--all`](#--all), `-a`
: Show all tokens including revoked

<a id="--limit"></a>[`--limit`](#--limit), `-l`
: Maximum number of tokens to return

### `statespace tokens get`

Show details for a token

**Usage**

```console
statespace tokens get <TOKEN_ID>
```

**Arguments**

<a id="token_id"></a>[`TOKEN_ID`](#token_id)
: Token ID

### `statespace tokens rotate`

Rotate a token (revoke old, issue new)

**Usage**

```console
statespace tokens rotate [OPTIONS] <TOKEN_ID>
```

**Arguments**

<a id="token_id"></a>[`TOKEN_ID`](#token_id)
: Token ID to rotate

**Options**

<a id="--name"></a>[`--name`](#--name)
: New name

<a id="--scope"></a>[`--scope`](#--scope)
: New scope (read or admin)

<a id="--app-id"></a>[`--app-id`](#--app-id)
: Restrict to specific application IDs

<a id="--expires"></a>[`--expires`](#--expires)
: New expiration (ISO 8601 datetime)

### `statespace tokens revoke`

Revoke a token

**Usage**

```console
statespace tokens revoke [OPTIONS] <TOKEN_ID>
```

**Arguments**

<a id="token_id"></a>[`TOKEN_ID`](#token_id)
: Token ID to revoke

**Options**

<a id="--reason"></a>[`--reason`](#--reason), `-r`
: Revocation reason

<a id="--yes"></a>[`--yes`](#--yes), `-y`
: Skip confirmation prompt

## `statespace docs`

Open the Statespace documentation in your browser

**Usage**

```console
statespace docs
```

## `statespace update`

Update this CLI to the latest version

**Usage**

```console
statespace update
```

