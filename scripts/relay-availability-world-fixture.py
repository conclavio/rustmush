#!/usr/bin/env python3
"""Dev1-only reset using a genuine R9b RDB and verified pre-gate backup.
A private, portless Redis reader loads the RDB. Only dev1 database 0 is replaced
through Redis' normal RESTORE API, so the live AOF remains consistent. Gateway
must be stopped. Snapshots/config/reader socket remain root-only on the VPS.
"""
import hashlib,json,os,shutil,socket,subprocess,sys,time
from pathlib import Path
root=Path('/opt/rustmush/dev1');out=root/'availability-evidence'
assert os.geteuid()==0 and root.resolve()==root and out.resolve().parent==root
assert subprocess.run(['systemctl','is-active','--quiet','dev1mush-gateway']).returncode!=0
for procdir in Path('/proc').iterdir():
 if procdir.name.isdigit():
  try:exe=os.readlink(procdir/'exe')
  except OSError:continue
  assert not exe.startswith('/opt/rustmush/dev1/build/target/release/mush-server'),'dev1 child must be stopped'
out.mkdir(mode=0o700,exist_ok=True)
def run(*args):return subprocess.check_output(args,text=True).strip()
def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()
class Redis:
 def __init__(self,path=None):
  self.sock=socket.socket(socket.AF_UNIX if path else socket.AF_INET,socket.SOCK_STREAM);self.sock.settimeout(30);self.sock.connect(str(path) if path else ('127.0.0.1',6788));self.wire=self.sock.makefile('rb')
 def read(self):
  line=self.wire.readline();assert line.endswith(b'\r\n');kind,data=line[:1],line[1:-2]
  if kind==b'+':return data
  if kind==b'-':raise RuntimeError(data.decode())
  if kind==b':':return int(data)
  if kind==b'$':
   n=int(data)
   if n<0:return None
   value=self.wire.read(n);assert self.wire.read(2)==b'\r\n';return value
  if kind==b'*':return [self.read() for _ in range(int(data))]
  raise AssertionError('RESP type')
 def cmd(self,*args):
  args=[a if isinstance(a,bytes) else str(a).encode() for a in args];self.sock.sendall(b'*'+str(len(args)).encode()+b'\r\n'+b''.join(b'$'+str(len(a)).encode()+b'\r\n'+a+b'\r\n' for a in args));return self.read()
 def keys(self):
  cursor=0;keys=set()
  while True:
   cursor,chunk=self.cmd('SCAN',cursor,'COUNT',1000);keys.update(chunk);assert len(keys)<100000
   if cursor==b'0':return sorted(keys)
live=Redis();assert live.cmd('CONFIG','GET','dir')==[b'dir',b'/var/lib/redis-dev1mush'];assert live.cmd('CONFIG','GET','dbfilename')==[b'dbfilename',b'dump.rdb']
assert live.cmd('SELECT',0)==b'OK'
def save_live():
 deadline=time.monotonic()+60
 while True:
  try:
   assert live.cmd('SAVE')==b'OK';return
  except RuntimeError as error:
   if 'Background save already in progress' not in str(error) or time.monotonic()>=deadline:raise
   time.sleep(.2)

baseline=out/'pre-gate.rdb';config=out/'pre-gate-mushd.toml';checkpoint=root/'r9b-evidence/acceptance-repair/funded-checkpoint.rdb'
mode=sys.argv[1];assert mode in ('prepare','restore','reset-failed','retest')
if mode=='prepare':
 assert not baseline.exists() and not config.exists(),'never overwrite baseline'
 save_live();shutil.copyfile('/var/lib/redis-dev1mush/dump.rdb',baseline);os.chmod(baseline,0o600)
 shutil.copyfile(root/'mushd.toml',config);os.chmod(config,0o600)
 meta={'baselineSha256':digest(baseline),'fundedCheckpointSha256':'7cecd71fe01a116829d3342fa7e6658da763f6c95528ecb74dcdd6d91a9a9c16','dev2Pid':int(run('systemctl','show','dev2mush-gateway','-p','MainPID','--value'))}
 (out/'fixture.json').write_text(json.dumps(meta,indent=2)+'\n')
else:
 meta=json.loads((out/'fixture.json').read_text());assert digest(baseline)==meta['baselineSha256'];assert (out/'live.json').exists(),'save the test outcome first'
 if mode=='retest':
  result=json.loads((out/'live.json').read_text());assert result.get('passed') and not result.get('failure')
  assert json.loads((out/'restore.json').read_text())['sourceSha256']==meta['baselineSha256']
 if mode=='reset-failed':
  result=json.loads((out/'live.json').read_text());assert result.get('failure') and not result.get('passed')
  failed=out/('live-failed-'+str(time.time_ns())+'.json');shutil.copyfile(out/'live.json',failed)
source=baseline if mode=='restore' else checkpoint
assert digest(source)==meta['baselineSha256' if mode=='restore' else 'fundedCheckpointSha256']
reader_dir=out/'checkpoint-reader';reader_dir.mkdir(mode=0o700,exist_ok=True);assert reader_dir.resolve().parent==out
shutil.copyfile(source,reader_dir/'source.rdb');os.chmod(reader_dir/'source.rdb',0o600)
path=reader_dir/'redis.sock';assert not path.exists(),'reader already running'
with (reader_dir/'reader.log').open('ab') as log:
 proc=subprocess.Popen(['/usr/bin/redis-server','/etc/redis/redis-dev1mush.conf','--port','0','--unixsocket',str(path),'--unixsocketperm','700','--dir',str(reader_dir),'--dbfilename','source.rdb','--appendonly','no','--daemonize','no','--supervised','no','--pidfile',str(reader_dir/'redis.pid'),'--logfile','', '--save',''],stdout=log,stderr=log)
 try:
  deadline=time.monotonic()+40
  while True:
   assert proc.poll() is None,'private RDB reader failed; inspect root-only reader.log'
   try:
    old=Redis(path)
    if old.cmd('PING')==b'PONG':break
   except (OSError,RuntimeError):pass
   assert time.monotonic()<deadline;time.sleep(.1)
  old.cmd('SELECT',0);keys=old.keys();assert b'mod:sim:relay:registry' in keys
  expected=[]
  assert live.cmd('FLUSHDB')==b'OK'
  for key in keys:
   payload=old.cmd('DUMP',key);ttl=old.cmd('PTTL',key)
   if payload is None or ttl==-2:continue
   assert live.cmd('RESTORE',key,max(0,ttl),payload)==b'OK'
   if ttl==-1:assert live.cmd('DUMP',key)==payload
   expected.append(key)
  assert live.cmd('JSON.GET','mod:sim:relay:registry')==old.cmd('JSON.GET','mod:sim:relay:registry')
  save_live()
  if mode=='restore':shutil.copyfile(config,root/'mushd.toml')
 finally:
  proc.terminate()
  try:proc.wait(timeout=10)
  except subprocess.TimeoutExpired:proc.kill();proc.wait()
assert int(run('systemctl','show','dev2mush-gateway','-p','MainPID','--value'))==meta['dev2Pid']
result={'action':mode,'sourceSha256':digest(source),'dev2Pid':meta['dev2Pid'],'keys':len(expected),'method':'portless RDB reader; dev1 DB0 RESTORE with AOF retained'}
(out/(mode+'.json')).write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
