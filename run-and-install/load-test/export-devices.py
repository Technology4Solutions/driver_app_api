#!/usr/bin/env python3
"""Export one related device/bus/driver/route combination per device; SELECT only."""
import argparse
from datetime import date, datetime
from zoneinfo import ZoneInfo
import json
import os
from pathlib import Path
import shlex
import subprocess

ROOT = Path(__file__).resolve().parents[2]


def read_env(path):
    values = {}
    for line in path.read_text().splitlines():
        if not line.strip() or line.lstrip().startswith('#'):
            continue
        key, sep, value = line.partition('=')
        if not sep or not key.strip().removeprefix('export ').startswith('DB_'):
            continue
        parts = shlex.split(value, comments=True)
        values[key.strip().removeprefix('export ')] = ' '.join(parts)
    return values


def query(config, sql):
    env = os.environ.copy()
    env['MYSQL_PWD'] = config['DB_PASSWORD']
    args = ['mysql', '--protocol=TCP', '--connect-timeout=10', '--batch', '--raw', '--skip-column-names',
            '-h', config['DB_HOST'], '-P', config.get('DB_PORT', '3306'),
            '-u', config['DB_USER'], '-D', config['DB_NAME']]
    result = subprocess.run(args, input=sql, env=env, text=True, capture_output=True, timeout=120)
    if result.returncode:
        raise RuntimeError(result.stderr.strip())
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--env-file', type=Path, default=ROOT / '.env')
    parser.add_argument('--passenger-date', type=date.fromisoformat, default=datetime.now(ZoneInfo('Africa/Casablanca')).date(),
                        help='Simulated test date; exclude passengers already recorded on this date')
    parser.add_argument('--limit', type=int, default=1000)
    parser.add_argument('--output', type=Path, default=ROOT / 'run-and-install/devices.local.json')
    args = parser.parse_args()
    if args.limit < 1:
        parser.error('--limit must be positive')
    config = read_env(args.env_file)
    # One candidate per device, with active relationships and a real driver.
    # Min/max stop IDs are representative linked stops, not geographic route order.
    sql = Path(__file__).with_name('export-devices.sql').read_text()
    rows = [json.loads(line) for line in query(config, sql.replace('__LIMIT__', str(args.limit))).splitlines() if line]
    if not rows:
        raise RuntimeError('No eligible combinations found; no output written. Check active device/bus/driver/route/stop relationships.')
    routes = ','.join(str(int(row['route_id'])) for row in rows)
    passenger_sql = Path(__file__).with_name('export-passengers.sql').read_text()
    passenger_sql = passenger_sql.replace('__ROUTES__', routes).replace('__DATE__', args.passenger_date.isoformat())
    passengers = [json.loads(line) for line in query(config, passenger_sql).splitlines() if line]
    for row in rows:
        row['passengers'] = [{k: passenger[k] for k in ('passenger_id', 'access_type')}
                             for passenger in passengers if passenger['route_id'] == row['route_id']]
        row['passenger_date'] = args.passenger_date.isoformat()
        row['passenger_id'] = row['passengers'][0]['passenger_id'] if row['passengers'] else None
        row['access_type'] = row['passengers'][0]['access_type'] if row['passengers'] else None
    rows.sort(key=lambda row: not row['passengers'])
    print('Eligible passenger counts per device: ' + ', '.join(str(row['device_id']) + ':' + str(len(row['passengers'])) for row in rows))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    os.fchmod(fd, 0o600)
    with os.fdopen(fd, 'w') as output:
        json.dump(rows, output, indent=2)
        output.write('\n')
    print(f'Exported {len(rows)} distinct devices to {args.output}. Database unchanged.')
    print('Use these IDs only against the matching isolated database copy.')


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.TimeoutExpired) as error:
        raise SystemExit(str(error))
