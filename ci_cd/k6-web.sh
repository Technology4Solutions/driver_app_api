#!/usr/bin/env bash
# Jenkins invokes this on the deployment host after a successful DEV deployment.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
action=${1:-status}
unit=driver-app-k6-web.service
case "$action" in
  start)
    host=${2:?Public hostname or IP required}
    [[ "$host" =~ ^[A-Za-z0-9.-]+$ ]] || { echo 'Invalid public host' >&2; exit 1; }
    [[ -s .k6-web/access-token ]] || { echo 'Missing Jenkins-provided runner access token' >&2; exit 1; }
    chmod 700 .k6-web
    chmod 600 .k6-web/access-token
    python3 -m py_compile run-and-install/load-test/web/server.py
    docker info >/dev/null
    docker pull grafana/k6:latest
    project_dir=$(pwd -P)
    # systemd paths must not contain unit-file expansion characters.
    [[ "$project_dir" =~ ^/[A-Za-z0-9_./-]+$ ]] || exit 1
    runner_user=$(id -un)
    runner_group=$(id -gn)
    unit_file=$(mktemp)
    trap 'rm -f "$unit_file"' EXIT
    cat > "$unit_file" <<EOF
[Unit]
Description=Driver API DEV k6 test control page
After=network.target docker.service
Requires=docker.service

[Service]
Type=simple
User=$runner_user
Group=$runner_group
WorkingDirectory=$project_dir
ExecStart=/usr/bin/python3 $project_dir/run-and-install/load-test/web/server.py --bind 0.0.0.0 --public-host $host --access-token-file $project_dir/.k6-web/access-token
Restart=on-failure
RestartSec=5
KillSignal=SIGINT
TimeoutStopSec=400
UMask=0077

[Install]
WantedBy=multi-user.target
EOF
    sudo -n install -m 644 "$unit_file" "/etc/systemd/system/$unit"
    sudo -n systemctl daemon-reload
    sudo -n systemctl enable "$unit"
    sudo -n systemctl restart "$unit"
    for ((i=0;i<20;i++)); do
      if python3 - <<'PY'
import base64
from pathlib import Path
import urllib.request
request = urllib.request.Request('http://127.0.0.1:5665/api/config')
secret = Path('.k6-web/access-token').read_text().strip()
request.add_header('Authorization', 'Basic ' + base64.b64encode(('runner:' + secret).encode()).decode())
try:
    with urllib.request.urlopen(request, timeout=2) as response:
        assert response.status == 200
except Exception:
    raise SystemExit(1)
PY
      then
        echo "k6 web runner ready: http://$host:5665 (username: runner; password: Jenkins k6-web-access-token credential)"
        exit 0
      fi
      sleep 1
    done
    echo 'k6 web runner failed its authenticated HTTP health check' >&2
    exit 1
    ;;
  stop) sudo -n systemctl disable --now "$unit" ;;
  status) systemctl is-active "$unit" ;;
  *) echo 'Usage: k6-web.sh start HOST | stop | status' >&2; exit 2 ;;
esac
