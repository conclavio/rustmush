#!/usr/bin/env python3
"""Real dev1 SSH echo, scene-history and TUI transition acceptance."""
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import struct
import subprocess
import termios
import time
import ssl
import urllib.request
import urllib.error

ROOT = Path('/opt/rustmush/dev1')
OUT = ROOT / 'client-debt-evidence'
CREDS = json.loads((ROOT / 'r1-evidence/credentials.json').read_text())
RESULT = {'passed': False, 'checks': [], 'transcripts': []}


def clean(raw):
    return re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]', '', raw.decode(errors='replace')).replace(CREDS['password'], '[redacted]')


class Terminal:
    def __init__(self, echo):
        self.master, slave = pty.openpty()
        settings = termios.tcgetattr(slave)
        if echo:
            settings[3] |= termios.ECHO
        else:
            settings[3] &= ~termios.ECHO
        termios.tcsetattr(slave, termios.TCSANOW, settings)
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 34, 120, 0, 0))
        def tty():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)
        self.proc = subprocess.Popen([
            'ssh', '-tt', '-o', 'StrictHostKeyChecking=accept-new',
            '-o', 'UserKnownHostsFile=' + str(OUT / 'game-known-hosts'),
            '-o', 'PubkeyAuthentication=no', '-p', '6789', 'play@conclav.io',
        ], stdin=slave, stdout=slave, stderr=slave, preexec_fn=tty)
        os.close(slave)
        self.read(lambda raw, text: 'connect' in text)
        self.line('connect ' + CREDS['account'] + ' ' + CREDS['password'])
        self.read(lambda raw, text: 'Connected as ' in text)

    def line(self, text):
        os.write(self.master, (text + '\r').encode())

    def read(self, predicate, timeout=35):
        raw = b''
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if select.select([self.master], [], [], .1)[0]:
                try:
                    chunk = os.read(self.master, 65536)
                except OSError as error:
                    raise AssertionError('SSH closed; tail: ' + clean(raw)[-800:]) from error
                if not chunk:
                    raise AssertionError('SSH closed; tail: ' + clean(raw)[-800:])
                raw += chunk
                assert len(raw) <= 2 * 1024 * 1024, 'bounded SSH read exceeded'
                text = clean(raw)
                if predicate(raw, text):
                    return raw, text
        raise AssertionError('SSH response timeout; tail: ' + clean(raw)[-800:])

    def close(self):
        self.proc.terminate()
        try:
            self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait(timeout=5)
        os.close(self.master)


def wait_ready():
    context = ssl.create_default_context(cafile=str(ROOT / 'tls/cert.pem'))
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen('https://conclav.io:6790/healthz', context=context, timeout=3) as response:
                if json.load(response)['ok']:
                    return
        except urllib.error.URLError:
            pass
        time.sleep(.25)
    raise AssertionError('dev1 did not become healthy')


term = None
try:
    wait_ready()
    for echo in (False, True):
        term = Terminal(echo)
        suffix = '__CLIENT_DONE__'
        term.line('OUTPUTSUFFIX ' + suffix)
        term.read(lambda raw, text: '\n' + suffix + '\r\n' in text)
        token = 'client-debt-' + str(time.time_ns())
        command = 'say ' + token
        term.line(command)
        _, reply = term.read(lambda raw, text: '\n' + suffix + '\r\n' in text)
        assert token in reply
        assert (command in reply) == echo, (echo, reply)
        RESULT['checks'].append('PTY ECHO=' + str(int(echo)) + ' honored')
        RESULT['transcripts'].append({'echo': echo, 'reply': reply})
        if not echo:
            term.line('@history 1')
            _, reply = term.read(lambda raw, text: '--- @history more for older ---' in text or '--- that is the oldest history the server holds ---' in text)
            assert token in reply and reply.count(token) == 1
            RESULT['checks'].append('line history recalls one gateway-stamped narrative once')
            RESULT['transcripts'].append({'history': reply})
            term.line('@tui')
            term.read(lambda raw, text: b'\x1b[?1049h' in raw)
            term.line('')  # Enter switches TUI navigation to command input.
            term.line('@notui')
            term.read(lambda raw, text: b'\x1b[?1049l' in raw)
            term.line('@history 1')
            _, reply = term.read(lambda raw, text: '--- @history more for older ---' in text or '--- that is the oldest history the server holds ---' in text)
            assert token in reply and reply.count(token) == 1
            RESULT['checks'].append('TUI entry/exit retains line-history request ownership')
            RESULT['transcripts'].append({'after_mode_flip': reply})
        term.line('QUIT')
        term.read(lambda raw, text: 'Goodbye.' in text)
        term.proc.wait(timeout=15)
        assert term.proc.returncode == 0
        RESULT['transcripts'].append({'quit_exit_status': term.proc.returncode})
        RESULT['checks'].append('stamped SSH output drains and closes on QUIT')
        term.close()
        term = None
    RESULT['passed'] = True
finally:
    if term is not None:
        term.close()
    (OUT / 'ssh-live.json').write_text(json.dumps(RESULT, indent=2) + '\n')
print('CLIENT_SSH_LIVE_PASSED')
