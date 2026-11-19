#!/usr/bin/env python3
"""Exercise real k6 requests against a local mock; never contacts the real API."""
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
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        if self.path == '/api/auth/token':
            assert body == {'username': 'mock-user', 'password': 'mock-password'}
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.end_headers()
            self.wfile.write(json.dumps({'access_token': 'mock.jwt.token', 'token_type': 'Bearer', 'expires_in': 3600}).encode())
            return
        parameters = body['ReaxiumParameters']
        assert self.headers['Authorization'] == 'Bearer mock.jwt.token'
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
        fixture.write_text(json.dumps([dict(attendance_user_id=10,attendance_activity_id=1,attendance_place_id=1,attendance_access_type=3,attendance_device_type=21,attendance_traffic_type=1,device_id=i,device_imei=f'mock-imei-{i}',business_id=1,route_id=1,route_ids='1,2',passenger_id=i*10,access_type=3,passengers=[dict(passenger_id=i*10+j,access_type=3) for j in range(2)],driver_id=1,bus_info='1',start_stop_id=1,end_stop_id=2,start_latitude=27,start_longitude=-80,end_latitude=27,end_longitude=-80) for i in [1,2]]))
        env = dict(os.environ, LOAD_TEST_OFFLINE='1', API_BASE_URL=f'http://127.0.0.1:{server.server_port}',REAXIUM_API_USER='mock-user',REAXIUM_API_PASSWORD='mock-password',START_DATE='2026-10-22T08:00:00Z',VUS='2',JOURNEYS='2',MAX_DURATION='30s', SERVICE='journey')
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
                    traces.setdefault((payload[0] if isinstance(payload, list) else payload)['trace_id'], []).append((path,payload))
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
        for service in [name for name, entry in catalog.items() if 'path' in entry] + ['startARoute', 'beginRouteAccess', 'allServices']:
            records.clear()
            env['SERVICE'] = service
            result = subprocess.run([str(Path(__file__).with_name('run.sh').resolve()),str(fixture)],env=env,text=True,capture_output=True,timeout=90)
            assert result.returncode == 0, result.stdout + result.stderr
            endpoint = 'beginARoute' if service == 'startARoute' else service
            expected_steps = catalog[service].get('steps', [endpoint]) if service != 'startARoute' else [endpoint]
            expected_paths = [catalog[step]['path'] for step in expected_steps]
            assert len(records) == 4 * len(expected_steps), (service, len(records))
            assert set(path for path, _ in records) == set(expected_paths)
            for trace in {(p[0] if isinstance(p, list) else p)['trace_id'] for _, p in records}:
                assert [path for path, p in records if (p[0] if isinstance(p, list) else p)['trace_id'] == trace] == expected_paths
            if 'executeAnAccessOfAUserInABus' in expected_steps:
                passenger_ids = [p['user_id'] for path, p in records if path.endswith('executeAnAccessOfAUserInABus')]
                assert len(passenger_ids) == len(set(passenger_ids)) == 4
            if service in ('executeAnAccessOfAUserInABus', 'beginRouteAccess'):
                from datetime import datetime
                from zoneinfo import ZoneInfo
                for trace in {(p[0] if isinstance(p, list) else p)['trace_id'] for _, p in records}:
                    events = [p for _, p in records if (p[0] if isinstance(p, list) else p)['trace_id'] == trace]
                    begin, access = events[:2]
                    assert len({p['device_id'] for p in events}) == 1
                    assert len({p['route_id'] for p in events}) == 1
                    local_access = datetime.fromtimestamp(access['accessDate'] / 1000, ZoneInfo('America/New_York')).strftime('%Y-%m-%d %H:%M:%S')
                    assert begin['traffic_date'] < local_access
                    if len(events) == 3:
                        assert local_access < events[2]['traffic_date']
            if service in ('saveDeviceTrafficAttendance', 'saveTrafficAttendance'):
                assert 'queue_accepted' in result.stdout, result.stdout
                assert 'database_saved' not in result.stdout, 'Queued response must not count as a database save'
                assert all(path.startswith('/api/Device/') for path, _ in records)
                if service == 'saveTrafficAttendance':
                    assert all(p['traffic_type'] == 1 and p['activity_id'] == 1 and p['place_id'] == 1 for _, p in records)
                    assert all('datetime' not in p for _, p in records)
                else:
                    assert all(p['traffic_type_id'] == 21 and 'traffic_attendance_date' in p for _, p in records)
            if service == 'bulkUserAccess':
                assert all(isinstance(batch, list) and len(batch) == 2 for _, batch in records)
                assert all(len({p['user_id'] for p in batch}) == 2 for _, batch in records)
                assert all(p['traffic_type'] == 1 and p['access_type'] == 3 for _, batch in records for p in batch)
                assert 'queue_accepted' in result.stdout and 'database_saved' not in result.stdout
            if service == 'deviceStatus':
                assert all('device_imei' in payload and 'device_location' in payload and 'device_status' in payload for _,payload in records)
            assert len({(payload[0] if isinstance(payload, list) else payload)['trace_id'] for _,payload in records}) == 4
            assert 'service_iterations_completed' in result.stdout
            assert 'journeys_completed' not in result.stdout
            print(f'PASS: {service}')
        original_fixture = fixture.read_text()
        template = json.loads(original_fixture)[0]
        fixture.write_text(json.dumps([dict(template, device_id=i, passengers=[dict(passenger_id=10, access_type=3)]) for i in range(1, 9)]))
        env.update(VUS='8', JOURNEYS='100', MAX_DURATION='90s')
        for service in ('beginRouteAccess', 'allServices'):
            records.clear()
            env['SERVICE'] = service
            result = subprocess.run([str(Path(__file__).with_name('run.sh').resolve()), str(fixture)], env=env, text=True, capture_output=True, timeout=90)
            assert result.returncode == 0, result.stdout + result.stderr
            assert len(records) == 800 * len(catalog[service]['steps']), (service, len(records), 800 * len(catalog[service]['steps']), result.stdout[-3000:], result.stderr[-1000:])
            assert len({(p[0] if isinstance(p, list) else p)['trace_id'] for _, p in records}) == 800
            passenger_events = [p for path, p in records if path.endswith('executeAnAccessOfAUserInABus')]
            assert len(passenger_events) == 800
            assert {p['user_id'] for p in passenger_events} == {10}
            assert len({(p['device_id'], p['accessDate']) for p in passenger_events}) == 800
            print(f'PASS: {service} with 8 devices × 100 repetitions and one shared passenger')
        fixture.write_text(original_fixture)
        env.update(VUS='2', JOURNEYS='2')
        records.clear()
        env.update(SERVICE='beginRouteAccess', JOURNEYS='3')
        result = subprocess.run([str(Path(__file__).with_name('run.sh').resolve()),str(fixture)],env=env,text=True,capture_output=True,timeout=90)
        assert result.returncode == 0, result.stdout + result.stderr
        assert len(records) == 12
        assert len({(p[0] if isinstance(p, list) else p)['trace_id'] for _, p in records}) == 6
        env['JOURNEYS'] = '2'
        print('PASS: passenger pools cycle for repeated events without exhaustion')
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
        env.update(SERVICE='bulkUserAccess', VUS='1', JOURNEYS='100', MAX_DURATION='1s')
        env['API_BASE_URL'] += '/school_bus_cloud'
        result = subprocess.run([str(Path(__file__).with_name('run.sh').resolve()), str(fixture)], env=env, text=True, capture_output=True, timeout=90)
        assert result.returncode != 0, 'A run that drops iterations must fail even when all HTTP requests succeeded'
        assert records and all(path == '/api/DeviceAccess/bulkUserAccess' for path, _ in records)
        assert len(records) < 100
        assert 'service_iterations_completed' in result.stdout
        print('PASS: incomplete bulk runs fail completion threshold; prefixed bases use API root')
        env['API_BASE_URL'] = env['API_BASE_URL'].removesuffix('/school_bus_cloud')
        env.update(JOURNEYS='2', MAX_DURATION='30s')
        records.clear()
        fixture.write_text(json.dumps([dict(device_id=1)]))
        env['SERVICE'] = 'allServices'
        result = subprocess.run([str(Path(__file__).with_name('run.sh').resolve()),str(fixture)],env=env,text=True,capture_output=True,timeout=90)
        assert result.returncode != 0 and not records
        print('PASS: invalid fixtures rejected before requests')



finally:
    server.shutdown()
