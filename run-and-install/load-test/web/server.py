#!/usr/bin/env python3
"""Local or authenticated HTTP k6 control panel. Requires Python 3.5+ and Docker on Linux."""
import argparse
import base64
import datetime
import json
import os
from pathlib import Path
import re
import hmac
import binascii
import shlex
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from socketserver import ThreadingMixIn
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parent


class ThreadingHTTPServer(ThreadingMixIn, HTTPServer):
    # Equivalent to the stdlib threaded HTTP server added in Python 3.7.
    daemon_threads = True


def token_urlsafe(size):
    return base64.urlsafe_b64encode(os.urandom(size)).rstrip(b'=').decode('ascii')


def load_credentials(path):
    keys = ('REAXIUM_API_USER', 'REAXIUM_API_PASSWORD')
    values = {}
    if path is not None and Path(path).exists():
        for line in Path(path).read_text().splitlines():
            key, separator, value = line.partition('=')
            key = key.strip()
            if key.startswith('export '):
                key = key[len('export '):]
            if separator and key in keys:
                try:
                    values[key] = ' '.join(shlex.split(value, comments=True))
                except ValueError:
                    raise ValueError('Invalid quoting for ' + key + ' in .env') from None
    return {key: os.environ.get(key) or values.get(key, '') for key in keys}


class Runner:
    def __init__(self, dataset, api_url, env_file=None):
        self.dataset = Path(dataset).resolve()
        devices = json.loads(self.dataset.read_text())
        if not isinstance(devices, list) or not devices:
            raise ValueError('Dataset must be a nonempty JSON array')
        self.count = len(devices)
        self.api_url = api_url
        self.credentials = load_credentials(env_file) if env_file is not None else {}
        self.lock = threading.RLock()
        self.state = {'status': 'idle', 'logs': '', 'summary': None}
        self.cancel = threading.Event()
        self.thread = None

    def credentials_ready(self):
        return bool(self.credentials.get('REAXIUM_API_USER') and self.credentials.get('REAXIUM_API_PASSWORD'))

    def snapshot(self):
        with self.lock:
            return dict(self.state)

    def start(self, data):
        data = dict(data)
        if self.credentials_ready():
            data['username'] = self.credentials['REAXIUM_API_USER']
            data['password'] = self.credentials['REAXIUM_API_PASSWORD']
        service = data.get('service')
        if service not in ('deviceStatus', 'beginARoute', 'endARoute', 'journey'):
            raise ValueError('Choose a valid service')
        vus, journeys = data.get('vus'), data.get('journeys')
        if type(vus) is not int or not 1 <= vus <= self.count:
            raise ValueError('Devices must be between 1 and %d' % self.count)
        if type(journeys) is not int or not 1 <= journeys <= 100000:
            raise ValueError('Repetitions must be between 1 and 100000')
        duration = data.get('duration', '')
        if not isinstance(duration, str) or not re.fullmatch(r'[1-9][0-9]{0,3}[smh]', duration):
            raise ValueError('Duration must look like 30m, 60s, or 1h')
        date = data.get('date', '')
        try:
            datetime.datetime.strptime(date, '%Y-%m-%dT%H:%M:%SZ')
        except (ValueError, TypeError):
            raise ValueError('Start date must be YYYY-MM-DDTHH:MM:SSZ')
        for key in ('username', 'password'):
            if not isinstance(data.get(key), str) or not data[key] or '\0' in data[key] or len(data[key]) > 4096:
                raise ValueError('API username and password are required')
        if data.get('acknowledge') is not True:
            raise ValueError('Confirm that this API uses a test database')
        with self.lock:
            if self.thread and self.thread.is_alive():
                raise RuntimeError('A test is already running')
            self.cancel.clear()
            run_id = binascii.hexlify(os.urandom(16)).decode('ascii')
            self.state = {'status': 'starting', 'id': run_id, 'logs': '', 'summary': None, 'exit_code': None}
            env = dict(os.environ, API_BASE_URL=self.api_url, SERVICE=service, VUS=str(vus),
                       JOURNEYS=str(journeys), MAX_DURATION=duration, START_DATE=date, RUN_ID=run_id,
                       REAXIUM_API_USER=data['username'], REAXIUM_API_PASSWORD=data['password'])
            self.thread = threading.Thread(target=self.execute, args=(env,), daemon=True)
            self.thread.start()

    def stop(self):
        self.cancel.set()

    def execute(self, env):
        name = 'driver-k6-web-' + env['RUN_ID']
        secret_values = [env['REAXIUM_API_PASSWORD'], env['REAXIUM_API_USER'],
                         base64.b64encode((env['REAXIUM_API_USER'] + ':' + env['REAXIUM_API_PASSWORD']).encode()).decode()]

        def log(line):
            for value in sorted(secret_values, key=len, reverse=True):
                line = line.replace(value, '[redacted]')
            with self.lock:
                self.state['logs'] = (self.state['logs'] + line)[-100000:]

        try:
            with tempfile.TemporaryDirectory(prefix='driver-k6-') as output:
                command = ['docker', 'create', '--name', name, '--network', 'host', '--user', '%s:%s' % (os.getuid(), os.getgid())]
                for key in ('API_BASE_URL', 'SERVICE', 'VUS', 'JOURNEYS', 'MAX_DURATION', 'START_DATE', 'RUN_ID', 'REAXIUM_API_USER', 'REAXIUM_API_PASSWORD'):
                    command += ['-e', key]
                command += ['--mount', 'type=bind,src=%s,dst=/scripts,readonly' % ROOT.parent,
                            '--mount', 'type=bind,src=%s,dst=/data/devices.json,readonly' % self.dataset,
                            '--mount', 'type=bind,src=%s,dst=/results' % output,
                            'grafana/k6:latest', 'run', '--summary-export=/results/summary.json', '/scripts/routes.js']
                result = subprocess.run(command, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, universal_newlines=True, timeout=300)
                if result.returncode:
                    log(result.stderr)
                    raise RuntimeError('Docker could not create the k6 container')
                if not self.cancel.is_set():
                    subprocess.run(['docker', 'start', name], check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)
                    with self.lock:
                        self.state['status'] = 'running'
                    while True:
                        if self.cancel.is_set():
                            subprocess.run(['docker', 'stop', '-t', '2', name], stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15)
                        result = subprocess.run(['docker', 'inspect', '--format', '{{json .State}}', name], stdout=subprocess.PIPE, stderr=subprocess.PIPE, universal_newlines=True, check=True, timeout=15)
                        state = json.loads(result.stdout)
                        logs = subprocess.run(['docker', 'logs', '--tail', '300', name], stdout=subprocess.PIPE, stderr=subprocess.PIPE, universal_newlines=True, timeout=15)
                        with self.lock:
                            self.state['logs'] = ''
                        log(logs.stdout + logs.stderr)
                        if not state['Running']:
                            with self.lock:
                                self.state['exit_code'] = state['ExitCode']
                                self.state['status'] = 'passed' if state['ExitCode'] == 0 else 'failed'
                            break
                        self.cancel.wait(1)
                summary = Path(output, 'summary.json')
                if summary.exists():
                    with self.lock:
                        self.state['summary'] = json.loads(summary.read_text())
        except Exception as error:
            log('\nRunner error: %s\n' % error)
            with self.lock:
                self.state['status'] = 'failed'
        finally:
            try:
                subprocess.run(['docker', 'rm', '-f', name], stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)
            except (OSError, subprocess.SubprocessError):
                log('\nContainer cleanup failed; check container ' + name + '\n')
            if self.cancel.is_set():
                with self.lock:
                    self.state['status'] = 'stopped'


def make_handler(runner, token, public_host=None, access_token=None):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def reply(self, status, body, kind='application/json'):
            raw = json.dumps(body).encode() if kind == 'application/json' else body
            self.send_response(status)
            self.send_header('Content-Type', kind)
            self.send_header('Content-Length', str(len(raw)))
            self.send_header('Cache-Control', 'no-store')
            self.send_header('X-Content-Type-Options', 'nosniff')
            self.send_header('Content-Security-Policy', "default-src 'self'; style-src 'self'; script-src 'self'; frame-ancestors 'none'; base-uri 'none'")
            self.end_headers()
            self.wfile.write(raw)

        def allowed(self):
            host = self.headers.get('Host', '')
            hosts = ['localhost', '127.0.0.1'] + ([public_host] if public_host else [])
            return host in ['%s:%d' % (name, self.server.server_port) for name in hosts]

        def authenticated(self):
            if access_token is None:
                return True
            expected = 'Basic ' + base64.b64encode(('runner:' + access_token).encode()).decode()
            if hmac.compare_digest(self.headers.get('Authorization', ''), expected):
                return True
            self.send_response(401)
            self.send_header('WWW-Authenticate', 'Basic realm="k6 test runner"')
            self.send_header('Content-Length', '0')
            self.send_header('Cache-Control', 'no-store')
            self.end_headers()
            return False

        def do_GET(self):
            if not self.allowed():
                return self.reply(403, {'error': 'Host is not allowed'})
            if not self.authenticated():
                return
            if self.path == '/api/state':
                return self.reply(200, runner.snapshot())
            if self.path == '/api/config':
                return self.reply(200, {'token': token, 'devices': runner.count, 'api_url': runner.api_url, 'dataset': runner.dataset.name, 'credentials_configured': runner.credentials_ready()})
            files = {'/': ('index.html', 'text/html; charset=utf-8'), '/app.js': ('app.js', 'text/javascript'), '/style.css': ('style.css', 'text/css')}
            if self.path not in files:
                return self.reply(404, {'error': 'Not found'})
            filename, kind = files[self.path]
            self.reply(200, (ROOT / filename).read_bytes(), kind)

        def do_POST(self):
            origin = self.headers.get('Origin')
            if not self.allowed() or self.headers.get('X-Runner-Token') != token or (origin and origin != 'http://' + self.headers.get('Host', '')):
                return self.reply(403, {'error': 'Invalid request origin or token'})
            if not self.authenticated():
                return
            try:
                size = int(self.headers.get('Content-Length', '0'))
                if not 0 < size <= 16384:
                    raise ValueError('Invalid request size')
                data = json.loads(self.rfile.read(size))
                if not isinstance(data, dict):
                    raise ValueError('Expected a JSON object')
                if self.path == '/api/start':
                    runner.start(data)
                elif self.path == '/api/stop':
                    runner.stop()
                else:
                    return self.reply(404, {'error': 'Not found'})
                self.reply(202, runner.snapshot())
            except (ValueError, TypeError) as error:
                self.reply(400, {'error': str(error)})
            except RuntimeError as error:
                self.reply(409, {'error': str(error)})
    return Handler


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check-config', action='store_true', help='Validate configuration without starting a server or test')
    parser.add_argument('--bind', choices=['127.0.0.1', '0.0.0.0'], default='127.0.0.1')
    parser.add_argument('--access-token-file', type=Path, help='Persistent browser login password, provisioned by Jenkins')
    parser.add_argument('--public-host', help='Browser-facing IPv4 address or hostname, without scheme or port')
    parser.add_argument('--port', type=int, default=5665)
    parser.add_argument('--api-url', default='http://127.0.0.1:8080')
    parser.add_argument('--env-file', type=Path, default=ROOT.parents[2] / '.env')
    parser.add_argument('--dataset', type=Path, default=ROOT.parent.parent / 'devices.local.json')
    args = parser.parse_args()
    if args.bind == '0.0.0.0' and not args.public_host:
        parser.error('--public-host is required when binding to 0.0.0.0')
    if args.public_host and not re.fullmatch(r'[A-Za-z0-9.-]+', args.public_host):
        parser.error('--public-host must be an IPv4 address or hostname')
    access_token = token_urlsafe(24) if args.bind == '0.0.0.0' else None
    if args.access_token_file:
        access_token = args.access_token_file.read_text().strip()
        if len(access_token) < 20 or any(c.isspace() for c in access_token):
            parser.error('Access token must contain at least 20 characters without whitespace')
    url = urlsplit(args.api_url)
    if url.scheme not in ('http', 'https') or not url.hostname or url.username or url.password or url.query or url.fragment:
        parser.error('--api-url must be an HTTP(S) API URL without credentials, query, or fragment')
    runner = Runner(args.dataset, args.api_url, args.env_file)
    if args.check_config:
        print('k6 web configuration validated; no server or test started', flush=True)
        return
    server = ThreadingHTTPServer((args.bind, args.port), make_handler(runner, token_urlsafe(32), args.public_host, access_token))
    print('k6 control page: http://%s:%d (Ctrl+C to stop)' % (args.public_host or '127.0.0.1', server.server_port), flush=True)
    if access_token:
        if args.access_token_file:
            print('Browser login username: runner; password supplied by access-token file', flush=True)
        else:
            print('Browser login username: runner\nBrowser login password: ' + access_token, flush=True)
        print('HTTP is unencrypted. Restrict inbound access to your IP.', flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        runner.stop()
        if runner.thread:
            runner.thread.join()
        server.server_close()


if __name__ == '__main__':
    main()
