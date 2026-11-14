#!/usr/bin/env python3
"""Prepare a private, validated dataset for one run; database queries are SELECT-only."""
import argparse
import itertools
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from datetime import datetime, timedelta
from zoneinfo import ZoneInfo

ROOT = Path(__file__).resolve().parent
ACCESS = 'executeAnAccessOfAUserInABus'


def positive(value):
    try:
        return float(value).is_integer() and float(value) > 0
    except (ValueError, TypeError, OverflowError):
        return False


def validate(device, steps):
    for key in ('device_id', 'route_id', 'driver_id', 'bus_info', 'start_stop_id', 'end_stop_id'):
        if not positive(device.get(key)):
            raise ValueError('%s must be a positive ID' % key)
    for key, limit in (('start_latitude', 90), ('end_latitude', 90), ('start_longitude', 180), ('end_longitude', 180)):
        value = device.get(key)
        if value is None or value == '' or not math.isfinite(float(value)) or abs(float(value)) > limit:
            raise ValueError('Invalid coordinate ' + key)
    if 'deviceStatus' in steps and (not device.get('device_imei') or not positive(device.get('business_id'))):
        raise ValueError('Device status needs device_imei and business_id')
    if any('Merge' in step for step in steps):
        ids = device.get('route_ids') or []
        ids = ids if isinstance(ids, list) else str(ids).split(',')
        if len(ids) < 2 or not all(positive(i) for i in ids) or len(set(map(int, ids))) != len(ids):
            raise ValueError('Merged tests need at least two distinct assigned routes')


def allocate(rows, count):
    # Match passenger IDs to slots, allowing reassignment when route pools overlap.
    owners, assignments = {}, {}
    pools = []
    for d in rows:
        pool = d.get('passengers', [{'passenger_id': d.get('passenger_id'), 'access_type': d.get('access_type')}])
        pools.append({int(p['passenger_id']): p for p in pool if positive(p.get('passenger_id')) and positive(p.get('access_type'))})

    def match(slot, seen):
        for user_id in pools[slot[0]]:
            if user_id in seen:
                continue
            seen.add(user_id)
            previous = owners.get(user_id)
            if previous is None or match(previous, seen):
                owners[user_id] = slot
                assignments[slot] = user_id
                return True
        return False

    for index in sorted(range(len(rows)), key=lambda i: len(pools[i])):
        if len(pools[index]) < count:
            return None
        for iteration in range(count):
            if not match((index, iteration), set()):
                return None
    return [[pools[i][assignments[(i, j)]] for j in range(count)] for i in range(len(rows))]


def select(rows, steps, vus, iterations, start):
    candidates, rejected, seen = [], [], set()
    for d in rows:
        try:
            validate(d, steps)
            device_id = int(d['device_id'])
            if device_id in seen:
                raise ValueError('Duplicate device ID')
            seen.add(device_id)
            if ACCESS in steps:
                if d.get('passenger_date') and d['passenger_date'] != start[:10]:
                    raise ValueError('Passenger date does not match simulated date')
                if allocate([d], iterations) is None:
                    raise ValueError('Needs %d unused passengers; has %d' % (iterations, len(d.get('passengers', []))))
            candidates.append(d)
        except (ValueError, TypeError, OverflowError) as error:
            rejected.append('Device %s: %s' % (d.get('device_id', '?'), error))
    if len(candidates) < vus:
        raise ValueError('Data preparation failed: need %d eligible devices, found %d. %s' % (vus, len(candidates), '; '.join(rejected)))
    if ACCESS not in steps:
        return candidates[:vus]
    # Prefer generous pools, but try other device combinations if shared passengers conflict.
    candidates.sort(key=lambda d: -len(d.get('passengers', [])))
    for attempt, subset in enumerate(itertools.combinations(candidates, vus)):
        if attempt >= 10000:
            break
        allocations = allocate(subset, iterations)
        if allocations is not None:
            result = []
            for d, passengers in zip(subset, allocations):
                result.append(dict(d, passengers=passengers, **passengers[0]))
            return result
    raise ValueError('Data preparation failed: not enough distinct unused passengers across %d devices for %d repetitions. Reduce devices/repetitions or add valid test relationships.' % (vus, iterations))


def validate_dates(steps, iterations, start):
    stamp = datetime.strptime(start, '%Y-%m-%dT%H:%M:%SZ')
    if ACCESS not in steps:
        stamp + timedelta(milliseconds=(iterations - 1) * max(120000, len(steps) * 60000) + (len(steps) - 1) * 60000)
        return
    zone = ZoneInfo('America/New_York')
    for iteration in range(iterations):
        event = stamp + timedelta(milliseconds=iteration * max(120000, len(steps) * 60000) + steps.index(ACCESS) * 60000)
        if event.year < 2007 or event.date() != stamp.date():
            raise ValueError('Passenger run must stay within the prepared date and use year 2007 or later')
        if event.replace(tzinfo=zone, fold=0).utcoffset() != event.replace(tzinfo=zone, fold=1).utcoffset():
            raise ValueError('Passenger time is ambiguous or nonexistent in America/New_York')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--env-file', type=Path, default=ROOT.parents[1] / '.env')
    parser.add_argument('--config-file', type=Path)
    parser.add_argument('--service', required=True)
    parser.add_argument('--vus', type=int, required=True)
    parser.add_argument('--iterations', type=int, required=True)
    parser.add_argument('--start', required=True)
    parser.add_argument('--offline', action='store_true', help='Validate supplied fixtures only; for local mock tests')
    args = parser.parse_args()
    catalog = json.loads((ROOT / 'services.json').read_text())
    service = 'beginARoute' if args.service == 'startARoute' else args.service
    if service not in catalog or args.vus < 1 or not 1 <= args.iterations <= 100000:
        parser.error('Choose a valid service, positive device count and 1-100000 repetitions')
    steps = catalog[service].get('steps', [service])
    validate_dates(steps, args.iterations, args.start)
    print('Step 1/2: preparing and checking data for %s (%d devices, %d repetitions).' % (service, args.vus, args.iterations), flush=True)
    source = json.loads(args.source.read_text())
    if not isinstance(source, list) or not source:
        raise ValueError('Source dataset must contain devices')
    if args.offline:
        rows = source
    else:
        with tempfile.TemporaryDirectory(prefix='driver-prepare-') as directory:
            exported = Path(directory, 'devices.json')
            command = [sys.executable, str(ROOT / 'export-devices.py'), '--env-file', str(args.env_file),
                       '--passenger-date', args.start[:10], '--output', str(exported)]
            if args.config_file:
                command.extend(['--config-file', str(args.config_file)])
            subprocess.run(command, check=True, timeout=180)
            allowed = {int(d['device_id']) for d in source}
            rows = [d for d in json.loads(exported.read_text()) if int(d['device_id']) in allowed]
    selected = select(rows, steps, args.vus, args.iterations, args.start)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(str(args.output), os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    os.fchmod(fd, 0o600)
    with os.fdopen(fd, 'w') as stream:
        json.dump(selected, stream)
    print('Data ready: devices %s%s. Database unchanged.' % (', '.join(str(d['device_id']) for d in selected),
          '; %d distinct passengers' % (args.vus * args.iterations) if ACCESS in steps else ''), flush=True)
    print('Step 2/2: ready for k6 execution.', flush=True)


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        raise SystemExit('Preparation stopped before k6: ' + str(error))
