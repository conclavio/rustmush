#!/usr/bin/env python3
"""rmbak.py: whole-instance backup, restore and check for a rustmush instance.

The design is docs/design/52-instance-backup-and-restore.md; the runbook is
docs/ops.md, "Whole-instance backup and restore". One file, the standard
library only, so the box that takes backups and the box that restores them
need nothing but python3, redis-server, redis-cli, tar's formats (read and
written here) and zstd (gzip when zstd is missing).

  rmbak.py backup <instance>            one archive of everything the instance needs
  rmbak.py run                          the timer's entry: every configured instance, prune, status
  rmbak.py verify [--deep] <archive>    checksums; --deep also loads the store and re-derives the facts
  rmbak.py show <archive>               the manifest, in brief
  rmbak.py pull                         fetch new archives from the box that takes them
  rmbak.py restore <archive> --as <n>   stand a COPY up beside nothing: own Redis, units, ports
  rmbak.py restore <archive> --in-place put the store and data back under the instance's own name
  rmbak.py check <instance>             the restored instance boots, answers, and agrees with its manifest
  rmbak.py remove <instance> --yes      take a restored copy away again
  rmbak.py tree <archive>               lay the archive's slice of the build tree on a box that lost its own
  rmbak.py facts <instance>             the facts of a live instance, as the manifest records them
  rmbak.py status                       the timer, the last run, the newest archives

WHAT AN ARCHIVE IS. `<instance>-<UTC>-<commit8>.rmbak.tar.zst`: a tar whose
first member is `manifest.json` and whose last is `manifest.sha256`. Between
them: `redis/dump.rdb` (the store at one instant), `data/` (the data directory
without its logs), `config/` (instance.env, the rendered mushd.toml, the unit
files, the Redis config) and `build/` (the commit stamps, both binaries, the
help files and the deploy directory). The manifest lists every member with its
size, mode and SHA-256, and carries the FACTS of the store: key counts and a
digest by family, every ledger's position and digest across the store and the
archive tier, the economy's sums, the IC clock's checkpoint.

THE CUT. The store is one Redis fork (`redis-cli --rdb`), which Redis takes
between two commands, so it is the state a `kill -9` at that instant would
have left: every MULTI/EXEC whole. The files the store refers to (the ledger
archive tier, the key families, the quarantine spool, launch.json) are copied
after the fork, and the copy is kept only if their names, sizes, mtimes and
inodes are the same after the copy as they were before the fork; otherwise the
whole cut is taken again. So the files are exactly what they were at the fork.

THE FACTS are computed from the snapshot itself, never from the live store: the
backup loads its own `dump.rdb` into a scratch Redis and reads it there. A
restore loads the same file into its own Redis and computes the same facts
before the server starts; any difference fails the restore.
"""

import argparse
import calendar
import fcntl
import gzip
import hashlib
import io
import json
import os
import pty
import pwd
import re
import secrets as pysecrets
import select
import shlex
import shutil
import socket
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
import termios
import time
import urllib.error
import urllib.request
import zlib

FORMAT = "rmbak/1"
NAME_RE = re.compile(r"^([a-z][a-z0-9]*)-(\d{8}T\d{6}Z)-([0-9a-f]{8})\.rmbak\.tar\.(zst|gz)$")
INSTANCE_RE = re.compile(r"^[a-z][a-z0-9]*$")
DEFAULT_CONF = "/etc/rustmush-backup.conf"
CONF_DEFAULTS = {
    "BACKUP_DIR": "/var/backups/rustmush",
    "INSTANCES": "",
    "KEEP": "18",
    "KEEP_DAILY_DAYS": "0",
    "SECRETS": "in-archive",
    "BINARIES": "yes",
    "READ_GROUP": "rmbackup",
    "PULL_SSH": "",
    "PULL_DIR": "/var/backups/rustmush-pulled",
    "PULL_KEEP": "18",
    "PULL_KEEP_DAILY_DAYS": "30",
    "PULL_DEEP": "yes",
    "MAX_AGE_HOURS": "9",
}
# Paths under the data directory that hold private key material. With
# SECRETS=apart they are left out of the archive and listed in the manifest
# with their digests, for a copy kept elsewhere to be checked against.
SECRET_PATHS = ("ssh_host_ed25519", "offer-keys/", "resolution-keys/", "decision-keys/", "geology-keys/")
# Never archived: logs (the journal has them), the bring-up's scratch, locks.
CUT_ATTEMPTS = 4
ANSI = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b[()][0-9A-Za-z]|\x1b[=>]|\r")
TRANSIENT = (r"You're doing that too fast\.|post-outage recovery countdown|still starting; please try again"
             r"|please retry in a moment|server is busy; please retry that input")
MONTHS = (31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31)
MONTH_NAMES = ("January", "February", "March", "April", "May", "June", "July", "August",
               "September", "October", "November", "December")


class Fail(Exception):
    """A refusal or a failed check: said once, exit 1."""


def say(text):
    print(f"rmbak: {text}", flush=True)


def shout(text):
    """A failure, on stderr; at the journal's `err` priority under systemd."""
    prefix = "<3>" if os.environ.get("JOURNAL_STREAM") else ""
    for line in text.splitlines() or [""]:
        print(f"{prefix}rmbak: {line}", file=sys.stderr, flush=True)


def utc(unix=None):
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(unix if unix is not None else time.time()))


def ic_date(ic):
    """An IC second as the game calendar reads it (mush_core::CivilDate:
    365-day years, year 0 at IC 0)."""
    days, secs = divmod(ic, 86400)
    year, doy = divmod(days, 365)
    month = 12
    for index, length in enumerate(MONTHS):
        if doy < length:
            month = index + 1
            break
        doy -= length
    return f"{year:04d}-{month:02d}-{doy + 1:02d} {secs // 3600:02d}:{secs % 3600 // 60:02d}:{secs % 60:02d}"


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def run(command, **kwargs):
    """Run a command; a non-zero exit is a Fail carrying what it printed."""
    done = subprocess.run(command, capture_output=True, text=True, **kwargs)
    if done.returncode != 0:
        raise Fail(f"`{' '.join(shlex.quote(str(c)) for c in command)}` exited {done.returncode}: "
                   f"{(done.stderr or done.stdout).strip()[-800:]}")
    return done.stdout


def is_root():
    return os.geteuid() == 0


def user_exists(name):
    try:
        pwd.getpwnam(name)
        return True
    except KeyError:
        return False


def write_atomic(path, data, mode=0o600):
    """Write a small file whole or not at all: temp, fsync, rename."""
    directory = os.path.dirname(path) or "."
    fd, tmp = tempfile.mkstemp(prefix=".tmp-", dir=directory)
    try:
        with os.fdopen(fd, "wb") as handle:
            handle.write(data if isinstance(data, bytes) else data.encode("utf-8"))
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(tmp, mode)
        os.rename(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise
    fsync_dir(directory)


def fsync_dir(directory):
    fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


# --- configuration -----------------------------------------------------------


def read_kv(path):
    """KEY=value lines, read and never evaluated (deploy/instance/render.sh)."""
    out = {}
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            line = line.rstrip("\r\n")
            if not line or line.lstrip().startswith("#") or "=" not in line:
                continue
            key, value = line.split("=", 1)
            out.setdefault(key.strip(), value.strip())
    return out


def load_conf(path):
    conf = dict(CONF_DEFAULTS)
    path = path or os.environ.get("RUSTMUSH_BACKUP_CONF", DEFAULT_CONF)
    if os.path.isfile(path):
        for key, value in read_kv(path).items():
            if key not in CONF_DEFAULTS:
                raise Fail(f"{path}: unknown key {key}")
            conf[key] = value
    if conf["SECRETS"] not in ("in-archive", "apart"):
        raise Fail(f"{path}: SECRETS is `in-archive` or `apart`, not {conf['SECRETS']!r}")
    for key in ("BINARIES", "PULL_DEEP"):
        if conf[key] not in ("yes", "no"):
            raise Fail(f"{path}: {key} is `yes` or `no`, not {conf[key]!r}")
    for key in ("KEEP", "PULL_KEEP", "MAX_AGE_HOURS"):
        if not conf[key].isdigit() or int(conf[key]) < 1:
            raise Fail(f"{path}: {key} is a whole number, at least 1")
    for key in ("KEEP_DAILY_DAYS", "PULL_KEEP_DAILY_DAYS"):
        if not conf[key].isdigit():
            raise Fail(f"{path}: {key} is a whole number of days (0 keeps no dailies)")
    return conf


class Layout:
    """Where an installed instance keeps things (deploy/README.md). RMBAK_SYSROOT
    moves the whole tree under a directory, for the tests."""

    def __init__(self, instance):
        if not INSTANCE_RE.match(instance):
            raise Fail(f"not an instance name: {instance!r}")
        self.sysroot = os.environ.get("RMBAK_SYSROOT", "")
        self.n = instance
        self.root = f"{self.sysroot}/opt/rustmush/{instance}"
        self.env_file = f"{self.root}/instance.env"
        self.config = f"{self.root}/mushd.toml"
        self.units_dir = f"{self.sysroot}/etc/systemd/system"
        self.redis_conf = f"{self.sysroot}/etc/redis/redis-{instance}mush.conf"
        self.redis_dir = f"{self.sysroot}/var/lib/redis-{instance}mush"
        self.redis_unit = f"redis-{instance}mush.service"
        self.server_unit = f"{instance}mush-server.service"
        self.gateway_unit = f"{instance}mush-gateway@a.service"
        self.gateway_units = [f"{instance}mush-gateway@{g}.service" for g in ("a", "b")]
        self.gw_state = f"{self.sysroot}/var/lib/{instance}mush-gateway"
        self.marker = f"{self.root}/.restored-by-rmbak"
        self.hold = f"{self.root}/server.hold"

    def env(self):
        if os.path.islink(self.env_file) or not os.path.isfile(self.env_file):
            raise Fail(f"no instance.env at {self.env_file} (is {self.n} installed on this box?)")
        env = read_kv(self.env_file)
        for key in ("INSTANCE", "DATA_DIR", "BUILD_DIR"):
            if not env.get(key):
                raise Fail(f"{self.env_file} has no {key} line")
        if env["INSTANCE"] != self.n:
            raise Fail(f"{self.env_file} is instance {env['INSTANCE']!r}, not {self.n!r}")
        return env

    def unit_files(self):
        """The instance's unit files and their drop-ins, as installed."""
        names = [self.server_unit, f"{self.n}mush-gateway@.service", self.redis_unit]
        found = []
        for name in names:
            path = os.path.join(self.units_dir, name)
            if os.path.isfile(path) and not os.path.islink(path):
                found.append((name, path))
            dropins = path + ".d"
            if os.path.isdir(dropins):
                for extra in sorted(os.listdir(dropins)):
                    if extra.endswith(".conf") and os.path.isfile(os.path.join(dropins, extra)):
                        found.append((f"{name}.d/{extra}", os.path.join(dropins, extra)))
        return found


def redis_target(url):
    """(unix path, host, port) of a REDIS_URL."""
    if url.startswith("redis+unix:///"):
        return url[len("redis+unix://"):].split("?", 1)[0], None, None
    if url.startswith("redis://"):
        rest = url[len("redis://"):]
        authority = re.split(r"[/?#]", rest, maxsplit=1)[0].rsplit("@", 1)[-1]
        host, _, port = authority.partition(":")
        return None, host or "127.0.0.1", int(port or 6379)
    raise Fail(f"unsupported REDIS_URL: {url!r}")


def redis_cli_args(url):
    path, host, port = redis_target(url)
    return ["-s", path] if path else ["-h", host, "-p", str(port)]


# --- a small Redis client ------------------------------------------------------


class RedisError(Exception):
    pass


class Redis:
    """RESP2 over a Unix socket or TCP: enough to read a whole store."""

    def __init__(self, unix=None, host=None, port=None, timeout=300):
        if unix:
            self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            self.sock.settimeout(timeout)
            try:
                self.sock.connect(unix)
            except OSError:
                self.sock.close()
                raise
        else:
            self.sock = socket.create_connection((host, port), timeout=timeout)
        self.file = self.sock.makefile("rwb")

    @classmethod
    def from_url(cls, url):
        path, host, port = redis_target(url)
        return cls(unix=path, host=host, port=port)

    def _send(self, args):
        out = [b"*%d\r\n" % len(args)]
        for arg in args:
            if isinstance(arg, str):
                arg = arg.encode("utf-8")
            elif isinstance(arg, int):
                arg = b"%d" % arg
            out.append(b"$%d\r\n%s\r\n" % (len(arg), arg))
        self.file.write(b"".join(out))

    def _read(self):
        line = self.file.readline()
        if not line:
            raise RedisError("the connection closed")
        kind, rest = line[:1], line[1:-2]
        if kind == b"+":
            return rest
        if kind == b"-":
            return RedisError(rest.decode("utf-8", "replace"))
        if kind == b":":
            return int(rest)
        if kind == b"$":
            size = int(rest)
            if size < 0:
                return None
            data = self.file.read(size + 2)
            return data[:-2]
        if kind == b"*":
            size = int(rest)
            return None if size < 0 else [self._read() for _ in range(size)]
        raise RedisError(f"unexpected reply {line!r}")

    def call(self, *args):
        self._send(args)
        self.file.flush()
        reply = self._read()
        if isinstance(reply, RedisError):
            raise reply
        return reply

    def pipeline(self, commands):
        """Replies in order; an error reply is returned in place, not raised."""
        replies = []
        for start in range(0, len(commands), 500):
            batch = commands[start:start + 500]
            for command in batch:
                self._send(command)
            self.file.flush()
            replies.extend(self._read() for _ in batch)
        return replies

    def info(self, section):
        out = {}
        for line in self.call("INFO", section).decode("utf-8", "replace").splitlines():
            if ":" in line and not line.startswith("#"):
                key, value = line.split(":", 1)
                out[key] = value
        return out

    def close(self):
        try:
            self.file.close()
            self.sock.close()
        except OSError:
            pass


# --- the facts of a store -------------------------------------------------------


def key_family(key):
    parts = key.split(b":")
    if len(parts) == 1:
        return "(bare)"
    if parts[0] == b"mod" and len(parts) > 2:
        return "mod:" + parts[1].decode("utf-8", "replace")
    return parts[0].decode("utf-8", "replace")


def flat(value):
    """Nested replies as bytes, unambiguously."""
    if value is None:
        return b"n"
    if isinstance(value, int):
        return b"i%d;" % value
    if isinstance(value, bytes):
        return b"b%d:" % len(value) + value
    return b"l%d[" % len(value) + b"".join(flat(v) for v in value) + b"]"


def parse_chunk_key(module_key):
    """(log id, chunk) of a `log:{id}:{chunk}` module key (mush_core::parse_chunk_key)."""
    if not module_key.startswith("log:"):
        return None
    log_id, _, chunk = module_key[4:].rpartition(":")
    if not log_id or not chunk.isdigit() or str(int(chunk)) != chunk:
        return None
    return log_id, int(chunk)


def read_archive_tier(root, problems):
    """Every chunk the ledger archive tier holds, read back and hash-checked
    (crates/mush-store/src/archive.rs): {(ns, log): {chunk: (doc, purged)}}."""
    held, segments = {}, 0
    if not root or not os.path.isdir(root):
        return held, {"segments": 0, "chunks": 0, "purged": 0}
    chunks = purged = 0
    for directory, _, files in sorted(os.walk(root)):
        for name in sorted(files):
            if not name.endswith(".idx.json"):
                continue
            path = os.path.join(directory, name)
            rel = os.path.relpath(path, root)
            try:
                with open(path, encoding="utf-8") as handle:
                    index = json.load(handle)
                ns, log_id = index["ns"], index["log_id"]
            except (OSError, ValueError, KeyError) as error:
                problems.append(f"archive tier: unreadable index {rel}: {error}")
                continue
            segments += 1
            for number, loc in sorted(index.get("chunks", {}).items(), key=lambda kv: int(kv[0])):
                where = f"archive tier: {ns}/{log_id} chunk {number} ({rel})"
                try:
                    with open(os.path.join(directory, loc["segment"]), "rb") as handle:
                        handle.seek(loc["offset"])
                        member = handle.read(loc["len"])
                    raw = zlib.decompress(member, wbits=31)
                except (OSError, zlib.error, KeyError) as error:
                    problems.append(f"{where}: cannot be read back: {error}")
                    continue
                if len(raw) != loc["raw_len"] or hashlib.sha256(raw).hexdigest() != loc["sha256"]:
                    problems.append(f"{where}: its bytes do not hash to what the index recorded")
                    continue
                try:
                    doc = json.loads(raw)
                except ValueError as error:
                    problems.append(f"{where}: not JSON: {error}")
                    continue
                chunks += 1
                purged += 1 if loc.get("purged") else 0
                held.setdefault((ns, log_id), {})[int(number)] = (doc, bool(loc.get("purged")))
    return held, {"segments": segments, "chunks": chunks, "purged": purged}


def ledger_facts(store_chunks, archive_chunks, problems, upto=None):
    """Each ledger across both tiers: whole, in order, the same where both hold
    a chunk. `upto` ({ledger: last_seq}) limits each digest to a prefix."""
    ledgers = {}
    for ledger in sorted(set(store_chunks) | set(archive_chunks)):
        ns, log_id = ledger
        name = f"{ns}/{log_id}"
        in_store = store_chunks.get(ledger, {})
        in_archive = archive_chunks.get(ledger, {})
        numbers = sorted(set(in_store) | set(in_archive))
        digest = hashlib.sha256()
        entries = 0
        first_seq = last_seq = None
        limit = None if upto is None else upto.get(name)
        archive_only = 0
        for position, number in enumerate(numbers):
            if position and number != numbers[position - 1] + 1:
                problems.append(f"ledger {name}: chunks {numbers[position - 1] + 1} to {number - 1} are in neither "
                                "the store nor the archive tier")
            doc = in_store.get(number)
            if number in in_archive:
                archived = in_archive[number][0]
                if doc is None:
                    doc = archived
                    archive_only += 1
                elif canonical(doc) != canonical(archived):
                    problems.append(f"ledger {name}: chunk {number} differs between the store and the archive tier")
            if not isinstance(doc, dict) or not isinstance(doc.get("entries"), list):
                problems.append(f"ledger {name}: chunk {number} is not a chunk document")
                continue
            if doc.get("chunk") != number:
                problems.append(f"ledger {name}: the document at chunk {number} says it is chunk {doc.get('chunk')}")
            if number != numbers[-1] and not doc.get("sealed"):
                problems.append(f"ledger {name}: chunk {number} is not sealed and is not the tail")
            for entry in doc["entries"]:
                seq = entry.get("seq") if isinstance(entry, dict) else None
                if not isinstance(seq, int):
                    problems.append(f"ledger {name}: chunk {number} holds an entry with no seq")
                    continue
                if last_seq is not None and seq != last_seq + 1:
                    problems.append(f"ledger {name}: seq {last_seq} is followed by {seq} (chunk {number})")
                if first_seq is None:
                    first_seq = seq
                last_seq = seq
                if limit is None or seq <= limit:
                    digest.update(canonical(entry))
                    digest.update(b"\n")
                    entries += 1
        tail = in_store.get(numbers[-1]) or in_archive.get(numbers[-1], (None,))[0] or {}
        ledgers[name] = {
            "chunks": len(numbers),
            "chunks_in_store": len(in_store),
            "chunks_in_archive": len(in_archive),
            "chunks_only_in_archive": archive_only,
            "first_chunk": numbers[0],
            "last_chunk": numbers[-1],
            "tail_sealed": bool(tail.get("sealed")),
            "first_seq": first_seq,
            "last_seq": last_seq,
            "entries": entries,
            "digest": digest.hexdigest(),
        }
    return ledgers


def amount(value):
    """A money figure as the store spells it: an integer, digits, or one field."""
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, str) and re.fullmatch(r"-?\d+", value):
        return int(value)
    if isinstance(value, dict) and len(value) == 1:
        return amount(next(iter(value.values())))
    return None


def collect_facts(redis, data_dir, upto=None):
    """The facts of one store and the data directory beside it. Persistent keys
    are digested by family; keys with a time to live (web tokens, tempbans,
    link codes) are only counted, because they leave by the wall clock."""
    problems = []
    keys, cursor = [], b"0"
    while True:
        cursor, batch = redis.call("SCAN", cursor, "COUNT", 2000)
        keys.extend(batch)
        if cursor == b"0":
            break
    keys = sorted(set(keys))
    types = redis.pipeline([("TYPE", k) for k in keys])
    ttls = redis.pipeline([("PTTL", k) for k in keys])
    readers = {b"string": lambda k: ("GET", k), b"ReJSON-RL": lambda k: ("JSON.GET", k),
               b"stream": lambda k: ("XRANGE", k, "-", "+"), b"hash": lambda k: ("HGETALL", k),
               b"set": lambda k: ("SMEMBERS", k), b"zset": lambda k: ("ZRANGE", k, 0, -1, "WITHSCORES"),
               b"list": lambda k: ("LRANGE", k, 0, -1)}
    values = redis.pipeline([readers.get(t, lambda k: ("DUMP", k))(k) for k, t in zip(keys, types)])
    families, docs = {}, {}
    volatile = 0
    for key, kind, ttl, value in zip(keys, types, ttls, values):
        family = families.setdefault(key_family(key), {"keys": 0, "volatile": 0, "_h": hashlib.sha256()})
        if isinstance(value, RedisError) or isinstance(kind, RedisError):
            problems.append(f"key {key!r}: cannot be read: {value}")
            continue
        if isinstance(ttl, int) and ttl != -1:
            family["volatile"] += 1
            volatile += 1
            continue
        family["keys"] += 1
        text = None
        if kind == b"ReJSON-RL" or (kind == b"string" and key.startswith(b"mod:") and value[:1] in (b"{", b"[")):
            try:
                parsed = json.loads(value)
                text = canonical(parsed)
                docs[key.decode("utf-8", "replace")] = parsed
            except ValueError:
                text = None
        if text is None:
            if kind == b"hash":
                value = sorted(zip(value[0::2], value[1::2]))
                value = [part for pair in value for part in pair]
            elif kind == b"set":
                value = sorted(value)
            text = flat(value)
            if kind == b"string" and key.startswith(b"meta:"):
                docs[key.decode("utf-8", "replace")] = value.decode("utf-8", "replace")
        if kind not in readers:
            family.setdefault("opaque", 0)
            family["opaque"] += 1
        family["_h"].update(flat([key, kind, text]))
    whole = hashlib.sha256()
    for name in sorted(families):
        families[name]["digest"] = families[name].pop("_h").hexdigest()
        whole.update(flat([name.encode(), families[name]["keys"], families[name]["digest"].encode()]))

    # The ledgers, across the store and the archive tier.
    store_chunks = {}
    for key, doc in docs.items():
        if not key.startswith("mod:"):
            continue
        ns, _, module_key = key[4:].partition(":")
        parsed = parse_chunk_key(module_key)
        if parsed:
            store_chunks.setdefault((ns, parsed[0]), {})[parsed[1]] = doc
    archive_dir = os.path.join(data_dir, "archive") if data_dir else None
    archive_chunks, archive_stats = read_archive_tier(archive_dir, problems)
    ledgers = ledger_facts(store_chunks, {k: v for k, v in archive_chunks.items()}, problems, upto)

    # The economy: every instrument's posted balances sum to zero (the boot's
    # own conservation tripwire, crates/mush-economy/src/boot.rs).
    instruments = {}
    for key, doc in docs.items():
        if key.startswith("mod:economy:account:") and isinstance(doc, dict):
            figure = amount(doc.get("posted_balance"))
            name = doc.get("instrument")
            name = name if isinstance(name, str) else json.dumps(name, sort_keys=True)
            row = instruments.setdefault(name, {"accounts": 0, "sum_posted": 0})
            row["accounts"] += 1
            if figure is None:
                problems.append(f"economy: {key} has no readable posted_balance")
            else:
                row["sum_posted"] += figure
    for name, row in sorted(instruments.items()):
        if row["sum_posted"] != 0:
            problems.append(f"economy: instrument {name} does not conserve: its posted balances sum to "
                            f"{row['sum_posted']}, not 0")

    # The IC clock (crates/mush-core/src/clock.rs).
    clock = {}
    state = docs.get("mod:rustmush_kernel_time:world_ic/state")
    if isinstance(state, dict):
        clock["checkpoint"] = state.get("checkpoint")
        clock["freeze"] = state.get("freeze")
        clock["reanchors"] = len(state.get("reanchors") or [])
    head = docs.get("mod:rustmush_kernel_time:world_ic/head")
    clock["head"] = head
    generation = (clock.get("checkpoint") or {}).get("generation")
    if generation is not None:
        clock["epoch"] = docs.get(f"mod:rustmush_kernel_time:world_ic/epoch/{generation:020d}")

    def count(prefix):
        return sum(1 for k in keys if k.startswith(prefix))

    def integer(name):
        try:
            return int(docs.get(name))
        except (TypeError, ValueError):
            return None

    quarantine = os.path.join(data_dir, "quarantine") if data_dir else None
    records = 0
    if quarantine and os.path.isdir(quarantine):
        records = sum(1 for name in os.listdir(quarantine) if name.endswith(".json"))
    return {
        "keys": len(keys),
        "persistent_keys": len(keys) - volatile,
        "volatile_keys": volatile,
        "store_digest": whole.hexdigest(),
        "families": families,
        "objects": count(b"obj:"),
        "accounts": count(b"account:"),
        "next_id": integer("meta:next_id"),
        "schema_version": integer("meta:schema_version"),
        "authoritative": {"intents": count(b"auth:intent:"), "receipts": count(b"auth:receipt:"),
                          "outbox": count(b"auth:outbox:")},
        "ledgers": ledgers,
        "archive_tier": archive_stats,
        "economy": {"accounts": sum(r["accounts"] for r in instruments.values()), "instruments": instruments},
        "clock": clock,
        "quarantine_records": records,
        "problems": problems,
    }


def compare_facts(want, got, what):
    """Every way `got` differs from `want`, in words."""
    out = []
    for key in ("persistent_keys", "store_digest", "objects", "accounts", "next_id", "schema_version",
                "authoritative", "archive_tier", "economy", "clock", "quarantine_records"):
        if want.get(key) != got.get(key):
            out.append(f"{what}: {key} is {json.dumps(got.get(key), sort_keys=True)[:200]}, "
                       f"the manifest says {json.dumps(want.get(key), sort_keys=True)[:200]}")
    if got.get("volatile_keys", 0) > want.get("volatile_keys", 0):
        out.append(f"{what}: {got['volatile_keys']} keys with a time to live, more than the manifest's "
                   f"{want['volatile_keys']}")
    for name in sorted(set(want["families"]) | set(got["families"])):
        a, b = want["families"].get(name), got["families"].get(name)
        if a is None or b is None:
            if (a or b)["keys"]:
                out.append(f"{what}: key family {name} is {'missing' if b is None else 'new'}")
        elif (a["keys"], a["digest"]) != (b["keys"], b["digest"]):
            out.append(f"{what}: key family {name} differs ({b['keys']} keys, the manifest says {a['keys']}; "
                       "its digest does not match)" if a["keys"] != b["keys"] else
                       f"{what}: key family {name} has the manifest's {a['keys']} keys and a different digest")
    for name in sorted(set(want["ledgers"]) | set(got["ledgers"])):
        if want["ledgers"].get(name) != got["ledgers"].get(name):
            out.append(f"{what}: ledger {name} is {json.dumps(got['ledgers'].get(name), sort_keys=True)}, "
                       f"the manifest says {json.dumps(want['ledgers'].get(name), sort_keys=True)}")
    return out


# --- a scratch Redis that holds one dump.rdb -------------------------------------


def loadmodule_args(conf_path):
    """The `loadmodule` lines of a Redis config, as command-line arguments."""
    args = []
    if conf_path and os.path.isfile(conf_path):
        with open(conf_path, encoding="utf-8") as handle:
            for line in handle:
                parts = line.split()
                if len(parts) >= 2 and parts[0] == "loadmodule":
                    if not os.path.isfile(parts[1]):
                        raise Fail(f"{conf_path} loads the module {parts[1]}, which this box does not have")
                    args += ["--loadmodule"] + parts[1:]
    return args


class ScratchRedis:
    """A redis-server over a copy of one dump.rdb: its own directory, a Unix
    socket, no TCP port, no persistence. As the `redis` user when run as root."""

    def __init__(self, rdb, modules):
        # Its own 0700 directory under the temp directory (the timer's unit has
        # a private one): the redis user can reach it there, and nowhere else.
        self.dir = tempfile.mkdtemp(prefix="rmbak-scratch-")
        shutil.copyfile(rdb, os.path.join(self.dir, "dump.rdb"))
        self.sock = os.path.join(self.dir, "s.sock")
        if len(self.sock.encode()) > 100:
            shutil.rmtree(self.dir, ignore_errors=True)
            raise Fail(f"the scratch socket path is too long for a Unix socket: {self.sock}")
        command = ["redis-server", "--port", "0", "--unixsocket", self.sock, "--unixsocketperm", "600",
                   "--dir", self.dir, "--dbfilename", "dump.rdb", "--appendonly", "no", "--save", "",
                   "--daemonize", "no", "--logfile", os.path.join(self.dir, "redis.log")] + modules
        if is_root() and user_exists("redis"):
            shutil.chown(self.dir, "redis", "redis")
            shutil.chown(os.path.join(self.dir, "dump.rdb"), "redis", "redis")
            command = ["runuser", "-u", "redis", "--"] + command
        self.proc = subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.client = None

    def __enter__(self):
        end = time.monotonic() + 600
        while True:
            if self.proc.poll() is not None:
                raise Fail(f"the scratch Redis exited {self.proc.returncode} loading the snapshot:\n{self.log()}")
            try:
                self.client = Redis(unix=self.sock)
                self.client.call("PING")
                return self.client
            except (OSError, RedisError):
                if self.client:
                    self.client.close()
                    self.client = None
                if time.monotonic() > end:
                    raise Fail(f"the scratch Redis did not load the snapshot in 600 s:\n{self.log()}") from None
                time.sleep(0.2)

    def log(self):
        try:
            with open(os.path.join(self.dir, "redis.log"), encoding="utf-8", errors="replace") as handle:
                return handle.read()[-1500:]
        except OSError:
            return "(no log)"

    def __exit__(self, *_):
        try:
            if self.client:
                try:
                    self.client.call("SHUTDOWN", "NOSAVE")
                except (OSError, RedisError):
                    pass
                self.client.close()
            self.proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
        shutil.rmtree(self.dir, ignore_errors=True)


def facts_of_rdb(rdb, data_dir, modules, upto=None):
    with ScratchRedis(rdb, modules) as client:
        facts = collect_facts(client, data_dir, upto)
        facts["loaded_by_redis"] = client.info("server").get("redis_version")
        return facts


# --- the archive: write, read, verify --------------------------------------------


def compressor():
    return "zst" if shutil.which("zstd") else "gz"


def open_archive_reader(path):
    """(tarfile in stream mode, closer) for an archive, by its first bytes."""
    with open(path, "rb") as handle:
        magic = handle.read(4)
    if magic == b"\x28\xb5\x2f\xfd":
        if not shutil.which("zstd"):
            raise Fail("this archive is zstd-compressed and this box has no zstd")
        proc = subprocess.Popen(["zstd", "-dc", "--", path], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)

        def abort():
            proc.kill()
            proc.stdout.close()
            proc.wait()
        try:
            tar = tarfile.open(fileobj=proc.stdout, mode="r|")
        except BaseException as error:
            abort()
            raise Fail(f"{os.path.basename(path)}: the archive cannot be read: {error}") from None

        def close():
            tar.close()
            rest = proc.stdout.read()
            proc.stdout.close()
            if proc.wait() != 0 or rest.strip(b"\0"):
                raise Fail(f"{os.path.basename(path)}: the compressed stream is damaged (zstd exited {proc.returncode})")
        return tar, close, abort
    if magic[:2] == b"\x1f\x8b":
        raw = gzip.open(path, "rb")
        try:
            tar = tarfile.open(fileobj=raw, mode="r|")
        except BaseException as error:
            raw.close()
            raise Fail(f"{os.path.basename(path)}: the archive cannot be read: {error}") from None

        def close():
            tar.close()
            raw.read()
            raw.close()
        return tar, close, raw.close
    raise Fail(f"{os.path.basename(path)}: neither a zstd nor a gzip stream")


def verify_archive(path, extract_to=None):
    """Read the whole archive once: the manifest first, every listed member
    present once with its size and SHA-256, nothing unlisted, the stream sound.
    With `extract_to`, the members are written there as they are checked."""
    name = os.path.basename(path)
    if not NAME_RE.match(name):
        raise Fail(f"{name}: not an archive name (<instance>-<UTC>-<commit8>.rmbak.tar.zst)")
    tar, close, abort = open_archive_reader(path)
    try:
        manifest, manifest_bytes, listed, seen, sealed = None, None, {}, set(), False
        for member in tar:
            if manifest is None:
                if member.name != "manifest.json" or not member.isfile():
                    raise Fail(f"{name}: the first member is {member.name!r}, not manifest.json")
                manifest_bytes = tar.extractfile(member).read()
                manifest = json.loads(manifest_bytes)
                if manifest.get("format") != FORMAT:
                    raise Fail(f"{name}: format {manifest.get('format')!r}, this tool reads {FORMAT}")
                listed = {entry["path"]: entry for entry in manifest["files"]}
                continue
            if sealed:
                raise Fail(f"{name}: {member.name!r} follows manifest.sha256")
            if member.name == "manifest.sha256":
                sealed = True
                recorded = tar.extractfile(member).read().decode("ascii", "replace").split()[0]
                if recorded != hashlib.sha256(manifest_bytes).hexdigest():
                    raise Fail(f"{name}: manifest.json does not hash to what manifest.sha256 recorded")
                continue
            entry = listed.get(member.name)
            if entry is None:
                raise Fail(f"{name}: holds {member.name!r}, which the manifest does not list")
            if not member.isfile():
                raise Fail(f"{name}: {member.name!r} is not a regular file")
            parts = member.name.split("/")
            if member.name.startswith("/") or ".." in parts or "" in parts:
                raise Fail(f"{name}: unsafe member path {member.name!r}")
            if member.name in seen:
                raise Fail(f"{name}: holds {member.name!r} twice")
            seen.add(member.name)
            source = tar.extractfile(member)
            digest, size, out = hashlib.sha256(), 0, None
            if extract_to:
                target = os.path.join(extract_to, *parts)
                os.makedirs(os.path.dirname(target), mode=0o700, exist_ok=True)
                out = open(os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600), "wb")
            for block in iter(lambda: source.read(1 << 20), b""):
                digest.update(block)
                size += len(block)
                if out:
                    out.write(block)
            if out:
                out.close()
            if size != entry["size"] or digest.hexdigest() != entry["sha256"]:
                raise Fail(f"{name}: {member.name} does not match its manifest entry "
                           f"({size} bytes, sha256 {digest.hexdigest()[:16]}…; the manifest says "
                           f"{entry['size']} bytes, {entry['sha256'][:16]}…)")
        close()
    except (tarfile.TarError, EOFError, OSError, zlib.error, ValueError, KeyError) as error:
        abort()
        raise Fail(f"{name}: the archive cannot be read: {error}") from None
    except Fail:
        abort()
        raise
    if manifest is None:
        raise Fail(f"{name}: empty archive")
    if not sealed:
        raise Fail(f"{name}: ends without manifest.sha256 (cut short)")
    missing = sorted(set(listed) - seen)
    if missing:
        raise Fail(f"{name}: {len(missing)} listed member(s) are missing, the first {missing[0]!r}")
    stamp = NAME_RE.match(name)
    if manifest["instance"] != stamp.group(1) or not manifest["commit"].startswith(stamp.group(3)):
        raise Fail(f"{name}: the name does not match its manifest ({manifest['instance']}, {manifest['commit'][:8]})")
    return manifest


def write_archive(stage, manifest, final_path):
    """Write the archive beside its final name and move it there only when it
    is complete, synced and has been read back whole."""
    directory = os.path.dirname(final_path)
    partial = os.path.join(directory, f".partial-{os.path.basename(final_path)}.{os.getpid()}")
    manifest_bytes = json.dumps(manifest, indent=1, sort_keys=True).encode("utf-8") + b"\n"
    fd = os.open(partial, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(fd, "wb") as out:
            if final_path.endswith(".zst"):
                proc = subprocess.Popen(["zstd", "-q", "-6", "-T2", "--check", "-c"], stdin=subprocess.PIPE, stdout=out)
                sink = proc.stdin
            else:
                proc = None
                sink = gzip.GzipFile(fileobj=out, mode="wb", compresslevel=6, mtime=0)
            with tarfile.open(fileobj=sink, mode="w|", format=tarfile.PAX_FORMAT) as tar:
                def add(name, data=None, source=None, mode=0o600):
                    info = tarfile.TarInfo(name)
                    info.mode, info.mtime, info.uname, info.gname = mode, int(manifest["created_unix"]), "root", "root"
                    if data is not None:
                        info.size = len(data)
                        tar.addfile(info, io.BytesIO(data))
                    else:
                        info.size = os.path.getsize(source)
                        with open(source, "rb") as handle:
                            tar.addfile(info, handle)
                add("manifest.json", data=manifest_bytes)
                for entry in manifest["files"]:
                    add(entry["path"], source=os.path.join(stage, entry["path"]), mode=int(entry["mode"], 8))
                add("manifest.sha256", data=f"{hashlib.sha256(manifest_bytes).hexdigest()}  manifest.json\n".encode())
            sink.close()
            if proc and proc.wait() != 0:
                raise Fail(f"zstd exited {proc.returncode} writing the archive")
            out.flush()
            os.fsync(out.fileno())
        verify_archive_as(partial, os.path.basename(final_path))
        os.rename(partial, final_path)
        fsync_dir(directory)
    except BaseException:
        try:
            os.unlink(partial)
        except OSError:
            pass
        raise


def verify_archive_as(path, name, deep=False):
    """Verify a file that does not carry its final name yet."""
    link = os.path.join(os.path.dirname(path), f".verify-{os.getpid()}", name)
    os.makedirs(os.path.dirname(link), mode=0o700)
    try:
        os.link(path, link)
        if not deep:
            return verify_archive(link)
        manifest, _, differences = deep_verify(link, os.path.dirname(link))
        if differences:
            raise Fail(f"{name}: the snapshot does not match its manifest: " + "; ".join(differences[:5]))
        return manifest
    finally:
        shutil.rmtree(os.path.dirname(link), ignore_errors=True)


def deep_verify(path, workdir=None):
    """Checksums, then the snapshot loaded into a scratch Redis and its facts
    derived again: (manifest, facts, every difference and broken invariant)."""
    stage = tempfile.mkdtemp(prefix="rmbak-verify-", dir=workdir)
    try:
        manifest = verify_archive(path, extract_to=stage)
        conf = os.path.join(stage, "config", f"redis-{manifest['instance']}mush.conf")
        facts = facts_of_rdb(os.path.join(stage, "redis", "dump.rdb"), os.path.join(stage, "data"),
                             loadmodule_args(conf))
    finally:
        shutil.rmtree(stage, ignore_errors=True)
    differences = compare_facts(manifest["facts"], facts, "the loaded snapshot") + \
        [f"the loaded snapshot: {p}" for p in facts["problems"]]
    return manifest, facts, differences


# --- the cut: what is copied, and whether it held still --------------------------


def data_walk(data_dir):
    """Every entry under the data directory, opened by directory descriptor so a
    swapped path component is never followed: (relative path, stat, dir fd, name)."""
    for directory, dirs, files, dir_fd in os.fwalk(data_dir, follow_symlinks=False):
        rel_dir = os.path.relpath(directory, data_dir)
        rel_dir = "" if rel_dir == "." else rel_dir + "/"
        dirs.sort()
        for name in sorted(files):
            try:
                info = os.stat(name, dir_fd=dir_fd, follow_symlinks=False)
            except FileNotFoundError:
                continue
            yield rel_dir + name, info, dir_fd, name


def excluded(rel):
    """Not part of what stands the instance back up."""
    top = rel.split("/", 1)[0]
    if top == "logs":
        return True
    if top == "bringup":
        return rel != "bringup/state.json"
    base = rel.rsplit("/", 1)[-1]
    return base.endswith(".lock") or base.endswith(".probe")


def is_secret(rel):
    return any(rel == p or (p.endswith("/") and rel.startswith(p)) for p in SECRET_PATHS)


def pinned_state(data_dir):
    """What the store refers to on disk, as (path, size, mtime, inode): the cut
    is kept only if this reads the same after the copy as before the fork."""
    state = []
    for rel, info, _, _ in data_walk(data_dir):
        if not excluded(rel):
            state.append((rel, info.st_size, info.st_mtime_ns, info.st_ino, stat.S_IFMT(info.st_mode)))
    return state


def copy_data(data_dir, stage, secrets_mode, manifest):
    """Copy the data directory into the stage. A temp file of a writer in
    flight is left out of the copy and stays in the pinned state."""
    target_root = os.path.join(stage, "data")
    dirs = []
    for directory, subdirs, _ in os.walk(data_dir, followlinks=False):
        subdirs.sort()
        rel = os.path.relpath(directory, data_dir)
        if rel != "." and not excluded(rel + "/x") and rel.split("/", 1)[0] not in ("logs", "bringup"):
            info = os.lstat(directory)
            dirs.append({"path": "data/" + rel, "mode": f"{stat.S_IMODE(info.st_mode):04o}"})
    apart, skipped = [], []
    for rel, info, dir_fd, name in data_walk(data_dir):
        if excluded(rel):
            continue
        if not stat.S_ISREG(info.st_mode):
            skipped.append({"path": "data/" + rel, "why": "not a regular file"})
            continue
        if rel.endswith(".tmp"):
            skipped.append({"path": "data/" + rel, "why": "a writer's temp file"})
            continue
        fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=dir_fd)
        with os.fdopen(fd, "rb") as source:
            if secrets_mode == "apart" and is_secret(rel):
                digest = hashlib.sha256(source.read()).hexdigest()
                apart.append({"path": "data/" + rel, "size": info.st_size, "sha256": digest,
                              "mode": f"{stat.S_IMODE(info.st_mode):04o}"})
                continue
            target = os.path.join(target_root, rel)
            os.makedirs(os.path.dirname(target), mode=0o700, exist_ok=True)
            with open(os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as out:
                shutil.copyfileobj(source, out, 1 << 20)
        os.chmod(target, stat.S_IMODE(info.st_mode))
    manifest["dirs"] = dirs
    manifest["skipped"] = skipped
    # The field is named for the files, and holds their names and digests only.
    manifest["key_files"] = {"mode": secrets_mode, "kept_apart": apart}


def snapshot_store(env, stage):
    """One Redis fork into the stage, as a replica would take it: the store's
    own dump.rdb and save points are not touched."""
    os.makedirs(os.path.join(stage, "redis"), mode=0o700, exist_ok=True)
    target = os.path.join(stage, "redis", "dump.rdb")
    if os.path.exists(target):
        os.unlink(target)
    url = env.get("REDIS_URL") or f"redis://127.0.0.1:{env.get('REDIS_PORT', '6379')}"
    before = time.time()
    done = subprocess.run(["redis-cli"] + redis_cli_args(url) + ["--rdb", target], capture_output=True, text=True)
    after = time.time()
    out = done.stdout + done.stderr
    if done.returncode != 0 or "Transfer finished with success" not in out or not os.path.getsize(target):
        raise Fail(f"redis-cli --rdb did not finish a transfer (exit {done.returncode}): {out.strip()[-300:]}")
    created = None
    try:
        found = re.search(r"AUX FIELD ctime = '(\d+)'", run(["redis-check-rdb", target]))
        created = int(found.group(1)) if found else None
    except (Fail, OSError):
        pass
    return {"taken_between_unix": [round(before, 3), round(after, 3)], "rdb_ctime_unix": created}


def running_exe(unit):
    """The executable a running unit's main process was started from."""
    try:
        pid = subprocess.run(["systemctl", "show", "-p", "MainPID", "--value", unit],
                             capture_output=True, text=True).stdout.strip()
        if pid.isdigit() and int(pid) > 0:
            return f"/proc/{pid}/exe"
    except OSError:
        pass
    return None


def unit_active(unit):
    try:
        return subprocess.run(["systemctl", "is-active", "--quiet", unit]).returncode == 0
    except OSError:
        return False


def stage_file(stage, rel, source, mode=None):
    target = os.path.join(stage, rel)
    os.makedirs(os.path.dirname(target), mode=0o700, exist_ok=True)
    shutil.copyfile(source, target, follow_symlinks=False)
    os.chmod(target, mode if mode is not None else stat.S_IMODE(os.lstat(source).st_mode))


def stage_tree(stage, rel, source):
    for directory, dirs, files in os.walk(source, followlinks=False):
        dirs.sort()
        for name in sorted(files):
            path = os.path.join(directory, name)
            if os.path.isfile(path) and not os.path.islink(path):
                stage_file(stage, os.path.join(rel, os.path.relpath(path, source)), path)


def do_backup(instance, conf, dest=None):
    """One archive of one instance. Returns its path."""
    for tool in ("redis-cli", "redis-server"):
        if not shutil.which(tool):
            raise Fail(f"{tool} is not on PATH")
    layout = Layout(instance)
    env = layout.env()
    data_dir, build = env["DATA_DIR"], env["BUILD_DIR"]
    if layout.sysroot:
        data_dir, build = layout.sysroot + data_dir, layout.sysroot + build
    if os.path.islink(data_dir) or not os.path.isdir(data_dir):
        raise Fail(f"{data_dir} is not a directory")
    try:
        with open(os.path.join(build, ".source-commit"), encoding="ascii") as handle:
            commit = handle.read().strip()
    except OSError:
        raise Fail(f"{build}/.source-commit is missing: which commit does {instance} run?") from None
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise Fail(f"{build}/.source-commit is not a commit: {commit[:60]!r}")
    dest = dest or conf["BACKUP_DIR"]
    os.makedirs(dest, mode=0o750, exist_ok=True)
    started = time.time()
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime(started))
    final = os.path.join(dest, f"{instance}-{stamp}-{commit[:8]}.rmbak.tar.{compressor()}")
    lock = open(os.path.join(dest, ".lock"), "w")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        lock.close()
        raise Fail(f"another backup holds {dest}/.lock: one backup at a time on a box") from None
    stage = tempfile.mkdtemp(prefix=".stage-", dir=dest)
    try:
        manifest = {"format": FORMAT, "instance": instance, "host": socket.gethostname(), "commit": commit,
                    "created_unix": int(started), "created_utc": utc(started)}
        # The cut: the fork, then the files it refers to, kept only if they held still.
        for attempt in range(1, CUT_ATTEMPTS + 1):
            shutil.rmtree(os.path.join(stage, "data"), ignore_errors=True)
            before = pinned_state(data_dir)
            snapshot = snapshot_store(env, stage)
            copy_data(data_dir, stage, conf["SECRETS"], manifest)
            if pinned_state(data_dir) == before:
                break
            say(f"{instance}: the files beside the store changed during the cut (attempt {attempt}); "
                "taking it again")
            time.sleep(10)
        else:
            raise Fail(f"{instance}: the files beside the store did not hold still for one cut in {CUT_ATTEMPTS} "
                       "attempts (is a ledger archive pass or a bring-up running?)")
        manifest["snapshot"] = dict(snapshot, cut_attempts=attempt, pinned_files=len(before),
                                    pinned_digest=hashlib.sha256(repr(before).encode()).hexdigest())

        # The configuration, as installed.
        stage_file(stage, "config/instance.env", layout.env_file, 0o644)
        if os.path.isfile(layout.config):
            stage_file(stage, "config/mushd.toml", layout.config, 0o644)
        for name, path in layout.unit_files():
            stage_file(stage, f"config/units/{name}", path, 0o644)
        if os.path.isfile(layout.redis_conf):
            stage_file(stage, f"config/{os.path.basename(layout.redis_conf)}", layout.redis_conf, 0o640)

        # The build: its stamps, the binaries, and what the server reads beside them.
        stage_file(stage, "build/.source-commit", os.path.join(build, ".source-commit"), 0o644)
        # The bundle's stamp and its index page are what a deploy's guard reads
        # (scripts/check-web-bundle.sh); the client itself is inside the gateway.
        for name in (".commit", "index.html"):
            bundle = os.path.join(build, "web", "dist", name)
            if os.path.isfile(bundle):
                stage_file(stage, f"build/web/dist/{name}", bundle, 0o644)
        for tree in ("docs/help", "deploy"):
            if os.path.isdir(os.path.join(build, tree)):
                stage_tree(stage, f"build/{tree}", os.path.join(build, tree))
        # The scripts a deploy and a reset run: with these, `rmbak.py tree` lays
        # out everything deploy.sh needs on a box that has lost its tree.
        scripts = os.path.join(build, "scripts")
        if os.path.isdir(scripts):
            for name in sorted(os.listdir(scripts)):
                path = os.path.join(scripts, name)
                if name.endswith(".sh") and os.path.isfile(path) and not os.path.islink(path):
                    stage_file(stage, f"build/scripts/{name}", path, 0o755)
        binaries = {}
        for name, unit in (("mush-server", layout.server_unit), ("mush-gateway", None)):
            path = os.path.join(build, "target", "release", name)
            if not os.path.isfile(path):
                raise Fail(f"{path} is missing")
            row = {"sha256": sha256_file(path), "size": os.path.getsize(path), "in_archive": conf["BINARIES"] == "yes"}
            units = [unit] if unit else [u for u in layout.gateway_units if unit_active(u)]
            for candidate in units:
                exe = running_exe(candidate)
                if exe and os.path.exists(exe):
                    row.setdefault("running", {})[candidate] = sha256_file(exe) == row["sha256"]
            binaries[name] = row
            if conf["BINARIES"] == "yes":
                stage_file(stage, f"build/target/release/{name}", path, 0o755)
        manifest["binaries"] = binaries
        manifest["live_errors"] = live_error_signatures(layout)
        manifest["never_archived"] = [
            f"/opt/rustmush/{instance}/{instance}.env (provider keys; the units' EnvironmentFile)",
            f"/opt/rustmush/{instance}/tls/ (regenerated by the deploy; unused behind a proxy)",
            f"{env['DATA_DIR']}/logs/ (the journal holds the same lines)",
            "the staff password file (the account's hash is in the store)",
        ]

        # The store this box runs, and the facts of the snapshot itself.
        modules = loadmodule_args(layout.redis_conf)
        try:
            manifest["redis"] = {"version": run(["redis-server", "--version"]).split("v=", 1)[1].split()[0],
                                 "modules": [m for m in modules if m != "--loadmodule"]}
        except (Fail, IndexError):
            manifest["redis"] = {"version": None, "modules": [m for m in modules if m != "--loadmodule"]}
        facts = facts_of_rdb(os.path.join(stage, "redis", "dump.rdb"), os.path.join(stage, "data"), modules)
        if facts["quarantine_records"]:
            # docs/ops.md, "Deterministic failure": a snapshot taken during an integrity
            # incident is evidence, and must never stand in for the last known-good backup.
            facts["problems"].append(
                f"the quarantine spool holds {facts['quarantine_records']} unresolved record(s): the instance is in "
                "an integrity incident, and this archive is its evidence, not a known-good backup")
        manifest["facts"] = facts
        checkpoint = (facts["clock"] or {}).get("checkpoint") or {}
        if checkpoint:
            manifest["snapshot"]["ic_checkpoint_unix"] = checkpoint.get("ic_unix")
            manifest["snapshot"]["ic_checkpoint"] = ic_date(checkpoint["ic_unix"])
            manifest["snapshot"]["checkpoint_real_unix"] = checkpoint.get("real_unix")

        files = []
        for directory, dirs, names in os.walk(stage):
            dirs.sort()
            for name in sorted(names):
                path = os.path.join(directory, name)
                rel = os.path.relpath(path, stage)
                files.append({"path": rel, "size": os.path.getsize(path), "sha256": sha256_file(path),
                              "mode": f"{stat.S_IMODE(os.stat(path).st_mode):04o}"})
        manifest["files"] = sorted(files, key=lambda entry: entry["path"])
        manifest["took_s"] = round(time.time() - started, 1)
        write_archive(stage, manifest, final)
        try:
            shutil.chown(final, group=conf["READ_GROUP"])
            os.chmod(final, 0o640)
        except (LookupError, PermissionError):
            os.chmod(final, 0o600)
    finally:
        shutil.rmtree(stage, ignore_errors=True)
        lock.close()
    size = os.path.getsize(final)
    say(f"{instance}: wrote {final} ({size / 1e6:.1f} MB, {len(manifest['files'])} files, "
        f"{facts['keys']} keys, {facts['accounts']} accounts, {len(facts['ledgers'])} ledgers, "
        f"IC {manifest['snapshot'].get('ic_checkpoint', 'unknown')}, {time.time() - started:.1f} s)")
    if facts["problems"]:
        for problem in facts["problems"]:
            say(f"{instance}: PROBLEM in the snapshot: {problem}")
        raise Fail(f"{instance}: the archive was written, and the snapshot it holds breaks "
                   f"{len(facts['problems'])} invariant(s), the live instance needs a look: {facts['problems'][0]}")
    return final


def archives_in(directory, instance=None):
    """Archive names in a directory, oldest first (the stamp sorts)."""
    names = []
    if os.path.isdir(directory):
        for name in os.listdir(directory):
            found = NAME_RE.match(name)
            if found and (instance is None or found.group(1) == instance):
                names.append(name)
    return sorted(names, key=lambda n: (NAME_RE.match(n).group(2), n))


def kept_names(names, keep, daily_days=0):
    """Which of one instance's archives stay (doc 52, Q-52-2): the newest
    `keep`, and beyond those ONE a UTC day, the day's LAST archive, for the
    `daily_days` days up to the newest archive's own day. The days are counted
    from the newest archive held, not from today, so a backup that has stopped
    never ages the history it left away."""
    names = sorted(names, key=lambda n: (NAME_RE.match(n).group(2), n))
    kept = set(names[-keep:])
    if daily_days and names:
        def day(name):
            return calendar.timegm(time.strptime(NAME_RE.match(name).group(2)[:8], "%Y%m%d")) // 86400
        newest_day = day(names[-1])
        last_of_day = {}
        for name in names:
            last_of_day[day(name)] = name  # sorted: the last one seen is the day's last
        kept.update(name for when, name in last_of_day.items() if newest_day - when < daily_days)
    return kept


def prune(directory, instance, keep, daily_days=0):
    names = archives_in(directory, instance)
    kept = kept_names(names, keep, daily_days)
    doomed = [name for name in names if name not in kept]
    for name in doomed:
        os.unlink(os.path.join(directory, name))
    return doomed


def sweep_leftovers(directory):
    """Stages and partial files a killed run left, a day old or more."""
    for name in os.listdir(directory):
        path = os.path.join(directory, name)
        if name.startswith((".stage-", ".partial-", ".verify-")) and time.time() - os.lstat(path).st_mtime > 86400:
            if os.path.isdir(path) and not os.path.islink(path):
                shutil.rmtree(path, ignore_errors=True)
            else:
                os.unlink(path)


def cmd_run(args):
    conf = load_conf(args.conf)
    instances = conf["INSTANCES"].split()
    if not instances:
        raise Fail(f"{args.conf or DEFAULT_CONF} names no INSTANCES")
    dest = conf["BACKUP_DIR"]
    os.makedirs(dest, mode=0o750, exist_ok=True)
    sweep_leftovers(dest)
    status_path = os.path.join(dest, "status.json")
    try:
        with open(status_path, encoding="utf-8") as handle:
            status = json.load(handle)
    except (OSError, ValueError):
        status = {}
    status.setdefault("instances", {})
    failed = []
    # One instance after another: never two forks at once on a box.
    for instance in instances:
        row = status["instances"].setdefault(instance, {})
        row["last_attempt_utc"] = utc()
        try:
            path = do_backup(instance, conf)
            row.update(ok=True, error=None, last_ok_utc=utc(), last_ok_unix=int(time.time()),
                       archive=os.path.basename(path), bytes=os.path.getsize(path))
        except Fail as error:
            row.update(ok=False, error=str(error))
            failed.append(instance)
            shout(f"FAILED: {error}")
        except Exception as error:  # a bug is a failed backup too, and the next instance still runs
            row.update(ok=False, error=f"{type(error).__name__}: {error}")
            failed.append(instance)
            shout(f"FAILED: {instance}: {type(error).__name__}: {error}")
        # Prune only behind a good backup: while an instance's backups fail, nothing
        # older is deleted, so the last known-good archive is never rotated out.
        if row["ok"]:
            removed = prune(dest, instance, int(conf["KEEP"]), int(conf["KEEP_DAILY_DAYS"]))
            if removed:
                say(f"{instance}: pruned {len(removed)} archive(s), keeping the newest {conf['KEEP']}" +
                    (f" and one a day for {conf['KEEP_DAILY_DAYS']} days" if int(conf["KEEP_DAILY_DAYS"]) else ""))
        row["kept"] = len(archives_in(dest, instance))
    # An archive written before the copy's account existed is root's alone: once
    # the group is there, every archive is readable by it, old ones included.
    for name in archives_in(dest):
        try:
            shutil.chown(os.path.join(dest, name), group=conf["READ_GROUP"])
            os.chmod(os.path.join(dest, name), 0o640)
        except (LookupError, PermissionError):
            break
    status["updated_utc"] = utc()
    status["host"] = socket.gethostname()
    write_atomic(status_path, json.dumps(status, indent=1, sort_keys=True) + "\n", 0o640)
    try:
        shutil.chown(status_path, group=conf["READ_GROUP"])
    except (LookupError, PermissionError):
        pass
    if failed:
        raise Fail(f"backup FAILED for: {', '.join(failed)}")


# --- pull -------------------------------------------------------------------------


def cmd_pull(args):
    conf = load_conf(args.conf)
    if not conf["PULL_SSH"]:
        raise Fail("PULL_SSH is not set: the command that reaches backup-serve.sh on the box that takes backups")
    remote = shlex.split(conf["PULL_SSH"])
    dest = conf["PULL_DIR"]
    os.makedirs(dest, mode=0o700, exist_ok=True)
    sweep_leftovers(dest)
    listing = run(remote + ["list"], timeout=120)
    offered = []
    for line in listing.splitlines():
        size, _, name = line.partition(" ")
        if NAME_RE.match(name) and size.isdigit():
            offered.append((name, int(size)))
    keep, daily_days = int(conf["PULL_KEEP"]), int(conf["PULL_KEEP_DAILY_DAYS"])
    wanted = set()
    for instance in sorted({NAME_RE.match(n).group(1) for n, _ in offered}):
        # What this box would keep of everything it holds or is offered: nothing
        # is fetched only to be pruned at the end of the same run.
        known = set(archives_in(dest, instance)) | {n for n, _ in offered if NAME_RE.match(n).group(1) == instance}
        wanted.update(kept_names(known, keep, daily_days))
    fetched = 0
    for name, size in offered:
        if name not in wanted or os.path.exists(os.path.join(dest, name)):
            continue
        partial = os.path.join(dest, f".partial-{name}.{os.getpid()}")
        try:
            with open(os.open(partial, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as out:
                done = subprocess.run(remote + ["fetch", name], stdout=out, stderr=subprocess.PIPE, timeout=3600)
                out.flush()
                os.fsync(out.fileno())
            if done.returncode != 0:
                raise Fail(f"fetching {name} exited {done.returncode}: {done.stderr.decode('utf-8', 'replace')[-300:]}")
            if os.path.getsize(partial) != size:
                raise Fail(f"{name}: fetched {os.path.getsize(partial)} bytes of {size}")
            deep = conf["PULL_DEEP"] == "yes"
            verify_archive_as(partial, name, deep=deep)
            os.rename(partial, os.path.join(dest, name))
            fsync_dir(dest)
            fetched += 1
            say(f"pulled {name} ({size / 1e6:.1f} MB): every member matches its checksum" +
                (", and its snapshot loads to the manifest's facts" if deep else ""))
        finally:
            if os.path.exists(partial):
                os.unlink(partial)
    for instance in sorted({NAME_RE.match(n).group(1) for n in archives_in(dest)}):
        removed = prune(dest, instance, keep, daily_days)
        if removed:
            say(f"{instance}: pruned {len(removed)} pulled archive(s), keeping the newest {keep}" +
                (f" and one a day for {daily_days} days" if daily_days else ""))
    # Loud when the other side is failing or has gone quiet.
    problems = []
    try:
        status = json.loads(run(remote + ["status"], timeout=120) or "{}")
    except ValueError:
        status = {}
        problems.append("the far side's status.json is not JSON")
    write_atomic(os.path.join(dest, "status.json"), json.dumps(status, indent=1, sort_keys=True) + "\n")
    for instance, row in sorted(status.get("instances", {}).items()):
        if not row.get("ok"):
            problems.append(f"{instance}: the last backup FAILED on the far side: {row.get('error')}")
    limit = int(conf["MAX_AGE_HOURS"]) * 3600
    instances = sorted(set(status.get("instances", {})) | {NAME_RE.match(n).group(1) for n in archives_in(dest)})
    for instance in instances:
        have = archives_in(dest, instance)
        if not have:
            problems.append(f"{instance}: no archive here")
            continue
        age = archive_age_s(have[-1])
        if age > limit:
            problems.append(f"{instance}: the newest archive here, {have[-1]}, is {age / 3600:.1f} h old "
                            f"(limit {conf['MAX_AGE_HOURS']} h)")
    say(f"pull: {fetched} new, {len(archives_in(dest))} held in {dest}")
    if problems:
        raise Fail("; ".join(problems))


# --- restore ------------------------------------------------------------------------

REDIS_CONF = """# Redis for the rustmush instance {n}: written by rmbak.py restore from
# {archive} (a copy of {src}). Unix socket only.
port 0
unixsocket /run/redis-{n}mush/redis.sock
unixsocketperm 660
protected-mode yes
{modules}
appendonly {aof}
appendfsync everysec
save 900 1
save 300 10
save 60 10000
dir /var/lib/redis-{n}mush
logfile /var/log/redis/redis-{n}mush.log
pidfile /run/redis-{n}mush/redis.pid
daemonize no
supervised systemd
"""

REDIS_UNIT = """[Unit]
Description=Redis for rustmush {n}, a RESTORED COPY of {src} (rmbak.py; /run/redis-{n}mush/redis.sock)
After=network.target

[Service]
Type=notify
ExecStart=/usr/bin/redis-server /etc/redis/redis-{n}mush.conf
User=redis
Group=mush
SupplementaryGroups=redis
RuntimeDirectory=redis-{n}mush
RuntimeDirectoryMode=0750
StateDirectory=redis-{n}mush
StateDirectoryMode=0700
UMask=007
{cpus}
[Install]
WantedBy=multi-user.target
"""

SERVER_UNIT = """[Unit]
Description=rustmush {n}, a RESTORED COPY of {src} (rmbak.py): the server half
After=network-online.target redis-{n}mush.service
Wants=network-online.target
Requires=redis-{n}mush.service
AssertPathExists=!/opt/rustmush/{n}/server.hold

[Service]
Type=notify
NotifyAccess=main
User=mush
Group=mush
WorkingDirectory=/opt/rustmush/{n}
# Empty on a restored copy: no provider key ever reaches it.
EnvironmentFile=/opt/rustmush/{n}/{n}.env
ExecStart=/opt/rustmush/{n}/build/target/release/mush-server --config /opt/rustmush/{n}/mushd.toml
TimeoutStartSec=300
TimeoutStopSec=60
# A copy that will not boot stays down and says so: no restart hides the first failure.
Restart=no
RuntimeDirectory=rustmush-{n}
RuntimeDirectoryMode=0750
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=/opt/rustmush/{n}/data
{cpus}
[Install]
WantedBy=multi-user.target
"""

GATEWAY_UNIT = """[Unit]
Description=rustmush {n}, a RESTORED COPY of {src} (rmbak.py): the gateway half, generation %i
After=network-online.target redis-{n}mush.service {n}mush-server.service
Wants=network-online.target {n}mush-server.service
Requires=redis-{n}mush.service

[Service]
Type=notify
NotifyAccess=main
User=mush
Group=mush
LimitNOFILE=65536
WorkingDirectory=/opt/rustmush/{n}
ExecStart=/opt/rustmush/{n}/build/target/release/mush-gateway --config /opt/rustmush/{n}/mushd.toml --generation %i
KillMode=mixed
KillSignal=SIGTERM
TimeoutStopSec=120
Restart=no
StateDirectory={n}mush-gateway
StateDirectoryMode=0700
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=/opt/rustmush/{n}/data
{cpus}
[Install]
WantedBy=multi-user.target
"""


def toml_edit(text, edits, drop=()):
    """A rendered mushd.toml with `edits` ({(section, key): toml value}) applied
    and the `drop` keys removed (deploy/instance/launch-bringup.py's private_config)."""
    out, seen, section = [], set(), ""
    for line in text.splitlines():
        header = re.match(r"^\[([^\[\]]+)\]", line)
        if header:
            section = header.group(1).strip()
        key = re.match(r"^([A-Za-z_]+)\s*=", line)
        if key and (section, key.group(1)) in drop:
            continue
        if key and (section, key.group(1)) in edits:
            seen.add((section, key.group(1)))
            line = f"{key.group(1)} = {edits[(section, key.group(1))]}"
        out.append(line)
    return "\n".join(out) + "\n", set(edits) - seen


def systemctl(*words):
    return run(["systemctl"] + list(words))


def wait_for(what, ready, timeout):
    end = time.monotonic() + timeout
    while not ready():
        if time.monotonic() > end:
            raise Fail(f"{what} within {timeout} s")
        time.sleep(0.25)


def place_data(stage, manifest, data_dir, secrets_dir, without_secrets):
    """The archive's data directory, whole, owned by the service user."""
    apart = manifest.get("key_files", {}).get("kept_apart", [])
    if apart and not without_secrets:
        # Before anything is placed: the files kept apart are here, and are the right ones.
        if not secrets_dir:
            raise Fail(f"this archive keeps {len(apart)} secret file(s) apart: pass --secrets <dir> holding them "
                       "(the data directory's layout), or --without-secrets to stand the copy up with fresh keys")
        for entry in apart:
            source = os.path.join(secrets_dir, entry["path"][len("data/"):])
            if not os.path.isfile(source) or sha256_file(source) != entry["sha256"]:
                raise Fail(f"--secrets: {source} is missing or is not the file the manifest recorded")
    os.makedirs(data_dir, mode=0o750, exist_ok=True)
    for entry in manifest.get("dirs", []):
        path = os.path.join(data_dir, entry["path"][len("data/"):])
        os.makedirs(path, exist_ok=True)
        os.chmod(path, int(entry["mode"], 8) & 0o777)
    modes = {entry["path"]: entry["mode"] for entry in manifest["files"]}
    source_root = os.path.join(stage, "data")
    for directory, _, files in os.walk(source_root):
        for name in files:
            source = os.path.join(directory, name)
            rel = os.path.relpath(source, source_root)
            target = os.path.join(data_dir, rel)
            os.makedirs(os.path.dirname(target), exist_ok=True)
            shutil.move(source, target)
            os.chmod(target, int(modes["data/" + rel], 8) & 0o777)
    if apart and not without_secrets:
        for entry in apart:
            rel = entry["path"][len("data/"):]
            source = os.path.join(secrets_dir, rel)
            target = os.path.join(data_dir, rel)
            os.makedirs(os.path.dirname(target), mode=0o700, exist_ok=True)
            shutil.copyfile(source, target)
            os.chmod(target, int(entry["mode"], 8) & 0o777)
    os.makedirs(os.path.join(data_dir, "logs"), exist_ok=True)
    if is_root():
        for directory, dirs, files in os.walk(data_dir):
            for name in dirs + files:
                os.chown(os.path.join(directory, name), pwd.getpwnam("mush").pw_uid, pwd.getpwnam("mush").pw_gid,
                         follow_symlinks=False)
        shutil.chown(data_dir, "mush", "mush")


def load_store(layout, rdb, keep_aside=None):
    """Put one dump.rdb into the instance's own Redis. Redis started with
    `appendonly yes` and no append-only directory comes up EMPTY beside a
    dump.rdb (seen on 8.10), so the file is loaded by a server run with the
    append-only log off, the log is turned on there (which rewrites it from the
    loaded data), and only then does the unit start, on its own config."""
    if unit_active(layout.redis_unit):
        systemctl("stop", layout.redis_unit)
    os.makedirs(layout.redis_dir, mode=0o700, exist_ok=True)
    if keep_aside and os.listdir(layout.redis_dir):
        # In place: the store that was there is moved aside, never deleted.
        os.makedirs(keep_aside, mode=0o700)
        shutil.chown(keep_aside, "redis", "redis")
        for name in os.listdir(layout.redis_dir):
            os.rename(os.path.join(layout.redis_dir, name), os.path.join(keep_aside, name))
        say(f"the Redis files that were there are kept at {keep_aside}")
    for name in os.listdir(layout.redis_dir):
        path = os.path.join(layout.redis_dir, name)
        shutil.rmtree(path) if os.path.isdir(path) and not os.path.islink(path) else os.unlink(path)
    shutil.copyfile(rdb, os.path.join(layout.redis_dir, "dump.rdb"))
    shutil.chown(layout.redis_dir, "redis", "redis")
    shutil.chown(os.path.join(layout.redis_dir, "dump.rdb"), "redis", "redis")
    sock = os.path.join(layout.redis_dir, "restore.sock")
    loader = subprocess.Popen(
        ["runuser", "-u", "redis", "--", "redis-server", layout.redis_conf, "--appendonly", "no",
         "--unixsocket", sock, "--unixsocketperm", "600", "--supervised", "no", "--daemonize", "no",
         "--pidfile", os.path.join(layout.redis_dir, "restore.pid"), "--logfile",
         os.path.join(layout.redis_dir, "restore.log")],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    client = None
    try:
        def up():
            nonlocal client
            if loader.poll() is not None:
                raise Fail(f"redis-server exited {loader.returncode} loading the snapshot; see "
                           f"{layout.redis_dir}/restore.log")
            try:
                client = Redis(unix=sock)
                client.call("PING")
                return True
            except (OSError, RedisError):
                return False
        wait_for("Redis did not load the snapshot", up, 600)
        loaded = client.call("DBSIZE")
        client.call("CONFIG", "SET", "appendonly", "yes")
        wait_for("the append-only log was not rewritten",
                 lambda: client.info("persistence").get("aof_rewrite_in_progress") == "0"
                 and client.info("persistence").get("aof_enabled") == "1"
                 and client.info("persistence").get("aof_rewrite_scheduled") == "0", 600)
        if client.info("persistence").get("aof_last_bgrewrite_status") != "ok":
            raise Fail("the append-only rewrite failed after the load")
        try:
            client.call("SHUTDOWN")
        except (OSError, RedisError):
            pass
        loader.wait(timeout=120)
    finally:
        if loader.poll() is None:
            loader.kill()
            loader.wait()
        for name in ("restore.sock", "restore.pid", "restore.log"):
            try:
                os.unlink(os.path.join(layout.redis_dir, name))
            except OSError:
                pass
    systemctl("start", layout.redis_unit)
    unit_sock = f"/run/redis-{layout.n}mush/redis.sock"
    wait_for("the Redis unit's socket did not appear", lambda: os.path.exists(unit_sock), 60)
    client = Redis(unix=unit_sock)
    wait_for("the Redis unit did not finish loading", lambda: _pong(client), 600)
    now = client.call("DBSIZE")
    if now > loaded:
        raise Fail(f"the Redis unit holds {now} keys after the restart, more than the {loaded} loaded")
    return client


def _pong(client):
    try:
        return client.call("PING") == b"PONG"
    except RedisError:
        return False


def cmd_restore(args):
    if not is_root():
        raise Fail("restore must run as root (it writes units and runs the instance's Redis)")
    for tool in ("redis-cli", "redis-server", "systemctl", "runuser"):
        if not shutil.which(tool):
            raise Fail(f"{tool} is not on PATH")
    timings, clock = {}, time.monotonic()

    def lap(name):
        nonlocal clock
        timings[name] = round(time.monotonic() - clock, 1)
        clock = time.monotonic()

    archive = os.path.abspath(args.archive)
    # Read it whole once before anything on the box is touched: a damaged
    # archive never costs an earlier copy its place.
    peek = verify_archive(archive)
    if args.in_place:
        name = peek["instance"]
    else:
        name = args.as_name
        if not name:
            raise Fail("say where: --as <instance name> for a copy, or --in-place")
    layout = Layout(name)
    if args.in_place:
        if not os.path.isfile(layout.env_file) or not os.path.isfile(os.path.join(layout.units_dir, layout.server_unit)):
            raise Fail(f"--in-place puts {name}'s store and data back on a box where {name} is installed "
                       f"(units, Redis, the tree at {peek['commit'][:8]}): deploy it first (docs/ops.md)")
        if not args.yes:
            raise Fail(f"--in-place REPLACES {name}'s store and data directory with the archive's. Re-run with --yes.")
        busy = [u for u in [layout.server_unit] + layout.gateway_units if unit_active(u)]
        if busy:
            raise Fail(f"{name} is running ({', '.join(busy)}): stop both halves first "
                       f"(systemctl stop {' '.join(busy)})")
    else:
        if os.path.exists(layout.root):
            if not os.path.exists(layout.marker):
                raise Fail(f"{layout.root} exists and was not made by a restore: refusing to touch it")
            if not args.replace:
                raise Fail(f"{layout.root} is an earlier restored copy: pass --replace, or `rmbak.py remove {name} --yes`")
            remove_copy(layout)
        # A name with no directory can still be an instance's: its units, its Redis
        # config or its store may be all that is left of it. None of them is touched.
        claimed = [p for p in (os.path.join(layout.units_dir, layout.server_unit),
                               os.path.join(layout.units_dir, f"{name}mush-gateway@.service"),
                               os.path.join(layout.units_dir, layout.redis_unit),
                               layout.redis_conf, layout.redis_dir, layout.gw_state) if os.path.lexists(p)]
        if claimed:
            raise Fail(f"{name} is not free on this box ({', '.join(claimed)} exist and no restore made them): "
                       "pick another name for the copy")
        if not user_exists("redis"):
            raise Fail("this box has no redis user (install Redis from packages.redis.io first)")
        if not user_exists("mush"):
            run(["useradd", "--system", "--no-create-home", "--shell", "/usr/sbin/nologin", "mush"])
            say("created the system user mush")

    stage = tempfile.mkdtemp(prefix=f".restore-{name}-", dir="/opt/rustmush" if os.path.isdir("/opt/rustmush")
                             else None)
    try:
        manifest = verify_archive(archive, extract_to=stage)
        lap("verify_and_extract_s")
        src = manifest["instance"]
        say(f"{os.path.basename(archive)}: {src} at {manifest['commit'][:8]}, taken {manifest['created_utc']}, "
            f"IC {manifest['snapshot'].get('ic_checkpoint', 'unknown')}; every member matches its checksum")
        data_dir = f"/opt/rustmush/{name}/data"

        redis_aside = None
        if args.in_place:
            env = layout.env()
            data_dir = env["DATA_DIR"]
            stamp = time.strftime("%Y%m%d-%H%M%S", time.gmtime())
            redis_aside = f"{layout.redis_dir}.pre-restore-{stamp}"
            if os.path.isdir(data_dir) and os.listdir(data_dir):
                aside = f"{data_dir}.pre-restore-{stamp}"
                os.rename(data_dir, aside)
                say(f"the data directory that was there is kept at {aside}")
        else:
            os.makedirs(layout.root, mode=0o755)
            write_atomic(layout.marker, json.dumps({"from": os.path.basename(archive), "source": src,
                                                    "restored_utc": utc()}) + "\n", 0o644)
            # The build subset: binaries, help, the deploy directory, the stamps.
            shutil.move(os.path.join(stage, "build"), os.path.join(layout.root, "build"))
            release = os.path.join(layout.root, "build", "target", "release")
            for binary in ("mush-server", "mush-gateway"):
                if not os.path.isfile(os.path.join(release, binary)):
                    # An archive written with BINARIES=no: both come from a build at its commit.
                    if not args.binaries or not os.path.isfile(os.path.join(args.binaries, binary)):
                        raise Fail("this archive carries no binaries: build mush-server and mush-gateway at "
                                   f"{manifest['commit']} and pass --binaries <the directory holding them>")
                    os.makedirs(release, exist_ok=True)
                    shutil.copyfile(os.path.join(args.binaries, binary), os.path.join(release, binary))
                    if sha256_file(os.path.join(release, binary)) != manifest["binaries"][binary]["sha256"]:
                        say(f"note: {binary} from --binaries is not byte for byte the one {src} ran "
                            "(a rebuild rarely is); the commit is what it has to match")
                os.chmod(os.path.join(release, binary), 0o755)
            for directory, dirs, files in os.walk(os.path.join(layout.root, "build")):
                os.chmod(directory, 0o755)
                for item in files:
                    path = os.path.join(directory, item)
                    os.chmod(path, 0o755 if os.stat(path).st_mode & 0o100 else 0o644)
        place_data(stage, manifest, data_dir, args.secrets, args.without_secrets)
        lap("place_files_s")

        if not args.in_place:
            # Its own Redis, and a config that reaches nothing outside this box.
            archived_conf = os.path.join(stage, "config", f"redis-{src}mush.conf")
            modules = []
            if os.path.isfile(archived_conf):
                with open(archived_conf, encoding="utf-8") as handle:
                    modules = [line.strip() for line in handle if line.split()[:1] == ["loadmodule"]]
            for line in modules:
                if not os.path.isfile(line.split()[1]):
                    raise Fail(f"the archived Redis config loads {line.split()[1]}, which this box does not have")
            cpus = f"CPUAffinity={args.cpus}\n" if args.cpus else ""
            fill = {"n": name, "src": src, "archive": os.path.basename(archive), "modules": "\n".join(modules),
                    "cpus": cpus, "aof": "yes"}
            os.makedirs("/etc/redis", exist_ok=True)
            write_atomic(layout.redis_conf, REDIS_CONF.format(**fill), 0o640)
            shutil.chown(layout.redis_conf, "redis", "redis")
            os.makedirs("/var/log/redis", exist_ok=True)
            shutil.chown("/var/log/redis", "redis", "redis")
            write_atomic(os.path.join(layout.units_dir, layout.redis_unit), REDIS_UNIT.format(**fill), 0o644)
            write_atomic(os.path.join(layout.units_dir, layout.server_unit), SERVER_UNIT.format(**fill), 0o644)
            write_atomic(os.path.join(layout.units_dir, f"{name}mush-gateway@.service"),
                         GATEWAY_UNIT.format(**fill), 0o644)
            write_atomic(os.path.join(layout.root, f"{name}.env"), "", 0o600)
            with open(os.path.join(stage, "config", "mushd.toml"), encoding="utf-8") as handle:
                config = handle.read().replace(f"/opt/rustmush/{src}/", f"/opt/rustmush/{name}/")
            title = re.search(r'^name = "(.*)"$', config, re.M)
            edits = {
                ("server", "name"): json.dumps(f"{title.group(1) if title else src} [restored copy {name}]"),
                ("server", "data_dir"): json.dumps(data_dir),
                ("store", "redis_url"): json.dumps(f"redis+unix:///run/redis-{name}mush/redis.sock"),
                ("ssh", "listen"): json.dumps(f"127.0.0.1:{args.ssh_port}"),
                ("ssh", "trusted_socket"): '""',
                ("ipc", "listen"): json.dumps(f"/run/rustmush-{name}/{name}.sock"),
                ("web", "listen"): json.dumps(f"127.0.0.1:{args.web_port}"),
                ("web", "trusted_proxies"): "[]",
                ("web", "client_downloads_dir"): '""',
                # A copy never spends on a provider and never speaks on Discord.
                ("llm", "enabled"): "false",
                ("discord", "enabled"): "false",
            }
            config, missing = toml_edit(config, edits, drop={("web", "tls")})
            if missing:
                raise Fail(f"the archived mushd.toml has no {sorted(missing)} to rewrite")
            write_atomic(layout.config, config, 0o644)
            with open(os.path.join(stage, "config", "instance.env"), encoding="utf-8") as handle:
                env_text = handle.read().replace(f"/opt/rustmush/{src}/", f"/opt/rustmush/{name}/")
            overrides = {"INSTANCE": name, "DATA_DIR": data_dir, "BUILD_DIR": f"/opt/rustmush/{name}/build",
                         "EXTERNAL_IP": "127.0.0.1", "SSH_PORT": str(args.ssh_port), "WEB_PORT": str(args.web_port),
                         "REDIS_URL": f"redis+unix:///run/redis-{name}mush/redis.sock", "WEB_BIND": "127.0.0.1",
                         "IPC_SOCKET": f"/run/rustmush-{name}/{name}.sock", "TRUSTED_SOCKET": "",
                         "WEB_TRUSTED_PROXIES": "[]", "WEB_CLIENT_DOWNLOADS_DIR": "", "LLM_ENABLED": "false"}
            lines = []
            for line in env_text.splitlines():
                key = line.split("=", 1)[0] if "=" in line and not line.lstrip().startswith("#") else None
                lines.append(f"{key}={overrides[key]}" if key in overrides else line)
            write_atomic(layout.env_file, "\n".join(lines) + "\n", 0o644)
            systemctl("daemon-reload")
            for binary, extra in (("mush-server", []), ("mush-gateway", ["--generation", "a"])):
                run(["runuser", "-u", "mush", "--", os.path.join(layout.root, "build", "target", "release", binary),
                     "--config", layout.config, "--check-config"] + extra)
        write_atomic(os.path.join(layout.root, "archive-manifest.json"),
                     json.dumps(manifest, indent=1, sort_keys=True) + "\n", 0o644)
        lap("config_and_units_s")

        # The store, then the facts BEFORE anything boots on it.
        client = load_store(layout, os.path.join(stage, "redis", "dump.rdb"), keep_aside=redis_aside)
        lap("load_store_s")
        facts = collect_facts(client, data_dir)
        client.close()
        differences = compare_facts(manifest["facts"], facts, "the restored store") + \
            [f"the restored store: {p}" for p in facts["problems"]]
        lap("facts_s")
        if differences:
            for line in differences:
                shout(f"MISMATCH: {line}")
            raise Fail(f"the restored store does not match its manifest in {len(differences)} way(s); "
                       "nothing was started on it")
        say(f"the restored store matches its manifest: {facts['persistent_keys']} keys, {facts['accounts']} accounts, "
            f"{len(facts['ledgers'])} ledgers whole across both tiers, every instrument sums to zero")
    finally:
        shutil.rmtree(stage, ignore_errors=True)

    if args.no_start or args.in_place:
        say("nothing was started. " + (
            f"Start {name} as a deploy does: `rm -f {layout.hold}` if it is held, then the instance's deploy.sh; "
            f"then `rmbak.py check {name}`." if args.in_place else f"Start it: systemctl start {layout.gateway_unit}"))
    else:
        systemctl("start", layout.server_unit)
        lap("server_boot_s")
        systemctl("start", layout.gateway_unit)
        wait_for("the gateway did not open its SSH port", lambda: port_open(args.ssh_port), 120)
        lap("gateway_up_s")
        say(f"{name} is up on 127.0.0.1: ssh {args.ssh_port}, web {args.web_port}")
    timings["total_s"] = round(sum(timings.values()), 1)
    write_atomic(os.path.join(layout.root, "restore-timings.json"), json.dumps(timings, indent=1) + "\n", 0o644)
    say("timings: " + ", ".join(f"{k[:-2]} {v} s" for k, v in timings.items()))
    say(f"next: rmbak.py check {name} [--staff <login> --password-file <file>]")


def port_open(port):
    try:
        socket.create_connection(("127.0.0.1", int(port)), timeout=1).close()
        return True
    except OSError:
        return False


def remove_copy(layout):
    units = [layout.server_unit] + layout.gateway_units + [layout.redis_unit]
    for unit in units[:3]:
        subprocess.run(["systemctl", "disable", "--now", unit], capture_output=True)
    subprocess.run(["systemctl", "disable", "--now", layout.redis_unit], capture_output=True)
    for unit in units:
        subprocess.run(["systemctl", "reset-failed", unit], capture_output=True)
    for path in (os.path.join(layout.units_dir, layout.server_unit),
                 os.path.join(layout.units_dir, f"{layout.n}mush-gateway@.service"),
                 os.path.join(layout.units_dir, layout.redis_unit), layout.redis_conf,
                 f"/var/log/redis/redis-{layout.n}mush.log"):
        if os.path.isfile(path):
            os.unlink(path)
    for path in (layout.redis_dir, layout.gw_state, layout.root):
        if os.path.isdir(path) and not os.path.islink(path):
            shutil.rmtree(path)
    subprocess.run(["systemctl", "daemon-reload"], capture_output=True)


def cmd_remove(args):
    if not is_root():
        raise Fail("remove must run as root")
    layout = Layout(args.instance)
    if not os.path.exists(layout.marker):
        raise Fail(f"{layout.root} was not made by `rmbak.py restore --as`: refusing to remove it")
    if not args.yes:
        raise Fail(f"this removes the restored copy {args.instance}: its units, its Redis and {layout.root}. "
                   "Re-run with --yes.")
    remove_copy(layout)
    say(f"removed the restored copy {args.instance}")


# --- check: the restored instance, booted ----------------------------------------------


class Session:
    """One line-mode SSH session at the instance's own front door, through a
    PTY (deploy/instance/launch-bringup.py's Session, without its transcript)."""

    def __init__(self, port, host_key_path, workdir):
        public = run(["ssh-keygen", "-y", "-f", host_key_path]).strip().splitlines()[0]
        known = os.path.join(workdir, "known_hosts")
        write_atomic(known, f"[127.0.0.1]:{port} {public}\n")
        self.raw, self.pos, self.suffix, self.bracketed, self.typed_at = b"", 0, None, 0, 0.0
        master, slave = pty.openpty()
        settings = termios.tcgetattr(slave)
        settings[3] &= ~termios.ECHO
        termios.tcsetattr(slave, termios.TCSANOW, settings)
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 500, 0, 0))

        def tty():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

        self.proc = subprocess.Popen(
            ["ssh", "-tt", "-p", str(port), "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes",
             "-o", f"UserKnownHostsFile={known}", "-o", "GlobalKnownHostsFile=/dev/null",
             "-o", "LogLevel=ERROR", "play@127.0.0.1"],
            stdin=slave, stdout=slave, stderr=slave, preexec_fn=tty, close_fds=True)
        os.close(slave)
        self.master = master
        self.wait(r"(?i)connect <account>", 60, "the welcome screen")

    def text(self):
        return ANSI.sub("", self.raw[self.pos:].decode("utf-8", "replace")).replace("\r\n", "\n")

    def pump(self, timeout):
        if select.select([self.master], [], [], timeout)[0]:
            try:
                chunk = os.read(self.master, 65536)
            except OSError:
                chunk = b""
            if not chunk:
                raise Fail(f"the staff session closed; its last lines:\n{self.text()[-600:]}")
            self.raw += chunk

    def wait(self, pattern, timeout, what=None):
        regex = re.compile(pattern, re.M)
        end = time.monotonic() + timeout
        while True:
            found = regex.search(self.text())
            if found:
                return found
            if time.monotonic() >= end:
                raise Fail(f"no {what or pattern!r} within {timeout} s; the reply so far:\n{self.text()[-800:]}")
            self.pump(0.2)

    def type(self, line):
        pause = self.typed_at + 0.15 - time.monotonic()
        if pause > 0:
            time.sleep(pause)
        if self.suffix:
            mark = self.suffix.encode()
            end = time.monotonic() + 20
            while self.raw.count(mark) < self.bracketed and time.monotonic() < end:
                self.pump(0.2)
            self.bracketed = self.raw.count(mark) + 1
        self.typed_at = time.monotonic()
        self.pos = len(self.raw)
        os.write(self.master, line.encode("utf-8") + b"\r")

    def connect(self, account, password):
        self.type(f"connect {account} {password}")
        found = self.wait(r"Connected as |does not exist, or the password is wrong|Characters on |banned", 60)
        if "Characters on" in found.group(0):
            self.type("1")
            found = self.wait(r"Connected as ", 60)
        if "Connected as" not in found.group(0):
            raise Fail(f"cannot connect as {account}: {found.group(0)}")
        self.suffix = "RMBAK-" + pysecrets.token_hex(6)
        self.type(f"OUTPUTSUFFIX {self.suffix}")
        self.wait(rf"^{self.suffix}$", 30, "the reply bracket")

    def ask(self, line, timeout=60, patient=True):
        """A read: the whole reply. A line the world turns away for a moment is
        typed again; with `patient` off it is None the first time it is turned away."""
        end = time.monotonic() + 420
        while True:
            self.type(line)
            found = self.wait(rf"^{self.suffix}$|(?P<later>{TRANSIENT})", timeout, f"the reply to {line!r}")
            if not found.groupdict().get("later"):
                return self.text().split(self.suffix)[0]
            if not patient:
                return None
            if time.monotonic() >= end:
                raise Fail(f"{line!r} was turned away for seven minutes: {found.group('later')}")
            time.sleep(5)

    def close(self):
        try:
            self.type("QUIT")
            self.pump(1)
            if "Really disconnect" in self.text():
                self.type("y")
        except (OSError, Fail):
            pass
        try:
            self.proc.terminate()
            self.proc.wait(timeout=5)
        except (OSError, subprocess.TimeoutExpired):
            self.proc.kill()
        os.close(self.master)


def boot_log(layout, only_errors=False):
    """What the server unit's current run has logged."""
    invocation = systemctl("show", "-p", "InvocationID", "--value", layout.server_unit).strip()
    if not invocation:
        raise Fail(f"{layout.server_unit} has not run on this box")
    command = ["journalctl", f"_SYSTEMD_INVOCATION_ID={invocation}", "-o", "cat", "--no-pager"]
    if only_errors:
        command += ["--case-sensitive=true", "-g", "ERROR"]
    done = subprocess.run(command, capture_output=True, text=True)
    if done.returncode != 0 and done.stdout.strip():
        raise Fail(f"journalctl exited {done.returncode}: {done.stderr.strip()[-300:]}")
    return ANSI.sub("", done.stdout)


def error_lines(log):
    return [line.strip() for line in log.splitlines() if re.search(r"\bERROR\b", line)]


def error_signature(line):
    """An ERROR line without what changes from one boot to the next: the half
    and the timestamp at its head, and every number in it."""
    line = ANSI.sub("", line).strip()
    line = re.sub(r"^half=\w+\s+", "", line)
    line = re.sub(r"^\d{4}-\d\d-\d\dT[\d:.]+Z\s+", "", line)
    return re.sub(r"\d+", "#", line)[:300]


def live_error_signatures(layout):
    """Every distinct ERROR the instance's server has logged since it booted.
    A restored copy that logs one of these is repeating the live world, not
    failing; one that logs anything else has a fault of its own. None when the
    journal cannot be read (the check then has nothing to excuse an ERROR by)."""
    try:
        return sorted({error_signature(line) for line in error_lines(boot_log(layout, only_errors=True))})[:200]
    except (Fail, OSError):
        return None


def cmd_check(args):
    layout = Layout(args.instance)
    env = layout.env()
    manifest_path = os.path.join(layout.root, "archive-manifest.json")
    if not os.path.isfile(manifest_path):
        raise Fail(f"{manifest_path} is missing: {args.instance} was not restored by this tool")
    with open(manifest_path, encoding="utf-8") as handle:
        manifest = json.load(handle)
    want = manifest["facts"]
    report, failures = [], []

    def ok(text):
        report.append(("ok", text))
        say(f"ok    {text}")

    def bad(text):
        failures.append(text)
        report.append(("FAIL", text))
        shout(f"FAIL  {text}")

    def note(text):
        report.append(("note", text))
        say(f"note  {text}")

    started = time.monotonic()
    # 1. The units, and what the boot said.
    serving = [u for u in layout.gateway_units if unit_active(u)]
    for unit in (layout.redis_unit, layout.server_unit):
        (ok if unit_active(unit) else bad)(f"{unit} is {'active' if unit_active(unit) else 'NOT active'}")
    (ok if serving else bad)(f"a gateway generation serves: {', '.join(serving) or 'none'}")
    if not unit_active(layout.server_unit):
        raise Fail("the server is not running: " + boot_log(layout)[-1500:])
    log = boot_log(layout)
    for needle in ("world loaded from store", "world timeline installed"):
        (ok if needle in log else bad)(f"the boot logged `{needle}`")
    # An ERROR the live instance had already logged is the live world's own
    # (the manifest carries those, and --known-errors adds a log by hand); any
    # other is this copy's, and fails the check.
    known = set(manifest.get("live_errors") or [])
    if args.known_errors:
        with open(args.known_errors, encoding="utf-8", errors="replace") as handle:
            known |= {error_signature(line) for line in error_lines(handle.read())}
    judged = set()

    def judge_errors(text):
        fresh = [line for line in error_lines(text) if line not in judged]
        judged.update(fresh)
        for line in fresh[:10]:
            if error_signature(line) in known:
                note(f"an ERROR the live instance logs too, so not the restore's: {line[:260]}")
            else:
                bad(f"the copy logged an ERROR the live instance had not: {line[:300]}")
        return fresh

    if not judge_errors(log):
        ok("the boot logged no ERROR")
    for needle in ("conservation broken", "contiguity breach", "freezing the economy", "quarantine record",
                   "corrupt object doc", "module boot failed"):
        if needle in log:
            bad(f"the boot logged `{needle}`")
    outage = re.search(r"boot_outage=(\w+)", log)
    loaded = re.search(r"world loaded from store.*?objects=(\d+)", log)
    if loaded:
        (ok if int(loaded.group(1)) == want["objects"] else bad)(
            f"the world loaded {loaded.group(1)} objects; the manifest counts {want['objects']}")
    # The IC clock: a restore boots long after the snapshot's last checkpoint,
    # so the world must freeze AT that checkpoint and count down; it never
    # jumps ahead by the time the archive spent on disk.
    checkpoint = (want["clock"] or {}).get("checkpoint") or {}
    if outage:
        fine = outage.group(1) in ("OutageFreeze", "SilentReanchor")
        (ok if fine else bad)(f"the boot's outage decision is {outage.group(1)}")
        if outage.group(1) == "NoPriorCheckpoint":
            bad("the boot found no clock checkpoint: this is not the archived store")

    # 2. The store under the running server: history kept, invariants held.
    client = Redis.from_url(env["REDIS_URL"])
    upto = {name: row["last_seq"] for name, row in want["ledgers"].items() if row["last_seq"] is not None}
    now = collect_facts(client, env["DATA_DIR"], upto)
    client.close()
    for problem in now["problems"]:
        bad(f"the running store: {problem}")
    if not now["problems"]:
        ok("the running store: every ledger whole across both tiers, every instrument sums to zero")
    (ok if now["accounts"] == want["accounts"] else bad)(
        f"accounts: {now['accounts']} (the manifest: {want['accounts']})")
    (ok if now["objects"] >= want["objects"] else bad)(f"objects: {now['objects']} (the manifest: {want['objects']})")
    kept = lost = 0
    for name, row in sorted(want["ledgers"].items()):
        have = now["ledgers"].get(name)
        if have is None:
            lost += 1
            (bad if name.startswith("economy/") else note)(f"ledger {name} is gone after the boot")
        elif have["digest"] != row["digest"]:
            lost += 1
            bad(f"ledger {name}: its first {row['entries']} entries differ from the archived ones after the boot")
        else:
            kept += 1
    if kept and not lost:
        ok(f"all {kept} ledgers still hold their archived entries, entry for entry")
    now_cp = (now["clock"] or {}).get("checkpoint") or {}
    if checkpoint and now_cp:
        (ok if now_cp.get("ic_unix", 0) >= checkpoint["ic_unix"] else bad)(
            f"the IC clock never went back: now IC {ic_date(now_cp['ic_unix'])}, archived at "
            f"IC {ic_date(checkpoint['ic_unix'])}")
    freeze = (now["clock"] or {}).get("freeze")
    if checkpoint and freeze and (freeze.get("kind") or {}).get("kind") == "outage_countdown":
        (ok if freeze.get("frozen_ic_unix") == checkpoint["ic_unix"] else bad)(
            f"the store holds the world frozen at IC {freeze.get('frozen_ic_unix')} "
            f"({ic_date(freeze.get('frozen_ic_unix', 0))}) until {utc(freeze['kind'].get('unfreeze_at_real_unix'))}: "
            f"the archive's last checkpoint is IC {checkpoint['ic_unix']}")
    elif checkpoint and freeze:
        note("the world carries a deliberate freeze from the archive (@tacsim/time freeze): it stays frozen")
    elif checkpoint:
        note("the world is not frozen: its post-outage countdown has ended, or it booted inside the threshold")
    if "geology key matches its pin" in log:
        ok("the geology key that came back is the one this world pinned")
    elif "geology key pinned" in log:
        note("the boot pinned a geology key: the archived world had pinned none")

    # 3. healthz. A world frozen in its post-outage countdown does not tick, so
    # healthz reads world_ok=false and answers 503 until the thaw: that is the
    # server's own behavior, and passes here only with everything else true.
    web_port = env.get("WEB_PORT")
    fields = ("ok", "world_ok", "store_ok", "integrity_ok", "server_ok")

    def health():
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{web_port}/healthz", timeout=10) as reply:
                return reply.status, json.loads(reply.read())
        except urllib.error.HTTPError as error:
            try:
                return error.code, json.loads(error.read())
            except ValueError:
                return error.code, {}
        except (OSError, ValueError) as error:
            return None, {"error": str(error)}

    def shown(body):
        return ", ".join(f"{k}={body.get(k)}" for k in fields) if "ok" in body else str(body)

    status, body = health()
    counting = bool(freeze and (freeze.get("kind") or {}).get("kind") == "outage_countdown")
    if status == 200 and all(body.get(k) for k in ("ok", "store_ok", "integrity_ok", "server_ok")):
        ok(f"healthz: {shown(body)}")
    elif status == 503 and counting and not body.get("world_ok") and \
            all(body.get(k) for k in ("store_ok", "integrity_ok", "server_ok")):
        ok(f"healthz: 503 while the world counts down its outage freeze and does not tick ({shown(body)})")
    else:
        bad(f"healthz on 127.0.0.1:{web_port}: HTTP {status}, {shown(body)}")

    def after_thaw():
        """The world ticks again: healthz must read ok, and what the first ticks logged is judged."""
        time.sleep(8)
        status, body = health()
        fine = status == 200 and all(body.get(k) for k in fields)
        (ok if fine else bad)(f"healthz after the thaw: HTTP {status}, {shown(body)}")
        judge_errors(boot_log(layout))

    # 4. A staff session: read-only checks at the front door.
    if args.staff:
        if not args.password_file:
            raise Fail("--staff needs --password-file")
        with open(args.password_file, encoding="utf-8") as handle:
            password = handle.readline().rstrip("\r\n")
        workdir = tempfile.mkdtemp(prefix="rmbak-check-")
        try:
            session = Session(env["SSH_PORT"], os.path.join(env["DATA_DIR"], "ssh_host_ed25519"), workdir)
            try:
                session.connect(args.staff, password)
                ok(f"the staff account {args.staff} from the archive logs in")
                reply = session.ask("@tacsim/time")
                line = re.search(r"World time: generation (\d+), (\d+) IC seconds per real second", reply)
                if line:
                    ok(f"@tacsim/time: {' '.join(reply.split())[:260]}")
                else:
                    bad(f"@tacsim/time did not answer: {reply.strip()[:300]}")
                frozen = re.search(r"frozen at IC (-?\d+)", reply)
                if frozen and checkpoint:
                    (ok if int(frozen.group(1)) == checkpoint["ic_unix"] else bad)(
                        f"the world is frozen at IC {frozen.group(1)} ({ic_date(int(frozen.group(1)))}); the "
                        f"archive's last checkpoint is IC {checkpoint['ic_unix']}")
                for line in ("time", "@stats"):
                    reply = session.ask(line)
                    (ok if reply.strip() else bad)(f"{line}: {' '.join(reply.split())[:200]}")

                def readiness():
                    # The port readiness report (docs/ops.md, "`@tacsim/port status`"): the
                    # geology line names the key this world pinned; anything not ready is said.
                    answer = session.ask("@tacsim/port status", patient=False)
                    if answer is None:
                        note("@tacsim/port status is not answered during the post-outage countdown: "
                             "--wait-unfreeze asks it after the thaw")
                        return
                    rows = [row.strip() for row in answer.splitlines() if row.strip()]
                    geology = [row for row in rows if row.startswith("geology:")]
                    if geology:
                        (ok if "pinned" in geology[0] and geology[0].endswith("[ready]") else bad)(
                            f"@tacsim/port status: {geology[0]}")
                    states = [row for row in rows if re.search(r"\[\w+\]$", row)]
                    waiting = [row for row in states if not row.endswith("[ready]")]
                    if states:
                        note(f"@tacsim/port status: {len(states) - len(waiting)} of {len(states)} lines ready" +
                             (": not ready: " + "; ".join(waiting)[:400] if waiting else ""))

                if not (args.wait_unfreeze and frozen):
                    readiness()
                if args.wait_unfreeze and frozen:
                    say("waiting for the post-outage countdown to end (five minutes by default)")
                    end, thawed = time.monotonic() + 420, False
                    while time.monotonic() < end and not thawed:
                        time.sleep(15)
                        thawed = "frozen at IC" not in session.ask("@tacsim/time")
                    if not thawed:
                        bad("the world was still frozen seven minutes after the boot")
                    else:
                        time.sleep(3)
                        client = Redis.from_url(env["REDIS_URL"])
                        later = collect_facts(client, None)["clock"].get("checkpoint") or {}
                        client.close()
                        moved = later.get("ic_unix", 0) - checkpoint.get("ic_unix", 0)
                        (ok if moved > 0 else bad)(
                            f"the clock resumed from the archived instant: IC {ic_date(later.get('ic_unix', 0))}, "
                            f"{moved} IC seconds on")
                        readiness()
                        after_thaw()
            finally:
                session.close()
        finally:
            shutil.rmtree(workdir, ignore_errors=True)
    else:
        note("no staff session (--staff, --password-file): @tacsim/time and the login were NOT checked")
        if args.wait_unfreeze and freeze:
            # Without a login the thaw is read where the world writes it: the store.
            say("waiting for the post-outage countdown to end (five minutes by default)")
            end, later = time.monotonic() + 420, {"freeze": freeze}
            while time.monotonic() < end and later.get("freeze"):
                time.sleep(15)
                client = Redis.from_url(env["REDIS_URL"])
                later = collect_facts(client, None)["clock"]
                client.close()
            if later.get("freeze"):
                bad("the store still held the world frozen seven minutes after the boot")
            else:
                time.sleep(12)  # an idle world checkpoints its clock every ten seconds
                client = Redis.from_url(env["REDIS_URL"])
                later = collect_facts(client, None)["clock"]
                client.close()
                moved = ((later.get("checkpoint") or {}).get("ic_unix", 0)) - checkpoint.get("ic_unix", 0)
                (ok if moved > 0 else bad)(
                    f"the clock resumed from the archived instant: IC "
                    f"{ic_date((later.get('checkpoint') or {}).get('ic_unix', 0))}, {moved} IC seconds on")
                after_thaw()

    result = {"instance": args.instance, "archive": manifest.get("instance"), "checked_utc": utc(),
              "took_s": round(time.monotonic() - started, 1), "failures": failures,
              "report": [{"result": r, "text": t} for r, t in report]}
    try:
        write_atomic(os.path.join(layout.root, "check-report.json"), json.dumps(result, indent=1) + "\n", 0o644)
    except OSError:
        pass
    if failures:
        raise Fail(f"{args.instance}: {len(failures)} check(s) FAILED")
    say(f"{args.instance}: every check passed ({result['took_s']} s)")


# --- the small commands ------------------------------------------------------------------


def cmd_backup(args):
    conf = load_conf(args.conf)
    if args.secrets:
        conf["SECRETS"] = args.secrets
    if args.no_binaries:
        conf["BINARIES"] = "no"
    print(do_backup(args.instance, conf, args.dest))


def cmd_verify(args):
    path = os.path.abspath(args.archive)
    if not args.deep:
        manifest = verify_archive(path)
        say(f"{os.path.basename(path)}: sound; {len(manifest['files'])} members match their checksums")
        return
    _, facts, differences = deep_verify(path, args.workdir)
    if differences:
        for line in differences:
            shout(f"MISMATCH: {line}")
        raise Fail(f"{os.path.basename(path)}: the snapshot does not match its manifest in {len(differences)} way(s)")
    say(f"{os.path.basename(path)}: sound, and its snapshot loads to the manifest's facts "
        f"({facts['persistent_keys']} keys, {facts['accounts']} accounts, {len(facts['ledgers'])} ledgers)")


def cmd_show(args):
    manifest = verify_archive(os.path.abspath(args.archive))
    facts, snap = manifest["facts"], manifest["snapshot"]
    if args.json:
        print(json.dumps(manifest, indent=1, sort_keys=True))
        return
    print(f"instance      {manifest['instance']} on {manifest['host']}")
    print(f"commit        {manifest['commit']}")
    print(f"taken         {manifest['created_utc']} (the fork between {utc(snap['taken_between_unix'][0])} and "
          f"{utc(snap['taken_between_unix'][1])}; {snap['cut_attempts']} attempt(s))")
    print(f"IC time       {snap.get('ic_checkpoint', 'unknown')} (the clock's last checkpoint)")
    print(f"store         {facts['keys']} keys ({facts['volatile_keys']} with a time to live), "
          f"{facts['objects']} objects, {facts['accounts']} accounts, digest {facts['store_digest'][:16]}…")
    print(f"ledgers       {len(facts['ledgers'])}; archive tier {facts['archive_tier']}")
    for name, row in sorted(facts["ledgers"].items()):
        print(f"  {name}: seq {row['first_seq']}..{row['last_seq']} in chunks {row['first_chunk']}..{row['last_chunk']} "
              f"({row['chunks_in_store']} in the store, {row['chunks_only_in_archive']} only in the archive tier)")
    for name, row in sorted(facts["economy"]["instruments"].items()):
        print(f"economy       {name}: {row['accounts']} accounts, posted balances sum to {row['sum_posted']}")
    key_files = manifest.get("key_files", {})
    print(f"key files     {key_files.get('mode')}: {len(key_files.get('kept_apart', []))} file(s) kept apart")
    print(f"binaries      " + ", ".join(f"{n} {'in the archive' if b['in_archive'] else 'NOT in the archive'} "
                                         f"({b['sha256'][:12]}…)" for n, b in sorted(manifest["binaries"].items())))
    print(f"files         {len(manifest['files'])}, {sum(f['size'] for f in manifest['files']) / 1e6:.1f} MB unpacked")
    live = manifest.get("live_errors")
    print("live errors   " + ("the journal was not read" if live is None else
                              f"{len(live)} distinct ERROR line(s) in the live server's log since its boot"))
    for line in live or []:
        print(f"  {line[:200]}")
    for problem in facts["problems"]:
        print(f"PROBLEM       {problem}")


def cmd_tree(args):
    """Lay out the archive's own slice of the build tree: both binaries, the
    stamps, the help files, the deploy directory and the deploy's scripts.
    It is everything `deploy/<instance>-split/deploy.sh` reads, so a box that
    has lost its tree needs the archive and nothing else to run a deploy."""
    archive = os.path.abspath(args.archive)
    manifest = verify_archive(archive)
    target = args.to or f"/opt/rustmush/{manifest['instance']}/build"
    if os.path.lexists(target) and (os.path.islink(target) or not os.path.isdir(target) or os.listdir(target)):
        raise Fail(f"{target} exists and is not empty: a tree is laid only where there is none")
    if not all(row["in_archive"] for row in manifest["binaries"].values()):
        raise Fail("this archive carries no binaries (BINARIES=no): stage and build the tree at "
                   f"{manifest['commit']} as a deploy does (deploy/redeploy/stage-build.sh)")
    parent = os.path.dirname(target.rstrip("/")) or "."
    os.makedirs(parent, mode=0o755, exist_ok=True)
    stage = tempfile.mkdtemp(prefix=".rmbak-tree-", dir=parent)
    try:
        verify_archive(archive, extract_to=stage)
        modes = {row["path"]: int(row["mode"], 8) for row in manifest["files"]}
        source = os.path.join(stage, "build")
        for directory, _, files in os.walk(source):
            os.chmod(directory, 0o755)
            for name in files:
                path = os.path.join(directory, name)
                rel = "build/" + os.path.relpath(path, source).replace(os.sep, "/")
                os.chmod(path, 0o755 if modes[rel] & 0o100 else 0o644)
        if os.path.isdir(target):
            os.rmdir(target)
        os.rename(source, target)
    finally:
        shutil.rmtree(stage, ignore_errors=True)
    say(f"laid {manifest['instance']}'s tree at {manifest['commit'][:8]} in {target}: both binaries, the stamps, "
        "docs/help, deploy/ and the deploy's scripts. Next: its deploy.sh, with the world held "
        f"(SERVER_HOLD=1 bash {target}/deploy/{manifest['instance']}-split/deploy.sh)")


def cmd_facts(args):
    layout = Layout(args.instance)
    env = layout.env()
    client = Redis.from_url(env["REDIS_URL"])
    print(json.dumps(collect_facts(client, env["DATA_DIR"]), indent=1, sort_keys=True))


def archive_age_s(name):
    return time.time() - calendar.timegm(time.strptime(NAME_RE.match(name).group(2), "%Y%m%dT%H%M%SZ"))


def cmd_status(args):
    """Is the backup healthy on this box: the timer, the last run, the newest archives."""
    conf = load_conf(args.conf)
    problems = []
    for directory, timer in ((conf["BACKUP_DIR"], "rustmush-backup.timer"),
                             (conf["PULL_DIR"], "rustmush-backup-pull.timer")):
        if not os.path.isdir(directory):
            continue
        if shutil.which("systemctl"):
            enabled = subprocess.run(["systemctl", "is-enabled", timer], capture_output=True, text=True).stdout.strip()
            last = subprocess.run(["systemctl", "show", "-p", "LastTriggerUSec", "--value", timer],
                                  capture_output=True, text=True).stdout.strip()
            print(f"{timer}: {enabled or 'not installed'}; last fired {last or 'never'}")
            if enabled != "enabled":
                problems.append(f"{timer} is not enabled")
        try:
            with open(os.path.join(directory, "status.json"), encoding="utf-8") as handle:
                status = json.load(handle)
        except (OSError, ValueError):
            status = {}
        instances = sorted(set(status.get("instances", {})) | {NAME_RE.match(n).group(1) for n in archives_in(directory)})
        for instance in instances:
            row = status.get("instances", {}).get(instance, {})
            have = archives_in(directory, instance)
            newest = f"{have[-1]} ({archive_age_s(have[-1]) / 3600:.1f} h old)" if have else "none"
            print(f"{directory}: {instance}: {len(have)} archive(s), newest {newest}; "
                  f"last run {'ok' if row.get('ok') else 'FAILED: ' + str(row.get('error')) if row else 'unknown'}")
            if row and not row.get("ok"):
                problems.append(f"{instance}: the last backup failed: {row.get('error')}")
            if not have or archive_age_s(have[-1]) > int(conf["MAX_AGE_HOURS"]) * 3600:
                problems.append(f"{instance}: no archive in {directory} newer than {conf['MAX_AGE_HOURS']} h")
    if problems:
        raise Fail("; ".join(problems))


def main():
    parser = argparse.ArgumentParser(description="Whole-instance backup, restore and check (docs/ops.md).")
    parser.add_argument("--conf", help=f"the configuration file ({DEFAULT_CONF})")
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("backup", help="one archive of one instance")
    p.add_argument("instance")
    p.add_argument("--dest", help="the directory to write into (BACKUP_DIR)")
    p.add_argument("--secrets", choices=("in-archive", "apart"))
    p.add_argument("--no-binaries", action="store_true")
    p.set_defaults(run=cmd_backup)
    p = sub.add_parser("run", help="the timer's entry: every configured instance, prune, status.json")
    p.set_defaults(run=cmd_run)
    p = sub.add_parser("verify", help="an archive is sound")
    p.add_argument("archive")
    p.add_argument("--deep", action="store_true", help="also load the snapshot and re-derive the facts")
    p.add_argument("--workdir", help="where --deep unpacks (a private temp directory)")
    p.set_defaults(run=cmd_verify)
    p = sub.add_parser("show", help="an archive's manifest, in brief")
    p.add_argument("archive")
    p.add_argument("--json", action="store_true")
    p.set_defaults(run=cmd_show)
    p = sub.add_parser("pull", help="fetch new archives from the box that takes them")
    p.set_defaults(run=cmd_pull)
    p = sub.add_parser("restore", help="stand an archive up")
    p.add_argument("archive")
    p.add_argument("--as", dest="as_name", help="the name of the copy (its own Redis, units and ports)")
    p.add_argument("--in-place", action="store_true", help="put the store and data back under the archive's own name")
    p.add_argument("--yes", action="store_true")
    p.add_argument("--replace", action="store_true", help="replace an earlier restored copy of that name")
    p.add_argument("--ssh-port", type=int, default=16892)
    p.add_argument("--web-port", type=int, default=16893)
    p.add_argument("--secrets", help="a directory holding the secret files an archive kept apart")
    p.add_argument("--without-secrets", action="store_true")
    p.add_argument("--binaries", help="a directory holding both binaries, for an archive written without them")
    p.add_argument("--cpus", help="pin the copy's units to these CPUs (CPUAffinity), e.g. 24-31")
    p.add_argument("--no-start", action="store_true")
    p.set_defaults(run=cmd_restore)
    p = sub.add_parser("check", help="a restored instance boots, answers and agrees with its manifest")
    p.add_argument("instance")
    p.add_argument("--staff", help="a staff login of the archived world")
    p.add_argument("--password-file", help="a file whose first line is that login's password")
    p.add_argument("--wait-unfreeze", action="store_true", help="wait out the post-outage countdown too")
    p.add_argument("--known-errors", help="a file of the LIVE instance's log lines: an ERROR found there is the live "
                                          "world's, not the restore's (for an archive whose manifest lists none)")
    p.set_defaults(run=cmd_check)
    p = sub.add_parser("remove", help="take a restored copy away")
    p.add_argument("instance")
    p.add_argument("--yes", action="store_true")
    p.set_defaults(run=cmd_remove)
    p = sub.add_parser("tree", help="lay the archive's slice of the build tree where a box has lost its own")
    p.add_argument("archive")
    p.add_argument("--to", help="the directory to make (/opt/rustmush/<instance>/build)")
    p.set_defaults(run=cmd_tree)
    p = sub.add_parser("facts", help="the facts of a live instance's store")
    p.add_argument("instance")
    p.set_defaults(run=cmd_facts)
    p = sub.add_parser("status", help="the timer, the last run and the newest archives; non-zero when stale or failed")
    p.set_defaults(run=cmd_status)
    args = parser.parse_args()
    try:
        args.run(args)
    except Fail as error:
        shout(f"FAILED: {error}")
        return 1
    except (subprocess.TimeoutExpired, RedisError, OSError) as error:
        shout(f"FAILED: {type(error).__name__}: {error}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
