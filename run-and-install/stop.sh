#!/usr/bin/env bash
source "$(dirname -- "${BASH_SOURCE[0]}")/common.sh"
require_docker
require_env
compose --profile monitoring stop api prometheus grafana rabbitmq redis
echo 'API, monitoring, RabbitMQ and Redis stopped. Data volumes are preserved.'
