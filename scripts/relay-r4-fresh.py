#!/usr/bin/env python3
"""R4 real fresh-store boot/reboot on unused dev1 Redis database 14.

Never clears a database. Refuses nonempty input; retains the test database and
root-only server logs under dev1/r4-evidence for inspection.
"""
import json
import os
from pathlib import Path
import subprocess
import time

root = Path('/opt/rustmush/dev1')
assert root.resolve() == root
out = root / 'r4-evidence'
assert out.resolve().parent == root

def redis(*args):
    return subprocess.check_output(['redis-cli','-p','6788','-n','14','--raw',*args],text=True).strip()

assert redis('DBSIZE') == '0', 'fresh gate requires empty dev1 database 14'
config = out / 'fresh.toml'
config.write_text(f'''[server]
data_dir = "{out}/fresh-data"
help_dir = "{root}/build/docs/help"
[store]
store = "redis"
redis_url = "redis://127.0.0.1:6788/14"
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
                    results.append({'pid':proc.pid,'database':14,'policy':state['service']['config'],'nodes':len(registry['nodes']),'pairs':len(registry['pairs'])})
                    break
                time.sleep(.2)
            else:
                raise AssertionError('fresh boot timeout')
        finally:
            proc.terminate()
            try: proc.wait(timeout=20)
            except subprocess.TimeoutExpired: proc.kill();proc.wait()
assert results[0]['policy']==results[1]['policy']
(out/'fresh.json').write_text(json.dumps({'passed':True,'boots':results},indent=2)+'\n')
print('R4_FRESH_REBOOT_PASSED')
