#!/usr/bin/env node
// R7 loss acceptance on disposable dev1. The surrounding gate snapshots/restores
// the whole test world; no product command can resurrect a destroyed pair.
import {assert,readFileSync,randomBytes,evidence,credentials,sockets,sleep,shell,redis,envelope,registry,save,auth,connect,command,until,crash,envelopes,actor,service,mail,box,milestone,close} from './relay-live-client.mjs';
let staff,bob,staffLogin,bobLogin;
async function reconnect(){for(const ws of sockets) if(ws.readyState===WebSocket.OPEN) ws.close();await sleep(300);staff=await connect(staffLogin);bob=await connect(bobLogin);}
async function damage(node,section,value){const plan={ops:[{SetCell:{addr:{Section:{section,cell:'internals'}},value}}]};await command(staff,`@unit/damage #${node}=${JSON.stringify(plan)}`,'Applied');}
async function send(label){const subject='R7-'+label+'-'+randomBytes(3).toString('hex');await command(staff,`@mail/quick ${bob.object}=${subject}/R7 real loss acceptance`,'queued for IC transit');const e=await until(()=>envelopes().find(e=>e.dispatch?.key.endsWith(':out')&&e.recipient===actor(bob)&&mail(Number(e.dispatch.key.split(':')[2]))?.subject===subject&&e.transit),'queue '+label);const id=Number(e.dispatch.key.split(':')[2]);assert(!box(bob,id));return{id,envelope:e.id,subject};}
function factions(){const keys=shell('redis-cli',['-p','6788','--scan','--pattern','mod:faction:fac:*']).split('\n').filter(Boolean);assert(keys.length<=4096);return keys.map(redis);}
try {
 staffLogin=await auth('login',credentials);bobLogin=await auth('login',JSON.parse(readFileSync('/opt/rustmush/dev1/r6a-evidence/actors.json','utf8')).bob);await reconnect();
 const base=registry();assert.equal(base.structural_version,1);assert.deepEqual(base.losses,{});const node=base.nodes.ceres.object;
 await command(staff,'@tacsim/time rate=1');await command(staff,'@teleport me=#'+base.nodes.malapert.coverage[0]);await command(staff,`@teleport ${bob.object}=#${base.nodes.ceres.coverage[0]}`);
 await command(staff,'@relay/check','Bindings OK');await command(staff,'@unit/show #'+node,'relay_station');
 const covered=await command(staff,'@recovery/status');assert(covered.includes('approved-operation loss'));assert(!covered.includes('cooldown runs to'));
 await command(staff,'@faction/create R7 Attention Fixture');await command(staff,`@faction/add R7 Attention Fixture=${staff.name}`);await command(staff,'@faction/resource R7 Attention Fixture=attention/999/controlled gate');await command(staff,'@faction/resource R7 Attention Fixture=legitimacy/5/controlled gate');
 await command(staff,'@front/create R7 Attention Fixture=R7 legitimacy crisis');
 const frontKeys=shell('redis-cli',['-p','6788','--scan','--pattern','mod:faction:front:*']).split('\n').filter(Boolean);const front=frontKeys.map(redis).find(f=>f.title==='R7 legitimacy crisis');assert(front);
 await command(staff,`@clock/create ${front.id}=R7 public condemnation/4`);
 const clockKeys=shell('redis-cli',['-p','6788','--scan','--pattern','mod:faction:clock:*']).split('\n').filter(Boolean);const clock=clockKeys.map(redis).find(c=>c.front===front.id);assert(clock);await command(staff,`@clock/set ${clock.id}=on:relay.node_destroyed`);
 await damage(node,'power',0);assert(registry().pairs.luna_ceres.halves.some(h=>h.damaged));assert.deepEqual(registry().losses,{});
 await command(staff,'@relay/power luna_ceres.ceres=on');assert(registry().pairs.luna_ceres.halves.some(h=>h.damaged));
 const flight=await send('pending-physical-leg');const frozen=await until(()=>envelope(flight.envelope)?.transit?.flight,'physical fallback frozen');assert(!box(bob,flight.id));
 await damage(node,'power',200);assert(registry().pairs.luna_ceres.halves.every(h=>!h.damaged&&!h.destroyed));
 milestone('power-disablement-and-repair',{node,physicalEnvelope:flight.envelope,frozen});
 await command(staff,`+relay/grant luna_ceres/${staff.object}=standard`);await command(staff,`@prop/jam #${base.nodes.ceres.place}=100`);
 const queued=await send('queued-at-destruction');assert(envelope(queued.envelope).transit.legs.some(l=>l.pair));
 await damage(node,'core',0);const loss=registry().losses.ceres;assert.equal(loss.responsible,actor(staff));assert(Number.isInteger(loss.canon));assert(registry().pairs.luna_ceres.halves.some(h=>h.destroyed));
 await command(staff,'@relay/show luna_ceres','orphaned=true');await command(staff,'+relay/status luna_ceres','permanent Exchange anathema');
 await command(staff,`+relay/policy luna_ceres/${staff.object}=served,operator.gate,manual cure cannot lift anathema`,'policy recorded');await command(staff,'+relay/status luna_ceres','Suspended');
 const uninsured=await command(staff,'@recovery/status','Tycho cover voided');assert(uninsured.includes('cooldown runs to'));assert(!uninsured.includes('so no cooldown'));
 await command(staff,`@prop/jam #${base.nodes.ceres.place}=0`);const rerouted=await until(()=>{const e=envelope(queued.envelope);return e?.transit?.flight&&e.transit.legs.every(l=>!l.pair)&&e;},'queued traffic falls back');
 assert.deepEqual(envelope(flight.envelope).transit.flight,frozen);
 const after=factions();assert(after.every(f=>f.relay_condemnations?.length===1));const cell=after.find(f=>f.name==='R7 Attention Fixture');assert.equal(cell.resources.attention,1000);assert.equal(cell.resources.legitimacy,4);assert.equal(redis('mod:faction:clock:'+clock.id).filled,1);
 milestone('physical-loss-and-single-political-insurance-response',{loss,queued:rerouted,coveredBefore:covered,coverAfter:uninsured,factions:after.map(f=>({id:f.id,condemnations:f.relay_condemnations.length})),attention:cell.resources.attention,legitimacy:cell.resources.legitimacy});
 await crash('terminal-half-death-and-frozen-leg-restart');await reconnect();assert.deepEqual(registry().losses.ceres,loss);assert.deepEqual(envelope(flight.envelope).transit.flight,frozen);await command(staff,'@relay/show luna_ceres','orphaned=true');await command(staff,'@recovery/status','Tycho cover voided');
 for(let i=0;i<2;i++){await damage(node,'core',1000);await damage(node,'core',0);}await damage(node,'core',1000);
 assert.deepEqual(registry().losses.ceres,loss);assert(registry().pairs.luna_ceres.halves.some(h=>h.destroyed));const repeated=factions().find(f=>f.name==='R7 Attention Fixture');assert.equal(repeated.resources.attention,1000);assert.equal(repeated.resources.legitimacy,4);assert.equal(repeated.relay_condemnations.length,1);assert.equal(redis('mod:faction:clock:'+clock.id).filled,1);
 await command(staff,'@relay/enable luna_ceres.ceres','cannot be repaired');await command(staff,'+relay/status luna_ceres','Suspended');milestone('repair-and-replay-cannot-resurrect-or-repeat',{loss,clock:redis('mod:faction:clock:'+clock.id),resources:repeated.resources});
 await command(staff,'@tacsim/time rate=240');await until(()=>box(bob,queued.id)&&box(bob,flight.id),'both conventional arrivals',120);assert.equal(service().receipts.filter(r=>r.envelope===queued.envelope).length,0);assert.equal(service().receipts.filter(r=>r.envelope===flight.envelope).length,0);
 await until(()=>envelopes().every(e=>!['queued','transmitted'].includes(e.state)),'all returns drain',120);await command(staff,'@tacsim/time rate=24');await command(bob,'pub R7 OOC remains immediate','R7 OOC remains immediate');
 milestone('real-mail-delivered-on-fallback',{mail:[queued.id,flight.id],pending:0});evidence.passed=true;
} catch(error){evidence.failure=String(error.stack??error);process.exitCode=1;} finally{close();}
