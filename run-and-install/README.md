# Build, run, and test

Run these Bash scripts from any directory. They resolve the project path automatically.
The Docker setup uses Linux host networking. Required tools: Docker with Compose v2,
Bash, curl, and Python 3. Rust is provided by Docker.

## Setup and start

```bash
./run-and-install/build.sh
# Edit .env with your database and shared API credentials.
./run-and-install/run.sh
./run-and-install/logs.sh
```

`build.sh` checks the tools and creates `.env` from `.env.example` only if missing.
It preserves existing settings, then builds the API image. It does not install system packages or create a database.
`run.sh` builds and starts/recreates the API. It waits for the container to run;
use `test-api.sh` to verify HTTP readiness. Press Ctrl+C to stop following logs.

If using the SSH setup, keep your tunnel running in another terminal:

```bash
ssh -N -o ExitOnForwardFailure=yes \
  -L 127.0.0.1:3307:127.0.0.1:3306 \
  -L 127.0.0.1:5555:127.0.0.1:5555 SSH_USER@SSH_SERVER
```

This example assumes MySQL and the ZeroMQ relay run on the SSH server. For other
hosts, replace each forwarding destination with the address reachable from that server.

## API tests

Check the greeting and liveness endpoints without writing data:

```bash
./run-and-install/test-api.sh
```

To test DeviceStatus:

```bash
cp run-and-install/device-status.example.json run-and-install/device-status.local.json
# Edit device-status.local.json: set an active IMEI, valid user/business IDs,
# the observation time, and the actual telemetry values.
./run-and-install/test-api.sh --device-status run-and-install/device-status.local.json
```

Enter the same API username and password configured in `.env`. The password prompt
is hidden. Alternatively, the script accepts already-exported `REAXIUM_API_USER`
and `REAXIUM_API_PASSWORD` (or their `REAXIUM_API_BASIC_AUTH_*` aliases).
It deliberately does not execute `.env` as shell code.

**The DeviceStatus test queues an update that can write to the configured database and broadcast to dashboards.**
A successful response confirms queue acceptance; check worker logs for persistence.
Without `device_activity`, it clears existing activity for that device, just like the endpoint.
The script does not automatically retry a POST. Exit status is nonzero for HTTP errors,
invalid responses, or a nonzero `ReaxiumResponse.code`.
Local JSON copies are excluded from Git and Docker build contexts.

For other HTTP addresses, override the URLs (these are independent of `.env` bind settings):

```bash
API_BASE_URL=http://127.0.0.1:8080 \
ADMIN_BASE_URL=http://127.0.0.1:9090 \
  ./run-and-install/test-api.sh
```

A successful liveness check alone does not prove the current database connection is healthy.
For a DeviceStatus save, look for `device status transaction committed` in API logs.
Broadcast failures return a nonzero application code with `database_saved=true` and a
`broadcast_status` of `failed` or `timed_out`. Do not repeat the POST when data was already saved.
Successful relay submission does not guarantee receipt by every dashboard client.

## Other commands

| Script | Purpose |
| --- | --- |
| `build.sh` | Check prerequisites, create `.env` if missing, and build the API image |
| `run.sh` | Build and start/recreate the API |
| `stop.sh` | Stop only the API |
| `logs.sh` | Follow the last 100 log lines and new logs |
| `test-api.sh` | Test the running HTTP endpoints |
| `test-code.sh` | Run formatting checks, Clippy, and ordinary automated tests in Docker |

`test-code.sh` uses named Docker volumes for the Cargo registry and build cache;
it does not create root-owned build files in the source checkout. Tests requiring
a live database are ignored by this command. The separate disposable-MySQL integration
test is documented in the project's main README.

To test a route start, update `json/begin-route.example.json` with valid database IDs,
then run:

```bash
./run-and-install/test-api.sh --begin-route run-and-install/json/begin-route.example.json
```

This writes `device_traffic` and `device_activity` and sends Vision/dashboard updates.
Configure `VISION_UDP_DESTINATION` and `VISION_TIMEZONE` in `.env`; the dashboard
uses `DEVICE_STATUS_ZMQ_ENDPOINT`. The script checks the response envelope, not just
HTTP 200. If the response says `database_saved: true`, do not resubmit the route start.

To finish a route, use `--end-route json/end-route.example.json` (from this directory),
or from the project root:

```bash
./run-and-install/test-api.sh --end-route run-and-install/json/end-route.example.json
```

Use valid IDs and `traffic_situation_id`. This inserts route-end traffic and removes
all activity rows for the device. It includes Vision and dashboard delivery; a saved
request must not be retried merely because the dashboard gateway is unavailable.

Multi-device datasets and concurrent HTTP/HTTPS tests are available in
[load-test/README.md](load-test/README.md). Export with
`python3 run-and-install/load-test/export-devices.py --limit 1000` from the project
root; the exporter is read-only. The k6 runner performs real writes and requires
an explicitly configured isolated test API URL and credentials.
