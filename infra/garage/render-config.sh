#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INFRA_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

set -a
source "$INFRA_DIR/.env"
set +a

envsubst < "$SCRIPT_DIR/garage.toml.template" > "$SCRIPT_DIR/garage.toml"

echo "Generado: $SCRIPT_DIR/garage.toml"
