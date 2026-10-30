# Multi-device HTTP/HTTPS load tests

Export IDs without writing to the database (Python 3 and the MySQL CLI required):

```bash
python3 run-and-install/load-test/export-devices.py --limit 1000
```

The exporter reads connection settings from `.env`; override with `--env-file PATH`.
It writes `run-and-install/devices.local.json` with mode 600. That dataset is
currently tracked in this repository and included in deployment archives, but
excluded from Docker builds. It selects one combination per active device using active bus-device
relationships, driver relation type 19 (the legacy active-driver rule), active users,
active routes, and related active stops with coordinates. Start/end stops are the
minimum/maximum linked stop IDs, representative test stops rather than geographic
route order. Review these choices against your fixture requirements.

The export produced **8 eligible devices** from the current database snapshot.
There are more device records, but most do not have all these eligible relationships.
No synthetic devices or production records were inserted. Do not invent extra IDs
or repeat a device across VUs. For more than 8 concurrent devices, seed valid related
records in a dedicated test database and export again.

Run against an **isolated API with a matching database copy**. Configure that API's
Vision and ZeroMQ destinations to local test receivers. This runner does not redirect
the API's downstream services. It inserts traffic and deletes device activity.
The exported IDs alone do not isolate your running application.

```bash
export API_BASE_URL='https://YOUR_TEST_HOST/school_bus_cloud'
export REAXIUM_API_USER='YOUR_TEST_API_USERNAME'
read -r -s -p 'Test API password: ' REAXIUM_API_PASSWORD; echo
export REAXIUM_API_PASSWORD
export START_DATE='2026-10-22T08:00:00Z'
VUS=8 JOURNEYS=100 ./run-and-install/load-test/run.sh run-and-install/devices.local.json
unset REAXIUM_API_PASSWORD
```

Requires Docker and Python 3. The wrapper uses the current OS user for reading the
private fixture and Linux host networking for localhost test endpoints. HTTPS
certificate verification is enabled; use a trusted certificate. The API's local
port 8080 currently serves HTTP, not HTTPS. `API_BASE_URL` may include the legacy
`/school_bus_cloud` prefix. No real load test has been run by the setup work.

Default VUs equals the exported device count; default JOURNEYS is 10. Eight VUs ×
100 journeys produces 800 journeys / 1,600 requests if all complete. One virtual
user owns each device and performs start then end sequentially. Requests overlap
across devices; VUS does not specify requests per second. Set MAX_DURATION (default
30m) high enough to complete the run, especially if broadcasts time out.

A new UUID run ID is generated every invocation. Each journey uses a trace containing
that ID, the VU ID, and iteration number. START_DATE supplies wall-clock date fields
for the API; the literal Z is used to format deterministically, not to override the
API's VISION_TIMEZONE. Start/end are a minute apart; successive journeys advance two
minutes. Event time is simulated and does not follow real elapsed test duration.

The report includes HTTP timing, application success, database-save rate, and
completed journeys. HTTP 200 alone is not success. Code 3 with database_saved=true
is counted as a saved request but fails the application-success threshold. The test
still ends the saved journey; it never retries the start. An unsaved or unknown
outcome aborts the test without automatic retries. Aborts or time limits may leave
in-progress journeys; restore/reset the isolated test database between runs.

The runner does not replay missed broadcasts or disable delivery failures.
With the dashboard gateway stopped, expect a nonzero exit even if database saves
succeed. Credentials are supplied through environment variables, never committed
in the script or dataset. Do not enable HTTP debug output with real credentials.

Verify the runner independently of your database/API:

```bash
python3 run-and-install/load-test/test-runner.py
```

This launches a local mock server and uses actual k6 HTTP requests to verify concurrent
journeys, shared trace IDs, and handling of partial saves and failures.

References: [k6 per-VU iterations](https://grafana.com/docs/k6/latest/using-k6/scenarios/executors/per-vu-iterations/),
[k6 data parameterization](https://grafana.com/docs/k6/latest/examples/data-parameterization/).

## Interactive Bash launcher

From the project root:

```bash
./run-and-install/load-test/start.sh
```

Or from `run-and-install`:

```bash
./load-test/start.sh
```

It prompts for the API URL (default localhost:8080), username, hidden password,
and simulated start date (default today's date in Africa/Casablanca at 08:00).
It uses `devices.local.json` and defaults to **one device, one journey**. Existing
exported settings are reused. Local fixed credentials are loaded from `credentials.local.sh` when present;
exported API credentials take precedence. This local file has mode 600 and is
excluded from Git and Docker builds. Missing credentials are prompted for. The Z suffix is
only used for deterministic wall-clock formatting as described above.

For eight devices, one journey each:

```bash
VUS=8 JOURNEYS=1 ./run-and-install/load-test/start.sh
```

You can pass a different dataset path as the first argument. Starting this script
runs real POSTs; it does not provision or isolate your API/database.

## Test one endpoint at a time

From `run-and-install`, use:

```bash
SERVICE=beginARoute VUS=1 JOURNEYS=1 ./load-test/start.sh
SERVICE=endARoute VUS=1 JOURNEYS=1 ./load-test/start.sh
```

`SERVICE=startARoute` is also accepted as an alias for `beginARoute`.
`SERVICE=journey` (the default) calls both endpoints. In single-service mode,
JOURNEYS means repetitions per device: VUS=8 JOURNEYS=10 sends 80 requests to only
the selected endpoint. The summary shows `service_iterations_completed`, not
`journeys_completed`. All response checks and failure thresholds remain active.

Begin-only tests leave device activity rows in the test database. End-only tests
clear the selected devices' activity rows and are allowed even without an existing
activity. Each invocation generates new trace IDs. For a matching start/end pair,
use journey mode; two separate default runs do not represent the same journey.

## DeviceStatus updates

From `run-and-install`:

```bash
SERVICE=deviceStatus VUS=1 JOURNEYS=1 ./load-test/start.sh
```

Each repetition sends only `POST /device/DeviceStatus/update`. The exporter now
includes `device_imei` and the driver's `business_id`; re-export older datasets.
Use VUS=8 to test eight exported devices concurrently. This inserts location data
and updates device status. Like the original sample, it omits `device_activity`,
which **clears existing device activity**. Run against a test database.
The runner strips a trailing `/school_bus_cloud` from the base URL for this endpoint,
since this API registers DeviceStatus at `/device/DeviceStatus/update` only.
# Run on a deployment server

Jenkins deploys the tracked project files, including these test scripts and the
tracked `run-and-install/devices.local.json` dataset. After committing and pushing
the updated deployment pipeline, rerun deployment; an image rebuild is not needed
just to deliver the scripts.

```bash
cd /home/ubuntu/installation/git_base_files/driver_app_api/MARTIN_COUNTY_DEV/run-and-install
SERVICE=deviceStatus VUS=8 JOURNEYS=100 ./load-test/start.sh
```

Use a dataset valid for the target environment with at least eight devices. You
can pass a different dataset as the final argument to `start.sh`. The launcher
prompts for the API address, credentials and simulated date. Use the API username
and password configured for that environment. The server needs Docker access and
Python 3; k6 runs in Docker. This command sends up to 800 real device-status
requests and writes to the configured database.

The archive includes every Git-tracked file, including tracked `.env`,
`credentials.local.sh`, and datasets. These replace matching files on the server
during deployment. Untracked/ignored local files and Git history are not included;
server-only files are left in place.
