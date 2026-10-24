#!/usr/bin/env node
// Real R8a retail gate on disposable dev1. Money is posted by normal commands;
// Redis is read-only here. The surrounding harness snapshots/restores test state.
import {assert,readFileSync,randomBytes,evidence,credentials,sockets,sleep,shell,redis,envelope,registry,save,auth,connect,command,until,crash,envelopes,actor,service,mail,box,milestone,close} from './relay-live-client.mjs';
let staff,bob,carol;let logins;
const market=()=>service().commerce;
const account=id=>redis('mod:economy:account:'+id);
function accounts(){const keys=shell('redis-cli',['-p','6788','--scan','--pattern','mod:economy:account:*']).split('\n').filter(Boolean);assert(keys.length<=2048);return keys.map(redis);}
const total=()=>accounts().filter(a=>a.instrument==='tacsim.credit').reduce((sum,a)=>sum+BigInt(a.posted_balance),0n).toString();
async function reconnect(){for(const ws of sockets) if(ws.readyState===WebSocket.OPEN) ws.close();await sleep(300);[staff,bob,carol]=await Promise.all(logins.map(connect));}
async function send(from,targets,body,label){const subject='R8a-'+label+'-'+randomBytes(3).toString('hex');await command(from,`@mail/quick ${targets.map(c=>c.object).join(',')}=${subject}/${body}`,'queued for IC transit');const copies=await until(()=>{const list=envelopes().filter(e=>e.dispatch?.key.endsWith(':out')&&e.issuer===actor(from)&&mail(Number(e.dispatch.key.split(':')[2]))?.subject===subject&&e.transit);return list.length===targets.length&&list;},'copies accepted '+label);const id=Number(copies[0].dispatch.key.split(':')[2]);return{id,copies,subject,bytes:Buffer.byteLength(subject+body)};}
async function quote(from,e,kind,payer){const q=market().next;await command(from,`+relay/quote ${e.id}=${kind},${payer}`,`Quote ${q}:`);return q;}
async function buy(from,q){await command(from,'+relay/buy '+q,'Reserved');return market().purchases[q];}
async function accelerate(predicate,label){await command(staff,'@tacsim/time rate=3600');const end=Date.now()+120000;try{while(Date.now()<end){if(predicate())return;await command(staff,'@prop/pass');await sleep(150);}throw new Error('accelerated timeout '+label);}finally{await command(staff,'@tacsim/time rate=1');}}
try {
 const actors=JSON.parse(readFileSync('/opt/rustmush/dev1/r6a-evidence/actors.json','utf8'));
 logins=[await auth('login',credentials),await auth('login',actors.bob),await auth('login',actors.carol)];await reconnect();
 await until(()=>market()?.capital_transaction,'Exchange funded');const initial=market();assert.equal(account(initial.payee).posted_balance,100000);assert.equal(account(1).posted_balance,4900000);
 const base=registry();await command(staff,'@tacsim/time rate=1');await command(staff,`@teleport ${bob.object}=#${base.nodes.malapert.coverage[0]}`);for(const c of [staff,carol])await command(staff,`@teleport ${c.object}=#${base.nodes.ceres.coverage[0]}`);
 for(const c of [bob,carol]) {await command(staff,`+relay/grant luna_ceres/${c.object}=deferred`);await command(staff,`@account/open ${c.object}=tacsim.credit`);}
 const payer=accounts().find(a=>a.owner.Object===actor(bob)).id;const empty=accounts().find(a=>a.owner.Object===actor(carol)).id;
 assert.equal(account(payer).posted_balance,0);assert.equal(account(empty).posted_balance,0);await command(staff,`@econ/pay 1=${payer} 10000`);assert.equal(account(payer).posted_balance,10000);
 const conserved=total();const exchangeStart=account(initial.payee).posted_balance;let charged=0;
 await command(bob,'+relay','free Deferred');milestone('funded-upgrade-and-authorized-payer',{market:initial,payer,empty,total:conserved,clearing:account(1).posted_balance});
 await command(staff,`@prop/jam #${base.nodes.ceres.place}=100`);
 const first=await send(bob,[carol],'\u00e9'.repeat(100),'priority');const q=await quote(bob,first.copies[0],'priority',payer);const p=market().purchases[q];assert.equal(p.bytes,first.bytes);assert.equal(p.units,Math.ceil(first.bytes/128));assert.equal(p.total_minor,p.units*500);
 await command(bob,`+relay/quote ${first.copies[0].id}=priority,${empty}`,'No usable payer');
 await crash('quoted-before-reservation');await reconnect();assert.equal(market().purchases[q].phase,'quoted');assert.equal(account(payer).held_amount,0);
 await buy(bob,q);await command(bob,'+relay/buy '+q,'Reserved');assert.equal(account(payer).held_amount,p.total_minor);assert.equal(account(payer).posted_balance,10000);
 await command(bob,'@hold/release '+market().purchases[q].hold);assert.equal(account(payer).held_amount,p.total_minor);
 await crash('reserved-before-transmission');await reconnect();assert.equal(market().purchases[q].phase,'reserved');assert.equal(account(payer).held_amount,p.total_minor);assert(!box(carol,first.id));
 await command(staff,`@prop/jam #${base.nodes.ceres.place}=0`);await command(staff,'@prop/pass');await until(()=>market().purchases[q].phase==='consumed','capture');charged+=p.total_minor;
 const settled=market().purchases[q];const consumedHold=redis('mod:economy:hold:'+settled.hold);assert.equal(consumedHold.status.Consumed.transaction,settled.transaction);assert.equal(consumedHold.status.Consumed.amount,settled.total_minor);await command(bob,'@econ/history '+payer,'relay quote');await command(staff,'@econ/audit','tx #'+settled.transaction);milestone('journal-and-consumed-hold',{purchase:settled,hold:consumedHold});await crash('settled-money-and-transmission');await reconnect();assert.deepEqual(market().purchases[q],settled);await command(bob,'+relay/buy '+q,'Consumed');await command(bob,'+relay/cancel '+q,'no refund');
 assert.equal(account(payer).posted_balance,10000-charged);assert.equal(account(payer).held_amount,0);assert.equal(account(initial.payee).posted_balance,exchangeStart+charged);assert.equal(total(),conserved);
 await until(()=>box(carol,first.id),'Priority inbox arrival');assert.equal(service().receipts.filter(r=>r.envelope===first.copies[0].id).length,1);milestone('priority-reserve-capture-three-process-deaths',{quote:settled,payer:account(payer),payee:account(initial.payee),total:total()});
 const fan=await send(bob,[staff,carol],'\u00e9'.repeat(65),'fanout');const qs=[];
 for(const e of fan.copies){const id=await quote(bob,e,'standard',payer);const p=await buy(bob,id);assert.equal(p.bytes,fan.bytes);assert.equal(p.total_minor,Math.ceil(fan.bytes/128)*100);qs.push(id);}
 assert(qs.every(id=>market().purchases[id].phase==='reserved'));await command(staff,'@tacsim/time rate=24');await until(()=>qs.every(id=>market().purchases[id].phase==='consumed'),'Standard delayed consumption');await command(staff,'@tacsim/time rate=1');charged+=qs.reduce((n,id)=>n+market().purchases[id].total_minor,0);await until(()=>box(staff,fan.id)&&box(carol,fan.id),'both recipient inboxes');milestone('standard-utf8-fanout',{bytes:fan.bytes,quotes:qs.map(id=>market().purchases[id]),total:total()});
 await command(staff,`@prop/jam #${base.nodes.ceres.place}=100`);
 const cancelled=await send(bob,[carol],'cancel upgrade expiry','refund');const standard=await quote(bob,cancelled.copies[0],'standard',payer);await buy(bob,standard);await command(bob,'+relay/cancel '+standard,'Released');assert.equal(account(payer).held_amount,0);
 const priority=await quote(bob,cancelled.copies[0],'priority',payer);await buy(bob,priority);assert(market().purchases[priority].total_minor>market().purchases[standard].total_minor);
 await command(staff,`@prop/jam #${base.nodes.ceres.coverage[0]}=100`);
 await command(staff,`+relay/policy luna_ceres/${carol.object}=suspended,operator.r8a,recipient denied`);
 await accelerate(()=>market().purchases[priority].phase==='released','unused reservation expiry');assert(!box(carol,cancelled.id));assert(['queued','transmitted'].includes(envelope(cancelled.copies[0].id).state));await command(staff,`@prop/jam #${base.nodes.ceres.place}=0`);await command(staff,`@prop/jam #${base.nodes.ceres.coverage[0]}=0`);assert.equal(account(payer).held_amount,0);assert.equal(account(payer).posted_balance,10000-charged);
 await crash('unused-release-restart');await reconnect();await command(bob,'+relay/cancel '+priority,'Released');assert.equal(account(payer).held_amount,0);await command(staff,`+relay/policy luna_ceres/${carol.object}=served,operator.r8a,arrears fixture cleared`);milestone('upgrade-cancel-receiver-denial-expiry-release',{standard:market().purchases[standard],priority:market().purchases[priority],total:total()});
 const poor=await send(carol,[bob],'empty account','poor');const poorq=await quote(carol,poor.copies[0],'priority',empty);await command(carol,'+relay/buy '+poorq,'Reservation refused');assert.equal(market().purchases[poorq].phase,'quoted');assert.equal(account(empty).held_amount,0);
 const big=await send(bob,[carol],'x'.repeat(1100),'oversize');await command(bob,`+relay/quote ${big.copies[0].id}=priority,${payer}`,'1024 content bytes');
 await accelerate(()=>box(bob,poor.id)&&box(carol,big.id)&&box(carol,cancelled.id),'free and conventional arrivals');assert.equal(account(empty).posted_balance,0);assert.equal(account(payer).posted_balance,10000-charged);assert.equal(account(initial.payee).posted_balance,exchangeStart+charged);assert.equal(total(),conserved);
 await command(bob,'+relay/purchases','Consumed');await command(carol,'pub R8a OOC immediate','R8a OOC immediate');await command(staff,'@tacsim/time rate=24');
 milestone('free-deferred-insufficient-funds-and-large-fallback',{poorQuote:market().purchases[poorq],charged,total:total(),payer:account(payer),payee:account(initial.payee)});evidence.passed=true;
} catch(error){evidence.failure=String(error.stack??error);process.exitCode=1;} finally{close();}
