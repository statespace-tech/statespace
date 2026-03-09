#!/bin/bash
# Deploy script for Fly.io documentation

set -e

cleanup() {
    echo "Cleaning up..."
    rm -f zensical.toml
}
trap cleanup EXIT

echo "Preparing deployment files..."

if [ "${SKIP_DOCS_NAV_CHECK:-0}" != "1" ]; then
    echo "Validating docs nav..."
    python3 ../scripts/check_docs_nav.py
fi

# Copy zensical.toml from parent directory
cp ../zensical.toml .

# Deploy to Fly
echo "Deploying to Fly.io..."
flyctl deploy

echo "Deployment complete!"
