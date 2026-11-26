# Multi-device HTTP/HTTPS load tests

Export IDs without writing to the database (Python 3 and the MySQL CLI required):

```bash
python3 run-and-install/load-test/export-devices.py --limit 1000
```

The exporter reads connection settings from `.env`; override with `--env-file PATH`.
Local preparation also accepts the same datasource names used in Vault:

```dotenv
SCHOOL_BUS_DATASOURCE_URL=jdbc:mysql://127.0.0.1:3306/school_bus
SCHOOL_BUS_DATASOURCE_USERNAME=local_user
SCHOOL_BUS_DATASOURCE_PASSWORD='local_password'
```

Put these in the local `.env` for the Docker web runner, or export them when
running the exporter directly. Existing `DB_*` settings take precedence over
the corresponding `SCHOOL_BUS_DATASOURCE_*` settings, matching the API's mapping;
remove conflicting `DB_*` settings to switch entirely to the shared names.
Within either naming scheme, exported environment values override file values.
Embedded `#` characters in passwords are preserved; an unquoted comment begins
after whitespace. Single quotes preserve literal `$` characters. Existing `DB_*`
environment variables take precedence over file values, as in the API.
After updating the exporter, deploy the updated source to the dev runner's project
directory; preparation reads this source through its bind mount.
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
Vision and HTTP dashboard destinations to local test receivers. This runner does not redirect
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

The report includes HTTP timing, application success, database-save or queue-acceptance rate, and
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
exported settings are reused. Load credentials from Vault with:
`python3 ci_cd/vault-exec.py run-and-install/load-test/start.sh` (from the repository root).
The Z suffix is
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

Use `SERVICE=beginRouteAccess` for **Begin route + Access** in the CLI or web
selector. Each iteration sends `beginARoute`, then passenger access one simulated
minute later, sharing the device, route, and trace ID. This scenario requires
`passenger_id` and `access_type` in the dataset and leaves the route open. Reset
the isolated database between runs as needed.

```bash
SERVICE=beginRouteAccess VUS=1 JOURNEYS=1 ./load-test/start.sh
```

`SERVICE=startARoute` is also accepted as an alias for `beginARoute`.
`SERVICE=journey` (the default) calls both endpoints. For single-endpoint services,
JOURNEYS means repetitions per device: VUS=8 JOURNEYS=10 sends 80 requests to only
the selected endpoint. The summary shows `service_iterations_completed`, not
`journeys_completed`. All response checks and failure thresholds remain active.

Passenger access is a trip scenario: selecting `executeAnAccessOfAUserInABus`
automatically sends `beginARoute → executeAnAccessOfAUserInABus → endARoute`.
Each repetition uses one device, route and trace ID across all three requests.
The exported passenger must belong to that route. For example, 8 devices and 10
repetitions send 240 requests (80 passenger events and 160 route events).
The web selector shows this sequence and request count. The test ends each route
after its passenger event, so routes are not left running on the dashboard.

The simulated start is New York wall time. Passenger `accessDate` milliseconds are
converted to the corresponding instant, so their stored local time lies between
the start and end events rather than shifting by four or five hours. Passenger
tests support dates from 2007 onward and reject ambiguous/nonexistent DST times
before sending that iteration's requests.

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

The archive includes Git-tracked source files and datasets. Private runtime files
are ignored and must never be added to the archive. These replace matching files on the server
during deployment. Untracked/ignored local files and Git history are not included;
server-only files are left in place.

## Web control page (local and DEV)

Start the API and monitoring using your existing local scripts, then start the
control page in a second terminal (Docker access required):

```bash
bash run-and-install/load-test/web.sh
```

The launcher builds its Docker image (cached on subsequent runs) and starts the
page container with its port published only on localhost. Log in as `runner` using
the generated password printed in the terminal. Ctrl+C stops and removes the container.
No separate Docker commands are needed.

For HTTPS behind Apache, configure the exact browser origin with
`--public-origin https://dev-driver-app-k6.reaxium.com` (or
`K6_WEB_PUBLIC_ORIGIN`). Jenkins defaults to this DEV hostname; override
`K6_WEB_PUBLIC_ORIGIN` when using another hostname. The runner accepts that origin
with either the proxy's local Host header or the preserved public Host header.
Browser authentication and the `X-Runner-Token` check remain required. Refresh the
page after restarting the runner to load its new request token.

Open **http://127.0.0.1:5665**. Select the service, concurrent devices, repetitions,
simulated date and maximum duration. Enter the API credentials and confirm that
the target uses a test database. Start with one device and one repetition.
The page runs the existing `routes.js`, shows live console output, and displays
response-time charts, request count, HTTP failure rate, application success and
database-save rate after completion. CPU/memory remain in your existing Grafana
dashboard (locally http://localhost:3000). Stop terminates the actual k6 container;
it cannot undo requests already sent. Only one test runs at a time. The last result
is held in memory and replaced on the next run; credentials are not saved by the UI.

For a different API or dataset:

```bash
bash run-and-install/load-test/web.sh \
  --api-url http://127.0.0.1:8080 \
  --dataset run-and-install/devices.local.json
```

On DEV, use the Jenkins deployment job described below. It starts a Docker
container automatically; no manual Python process or SSH tunnel is needed.

### Automatic DEV deployment through Jenkins

The Jenkins deployment job installs and starts the control page automatically
for `ENVIRONMENT=DEV`, including `APPLICATION_ONLY` deployments. It runs as
`driver-app-k6-web` in Docker, publishes port 5665, and uses `unless-stopped`
to restart after a crash or server reboot. Jenkins builds its runtime image
on the deployment host. The old systemd service is disabled automatically.
Jenkins stops it before replacing project files (stopping an active test), then
restarts it after API deployment and verifies authenticated HTTP access. PROD
has no runner deployment stage. Local use remains `bash run-and-install/load-test/web.sh`.

One-time setup:

1. In Jenkins Credentials, add **Secret text** with ID **k6-web-access-token**.
   Choose a random password of at least 20 characters without whitespace.
2. The deployment SSH user needs Docker access. On the first migration only,
   passwordless sudo is needed to disable the existing systemd service.
   Python is supplied by the container image.
3. Allow inbound TCP **5665** from your IP in the DEV server security group.

Commit/push the files to `develop`, then run the existing Jenkins deployment job.
No application image rebuild is required for the web runner. Jenkins uses the
host IP already obtained from Vault and reports `http://HOST_IP:5665` on success.
Log in with username **runner** and the password stored in the Jenkins credential.
The credential is stored in a private `.k6-web/access-token` file on the server,
excluded from Git and Docker builds; it is not printed in service logs.
HTTP is unencrypted, including browser login and API credentials.


The web runner automatically reads `REAXIUM_API_USER` and `REAXIUM_API_PASSWORD`
from the project-root `.env` file. Exported environment variables take precedence.
When both are configured, the page hides the credential fields and uses them on
the server; it never sends these values to the browser. If either is missing,
the page asks for both credentials. Use `--env-file /path/to/.env` to select another
file. Restart the runner after changing credentials (rerun deployment on DEV).
The DEV browser login (`runner` / Jenkins credential) remains separate.


The web runner supports the deployment host's Python 3.5+ runtime without extra
packages. It uses OS-provided secure randomness, constant-time login comparison,
and a threaded HTTP server compatible with older Python installations.


### DEV browser access

Jenkins publishes Grafana at `http://DEV_SERVER_IP:3000` on DEV only; Prometheus
remains private. The test page listens at `http://DEV_SERVER_IP:5665` after its
Jenkins deployment stage succeeds. Allow inbound TCP ports 3000 and 5665 from your
public IP in the EC2 security group and any host firewall. This repository does
not change AWS security groups. Grafana uses its own existing admin credentials;
the test page uses `runner` and the Jenkins secret. HTTP traffic is unencrypted.

If a page is unreachable, run on the server:

```bash
docker inspect --format '{{.State.Status}} {{.State.Health.Status}}' driver-app-k6-web
curl -s -o /dev/null -w 'Test page HTTP %{http_code}\n' http://127.0.0.1:5665/
curl -s -o /dev/null -w 'Grafana HTTP %{http_code}\n' http://127.0.0.1:3000/api/health
```

The test page should return 401 (login required); Grafana health should return 200.
If these succeed but your browser times out, check the public IP and firewall rules.

The runner mounts the Docker socket to launch sibling k6 containers and requires
trusted access to the host Docker daemon. The project is mounted read-only at its
host absolute path; `.k6-web/results` is a separate writable mount for temporary
summaries. k6 uses host networking, so its default API URL remains
`http://127.0.0.1:8080`. The control page uses Docker bridge networking with
`5665:5665` published. Troubleshoot with `docker logs --tail 60 driver-app-k6-web`.

## Coverage for every business service

The CLI and web page share `services.json`. Select any of these `SERVICE` values:

| SERVICE | Endpoint |
|---|---|
| `bulkUserAccess` | `/api/DeviceAccess/bulkUserAccess` |
| `saveDeviceTrafficAttendance` | `/api/Device/saveDeviceTrafficAttendance` |
| `saveTrafficAttendance` | `/api/Device/saveTrafficAttendance` |
| `deviceStatus` | `/device/DeviceStatus/update` |
| `beginARoute` | `/DeviceTraffic/beginARoute` |
| `endARoute` | `/DeviceTraffic/endARoute` |
| `interruptARoute` | `/DeviceTraffic/interruptARoute` |
| `sendTraffic` | `/DeviceTraffic/sendTraffic` |
| `beginAMergeRoute` | `/DeviceTraffic/beginAMergeRoute` |
| `endAMergedRoute` | `/DeviceTraffic/endAMergedRoute` |
| `interruptAMergedRoute` | `/DeviceTraffic/interruptAMergedRoute` |
| `sendNotification` | `/Alarm/sendNotification` |
| `sendNextStopNotification` | `/Alarm/sendNextStopNotification` |
| `sendSkippedStopNotification` | `/Alarm/sendSkippedStopNotification` |
| `sendManualSkippedStopNotification` | `/Alarm/sendManualSkippedStopNotification` |
| `executeAnAccessOfAUserInABus` | `/UserAccess/executeAnAccessOfAUserInABus` |

`SERVICE=journey` remains begin + end. `SERVICE=allServices` sends each of the
16 business requests once per iteration, in the order in `services.json`. One VU
owns one device; all requests in an iteration share a unique trace. Start with:

```bash
SERVICE=sendNextStopNotification VUS=1 JOURNEYS=1 ./run-and-install/load-test/start.sh
SERVICE=allServices VUS=1 JOURNEYS=1 ./run-and-install/load-test/start.sh
```

The web selector offers the same services and shows the expected request count.
`allServices` is a coverage scenario, not a realistic bus trip: it exercises normal,
merged, and interrupted endings in the same iteration. It does not wait for the
asynchronous DeviceStatus worker, which can change device activity after later
requests. Run the individual services when measuring endpoint performance.

### Additional fixture requirements

Re-export datasets using `export-devices.py` for the new services. The SELECT-only
export also includes:

- `route_ids`: comma-separated active routes assigned to the bus, with valid linked
  stops. Merged tests require at least two distinct routes; single-route devices
  remain usable for the other tests.
- `passenger_id` and `access_type`: an active passenger on the selected route with
  a matching `user_access_data` entry. Null values mean this device cannot run the
  passenger or all-services test. These fields are IDs, not credential secrets.

Existing datasets continue to work for existing services. Missing related IDs fail
validation before k6 sends any request. Choose eligible rows in a separate dataset
or seed the necessary relationships in the isolated database; do not substitute
unrelated passenger or route IDs. The exporter does not modify the database.

`sendNotification` uses notification type 9; next stop uses 5; skipped stop uses 18;
manual skipped stop includes `traffic_date`. `sendTraffic` uses traffic type 15.
Passenger access uses boarding type 1 with the selected credential method. These
are baseline success scenarios, not every possible business input combination.
End/interruption tests can remove device activity. Merged interruption can also
create automatic passenger dropoffs for the shared trace.

### Results and verification

HTTP 200 alone does not pass a test. Every endpoint has an application-success
threshold. `database_saved` covers synchronous services; `queue_accepted` covers
DeviceStatus, bulk passenger access and both attendance services, whose successful responses only confirm
enqueueing. Verify actual persistence separately after their workers process the queues.
Merged error responses are checked route by route: incomplete/unknown saves abort;
confirmed saves with downstream failures fail the success threshold without retries.

The local verification commands never call your deployed API:

```bash
node run-and-install/load-test/test-payloads.mjs
python3 run-and-install/load-test/test-runner.py
python3 -m unittest discover -s run-and-install/load-test/web -p test_server.py
```

The k6 mock checks exercise all services, the all-services sequence, namespaces,
authentication, unique traces, validation, and partial-save failures. Node checks
also compare the service catalog with the registered Rust business routes.

Commit and deploy the runner files to update the DEV web page; the API binary
has not changed and does not need rebuilding for these tests.

### Prepare fresh passenger test data

The web page and CLI refresh data automatically. To refresh the source dataset
manually, use the date selected in the test page:

```bash
./run-and-install/load-test/prepare-data.sh 2026-11-14
```

Omit the date to use today in Africa/Casablanca. The script works from any working
directory and forwards exporter options such as `--env-file`, `--output`, and `--limit`.

The SELECT-only export selects active `users.user_type_id = 2` passengers assigned
through `route_users`, with a matching `user_access_data` method. Existing traffic
records do not exclude passengers. Each repetition cycles through the route's
passenger pool, creating a new event with its own trace and simulated timestamp.
At least one valid passenger per device is required for passenger scenarios.
Repeated events do not increase a dashboard's distinct-passenger count; compare
request and traffic-event counts when assessing load-test volume.

### Automatic preparation before every test

The web page and `run.sh` now prepare data automatically before launching k6.
The page shows **preparing**, then **running**. Live output retains preparation
results and the selected device IDs. Preparation refreshes valid device relationships
from the configured database and selects from the freshly exported devices. Old
device IDs in the source snapshot do not restrict online selection. It selects
devices that meet the chosen service's requirements, including
merged routes, status fields, and valid passengers with matching access methods.

For example, 8 devices × 100 repetitions runs 800 passenger events even if each
route has only one passenger. Shared route passengers can be used by multiple
devices. Preparation never creates users or deletes existing traffic. Each run
uses a private temporary snapshot and leaves the source dataset unchanged.
Simulated events may cross midnight; ambiguous or nonexistent New York DST times
are still rejected before execution.

CLI preparation reads the project `.env` by default; set `LOAD_TEST_ENV_FILE` to
use another configuration file. The local web runner uses its `--env-file` setting.
For DEV, Jenkins reads `kv/reaxium_ci/device_api/<CLIENT>_<ENVIRONMENT>` and
provisions `.k6-web/runtime-config.json` (directory mode 700, file mode 600).
The web runner and preparation helper use `--config-file` to read that private
JSON directly: `SCHOOL_BUS_DATASOURCE_URL`, `SCHOOL_BUS_DATASOURCE_USERNAME`,
`SCHOOL_BUS_DATASOURCE_PASSWORD`, `REAXIUM_API_USER`, and `REAXIUM_API_PASSWORD`.
DEV does not fall back to `.env` database credentials. JSON preserves raw Vault
passwords without shell or dotenv expansion. Credentials are not sent to the
browser or included in Docker command arguments. Redeploy with Jenkins to
provision this file and restart the runner; updating source alone is insufficient.
After Vault credentials change, redeploy to refresh the private configuration.
The web image includes the MySQL client and runs preparation in a helper container
with host networking, matching k6's access to local database tunnels. Rebuild and
restart the web runner to apply these changes; the API image does not need rebuilding.
`LOAD_TEST_OFFLINE=1` is reserved for the mock suite: it validates supplied fixtures
without querying a database. Real runs always use automatic database preparation.

```bash
python3 run-and-install/load-test/test-prepare.py
```


### Attendance

The web runner's **Service → Attendance** section contains device attendance events
and user attendance access. Both also run in `allServices` (16 endpoints).

```sh
SERVICE=saveDeviceTrafficAttendance VUS=1 JOURNEYS=1 ./run-and-install/load-test/start.sh
SERVICE=saveTrafficAttendance VUS=1 JOURNEYS=1 ./run-and-install/load-test/start.sh
```

Preparation selects an active user within each device driver's business and reads
existing activity/place IDs, preferring user assignments and business references,
then falling back to existing reference rows in the test database. An
`activities_users` assignment is not required by these services. User attendance
also requires an access method from `user_access_data` and traffic type **1 (IN)**;
device attendance requires type **21 (activity end)**, without an access method or
participant assignment. Missing required reference rows stop the run before API
requests. No reference data or relationships are created.
The device event uses the driver's user ID; individual access uses the selected
attendance user's ID and access method. Payloads use fresh run trace IDs, activity
end type 21, and user IN type 1. Attendance requests go to `/api/Device/` at the API
root, including when API_BASE_URL ends in `/school_bus_cloud`.

`queue_accepted` measures HTTP 200/application code 0 for these endpoints;
`database_saved` excludes them. Queue success does not prove insertion. Check worker
errors and query the run's trace prefix after processing:

```sql
SELECT * FROM device_traffic_attendance WHERE trace_id LIKE 'load-YOUR_RUN_ID-%';
SELECT * FROM traffic_attendance WHERE trace_id LIKE 'load-YOUR_RUN_ID-%';
```

Individual attendance timestamps use server processing time; device attendance
uses the simulated time. Repeated valid events insert separate rows. Restart the
web runner after updating so its service catalog is reloaded, then refresh the page.


### Bulk passenger access

Select **Service → Passenger access → Bulk passenger access** in the web runner,
or run:

```sh
SERVICE=bulkUserAccess VUS=1 JOURNEYS=1 ./run-and-install/load-test/start.sh
```

Each repetition sends one HTTP request to `/api/DeviceAccess/bulkUserAccess`, with
an array under `ReaxiumParameters.BulkUserAccess`. A batch contains up to 10 distinct
eligible passengers for that device's route. Each passenger retains their matching
access method; types use IN (1). Small pools send their available passengers,
including a one-passenger batch when only one is available. Repetitions rotate
through the pool, reuse identities with new run/iteration traces, and do not invent
user IDs. Preparation refreshes and validates passenger references before k6 starts.
Simulated New York wall time is converted to Unix milliseconds for `accessDate`;
ambiguous/nonexistent DST times are rejected before requests.

HTTP request counts measure batches, not passenger rows. The run also requires
all requested repetitions to complete; hitting the maximum duration with dropped
iterations now fails the completion threshold instead of reporting a pass. `queue_accepted` measures
HTTP 200/application code 0; these responses are never counted as `database_saved`.
The endpoint is also included in `allServices` (16 endpoints). API bases ending in
`/school_bus_cloud` are normalized to the API root for this queued service.

After workers drain, verify the run separately against the configured test database:

```sql
SELECT trace_id, COUNT(*) AS passenger_events
FROM traffic
WHERE trace_id LIKE 'load-YOUR_RUN_ID-%'
GROUP BY trace_id;
SELECT id, topic, status, device_id FROM messages
WHERE topic = 'DEVICE_ACCESS' ORDER BY id DESC LIMIT 10;
```

Queue acceptance cannot establish that every member was stored; inspect worker
errors and saved rows for skipped users, duplicates, or integrity failures.
Restart the web runner and refresh the page after updating the service catalog.
