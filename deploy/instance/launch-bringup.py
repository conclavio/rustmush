#!/usr/bin/env python3
"""launch-bringup.py: take one instance's fresh store to a live mind market.

Doc 44 cut MW.1. `[tacsim.minds].launch_path` names a document full of ids
(organisations, accounts, rooms, characters) that only a booted world can make,
and a named identity that does not exist refuses the boot. So the named launch
is brought up in three boots, and this script walks all of them (the runbook is
deploy/README.md, "Named launch and the player market"):

  1. PLAIN. No launch. As #1: the Republic, the system accounts, the three
     Houses, the Basin importer and its fitting shop, the Tether back room (the
     ungoverning shop), the Registry, the staff login that holds the stock
     agent, the clerk and the Registry operator, and each House's procurement
     agent on a login of its own. Then <DATA_DIR>/launch.json is written in
     mode `initialize` from the ids the world printed, and the config is
     rendered again, which now makes `launch_path` live (render.sh renders it
     only beside its document).
  2. INITIALIZE. The seven Registry grants to every operator, then the four
     initialize actions. The document's mode is set to `active`.
  3. ACTIVE. `@mind/launch` must name every operator `ready`. The counter is
     given its door from the Registry's desk at Ceres Freeport (a dug room has
     no exit, and the market's verbs are refused anywhere else). Then the major
     NPCs are enabled (Q-44-47).

Then the instance's own unit is started, and on it: `@mind/launch` again, the
LLM routes' check, and last the dev launch's seed: one line rated by the
Registry, filed by #1 and listed on the player market, and one House's
purchase of it by its own agent. The seed comes after players are let in
because a rating is long at any pulse (about half an hour for a copy of
Standard Doctrine, longer for a House source) and nobody is kept off for it;
`--seed-only` takes it up again after a stop, and `--no-seed` leaves it out.

HOW THE IN-GAME STEPS ARE TYPED AS #1. The script runs the world itself for the
three boots: `mush-server` on a private IPC socket, and a private
`mush-gateway` beside it whose SSH listener is 127.0.0.1:<private port> (or one
`mushd` bound the same way where the split is not built). It opens SSH sessions
to that port and types `connect <staff account> <password>` at the welcome
screen; the staff account is the one `--bootstrap-staff` linked to God #1 (the
script bootstraps it on a store with no account). Every line it types and every
reply is kept in <DATA_DIR>/bringup/transcript.log; a line that carries a
password is recorded without it (the front door does not echo).

HOW PLAYERS ARE KEPT OFF UNTIL THE ACTIVE BOOT. The instance's unit (the world)
is stopped before the first boot and started only after the Active boot has
passed its checks. While it is stopped nothing answers on the instance's own
IPC socket, so the public gateway generations, which stay up, have no world to
log anybody in to or register anybody on; the private gateway listens on
loopback only. A private `--ipc` socket alone would lock the operator out as
well, which is why the script brings its own gateway. Start from a stopped
unit (`scripts/reset-instance.sh ... --no-start`, then the deploy with
`SERVER_HOLD=1`) and no player ever sees the world without its market.

SAFE TO INTERRUPT AND RUN AGAIN. Where the bring-up stands is read from the
world and from launch.json, never assumed: every identity is looked up by name
before it is made, a grant is made only when `@org/show` does not list it, an
account is funded only up to its figure, and the phase is the document's own
(absent, `initialize`, `active`). The only thing the script keeps for itself is
<DATA_DIR>/bringup/state.json: the hulls it has lent a House, so that a run
which stops in between hands back what it took. It lives in the data
directory, so a reset removes it with the world it belonged to. No password is
kept: each service login (the stock agent, the clerk, the Registry operator,
the three House agents) gets a random one that is thrown away, since nobody
plays them, and #1 sets one with `@newpassword` on the day somebody must.

Usage:
  sudo python3 deploy/instance/launch-bringup.py <instance.env> [options]

<instance.env> is the installed file the instance's mushd.toml was rendered
from, with mushd.toml beside it (deploy/README.md). It is read, never sourced.
The staff password is one line on stdin, or `--password-file`, or a prompt.

Linux, Python 3.8+, the standard library only; `ssh` and (for the purchase
check) `redis-cli` on PATH.
"""

import argparse
import fcntl
import getpass
import json
import os
import pty
import re
import secrets
import select
import shutil
import signal
import socket
import struct
import subprocess
import sys
import termios
import time

HOUSES = [
    # launch.json key, organisation, configured source name, @mind/launch selector
    ("meridiani", "Meridiani", "Meridiani Field Series", "meridiani"),
    ("coprates", "Coprates", "Coprates Drill", "coprates"),
    ("tycho", "Tycho", "Safe Conduct", "safe_conduct"),
]
REGISTRY_GRANTS = ["recall", "revoke", "inspect", "fine", "claims", "seize", "forfeit"]
# The Roll's action is typed LAST. Once it has landed the world boots only
# Active (the Initialize boot's preflight wants untouched commerce records), so
# a run that stopped after it could not come back for the other three.
INITIALIZE_ACTIONS = [
    "initialize-core-die-stock",
    "initialize-core-factory",
    "initialize-core-refit",
    "roll-initialize-commerce",
]
CREDIT = "tacsim.credit"
FILING_FEE_MINOR = 10_000  # doc 17: 100cr, the launch's content constant
# deploy/instance/launch.json.example's terms: 17g section 5.3's written numbers
# (provisional, Q-44-5).
MARKET_TERMS = {
    "price_minor": 40000,
    "renewal_price_minor": 40000,
    "quote_seconds": 300,
    "renewal_offer_ic_seconds": 2419200,
    "resale_offer_seconds": 86400,
    "charter_settle_ic_seconds": 259200,
}
TRUST = "Co-op First Captain Trust"
# The counter's door (doc 44 unit MW.19). The counter is a room the bring-up
# digs, and a dug room has no exit: until this, no player could walk to it, so
# nobody but staff could list a mind or buy one. The door is hung off the
# Registry's own desk at Ceres Freeport, where a new captain already stands to
# enrol. Provisional: where the counter belongs in the port is the owner's call.
COUNTER_DESK = "Registry & Co-op Desk"
COUNTER_DOOR = "Mind Counter <MC>;mc;minds;mind counter;counter"
COUNTER_BACK = "Registry <R>;r;registry;out;o"
# Replies that mean "not now", to a line that was not run.
TRANSIENT = (r"You're doing that too fast\.|post-outage recovery countdown"
             r"|still starting; please try again|please retry in a moment"
             r"|server is busy; please retry that input")
ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b[()][A-Z0-9]|\x1b[=>]")


class Stop(Exception):
    """A refusal the operator has to read. Nothing after it is attempted."""


class Interrupted(BaseException):
    """SIGINT, SIGTERM or SIGHUP. Raised from the handler so that the private
    boots are stopped on the way out, whatever ended the run."""


def interrupted(number, _frame):
    raise Interrupted(signal.Signals(number).name)


def say(text):
    print(f"launch-bringup: {text}", flush=True)


# --- instance.env and the rendered config ---------------------------------


def read_env(path):
    """KEY=value lines, read and never evaluated (deploy/instance/render.sh)."""
    env = {}
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            line = line.rstrip("\r\n")
            if not line or line.lstrip().startswith("#") or "=" not in line:
                continue
            key, value = line.split("=", 1)
            env.setdefault(key.strip(), value.strip())
    return env


def toml_sections(text):
    """(section, line) pairs of a rendered config, comments included."""
    section = ""
    for line in text.splitlines():
        header = re.match(r"^\[([^\[\]]+)\]", line)
        if header:
            section = header.group(1).strip()
        yield section, line


def config_value(text, section, key):
    for name, line in toml_sections(text):
        found = re.match(rf"^{re.escape(key)}\s*=\s*(.*?)\s*(?:#.*)?$", line)
        if name == section and found:
            return found.group(1).strip().strip('"')
    return None


def private_config(text, edits, drop=()):
    """The rendered config with `edits` ({(section, key): toml value}) applied
    and the `drop` keys removed: what the private gateway (or mushd) reads."""
    out, seen = [], set()
    for name, line in toml_sections(text):
        key = re.match(r"^([A-Za-z_]+)\s*=", line)
        if key and (name, key.group(1)) in drop:
            continue
        if key and (name, key.group(1)) in edits:
            seen.add((name, key.group(1)))
            line = f"{key.group(1)} = {edits[(name, key.group(1))]}"
        out.append(line)
    missing = set(edits) - seen
    if missing:
        raise Stop(f"the rendered config has no {sorted(missing)} to rewrite for the private boot")
    return "\n".join(out) + "\n"


# --- one SSH session at the front door --------------------------------------


class Session:
    """One line-mode SSH session through a PTY (the front door refuses a
    session without one). Replies are matched as text, ANSI removed."""

    def __init__(self, box, who):
        self.box, self.who = box, who
        self.raw = b""
        self.pos = 0
        self.typed_at = 0.0
        self.bracketed = 0
        self.closed = False
        self.suffix = None
        master, slave = pty.openpty()
        settings = termios.tcgetattr(slave)
        settings[3] &= ~termios.ECHO
        termios.tcsetattr(slave, termios.TCSANOW, settings)
        # Wide, so no reply is wrapped across the lines a pattern is read from.
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 500, 0, 0))

        def tty():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

        host, port = box.ssh_endpoint
        self.proc = subprocess.Popen(
            ["ssh", "-tt", "-p", str(port), "-o", "BatchMode=yes",
             "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null",
             "-o", "LogLevel=ERROR", f"play@{host}"],
            stdin=slave, stdout=slave, stderr=slave, preexec_fn=tty, close_fds=True)
        os.close(slave)
        self.master = master
        self.wait(r"(?i)connect <account>", 60, "the welcome screen")

    def text(self):
        clean = ANSI.sub("", self.raw[self.pos:].decode("utf-8", "replace"))
        return clean.replace("\r\n", "\n").replace("\r", "\n")

    def pump(self, timeout):
        if select.select([self.master], [], [], timeout)[0]:
            try:
                chunk = os.read(self.master, 65536)
            except OSError:
                chunk = b""
            if not chunk:
                raise Stop(f"the {self.who} session closed; its last lines:\n{self.text()[-600:]}")
            self.raw += chunk
            self.box.transcript(self.who, "<", chunk)

    def wait(self, pattern, timeout, what=None):
        """The first match of `pattern` in what arrived since the last line was
        typed, waiting up to `timeout` seconds."""
        regex = re.compile(pattern, re.M)
        end = time.monotonic() + timeout
        while True:
            found = regex.search(self.text())
            if found:
                return found
            if time.monotonic() >= end:
                raise Stop(f"{self.who}: no {what or pattern!r} within {timeout}s; "
                           f"the reply so far:\n{self.text()[-800:]}")
            self.pump(0.2)

    def type(self, line):
        """Type one line, and record it in the transcript."""
        self.ready()
        self.box.transcript(self.who, ">", line.encode())
        os.write(self.master, line.encode("utf-8") + b"\r")

    def type_unrecorded(self, line, shown):
        """Type a line that carries a password. The transcript gets `shown`,
        never the line: the line goes to the terminal and nowhere else."""
        self.ready()
        self.box.transcript(self.who, ">", shown.encode())
        os.write(self.master, line.encode("utf-8") + b"\r")

    def ready(self):
        """What is owed before any line is typed."""
        # The gateway's flood bucket: ten lines a second sustained, thirty in a
        # burst, for #1 as for anybody. Stay under it.
        pause = self.typed_at + 0.15 - time.monotonic()
        if pause > 0:
            time.sleep(pause)
        # Every line typed after the bracket is set is answered with it, even
        # one that prints nothing else. Read the brackets of the lines already
        # typed before typing the next, so that what is read as this line's
        # reply never starts with the end of the last one.
        if self.suffix:
            mark = self.suffix.encode()
            end = time.monotonic() + 20
            while self.raw.count(mark) < self.bracketed and time.monotonic() < end:
                self.pump(0.2)
            self.bracketed = self.raw.count(mark) + 1
        self.typed_at = time.monotonic()
        self.pos = len(self.raw)

    def connect(self, account, password, create=False):
        verb = "create" if create else "connect"
        self.type_unrecorded(f"{verb} {account} {password}", f"{verb} {account} [password]")
        found = self.wait(r"Connected as |already taken|does not exist, or the password is wrong"
                          r"|too weak|Characters on |has no characters yet|banned", 60)
        if "Characters on" in found.group(0):
            # More than one character on the login: the first is the login's own.
            self.type("1")
            found = self.wait(r"Connected as ", 60)
        if "Connected as" not in found.group(0):
            return found.group(0)
        self.suffix = "BRINGUP-" + secrets.token_hex(6)
        self.type(f"OUTPUTSUFFIX {self.suffix}")
        self.wait(rf"^{self.suffix}$", 30, "the reply bracket")
        return None

    def answered(self, line, pattern, timeout, what):
        """Type `line` and wait for `pattern`. A line the world turned away
        without running it is typed again: the gateway's flood bucket, a world
        still starting or committing, and the post-outage recovery countdown,
        which a boot more than `[game].outage_freeze_threshold_s` (30 s) after
        the store's last clock checkpoint runs for five minutes with every
        world action paused."""
        end = time.monotonic() + 420
        told = False
        while True:
            self.type(line)
            found = self.wait(rf"{pattern}|(?P<later>{TRANSIENT})", timeout, what)
            later = found.groupdict().get("later")
            if not later:
                return found
            if time.monotonic() >= end:
                raise Stop(f"{self.who}: {line!r} was turned away for seven minutes: {later}")
            if "countdown" in later and not told:
                told = True
                say("the world is in its post-outage recovery countdown (it booted more than 30 s "
                    "after the store's last clock checkpoint): waiting it out, five minutes at most")
            time.sleep(10 if "countdown" in later else 3)

    def ask(self, line, timeout=60):
        """A read: the whole reply, which the reply bracket ends."""
        self.answered(line, rf"^{self.suffix}$", timeout, f"the end of the reply to {line!r}")
        return self.text().split(self.suffix)[0]

    def do(self, line, expect, timeout=60):
        """A write: wait for `expect`, which names every reply that is an
        answer; anything else runs out the clock and is shown."""
        found = self.answered(line, expect, timeout, f"answer to {line!r}")
        # Keep in step with the bracket where the line has one (a reply held
        # for its durable write, and a gateway line, may come without it).
        end = time.monotonic() + 2
        while self.suffix not in self.text() and time.monotonic() < end:
            self.pump(0.1)
        return found

    def settle(self, quiet=1.0, limit=15):
        """Read until nothing has arrived for `quiet` seconds: the end of a
        reply that carries no bracket (a line the gateway answers itself)."""
        end = time.monotonic() + limit
        while time.monotonic() < end:
            before = len(self.raw)
            self.pump(quiet)
            if len(self.raw) == before:
                break
        return self.text()

    def close(self):
        if self.closed:
            return
        self.closed = True
        try:
            self.type("QUIT")
            self.pump(1)
            if "Really disconnect" in self.text():
                self.type("y")
            end = time.monotonic() + 5
            while self.proc.poll() is None and time.monotonic() < end:
                try:
                    self.pump(0.2)
                except Stop:
                    break
        except OSError:
            pass
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        os.close(self.master)


# --- the box: paths, the unit, the private boots -----------------------------


class Box:
    def __init__(self, args):
        self.args = args
        self.env_file = os.path.abspath(args.env)
        self.env = read_env(self.env_file)
        for key in ("INSTANCE", "DATA_DIR", "BUILD_DIR", "SSH_PORT", "REDIS_PORT"):
            if not self.env.get(key):
                raise Stop(f"{self.env_file} has no {key} line")
        self.instance = self.env["INSTANCE"]
        self.data = self.env["DATA_DIR"]
        self.config = os.path.join(os.path.dirname(self.env_file), "mushd.toml")
        self.here = os.path.dirname(os.path.abspath(__file__))
        self.repo = os.path.dirname(os.path.dirname(self.here))
        self.bin = args.bin_dir or os.path.join(self.env["BUILD_DIR"], "target", "release")
        self.work = os.path.join(self.data, "bringup")
        self.document = os.path.join(self.data, "launch.json")
        self.state_file = os.path.join(self.work, "state.json")
        self.pids_file = os.path.join(self.work, "private.pids")
        self.children = []
        self.ssh_endpoint = None
        self.unit = None
        self.owner = None
        self.boots = 0
        self.timings = {}
        self.split = all(os.access(os.path.join(self.bin, b), os.X_OK)
                         for b in ("mush-server", "mush-gateway"))
        if not self.split and not os.access(os.path.join(self.bin, "mushd"), os.X_OK):
            raise Stop(f"neither mush-server with mush-gateway nor mushd is built in {self.bin}")
        self.world_binary = os.path.join(self.bin, "mush-server" if self.split else "mushd")
        if not os.path.isfile(self.config):
            raise Stop(f"no rendered config at {self.config}: deploy the instance first")
        if self.env.get("MINDS_LAUNCH", "") != "on":
            raise Stop(f"MINDS_LAUNCH is not `on` in {self.env_file}: this instance does not "
                       "take the named launch, and render.sh would leave launch_path commented")
        self.find_unit()
        os.makedirs(self.work, mode=0o700, exist_ok=True)
        self.own(self.work)
        self.state = {}
        if os.path.isfile(self.state_file):
            with open(self.state_file, encoding="utf-8") as handle:
                self.state = json.load(handle)
        self.log = open(os.path.join(self.work, "transcript.log"), "ab")
        os.chmod(self.log.name, 0o600)
        self.reap_leftovers()

    # ---- the unit and the service user

    def find_unit(self):
        if self.args.no_systemd:
            return
        if os.geteuid() != 0:
            raise Stop("must run as root to stop and start the instance's unit "
                       "(a rig with no unit passes --no-systemd)")
        names = [self.args.unit] if self.args.unit else [
            f"{self.instance}mush-server.service", f"{self.instance}mush-gateway.service",
            f"{self.instance}mush.service"]
        for name in names:
            if subprocess.run(["systemctl", "cat", name], capture_output=True).returncode == 0:
                self.unit = name
                break
        if not self.unit:
            raise Stop(f"no unit found for instance '{self.instance}' (tried {', '.join(names)}); "
                       "pass --unit, or --no-systemd for a rig")
        user = subprocess.run(["systemctl", "show", "-p", "User", "--value", self.unit],
                              capture_output=True, text=True).stdout.strip()
        self.owner = user or "mush"

    def as_owner(self, command):
        if self.owner and os.geteuid() == 0:
            return ["runuser", "-u", self.owner, "--"] + command
        return command

    def own(self, path):
        if self.owner and os.geteuid() == 0:
            shutil.chown(path, self.owner, self.owner)

    def systemctl(self, verb):
        done = subprocess.run(["systemctl", verb, self.unit], capture_output=True, text=True)
        if done.returncode != 0:
            raise Stop(f"systemctl {verb} {self.unit} failed ({done.returncode}): "
                       f"{(done.stderr or done.stdout).strip()}")

    def unit_active(self):
        return bool(self.unit) and subprocess.run(
            ["systemctl", "is-active", "--quiet", self.unit]).returncode == 0

    # ---- what is kept: the transcript, and the hulls lent to a House

    def transcript(self, who, way, data):
        stamp = time.strftime("%H:%M:%S").encode()
        self.log.write(b"%s %s %s %s\n" % (stamp, who.encode(), way.encode(), data))
        self.log.flush()

    def save_state(self):
        staged = self.state_file + ".new"
        with open(staged, "w", encoding="utf-8") as handle:
            json.dump(self.state, handle, indent=2)
        os.chmod(staged, 0o600)
        os.replace(staged, self.state_file)

    # ---- the launch document

    def read_document(self):
        if not os.path.isfile(self.document):
            return None
        with open(self.document, encoding="utf-8") as handle:
            return json.load(handle)

    def write_document(self, document):
        staged = self.document + ".new"
        with open(staged, "w", encoding="utf-8") as handle:
            json.dump(document, handle, indent=2)
            handle.write("\n")
        os.chmod(staged, 0o640)
        self.own(staged)
        os.replace(staged, self.document)

    # ---- the render: the one in deploy/instance/render.sh, never a copy

    def render(self):
        before = os.stat(self.config)
        done = subprocess.run(
            ["bash", os.path.join(self.here, "render.sh"), self.env_file,
             os.path.join(self.here, "mushd.toml.tmpl"), self.config, self.world_binary],
            capture_output=True, text=True)
        if done.returncode != 0:
            raise Stop(f"the render refused:\n{done.stdout}{done.stderr}")
        if os.geteuid() == 0:
            os.chown(self.config, before.st_uid, before.st_gid)
        os.chmod(self.config, before.st_mode & 0o7777)

    def launch_is_live(self):
        with open(self.config, encoding="utf-8") as handle:
            return config_value(handle.read(), "tacsim.minds", "launch_path") is not None

    # ---- the private boots

    def start_private(self, label):
        """Boot the world on the private socket with a private gateway on
        loopback beside it, and wait until the front door answers."""
        self.boots += 1
        with open(self.config, encoding="utf-8") as handle:
            rendered = handle.read()
        port = self.args.private_port or (int(self.env["SSH_PORT"]) + 20000) % 65536
        edits = {
            ("ssh", "listen"): f'"127.0.0.1:{port}"',
            ("ssh", "trusted_socket"): '""',
            ("web", "listen"): f'"127.0.0.1:{port + 1}"',
            ("web", "trusted_proxies"): "[]",
        }
        sock = os.path.join(self.work, "ipc.sock")
        private = os.path.join(self.work, "private.toml")
        if self.split:
            gateway_data = os.path.join(self.work, "gateway-data")
            os.makedirs(gateway_data, exist_ok=True)
            self.own(gateway_data)
            edits[("ipc", "listen")] = f'"{sock}"'
            edits[("server", "data_dir")] = f'"{gateway_data}"'
        with open(private, "w", encoding="utf-8") as handle:
            handle.write(private_config(rendered, edits, drop={("web", "tls")}))
        self.own(private)
        started = time.monotonic()
        if self.split:
            if os.path.exists(sock):
                os.remove(sock)
            server = self.spawn([self.world_binary, "--config", self.config, "--ipc", sock],
                                f"server-{self.boots}.log")
            self.until(lambda: os.path.exists(sock), server, f"the {label} boot's IPC socket")
            front = self.spawn([os.path.join(self.bin, "mush-gateway"), "--config", private],
                               f"gateway-{self.boots}.log")
        else:
            front = self.spawn([self.world_binary, "--config", private], f"server-{self.boots}.log")
        self.until(lambda: port_open(port), front, f"the {label} boot's front door on 127.0.0.1:{port}")
        self.ssh_endpoint = ("127.0.0.1", port)
        self.timings[f"boot {self.boots} ({label})"] = round(time.monotonic() - started, 1)
        say(f"the {label} boot is up: the world on a private socket, the front door on "
            f"127.0.0.1:{port} (loopback only)")

    def spawn(self, command, log_name):
        log_path = os.path.join(self.work, log_name)
        log = open(log_path, "ab")
        self.own(log_path)
        child = subprocess.Popen(self.as_owner(command), stdout=log, stderr=log,
                                 stdin=subprocess.DEVNULL, start_new_session=True)
        child.log_path = log_path
        self.children.append(child)
        with open(self.pids_file, "a", encoding="utf-8") as handle:
            handle.write(f"{child.pid}\n")
        return child

    def reap_leftovers(self):
        """Stop the private boots of a run that was killed before it could:
        the pids it wrote down, where each is still a process of this work
        directory."""
        if not os.path.isfile(self.pids_file):
            return
        with open(self.pids_file, encoding="utf-8") as handle:
            pids = [int(line) for line in handle.read().split() if line.isdigit()]
        left = []
        for pid in pids:
            try:
                with open(f"/proc/{pid}/cmdline", "rb") as handle:
                    if self.work.encode() in handle.read():
                        left.append(pid)
            except OSError:
                pass
        for pid in left:
            say(f"stopping a private boot an earlier run left behind (pid {pid})")
            try:
                os.killpg(pid, signal.SIGTERM)
            except (ProcessLookupError, PermissionError):
                pass
        end = time.monotonic() + 90
        while left and time.monotonic() < end:
            left = [pid for pid in left if os.path.exists(f"/proc/{pid}")]
            time.sleep(0.5)
        for pid in left:
            try:
                os.killpg(pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass
        os.remove(self.pids_file)

    def until(self, ready, child, what):
        end = time.monotonic() + self.args.boot_timeout
        while not ready():
            if child.poll() is not None:
                raise Stop(f"{what} never came: the process exited {child.returncode}. "
                           f"The end of {child.log_path}:\n{tail(child.log_path)}")
            if time.monotonic() >= end:
                raise Stop(f"gave up after {self.args.boot_timeout}s waiting for {what}; "
                           f"see {child.log_path}")
            time.sleep(0.5)

    def stop_private(self):
        """Stop the gateway, then the world, and wait for both: the next boot
        takes the world's lifetime lock."""
        for child in reversed(self.children):
            if child.poll() is None:
                try:
                    os.killpg(child.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
        for child in reversed(self.children):
            try:
                child.wait(timeout=90)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
        self.children = []
        self.ssh_endpoint = None
        if os.path.isfile(self.pids_file):
            os.remove(self.pids_file)

    def public_endpoint(self):
        with open(self.config, encoding="utf-8") as handle:
            listen = config_value(handle.read(), "ssh", "listen")
        host, _, port = listen.rpartition(":")
        return (host, int(port))


def port_open(port):
    with socket.socket() as probe:
        probe.settimeout(1)
        return probe.connect_ex(("127.0.0.1", port)) == 0


def tail(path, lines=25):
    try:
        with open(path, encoding="utf-8", errors="replace") as handle:
            return "".join(handle.readlines()[-lines:])
    except OSError as error:
        return str(error)


def minor(text):
    """`2,000.00cr` as minor units."""
    found = re.match(r"^(-?)([\d,]+)(?:\.(\d\d))?cr$", text.strip())
    if not found:
        raise Stop(f"cannot read the amount {text!r}")
    value = int(found.group(2).replace(",", "")) * 100 + int(found.group(3) or 0)
    return -value if found.group(1) else value


# --- #1's hands: every identity looked up before it is made -------------------


class Staff:
    """The staff session and the idempotent steps typed in it. Names are
    matched by `#dbref` everywhere a command takes an object."""

    def __init__(self, box, account, password):
        self.box = box
        self.s = Session(box, "staff")
        refusal = self.s.connect(account, password)
        if refusal:
            raise Stop(f"cannot connect as the staff account {account}: {refusal}")
        self.accounts = None

    # ---- organisations, scopes, grants

    def orgs(self):
        found = {}
        for row in re.finditer(r"^#(\d+) (.+?) \[(\w+)\](?: \(hidden\))? — ",
                               self.s.ask("@org/list"), re.M):
            found.setdefault(row.group(2), int(row.group(1)))
        return found

    def org_show(self, org):
        text = self.s.ask(f"@org/show {org}")
        head = re.search(r"^#(\d+) .+ \[\w+\] — root scope #(\d+)$", text, re.M)
        if not head:
            raise Stop(f"@org/show {org} has no header:\n{text}")
        scopes = {m.group(2): int(m.group(1)) for m in
                  re.finditer(r"^  scope #(\d+) (.+?) \(depth ", text, re.M)}
        grants = {(int(m.group(1)), m.group(2), int(m.group(3)), "limit" in m.group(4))
                  for m in re.finditer(
                      r"^  grant #\d+ -> #(\d+) (\S+) on \w+\(OrgScopeId\((\d+)\)\)(.*)$", text, re.M)}
        return int(head.group(1)), int(head.group(2)), scopes, grants

    def org(self, name):
        """(id, root scope) of the organisation `name`, made if it is not there."""
        known = self.orgs()
        if name not in known:
            made = self.s.do(f"@org/create {name}", r"Created org #(\d+) \(.*\); root scope #(\d+)\.")
            return int(made.group(1)), int(made.group(2))
        org_id, root, _, _ = self.org_show(known[name])
        return org_id, root

    def scope(self, org_id, org_name, name):
        _, _, scopes, _ = self.org_show(org_id)
        if name in scopes:
            return scopes[name]
        made = self.s.do(f"@org/scope create {org_name}={name}", r"Created scope #(\d+) ")
        return int(made.group(1))

    def grant(self, actor, org_id, org_name, capability, scope=None, limit=None):
        _, root, _, grants = self.org_show(org_id)
        if (actor, capability, scope or root, limit is not None) in grants:
            return
        line = f"@org/grant #{actor}={org_name} {capability}"
        if scope is not None:
            line += f" scope=#{scope}"
        if limit is not None:
            line += f" limit={limit}"
        self.s.do(line, r"Granted .* \(grant #\d+\)\.")

    # ---- accounts

    def scan_accounts(self):
        """Every account by (owner, scope). `@account/list` stops at twenty rows
        with no cursor, so the accounts are read one by one."""
        self.accounts, number, misses = {}, 0, 0
        while misses < 4:
            number += 1
            text = self.s.ask(f"@account/show {number}")
            if "No such account." in text:
                misses += 1
                continue
            misses = 0
            head = re.search(r'^account #(\d+) ".*" \((\S+)\) \[(\w+)\]', text, re.M)
            owner = re.search(r"^  owner (.+)$", text, re.M)
            if not head or not owner or head.group(2) != CREDIT or head.group(3) != "Active":
                continue
            scope = re.search(r"^  scope #(\d+)$", text, re.M)
            self.accounts[(self.owner_key(owner.group(1)), int(scope.group(1)) if scope else None)] = number

    @staticmethod
    def owner_key(rendered):
        system = re.match(r"^sys:(\S+)$", rendered)
        if system:
            return ("sys", system.group(1))
        org = re.match(r"^org:.* \(#(\d+)\)$", rendered)
        if org:
            return ("org", int(org.group(1)))
        thing = re.match(r"^.* \(#(\d+)\)$", rendered)
        if thing:
            return ("obj", int(thing.group(1)))
        return ("?", rendered)

    def account(self, owner, scope, line):
        if self.accounts is None:
            self.scan_accounts()
        if (owner, scope) not in self.accounts:
            made = self.s.do(line, r"Opened account #(\d+) ")
            self.accounts[(owner, scope)] = int(made.group(1))
        return self.accounts[(owner, scope)]

    def system_account(self, name):
        return self.account(("sys", name), None, f"@account/system {name}={CREDIT}")

    def org_account(self, org_id, org_name, scope):
        return self.account(("org", org_id), scope, f"@account/open org:{org_name}/{scope}={CREDIT}")

    def balance(self, account):
        text = self.s.ask(f"@account/show {account}")
        found = re.search(r"^  balance (\S+)", text, re.M)
        if not found:
            raise Stop(f"@account/show {account} shows no balance:\n{text}")
        return minor(found.group(1))

    def fund(self, faucet, account, figure):
        """Mint up to `figure`: a second run tops nothing up twice."""
        short = figure - self.balance(account)
        if short > 0:
            self.s.do(f"@econ/mint {faucet}={account} {short}", r"Minted: transaction #\d+ posted\.")

    # ---- objects

    def find(self, name):
        rows = re.findall(r"^(.+)\(#(\d+)\)$", self.s.ask(f"@find {name}"), re.M)
        ids = sorted(int(number) for found, number in rows if found.strip() == name)
        return ids[0] if ids else None

    def room(self, name):
        found = self.find(name)
        if found is None:
            found = int(self.s.do(f"@dig {name}", r"created with room number #(\d+)\.").group(1))
        return found

    def thing(self, name):
        found = self.find(name)
        if found is None:
            found = int(self.s.do(f"@create {name}", r"Created: .*\(#(\d+)\)\.").group(1))
        return found

    def vault(self, name, container, org_name, scope):
        made = self.s.do(f"@vault/create {name}=container:#{container} owner=org:{org_name} scope={scope}",
                         r"Created vault #(\d+) |already backs vault #(\d+)\.")
        return int(made.group(1) or made.group(2))

    def facility(self, thing, capabilities, org_id, org_name, scope):
        """A facility of `capabilities`, owned by and titled to the organisation.
        `@facility/open` on an open facility would hand it back to #1."""
        shown = self.s.ask(f"@facility/show {thing}")
        if "No such facility." in shown:
            self.s.do(f"@facility/open {thing}=2 {' '.join(capabilities)}", r"opened with 2 slots")
            shown = self.s.ask(f"@facility/show {thing}")
        if f"owner org:{org_id}," not in shown:
            self.s.do(f"@facility/owner {thing}=org:{org_name}", r"re-assigned|already has that owner")
        self.s.do(f"@asset/title #{thing}=org:{org_name} scope={scope}",
                  r"Title recorded \(transaction #\d+\)|is already titled")

    def character(self, name):
        """The service character `name`: the one character of a login of the
        same name, made at the welcome screen with `create`. The login's
        password is random and is kept nowhere. Nobody plays these logins; on
        the day somebody must (the Registry operator, to recall a revision),
        #1 sets one with `@newpassword <name>=<password>`."""
        found = self.find(name)
        if found is not None:
            return found
        other = Session(self.box, name)
        try:
            refusal = other.connect(name, secrets.token_urlsafe(24), create=True)
        finally:
            other.close()
        found = self.find(name)
        if refusal or found is None:
            raise Stop(f"cannot make the service login {name}: {refusal or 'no such character after create'}")
        return found

    def close(self):
        self.s.close()


# --- phase 1: the plain boot --------------------------------------------------


def identities(box, staff):
    """Everything the document names, by the runbook's own commands, and the
    document itself in mode `initialize`."""
    say("the launch's identities, as #1 (deploy/README.md, steps 2 to 7)")
    faction = re.search(r"^\s+(\d+)\s\sRepublic(?:\s\[HIDDEN\])?\s\s\(", staff.s.ask("@faction"), re.M)
    if not faction:
        faction = staff.s.do("@faction/create Republic", r"Created faction Republic \(id (\d+)\)\.")
    republic = int(faction.group(1))
    registry_account = staff.system_account("launch.registry")
    venue_account = staff.system_account("launch.venue")
    faucet = staff.system_account("launch.faucet")

    # The service logins: the second origin agent, the clerk and the Registry
    # operator, each on a login of its own, so none is on #1's.
    agent = staff.character("LaunchAgent")
    clerk = staff.character("MarketClerk")
    operator = staff.character("RegistryOperator")

    houses, procurement = {}, []
    counter = staff.room("Named launch counter")
    for key, name, source, _ in HOUSES:
        org, root = staff.org(name)
        publication = staff.scope(org, name, "publication")
        treasury = staff.org_account(org, name, publication)
        for capability in ("roll.mind.publish", "asset.title.transfer", "economy.account.post"):
            staff.grant(1, org, name, capability, scope=publication)
        staff.fund(faucet, treasury, FILING_FEE_MINOR)
        houses[key] = {"organization": org, "scope": publication, "treasury": treasury,
                       "publisher": 1, "source_name": source}
        # Procurement (doc 17h cut M8f, Q-44-8): the agent on a login of its
        # own, at the counter, a member of the Republic, holding the House's
        # licences on the root scope up to the month's budget and attesting on
        # the House's scope; the root-scope treasury holds a month's budget.
        budget = box.args.procurement_budget
        buyer = staff.character(f"{name}Agent")
        staff.s.do(f"@faction/add Republic=#{buyer}/level:1", r"Added .* to Republic|Already a member")
        staff.s.do(f"@teleport #{buyer}=#{counter}", r"Teleported\.")
        staff.grant(buyer, org, name, "roll.holder.org", limit=budget)
        staff.grant(buyer, org, name, "roll.holder.org", scope=publication)
        staff.fund(faucet, staff.org_account(org, name, root), budget)
        procurement.append({"org": org, "scope": root, "agent": buyer,
                            "budget_minor_per_ic_month": budget, "standing": republic})

    basin, _ = staff.org("Basin")
    die_stock = staff.scope(basin, "Basin", "die-stock")
    factory_scope = staff.scope(basin, "Basin", "factory")
    shop = staff.thing("Basin fitting shop")
    output = staff.thing("Basin completed work")
    vaults = [staff.vault(f"HardwareStock{i}", staff.thing(thing), "Basin", die_stock)
              for i, thing in enumerate(("Basin die depot", "Basin agent stock one",
                                         "Basin agent stock two"))]
    supplier = staff.org_account(basin, "Basin", die_stock)
    integrator = staff.org_account(basin, "Basin", factory_scope)
    staff.facility(shop, ("core_integration", "core_refit"), basin, "Basin", factory_scope)

    # The ungoverning shop (doc 17h cut M8g; HardwareCoordinates.ungovern): the
    # Tether back room, chartered `core_ungovern` and never `core_refit`.
    tether, _ = staff.org("Tether")
    backroom = staff.scope(tether, "Tether", "backroom")
    back_account = staff.org_account(tether, "Tether", backroom)
    back_shop = staff.thing("Back-room shop")
    back_output = staff.thing("Back-room completed work")
    staff.facility(back_shop, ("core_ungovern",), tether, "Tether", backroom)

    for actor in (1, agent):
        for capability in ("industry.core_die.issue", "vault.deposit", "vault.withdraw"):
            staff.grant(actor, basin, "Basin", capability, scope=die_stock)
        for capability in ("industry.core.integrate", "industry.core.refit"):
            staff.grant(actor, basin, "Basin", capability, scope=factory_scope)
        staff.grant(actor, tether, "Tether", "industry.core.ungovern", scope=backroom)

    registry, _ = staff.org("Registry")
    review = staff.scope(registry, "Registry", "review")

    document = {
        "format": 2,
        "mode": "initialize",
        "registry_issuer": "hardware.factory",
        "registry": {"account": registry_account, "owner": {"System": "launch.registry"}},
        "venue": {"name": "launch.counter", "room": counter,
                  "recipient": {"account": venue_account, "owner": {"System": "launch.venue"}}},
        "houses": houses,
        "republic": republic,
        "hardware": {
            "importer": basin, "stock_scope": die_stock, "depot": vaults[0],
            "agents": [{"actor": 1, "stock": vaults[1]}, {"actor": agent, "stock": vaults[2]}],
            "grades": [10, 20, 30],
            "supplier_account": supplier,
            "integrator": {"account": integrator, "organization": basin, "scope": factory_scope},
            "factory": shop, "output_container": output,
            "ungovern": {
                "provider": {"account": back_account, "organization": tether, "scope": backroom},
                "factory": back_shop, "output_container": back_output},
        },
        "market": {
            "clerk": clerk,
            "registry_authority": {"organization": registry, "scope": review, "operators": [operator]},
            "terms": MARKET_TERMS,
        },
        "procurement": procurement,
    }
    box.write_document(document)
    say(f"wrote {box.document} in mode initialize")


# --- phase 2: the Initialize boot ---------------------------------------------


def registry_grants(staff, document):
    authority = document["market"]["registry_authority"]
    for operator in authority["operators"]:
        for word in REGISTRY_GRANTS:
            staff.grant(operator, authority["organization"], "Registry",
                        f"roll.registry.{word}", scope=authority["scope"])


def initialize(staff):
    """The four initialize actions, each typed while it is offered.

    A landed action answers `Action accepted.` and then `Action committed.`.
    The three industry actions are withdrawn once they have landed. The Roll's
    stays offered and is refused when typed again (`That input was rejected by
    the server.`: its provider wants empty commerce storage), so on a second
    run that refusal is how it is known to be done. The Active boot is the
    judge either way: it refuses a world whose commerce was never initialized.
    """
    for verb in INITIALIZE_ACTIONS:
        end = time.monotonic() + 180
        while True:
            # `+actions` and `+act` are the gateway's own lines.
            staff.s.type("+actions")
            staff.s.wait(r"Available actions:|No actions are available\.", 60)
            if not re.search(rf"^\s+\+act {re.escape(verb)} ", staff.s.settle(), re.M):
                say(f"initialize action already landed: {verb}")
                break
            staff.s.type(f"+act {verb} with {{}}")
            # Anything else that answers is "not now" (the world busy with the
            # last action's commit, most often): the loop types it again.
            answer = staff.s.wait(r"Action committed\.|rejected by the server|no longer available"
                                  rf"|game is busy|game is unavailable|{TRANSIENT}",
                                  120, f"the answer to +act {verb}").group(0)
            if "committed" in answer:
                say(f"initialize action landed: {verb}")
                break
            if "rejected" in answer or "no longer" in answer:
                say(f"initialize action already landed: {verb} (typed again, it is refused)")
                break
            if time.monotonic() >= end:
                raise Stop(f"+act {verb} did not land in 180s; the last answer: {answer}")
            time.sleep(3)


# --- phase 3: the Active boot --------------------------------------------------


def readiness(staff, document):
    """`@mind/launch` as #1: every operator `ready`. An operator that `lacks` a
    grant is given the seven again once, and asked again."""
    for attempt in (1, 2):
        text = staff.s.ask("@mind/launch")
        line = re.search(r"^Registry operators: (.+)\.$", text, re.M)
        if not line:
            raise Stop("@mind/launch names no Registry operators: the player market is not "
                       f"composed (is the document format 2 and Active?)\n{text[-600:]}")
        states = dict(re.findall(r"#(\d+) (ready|lacks [a-z, ]+?)(?=, #|$)", line.group(1)))
        wanted = {str(op) for op in document["market"]["registry_authority"]["operators"]}
        if set(states) == wanted and all(state == "ready" for state in states.values()):
            say(f"@mind/launch: {line.group(0)}")
            return line.group(0)
        if attempt == 1:
            say(f"@mind/launch: {line.group(0)} Giving the grants again.")
            registry_grants(staff, document)
    raise Stop(f"@mind/launch does not name every operator ready: {line.group(0)}")


def counter_door(staff, document):
    """A way to the counter on foot, and back: an exit from the Registry's desk
    at Ceres Freeport into the venue room, and one out of it. Made only where
    the room does not already show an exit of that name. #1 is walked through
    both rooms to read them, and is left in Limbo."""
    counter = document["venue"]["room"]
    desk = staff.find(COUNTER_DESK)
    if desk is None:
        raise Stop(f"no room named '{COUNTER_DESK}' in this world: the counter (#{counter}) has no "
                   "door, and no player can reach it to list or buy a mind")
    opened = 0
    for room, names, to in ((desk, COUNTER_DOOR, counter), (counter, COUNTER_BACK, desk)):
        view = staff.s.ask(f"@teleport #{room}")
        label = names.split(";")[0]
        # The label is in the room's view only as an exit's name, on the Exits
        # line or on a line it wrapped onto.
        if label in view:
            continue
        staff.s.do(f"@open {names}=#{to}", r"Opened\. \(#\d+\)")
        opened += 1
    staff.s.ask("@teleport #0")
    if opened:
        say(f"opened the counter's door: '{COUNTER_DOOR.split(';')[0]}' from the {COUNTER_DESK} "
            f"(#{desk}) to the counter (#{counter}), and '{COUNTER_BACK.split(';')[0]}' back")
    else:
        say(f"the counter (#{counter}) has its door from the {COUNTER_DESK} (#{desk})")


def major_npcs(box):
    """The major NPCs the shipped cast packs park (`"enabled": false`), read
    from the tree this script ships in, so the list is the content's own."""
    names, cast = [], os.path.join(box.repo, "crates", "tacsim", "content", "cast")
    for file in sorted(os.listdir(cast)) if os.path.isdir(cast) else []:
        if not file.endswith(".json"):
            continue
        with open(os.path.join(cast, file), encoding="utf-8") as handle:
            pack = json.load(handle)
        for entry in pack.get("entries", {}).values():
            definition = entry.get("def", {}) if isinstance(entry, dict) else {}
            if definition.get("tier") == "major" and definition.get("profile", {}).get("enabled") is False:
                names.append(definition["name"])
    return names


def enable_npcs(box, staff):
    """Q-44-47: the dev boxes run the major NPCs. Enabled here with
    `@agent/enable`, never by editing the shipped packs."""
    names = major_npcs(box)
    missing = []
    for name in names:
        npc = staff.find(name)
        if npc is None:
            missing.append(name)
            continue
        staff.s.do(f"@agent/enable #{npc}", r"Agent enabled\.")
    if missing:
        raise Stop(f"{len(missing)} major NPCs of the cast packs are not in the world: {missing}")
    say(f"enabled the {len(names)} major NPCs")
    return names


def seed_listing(box, staff, document):
    """One rated line on the player market, so a House has something to buy:
    #1 copies Standard Doctrine (or, as the configured publisher, imports a
    named House source), saves it, has the Registry rate it, files it
    personally and lists it at the counter. A House buys only a line whose
    rating scored above nothing; a copy of Standard does. Returns the
    listing's row."""
    if box.args.seed_line == "standard":
        source, selector = "Seedline", None
    else:
        _, _, source, selector = next(h for h in HOUSES if h[0] == box.args.seed_line)
    counter = document["venue"]["room"]
    staff.s.type(f"@teleport #{counter}")
    time.sleep(1)
    charter = f"{source} Market"

    def row_naming(text, pattern):
        # The lists cut a name to its column (twenty characters on
        # `+minds/holdings`), so a row is known by the start of the name.
        start = source.lower()[:16]
        for found in re.finditer(pattern, text, re.M):
            if start in found.group(0).lower():
                return found
        return None

    def listed():
        return row_naming(staff.s.ask("+minds"), r"^\s+(\S{8})\s\s.*$")

    row = listed()
    if row:
        return row.group(0).strip()
    mine = staff.account(("obj", 1), None, f"@account/open #1={CREDIT}")
    staff.fund(staff.system_account("launch.faucet"), mine, 2 * FILING_FEE_MINOR)
    if source.lower() not in [name.strip().lower() for name in staff.s.ask("@mind/list").split(",")]:
        if selector:
            staff.s.do(f"@mind/launch {selector}={source}", r"Launch draft imported\.")
        else:
            staff.s.do(f"@mind/new {source} FROM Standard", r"Mind draft created\.")
    info = staff.s.ask(f"@mind/info {source}")
    revision = re.search(r"revision (\d+);", info)
    if not revision:
        staff.s.type(f"@mind/save {source}=bring-up")
        revision = staff.s.wait(r"Saved revision (\d+) \(", 300, "the saved revision")
    reference = f"{source}@{revision.group(1)}"

    def filed():
        return row_naming(staff.s.ask("+minds/holdings"), r"^\s+filed (\d+)\s+owner\s.*$")

    # `@mind/file` reserves the charter the first time it is typed with none
    # open, refuses a revision the Registry has not rated, and files it once it
    # has. Each answer says which step is owed; the filing is done when
    # `+minds/holdings` shows the owner's row, whatever else scrolled past
    # (the rating's own steps answer `Queued.` and `Done.` in this session).
    rated = False
    end = time.monotonic() + 900
    while not filed():
        if time.monotonic() >= end:
            raise Stop(f"{reference} is not filed: +minds/holdings shows no owner row for {charter}")
        answer = staff.s.do(f"@mind/file {reference}={charter}",
                            r"Reserving a charter|has no finished Registry rating|Filing .* as "
                            r"|still being recorded|did not go through").group(0)
        if "no finished" in answer:
            if rated:
                raise Stop(f"the Registry finished its work on {reference} and left no rating to "
                           f"file with: {staff.s.text()[-300:]}")
            # A job that has just finished is still held for a pulse: only
            # that refusal is typed again.
            patience = time.monotonic() + 180
            while True:
                taken = staff.s.do(f"@mind/rate {reference}",
                                   r"Registry preparation queued\.|already working|unavailable").group(0)
                if "unavailable" not in taken:
                    break
                if time.monotonic() >= patience:
                    raise Stop(f"the Registry would not take {reference}: {staff.s.text()[-300:]}")
                time.sleep(3)
            started = time.monotonic()
            pulse = box.env.get("REGISTRY_PULSE_MS") or "1000"
            say(f"the Registry is rating {reference} at registry_pulse_ms = {pulse}: about half an "
                "hour for a copy of Standard Doctrine, longer for a House source (trial matches, "
                "each paced by its own simulation)")
            staff.s.wait(r"Registry work finished\.|Registry preparation refused|was not admitted",
                         box.args.rating_timeout, "the end of the rating")
            box.timings["rating (queued to finished)"] = round(time.monotonic() - started, 1)
            say(f"the rating took {box.timings['rating (queued to finished)']}s")
            rated = True
            end = time.monotonic() + 900
        time.sleep(3)
    number = filed()
    staff.s.do(f"@mind/publish {number.group(1)} form=sealed",
               r"Listing filed mind \d+ here as sealed|still being recorded")
    end = time.monotonic() + 180
    row = listed()
    while not row:
        if time.monotonic() >= end:
            raise Stop(f"{charter} is filed as {number.group(1)} and not listed: +minds does not show it")
        time.sleep(3)
        row = listed()
    say(f"listed on the player market: {row.group(0).strip()}")
    return row.group(0).strip()


def house_marks(name, count):
    """A House's seeded hulls' registry marks: `MER-01`, `MER-02`, ..."""
    return [f"{name[:3].upper()}-{n:02d}" for n in range(1, count + 1)]


def house_ships(staff, org):
    """The House's titled hulls, as `@asset/list` shows them."""
    return re.findall(rf"^#(\d+) (.+?) — org:.* \(#{org}\) \[Active\]\s*$",
                      staff.s.ask(f"@asset/list owner=org:{org}"), re.M)


def seed_house_hulls(box, staff, document):
    """Every House's own hulls (doc 47 cut CO.9). A House buys nothing with no
    hull: its cap is the floor of 400 per mille of its titled hulls. So #1
    seeds each House `--house-hulls` ships of `--house-design` at Ceres
    Freeport with `@tacsim/port seed`, each under a mark of its own, and the
    port's next fires lay them up and title them to the House on its root
    scope. A mark seeds once, so a second run seeds nothing new. Nothing of the
    Co-op's pool moves, so enrolment stays open. Who may fly them is the
    House's to grant: `@org/grant #<member>=<House> unit.deploy`."""
    if box.args.house_hulls <= 0:
        return
    for key, name, _, _ in HOUSES:
        org = document["houses"][key]["organization"]
        for mark in house_marks(name, box.args.house_hulls):
            staff.s.do(f"@tacsim/port seed ceres_port {box.args.house_design} {mark}=org:{org}",
                       r"Seeded order \d+|already a hull's")
    end = time.monotonic() + box.args.boot_timeout
    while True:
        short = [(name, len(house_ships(staff, document["houses"][key]["organization"])))
                 for key, name, _, _ in HOUSES]
        short = [(name, held) for name, held in short if held < box.args.house_hulls]
        if not short:
            break
        if time.monotonic() >= end:
            raise Stop("the seeded hulls were not titled in time: "
                       + "; ".join(f"{name} holds {held}" for name, held in short))
        time.sleep(3)
    say(f"each House holds {box.args.house_hulls} hulls of its own "
        f"({box.args.house_design}, docked at Ceres Freeport)")


def return_hulls(box, staff):
    """Hulls an older bring-up LENT a House from the Co-op's pool (before doc 47
    cut CO.9 gave the Houses their own) go back to the Trust: a Dray titled to
    a House closes `+coop/enrol` for every player. A no-op unless the state
    file says something is lent."""
    lent = box.state.get("lent")
    if not lent:
        return
    for hull in lent["hulls"]:
        shown = staff.s.ask(f"@asset/show #{hull['hull']}")
        if re.search(rf"title org:.* \(#{lent['org']}\) ", shown):
            staff.s.do(f"@asset/transfer #{hull['hull']}=org:{lent['trust']} scope={hull['scope']}",
                       r"Title transferred \(transaction #\d+\)")
    # The Co-op's own word that its pool is whole again: the port's readiness
    # report has a line for each programme's hulls, and enrolment follows it.
    report = staff.s.ask("@tacsim/port status")
    wrong = [line.strip() for line in re.findall(r"^\s+\S+ hulls: .*$", report, re.M)
             if not line.rstrip().endswith("[ready]")]
    if wrong:
        raise Stop("the hulls went back and the port still reads them not ready, so Co-op "
                   f"enrolment may be closed: {'; '.join(wrong)}")
    del box.state["lent"]
    box.save_state()
    say(f"handed the {len(lent['hulls'])} Drays back to the {TRUST}; Co-op enrolment is open again "
        "(@tacsim/port status reads every programme's hulls ready)")


def purchases(box, org):
    """The House's procurement ledger, read from the store: the rows whose
    licence was granted. No typed command shows it."""
    port = box.env["REDIS_PORT"]
    scan = subprocess.run(
        ["redis-cli", "-h", "127.0.0.1", "-p", port, "--scan", "--pattern",
         "mod:tacsim:action-state/v1/tacsim.procurement_ledger/*"],
        capture_output=True, text=True)
    if scan.returncode != 0:
        raise Stop(f"redis-cli cannot read the procurement ledger on 127.0.0.1:{port}: {scan.stderr}")
    rows = []
    for key in scan.stdout.split():
        got = subprocess.run(["redis-cli", "-h", "127.0.0.1", "-p", port, "JSON.GET", key],
                             capture_output=True, text=True).stdout.strip()
        if got:
            value = json.loads(got)
            rows.append(value.get("value", value))
    return [row for row in rows if row.get("org") == org and row.get("phase") == "granted"]


def await_purchase(box, staff, document):
    """A licence the House's own agent bought, read from its ledger, with the
    treasury's balance beside it."""
    house = next(h for h in HOUSES if h[0] == box.args.purchase_house)
    org = document["houses"][house[0]]["organization"]
    root = next(p["scope"] for p in document["procurement"] if p["org"] == org)
    treasury = staff.org_account(org, house[1], root)
    started = time.monotonic()
    while True:
        bought = purchases(box, org)
        if bought:
            row = bought[0]
            say(f"{house[1]}'s agent #{row.get('agent')} bought licence {row.get('licence')} of "
                f"charter {row.get('charter')} for {row.get('price_minor')} minor credits "
                f"({round(time.monotonic() - started)}s after the boot); the root treasury holds "
                f"{staff.balance(treasury)} minor")
            box.timings["purchase (boot to granted)"] = round(time.monotonic() - started, 1)
            return row
        if time.monotonic() - started > box.args.purchase_timeout:
            raise Stop(f"{house[1]} bought nothing in {box.args.purchase_timeout}s. The world's log "
                       "says why, in its tacsim.procurement.refused and "
                       "tacsim.procurement.month_closed lines; run again with --seed-only")
        time.sleep(5)


# --- after the unit is up: the LLM routes (doc 44 section 8, step 8) -------------


def llm_check(box, staff):
    """Ask an NPC a question and let an AI squad deliberate, then read
    `@llm/log`: there must be an `ok` row for an `npc_dialogue_*` route and one
    for `tacsim_deliberate`. A reply proves nothing, because an NPC whose
    provider fails speaks its fallback line. `@llm/budget` is recorded."""
    report, failures = [], []
    names = major_npcs(box)
    name = box.args.llm_npc or (names[0] if names else None)
    npc = staff.find(name) if name else None
    if npc is None:
        failures.append(f"no NPC to ask ({name!r} is not in the world)")
    else:
        staff.s.type(f"@teleport #{npc}")
        time.sleep(2)
        staff.s.do(f"@agent/enable #{npc}", r"Agent enabled\.")
        staff.s.type(f"say {name}, what is your work here, in a sentence?")
        end = time.monotonic() + box.args.llm_timeout
        row = None
        while time.monotonic() < end and not row:
            time.sleep(5)
            row = re.search(r"^  (npc_dialogue_\w+) (ok|refused|error|budget|disabled) .*$",
                            staff.s.ask("@llm/log"), re.M)
        shown = staff.s.ask(f"@agent/show #{npc}")
        result = re.search(r"^  Last turn result: (.+)$", shown, re.M)
        report.append(f"NPC {name} (#{npc}): {row.group(0).strip() if row else 'no npc_dialogue row'}")
        report.append(f"  @agent/show: {result.group(1) if result else 'no last turn'}")
        if not row or row.group(2) != "ok":
            failures.append("no `ok` row for an npc_dialogue_* route")
        elif not result or not result.group(1).startswith("the model answered"):
            failures.append("the NPC's last turn was not the model's")

    lobby = staff.find("Simulator Lobby")
    if lobby is None:
        failures.append("no Simulator Lobby in the world, so no match to deliberate in")
    else:
        staff.s.type(f"@teleport #{lobby}")
        time.sleep(2)
        try:
            staff.s.do("SIM TEAM red", r"takes red slot|already", 30)
            staff.s.do("SIM START", r"Match \d+ is boarding", 30)
            staff.s.type("red")
            time.sleep(2)
            staff.s.do("BOARD", r"Match \d+ is running", 180)
            end = time.monotonic() + box.args.llm_timeout
            row = None
            while time.monotonic() < end and not row:
                time.sleep(5)
                row = re.search(r"^  tacsim_deliberate (ok|refused|error|budget|disabled) .*$",
                                staff.s.ask("@llm/log tacsim_deliberate"), re.M)
            report.append(f"AI squad: {row.group(0).strip() if row else 'no tacsim_deliberate row'}")
            if not row or row.group(1) != "ok":
                failures.append("no `ok` row for tacsim_deliberate")
        except Stop as error:
            failures.append(f"the match did not run: {str(error).splitlines()[0]}")
        finally:
            staff.s.type("SIM ABORT")
            time.sleep(3)
            staff.s.type("SIM LEAVE")
            time.sleep(1)
    staff.s.type("@teleport #0")
    time.sleep(1)
    for line in ("@llm/budget", "@llm/dashboard"):
        report.append(f"{line}:")
        report.extend("  " + row for row in staff.s.ask(line).strip().splitlines() if row.strip())
    path = os.path.join(box.work, "llm-check.txt")
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(time.strftime("%Y-%m-%d %H:%M:%S %z\n") + "\n".join(report) + "\n")
        handle.write(("FAILED: " + "; ".join(failures) if failures else "PASSED") + "\n")
    for line in report:
        say(line)
    say(f"recorded in {path}")
    if failures:
        raise Stop("the LLM check FAILED: " + "; ".join(failures) + ". Play does not run without "
                   "the LLM (doc 44 Q-44-6), so this holds the box back: check [llm].enabled, the "
                   "provider keys in the unit's EnvironmentFile and the routes, then run again "
                   "with --llm-check")
    say("the LLM check passed: an npc_dialogue row and a tacsim_deliberate row, both ok")


# --- the walk ---------------------------------------------------------------------


def staff_password(args):
    if args.password_file:
        with open(args.password_file, encoding="utf-8") as handle:
            return handle.readline().rstrip("\r\n")
    if sys.stdin.isatty():
        return getpass.getpass(f"Password for the staff account {args.staff}: ")
    return sys.stdin.readline().rstrip("\r\n")


def bootstrap_staff(box, password):
    """The first staff login, on a store that has no account yet. Any existing
    account is the binary's own refusal and means the login is already there."""
    done = subprocess.run(
        box.as_owner([box.world_binary, "--config", box.config, "--bootstrap-staff", box.args.staff]),
        input=password + "\n", capture_output=True, text=True)
    if done.returncode == 0:
        say(f"bootstrapped the staff account {box.args.staff}, linked to God #1")
    elif "an account already exists" in done.stdout + done.stderr:
        say("the store already has an account: connecting as the staff account that is there")
    else:
        raise Stop(f"--bootstrap-staff refused:\n{done.stdout}{done.stderr}")


def bring_up(box, password):
    args = box.args
    document = box.read_document()
    if document is None and box.launch_is_live():
        # A document that went with a wiped world, and a config still naming it.
        box.render()
    if box.unit_active():
        say(f"stopping {box.unit}: the world is down until the Active boot has passed")
        box.systemctl("stop")
    elif box.unit:
        say(f"{box.unit} is stopped, and stays stopped until the Active boot has passed")
    if box.split and not args.no_systemd:
        say("players are kept off: nothing answers on the instance's IPC socket, so the public "
            "gateway has no world to log in to; the bring-up's own gateway is on loopback")
    bootstrap_staff(box, password)

    if document is None:
        box.start_private("plain")
        staff = Staff(box, args.staff, password)
        try:
            identities(box, staff)
        finally:
            staff.close()
        box.stop_private()
        document = box.read_document()

    box.render()
    if not box.launch_is_live():
        raise Stop(f"{box.document} exists and the render left launch_path commented")
    if document["mode"] == "initialize":
        try:
            box.start_private("Initialize")
        except Stop as stop:
            # A run that stopped between the last initialize action and the
            # document's edit: the Roll's records exist, which only its own
            # action writes, and it is typed after the other three.
            if "commerce initialization preflight" not in str(stop):
                raise
            say("the Initialize boot is refused because commerce is already initialized: "
                "all four actions landed in an earlier run")
            box.stop_private()
        else:
            staff = Staff(box, args.staff, password)
            try:
                registry_grants(staff, document)
                initialize(staff)
            finally:
                staff.close()
            box.stop_private()
        document["mode"] = "active"
        box.write_document(document)
        say(f"set {box.document} to mode active")

    box.start_private("Active")
    staff = Staff(box, args.staff, password)
    try:
        registry_grants(staff, document)
        ready = readiness(staff, document)
        if not args.no_npcs:
            enable_npcs(box, staff)
        # A store an older run lent the Co-op's Drays on: they go back before
        # anybody is let in, so enrolment is never left closed.
        return_hulls(box, staff)
        # Doc 47 cut CO.9: the Houses' own hulls.
        seed_house_hulls(box, staff, document)
        # Last, so that a world with no desk to hang the door on stops here with
        # the NPCs enabled, every lent hull back and the Houses' hulls laid up.
        counter_door(staff, document)
    finally:
        staff.close()
        box.stop_private()
    return ready


def staff_on_the_unit(box, password):
    """A staff session at the instance's own front door, once it answers."""
    box.ssh_endpoint = box.public_endpoint()
    end = time.monotonic() + box.args.boot_timeout
    while True:
        try:
            return Staff(box, box.args.staff, password)
        except (Stop, OSError):
            if time.monotonic() >= end:
                raise
            time.sleep(5)


def checks_on_the_unit(box, password):
    """What is checked on the instance's own unit, with its own environment:
    the readiness line again, and the LLM routes."""
    staff = staff_on_the_unit(box, password)
    try:
        readiness(staff, box.read_document())
        if not box.args.no_llm_check:
            llm_check(box, staff)
    finally:
        staff.close()


def door_on_the_unit(box, password):
    """A box that was brought up before the counter had a door: hang it on the
    running world. The one step a second run on a live box writes."""
    staff = staff_on_the_unit(box, password)
    try:
        counter_door(staff, box.read_document())
    finally:
        staff.close()


def seed_and_purchase(box, password):
    """On the running unit, with players already let in: one rated line on the
    player market, and one House's purchase of it.

    It is here and not in the Active boot because a rating is long at any
    pulse: its trial matches are paced by their own simulation, not by the
    Registry's pulse (a copy of Standard Doctrine took 32 minutes at 50 ms,
    and a House source takes longer). Nobody is kept off for that. The
    Registry's job runs in the world whether or not
    this session stays connected, so a run that is stopped here can be started
    again (`--seed-only`) and takes the line up where it stands."""
    args = box.args
    document = box.read_document()
    house = next(h for h in HOUSES if h[0] == args.purchase_house)
    org = document["houses"][house[0]]["organization"]
    staff = staff_on_the_unit(box, password)
    try:
        seed_listing(box, staff, document)
        if not args.no_purchase and not purchases(box, org):
            # The House's own hulls, seeded in the Active boot (doc 47 cut CO.9);
            # a store brought up before them gets them here.
            return_hulls(box, staff)
            seed_house_hulls(box, staff, document)
            # A House shops once an IC month, and a month it found nothing in
            # stays closed until the next one or a restart of the world. The
            # gateway holds every connection across a server restart.
            staff.close()
            if box.unit:
                say(f"restarting {box.unit} so {house[1]}'s month opens again (the gateway holds "
                    "connections; players see a restart, not a drop)")
                box.systemctl("restart")
            else:
                say(f"no unit to restart: restart the world now, or {house[1]} waits for the next IC month")
            time.sleep(5)
            staff = staff_on_the_unit(box, password)
            await_purchase(box, staff, document)
    finally:
        staff.close()


def main():
    parser = argparse.ArgumentParser(
        description="Bring the named launch up on one instance (doc 44 cut MW.1).",
        epilog="deploy/README.md, \"Named launch and the player market\", is the runbook.")
    parser.add_argument("env", help="the installed instance.env, with mushd.toml beside it")
    parser.add_argument("--staff", default="keeper", help="the staff login linked to God #1 (keeper)")
    parser.add_argument("--password-file", help="a file whose first line is the staff password")
    parser.add_argument("--unit", help="the instance's world unit, when it is not found by name")
    parser.add_argument("--no-systemd", action="store_true",
                        help="a rig: no unit is stopped or started, and the instance must be down")
    parser.add_argument("--bin-dir", help="where the binaries are (<BUILD_DIR>/target/release)")
    parser.add_argument("--private-port", type=int,
                        help="the loopback SSH port of the private gateway (SSH_PORT + 20000)")
    parser.add_argument("--seed-line", default="standard",
                        choices=["standard"] + [h[0] for h in HOUSES],
                        help="what #1 rates and lists on the player market: `standard`, a copy of "
                             "Standard Doctrine named Seedline (the default, and the quickest to "
                             "rate), or a named House source")
    parser.add_argument("--purchase-house", default="meridiani", choices=[h[0] for h in HOUSES],
                        help="the House whose purchase is waited for")
    parser.add_argument("--house-hulls", type=int, default=3,
                        help="the hulls of its own each House is seeded at Ceres Freeport (3; 0 "
                             "seeds none)")
    parser.add_argument("--house-design", default="dray",
                        help="the design each House's hulls are seeded as (dray)")
    parser.add_argument("--procurement-budget", type=int, default=200_000,
                        help="each House's monthly budget and its treasury's funding, minor credits")
    parser.add_argument("--no-npcs", action="store_true", help="leave the major NPCs parked")
    parser.add_argument("--no-seed", action="store_true", help="rate and list no line (and buy none)")
    parser.add_argument("--no-purchase", action="store_true", help="do not wait for a House's purchase")
    parser.add_argument("--no-start", action="store_true",
                        help="leave the unit stopped after the Active boot's checks")
    parser.add_argument("--no-llm-check", action="store_true", help="skip the LLM routes' check")
    parser.add_argument("--llm-check", action="store_true",
                        help="only the checks on the running unit: readiness and the LLM routes")
    parser.add_argument("--seed-only", action="store_true",
                        help="only the seed line and the House's purchase, on the running unit "
                             "(what a stopped run comes back to; a rating is half an hour or more)")
    parser.add_argument("--llm-npc", help="the NPC to ask (the first major NPC of the cast packs)")
    parser.add_argument("--llm-timeout", type=int, default=120)
    parser.add_argument("--boot-timeout", type=int, default=600)
    parser.add_argument("--rating-timeout", type=int, default=5 * 3600)
    parser.add_argument("--purchase-timeout", type=int, default=600)
    args = parser.parse_args()
    # Its own handlers, SIGINT's included: a shell that started this in the
    # background hands it SIGINT ignored, and a closed terminal sends SIGHUP.
    for number in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(number, interrupted)
    box = None
    try:
        box = Box(args)
        password = staff_password(args)
        if not 4 <= len(password.encode()) <= 128:
            raise Stop("the staff password must be 4 to 128 bytes")
        if args.llm_check:
            checks_on_the_unit(box, password)
            return 0
        if args.seed_only:
            seed_and_purchase(box, password)
            for what, seconds in box.timings.items():
                say(f"  {what}: {seconds}s")
            return 0
        document = box.read_document()
        if document and document["mode"] == "active" and box.launch_is_live() and box.unit_active():
            # A second run on a box that is already live: nothing is stopped.
            say(f"the launch is Active and {box.unit} is running: nobody is put off; "
                "checking it where it runs")
            door_on_the_unit(box, password)
        else:
            ready = bring_up(box, password)
            say("the Active boot passed: " + ready)
            for what, seconds in box.timings.items():
                say(f"  {what}: {seconds}s")
        if args.no_systemd or args.no_start:
            say("the world is left stopped; start it, then run this again with --llm-check and, "
                "for the seed line and the House's purchase, with --seed-only")
            return 0
        if not box.unit_active():
            say(f"starting {box.unit}: players are let in from here")
            box.systemctl("start")
        held_back = None
        try:
            checks_on_the_unit(box, password)
        except Stop as stop:
            # The seed line does not wait on the LLM; the failure is still the
            # run's result.
            held_back = stop
            print(f"launch-bringup: {stop}", file=sys.stderr)
        if not args.no_seed:
            box.timings.clear()
            say("the seed line and the House's purchase, on the running world. The rating is half "
                "an hour or more; stop this whenever, and `--seed-only` takes it up again")
            seed_and_purchase(box, password)
            for what, seconds in box.timings.items():
                say(f"  {what}: {seconds}s")
        if held_back:
            raise held_back
        say("done")
        return 0
    except Stop as stop:
        print(f"launch-bringup: STOPPED: {stop}", file=sys.stderr)
        if box and box.unit and not box.unit_active():
            print(f"launch-bringup: {box.unit} is left STOPPED, so nobody is let in to a half-made "
                  "launch. Run this again: it takes up where it stopped", file=sys.stderr)
        return 1
    except Interrupted as signal_name:
        print(f"launch-bringup: interrupted ({signal_name}). The private boots are stopped; run "
              "this again and it takes up where it stopped", file=sys.stderr)
        return 130
    finally:
        if box:
            # Nothing may cut the clean-up short.
            for number in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
                signal.signal(number, signal.SIG_IGN)
            box.stop_private()


if __name__ == "__main__":
    sys.exit(main())
