#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
client=${CLIENT:-}; environment=${ENVIRONMENT:-}; version=; mode=all; force=false; action=deploy
stack=driver_app_api_stack
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
if [[ "$action" == status ]]; then docker stack services "$stack"; exit; fi
if [[ "$action" == remove ]]; then
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
if [[ $(docker info --format '{{.Swarm.LocalNodeState}}') == inactive ]]; then docker swarm init; fi
[[ $(docker info --format '{{.Swarm.ControlAvailable}}') == true ]] || { echo 'A Swarm manager is required' >&2; exit 1; }
docker secret inspect VAULT_TOKEN >/dev/null
if ! docker network inspect my_reaxium_api_network >/dev/null 2>&1; then
  docker network create --driver overlay --attachable my_reaxium_api_network
fi
image="reaxiumapps/driver_app_api:$version"
service="${stack}_driver-app-api"
docker pull "$image"
force_args=(); if $force; then force_args=(--force); fi
if [[ "$mode" == app ]]; then
  docker service inspect "$service" >/dev/null
  docker service update --detach=true --with-registry-auth "${force_args[@]}" --image "$image" \
    --env-add "CLIENT=$client" --env-add "ENVIRONMENT=$environment" "$service"
else
  docker pull "reaxiumapps/driver_app_api-rabbitmq:$version"
  docker stack deploy --with-registry-auth --resolve-image always -c ci_cd/docker-compose.swarm.yml "$stack"
  if $force; then docker service update --detach=true --force "$service"; fi
fi
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
    echo "Deployment ready: $stack version $version"; exit 0
  fi
  sleep 5
done
echo 'Deployment did not converge within five minutes' >&2
exit 1
