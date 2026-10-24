#!/usr/bin/env node
// Availability/privacy acceptance: existing funded hardware, ordinary player APIs,
// read-only durable assertions, and verified dev1-only server-child crashes.
import {assert,readFileSync,randomBytes,evidence,credentials,sockets,sleep,shell,redis,envelope,registry,save,auth,connect,command,until,crash,envelopes,actor,service,mail,box,milestone,close} from './relay-live-client.mjs';
const read=k=>JSON.parse(shell('redis-cli',['-p','6788','--raw','JSON.GET',k])||'null',(_k,v,c)=>typeof v==='number'&&!Number.isSafeInteger(v)?c.source:v);
const docs=p=>shell('redis-cli',['-p','6788','--scan','--pattern',p]).split('\n').filter(Boolean).map(read);
const programme=()=>read('mod:tacsim:tacsim:relay_mobile');
const port=id=>read('mod:tacsim:tacsim:port:tacsim.lane.ceres_phobos/'+id);
const now=()=>{const h=read('mod:rustmush_kernel_time:world_ic/head'),e=read('mod:rustmush_kernel_time:world_ic/epoch/'+String(h.generation).padStart(20,'0'));return e.ic_anchor+(Math.floor(Date.now()/1000)-e.real_anchor)*e.rate;};
const cruising=()=>read('mod:sim:cjourney:'+programme().hull)?.state?.state==='Cruising';
let clients={},logins={},pairId;
async function reconnect(){for(const ws of sockets)if(ws.readyState===WebSocket.OPEN)ws.close();await sleep(300);for(const [k,v]of Object.entries(logins))clients[k]=await connect(v);}
async function death(label){await command(clients.staff,'@dump','snapshot acknowledged');await crash(label);await reconnect();}
async function place(name,room){const c=clients[name];const reply=await command(clients.staff,`@spatial/unplace ${c.object}=availability gate transfer`);assert(reply.includes('Unplaced ')||reply.includes(c.object+' is not placed'),reply);await command(clients.staff,`@teleport ${c.object}=#${room}`);await until(()=>read('obj:'+actor(c))?.location===room,'verified placement '+name,15);}
async function send(from,to,subject){await command(clients[from],`@mail/quick ${clients[to].object}=${subject}/checked transit`,'queued for IC transit');const accepted=await until(()=>envelopes().find(e=>e.issuer===actor(clients[from])&&e.dispatch?.key.endsWith(':out')&&mail(Number(e.dispatch.key.split(':')[2]))?.subject===subject),'durable mail '+subject);await command(clients.staff,'@prop/pass');return await until(()=>{const e=envelope(accepted.id);return e?.transit&&e;},'prepared route '+subject);}
const uses=e=>e.transit?.legs.some(l=>l.pair===pairId);
async function rights(name,value){await command(clients.staff,`+relay/access ${pairId}/${clients[name].object}=${value}`,'policy recorded');}
async function buy(good,qty){const vendor=port('luna_port').vendors.highport_victualling.vendor;const l=docs('mod:market:market:listing:*').find(l=>l.vendor===vendor&&l.catalog.code===good&&!l.source.wanted);assert(l);const out=await command(clients.pilot,`+market/quote #${l.id}=${qty}`,'Quote #');await command(clients.pilot,'+market/accept #'+out.match(/Quote #(\d+)/)[1],'Agreed.');await sleep(1200);}
try{
 const actors=JSON.parse(readFileSync('/opt/rustmush/dev1/r6a-evidence/actors.json','utf8'));
 logins={staff:await auth('login',credentials),pilot:await auth('login',actors.bob),observer:await auth('login',actors.carol)};await reconnect();
 const p=programme();assert.equal(new Set(p.payments).size,6);assert(p.refit);assert(!p.launch);assert(!cruising());
 pairId='private_'+BigInt(p.refit).toString(16).padStart(16,'0');assert.equal(registry().pairs[pairId].class_policy,'private');
 milestone('genuine-funded-checkpoint',{fixture:JSON.parse(readFileSync('/opt/rustmush/dev1/availability-evidence/prepare.json','utf8')),programme:p,pair:registry().pairs[pairId]});
 await command(clients.staff,'@tacsim/time rate=1');
 const foundry=read('mod:tacsim:tacsim:relay_foundry');for(const name of ['staff','pilot','observer'])await place(name,foundry.place);
 for(const name of ['pilot','observer'])for(const sw of ['status','load','audit','discover','publish','access','network']){
  assert.equal(await command(clients[name],`+relay/${sw} ${pairId}`),await command(clients[name],`+relay/${sw} no_such_pair`));
 }
 assert(!(await command(clients.observer,'+relay')).includes(pairId));
 await rights('observer','discover');await command(clients.observer,`+relay/status ${pairId}`,'not granted');
 const op=registry().nodes[registry().pairs[pairId].halves[0].node].operator;
 await command(clients.staff,`@org/grant ${clients.observer.object}=${op} relay.manage`);
 await command(clients.observer,`+relay/network ${pairId}=public`,'relay.publish authority');
 await rights('observer','none');milestone('private-id-opaque-and-management-does-not-publish');
 // Get the genuinely installed mobile endpoint into distinct physical coverage.
 await command(clients.staff,'@tacsim/time rate=3600');
 await until(()=>{const lots=docs('mod:inventory:action-state/v1/inventory_lots/*');return lots.length>0;},'inventory ready');
 for(let n=0;n<10;n++){const plan=await command(clients.pilot,'+ship/plan deimos');const burn=plan.match(/Burn (\d+) t.*\(tank (\d+)/),food=plan.match(/Stores (\d+) crew-days.*\(locker (\d+)/);assert(burn&&food,plan);const fuel=+burn[1]+3,stores=+food[1]+2;if(+burn[2]>=fuel&&+food[2]>=stores)break;if(+burn[2]<fuel)await buy('propellant',fuel-+burn[2]);if(+food[2]<stores)await buy('provisions',stores-+food[2]);}
 await until(()=>docs('mod:market:market:quote:*').filter(q=>q.buyer.Object===actor(clients.pilot)&&q.contract!==null).every(q=>q.settled!==null),'real stores settled');
 await command(clients.pilot,'+relayfleet','8/8');await command(clients.pilot,'+ship/board','cycle the lock');await command(clients.pilot,'+ship/undock deimos','Clearance filed');await until(cruising,'real departure',90);
 const ticket=await until(()=>read('mod:tacsim:action-state/v1/tacsim.ticket/'+p.hull+':1')?.value,'real passage');const last=ticket.plan.legs.at(-1),end=last.t0+last.secs;
 await until(()=>now()>=ticket.plan.depart_ic+Math.floor((end-ticket.plan.depart_ic)/3),'outside Luna coverage',720);await command(clients.staff,'@tacsim/time rate=1');
 for(const id of Object.keys(registry().pairs).filter(id=>id.startsWith('luna_')))await command(clients.staff,'@relay/disable '+id+'.luna');
 await command(clients.staff,`+relay/grant ${pairId}/${clients.observer.object}=priority`);
 let e=await send('observer','pilot','AP class is not access');assert(!uses(e));
 await rights('observer','discover,use');await rights('pilot','discover');
 e=await send('observer','pilot','AP both endpoints required');assert(!uses(e));
 await rights('pilot','discover,use');await command(clients.staff,`+relay/grant ${pairId}/${clients.observer.object}=standard`);
 e=await send('observer','pilot','AP revoke queued');assert(uses(e));assert(!e.transit.flight);const queued=e;
 await rights('pilot','discover');await command(clients.staff,'@prop/pass');await until(()=>!uses(envelope(e.id)),'revocation replans before transmission');assert(!box(clients.pilot,Number(e.dispatch.key.split(':')[2])));
 milestone('private-use-positive-at-both-ends-and-revoked-before-hop',{queued,rerouted:envelope(e.id),ticket});
 await rights('pilot','discover,use');
 await command(clients.staff,`@org/grant ${clients.observer.object}=${op} economy.account.post`);
 await command(clients.observer,`+relay/represent org:${op}`,'Permission denied');
 await command(clients.staff,`@org/grant ${clients.observer.object}=${op} relay.speak`);
 await command(clients.observer,`+relay/represent org:${op}`,'principal selected');await rights('observer','none');
 e=await send('observer','pilot','AP frozen organization');assert.equal(e.representation.organization,op);assert(uses(e));const official=e;
 await command(clients.observer,'+relay/represent personal');await death('organization-snapshot-and-private-acl-restart');assert.deepEqual(envelope(e.id).representation,official.representation);assert(uses(envelope(e.id)));
 await command(clients.staff,`@org/status ${op}=suspended`,'now suspended');await command(clients.staff,'@prop/pass');await until(()=>!uses(envelope(e.id)),'revoked organization reroute');await command(clients.observer,`+relay/represent org:${op}`,'Permission denied');await command(clients.staff,`@org/status ${op}=active`,'now active');
 milestone('explicit-frozen-agency-revalidated-after-restart',{accepted:official,after:envelope(e.id)});
 // Public carriage is an explicit independent choice; class remains distinct.
 await command(clients.staff,`+relay/network ${pairId}=public`,'policy recorded');await command(clients.observer,`+relay/status ${pairId}`,'allowed');
 await command(clients.staff,`+relay/grant ${pairId}/${clients.observer.object}=priority`);e=await send('observer','pilot','AP public carriage');assert(uses(e));
 const publicAccepted=e;await command(clients.staff,'@tacsim/time rate=24');await until(()=>box(clients.pilot,Number(e.dispatch.key.split(':')[2])),'actual private-link arrival',180);
 await command(clients.staff,`+relay/network ${pairId}=private`,'policy recorded');await rights('pilot','use');
 assert(!(await command(clients.pilot,'+relay/receipts')).includes(pairId),'use alone reveals no topology receipts');
 await rights('pilot','discover,use');assert((await command(clients.pilot,'+relay/receipts')).includes(pairId));
 await rights('pilot','use');assert(!(await command(clients.pilot,'+relay/receipts')).includes(pairId));
 await death('arrived-private-receipt-filter');assert(box(clients.pilot,Number(e.dispatch.key.split(':')[2])));assert(!(await command(clients.pilot,'+relay/receipts')).includes(pairId));
 milestone('public-carriage-delivers-and-receipts-observer-filtered',{accepted:publicAccepted,arrived:envelope(e.id)});
 // Unplaced recipients keep pending traffic stable; ordinary sender hits 32 copies.
 await command(clients.staff,'@tacsim/time rate=1');const accounts={};
 for(const name of ['flood','unknownA','unknownB']){const c={account:'ap'+name+randomBytes(3).toString('hex'),password:randomBytes(24).toString('hex'),char_name:'AP'+name+randomBytes(3).toString('hex')};accounts[name]=c;save('actors.json',accounts);logins[name]=await auth('register',c);clients[name]=await connect(logins[name]);}
 await place('flood',foundry.place);await place('unknownA',0);await place('unknownB',0);
 for(let n=0;n<16;n++)await command(clients.flood,`@mail/quick ${clients.unknownA.object} ${clients.unknownB.object}=AP saturation ${n}/unresolved`,'queued for IC transit');
 await command(clients.flood,`@mail/quick ${clients.unknownA.object}=AP overflow/body`,'Your communication allowance is full');
 const before=envelopes().filter(e=>e.issuer===actor(clients.flood));await death('unresolved-mail-saturation');
 await until(()=>{const rows=envelopes().filter(e=>e.issuer===actor(clients.flood));return rows.length===32&&rows.every(e=>!e.transit);},'32 unresolved copies after source handoff and restart');
 await command(clients.flood,`@mail/quick ${clients.unknownA.object}=AP overflow after restart/body`,'Your communication allowance is full');
 await place('observer',foundry.place);await command(clients.staff,`+order ${clients.observer.object}=AP unrelated order`,'Queued revision');const order=await until(()=>envelopes().find(e=>e.issuer===actor(clients.staff)&&e.kind==='order'&&e.payload?.includes?.('AP unrelated order'))||envelopes().filter(e=>e.issuer===actor(clients.staff)&&e.kind==='order').sort((a,b)=>b.id-a.id)[0],'unrelated order');
 await until(()=>envelope(order.id)?.state==='delivered','unrelated order delivery');await command(clients.observer,`+orders/ack ${order.id}`,'Acknowledgement queued');await until(()=>envelopes().some(e=>e.ack_of===order.id&&e.state==='delivered'),'actual acknowledgement arrival');
 await command(clients.staff,'@channel/create APIndependent');for(const flag of ['ic','logged','persist'])await command(clients.staff,'@channel/set APIndependent='+flag);for(const name of ['staff','observer'])await command(clients[name],'addcom ap=APIndependent','added');await command(clients.observer,'ap AP independent channel','queued for IC transit');await until(()=>clients.staff.frames.some(f=>JSON.stringify(f).includes('AP independent channel')),'unrelated channel arrival');
 milestone('one-sender-cannot-starve-orders-acks-or-channels',{beforeHandoff:before,pendingFlood:envelopes().filter(e=>e.issuer===actor(clients.flood)),order:envelope(order.id)});
 await command(clients.staff,'@dump','snapshot acknowledged');evidence.passed=true;
}catch(error){evidence.failure=String(error.stack??error);process.exitCode=1;}finally{close();}
