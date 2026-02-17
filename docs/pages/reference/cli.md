---
icon: lucide/code
---

# Cloud deployment

Deploy and manage applications with Statespace's CLI.

## Quick start

```console
$ toolfront app deploy ./project --private
```

## CLI usage

### `toolfront app deploy`

Deploy your application to the cloud and get a shareable URL.

**Usage:**

```bash
toolfront app deploy [OPTIONS] PATH
```

**Arguments:**

`PATH`

: Path to application repository

**Options:**

`--name`

: Custom environment name (defaults to directory name)

`--api-key`

: Gateway API key (overrides config)

`--gateway-url`

: Gateway base URL (overrides config)

`--verify`

: Wait and verify environment is accessible after deployment

### `toolfront app list`

View all your deployed applications.

**Usage:**

```bash
toolfront app list [OPTIONS]
```

**Options:**

`--api-key`

: Gateway API key (overrides config)

`--gateway-url`

: Gateway base URL (overrides config)

### `toolfront app update`

Update an existing deployment with new markdown files.

**Usage:**

```bash
toolfront app update [OPTIONS] DEPLOYMENT_ID PATH
```

**Arguments:**

`DEPLOYMENT_ID`

: ID of app to update

`PATH`

: Path to application repository

**Options:**

`--api-key`

: Gateway API key (overrides config)

`--gateway-url`

: Gateway base URL (overrides config)

### `toolfront app delete`

Remove a deployment from the cloud.

**Usage:**

```bash
toolfront app delete [OPTIONS] DEPLOYMENT_ID
```

**Arguments:**

`DEPLOYMENT_ID`

: ID of app to delete

**Options:**

`--api-key`

: Gateway API key (overrides config)

`--gateway-url`

: Gateway base URL (overrides config)

`--yes, -y`

: Skip confirmation prompt

## Authentication

### `statespace auth login`

Authenticate with Statespace using device authorization flow. Opens a browser for login and saves credentials locally.

**Usage:**

```bash
statespace auth login
```

The command will:
- Open your browser to the authorization page
- Display a user code to enter
- Wait for authorization
- Exchange the token for an API key
- Save credentials to `~/.config/statespace/credentials.json`
- Automatically set up SSH access

### `statespace auth logout`

Log out and remove stored credentials.

**Usage:**

```bash
statespace auth logout
```

### `statespace auth status`

Display current authentication status and credentials information.

**Usage:**

```bash
statespace auth status
```

Shows:
- Email and name
- User ID
- API URL
- Token expiration
- Credentials file location

### `statespace auth token`

Output the API token for use in scripts or CI/CD.

**Usage:**

```bash
statespace auth token [OPTIONS]
```

**Options:**

`--format`

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

**Usage:**

```bash
statespace org list
```

### `statespace org current`

Display the currently selected organization.

**Usage:**

```bash
statespace org current
```

### `statespace org use`

Switch to a different organization.

**Usage:**

```bash
statespace org use [ORG]
```

**Arguments:**

`ORG`

: Organization ID or name (optional - will prompt interactively if not provided)

**Examples:**

```bash
# Interactive selection
statespace org use

# Select by name or ID
statespace org use my-organization
```

## SSH Access

### `statespace ssh setup`

Configure SSH access for connecting to environments. This command:
- Finds or generates an SSH key
- Uploads the key to Statespace
- Configures `~/.ssh/config` with Statespace settings

**Usage:**

```bash
statespace ssh setup [OPTIONS]
```

**Options:**

`--yes`

: Skip confirmation prompts

### `statespace ssh uninstall`

Remove Statespace SSH configuration from your system.

**Usage:**

```bash
statespace ssh uninstall [OPTIONS]
```

**Options:**

`--yes`

: Skip confirmation prompt

This removes:
- `~/.ssh/statespace_config` file
- Include directive from `~/.ssh/config`

## SSH Keys

### `statespace ssh-key list`

List all SSH public keys registered with your account.

**Usage:**

```bash
statespace ssh-key list
```

Shows for each key:
- Key name and ID
- Fingerprint
- Creation date

### `statespace ssh-key add`

Register a new SSH public key with your account.

**Usage:**

```bash
statespace ssh-key add [OPTIONS]
```

**Options:**

`--file`

: Path to public key file (defaults to `~/.ssh/id_ed25519.pub`, `~/.ssh/id_rsa.pub`, or `~/.ssh/id_ecdsa.pub`)

`--name`

: Custom name for the key (defaults to filename)

**Examples:**

```bash
# Add default key
statespace ssh-key add

# Add specific key with custom name
statespace ssh-key add --file ~/.ssh/custom_key.pub --name "Work Laptop"
```

### `statespace ssh-key remove`

Remove an SSH key from your account.

**Usage:**

```bash
statespace ssh-key remove FINGERPRINT
```

**Arguments:**

`FINGERPRINT`

: SSH key fingerprint (from `ssh-key list`)

## App Management

### `statespace app ssh`

Connect to an environment via SSH.

**Usage:**

```bash
statespace app ssh APP
```

**Arguments:**

`APP`

: Environment name or ID to connect to

**Examples:**

```bash
statespace app ssh my-project
```

### `statespace app sync`

Sync markdown files from a local directory to a Statespace environment. Creates or updates the environment with the latest files.

**Usage:**

```bash
statespace app sync [OPTIONS] PATH
```

**Arguments:**

`PATH`

: Path to directory containing markdown files

**Options:**

`--name`

: Custom environment name (defaults to directory name or previously synced name)

**Examples:**

```bash
# Sync current directory
statespace app sync .

# Sync specific directory with custom name
statespace app sync ./my-docs --name "production-docs"
```

The command:
- Scans for `.md` files in the directory
- Detects changes using checksums
- Creates or updates the environment
- Saves sync state for future incremental updates