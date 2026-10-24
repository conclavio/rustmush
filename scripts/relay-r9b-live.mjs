#!/usr/bin/env node
// R9b live Fleet programme: ordinary commands, real clock and durable receipts.
import {assert,readFileSync,evidence,credentials,sockets,sleep,shell,registry,auth,connect,command,until,crash,envelopes,actor,mail,box,milestone,close} from './relay-live-client.mjs';
const read=(k,db=0)=>JSON.parse(shell('redis-cli',['-p','6788','-n',String(db),'--raw','JSON.GET',k])||'null',(_k,v,c)=>typeof v==='number'&&!Number.isSafeInteger(v)?c.source:v);
const docs=(p,db=0)=>{const ks=shell('redis-cli',['-p','6788','-n',String(db),'--scan','--pattern',p]).split('\n').filter(Boolean);assert(ks.length<10000);return ks.map(k=>read(k,db));};
const foundry=()=>read('mod:tacsim:tacsim:relay_foundry');
const programme=()=>read('mod:tacsim:tacsim:relay_mobile');
const port=id=>read('mod:tacsim:tacsim:port:tacsim.lane.ceres_phobos/'+id);
const orders=()=>docs('mod:industry:action-state/v1/industry_work_orders/*').map(r=>r.value);
const lots=()=>docs('mod:inventory:action-state/v1/inventory_lots/*').map(r=>r.value);
const accounts=()=>docs('mod:economy:account:*');
const total=()=>accounts().filter(a=>a.instrument==='tacsim.credit').reduce((n,a)=>n+BigInt(a.posted_balance),0n).toString();
const oid=id=>'wo#'+BigInt(id).toString(16).padStart(16,'0');
const now=()=>{const h=read('mod:rustmush_kernel_time:world_ic/head'),e=read('mod:rustmush_kernel_time:world_ic/epoch/'+String(h.generation).padStart(20,'0'));return e.ic_anchor+(Math.floor(Date.now()/1000)-e.real_anchor)*e.rate;};
const envelope=id=>read('mod:propagation:envelope:'+id);
const cruising=()=>read('mod:sim:cjourney:'+programme().hull)?.state?.state==='Cruising';
let staff,pilot,observer,logins,f,row,payer,pairId,conserved;
async function reconnect(){for(const ws of sockets)if(ws.readyState===WebSocket.OPEN)ws.close();await sleep(300);[staff,pilot,observer]=await Promise.all(logins.map(connect));}
async function death(label){await crash(label);await reconnect();}
async function buy(good,vendor,qty,dest){const l=docs('mod:market:market:listing:*').find(l=>l.vendor===vendor&&l.catalog.code===good&&!l.source.wanted);assert(l,'listing '+good);const out=await command(pilot,`+market/quote #${l.id}=${qty}`,'Quote #');const q=out.match(/Quote #(\d+)/)[1];await command(pilot,'+market/accept #'+q+(dest?'='+dest:''),'Agreed.');await sleep(1200);}
async function settled(){await until(()=>docs('mod:market:market:quote:*').filter(q=>q.buyer.Object===actor(pilot)&&q.contract!==null).every(q=>q.settled!==null),'all real purchases settled',180);}
async function cell(section,value){await command(staff,`@unit/damage #${row.hull}=`+JSON.stringify({ops:[{SetCell:{addr:{Section:{section,cell:'internals'}},value}}]}),'Applied');}
async function send(recipient,title){await command(staff,`@mail/quick ${recipient.object}=${title}/Real mobile half acceptance.`,'queued for IC transit');const e=await until(()=>envelopes().find(e=>e.issuer===actor(staff)&&e.dispatch?.key.endsWith(':out')&&mail(Number(e.dispatch.key.split(':')[2]))?.subject===title),'mail envelope');await command(staff,'@prop/pass');return e.id;}
async function received(recipient,id){const e=envelope(id);await until(()=>box(recipient,Number(e.dispatch.key.split(':')[2])),'actual inbox delivery',180);return envelope(id);}
const uses=id=>envelope(id).transit?.legs.some(l=>l.pair===pairId);
try{
 const actors=JSON.parse(readFileSync('/opt/rustmush/dev1/r6a-evidence/actors.json','utf8'));
 logins=[await auth('login',credentials),await auth('login',actors.bob),await auth('login',actors.carol)];await reconnect();
 const latch=read('mod:content:applied:tacsim.lane.ceres_phobos');assert.equal(latch.version,12,'upgrade starts from accepted R9a content');
 const originalRegistry=registry(),originalFoundry=foundry();
 const plan=await command(staff,'@atlas/pack plan tacsim.lane.ceres_phobos','fingerprint ');const fingerprint=plan.match(/fingerprint ([0-9a-f]{16})/)[1];
 const applied=await command(staff,'@atlas/pack apply '+fingerprint,'Applied tacsim.lane.ceres_phobos@13');assert(!applied.includes('STOPPED PART WAY'));
 await death('explicit-lane-v12-to-v13-upgrade');assert.equal(read('mod:content:applied:tacsim.lane.ceres_phobos').version,13);
 await until(()=>programme(),'upgraded Fleet programme');assert.deepEqual(registry(),originalRegistry);assert.deepEqual(foundry(),originalFoundry);
 await until(()=>accounts().find(a=>a.owner.Organization===programme().fleet&&a.instrument==='tacsim.credit')?.posted_balance===10000000,'finite Fleet capitalization',120);
 milestone('explicit-upgrade-preserves-relays-and-foundry',{from:12,to:13,registry:registry(),programme:programme()});
 f=foundry();const p=programme();assert(p&&f);assert.deepEqual(p.payments,[]);assert.equal(p.launch,null);assert.equal(p.refit,null);assert.equal(Object.keys(registry().pairs).length,1);row=port('luna_port').hulls[0];assert.equal(row.hull,p.hull);conserved=total();
 for(const org of [f.owner,p.fleet])for(const cap of ['economy.account.post','economy.account.view','vault.deposit','vault.withdraw'])await command(staff,`@org/grant ${pilot.object}=${org} ${cap}`);
 await command(observer,'+relayfleet/fund','authority');await command(observer,'+relayfleet/consent yes','authority');
 await command(staff,`@asset/assign #${row.hull}=${pilot.object} Fleet command`,'Assigned');
 for(const c of [staff,pilot,observer])await command(staff,`@teleport ${c.object}=#${f.place}`);
 await command(pilot,'+ship','Halcyon');await command(staff,`@account/open ${pilot.object}=tacsim.credit`);payer=accounts().find(a=>a.owner.Object===actor(pilot)&&a.instrument==='tacsim.credit').id;
 await command(staff,`@econ/pay 1=${payer} 4000000`);assert.equal(total(),conserved);
 const fleetAccount=accounts().find(a=>a.owner.Organization===p.fleet&&a.instrument==='tacsim.credit');assert(fleetAccount);const fleetBalance=fleetAccount.posted_balance;
 await command(staff,'@tacsim/time rate=3600');await command(pilot,'+relayfleet/fund','authorized');
 await until(()=>programme().payments.length===1,'first actual treasury installment',30);
 const firstAt=now();milestone('first-real-monthly-payment',{programme:programme(),fleetAccount,fleetAfter:read('mod:economy:account:'+fleetAccount.id),firstAt});
 await command(pilot,'+relaypair/stock');await sleep(1500);await buy('instrument_boards',port('luna_port').vendors.crisium_instruments.vendor,2,f.vault);
 const before=new Set(orders().map(o=>String(o.id)));
 await command(pilot,`@wo/draft tacsim.relay.manufacture/private_pair/1=${f.facility} 1 ${payer} ${f.vault} ${f.vault}`);
 const order=await until(()=>orders().find(o=>!before.has(String(o.id))&&o.facility===f.facility),'draft order');const orderId=String(order.id);
 await command(pilot,'@wo/submit '+oid(order.id));await until(()=>orders().find(o=>String(o.id)===orderId)?.state.kind==='running','paid running manufacture',90);
 await death('funding-and-paid-production-running');
 await until(()=>orders().find(o=>String(o.id)===orderId)?.state.kind==='awaiting_delivery','actual fabrication complete',360);
 await command(pilot,'@wo/deliver '+oid(order.id));await until(()=>orders().find(o=>String(o.id)===orderId)?.state.kind==='completed','industrial delivery',90);
 await until(()=>docs('mod:economy:hold:*').some(h=>String(h.purpose?.Reservation?.id)===orderId&&JSON.stringify(h.status).includes('Consumed')),'real production budget consumption',90);
 const lot=await until(()=>lots().find(l=>l.endpoint===f.vault&&l.commodity==='relay_pair'),'actual pair output');
 await command(pilot,`+relaypair/claim ${oid(order.id)}=${lot.id}`,'Foundry action submitted');await until(()=>!lots().some(l=>String(l.id)===String(lot.id)),'claimed output consumed');
 await command(pilot,'+relaypair/collect '+oid(order.id),'titled halves');await settled();
 await command(observer,'+relayfleet/refit '+oid(orderId),'authority');await command(pilot,'+relayfleet/refit '+oid(orderId),'Fleet refit complete');
 const machines=foundry().machines[orderId];pairId='private_'+BigInt(orderId).toString(16).padStart(16,'0');assert.equal(registry().pairs[pairId].class_policy,'private');
 await command(staff,'+relayfleet/consent yes','consent recorded');await command(staff,'+relayfleet/consent yes','already recorded');await command(pilot,'+relayfleet/fund','already authorized');
 assert.equal(programme().launch,null);await death('real-refit-and-consent-before-commissioning');
 milestone('real-paid-refit-awaits-six-months',{order:orders().find(o=>String(o.id)===orderId),machines,programme:programme(),total:total()});
 let seen=programme().payments.length;
 // Real wall time, maximum supported IC rate. No clock anchors or receipts are edited.
 const deadline=Date.now()+75*60*1000;
 while(programme().payments.length<6){assert(Date.now()<deadline,'six-month programme timeout');await sleep(5000);const current=programme();if(current.payments.length>seen){seen=current.payments.length;milestone('funded-month-'+seen,{programme:current,icNow:now(),wallAt:new Date().toISOString()});if(seen===3)await death('three-real-monthly-installments');}}
 assert.equal(read('mod:economy:account:'+fleetAccount.id).posted_balance,fleetBalance-6000000);assert.equal(new Set(programme().payments).size,6);assert.equal(programme().launch,null);assert.equal(programme().next_payment,null);
 await command(pilot,'+relayfleet','8/8');await death('commissioned-before-actual-launch');
 await command(observer,'+relayfleet/remove','authority');await command(pilot,'+relayfleet/remove','pair removed');assert(!registry().pairs[pairId]);
 await death('removed-physical-pair');assert.deepEqual(foundry().machines[orderId].dismounted,machines.installed);assert.deepEqual(foundry().machines[orderId].halves,machines.halves);
 await command(pilot,'+relayfleet/refit '+oid(orderId),'refit restored');assert.deepEqual(foundry().machines[orderId].installed,machines.installed);assert.equal(programme().launch,null);
 await cell('power',0);assert.equal(registry().pairs[pairId].halves[1].damaged,true);await death('carrier-power-outage');await cell('power',3000);assert.equal(registry().pairs[pairId].halves[1].damaged,false);
 milestone('removal-remount-and-carrier-power',{programme:programme(),machines:foundry().machines[orderId],pair:registry().pairs[pairId]});
 const luna=port('luna_port');for(let n=0;n<10;n++){const plan=await command(pilot,'+ship/plan deimos');const burn=plan.match(/Burn (\d+) t.*\(tank (\d+)/),food=plan.match(/Stores (\d+) crew-days.*\(locker (\d+)/);assert(burn&&food,plan);const fuel=+burn[1]+3,stores=+food[1]+2;if(+burn[2]>=fuel&&+food[2]>=stores)break;if(+burn[2]<fuel)await buy('propellant',luna.vendors.highport_victualling.vendor,fuel-+burn[2]);if(+food[2]<stores)await buy('provisions',luna.vendors.highport_victualling.vendor,stores-+food[2]);}
 await settled();await command(pilot,'+ship/board','cycle the lock');await command(pilot,'+ship/undock deimos','Clearance filed');
 const ticket=await until(()=>read('mod:tacsim:action-state/v1/tacsim.ticket/'+row.hull+':1')?.value,'actual passage ticket',90);assert(ticket.payload_kg>=5000);await until(cruising,'cruiser departure',90);await until(()=>programme().launch,'first-launch canon after departure',30);const launch=programme().launch;
 await death('mobile-half-in-real-passage');assert.equal(programme().launch,launch);await command(staff,'@relay/disable luna_ceres.luna');
 const last=ticket.plan.legs.at(-1),end=last.t0+last.secs;await until(()=>now()>=ticket.plan.depart_ic+Math.floor((end-ticket.plan.depart_ic)/3),'mid-passage position',720);
 await command(staff,'@tacsim/time rate=24');const aboard=await send(pilot,'R9b aboard');await received(pilot,aboard);assert(uses(aboard));await command(pilot,'@mail','R9b aboard');
 await command(staff,`@teleport ${observer.object}=#${port('ceres_port').presentation}`);const outside=await send(observer,'R9b outside');await received(observer,outside);assert(!uses(outside));
 milestone('onboard-and-outside-coverage',{ticket,aboard:envelope(aboard),outside:envelope(outside),launch});
 const nearbySeconds=Math.floor(200000*last.secs/last.dist_km);assert(nearbySeconds>60);
 // Approach using supported clock rates, slowing before the actual coverage boundary.
 for(const rate of [3600,300,30]){await command(staff,'@tacsim/time rate='+rate);await until(()=>end-now()<=nearbySeconds+rate*3,'approach at '+rate,1200);}
 await command(staff,'@tacsim/time rate=1');await until(()=>end-now()<=nearbySeconds,'inside moving local coverage',120);assert(cruising());
 await command(staff,`@teleport ${observer.object}=#${port('deimos_port').presentation}`);const nearby=await send(observer,'R9b nearby');await received(observer,nearby);assert(uses(nearby));assert(cruising());
 const moving=await send(pilot,'R9b forwarding');assert(uses(moving));assert.notEqual(envelope(moving).state,'delivered','move before arrival');await command(staff,`@teleport ${pilot.object}=#${port('ceres_port').presentation}`);await command(staff,'@tacsim/time rate=24');await received(pilot,moving);await command(pilot,'@mail','R9b forwarding');
 milestone('nearby-coverage-and-recipient-transfer',{nearby:envelope(nearby),forwarded:envelope(moving),icNow:now(),end});
 await command(staff,`@teleport ${pilot.object}=#${row.rooms.airlock}`);await command(staff,'@tacsim/time rate=3600');await until(()=>!cruising(),'real docking',90);await command(pilot,'+ship/leave','cycle the lock');
 const half=registry().pairs[pairId].halves[1].id;await command(staff,'@relay/disable '+half);assert.equal(registry().pairs[pairId].halves[1].enabled,false);await command(staff,'@relay/enable '+half);assert.equal(registry().pairs[pairId].halves[1].enabled,true);
 await cell('hull',0);assert.equal(registry().pairs[pairId].halves[1].destroyed,true);const node=registry().pairs[pairId].halves[1].node;const loss=registry().losses[node];assert(loss&&loss.canon);await death('destroyed-capital-carrier');assert.deepEqual(registry().losses[node],loss);assert.equal(programme().launch,launch);
 await cell('hull',12000);assert.equal(registry().pairs[pairId].halves[1].destroyed,true,'repair cannot erase terminal relay loss');assert.equal(total(),conserved);
 await command(staff,'@tacsim/time rate=24');milestone('carrier-loss-terminal-and-launch-once',{programme:programme(),pair:registry().pairs[pairId],loss,total:total(),expectedTotal:conserved});evidence.passed=true;
}catch(error){evidence.failure=String(error.stack??error);process.exitCode=1;}finally{close();}
