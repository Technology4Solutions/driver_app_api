#!/usr/bin/env python3
"""Offline checks: no Docker daemon, Vault server, or credentials needed."""
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('entrypoint', ROOT / 'vault-entrypoint.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ConfigurationTests(unittest.TestCase):
    def bundle(self):
        return dict(SCHOOL_BUS_DATASOURCE_URL='jdbc:mysql://db:3307/school_bus?useSSL=false',
                    SCHOOL_BUS_DATASOURCE_USERNAME='test', SCHOOL_BUS_DATASOURCE_PASSWORD='test',
                    REAXIUM_API_USER='api', REAXIUM_API_PASSWORD='test',
                    RABBITMQ_USER='a@b', RABBITMQ_PASSWORD='a/b:c', BROADCAST_ENABLED=False)

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
if os.getenv('MOCK_PULL_FAIL') and a[0]=='pull':sys.exit(1)
if os.getenv('MOCK_SECRET_MISSING') and a[:2]==['secret','inspect']:sys.exit(1)
if os.getenv('MOCK_MONITORING_FAIL') and a[0]=='compose' and 'up' in a:sys.exit(1)
if a[:2]==['service','ls'] and not os.getenv('MOCK_EMPTY_STACK'):print('driver_app_api_stack_driver-app-api')
if a[0]=='info':print('active' if 'LocalNodeState' in a[-1] else 'true')
if a[:2]==['stack','services']:
 for name in ['driver-app-api','driver-app-rabbitmq','driver-app-redis']:print('driver_app_api_stack_'+name+' 1/1')
if a[:2]==['service','inspect'] and '--format' in a:
 fmt=a[a.index('--format')+1]
 print('reaxiumapps/driver_app_api:1.2.3@sha256:mock' if 'ContainerSpec.Image' in fmt else os.getenv('MOCK_STATE','completed'))
''')
        mock.chmod(0o755)
        self.env = dict(os.environ, PATH=str(self.path)+':'+os.environ['PATH'], MOCK_LOG=str(self.log))

    def run_script(self, name, *args, **env):
        return subprocess.run(['bash', str(self.path / 'ci_cd' / name), *args],
                              env=dict(self.env, **env), capture_output=True, text=True, timeout=10)

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
        self.assertNotIn('stack deploy', calls)
        self.assertIn('service update', calls)
        self.assertIn('--force', calls)
        self.assertIn('up -d --pull always --force-recreate --wait', calls)

    def test_monitoring_failure_fails_deployment(self):
        result = self.run_script('deploy.sh', '--client', 'TEST', '--env', 'PROD', MOCK_MONITORING_FAIL='1')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('starting Prometheus and Grafana', result.stderr)
        self.assertNotIn('Deployment ready:', result.stdout)

    def test_monitoring_custom_stack_and_stop_preserve_data(self):
        result = self.run_script('monitoring.sh', 'start', 'custom_stack')
        self.assertEqual(result.returncode, 0, result.stderr)
        config = (self.path / 'monitoring/prometheus.generated.yml').read_text()
        self.assertIn('tasks.custom_stack_driver-app-api', config)
        result = self.run_script('monitoring.sh', 'stop')
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.log.read_text()
        self.assertIn('down', calls)
        self.assertNotIn('--volumes', calls)

    def test_rollback_and_invalid_environment_fail(self):
        result = self.run_script('deploy.sh', '--client', 'TEST', '--env', 'DEV', MOCK_STATE='rollback_completed')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('waiting for stack', result.stderr)
        self.assertIn('stack ps --no-trunc', self.log.read_text())
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
            self.assertNotIn('stack deploy', self.log.read_text())


if __name__ == '__main__':
    unittest.main()
