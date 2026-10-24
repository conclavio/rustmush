#!/usr/bin/env node
// R3 live acceptance on the explicitly disposable dev1 instance. Node >=22.
// Usage: relay-r3-live.mjs https-url staff-credentials.json output-dir redis-port
// Requires root to kill/restart only dev1's verified server child. Credentials
// stay in the root-only output directory; evidence contains no token/password.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdirSync, realpathSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { join } from 'node:path';
const [target, credentialsPath, output, redisPort] = process.argv.slice(2);
assert(target === 'https://conclav.io:6790' && redisPort === '6788', 'dev1 only');
assert.equal(realpathSync(output), '/opt/rustmush/dev1/r3-evidence');
const credentials = JSON.parse(readFileSync(credentialsPath, 'utf8'));
mkdirSync(output, { recursive: true });
const evidence={target,at:new Date().toISOString(),checks:[],milestones:[]};
const sockets = [];
const sleep = ms => new Promise(r => setTimeout(r, ms));
const shell = (cmd, args) => execFileSync(cmd, args, {encoding:'utf8',timeout:30000}).trim();
const gatewayPid=Number(shell('systemctl',['show','dev1mush-gateway','-p','MainPID','--value']));
const serverPid=Number(shell('pgrep',['-P',String(gatewayPid),'-x','mush-server']));
evidence.runs??=[];
evidence.runs.push({at:new Date().toISOString(),gatewayPid,serverPid,
  serverSha256:shell('sha256sum',['/proc/'+serverPid+'/exe']).split(' ')[0]});
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
  await command(client,'OUTPUTSUFFIX __R3_DONE__');
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
  await wait(client,f=>text(f.d).includes('__R3_DONE__'),start);
  const reply=client.frames.slice(start).filter(f=>!text(f.d).includes('__R3_DONE__')).map(f=>text(f.d)).join('\n');
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
async function crash(label, beforeKill = () => {}) {
  const service='dev1mush-gateway';
  const gateway=Number(shell('systemctl',['show',service,'-p','MainPID','--value']));
  const old=Number(shell('pgrep',['-P',String(gateway),'-x','mush-server']));
  assert.equal(realpathSync('/proc/'+old+'/exe'),'/opt/rustmush/dev1/build/target/release/mush-server');
  const cursor=shell('journalctl',['-u',service,'-n','0','--show-cursor','--no-pager']).split('-- cursor: ')[1];
  assert(cursor,'journal cursor');
  beforeKill();
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
function envelopes() {
  const keys=shell('redis-cli',['-p',redisPort,'--scan','--pattern','mod:propagation:envelope:*']).split('\n').filter(Boolean);
  assert(keys.length<=4096);
  if(!keys.length) return [];
  return shell('redis-cli',['-p',redisPort,'--raw','JSON.MGET',...keys,'.']).split('\n').filter(Boolean).map(JSON.parse).filter(Boolean);
}


let staff,staffLogin,clients={},logins={},jammed=null,channel;
const actor=c=>Number(c.object.slice(1));
const transport=()=>redis('mod:propagation:relay_transport');
const service=()=>transport().service;
const mail=id=>redis('mod:mail:msg:'+id);
const box=(c,id)=>redis(`mod:mail:box:${actor(c)}:${id}`);
const receipt=id=>service().receipts.find(r=>r.envelope===id);
function milestone(test,data={}){evidence.milestones.push({test,...data});save('live.json',evidence);console.log('PASS '+test);}
async function reconnect(){for(const ws of sockets)if(ws.readyState===WebSocket.OPEN)ws.close();await sleep(500);staff=await connect(staffLogin);for(const [name,login] of Object.entries(logins))clients[name]=await connect(login);}
async function boardQuery(c){
 const id=9021,start=c.frames.length;
 c.ws.send(JSON.stringify({t:'req',d:{id,q:{t:'panel_subscribe',d:{feed:'relay.board',params:{actor:1,actual:true}}}}}));
 const f=await wait(c,f=>f.t==='reply'&&f.d.id===id,start);
 evidence.checks.push({actor:c.name,query:'relay.board',reply:f.d.event});
 assert.equal(f.d.event.t,'panel');return f.d.event.d.data;
}
async function send(className){
 await command(staff,`+relay/grant luna_ceres/${staff.object}=${className}`,'Class grant recorded');
 const subject='R3-'+className+'-'+randomBytes(3).toString('hex');
 await command(staff,`@mail/quick ${clients.bob.object}=${subject}/${'x'.repeat(900)}`,'queued for IC transit');
 const queued=await until(()=>envelopes().find(e=>e.dispatch?.key.endsWith(':out')&&e.recipient===actor(clients.bob)&&mail(Number(e.dispatch.key.split(':')[2]))?.subject===subject&&e.transit?.service),'queued '+className);
 assert.equal(queued.transit.service.class,className);assert.equal(queued.transit.service.units,8);
 const id=Number(queued.dispatch.key.split(':')[2]);assert(!box(clients.bob,id));return {id,envelope:queued.id,subject,ticket:queued.transit.service,queued:queued.transit.queued_at};
}
try {
 staffLogin=await auth('login',credentials);
 const accounts=JSON.parse(readFileSync('/opt/rustmush/dev1/r6a-evidence/actors.json','utf8'));
 for(const name of ['bob','carol'])logins[name]=await auth('login',accounts[name]);
 await reconnect();
 const catalog=registry(),luna=catalog.nodes.malapert.coverage[0],ceres=catalog.nodes.ceres.coverage[0];
 assert(service()?.config.version===1,'upgrade seeded service policy');
 evidence.policy=service().config;
 await command(staff,'@tacsim/time rate=1');
 await command(staff,'@teleport me=#'+luna);
 await command(staff,`@teleport ${clients.bob.object}=#${ceres}`);
 await command(staff,'@relay/enable luna_ceres.luna');
 jammed=catalog.nodes.ceres.place;
 await command(staff,`@prop/jam #${jammed}=100`,'is now 100');
 const board=await command(clients.bob,'+relay','Standard declared wait 60');
 assert(!board.includes('actual queued'));
 const panel=await boardQuery(clients.bob);assert.equal(panel.lines.join('\n'),board);
 milestone('web-board-query-line-parity');
 await command(clients.bob,'+relay/load luna_ceres','Permission denied');
 await command(clients.bob,`+relay/grant luna_ceres/${clients.bob.object}=sovereign`,'Permission denied');
 const items={};
 for(const cls of ['standard','deferred','priority','sovereign'])items[cls]=await send(cls);
 evidence.messages=items;save('live.json',evidence);
 const before=Object.fromEntries(Object.entries(items).map(([k,v])=>[k,envelope(v.envelope)]));
 await crash('all-four-classes-queued');await reconnect();
 for(const [cls,item] of Object.entries(items))assert.deepEqual(envelope(item.envelope),before[cls]);
 for(const item of Object.values(items))assert(!box(clients.bob,item.id));
 milestone('restart-preserves-all-four-tickets');
 await command(staff,`@prop/jam #${jammed}=0`,'is now 0');jammed=null;
 await until(()=>receipt(items.priority.envelope)?.arrived&&receipt(items.sovereign.envelope)?.arrived,'high classes delivered',30);
 assert(!box(clients.bob,items.standard.id));assert(!box(clients.bob,items.deferred.id));
 assert.equal(receipt(items.standard.envelope),undefined);
 await command(clients.bob,'+relay/receipts');
 await until(()=>box(clients.bob,items.standard.id),'Standard board wait',90);
 const sr=receipt(items.standard.envelope),pr=receipt(items.priority.envelope),so=receipt(items.sovereign.envelope);
 assert(sr.transmitted>=sr.queued+60);assert(pr.transmitted<sr.transmitted);assert(so.transmitted<sr.transmitted);
 assert(sr.ticket.actual_ready<sr.ticket.eligible_at,'retained evidence of free capacity during declared wait');
 assert(!box(clients.bob,items.deferred.id));
 milestone('declared-delay-and-priority-advantage',{standard:sr,priority:pr,sovereign:so});
 await command(staff,'@tacsim/time rate=3600');
 await until(()=>box(clients.bob,items.deferred.id),'Deferred off-peak window',60);
 await command(staff,'@tacsim/time rate=24');
 const dr=receipt(items.deferred.envelope),cfg=service().config,phase=((dr.transmitted%cfg.window_period_s)+cfg.window_period_s)%cfg.window_period_s;
 assert(dr.transmitted>=dr.queued+cfg.deferred_min_s);assert(phase>=cfg.window_start_s&&phase<cfg.window_start_s+cfg.window_duration_s);
 const allReceipts=service().receipts;
 const spent=new Map();for(const r of allReceipts){const k=r.pair+':'+r.transmitted;spent.set(k,(spent.get(k)||0)+r.ticket.units);}
 assert([...spent.values()].every(v=>v<=cfg.burst_units));
 milestone('capacity-and-deferred-window',{deferred:dr,maxUnitsPerSecond:Math.max(...spent.values()),retainedReceipts:allReceipts.length});
 const org=catalog.operator;
 await command(clients.carol,'+relay/discover luna_ceres','Permission denied');
 await command(staff,`@org/grant ${clients.carol.object}=${org} relay.inspect`);
 await command(clients.carol,'+relay/load luna_ceres','actual queued');
 await command(clients.carol,'+relay/discover luna_ceres','Evidence filed');
 await command(clients.carol,'+intel','Standard queued');
 assert(!(await command(clients.bob,'+intel')).includes('Standard queued'));
 await command(clients.carol,'+relay/publish luna_ceres','Permission denied');
 await command(staff,`@org/grant ${clients.carol.object}=${org} relay.publish`);
 await command(clients.carol,'+relay/publish luna_ceres','scandal front');
 const front=service().published.luna_ceres;
 assert(redis('mod:faction:front:'+front));
 await crash('publication-commit');await reconnect();
 await command(clients.carol,'+relay/publish luna_ceres','already published');
 assert.equal(service().published.luna_ceres,front);
 await command(staff,'@front','The Exchange congestion board');
 milestone('permissioned-knowledge-and-single-publication',{front});
 // Saturate with real mail/channel/order producers while the exit is jammed.
 await command(staff,'@tacsim/time rate=1');
 jammed=ceres;await command(staff,`@prop/jam #${jammed}=100`);
 await command(staff,`+relay/grant luna_ceres/${staff.object}=priority`);
 channel='R3Radio'+randomBytes(3).toString('hex');
 await command(staff,'@channel/create '+channel,'created');
 for(const flag of ['ic','logged','persist'])await command(staff,`@channel/set ${channel}=${flag}`);
 for(const c of [staff,clients.bob]){await command(c,'delcom r3');await command(c,'addcom r3='+channel,'added');}
 const earlierIds=new Set(envelopes().map(e=>e.id));
 for(let i=0;i<6;i++){
  await command(staff,`r3 R3-channel-${i}-${'c'.repeat(400)}`,'queued for IC transit');
  await command(staff,`+order ${clients.bob.object}=R3-order-${i}-${'o'.repeat(400)}`,'Queued revision');
  await command(staff,`@mail/quick ${clients.bob.object}=R3-mixed-${i}/${'m'.repeat(400)}`,'queued for IC transit');
 }
 const queued=await until(()=>{const rows=envelopes().filter(e=>!earlierIds.has(e.id)&&e.transit?.queued_at);return rows.length>=18&&rows;},'mixed queued producers');
 const kinds=new Set(queued.map(e=>e.kind));assert(kinds.has('mail')&&kinds.has('channel')&&kinds.has('order'));
 const beforeIds=new Set(service().receipts.map(r=>r.envelope));
 await command(staff,`@prop/jam #${jammed}=0`);jammed=null;
 await until(()=>queued.every(e=>receipt(e.id)),'mixed classes drain',60);
 const mixed=service().receipts.filter(r=>!beforeIds.has(r.envelope));
 const bins=new Map();for(const r of mixed){const k=r.pair+':'+r.transmitted;bins.set(k,(bins.get(k)||0)+r.ticket.units);}
 assert([...bins.values()].every(v=>v<=8));
 await command(staff,'@tacsim/time rate=24');
 await until(()=>service().receipts.filter(r=>r.kind==='channel'&&r.recipient===actor(clients.bob)).some(r=>r.arrived),'channel arrivals');
 await command(clients.bob,'@channel/recall r3','R3-channel');
 await command(clients.bob,'+relay/receipts','Channel');
 await command(clients.bob,'delcom r3');
 assert(!(await command(clients.bob,'+relay/receipts')).includes('Channel'));
 await command(clients.bob,'addcom r3='+channel,'added');
 assert(!(await command(clients.bob,'+relay/receipts')).includes('Channel'));
 await command(clients.bob,'pub R3 OOC remains immediate','R3 OOC remains immediate');
 milestone('mixed-producers-and-membership-safe-receipts',{copies:queued.length,maxUnitsPerSecond:Math.max(...bins.values()),classes:[...kinds]});
 await command(staff,`+relay/grant luna_ceres/${staff.object}=standard`);
 evidence.channel=channel;
 evidence.passed=true;
} catch(error){evidence.passed=false;evidence.failure=error.stack;throw error;}
finally{
 try{if(staff?.ws.readyState===WebSocket.OPEN){if(jammed!==null)await command(staff,`@prop/jam #${jammed}=0`);await command(staff,'@tacsim/time rate=24');}}catch{}
 save('live.json',evidence);for(const ws of sockets)ws.close();
}
