#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "Levantando infraestructura de FerroBox (PostgreSQL + Garage)..."
docker compose up -d

wait_for_healthy() {
  local service="$1"
  local container
  container="$(docker compose ps -q "$service")"
  echo "Esperando a que '$service' esté saludable..."
  until [ "$(docker inspect --format='{{.State.Health.Status}}' "$container" 2>/dev/null)" = "healthy" ]; do
    sleep 1
  done
  echo "  '$service' está healthy."
}

wait_for_healthy postgres
wait_for_healthy garage

echo ""
echo "Infraestructura lista:"
docker compose ps
