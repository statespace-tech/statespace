FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends python3 python3-pip \
    && pip3 install --no-cache-dir snowflake-cli \
    && rm -rf /var/lib/apt/lists/*
