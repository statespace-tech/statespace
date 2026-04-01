FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends redis-tools \
    && rm -rf /var/lib/apt/lists/*
