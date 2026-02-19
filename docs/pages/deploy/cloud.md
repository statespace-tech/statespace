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
$ statespace deploy <path>
Deploying 'myapp'...

  URL:  https://myapp.statespace.app
  Token:  <your-access-token>

✓ Deployment complete
```

!!! info "Learn more"

    Authenticate requests to your deployed app with [access tokens](tokens.md).

## Update

Re-run `statespace deploy` to push changes:

```console
$ statespace deploy <path>
```

> **Note**: The existing deployment updates in place.

## Delete

Remove a deployment:

```console
$ statespace delete <app-id>
```

## Dependencies

Optionally, include a `Dockerfile` to customize the environment for your [tools](../develop/tools.md) and [components](../develop/components.md).

```console
myapp/
├── app.md
├── Dockerfile  # optional
└── ...
```

> **Note**: The Dockerfile is only processed when deploying or updating an app.

### Packages

Use `RUN` to install CLI tools and libraries.

```dockerfile title="Dockerfile"
# Install ripgrep for fast code search
RUN apt-get update && apt-get install -y ripgrep

# Install jq for JSON processing
RUN apt-get install -y --no-install-recommends jq

# Install Python packages
RUN pip install pandas numpy
```

### Environment variables

Use `ENV` to set environment variables.

```dockerfile title="Dockerfile"
# Use an external hostname - localhost refers to the container itself
ENV DATABASE_URL=postgres://db.example.com:5432/mydb
ENV DEBUG=false
```
