#!/usr/bin/env bash
# Packs a fake bundle and checks the layout. Does not pull images.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

hex64() {
  python3 -c 'print("a"*64)'
}

mkdir -p "$tmp/src"
printf 'ferrobox-image' > "$tmp/src/ferrobox.tar"
printf 'postgres-image' > "$tmp/src/postgres.tar"
printf 'chart-bytes' > "$tmp/src/chart.tgz"
printf 'index-bytes' > "$tmp/src/ferrobox-osv-index.json.gz"
(
  cd "$tmp/src"
  sha256sum ferrobox-osv-index.json.gz > ferrobox-osv-index.json.gz.sha256
)
ferrobox_digest="sha256:$(hex64)"
postgres_digest="sha256:$(python3 -c 'print("b"*64)')"

digest="sha256:$(python3 -c 'print("ab"*32)')"
got="$(printf '%s\n' 'Verification for example --' \
  "[{\"critical\":{\"image\":{\"docker-manifest-digest\":\"${digest}\"}}}]" \
  | "$SCRIPT_DIR/offline-bundle.sh" --manifest-digest-from-json)"
test "$got" = "$digest"
if printf 'nope\n' | "$SCRIPT_DIR/offline-bundle.sh" --manifest-digest-from-json; then
  echo "garbage cosign output must be rejected" >&2
  exit 1
fi
if printf '%s\n' '[{"critical":{"image":{"docker-manifest-digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}},{"critical":{"image":{"docker-manifest-digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}}]' \
  | "$SCRIPT_DIR/offline-bundle.sh" --manifest-digest-from-json
then
  echo "two manifest digests must be rejected" >&2
  exit 1
fi
digest_header="$(printf 'HTTP/1.1 200 OK\r\nDocker-Content-Digest: %s\r\n\r\n' "$digest")"
got_header="$(printf '%s' "$digest_header" | "$SCRIPT_DIR/offline-bundle.sh" --digest-from-headers)"
test "$got_header" = "$digest"
if printf 'HTTP/1.1 200 OK\r\n\r\n' | "$SCRIPT_DIR/offline-bundle.sh" --digest-from-headers; then
  echo "headers without a digest must be rejected" >&2
  exit 1
fi
amd64_digest="sha256:$(python3 -c 'print("cd"*32)')"
arm64_digest="sha256:$(python3 -c 'print("ef"*32)')"
attestation_digest="sha256:$(python3 -c 'print("12"*32)')"
got_platform="$(
  python3 - "$amd64_digest" "$arm64_digest" "$attestation_digest" <<'PY' | "$SCRIPT_DIR/offline-bundle.sh" --platform-digest-from-index linux/arm64
import json, sys
json.dump({
    "manifests": [
        {"digest": sys.argv[1], "platform": {"os": "linux", "architecture": "amd64"}},
        {"digest": sys.argv[3], "platform": {"os": "unknown", "architecture": "unknown"}},
        {"digest": sys.argv[2], "platform": {"os": "linux", "architecture": "arm64"}},
    ]
}, sys.stdout)
PY
)"
test "$got_platform" = "$arm64_digest"
if printf '%s\n' '{"manifests":[{"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","platform":{"os":"unknown","architecture":"unknown"}}]}' \
  | "$SCRIPT_DIR/offline-bundle.sh" --platform-digest-from-index linux/amd64
then
  echo "an attestation manifest must not count as linux/amd64" >&2
  exit 1
fi
if "$SCRIPT_DIR/offline-bundle.sh" --assemble \
  --version 0.2.4 \
  --platform linux/ppc64le \
  --osv-ref ghcr.io/ferrobox/osv-db:2026-10-12 \
  --out "$tmp/rejected-platform"
then
  echo "an unsupported platform must be rejected" >&2
  exit 1
fi

if "$SCRIPT_DIR/offline-bundle.sh" --assemble \
  --version v0.2.4 \
  --osv-ref ghcr.io/ferrobox/osv-db:2026-10-12 \
  --out "$tmp/rejected-version"
then
  echo "a leading v must be rejected" >&2
  exit 1
fi

"$SCRIPT_DIR/offline-bundle.sh" --assemble \
  --version 0.2.4 \
  --platform linux/amd64 \
  --osv-ref ghcr.io/ferrobox/osv-db:2026-10-12 \
  --out "$tmp/bundle" \
  --ferrobox-tar "$tmp/src/ferrobox.tar" \
  --ferrobox-digest "$ferrobox_digest" \
  --postgres-tar "$tmp/src/postgres.tar" \
  --postgres-digest "$postgres_digest" \
  --chart "$tmp/src/chart.tgz" \
  --index "$tmp/src/ferrobox-osv-index.json.gz" \
  --index-sha "$tmp/src/ferrobox-osv-index.json.gz.sha256"

(
  cd "$tmp/bundle"
  sha256sum -c SHA256SUMS
)
test -f "$tmp/bundle/images/ferrobox.tar"
test -f "$tmp/bundle/images/postgres.tar"
test -f "$tmp/bundle/chart/ferrobox-0.2.4.tgz"
test -f "$tmp/bundle/osv/osv-sync.pub.pem"
cmp -s "$ROOT/osv-sync.pub.pem" "$tmp/bundle/osv/osv-sync.pub.pem"
grep -q 'STORAGE_BACKEND: filesystem' "$tmp/bundle/compose/docker-compose.yml"
grep -q 'pull_policy: never' "$tmp/bundle/compose/docker-compose.yml"
if grep -qi garage "$tmp/bundle/compose/docker-compose.yml"; then
  echo "offline compose must not start Garage" >&2
  exit 1
fi
if grep -q 'build:' "$tmp/bundle/compose/docker-compose.yml"; then
  echo "offline compose must load images, not build them" >&2
  exit 1
fi
grep -q 'image: ghcr.io/ferrobox/ferrobox:0.2.4' "$tmp/bundle/compose/docker-compose.yml"
grep -q 'image: postgres:18-alpine' "$tmp/bundle/compose/docker-compose.yml"
python3 - "$tmp/bundle/bundle.json" "$ferrobox_digest" <<'PY'
import json, sys
doc = json.load(open(sys.argv[1]))
assert doc["ferrobox_version"] == "0.2.4"
assert doc["ferrobox_digest"] == sys.argv[2]
assert doc["storage"] == "filesystem"
assert doc["osv_ref"] == "ghcr.io/ferrobox/osv-db:2026-10-12"
assert doc["image_signature"] == "keyless-cosign-checked-while-online"
assert "osv_manifest_digest" not in doc
assert doc["ferrobox_platform"] == "linux/amd64"
assert "ferrobox_platform_digest" not in doc
assert doc["osv_sha256"] == open(sys.argv[1].replace("bundle.json", "osv/ferrobox-osv-index.json.gz.sha256")).read().split()[0]
PY
bash -n "$tmp/bundle/install.sh"
test -x "$tmp/bundle/install.sh"
grep -q 'ferrobox_platform' "$tmp/bundle/install.sh"
grep -q 'linux/amd64' "$tmp/bundle/README.txt"
if "$SCRIPT_DIR/offline-bundle.sh" --assemble \
  --version 0.2.4 \
  --osv-ref ghcr.io/ferrobox/osv-db:2026-10-12 \
  --out "$tmp/bundle" \
  --ferrobox-tar "$tmp/src/ferrobox.tar" \
  --ferrobox-digest "$ferrobox_digest" \
  --postgres-tar "$tmp/src/postgres.tar" \
  --postgres-digest "$postgres_digest" \
  --chart "$tmp/src/chart.tgz" \
  --index "$tmp/src/ferrobox-osv-index.json.gz" \
  --index-sha "$tmp/src/ferrobox-osv-index.json.gz.sha256"
then
  echo "a non-empty output directory must be rejected" >&2
  exit 1
fi

printf 'x' >> "$tmp/bundle/README.txt"
if (cd "$tmp/bundle" && sha256sum -c SHA256SUMS); then
  echo "a changed file must fail the checksum" >&2
  exit 1
fi

bad="$(mktemp -d)"
printf 'nope\n' > "$tmp/src/wrong.sha256"
if "$SCRIPT_DIR/offline-bundle.sh" --assemble \
  --version 0.2.4 \
  --osv-ref ghcr.io/ferrobox/osv-db:2026-10-12 \
  --out "$bad" \
  --ferrobox-tar "$tmp/src/ferrobox.tar" \
  --ferrobox-digest "$ferrobox_digest" \
  --postgres-tar "$tmp/src/postgres.tar" \
  --postgres-digest "$postgres_digest" \
  --chart "$tmp/src/chart.tgz" \
  --index "$tmp/src/ferrobox-osv-index.json.gz" \
  --index-sha "$tmp/src/wrong.sha256"
then
  echo "a bad index checksum must be rejected" >&2
  exit 1
fi
rm -rf "$bad"

echo "offline-bundle-test: ok"
