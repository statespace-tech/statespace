---
icon: lucide/cloud-upload
---

# Cloud deployment

Deploy your app to get a URL that agents can connect to.

!!! abstract "First time?"

    Create a free [Statespace](https://statespace.com) account.

## Deploy

Run `statespace deploy` to upload your app:

```console
$ statespace deploy ./myapp
Creating 'myapp'...

  ID:  myapp
  URL:  https://myapp.statespace.app
  Token:  <your-access-token>

✓ Created 'myapp'
```

!!! info "Learn more"

    Authenticate requests to your deployed app with [access tokens](security.md).

## Update

Run `statespace sync` to push changes:

```console
$ statespace sync
```

> **Note**: This creates the app if it doesn't exist, or updates it in place if it does.

## Delete

Remove a deployment:

```console
$ statespace app delete <app-name>
```

## Dependencies

Optionally, include a `Dockerfile` to customize the environment for your [tools](../develop/tools.md) and [components](../develop/components.md).

```text
myapp/
├── app.md
├── Dockerfile  # optional
└── ...
```

> **Note**: The Dockerfile is only processed when deploying or updating an app.

### Packages

Use `RUN` to install additional CLI tools. The Dockerfile is a snippet — just `RUN` and `ENV` lines, no `FROM` needed.

```dockerfile title="Dockerfile"
# Install ripgrep for fast code search
RUN apt-get update && apt-get install -y ripgrep

# Install jq for JSON processing
RUN apt-get install -y --no-install-recommends jq

# Install figlet for ASCII art
RUN apt-get install -y --no-install-recommends figlet
```

### Environment variables

Use `ENV` to set environment variables.

```dockerfile title="Dockerfile"
# Use an external hostname - localhost refers to the container itself
ENV DATABASE_URL=postgres://db.example.com:5432/mydb
ENV DEBUG=false
```
