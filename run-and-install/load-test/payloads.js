// Pure request builders, shared by every k6 scenario.
export function date(ms) { return new Date(ms).toISOString().slice(0, 19).replace('T', ' '); }
// START_DATE represents device wall time, even though its carrier string ends in Z.
// UserAccess requires an actual epoch; the backend converts it to New York time.
export function accessTimestamp(wallMs) {
  const year = new Date(wallMs).getUTCFullYear();
  if (!Number.isFinite(wallMs) || year < 2007 || year > 9999) throw new Error('Passenger tests require a simulated date from 2007 onward');
  const firstSunday = month => 1 + (7 - new Date(Date.UTC(year, month, 1)).getUTCDay()) % 7;
  const spring = Date.UTC(year, 2, firstSunday(2) + 7, 2);
  const fall = Date.UTC(year, 10, firstSunday(10), 2);
  const hour = 3600000;
  if ((wallMs >= spring && wallMs < spring + hour) || (wallMs >= fall - hour && wallMs < fall)) {
    throw new Error('Passenger test time is ambiguous or nonexistent in America/New_York');
  }
  const daylight = wallMs >= spring + hour && wallMs < fall - hour;
  return wallMs + (daylight ? 4 : 5) * hour;
}
export function mergedIds(device) {
  const value = device.route_ids;
  return Array.isArray(value) ? value.map(Number) : String(value || '').split(',').filter(Boolean).map(Number);
}
function positive(value) { return Number.isInteger(Number(value)) && Number(value) > 0; }
export function validateDevice(device, steps) {
  for (const key of ['device_id', 'route_id', 'driver_id', 'bus_info', 'start_stop_id', 'end_stop_id']) {
    if (!positive(device[key])) throw new Error(`Dataset device ${device.device_id}: ${key} must be a positive ID`);
  }
  for (const [key, limit] of [['start_latitude', 90], ['end_latitude', 90], ['start_longitude', 180], ['end_longitude', 180]]) {
    if (device[key] == null || device[key] === '' || !Number.isFinite(Number(device[key])) || Math.abs(Number(device[key])) > limit) throw new Error(`Invalid dataset coordinate ${key}`);
  }
  if (steps.includes('deviceStatus') && (!device.device_imei || !positive(device.business_id))) throw new Error('Re-export dataset: DeviceStatus requires device_imei and business_id');
  if (steps.some(s => s.includes('Merge'))) {
    const ids = mergedIds(device);
    if (ids.length < 2 || !ids.every(positive) || new Set(ids).size !== ids.length) throw new Error(`Dataset device ${device.device_id} needs route_ids with at least two distinct valid routes. Re-export using export-devices.py; older datasets only contain route_id. Select devices with two related routes before running a merged test.`);
  }
  if (steps.includes('executeAnAccessOfAUserInABus') && (!positive(device.passenger_id) || !positive(device.access_type))) throw new Error('Passenger service requires passenger_id and access_type matching user_access_data');
}
export function payload(action, d, start, trace) {
  const finishing = /^(end|interrupt)/.test(action);
  const common = {device_id: Number(d.device_id), route_id: Number(d.route_id), driver_id: Number(d.driver_id),
    bus_info: String(d.bus_info), trace_id: trace, stop_id: Number(finishing ? d.end_stop_id : d.start_stop_id),
    latitude: Number(finishing ? d.end_latitude : d.start_latitude), longitude: Number(finishing ? d.end_longitude : d.start_longitude)};
  if (action === 'deviceStatus') return {
    device_imei: d.device_imei, user_id: Number(d.driver_id), business_id: Number(d.business_id), status_date: date(start), trace_id: trace,
    device_status: {temperature: 30, battery_power_level: 85, is_charging: 0, installed_app_version: '1.0.0', bus_info: String(d.bus_info)},
    device_location: {latitude: common.latitude, longitude: common.longitude, speed: 0, bearing: 0, accuracy: 10, lag_time: 0},
  };
  if (action === 'executeAnAccessOfAUserInABus') {
    const {driver_id, ...access} = common;
    return {...access, user_id: Number(d.passenger_id), access_type: Number(d.access_type), traffic_type: 1,
      traffic_info: 'k6 passenger boarding', accessDate: accessTimestamp(start), stop_id_by_distance: common.stop_id, access_situation_id: 1};
  }
  if (action.startsWith('send') && action !== 'sendTraffic') {
    if (action === 'sendManualSkippedStopNotification') return {...common, traffic_date: date(start)};
    return {...common, notification_type: action === 'sendNotification' ? 9 : action === 'sendNextStopNotification' ? 5 : 18};
  }
  const result = {...common, traffic_date: date(start), traffic_situation_id: finishing ? 2 : 1};
  if (action === 'endARoute' || action === 'endAMergedRoute') result.traffic_situation_id = 1;
  if (action === 'sendTraffic') result.traffic_type_id = 15;
  if (action.includes('Merge')) { result.route_ids = mergedIds(d).join(','); delete result.route_id; delete result.stop_id; }
  return result;
}
export function databaseSaved(envelope, expectedRoutes) {
  if (envelope?.code === 0 || envelope?.object?.database_saved === true) return true;
  const routes = envelope?.object?.routes;
  return expectedRoutes > 0 && Array.isArray(routes) && routes.length === expectedRoutes && routes.every(r => r.response?.code === 0 || r.response?.object?.database_saved === true);
}
