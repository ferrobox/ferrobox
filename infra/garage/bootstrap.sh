#!/bin/sh
# Crea el layout de un nodo, importa la clave S3 y el bucket.
# Es idempotente: un compose up repetido no falla.
set -eu

CFG="${GARAGE_CONFIG:-/config/garage.toml}"
G="/garage -c ${CFG}"
BUCKET="${S3_BUCKET:-ferrobox}"

if [ -z "${S3_ACCESS_KEY_ID:-}" ] || [ -z "${S3_SECRET_ACCESS_KEY:-}" ]; then
  echo "S3_ACCESS_KEY_ID y S3_SECRET_ACCESS_KEY son obligatorias" >&2
  exit 1
fi

echo "Esperando a Garage..."
i=0
until $G status >/dev/null 2>&1; do
  i=$((i + 1))
  if [ "$i" -gt 60 ]; then
    echo "Garage no respondió" >&2
    exit 1
  fi
  sleep 1
done

if ! $G layout show 2>/dev/null | grep -q "Current cluster layout"; then
  NODE="$($G node id -q 2>/dev/null || $G node id)"
  NODE="$(printf '%s' "$NODE" | tr -d '\r' | awk '{print $1}')"
  echo "Asignando layout al nodo ${NODE}..."
  $G layout assign -z dc1 -c 1G "$NODE" || $G layout assign -z dc1 -c 1 "$NODE"
  $G layout apply --version 1 || true
fi

if ! $G key info "$S3_ACCESS_KEY_ID" >/dev/null 2>&1; then
  echo "Importando clave S3..."
  $G key import --yes "$S3_ACCESS_KEY_ID" "$S3_SECRET_ACCESS_KEY" --name ferrobox \
    || $G key import "$S3_ACCESS_KEY_ID" "$S3_SECRET_ACCESS_KEY" \
    || true
fi

if ! $G bucket info "$BUCKET" >/dev/null 2>&1; then
  echo "Creando bucket ${BUCKET}..."
  $G bucket create "$BUCKET"
fi

$G bucket allow --read --write --key "$S3_ACCESS_KEY_ID" "$BUCKET" \
  || $G bucket allow "$BUCKET" --read --write --key "$S3_ACCESS_KEY_ID" \
  || true

echo "Garage listo: bucket ${BUCKET}"
