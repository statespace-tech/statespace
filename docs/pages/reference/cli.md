---
icon: lucide/terminal
---

# CLI reference

The Statespace CLI (`statespace`) lets you deploy, manage, and connect to environments.

## Global options

These options apply to all commands:

`--api-key`
: API key override (uses stored credentials by default)

`--org-id`
: Organization ID override

## Cloud deployment

### `statespace deploy`

Deploy an app. Creates a new environment if one doesn't exist, or updates it if it does. Tracks file checksums to skip unchanged files.

```bash
statespace deploy [OPTIONS] [PATH]
```

**Arguments:**

`PATH`
: Directory containing markdown files (optional, omit to create an empty environment)

**Options:**

`--name, -n`
: Application name (default: randomly generated on first run, then reused from `.statespace/state.json`)

`--visibility`
: Application visibility: `public` or `private` (default: public on free-tier, otherwise private)

`--env, -e`
: Environment variables for component blocks (`KEY=VALUE`, can be specified multiple times)

`--env-file`
: Load environment variables from a file

**Examples:**

```bash
# Deploy from a directory
statespace deploy ./my-docs --name production

# Deploy empty environment
statespace deploy --name scratch-env

# Deploy private environment
statespace deploy ./project --visibility private

# Deploy with environment variables
statespace deploy ./my-app --env API_KEY=abc123 --env-file .env
```

## Local development

### `statespace serve`

Serve a local app for development. No account required.

```bash
statespace serve [OPTIONS] [PATH]
```

**Arguments:**

`PATH`
: Directory to serve (default: current directory)

**Options:**

`--host`
: Host to bind to (default: `127.0.0.1`)

`--port`
: Port to bind to (default: `8000`)

`--env, -e`
: Environment variables for component blocks (`KEY=VALUE`, can be specified multiple times)

`--env-file`
: Load environment variables from a file

**Example:**

```bash
statespace serve ./my-app --port 3000

# Pass environment variables
statespace serve ./my-app --env API_KEY=abc123 --env DEBUG=true

# Load environment variables from a file
statespace serve ./my-app --env-file .env
```

## Authentication

### `statespace auth login`

Log in via browser using device authorization flow. Opens a browser, waits for authorization, and saves credentials locally.

```bash
statespace auth login
```

### `statespace auth logout`

Log out and clear stored credentials.

```bash
statespace auth logout
```

### `statespace auth status`

Show current authentication status.

```bash
statespace auth status
```

Displays:

- Email and name
- User ID
- API URL
- Token expiration
- Credentials file location

### `statespace auth token`

Print the current API token for use in scripts or CI/CD.

```bash
statespace auth token [OPTIONS]
```

**Options:**

`--format, -f`
: Output format: `plain` (default) or `json`

**Examples:**

```bash
# Plain token output
statespace auth token

# JSON output with metadata
statespace auth token --format json
```

## App management

### `statespace app list`

List all applications in the current organization.

```bash
statespace app list
```

### `statespace app get`

Show details for an application.

```bash
statespace app get <APP>
```

**Arguments:**

`APP`
: Application name, ID, or URL

### `statespace app delete`

Delete an application.

```bash
statespace app delete [OPTIONS] <APP>
```

**Arguments:**

`APP`
: Application name, ID, or URL

**Options:**

`--yes, -y`
: Skip confirmation prompt

## Token management

Personal access tokens for API authentication and CI/CD integrations.

### `statespace tokens create`

Create a new personal access token.

```bash
statespace tokens create [OPTIONS] <NAME>
```

**Arguments:**

`NAME`
: Token name

**Options:**

`--scope, -s`
: Token scope: `read` (default) or `admin`

`--app-id`
: Restrict token to specific application IDs (can be specified multiple times)

`--expires`
: Expiration datetime (ISO 8601 format, e.g., `2026-12-31T00:00:00Z`)

**Examples:**

```bash
# Create a read-only token
statespace tokens create ci-readonly

# Create an admin token for specific apps
statespace tokens create deploy-token --scope admin --app-id abc123 --app-id def456

# Create a token with expiration
statespace tokens create temp-access --expires 2026-06-01T00:00:00Z
```

### `statespace tokens list`

List personal access tokens.

```bash
statespace tokens list [OPTIONS]
```

**Options:**

`--all, -a`
: Show all tokens including revoked

`--limit, -l`
: Maximum number of tokens to return (default: 100)

### `statespace tokens get`

Show details for a token.

```bash
statespace tokens get <TOKEN_ID>
```

**Arguments:**

`TOKEN_ID`
: Token ID

### `statespace tokens rotate`

Rotate a token (revoke old, issue new). The new token inherits properties from the old one unless overridden.

```bash
statespace tokens rotate [OPTIONS] <TOKEN_ID>
```

**Arguments:**

`TOKEN_ID`
: Token ID to rotate

**Options:**

`--name`
: New name

`--scope`
: New scope (`read` or `admin`)

`--app-id`
: Restrict to specific application IDs

`--expires`
: New expiration (ISO 8601 datetime)

### `statespace tokens revoke`

Revoke a token.

```bash
statespace tokens revoke [OPTIONS] <TOKEN_ID>
```

**Arguments:**

`TOKEN_ID`
: Token ID to revoke

**Options:**

`--reason, -r`
: Revocation reason

`--yes, -y`
: Skip confirmation prompt
