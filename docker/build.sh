#!/usr/bin/env bash
# ============================================================
# Build Linux amd64 binaries + UI for Docker packaging.
# Run from the repository root: bash docker/build.sh
#
# Outputs everything into dist/:
#   dist/forge-server
#   dist/forge-web
#   dist/ui/
# ============================================================
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# On Windows (Git Bash / MSYS2), prevent path mangling
export MSYS_NO_PATHCONV=1

echo "==> Building Rust binaries (forge-server, forge-web) in Docker..."
docker run --rm \
  -v "$REPO_ROOT:/src" \
  -w /src \
  rust:1-bookworm \
  bash -c '
    apt-get update -qq && apt-get install -y -qq protobuf-compiler > /dev/null 2>&1 && \
    cargo build --release --bin forge-server --bin forge-web
  '

echo "==> Building Web UI..."
docker run --rm \
  -v "$REPO_ROOT/crates/forge-web/ui:/app" \
  -w /app \
  node:20-alpine \
  sh -c "npm ci --silent && npm run build"

echo "==> Collecting artifacts into dist/..."
rm -rf dist
mkdir -p dist/ui

cp target/release/forge-server dist/forge-server
cp target/release/forge-web    dist/forge-web
cp -r crates/forge-web/ui/dist/* dist/ui/

echo "==> Done. Now run: docker compose build && docker compose up -d"
