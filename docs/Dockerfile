FROM python:3.11-slim

RUN apt-get update && apt-get install -y \
    libcairo2 \
    libcairo2-dev \
    libpango-1.0-0 \
    libpangocairo-1.0-0 \
    libgdk-pixbuf-2.0-0 \
    libffi-dev \
    shared-mime-info \
    git \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN pip install --no-cache-dir uv==0.10.4 && uv pip install --system --no-cache zensical==0.0.23

WORKDIR /app

COPY zensical.toml ./

COPY pages ./docs/pages
COPY assets ./docs/assets
COPY index.md install.md ./docs/

EXPOSE 8080

CMD ["zensical", "serve", "--dev-addr", "0.0.0.0:8080"]
