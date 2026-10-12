#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

MODE=s3
if [ "${1:-}" = "--filesystem" ]; then
  MODE=filesystem
elif [ $# -gt 0 ]; then
  echo "Usage: ./dev-up.sh [--filesystem]" >&2
  exit 1
fi

compose() {
  if [ "$MODE" = filesystem ]; then
    docker compose -f docker-compose.filesystem.yml "$@"
  else
    docker compose "$@"
  fi
}

# 1. Automatic .env generation if not present
if [ ! -f .env ]; then
  echo "No .env file found. Initializing from .env.example with generated secrets..."
  
  RPC_SECRET="$(openssl rand -hex 32)"
  ADMIN_TOKEN="$(openssl rand -hex 32)"
  METRICS_TOKEN="$(openssl rand -hex 32)"
  S3_KEY="GK$(openssl rand -hex 12)"
  S3_SECRET="$(openssl rand -hex 32)"
  PG_PASSWORD="$(openssl rand -hex 16)"

  ENV_CONTENT="$(cat .env.example)"
  ENV_CONTENT="${ENV_CONTENT//POSTGRES_PASSWORD=CHANGE_ME/POSTGRES_PASSWORD=${PG_PASSWORD}}"
  ENV_CONTENT="${ENV_CONTENT//GARAGE_RPC_SECRET=CHANGE_ME_openssl_rand_hex_32/GARAGE_RPC_SECRET=${RPC_SECRET}}"
  ENV_CONTENT="${ENV_CONTENT//GARAGE_ADMIN_TOKEN=CHANGE_ME_openssl_rand_base64_32/GARAGE_ADMIN_TOKEN=${ADMIN_TOKEN}}"
  ENV_CONTENT="${ENV_CONTENT//GARAGE_METRICS_TOKEN=CHANGE_ME_openssl_rand_base64_32/GARAGE_METRICS_TOKEN=${METRICS_TOKEN}}"
  ENV_CONTENT="${ENV_CONTENT//S3_ACCESS_KEY_ID=GK0123456789abcdef01234567/S3_ACCESS_KEY_ID=${S3_KEY}}"
  ENV_CONTENT="${ENV_CONTENT//S3_SECRET_ACCESS_KEY=CHANGE_ME_openssl_rand_hex_32/S3_SECRET_ACCESS_KEY=${S3_SECRET}}"

  printf '%s\n' "$ENV_CONTENT" > .env
  chmod 600 .env
  echo "  Generated secure secrets in .env successfully."
fi

wait_for_healthy() {
  local service="$1"
  local container
  container="$(compose ps -q "$service")"
  echo "Waiting for '$service' to become healthy..."
  until [ "$(docker inspect --format='{{.State.Health.Status}}' "$container" 2>/dev/null)" = "healthy" ]; do
    sleep 1
  done
  echo "  '$service' is healthy."
}

if [ "$MODE" = filesystem ]; then
  echo "Starting FerroBox (PostgreSQL + a directory for artifact bytes)..."
  compose up -d postgres
  wait_for_healthy postgres

  echo "Starting FerroBox application..."
  compose up -d --build ferrobox
  wait_for_healthy ferrobox

  echo ""
  echo "FerroBox is up. Artifact bytes are in the ferrobox_blobs volume (/var/lib/ferrobox/blobs)."
  echo "This is not a new repository type. Forge, Mirror, and Alloy are unchanged."
  echo "Web UI: http://localhost:3000"
  compose ps
  exit 0
fi

# 2. Boot infrastructure
echo "Starting FerroBox infrastructure (PostgreSQL + Garage)..."
compose up -d postgres garage-config garage

wait_for_healthy postgres
wait_for_healthy garage

# 3. Provision Garage storage
echo "Initializing Garage storage..."
compose up --no-deps --exit-code-from garage-init garage-init

# 4. Launch application
echo "Starting FerroBox application..."
# --build: the ferrobox:local image is only built once; without --build,
# `up` reuses whatever image already exists and silently ignores any
# source changes since the last build.
compose up -d --build ferrobox

wait_for_healthy ferrobox

echo ""
echo "FerroBox stack is up and running!"
echo "Web UI: http://localhost:3000"
compose ps
