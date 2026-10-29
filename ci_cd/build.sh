#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
bump=patch; platform=linux/amd64; push=true; save=true
while (($#)); do
  case "$1" in
    -t) bump=${2:?}; shift 2 ;;
    --platform) platform=${2:?}; shift 2 ;;
    --dry-run) push=false; save=false; shift ;;
    --no-push) push=false; shift ;;
    --versions) cat VERSION; exit 0 ;;
    -h|--help) echo 'build.sh [-t patch|minor|major] [--platform linux/amd64] [--dry-run] [--no-push] [--versions]'; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; exit 1 ;;
  esac
done
current=$(tr -d '[:space:]' < VERSION)
[[ "$current" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || { echo 'Invalid VERSION' >&2; exit 1; }
IFS=. read -r major minor patch <<< "$current"
case "$bump" in
  major) major=$((major+1)); minor=0; patch=0 ;;
  minor) minor=$((minor+1)); patch=0 ;;
  patch) patch=$((patch+1)) ;;
  *) echo 'Invalid version increment' >&2; exit 1 ;;
esac
version="$major.$minor.$patch"
# Monitoring configuration is validated on the target manager by deploy.sh.
# Image builds require only the Docker CLI, not the Compose plugin.
for component in api rabbitmq; do
  image=reaxiumapps/driver_app_api; file=ci_cd/Dockerfile
  if [[ "$component" == rabbitmq ]]; then image+=-rabbitmq; file+=.rabbitmq; fi
  docker build --platform "$platform" -f "$file" -t "$image:$version" .
done
if $push; then
  docker push "reaxiumapps/driver_app_api:$version"
  docker push "reaxiumapps/driver_app_api-rabbitmq:$version"
fi
if $save; then printf '%s\n' "$version" > VERSION; fi
echo "Built version $version"
