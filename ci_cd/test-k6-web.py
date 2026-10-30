"""Verify Jenkins runner migration without changing Docker or systemd."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent


class DeploymentTests(unittest.TestCase):
    def test_docker_runner_mounts_and_migration(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'ci_cd').mkdir()
            shutil.copy(ROOT / 'k6-web.sh', root / 'ci_cd/k6-web.sh')
            (root / '.k6-web').mkdir()
            (root / '.k6-web/access-token').write_text('test-access-token-never-log')
            binaries = root / 'bin'
            binaries.mkdir()
            mock = '''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
args=sys.argv[1:]
with open(os.environ['CALLS'], 'a') as stream:
    stream.write(json.dumps([Path(sys.argv[0]).name] + args) + '\\n')
if Path(sys.argv[0]).name == 'stat': print('999')
if Path(sys.argv[0]).name == 'docker':
    if args[:2] == ['container', 'inspect']: sys.exit(1)
    if args[0] == 'inspect': print('running healthy')
'''
            for name in ('docker', 'sudo', 'systemctl', 'stat'):
                path = binaries / name
                path.write_text(mock)
                path.chmod(0o755)
            log = root / 'calls'
            result = subprocess.run(['bash', str(root / 'ci_cd/k6-web.sh'), 'start', 'dev.example'],
                                    env=dict(os.environ, PATH=str(binaries) + ':' + os.environ['PATH'], CALLS=str(log)),
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            run = next(c for c in calls if c[:3] == ['docker', 'run', '-d'])
            self.assertIn('5665:5665', run)
            self.assertIn('unless-stopped', run)
            self.assertIn('type=bind,src={0},dst={0},readonly'.format(root), run)
            results = str(root / '.k6-web/results')
            self.assertIn('type=bind,src={0},dst={0}'.format(results), run)
            self.assertIn('TMPDIR=' + results, run)
            self.assertIn('type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock', run)
            self.assertNotIn('test-access-token-never-log', log.read_text() + result.stdout)
            check = next(i for i, c in enumerate(calls) if '--check-config' in c)
            migration = calls.index(['sudo', '-n', 'systemctl', 'disable', '--now', 'driver-app-k6-web.service'])
            self.assertLess(check, migration)
            self.assertLess(migration, calls.index(run))


if __name__ == '__main__':
    unittest.main()
