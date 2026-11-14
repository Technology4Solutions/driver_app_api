#!/usr/bin/env bash
# Refresh the local load-test dataset without changing database records.
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
if [[ "${1:-}" == --help || "${1:-}" == -h ]]; then
  echo "Usage: $0 [YYYY-MM-DD] [export-devices.py options]"
  echo 'Defaults to today in Africa/Casablanca. Use the date selected in the test page.'
  echo 'Optional exporter flags: --env-file PATH, --output PATH, --limit N.'
  exit 0
fi
command -v python3 >/dev/null || { echo 'Python 3 is required' >&2; exit 1; }
arguments=()
if [[ $# -gt 0 && "$1" != -* ]]; then
  arguments+=(--passenger-date "$1")
  shift
fi
exec python3 "$script_dir/export-devices.py" "${arguments[@]}" "$@"
