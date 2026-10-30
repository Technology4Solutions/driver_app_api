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
