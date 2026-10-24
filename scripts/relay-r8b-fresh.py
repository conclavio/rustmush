#!/usr/bin/env python3
"""R8b real fresh-store boot/reboot on unused dev1 Redis database 8.

Never clears a database. Refuses nonempty input; retains the test database and
root-only server logs under dev1/r8b-evidence for inspection.
"""
import json
import os
from pathlib import Path
import subprocess
import time

root = Path('/opt/rustmush/dev1')
assert root.resolve() == root
out = root / 'r8b-evidence'
assert out.resolve().parent == root

def redis(*args):
    return subprocess.check_output(['redis-cli','-p','6788','-n','8','--raw',*args],text=True).strip()

assert redis('DBSIZE') == '0', 'fresh gate requires empty dev1 database 8'
config = out / 'fresh.toml'
config.write_text(f'''[server]
data_dir = "{out}/fresh-data"
help_dir = "{root}/build/docs/help"
[store]
store = "redis"
redis_url = "redis://127.0.0.1:6788/8"
[tacsim]
enabled = true
[ipc]
listen = "{out}/fresh.sock"
''')
os.chmod(config,0o600)
template=config.read_text()
results=[]
for attempt in range(2):
    socket_path=out/f'fresh-{attempt}.sock'
    assert not socket_path.exists(), 'fresh gate socket must be new'
    config.write_text(template.replace(str(out/'fresh.sock'),str(socket_path)))
    with (out / f'fresh-{attempt}.log').open('w') as log:
        proc=subprocess.Popen([str(root/'build/target/release/mush-server'),'--config',str(config)],cwd=root/'build',stdout=log,stderr=log)
        try:
            end=time.monotonic()+90
            while time.monotonic()<end:
                assert proc.poll() is None, f'fresh server exited {proc.returncode}; inspect fresh-{attempt}.log'
                raw=redis('JSON.GET','mod:propagation:relay_transport')
                if raw and socket_path.exists():
                    state=json.loads(raw)
                    assert state['service']['config']['version']==1
                    assert not state['service']['receipts'] and not state['service']['grants'] and not state['service']['restrictions']
                    registry=json.loads(redis('JSON.GET','mod:sim:relay:registry'))
                    assert registry['structural_version']==1 and not registry['losses']
                    for node in registry['nodes'].values():
                        assert redis('JSON.GET','mod:tactics:unit:'+str(node['object']))
                    market=state['service'].get('commerce')
                    if not market or market['capital_transaction'] is None:
                        time.sleep(.2);continue
                    account=json.loads(redis('JSON.GET','mod:economy:account:'+str(market['payee'])))
                    assert account['posted_balance']==100000 and account['held_amount']==0
                    assert market['next']==1 and not market['purchases']
                    billing=state['service'].get('billing')
                    if not billing: time.sleep(.2);continue
                    assert billing['tariff']['version']==1 and billing['tariff']['period_secs']==86400
                    assert not billing['subscriptions'] and not billing['treaties']
                    results.append({'billing':billing,'pid':proc.pid,'database':8,'policy':state['service']['config'],'market':market,'operatingBalance':account['posted_balance'],'nodes':len(registry['nodes']),'pairs':len(registry['pairs'])})
                    break
                time.sleep(.2)
            else:
                raise AssertionError('fresh boot timeout')
        finally:
            proc.terminate()
            try: proc.wait(timeout=20)
            except subprocess.TimeoutExpired: proc.kill();proc.wait()
assert results[0]['policy']==results[1]['policy']
assert results[0]['market']==results[1]['market']
assert results[0]['billing']==results[1]['billing']
(out/'fresh.json').write_text(json.dumps({'passed':True,'boots':results},indent=2)+'\n')
print('R8b_FRESH_REBOOT_PASSED')
