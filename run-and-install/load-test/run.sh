#!/usr/bin/env bash
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
[[ $# == 1 && -f "$1" ]] || { echo "Usage: $0 DATASET.json" >&2; exit 2; }
: "${API_BASE_URL:?Set API_BASE_URL to your isolated test API}"
: "${REAXIUM_API_USER:?Set REAXIUM_API_USER}"
: "${REAXIUM_API_PASSWORD:?Set REAXIUM_API_PASSWORD}"
: "${START_DATE:?Set START_DATE, e.g. 2026-10-22T08:00:00Z}"
device_count="$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))))' "$1")"
export VUS="${VUS:-$device_count}" JOURNEYS="${JOURNEYS:-10}" MAX_DURATION="${MAX_DURATION:-30m}"
export SERVICE="${SERVICE:-journey}"
case "$SERVICE" in journey|beginARoute|endARoute|deviceStatus) ;; *) echo 'SERVICE must be journey, beginARoute, endARoute, or deviceStatus' >&2; exit 2 ;; esac
export RUN_ID="${RUN_ID:-$(python3 -c 'import uuid; print(uuid.uuid4().hex)')}"
dataset="$(realpath -- "$1")"
echo "Run $RUN_ID: $VUS devices, $JOURNEYS iterations each, service=$SERVICE; writes to $API_BASE_URL."
test_exit=0
docker run --rm --network host --user "$(id -u):$(id -g)" \
  -e API_BASE_URL -e REAXIUM_API_USER -e REAXIUM_API_PASSWORD \
  -e SERVICE -e VUS -e JOURNEYS -e MAX_DURATION -e RUN_ID -e START_DATE \
  --mount "type=bind,src=$script_dir,dst=/scripts,readonly" \
  --mount "type=bind,src=$dataset,dst=/data/devices.json,readonly" \
  grafana/k6:latest run /scripts/routes.js || test_exit=$?
case "$SERVICE" in
  deviceStatus) service_label='device/DeviceStatus/update' ;;
  journey) service_label='beginARoute + endARoute (journey)' ;;
  *) service_label="$SERVICE" ;;
esac
printf '\nTest service: %s\nDevices: %s | Iterations per device: %s\nExit code: %s\n' \
  "$service_label" "$VUS" "$JOURNEYS" "$test_exit"
exit "$test_exit"
