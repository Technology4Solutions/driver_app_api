#!/usr/bin/env bash
# Prepare local configuration, check prerequisites, and build the API image.
source "$(dirname -- "${BASH_SOURCE[0]}")/common.sh"
require_docker
require_command curl
require_command python3
if [[ ! -f "$PROJECT_ROOT/.env" ]]; then
  (umask 077; cp -- "$PROJECT_ROOT/.env.example" "$PROJECT_ROOT/.env")
  echo "Created $PROJECT_ROOT/.env. Set your remote database and shared API credentials."
else
  echo 'Existing .env preserved.'
fi
echo 'Setup checks passed. Rust does not need to be installed locally.'
compose build api
compose --profile monitoring pull prometheus grafana
echo 'API image built and monitoring images downloaded. If using SSH forwarding, start the database/relay tunnel before running the API.'
echo "Next: $SCRIPT_DIR/run.sh"
