#!/usr/bin/env python3
"""Provision project Grafana alerts; store the Slack webhook only in a local env file."""
import getpass
import json
import os
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parent.parent


def configuration():
    rules = []
    specs = [
        ('down', 'API unavailable', 'up{job="device-api"}', 'lt', 1, '1m', 'critical'),
        ('latency', 'Slow API responses', 'histogram_quantile(0.95, sum by (le, route) (rate(http_request_duration_seconds_bucket{job="device-api"}[5m])))', 'gt', 1, '5m', 'warning'),
        ('cpu', 'High API CPU', '100 * rate(process_cpu_seconds_total{job="device-api"}[5m])', 'gt', 80, '5m', 'warning'),
        ('memory', 'High API resident memory', 'process_resident_memory_bytes{job="device-api"}', 'gt', 750 * 1024 * 1024, '5m', 'warning'),
        ('errors', 'High HTTP server error rate', '(sum(rate(http_requests_total{job="device-api",status=~"5.."}[5m])) or vector(0)) / clamp_min(sum(rate(http_requests_total{job="device-api"}[5m])), 0.001)', 'gt', 0.05, '5m', 'critical'),
    ]
    for uid, title, expr, operation, threshold, pending, severity in specs:
        rules.append(dict(
            uid='device-api-' + uid, title=title, condition='C',
            data=[
                dict(refId='A', datasourceUid='prometheus', relativeTimeRange=dict(from_=600, to=0),
                     model=dict(refId='A', expr=expr, instant=True, range=False, intervalMs=1000, maxDataPoints=43200)),
                dict(refId='C', datasourceUid='__expr__', relativeTimeRange=dict(from_=0, to=0),
                     model=dict(refId='C', type='threshold', expression='A',
                                conditions=[dict(evaluator=dict(type=operation, params=[threshold]),
                                                 operator=dict(type='and'), query=dict(params=['C']),
                                                 reducer=dict(type='last', params=[]), type='query')]))
            ],
            noDataState='Alerting' if uid == 'down' else 'OK', execErrState='Error',
            **{'for': pending}, annotations=dict(summary=title),
            labels=dict(service='device-api', severity=severity), isPaused=False,
            notification_settings=dict(receiver='Device API Slack', group_by=['grafana_folder', 'alertname'],
                                       group_wait='30s', group_interval='5m', repeat_interval='4h')))
    for rule in rules:
        for query in rule['data']:
            query['relativeTimeRange']['from'] = query['relativeTimeRange'].pop('from_')
    return dict(apiVersion=1,
                contactPoints=[dict(orgId=1, name='Device API Slack', receivers=[dict(
                    uid='device-api-slack', type='slack', disableResolveMessage=False,
                    settings=dict(url='$SLACK_WEBHOOK_URL'))])],
                groups=[dict(orgId=1, name='Device API alerts', folder='Device API', interval='1m', rules=rules)])


def main():
    webhook = os.environ.get('SLACK_WEBHOOK_URL') or getpass.getpass('Slack incoming webhook URL (hidden): ')
    url = urlsplit(webhook)
    if (url.scheme != 'https' or url.netloc not in ('hooks.slack.com', 'hooks.slack-gov.com')
            or not url.path.startswith('/services/') or any(c.isspace() for c in webhook)
            or '$' in webhook or url.query or url.fragment):
        raise SystemExit('Expected a Slack HTTPS incoming webhook URL under /services/.')
    secret = ROOT / 'monitoring/slack.local.env'
    # Restrict access before writing, including when updating an existing file.
    fd = os.open(secret, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    os.fchmod(fd, 0o600)
    with os.fdopen(fd, 'w') as output:
        output.write('SLACK_WEBHOOK_URL=' + webhook + '\n')
    target = ROOT / 'monitoring/grafana/alerting/device-api.local.json'
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(configuration(), indent=2) + '\n')
    print('Created five Grafana alerts and the Slack contact point. No notification was sent.')
    print('Apply locally: ./run-and-install/run.sh')
    print('Apply on server: bash ci_cd/monitoring.sh start')


if __name__ == '__main__':
    main()
