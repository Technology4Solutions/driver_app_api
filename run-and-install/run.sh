#!/usr/bin/env bash
# Start/recreate the API and monitoring using the current .env.
source "$(dirname -- "${BASH_SOURCE[0]}")/common.sh"
require_docker
require_env
compose --profile monitoring up --no-build -d --wait --wait-timeout 60 api prometheus grafana
if [[ -f "$PROJECT_ROOT/monitoring/grafana/alerting/device-api.local.json" ]]; then
  # File provisioning is applied at Grafana startup, including threshold changes.
  compose --profile monitoring restart grafana
fi
compose --profile monitoring ps
echo "API started. Run $SCRIPT_DIR/test-api.sh to check HTTP readiness."
echo "Logs: $SCRIPT_DIR/logs.sh"
echo 'Grafana: http://localhost:3000 (initial login admin/admin; open Dashboards > Device API)'
echo 'Prometheus targets: http://localhost:9091/targets'
