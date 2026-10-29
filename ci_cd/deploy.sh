#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
client=${CLIENT:-}; environment=${ENVIRONMENT:-}; version=; mode=all; force=false; action=deploy
stack=driver_app_api_stack
stage='validating deployment arguments'
diagnostics=false
deployment_exit() {
  local status=$?
  if ((status != 0)); then
    echo "Deployment failed while $stage (exit $status)." >&2
    if $diagnostics; then
      docker stack services "$stack" >&2 || true
      docker stack ps --no-trunc "$stack" >&2 || true
    fi
  fi
}
trap deployment_exit EXIT
while (($#)); do
  case "$1" in
    --client) client=${2:?}; shift 2 ;;
    --env) environment=${2:?}; shift 2 ;;
    --version) version=${2:?}; shift 2 ;;
    --stack) stack=${2:?}; shift 2 ;;
    --all) mode=all; shift ;;
    --app) mode=app; shift ;;
    --force) force=true; shift ;;
    --remove) action=remove; shift ;;
    --status) action=status; shift ;;
    -h|--help) echo 'deploy.sh --client CLIENT --env DEV|PROD [--version TAG] [--all|--app] [--force] [--stack NAME] [--remove|--status]'; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; exit 1 ;;
  esac
done
[[ "$stack" =~ ^[a-zA-Z0-9][a-zA-Z0-9_-]*$ ]] || exit 1
if [[ "$action" == status ]]; then
  docker stack services "$stack"
  bash ci_cd/monitoring.sh status "$stack"
  exit
fi
if [[ "$action" == remove ]]; then
  stage='stopping monitoring (preserving volumes)'
  bash ci_cd/monitoring.sh stop "$stack"
  docker stack rm "$stack"
  for ((i=0;i<60;i++)); do
    [[ -z $(docker service ls --filter "label=com.docker.stack.namespace=$stack" -q) ]] && exit 0
    sleep 2
  done
  echo 'Stack removal timed out' >&2; exit 1
fi
[[ "$client" =~ ^[A-Za-z0-9_]+$ && ( "$environment" == DEV || "$environment" == PROD ) ]] || { echo 'Valid --client and --env DEV|PROD are required' >&2; exit 1; }
version=${version:-$(tr -d '[:space:]' < VERSION)}
[[ "$version" =~ ^[a-zA-Z0-9_][a-zA-Z0-9_.-]{0,127}$ ]] || exit 1
export CLIENT="$client" ENVIRONMENT="$environment" API_VERSION="$version"
export VAULT_BASE_PATH="${VAULT_BASE_PATH:-reaxium_ci/device_api}"
stage='checking monitoring files and Swarm deployment support'
bash ci_cd/monitoring.sh check "$stack"
stage='checking the Docker Swarm manager'
if [[ $(docker info --format '{{.Swarm.LocalNodeState}}') == inactive ]]; then docker swarm init; fi
[[ $(docker info --format '{{.Swarm.ControlAvailable}}') == true ]] || { echo 'A Swarm manager is required' >&2; exit 1; }
stage='checking the required VAULT_TOKEN Swarm secret'
docker secret inspect VAULT_TOKEN >/dev/null
stage='checking the application overlay network'
if ! docker network inspect my_reaxium_api_network >/dev/null 2>&1; then
  docker network create --driver overlay --attachable my_reaxium_api_network
fi
image="reaxiumapps/driver_app_api:$version"
service="${stack}_driver-app-api"
stage='checking existing stack services'
existing_services=$(docker service ls --filter "label=com.docker.stack.namespace=$stack" --format '{{.Name}}')
if [[ "$mode" == app ]] && [[ -z "$existing_services" ]]; then
  echo "Stack $stack does not exist yet; creating the full stack for this first deployment."
  mode=all
fi
if [[ "$mode" == app ]]; then
  stage="checking existing API service $service (use FULL_STACK to repair a partial stack)"
  docker service inspect "$service" >/dev/null
fi
stage="pulling API image $image"
docker pull "$image"
force_args=(); if $force; then force_args=(--force); fi
if [[ "$mode" == app ]]; then
  stage="updating API service $service"
  docker service update --detach=true --with-registry-auth "${force_args[@]}" --image "$image" \
    --env-add "CLIENT=$client" --env-add "ENVIRONMENT=$environment" \
    --env-add "VAULT_BASE_PATH=$VAULT_BASE_PATH" "$service"
else
  stage="pulling RabbitMQ image reaxiumapps/driver_app_api-rabbitmq:$version"
  docker pull "reaxiumapps/driver_app_api-rabbitmq:$version"
  stage="deploying stack $stack"
  docker stack deploy --with-registry-auth --resolve-image always -c ci_cd/docker-compose.swarm.yml "$stack"
  if $force; then docker service update --detach=true --force "$service"; fi
fi
stage="waiting for stack $stack to converge"
diagnostics=true
for ((i=0;i<60;i++)); do
  ready=true; count=0
  while read -r name replicas; do
    [[ -n "$name" ]] || continue
    count=$((count+1))
    state=$(docker service inspect --format '{{if .UpdateStatus}}{{.UpdateStatus.State}}{{end}}' "$name")
    case "$state" in rollback*|paused) echo "Deployment failed: $name ($state)" >&2; exit 1 ;; esac
    [[ -z "$state" || "$state" == completed ]] || ready=false
    [[ "$replicas" =~ ^([0-9]+)/([0-9]+)$ ]] || { ready=false; continue; }
    [[ "${BASH_REMATCH[1]}" == "${BASH_REMATCH[2]}" && "${BASH_REMATCH[2]}" != 0 ]] || ready=false
  done < <(docker stack services "$stack" --format '{{.Name}} {{.Replicas}}')
  actual=$(docker service inspect --format '{{.Spec.TaskTemplate.ContainerSpec.Image}}' "$service")
  if $ready && ((count >= 3)) && [[ "$actual" == "$image" || "$actual" == "$image@sha256:"* ]]; then
    stage='starting Prometheus and Grafana'
    bash ci_cd/monitoring.sh start "$stack"
    echo "Deployment ready: $stack version $version"; exit 0
  fi
  sleep 5
done
echo 'Deployment did not converge within five minutes' >&2
exit 1
