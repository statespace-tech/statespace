FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends curl \
    && curl -fsSL https://github.com/microsoft/go-sqlcmd/releases/latest/download/sqlcmd-linux-amd64.tar.bz2 \
       | tar -xj -C /usr/local/bin sqlcmd \
    && rm -rf /var/lib/apt/lists/*
