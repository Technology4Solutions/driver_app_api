#!/usr/bin/env python3
"""Compare deployed Vault settings and probe API access from the local broker."""
import json
import subprocess

STACK = 'driver_app_api_stack'
KEYS = ('VAULT_ADDR', 'VAULT_MOUNT', 'VAULT_BASE_PATH', 'VAULT_KV_VERSION', 'CLIENT', 'ENVIRONMENT')
DEFAULTS = dict(VAULT_ADDR='https://vault.reaxium.com', VAULT_MOUNT='kv',
                VAULT_BASE_PATH='reaxium_ci/device_api', VAULT_KV_VERSION='auto')


def docker(*args):
    return subprocess.check_output(['docker', *args], text=True).strip()


def main():
    settings = {}
    secrets = {}
    for component in ('api', 'rabbitmq'):
        service = f'{STACK}_driver-app-{component}'
        spec = json.loads(docker('service', 'inspect', service))[0]['Spec']['TaskTemplate']['ContainerSpec']
        env = dict(item.split('=', 1) for item in spec.get('Env', []) if '=' in item)
        settings[component] = {key: env.get(key, DEFAULTS.get(key, '')) for key in KEYS}
        secrets[component] = next((s['SecretID'] for s in spec.get('Secrets', [])
                                   if s.get('File', {}).get('Name') == 'VAULT_TOKEN'), None)
        print(f'{component}: image={spec["Image"].split("@")[0]}')
        # Print names/paths, never Vault address credentials or secret values.
        for key in KEYS[1:]:
            print(f'  {key}={settings[component][key]}')
    print('Same Vault address:', settings['api']['VAULT_ADDR'] == settings['rabbitmq']['VAULT_ADDR'])
    same_token = secrets['api'] is not None and secrets['api'] == secrets['rabbitmq']
    print('Same VAULT_TOKEN secret:', same_token)
    if not same_token:
        raise SystemExit('Cannot probe API access with a different broker token. Check service secret configuration.')
    containers = docker('ps', '-q', '--filter', f'label=com.docker.swarm.service.name={STACK}_driver-app-rabbitmq').splitlines()
    if len(containers) != 1:
        raise SystemExit('Run this on the node with the single running RabbitMQ container.')
    probe = '''
import json, sys
from pathlib import Path
from urllib.request import Request, urlopen
from urllib.error import HTTPError, URLError
s=json.loads(sys.argv[1])
version=s['VAULT_KV_VERSION']
if version == 'auto':
    try:
        req=Request(s['VAULT_ADDR'].rstrip('/') + '/v1/sys/mounts/' + s['VAULT_MOUNT'].strip('/') + '/tune', headers={'X-Vault-Token':Path('/run/secrets/VAULT_TOKEN').read_text().strip()})
        with urlopen(req, timeout=30) as response:
            metadata=json.load(response)
        options=metadata.get('data', metadata).get('options') or {}
        version='2' if str(options.get('version', '1')) == '2' else '1'
    except (URLError, ValueError):
        version='1'
        print('Mount detection unavailable; Java-compatible fallback to KV v1.')
path=s['VAULT_MOUNT'].strip('/') + ('/data/' if version == '2' else '/') + s['VAULT_BASE_PATH'].strip('/') + '/' + s['CLIENT'] + '_' + s['ENVIRONMENT']
print('Probing API Vault path: /v1/' + path)
request=Request(s['VAULT_ADDR'].rstrip('/') + '/v1/' + path, headers={'X-Vault-Token':Path('/run/secrets/VAULT_TOKEN').read_text().strip()})
try:
    with urlopen(request, timeout=30) as response:
        print('Vault HTTP status:', response.status)
except HTTPError as error:
    print('Vault HTTP status:', error.code)
    print({403:'Access denied: check token policy and expiration.',404:'Path not found: check mount, KV version and client/environment path.'}.get(error.code, 'Vault or its HTTP proxy returned an error.'))
except URLError:
    print('Vault connection failed (DNS, TLS or network); no credentials printed.')
'''
    subprocess.run(['docker', 'exec', '-i', containers[0], 'python3', '-', json.dumps(settings['api'])],
                   input=probe, text=True, check=True)
    print('Probe uses the broker network and the same secret; API-container network differences remain possible.')


if __name__ == '__main__':
    main()
