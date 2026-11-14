#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
action=${1:-start}
stack=${2:-driver_app_api_stack}
[[ "$stack" =~ ^[a-zA-Z0-9][a-zA-Z0-9_-]*$ ]] || exit 1
monitoring_stack="${stack}_monitoring"
on_exit() {
  local status=$?
  if ((status != 0)) && [[ "$action" == start ]]; then
    docker stack ps --no-trunc "$monitoring_stack" >&2 || true
    for component in prometheus loki alloy grafana; do
      docker service logs --timestamps --since 5m --tail 80 "${monitoring_stack}_${component}" >&2 || true
    done
  fi
}
trap on_exit EXIT
export MONITORING_DIR="$PWD/monitoring"
export API_LOG_SERVICE="${stack}_driver-app-api"
export MONITORING_NODE_ID="${MONITORING_NODE_ID:-$(docker info --format '{{.Swarm.NodeID}}')}"
export MONITORING_ENV_FILE=/dev/null
if [[ -f monitoring/slack.local.env ]]; then
  export MONITORING_ENV_FILE="$MONITORING_DIR/slack.local.env"
fi
stack_files=(-c monitoring/stack.yml)
if [[ "${ENVIRONMENT:-}" == DEV ]]; then
  [[ -r monitoring/stack.dev.yml ]] || { echo 'Missing monitoring file: monitoring/stack.dev.yml' >&2; exit 1; }
  stack_files+=(-c monitoring/stack.dev.yml)
fi
case "$action" in
  check)
    # Older Docker CLIs support stack deploy but not stack config.
    # Match the existing device_api workflow: deploy parses the YAML itself.
    docker stack deploy --help >/dev/null
    for file in monitoring/stack.yml monitoring/prometheus.swarm.yml monitoring/alerts.yml monitoring/loki.yml monitoring/alloy.alloy \
                monitoring/grafana/datasources/loki.yml monitoring/grafana/alerting/api-log-errors.yml \
                monitoring/grafana/datasources/prometheus.yml \
                monitoring/grafana/dashboards/provider.yml \
                monitoring/grafana/dashboards/device-api.json; do
      [[ -r "$file" ]] || { echo "Missing monitoring file: $file" >&2; exit 1; }
    done
    ;;
  start)
    # Swarm configs are immutable: version the distributed collector config.
    export ALLOY_CONFIG_HASH="$(sha256sum monitoring/alloy.alloy | cut -c1-12)"
    # Drain the existing singleton before stack deploy or a forced restart can
    # create another writer. This also recovers an update paused on a TSDB lock.
    prometheus_service="${monitoring_stack}_prometheus"
    if docker service inspect "$prometheus_service" >/dev/null 2>&1; then
      docker service scale --detach=true "$prometheus_service=0"
      drained=false
      for ((attempt=0;attempt<60;attempt++)); do
        containers=$(docker ps -q --filter "label=com.docker.swarm.service.name=$prometheus_service")
        if [[ -z "$containers" ]]; then drained=true; break; fi
        sleep 2
      done
      $drained || { echo 'Prometheus did not stop; refusing to start another TSDB writer' >&2; exit 1; }
    fi
    owners=$(docker ps -q --filter volume=driver-api-monitoring_prometheus-data)
    if [[ -n "$owners" ]]; then
      echo "Prometheus data volume is still used by container(s): $owners. Stop the competing monitoring instance before retrying; keep the volume." >&2
      exit 1
    fi
    sed "s/tasks.driver_app_api_stack_driver-app-api/tasks.${stack}_driver-app-api/" \
      monitoring/prometheus.swarm.yml > monitoring/prometheus.generated.yml
    docker stack deploy --with-registry-auth --resolve-image always "${stack_files[@]}" "$monitoring_stack"
    # Bind-mounted provisioning files are re-read at process startup.
    for component in prometheus grafana loki alloy; do
      docker service update --detach=true --update-order stop-first --rollback-order stop-first --force "${monitoring_stack}_${component}"
    done
    for ((i=0;i<60;i++)); do
      ready=true
      for component in prometheus grafana loki; do
        service="${monitoring_stack}_${component}"
        state=$(docker service inspect --format '{{if .UpdateStatus}}{{.UpdateStatus.State}}{{end}}' "$service")
        case "$state" in paused|rollback*) echo "Monitoring update failed: $service ($state)" >&2; exit 1 ;; esac
        [[ -z "$state" || "$state" == completed ]] || ready=false
        container=$(docker ps -q --filter "label=com.docker.swarm.service.name=$service")
        if [[ -z "$container" || "$container" == *$'\n'* ]]; then
          ready=false
        elif [[ $(docker inspect --format '{{.State.Health.Status}}' "$container") != healthy ]]; then
          ready=false
        fi
      done
      # Alloy is global, so verify health on every node through its task status.
      alloy_service="${monitoring_stack}_alloy"
      alloy_state=$(docker service inspect --format '{{if .UpdateStatus}}{{.UpdateStatus.State}}{{end}}' "$alloy_service")
      case "$alloy_state" in paused|rollback*) echo "Monitoring update failed: $alloy_service ($alloy_state)" >&2; exit 1 ;; esac
      [[ -z "$alloy_state" || "$alloy_state" == completed ]] || ready=false
      alloy_counts=$(docker service inspect --format '{{.ServiceStatus.RunningTasks}}/{{.ServiceStatus.DesiredTasks}}' --status "$alloy_service")
      if [[ "$alloy_counts" =~ ^([0-9]+)/([0-9]+)$ ]]; then
        [[ "${BASH_REMATCH[1]}" == "${BASH_REMATCH[2]}" && "${BASH_REMATCH[2]}" != 0 ]] || ready=false
      else
        ready=false
      fi
      if $ready; then
        docker stack services "$monitoring_stack"
        if [[ "${ENVIRONMENT:-}" == DEV ]]; then
          echo 'Monitoring ready. Grafana: http://DEV_SERVER_IP:3000 (allow TCP 3000 from your IP in the server firewall/security group).'
        else
          echo 'Monitoring ready. Services are private; use an SSH tunnel to the Grafana container IP.'
        fi
        exit 0
      fi
      sleep 5
    done
    echo 'Monitoring did not become healthy within five minutes' >&2
    exit 1
    ;;
  status) docker stack services "$monitoring_stack" ;;
  stop)
    docker stack rm "$monitoring_stack"
    for ((i=0;i<60;i++)); do
      if [[ -z $(docker service ls -q --filter "label=com.docker.stack.namespace=$monitoring_stack") && -z $(docker network ls -q --filter "label=com.docker.stack.namespace=$monitoring_stack") ]]; then exit 0; fi
      sleep 2
    done
    echo 'Monitoring removal timed out' >&2; exit 1
    ;;
  *) echo 'monitoring.sh check|start|status|stop [STACK]' >&2; exit 2 ;;
esac
