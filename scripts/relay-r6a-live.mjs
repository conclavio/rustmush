#!/usr/bin/env node
// R6a live acceptance on the explicitly disposable dev1 instance. Node >=22.
// Usage: relay-r6a-live.mjs https-url staff-credentials.json output-dir redis-port
// Requires root to kill/restart only dev1's verified server child. Credentials
// stay in the root-only output directory; evidence contains no token/password.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdirSync, realpathSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { join } from 'node:path';
const [target, credentialsPath, output, redisPort] = process.argv.slice(2);
assert(target === 'https://conclav.io:6790' && redisPort === '6788', 'dev1 only');
assert.equal(realpathSync(output), '/opt/rustmush/dev1/r6a-evidence');
const credentials = JSON.parse(readFileSync(credentialsPath, 'utf8'));
mkdirSync(output, { recursive: true });
const resuming=process.env.R6A_RESUME==='1';
const evidence = resuming ? JSON.parse(readFileSync(join(output,'live.json'),'utf8'))
  : { target, at: new Date().toISOString(), checks: [], milestones: [] };
evidence.candidate=readFileSync('/opt/rustmush/dev1/build/.source-commit','utf8').trim();
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
  await command(client,'OUTPUTSUFFIX __R6A_DONE__');
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
  await wait(client,f=>text(f.d).includes('__R6A_DONE__'),start);
  const reply=client.frames.slice(start).filter(f=>!text(f.d).includes('__R6A_DONE__')).map(f=>text(f.d)).join('\n');
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
function milestone(test, facts) {
  evidence.milestones.push({test,...facts});
  save('live.json',evidence);
  console.log(JSON.stringify({completed:test,...facts}));
}

const prepareOnly=process.env.R6A_PREPARE==='1';
const actorsPath=join(output,'actors.json');
const objectId=client=>Number(client.object.slice(1));
const mail=id=>redis('mod:mail:msg:'+id);
const box=(client,id)=>redis('mod:mail:box:'+objectId(client)+':'+id);
const pending=id=>redis('mod:mail:dispatch:'+id);
function envelopes() {
  const keys=shell('redis-cli',['-p',redisPort,'--scan','--pattern','mod:propagation:envelope:*']).split('\n').filter(Boolean);
  assert(keys.length<=4096);
  if(!keys.length) return [];
  return shell('redis-cli',['-p',redisPort,'--raw','JSON.MGET',...keys,'.']).split('\n').filter(Boolean).map(JSON.parse).filter(Boolean);
}
const dispatch=(id,to,returning=false)=>envelopes().find(e=>e.dispatch?.key===`mail:v1:${id}:${objectId(to)}:${returning?'return':'out'}`);
let queryId=6000, staff, staffLogin, accounts, clients={}, logins={}, restoreFault=null;
async function panel(client,folder='inbox') {
  const id=++queryId, start=client.frames.length;
  client.ws.send(JSON.stringify({t:'req',d:{id,q:{t:'mail_list',d:{folder}}}}));
  const reply=await wait(client,f=>f.t==='reply' && f.d.id===id,start);
  assert.equal(reply.d.event.t,'panel');
  evidence.checks.push({actor:client.name,query:{folder},reply:reply.d.event.d,wallMs:0,replyBytes:Buffer.byteLength(JSON.stringify(reply.d))});
  return reply.d.event.d.data;
}
async function hidden(client,id,subject) {
  assert(!box(client,id),'no early mailbox entry');
  const data=await panel(client);
  assert(!data.entries.some(e=>e.id===id),'no early hydration');
  // Replies can share a subject with already-delivered letters. Their stable
  // identity must remain absent; a prior letter is still allowed to be read.
  const existingSubject=data.entries.some(e=>e.subject===subject);
  if(!existingSubject) assert(!JSON.stringify(data).includes(subject),'no early subject/thread');
  const listing=await command(client,'@mail');
  if(!existingSubject) assert(!listing.includes(subject),'no early line list');
  const read=await command(client,'@mail '+(data.entries.length+1));
  assert(!read.includes(subject),'no early line read');
  assert(!client.frames.some(f=>f.t==='mail_notice'&&f.d.id===id),'no early notice');
}
async function connectAll() {
  // The gateway preserves sockets across a child crash. Close those held
  // sessions before reconnecting, otherwise the harness hits its own IP cap.
  for(const ws of sockets) if(ws.readyState===WebSocket.OPEN) ws.close();
  await sleep(500);
  staff=await connect(staffLogin);
  for(const [name,login] of Object.entries(logins)) clients[name]=await connect(login);
}
async function queued(from,to,subject,body='Private letter') {
  const reply=await command(from,`@mail/quick ${to.map(c=>c.object).join(',')}=${subject}/${body}`,'queued for IC transit');
  const id=Number(reply.match(/Message (\d+) queued/)[1]);
  assert(mail(id)&&pending(id),'source acceptance is durable');
  return id;
}
async function received(id,to) { return until(()=>box(to,id),'mailbox insertion '+id,180); }
async function confirmed(id,to,outcome='delivered') {
  await until(()=>mail(id)?.delivery?.[objectId(to)]===outcome,'return confirmation '+id,180);
}
function roomNamed(name) {
  const keys=shell('redis-cli',['-p',redisPort,'--scan','--pattern','obj:*']).split('\n').filter(Boolean);
  for(const key of keys) { const value=redis(key); if(value?.name===name) return Number(key.slice(4)); }
  throw new Error('Room not found: '+name);
}
try {
  staffLogin=await auth('login',credentials); staff=await connect(staffLogin);
  if(prepareOnly) {
    accounts={};
    for(const name of ['alice','bob','carol']) {
      const c={account:'r6a'+name+randomBytes(3).toString('hex'),password:randomBytes(24).toString('hex'),char_name:'R6'+name+randomBytes(3).toString('hex')};
      accounts[name]=c; save('actors.json',accounts);
      logins[name]=await auth('register',c); clients[name]=await connect(logins[name]);
    }
    const catalog=registry();
    const ceres=catalog.nodes.ceres.coverage[0];
    const stickney=roomNamed('Kasei Receiving Office');
    for(const name of ['alice','carol']) await command(staff,`@teleport ${clients[name].object}=#${ceres}`);
    await command(staff,`@teleport ${clients.bob.object}=#${stickney}`);
    const legacy=await command(clients.alice,`@mail/quick ${clients.bob.object}=R6a legacy/pre-upgrade letter`,'sent to');
    const id=Number(legacy.match(/Message (\d+) sent/)[1]);
    assert(box(clients.bob,id));
    save('preupgrade.json',{id,message:mail(id),entry:box(clients.bob,id),actors:Object.fromEntries(Object.entries(clients).map(([k,c])=>[k,{id:objectId(c),name:c.name}])),ceres,stickney});
    evidence.passed=true;
  } else {
    accounts=JSON.parse(readFileSync(actorsPath,'utf8'));
    for(const [name,c] of Object.entries(accounts)) logins[name]=await auth('login',c);
    await connectAll();
    const old=JSON.parse(readFileSync(join(output,'preupgrade.json'),'utf8'));
    if(!done('upgrade')) {
      assert.deepEqual(mail(old.id),old.message,'legacy message bytes preserved');
      assert.deepEqual(box(clients.bob,old.id),old.entry,'legacy mailbox preserved');
      assert(!dispatch(old.id,clients.bob),'legacy letter not resent');
      await command(clients.bob,'@mail 1','pre-upgrade letter');
      milestone('upgrade',{id:old.id});
    }
    await command(staff,'@tacsim/time rate=24');
    if(!done('durable-boundaries')) {
      await until(()=>!shell('redis-cli',['-p',redisPort,'--scan','--pattern','mod:mail:dispatch:*']),'previous attempt drained');
      const unreadBefore=(await panel(clients.bob)).folders.find(f=>f.name==='inbox').unread;
      const subject='R6a private '+randomBytes(3).toString('hex');
      const id=await queued(clients.alice,[clients.bob],subject);
      await crash('source-acceptance',()=>{assert(pending(id));assert(!dispatch(id,clients.bob),'kill before transport accepts');});
      await connectAll();
      await hidden(clients.bob,id,subject);
      const outbound=await until(()=>dispatch(id,clients.bob)?.transit?.flight && dispatch(id,clients.bob),'outbound flight');
      assert(outbound.transit.flight.seconds>0);
      const frozen=outbound.transit.flight;
      await crash('transport-acceptance',()=>assert.deepEqual(dispatch(id,clients.bob).transit.flight,frozen));
      await connectAll();
      assert.deepEqual(dispatch(id,clients.bob).transit.flight,frozen,'frozen leg survives SIGKILL');
      await hidden(clients.bob,id,subject);
      // Delete the exact Sent copy by panel ordinal; payload must remain.
      const sent=await panel(clients.alice,'sent');
      const ordinal=sent.entries.find(e=>e.id===id).index;
      await command(clients.alice,'@mail/folder sent'); await command(clients.alice,'@mail/delete '+ordinal);
      await command(clients.alice,'@mail/purge'); assert(mail(id),'pending payload survives Sent purge');
      await command(clients.alice,'@mail/folder inbox');
      const receipt=await until(()=>{const e=dispatch(id,clients.bob);return e?.state==='delivered'&&e;},'arrival receipt');
      assert(receipt.advanced.at>=frozen.due.at);
      await crash('arrival-receipt',()=>assert(!box(clients.bob,id),'receipt committed before mailbox insertion'));
      await connectAll();
      await received(id,clients.bob);
      const entry=box(clients.bob,id);
      assert.equal(entry.status,'unread');
      await crash('mailbox-insertion',()=>assert.deepEqual(box(clients.bob,id),entry));
      await connectAll();
      assert.deepEqual(box(clients.bob,id),entry,'mailbox applies receipt once');
      const data=await panel(clients.bob);
      assert.equal(data.entries.filter(e=>e.id===id).length,1);
      assert.equal(data.folders.find(f=>f.name==='inbox').unread,unreadBefore+1,'one unread increment across all crashes');
      await confirmed(id,clients.bob);
      milestone('durable-boundaries',{id,frozen,receipt,entry,unread:data.folders.find(f=>f.name==='inbox').unread});
    }
    if(!done('send-paths')) {
      const seed=await queued(clients.alice,[clients.bob,clients.carol],'R6a reply chain');
      await received(seed,clients.bob); await received(seed,clients.carol);
      let list=await panel(clients.bob); const index=list.entries.find(e=>e.id===seed).index;
      const ids=[];
      for(const commandName of ['reply','replyall']) {
        await command(clients.bob,`@mail/${commandName} ${index}`,'Composing');
        await command(clients.bob,'R6a '+commandName+' body');
        const reply=await command(clients.bob,'--','queued for IC transit');
        const id=Number(reply.match(/Message (\d+) queued/)[1]); ids.push(id);
        await hidden(clients.alice,id,'Re: R6a reply chain');
        await received(id,clients.alice); await confirmed(id,clients.alice);
        if(commandName==='replyall') await received(id,clients.carol);
      }
      const fwd=await command(clients.bob,`@mail/fwd ${index}=${clients.alice.object}`,'queued for IC transit');
      const forward=Number(fwd.match(/Message (\d+) queued/)[1]);
      await received(forward,clients.alice); await confirmed(forward,clients.alice);
      const self=await queued(clients.alice,[clients.alice],'R6a self');
      await received(self,clients.alice); await confirmed(self,clients.alice);
      assert.equal(box(clients.alice,self).folder,'inbox');
      milestone('send-paths',{seed,ids,forward,self});
    }
    if(!done('large-letter-offline')) {
      // Two covered endpoints would use a pair for a short letter.
      const luna=registry().nodes.malapert.coverage[0];
      await command(staff,`@teleport ${clients.bob.object}=#${luna}`);
      await command(clients.alice,`@mail/send ${clients.bob.object}=R6a full-sized letter`,'Composing');
      for(let i=0;i<511;i++) await command(clients.alice,'x'.repeat(127));
      await command(clients.alice,'x'.repeat(128));
      const sent=await command(clients.alice,'--','queued for IC transit');
      const id=Number(sent.match(/Message (\d+) queued/)[1]);
      assert.equal(Buffer.byteLength(mail(id).body),65536);
      await hidden(clients.bob,id,'R6a full-sized letter');
      clients.bob.ws.close();
      const outbound=await until(()=>dispatch(id,clients.bob)?.transit?.flight&&dispatch(id,clients.bob),'bulk radio flight');
      assert(outbound.transit.legs.every(l=>!l.pair));
      await received(id,clients.bob);
      clients.bob=await connect(logins.bob);
      const listing=await panel(clients.bob);
      assert(listing.entries.some(e=>e.id===id));
      await confirmed(id,clients.bob);
      milestone('large-letter-offline',{id,bytes:65536,flight:outbound.transit.flight});
      await command(staff,`@teleport ${clients.bob.object}=#${old.stickney}`);
    }

    if(!done('changed-lock') && process.env.R6A_DEFER_LOCK!=='1') {
      const id=await queued(clients.alice,[clients.bob],'R6a lock changes');
      await command(clients.bob,'@lock/mail me=#FALSE','Locked.');
      const outbound=await until(()=>dispatch(id,clients.bob)?.transit?.flight&&dispatch(id,clients.bob),'lock test flight');
      assert(!mail(id).delivery[objectId(clients.bob)]);
      const returning=await until(()=>dispatch(id,clients.bob,true)?.transit?.flight&&dispatch(id,clients.bob,true),'failure return flight');
      assert(!box(clients.bob,id)); assert(!mail(id).delivery[objectId(clients.bob)],'failure not known before return');
      await confirmed(id,clients.bob,'refused');
      await command(clients.bob,'@unlock/mail me','Unlocked.');
      milestone('changed-lock',{id,outbound:outbound.transit.flight,returning:returning.transit.flight});
    }
    if(!done('changed-ignore')) {
      const id=await queued(clients.alice,[clients.bob],'R6a ignore changes');
      await command(clients.bob,'@ignore '+clients.alice.object,'now ignoring');
      await until(()=>dispatch(id,clients.bob)?.transit?.flight,'ignore test flight');
      assert(!mail(id).delivery[objectId(clients.bob)]);
      await confirmed(id,clients.bob,'refused');
      assert(!box(clients.bob,id));
      await command(clients.bob,'@ignore/clear '+clients.alice.object,'no longer ignoring');
      milestone('changed-ignore',{id});
    }
    async function restartGateway() {
      shell('systemctl',['start','dev1mush-gateway']);
      await until(()=>{try{return shell('curl',['-fsS','--cacert','/opt/rustmush/dev1/tls/cert.pem',target+'/']).length>0;}catch{return false;}},'HTTPS ready',60);
      staffLogin=await auth('login',credentials);
      for(const [name,c] of Object.entries(accounts)) logins[name]=await auth('login',c);
      await connectAll();
    }
    async function restore() {
      if(!restoreFault) return;
      shell('systemctl',['stop','dev1mush-gateway']);
      if(restoreFault.value===null) shell('redis-cli',['-p',redisPort,'DEL',restoreFault.key]);
      else shell('redis-cli',['-p',redisPort,'JSON.SET',restoreFault.key,'$',JSON.stringify(restoreFault.value)]);
      restoreFault=null;
      await restartGateway();
    }
    if(!done('full-mailbox')) {
      const id=await queued(clients.alice,[clients.bob],'R6a full mailbox');
      restoreFault={key:'mod:mail:config',value:redis('mod:mail:config')}; save('config-backup.json',restoreFault);
      shell('systemctl',['stop','dev1mush-gateway']);
      shell('redis-cli',['-p',redisPort,'JSON.SET',restoreFault.key,'$',JSON.stringify({quota:0})]);
      await restartGateway();
      assert(!box(clients.bob,id));
      await confirmed(id,clients.bob,'full');
      assert(!box(clients.bob,id));
      await restore();
      milestone('full-mailbox',{id});
    }
    if(!done('destroyed-recipient')) {
      await command(staff,`@teleport ${clients.carol.object}=#${old.stickney}`);
      const id=await queued(clients.alice,[clients.carol],'R6a vanished recipient');
      await until(()=>dispatch(id,clients.carol)?.transit?.flight,'vanishing recipient flight');
      // Isolated object-loss fault, with the exact fixture object restored after
      // its failed delivery. The frozen addressed endpoint still takes time.
      restoreFault={key:'obj:'+objectId(clients.carol),value:redis('obj:'+objectId(clients.carol))};
      save('recipient-backup.json',restoreFault);
      shell('systemctl',['stop','dev1mush-gateway']);
      shell('redis-cli',['-p',redisPort,'DEL',restoreFault.key]);
      // Avoid trying to attach the deliberately absent character until restored.
      const saved=logins.carol; delete logins.carol;
      shell('systemctl',['start','dev1mush-gateway']);
      await sleep(3000);
      staffLogin=await auth('login',credentials); staff=await connect(staffLogin);
      logins.alice=await auth('login',accounts.alice); clients.alice=await connect(logins.alice);
      await confirmed(id,clients.carol,'unavailable');
      assert(!box(clients.carol,id));
      logins.carol=saved;
      await restore();
      milestone('destroyed-recipient',{id});
    }
    if(!done('saturation-expiry')) {
      await command(staff,`@teleport ${clients.carol.object}=#0`);
      const ids=[];
      // The source's 64-letter bound is enforced before transport acknowledgement.
      for(let i=0;i<64;i++) ids.push(await queued(clients.alice,[clients.carol],'R6a expiry '+i));
      const before=redis('mod:mail:next_id');
      await command(clients.alice,`@mail/quick ${clients.carol.object}=overload/no payload`,'queue is full');
      assert.equal(redis('mod:mail:next_id'),before,'overload allocates nothing');
      const first=ids[0];
      const sent=await panel(clients.alice,'sent');
      const ordinal=sent.entries.find(e=>e.id===first).index;
      await command(clients.alice,'@mail/folder sent'); await command(clients.alice,'@mail/delete '+ordinal); await command(clients.alice,'@mail/purge');
      await command(clients.alice,'@mail/folder inbox');
      assert(mail(first));
      const expiry=pending(first).at.at+7*86400+120;
      await command(staff,'@tacsim/time rate=3600');
      await until(()=>nowIc()>=expiry,'real IC expiry',240);
      await command(staff,'@tacsim/time rate=24');
      await until(()=>ids.every(id=>!pending(id)),'all expiry receipts settled',180);
      assert.equal(mail(first),null,'orphan payload reaped after handoffs finish');
      for(const id of ids) assert(!box(clients.carol,id));
      await command(staff,`@teleport ${clients.carol.object}=#${old.ceres}`);
      milestone('saturation-expiry',{accepted:ids.length,refused:1,first,expiry});
    }
    if(!done('moving-hull')) {
      // Depart a new Dray through the actual registry, market and clearance.
      const pilot=clients.alice;
      const portMap=()=>redis('mod:tacsim:tacsim:port:tacsim.lane.ceres_phobos/ceres_port');
      const before=portMap();
      let row=evidence.charter, ticket;
      if(!done('moving-departure')) {
      await command(staff,`@teleport ${pilot.object}=#${roomNamed('Registry & Co-op Desk')}`);
      if(!row) {
      const name='Mail Dray '+randomBytes(3).toString('hex');
      await command(pilot,'+coop/enrol '+name,'now the '+name);
      row=before.hulls.find(r=>JSON.stringify(redis('obj:'+r.hull)).includes(name));
      assert(row,'real chartered hull');
      evidence.charter=row; save('live.json',evidence);
      }
      for(const move of ['out','concourse','exchange']) await command(pilot,move);
      function listing(good,vendor) {
        const keys=shell('redis-cli',['-p',redisPort,'--scan','--pattern','mod:market:market:listing:*']).split('\n').filter(Boolean);
        assert(keys.length<1024);
        const rows=keys.map(redis);
        const item=rows.find(r=>r.vendor===vendor&&r.catalog.code===good&&!r.source.wanted);
        assert(item,'real market listing '+good);return item.id;
      }
      async function buy(good,vendor,qty) {
        const quote=await command(pilot,`+market/quote #${listing(good,vendor)}=${qty}`,'Quote #');
        await command(pilot,'+market/accept #'+Number(quote.match(/Quote #(\d+)/)[1]),'Agreed.');
        await sleep(1000);
      }
      await command(pilot,'+market/list');
      await buy('metals',before.vendors.freeport_exchange.vendor,10);
      await command(pilot,'chandlery');
      for(let i=0;i<8;i++) {
        const plan=await command(pilot,'+ship/plan stickney');
        const burn=plan.match(/Burn (\d+) t.*\(tank (\d+)/);
        const food=plan.match(/Stores (\d+) crew-days.*\(locker (\d+)/);
        assert(burn&&food,'planner figures: '+plan);
        if(+burn[2]>=+burn[1] && +food[2]>=+food[1]) break;
        if(+burn[2]<+burn[1]) await buy('propellant',before.vendors.occator.vendor,+burn[1]-+burn[2]);
        if(+food[2]<+food[1]) await buy('provisions',before.vendors.coop.vendor,+food[1]-+food[2]);
      }
      await sleep(65000);
      assert((await command(pilot,'+ship/hold')).toLowerCase().includes('metal'));
      for(const move of ['exchange','concourse','arrivals','lobby','gallery']) await command(pilot,move);
      await command(pilot,'+ship/board','cycle the lock');
      await command(pilot,'+ship/undock stickney','Clearance filed');
      const ticketKey='mod:tacsim:action-state/v1/tacsim.ticket/'+row.hull+':1';
      ticket=(await until(()=>redis(ticketKey),'burned ticket',90)).value;
      await until(()=>redis('mod:sim:cjourney:'+row.hull)?.state?.state==='Cruising','real departure',90);
      milestone('moving-departure',{hull:row.hull,ticket});
      } else {
        ticket=evidence.milestones.find(m=>m.test==='moving-departure').ticket;
      }
      const first=ticket.plan.legs[0], checkpoint=first.t0+Math.floor(first.secs/4);
      if(checkpoint-nowIc()>10000) {
        await command(staff,'@tacsim/time rate=3600');
        await until(()=>nowIc()>=checkpoint-7200,'quarter-leg approach',1800);
      }
      if(checkpoint-nowIc()>720) {
        await command(staff,'@tacsim/time rate=360');
        await until(()=>nowIc()>=checkpoint-720,'quarter-leg approach',120);
      }
      await command(staff,'@tacsim/time rate=24');
      await until(()=>nowIc()>=checkpoint,'quarter-leg checkpoint',420);
      const inbound=await queued(clients.bob,[pilot],'R6a shore to moving Dray');
      await hidden(pilot,inbound,'R6a shore to moving Dray');
      const flight=(await until(()=>dispatch(inbound,pilot)?.transit?.flight&&dispatch(inbound,pilot),'moving mail flight')).transit.flight;
      assert(flight.seconds>0 && flight.basis.startsWith('ship.v1:'),'real moving endpoint metric');
      await received(inbound,pilot); await confirmed(inbound,pilot);
      const inbox=await panel(pilot), ordinal=inbox.entries.find(e=>e.id===inbound).index;
      await command(pilot,'@mail/reply '+ordinal,'Composing');
      await command(pilot,'Reply from an actual underway Dray.');
      const sent=await command(pilot,'--','queued for IC transit');
      const outbound=Number(sent.match(/Message (\d+) queued/)[1]);
      await hidden(clients.bob,outbound,'Re: R6a shore to moving Dray');
      const replyFlight=(await until(()=>dispatch(outbound,clients.bob)?.transit?.flight&&dispatch(outbound,clients.bob),'moving reply flight')).transit.flight;
      assert(replyFlight.seconds>0&&replyFlight.basis.startsWith('ship.v1:'));
      await received(outbound,clients.bob); await confirmed(outbound,clients.bob);
      milestone('moving-hull',{hull:row.hull,ticket,inbound,outbound,flight,replyFlight});
    }

    await command(staff,'@relay/check','Bindings OK');
    await command(staff,'@tacsim/time rate=24');
    evidence.pending=['upgrade','durable-boundaries','send-paths','large-letter-offline',
      'changed-lock','changed-ignore','full-mailbox','destroyed-recipient',
      'saturation-expiry','moving-hull'].filter(name=>!done(name));
    evidence.passed=evidence.pending.length===0;
  }
} catch(error) { evidence.passed=false; evidence.error=error.stack; process.exitCode=1; }
finally {
  if(restoreFault) {
    shell('systemctl',['stop','dev1mush-gateway']);
    if(restoreFault.value===null) shell('redis-cli',['-p',redisPort,'DEL',restoreFault.key]);
    else shell('redis-cli',['-p',redisPort,'JSON.SET',restoreFault.key,'$',JSON.stringify(restoreFault.value)]);
    shell('systemctl',['start','dev1mush-gateway']);
  }
  if(staff?.ws.readyState===WebSocket.OPEN) {
    try { await command(staff,'@tacsim/time rate=24'); } catch { /* reconnect on resume */ }
  }
  save(prepareOnly?'prepare.json':'live.json',evidence);
  for(const ws of sockets) ws.close();
  console.log(JSON.stringify({passed:evidence.passed,checks:evidence.checks.length,error:evidence.error}));
}
