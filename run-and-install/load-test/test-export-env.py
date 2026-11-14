"""Credential parsing regressions; no database connections or real secrets."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from runtime_config import load_runtime_config
import json

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
