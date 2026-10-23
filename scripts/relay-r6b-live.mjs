#!/usr/bin/env node
// R6b live acceptance on the explicitly disposable dev1 instance. Node >=22.
// Usage: relay-r6b-live.mjs https-url staff-credentials.json output-dir redis-port
// Requires root to kill/restart only dev1's verified server child. Credentials
// stay in the root-only output directory; evidence contains no token/password.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdirSync, realpathSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { join } from 'node:path';
const [target, credentialsPath, output, redisPort] = process.argv.slice(2);
assert(target === 'https://conclav.io:6790' && redisPort === '6788', 'dev1 only');
assert.equal(realpathSync(output), '/opt/rustmush/dev1/r6b-evidence');
const credentials = JSON.parse(readFileSync(credentialsPath, 'utf8'));
mkdirSync(output, { recursive: true });
const prepareOnly=process.env.R6B_PREPARE==='1';
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
  await command(client,'OUTPUTSUFFIX __R6B_DONE__');
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
  await wait(client,f=>text(f.d).includes('__R6B_DONE__'),start);
  const reply=client.frames.slice(start).filter(f=>!text(f.d).includes('__R6B_DONE__')).map(f=>text(f.d)).join('\n');
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

let queryId=6000,staff,staffLogin,clients={},logins={},channel,restoreSource=null;
const objectId=c=>Number(c.object.slice(1));
function milestone(test,data={}) {evidence.milestones.push({test,...data});save('live.json',evidence);console.log('PASS '+test);}
async function connectAll() {
  for(const ws of sockets) if(ws.readyState===WebSocket.OPEN) ws.close();
  await sleep(500);staff=await connect(staffLogin);
  for(const [name,login] of Object.entries(logins)) clients[name]=await connect(login);
}
async function query(client,q) {
  const id=++queryId,start=client.frames.length;
  client.ws.send(JSON.stringify({t:'req',d:{id,q}}));
  const reply=await wait(client,f=>f.t==='reply'&&f.d.id===id,start);
  evidence.checks.push({actor:client.name,query:q,reply:reply.d.event});
  assert.equal(reply.d.event.t,'panel',JSON.stringify(reply));
  return reply.d.event.d.data;
}
const state=c=>query(c,{t:'panel_subscribe',d:{feed:'comsys',params:{}}});
const backlog=(c,name=channel)=>query(c,{t:'channel_backlog',d:{channel:name,before_seq:null,limit:200}});
const knowledge=(c,id)=>redis(`mod:comsys:member:${objectId(c)}:${id}`)?.ic;
const source=(id,seq)=>redis(`mod:comsys:ic:msg:${id}:${seq}`);
const route=(id,seq,c)=>envelopes().find(e=>e.dispatch?.key===`channel:v1:${id}:${seq}:${objectId(c)}`);
async function channelId(name) {return (await until(()=>{
 const keys=shell('redis-cli',['-p',redisPort,'--scan','--pattern','mod:comsys:chan:*']).split('\n').filter(Boolean);
 return keys.map(redis).find(c=>c?.name===name);
},'channel persisted')).id;}
async function hidden(c,marker,seq) {
 const data=await state(c),meta=data.channels.find(c=>c.name===channel);
 assert.equal(meta.seq,seq,'only arrived sequence visible');
 assert(!JSON.stringify(await backlog(c)).includes(marker),'no early panel history');
 assert(!(await command(c,'@channel/recall r6b')).includes(marker),'no early recall');
 assert(!c.frames.some(f=>(f.t==='channel'||(f.t==='notice'&&f.d.category==='mention'))&&text(f.d).includes(marker)),'no early live line or mention');
}
try {
 staffLogin=await auth('login',credentials);
 const accounts=JSON.parse(readFileSync('/opt/rustmush/dev1/r6a-evidence/actors.json','utf8'));
 for(const [name,c] of Object.entries(accounts)) logins[name]=await auth('login',c);
 await connectAll();
 if(prepareOnly) {
  channel='R6bLegacy'+randomBytes(3).toString('hex');
  await command(staff,'@channel/create '+channel,'created');
  for(const flag of ['ic','logged','persist']) await command(staff,`@channel/set ${channel}=${flag}`);
  for(const c of [staff,clients.bob]) await command(c,'addcom legacy='+channel,'added');
  await command(staff,'legacy legacy-already-delivered');
  const id=await channelId(channel);
  assert.equal(knowledge(clients.bob,id),undefined,'old server has no transit knowledge');
  assert((await command(clients.bob,'@channel/recall legacy')).includes('legacy-already-delivered'));
  save('preupgrade.json',{channel,id});milestone('legacy-prepared',{channel,id});
 } else {
  const old=JSON.parse(readFileSync(join(output,'preupgrade.json'),'utf8'));
  assert(JSON.stringify(await backlog(clients.bob,old.channel)).includes('legacy-already-delivered'));
  assert(!envelopes().some(e=>e.dispatch?.key?.startsWith(`channel:v1:${old.id}:`)),'legacy history not resent');
  milestone('upgrade',old);
  const catalog=registry();
  const pre=JSON.parse(readFileSync('/opt/rustmush/dev1/r6a-evidence/preupgrade.json','utf8'));
  await command(staff,'@teleport me=#'+catalog.nodes.malapert.coverage[0]);
  await command(staff,'@teleport '+clients.bob.object+'=#'+catalog.nodes.ceres.coverage[0]);
  await command(staff,'@teleport '+clients.carol.object+'=#'+pre.stickney);
  await command(staff,'@tacsim/time rate=1');
  await command(staff,'@relay/enable luna_ceres.luna');
  channel='R6bRadio'+randomBytes(3).toString('hex');
  await command(staff,'@channel/create '+channel,'created');
  for(const flag of ['ic','logged','persist','loud','relay']) await command(staff,`@channel/set ${channel}=${flag}`);
  for(const c of [staff,...Object.values(clients)]) {await command(c,'delcom r6b');await command(c,'addcom r6b='+channel,'added');}
  const id=await channelId(channel);evidence.channel={id,name:channel};save('live.json',evidence);
  await command(staff,'@relay/disable luna_ceres.luna');
  const first='first-hidden-'+randomBytes(3).toString('hex');
  await command(staff,`r6b ${first} ${clients.bob.name} ${clients.carol.name}`,'queued for IC transit');
  assert(source(id,1),'fanout intent durable at acceptance');
  const frozen=await until(()=>route(id,1,clients.bob)?.transit?.flight,'slow physical flight');
  await hidden(clients.bob,first,0);await hidden(clients.carol,first,0);
  await crash('restart-fanout');await connectAll();
  assert.deepEqual(route(id,1,clients.bob).transit.flight,frozen);
  await hidden(clients.bob,first,0);
  await command(staff,'@relay/enable luna_ceres.luna');
  await command(staff,'r6b second-fast','queued for IC transit');
  await until(()=>knowledge(clients.bob,id)?.sequence===1,'fast second copy');
  assert.equal(knowledge(clients.bob,id).received[0].message,2);
  await hidden(clients.bob,first,1);
  const early=await backlog(clients.bob);assert(JSON.stringify(early).includes('second-fast'));
  await hidden(clients.carol,'second-fast',0);
  const ship=await until(()=>{
   const t=route(id,2,clients.alice)?.transit;
   return [t?.flight,...(t?.history??[]).map(h=>h.flight)].find(f=>f?.basis?.startsWith('ship.v1:')&&f.seconds>0);
  },'actual underway Dray flight');
  milestone('overtake-and-moving-recipient',{frozen,ship,bob:early});
  // Bob is offline when the older first line arrives; receipt/unread survive.
  clients.bob.ws.close();await sleep(600);
  await command(staff,'@tacsim/time rate=24');
  await until(()=>knowledge(clients.bob,id)?.sequence===2,'offline older arrival');
  assert.equal(redis(`mod:comsys:member:${objectId(clients.bob)}:${id}`).last_read,1);
  clients.bob=await connect(logins.bob);
  const caught=await backlog(clients.bob);
  assert.equal(caught.backlog.messages.length,2);assert.equal(caught.backlog.messages[0].seq,2);
  assert(caught.backlog.messages[0].text.includes(first));
  await until(()=>knowledge(clients.carol,id)?.sequence===2,'uncovered destination arrival');
  milestone('offline-arrival-and-uncovered',{backlog:caught});
  // A second slow physical copy cannot cross a leave/rejoin boundary.
  await command(staff,'@tacsim/time rate=1');await command(staff,'@relay/disable luna_ceres.luna');
  const beforeEpoch=knowledge(clients.bob,id).epoch;
  await command(staff,'r6b before-rejoin','queued for IC transit');
  await until(()=>route(id,3,clients.bob)?.transit?.flight,'rejoin flight');
  await command(clients.bob,'delcom r6b','removed');await command(clients.bob,'addcom r6b='+channel,'added');
  assert.notEqual(knowledge(clients.bob,id).epoch,beforeEpoch);assert.equal(knowledge(clients.bob,id).sequence,0);
  await command(staff,'@tacsim/time rate=24');await until(()=>!source(id,3)?.pending.includes(objectId(clients.bob)),'rejoin rejection');
  assert(!JSON.stringify(await backlog(clients.bob)).includes('before-rejoin'));
  milestone('leave-rejoin',{oldEpoch:beforeEpoch,newEpoch:knowledge(clients.bob,id).epoch});
  await command(staff,'@tacsim/time rate=1');
  await command(staff,'r6b revoked-on-arrival','queued for IC transit');
  await until(()=>route(id,4,clients.bob)?.transit?.flight,'revocation flight');
  await command(staff,`@channel/perm ${channel}=see=#FALSE`);
  await command(staff,'@tacsim/time rate=24');await until(()=>!source(id,4)?.pending.includes(objectId(clients.bob)),'arrival refusal');
  await command(staff,`@channel/perm ${channel}=see=#TRUE`);
  assert(!JSON.stringify(await backlog(clients.bob)).includes('revoked-on-arrival'));
  milestone('arrival-revocation');
  await command(staff,'@relay/enable luna_ceres.luna');
  await command(clients.alice,'r6b from-underway-hull','queued for IC transit');
  const outbound=await until(()=>{
   const t=route(id,5,clients.bob)?.transit;
   return [t?.flight,...(t?.history??[]).map(h=>h.flight)].find(f=>f?.basis?.startsWith('ship.v1:')&&f.seconds>0);
  },'physical outbound ship flight');
  await until(()=>knowledge(clients.bob,id)?.sequence===1,'ship-to-shore arrival');
  assert(JSON.stringify(await backlog(clients.bob)).includes('from-underway-hull'));
  await command(clients.bob,'pub R6b OOC immediate','R6b OOC immediate');
  await command(staff,'@channel/transit');
  milestone('ship-source-and-ooc',{outbound});
  await command(staff,'@tacsim/time rate=1');
  await command(clients.carol,'r6b source-survives-object-loss','queued for IC transit');
  await until(()=>route(id,6,clients.bob)?.transit?.flight,'source-loss physical flight');
  restoreSource={key:'obj:'+objectId(clients.carol),value:redis('obj:'+objectId(clients.carol))};
  save('source-backup.json',restoreSource);
  shell('systemctl',['stop','dev1mush-gateway']);
  shell('redis-cli',['-p',redisPort,'DEL',restoreSource.key]);
  const carolLogin=logins.carol;delete logins.carol;
  shell('systemctl',['start','dev1mush-gateway']);await sleep(3000);
  staffLogin=await auth('login',credentials);for(const name of Object.keys(logins)) logins[name]=await auth('login',accounts[name]);
  await connectAll();await command(staff,'@tacsim/time rate=24');
  await until(()=>knowledge(clients.bob,id)?.sequence===2,'delivery after source object loss');
  assert(JSON.stringify(await backlog(clients.bob)).includes('source-survives-object-loss'));
  shell('systemctl',['stop','dev1mush-gateway']);
  shell('redis-cli',['-p',redisPort,'JSON.SET',restoreSource.key,'$',JSON.stringify(restoreSource.value)]);
  restoreSource=null;logins.carol=carolLogin;
  shell('systemctl',['start','dev1mush-gateway']);await sleep(3000);
  staffLogin=await auth('login',credentials);for(const name of Object.keys(logins)) logins[name]=await auth('login',accounts[name]);await connectAll();
  await command(clients.carol,'addcom pub=Public');await command(clients.carol,'addcom r6b='+channel);
  milestone('source-object-loss',{source:objectId(clients.carol),controlledFault:true,restored:true});
  evidence.open=['Live Discord provider integration: dev1 has no enabled bot or configured relay mapping; real bridge controlled-provider test is recorded separately.'];
 }
 evidence.passed=true;
} catch(error) {evidence.passed=false;evidence.error=String(error.stack??error);console.error(evidence.error);process.exitCode=1;}
finally {
 if(restoreSource){
  shell('systemctl',['stop','dev1mush-gateway']);
  shell('redis-cli',['-p',redisPort,'JSON.SET',restoreSource.key,'$',JSON.stringify(restoreSource.value)]);
  shell('systemctl',['start','dev1mush-gateway']);
 }
 if(!prepareOnly&&staff?.ws.readyState===WebSocket.OPEN){try {await command(staff,'@tacsim/time rate=24');await command(staff,'@relay/enable luna_ceres.luna');}catch{}}
 save(prepareOnly?'prepare.json':'live.json',evidence);
 for(const ws of sockets) ws.close();
}
