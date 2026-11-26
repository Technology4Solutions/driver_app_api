#!/usr/bin/env python3
"""Inject Vault settings into a local command without writing secret files."""
import importlib.util
import json
import os
from pathlib import Path
import sys

spec = importlib.util.spec_from_file_location('vault_entrypoint', Path(__file__).with_name('vault-entrypoint.py'))
vault = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vault)


def main():
    if len(sys.argv) < 2:
        raise vault.ConfigurationError('Usage: vault-exec.py COMMAND [ARG ...]')
    client = os.environ.get('CLIENT', '')
    environment = os.environ.get('ENVIRONMENT', '')
    if environment not in ('DEV', 'PROD') or not client or not all(c.isalnum() or c == '_' for c in client):
        raise vault.ConfigurationError('Set CLIENT and ENVIRONMENT (DEV or PROD)')
    token_file = Path(os.environ.get('VAULT_TOKEN_FILE', '/run/secrets/VAULT_TOKEN'))
    token = token_file.read_text().strip()
    if not token:
        raise vault.ConfigurationError('Vault token file is empty')
    addr = os.environ.get('VAULT_ADDR', 'https://vault.reaxium.com').rstrip('/')
    if not addr.startswith('https://'):
        raise vault.ConfigurationError('Local Vault access requires HTTPS')
    mount = os.environ.get('VAULT_MOUNT', 'kv').strip('/')
    base = os.environ.get('VAULT_BASE_PATH', 'reaxium_ci/device_api').strip('/')
    version = vault.kv_version(addr, mount, token, os.environ.get('VAULT_KV_VERSION', 'auto'))
    path = f'{mount}/' + ('data/' if version == '2' else '') + f'{base}/{client}_{environment}'
    request = vault.Request(f'{addr}/v1/{path}', headers={'X-Vault-Token': token})
    with vault.urlopen(request, timeout=30) as response:
        payload = json.load(response)
    bundle = payload['data']['data'] if version == '2' else payload['data']
    # Never allow stale ambient credentials to satisfy missing Vault settings.
    environ = {k: v for k, v in os.environ.items()
               if not k.startswith(('DB_', 'SCHOOL_BUS_DATASOURCE_', 'REAXIUM_API_'))}
    settings = vault.configuration(environ, bundle, 'api')
    settings.pop('VAULT_TOKEN', None)
    os.execvpe(sys.argv[1], sys.argv[1:], settings)


if __name__ == '__main__':
    try:
        main()
    except vault.ConfigurationError as error:
        print(f'Vault configuration error: {error}', file=sys.stderr)
        sys.exit(1)
    except Exception as error:
        print(f'Vault command failed ({type(error).__name__}); check configuration and Vault access.', file=sys.stderr)
        sys.exit(1)
