// Pure checks run without k6 or an API. Import as a data URL so no package mode is required.
import {readFileSync} from 'node:fs';
import assert from 'node:assert/strict';
const root = new URL('./', import.meta.url);
const {payload, validateDevice, databaseSaved} = await import('data:text/javascript;base64,' + Buffer.from(readFileSync(new URL('payloads.js', root))).toString('base64'));
const catalog = JSON.parse(readFileSync(new URL('services.json', root)));
const fixture = {device_id: 1, route_id: 2, route_ids: '2,3', driver_id: 4, passenger_id: 5, access_type: 3, bus_info: '6', device_imei: 'mock', business_id: 1,
  start_stop_id: 7, end_stop_id: 8, start_latitude: 27, start_longitude: -80, end_latitude: 28, end_longitude: -81};
const steps = catalog.allServices.steps;
assert.equal(steps.length, 13);
for (const service of steps) {
  validateDevice(fixture, [service]);
  const request = payload(service, fixture, Date.parse('2026-11-09T08:00:00Z'), 'load-test');
  assert.equal(request.trace_id, 'load-test');
  assert.equal(request.bus_info ?? request.device_status.bus_info, '6');
  if (service.includes('Merge')) {assert.equal(request.route_ids, '2,3'); assert.equal(request.route_id, undefined);}
  if (service === 'executeAnAccessOfAUserInABus') {assert.equal(request.user_id, 5); assert.equal(request.access_type, 3); assert.equal(request.accessDate, 1794211200000);}
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
const endpoints = [...routes.matchAll(/"(\/(?:Alarm|DeviceTraffic|UserAccess|device\/DeviceStatus)\/[^"\n]+)"/g)].map(m => m[1]);
for (const endpoint of endpoints) assert(Object.values(catalog).some(s => s.path === endpoint), `Missing k6 service ${endpoint}`);
console.log('PASS: all 13 service payloads, dataset checks, route coverage, and merged partial saves');
