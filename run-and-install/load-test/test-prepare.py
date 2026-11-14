"""Data selection tests; no database, Docker, or API required."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch
import json
import sys
import tempfile

spec = importlib.util.spec_from_file_location('prepare_run', Path(__file__).with_name('prepare-run.py'))
prepare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare)


def device(id, passengers):
    return dict(device_id=id, route_id=id, driver_id=1, bus_info='1', start_stop_id=1,
                end_stop_id=2, start_latitude=27, start_longitude=-80, end_latitude=27,
                end_longitude=-80, device_imei='mock', business_id=1, route_ids='1,2',
                passenger_date='2026-11-14',
                passengers=[dict(passenger_id=p, access_type=3) for p in passengers])


class PreparationTests(unittest.TestCase):
    def test_cli_forwards_vault_file_to_exporter(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, output, config = (root / name for name in ('source.json', 'prepared.json', 'vault.json'))
            source.write_text(json.dumps([device(1, [10])]))
            config.write_text('{}')

            def export(command, **kwargs):
                self.assertEqual(command[command.index('--config-file') + 1], str(config))
                Path(command[command.index('--output') + 1]).write_text(source.read_text())

            args = ['prepare-run.py', '--source', str(source), '--output', str(output),
                    '--config-file', str(config), '--service', 'beginRouteAccess', '--vus', '1',
                    '--iterations', '1', '--start', '2026-11-14T08:00:00Z']
            with patch.object(sys, 'argv', args), patch.object(prepare.subprocess, 'run', side_effect=export):
                prepare.main()
            self.assertEqual(json.loads(output.read_text())[0]['passenger_id'], 10)

    def test_selects_devices_with_enough_passengers(self):
        rows = [device(1, [10, 11]), device(2, [20, 21, 22]), device(3, [30, 31, 32])]
        result = prepare.select(rows, [prepare.ACCESS], 2, 3, '2026-11-14')
        self.assertEqual({d['device_id'] for d in result}, {2, 3})
        self.assertEqual(len({p['passenger_id'] for d in result for p in d['passengers']}), 6)

    def test_shared_passengers_are_reassigned(self):
        rows = [device(1, [10, 11]), device(2, [10])]
        result = prepare.select(rows, [prepare.ACCESS], 2, 1, '2026-11-14')
        self.assertEqual(len({d['passenger_id'] for d in result}), 2)

    def test_insufficient_unique_passengers_are_rejected(self):
        with self.assertRaisesRegex(ValueError, 'distinct unused passengers'):
            prepare.select([device(1, [10]), device(2, [10])], [prepare.ACCESS], 2, 1, '2026-11-14')

    def test_other_services_do_not_require_passengers(self):
        self.assertEqual(len(prepare.select([device(1, [])], ['beginARoute'], 1, 100, '2026-11-14')), 1)

    def test_service_specific_invalid_devices_are_skipped(self):
        bad = device(1, [])
        bad['route_ids'] = '1'
        result = prepare.select([bad, device(2, [])], ['beginAMergeRoute'], 1, 1, '2026-11-14')
        self.assertEqual(result[0]['device_id'], 2)
        bad['device_imei'] = ''
        with self.assertRaisesRegex(ValueError, 'device_imei'):
            prepare.select([bad], ['deviceStatus'], 1, 1, '2026-11-14')

    def test_invalid_coordinates_and_stale_dates_rejected(self):
        bad = device(1, [10])
        bad['start_latitude'] = None
        with self.assertRaisesRegex(ValueError, 'coordinate'):
            prepare.select([bad], ['beginARoute'], 1, 1, '2026-11-14')
        with self.assertRaisesRegex(ValueError, 'date'):
            prepare.select([device(1, [10])], [prepare.ACCESS], 1, 1, '2026-11-15')

    def test_date_validation(self):
        prepare.validate_dates(['beginARoute', prepare.ACCESS], 3, '2026-11-14T08:00:00Z')
        for stamp in ('2026-03-08T02:00:00Z', '2026-11-01T01:00:00Z', '2026-11-14T23:59:00Z'):
            with self.assertRaises(ValueError):
                prepare.validate_dates(['beginARoute', prepare.ACCESS], 2, stamp)


if __name__ == '__main__':
    unittest.main()
