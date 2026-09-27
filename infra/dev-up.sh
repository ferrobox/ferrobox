#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [ ! -f .env ]; then
  echo "Copia infra/.env.example a infra/.env y rellena los secretos." >&2
  exit 1
fi

echo "Levantando infraestructura de FerroBox (PostgreSQL + Garage)..."
echo "Para la imagen completa (API + UI): docker compose up --build"
docker compose up -d postgres garage-config garage

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
docker compose up --no-deps --exit-code-from garage-init garage-init

echo ""
echo "Infraestructura lista:"
docker compose ps
