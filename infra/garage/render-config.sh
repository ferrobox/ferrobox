#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INFRA_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

set -a
source "$INFRA_DIR/.env"
: "${GARAGE_RPC_PUBLIC_ADDR:=127.0.0.1:3901}"
set +a

envsubst < "$SCRIPT_DIR/garage.toml.template" > "$SCRIPT_DIR/garage.toml"

echo "Generado: $SCRIPT_DIR/garage.toml"
