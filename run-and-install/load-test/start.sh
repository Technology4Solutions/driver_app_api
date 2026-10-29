#!/usr/bin/env bash
# Interactive launcher with optional local fixed credentials.
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
if [[ "${1:-}" == --help || $# -gt 1 ]]; then
  echo "Usage: $0 [DATASET.json]"
  echo 'Defaults: devices.local.json, VUS=1, JOURNEYS=1. SERVICE=journey|beginARoute|endARoute|deviceStatus. Prompts for missing API settings.'
  exit 0
fi
dataset="${1:-$script_dir/../devices.local.json}"
[[ -f "$dataset" ]] || { echo "Dataset not found: $dataset" >&2; exit 1; }

if [[ -z "${API_BASE_URL:-}" ]]; then
  read -r -p 'Test API URL [http://127.0.0.1:8080]: ' API_BASE_URL
  API_BASE_URL="${API_BASE_URL:-http://127.0.0.1:8080}"
fi
if [[ -f "$script_dir/credentials.local.sh" ]]; then
  source "$script_dir/credentials.local.sh"
  REAXIUM_API_USER="${REAXIUM_API_USER:-${LOAD_TEST_API_USER:-}}"
  REAXIUM_API_PASSWORD="${REAXIUM_API_PASSWORD:-${LOAD_TEST_API_PASSWORD:-}}"
  unset LOAD_TEST_API_USER LOAD_TEST_API_PASSWORD
fi
if [[ -z "${REAXIUM_API_USER:-}" ]]; then
  read -r -p 'API username (same as API .env): ' REAXIUM_API_USER
fi
if [[ -z "${REAXIUM_API_PASSWORD:-}" ]]; then
  read -r -s -p 'API password: ' REAXIUM_API_PASSWORD
  echo
fi
[[ -n "$REAXIUM_API_USER" && -n "$REAXIUM_API_PASSWORD" ]] || {
  echo 'API username and password must not be empty.' >&2; exit 1;
}
if [[ -z "${START_DATE:-}" ]]; then
  default_date="$(TZ=Africa/Casablanca date +%Y-%m-%d)T08:00:00Z"
  read -r -p "Simulated start date [$default_date]: " START_DATE
  START_DATE="${START_DATE:-$default_date}"
fi
export API_BASE_URL REAXIUM_API_USER REAXIUM_API_PASSWORD START_DATE
export VUS="${VUS:-1}" JOURNEYS="${JOURNEYS:-1}"
echo 'This sends real requests: use an API connected to a test database and test downstream receivers.'
exec "$script_dir/run.sh" "$dataset"
