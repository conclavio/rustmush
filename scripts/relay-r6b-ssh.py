#!/usr/bin/env python3
"""Real SSH IC channel surface check on the explicitly disposable dev1 instance.

Run on the VPS after relay-r6b-live.mjs. Credentials stay in root-only files.
"""
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import subprocess
import termios
import time

root = Path('/opt/rustmush/dev1')
evidence_dir = root / 'r6b-evidence'
assert root.resolve() == root and evidence_dir.resolve().parent == root
credentials = json.loads((root / 'r6a-evidence' / 'actors.json').read_text())['bob']
live = json.loads((evidence_dir / 'live.json').read_text())
assert live['passed']
actors = json.loads((root / 'r6a-evidence' / 'preupgrade.json').read_text())['actors']
bob = actors['bob']['id']
channel = live['channel']['id']


def redis(key):
    return json.loads(subprocess.check_output(
        ['redis-cli', '-p', '6788', '--raw', 'JSON.GET', key], text=True) or 'null')


master, slave = pty.openpty()


def child_tty():
    os.setsid()
    fcntl.ioctl(0, termios.TIOCSCTTY, 0)


process = subprocess.Popen([
    'ssh', '-tt', '-o', 'StrictHostKeyChecking=accept-new',
    '-o', 'UserKnownHostsFile=' + str(evidence_dir / 'game-known-hosts'),
    '-o', 'PubkeyAuthentication=no', '-p', '6789', 'play@conclav.io',
], stdin=slave, stdout=slave, stderr=slave, preexec_fn=child_tty)
os.close(slave)
replies = []


def read_until(needle, timeout=30):
    raw = b''
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if select.select([master], [], [], .1)[0]:
            raw += os.read(master, 65536)
            text = re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]', '', raw.decode(errors='replace'))
            text = text.replace(credentials['password'], '[redacted]')
            if needle in text:
                return text
    raise AssertionError('SSH response timeout: ' + needle)


def command(line):
    os.write(master, (line + '\r').encode())
    reply = read_until('__R6B_SSH_DONE__')
    replies.append({'command': line, 'reply': reply})
    return reply


try:
    read_until('connect')
    os.write(master, ('connect ' + credentials['account'] + ' ' + credentials['password'] + '\r').encode())
    read_until('Connected as ' + credentials['char_name'])
    os.write(master, b'OUTPUTSUFFIX __R6B_SSH_DONE__\r')
    read_until('\n__R6B_SSH_DONE__\r\n')
    received = command('@channel/recall r6b')
    assert 'from-underway-hull' in received
    assert 'source-survives-object-loss' in received
    before = redis(f'mod:comsys:member:{bob}:{channel}')['ic']['sequence']
    queued = command('r6b R6b SSH arrived channel line')
    assert 'queued for IC transit' in queued
    end = time.monotonic() + 45
    while time.monotonic() < end:
        row = redis(f'mod:comsys:member:{bob}:{channel}')
        if row['ic']['sequence'] > before:
            break
        time.sleep(.2)
    else:
        raise AssertionError('Self channel receipt never arrived')
    delivered = command('@channel/recall r6b')
    assert 'R6b SSH arrived channel line' in delivered
    assert 'R6b SSH OOC immediate' in command('pub R6b SSH OOC immediate')
    assert 'Permission denied' in command('@channel/transit')
    result = {'passed': True, 'transport': 'SSH :6789, play front door, in-band login',
              'channel': channel, 'checks': replies}
    (evidence_dir / 'ssh.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'ssh': True, 'checks': len(replies)}))
finally:
    process.terminate()
    os.close(master)
