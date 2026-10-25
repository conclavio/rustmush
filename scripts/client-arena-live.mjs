#!/usr/bin/env node
// Disposable dev1: 12 real pilot connections exercise the parallel arena host.
// Uses separate loopback source addresses; the production per-IP cap stays intact.
// Fixture dependency: npm install --prefix /opt/rustmush/dev1/r1-tools ws@8
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,existsSync,readdirSync} from 'node:fs';
import {execFileSync} from 'node:child_process';
import {randomBytes} from 'node:crypto';
import {createRequire} from 'node:module';
const WebSocket=createRequire('/opt/rustmush/dev1/r1-tools/package.json')('ws');
const root='/opt/rustmush/dev1', target='https://conclav.io:6790';
const staff=JSON.parse(readFileSync(root+'/r1-evidence/credentials.json','utf8'));
const actors=JSON.parse(readFileSync(root+'/r6a-evidence/actors.json','utf8'));
const result={passed:false,checks:[],commands:[]}, clients=[];
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
function text(v){if(typeof v==='string')return v;if(Array.isArray(v))return v.map(text).join('');if(v&&typeof v==='object')return typeof v.text==='string'?v.text:Object.values(v).map(text).join(' ');return '';}
async function wait(c,predicate,start=0,seconds=45){const end=Date.now()+seconds*1000;while(Date.now()<end){const found=c.frames.slice(start).find(predicate);if(found)return found;assert.equal(c.ws.readyState,WebSocket.OPEN);await sleep(25);}throw Error('Missing arena response: '+JSON.stringify(c.frames.slice(start)).slice(-2500));}
async function connect(credentials,route='login'){
 let login;
 for(let attempt=0;attempt<20;attempt++){
  const response=await fetch(target+'/api/auth/'+route,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(credentials),signal:AbortSignal.timeout(15000)});
  if(response.status===429){const delay=Number(response.headers.get('retry-after'))||15;await response.body?.cancel();await sleep(Math.min(60000,Math.max(1000,delay*1000)));continue;}
  assert.equal(response.status,200,'auth status');login=await response.json();break;
 }
 assert(login,'auth retry budget');
 const c={name:login.characters[0].name,id:login.characters[0].key,frames:[]};
 c.ws=new WebSocket('wss://conclav.io:6790/ws?char='+c.id,'rustmush.v1.'+login.token,{servername:'conclav.io',family:4,localAddress:'127.0.0.'+(clients.length+2)});clients.push(c);
 c.ws.addEventListener('message',event=>{if(typeof event.data!=='string')return;const f=JSON.parse(event.data);if(f.t==='ping')c.ws.send(JSON.stringify({t:'pong',d:{nonce:f.d.nonce}}));else c.frames.push(f);});
 await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('connect timeout')),15000);c.ws.addEventListener('open',()=>{clearTimeout(timer);resolve();},{once:true});c.ws.addEventListener('error',event=>{clearTimeout(timer);reject(Error('websocket error: '+(event.error?.code||event.message||'unknown')));},{once:true});});
 await wait(c,f=>f.t==='hello');await command(c,'OUTPUTSUFFIX __ARENA_DONE__');return c;
}
async function command(c,line,expected){
 for(let attempt=0;attempt<20;attempt++){
  const start=c.frames.length;c.ws.send(JSON.stringify({t:'line',d:line}));
  await wait(c,f=>text(f.d).includes('__ARENA_DONE__'),start);
  const reply=c.frames.slice(start).filter(f=>!text(f.d).includes('__ARENA_DONE__')).map(f=>text(f.d)).join('\n');
  if(reply.includes('please retry in a moment')||reply.includes('Control service unavailable')||reply.includes('control state unknown')){await sleep(250);continue;}
  result.commands.push({actor:c.name,line,reply});if(expected)assert(reply.includes(expected),line+': '+reply);return reply;
 }
 throw Error('Repeated pre-admission refusal: '+line);
}
let god,bob,carol,started=false;const pilots=[];
try {
 god=await connect(staff);bob=await connect(actors.bob);carol=await connect(actors.carol);pilots.push(bob,carol);
 const privatePath=root+'/client-debt-evidence/arena-actors.private.json';
 const saved=existsSync(privatePath)?JSON.parse(readFileSync(privatePath,'utf8')):[];
 for(let i=0;i<10;i++){
  if(saved[i])pilots.push(await connect(saved[i]));
  else {
   const c={account:'frame'+randomBytes(6).toString('hex'),password:randomBytes(24).toString('hex'),char_name:'Frame'+randomBytes(5).toString('hex')};
   pilots.push(await connect(c,'register'));saved.push(c);writeFileSync(privatePath,JSON.stringify(saved),{mode:0o600});
  }
 }
 for(const [i,pilot] of pilots.entries()){pilot.team=i%2===0?'red':'blue';pilot.callsign=(i%2===0?'R':'B')+String.fromCharCode(65+Math.floor(i/2));}
 result.pilots=pilots.length;console.log('Connected all '+pilots.length+' pilots.');
 const listing=await command(god,'@find Simulator Lobby','END OF LIST');
 const lobby=listing.match(/Simulator Lobby[^\n]*?#(\d+)/)?.[1];assert(lobby,listing);
 for(const pilot of pilots){
  await command(god,'@spatial/unplace #'+pilot.id+'=performance acceptance fixture');
  await command(god,'@teleport #'+pilot.id+'=#'+lobby);
  await command(pilot,'look','Simulator Lobby');
 }
 for(const pilot of pilots)await command(pilot,'SIM TEAM '+pilot.team,'takes '+pilot.team+' slot');
 await command(bob,'SIM START','is boarding');started=true;
 for(const pilot of pilots){await command(pilot,pilot.team,pilot.team==='red'?'Red Room':'Blue Room');await command(pilot,'BOARD','climb into');}
 result.checks.push('12 real accounts board opposite sides through gateway, server, and Redis');console.log('All twelve pilots boarded.');
 for(const pilot of pilots){pilot.powerCursor=pilot.frames.length;await command(pilot,'STARTUP','Startup sequence');}
 for(const pilot of pilots){
  await wait(pilot,f=>text(f.d).includes('Power online.'),pilot.powerCursor);
  await command(pilot,'STATUS','Callsign '+pilot.callsign);
  pilot.panelCursor=pilot.frames.length;
  pilot.ws.send(JSON.stringify({t:'req',d:{id:88001,q:{t:'panel_subscribe',d:{feed:'arena.cockpit'}}}}));
  const reply=await wait(pilot,f=>f.t==='reply'&&f.d.id===88001,pilot.panelCursor);
  assert.equal(reply.d.event.t,'panel');assert.equal(reply.d.event.d.data.callsign,pilot.callsign);
 }
 // An unchanged stationary view has no obligation to publish a new frame.
 // Make every pilot's own view change, and require a fresh movement push.
 for(const pilot of pilots){
  pilot.steerCursor=pilot.frames.length;
  await command(pilot,'HEADING '+(pilot.team==='red'?90:270));
  await command(pilot,'SPEED 10');
 }
 result.cockpitFrames={};
 for(const pilot of pilots){
  await wait(pilot,f=>f.t==='panel'&&f.d?.feed==='arena.cockpit'&&f.d.data?.speed_cms>0,pilot.steerCursor);
  const panels=pilot.frames.slice(pilot.panelCursor).filter(f=>f.t==='panel'&&f.d?.feed==='arena.cockpit');
  assert(panels.every(f=>f.d.data.callsign===pilot.callsign),'only the pilot’s own cockpit');
  result.cockpitFrames[pilot.callsign]=panels.length;
 }
 const main=execFileSync('systemctl',['show','dev1mush-gateway','-p','MainPID','--value'],{encoding:'utf8'}).trim();
 const child=execFileSync('pgrep',['-P',main,'-x','mush-server'],{encoding:'utf8'}).trim();
 result.serverSha256=execFileSync('sha256sum',['/proc/'+child+'/exe'],{encoding:'utf8'}).split(' ')[0];
 result.frameWorkers=readdirSync('/proc/'+child+'/task').map(id=>{try{return readFileSync('/proc/'+child+'/task/'+id+'/comm','utf8').trim();}catch{return '';}}).filter(name=>name.startsWith('tacsim-frame-'));
 assert.equal(result.frameWorkers.length,3,'bounded reusable pool active on dev1');
 result.checks.push('startup, steering, and private cockpit frames cross the real split host with three reusable worker threads');
 await command(bob,'SIM ABORT','aborts match');
 let ended='';
 for(let attempt=0;attempt<40;attempt++){
  ended=await command(god,'@arena/status');
  if(ended.includes('Simulator centre: aborted')&&! /match \d+: (running|stopping)/.test(ended))break;
  await sleep(250);
 }
 assert(ended.includes('Simulator centre: aborted'),'abort completed');
 assert(! /match \d+: (running|stopping)/.test(ended),'no active match remains');
 assert(!ended.includes('stranded deployed marker'),'no stranded custody');
 for(const pilot of pilots)await command(pilot,'look',pilot.team==='red'?'Red Room':'Blue Room');
 started=false;
 result.checks.push('match abort returns custody through the real server');result.passed=true;
} finally {
 if(started&&bob?.ws.readyState===WebSocket.OPEN){try{await command(bob,'SIM ABORT');}catch{}}
 writeFileSync(root+'/client-debt-evidence/arena-live.json',JSON.stringify(result,null,2)+'\n');
 for(const c of clients)c.ws.close();
}
console.log('CLIENT_ARENA_LIVE_PASSED');
