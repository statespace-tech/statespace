---
icon: lucide/terminal
---

# CLI Reference

The Statespace CLI (`statespace`) lets you deploy, manage, and connect to environments.

## Global Options

These options apply to all commands:

`--api-key`
: API key override (uses stored credentials by default)

`--org-id`
: Organization ID override

## Local Development

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

**Example:**

```bash
statespace serve ./my-app --port 3000
```

## Authentication

### `statespace auth login`

Log in via browser using device authorization flow. Opens a browser, waits for authorization, and saves credentials locally. Also sets up SSH access automatically.

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

## Organizations

### `statespace org list`

List all organizations you have access to. Current organization is marked with `*`.

```bash
statespace org list
```

### `statespace org current`

Display the currently selected organization.

```bash
statespace org current
```

### `statespace org use`

Switch to a different organization.

```bash
statespace org use [ORG]
```

**Arguments:**

`ORG`
: Organization name or ID (optional — prompts interactively if omitted)

**Examples:**

```bash
# Interactive selection
statespace org use

# Select by name or ID
statespace org use my-organization
```

## App Management

### `statespace app create`

Create a new environment, optionally with markdown files.

```bash
statespace app create [OPTIONS] [PATH]
```

**Arguments:**

`PATH`
: Directory containing markdown files (optional — omit to create an empty environment)

**Options:**

`--name, -n`
: Environment name (default: directory name, or randomly generated)

`--visibility`
: Environment visibility: `public` (default) or `private`

`--verify`
: Wait for the environment to become ready

**Examples:**

```bash
# Create from a directory
statespace app create ./my-docs --name production

# Create empty environment
statespace app create --name scratch-env

# Create private environment
statespace app create ./project --visibility private --verify
```

### `statespace app list`

List all environments in the current organization.

```bash
statespace app list
```

### `statespace app get`

Show details for an environment.

```bash
statespace app get <ID>
```

**Arguments:**

`ID`
: Environment ID or name

### `statespace app delete`

Delete an environment.

```bash
statespace app delete [OPTIONS] <ID>
```

**Arguments:**

`ID`
: Environment ID or name

**Options:**

`--yes, -y`
: Skip confirmation prompt

### `statespace app sync`

Sync markdown files to an environment. Creates the environment if it doesn't exist, or updates it if it does. Tracks file checksums to skip unchanged files.

```bash
statespace app sync [OPTIONS] [PATH]
```

**Arguments:**

`PATH`
: Directory to sync (default: current directory)

**Options:**

`--name, -n`
: Environment name (default: directory name or previously synced name)

**Examples:**

```bash
# Sync current directory
statespace app sync

# Sync specific directory with custom name
statespace app sync ./docs --name my-docs
```

### `statespace app ssh`

Connect to an environment via SSH.

```bash
statespace app ssh <APP>
```

**Arguments:**

`APP`
: Environment name or ID

**Examples:**

```bash
statespace app ssh my-project
```

## SSH Configuration

### `statespace ssh setup`

Configure SSH for native `ssh`, `scp`, and `rsync` access. This command:

- Finds or generates an SSH key
- Uploads the key to Statespace
- Configures `~/.ssh/config` with Statespace settings

```bash
statespace ssh setup [OPTIONS]
```

**Options:**

`--yes`
: Skip confirmation prompts

After setup, you can use:

```bash
ssh env@<environment>.statespace
scp file.txt env@<environment>.statespace:~
rsync -av ./dir env@<environment>.statespace:~
```

### `statespace ssh uninstall`

Remove Statespace SSH configuration from your system.

```bash
statespace ssh uninstall [OPTIONS]
```

**Options:**

`--yes`
: Skip confirmation prompt

This removes:

- `~/.ssh/statespace_config` file
- Include directive from `~/.ssh/config`

### `statespace ssh keys list`

List all SSH public keys registered with your account.

```bash
statespace ssh keys list
```

Shows for each key:

- Key name and ID
- Fingerprint
- Creation date

### `statespace ssh keys add`

Register a new SSH public key with your account.

```bash
statespace ssh keys add [OPTIONS]
```

**Options:**

`--file, -f`
: Path to public key file (default: `~/.ssh/id_ed25519.pub`, `~/.ssh/id_rsa.pub`, or `~/.ssh/id_ecdsa.pub`)

`--name, -n`
: Custom name for the key (default: derived from key comment or filename)

**Examples:**

```bash
# Add default key
statespace ssh keys add

# Add specific key with custom name
statespace ssh keys add --file ~/.ssh/work_key.pub --name "Work Laptop"
```

### `statespace ssh keys remove`

Remove an SSH key from your account.

```bash
statespace ssh keys remove <FINGERPRINT>
```

**Arguments:**

`FINGERPRINT`
: SSH key fingerprint (from `ssh keys list`)

## Token Management

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
: Token scope: `read`, `execute` (default), or `admin`

`--app-id`
: Restrict token to specific environment IDs (can be specified multiple times)

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
: Restrict to specific environment IDs

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
