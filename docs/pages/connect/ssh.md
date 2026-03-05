---
icon: lucide/terminal
---

# SSH access

SSH directly into private apps for debugging and hotfixes.

!!! abstract "Work in progress"

    SSH access is in closed beta for pro and enterprise users. Please [contact us](https://statespace.com/contact) to request early access.

## Connect

Open an interactive shell session on your deployed app:

```console
$ ssh <app-id>@ssh.statespace.app
```

## Hotfixes

Edit files directly on the deployed instance:

```console
$ ssh <app-id>@ssh.statespace.app
$ vim README.md
```

## Run commands

Execute individual commands on your remote apps:

```console
$ ssh <app-id>@ssh.statespace.app "cat README.md"
```

## Sync files

Use `rsync` to copy files between your local machine and the deployed app:

```console
$ rsync -avz ./data/ <app-id>@ssh.statespace.app:data/
```

Pull files from the app:

```console
$ rsync -avz <app-id>@ssh.statespace.app:logs/ ./logs/
```