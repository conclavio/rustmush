// Documentary continuation of the retained real campaign; helpers and final acceptance block match the repaired harness.
// R9b live Fleet programme: ordinary commands, real clock and durable receipts.
import {assert,readFileSync,evidence,credentials,sockets,sleep,shell,registry,auth,connect,command,until,crash,envelopes,actor,mail,box,milestone,close} from '/opt/rustmush/dev1/build/scripts/relay-live-client.mjs';
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
async function place(client,room){
 const before=read('obj:'+actor(client))?.location;
 const unplaced=await command(staff,`@spatial/unplace ${client.object}=R9b acceptance fixture transfer`);
 assert(unplaced.includes('Unplaced ')||unplaced.includes(client.object+' is not placed'),unplaced);
 await command(staff,`@teleport ${client.object}=#${room}`);
 await until(()=>read('obj:'+actor(client))?.location===room,'authoritative recipient placement',15);
 return {actor:actor(client),before,after:room,at:now()};
}
async function buy(good,vendor,qty,dest){const l=docs('mod:market:market:listing:*').find(l=>l.vendor===vendor&&l.catalog.code===good&&!l.source.wanted);assert(l,'listing '+good);const out=await command(pilot,`+market/quote #${l.id}=${qty}`,'Quote #');const q=out.match(/Quote #(\d+)/)[1];await command(pilot,'+market/accept #'+q+(dest?'='+dest:''),'Agreed.');await sleep(1200);}
async function settled(){await until(()=>docs('mod:market:market:quote:*').filter(q=>q.buyer.Object===actor(pilot)&&q.contract!==null).every(q=>q.settled!==null),'all real purchases settled',180);}
async function cell(section,value){await command(staff,`@unit/damage #${row.hull}=`+JSON.stringify({ops:[{SetCell:{addr:{Section:{section,cell:'internals'}},value}}]}),'Applied');}
async function send(recipient,title){await command(staff,`@mail/quick ${recipient.object}=${title}/Real mobile half acceptance.`,'queued for IC transit');const e=await until(()=>envelopes().find(e=>e.issuer===actor(staff)&&e.dispatch?.key.endsWith(':out')&&mail(Number(e.dispatch.key.split(':')[2]))?.subject===title),'mail envelope');await command(staff,'@prop/pass');return e.id;}
async function received(recipient,id){const e=envelope(id);await until(()=>box(recipient,Number(e.dispatch.key.split(':')[2])),'actual inbox delivery',180);return envelope(id);}
const uses=id=>envelope(id).transit?.legs.some(l=>l.pair===pairId);
try{
 const previousPath='/opt/rustmush/dev1/r9b-evidence/acceptance-repair/live-full-campaign-transfer-refusal.json';
 const previous=JSON.parse(readFileSync(previousPath,'utf8'));
 assert(previous.failure?.includes('nothing crosses a carrier'));assert(!previous.passed);
 assert.equal(previous.runs[0].serverSha256,evidence.runs[0].serverSha256);
 const priorPaid=previous.milestones.find(m=>m.test==='real-paid-refit-awaits-six-months');
 const priorVoyage=previous.milestones.find(m=>m.test==='onboard-and-outside-coverage');assert(priorPaid&&priorVoyage);
 const actors=JSON.parse(readFileSync('/opt/rustmush/dev1/r6a-evidence/actors.json','utf8'));
 logins=[await auth('login',credentials),await auth('login',actors.bob),await auth('login',actors.carol)];await reconnect();
 f=foundry();row=port('luna_port').hulls[0];const p=programme();
 assert.equal(p.hull,row.hull);assert.equal(new Set(p.payments).size,6);assert.equal(p.launch,priorVoyage.launch);assert(cruising());
 conserved=priorPaid.total;assert.equal(total(),conserved);pairId='private_'+BigInt(p.refit).toString(16).padStart(16,'0');assert(registry().pairs[pairId]);
 const launch=p.launch,ticket=priorVoyage.ticket,last=ticket.plan.legs.at(-1),end=last.t0+last.secs;
 assert(end>now());await command(staff,'@tacsim/time rate=1');
 await place(observer,port('deimos_port').presentation);
 const nearby=await send(observer,'R9b nearby continuation');await received(observer,nearby);assert(uses(nearby));assert(cruising());
 milestone('continuation-of-genuine-paid-voyage',{previousEvidenceSha256:shell('sha256sum',[previousPath]).split(' ')[0],previousCrashes:previous.milestones.filter(m=>m.signal==='SIGKILL').length,programme:p,pairId,total:conserved,nearby:envelope(nearby)});
 const moving=await send(observer,'R9b forwarding repaired');assert(uses(moving));
 const beforeTransfer=envelope(moving),message=Number(beforeTransfer.dispatch.key.split(':')[2]);
 assert.notEqual(beforeTransfer.state,'delivered','move before arrival');assert(!box(observer,message));
 // Move the nearby shore recipient out of mobile coverage. An embarked actor
 // cannot cross the carrier boundary underway, including by staff unplacement.
 const transfer=await place(observer,port('ceres_port').presentation);assert.notEqual(transfer.before,transfer.after);
 const forwarded=await until(()=>{const e=envelope(moving);return e.transit?.forwardings>beforeTransfer.transit.forwardings&&e.transit?.flight&&e;},'forwarding to the changed endpoint',30);
 assert.notEqual(forwarded.transit.destination,beforeTransfer.transit.destination);
 assert.notEqual(forwarded.state,'delivered');assert(!box(observer,message));
 const flight=forwarded.transit.flight;assert(flight.seconds>0);assert(flight.due.at>now()+15,'restart before the forwarded physical leg arrives');
 milestone('recipient-transfer-in-flight',{before:beforeTransfer,transfer,forwarded,icNow:now()});
 await command(staff,'@dump','snapshot acknowledged');await death('recipient-transfer-forwarding-in-flight');
 assert.equal(read('obj:'+actor(pilot)).location,transfer.after);assert.equal(envelope(moving).transit.forwardings,forwarded.transit.forwardings);
 assert.equal(envelope(moving).transit.flight.due.at,flight.due.at,'restart preserves frozen arrival');assert(!box(observer,message));
 await command(staff,'@tacsim/time rate=24');
 const arrivalDeadline=Date.now()+180000;let earlyChecks=0;
 while(!box(observer,message)){assert(Date.now()<arrivalDeadline,'forwarded inbox arrival timeout');if(now()<flight.due.at)earlyChecks++;await sleep(200);}
 const receivedAt=now();assert(receivedAt>=flight.due.at,'no inbox before frozen arrival');assert(earlyChecks>0);
 assert.equal(envelope(moving).state,'delivered');assert(envelope(moving).transit.history.some(h=>h.flight.due.at===flight.due.at&&h.arrived.at>=flight.due.at));
 const inbox=await command(observer,'@mail','R9b forwarding repaired');assert.equal(inbox.split('\n').filter(line=>line.includes('R9b forwarding repaired')).length,1,'one inbox entry');
 await command(staff,'@dump','snapshot acknowledged');await death('recipient-transfer-delivered-once');
 assert.equal((await command(observer,'@mail','R9b forwarding repaired')).split('\n').filter(line=>line.includes('R9b forwarding repaired')).length,1,'restart does not duplicate delivery');
 milestone('nearby-coverage-and-recipient-transfer',{nearby:envelope(nearby),before:beforeTransfer,transfer,forwarded:envelope(moving),flight,receivedAt,earlyChecks,icNow:now(),end});
 await command(staff,'@tacsim/time rate=3600');await until(()=>!cruising(),'real docking',90);
 await command(pilot,'+ship/leave','cycle the lock');
 const half=registry().pairs[pairId].halves[1].id;await command(staff,'@relay/disable '+half);assert.equal(registry().pairs[pairId].halves[1].enabled,false);await command(staff,'@relay/enable '+half);assert.equal(registry().pairs[pairId].halves[1].enabled,true);
 await cell('hull',0);assert.equal(registry().pairs[pairId].halves[1].destroyed,true);const node=registry().pairs[pairId].halves[1].node;const loss=registry().losses[node];assert(loss&&loss.canon);await death('destroyed-capital-carrier');assert.deepEqual(registry().losses[node],loss);assert.equal(programme().launch,launch);
 await cell('hull',12000);assert.equal(registry().pairs[pairId].halves[1].destroyed,true,'repair cannot erase terminal relay loss');assert.equal(total(),conserved);
 await command(staff,'@tacsim/time rate=24');milestone('carrier-loss-terminal-and-launch-once',{programme:programme(),pair:registry().pairs[pairId],loss,total:total(),expectedTotal:conserved});evidence.passed=true;
}catch(error){evidence.failure=String(error.stack??error);process.exitCode=1;}finally{close();}
