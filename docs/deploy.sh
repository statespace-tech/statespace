#!/bin/bash
# Deploy script for Fly.io documentation

set -e

cleanup() {
    echo "Cleaning up..."
    rm -f zensical.toml
}
trap cleanup EXIT

echo "Preparing deployment files..."

# Copy zensical.toml from parent directory
cp ../zensical.toml .

# Deploy to Fly
echo "Deploying to Fly.io..."
flyctl deploy

echo "Deployment complete!"
