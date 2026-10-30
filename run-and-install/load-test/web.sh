#!/usr/bin/env bash
# One-command local Docker runner. DEV is managed separately by Jenkins.
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
project_dir="$(cd -- "$script_dir/../.." && pwd -P)"
port=5665
arguments=()
extra_mounts=()
while (($#)); do
  case "$1" in
    --port)
      port=${2:?Port required}
      [[ "$port" =~ ^[0-9]+$ ]] && ((port >= 1024 && port <= 65535)) || { echo 'Port must be 1024-65535' >&2; exit 2; }
      shift 2 ;;
    --api-url) arguments+=("$1" "${2:?API URL required}"); shift 2 ;;
    --dataset|--env-file)
      path=$(realpath -- "${2:?File path required}")
      [[ -f "$path" ]] || { echo "File not found: $path" >&2; exit 2; }
      extra_mounts+=(--mount "type=bind,src=$path,dst=$path,readonly")
      arguments+=("$1" "$path"); shift 2 ;;
    -h|--help)
      echo 'Usage: web.sh [--api-url URL] [--port 5665] [--dataset FILE] [--env-file FILE]'
      echo 'Builds and runs the local Docker page. Open http://127.0.0.1:5665; login password is printed at startup.'
      exit 0 ;;
    *) echo "Unknown option: $1 (use --help). DEV deployment is managed by Jenkins." >&2; exit 2 ;;
  esac
done
command -v docker >/dev/null || { echo 'Docker is required' >&2; exit 1; }
docker info >/dev/null
[[ -S /var/run/docker.sock ]] || { echo 'A local Docker socket at /var/run/docker.sock is required' >&2; exit 1; }
mkdir -p "$project_dir/.k6-web/local-results"
chmod 700 "$project_dir/.k6-web/local-results"
docker build -t driver-app-k6-web:local "$script_dir/web"
echo "Open http://127.0.0.1:$port. Login: runner; password will be printed below. Ctrl+C stops the container."
exec docker run --rm --name "driver-app-k6-web-local-$port" \
  --publish "127.0.0.1:$port:$port" --stop-signal SIGINT --no-healthcheck \
  --user "$(id -u):$(id -g)" --group-add "$(stat -c '%g' /var/run/docker.sock)" \
  --mount "type=bind,src=$project_dir,dst=$project_dir,readonly" \
  --mount "type=bind,src=$project_dir/.k6-web/local-results,dst=$project_dir/.k6-web/local-results" \
  --mount type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock \
  "${extra_mounts[@]}" --env "TMPDIR=$project_dir/.k6-web/local-results" \
  driver-app-k6-web:local "$script_dir/web/server.py" \
  "${arguments[@]}" --port "$port" --bind 0.0.0.0 --public-host 127.0.0.1
