#!/usr/bin/env node
// Shared relay live acceptance on the explicitly disposable dev1 instance. Node >=22.
// Usage: relay-r3-live.mjs https-url staff-credentials.json output-dir redis-port
// Requires root to kill/restart only dev1's verified server child. Credentials
// stay in the root-only output directory; evidence contains no token/password.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdirSync, realpathSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { join } from 'node:path';
assert(Number(process.versions.node.split('.')[0]) >= 22, 'relay live acceptance requires Node >=22');
const [target, credentialsPath, output, redisPort] = process.argv.slice(2);
assert(target === 'https://conclav.io:6790' && redisPort === '6788', 'dev1 only');
assert(/^\/opt\/rustmush\/dev1\/(?:r[0-9]+[ab]?-evidence(?:\/acceptance-repair)?|availability-evidence|policy-evidence)$/.test(realpathSync(output)), 'dev1 gate evidence only');
const credentials = JSON.parse(readFileSync(credentialsPath, 'utf8'));
mkdirSync(output, { recursive: true });
const evidence={target,at:new Date().toISOString(),checks:[],milestones:[]};
const sockets = [];
const sleep = ms => new Promise(r => setTimeout(r, ms));
const shell = (cmd, args) => execFileSync(cmd, args, {encoding:'utf8',timeout:30000}).trim();
const mode=process.env.RELAY_GATE_MODE || 'split';
assert(['split','all-in-one'].includes(mode),'known deployment mode');
if(mode==='all-in-one') assert.equal(realpathSync(output),'/opt/rustmush/dev1/r10-evidence','all-in-one belongs to the R10 dev1 gate');
const serviceName=mode==='split'?'dev1mush-gateway':'rustmush-r10-allinone';
const expectedBinary='/opt/rustmush/dev1/build/target/release/'+(mode==='split'?'mush-server':'mushd');
function processIds(){
 const main=Number(shell('systemctl',['show',serviceName,'-p','MainPID','--value']));
 assert(main>1,'gate service is running');
 const child=mode==='split'?Number(shell('pgrep',['-P',String(main),'-x','mush-server'])):main;
 assert.equal(realpathSync('/proc/'+child+'/exe'),expectedBinary);
 return {main,child};
}
const initialProcess=processIds(),gatewayPid=mode==='split'?initialProcess.main:null,serverPid=initialProcess.child;
evidence.mode=mode;
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
  await command(client,'OUTPUTSUFFIX __RELAY_DONE__');
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
  await wait(client,f=>text(f.d).includes('__RELAY_DONE__'),start);
  const reply=client.frames.slice(start).filter(f=>!text(f.d).includes('__RELAY_DONE__')).map(f=>text(f.d)).join('\n');
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
  const service=serviceName;
  const before=processIds(),gateway=before.main,old=before.child;
  const cursor=shell('journalctl',['-u',service,'-n','0','--show-cursor','--no-pager']).split('-- cursor: ')[1];
  assert(cursor,'journal cursor');
  beforeKill();
  process.kill(old,'SIGKILL');
  const fresh=await until(()=>{
    try {
      const pid=processIds().child;
      return pid!==old && pid;
    } catch {return false;}
  },'replacement server',60);
  await until(()=>{
    const logs=shell('journalctl',['-u',service,'--after-cursor',cursor,'--no-pager','-o','cat']);
    if(mode==='split') return logs.includes('gateway connected');
    try {shell('curl',['-fsS','--cacert','/opt/rustmush/dev1/tls/cert.pem',target+'/']);return true;} catch {return false;}
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

const actor=c=>Number(c.object.slice(1));
const transport=()=>redis('mod:propagation:relay_transport');
const service=()=>transport().service;
const mail=id=>redis('mod:mail:msg:'+id);
const box=(c,id)=>redis(`mod:mail:box:${actor(c)}:${id}`);
const receipt=id=>service().receipts.find(r=>r.envelope===id);
function milestone(test,data={}) { evidence.milestones.push({test,...data}); save('live.json',evidence); console.log('PASS '+test); }
function close() { save('live.json', evidence); for(const ws of sockets) ws.close(); }
export { assert,readFileSync,writeFileSync,randomBytes,evidence,credentials,sockets,sleep,shell,redis,envelope,registry,save,text,auth,connect,wait,command,until,crash,envelopes,actor,transport,service,mail,box,receipt,milestone,close };
