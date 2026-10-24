#!/usr/bin/env python3
"""R10 real process-fault contracts on a new, loopback-only dev1 Redis instance."""
import json,os,socket,subprocess,time
from pathlib import Path
root=Path('/opt/rustmush/dev1');out=root/'r10-evidence';assert root.resolve()==root
folder=out/'process-redis';folder.mkdir(exist_ok=False)
with socket.socket() as reserve:
    reserve.bind(('127.0.0.1',0));port=reserve.getsockname()[1]
with (out/'process-redis.log').open('w') as log:
    proc=subprocess.Popen(['/usr/bin/redis-server','--bind','127.0.0.1','--port',str(port),'--protected-mode','yes','--save','','--appendonly','no','--dir',str(folder),'--loadmodule','/usr/lib/redis/modules/rejson.so'],stdout=log,stderr=log)
    try:
        for _ in range(100):
            assert proc.poll() is None,'isolated Redis exited'
            ping=subprocess.run(['redis-cli','-p',str(port),'PING'],capture_output=True,text=True)
            if ping.stdout.strip()=='PONG':break
            time.sleep(.1)
        else:raise AssertionError('isolated Redis did not start')
        env=os.environ.copy();env['REDIS_URL']=f'redis://127.0.0.1:{port}/0';env['CARGO_PROFILE_TEST_DEBUG']='0';env['CARGO_PROFILE_DEV_DEBUG']='0'
        with (out/'process-tests.log').open('w') as tests:
            result=subprocess.run(['/root/.cargo/bin/cargo','test','--locked','-p','mushd','--test','durability_e2e','-j4','--','--test-threads=1'],cwd=root/'build',env=env,stdout=tests,stderr=tests)
        assert result.returncode==0,'real process contracts failed'
        (out/'process.json').write_text(json.dumps({'passed':True,'redisPort':port,'redisPid':proc.pid,'target':'mushd durability_e2e','REDIS_URL':'isolated dev1 process','skips':0})+'\n')
    finally:
        proc.terminate()
        try:proc.wait(timeout=15)
        except subprocess.TimeoutExpired:proc.kill();proc.wait()
print('R10_REAL_PROCESS_TESTS_PASSED')
