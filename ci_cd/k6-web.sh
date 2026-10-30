#!/usr/bin/env bash
# Jenkins manages the DEV web runner with Docker, independently of the API image.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
action=${1:-status}
container=driver-app-k6-web
image=driver-app-k6-web:local
unit=driver-app-k6-web.service
stop_legacy() {
  if command -v systemctl >/dev/null && systemctl cat "$unit" >/dev/null 2>&1; then
    sudo -n systemctl disable --now "$unit"
    sudo -n rm -f "/etc/systemd/system/$unit"
    sudo -n systemctl daemon-reload
  fi
}
stop_container() {
  if docker container inspect "$container" >/dev/null 2>&1; then
    docker stop --time 400 "$container" >/dev/null
    docker rm "$container" >/dev/null
  fi
}
case "$action" in
  start)
    host=${2:?Public hostname or IP required}
    [[ "$host" =~ ^[A-Za-z0-9.-]+$ ]] || { echo 'Invalid public host' >&2; exit 1; }
    [[ -s .k6-web/access-token ]] || { echo 'Missing Jenkins-provided runner access token' >&2; exit 1; }
    chmod 700 .k6-web
    chmod 600 .k6-web/access-token
    project_dir=$(pwd -P)
    [[ "$project_dir" =~ ^/[A-Za-z0-9_./-]+$ ]] || exit 1
    mkdir -p .k6-web/results
    chmod 700 .k6-web/results
    docker info >/dev/null
    docker build -t "$image" run-and-install/load-test/web
    docker pull grafana/k6:latest
    # Sibling k6 containers use the host daemon. All bind paths, including result
    # directories, must exist at exactly the same absolute path on both sides.
    mounts=(--mount "type=bind,src=$project_dir,dst=$project_dir,readonly"
            --mount "type=bind,src=$project_dir/.k6-web/results,dst=$project_dir/.k6-web/results")
    identity=(--user "$(id -u):$(id -g)")
    command=("$project_dir/run-and-install/load-test/web/server.py"
             --bind 0.0.0.0 --public-host "$host"
             --access-token-file "$project_dir/.k6-web/access-token")
    docker run --rm "${identity[@]}" "${mounts[@]}" "$image" "${command[@]}" --check-config
    docker run --rm "${identity[@]}" --group-add "$(stat -c '%g' /var/run/docker.sock)" \
      --mount type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock \
      --entrypoint docker "$image" version --format '{{.Server.Version}}'
    stop_legacy
    stop_container
    docker run -d --name "$container" --restart unless-stopped \
      --publish 5665:5665 --stop-signal SIGINT \
      "${identity[@]}" --group-add "$(stat -c '%g' /var/run/docker.sock)" \
      "${mounts[@]}" --mount type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock \
      --env "TMPDIR=$project_dir/.k6-web/results" \
      --env "K6_WEB_TOKEN_FILE=$project_dir/.k6-web/access-token" \
      "$image" "${command[@]}" >/dev/null
    for ((i=0;i<30;i++)); do
      state=$(docker inspect --format '{{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{end}}' "$container")
      if [[ "$state" == 'running healthy' ]]; then
        echo "k6 web runner ready: http://$host:5665 (username: runner; password: Jenkins k6-web-access-token credential)"
        exit 0
      fi
      case "$state" in exited*|dead*|*unhealthy) break ;; esac
      sleep 2
    done
    echo 'k6 web runner failed its authenticated HTTP health check' >&2
    docker logs --tail 60 "$container" >&2 || true
    exit 1
    ;;
  stop) stop_legacy; stop_container ;;
  status) docker inspect --format '{{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{end}}' "$container" ;;
  *) echo 'Usage: k6-web.sh start HOST | stop | status' >&2; exit 2 ;;
esac
