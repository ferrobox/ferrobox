#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [ ! -f .env ]; then
  echo "Error: .env file not found. Copy infra/.env.example to infra/.env and populate secrets." >&2
  exit 1
fi

echo "Starting FerroBox infrastructure (PostgreSQL + Garage)..."
docker compose up -d postgres garage-config garage

wait_for_healthy() {
  local service="$1"
  local container
  container="$(docker compose ps -q "$service")"
  echo "Waiting for '$service' to become healthy..."
  until [ "$(docker inspect --format='{{.State.Health.Status}}' "$container" 2>/dev/null)" = "healthy" ]; do
    sleep 1
  done
  echo "  '$service' is healthy."
}

wait_for_healthy postgres
wait_for_healthy garage

echo "Initializing Garage storage..."
docker compose up --no-deps --exit-code-from garage-init garage-init

echo "Starting FerroBox application..."
docker compose up -d ferrobox

echo ""
echo "FerroBox stack is up and running:"
docker compose ps
