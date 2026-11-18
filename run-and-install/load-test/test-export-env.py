"""Credential parsing regressions; no database connections or real secrets."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from runtime_config import load_runtime_config
import json
import sqlite3

spec = importlib.util.spec_from_file_location('export_devices', Path(__file__).with_name('export-devices.py'))
exporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(exporter)


class EnvTests(unittest.TestCase):
    def test_vault_configuration_preserves_raw_credentials_and_ignores_stale_env(self):
        secret = "fixture#pass$word'\\\""
        values = dict(SCHOOL_BUS_DATASOURCE_URL='jdbc:mysql://dev-db:3307/school_bus?useSSL=false',
                      SCHOOL_BUS_DATASOURCE_USERNAME='vault-user', SCHOOL_BUS_DATASOURCE_PASSWORD=secret,
                      REAXIUM_API_USER='api-user', REAXIUM_API_PASSWORD='api-password')
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory, 'runtime.json')
            path.write_text(json.dumps(values))
            with patch.dict('os.environ', {'DB_USER': 'root', 'DB_PASSWORD': 'stale'}):
                config = load_runtime_config(path)
            self.assertEqual(config['DB_USER'], 'vault-user')
            self.assertEqual(config['DB_PASSWORD'], secret)
            self.assertEqual((config['DB_HOST'], config['DB_PORT'], config['DB_NAME']), ('dev-db', '3307', 'school_bus'))
            values['SCHOOL_BUS_DATASOURCE_URL'] = 'private-invalid-url'
            path.write_text(json.dumps(values))
            with self.assertRaises(ValueError) as error:
                load_runtime_config(path)
            self.assertNotIn('private-invalid-url', str(error.exception))
            del values['SCHOOL_BUS_DATASOURCE_PASSWORD']
            path.write_text(json.dumps(values))
            with self.assertRaises(ValueError):
                load_runtime_config(path)

    def load(self, text, environment=None):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory, '.env')
            path.write_text(text)
            with patch.dict('os.environ', environment or {}, clear=True):
                return exporter.read_env(path)

    def test_embedded_hash_is_not_a_comment(self):
        self.assertEqual(self.load('DB_PASSWORD=abc#def # comment')['DB_PASSWORD'], 'abc#def')

    def test_local_shared_datasource_variables(self):
        config = self.load("SCHOOL_BUS_DATASOURCE_URL=jdbc:mysql://127.0.0.1:3308/school?useSSL=false\n"
                           "SCHOOL_BUS_DATASOURCE_USERNAME=local-user\n"
                           "SCHOOL_BUS_DATASOURCE_PASSWORD='local#pass$word'\n")
        self.assertEqual(config, dict(DB_HOST='127.0.0.1', DB_PORT='3308', DB_NAME='school',
                                      DB_USER='local-user', DB_PASSWORD='local#pass$word'))

    def test_shared_environment_overrides_shared_file(self):
        config = self.load('SCHOOL_BUS_DATASOURCE_USERNAME=file-user\n',
                           {'SCHOOL_BUS_DATASOURCE_USERNAME': 'runtime-user',
                            'SCHOOL_BUS_DATASOURCE_PASSWORD': 'raw#pass$word'})
        self.assertEqual(config['DB_USER'], 'runtime-user')
        self.assertEqual(config['DB_PASSWORD'], 'raw#pass$word')

    def test_native_settings_take_precedence_like_api(self):
        config = self.load('DB_USER=native\nDB_HOST=tunnel\nDB_PORT=3310\nDB_NAME=copy\n'
                           'DB_PASSWORD=native-pass\nSCHOOL_BUS_DATASOURCE_URL=jdbc:mysql://remote/original\n'
                           'SCHOOL_BUS_DATASOURCE_USERNAME=shared\nSCHOOL_BUS_DATASOURCE_PASSWORD=shared-pass\n')
        self.assertEqual(config, dict(DB_USER='native', DB_HOST='tunnel', DB_PORT='3310',
                                      DB_NAME='copy', DB_PASSWORD='native-pass'))

    def test_environment_only_without_env_file(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict('os.environ', {
                'SCHOOL_BUS_DATASOURCE_URL': 'jdbc:mysql://localhost/school',
                'SCHOOL_BUS_DATASOURCE_USERNAME': 'runtime-user',
                'SCHOOL_BUS_DATASOURCE_PASSWORD': 'runtime-pass'}, clear=True):
            config = exporter.read_env(Path(directory, 'missing.env'))
        self.assertEqual(config['DB_USER'], 'runtime-user')
        self.assertEqual(config['DB_PORT'], '3306')

    def test_quotes_preserve_literal_password_and_spacing(self):
        for value in ("'abc#def$word'", '"abc#def\\$word"'):
            self.assertEqual(self.load('DB_PASSWORD=' + value)['DB_PASSWORD'], 'abc#def$word')
        self.assertEqual(self.load("DB_PASSWORD='two  spaces'")['DB_PASSWORD'], 'two  spaces')

    def test_interpolation_and_environment_precedence(self):
        result = self.load('PREFIX=fixture\nDB_PASSWORD=${PREFIX}#suffix\nDB_USER=file\n', {'DB_USER': 'runtime', 'DB_HOST': 'db'})
        self.assertEqual(result['DB_PASSWORD'], 'fixture#suffix')
        self.assertEqual(result['DB_USER'], 'runtime')
        self.assertEqual(result['DB_HOST'], 'db')

    def test_export_empty_and_duplicate_values(self):
        result = self.load('export DB_USER = first\nDB_USER=second\nDB_PASSWORD= # empty\n')
        self.assertEqual(result, {'DB_USER': 'first', 'DB_PASSWORD': ''})

    def test_invalid_values_do_not_expose_secrets(self):
        with self.assertRaises(ValueError) as caught:
            self.load("DB_PASSWORD='private-secret")
        self.assertNotIn('private-secret', str(caught.exception))

    def test_attendance_export_matches_business_and_preserves_unmatched_devices(self):
        with tempfile.TemporaryDirectory() as folder:
            output = Path(folder, 'devices.json')
            rows = [dict(device_id=1, business_id=7, route_id=1), dict(device_id=2, business_id=8, route_id=2)]
            context = dict(business_id=7, attendance_user_id=10, attendance_activity_id=4,
                           attendance_place_id=5, attendance_access_type=1,
                           attendance_traffic_type=1, attendance_device_type=21)
            results = ['\n'.join(map(json.dumps, rows)), '', json.dumps(context)]
            args = ['export-devices.py', '--attendance', '--output', str(output)]
            with patch('sys.argv', args), patch.object(exporter, 'read_env', return_value={}), \
                 patch.object(exporter, 'query', side_effect=results) as query:
                exporter.main()
            exported = json.loads(output.read_text())
            self.assertEqual(exported[0]['attendance_activity_id'], 4)
            self.assertNotIn('attendance_activity_id', exported[1])
            self.assertIn('activities_users', query.call_args_list[-1].args[1])
            self.assertTrue(all('INSERT' not in call.args[1] for call in query.call_args_list))

    def test_attendance_reference_selection_without_assignments_or_access_methods(self):
        # This SELECT uses portable CTE/window/JSON syntax; execute it against
        # fixture tables to catch accidental INNER JOIN gates, without a live DB.
        db = sqlite3.connect(':memory:')
        self.addCleanup(db.close)
        db.executescript("""
            CREATE TABLE users (user_id INT, business_id INT, status_id INT);
            CREATE TABLE activities_users (user_id INT, activity_id INT, place_id INT);
            CREATE TABLE activities (activity_id INT);
            CREATE TABLE places (place_id INT);
            CREATE TABLE business_activities (business_id INT, activity_id INT);
            CREATE TABLE business_places (business_id INT, place_id INT);
            CREATE TABLE user_access_data (user_id INT, access_type_id INT);
            CREATE TABLE traffic_type (traffic_type_id INT);
            INSERT INTO users VALUES (6, 7, 1), (9, 7, 1);
            INSERT INTO activities VALUES (4), (8);
            INSERT INTO places VALUES (5), (9);
            INSERT INTO traffic_type VALUES (21);
        """)
        sql = Path(__file__).with_name('export-attendance.sql').read_text()
        context = json.loads(db.execute(sql).fetchone()[0])
        self.assertEqual(context['attendance_user_id'], 6)
        self.assertEqual(context['attendance_activity_id'], 4)
        self.assertEqual(context['attendance_place_id'], 5)
        self.assertEqual(context['attendance_device_type'], 21)
        self.assertIsNone(context['attendance_access_type'])
        self.assertIsNone(context['attendance_traffic_type'])
        db.executescript("""
            INSERT INTO user_access_data VALUES (9, 2);
            INSERT INTO business_activities VALUES (7, 8);
            INSERT INTO business_places VALUES (7, 9);
            INSERT INTO traffic_type VALUES (1);
        """)
        context = json.loads(db.execute(sql).fetchone()[0])
        self.assertEqual(context['attendance_user_id'], 9)
        self.assertEqual(context['attendance_access_type'], 2)
        self.assertEqual(context['attendance_activity_id'], 8)
        self.assertEqual(context['attendance_place_id'], 9)
        self.assertEqual(context['attendance_traffic_type'], 1)

    def test_mysql_receives_full_password_outside_command_arguments(self):
        config = self.load('DB_HOST=localhost\nDB_USER=root\nDB_NAME=test\nDB_PASSWORD=abc#def\n')
        with patch.object(exporter.subprocess, 'run') as run:
            run.return_value.returncode = 0
            run.return_value.stdout = ''
            exporter.query(config, 'SELECT 1')
        self.assertEqual(run.call_args.kwargs['env']['MYSQL_PWD'], 'abc#def')
        self.assertNotIn('abc#def', ' '.join(run.call_args.args[0]))


if __name__ == '__main__':
    unittest.main()
