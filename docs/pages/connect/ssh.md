---
icon: lucide/terminal
---

# SSH access

Connect directly to deployed apps for debugging and hotfixes.

> **Note**: SSH access requires a [Pro plan](https://statespace.com/pricing).

## Connect

```console
$ ssh <app-name>@ssh.statespace.com
```

Or use the CLI:

```console
$ statespace app ssh <app-name>
```

## Hotfixes

Edit files directly on the deployed instance:

```console
$ ssh myapp@ssh.statespace.com
$ vim README.md
```

Changes take effect immediately without redeploying.

!!! warning "Warning"

    SSH changes persist across environment restarts — the environment automatically syncs local changes to storage every 5 minutes and on shutdown. However, running `statespace sync` will overwrite remote files that also exist in your local directory.

## Run commands

Execute a single command without an interactive session:

```console
$ ssh myapp@ssh.statespace.com "cat README.md"
```
