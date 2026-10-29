#!/usr/bin/env python3
"""Load a Vault KV bundle using the Swarm VAULT_TOKEN, then exec the service."""
import json
import os
from pathlib import Path
import sys
from urllib.parse import quote, urlsplit
from urllib.request import Request, urlopen
from urllib.error import HTTPError, URLError


def kv_version(addr, mount, token, configured='auto'):
    if configured in ('1', '2'):
        return configured
    if configured != 'auto':
        raise ValueError('VAULT_KV_VERSION must be auto, 1 or 2')
    request = Request(f'{addr}/v1/sys/mounts/{mount}/tune', headers={'X-Vault-Token': token})
    try:
        with urlopen(request, timeout=30) as response:
            metadata = json.load(response)
    except (URLError, ValueError):
        # Match the Java entrypoint when mount metadata is inaccessible.
        print('Vault KV detection unavailable; using KV v1 as in the Java entrypoint. Set VAULT_KV_VERSION=2 explicitly for a known v2 mount.', file=sys.stderr)
        return '1'
    # Vault normally wraps mount metadata in data; also accept the Java shape.
    options = metadata.get('data', metadata).get('options') or {}
    return '2' if str(options.get('version', '1')) == '2' else '1'


def configuration(environ, bundle, mode):
    result = dict(environ)
    # Export only application configuration, never arbitrary executable/shell settings.
    allowed = ('DB_', 'REAXIUM_API_', 'DEVICE_STATUS_', 'VISION_')
    exact = {'BROADCAST_ENABLED', 'DASHBOARD_BROADCAST_ENABLED', 'DEVICES_API_EXPORT_DEVICE_STATUS_URL',
             'RABBITMQ_URL', 'REDIS_URL', 'RUST_LOG'}
    for key, value in bundle.items():
        if value is not None and (key.startswith(allowed) or key in exact):
            result[key] = str(value).lower() if isinstance(value, bool) else str(value)
    def require(key):
        value = bundle.get(key)
        if not value or not isinstance(value, str):
            raise ValueError(f'Missing Vault key: {key}')
        return value
    if mode == 'rabbitmq':
        result['RABBITMQ_DEFAULT_USER'] = require('RABBITMQ_USER')
        result['RABBITMQ_DEFAULT_PASS'] = require('RABBITMQ_PASSWORD')
        return result
    # Accept the Java bundle's school-bus keys as well as native Rust DB_* keys.
    if not result.get('DB_HOST'):
        url = urlsplit(require('SCHOOL_BUS_DATASOURCE_URL').removeprefix('jdbc:'))
        if url.scheme != 'mysql' or not url.hostname or not url.path.strip('/'):
            raise ValueError('SCHOOL_BUS_DATASOURCE_URL must identify a MySQL database')
        result.update(DB_HOST=url.hostname, DB_PORT=str(url.port or 3306), DB_NAME=url.path.lstrip('/'))
    for native, java in [('DB_USER', 'SCHOOL_BUS_DATASOURCE_USERNAME'), ('DB_PASSWORD', 'SCHOOL_BUS_DATASOURCE_PASSWORD')]:
        if not result.get(native):
            result[native] = require(java)
    for key in ['DB_HOST', 'DB_NAME', 'DB_USER', 'DB_PASSWORD', 'REAXIUM_API_USER', 'REAXIUM_API_PASSWORD']:
        if not result.get(key):
            raise ValueError(f'Missing application setting: {key}')
    if not result.get('RABBITMQ_URL'):
        user = quote(require('RABBITMQ_USER'), safe='')
        password = quote(require('RABBITMQ_PASSWORD'), safe='')
        result['RABBITMQ_URL'] = f'amqp://{user}:{password}@driver-app-rabbitmq:5672/%2f'
    result.setdefault('REDIS_URL', 'redis://driver-app-redis:6379/')
    result['APP_ENV'] = environ['ENVIRONMENT'].lower()
    result.setdefault('BROADCAST_ENABLED', 'true')
    if result['BROADCAST_ENABLED'] == 'true':
        for key in ['DEVICES_API_EXPORT_DEVICE_STATUS_URL', 'DEVICE_STATUS_BROADCAST_USERNAME', 'DEVICE_STATUS_BROADCAST_PASSWORD']:
            if not result.get(key):
                raise ValueError(f'Missing enabled broadcast setting: {key}')
    return result


def main():
    mode = sys.argv[1]
    if mode not in ('api', 'rabbitmq'):
        raise ValueError('Unsupported entrypoint mode')
    token = Path('/run/secrets/VAULT_TOKEN').read_text().strip()
    if not token:
        raise ValueError('VAULT_TOKEN is empty')
    client, environment = os.environ['CLIENT'], os.environ['ENVIRONMENT']
    if environment not in ('DEV', 'PROD') or not client or not all(c.isalnum() or c == '_' for c in client):
        raise ValueError('Invalid CLIENT or ENVIRONMENT')
    addr = os.getenv('VAULT_ADDR', 'https://vault.reaxium.com').rstrip('/')
    mount = os.getenv('VAULT_MOUNT', 'kv').strip('/')
    base = os.getenv('VAULT_BASE_PATH', 'reaxium_ci/device_api').strip('/')
    version = kv_version(addr, mount, token, os.getenv('VAULT_KV_VERSION', 'auto'))
    print(f'Vault KV engine: v{version}', flush=True)
    path = f'{mount}/' + ('data/' if version == '2' else '') + f'{base}/{client}_{environment}'
    request = Request(f'{addr}/v1/{path}', headers={'X-Vault-Token': token})
    with urlopen(request, timeout=30) as response:
        payload = json.load(response)
    bundle = payload['data']['data'] if version == '2' else payload['data']
    settings = configuration(os.environ, bundle, mode)
    command = ['/usr/local/bin/device-api'] if mode == 'api' else ['docker-entrypoint.sh', 'rabbitmq-server']
    os.execvpe(command[0], command, settings)


if __name__ == '__main__':
    try:
        main()
    except HTTPError as error:
        # Do not print the URL, response body, headers or token.
        print(f'Vault startup failed (HTTP {error.code}); check Vault path, KV version and token permissions.', file=sys.stderr)
        sys.exit(1)
    except Exception as error:
        # Never print Vault responses, credentials, or URLs containing secrets.
        print(f'Vault startup failed ({type(error).__name__}); check configuration and Vault access.', file=sys.stderr)
        sys.exit(1)
