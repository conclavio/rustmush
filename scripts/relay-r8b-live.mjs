#!/usr/bin/env node
// R8b: real IC cadence, money, subscription policy and process deaths on dev1.
// Redis is read-only. Normal commands own money, membership, title and transit.
import {assert,readFileSync,randomBytes,evidence,sockets,sleep,shell,redis,registry,auth,connect,command,until,crash,envelopes,actor,service,mail,box,receipt,milestone,close,credentials,text} from './relay-live-client.mjs';
let staff,bob,carol,logins;
const billing=()=>service().billing;
const sub=id=>billing().subscriptions[id];
const obligation=id=>redis('mod:economy:obligation:'+sub(id).obligation);
const account=id=>redis('mod:economy:account:'+id);
function docs(pattern){const keys=shell('redis-cli',['-p','6788','--scan','--pattern',pattern]).split('\n').filter(Boolean);assert(keys.length<4096);return keys.map(redis);}
const accounts=()=>docs('mod:economy:account:*');
const total=()=>accounts().filter(a=>a.instrument==='tacsim.credit').reduce((n,a)=>n+BigInt(a.posted_balance),0n).toString();
function now(){const head=redis('mod:rustmush_kernel_time:world_ic/head');const e=redis('mod:rustmush_kernel_time:world_ic/epoch/'+String(head.generation).padStart(20,'0'));return e.ic_anchor+(Math.floor(Date.now()/1000)-e.real_anchor)*e.rate;}
async function reconnect(){for(const ws of sockets)if(ws.readyState===WebSocket.OPEN)ws.close();await sleep(300);[staff,bob,carol]=await Promise.all(logins.map(connect));}
async function advanceTo(ic){assert(ic>now());await command(staff,'@tacsim/time rate=3600');try{await until(()=>now()>=ic,'IC boundary',120);}finally{await command(staff,'@tacsim/time rate=1');}assert(now()<ic+7200,'clock boundary overshoot');}
async function nextBill(id){const before=obligation(id).periods_settled;await advanceTo(obligation(id).schedule.next_due+5);await until(()=>obligation(id).periods_settled>before,'real scheduled collection',60);await command(staff,'@prop/pass');}
async function quote(c,principal,kind,payer){const id=billing().next;await command(c,`+relay/subscribe ${principal}=${kind},${payer}`,`Subscription ${id} for`);return id;}
async function enroll(c,id){await command(c,'+relay/enroll '+id,'Active');return obligation(id);}
async function send(c,to,label){const subject='R8b-'+label+'-'+randomBytes(3).toString('hex');await command(c,`@mail/quick ${to.object}=${subject}/live subscription crossing`,'queued for IC transit');return until(()=>envelopes().find(e=>e.issuer===actor(c)&&e.dispatch?.key.endsWith(':out')&&mail(Number(e.dispatch.key.split(':')[2]))?.subject===subject),'mail accepted');}
async function relay(e,kind){await command(staff,'@prop/pass');const r=await until(()=>receipt(e.id),'relay receipt',90);assert.equal(r.ticket.class,kind);return r;}
try{
 const actors=JSON.parse(readFileSync('/opt/rustmush/dev1/r6a-evidence/actors.json','utf8'));
 logins=[await auth('login',credentials),await auth('login',actors.bob),await auth('login',actors.carol)];await reconnect();
 const base=registry(),payee=service().commerce.payee;const exchangeStart=account(payee).posted_balance;assert.equal(exchangeStart,100000);assert(!Object.keys(billing().subscriptions).length);
 await command(staff,'@tacsim/time rate=1');
 await command(staff,`@teleport ${bob.object}=#${base.nodes.malapert.coverage[0]}`);
 for(const c of [staff,carol])await command(staff,`@teleport ${c.object}=#${base.nodes.ceres.coverage[0]}`);
 for(const c of [bob,carol]){await command(staff,`+relay/grant luna_ceres/${c.object}=deferred`);await command(staff,`@account/open ${c.object}=tacsim.credit`);}
 const payer=accounts().find(a=>a.owner.Object===actor(bob)).id,empty=accounts().find(a=>a.owner.Object===actor(carol)).id;
 await command(staff,`@econ/pay 1=${payer} 10000`);const conserved=total();
 await command(bob,`+relay/subscribe ${carol.object}=priority,${payer}`,'No such represented');
 await command(bob,`+relay/subscribe ${bob.object}=priority,${empty}`,'No usable payer');
 const id=await quote(bob,bob.object,'priority',payer);assert.equal(account(payer).posted_balance,10000);
 await crash('quoted-before-enrollment');await reconnect();assert.equal(sub(id).phase,'quoted');assert.equal(sub(id).obligation,null);
 await enroll(bob,id);await command(bob,'+relay','your class Deferred');assert.equal(account(payer).posted_balance,10000);
 await crash('enrolled-before-first-settlement');await reconnect();await command(bob,'+relay/enroll '+id,'Active');
 await until(()=>obligation(id).settled_total===1,'first real scheduled installment',60);assert.equal(account(payer).posted_balance,0);assert.equal(account(payee).posted_balance,exchangeStart+10000);
 await command(bob,'+relay','your class Priority');const first=await send(bob,carol,'paid-priority');await relay(first,'priority');await until(()=>box(carol,Number(first.dispatch.key.split(':')[2])),'Priority inbox arrival');
 const settled=obligation(id);await crash('settled-installment-and-transmitted-message');await reconnect();assert.equal(obligation(id).settled_total,1);assert.equal(account(payee).posted_balance,exchangeStart+10000);
 await command(staff,'@tacsim/time rate=24');await sleep(1000);await command(staff,'@tacsim/time rate=1');await sleep(31000);assert.equal(obligation(id).settled_total,1);assert.equal(obligation(id).periods_settled,settled.periods_settled);
 milestone('real-priority-installment-delivery-restarts-and-rate-idempotency',{subscription:sub(id),obligation:obligation(id),payer:account(payer),payee:account(payee),receipt:receipt(first.id),total:total()});
 await nextBill(id);assert.equal(obligation(id).missed_total,1);await command(bob,'+relay/status luna_ceres','Throttled');
 const slow=await send(bob,carol,'throttled-standard');await command(staff,'@prop/pass');assert.equal(redis('mod:propagation:envelope:'+slow.id).transit.service.class,'standard');
 await command(staff,'@tacsim/time rate=24');await relay(slow,'standard');await command(staff,'@tacsim/time rate=1');
 await nextBill(id);assert.equal(obligation(id).missed_total,2);await command(bob,'+relay/status luna_ceres','Suspended');
 const denied=await send(bob,carol,'suspended-fallback');await command(staff,'@prop/pass');assert(!receipt(denied.id));assert(!box(carol,Number(denied.dispatch.key.split(':')[2])));
 const notices=bob.frames.map(f=>text(f.d)).filter(t=>t.includes('Relay subscription'));assert(notices.length>0);milestone('suspension-uses-conventional-transit-and-notifies-principal',{envelope:redis('mod:propagation:envelope:'+denied.id),notices});
 await command(staff,`+relay/policy luna_ceres/${bob.object}=suspended,operator.r8b,independent incident`);
 await command(bob,'+relay/cure '+id,'200.00 cr');await command(carol,`+relay/cure ${id}=2,0`,'No such represented');await command(bob,`+relay/cure ${id}=2,0`,'Cure refused');
 await command(staff,`@econ/pay 1=${payer} 30000`);await command(bob,`+relay/cure ${id}=1,0`,'Cure refused');await command(bob,`+relay/cure ${id}=2,0`,'Arrears paid');
 assert.equal(account(payer).posted_balance,10000);assert.equal(obligation(id).service_terms.cured_misses,2);const cure=obligation(id).service_terms.cure_transaction;assert(cure);
 await crash('paid-cure-with-independent-sanction');await reconnect();await command(bob,`+relay/cure ${id}=2,0`,'Arrears paid');assert.equal(account(payer).posted_balance,10000);assert.equal(obligation(id).service_terms.cure_transaction,cure);
 await command(bob,'+relay/status luna_ceres','operator.r8b Suspended');await command(bob,'@econ/history '+payer,'service obligation');await command(staff,'@econ/audit','tx #'+cure);
 await command(staff,`+relay/policy luna_ceres/${bob.object}=served,operator.r8b,incident resolved`);await command(bob,'+relay/status luna_ceres','luna_ceres: Served');await command(bob,'+relay','your class Priority');
 const recovered=await send(bob,carol,'cured-priority');await relay(recovered,'priority');
 await nextBill(id);assert.equal(obligation(id).settled_total,2);assert.equal(account(payer).posted_balance,0);await command(bob,'+relay/unsubscribe '+id,'Cancelled');
 milestone('missed-bills-real-cure-source-isolation-and-recovered-delivery',{subscription:sub(id),obligation:obligation(id),cure,standardReceipt:receipt(slow.id),recoveredReceipt:receipt(recovered.id),total:total()});
 // Initial grace, later normal collection, cancellation and terminal debt.
 const early=await quote(carol,carol.object,'standard',empty);await enroll(carol,early);await until(()=>obligation(early).missed_total===1,'empty first installment',60);await command(carol,'+relay','your class Deferred');
 await command(staff,`@econ/pay 1=${empty} 5000`);await nextBill(early);assert.equal(obligation(early).settled_total,1);assert.equal(obligation(early).missed_total,1);await command(carol,'+relay/status luna_ceres','Throttled');
 await command(carol,`+relay/cure ${early}=1,0`,'Arrears paid');assert.equal(account(empty).posted_balance,0);await nextBill(early);await command(carol,'+relay/unsubscribe '+early,'Cancelled');
 await command(carol,`+relay/subscribe ${carol.object}=standard,${empty}`,'existing subscription first');await command(staff,`@econ/pay 1=${empty} 2500`);await command(carol,`+relay/cure ${early}=2,1`,'Arrears paid');assert.equal(obligation(early).status,'Cancelled');await command(carol,'+relay','your class Deferred');
 milestone('later-normal-payment-keeps-old-debt-and-cancelled-cure-stays-terminal',{obligation:obligation(early),total:total()});
 // Real seeded government institution, authorized representative and titled unit.
 const org=docs('mod:org:org:*').find(o=>o.name==='Freeport Registry').id;const treasury=accounts().find(a=>a.owner.Organization===org).id;const treasuryBefore=account(treasury).posted_balance;assert(treasuryBefore>=100000);
 await command(bob,`+relay/subscribe org:${org}=sovereign,${treasury}`,'No such represented');
 await command(staff,`@org/grant ${carol.object}=${org} economy.account.post`);await command(staff,`@org/grant ${carol.object}=${org} economy.account.view`);
 await command(carol,`+relay/subscribe org:${org}=sovereign,${treasury}`,'No treaty');await command(bob,`+relay/treaty org:${org}=2,unauthorized`,'Permission denied');
 await command(staff,`+relay/treaty org:${base.operator}=2,not a government`,'No recognized');await command(staff,`+relay/treaty org:${org}=2,Registry public service`,'Treaty offered');await command(carol,`+relay/treaty org:${org}`,'Current');
 const sovereign=await quote(carol,'org:'+org,'sovereign',treasury);await enroll(carol,sovereign);await until(()=>obligation(sovereign).settled_total===1,'Sovereign real installment',60);await command(carol,'+relay','your class Deferred');assert.equal(account(treasury).posted_balance,treasuryBefore-50000);
 const created=await command(staff,'@create R8bRegistryDispatch');const unit=Number(created.match(/#(\d+)/)?.[1]);assert(unit>0,created);await command(staff,`@teleport #${unit}=#${base.nodes.malapert.coverage[0]}`);await command(staff,`@asset/title #${unit}=org:${org}`,'Title recorded');await command(staff,`@teleport ${bob.object}=#${base.nodes.malapert.coverage[0]}`);
 // The network has no puppet command. Exercise the real recipient's government
 // service policy over ordinary +order ingress; outbound unit Sovereign class is
 // covered by the production engine scenario, which can submit as a non-player.
 await command(staff,`@teleport #${unit}=#${base.nodes.ceres.coverage[0]}`);await command(staff,`@teleport ${staff.object}=#${base.nodes.malapert.coverage[0]}`);
 const before=new Set(envelopes().map(e=>e.id));await command(staff,`+order #${unit}=R8b treaty dispatch`,'Queued revision');const order=await until(()=>envelopes().find(e=>e.issuer===actor(staff)&&e.recipient===unit&&!before.has(e.id)),'ordinary order to government unit');
 await command(staff,'@tacsim/time rate=3600');try{await relay(order,'deferred');await until(()=>redis('mod:propagation:envelope:'+order.id).state==='delivered','government unit receives order',60);}finally{await command(staff,'@tacsim/time rate=1');}
 milestone('real-government-authority-treaty-and-inbound-titled-unit-delivery',{organization:org,subscription:sub(sovereign),obligation:obligation(sovereign),receipt:receipt(order.id),treasury:account(treasury)});
 await crash('sovereign-settlement-before-exclusive-expiry');await reconnect();await advanceTo(sub(sovereign).until_ic+5);await until(()=>obligation(sovereign).status==='Cancelled','expired obligation sweep',60);const endedBalance=account(treasury).posted_balance;
 // A second installment may legitimately settle while crossing the intervening day.
 assert([treasuryBefore-50000,treasuryBefore-100000].includes(endedBalance));await command(carol,`+relay/treaty org:${org}`,'Expired');await command(carol,'+relay/subscriptions','Ended');
 await crash('expired-treaty-reboot');await reconnect();await sleep(31000);assert.equal(account(treasury).posted_balance,endedBalance);assert.equal(obligation(sovereign).status,'Cancelled');
 assert.equal(total(),conserved);assert.equal(account(payer).posted_balance,0);assert.equal(account(empty).posted_balance,0);
 await command(bob,'+relay/subscriptions','Cancelled');await command(carol,'@obligation/show '+sub(sovereign).obligation,'[cancelled]');await command(bob,'pub R8b OOC immediate','R8b OOC immediate');await command(staff,'@tacsim/time rate=24');
 milestone('exclusive-treaty-expiry-no-rebilling-notices-and-conservation',{obligation:obligation(sovereign),treasury:account(treasury),payee:account(payee),total:total(),expectedTotal:conserved});evidence.passed=true;
}catch(error){evidence.failure=String(error.stack??error);process.exitCode=1;}finally{close();}
