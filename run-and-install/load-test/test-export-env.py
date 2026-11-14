"""Credential parsing regressions; no database connections or real secrets."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('export_devices', Path(__file__).with_name('export-devices.py'))
exporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(exporter)


class EnvTests(unittest.TestCase):
    def load(self, text, environment=None):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory, '.env')
            path.write_text(text)
            with patch.dict('os.environ', environment or {}, clear=True):
                return exporter.read_env(path)

    def test_embedded_hash_is_not_a_comment(self):
        self.assertEqual(self.load('DB_PASSWORD=abc#def # comment')['DB_PASSWORD'], 'abc#def')

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
