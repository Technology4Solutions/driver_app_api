#!/usr/bin/env bash
# Shared helpers. Do not source .env: dotenv files are configuration, not shell code.
set -euo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"

require_command() {
  command -v "$1" >/dev/null 2>&1 || { echo "Required command not found: $1" >&2; exit 1; }
}

require_docker() {
  require_command docker
  docker compose version >/dev/null
  docker info >/dev/null 2>&1 || {
    echo 'Docker is unavailable. Start Docker and check that your user can access it.' >&2
    exit 1
  }
}

require_env() {
  [[ -f "$PROJECT_ROOT/.env" ]] || {
    echo "Missing .env. Run $SCRIPT_DIR/build.sh first." >&2
    exit 1
  }
}

compose() {
  docker compose --project-directory "$PROJECT_ROOT" --env-file "$PROJECT_ROOT/.env" -f "$PROJECT_ROOT/compose.yaml" "$@"
}
