#!/usr/bin/env python3
"""Exercise real k6 requests against a local mock; never contacts the real API."""
import base64
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

records = []
mode = 'success'
class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        parameters = json.loads(self.rfile.read(int(self.headers['Content-Length'])))['ReaxiumParameters']
        assert self.headers['Authorization'] == 'Basic ' + base64.b64encode(b'mock-user:mock-password').decode()
        payload = next(iter(parameters.values()))
        catalog = json.loads(Path(__file__).with_name('services.json').read_text())
        descriptor = next(entry for entry in catalog.values() if entry.get('path') == self.path.replace('/school_bus_cloud', ''))
        assert list(parameters) == [descriptor['namespace']]
        records.append((self.path, payload))
        if mode == 'merged-partial':
            response = {'code': 3, 'object': {'routes': [{'route_id': r, 'response': {'code': 3, 'object': {'database_saved': True}}} for r in [1, 2]]}}
        elif mode == 'failure':
            response = {'code': 3, 'object': [], 'message': 'invalid data'}
        elif mode == 'partial':
            response = {'code': 3, 'object': {'database_saved': True}, 'message': 'delivery failed'}
        else:
            response = {'code': 0, 'object': [], 'message': 'SUCCESSFUL REQUEST'}
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(json.dumps({'ReaxiumResponse': response}).encode())
    def log_message(self, *args):
        pass

server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
try:
    with tempfile.TemporaryDirectory() as folder:
        fixture = Path(folder) / 'devices.json'
        fixture.write_text(json.dumps([dict(device_id=i,device_imei=f'mock-imei-{i}',business_id=1,route_id=1,route_ids='1,2',passenger_id=3,access_type=3,driver_id=1,bus_info='1',start_stop_id=1,end_stop_id=2,start_latitude=27,start_longitude=-80,end_latitude=27,end_longitude=-80) for i in [1,2]]))
        env = dict(os.environ, API_BASE_URL=f'http://127.0.0.1:{server.server_port}',REAXIUM_API_USER='mock-user',REAXIUM_API_PASSWORD='mock-password',START_DATE='2026-10-22T08:00:00Z',VUS='2',JOURNEYS='2',MAX_DURATION='30s', SERVICE='journey')
        for test_mode in ['success', 'partial', 'failure']:
            mode = test_mode
            records.clear()
            result = subprocess.run([str(Path(__file__).with_name('run.sh').resolve()),str(fixture)],env=env,text=True,capture_output=True,timeout=90)
            if mode == 'success':
                assert result.returncode == 0, result.stdout + result.stderr
            else:
                assert result.returncode != 0, 'Failure must cause a nonzero exit'
            if mode != 'failure':
                assert len(records) == 8, (mode, len(records), result.stdout, result.stderr)
                traces = {}
                for path, payload in records:
                    traces.setdefault(payload['trace_id'], []).append((path,payload))
                assert len(traces) == 4
                for events in traces.values():
                    assert events[0][0].endswith('beginARoute') and events[1][0].endswith('endARoute')
                    assert events[0][1]['device_id'] == events[1][1]['device_id']
                    assert events[0][1]['traffic_date'] < events[1][1]['traffic_date']
            else:
                assert records and all(path.endswith('beginARoute') for path,_ in records)
            print(f'PASS: {mode}')
        mode = 'success'
        catalog = json.loads(Path(__file__).with_name('services.json').read_text())
        for service in [name for name, entry in catalog.items() if 'path' in entry] + ['startARoute', 'allServices']:
            records.clear()
            env['SERVICE'] = service
            result = subprocess.run([str(Path(__file__).with_name('run.sh').resolve()),str(fixture)],env=env,text=True,capture_output=True,timeout=90)
            assert result.returncode == 0, result.stdout + result.stderr
            endpoint = 'beginARoute' if service == 'startARoute' else service
            expected_steps = catalog[service].get('steps', [endpoint]) if service != 'startARoute' else [endpoint]
            expected_paths = [catalog[step]['path'] for step in expected_steps]
            assert len(records) == 4 * len(expected_steps), (service, len(records))
            assert set(path for path, _ in records) == set(expected_paths)
            for trace in {p['trace_id'] for _, p in records}:
                assert [path for path, p in records if p['trace_id'] == trace] == expected_paths
            if service == 'executeAnAccessOfAUserInABus':
                from datetime import datetime
                from zoneinfo import ZoneInfo
                for trace in {p['trace_id'] for _, p in records}:
                    events = [p for _, p in records if p['trace_id'] == trace]
                    begin, access, end = events
                    assert len({p['device_id'] for p in events}) == 1
                    assert len({p['route_id'] for p in events}) == 1
                    local_access = datetime.fromtimestamp(access['accessDate'] / 1000, ZoneInfo('America/New_York')).strftime('%Y-%m-%d %H:%M:%S')
                    assert begin['traffic_date'] < local_access < end['traffic_date']
            if service == 'deviceStatus':
                assert all('device_imei' in payload and 'device_location' in payload and 'device_status' in payload for _,payload in records)
            assert len({payload['trace_id'] for _,payload in records}) == 4
            assert 'service_iterations_completed' in result.stdout
            assert 'journeys_completed' not in result.stdout
            print(f'PASS: {service}')
        mode = 'merged-partial'
        records.clear()
        env.update(SERVICE='beginAMergeRoute', VUS='1', JOURNEYS='1')
        result = subprocess.run([str(Path(__file__).with_name('run.sh').resolve()),str(fixture)],env=env,text=True,capture_output=True,timeout=90)
        assert result.returncode != 0
        assert len(records) == 1, 'Partial saves must never be retried'
        assert 'save/queue failed or outcome unknown' not in result.stdout + result.stderr
        print('PASS: merged partial saves fail application threshold without retries')
        mode = 'success'
        records.clear()
        fixture.write_text(json.dumps([dict(device_id=1)]))
        env['SERVICE'] = 'allServices'
        result = subprocess.run([str(Path(__file__).with_name('run.sh').resolve()),str(fixture)],env=env,text=True,capture_output=True,timeout=90)
        assert result.returncode != 0 and not records
        print('PASS: invalid fixtures rejected before requests')



finally:
    server.shutdown()
