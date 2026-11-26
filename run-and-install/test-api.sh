#!/usr/bin/env bash
source "$(dirname -- "${BASH_SOURCE[0]}")/common.sh"
usage() {
  cat <<'HELP'
Usage:
  test-api.sh                              Check greeting and liveness (no writes).
  test-api.sh --interrupt-route PAYLOAD.json Interrupt a route and drop off passengers.
  test-api.sh --send-traffic PAYLOAD.json   Send a device traffic event.
  test-api.sh --end-route PAYLOAD.json      End a route and clear device activity.
  test-api.sh --begin-route PAYLOAD.json    Start a route (database + Vision + dashboards).
  test-api.sh --device-status PAYLOAD.json  Queue supplied telemetry for background processing.

API_BASE_URL defaults to http://127.0.0.1:8080.
ADMIN_BASE_URL defaults to http://127.0.0.1:9090.
The POST uses CLOUD_USERNAME / CLOUD_PASSWORD from the shell environment,
or prompts for them. Enter credentials for an active Cloud database account.
Omitting device_activity in the payload clears the device's existing activity.
HELP
}
payload=''
endpoint='/device/DeviceStatus/update'
case "${1:-}" in
  '') [[ $# -eq 0 ]] || { usage; exit 2; } ;;
  --help|-h) usage; exit 0 ;;
  --device-status|--begin-route|--end-route|--interrupt-route|--send-traffic)
    [[ $# -eq 2 && -f "$2" ]] || { usage >&2; exit 2; }
    payload="$2"
    if [[ "$1" == --send-traffic ]]; then endpoint="/DeviceTraffic/sendTraffic"; fi
    if [[ "$1" == --interrupt-route ]]; then endpoint="/DeviceTraffic/interruptARoute"; fi
    if [[ "$1" == --end-route ]]; then endpoint="/DeviceTraffic/endARoute"; fi
    if [[ "$1" == --begin-route ]]; then endpoint="/DeviceTraffic/beginARoute"; fi
    ;;
  *) usage >&2; exit 2 ;;
esac
require_command curl
require_command python3
API_BASE_URL="${API_BASE_URL:-http://127.0.0.1:8080}"
ADMIN_BASE_URL="${ADMIN_BASE_URL:-http://127.0.0.1:9090}"
work_dir="$(mktemp -d)"
trap 'rm -rf -- "$work_dir"' EXIT

# Bounded retries allow the API to finish its initial database connection.
curl --fail --silent --show-error --connect-timeout 5 --max-time 15 \
  --retry 5 --retry-connrefused --retry-delay 2 \
  "${API_BASE_URL%/}/" > "$work_dir/greeting"
[[ "$(cat "$work_dir/greeting")" == 'Hello, world!' ]] || { echo 'Unexpected greeting response.' >&2; exit 1; }
echo 'PASS: GET /'
curl --fail --silent --show-error --connect-timeout 5 --max-time 15 \
  "${ADMIN_BASE_URL%/}/health/live" > "$work_dir/health"
[[ "$(cat "$work_dir/health")" == 'ok' ]] || { echo 'Unexpected liveness response.' >&2; exit 1; }
echo 'PASS: GET /health/live'
[[ -n "$payload" ]] || exit 0

python3 -m json.tool "$payload" >/dev/null
api_username="${CLOUD_USERNAME:-}"
api_password="${CLOUD_PASSWORD:-}"
if [[ -z "$api_username" ]]; then read -r -p 'API username: ' api_username; fi
if [[ -z "$api_password" ]]; then read -r -s -p 'API password: ' api_password; echo; fi
[[ -n "$api_username" && -n "$api_password" ]] || { echo 'API credentials must not be empty.' >&2; exit 1; }
# Keep login credentials and bearer tokens out of process arguments and logs.
printf '%s\0%s' "$api_username" "$api_password" | python3 -c '
import json, pathlib, sys
user, password = sys.stdin.buffer.read().split(b"\0", 1)
p = pathlib.Path(sys.argv[1])
p.touch(mode=0o600)
p.write_text(json.dumps({"username": user.decode(), "password": password.decode()}))
' "$work_dir/login.json"
unset api_password
curl --fail --silent --show-error --connect-timeout 5 --max-time 15 \
  -H 'Content-Type: application/json' --data-binary "@$work_dir/login.json" \
  "${API_BASE_URL%/}/api/auth/token" > "$work_dir/token.json"
python3 - "$work_dir/token.json" "$work_dir/auth.conf" <<'PYTOKEN'
import json, pathlib, re, sys
token = json.loads(pathlib.Path(sys.argv[1]).read_text())["access_token"]
if not re.fullmatch(r"[A-Za-z0-9_.-]+", token):
    sys.exit("Invalid bearer token response")
p = pathlib.Path(sys.argv[2])
p.touch(mode=0o600)
p.write_text('header = "Authorization: Bearer ' + token + '"\n')
PYTOKEN

echo 'Submitting telemetry: this writes to the configured database.'
# Do not retry POST: doing so could insert duplicate location records.
curl --config "$work_dir/auth.conf" --fail --silent --show-error \
  --connect-timeout 5 --max-time 60 \
  -H 'Content-Type: application/json' \
  --data-binary "@$payload" \
  "${API_BASE_URL%/}${endpoint}" > "$work_dir/response.json"
python3 - "$work_dir/response.json" <<'PY'
import json, sys
with open(sys.argv[1]) as response_file:
    response = json.load(response_file)
print(json.dumps(response, indent=2))
code = response.get("ReaxiumResponse", {}).get("code")
if type(code) is not int or code != 0:
    sys.exit("FAIL: API returned an application error (HTTP 200 alone is not success).")
print("PASS: Request completed. Check API logs for record IDs and downstream results.")
PY
