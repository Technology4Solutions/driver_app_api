'use strict';
const $ = id => document.getElementById(id);
let configuredCredentials = false;
let token, ready = false, busy = false, submitting = false;
function buttons() { $('start').disabled = !ready || busy || submitting; $('stop').disabled = !ready || !busy || submitting; }
function estimate() { $('estimate').textContent = `Up to ${Number($('vus').value) * Number($('journeys').value) * ($('service').value === 'journey' ? 2 : 1)} requests. Each device runs its repetitions sequentially.`; }
['vus', 'journeys', 'service'].forEach(id => $(id).addEventListener('input', estimate));
const today = new Date();
$('date').value = `${today.getFullYear()}-${String(today.getMonth()+1).padStart(2,'0')}-${String(today.getDate()).padStart(2,'0')}T08:00:00`;
estimate();
async function request(path, data) {
  const response = await fetch(path, data === undefined ? {} : {method:'POST', headers:{'Content-Type':'application/json','X-Runner-Token':token},body:JSON.stringify(data)});
  const body = await response.json();
  if (!response.ok) throw new Error(body.error || `HTTP ${response.status}`);
  return body;
}
function metrics(summary) {
  const metric = name => { const m = summary?.metrics?.[name]; return m?.values || m || {}; };
  const time = metric('http_req_duration');
  const number = v => Number.isFinite(v) ? v.toFixed(1) : '—';
  $('avg').textContent = number(time.avg);
  $('requests').textContent = metric('http_reqs').count ?? '—';
  const percent = v => Number.isFinite(v) ? `${(v*100).toFixed(1)}%` : '—';
  // Legacy summary-export uses "value" for Rate metrics; newer summaries use "rate".
  const rate = name => metric(name).rate ?? metric(name).value;
  $('failed').textContent = percent(rate('http_req_failed'));
  $('success').textContent = `Application success: ${percent(rate('application_success'))} · Database saved: ${percent(rate('database_saved'))}`;
  $('chart').replaceChildren();
  if (!summary) { const p = document.createElement('p'); p.className='empty'; p.textContent='Your response-time chart will appear after a test finishes.'; $('chart').append(p); return; }
  const values = [['Average',time.avg],['Median',time.med],['p90',time['p(90)']],['p95',time['p(95)']],['Maximum',time.max]];
  const maximum = Math.max(1,...values.map(([,v]) => Number.isFinite(v) ? v : 0));
  for (const [label,value] of values) {
    const row=document.createElement('div'); row.className='bar-row';
    const caption=document.createElement('span');caption.textContent=label;
    // SVG width attributes work under the page's strict no-inline-style CSP.
    const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');svg.setAttribute('viewBox','0 0 100 8');svg.setAttribute('width','100%');svg.setAttribute('height','16');svg.setAttribute('preserveAspectRatio','none');
    const rect=document.createElementNS(svg.namespaceURI,'rect');rect.setAttribute('width',String(Number.isFinite(value) ? value/maximum*100 : 0));rect.setAttribute('height','8');rect.setAttribute('fill','#c4f37b');svg.append(rect);
    const output=document.createElement('output');output.textContent=number(value)+' ms';row.append(caption,svg,output);$('chart').append(row);
  }
}
function render(state) {
  busy=['starting','running'].includes(state.status);buttons();$('status').textContent=state.status;
  $('run-id').textContent=state.id ? `Run ${state.id.slice(0,8)}${state.exit_code == null ? '' : ' · exit '+state.exit_code}` : 'No active run';
  const logs=$('logs'), bottom=logs.scrollHeight-logs.scrollTop-logs.clientHeight<40;
  logs.textContent=state.logs || (busy ? 'Starting k6… Docker may need to download its image.' : 'Ready when you are. Start with one device and one repetition.');
  if(bottom) logs.scrollTop=logs.scrollHeight;
  metrics(state.summary);
}
$('form').addEventListener('submit',async event=>{
  event.preventDefault();submitting=true;buttons();$('error').textContent='';
  try {
    let date=$('date').value; if(date.length===16) date+=':00';
    render(await request('/api/start',{service:$('service').value,vus:Number($('vus').value),journeys:Number($('journeys').value),duration:$('duration').value,date:date+'Z',username:configuredCredentials ? '' : $('username').value,password:configuredCredentials ? '' : $('password').value,acknowledge:$('acknowledge').checked}));
    $('password').value='';
  } catch(error){$('error').textContent=error.message;} finally{submitting=false;buttons();}
});
$('stop').addEventListener('click',async()=>{submitting=true;buttons();try{await request('/api/stop');$('status').textContent='Stopping…';}catch(error){$('error').textContent=error.message;}finally{submitting=false;buttons();}});
async function poll(){try{render(await request('/api/state'));ready=true;buttons();}catch(error){ready=false;buttons();$('status').textContent='Disconnected';}setTimeout(poll,1500);}
(async()=>{try{const config=await request('/api/config');token=config.token;configuredCredentials=config.credentials_configured;if(configuredCredentials){$('credentials').hidden=true;$('credential-note').hidden=false;$('username').required=false;$('password').required=false;}$('target').textContent=`${config.api_url} · ${config.devices} devices in ${config.dataset}`;$('vus').max=config.devices;ready=true;buttons();poll();}catch(error){$('error').textContent=error.message;}})();
