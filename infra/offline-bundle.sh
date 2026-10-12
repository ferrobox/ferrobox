#!/usr/bin/env bash
# Build a directory you can carry to a machine that cannot reach a registry.
#
# Connected machine (needs Docker, and the network):
#   ./offline-bundle.sh --version 0.2.4 \
#     --osv-ref ghcr.io/ferrobox/osv-db:2026-10-12 \
#     --expect-ferrobox-digest sha256:cec6f297ba140724b6208d5118a893226bb1fb66d57616ad1920448684bb6591 \
#     --out "$HOME/ferrobox-offline"
#
# The directory contains the FerroBox image, the postgres:18-alpine image,
# the Helm chart, the vulnerability index, osv-sync.pub.pem, and SHA256SUMS.
# Garage is not included. A container scanner is not included.
# On the offline machine: sha256sum -c SHA256SUMS && ./install.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Fallbacks when the binaries are not on PATH. Cosign 2.6.5 is the 2.x
# release that includes the legacy-bundle verification fix. Helm 3.16.4 is
# the version that publishes this chart. Both image tags exist on the
# registries; the script does not pull them when the local binary exists.
COSIGN_IMAGE="${COSIGN_IMAGE:-ghcr.io/sigstore/cosign/cosign:v2.6.5}"
ORAS_IMAGE="${ORAS_IMAGE:-ghcr.io/oras-project/oras:v1.2.2}"
HELM_IMAGE="${HELM_IMAGE:-alpine/helm:3.16.4}"

usage() {
  cat <<'EOF'
Usage:
  offline-bundle.sh --version VERSION --osv-ref REF --out DIR
                    [--expect-ferrobox-digest sha256:HEX]
                    [--platform linux/amd64|linux/arm64]
  offline-bundle.sh --assemble --version VERSION --osv-ref REF --out DIR \
                    --ferrobox-tar FILE --ferrobox-digest sha256:HEX \
                    --postgres-tar FILE --postgres-digest sha256:HEX \
                    --chart FILE --index FILE --index-sha FILE \
                    [--ferrobox-platform linux/amd64|linux/arm64]

--version is the FerroBox image tag without a leading v (0.2.4).
--osv-ref is the signed index, for example ghcr.io/ferrobox/osv-db:2026-10-12.
--expect-ferrobox-digest is the manifest-list digest on the GitHub Release.
--platform chooses which image docker save writes. The default is the
architecture of this machine. The assemble form only packs files you
already have. It is what the test uses.
EOF
}

die() {
  echo "offline-bundle: $*" >&2
  exit 1
}

need_value() {
  if [ $# -lt 2 ] || [ -z "${2:-}" ]; then
    die "missing value for $1"
  fi
}

VERSION=""
OSV_REF=""
OUT=""
EXPECT_FERROBOX_DIGEST=""
ASSEMBLE=0
FERROBOX_TAR=""
FERROBOX_DIGEST=""
POSTGRES_TAR=""
POSTGRES_DIGEST=""
CHART_FILE=""
INDEX_FILE=""
INDEX_SHA_FILE=""
OSV_MANIFEST_DIGEST=""
MANIFEST_FROM_JSON=0
DIGEST_FROM_HEADERS=0
PLATFORM_DIGEST_OSARCH=""
PLATFORM=""
FERROBOX_PLATFORM=""
FERROBOX_PLATFORM_DIGEST=""

while [ $# -gt 0 ]; do
  case "$1" in
    --version)
      need_value "$@"
      VERSION="$2"
      shift 2
      ;;
    --osv-ref)
      need_value "$@"
      OSV_REF="$2"
      shift 2
      ;;
    --out)
      need_value "$@"
      OUT="$2"
      shift 2
      ;;
    --expect-ferrobox-digest)
      need_value "$@"
      EXPECT_FERROBOX_DIGEST="$2"
      shift 2
      ;;
    --platform|--ferrobox-platform)
      need_value "$@"
      PLATFORM="$2"
      shift 2
      ;;
    --assemble)
      ASSEMBLE=1
      shift
      ;;
    --ferrobox-tar)
      need_value "$@"
      FERROBOX_TAR="$2"
      shift 2
      ;;
    --ferrobox-digest)
      need_value "$@"
      FERROBOX_DIGEST="$2"
      shift 2
      ;;
    --postgres-tar)
      need_value "$@"
      POSTGRES_TAR="$2"
      shift 2
      ;;
    --postgres-digest)
      need_value "$@"
      POSTGRES_DIGEST="$2"
      shift 2
      ;;
    --chart)
      need_value "$@"
      CHART_FILE="$2"
      shift 2
      ;;
    --index)
      need_value "$@"
      INDEX_FILE="$2"
      shift 2
      ;;
    --index-sha)
      need_value "$@"
      INDEX_SHA_FILE="$2"
      shift 2
      ;;
    # Reads Cosign's JSON on stdin and prints the manifest digest.
    # The bundle test uses this. It does not pack a directory.
    --manifest-digest-from-json)
      MANIFEST_FROM_JSON=1
      shift
      ;;
    # Reads HTTP response headers on stdin and prints Docker-Content-Digest.
    --digest-from-headers)
      DIGEST_FROM_HEADERS=1
      shift
      ;;
    # Reads an OCI index on stdin and prints the manifest for OS/ARCH.
    --platform-digest-from-index)
      need_value "$@"
      PLATFORM_DIGEST_OSARCH="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

command -v python3 >/dev/null 2>&1 || die "python3 is required"
export LC_ALL=C

if [ "$MANIFEST_FROM_JSON" -eq 0 ] && [ "$DIGEST_FROM_HEADERS" -eq 0 ] && [ -z "$PLATFORM_DIGEST_OSARCH" ]; then
  [ -n "$VERSION" ] || die "--version is required"
  [ -n "$OSV_REF" ] || die "--osv-ref is required"
  [ -n "$OUT" ] || die "--out is required"
  [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "--version must look like 0.2.4, without a leading v"
  [[ "$OSV_REF" =~ ^[A-Za-z0-9._/-]+:[A-Za-z0-9._-]+$ ]] || die "--osv-ref must be a registry tag, not a digest"
  if [ -n "$PLATFORM" ]; then
    [[ "$PLATFORM" =~ ^linux/(amd64|arm64)$ ]] || die "--platform must be linux/amd64 or linux/arm64"
    FERROBOX_PLATFORM="$PLATFORM"
  fi
  case "$EXPECT_FERROBOX_DIGEST$FERROBOX_DIGEST$POSTGRES_DIGEST" in
    *" "*) die "digests cannot contain spaces" ;;
  esac
  command -v sha256sum >/dev/null 2>&1 || die "sha256sum is required"
fi

KEY_FILE="$REPO_ROOT/osv-sync.pub.pem"
[ -f "$KEY_FILE" ] || die "missing $KEY_FILE"

proxy_args() {
  local name
  for name in HTTP_PROXY HTTPS_PROXY NO_PROXY http_proxy https_proxy no_proxy; do
    if [ -n "${!name:-}" ]; then
      printf '%s\0%s\0' -e "${name}=${!name}"
    fi
  done
}

verify_ferrobox_image() {
  local ref="$1"
  local identity="https://github.com/ferrobox/ferrobox/.github/workflows/release.yml@refs/tags/v${VERSION}"
  if command -v cosign >/dev/null 2>&1; then
    cosign verify \
      --certificate-identity="$identity" \
      --certificate-oidc-issuer=https://token.actions.githubusercontent.com \
      "$ref"
    return
  fi
  local -a extra=()
  while IFS= read -r -d '' arg; do
    extra+=("$arg")
  done < <(proxy_args)
  docker run --rm -e HOME=/tmp "${extra[@]}" "$COSIGN_IMAGE" verify \
    --certificate-identity="$identity" \
    --certificate-oidc-issuer=https://token.actions.githubusercontent.com \
    "$ref"
}

verify_index() {
  if command -v cosign >/dev/null 2>&1; then
    cosign verify --key "$KEY_FILE" --insecure-ignore-tlog=true "$OSV_REF"
    return
  fi
  local -a extra=()
  while IFS= read -r -d '' arg; do
    extra+=("$arg")
  done < <(proxy_args)
  docker run --rm -e HOME=/tmp "${extra[@]}" -v "$KEY_FILE:/key.pem:ro" "$COSIGN_IMAGE" \
    verify --key /key.pem --insecure-ignore-tlog=true "$OSV_REF"
}

# Cosign prints a JSON list. Warnings stay on stderr. Every signature in
# that list must name the same manifest digest; that is what we pull.
signed_manifest_digest() {
  python3 -c '
import json, sys
raw = sys.stdin.read()
start = raw.find("[")
end = raw.rfind("]")
if start < 0 or end < start:
    sys.exit("cosign verify did not print a signature list")
doc = json.loads(raw[start:end + 1])
if not isinstance(doc, list) or not doc:
    sys.exit("cosign verify returned no signatures")
digests = set()
for item in doc:
    digests.add(item["critical"]["image"]["docker-manifest-digest"])
if len(digests) != 1:
    sys.exit("cosign verify named more than one manifest")
digest = digests.pop()
hexpart = digest[7:] if digest.startswith("sha256:") else ""
if len(hexpart) != 64 or any(c not in "0123456789abcdef" for c in hexpart):
    sys.exit("unexpected manifest digest")
print(digest)
'
}

digest_from_headers() {
  python3 -c '
import sys
digest = ""
for line in sys.stdin:
    if line.lower().startswith("docker-content-digest:"):
        digest = line.split(":", 1)[1].strip()
hexpart = digest[7:] if digest.startswith("sha256:") else ""
if len(hexpart) != 64 or any(c not in "0123456789abcdef" for c in hexpart):
    sys.exit("registry response has no sha256 digest")
print(digest)
'
}

platform_digest_from_index() {
  python3 -c '
import json, sys
os_name, arch = sys.argv[1].split("/", 1)
doc = json.load(sys.stdin)
for item in doc.get("manifests") or []:
    plat = item.get("platform") or {}
    if plat.get("os") == os_name and plat.get("architecture") == arch:
        print(item["digest"])
        raise SystemExit
sys.exit("index has no %s manifest" % sys.argv[1])
' "$1"
}

if [ "$MANIFEST_FROM_JSON" -eq 1 ]; then
  signed_manifest_digest
  exit
fi

if [ "$DIGEST_FROM_HEADERS" -eq 1 ]; then
  digest_from_headers
  exit
fi

if [ -n "$PLATFORM_DIGEST_OSARCH" ]; then
  [[ "$PLATFORM_DIGEST_OSARCH" =~ ^linux/(amd64|arm64)$ ]] || die "--platform-digest-from-index expects linux/amd64 or linux/arm64"
  platform_digest_from_index "$PLATFORM_DIGEST_OSARCH"
  exit
fi

ghcr_tag_digest() {
  local repo="$1" tag="$2" body_file="$3" token headers
  token="$(curl -fsSL "https://ghcr.io/token?service=ghcr.io&scope=repository:${repo}:pull" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["token"])')"
  headers="$(curl -fsSL -D - -o "$body_file" \
    -H "Authorization: Bearer ${token}" \
    -H "Accept: application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json" \
    "https://ghcr.io/v2/${repo}/manifests/${tag}")"
  printf '%s\n' "$headers" | digest_from_headers
}

run_oras_pull() {
  local dest="$1"
  local ref="$2"
  if command -v oras >/dev/null 2>&1; then
    (cd "$dest" && oras pull "$ref")
    return
  fi
  local -a extra=()
  while IFS= read -r -d '' arg; do
    extra+=("$arg")
  done < <(proxy_args)
  docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp "${extra[@]}" \
    -v "$dest:/out" -w /out "$ORAS_IMAGE" pull "$ref"
}

run_helm_pull() {
  local dest="$1"
  if command -v helm >/dev/null 2>&1; then
    helm pull "oci://ghcr.io/ferrobox/charts/ferrobox" --version "$VERSION" --destination "$dest"
    return
  fi
  local -a extra=()
  while IFS= read -r -d '' arg; do
    extra+=("$arg")
  done < <(proxy_args)
  docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp "${extra[@]}" \
    -v "$dest:/out" "$HELM_IMAGE" pull \
    "oci://ghcr.io/ferrobox/charts/ferrobox" --version "$VERSION" --destination /out
}

image_digest() {
  local image="$1"
  local ref
  ref="$(docker image inspect --format '{{index .RepoDigests 0}}' "$image")"
  [ -n "$ref" ] || die "no registry digest for $image"
  printf '%s\n' "${ref##*@}"
}

fetch_into() {
  local stage="$1"
  mkdir -p "$stage"
  command -v docker >/dev/null 2>&1 || die "docker is required to fetch the bundle"
  local ferrobox_image="ghcr.io/ferrobox/ferrobox:${VERSION}"
  local manifest_body="$stage/ferrobox-manifest.json"
  command -v curl >/dev/null 2>&1 || die "curl is required to read the image digest"
  echo "Reading the tag digest for ${ferrobox_image}"
  FERROBOX_DIGEST="$(ghcr_tag_digest "ferrobox/ferrobox" "$VERSION" "$manifest_body")" \
    || die "could not read the FerroBox tag digest"
  if [ -n "$EXPECT_FERROBOX_DIGEST" ] && [ "$FERROBOX_DIGEST" != "$EXPECT_FERROBOX_DIGEST" ]; then
    die "FerroBox digest is ${FERROBOX_DIGEST}, expected ${EXPECT_FERROBOX_DIGEST}"
  fi
  local -a pull_args=()
  if [ -n "$PLATFORM" ]; then
    pull_args=(--platform "$PLATFORM")
  fi
  echo "Pulling ${ferrobox_image}${PLATFORM:+ for ${PLATFORM}}"
  docker pull "${pull_args[@]}" "$ferrobox_image"
  local image_os image_arch
  image_os="$(docker image inspect --format '{{.Os}}' "$ferrobox_image")"
  image_arch="$(docker image inspect --format '{{.Architecture}}' "$ferrobox_image")"
  FERROBOX_PLATFORM="${image_os}/${image_arch}"
  [[ "$FERROBOX_PLATFORM" =~ ^linux/(amd64|arm64)$ ]] || die "pulled ${FERROBOX_PLATFORM}; this bundle supports linux/amd64 and linux/arm64"
  if [ -n "$PLATFORM" ] && [ "$FERROBOX_PLATFORM" != "$PLATFORM" ]; then
    die "pulled ${FERROBOX_PLATFORM}, requested ${PLATFORM}"
  fi
  if python3 -c 'import json,sys; doc=json.load(open(sys.argv[1])); raise SystemExit(0 if doc.get("manifests") else 1)' "$manifest_body"; then
    FERROBOX_PLATFORM_DIGEST="$(platform_digest_from_index "$FERROBOX_PLATFORM" < "$manifest_body")" \
      || die "the tag digest has no ${FERROBOX_PLATFORM} image"
  else
    FERROBOX_PLATFORM_DIGEST="$FERROBOX_DIGEST"
  fi
  echo "Checking the FerroBox image signature (keyless; this step needs Sigstore)"
  verify_ferrobox_image "${ferrobox_image}@${FERROBOX_DIGEST}"
  echo "Pulling postgres:18-alpine${PLATFORM:+ for ${PLATFORM}}"
  docker pull "${pull_args[@]}" postgres:18-alpine
  POSTGRES_DIGEST="$(image_digest postgres:18-alpine)"
  echo "Checking the vulnerability index signature (key in osv-sync.pub.pem, no transparency log)"
  local verify_json index_ref
  verify_json="$(verify_index)" || die "the vulnerability index signature was rejected"
  OSV_MANIFEST_DIGEST="$(printf '%s\n' "$verify_json" | signed_manifest_digest)" \
    || die "could not read the signed manifest digest"
  index_ref="${OSV_REF%:*}@${OSV_MANIFEST_DIGEST}"
  echo "Pulling ${index_ref}"
  local osv_dir="$stage/osv-pull"
  mkdir -p "$osv_dir"
  run_oras_pull "$osv_dir" "$index_ref"
  INDEX_FILE="$osv_dir/ferrobox-osv-index.json.gz"
  INDEX_SHA_FILE="$osv_dir/ferrobox-osv-index.json.gz.sha256"
  [ -f "$INDEX_FILE" ] || die "oras pull did not write ferrobox-osv-index.json.gz"
  [ -f "$INDEX_SHA_FILE" ] || die "oras pull did not write the sha256 file"
  local chart_dir="$stage/chart-pull"
  mkdir -p "$chart_dir"
  run_helm_pull "$chart_dir"
  CHART_FILE="$(find "$chart_dir" -maxdepth 1 -name 'ferrobox-*.tgz' -print -quit)"
  [ -n "$CHART_FILE" ] || die "helm pull did not write a chart archive"
  echo "Saving images"
  docker save -o "$stage/ferrobox.tar" "$ferrobox_image"
  docker save -o "$stage/postgres.tar" postgres:18-alpine
  FERROBOX_TAR="$stage/ferrobox.tar"
  POSTGRES_TAR="$stage/postgres.tar"
}

write_bundle() {
  [ -f "$FERROBOX_TAR" ] || die "missing FerroBox image archive"
  [ -f "$POSTGRES_TAR" ] || die "missing postgres image archive"
  [ -f "$CHART_FILE" ] || die "missing chart archive"
  [ -f "$INDEX_FILE" ] || die "missing vulnerability index"
  [ -f "$INDEX_SHA_FILE" ] || die "missing index checksum file"
  [ -n "$FERROBOX_DIGEST" ] || die "missing FerroBox digest"
  [ -n "$POSTGRES_DIGEST" ] || die "missing postgres digest"
  [[ "$FERROBOX_DIGEST" =~ ^sha256:[0-9a-f]{64}$ ]] || die "FerroBox digest must be sha256: and 64 hex digits"
  [[ "$POSTGRES_DIGEST" =~ ^sha256:[0-9a-f]{64}$ ]] || die "postgres digest must be sha256: and 64 hex digits"
  if [ -n "$FERROBOX_PLATFORM" ]; then
    [[ "$FERROBOX_PLATFORM" =~ ^linux/(amd64|arm64)$ ]] || die "FerroBox platform must be linux/amd64 or linux/arm64"
  fi
  if [ -n "$FERROBOX_PLATFORM_DIGEST" ]; then
    [[ "$FERROBOX_PLATFORM_DIGEST" =~ ^sha256:[0-9a-f]{64}$ ]] || die "FerroBox platform digest must be sha256: and 64 hex digits"
  fi

  if [ -e "$OUT" ] && [ -n "$(ls -A "$OUT" 2>/dev/null || true)" ]; then
    die "output directory is not empty: $OUT"
  fi
  mkdir -p "$OUT/images" "$OUT/chart" "$OUT/osv" "$OUT/compose"

  cp "$FERROBOX_TAR" "$OUT/images/ferrobox.tar"
  cp "$POSTGRES_TAR" "$OUT/images/postgres.tar"
  cp "$CHART_FILE" "$OUT/chart/ferrobox-${VERSION}.tgz"
  cp "$INDEX_FILE" "$OUT/osv/ferrobox-osv-index.json.gz"
  cp "$INDEX_SHA_FILE" "$OUT/osv/ferrobox-osv-index.json.gz.sha256"
  cp "$KEY_FILE" "$OUT/osv/osv-sync.pub.pem"
  (
    cd "$OUT/osv"
    sha256sum -c ferrobox-osv-index.json.gz.sha256
  ) || die "index checksum does not match ferrobox-osv-index.json.gz"

  cat > "$OUT/compose/docker-compose.yml" <<EOF
# Loaded from this bundle. pull_policy: never refuses to dial a registry.
name: ferrobox
services:
  postgres:
    image: postgres:18-alpine
    pull_policy: never
    container_name: ferrobox-postgres
    restart: unless-stopped
    environment:
      POSTGRES_USER: \${POSTGRES_USER}
      POSTGRES_PASSWORD: \${POSTGRES_PASSWORD}
      POSTGRES_DB: \${POSTGRES_DB}
    ports:
      - "\${POSTGRES_PORT:-5432}:5432"
    volumes:
      - postgres_data:/var/lib/postgresql
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U \${POSTGRES_USER} -d \${POSTGRES_DB}"]
      interval: 5s
      timeout: 5s
      retries: 5

  ferrobox:
    image: ghcr.io/ferrobox/ferrobox:${VERSION}
    pull_policy: never
    container_name: ferrobox
    restart: unless-stopped
    depends_on:
      postgres:
        condition: service_healthy
    ports:
      - "\${FERROBOX_PORT:-3000}:3000"
    volumes:
      - ferrobox_blobs:/var/lib/ferrobox/blobs
    environment:
      DATABASE_URL: postgres://\${POSTGRES_USER}:\${POSTGRES_PASSWORD}@postgres:5432/\${POSTGRES_DB}
      STORAGE_BACKEND: filesystem
      STORAGE_PATH: /var/lib/ferrobox/blobs
      BIND_ADDRESS: 0.0.0.0:3000
      PUBLIC_BASE_URL: \${PUBLIC_BASE_URL:-http://127.0.0.1:3000}
      ADMIN_USERNAME: \${ADMIN_USERNAME:-admin}
      ADMIN_PASSWORD: \${ADMIN_PASSWORD:-admin}
      FRONTEND_DIR: /app/www

volumes:
  postgres_data:
  ferrobox_blobs:
EOF

  INDEX_HEX="$(awk 'NR==1 { print $1 }' "$OUT/osv/ferrobox-osv-index.json.gz.sha256")"
  [[ "$INDEX_HEX" =~ ^[0-9a-f]{64}$ ]] || die "index checksum file must start with 64 lowercase hex digits"
  export BUNDLE_VERSION="$VERSION" BUNDLE_OSV_REF="$OSV_REF" \
    BUNDLE_FERROBOX_DIGEST="$FERROBOX_DIGEST" BUNDLE_POSTGRES_DIGEST="$POSTGRES_DIGEST" \
    BUNDLE_INDEX_SHA256="$INDEX_HEX" BUNDLE_OSV_MANIFEST_DIGEST="$OSV_MANIFEST_DIGEST" \
    BUNDLE_FERROBOX_PLATFORM="$FERROBOX_PLATFORM" \
    BUNDLE_FERROBOX_PLATFORM_DIGEST="$FERROBOX_PLATFORM_DIGEST"
  python3 - <<'PY' > "$OUT/bundle.json"
import json
import os
import sys

document = {
    "ferrobox_version": os.environ["BUNDLE_VERSION"],
    "ferrobox_image": "ghcr.io/ferrobox/ferrobox:" + os.environ["BUNDLE_VERSION"],
    "ferrobox_digest": os.environ["BUNDLE_FERROBOX_DIGEST"],
    "postgres_image": "postgres:18-alpine",
    "postgres_digest": os.environ["BUNDLE_POSTGRES_DIGEST"],
    "chart": "ferrobox-" + os.environ["BUNDLE_VERSION"] + ".tgz",
    "osv_ref": os.environ["BUNDLE_OSV_REF"],
    "osv_sha256": os.environ["BUNDLE_INDEX_SHA256"],
    "public_key": "osv/osv-sync.pub.pem",
    "storage": "filesystem",
    "image_signature": "keyless-cosign-checked-while-online",
    "index_signature": "cosign-key-checked-while-online",
}
manifest = os.environ.get("BUNDLE_OSV_MANIFEST_DIGEST", "")
if manifest:
    document["osv_manifest_digest"] = manifest
platform = os.environ.get("BUNDLE_FERROBOX_PLATFORM", "")
if platform:
    document["ferrobox_platform"] = platform
platform_digest = os.environ.get("BUNDLE_FERROBOX_PLATFORM_DIGEST", "")
if platform_digest:
    document["ferrobox_platform_digest"] = platform_digest
json.dump(document, sys.stdout, indent=2)
sys.stdout.write("\n")
PY

  cat > "$OUT/README.txt" <<EOF
FerroBox offline bundle ${VERSION}

This directory was packed on a machine that could reach the registries.
The FerroBox image signature was checked there with keyless Cosign, which
needs Sigstore and does not work on an offline machine. The vulnerability
index signature was checked there with osv/osv-sync.pub.pem. That signature
does not use the transparency log.

Offline, trust this directory only if sha256sum -c SHA256SUMS succeeds and
the SHA256SUMS fingerprint you wrote down on the connected machine matches.

Install, from this directory:

  sha256sum -c SHA256SUMS
  ./install.sh

Then import the index (the Sync button would call the registry):

  SHA=\$(awk 'NR==1 { print \$1 }' osv/ferrobox-osv-index.json.gz.sha256)
  curl -fsS http://127.0.0.1:3000/api/auth/login \\
    -H 'content-type: application/json' \\
    -d '{"username":"admin","password":"admin"}'
  curl -fsS http://127.0.0.1:3000/api/security/osv-feed \\
    -H "Authorization: Bearer TOKEN" \\
    -H "X-FerroBox-Sha256: \$SHA" \\
    --data-binary @osv/ferrobox-osv-index.json.gz

The header is the hex digest, without a sha256: prefix.
The FerroBox archive in this directory is ${FERROBOX_PLATFORM:-the architecture of the machine that packed it}.
install.sh loads it only on that architecture.
Settings shows a directory. The Vulnerability feed page then shows mode file.
Garage is not part of this bundle. The index covers npm, PyPI, crates.io,
Maven, NuGet, and Go.
EOF

  cat > "$OUT/install.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
export LC_ALL=C
sha256sum -c SHA256SUMS
platform="$(sed -n 's/.*"ferrobox_platform": "\([^"]*\)".*/\1/p' bundle.json)"
case "$(uname -m)" in
  x86_64|amd64) host_platform=linux/amd64 ;;
  aarch64|arm64) host_platform=linux/arm64 ;;
  *) host_platform="$(uname -m)" ;;
esac
if [ -n "$platform" ] && [ "$platform" != "$host_platform" ]; then
  echo "This bundle contains ${platform} images. This machine is ${host_platform}." >&2
  exit 1
fi
docker load -i images/ferrobox.tar
docker load -i images/postgres.tar
if [ ! -f compose/.env ]; then
  umask 077
  cat > compose/.env <<'ENV'
POSTGRES_USER=ferrobox
POSTGRES_PASSWORD=ferrobox
POSTGRES_DB=ferrobox
POSTGRES_PORT=5432
FERROBOX_PORT=3000
PUBLIC_BASE_URL=http://127.0.0.1:3000
ADMIN_USERNAME=admin
ADMIN_PASSWORD=admin
ENV
  chmod 600 compose/.env
fi
docker compose -f compose/docker-compose.yml --env-file compose/.env up -d
echo "Waiting for ferrobox to become healthy..."
container=""
deadline=$((SECONDS + 180))
until [ "$(docker inspect --format='{{if .State.Health}}{{.State.Health.Status}}{{end}}' "$container" 2>/dev/null)" = "healthy" ]; do
  if [ "$SECONDS" -ge "$deadline" ]; then
    echo "ferrobox did not become healthy within 180s" >&2
    docker compose -f compose/docker-compose.yml logs --tail=80 >&2 || true
    exit 1
  fi
  container="$(docker compose -f compose/docker-compose.yml ps -q ferrobox || true)"
  sleep 1
done
echo "FerroBox is up at http://127.0.0.1:3000"
echo "Sign in with the ADMIN_USERNAME and ADMIN_PASSWORD in compose/.env"
EOF
  chmod 755 "$OUT/install.sh"

  (
    cd "$OUT"
    find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS
  ) || die "could not write SHA256SUMS"
  echo "Bundle written to $OUT"
  echo "Write down this fingerprint before you disconnect:"
  sha256sum "$OUT/SHA256SUMS"
}

if [ "$ASSEMBLE" -eq 0 ]; then
  stage="$(mktemp -d)"
  trap 'rm -rf "$stage"' EXIT
  fetch_into "$stage"
fi

write_bundle
