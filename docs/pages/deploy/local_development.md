---
icon: lucide/server
---

# Local development

Run apps on your own machine for development or self-hosting.

## Run

Run your app locally:

```bash
statespace run <PATH>
Serving 'myapp' at http://127.0.0.1:8000
```

Optionally, bind the app to a specific host and port:

```bash
statespace run <PATH> --host 0.0.0.0 --port 8080
```

## Docker

Run apps in a container for self-hosting:

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
CMD ["statespace", "run", ".", "--host", "0.0.0.0"]
```

Build and run:

```bash
docker build -t myapp .
docker run -p 8000:8000 myapp
```
