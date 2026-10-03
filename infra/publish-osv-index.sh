#!/usr/bin/env bash
# Publish a ferrobox-osv-index file as a Cosign-signed OCI artifact.
#
# The layer bytes are that file. An air-gapped instance imports the same
# file with curl; it does not use docker save.
#
# Usage, from infra/, after creating an OCI Forge in the UI:
#   ./publish-osv-index.sh <repository-uuid> <index-file> <tag>
#
# Prints OSV_SYNC_REF and OSV_SYNC_PUBKEY_B64 for infra/.env.
set -euo pipefail
shopt -s inherit_errexit

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <repository-uuid> <index-file> <tag>" >&2
  exit 1
fi

REPO="$1"
INDEX="$2"
TAG="$3"
IMAGE="osv-db"
MEDIA_TYPE="application/vnd.ferrobox.osv-index.v1"
MANIFEST_TYPE="application/vnd.oci.image.manifest.v1+json"
COSIGN_TYPE="application/vnd.dev.cosign.simplesigning.v1+json"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [[ ! -f .env ]]; then
  echo "infra/.env is missing; run ./dev-up.sh first" >&2
  exit 1
fi
set -a
# shellcheck disable=SC1091
source .env
set +a

REGISTRY="${PUBLIC_BASE_URL:-http://127.0.0.1:3000}"
REGISTRY="${REGISTRY%/}"

if [[ ! -f "$INDEX" ]]; then
  echo "index file not found: $INDEX" >&2
  exit 1
fi

LOGIN_BODY="$(jq -n --arg username "$ADMIN_USERNAME" --arg password "$ADMIN_PASSWORD" '{username:$username,password:$password}')"
TOKEN="$(
  curl -fsS -X POST "$REGISTRY/api/auth/login" \
    -H 'content-type: application/json' \
    -d "$LOGIN_BODY" \
    | jq -r .token
)"
if [[ -z "$TOKEN" || "$TOKEN" == "null" ]]; then
  echo "login did not return a token" >&2
  exit 1
fi

KEY_DIR="$SCRIPT_DIR/.osv-sync"
mkdir -p "$KEY_DIR"
KEY="$KEY_DIR/osv-sync.key.pem"
PUB="$KEY_DIR/osv-sync.pub.pem"
if [[ ! -f "$KEY" ]]; then
  openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$KEY"
  openssl pkey -in "$KEY" -pubout -out "$PUB"
  chmod 600 "$KEY" "$PUB"
fi

sha256_file() {
  sha256sum "$1" | awk '{print $1}'
}

upload_blob() {
  local file="$1"
  local hex digest
  hex="$(sha256_file "$file")"
  digest="sha256:${hex}"
  curl -fsS -X POST \
    -H "authorization: Bearer ${TOKEN}" \
    -H "content-type: application/octet-stream" \
    --data-binary @"$file" \
    "${REGISTRY}/v2/${REPO}/${IMAGE}/blobs/uploads/?digest=sha256%3A${hex}" \
    >/dev/null
  printf '%s' "$digest"
}

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

printf '%s' '{}' >"$WORKDIR/empty.json"
EMPTY_DIGEST="$(upload_blob "$WORKDIR/empty.json")"
INDEX_DIGEST="$(upload_blob "$INDEX")"
INDEX_SIZE="$(wc -c <"$INDEX" | tr -d ' ')"

printf '%s' \
  "{\"schemaVersion\":2,\"mediaType\":\"${MANIFEST_TYPE}\",\"artifactType\":\"${MEDIA_TYPE}\",\"config\":{\"mediaType\":\"application/vnd.oci.empty.v1+json\",\"digest\":\"${EMPTY_DIGEST}\",\"size\":2},\"layers\":[{\"mediaType\":\"${MEDIA_TYPE}\",\"digest\":\"${INDEX_DIGEST}\",\"size\":${INDEX_SIZE}}]}" \
  >"$WORKDIR/index.manifest.json"

MANIFEST_HEADERS="$WORKDIR/manifest.headers"
curl -fsS -D "$MANIFEST_HEADERS" -o /dev/null -X PUT \
  -H "authorization: Bearer ${TOKEN}" \
  -H "content-type: ${MANIFEST_TYPE}" \
  --data-binary @"$WORKDIR/index.manifest.json" \
  "${REGISTRY}/v2/${REPO}/${IMAGE}/manifests/${TAG}"
MANIFEST_DIGEST="sha256:$(sha256_file "$WORKDIR/index.manifest.json")"
DECLARED="$(awk 'tolower($1)=="docker-content-digest:" {print $2}' "$MANIFEST_HEADERS" | tr -d '\r')"
if [[ -n "$DECLARED" && "$DECLARED" != "$MANIFEST_DIGEST" ]]; then
  echo "manifest digest mismatch: local ${MANIFEST_DIGEST}, registry ${DECLARED}" >&2
  exit 1
fi

printf '%s' \
  "{\"critical\":{\"identity\":{\"docker-reference\":\"${IMAGE}\"},\"image\":{\"docker-manifest-digest\":\"${MANIFEST_DIGEST}\"},\"type\":\"cosign container image signature\"}}" \
  >"$WORKDIR/payload.json"
PAYLOAD_DIGEST="$(upload_blob "$WORKDIR/payload.json")"
PAYLOAD_SIZE="$(wc -c <"$WORKDIR/payload.json" | tr -d ' ')"
MANIFEST_SIZE="$(wc -c <"$WORKDIR/index.manifest.json" | tr -d ' ')"
openssl dgst -sha256 -sign "$KEY" -out "$WORKDIR/sig.bin" "$WORKDIR/payload.json"
SIG_B64="$(openssl base64 -A -in "$WORKDIR/sig.bin")"
HEX="${MANIFEST_DIGEST#sha256:}"
SIG_TAG="sha256-${HEX}.sig"

printf '%s' \
  "{\"schemaVersion\":2,\"mediaType\":\"${MANIFEST_TYPE}\",\"artifactType\":\"${COSIGN_TYPE}\",\"config\":{\"mediaType\":\"application/vnd.oci.empty.v1+json\",\"digest\":\"${PAYLOAD_DIGEST}\",\"size\":${PAYLOAD_SIZE}},\"layers\":[{\"mediaType\":\"${COSIGN_TYPE}\",\"digest\":\"${PAYLOAD_DIGEST}\",\"size\":${PAYLOAD_SIZE},\"annotations\":{\"dev.cosignproject.cosign/signature\":\"${SIG_B64}\"}}],\"subject\":{\"mediaType\":\"${MANIFEST_TYPE}\",\"digest\":\"${MANIFEST_DIGEST}\",\"size\":${MANIFEST_SIZE}}}" \
  >"$WORKDIR/sig.manifest.json"

curl -fsS -o /dev/null -X PUT \
  -H "authorization: Bearer ${TOKEN}" \
  -H "content-type: ${MANIFEST_TYPE}" \
  --data-binary @"$WORKDIR/sig.manifest.json" \
  "${REGISTRY}/v2/${REPO}/${IMAGE}/manifests/${SIG_TAG}"

PUB_B64="$(openssl base64 -A -in "$PUB")"
echo "OSV_SYNC_REF=${REGISTRY}/${REPO}/${IMAGE}:${TAG}"
echo "OSV_SYNC_PUBKEY_B64=${PUB_B64}"
