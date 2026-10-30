import ast
import base64
import io
import json
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest
from unittest.mock import patch
from types import SimpleNamespace

from server import Runner, make_handler, load_credentials


class Connection:
    def __init__(self, request):
        self.input = io.BytesIO(request)
        self.output = bytearray()

    def makefile(self, *args):
        return self.input

    def sendall(self, data):
        self.output.extend(data)


class Tests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        dataset = Path(self.temp.name, 'devices.json')
        dataset.write_text('[{"device_id":1}]')
        self.runner = Runner(dataset, 'http://127.0.0.1:8080')
        self.data = dict(service='deviceStatus', vus=1, journeys=1, duration='30m',
                         date='2026-10-30T08:00:00Z', username='test-user',
                         password='test-password', acknowledge=True)

    def request(self, method='GET', path='/api/config', auth=True, host='dev.example:5665', origin=None, token='csrf'):
        headers = [f'{method} {path} HTTP/1.1', f'Host: {host}']
        if auth:
            headers.append('Authorization: Basic ' + base64.b64encode(b'runner:access').decode())
        if origin:
            headers.append('Origin: ' + origin)
        headers += ['X-Runner-Token: ' + token, 'Content-Length: 2', 'Connection: close']
        connection = Connection(('\r\n'.join(headers) + '\r\n\r\n{}').encode())
        make_handler(self.runner, 'csrf', 'dev.example', 'access')(connection, ('127.0.0.1', 1234), SimpleNamespace(server_port=5665))
        return bytes(connection.output)

    def test_auth_required_for_config_and_page(self):
        for path in ('/', '/api/config', '/api/state'):
            result = self.request(path=path, auth=False)
            self.assertIn(b'401 Unauthorized', result)
            self.assertNotIn(b'csrf', result)
        self.assertIn(b'200 OK', self.request())

    def test_host_origin_and_csrf(self):
        self.assertIn(b'403', self.request(host='attacker.example:5665'))
        self.assertIn(b'403', self.request('POST', '/api/stop', origin='http://attacker.example'))
        self.assertIn(b'403', self.request('POST', '/api/stop', token='wrong'))
        self.assertIn(b'401', self.request('POST', '/api/stop', auth=False))
        self.assertIn(b'202', self.request('POST', '/api/stop', origin='http://dev.example:5665'))

    def test_validation_does_not_launch(self):
        for changes in ({'vus':2}, {'journeys':0}, {'acknowledge':False}, {'service':'bad'}, {'date':'bad'}):
            with self.assertRaises(ValueError):
                self.runner.start(dict(self.data, **changes))
        self.assertIsNone(self.runner.thread)

    def test_container_lifecycle_and_summary(self):
        commands = []
        def docker(command, **kwargs):
            commands.append(command)
            stdout = ''
            if command[1] == 'create':
                mount = next(arg for arg in command if 'dst=/results' in arg)
                directory = mount.split('src=')[1].split(',dst=')[0]
                Path(directory, 'summary.json').write_text('{"metrics":{"http_reqs":{"count":1}}}')
                self.assertNotIn('test-password', ' '.join(command))
            if command[1] == 'inspect':
                stdout = json.dumps({'Running':False, 'ExitCode':0})
            if command[1] == 'logs':
                stdout = 'test-password sample output'
            return SimpleNamespace(returncode=0, stdout=stdout, stderr='')
        with patch('server.subprocess.run', side_effect=docker):
            self.runner.start(self.data)
            self.runner.thread.join(3)
        state = self.runner.snapshot()
        self.assertEqual(state['status'], 'passed')
        self.assertEqual(state['summary']['metrics']['http_reqs']['count'], 1)
        self.assertNotIn('test-password', state['logs'])
        self.assertEqual(commands[-1][1:3], ['rm', '-f'])

    def test_env_credentials_used_without_browser_values(self):
        env_file = Path(self.temp.name, '.env')
        env_file.write_text("export REAXIUM_API_USER='fixture user'\nREAXIUM_API_PASSWORD='fixture#password' # comment\n")
        with patch.dict('os.environ', {}, clear=True):
            runner = Runner(self.runner.dataset, self.runner.api_url, env_file)
        self.assertTrue(runner.credentials_ready())
        data = dict(self.data)
        del data['username']
        del data['password']
        with patch.object(runner, 'execute') as execute:
            runner.start(data)
            runner.thread.join(3)
        env = execute.call_args.args[0]
        self.assertEqual(env['REAXIUM_API_USER'], 'fixture user')
        self.assertEqual(env['REAXIUM_API_PASSWORD'], 'fixture#password')
        self.runner = runner
        response = self.request()
        self.assertIn(b'"credentials_configured": true', response)
        self.assertNotIn(b'fixture', response)
        self.assertNotIn('fixture', json.dumps(runner.snapshot()))
        with patch.dict('os.environ', {'REAXIUM_API_USER': 'override'}, clear=True):
            self.assertEqual(load_credentials(env_file)['REAXIUM_API_USER'], 'override')

    def test_python35_compatibility_and_random_tokens(self):
        from server import token_urlsafe
        source = Path(__file__).with_name('server.py').read_text()
        tree = ast.parse(source, feature_version=(3, 5))
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                self.assertNotIn('secrets', [alias.name for alias in node.names])
            if isinstance(node, ast.Call):
                self.assertFalse({'capture_output', 'text'} & {kw.arg for kw in node.keywords})
        tokens = {token_urlsafe(24) for _ in range(100)}
        self.assertEqual(len(tokens), 100)
        for token in tokens:
            self.assertRegex(token, r'^[A-Za-z0-9_-]{32}$')

    def test_config_check_without_starting_server(self):
        token = Path(self.temp.name, 'token')
        token.write_text('test-browser-token-long-enough')
        env_file = Path(self.temp.name, '.env')
        env_file.write_text('REAXIUM_API_USER=test-user\nREAXIUM_API_PASSWORD=test-password\n')
        command = [sys.executable, str(Path(__file__).with_name('server.py')),
                   '--check-config', '--bind', '0.0.0.0', '--public-host', 'dev.example',
                   '--dataset', str(self.runner.dataset), '--env-file', str(env_file),
                   '--access-token-file', str(token)]
        result = subprocess.run(command, capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('configuration validated', result.stdout)
        self.assertNotIn(token.read_text(), result.stdout + result.stderr)
        token.write_text('short')
        result = subprocess.run(command, capture_output=True, text=True, timeout=5)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('at least 20 characters', result.stderr)

    def test_stop_removes_container(self):
        commands = []
        def docker(command, **kwargs):
            commands.append(command)
            if command[1] == 'start':
                self.runner.stop()
            return SimpleNamespace(returncode=0, stdout=json.dumps({'Running':False, 'ExitCode':137}), stderr='')
        with patch('server.subprocess.run', side_effect=docker):
            self.runner.start(self.data)
            self.runner.thread.join(3)
        self.assertEqual(self.runner.snapshot()['status'], 'stopped')
        self.assertTrue(any(c[1] == 'stop' for c in commands))
        self.assertEqual(commands[-1][1:3], ['rm', '-f'])


if __name__ == '__main__':
    unittest.main()
