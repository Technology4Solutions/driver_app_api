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
  fi
}
trap on_exit EXIT
export MONITORING_DIR="$PWD/monitoring"
export MONITORING_NODE_ID="${MONITORING_NODE_ID:-$(docker info --format '{{.Swarm.NodeID}}')}"
export MONITORING_ENV_FILE=/dev/null
if [[ -f monitoring/slack.local.env ]]; then
  export MONITORING_ENV_FILE="$MONITORING_DIR/slack.local.env"
fi
case "$action" in
  check)
    # Older Docker CLIs support stack deploy but not stack config.
    # Match the existing device_api workflow: deploy parses the YAML itself.
    docker stack deploy --help >/dev/null
    for file in monitoring/stack.yml monitoring/prometheus.swarm.yml monitoring/alerts.yml \
                monitoring/grafana/datasources/prometheus.yml \
                monitoring/grafana/dashboards/provider.yml \
                monitoring/grafana/dashboards/device-api.json; do
      [[ -r "$file" ]] || { echo "Missing monitoring file: $file" >&2; exit 1; }
    done
    ;;
  start)
    sed "s/tasks.driver_app_api_stack_driver-app-api/tasks.${stack}_driver-app-api/" \
      monitoring/prometheus.swarm.yml > monitoring/prometheus.generated.yml
    docker stack deploy --with-registry-auth --resolve-image always -c monitoring/stack.yml "$monitoring_stack"
    # Bind-mounted provisioning files are re-read at process startup.
    for component in prometheus grafana; do
      docker service update --detach=true --force "${monitoring_stack}_${component}"
    done
    for ((i=0;i<60;i++)); do
      ready=true
      for component in prometheus grafana; do
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
      if $ready; then
        docker stack services "$monitoring_stack"
        echo 'Monitoring ready. Services are private; use an SSH tunnel to the Grafana container IP.'
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
