#!/usr/bin/env python3
"""Export one related device/bus/driver/route combination per device; SELECT only."""
import argparse
from datetime import date, datetime
from zoneinfo import ZoneInfo
import json
import os
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]


def env_value(raw, values):
    """Parse dotenv values, preserving embedded # characters in credentials."""
    raw = raw.strip()
    if not raw or raw.startswith('#'):
        return ''
    output, quote, index = [], None, 0
    while index < len(raw):
        char = raw[index]
        if quote == "'":
            if char == "'":
                quote = None
            else:
                output.append(char)
        elif char == '\\':
            index += 1
            if index >= len(raw):
                raise ValueError('Incomplete escape')
            escaped = raw[index]
            if escaped not in "\\'\"$ n":
                raise ValueError('Invalid escape')
            output.append('\n' if escaped == 'n' else escaped)
        elif char == '$':
            # dotenvy expands variables outside single quotes; never run shell code.
            match = re.match(r'\$\{([^}]+)\}|\$([a-zA-Z0-9]+)', raw[index:])
            if match:
                name = match.group(1) or match.group(2)
                output.append(os.environ.get(name, values.get(name, '')))
                index += len(match.group()) - 1
            elif raw[index:].startswith('${'):
                raise ValueError('Incomplete variable reference')
        elif char == quote:
            quote = None
        elif quote == '"':
            output.append(char)
        elif char in "'\"":
            quote = char
        elif char in ' \t':
            tail = raw[index:].lstrip()
            if tail and not tail.startswith('#'):
                raise ValueError('Unquoted whitespace')
            break
        else:
            output.append(char)
        index += 1
    if quote:
        raise ValueError('Unclosed quote')
    return ''.join(output)


def read_env(path):
    values = {}
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if not line.strip() or line.lstrip().startswith('#'):
            continue
        key, sep, value = line.partition('=')
        key = key.strip().removeprefix('export ').strip()
        if not sep:
            continue
        try:
            parsed = env_value(value, values)
        except ValueError:
            # Never include the source line: it may contain a password.
            raise ValueError('Invalid .env value for %s at line %d' % (key, number)) from None
        values.setdefault(key, parsed)
    values.update({key: value for key, value in os.environ.items() if key.startswith('DB_')})
    return {key: value for key, value in values.items() if key.startswith('DB_')}


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
