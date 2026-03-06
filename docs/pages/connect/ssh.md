---
icon: lucide/terminal
---

# SSH access

SSH directly into private apps for debugging and hotfixes.

!!! abstract "Work in progress"

    SSH access is in closed beta for pro and enterprise users. Please [contact us](https://statespace.com/contact) to request early access.

## Connect

Open an interactive shell on your deployed app:

```console
$ ssh <APP>@ssh.statespace.app
```

Once connected, you can directly edit files and install packages:

```console
$ mkdir -p data/
$ echo "# Updated" > README.md
$ apt-get install -y python3
```

## Run commands

Execute individual commands on your remote apps:

```console
$ ssh <APP>@ssh.statespace.app "cat README.md"
```

## Sync files

Use `rsync` to copy files between your local machine and the deployed app:

```console
$ rsync -avz ./data/ <APP>@ssh.statespace.app:data/
```

Pull files from the app:

```console
$ rsync -avz <APP>@ssh.statespace.app:logs/ ./logs/
```