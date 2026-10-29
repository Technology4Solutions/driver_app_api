import http from 'k6/http';
import encoding from 'k6/encoding';
import exec from 'k6/execution';
import { SharedArray } from 'k6/data';
import { check, sleep } from 'k6';
import { Rate, Counter } from 'k6/metrics';

const devices = new SharedArray('devices', () => JSON.parse(open(__ENV.DATA_FILE || '/data/devices.json')));
const requestedService = __ENV.SERVICE || 'journey';
const service = requestedService === 'startARoute' ? 'beginARoute' : requestedService;
if (!['journey', 'beginARoute', 'endARoute', 'deviceStatus'].includes(service)) throw new Error('SERVICE must be journey, beginARoute, endARoute, or deviceStatus');
const vus = Number(__ENV.VUS || devices.length);
const journeys = Number(__ENV.JOURNEYS || 10);
if (!Number.isInteger(vus) || vus < 1 || vus > devices.length) throw new Error('VUS must be between 1 and the number of exported devices');
if (!Number.isInteger(journeys) || journeys < 1) throw new Error('JOURNEYS must be a positive integer');
if (new Set(devices.map(d => d.device_id)).size !== devices.length) throw new Error('Dataset contains duplicate device IDs');
if (service === 'deviceStatus' && devices.slice(0, vus).some(d => !d.device_imei || !(Number(d.business_id) > 0))) throw new Error('Re-export dataset: DeviceStatus requires device_imei and business_id');
const base = (__ENV.API_BASE_URL || '').replace(/\/$/, '');
if (!/^https?:\/\//.test(base)) throw new Error('Set API_BASE_URL explicitly to the isolated test API');
if (!__ENV.REAXIUM_API_USER || !__ENV.REAXIUM_API_PASSWORD) throw new Error('API credentials required');
if (!__ENV.RUN_ID || !/^[a-zA-Z0-9-]{1,40}$/.test(__ENV.RUN_ID)) throw new Error('RUN_ID must be a unique 1-40 character alphanumeric/hyphen run identifier');
const dateBase = Date.parse(__ENV.START_DATE || '');
if (!Number.isFinite(dateBase)) throw new Error('Set START_DATE, for example 2026-10-22T08:00:00Z (wall-clock fields sent to API)');
const savedRate = new Rate('database_saved');
const successRate = new Rate('application_success');
const completed = new Counter(service === 'journey' ? 'journeys_completed' : 'service_iterations_completed');
export const options = {
  scenarios: { routes: { executor: 'per-vu-iterations', vus, iterations: journeys, maxDuration: __ENV.MAX_DURATION || '30m' } },
  thresholds: { http_req_failed: ['rate==0'], database_saved: ['rate==1'], application_success: ['rate==1'] },
};
const headers = { 'Content-Type': 'application/json', Authorization: `Basic ${encoding.b64encode(`${__ENV.REAXIUM_API_USER}:${__ENV.REAXIUM_API_PASSWORD}`)}` };
function date(ms) { return new Date(ms).toISOString().slice(0, 19).replace('T', ' '); }
function post(action, payload) {
  const isStatus = action === 'DeviceStatus/update';
  const path = isStatus ? '/device/DeviceStatus/update' : `/DeviceTraffic/${action}`;
  const namespace = isStatus ? 'DeviceStatus' : 'DeviceTraffic';
  const requestBase = isStatus ? base.replace(/\/school_bus_cloud$/, '') : base;
  const res = http.post(`${requestBase}${path}`, JSON.stringify({ ReaxiumParameters: { [namespace]: payload } }), {
    headers, timeout: '30s', redirects: 0, tags: { name: action },
  });
  let envelope;
  try { envelope = res.json().ReaxiumResponse; } catch (_) { envelope = null; }
  const success = res.status === 200 && envelope?.code === 0;
  const saved = res.status === 200 && (success || envelope?.object?.database_saved === true);
  savedRate.add(saved, { endpoint: action });
  successRate.add(success, { endpoint: action });
  check(res, { [`${action}: application code is 0`]: () => success }, { endpoint: action });
  if (!saved) exec.test.abort(`${action}: save failed or outcome unknown; stopping without retries (trace ${payload.trace_id})`);
  return saved;
}
export default function () {
  const d = devices[exec.vu.idInTest - 1];
  const iteration = exec.vu.iterationInScenario;
  const start = dateBase + iteration * 120000;
  const common = { device_id: d.device_id, route_id: d.route_id, driver_id: d.driver_id,
    bus_info: String(d.bus_info), trace_id: `load-${__ENV.RUN_ID}-${exec.vu.idInTest}-${iteration}` };
  if (service === 'deviceStatus') {
    post('DeviceStatus/update', {
      device_imei: d.device_imei, user_id: d.driver_id, business_id: Number(d.business_id),
      status_date: date(start), trace_id: common.trace_id,
      device_status: { temperature: 30, battery_power_level: 85, is_charging: 0, installed_app_version: '1.0.0', bus_info: String(d.bus_info) },
      device_location: { latitude: Number(d.start_latitude), longitude: Number(d.start_longitude), speed: 0, bearing: 0, accuracy: 10, lag_time: 0 },
    });
    completed.add(1);
    sleep(0.1);
    return;
  }
  if (service !== 'endARoute') post('beginARoute', { ...common, stop_id: d.start_stop_id, latitude: Number(d.start_latitude), longitude: Number(d.start_longitude), traffic_date: date(start) });
  // Partial downstream failure still means begin committed: finish the journey, never retry begin.
  if (service !== 'beginARoute') post('endARoute', { ...common, stop_id: d.end_stop_id, latitude: Number(d.end_latitude), longitude: Number(d.end_longitude), traffic_date: date(start + 60000), traffic_situation_id: 1 });
  completed.add(1);
  sleep(0.1);
}
