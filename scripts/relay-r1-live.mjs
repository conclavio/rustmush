#!/usr/bin/env node
// R1 live acceptance against a disposable instance. Node >=22, no npm dependencies.
// Usage: node scripts/relay-r1-live.mjs <exercise|verify> <https-url> <credentials.json> <output-dir> <redis-port>
// credentials: {"account":"...","password":"..."}; never included in evidence.
// Use NODE_EXTRA_CA_CERTS for a private test certificate. TLS verification stays on.
// This creates a test account/channel/mail and changes relay availability; it does not reset a server.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { join } from 'node:path';
const [mode, target, credentialsPath, output, redisPort] = process.argv.slice(2);
assert(['exercise','verify'].includes(mode) && target && credentialsPath && output && /^\d+$/.test(redisPort || ''),
  'usage: relay-r1-live.mjs exercise|verify https-url credentials.json output-dir redis-port');
const credentials = JSON.parse(readFileSync(credentialsPath, 'utf8'));
mkdirSync(output, { recursive: true });
const evidence = { mode, target, at: new Date().toISOString(), checks: [] };
const sockets = [];
const sleep = ms => new Promise(r => setTimeout(r, ms));
function text(value) {
  if (typeof value === 'string') return value;
  if (Array.isArray(value)) return value.map(text).join('');
  if (value && typeof value === 'object') {
    if (typeof value.text === 'string') return value.text;
    return Object.values(value).map(text).filter(Boolean).join(' ');
  }
  return '';
}
async function auth(route, body) {
  const response = await fetch(target + '/api/auth/' + route, {
    method: 'POST', headers: {'content-type': 'application/json'}, body: JSON.stringify(body),
    signal: AbortSignal.timeout(15000),
  });
  assert.equal(response.status, 200, 'authentication endpoint ' + route + ': ' + response.status);
  return response.json();
}
async function connect(login) {
  const ws = new WebSocket(target.replace(/^http/, 'ws') + '/ws?char=' + login.characters[0].key,
    'rustmush.v1.' + login.token);
  sockets.push(ws);
  const frames = [];
  ws.addEventListener('message', event => {
    if (typeof event.data !== 'string') return;
    const frame = JSON.parse(event.data);
    if (frame.t === 'ping') ws.send(JSON.stringify({t:'pong',d:{nonce:frame.d.nonce}}));
    else frames.push(frame);
  });
  await new Promise((resolve,reject) => {
    const timer = setTimeout(() => reject(new Error('WebSocket open timeout')),15000);
    ws.addEventListener('open', () => {clearTimeout(timer);resolve();},{once:true});
    ws.addEventListener('error', () => {clearTimeout(timer);reject(new Error('WebSocket connection failed'));},{once:true});
  });
  const client = {ws, frames, name:login.characters[0].name};
  await wait(client, f => f.t === 'hello', 0);
  await command(client, 'OUTPUTSUFFIX __R1_DONE__');
  return client;
}
async function wait(client, predicate, start) {
  const end = Date.now() + 15000;
  while (Date.now() < end) {
    const found = client.frames.slice(start).find(predicate);
    if (found) return found;
    assert(client.ws.readyState === WebSocket.OPEN, 'connection closed while waiting');
    await sleep(25);
  }
  throw new Error('No expected frame: ' + JSON.stringify(client.frames.slice(start)).slice(0,4000));
}
async function command(client, line, expected) {
  const start = client.frames.length;
  client.ws.send(JSON.stringify({t:'line',d:line}));
  await wait(client, f => text(f.d).includes('__R1_DONE__'), start);
  const reply = client.frames.slice(start).filter(f => !text(f.d).includes('__R1_DONE__')).map(f => text(f.d)).join('\n');
  if (expected) assert(reply.includes(expected), line + ': ' + reply);
  if (!line.startsWith('OUTPUT')) evidence.checks.push({actor:client.name,line,reply});
  return reply;
}
function registry() {
  return JSON.parse(execFileSync('redis-cli',['-p',redisPort,'--raw','JSON.GET','mod:sim:relay:registry'],{encoding:'utf8'}));
}
function save(name,value) { writeFileSync(join(output,name),JSON.stringify(value,null,2)+'\n',{mode:0o600}); }
try {
  const staff = await connect(await auth('login', credentials));
  const before = registry();
  assert.equal(Object.keys(before.nodes).length,2);
  assert.equal(Object.keys(before.pairs).length,1);
  await command(staff,'@relay/check','Bindings OK');
  const details = await command(staff,'@relay/show luna_ceres','luna_ceres.ceres');
  assert(details.includes('luna_ceres.luna') && details.length < 8000, 'bounded two-half diagnostic');
  if (mode === 'exercise') {
    const countBefore = execFileSync('redis-cli',['-p',redisPort,'--raw','DBSIZE'],{encoding:'utf8'}).trim();
    await command(staff,'@atlas/pack apply tacsim.relay.luna_ceres');
    assert.deepEqual(registry(),before,'same-pack application preserves registry');
    evidence.databaseKeysBeforeReapply = Number(countBefore);
    for (const [line, state] of [
      ['@relay/disable luna_ceres.luna','Degraded'],
      ['@relay/enable luna_ceres.luna','Live'],
      ['@relay/power luna_ceres.ceres=off','Degraded'],
      ['@relay/power luna_ceres.ceres=on','Live'],
      ['@relay/staff luna_ceres.ceres=off','Degraded'],
      ['@relay/staff luna_ceres.ceres=on','Live'],
    ]) {
      await command(staff,line,'revision');
      await command(staff,'@relay/check',state);
    }
    await command(staff,'@relay/disable luna_ceres.luna','revision');
    const disabled = registry();
    await command(staff,'@relay/disable luna_ceres.luna','no change');
    assert.deepEqual(registry(),disabled,'idempotent disable leaves durable revision unchanged');
    const visitorAccount = 'r1v' + randomBytes(4).toString('hex');
    const visitorName = 'R1Visitor' + randomBytes(3).toString('hex');
    const visitor = await connect(await auth('register', {account:visitorAccount,password:randomBytes(24).toString('hex'),char_name:visitorName}));
    const denied = await command(visitor,'@relay/show luna_ceres','Permission denied');
    assert.equal(await command(visitor,'@relay/show nonexistent','Permission denied'),denied);
    assert.equal(await command(visitor,'@relay/disable luna_ceres.luna','Permission denied'),denied);
    assert.deepEqual(registry(),disabled,'unauthorized command leaves durable state unchanged');
    const half = disabled.pairs.luna_ceres.halves[0].object;
    await command(staff,'@teleport #' + half + '=#0','cannot be moved');
    await command(staff,'@behavior/remove #' + half + '=sim.relay_half','still referenced');
    await command(staff,'@relay/check','Bindings OK');
    await command(staff,'@teleport me=#0');
    await command(staff,'@teleport ' + visitorName + '=#0');
    let start = visitor.frames.length;
    await command(staff,'say R1-live-speech');
    await wait(visitor,f => text(f.d).includes('R1-live-speech'),start);
    start = visitor.frames.length;
    await command(staff,'page ' + visitorName + '=R1-live-page');
    await wait(visitor,f => text(f.d).includes('R1-live-page'),start);
    await command(staff,'@channel/create R1Gate');
    await command(staff,'addcom r1=R1Gate');
    await command(visitor,'addcom r1=R1Gate');
    start = visitor.frames.length;
    await command(staff,'r1 R1-live-channel');
    await wait(visitor,f => text(f.d).includes('R1-live-channel'),start);
    start = visitor.frames.length;
    await command(staff,'@mail/quick ' + visitorName + '=R1-live-mail/R1-live-body');
    await wait(visitor,f => f.t === 'mail_notice' && f.d.subject === 'R1-live-mail',start);
    await command(visitor,'@mail 1','R1-live-body');
    evidence.traffic = ['room speech','page','channel','mail notice and body'];
    save('before-restart.json',disabled);
    evidence.finalRegistry = registry();
    assert.deepEqual(evidence.finalRegistry,disabled,'traffic does not change relay catalog');
  } else {
    const disabled = JSON.parse(readFileSync(join(output,'before-restart.json'),'utf8'));
    assert.deepEqual(before,disabled,'restart preserves exact registry including disabled flag');
    await command(staff,'@relay/check','Degraded');
    await command(staff,'@relay/enable luna_ceres.luna','revision');
    await command(staff,'@relay/check','Live');
    evidence.finalRegistry = registry();
    assert.equal(evidence.finalRegistry.revision,disabled.revision+1);
    save('final-registry.json',evidence.finalRegistry);
  }
  evidence.passed = true;
} catch(error) {
  evidence.passed = false;
  evidence.error = error.message;
  process.exitCode = 1;
} finally {
  save(mode + '.json',evidence);
  for(const ws of sockets) ws.close();
  console.log(JSON.stringify({mode,passed:evidence.passed,checks:evidence.checks.length,error:evidence.error}));
}

