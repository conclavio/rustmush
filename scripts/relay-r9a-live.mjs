#!/usr/bin/env node
// R9a real paid production, physical freight and private mail. Redis reads only.
import {assert,readFileSync,evidence,credentials,sockets,sleep,shell,registry,auth,connect,command,until,crash,envelopes,actor,mail,box,milestone,close} from './relay-live-client.mjs';
// Work-order identities are u64, not JavaScript numbers. Preserve their source.
const read=(k,db=0)=>JSON.parse(shell('redis-cli',['-p','6788','-n',String(db),'--raw','JSON.GET',k])||'null',(_k,v,c)=>typeof v==='number'&&!Number.isSafeInteger(v)?c.source:v);
const docs=(p,db=0)=>{const ks=shell('redis-cli',['-p','6788','-n',String(db),'--scan','--pattern',p]).split('\n').filter(Boolean);assert(ks.length<10000);return ks.map(k=>read(k,db));};
const foundry=()=>read('mod:tacsim:tacsim:relay_foundry');
const port=id=>read('mod:tacsim:tacsim:port:tacsim.lane.ceres_phobos/'+id);
const orders=()=>docs('mod:industry:action-state/v1/industry_work_orders/*').map(r=>r.value);
const lots=()=>docs('mod:inventory:action-state/v1/inventory_lots/*').map(r=>r.value);
const accounts=()=>docs('mod:economy:account:*');
const total=(db=0)=>docs('mod:economy:account:*',db).filter(a=>a.instrument==='tacsim.credit').reduce((n,a)=>n+BigInt(a.posted_balance),0n).toString();
const oid=id=>'wo#'+BigInt(id).toString(16).padStart(16,'0');
const order=id=>orders().find(o=>String(o.id)===String(id));
const state=o=>o.state.kind;
const now=()=>{const h=read('mod:rustmush_kernel_time:world_ic/head'),e=read('mod:rustmush_kernel_time:world_ic/epoch/'+String(h.generation).padStart(20,'0'));return e.ic_anchor+(Math.floor(Date.now()/1000)-e.real_anchor)*e.rate;};
const resumeDeimos=process.argv[6]==='resume-deimos';
const resumeCarrier=process.argv[6]==='resume-carrier';
let resumeMount;
const resumePaid=process.argv[6]==='resume-paid'||resumeCarrier||resumeDeimos;
if(resumePaid){
 const prior=JSON.parse(readFileSync('/opt/rustmush/dev1/r9a-evidence/live.json','utf8'));
 assert(prior.failure&&(resumeDeimos?(prior.failure.includes('Local cargo title, owned empty mount')&&prior.milestones.some(m=>m.test==='arrival-deimos')&&!prior.milestones.some(m=>m.test==='first-installed-half-without-working-link')):resumeCarrier?(prior.failure.includes('Nothing was burned')&&prior.milestones.some(m=>m.test==='title-refusal-and-loss-remain-after-restart')&&!prior.milestones.some(m=>m.test==='departure-luna')):(prior.checks.at(-1).line==='@create R9a Negative Mount'&&prior.checks.at(-1).reply==='Permission denied.')));
 if(resumeDeimos){resumeMount=Number(prior.checks.at(-1).line.match(/^@chown #(\d+)=/)?.[1]);assert(resumeMount);}
 assert(prior.milestones.some(m=>m.test==='second-paid-pair-for-negative-lifecycle'));
 const runs=evidence.runs;Object.assign(evidence,prior);evidence.runs.push(...runs);delete evidence.failure;
 evidence.resumes??=[];evidence.resumes.push({at:new Date().toISOString(),reason:resumeDeimos?'Place the existing empty player-owned mount in the arrival room after the installation guard refused its inventory location.':resumeCarrier?'Refused before burning: disembark, add supply margin and continue the same carrier.':'Use builder-created empty sites with player ownership; continue the same paid lots, halves, titles and world.'});
}
let staff,pilot,stranger,logins,f,row,payer,conserved;
async function mount(name,existing){
 const here=read('obj:'+actor(pilot)).location;let id=existing;
 if(id){const obj=read('obj:'+id);assert.equal(obj.owner,actor(pilot));assert.equal(obj.name,name);}
 else{await command(staff,`@teleport ${staff.object}=#${here}`);const created=await command(staff,'@create '+name);id=Number(created.match(/#(\d+)/)?.[1]);assert(id,created);await command(staff,`@chown #${id}=${pilot.object}`,'Owner changed.');}
 await command(staff,`@teleport #${id}=#${here}`);assert.equal(read('obj:'+id).location,here);return id;
}
async function reconnect(){for(const ws of sockets)if(ws.readyState===WebSocket.OPEN)ws.close();await sleep(300);[staff,pilot,stranger]=await Promise.all(logins.map(connect));}
async function death(label){await crash(label);await reconnect();}
async function buy(good,vendor,qty,dest){const l=docs('mod:market:market:listing:*').find(l=>l.vendor===vendor&&l.catalog.code===good&&!l.source.wanted);assert(l,'listing '+good);const out=await command(pilot,`+market/quote #${l.id}=${qty}`,'Quote #');const q=out.match(/Quote #(\d+)/)[1];await command(pilot,'+market/accept #'+q+(dest?'='+dest:''),'Agreed.');await sleep(1200);}
async function manufacture(label,restarts){
 await command(staff,'@tacsim/time rate=3600');
 await buy('instrument_boards',port('luna_port').vendors.crisium_instruments.vendor,2,f.vault);
 const before=new Set(orders().map(o=>String(o.id)));
 await command(pilot,`@wo/draft tacsim.relay.manufacture/private_pair/1=${f.facility} 1 ${payer} ${f.vault} ${f.vault}`);
 const o=await until(()=>orders().find(o=>!before.has(String(o.id))&&o.facility===f.facility),'draft order');
 await command(pilot,'@wo/submit '+oid(o.id));await until(()=>state(order(o.id))==='running','paid running order',90);
 if(restarts){await death('paid-production-running');await command(staff,'@tacsim/time rate=1800');const paused=order(o.id).schedule.progress_units;await sleep(5000);assert.equal(order(o.id).schedule.progress_units,paused,'stale generation cannot advance production');await command(pilot,`@wo/restamp ${oid(o.id)}=production`);await until(()=>order(o.id).schedule.progress_units>paused,'restamped production resumes',30);}
 await until(()=>state(order(o.id))==='awaiting_delivery','finite production completion',360);
 await command(pilot,'@wo/deliver '+oid(o.id));await until(()=>state(order(o.id))==='completed','delivered order');
 await until(()=>docs('mod:economy:hold:*').some(h=>String(h.purpose?.Reservation?.id)===String(o.id)&&JSON.stringify(h.status).includes('Consumed')),'real budget consumption',90);
 const lot=await until(()=>lots().find(l=>l.endpoint===f.vault&&l.commodity==='relay_pair'),'real industrial output');
 await command(pilot,`+relaypair/claim ${oid(o.id)}=${lot.id}`);await until(()=>!lots().some(l=>String(l.id)===String(lot.id)),'output consumed once');
 if(restarts)await death('output-consumed-before-title-collection');
 await command(pilot,'+relaypair/collect '+oid(o.id),'titled halves');const m=foundry().machines[String(o.id)];assert(m&&m.titled.every(Boolean));
 await command(pilot,'+relaypair/collect '+oid(o.id),'titled halves');assert.deepEqual(foundry().machines[String(o.id)],m);
 await command(stranger,'+relaypair/collect '+oid(o.id),'Output owner');
 milestone(label,{order:order(o.id),lot,halves:m,total:total()});return {id:o.id,...m};
}
async function sail(portId,dest,seq,loaded){
 const p=port(portId),vendors=portId==='ceres_port'?['occator','coop']:portId==='luna_port'?['highport_victualling','highport_victualling']:['strategic_reserve','strategic_reserve'];
 for(let n=0;n<10;n++){const plan=await command(pilot,'+ship/plan '+dest);const burn=plan.match(/Burn (\d+) t.*\(tank (\d+)/),food=plan.match(/Stores (\d+) crew-days.*\(locker (\d+)/);assert(burn&&food,plan);const fuel=+burn[1]+3,stores=+food[1]+2;if(+burn[2]>=fuel&&+food[2]>=stores)break;if(+burn[2]<fuel)await buy('propellant',p.vendors[vendors[0]].vendor,fuel-+burn[2]);if(+food[2]<stores)await buy('provisions',p.vendors[vendors[1]].vendor,stores-+food[2]);}
 // Material/budget reconciliation runs at the normal minute sweep.
 await sleep(65000);await command(pilot,'+ship/board','cycle the lock');await command(staff,'@tacsim/time rate=3600');await command(pilot,'+ship/undock '+dest,'Clearance filed');
 const ticket=await until(()=>read('mod:tacsim:action-state/v1/tacsim.ticket/'+row.hull+':'+seq)?.value,'real ticket',90);
 await until(()=>read('mod:sim:cjourney:'+row.hull)?.state?.state==='Cruising','actual departure',90);
 if(loaded){assert(ticket.payload_kg>=10000);await death('two-titled-halves-in-real-passage');}
 const last=ticket.plan.legs.at(-1),due=last.t0+last.secs;milestone('departure-'+dest,{hull:row.hull,ticket,expectedRealSeconds:Math.ceil((due-now())/3600)});
 await until(()=>now()>due,'real world IC arrival',7200);await until(()=>read('mod:sim:cjourney:'+row.hull)?.state?.state!=='Cruising','arrival installed',90);
 await command(pilot,'+ship/leave','cycle the lock');milestone('arrival-'+dest,{location:read('obj:'+actor(pilot)).location});
}
try{
 const actors=JSON.parse(readFileSync('/opt/rustmush/dev1/r6a-evidence/actors.json','utf8'));
 logins=[await auth('login',credentials),await auth('login',actors.bob),await auth('login',actors.carol)];await reconnect();f=foundry();assert(f&&(resumePaid?Object.keys(f.machines).length===2:!Object.keys(f.machines).length));assert.equal(Object.keys(registry().pairs).length,1);
 let good,bad;
 if(!resumePaid){
 await command(staff,'@tacsim/time rate=24');const desk=docs('obj:*').find(o=>o.name==='Registry & Co-op Desk');assert(desk);await command(staff,`@teleport ${pilot.object}=#${desk.id}`);
 const name='R9a Relay Carrier';await command(pilot,'+coop/enrol '+name,'now the '+name);row=port('ceres_port').hulls.find(r=>read('obj:'+r.hull).name.includes(name));assert(row);await command(pilot,'+coop/refit prospector','as a Dray-P');
 for(const cap of ['economy.account.post','economy.account.view','vault.deposit','vault.withdraw'])await command(staff,`@org/grant ${pilot.object}=${f.owner} ${cap}`);
 for(const c of [pilot,stranger])await command(staff,`@teleport ${c.object}=#${f.place}`);
 await command(stranger,'+relaypair/stock','authority');await command(staff,`@account/open ${pilot.object}=tacsim.credit`);payer=accounts().find(a=>a.owner.Object===actor(pilot)&&a.instrument==='tacsim.credit').id;
 await command(staff,`@econ/pay 1=${payer} 1500000`);conserved=total(5);assert.equal(total(),conserved);await command(pilot,'+relaypair/stock');await sleep(1500);
 good=await manufacture('paid-unique-output-and-restart',true);bad=await manufacture('second-paid-pair-for-negative-lifecycle',false);
 await command(pilot,'get #'+good.halves[0],'checked carrier');await command(pilot,`@asset/stolen #${bad.halves[0]}=public R9a stolen freight`,'Reported stolen');
 await command(staff,'@destroy #'+bad.halves[1],'live title record');await command(staff,`@asset/writeoff #${bad.halves[1]}=R9a destructive test`);await command(staff,'@destroy #'+bad.halves[1],'Destroyed.');
 }else{
 const a=evidence.milestones.find(m=>m.test==='paid-unique-output-and-restart'),b=evidence.milestones.find(m=>m.test==='second-paid-pair-for-negative-lifecycle');
 good={id:a.order.id,...a.halves};bad={id:b.order.id,...b.halves};
 for(const m of [good,bad])assert.deepEqual(foundry().machines[String(m.id)].halves,m.halves);
 row=port('ceres_port').hulls.find(r=>read('obj:'+r.hull).name.includes('R9a Relay Carrier'));assert(row);
 assert(read('mod:economy:asset:'+bad.halves[0]).stolen);assert.equal(read('mod:economy:asset:'+bad.halves[1]).lifecycle,'WrittenOff');
 conserved=total(5);assert.equal(total(),conserved);
 }
 if(!resumeCarrier&&!resumeDeimos){
 const negativeMount=await mount('R9a Negative Mount');
 for(const h of bad.halves){await command(pilot,'+relaypair/load #'+h,'No usable titled');await command(pilot,`+relaypair/install #${h}=#${negativeMount} org:${f.owner}`,'authority');}
 await death('stolen-and-destroyed-cargo');await command(pilot,'+relaypair/collect '+oid(bad.id),'titled halves');assert.deepEqual(foundry().machines[String(bad.id)].halves,bad.halves);assert.equal(read('mod:economy:asset:'+bad.halves[1]).lifecycle,'WrittenOff');
 milestone('title-refusal-and-loss-remain-after-restart',{bad:foundry().machines[String(bad.id)],assets:bad.halves.map(h=>read('mod:economy:asset:'+h))});
 }else if(resumeCarrier){await command(pilot,'+ship/leave','cycle the lock');}
 if(!resumeDeimos){
 await command(staff,`@teleport ${pilot.object}=#${port('ceres_port').presentation}`);await sail('ceres_port','luna',1,false);
 for(const h of good.halves)await command(pilot,'+relaypair/load #'+h,'loaded');await sail('luna_port','deimos',2,true);
 await command(pilot,'+relaypair/unload #'+good.halves[0],'unloaded');}
 const deimos=read('obj:'+actor(pilot)).location;
 const a=await mount('R9a Deimos Private Mount',resumeMount);await command(pilot,`+relaypair/install #${good.halves[0]}=#${a} org:${f.owner}`,'installed');assert.equal(Object.keys(registry().pairs).length,1);
 await death('first-installed-half-without-working-link');assert.deepEqual(foundry().machines[String(good.id)].installed,[a,null]);
 await sail('deimos_port','stickney',3,false);await command(pilot,'+relaypair/unload #'+good.halves[1],'unloaded');const b=await mount('R9a Phobos Private Mount');
 await command(stranger,`+relaypair/install #${good.halves[1]}=#${b} org:${f.owner}`,'authority');await command(pilot,`+relaypair/install #${good.halves[1]}=#${b} org:${f.owner}`,'installed');
 await death('complete-private-installation');const pairId='private_'+BigInt(good.id).toString(16).padStart(16,'0'),pair=registry().pairs[pairId];assert.equal(pair.capacity_units_per_ic_second,1);assert.equal(pair.class_policy,'private');
 for(const endpoint of [pilot,stranger])await command(staff,`+relay/access ${pairId}/${endpoint.object}=discover,use`,'policy recorded');
 await command(staff,'@relay/disable luna_ceres.luna');await command(staff,`@teleport ${stranger.object}=#${deimos}`);
 await command(pilot,`@mail/quick ${stranger.object}=R9a private letter/Manufactured freight bypasses Luna.`,'queued for IC transit');const e=await until(()=>envelopes().find(e=>e.issuer===actor(pilot)&&e.dispatch?.key.endsWith(':out')&&mail(Number(e.dispatch.key.split(':')[2]))?.subject==='R9a private letter'),'private mail envelope');
 await command(staff,'@prop/pass');await until(()=>box(stranger,Number(e.dispatch.key.split(':')[2])),'private inbox delivery',180);assert(read('mod:propagation:envelope:'+e.id).transit.legs.some(l=>l.pair===pairId));await command(stranger,'@mail','R9a private letter');
 await command(pilot,'+relaypair/discover #'+good.halves[1],'consumed');const discovery=foundry().machines[String(good.id)].discovery;await death('anti-proliferation-discovery-consumed');await command(pilot,'+relaypair/discover #'+good.halves[1],'already recorded');assert.equal(foundry().machines[String(good.id)].discovery,discovery);
 assert.equal(total(),conserved);await command(staff,'@tacsim/time rate=24');milestone('private-mail-with-luna-disabled-and-once-only-discovery',{pair,envelope:read('mod:propagation:envelope:'+e.id),discovery,total:total(),expectedTotal:conserved,foundry:foundry()});evidence.passed=true;
}catch(error){evidence.failure=String(error.stack??error);process.exitCode=1;}finally{close();}
