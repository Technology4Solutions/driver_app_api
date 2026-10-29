# Rust REST API starter

`GET /` returns `Hello, world!`. Device status updates use the Java-compatible RabbitMQ workflow; `/device/DeviceStatus/update` remains available as an alias.
MySQL is connected at startup through a SQLx connection pool. Schema migrations are not included.

## Structure

- `services/device-api/src/lib.rs`: HTTP routes and application composition.
- `services/device-api/src/database.rs`: MySQL configuration and connection pool.
- `services/device-api/src/device_status/`: telemetry validation, authentication, persistence, and broadcasts.
- `services/device-api/src/main.rs`: configuration, listeners, and graceful shutdown.
- `services/device-api/src/telemetry.rs`: request counts and latency.
- `services/device-api/tests/http.rs`: Hello World and monitoring tests.
- `monitoring/`: optional Prometheus, alerts, and Grafana dashboard.
- `.github/workflows/ci.yml`: formatting, lint, tests, and release build.

The Cargo workspace supports additional services. Handlers share a SQLx MySQL connection pool.

## Run

Bash helpers for setup, build, start/stop, logs, and API tests are in
[`run-and-install/`](run-and-install/README.md). Start with `./run-and-install/build.sh`
and `./run-and-install/run.sh`.

Install Rust with rustup, then:

```sh
# If .env does not exist yet, copy .env.example to .env.
# Set the remote MySQL host, port, database, username, and password in .env.
cargo run -p device-api
curl http://localhost:8080/
# Hello, world!
```

Alternatively:

```sh
docker compose up --build -d
# Include optional monitoring:
docker compose --profile monitoring up --build -d
```

The API listens on localhost:8080. The separate admin listener on localhost:9090 exposes `/health/live` and `/metrics`. With monitoring enabled, Prometheus is at localhost:9091 and Grafana at localhost:3000 (initial local login admin/admin). Grafana includes a provisioned dashboard.

Local monitoring uses Linux host networking, matching the API, so Prometheus can
reach the private admin listener. Keep the default admin port 9090 or update
`monitoring/prometheus.yml` to match. Open Grafana's **Dashboards → Device API**.
CPU and memory panels require rebuilding the API image with the process collector.
To start only monitoring for an already running native API, use
`docker compose --profile monitoring up -d --no-deps prometheus grafana`.
Generate traffic with `curl http://127.0.0.1:8080/` and allow at least two 15-second
scrapes for rates. Check http://localhost:9091/targets if panels are empty.

Configuration is loaded automatically from `.env`; existing process environment variables take precedence. `.env` is excluded from Git and Docker build contexts. Use `.env.example` as a template.

Database parameters: `DB_HOST`, `DB_PORT`, `DB_NAME`, `DB_USER`, `DB_PASSWORD`, `DB_MAX_CONNECTIONS`, and `DB_CONNECT_TIMEOUT_SECONDS`. All are required for native execution. The API verifies the connection before opening HTTP listeners and closes the pool on graceful shutdown. `API_BIND`, `ADMIN_BIND`, and `RUST_LOG` configure listeners and logging.

Both native execution and Docker Compose connect to the existing remote MySQL server using the settings in `.env`. No database container is created. The database and user must already exist, and the server must be reachable from the API. The current Compose API uses Linux host networking and binds its HTTP listeners to localhost by default.

## Test

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Tests run in-process without a database or external services and verify the Hello World response and monitoring isolation.

To also verify the MySQL connection using `.env`:

```sh
cargo test -p device-api --test database -- --ignored
```

JSON logs and Prometheus metrics are included. Alert notification delivery requires Alertmanager configuration later. The DeviceStatus endpoint requires shared Basic authentication. The greeting and admin endpoints do not require authentication; keep the admin listener private.

## Device status update

`POST /api/DeviceStatus/updateStatus` follows the Java `reaxium-device-api` queue workflow.
The existing `/device/DeviceStatus/update` path is an alias. Both accept JSON in the
`ReaxiumParameters.DeviceStatus` envelope and keep the existing shared Basic authentication
(`REAXIUM_API_USER` / `REAXIUM_API_PASSWORD`, with the existing rotation aliases).

The HTTP handler validates input and publishes a persistent RabbitMQ message with a publisher
confirmation. It returns HTTP 200 once queued, before any database processing:

```json
{"ReaxiumResponse":{"code":0,"message":"Device Status queued","object":[]}}
```

Invalid payloads and queueing failures return HTTP 400 with envelope code 500 and
`Error reporting device status`. Authentication failures retain HTTP 200/code 3.
A queued response does not mean a database write has succeeded. Unknown devices, filtered
updates, and worker failures are reported in logs rather than in the already-completed HTTP request.

The durable topic exchange `devicesAPIExchange` is bound to durable queue `devicesAPIQueue`
with `reaxium.devices.api.#`, matching Java. Each process has one consumer, prefetch 1,
and manual acknowledgements. Filtered updates and unknown devices are acknowledged;
processing errors are rejected without requeue. Saved messages are acknowledged before
asynchronous HTTP broadcasting; broadcast failures are logged and do not retry telemetry.
Connections recover after broker outages. Startup requires RabbitMQ; publishing during an
outage fails instead of reporting success. Shutdown drains in-flight work within a bounded timeout.

Redis uses Java's `device:last_status_date:<imei>` key, the same date string, and a 24-hour TTL.
Updates with older/equal timestamps or less than 15 seconds since the last accepted update
are discarded. `DEVICE_STATUS_MIN_UPDATE_INTERVAL_SECONDS` overrides the interval.
Like Java, this cache advances before persistence and fails open when Redis is unavailable.
The Java status receiver does not pass coordinates to its filter, so this worker also uses
only timestamps. Rust performs the Redis check/update atomically across Rust replicas.

For accepted updates, the worker finds an active device by IMEI, inserts a location, upserts
status, and upserts activity (or deletes it when absent/null). `last_sync` is replaced from the
request, with missing/invalid values becoming NULL. `bus_info` remains NULL when omitted.
Activity timestamps use server processing time in UTC, rather than device time. Missing users
produce a NULL location user, as in Java. Rust retains one atomic transaction and device-row
locking, rather than reproducing Java's separate location/activity transactions. Rust also retains
strict date/coordinate/required-field validation. Charging accepts Java boolean/numeric values.

Status broadcasts now use HTTP `WebSocketPusher/dispatch`, with the envelope
`ReaxiumParameters.WebSocketPusher.content`, Java-style device/status/activity fields,
device serial as `gps_serial`, passenger ID lists, and `last_associated_traffic=3`.
The route stop join uses Java's `routes_stops_relationship` table. Configure dispatcher Basic auth
explicitly; Java's hardcoded dispatcher credentials are not copied into Rust.

```dotenv
APP_ENV=local
# Blank/unset: disabled for local, enabled for other APP_ENV values.
BROADCAST_ENABLED=
DEVICES_API_EXPORT_DEVICE_STATUS_URL=https://your-backend/school_bus_cloud/WebSocketPusher/dispatch
DEVICE_STATUS_BROADCAST_USERNAME=your_dispatcher_username
DEVICE_STATUS_BROADCAST_PASSWORD=your_dispatcher_password
RABBITMQ_URL=amqp://guest:guest@127.0.0.1:5674/%2f
REDIS_URL=redis://127.0.0.1:6381/
DEVICE_STATUS_QUEUE=devicesAPIQueue
DEVICE_STATUS_EXCHANGE=devicesAPIExchange
DEVICE_STATUS_ROUTING_KEY=reaxium.devices.api.#
DEVICE_STATUS_MIN_UPDATE_INTERVAL_SECONDS=15
```

`APP_ENV` defaults to `local` for direct execution. Set it explicitly in non-local deployments.
Enabled broadcasts require a URL and credentials at startup. HTTP connection/request timeouts
are 3 seconds. `DASHBOARD_BROADCAST_ENABLED` and ZeroMQ settings now apply only to route endpoints.

Compose includes persistent RabbitMQ and Redis services on loopback ports 5674/15674 and 6381,
separate from the Java local stack's ports. The API uses host networking to reach these services
and the existing database tunnel. To run against a shared broker/cache, override the URLs;
using Java's queue means Java and Rust become competing consumers, so each message is handled
by only one of them. Use a distinct queue/exchange for isolated testing.

```sh
docker compose up --build -d api
curl -i http://127.0.0.1:8080/api/DeviceStatus/updateStatus \
  --user 'API_USERNAME:API_PASSWORD' -H 'Content-Type: application/json' \
  --data-binary @services/device-api/tests/fixtures/device_status.json
```

### Tests and troubleshooting

`cargo test --workspace` covers authentication, validation, endpoint routing, and unavailable
publisher behavior. The integration test requires disposable MySQL, RabbitMQ, and Redis:

```sh
TEST_DATABASE_URL='mysql://TEST_USER:TEST_PASSWORD@127.0.0.1:3306/device_api_migration_test' \
TEST_RABBITMQ_URL='amqp://guest:guest@127.0.0.1:5672/%2f' \
TEST_REDIS_URL='redis://127.0.0.1:6379/' \
  cargo test -p device-api --test device_status -- --ignored
```

The test verifies the database name before resetting fixture tables. It checks queue consumption,
filtering/TTL, activity updates/deletion, `last_sync`, rollback, poison-message rejection, HTTP
broadcast content/authentication, broadcast failure without requeue, restart with pending messages,
and disabled broadcasting. CI provisions all three services. Never use application services for tests.

Watch `docker compose logs -f api`. A queued HTTP response confirms broker acceptance only;
`device status transaction committed` confirms database writes. Processing failures identify the
failed table and operation. Broadcast failures occur after acknowledgement and do not undo saves.

## Start a route (ported from ReaxiumCloud)

`POST /DeviceTraffic/beginARoute` (also `/school_bus_cloud/DeviceTraffic/beginARoute`)
uses the same `.env` Basic authentication as DeviceStatus. The payload is in
[`run-and-install/json/begin-route.example.json`](run-and-install/json/begin-route.example.json).
Numeric fields accept JSON numbers or numeric strings. Dates use `YYYY-MM-DD HH:MM:SS`
(the legacy `HH::MM::SS` format is also accepted). Optional `users_in_track` is a
comma-separated string, as in the original route activity DAO.

The handler inserts `device_traffic` with type **3** (route start), situation **1**,
a snapshot of the stop number/name, and the bus number looked up using the request’s
`bus_info` as `bus.bus_id`. It inserts
a new `device_activity` in the same transaction. Existing database triggers execute
normally; their writes roll back with the transaction when using transactional tables.
The endpoint requires the existing ReaxiumCloud schema, including the stop snapshot
columns. Test fixture SQL is **not** a production migration. A missing trigger definer
still needs correcting in MySQL; this port does not change database users or triggers.

After commit, it sends the original Vision route-start event **401** via UDP and
publishes `ROUTE_MAJOR_TRAFFIC` through the existing ZeroMQ dashboard gateway.
The dashboard payload matches `GET_DEVICE_TRAFFIC_INFO_BY_ID`; the port uses a bound
SELECT instead of calling that procedure. Channels use the device's most recent
traffic route, matching ReaxiumCloud. The two downstream operations are attempted
independently, each bounded by `DEVICE_STATUS_BROADCAST_TIMEOUT_SECONDS`.

Configure in `.env` (Compose passes these to the API):

```dotenv
VISION_UDP_DESTINATION=YOUR_VISION_SERVER_IP:10004
VISION_TIMEZONE=America/New_York
DEVICE_STATUS_ZMQ_ENDPOINT=tcp://127.0.0.1:5555
DEVICE_STATUS_BROADCAST_TIMEOUT_SECONDS=5
```

`VISION_UDP_DESTINATION` accepts an IPv4 `IP:port` or IPv6 `[IP]:port`. The local
`.env` initially uses the destination found in ReaxiumCloud; change it for your
environment. `VISION_TIMEZONE` interprets the device's local timestamp for Vision,
matching the legacy New York default. Ambiguous/nonexistent daylight-saving times
are reported as a Vision error. The UDP encoder preserves the original PHP wire
format (including its nonstandard header-length and coordinate padding).

The legacy HTTP-200 envelope is retained. Code `0` means the database committed
and applicable downstream sends completed. UDP has no receiver acknowledgement,
and ZeroMQ acceptance does not confirm dashboard delivery. Inactive devices skip
Vision, as in the source. No interested businesses means no dashboard messages.
An empty Vision destination is reported as `vision_status: "not_configured"`,
not silently counted as success.

Code `3` with `object.database_saved: true` means the rows **are saved** but a
downstream step failed, timed out, or is unconfigured. The object includes both
inserted IDs and `vision_status`/`broadcast_status`. **Do not retry that POST**:
`trace_id` is not an idempotency key; repeated starts can create duplicate traffic
and activity rows. Database errors also return code `3`; inspect logs and rows
before retrying if a connection failed during commit.

Logs include per-table pending writes, a `begin route transaction committed` event
with both IDs, and final delivery statuses. A pending write log alone does not
prove the transaction committed. No `device_location` or `device_status` row is
written by this endpoint.

```bash
./run-and-install/run.sh
./run-and-install/test-api.sh --begin-route run-and-install/json/begin-route.example.json
```

The POST test prompts for credentials unless supplied through the shell environment.
Update the sample IDs/date/trace first: this command writes to your configured database
and sends to your configured downstream services. It does not retry the POST.

The ignored `begin_route` integration test requires a disposable MySQL database named
`device_api_migration_test`; it replaces the test schema. CI runs it after the existing
DeviceStatus integration test. It uses local UDP/ZeroMQ receivers, verifies automatic
trigger writes, rollback, TIMESTAMP serialization, and truthful partial-failure responses.

## End a route

`POST /DeviceTraffic/endARoute` and `/school_bus_cloud/DeviceTraffic/endARoute`
use the same authentication, Vision settings and dashboard gateway as `beginARoute`.
See `run-and-install/json/end-route.example.json` for the payload. In addition to the
start-route fields, `traffic_situation_id` is required (a positive integer or numeric
string). The endpoint forces traffic type **4**, inserts a stop snapshot in
`device_traffic`, and deletes **all `device_activity` rows for the specified device**,
matching ReaxiumCloud. Other devices' activities are untouched. An empty activity
set is allowed. Both writes and transactional trigger writes commit or roll back together.

After commit, Vision receives event **402** and dashboards receive
`ROUTE_MAJOR_TRAFFIC` with `typeTrafficId: 4`. Delivery failures return code `3` with
`database_saved: true`, the traffic ID, `activities_deleted`, a null `device_activity_id`,
and both delivery statuses. Do not retry a saved event: trace IDs do not deduplicate
requests. The gateway can be started later, but missed broadcasts are not replayed.

```bash
./run-and-install/run.sh
./run-and-install/test-api.sh --end-route run-and-install/json/end-route.example.json
```

Update the example IDs and timestamp before testing. The POST writes to the configured
database, clears the device's activity, and sends to the configured Vision/dashboard
services. No new schema changes are required beyond those used for `beginARoute`.

For both route endpoints, `bus_info` must contain a positive bus ID as a string
(for example `"364"`). It is stored unchanged in `device_traffic.bus_info`;
`assigned_bus` is populated from `bus.bus_number` for that ID. This intentionally
differs from the original PHP route-assignment lookup. No entry in
`bus_routes_relationship` is needed. If the bus is missing or its number is NULL,
`assigned_bus` remains NULL and the API logs a warning. This change applies to
new traffic records; it does not backfill existing records.

Vision delivery for both route endpoints can be disabled with `VISION_ENABLED=false`
in `.env` (currently set locally). This skips the Vision lookup and UDP send and
logs `vision_status: "disabled"`; intentional disabling is not an application error.
Database writes and dashboard broadcasts still run, so dashboard failures still
produce code 3. To restore Vision, set `VISION_ENABLED=true` and recreate the API
with `./run-and-install/run.sh`. The existing destination is retained.

Dashboard delivery can be disabled temporarily with
`DASHBOARD_BROADCAST_ENABLED=false` in `.env` (currently set locally). This applies
to both route endpoints and skips both dashboard data loading and
ZeroMQ publication. Logs show `broadcast_status: "disabled"`. With Vision also
disabled, successful database commits return code 0; validation/database errors still
fail. No skipped messages are queued or replayed. Set the switch to `true` and recreate
the API to restore broadcasts. Existing gateway settings are retained.

## Jenkins and Swarm deployment (DEV / PROD)

This setup follows `reaxium-device-api` **develop, cb0f90e**, using the `ci_cd/`
layout and Vault workflow. It replaces the earlier root-level Jenkinsfiles and
per-environment dotenv-secret setup. Local Dockerfile, compose.yaml, .env and
run-and-install scripts retain their original behavior.

Configure two Jenkins jobs with these script paths:

- `ci_cd/JenkinsfileBuild`: BUILD_SIZE = PATCH / MINOR / MAJOR. Clones develop,
  tests Rust, builds and pushes the API and RabbitMQ images for linux/amd64, then
  commits the incremented root VERSION to develop. VERSION starts at 0.1.0.
- `ci_cd/JenkinsfileDeploy`: CLIENT, ENVIRONMENT = DEV / PROD,
  DEPLOYMENT_TYPE = FULL_STACK / APPLICATION_ONLY / CLEAN_DEPLOYMENT,
  IMAGE_VERSION = LATEST or a published tag, and FORCE_DEPLOYMENT. LATEST resolves
  to the VERSION in the develop checkout; an explicit tag supports rollback.

Use the same Jenkins Vault setup as Java: vault-token credential, Vault URL
http://vault:8200, KV v2 paths `kv/jenkins/GITHUB_TOKEN` (token),
`kv/jenkins/HUB_DOCKER` (username/password),
`kv/jenkins/DEV_SSH_PRIVATE_KEY` or `PROD_SSH_PRIVATE_KEY` (username/private_key),
and `kv/jenkins/MACHINE` (`${CLIENT}_${ENVIRONMENT}_IP`). Git, Credentials Binding,
HashiCorp Vault and SSH Steps plugins are required. Configure SCM credentials on
the deploy job. The build defaults to Technology4Solutions/driver_app_api.git;
set Jenkins REPOSITORY_URL if the remote repository has a different name. Configure
both jobs' SCM against the actual repository; a local directory rename does not
rename the GitHub repository.

Deployment transfers a Git archive of ci_cd and VERSION to the target, rather than
storing a GitHub token in a remote clone URL. The remote account needs Docker and
write access to /home/ubuntu/installation/git_base_files. Authenticate Docker to
private images on the remote manager before deployment. Image authentication is
forwarded to Swarm. Jenkins secrets and temporary SSH keys are cleaned up afterward.

Runtime configuration matches Java's Vault model: create the external Swarm secret
`VAULT_TOKEN` on each target manager. The API and broker read the KV bundle at
`kv/reaxium_ci/driver_app_api/${CLIENT}_${ENVIRONMENT}`. Defaults are Vault address
https://vault.reaxium.com and KV version 2; VAULT_ADDR, VAULT_MOUNT, VAULT_BASE_PATH
and VAULT_KV_VERSION can be set in the deployment shell. Missing secrets stop startup.
DEV and PROD use separate Vault bundles on their respective target hosts, with two
API replicas by default. No filled development/production env files are required.

Populate each bundle with:

- DB_HOST, DB_PORT, DB_NAME, DB_USER, DB_PASSWORD; alternatively Java's
  SCHOOL_BUS_DATASOURCE_URL (jdbc:mysql://host:port/database),
  SCHOOL_BUS_DATASOURCE_USERNAME and SCHOOL_BUS_DATASOURCE_PASSWORD.
- REAXIUM_API_USER and REAXIUM_API_PASSWORD for device API authentication.
- RABBITMQ_USER and RABBITMQ_PASSWORD. They initialize the bundled broker and must
  continue to match existing broker storage. RabbitMQ and Redis have dedicated
  stack volumes and internal endpoints; host ports are not published.
- DEVICES_API_EXPORT_DEVICE_STATUS_URL, DEVICE_STATUS_BROADCAST_USERNAME and
  DEVICE_STATUS_BROADCAST_PASSWORD for enabled HTTP broadcasts. BROADCAST_ENABLED
  defaults to true for DEV/PROD; set false in the bundle to disable it.
- Existing optional DEVICE_STATUS_*, VISION_*, DB_* settings, DASHBOARD_BROADCAST_ENABLED,
  RABBITMQ_URL and REDIS_URL can also be supplied. Route dashboard broadcasts and
  Vision delivery retain their application defaults; configure their endpoints
  or explicitly disable them in the bundle as appropriate.

The Rust stack contains API, RabbitMQ and Redis. It connects to the existing
school-bus database and does not create Java's separate application database.
It uses the same external overlay my_reaxium_api_network; deploy.sh creates that
network and initializes Swarm if necessary. The default stack is driver_app_api_stack,
with API port 8080 and internal admin port 9090. Use separate target hosts for DEV
and PROD, as in the Java workflow.

Manual commands mirror Java's deployment interface:

```bash
bash ci_cd/build.sh --versions
bash ci_cd/build.sh -t patch --platform linux/amd64
bash ci_cd/deploy.sh --client MARTIN_COUNTY --env DEV --all
bash ci_cd/deploy.sh --client MARTIN_COUNTY --env PROD --app --version 0.1.1
bash ci_cd/deploy.sh --status
bash ci_cd/deploy.sh --remove
python3 ci_cd/test-deployment.py
```

### Deployment monitoring with Grafana

`/metrics` is the raw Prometheus feed. Build and deploy a new API image to enable
the new Linux process CPU, resident memory, threads, file descriptor and uptime
metrics. CPU at 100% means one CPU core; memory is API process memory, not total
server or container memory.

Jenkins includes monitoring and the optional Slack setup script in its deployment
bundle. Both DEV and PROD deployments automatically start/update Prometheus and
Grafana after API convergence, including APPLICATION_ONLY deployments. The build
validates the monitoring Compose configuration; deployment pulls the pinned
monitoring images and waits for their health checks. Docker Compose 2.24+ is
required on the build agent and target manager. No Slack webhook is required.

For a manual monitoring restart on the Swarm manager, run:

```bash
cd /home/ubuntu/installation/git_base_files/driver_app_api/MARTIN_COUNTY_DEV
bash ci_cd/monitoring.sh start
```

This joins the existing attachable overlay network and discovers every API replica
on internal port 9090. The deployment generates task DNS configuration for its stack name; for manual
use pass it as `bash ci_cd/monitoring.sh start CUSTOM_STACK`. Prometheus retains 15 days of metrics in a volume.
Grafana settings also persist in a volume. Neither service is publicly exposed.

From your laptop, open an SSH tunnel (replace `SERVER_IP`):

```bash
ssh -N -L 3000:127.0.0.1:3000 -L 9091:127.0.0.1:9091 ubuntu@SERVER_IP
```

Open http://localhost:3000, log in initially with `admin` / `admin`, change the
password when prompted, and open **Dashboards → Device API**. The dashboard shows
request rate, mean/p95/p99 response time, status codes, error ratio, availability,
CPU, resident memory, uptime, threads and file descriptors per API replica.
Rates need at least two scrapes and HTTP graphs need traffic. If graphs are empty,
check http://localhost:9091/targets: each API replica should be UP.

Monitoring configuration is reapplied automatically on each deployment.
`deploy.sh --status` includes monitoring status. `deploy.sh --remove` stops
monitoring and removes the API stack while preserving all data volumes.
CLEAN_DEPLOYMENT restarts both afterward. To stop only monitoring, use
`bash ci_cd/monitoring.sh stop`. The setup assumes one client/environment per
target host, as for the API; DEV and PROD retain separate data on their hosts.

`--app` updates only the API in an existing stack. When the stack has no services,
it creates the full stack for the first deployment. A partial stack missing its API
requires `--all` (Jenkins `FULL_STACK`) to repair it. `--all` updates the complete Rust stack. `--force`
recreates API tasks. CLEAN_DEPLOYMENT removes the stack first and preserves volumes.
Rollbacks, paused updates and convergence timeouts fail the deployment job. The
script reports the failing deployment stage and prints service/task status on
convergence failures so Jenkins shows Docker errors before the SSH exception. The
health check verifies the HTTP listener after startup, not ongoing dependency health.
Tests in test-deployment.py are offline; Jenkins/Vault/Swarm integration requires
configured infrastructure. Ignored database integration tests remain in GitHub CI.
