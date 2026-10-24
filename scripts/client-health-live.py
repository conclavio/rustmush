#!/usr/bin/env python3
"""Exercise real dev1 world health without changing its database or configuration."""
import json, os, signal, ssl, subprocess, time, urllib.request, urllib.error
from pathlib import Path
ROOT = Path('/opt/rustmush/dev1')
OUT = ROOT / 'client-debt-evidence'
CTX = ssl.create_default_context(cafile=str(ROOT / 'tls/cert.pem'))
URL = 'https://conclav.io:6790/healthz'

def pid(service):
    return int(subprocess.check_output(['systemctl', 'show', service, '-p', 'MainPID', '--value'], text=True))

def server(gateway):
    children = subprocess.check_output(['ps', '--ppid', str(gateway), '-o', 'pid='], text=True).split()
    found = [int(p) for p in children if b'/opt/rustmush/dev1/' in Path('/proc', p, 'cmdline').read_bytes() and b'mush-server' in Path('/proc', p, 'cmdline').read_bytes()]
    assert len(found) == 1, found
    return found[0]

def read():
    try:
        response = urllib.request.urlopen(URL, context=CTX, timeout=3)
    except urllib.error.HTTPError as error:
        response = error
    except urllib.error.URLError as error:
        return {'at': time.time(), 'status': 0, 'body': {}, 'error': str(error.reason)}
    with response:
        return {'at': time.time(), 'status': response.code, 'body': json.load(response)}

def until(predicate, seconds=40):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        sample = read()
        samples.append(sample)
        if predicate(sample): return sample
        time.sleep(.25)
    raise AssertionError(samples[-5:])

OUT.mkdir(exist_ok=True)
samples = []
result = {'passed': False, 'samples': samples}
stopped = None
try:
    before_dev2 = pid('dev2mush-gateway')
    gateway = pid('dev1mush-gateway')
    first = until(lambda h: h['status'] == 200 and h['body']['ok'])
    assert first['body']['world_loop_unixtime'] > 0
    child = server(gateway)
    result.update(gateway=gateway, originalServer=child, dev2=before_dev2)
    os.kill(child, signal.SIGSTOP)
    stopped = child
    stale = until(lambda h: h['status'] == 503 and not h['body']['world_ok'])
    assert stale['body']['supervision_ok'] and stale['body']['store_ok']
    assert pid('dev1mush-gateway') == gateway
    os.kill(child, signal.SIGCONT)
    stopped = None
    resumed = until(lambda h: h['status'] == 200 and h['body']['world_loop_unixtime'] > stale['body']['world_loop_unixtime'])
    os.kill(child, signal.SIGKILL)
    until(lambda h: h['status'] == 503 and not h['body']['world_ok'])
    restarted = until(lambda h: h['status'] == 200 and h['body']['ok'], 90)
    replacement = server(gateway)
    assert replacement != child and pid('dev1mush-gateway') == gateway
    assert pid('dev2mush-gateway') == before_dev2
    result.update(passed=True, replacementServer=replacement, checks=['authoritative stamp', 'live gateway over stopped world returns 503', 'continued world recovers', 'child crash invalidates health', 'replacement recovers', 'gateway survives', 'dev2 untouched'])
finally:
    if stopped is not None: os.kill(stopped, signal.SIGCONT)
    (OUT / 'health-live.json').write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
print('CLIENT_HEALTH_LIVE_PASSED')
