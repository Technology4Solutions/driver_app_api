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
    return dict(attendance_user_id=10, attendance_activity_id=1, attendance_place_id=1, attendance_access_type=3, attendance_device_type=21, attendance_traffic_type=1, device_id=id, route_id=id, driver_id=1, bus_info='1', start_stop_id=1,
                end_stop_id=2, start_latitude=27, start_longitude=-80, end_latitude=27,
                end_longitude=-80, device_imei='mock', business_id=1, route_ids='1,2',
                passenger_date='2026-11-14',
                passengers=[dict(passenger_id=p, access_type=3) for p in passengers])


class PreparationTests(unittest.TestCase):
    def test_online_run_uses_all_eight_fresh_devices_despite_stale_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, output = root / 'source.json', root / 'prepared.json'
            old_ids = [699, 890, 931, 939, 940, 943, 950, 186]
            fresh_ids = [699, 757, 880, 890, 899, 939, 943, 961]
            source.write_text(json.dumps([device(id, [id]) for id in old_ids]))
            original_source = source.read_bytes()
            fresh_rows = [device(id, [id + 10000]) for id in fresh_ids]

            def export(command, **kwargs):
                Path(command[command.index('--output') + 1]).write_text(json.dumps(fresh_rows))

            args = ['prepare-run.py', '--source', str(source), '--output', str(output),
                    '--service', 'beginRouteAccess', '--vus', '8', '--iterations', '1',
                    '--start', '2026-11-14T08:00:00Z']
            with patch.object(sys, 'argv', args), patch.object(prepare.subprocess, 'run', side_effect=export):
                prepare.main()
            selected = json.loads(output.read_text())
            self.assertEqual({row['device_id'] for row in selected}, set(fresh_ids))
            self.assertEqual(len({row['passenger_id'] for row in selected}), 8)
            self.assertEqual(source.read_bytes(), original_source)

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
        self.assertEqual({d['device_id'] for d in result}, {1, 2})
        self.assertEqual(len(result), 2)

    def test_shared_passengers_are_reassigned(self):
        rows = [device(1, [10, 11]), device(2, [10])]
        result = prepare.select(rows, [prepare.ACCESS], 2, 1, '2026-11-14')
        self.assertEqual(len(result), 2)

    def test_eight_devices_can_repeat_one_passenger_100_times_for_every_service(self):
        rows = [device(i, [10]) for i in range(1, 9)]
        catalog = json.loads((Path(__file__).parent / 'services.json').read_text())
        for name, descriptor in catalog.items():
            steps = descriptor.get('steps', [name])
            result = prepare.select(rows, steps, 8, 100, '2026-11-14')
            self.assertEqual(len(result), 8)
            prepare.validate_dates(steps, 100, '2026-11-14T08:00:00Z')
        assigned = prepare.allocate(rows, 100)
        self.assertTrue(all(len(pool) == 100 for pool in assigned))
        self.assertEqual({p['passenger_id'] for pool in assigned for p in pool}, {10})
        with self.assertRaisesRegex(ValueError, 'at least one valid'):
            prepare.select([device(1, [])], [prepare.ACCESS], 1, 100, '2026-11-14')

    def test_attendance_preflight_requires_reference_data(self):
        for action in ('saveDeviceTrafficAttendance', 'saveTrafficAttendance'):
            row = device(1, [])
            del row['attendance_place_id']
            with self.assertRaisesRegex(ValueError, 'attendance_place_id'):
                prepare.select([row], [action], 1, 1, '2026-11-14')
        row = device(1, [])
        row['attendance_traffic_type'] = 1001
        with self.assertRaisesRegex(ValueError, 'traffic type'):
            prepare.select([row], ['saveTrafficAttendance'], 1, 1, '2026-11-14')

    def test_device_attendance_does_not_require_participant_or_access_reference(self):
        row = device(1, [])
        for key in ('attendance_user_id', 'attendance_access_type', 'attendance_traffic_type'):
            row.pop(key)
        self.assertEqual(len(prepare.select([row], ['saveDeviceTrafficAttendance'], 1, 1, '2026-11-14')), 1)
        with self.assertRaisesRegex(ValueError, 'active user'):
            prepare.select([row], ['saveTrafficAttendance'], 1, 1, '2026-11-14')
        row['attendance_user_id'] = 10
        with self.assertRaisesRegex(ValueError, 'access type'):
            prepare.select([row], ['saveTrafficAttendance'], 1, 1, '2026-11-14')

    def test_bulk_passenger_preparation_and_dates(self):
        rows = [device(1, [10, 11])]
        result = prepare.select(rows, [prepare.BULK], 1, 10, '2026-11-14')
        self.assertEqual([p['passenger_id'] for p in result[0]['passengers']], [10, 11])
        with self.assertRaisesRegex(ValueError, 'at least one valid'):
            prepare.select([device(1, [])], [prepare.BULK], 1, 1, '2026-11-14')
        with self.assertRaisesRegex(ValueError, 'date'):
            prepare.select(rows, [prepare.BULK], 1, 1, '2026-11-15')
        for stamp in ('2026-03-08T02:30:00Z', '2026-11-01T01:30:00Z'):
            with self.assertRaisesRegex(ValueError, 'ambiguous or nonexistent'):
                prepare.validate_dates([prepare.BULK], 1, stamp)

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
        for stamp in ('2026-03-08T02:00:00Z', '2026-11-01T01:00:00Z'):
            with self.assertRaises(ValueError):
                prepare.validate_dates(['beginARoute', prepare.ACCESS], 2, stamp)


if __name__ == '__main__':
    unittest.main()
