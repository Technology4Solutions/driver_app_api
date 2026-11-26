#!/usr/bin/env bash
# Interactive launcher; enter credentials for an active Cloud database account.
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
if [[ "${1:-}" == --help || $# -gt 1 ]]; then
  echo "Usage: $0 [DATASET.json]"
  echo 'Defaults: devices.local.json, VUS=1, JOURNEYS=1. SERVICE selects any business service, journey, or allServices (see services.json). startARoute aliases beginARoute. Prompts for missing API settings.'
  exit 0
fi
dataset="${1:-$script_dir/../devices.local.json}"
[[ -f "$dataset" ]] || { echo "Dataset not found: $dataset" >&2; exit 1; }

if [[ -z "${API_BASE_URL:-}" ]]; then
  read -r -p 'Test API URL [http://127.0.0.1:8080]: ' API_BASE_URL
  API_BASE_URL="${API_BASE_URL:-http://127.0.0.1:8080}"
fi
if [[ -z "${CLOUD_USERNAME:-}" ]]; then
  read -r -p 'API username: ' CLOUD_USERNAME
fi
if [[ -z "${CLOUD_PASSWORD:-}" ]]; then
  read -r -s -p 'API password: ' CLOUD_PASSWORD
  echo
fi
[[ -n "$CLOUD_USERNAME" && -n "$CLOUD_PASSWORD" ]] || {
  echo 'API username and password must not be empty.' >&2; exit 1;
}
if [[ -z "${START_DATE:-}" ]]; then
  default_date="$(TZ=Africa/Casablanca date +%Y-%m-%d)T08:00:00Z"
  read -r -p "Simulated start date [$default_date]: " START_DATE
  START_DATE="${START_DATE:-$default_date}"
fi
export API_BASE_URL CLOUD_USERNAME CLOUD_PASSWORD START_DATE
export VUS="${VUS:-1}" JOURNEYS="${JOURNEYS:-1}"
echo 'This sends real requests: use an API connected to a test database and test downstream receivers.'
exec "$script_dir/run.sh" "$dataset"
