#!/usr/bin/env node
// RR-5..RR-7: real dev1 policy changes, money, narrow-link progress and crashes.
import {assert,readFileSync,evidence,credentials,sockets,sleep,shell,redis,envelope,registry,auth,connect,command,until,crash,envelopes,actor,transport,service,mail,box,receipt,milestone,close} from './relay-live-client.mjs';
const read=k=>JSON.parse(shell('redis-cli',['-p','6788','--raw','JSON.GET',k])||'null',(_k,v,c)=>typeof v==='number'&&!Number.isSafeInteger(v)?c.source:v);
const docs=p=>shell('redis-cli',['-p','6788','--scan','--pattern',p]).split('\n').filter(Boolean).map(read);
const programme=()=>read('mod:tacsim:tacsim:relay_mobile');
const port=id=>read('mod:tacsim:tacsim:port:tacsim.lane.ceres_phobos/'+id);
const now=()=>{const h=read('mod:rustmush_kernel_time:world_ic/head'),e=read('mod:rustmush_kernel_time:world_ic/epoch/'+String(h.generation).padStart(20,'0'));return e.ic_anchor+(Math.floor(Date.now()/1000)-e.real_anchor)*e.rate;};
const total=()=>docs('mod:economy:account:*').filter(a=>a.instrument==='tacsim.credit').reduce((n,a)=>n+BigInt(a.posted_balance),0n).toString();
const acct=id=>read('mod:economy:account:'+id);
const mid=e=>Number(e.dispatch.key.split(':')[2]);
const market=()=>service().commerce;
let clients={},logins={},pairId;
async function reconnect(){for(const ws of sockets)if(ws.readyState===WebSocket.OPEN)ws.close();await sleep(300);for(const [k,v]of Object.entries(logins))clients[k]=await connect(v);}
async function death(label){await command(clients.staff,'@dump','snapshot acknowledged');await crash(label);await reconnect();}
async function place(name,room){const c=clients[name],reply=await command(clients.staff,`@spatial/unplace ${c.object}=policy gate transfer`);assert(reply.includes('Unplaced ')||reply.includes(c.object+' is not placed'),reply);await command(clients.staff,`@teleport ${c.object}=#${room}`);await until(()=>read('obj:'+actor(c))?.location===room,'verified placement '+name,15);}
async function send(from,to,subject,body='checked transit'){await command(clients[from],`@mail/quick ${clients[to].object}=${subject}/${body}`,'queued for IC transit');return until(()=>envelopes().find(e=>e.issuer===actor(clients[from])&&e.dispatch?.key.endsWith(':out')&&mail(mid(e))?.subject===subject),'durable mail '+subject);}
async function schedule(pair,fields){await command(clients.staff,`+relay/schedule ${pair}=${fields};policy acceptance`,'recorded');return service().schedules[pair];}
async function jam(room,value){await command(clients.staff,`@prop/jam #${room}=${value}`,'is now '+value);}
async function received(to,e){await until(()=>box(clients[to],mid(e)),'actual inbox arrival '+mid(e),180);const out=envelope(e.id);assert.equal(out.state,'delivered');return out;}
async function upgrade(){
 const before=programme(),old=registry(),half=old.pairs[pairId];
 const lane=read('mod:content:applied:tacsim.lane.ceres_phobos');if(lane.version===12){const p=await command(clients.staff,'@atlas/pack plan tacsim.lane.ceres_phobos','fingerprint ');await command(clients.staff,'@atlas/pack apply '+p.match(/fingerprint ([0-9a-f]{16})/)[1],'Applied tacsim.lane.ceres_phobos@13');await death('lane-upgrade');}
 const pack=read('mod:content:applied:tacsim.relay.luna_ceres');if(pack.version===1){const p=await command(clients.staff,'@atlas/pack plan tacsim.relay.luna_ceres','fingerprint ');await command(clients.staff,'@atlas/pack apply '+p.match(/fingerprint ([0-9a-f]{16})/)[1],'Applied tacsim.relay.luna_ceres@2');await death('star-upgrade-with-real-private-hardware');}
 for(const pair of ['luna_ceres','luna_phobos','luna_kirkwood','luna_mercury'])assert(registry().pairs[pair]);
 assert.deepEqual(registry().pairs[pairId],half);assert.deepEqual(programme().payments,before.payments);assert.equal(programme().refit,before.refit);
 milestone('explicit-star-upgrade-preserves-real-funded-private-hardware',{programme:programme(),registry:registry()});
 // A legitimate unmount removes the pair; its policy must remain dormant, not break boot.
 await place('pilot',read('mod:tacsim:tacsim:relay_foundry').place);
 const saved=await schedule(pairId,'aging:301');
 await command(clients.pilot,'+relayfleet/remove','pair removed');assert(!registry().pairs[pairId]);
 await death('unmounted-private-policy-retained');assert.deepEqual(service().schedules[pairId],saved);assert(!registry().pairs[pairId]);
 await command(clients.pilot,'+relayfleet/refit wo#'+BigInt(before.refit).toString(16).padStart(16,'0'),'refit restored');
 assert.deepEqual(service().schedules[pairId],saved);assert.deepEqual(registry().pairs[pairId],half);
 milestone('unmount-restart-refit-retains-policy-and-physical-identities',{schedule:saved,pair:registry().pairs[pairId]});
}
async function twoSpokes(){
 const r=registry();await place('observer',r.nodes.ceres.coverage[0]);await place('pilot',r.nodes.mercury.place);
 for(const pair of ['luna_ceres','luna_mercury'])await command(clients.staff,`+relay/grant ${pair}/${clients.observer.object}=standard`,'Class grant recorded');
 await schedule('luna_ceres','standard:60,rate:8,burst:8');await schedule('luna_mercury','standard:1800');
 await command(clients.observer,'+relay/schedule luna_ceres=standard:1;unauthorized','Permission denied');
 const unchanged=service().schedules.luna_ceres;
 for(const fields of ['expect:0,rate:1','rate:9','burst:0','max:129','period:10,start:9,window:2']){
  assert(!(await command(clients.staff,`+relay/schedule luna_ceres=${fields};refuse`)).includes('recorded'));assert.deepEqual(service().schedules.luna_ceres,unchanged);
 }
 for(const name of ['pilot','observer'])assert.equal(await command(clients[name],`+relay/schedule ${pairId}`),await command(clients[name],'+relay/schedule no_such_pair'));
 await jam(r.nodes.mercury.place,100);
 const e=await send('observer','pilot','PB two spokes');await until(()=>envelope(e.id).transit?.service?.class==='standard','first-spoke ticket');const first=envelope(e.id);
 assert.equal(first.transit.service.declared_s,60);assert.equal(first.transit.service.repeat_operator,false);
 await schedule('luna_ceres','standard:900');assert.deepEqual(envelope(e.id).transit.service,first.transit.service);
 await command(clients.staff,'@tacsim/time rate=24');await until(()=>service().receipts.some(x=>x.envelope===e.id&&x.pair==='luna_ceres'),'first-spoke receipt');await command(clients.staff,'@tacsim/time rate=1');
 await until(()=>envelope(e.id).transit?.service?.repeat_operator,'second-spoke ticket');const second=envelope(e.id),ticket=second.transit.service;
 assert.equal(ticket.policy.settings.standard_delay_s,1800);assert.equal(ticket.declared_s,0);assert.equal(ticket.eligible_at,second.transit.queued_at.at);assert(!box(clients.pilot,mid(e)));
 await death('operator-admission-survives-between-spokes');assert.deepEqual(envelope(e.id).transit.service,ticket);
 await jam(r.nodes.mercury.place,0);await received('pilot',e);const receipts=service().receipts.filter(x=>x.envelope===e.id);
 assert.equal(receipts.length,2);assert(receipts[1].transmitted<second.transit.queued_at.at+1800);assert(receipts[0].transmitted>=first.transit.service.eligible_at);
 milestone('one-operator-admission-two-pairs-frozen-policy-and-real-inbox',{first,second,receipts,arrived:envelope(e.id)});
}
async function paidSerialization(){
 const r=registry();await place('observer',r.nodes.malapert.coverage[0]);await place('pilot',r.nodes.ceres.place);
 await command(clients.staff,`+relay/grant luna_ceres/${clients.observer.object}=deferred`);
 await schedule('luna_ceres','standard:60,deferred:7200,rate:1,burst:1,max:1024');
 await command(clients.staff,`@account/open ${clients.observer.object}=tacsim.credit`);
 const payer=docs('mod:economy:account:*').find(a=>a.owner.Object===actor(clients.observer)&&a.instrument==='tacsim.credit');assert(payer);
 const conserved=total();await command(clients.staff,`@econ/pay 1=${payer.id} 10000`,'Paid: transaction');await until(()=>acct(payer.id).posted_balance===payer.posted_balance+10000,'finite actual retail budget');assert.equal(total(),conserved);
 const before=acct(payer.id).posted_balance;
 const e=await send('observer','pilot','PB paid partial','p'.repeat(750));const q=market().next;
 await command(clients.observer,`+relay/quote ${e.id}=priority,${payer.id}`,'Quote '+q+':');await command(clients.observer,'+relay/buy '+q,'Reserved');
 await until(()=>envelope(e.id).transit?.serialization?.sent>0,'first paid unit');const room=envelope(e.id).transit.legs[envelope(e.id).transit.next].to;await jam(room,100);
 const partial=envelope(e.id),progress=partial.transit.serialization,purchase=market().purchases[q];assert(progress&&progress.total>progress.sent&&progress.total>=6);assert.equal(purchase.phase,'consumed');assert.equal(partial.path.length,0);assert(!receipt(e.id));assert(!box(clients.pilot,mid(e)));
 const hold=read('mod:economy:hold:'+purchase.hold);assert.equal(hold.status.Consumed.transaction,purchase.transaction);assert.equal(acct(payer.id).posted_balance,before-purchase.total_minor);assert.equal(total(),conserved);
 await schedule('luna_ceres','burst:8,max:128');assert(transport().capacity.luna_ceres.tokens<=1,'increased burst cannot mint units');assert.equal(envelope(e.id).transit.service.policy.settings.max_bytes,1024);
 await death('paid-partial-first-unit-capture');assert.deepEqual(envelope(e.id).transit.serialization,progress);assert.deepEqual(market().purchases[q],purchase);assert(!box(clients.pilot,mid(e)));
 await command(clients.observer,'+relay/cancel '+q,'no refund');await command(clients.observer,`+relay/quote ${e.id}=priority,${payer.id}`,'No such unconsumed outgoing copy');
 await jam(room,0);const arrived=await received('pilot',e),stamp=receipt(e.id);assert(stamp.transmitted>=progress.started+progress.total-1);assert.equal(stamp.serialized_from,progress.started);assert.equal(stamp.ticket.policy.settings.max_bytes,1024);
 assert.equal(service().receipts.filter(x=>x.envelope===e.id).length,1);assert.deepEqual(market().purchases[q],purchase);assert.equal(acct(payer.id).posted_balance,before-purchase.total_minor);assert.equal(total(),conserved);
 await death('paid-complete-and-delivered-once');assert(box(clients.pilot,mid(e)));assert.equal(service().receipts.filter(x=>x.envelope===e.id).length,1);assert.deepEqual(market().purchases[q],purchase);
 const later=await send('observer','pilot','PB new maximum','n'.repeat(200));await until(()=>envelope(later.id).transit,'new-limit route');assert(envelope(later.id).transit.legs.every(l=>l.pair!=='luna_ceres'));
 milestone('paid-serialization-no-early-visibility-one-charge-conserved-capacity',{partial,purchase,hold,receipt:stamp,arrived,later:envelope(later.id),total:total()});
}
async function buy(good,qty){const vendor=port('luna_port').vendors.highport_victualling.vendor,l=docs('mod:market:market:listing:*').find(l=>l.vendor===vendor&&l.catalog.code===good&&!l.source.wanted);assert(l);const out=await command(clients.pilot,`+market/quote #${l.id}=${qty}`,'Quote #');await command(clients.pilot,'+market/accept #'+out.match(/Quote #(\d+)/)[1],'Agreed.');await sleep(1200);}
async function realPrivate(){
 const p=programme(),foundry=read('mod:tacsim:tacsim:relay_foundry');for(const name of ['staff','pilot','observer'])await place(name,foundry.place);
 const payer=docs('mod:economy:account:*').find(a=>a.owner.Object===actor(clients.pilot)&&a.instrument==='tacsim.credit');assert(payer);const conserved=total();await command(clients.staff,`@econ/pay 1=${payer.id} 800000`,'Paid: transaction');await until(()=>acct(payer.id).posted_balance===payer.posted_balance+800000,'actual finite voyage budget');assert.equal(total(),conserved);
 await command(clients.staff,'@tacsim/time rate=3600');
 for(let n=0;n<10;n++){const plan=await command(clients.pilot,'+ship/plan deimos'),burn=plan.match(/Burn (\d+) t.*\(tank (\d+)/),food=plan.match(/Stores (\d+) crew-days.*\(locker (\d+)/);assert(burn&&food,plan);const fuel=+burn[1]+3,stores=+food[1]+2;if(+burn[2]>=fuel&&+food[2]>=stores)break;if(+burn[2]<fuel)await buy('propellant',fuel-+burn[2]);if(+food[2]<stores)await buy('provisions',stores-+food[2]);}
 await until(()=>docs('mod:market:market:quote:*').filter(q=>q.buyer.Object===actor(clients.pilot)&&q.contract!==null).every(q=>q.settled!==null),'real supplies settled');
 await command(clients.pilot,'+relayfleet','8/8');await command(clients.pilot,'+ship/board','cycle the lock');await command(clients.pilot,'+ship/undock deimos','Clearance filed');await until(()=>read('mod:sim:cjourney:'+p.hull)?.state?.state==='Cruising','real departure',90);
 const ticket=await until(()=>read('mod:tacsim:action-state/v1/tacsim.ticket/'+p.hull+':1')?.value,'real passage'),last=ticket.plan.legs.at(-1),end=last.t0+last.secs;milestone('real-funded-carrier-departure',{ticket,programme:programme(),payer:payer.id,minor:800000});
 await until(()=>now()>=ticket.plan.depart_ic+Math.floor((end-ticket.plan.depart_ic)/3),'outside Luna coverage',720);await command(clients.staff,'@tacsim/time rate=1');
 for(const id of Object.keys(registry().pairs).filter(id=>id.startsWith('luna_')))await command(clients.staff,'@relay/disable '+id+'.luna');
 for(const name of ['observer','pilot'])await command(clients.staff,`+relay/access ${pairId}/${clients[name].object}=discover,use`,'policy recorded');
 assert.equal(registry().pairs[pairId].capacity_units_per_ic_second,1);await schedule(pairId,'standard:0,rate:1,burst:1,max:1024');
 await command(clients.observer,`+relay/schedule ${pairId}=standard:30;not operator`,'Permission denied');await command(clients.staff,`+relay/schedule ${pairId}=rate:2;hardware cap`,'exceeds installed');
 const e=await send('observer','pilot','PB real narrow hardware','h'.repeat(750));await until(()=>envelope(e.id).transit?.serialization,'private-link partial progress');const room=envelope(e.id).transit.legs[envelope(e.id).transit.next].to;await jam(room,100);
 const partial=envelope(e.id),progress=partial.transit.serialization;assert(progress&&progress.sent<progress.total&&progress.total>=6);assert.equal(partial.transit.legs[partial.transit.next].pair,pairId);assert(!box(clients.pilot,mid(e)));assert(!receipt(e.id));assert.equal(partial.path.length,0);
 await command(clients.staff,'@prop/route '+e.id,'Serialization');await death('real-one-unit-hardware-partial-progress');assert.deepEqual(envelope(e.id).transit.serialization,progress);assert(!box(clients.pilot,mid(e)));
 await jam(room,0);const arrived=await received('pilot',e),stamp=receipt(e.id);assert.equal(stamp.pair,pairId);assert.equal(stamp.serialized_from,progress.started);assert(stamp.transmitted>=progress.started+progress.total-1);assert.equal(service().receipts.filter(x=>x.envelope===e.id).length,1);
 milestone('actual-one-unit-private-hardware-multiunit-delivery-after-crash',{pair:registry().pairs[pairId],partial,arrived,receipt:stamp,ticket});
}
try{
 const actors=JSON.parse(readFileSync('/opt/rustmush/dev1/r6a-evidence/actors.json','utf8'));
 logins={staff:await auth('login',credentials),pilot:await auth('login',actors.bob),observer:await auth('login',actors.carol)};await reconnect();
 const p=programme();assert.equal(new Set(p.payments).size,6);assert(p.refit);assert(!p.launch);pairId='private_'+BigInt(p.refit).toString(16).padStart(16,'0');
 milestone('genuine-funded-checkpoint',{fixture:JSON.parse(readFileSync('/opt/rustmush/dev1/policy-evidence/prepare.json','utf8')),programme:p,pair:registry().pairs[pairId]});
 await command(clients.staff,'@tacsim/time rate=1');await upgrade();await twoSpokes();await paidSerialization();await realPrivate();
 await command(clients.staff,'@dump','snapshot acknowledged');evidence.passed=true;
}catch(error){evidence.failure=String(error.stack??error);process.exitCode=1;}finally{close();}
