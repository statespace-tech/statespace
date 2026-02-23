---
icon: lucide/server
---

# Self-hosting

Run your app on your own infrastructure.

## Quick start

Serve your app locally:

```console
$ statespace serve <path>
Serving 'myapp' at http://127.0.0.1:8000
```

## Configuration

Bind to a specific host and port:

```console
$ statespace serve <path> --host 0.0.0.0 --port 8080
```

## Docker

Create a `Dockerfile` to containerize your app:

```dockerfile title="Dockerfile"
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends curl ca-certificates bash \
    && apt-get clean && rm -rf /var/lib/apt/lists/*

ENV STATESPACE_INSTALL_DIR=/usr/local
RUN curl -fsSL https://statespace.com/install.sh | bash

WORKDIR /app
COPY . .

EXPOSE 8000
CMD ["statespace", "serve", ".", "--host", "0.0.0.0"]
```

Build and run:

```console
$ docker build -t myapp .
$ docker run -p 8000:8000 myapp
```
