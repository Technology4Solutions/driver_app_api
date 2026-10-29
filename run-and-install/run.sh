#!/usr/bin/env bash
# Build and start/recreate the API using the current .env.
source "$(dirname -- "${BASH_SOURCE[0]}")/common.sh"
require_docker
require_env
compose up --build -d --wait --wait-timeout 60 api
compose ps api
echo "API started. Run $SCRIPT_DIR/test-api.sh to check HTTP readiness."
echo "Logs: $SCRIPT_DIR/logs.sh"
