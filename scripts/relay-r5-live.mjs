#!/usr/bin/env node
// R5 live acceptance on the explicitly disposable dev1 instance. Node >=22.
// Usage: relay-r5-live.mjs https-url staff-credentials.json output-dir redis-port
// Requires root to kill/restart only dev1's verified server child. Credentials
// stay in the root-only output directory; evidence contains no token/password.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdirSync, realpathSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { join } from 'node:path';
const [target, credentialsPath, output, redisPort] = process.argv.slice(2);
assert(target === 'https://conclav.io:6790' && redisPort === '6788', 'dev1 only');
assert.equal(realpathSync(output), '/opt/rustmush/dev1/r5-evidence');
const credentials = JSON.parse(readFileSync(credentialsPath, 'utf8'));
mkdirSync(output, { recursive: true });
const resuming=process.env.R5_RESUME==='1';
const evidence = resuming ? JSON.parse(readFileSync(join(output,'live.json'),'utf8'))
  : { target, at: new Date().toISOString(), checks: [], milestones: [] };
const done = name => evidence.milestones.some(m=>m.test===name);
if(resuming) { delete evidence.passed; delete evidence.error; }
const sockets = [];
const sleep = ms => new Promise(r => setTimeout(r, ms));
const shell = (cmd, args) => execFileSync(cmd, args, {encoding:'utf8',timeout:30000}).trim();
const redis = key => JSON.parse(shell('redis-cli',['-p',redisPort,'--raw','JSON.GET',key]) || 'null');
const envelope = id => redis('mod:propagation:envelope:' + id);
const registry = () => redis('mod:sim:relay:registry');
function save(name, value) { writeFileSync(join(output,name), JSON.stringify(value,null,2)+'\n', {mode:0o600}); }
function text(value) {
  if (typeof value === 'string') return value;
  if (Array.isArray(value)) return value.map(text).join('');
  if (value && typeof value === 'object') return typeof value.text === 'string' ? value.text : Object.values(value).map(text).filter(Boolean).join(' ');
  return '';
}
async function auth(route, body) {
  const deadline=Date.now()+120000;
  while(true) {
    const response = await fetch(target + '/api/auth/' + route, {
      method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body),
      signal:AbortSignal.timeout(15000),
    });
    if(response.status===429 && Date.now()<deadline) {
      const seconds=Number(response.headers.get('retry-after')) || 15;
      await response.body?.cancel();
      await sleep(Math.min(60000,Math.max(1000,seconds*1000)));
      continue;
    }
    assert.equal(response.status,200,'authentication endpoint '+route+': '+response.status);
    return response.json();
  }
}
async function connect(login) {
  const ws = new WebSocket(target.replace(/^http/,'ws')+'/ws?char='+login.characters[0].key,'rustmush.v1.'+login.token);
  sockets.push(ws);
  const client = {ws,frames:[],name:login.characters[0].name,object:'#'+login.characters[0].key};
  ws.addEventListener('message',event=>{
    if(typeof event.data !== 'string') return;
    const f=JSON.parse(event.data);
    if(f.t==='ping') ws.send(JSON.stringify({t:'pong',d:{nonce:f.d.nonce}}));
    else client.frames.push(f);
  });
  await new Promise((resolve,reject)=>{
    const timer=setTimeout(()=>reject(new Error('WebSocket timeout')),15000);
    ws.addEventListener('open',()=>{clearTimeout(timer);resolve();},{once:true});
    ws.addEventListener('error',()=>{clearTimeout(timer);reject(new Error('WebSocket failed'));},{once:true});
  });
  await wait(client,f=>f.t==='hello',0);
  await command(client,'OUTPUTSUFFIX __R5_DONE__');
  return client;
}
async function wait(client,predicate,start) {
  const end=Date.now()+15000;
  while(Date.now()<end) {
    const found=client.frames.slice(start).find(predicate);
    if(found) return found;
    assert.equal(client.ws.readyState,WebSocket.OPEN,'socket closed');
    await sleep(25);
  }
  throw new Error('Missing response: '+JSON.stringify(client.frames.slice(start)).slice(0,3000));
}
async function command(client,line,expected) {
  const started=performance.now();
  const start=client.frames.length;
  client.ws.send(JSON.stringify({t:'line',d:line}));
  await wait(client,f=>text(f.d).includes('__R5_DONE__'),start);
  const reply=client.frames.slice(start).filter(f=>!text(f.d).includes('__R5_DONE__')).map(f=>text(f.d)).join('\n');
  if(expected) assert(reply.includes(expected),line+': '+reply);
  if(!line.startsWith('OUTPUT')) evidence.checks.push({actor:client.name,line,reply,wallMs:Math.round(performance.now()-started),replyBytes:Buffer.byteLength(reply)});
  return reply;
}
async function until(test, label, seconds=180) {
  const end=Date.now()+seconds*1000;
  while(Date.now()<end) {
    const value=test();
    if(value) return value;
    await sleep(200);
  }
  throw new Error('Timed out: '+label);
}
function nowIc() {
  const head=redis('mod:rustmush_kernel_time:world_ic/head');
  const epoch=redis('mod:rustmush_kernel_time:world_ic/epoch/'+String(head.generation).padStart(20,'0'));
  return epoch.ic_anchor+(Math.floor(Date.now()/1000)-epoch.real_anchor)*epoch.rate;
}
async function issue(staff, receiver, body) {
  const reply=await command(staff,'+order '+receiver+'='+body,'Queued');
  const id=Number(reply.match(/envelope #(\d+)/)[1]);
  assert(envelope(id),'accepted dispatch is durable');
  return id;
}
async function crash(label) {
  const service='dev1mush-gateway';
  const gateway=Number(shell('systemctl',['show',service,'-p','MainPID','--value']));
  const old=Number(shell('pgrep',['-P',String(gateway),'-x','mush-server']));
  assert.equal(realpathSync('/proc/'+old+'/exe'),'/opt/rustmush/dev1/build/target/release/mush-server');
  const cursor=shell('journalctl',['-u',service,'-n','0','--show-cursor','--no-pager']).split('-- cursor: ')[1];
  assert(cursor,'journal cursor');
  process.kill(old,'SIGKILL');
  const fresh=await until(()=>{
    try {
      const pid=Number(shell('pgrep',['-P',String(gateway),'-x','mush-server']));
      return pid!==old && pid;
    } catch {return false;}
  },'replacement server',60);
  await until(()=>{
    const logs=shell('journalctl',['-u',service,'--after-cursor',cursor,'--no-pager','-o','cat']);
    return logs.includes('gateway connected');
  },'gateway reconnected',60);
  evidence.milestones.push({test:label,signal:'SIGKILL',gateway,old,fresh});
  // Authentication is served by the gateway; game hello waits for the new child.
  await sleep(1000);
}
function milestone(test, facts) {
  evidence.milestones.push({test,...facts});
  save('live.json',evidence);
  console.log(JSON.stringify({completed:test,...facts}));
}
const portMap = () => redis('mod:tacsim:tacsim:port:tacsim.lane.ceres_phobos/ceres_port');
const journey = hull => redis('mod:sim:cjourney:'+hull);
let staff, pilot, staffLogin, pilotLogin, ticketBackup;
async function reconnect() {
  staffLogin=await auth('login',credentials);
  staff=await connect(staffLogin);
  pilotLogin=await auth('login',JSON.parse(readFileSync(join(output,'pilot-credentials.json'),'utf8')));
  pilot=await connect(pilotLogin);
}
async function ready() {
  await until(()=>{try{return shell('curl',['-fsS','--cacert','/opt/rustmush/dev1/tls/cert.pem',target+'/']).length>0;}catch{return false;}},'HTTPS ready',60);
}
async function rate(n) { await command(staff,'@tacsim/time rate='+n); }
async function advance(ic) {
  if(ic-nowIc()>10000) {
    await rate(3600);
    await until(()=>nowIc()>=ic-7200,'accelerated passage progress',1800);
  }
  if(ic-nowIc()>720) {
    await rate(360);
    await until(()=>nowIc()>=ic-720,'approach to passage checkpoint',120);
  }
  await rate(24);
  await until(()=>nowIc()>=ic,'passage checkpoint',420);
}
async function radioFlight(id) {
  return until(()=>envelope(id)?.transit?.flight,'positive physical flight',30);
}
async function arrive(id) {
  return until(()=>{const e=envelope(id);return e?.state==='delivered'&&e;},'radio arrival '+id,180);
}
async function exchange(label, options={}) {
  const body='R5-'+label+'-'+randomBytes(3).toString('hex');
  // A first-leg relay flight can be only a few IC seconds. Give the crash
  // proof a real longer fallback flight so it remains in flight during reboot.
  if(options.crash) await command(staff,'@relay/disable luna_ceres.luna');
  const id=await issue(staff,pilot.object,body);
  const frozen=await radioFlight(id);
  assert(frozen.seconds>0 && frozen.basis.startsWith('ship.v1:'),'real ship metric');
  if(nowIc()<frozen.due.at-24) {
    assert(!(await command(pilot,'+orders')).includes(body),'no early payload');
  }
  if(options.crash) {
    await crash('mid-passage-radio-flight');
    staff=await connect(staffLogin); pilot=await connect(pilotLogin);
    assert.deepEqual(envelope(id).transit.flight,frozen,'exact flight survived SIGKILL');
    await command(pilot,'in');
    const before=nowIc();
    await rate(12);
    await sleep(1200);
    const flight=envelope(id).transit.flight;
    assert.equal(flight.due.at,frozen.due.at,'rate preserves IC due');
    assert.equal(flight.basis,frozen.basis,'rate preserves metric inputs');
    milestone('world-rate-change',{before,after:nowIc(),flight});
    await command(staff,'@relay/enable luna_ceres.luna');
    assert.equal(envelope(id).transit.flight.due.at,frozen.due.at,'repair cannot accelerate a frozen ship flight');
    await rate(24);
  }
  const delivered=await arrive(id);
  assert(delivered.advanced.at>=frozen.due.at);
  assert.equal(delivered.transit.forwardings,0,'walking the hull is not forwarding');
  await command(pilot,'+orders',body);
  const ackReply=await command(pilot,'+orders/ack '+id,'Acknowledgement queued');
  const ackId=Number(ackReply.match(/envelope #(\d+)/)[1]);
  const ackFlight=await radioFlight(ackId);
  assert.equal(envelope(id).state,'delivered','ACK not known remotely at submission');
  await arrive(ackId);
  await until(()=>envelope(id).state==='acknowledged','return ACK settlement');
  milestone(label,{id,ackId,frozen,ackFlight,delivered});
}
async function buy(listing, qty) {
  const quote=await command(pilot,'+market/quote #'+listing+'='+qty,'Quote #');
  const id=Number(quote.match(/Quote #(\d+)/)[1]);
  await command(pilot,'+market/accept #'+id,'Agreed.');
  await sleep(1000);
}
function listing(good, vendor) {
  const keys=shell('redis-cli',['-p',redisPort,'--scan','--pattern','mod:market:market:listing:*']).split('\n').filter(Boolean);
  assert(keys.length<1024,'bounded fixture listing scan');
  const row=keys.map(redis).find(l=>l.vendor===vendor && l.catalog.code===good && !l.source.wanted);
  assert(row,'published listing '+good+' from vendor '+vendor);
  return row.id;
}
try {
  let hull, ticketKey, ticketRecord, ticket, plan;
  if(resuming) {
    assert(done('real-trade-departure'),'resume requires a recorded real departure');
    await reconnect();
    hull=evidence.hull;
    ticketKey='mod:tacsim:action-state/v1/tacsim.ticket/'+hull+':1';
    ticketRecord=redis(ticketKey); ticket=ticketRecord.value; plan=ticket.plan;
  } else {
  staffLogin=await auth('login',credentials); staff=await connect(staffLogin);
  await rate(24);
  const pc={account:'r5p'+randomBytes(4).toString('hex'),password:randomBytes(24).toString('hex'),char_name:'R5Pilot'+randomBytes(3).toString('hex')};
  save('pilot-credentials.json',pc);
  pilotLogin=await auth('register',pc); pilot=await connect(pilotLogin);
  const beforeMap=portMap();
  // Site geology can still be mapping. Enrolment and departure enforce the
  // readiness of this actual station trade path themselves.
  await command(staff,'@tacsim/port status');
  await command(pilot,'c'); await command(pilot,'registry');
  const name='Relay Dray '+randomBytes(3).toString('hex');
  await command(pilot,'+coop/enrol '+name,'now the '+name);
  await command(pilot,'+ship',name);
  // Enrolment chooses an existing laid-up hull; identify it by its new IC name.
  const hullRow=beforeMap.hulls.find(r=>{
    const o=redis('obj:'+r.hull); return o && JSON.stringify(o).includes(name);
  });
  assert(hullRow,'chartered hull');
  hull=hullRow.hull;
  evidence.hull=hull;
  for(const move of ['out','concourse','exchange']) await command(pilot,move);
  await command(pilot,'+market/list');
  await buy(listing('metals',beforeMap.vendors.freeport_exchange.vendor),10);
  await command(pilot,'chandlery');
  for(let i=0;i<8;i++) {
    const plan=await command(pilot,'+ship/plan stickney');
    const burn=plan.match(/Burn (\d+) t.*\(tank (\d+)/);
    const food=plan.match(/Stores (\d+) crew-days.*\(locker (\d+)/);
    assert(burn&&food,'planner figures: '+plan);
    if(+burn[2]>=+burn[1] && +food[2]>=+food[1]) break;
    if(+burn[2]<+burn[1]) await buy(listing('propellant',beforeMap.vendors.occator.vendor),+burn[1]-+burn[2]);
    if(+food[2]<+food[1]) await buy(listing('provisions',beforeMap.vendors.coop.vendor),+food[1]-+food[2]);
  }
  // Draw receipts settle through the existing market sweep, not direct pool edits.
  await sleep(65000);
  assert((await command(pilot,'+ship/hold')).toLowerCase().includes('metal'),'trade cargo aboard');
  for(const move of ['exchange','concourse','arrivals','lobby','gallery']) await command(pilot,move);
  await command(pilot,'+ship/board','cycle the lock');
  await command(staff,'@teleport me=#'+registry().nodes.malapert.coverage[0]);
  await command(staff,'@relay/enable luna_ceres.luna');
  await command(staff,'@relay/enable luna_ceres.ceres');
  await command(pilot,'+ship/undock stickney','Clearance filed');
  ticketKey='mod:tacsim:action-state/v1/tacsim.ticket/'+hull+':1';
  ticketRecord=await until(()=>redis(ticketKey),'real burned ticket',90);
  ticket=ticketRecord.value;
  plan=ticket.plan;
  await until(()=>journey(hull)?.state?.state==='Cruising','actual carrier departure',90);
  milestone('real-trade-departure',{hull,ticket,journey:journey(hull)});
  await command(pilot,'+ship','IC seconds one way');
  }
  if(!done('departure')) await exchange('departure');

  const first=plan.legs[0];
  if(!done('mid-leg')) {
  await advance(first.t0+Math.floor(first.secs/4));
  // OOC channel delivery stays immediate while IC traffic has minutes of lag.
  const channel='R5OOC'+randomBytes(3).toString('hex');
  await command(staff,'@channel/create '+channel);
  await command(staff,'addcom r5ooc='+channel); await command(pilot,'addcom r5ooc='+channel);
  const frame=pilot.frames.length, start=performance.now();
  await command(staff,'r5ooc R5-immediate-OOC');
  await wait(pilot,f=>text(f.d).includes('R5-immediate-OOC'),frame);
  milestone('ooc-immediate',{wallMs:Math.round(performance.now()-start)});
  await exchange('mid-leg',{crash:true});
  }

  if(!done('missing-ticket-recovery')) {
  // Deliberately remove ONLY this fixture's ticket with dev1 stopped. A finally
  // block restores it if any assertion fails. All other records are retained.
  ticketBackup={key:ticketKey,value:ticketRecord}; save('ticket-backup.json',ticketBackup);
  shell('systemctl',['stop','dev1mush-gateway']);
  shell('redis-cli',['-p',redisPort,'DEL',ticketKey]);
  shell('systemctl',['start','dev1mush-gateway']); await ready(); await reconnect();
  await command(pilot,'+ship','physical lag unavailable');
  const unknown=await issue(staff,pilot.object,'R5-missing-ticket');
  await sleep(2000);
  assert.equal(envelope(unknown).state,'queued'); assert(envelope(unknown).transit==null);
  assert(!(await command(pilot,'+orders')).includes('R5-missing-ticket'));
  shell('systemctl',['stop','dev1mush-gateway']);
  shell('redis-cli',['-p',redisPort,'JSON.SET',ticketKey,'$',JSON.stringify(ticketRecord)]);
  ticketBackup=null;
  shell('systemctl',['start','dev1mush-gateway']); await ready(); await reconnect();
  await arrive(unknown);
  milestone('missing-ticket-recovery',{unknown,delivered:envelope(unknown)});
  }

  if(!done('nearer-end-changed')) {
  await advance(first.t0+Math.floor(first.secs*3/4));
  await exchange('nearer-end-changed');
  }
  if(!done('leg-boundary')) {
  await advance(first.t1+240);
  await until(()=>journey(hull).cursor>=1,'actual leg boundary',30);
  await exchange('leg-boundary');
  }
  if(!done('docking-and-forwarding')) {
  const arrival=plan.depart_ic+plan.total_s;
  await advance(arrival-100);
  const landing=await issue(staff,pilot.object,'R5-dock-and-forward');
  const landingFlight=await radioFlight(landing);
  assert(landingFlight.due.at>arrival,'radio flight spans docking');
  await until(()=>journey(hull).state.state==='Arrived','actual docking',60);
  await command(pilot,'airlock');
  await command(pilot,'+ship/leave');
  const landed=await arrive(landing);
  assert.equal(landed.transit.forwardings,1,'disembarking forwards from hull to berth');
  await command(pilot,'+orders','R5-dock-and-forward');
  milestone('docking-and-forwarding',{landing,landingFlight,landed,journey:journey(hull)});
  }
  if(!done('docking-return-ack')) {
    const landing=evidence.milestones.find(m=>m.test==='docking-and-forwarding').landing;
    const reply=await command(pilot,'+orders/ack '+landing,'Acknowledgement queued');
    const ackId=Number(reply.match(/envelope #(\d+)/)[1]);
    const ackFlight=await radioFlight(ackId);
    assert(ackFlight.seconds>0,'docked return ACK has physical lag');
    assert.equal(envelope(landing).state,'delivered','docked ACK not known at submission');
    const delivered=await arrive(ackId);
    assert(delivered.advanced.at>=ackFlight.due.at,'no early docked return ACK');
    await until(()=>envelope(landing).state==='acknowledged','docked return ACK settlement');
    milestone('docking-return-ack',{landing,ackId,ackFlight,delivered});
  }
  await command(staff,'@relay/check','Bindings OK');
  await rate(24);
  evidence.dev2Pid=Number(shell('systemctl',['show','dev2mush-gateway','-p','MainPID','--value']));
  assert.equal(evidence.dev2Pid,539467,'dev2 unchanged');
  evidence.passed=true;
} catch(error) {
  evidence.passed=false; evidence.error=error.stack; process.exitCode=1;
} finally {
  if(ticketBackup) {
    shell('systemctl',['stop','dev1mush-gateway']);
    shell('redis-cli',['-p',redisPort,'JSON.SET',ticketBackup.key,'$',JSON.stringify(ticketBackup.value)]);
    shell('systemctl',['start','dev1mush-gateway']);
  }
  save('live.json',evidence);
  for(const ws of sockets) ws.close();
  console.log(JSON.stringify({passed:evidence.passed,checks:evidence.checks.length,error:evidence.error}));
}
