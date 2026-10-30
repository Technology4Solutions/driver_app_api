#!/usr/bin/env python3
"""Offline checks: no Docker daemon, Vault server, or credentials needed."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch, MagicMock

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('entrypoint', ROOT / 'vault-entrypoint.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ConfigurationTests(unittest.TestCase):
    def test_database_pool_defaults_and_overrides(self):
        result = module.configuration({'ENVIRONMENT': 'DEV'}, self.bundle(), 'api')
        self.assertEqual(result['DB_MAX_CONNECTIONS'], '5')
        self.assertEqual(result['DB_CONNECT_TIMEOUT_SECONDS'], '10')
        bundle = self.bundle()
        bundle.update(DB_HOST='db', DB_NAME='school_bus', DB_MAX_CONNECTIONS=12,
                      DB_CONNECT_TIMEOUT_SECONDS=30)
        result = module.configuration({'ENVIRONMENT': 'DEV'}, bundle, 'api')
        self.assertEqual(result['DB_PORT'], '3306')
        self.assertEqual(result['DB_MAX_CONNECTIONS'], '12')
        self.assertEqual(result['DB_CONNECT_TIMEOUT_SECONDS'], '30')
        for key in ('DB_MAX_CONNECTIONS', 'DB_CONNECT_TIMEOUT_SECONDS'):
            for value in ('', '0', '-1', 'invalid', str(2**64)):
                with self.assertRaises(module.ConfigurationError):
                    module.configuration({'ENVIRONMENT': 'DEV'}, dict(self.bundle(), **{key: value}), 'api')

    def test_check_mode_validates_without_starting_application(self):
        response = MagicMock()
        response.__enter__.return_value = response
        response.read.return_value = json.dumps({'data': {'data': self.bundle()}}).encode()
        with patch.dict(os.environ, dict(CLIENT='TEST', ENVIRONMENT='DEV', VAULT_KV_VERSION='2'), clear=True), \
             patch.object(module.sys, 'argv', ['entrypoint', 'api', '--check']), \
             patch.object(module.Path, 'read_text', return_value='test-token'), \
             patch.object(module, 'urlopen', return_value=response), \
             patch.object(module.os, 'execvpe') as execute:
            module.main()
            execute.assert_not_called()

    def test_missing_credentials_report_only_the_setting_name(self):
        bundle = self.bundle()
        del bundle['REAXIUM_API_USER']
        with self.assertRaises(module.ConfigurationError) as error:
            module.configuration({'ENVIRONMENT': 'DEV'}, bundle, 'api')
        self.assertEqual(str(error.exception), 'Missing application setting: REAXIUM_API_USER')

    def test_shared_vault_path_and_override(self):
        for mode in ('api', 'rabbitmq'):
            for environment in ('DEV', 'PROD'):
                for override in (None, 'custom/device_api'):
                    env = dict(CLIENT='MARTIN_COUNTY', ENVIRONMENT=environment, VAULT_KV_VERSION='2')
                    if override:
                        env['VAULT_BASE_PATH'] = override
                    response = MagicMock()
                    response.__enter__.return_value = response
                    response.read.return_value = json.dumps({'data': {'data': self.bundle()}}).encode()
                    with patch.dict(os.environ, env, clear=True), \
                         patch.object(module.sys, 'argv', ['entrypoint', mode]), \
                         patch.object(module.Path, 'read_text', return_value='test-token'), \
                         patch.object(module, 'urlopen', return_value=response) as request, \
                         patch.object(module.os, 'execvpe'):
                        module.main()
                    base = override or 'reaxium_ci/device_api'
                    self.assertEqual(request.call_args.args[0].full_url,
                                     f'https://vault.reaxium.com/v1/kv/data/{base}/MARTIN_COUNTY_{environment}')

    def bundle(self):
        return dict(SCHOOL_BUS_DATASOURCE_URL='jdbc:mysql://db:3307/school_bus?useSSL=false',
                    SCHOOL_BUS_DATASOURCE_USERNAME='test', SCHOOL_BUS_DATASOURCE_PASSWORD='test',
                    REAXIUM_API_USER='api', REAXIUM_API_PASSWORD='test',
                    RABBITMQ_USER='a@b', RABBITMQ_PASSWORD='a/b:c', BROADCAST_ENABLED=False)

    def test_kv_detection_matches_java_with_wrapped_metadata_support(self):
        for payload, expected in [({'options': {'version': '2'}}, '2'),
                                  ({'data': {'options': {'version': '2'}}}, '2'),
                                  ({'data': {'options': None}}, '1'), ({}, '1')]:
            response = MagicMock()
            response.__enter__.return_value = response
            response.read.return_value = json.dumps(payload).encode()
            with patch.object(module, 'urlopen', return_value=response) as request:
                self.assertEqual(module.kv_version('https://vault.test', 'kv', 'test-token'), expected)
                self.assertEqual(request.call_args.args[0].full_url, 'https://vault.test/v1/sys/mounts/kv/tune')
                self.assertEqual(request.call_args.args[0].get_header('X-vault-token'), 'test-token')
        with patch.object(module, 'urlopen', side_effect=module.HTTPError('https://vault.test', 403, 'denied', {}, None)):
            self.assertEqual(module.kv_version('https://vault.test', 'kv', 'test-token'), '1')
        with patch.object(module, 'urlopen') as request:
            for version in ('1', '2'):
                self.assertEqual(module.kv_version('https://vault.test', 'kv', 'test-token', version), version)
            request.assert_not_called()

    def test_java_keys_and_url_encoding(self):
        for environment in ('DEV', 'PROD'):
            result = module.configuration({'ENVIRONMENT': environment}, self.bundle(), 'api')
            self.assertEqual(result['DB_PORT'], '3307')
            self.assertEqual(result['DB_NAME'], 'school_bus')
            self.assertEqual(result['APP_ENV'], environment.lower())
            self.assertEqual(result['RABBITMQ_URL'], 'amqp://a%40b:a%2Fb%3Ac@driver-app-rabbitmq:5672/%2f')

    def test_native_keys_and_broadcast_validation(self):
        bundle = self.bundle()
        bundle.update(DB_HOST='native', DB_NAME='native_db', DB_USER='native_user', DB_PASSWORD='native_pass')
        result = module.configuration({'ENVIRONMENT': 'DEV'}, bundle, 'api')
        self.assertEqual(result['DB_HOST'], 'native')
        del bundle['BROADCAST_ENABLED']
        with self.assertRaises(ValueError):
            module.configuration({'ENVIRONMENT': 'DEV'}, bundle, 'api')

    def test_rejects_missing_broker_secret_and_ignores_executable_settings(self):
        bundle = self.bundle()
        bundle['LD_PRELOAD'] = 'unexpected'
        result = module.configuration({'ENVIRONMENT': 'DEV'}, bundle, 'api')
        self.assertNotIn('LD_PRELOAD', result)
        del bundle['RABBITMQ_PASSWORD']
        with self.assertRaises(ValueError):
            module.configuration({}, bundle, 'rabbitmq')


class ScriptTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name)
        shutil.copytree(ROOT, self.path / 'ci_cd', ignore=shutil.ignore_patterns('__pycache__'))
        shutil.copytree(ROOT.parent / 'monitoring', self.path / 'monitoring',
                        ignore=shutil.ignore_patterns('*.local.*', '*.generated.*'))
        (self.path / 'VERSION').write_text('1.2.3\n')
        self.log = self.path / 'calls'
        mock = self.path / 'docker'
        mock.write_text('''#!/usr/bin/env python3
import os,sys
from pathlib import Path
a=sys.argv[1:]
with open(os.environ['MOCK_LOG'],'a') as f:f.write(' '.join(a)+'\\n')
if os.getenv('MOCK_FAIL') and a[0]=='push':sys.exit(1)
if os.getenv('MOCK_NO_COMPOSE') and a[0]=='compose':sys.exit(1)
if os.getenv('MOCK_NO_STACK_CONFIG') and a[:2]==['stack','config']:sys.exit(125)
if os.getenv('MOCK_PULL_FAIL') and a[0]=='pull':sys.exit(1)
if os.getenv('MOCK_SECRET_MISSING') and a[:2]==['secret','inspect']:sys.exit(1)
if os.getenv('MOCK_MONITORING_FAIL') and a[:2]==['stack','deploy'] and a[-1].endswith('_monitoring'):sys.exit(1)
if a[0]=='ps':print('mock-container')
if a[0]=='inspect':print('healthy')
if os.getenv('MOCK_DEPENDENCY_PAUSED') and a[:2]==['service','inspect'] and a[-1].endswith('driver-app-rabbitmq'):print('paused');sys.exit(0)
if a[:2]==['service','ls'] and not any('_monitoring' in x for x in a) and not os.getenv('MOCK_EMPTY_STACK'):print('driver_app_api_stack_driver-app-api')
if a[0]=='info':print('active' if 'LocalNodeState' in a[-1] else 'true')
if a[:2]==['stack','services']:
 for name in ['driver-app-api','driver-app-rabbitmq','driver-app-redis']:print('driver_app_api_stack_'+name+' 1/1')
if a[:2]==['service','inspect'] and '--format' in a:
 fmt=a[a.index('--format')+1]
 if os.getenv('MOCK_MONITORING_PAUSED') and '_monitoring_' in a[-1]:print('paused');sys.exit(0)
 print('reaxiumapps/driver_app_api:1.2.3@sha256:mock' if 'ContainerSpec.Image' in fmt else os.getenv('MOCK_STATE','completed'))
''')
        mock.chmod(0o755)
        self.env = dict(os.environ, PATH=str(self.path)+':'+os.environ['PATH'], MOCK_LOG=str(self.log))

    def run_script(self, name, *args, **env):
        return subprocess.run(['bash', str(self.path / 'ci_cd' / name), *args],
                              env=dict(self.env, **env), capture_output=True, text=True, timeout=30)

    def test_versioning_and_failed_push(self):
        for bump, expected in [('patch','1.2.4'), ('minor','1.3.0'), ('major','2.0.0')]:
            (self.path / 'VERSION').write_text('1.2.3\n')
            result = self.run_script('build.sh', '-t', bump, '--no-push')
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((self.path / 'VERSION').read_text().strip(), expected)
        (self.path / 'VERSION').write_text('1.2.3\n')
        self.assertNotEqual(self.run_script('build.sh', MOCK_FAIL='1').returncode, 0)
        self.assertEqual((self.path / 'VERSION').read_text().strip(), '1.2.3')
        self.assertEqual(self.run_script('build.sh', '--dry-run').returncode, 0)
        self.assertEqual((self.path / 'VERSION').read_text().strip(), '1.2.3')

    def test_full_stack_and_app_only(self):
        for env in ['DEV', 'PROD']:
            result = self.run_script('deploy.sh', '--client', 'TEST', '--env', env, '--all')
            self.assertEqual(result.returncode, 0, result.stderr)
        self.log.write_text('')
        result = self.run_script('deploy.sh', '--client', 'TEST', '--env', 'DEV', '--app', '--force')
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.log.read_text()
        self.assertNotIn('-c ci_cd/docker-compose.swarm.yml', calls)
        self.assertIn('service update', calls)
        self.assertIn('--force', calls)
        self.assertIn('-c monitoring/stack.yml -c monitoring/stack.dev.yml driver_app_api_stack_monitoring', calls)
        self.assertIn('--env-add VAULT_BASE_PATH=reaxium_ci/device_api', calls)
        self.assertIn('--env-add VAULT_KV_VERSION=auto', calls)

    def test_app_update_preserves_explicit_vault_path(self):
        result = self.run_script('deploy.sh', '--client', 'TEST', '--env', 'DEV', '--app',
                                 VAULT_BASE_PATH='custom/device_api')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('--env-add VAULT_BASE_PATH=custom/device_api', self.log.read_text())

    def test_build_without_compose(self):
        result = self.run_script('build.sh', '-t', 'patch', MOCK_NO_COMPOSE='1')
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.log.read_text()
        self.assertNotIn('compose', calls)
        self.assertIn('push reaxiumapps/driver_app_api:1.2.4', calls)
        self.assertIn('push reaxiumapps/driver_app_api-rabbitmq:1.2.4', calls)
        self.assertEqual((self.path / 'VERSION').read_text().strip(), '1.2.4')

    def test_deploy_without_compose(self):
        result = self.run_script('deploy.sh', '--client', 'TEST', '--env', 'DEV', MOCK_NO_COMPOSE='1')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn('compose ', self.log.read_text())
        self.assertIn('-c monitoring/stack.yml', self.log.read_text())

    def test_monitoring_failure_fails_deployment(self):
        result = self.run_script('deploy.sh', '--client', 'TEST', '--env', 'PROD', MOCK_MONITORING_FAIL='1')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('starting Prometheus and Grafana', result.stderr)
        self.assertNotIn('Deployment ready:', result.stdout)

    def test_older_docker_without_stack_config(self):
        for mode in ('--all', '--app'):
            self.log.write_text('')
            result = self.run_script('deploy.sh', '--client', 'TEST', '--env', 'DEV', mode,
                                     MOCK_NO_COMPOSE='1', MOCK_NO_STACK_CONFIG='1')
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = self.log.read_text()
            self.assertNotIn('stack config', calls)
            self.assertNotIn('compose ', calls)
            self.assertIn('-c monitoring/stack.yml', calls)

    def test_missing_monitoring_file_fails_before_deployment(self):
        (self.path / 'monitoring/stack.yml').unlink()
        result = self.run_script('deploy.sh', '--client', 'TEST', '--env', 'DEV')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Missing monitoring file: monitoring/stack.yml', result.stderr)
        self.assertNotIn('stack deploy --with-registry-auth', self.log.read_text())

    def test_monitoring_custom_stack_and_stop_preserve_data(self):
        result = self.run_script('monitoring.sh', 'start', 'custom_stack')
        self.assertEqual(result.returncode, 0, result.stderr)
        config = (self.path / 'monitoring/prometheus.generated.yml').read_text()
        self.assertIn('tasks.custom_stack_driver-app-api', config)
        result = self.run_script('monitoring.sh', 'stop')
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.log.read_text()
        self.assertIn('stack rm driver_app_api_stack_monitoring', calls)
        self.assertNotIn('--volumes', calls)

    def test_paused_monitoring_fails_with_task_diagnostics(self):
        result = self.run_script('monitoring.sh', 'start', MOCK_MONITORING_PAUSED='1')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Monitoring update failed', result.stderr)
        self.assertIn('stack ps --no-trunc driver_app_api_stack_monitoring', self.log.read_text())

    def test_grafana_port_override_only_in_dev(self):
        for environment in ('DEV', 'PROD'):
            self.log.write_text('')
            result = self.run_script('monitoring.sh', 'start', ENVIRONMENT=environment)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual('-c monitoring/stack.dev.yml' in self.log.read_text(), environment == 'DEV')

    def test_dependencies_healthy_before_api_deployment(self):
        result = self.run_script('deploy.sh', '--client', 'TEST', '--env', 'DEV', '--all')
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.log.read_text()
        dependencies = calls.index('-c ci_cd/docker-compose.dependencies.yml')
        application = calls.index('-c ci_cd/docker-compose.swarm.yml')
        self.assertLess(dependencies, application)
        self.assertIn('inspect --format {{.State.Health.Status}}', calls[dependencies:application])
        self.assertNotIn('--prune', calls)

    def test_failed_dependency_prevents_api_update(self):
        result = self.run_script('deploy.sh', '--client', 'TEST', '--env', 'DEV', '--all', MOCK_DEPENDENCY_PAUSED='1')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Dependency deployment failed', result.stderr)
        self.assertNotIn('-c ci_cd/docker-compose.swarm.yml', self.log.read_text())

    def test_dependency_definitions_match_full_stack(self):
        full = (ROOT / 'docker-compose.swarm.yml').read_text()
        dependencies = (ROOT / 'docker-compose.dependencies.yml').read_text().split('\n', 1)[1]
        start = full.index('  driver-app-api:\n')
        end = full.index('  driver-app-rabbitmq:\n')
        self.assertEqual(dependencies, full[:start] + full[end:])

    def test_full_force_waits_for_stack_before_recreation(self):
        result = self.run_script('deploy.sh', '--client', 'TEST', '--env', 'DEV', '--all', '--force')
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.log.read_text()
        forced = calls.index('service update --detach=true --force')
        self.assertIn('stack services', calls[:forced])
        self.assertIn('service inspect --format', calls[:forced])
        self.assertIn('stack services', calls[forced:])
        self.assertEqual(calls.splitlines().count('service update --detach=true --force driver_app_api_stack_driver-app-api'), 1)

    def test_rollback_and_invalid_environment_fail(self):
        result = self.run_script('deploy.sh', '--client', 'TEST', '--env', 'DEV', MOCK_STATE='rollback_completed')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('updating RabbitMQ and Redis', result.stderr)
        self.assertIn('stack ps --no-trunc', self.log.read_text())
        self.assertIn('service logs --timestamps --since 5m --tail 80 driver_app_api_stack_driver-app-api', self.log.read_text())
        self.assertIn('service logs --timestamps --since 5m --tail 80 driver_app_api_stack_driver-app-rabbitmq', self.log.read_text())
        self.assertNotEqual(self.run_script('deploy.sh', '--client', 'TEST', '--env', 'invalid').returncode, 0)

    def test_app_only_bootstraps_missing_stack(self):
        result = self.run_script('deploy.sh', '--client', 'TEST', '--env', 'DEV', '--app', '--force', MOCK_EMPTY_STACK='1')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('first deployment', result.stdout)
        self.assertIn('stack deploy', self.log.read_text())
        self.assertIn('pull reaxiumapps/driver_app_api-rabbitmq:1.2.3', self.log.read_text())

    def test_preflight_errors_identify_failing_stage(self):
        for setting, message in [('MOCK_PULL_FAIL', 'pulling API image'),
                                 ('MOCK_SECRET_MISSING', 'VAULT_TOKEN Swarm secret')]:
            self.log.write_text('')
            result = self.run_script('deploy.sh', '--client', 'TEST', '--env', 'DEV', **{setting: '1'})
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(message, result.stderr)
            self.assertNotIn('stack deploy --with-registry-auth', self.log.read_text())


if __name__ == '__main__':
    unittest.main()
