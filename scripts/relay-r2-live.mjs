#!/usr/bin/env node
// R2 live acceptance on the explicitly disposable dev1 instance. Node >=22.
// Usage: relay-r2-live.mjs https-url staff-credentials.json output-dir redis-port
// Requires root to kill/restart only dev1's verified server child. Credentials
// stay in the root-only output directory; evidence contains no token/password.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdirSync, realpathSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { join } from 'node:path';
const [target, credentialsPath, output, redisPort] = process.argv.slice(2);
assert(target === 'https://conclav.io:6790' && redisPort === '6788', 'dev1 only');
assert.equal(realpathSync(output), '/opt/rustmush/dev1/r2-evidence');
const credentials = JSON.parse(readFileSync(credentialsPath, 'utf8'));
mkdirSync(output, { recursive: true });
const evidence = { target, at: new Date().toISOString(), checks: [], milestones: [] };
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
  const response = await fetch(target + '/api/auth/' + route, {
    method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body),
    signal:AbortSignal.timeout(15000),
  });
  assert.equal(response.status,200,'authentication endpoint '+route+': '+response.status);
  return response.json();
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
  await command(client,'OUTPUTSUFFIX __R2_DONE__');
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
  await wait(client,f=>text(f.d).includes('__R2_DONE__'),start);
  const reply=client.frames.slice(start).filter(f=>!text(f.d).includes('__R2_DONE__')).map(f=>text(f.d)).join('\n');
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
let staff, visitor;
try {
  staff=await connect(await auth('login',credentials));
  const vc={account:'r2v'+randomBytes(4).toString('hex'),password:randomBytes(24).toString('hex'),char_name:'R2Visitor'+randomBytes(3).toString('hex')};
  save('visitor-credentials.json',vc);
  visitor=await connect(await auth('register',vc));
  const reg=registry(), luna=reg.nodes.malapert.coverage[0], ceres=reg.nodes.ceres.coverage[0], node=reg.nodes.ceres.place;
  await command(staff,'@relay/enable luna_ceres.luna');
  await command(staff,'@relay/check','Live');
  await command(staff,'@teleport me=#'+luna);
  await command(staff,'@teleport '+visitor.object+'=#'+ceres);
  await command(visitor,'@prop/route 999999','Permission denied');
  await command(staff,'@prop/jam #'+node+'=9');
  const pairId=await issue(staff,visitor.object,'R2-pair-crash');
  await command(staff,'@prop/pass');
  await until(()=>envelope(pairId).transit?.queued_at,'durable FIFO ticket');
  const queued=envelope(pairId);
  assert(!(await command(visitor,'+orders')).includes('R2-pair-crash'));
  await command(staff,'@prop/route '+pairId,'FIFO queued');
  await crash('queued-restart');
  staff=await connect(await auth('login',credentials));
  visitor=await connect(await auth('login',vc));
  assert.deepEqual(envelope(pairId),queued,'queue ticket and progress survive process death');
  await command(staff,'@prop/jam #'+node+'=0');
  await command(staff,'@prop/pass');
  await until(()=>envelope(pairId).state==='delivered','pair delivery');
  const arrived=envelope(pairId);
  assert.equal(arrived.path.length,1);
  await command(visitor,'+orders','R2-pair-crash');
  assert(!(await command(staff,'+orders')).includes('Delivered'),'sender has no remote delivery oracle');
  const ackReply=await command(visitor,'+orders/ack '+pairId,'queued');
  const ackId=Number(ackReply.match(/envelope #(\d+)/)[1]);
  await until(()=>envelope(pairId).state==='acknowledged','returning pair ACK');
  assert.equal(envelope(ackId).path.length,1);
  milestone('pair-order-and-returning-ack',{pairId,ackId,queued,arrived,ack:envelope(ackId)});
  // Fill a durable link reservation while jamming the egress; the 65th is refused.
  await command(staff,'@prop/jam #'+node+'=9');
  const ids=[];
  for(let i=0;i<64;i++) ids.push(await issue(staff,visitor.object,'R2-fifo-'+i+'-'+('x'.repeat(480))));
  await command(staff,'+order '+visitor.object+'=R2-overflow','link queue is full');
  assert(!(await command(visitor,'+orders')).includes('R2-fifo-'));
  await command(staff,'@prop/jam #'+node+'=0');
  await until(()=>ids.every(id=>!['queued','transmitted'].includes(envelope(id).state)),'queue drain',180);
  const rows=ids.map(id=>({id,state:envelope(id).state,hop:envelope(id).path[0]}));
  const buckets=new Map();
  for(const row of rows) {
    assert(row.hop,'each accepted copy crossed once');
    const at=row.hop.at.at;
    buckets.set(at,(buckets.get(at)||0)+4);
  }
  assert([...buckets.values()].every(n=>n<=8),'per-IC-second shared capacity');
  assert(rows.every((r,i)=>i===0 || r.hop.at.at>=rows[i-1].hop.at.at),'FIFO order');
  milestone('bounded-fifo',{accepted:64,refused:1,rows,maxUnitsPerIcSecond:Math.max(...buckets.values())});
  // Disable the pair: the order must traverse actual Sky light-time.
  await command(staff,'@relay/disable luna_ceres.luna','revision');
  const fallbackId=await issue(staff,visitor.object,'R2-frozen-light-time');
  await command(staff,'@prop/pass');
  await until(()=>envelope(fallbackId).transit?.flight?.seconds>0,'positive physical leg');
  const frozen=envelope(fallbackId).transit.flight;
  assert(frozen.seconds>60 && frozen.basis.startsWith('sky.v1:'),'stationary strategic metric');
  assert(nowIc()<frozen.due.at,'flight remains in the future');
  await command(staff,'@prop/route '+fallbackId,'Frozen');
  assert(!(await command(visitor,'+orders')).includes('R2-frozen-light-time'));
  await crash('in-flight-restart');
  staff=await connect(await auth('login',credentials));
  visitor=await connect(await auth('login',vc));
  assert.deepEqual(envelope(fallbackId).transit.flight,frozen);
  // Repairing the network cannot accelerate a leg already in flight.
  await command(staff,'@relay/enable luna_ceres.luna','revision');
  await command(staff,'@prop/pass');
  assert(nowIc()<frozen.due.at,'pre-due check is actually before due');
  assert.deepEqual(envelope(fallbackId).transit.flight,frozen);
  assert(!(await command(visitor,'+orders')).includes('R2-frozen-light-time'));
  await until(()=>envelope(fallbackId).state==='delivered','physical arrival',300);
  const fallback=envelope(fallbackId);
  assert(fallback.advanced.at>=frozen.due.at);
  assert.equal(fallback.path.length,0,'no completed physical leg replayed as a pair hop');
  assert.deepEqual(fallback.transit.history[0].flight,frozen);
  const settled=structuredClone(fallback);
  await command(staff,'@prop/pass');
  assert.deepEqual(envelope(fallbackId),settled,'arrival settles once');
  await command(visitor,'+orders','R2-frozen-light-time');
  await command(staff,'@relay/disable luna_ceres.luna','revision');
  const fallbackAckReply=await command(visitor,'+orders/ack '+fallbackId,'queued');
  const fallbackAckId=Number(fallbackAckReply.match(/envelope #(\d+)/)[1]);
  await until(()=>envelope(fallbackAckId).transit?.flight?.seconds>0,'physical return ACK');
  const ackFlight=envelope(fallbackAckId).transit.flight;
  assert.equal(envelope(fallbackId).state,'delivered','ACK has not arrived yet');
  await until(()=>envelope(fallbackId).state==='acknowledged','fallback order ACK',300);
  assert(envelope(fallbackAckId).advanced.at>=ackFlight.due.at);
  assert.equal(envelope(fallbackAckId).path.length,0);
  milestone('frozen-fallback-and-repair',{fallbackId,frozen,arrived:settled,ackFlight,ack:envelope(fallbackAckId),acknowledged:envelope(fallbackId)});
  await command(staff,'@relay/enable luna_ceres.luna','revision');
  // Unknown endpoint remains invisible, then re-enters routing when placed.
  await command(staff,'@teleport '+visitor.object+'=#0');
  const unknown=await issue(staff,visitor.object,'R2-unknown-endpoint');
  await command(staff,'@prop/pass');
  assert.equal(envelope(unknown).transit,undefined);
  assert(!(await command(visitor,'+orders')).includes('R2-unknown-endpoint'));
  await command(staff,'@prop/route '+unknown,'no route');
  await command(staff,'@teleport '+visitor.object+'=#'+ceres);
  await until(()=>envelope(unknown).state==='delivered','endpoint recovery');
  milestone('missing-endpoint-recovers',{unknown,envelope:envelope(unknown)});
  await command(staff,'@relay/check','Live');
  const other=Number(shell('systemctl',['show','dev2mush-gateway','-p','MainPID','--value']));
  const before=Number(readFileSync(join(output,'dev2-before.txt'),'utf8').trim().split('=')[1]);
  assert.equal(other,before,'dev2 process untouched');
  evidence.finalRegistry=registry();
  evidence.capacity=redis('mod:propagation:relay_transport');
  evidence.dev2Pid=other;
  evidence.passed=true;
} catch(error) {
  evidence.passed=false;
  evidence.error=error.stack;
  process.exitCode=1;
} finally {
  save('live.json',evidence);
  for(const ws of sockets) ws.close();
  console.log(JSON.stringify({passed:evidence.passed,checks:evidence.checks.length,error:evidence.error}));
}
