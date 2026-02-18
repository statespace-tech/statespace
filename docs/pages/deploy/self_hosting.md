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

RUN apt-get update && apt-get install -y curl && \
    curl -fsSL https://statespace.com/install.sh | sh

WORKDIR /app
COPY . .

EXPOSE 8000
CMD ["statespace", "serve", ".", "--host", "0.0.0.0"]
```

Build and run:

```console
$ docker build -t myapp .
$ docker run -p 8000:8000 -e API_KEY=xxx myapp
```

## Reverse proxy

Use a reverse proxy like nginx to serve your app behind a custom domain:

```nginx title="nginx.conf"
server {
    listen 80;
    server_name myapp.example.com;

    location / {
        proxy_pass http://127.0.0.1:8000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }
}
```
