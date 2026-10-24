#!/usr/bin/env python3
"""Real SSH IC channel surface check on the explicitly disposable dev1 instance.

Run on the VPS after relay-r9a-live.mjs. Credentials stay in root-only files.
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
evidence_dir = root / 'r9a-evidence'
assert root.resolve() == root and evidence_dir.resolve().parent == root
credentials = json.loads((root / 'r6a-evidence' / 'actors.json').read_text())['bob']
live = json.loads((evidence_dir / 'live.json').read_text())
assert live['passed']
actors = json.loads((root / 'r6a-evidence' / 'preupgrade.json').read_text())['actors']
bob = actors['bob']['id']
channel = None


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
    reply = read_until('__R9a_SSH_DONE__')
    replies.append({'command': line, 'reply': reply})
    return reply


try:
    read_until('connect')
    os.write(master, ('connect ' + credentials['account'] + ' ' + credentials['password'] + '\r').encode())
    read_until('Connected as ' + credentials['char_name'])
    os.write(master, b'OUTPUTSUFFIX __R9a_SSH_DONE__\r')
    read_until('\n__R9a_SSH_DONE__\r\n')
    board = ' '.join(command('+relay').split())
    assert 'Standard declared wait 60' in board
    assert 'free Deferred' in board
    assert 'Retail tariff v1' in board
    assert 'Subscriptions v1 per 1 IC day(s)' in board
    assert 'Lunar relay foundry' in command('+relaypair')
    assert 'tacsim.relay.manufacture/private_pair/1' in command('+relaypair')
    assert 'No such quote' in command('+relay/buy 99999999')
    assert 'actual queued' not in board
    assert 'luna_ceres' in command('+relay/status luna_ceres')
    assert 'Permission denied' in command('+relay/policy luna_ceres/#1=suspended,operator.bad,Unauthorized')
    assert 'Permission denied' in command('+relay/load luna_ceres')
    assert 'Permission denied' in command('+relay/discover luna_ceres')
    assert 'R9a SSH OOC immediate' in command('pub R9a SSH OOC immediate')
    result = {'passed': True, 'transport': 'SSH :6789, play front door, in-band login',
              'channel': channel, 'checks': replies}
    (evidence_dir / 'ssh.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'ssh': True, 'checks': len(replies)}))
finally:
    process.terminate()
    os.close(master)
