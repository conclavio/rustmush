#!/usr/bin/env node
// R4: actual command/client, durable operator policy and recipient delivery.
import { assert,readFileSync,randomBytes,evidence,credentials,sockets,sleep,shell,redis,envelope,registry,save,text,auth,connect,wait,command,until,crash,envelopes,actor,transport,service,mail,box,receipt,milestone,close } from './relay-live-client.mjs';
let staff, bob, carol, staffLogin, bobLogin, carolLogin, jammed;
const resumeCleanup=process.argv.includes('--resume-cleanup');
if(resumeCleanup){Object.assign(evidence,JSON.parse(readFileSync('/opt/rustmush/dev1/r4-evidence/live.json','utf8')));assert(['sender-throttle-restart-and-board-parity','receiver-throttle-source-specific-restore','sender-suspension-fallback-and-restore','receiver-suspension-fallback-and-restore','independent-operator-fixture-boot','independent-operator-delivery-and-scope'].every(test=>evidence.milestones.some(m=>m.test===test)),'resume requires every policy scenario already passed');delete evidence.failure;}
async function reconnect() { for(const ws of sockets) if(ws.readyState===WebSocket.OPEN) ws.close(); await sleep(300); staff=await connect(staffLogin); bob=await connect(bobLogin); carol=await connect(carolLogin); }
async function policy(who,state,source,reason) { return command(carol,`+relay/policy luna_ceres/${who.object}=${state},operator.${source},${reason}`,'policy recorded'); }
async function send(label) {
 const subject='R4-'+label+'-'+randomBytes(3).toString('hex');
 await command(staff,`@mail/quick ${bob.object}=${subject}/${'x'.repeat(350)}`,'queued for IC transit');
 const e=await until(()=>envelopes().find(e=>e.dispatch?.key.endsWith(':out')&&e.recipient===actor(bob)&&mail(Number(e.dispatch.key.split(':')[2]))?.subject===subject&&e.transit?.service),'queue '+label);
 const id=Number(e.dispatch.key.split(':')[2]);assert(!box(bob,id));return {id,envelope:e.id,subject};
}
async function jam(on) { const place=registry().nodes.ceres.place;await command(staff,`@prop/jam #${place}=${on?100:0}`);jammed=on; }
async function boardQuery(c) {
 const id=9041,start=c.frames.length;c.ws.send(JSON.stringify({t:'req',d:{id,q:{t:'panel_subscribe',d:{feed:'relay.board',params:{actor:1,actual:true}}}}}));
 const f=await wait(c,f=>f.t==='reply'&&f.d.id===id,start);assert.equal(f.d.event.t,'panel');return f.d.event.d.data;
}
try {
 staffLogin=await auth('login',credentials);
 const accounts=JSON.parse(readFileSync('/opt/rustmush/dev1/r6a-evidence/actors.json','utf8'));
 bobLogin=await auth('login',accounts.bob);carolLogin=await auth('login',accounts.carol);await reconnect();
 if(resumeCleanup){
  const fixture=JSON.parse(readFileSync('/opt/rustmush/dev1/r4-evidence/independent-fixture.json','utf8'));
  assert.deepEqual(Object.keys(registry().pairs),['luna_ceres']);
  for(const row of fixture.created.reverse()){const obj=redis('obj:'+row.object);if(obj){assert(!obj.behaviors?.[row.behavior]);await command(staff,`@destroy #${row.object}`);}}
  await command(staff,'@relay/check','Bindings OK');
  await command(staff,'@tacsim/time rate=24');
  await command(staff,`+relay/grant luna_ceres/${staff.object}=standard`);await command(staff,`+relay/grant luna_ceres/${bob.object}=standard`);
  await command(carol,'+relay/audit luna_ceres','receiver restored');
  await command(bob,'pub R4 OOC remains immediate','R4 OOC remains immediate');
  await until(()=>envelopes().every(e=>!['queued','transmitted'].includes(e.state)),'queue drained',60);
  assert.deepEqual(service().restrictions,[]);
  milestone('seed-topology-restored',{revision:registry().revision,fixtureBindingsRemoved:fixture.created.map(r=>r.object),recoveredTeardown:true});
  evidence.passed=true;
 } else {
 const catalog=registry();
 assert.deepEqual(service().restrictions ?? [],[],'R3 upgrade starts with no policy restrictions; absent legacy field defaults empty');
 await command(staff,'@tacsim/time rate=1');
 await command(staff,'@teleport me=#'+catalog.nodes.malapert.coverage[0]);
 await command(staff,`@teleport ${bob.object}=#${catalog.nodes.ceres.coverage[0]}`);
 await command(bob,`+relay/policy luna_ceres/${staff.object}=suspended,operator.denied,not authorized`,'Permission denied');
 await command(staff,`@org/grant ${carol.object}=${catalog.operator} relay.manage`);
 await command(staff,`+relay/grant luna_ceres/${staff.object}=priority`);
 await command(staff,`+relay/grant luna_ceres/${bob.object}=priority`);
 await jam(true);const first=await send('sender-throttle');
 await policy(staff,'throttled','incident','Sender review pending');
 const ticket=await until(()=>{const t=envelope(first.envelope)?.transit?.service;return t?.class==='standard'&&t;},'sender ticket demoted');
 assert.equal(ticket.base_class,'priority');assert(ticket.throttled);
 assert((await command(staff,'+relay/status luna_ceres')).includes('Sender review pending'));
 assert(!(await command(bob,'+relay/status luna_ceres')).includes('Sender review pending'));
 const board=await boardQuery(staff);assert(JSON.stringify(board).includes('service Throttled'));assert(!JSON.stringify(board).includes('actual queued'));
 await crash('sender-throttle-durable-policy');await reconnect();
 assert((await command(staff,'+relay/status luna_ceres')).includes('Sender review pending'));
 await jam(false);assert(!box(bob,first.id));
 await until(()=>box(bob,first.id),'throttled Standard arrival',95);
 const firstReceipt=receipt(first.envelope);assert(firstReceipt.transmitted>=firstReceipt.queued+60);assert(firstReceipt.ticket.throttled);
 milestone('sender-throttle-restart-and-board-parity',{receipt:firstReceipt});
 await policy(staff,'served','incident','Sender cleared');
 await jam(true);const second=await send('receiver-throttle');
 await policy(bob,'throttled','incident','Receiver review');await policy(bob,'throttled','review','Independent reason');
 await until(()=>envelope(second.envelope)?.transit?.service?.class==='standard','receiver demotion');
 await policy(bob,'served','incident','First reason cleared');
 assert((await command(bob,'+relay/status luna_ceres')).includes('Throttled'));
 await policy(bob,'served','review','Second reason cleared');await jam(false);
 await until(()=>box(bob,second.id),'restored queued priority',30);
 assert.equal(receipt(second.envelope).ticket.class,'priority');
 milestone('receiver-throttle-source-specific-restore',{receipt:receipt(second.envelope)});
 for(const endpoint of ['sender','receiver']) {
  await command(staff,'@tacsim/time rate=1');await command(staff,`+relay/grant luna_ceres/${staff.object}=sovereign`);
  await jam(true);const item=await send(endpoint+'-suspension');
  await policy(endpoint==='sender'?staff:bob,'suspended','incident',endpoint+' suspended');
  const flight=await until(()=>{const t=envelope(item.envelope)?.transit;return t?.flight?.seconds>1&&t.legs.every(l=>!l.pair)&&t.flight;},'conventional fallback');
  assert(!box(bob,item.id));assert(!receipt(item.envelope));
  if(endpoint==='receiver'){await crash('receiver-suspension-durable-fallback');await reconnect();assert((await command(bob,'+relay/status luna_ceres')).includes('receiver suspended'));}
  await policy(endpoint==='sender'?staff:bob,'served','incident',endpoint+' restored');await jam(false);
  assert.deepEqual(envelope(item.envelope).transit.flight,flight,'restore preserves frozen light-time');
  await command(staff,'@tacsim/time rate=3600');await until(()=>box(bob,item.id),'conventional arrival',45);
  assert(!receipt(item.envelope),'no phantom consumed relay service');
  milestone(endpoint+'-suspension-fallback-and-restore',{envelope:item.envelope,flight});
 }
 // Controlled topology input for R4's independent-operator consumer. All
 // objects, placements and organization authority use ordinary commands. R9
 // will supply the manufacturing/deployment producer; this fixture cannot mint
 // a pair through any shipped player command. Restore the seed after the gate.
 await command(staff,'@tacsim/time rate=1');
 const original=registry(), fixture=structuredClone(original), created=[];
 const orgReply=await command(staff,'@org/create R4 Independent '+randomBytes(3).toString('hex'),'Created org #');
 const other=Number(orgReply.match(/Created org #(\d+)/)[1]);
 const pair=structuredClone(original.pairs.luna_ceres);pair.class_policy='private';pair.capacity_units_per_ic_second=4;
 for(const [i,key] of ['malapert','ceres'].entries()) {
  const node=structuredClone(original.nodes[key]);
  await command(staff,'@teleport me=#'+node.place);
  const n=await command(staff,'@create R4 fixture node '+i+' '+randomBytes(3).toString('hex'),'Created:');
  node.object=Number(n.match(/\(#(\d+)\)/)[1]);created.push({object:node.object,behavior:'sim.relay_node'});
  await command(staff,`@teleport #${node.object}=#${node.place}`,'Teleported');
  await command(staff,`@set #${node.object}=ENTER_OK`);
  const h=await command(staff,'@create R4 fixture half '+i+' '+randomBytes(3).toString('hex'),'Created:');
  const half=Number(h.match(/\(#(\d+)\)/)[1]);created.push({object:half,behavior:'sim.relay_half'});
  await command(staff,`give #${half}=#${node.object}`,'You give');
  await command(staff,`@behavior/add #${node.object}=sim.relay_node`);
  await command(staff,`@behavior/add #${half}=sim.relay_half`);
  node.operator=other;fixture.nodes['r4_'+key]=node;
  pair.halves[i]={...pair.halves[i],object:half,id:'r4_independent.half'+i,node:'r4_'+key};
 }
 fixture.pairs.r4_independent=pair;fixture.revision++;
 async function installFixture(catalog,label,teardown=[]) {
  await command(staff,'@dump','snapshot acknowledged by the store');
  for(const ws of sockets) if(ws.readyState===WebSocket.OPEN) ws.close();
  shell('systemctl',['stop','dev1mush-gateway']);
  // Teardown is the inverse of the controlled offline fixture install. Remove
  // only its known bindings in the same stopped-server interval as its topology;
  // boot correctly refuses any hardware binding absent from the registry.
  for(const row of teardown){const key='obj:'+row.object,obj=redis(key);assert(obj.name.startsWith('R4 fixture '));assert(obj.behaviors[row.behavior]);delete obj.behaviors[row.behavior];assert.equal(shell('redis-cli',['-p','6788','JSON.SET',key,'$',JSON.stringify(obj)]),'OK');}
  assert.equal(shell('redis-cli',['-p','6788','JSON.SET','mod:sim:relay:registry','$',JSON.stringify(catalog)]),'OK');
  shell('systemctl',['start','dev1mush-gateway']);
  await until(()=>{try{return shell('curl',['-fsS','--cacert','/opt/rustmush/dev1/tls/cert.pem','https://conclav.io:6790/']).length>0;}catch{return false;}},'gateway ready',60);
  await reconnect();await command(staff,'@relay/check','Bindings OK');milestone(label,{revision:catalog.revision,operators:[original.operator,other]});
 }
 save('independent-fixture.json',{original,fixture,created});
 await installFixture(fixture,'independent-operator-fixture-boot');
 await command(staff,'@atlas/pack plan tacsim.relay.luna_ceres','Nothing to do');
 await command(carol,`+relay/policy r4_independent/${bob.object}=suspended,operator.no,Wrong scope`,'Permission denied');
 await command(staff,'@teleport me=#'+original.nodes.malapert.coverage[0]);
 await policy(bob,'suspended','independent','Exchange denial still active');
 await command(staff,`+relay/grant r4_independent/${staff.object}=priority`);
 await jam(true);const independent=await send('independent');
 const independentRoute=envelope(independent.envelope).transit;assert(independentRoute.legs.some(l=>l.pair==='r4_independent'));assert(!independentRoute.legs.some(l=>l.pair==='luna_ceres'));
 await jam(false);await until(()=>box(bob,independent.id),'private operator arrival',30);
 assert.equal(receipt(independent.envelope).pair,'r4_independent');
 assert((await command(bob,'+relay/status luna_ceres')).includes('Suspended'));
 milestone('independent-operator-delivery-and-scope',{receipt:receipt(independent.envelope),operator:other});
 await policy(bob,'served','independent','Independent test complete');
 await command(staff,`+relay/grant r4_independent/${staff.object}=standard`);
 await command(staff,'@tacsim/time rate=3600');await until(()=>envelopes().every(e=>!['queued','transmitted'].includes(e.state)),'fixture traffic drained',45);
 const restored=structuredClone(original);restored.revision=registry().revision+1;await installFixture(restored,'seed-topology-restored',created);
 for(const row of created.reverse()) await command(staff,`@destroy #${row.object}`);
 await command(staff,'@tacsim/time rate=24');
 await command(carol,'+relay/audit luna_ceres','receiver restored');
 await command(bob,'pub R4 OOC remains immediate','R4 OOC remains immediate');
 await command(staff,`+relay/grant luna_ceres/${staff.object}=standard`);await command(staff,`+relay/grant luna_ceres/${bob.object}=standard`);
 await until(()=>envelopes().filter(e=>['queued','transmitted'].includes(e.state)).length===0,'queue drained',60);
 assert.deepEqual(service().restrictions,[]);
 evidence.passed=true;
 }
} catch(error){evidence.passed=false;evidence.failure=error.stack;throw error;}
finally { try {if(staff?.ws.readyState===WebSocket.OPEN){if(jammed)await jam(false);await command(staff,'@tacsim/time rate=24');}}catch{} close(); }
