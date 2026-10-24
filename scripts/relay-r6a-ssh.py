#!/usr/bin/env python3
"""Real SSH mail surface check on the explicitly disposable dev1 instance.

Run on the VPS after relay-r6a-live.mjs. Credentials stay in root-only files.
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
evidence_dir = root / 'r6a-evidence'
assert root.resolve() == root and evidence_dir.resolve().parent == root
credentials = json.loads((evidence_dir / 'actors.json').read_text())['bob']
live = json.loads((evidence_dir / 'live.json').read_text())
assert live['passed']
moving = next(m for m in live['milestones'] if m['test'] == 'moving-hull')
actors = json.loads((evidence_dir / 'preupgrade.json').read_text())['actors']
bob = actors['bob']['id']


def redis(key):
    return json.loads(subprocess.check_output(
        ['redis-cli', '-p', '6788', '--raw', 'JSON.GET', key], text=True) or 'null')


def ordinal(folder, message):
    keys = subprocess.check_output(
        ['redis-cli', '-p', '6788', '--scan', '--pattern', f'mod:mail:box:{bob}:*'],
        text=True).splitlines()
    assert len(keys) <= 300
    rows = sorted((redis(k) for k in keys), key=lambda row: row['msg'])
    ids = [r['msg'] for r in rows if r['folder'] == folder]
    return ids.index(message) + 1


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
    reply = read_until('__R6A_SSH_DONE__')
    replies.append({'command': line, 'reply': reply})
    return reply


try:
    read_until('connect')
    os.write(master, ('connect ' + credentials['account'] + ' ' + credentials['password'] + '\r').encode())
    read_until('Connected as ' + credentials['char_name'])
    os.write(master, b'OUTPUTSUFFIX __R6A_SSH_DONE__\r')
    read_until('\n__R6A_SSH_DONE__\r\n')
    command('@mail/folder inbox')
    received = command('@mail ' + str(ordinal('inbox', moving['outbound'])))
    assert 'Reply from an actual underway Dray.' in received
    queued = command(f'@mail/quick #{bob}=R6a SSH self/Accepted through the real SSH front door.')
    assert 'queued for IC transit' in queued
    message = int(re.search(r'Message (\d+) queued', queued)[1])
    # The normal self route still has a transport receipt and mail settlement.
    end = time.monotonic() + 45
    while time.monotonic() < end:
        row = redis(f'mod:mail:box:{bob}:{message}')
        if row and row['folder'] == 'inbox':
            break
        time.sleep(.2)
    else:
        raise AssertionError('Self mail never arrived')
    delivered = command('@mail ' + str(ordinal('inbox', message)))
    assert 'Accepted through the real SSH front door.' in delivered
    result = {'passed': True, 'transport': 'SSH :6789, play front door, in-band login',
              'read': moving['outbound'], 'self': message, 'checks': replies}
    (evidence_dir / 'ssh.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'ssh': True, 'checks': len(replies)}))
finally:
    process.terminate()
    os.close(master)
