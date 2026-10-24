#!/usr/bin/env python3
"""Root-only dev1 gate fixture. Snapshot/restore only Redis :6788 db0 via db11.

The two databases were approved as disposable dev1 test storage. The gateway
must be stopped. No credentials or key names are printed. Restore is a test-world
reset, never a product repair of a dead entangled pair. A historical, ended
approved-loss grant exercises the existing insurance consumer without claiming
that this gate tests loaner issuance (which belongs to E7).
"""
import base64,hashlib,json,os,re,socket,subprocess,sys,time,tomllib
from pathlib import Path
root=Path('/opt/rustmush/dev1');out=root/'r7-evidence'
assert root.resolve()==root and out.resolve().parent==root
assert subprocess.run(['systemctl','is-active','--quiet','dev1mush-gateway']).returncode!=0
sock=socket.create_connection(('127.0.0.1',6788),timeout=20);wire=sock.makefile('rb')
def read():
    line=wire.readline();assert line.endswith(b'\r\n')
    typ,data=line[:1],line[1:-2]
    if typ==b'+':return data
    if typ==b'-':raise RuntimeError(data.decode())
    if typ==b':':return int(data)
    if typ==b'$':
        n=int(data)
        if n<0:return None
        value=wire.read(n);assert wire.read(2)==b'\r\n';return value
    if typ==b'*':return [read() for _ in range(int(data))]
    raise AssertionError('unknown RESP type')
def cmd(*args):
    args=[a if isinstance(a,bytes) else str(a).encode() for a in args]
    sock.sendall(b'*'+str(len(args)).encode()+b'\r\n'+b''.join(b'$'+str(len(a)).encode()+b'\r\n'+a+b'\r\n' for a in args));return read()
def keys(db):
    cmd('SELECT',db);cursor=0;result=set()
    while True:
        cursor,chunk=cmd('SCAN',cursor,'COUNT',1000);result.update(chunk);assert len(result)<=100000
        if cursor==b'0':break
    return sorted(result)
def fingerprint(items):return hashlib.sha256(b'\0'.join(items)).hexdigest()
def data_fingerprint(items):
    h=hashlib.sha256()
    for key in items:
        payload=cmd('DUMP',key);assert payload is not None
        for part in (key,payload):h.update(len(part).to_bytes(8,'big'));h.update(part)
    return h.hexdigest()
def recovery_config():
    # Existing funded clearing account, used only as a checked provider identity;
    # this gate never applies for, issues or spends a recovery grant.
    cmd('SELECT',0)
    assert json.loads(cmd('JSON.GET','mod:economy:account:1'))['id']==1
    config=root/'mushd.toml';text=config.read_text();parsed=tomllib.loads(text)
    fields={'enabled':'true','pool_account':'1','tiers':'["scout"]'}
    if 'recovery' in parsed.get('tacsim',{}):
        match=re.search(r'(?ms)^\[tacsim\.recovery\]\s*\n(.*?)(?=^\[|\Z)',text);assert match
        body=match.group(1)
        for name,value in fields.items():
            pattern=rf'(?m)^{name}\s*=.*$'
            body=re.sub(pattern,f'{name} = {value}',body) if re.search(pattern,body) else f'{name} = {value}\n'+body
        text=text[:match.start(1)]+body+text[match.end(1):]
    else:text+='\n[tacsim.recovery]\n'+''.join(f'{k} = {v}\n' for k,v in fields.items())
    config.write_text(text);os.chmod(config,0o600)

mode=sys.argv[1]
if mode=='snapshot':
    assert not (out/'snapshot.json').exists(),'never replace the baseline snapshot'
    assert not keys(11),'backup database must be empty'
    source=keys(0);assert source and b'mod:sim:relay:registry' in source
    expiries={}
    for key in source:
        ttl=cmd('PTTL',key)
        if cmd('COPY',key,key,'DB',11):
            if ttl>=0:expiries[base64.b64encode(key).decode()]=int(time.time()*1000)+ttl
    cmd('SELECT',11)
    for key in source:cmd('PERSIST',key)
    backup=keys(11);assert backup,'empty backup'
    meta={'database':11,'keys':len(backup),'keyDigest':fingerprint(backup),'dataDigest':data_fingerprint(backup),'expiries':expiries}
    (out/'snapshot.json').write_text(json.dumps(meta)+'\n');os.chmod(out/'snapshot.json',0o600)
    config=root/'mushd.toml';(out/'pre-gate-mushd.toml').write_bytes(config.read_bytes());os.chmod(out/'pre-gate-mushd.toml',0o600)
    print(json.dumps({'snapshot':True,'keys':len(backup)}))
elif mode=='cover':
    meta=json.loads((out/'snapshot.json').read_text());assert len(keys(11))==meta['keys']
    cmd('SELECT',0)
    assert cmd('EXISTS','mod:campaign:recovery_grant:1')==0,'never replace a real historical grant'
    head=json.loads(cmd('JSON.GET','mod:rustmush_kernel_time:world_ic/head'))['generation']
    epoch=json.loads(cmd('JSON.GET',f'mod:rustmush_kernel_time:world_ic/epoch/{head:020d}'))
    # ic_anchor is the persisted world timestamp; recent completion makes the
    # default fourteen-day cooldown observable once the waiver is cancelled.
    now=epoch['ic_anchor']+max(0,int(time.time())-epoch['real_anchor'])*epoch['rate']
    if isinstance(now,dict):now=now['at']
    grant={'applicant':1,'contract':777777,'loaner':412,'tier':'scout','granted_at':now-100,'ended_at':now-1,'loss_covered':True}
    cmd('JSON.SET','mod:campaign:recovery_grant:1','.',json.dumps(grant))
    recovery_config()
    (out/'cover-fixture.json').write_text(json.dumps({'purpose':'Historical ended approved-loss grant; E7 issuance not under test','grant':grant})+'\n')
    print('R7_COVER_FIXTURE_INSTALLED')
elif mode=='repair-config':
    assert (out/'snapshot.json').exists() and (out/'cover-fixture.json').exists()
    recovery_config()
    print('R7_COVER_CONFIG_CORRECTED')
elif mode=='restore':
    assert json.loads((out/'live.json').read_text())['passed'],'restore only after the loss evidence is recorded'
    meta=json.loads((out/'snapshot.json').read_text());backup=keys(11)
    assert len(backup)==meta['keys'] and fingerprint(backup)==meta['keyDigest'] and data_fingerprint(backup)==meta['dataDigest'],'backup identity or content changed'
    assert b'mod:sim:relay:registry' in backup
    # Explicitly scoped to the authorized dev1 world; dev2 uses another service.
    cmd('SELECT',0);cmd('FLUSHDB')
    cmd('SELECT',11)
    expected=[]
    for key in backup:
        expiry=meta['expiries'].get(base64.b64encode(key).decode())
        if expiry is not None and expiry<=int(time.time()*1000):continue
        assert cmd('COPY',key,key,'DB',0,'REPLACE')==1
        if expiry is not None:
            cmd('SELECT',0);cmd('PEXPIREAT',key,expiry);cmd('SELECT',11)
        expected.append(key)
    restored=keys(0);assert restored==expected
    (root/'mushd.toml').write_bytes((out/'pre-gate-mushd.toml').read_bytes())
    (out/'restore.json').write_text(json.dumps({'restored':True,'keys':len(restored),'keyDigest':fingerprint(restored)})+'\n')
    print('R7_BASELINE_WORLD_RESTORED')
else:raise AssertionError('unknown fixture action')
