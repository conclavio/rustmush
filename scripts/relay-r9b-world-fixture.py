#!/usr/bin/env python3
"""Disposable dev1 whole-world snapshot/restore using verified empty Redis db3.
Gateway must be stopped. Retains root-only backup and verifies its content digest.
Restoring test accounts/payments is a gate reset, never a live product refund.
"""
import base64,hashlib,json,os,re,socket,subprocess,sys,time,tomllib
from pathlib import Path
root=Path('/opt/rustmush/dev1');out=root/'r9b-evidence'
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
mode=sys.argv[1]
if mode=='snapshot':
    assert not (out/'snapshot.json').exists(),'never replace the baseline snapshot'
    assert not keys(3),'backup database must be empty'
    source=keys(0);assert source and b'mod:sim:relay:registry' in source
    expiries={}
    for key in source:
        ttl=cmd('PTTL',key)
        if cmd('COPY',key,key,'DB',3):
            if ttl>=0:expiries[base64.b64encode(key).decode()]=int(time.time()*1000)+ttl
    cmd('SELECT',3)
    for key in source:cmd('PERSIST',key)
    backup=keys(3);assert backup,'empty backup'
    meta={'database':3,'keys':len(backup),'keyDigest':fingerprint(backup),'dataDigest':data_fingerprint(backup),'expiries':expiries}
    (out/'snapshot.json').write_text(json.dumps(meta)+'\n');os.chmod(out/'snapshot.json',0o600)
    config=root/'mushd.toml';(out/'pre-gate-mushd.toml').write_bytes(config.read_bytes());os.chmod(out/'pre-gate-mushd.toml',0o600)
    print(json.dumps({'snapshot':True,'keys':len(backup)}))
elif mode in ('restore','reset-failed'):
    result=json.loads((out/'live.json').read_text())
    if mode=='restore':
        assert result.get('passed'),'restore only after manufacture, freight and crash evidence is recorded'
    else:
        assert result.get('failure') and not result.get('passed'),'failed reset requires recorded failure'
        assert (out/'live-failed.json').read_bytes()==(out/'live.json').read_bytes(),'preserve failed evidence before reset'
    meta=json.loads((out/'snapshot.json').read_text());backup=keys(3)
    assert len(backup)==meta['keys'] and fingerprint(backup)==meta['keyDigest'] and data_fingerprint(backup)==meta['dataDigest'],'backup identity or content changed'
    assert b'mod:sim:relay:registry' in backup
    # Explicitly scoped to the authorized dev1 world; dev2 uses another service.
    cmd('SELECT',0);cmd('FLUSHDB')
    cmd('SELECT',3)
    expected=[]
    for key in backup:
        expiry=meta['expiries'].get(base64.b64encode(key).decode())
        if expiry is not None and expiry<=int(time.time()*1000):continue
        assert cmd('COPY',key,key,'DB',0,'REPLACE')==1
        if expiry is not None:
            cmd('SELECT',0);cmd('PEXPIREAT',key,expiry);cmd('SELECT',3)
        expected.append(key)
    restored=keys(0);assert restored==expected
    (root/'mushd.toml').write_bytes((out/'pre-gate-mushd.toml').read_bytes())
    (out/'restore.json').write_text(json.dumps({'restored':True,'purpose':mode,'keys':len(restored),'keyDigest':fingerprint(restored)})+'\n')
    print('R9b_BASELINE_WORLD_RESTORED')
else:raise AssertionError('unknown fixture action')
