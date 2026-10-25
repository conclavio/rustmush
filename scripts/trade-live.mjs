#!/usr/bin/env node
// Real dev1 trade acceptance. Run on conclav.io with NODE_EXTRA_CA_CERTS set.
// stdin: {"role":"pilot"|"staff","commands":["look",...]}. No clock overrides.
// State and credentials stay in /opt/rustmush/dev1/trade-acceptance/evidence.
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,existsSync,readdirSync} from 'node:fs';
import {randomBytes} from 'node:crypto';
const out='/opt/rustmush/dev1/trade-acceptance/evidence';
const target='https://conclav.io:6790';
const request=JSON.parse(readFileSync(0,'utf8'));
assert(['staff','pilot'].includes(request.role));
assert(Array.isArray(request.commands));
assert(request.commands.every(line=>typeof line==='string'&&!/\/time|@teleport|@tacsim\/(?:apply|seed)/i.test(line)), 'acceptance uses ordinary play and an unchanged clock');
const path=out+'/'+request.role+'.private.json';
let route='login',credentials;
if(existsSync(path)) credentials=JSON.parse(readFileSync(path,'utf8'));
else {
 assert.equal(request.role,'pilot');route='register';
 credentials={account:'trade'+randomBytes(6).toString('hex'),password:randomBytes(24).toString('hex'),char_name:'TradeWren'};
 writeFileSync(path,JSON.stringify(credentials)+'\n',{mode:0o600});
}
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
function text(v){if(typeof v==='string')return v;if(Array.isArray(v))return v.map(text).join('');if(v&&typeof v==='object')return typeof v.text==='string'?v.text:Object.values(v).map(text).join(' ');return '';}
let login;
for(let n=0;n<12;n++){
 const r=await fetch(target+'/api/auth/'+route,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(credentials),signal:AbortSignal.timeout(15000)});
 if(r.status===429){await r.body?.cancel();await sleep(15000);continue;}
 assert.equal(r.status,200,'auth status '+r.status);login=await r.json();break;
}
assert(login,'auth retry budget');
const frames=[],ws=new WebSocket(target.replace(/^http/,'ws')+'/ws?char='+login.characters[0].key,'rustmush.v1.'+login.token);
ws.addEventListener('message',e=>{if(typeof e.data!=='string')return;const f=JSON.parse(e.data);if(f.t==='ping')ws.send(JSON.stringify({t:'pong',d:{nonce:f.d.nonce}}));else frames.push(f);});
async function wait(predicate,start=0){const end=Date.now()+45000;while(Date.now()<end){const found=frames.slice(start).find(predicate);if(found)return found;assert(ws.readyState<2,'socket closed');await sleep(25);}throw Error('Missing reply: '+JSON.stringify(frames.slice(start)).slice(-2500));}
const evidence={at:new Date().toISOString(),role:request.role,character:login.characters[0].key,commands:[],passed:false};
const evidencePath=out+'/session-'+Date.now()+'-'+request.role+'.json';
function save(){writeFileSync(evidencePath,JSON.stringify(evidence,null,2)+'\n',{mode:0o600});}
async function command(line){
 const start=frames.length;ws.send(JSON.stringify({t:'line',d:line}));
 await wait(f=>text(f.d).includes('__TRADE_DONE__'),start);
 const reply=frames.slice(start).filter(f=>!text(f.d).includes('__TRADE_DONE__')).map(f=>text(f.d)).join('\n');
 if(!line.startsWith('OUTPUT')){evidence.commands.push({line,reply,at:new Date().toISOString()});save();console.log(JSON.stringify({line,reply}));}
 assert(!reply.includes('please retry in a moment'),'pre-admission refusal; inspect before retry');
 return reply;
}
try {
 await wait(f=>f.t==='hello');
 await command('OUTPUTSUFFIX __TRADE_DONE__');
 evidence.login=frames.filter(f=>f.t!=='hello'&&!text(f.d).includes('__TRADE_DONE__')).map(f=>text(f.d)).join('\n');
 if(request.mode==='day1') await dayOne();
 else if(request.mode==='day2'||request.mode==='day2-resume') await dayTwo();
 else for(const line of request.commands) await command(line);
 evidence.passed=true;
}finally{save();ws.close();}

async function expect(line, fragment){const reply=await command(line);assert(reply.includes(fragment),line+': '+reply);return reply;}
async function buy(label,qty){
 const listings=await command('+market/list');
 const matches=listings.split('\n').filter(l=>l.toLowerCase().includes(label)&&!l.includes('wanted'));
 assert.equal(matches.length,1,'one current listing for '+label+': '+listings);
 const id=matches[0].match(/^#(\d+) /)?.[1];assert(id,matches[0]);
 const quote=await expect('+market/quote '+id+'='+qty,'Quote #');
 const q=quote.match(/Quote #(\d+)/)?.[1];assert(q,quote);
 await expect('+market/accept '+q,'Agreed.');
 // Draws settle asynchronously; read the hold until the transaction is visible.
 await sleep(1500);
}
async function dayOne(){
 assert.equal(request.role,'pilot');assert.equal(route,'register','day one starts with a new character');
 await expect('c','Port Watch');await expect('registry','Registry');
 await expect('+coop','stake');await expect('+coop/enrol Salt Moth','now the Salt Moth');
 for(const line of ['arrivals','concourse','exchange']) await command(line);
 await buy('metal concentrate',20);await buy('water ice',80);
 await expect('chandlery','Chandlery');
 for(let n=0;n<8;n++){
  const plan=await command('+ship/plan stickney');
  const fuel=plan.match(/Burn (\d+) t \(tank (\d+)/),food=plan.match(/Stores (\d+) crew-days \(locker (\d+)/);
  assert(fuel&&food,plan);
  if(+fuel[2]>=+fuel[1]&&+food[2]>=+food[1])break;
  if(+fuel[2]<+fuel[1])await buy('reaction mass',+fuel[1]-+fuel[2]);
  if(+food[2]<+food[1])await buy('provisions',+food[1]-+food[2]);
  assert(n<7,'planner outfit converges');
 }
 await expect('+ship/hold','FULL: mass and packed volume');
 await command('+credit');await command('+ship/plan stickney');
 for(const line of ['exchange','concourse','arrivals','lobby','gallery'])await command(line);
 await expect('+ship/board','You cycle the lock into the Salt Moth.');
 for(const line of ['in','crew'])await command(line);
 // Keep the ship docked for session two. +72h also gives fill a real home leg
 // to reserve against, without the accelerated fixture's immediate return.
 const queue=['depart stickney at=now','sell metals all min=250','sell volatiles all min=150',
  'buy propellant fill max=60','buy provisions fill','buy industrial_parts 49 max=400',
  'buy structural_alloys 55 max=400','depart ceres at=+72h'];
 for(const [i,line] of queue.entries())await expect('+ship/order '+line,'Order '+(i+1)+' filed');
 await command('+ship/orders');
 evidence.departureFiledAt=new Date().toISOString();
 evidence.pilotDisconnectedForPassage=true;
}

async function dayTwo(){
 assert.equal(request.role,'pilot');assert.equal(route,'login');
 if(request.mode==='day2-resume'){
  const previous=readdirSync(out).filter(n=>/^session-.*-pilot\.json$/.test(n)&&out+'/'+n!==evidencePath).sort()
   .map(n=>JSON.parse(readFileSync(out+'/'+n,'utf8'))).filter(s=>s.character===evidence.character&&s.checkedNoticeLines);
  const prior=previous.at(-1);
  assert(prior&&!prior.passed&&prior.commands.length===1&&prior.commands[0].line==='+ship',
   'resume only the read-only status assertion interrupted after verified watch delivery');
  assert.equal(evidence.login.split('While you were off watch:').length-1,0,'no duplicate watch log on recovery');
  evidence.resumedWatchAt=prior.at;evidence.checkedNoticeLines=prior.checkedNoticeLines;
 }else{
 assert.equal(evidence.login.split('While you were off watch:').length-1,1,'one offline watch log on login');
 const before=JSON.parse(readFileSync(out+'/before-day2-snapshot.json','utf8'));
 const notice=before['mod:tacsim:tacsim:notice:'+evidence.character];
 assert(notice?.lines?.length,'capture the pending watch log before reconnecting');
 const counts=new Map();for(const {text} of notice.lines)counts.set(text,(counts.get(text)||0)+1);
 for(const [line,count] of counts)assert.equal(evidence.login.split(line).length-1,count,'exact watch line delivery: '+line);
 evidence.checkedNoticeLines=notice.lines.length;
 }
 await expect('+ship','docked Stickney Yards');
 await command('+ship/orders');await command('+ship/hold');await command('+credit');
 await expect('+coop/statement','Sales: 2;');
 for(const line of ['passage','airlock'])await command(line);
 await expect('+ship/leave','Visiting Berth Gallery');
 await expect('look','Berth board:');
 for(const line of ['lobby','customs','concourse','harbourmaster'])await command(line);
 await expect('look','Departures board:');
 for(const line of ['concourse','yard'])await command(line);
 await expect('look','Service Counter');
 // Refile the home leg after walking the Yard, retaining the settled order log.
 await expect('+ship/order del 8','struck from the sailing plan');
 await expect('+ship/order depart ceres at=+1h','filed');
 await command('+ship/orders');
 for(const line of ['concourse','customs','lobby','gallery'])await command(line);
 await expect('+ship/board','Salt Moth');
 for(const line of ['in','crew'])await command(line);
 await command('+ship/plan ceres');
}
