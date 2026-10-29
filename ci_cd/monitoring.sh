#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
action=${1:-start}
stack=${2:-driver_app_api_stack}
[[ "$stack" =~ ^[a-zA-Z0-9][a-zA-Z0-9_-]*$ ]] || exit 1
compose() { docker compose -f monitoring/compose.swarm.yaml "$@"; }
case "$action" in
  check)
    if ! docker compose version >/dev/null 2>&1; then
      echo 'Monitoring requires Docker Compose 2.24+ on the deployment server. Install the Docker Compose plugin there, then retry deployment.' >&2
      exit 1
    fi
    compose config --quiet
    ;;
  start)
    # Use task discovery, rather than the service VIP, to scrape every replica.
    sed "s/tasks.driver_app_api_stack_driver-app-api/tasks.${stack}_driver-app-api/" \
      monitoring/prometheus.swarm.yml > monitoring/prometheus.generated.yml
    export PROMETHEUS_CONFIG=./prometheus.generated.yml
    compose up -d --pull always --force-recreate --wait --wait-timeout 120
    compose ps
    echo 'Monitoring ready: Grafana on manager localhost:3000; Prometheus on localhost:9091 (use an SSH tunnel).'
    ;;
  status) compose ps ;;
  stop) compose down ;;
  *) echo 'monitoring.sh check|start|status|stop [STACK]' >&2; exit 2 ;;
esac
