#!/usr/bin/env bash
source "$(dirname -- "${BASH_SOURCE[0]}")/common.sh"
require_docker
require_env
compose logs --tail=100 -f api
