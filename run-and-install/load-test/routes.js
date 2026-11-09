import http from 'k6/http';
import encoding from 'k6/encoding';
import exec from 'k6/execution';
import { SharedArray } from 'k6/data';
import { check, sleep } from 'k6';
import { Rate, Counter } from 'k6/metrics';
import { validateDevice, payload, databaseSaved, mergedIds } from './payloads.js';

const catalog = JSON.parse(open('./services.json'));
const devices = new SharedArray('devices', () => JSON.parse(open(__ENV.DATA_FILE || '/data/devices.json')));
const service = __ENV.SERVICE === 'startARoute' ? 'beginARoute' : (__ENV.SERVICE || 'journey');
if (!Object.prototype.hasOwnProperty.call(catalog, service)) throw new Error(`Unknown SERVICE: ${service}`);
const steps = catalog[service].steps || [service];
const vus = Number(__ENV.VUS || devices.length);
const iterations = Number(__ENV.JOURNEYS || 10);
if (!Number.isInteger(vus) || vus < 1 || vus > devices.length) throw new Error('VUS must be between 1 and the number of exported devices');
if (!Number.isInteger(iterations) || iterations < 1) throw new Error('JOURNEYS must be a positive integer');
if (new Set(devices.map(d => String(d.device_id))).size !== devices.length) throw new Error('Dataset contains duplicate device IDs');
devices.slice(0, vus).forEach(d => validateDevice(d, steps));
const base = (__ENV.API_BASE_URL || '').replace(/\/$/, '');
if (!/^https?:\/\//.test(base)) throw new Error('Set API_BASE_URL explicitly to the isolated test API');
if (!__ENV.REAXIUM_API_USER || !__ENV.REAXIUM_API_PASSWORD) throw new Error('API credentials required');
if (!__ENV.RUN_ID || !/^[a-zA-Z0-9-]{1,40}$/.test(__ENV.RUN_ID)) throw new Error('RUN_ID must be a unique 1-40 character alphanumeric/hyphen run identifier');
const dateBase = Date.parse(__ENV.START_DATE || '');
if (!Number.isFinite(dateBase)) throw new Error('Set START_DATE, for example 2026-10-22T08:00:00Z');
const savedRate = new Rate('database_saved');
const queueRate = new Rate('queue_accepted');
const successRate = new Rate('application_success');
const completed = new Counter(service === 'journey' ? 'journeys_completed' : 'service_iterations_completed');
const thresholds = {http_req_failed: ['rate==0'], application_success: ['rate==1']};
if (steps.some(s => s !== 'deviceStatus')) thresholds.database_saved = ['rate==1'];
if (steps.includes('deviceStatus')) thresholds.queue_accepted = ['rate==1'];
for (const action of steps) thresholds[`application_success{endpoint:${action}}`] = ['rate==1'];
export const options = {
  scenarios: {services: {executor: 'per-vu-iterations', vus, iterations, maxDuration: __ENV.MAX_DURATION || '30m'}}, thresholds,
};
const headers = {'Content-Type': 'application/json', Authorization: `Basic ${encoding.b64encode(`${__ENV.REAXIUM_API_USER}:${__ENV.REAXIUM_API_PASSWORD}`)}`};
function post(action, data, d) {
  const descriptor = catalog[action];
  const isStatus = action === 'deviceStatus';
  const requestBase = isStatus ? base.replace(/\/school_bus_cloud$/, '') : base;
  const res = http.post(`${requestBase}${descriptor.path}`, JSON.stringify({ReaxiumParameters: {[descriptor.namespace]: data}}), {
    headers, timeout: '30s', redirects: 0, tags: {name: action, endpoint: action},
  });
  let envelope;
  try { envelope = res.json().ReaxiumResponse; } catch (_) { envelope = null; }
  const success = res.status === 200 && envelope?.code === 0;
  const saved = res.status === 200 && databaseSaved(envelope, mergedIds(d).length);
  if (isStatus) queueRate.add(success, {endpoint: action});
  else savedRate.add(saved, {endpoint: action});
  successRate.add(success, {endpoint: action});
  check(res, {[`${action}: application code is 0`]: () => success}, {endpoint: action});
  if (!(isStatus ? success : saved)) exec.test.abort(`${action}: save/queue failed or outcome unknown; stopping without retries (trace ${data.trace_id})`);
}
export default function () {
  const d = devices[exec.vu.idInTest - 1];
  const iteration = exec.vu.iterationInScenario;
  const start = dateBase + iteration * Math.max(120000, steps.length * 60000);
  const trace = `load-${__ENV.RUN_ID}-${exec.vu.idInTest}-${iteration}`;
  steps.forEach((action, index) => post(action, payload(action, d, start + index * 60000, trace), d));
  completed.add(1);
  sleep(0.1);
}
