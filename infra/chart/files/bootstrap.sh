#!/bin/sh
# Crea el layout de un nodo, importa la clave S3 y el bucket.
# Es idempotente: un compose up o un helm upgrade no falla.
# La imagen oficial de Garage es FROM scratch (sin shell ni CLI usable
# desde otro contenedor), así que esto habla con la admin API v2.
# Compose monta infra/garage/bootstrap.sh, que apunta aquí.
set -eu

ADMIN="${GARAGE_ADMIN_URL:-http://garage:3903}"
BUCKET="${S3_BUCKET:-ferrobox}"
# 1 GB SI, como `garage layout assign -c 1G`.
CAPACITY="${GARAGE_CAPACITY_BYTES:-1000000000}"

if [ -z "${GARAGE_ADMIN_TOKEN:-}" ]; then
  echo "GARAGE_ADMIN_TOKEN es obligatoria" >&2
  exit 1
fi

if [ -z "${S3_ACCESS_KEY_ID:-}" ] || [ -z "${S3_SECRET_ACCESS_KEY:-}" ]; then
  echo "S3_ACCESS_KEY_ID y S3_SECRET_ACCESS_KEY son obligatorias" >&2
  exit 1
fi

apk add --no-cache curl jq >/dev/null

admin() {
  method="$1"
  path="$2"
  body="${3:-}"
  if [ -n "$body" ]; then
    curl -sS -f -X "$method" \
      -H "Authorization: Bearer ${GARAGE_ADMIN_TOKEN}" \
      -H "Content-Type: application/json" \
      -d "$body" \
      "${ADMIN}${path}"
  else
    curl -sS -f -X "$method" \
      -H "Authorization: Bearer ${GARAGE_ADMIN_TOKEN}" \
      "${ADMIN}${path}"
  fi
}

admin_code() {
  method="$1"
  path="$2"
  curl -s -o /dev/null -w "%{http_code}" -X "$method" \
    -H "Authorization: Bearer ${GARAGE_ADMIN_TOKEN}" \
    "${ADMIN}${path}" || true
}

echo "Esperando a Garage en ${ADMIN}..."
i=0
code="$(admin_code GET /v2/GetClusterStatus)"
until [ "$code" = "200" ]; do
  i=$((i + 1))
  if [ "$i" -gt 180 ]; then
    echo "Garage no respondió (último HTTP ${code}) en ${ADMIN}" >&2
    exit 1
  fi
  if [ $((i % 15)) -eq 0 ]; then
    echo "  aún no listo (HTTP ${code}), ${i}s..."
  fi
  sleep 1
  code="$(admin_code GET /v2/GetClusterStatus)"
done

STATUS="$(admin GET /v2/GetClusterStatus)"
NODE="$(printf '%s' "$STATUS" | jq -r '.nodes[] | select(.isUp == true) | .id' | head -n1)"
if [ -z "$NODE" ] || [ "$NODE" = "null" ]; then
  echo "No hay nodos Garage activos" >&2
  exit 1
fi

LAYOUT="$(admin GET /v2/GetClusterLayout)"
ROLES="$(printf '%s' "$LAYOUT" | jq '.roles | length')"
STAGED="$(printf '%s' "$LAYOUT" | jq '.stagedRoleChanges | length')"
VERSION="$(printf '%s' "$LAYOUT" | jq -r '.version')"

if [ "$ROLES" -eq 0 ]; then
  if [ "$STAGED" -eq 0 ]; then
    echo "Asignando layout al nodo ${NODE}..."
    admin POST /v2/UpdateClusterLayout "$(
      jq -n --arg id "$NODE" --argjson capacity "$CAPACITY" \
        '{roles:[{id:$id,zone:"dc1",capacity:$capacity,tags:["ferrobox"]}]}'
    )" >/dev/null
  fi
  NEXT=$((VERSION + 1))
  echo "Aplicando layout versión ${NEXT}..."
  admin POST /v2/ApplyClusterLayout "$(jq -n --argjson version "$NEXT" '{version:$version}')" >/dev/null
fi

KEY_CODE="$(admin_code GET "/v2/GetKeyInfo?id=${S3_ACCESS_KEY_ID}")"
if [ "$KEY_CODE" != "200" ]; then
  echo "Importando clave S3..."
  admin POST /v2/ImportKey "$(
    jq -n \
      --arg accessKeyId "$S3_ACCESS_KEY_ID" \
      --arg secretAccessKey "$S3_SECRET_ACCESS_KEY" \
      '{accessKeyId:$accessKeyId,secretAccessKey:$secretAccessKey,name:"ferrobox"}'
  )" >/dev/null
fi

BUCKET_ID=""
if [ "$(admin_code GET "/v2/GetBucketInfo?globalAlias=${BUCKET}")" = "200" ]; then
  BUCKET_ID="$(admin GET "/v2/GetBucketInfo?globalAlias=${BUCKET}" | jq -r '.id')"
fi
if [ -z "$BUCKET_ID" ] || [ "$BUCKET_ID" = "null" ]; then
  echo "Creando bucket ${BUCKET}..."
  BUCKET_JSON="$(
    admin POST /v2/CreateBucket "$(
      jq -n --arg globalAlias "$BUCKET" '{globalAlias:$globalAlias}'
    )"
  )"
  BUCKET_ID="$(printf '%s' "$BUCKET_JSON" | jq -r '.id')"
fi

admin POST /v2/AllowBucketKey "$(
  jq -n \
    --arg accessKeyId "$S3_ACCESS_KEY_ID" \
    --arg bucketId "$BUCKET_ID" \
    '{accessKeyId:$accessKeyId,bucketId:$bucketId,permissions:{read:true,write:true,owner:true}}'
)" >/dev/null

echo "Garage listo: bucket ${BUCKET}"
