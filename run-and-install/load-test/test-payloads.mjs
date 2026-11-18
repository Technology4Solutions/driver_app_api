// Pure checks run without k6 or an API. Import as a data URL so no package mode is required.
import {readFileSync} from 'node:fs';
import assert from 'node:assert/strict';
const root = new URL('./', import.meta.url);
const {payload, validateDevice, databaseSaved, accessTimestamp, allocatePassengers} = await import('data:text/javascript;base64,' + Buffer.from(readFileSync(new URL('payloads.js', root))).toString('base64'));
const catalog = JSON.parse(readFileSync(new URL('services.json', root)));
const fixture = {attendance_user_id: 10, attendance_activity_id: 1, attendance_place_id: 1, attendance_access_type: 3, attendance_device_type: 21, attendance_traffic_type: 1, device_id: 1, route_id: 2, route_ids: '2,3', driver_id: 4, passenger_id: 5, access_type: 3, bus_info: '6', device_imei: 'mock', business_id: 1,
  start_stop_id: 7, end_stop_id: 8, start_latitude: 27, start_longitude: -80, end_latitude: 28, end_longitude: -81};
const steps = catalog.allServices.steps;
assert.equal(steps.length, 16);
for (const service of steps) {
  validateDevice(fixture, [service]);
  const request = payload(service, fixture, Date.parse('2026-11-09T08:00:00Z'), 'load-test');
  if (Array.isArray(request)) {assert(request.length > 0); assert(request.every(p => p.trace_id === 'load-test')); continue;}
  assert.equal(request.trace_id, 'load-test');
  if (catalog[service].category !== 'Attendance') assert.equal(request.bus_info ?? request.device_status.bus_info, '6');
  if (service.includes('Merge')) {assert.equal(request.route_ids, '2,3'); assert.equal(request.route_id, undefined);}
  if (service === 'executeAnAccessOfAUserInABus') {assert.equal(request.user_id, 5); assert.equal(request.access_type, 3); assert.equal(request.accessDate, 1794229200000);}
  if (service === 'sendNextStopNotification') assert.equal(request.notification_type, 5);
  if (service === 'sendSkippedStopNotification') assert.equal(request.notification_type, 18);
  if (service === 'sendManualSkippedStopNotification') {assert.equal(request.notification_type, undefined); assert.equal(request.traffic_date, '2026-11-09 08:00:00');}
}
for (const invalid of [{...fixture,route_ids:'2'}, {...fixture,route_ids:'2,2'}, {...fixture,passenger_id:null}, {...fixture,start_latitude:null}]) assert.throws(() => validateDevice(invalid, steps));
assert(databaseSaved({code:0},2));
assert(databaseSaved({code:3,object:{database_saved:true}},2));
const saved = {response:{code:3,object:{database_saved:true}}};
assert(databaseSaved({code:3,object:{routes:[saved,saved]}},2));
assert(!databaseSaved({code:3,object:{routes:[saved]}},2));
assert(!databaseSaved({code:3,object:{routes:[saved,{response:{code:3}}]}},2));
assert(!databaseSaved(null,2));
assert(!databaseSaved({code:3,object:{routes:[]}},0));
// Every registered business handler has a corresponding k6 entry; legacy prefix aliases share coverage.
const routes = readFileSync(new URL('../../services/device-api/src/lib.rs', root), 'utf8');
const endpoints = [...routes.matchAll(/"(\/(?:Alarm|DeviceTraffic|UserAccess|api\/Device|api\/DeviceAccess|device\/DeviceStatus)\/[^"\n]+)"/g)].map(m => m[1]);
for (const endpoint of endpoints) assert(Object.values(catalog).some(s => s.path === endpoint), `Missing k6 service ${endpoint}`);
console.log('PASS: all 16 service payloads, dataset checks, route coverage, and merged partial saves');

assert.deepEqual(catalog.executeAnAccessOfAUserInABus.steps, ['beginARoute', 'executeAnAccessOfAUserInABus', 'endARoute']);
for (const [wall, expected] of [
  ['2026-01-21T08:00:00Z', '2026-01-21T13:00:00Z'],
  ['2026-07-21T08:00:00Z', '2026-07-21T12:00:00Z'],
  ['2026-03-08T01:59:00Z', '2026-03-08T06:59:00Z'],
  ['2026-03-08T03:00:00Z', '2026-03-08T07:00:00Z'],
  ['2026-11-01T00:59:00Z', '2026-11-01T04:59:00Z'],
  ['2026-11-01T02:00:00Z', '2026-11-01T07:00:00Z'],
]) assert.equal(accessTimestamp(Date.parse(wall)), Date.parse(expected));
for (const invalid of ['2026-03-08T02:30:00Z', '2026-11-01T01:30:00Z', '2006-01-01T08:00:00Z']) assert.throws(() => accessTimestamp(Date.parse(invalid)));
const trip = catalog.executeAnAccessOfAUserInABus.steps.map((step, index) => payload(step, fixture, Date.parse('2026-11-12T08:00:00Z') + index * 60000, 'shared-trip'));
assert(trip.every(p => p.trace_id === 'shared-trip' && p.device_id === fixture.device_id && p.route_id === fixture.route_id));
assert.equal(new Date(trip[1].accessDate).toISOString(), '2026-11-12T13:01:00.000Z');
assert.equal(trip[0].traffic_date, '2026-11-12 08:00:00');
assert.equal(trip[2].traffic_date, '2026-11-12 08:02:00');
console.log('PASS: passenger trip sequence and New York timestamp consistency');

assert.deepEqual(catalog.beginRouteAccess.steps, ['beginARoute', 'executeAnAccessOfAUserInABus']);
validateDevice(fixture, catalog.beginRouteAccess.steps);
assert.throws(() => validateDevice({...fixture, passenger_id: null}, catalog.beginRouteAccess.steps));
assert.throws(() => validateDevice({...fixture, access_type: null}, catalog.beginRouteAccess.steps));
console.log('PASS: begin route + access sequence and passenger fixture requirements');

const accessSteps = catalog.beginRouteAccess.steps;
const pools = [
  {...fixture, passengers: [{passenger_id: 10, access_type: 3}, {passenger_id: 11, access_type: 4}]},
  {...fixture, device_id: 2, passengers: [{passenger_id: 10, access_type: 3}, {passenger_id: 12, access_type: 3}, {passenger_id: 13, access_type: 4}]},
];
assert.deepEqual(allocatePassengers(pools, accessSteps, 2, '2026-11-14'), [pools[0].passengers, pools[1].passengers.slice(0, 2)]);
assert.equal(allocatePassengers(pools, accessSteps, 100, '2026-11-14')[0].length, 100);
assert.throws(() => allocatePassengers([{...pools[0], passenger_date: '2026-11-13'}], accessSteps, 1, '2026-11-14'), /simulated start date/);
assert.equal(allocatePassengers([fixture, {...fixture, device_id: 2}], accessSteps, 100, '2026-11-14')[1][99].passenger_id, fixture.passenger_id);
assert.throws(() => allocatePassengers([{...fixture, passengers: []}], accessSteps, 1, '2026-11-14'), /valid route passengers/);
assert.deepEqual(allocatePassengers([fixture], ['beginARoute'], 100, '2026-11-14'), [[]]);
console.log('PASS: passenger pool cycling across devices/repetitions, missing pools, and date checks');

for (const action of ['saveDeviceTrafficAttendance', 'saveTrafficAttendance']) {
  const request = payload(action, fixture, Date.parse('2026-11-15T08:00:00Z'), 'attendance-run');
  assert.equal(request.device_id, '1');
  assert.equal(request.activity_id, 1);
  assert.equal(request.place_id, 1);
  assert.equal(catalog[action].queued, true);
  assert.equal(catalog[action].category, 'Attendance');
  assert.throws(() => validateDevice({...fixture, attendance_place_id: null}, [action]), /attendance_place_id/);
  if (action === 'saveTrafficAttendance') {
    assert.equal(request.user_id, 10);
    assert.equal(request.traffic_type, 1);
    assert.equal(request.access_type, 3);
    assert.equal(request.datetime, undefined);
    assert.throws(() => validateDevice({...fixture, attendance_traffic_type: 1001}, [action]), /traffic type/);
  } else {
    assert.equal(request.user_id, fixture.driver_id);
    assert.equal(request.traffic_type_id, 21);
    assert.equal(request.traffic_attendance_date, '2026-11-15 08:00:00');
  }
}
console.log('PASS: attendance envelopes, configured IDs, valid traffic types, and queue semantics');

// Render the actual service selector with a small DOM stand-in.
const {runInNewContext} = await import('node:vm');
const selector = {children: [], replaceChildren() {this.children = [];}, append(child) {this.children.push(child);}};
const appSource = readFileSync(new URL('web/app.js', root), 'utf8');
const populate = appSource.slice(appSource.indexOf('function populateServices('), appSource.indexOf("['vus', 'journeys', 'service']"));
runInNewContext(populate + '\npopulateServices(catalog);', {
  catalog, services: {}, $: () => selector, estimate() {},
  document: {createElement() {return {children: [], append(child) {this.children.push(child);}};}},
});
const attendanceGroup = selector.children.find(group => group.label === 'Attendance');
assert.deepEqual(Array.from(attendanceGroup.children, option => option.value), ['saveDeviceTrafficAttendance', 'saveTrafficAttendance']);
const passengerGroup = selector.children.find(group => group.label === 'Passenger access');
assert(!passengerGroup.children.some(option => option.value === 'saveTrafficAttendance'));
console.log('PASS: Attendance section renders both services without duplicating passenger access');

const bulkFixture = {...fixture, passengers: Array.from({length: 25}, (_, index) => ({passenger_id: index + 100, access_type: 3}))};
validateDevice(bulkFixture, ['bulkUserAccess']);
for (const [iteration, first] of [[0, 100], [1, 110], [2, 120]]) {
  const batch = payload('bulkUserAccess', {...bulkFixture, bulk_iteration: iteration}, Date.parse('2026-11-16T08:00:00Z'), 'bulk-test');
  assert.equal(batch.length, 10);
  assert.equal(batch[0].user_id, first);
  assert.equal(new Set(batch.map(p => p.user_id)).size, 10);
  assert(batch.every(p => p.trace_id === 'bulk-test' && p.device_id === fixture.device_id && p.route_id === fixture.route_id && p.traffic_type === 1 && p.access_type === 3));
  assert(batch.every(p => p.accessDate === Date.parse('2026-11-16T13:00:00Z')));
}
const smallBatch = payload('bulkUserAccess', {...fixture, passengers: [{passenger_id: 10, access_type: 3}, {passenger_id: 11, access_type: 4}]}, Date.parse('2026-11-16T08:00:00Z'), 'bulk-test');
assert.deepEqual(smallBatch.map(p => p.user_id), [10, 11]);
assert.deepEqual(smallBatch.map(p => p.access_type), [3, 4]);
assert.equal(payload('bulkUserAccess', fixture, Date.parse('2026-11-16T08:00:00Z'), 'bulk-test').length, 1);
assert.throws(() => allocatePassengers([{...fixture, passengers: []}], ['bulkUserAccess'], 1, '2026-11-16'), /valid route passengers/);
assert.throws(() => allocatePassengers([{...fixture, passenger_date: '2026-11-15'}], ['bulkUserAccess'], 1, '2026-11-16'), /simulated start date/);
assert(catalog.bulkUserAccess.queued);
assert.equal(catalog.bulkUserAccess.namespace, 'BulkUserAccess');
assert(passengerGroup.children.some(option => option.value === 'bulkUserAccess'));
console.log('PASS: bulk passenger arrays, credential methods, pool rotation, timezone conversion, preflight, and Passenger access section');
