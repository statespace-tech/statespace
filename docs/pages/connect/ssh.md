---
icon: lucide/terminal
---

# SSH access

Connect directly to deployed apps for debugging, hotfixes, and file transfers.

## Connect

SSH into a deployed app:

```console
$ ssh <app-id>@ssh.statespace.com
```

## Hotfixes

Edit files directly on the deployed instance:

```console
$ ssh myapp@ssh.statespace.com
$ vim README.md
```

Changes take effect immediately without redeploying.

!!! warning "Warning"

    Hotfixes are overwritten when running `statespace update`. Commit changes locally before redeploying.

## File transfer

Copy files to your deployed app with `scp`:

```console
$ scp data.csv myapp@ssh.statespace.com:./data/
```

Copy files from your deployed app:

```console
$ scp myapp@ssh.statespace.com:./logs/error.log ./
```

## Sync directories

Use `rsync` to sync entire directories:

```console
$ rsync -avz ./scripts/ myapp@ssh.statespace.com:./scripts/
```

Sync from remote to local:

```console
$ rsync -avz myapp@ssh.statespace.com:./data/ ./backup/
```

## Run commands

Execute a single command without an interactive session

```console
$ ssh myapp@ssh.statespace.com "cat README.md"
```