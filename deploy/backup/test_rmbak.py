#!/usr/bin/env python3
"""Tests for deploy/backup/rmbak.py and deploy/backup/backup-serve.sh.

    python3 -B deploy/backup/test_rmbak.py

The cycle tests stand a tiny instance up under a temp directory (RMBAK_SYSROOT)
with its own redis-server on a Unix socket, back it up, and then damage the
archive in every way the checks claim to catch. They need redis-server and
redis-cli on PATH (any 6.2 or later; no module: the documents are stored as
strings there) and are SKIPPED, out loud, where those are missing. Restore,
check and remove write systemd units and are proved by hand on a box
(docs/ops.md, "Whole-instance backup and restore").

It is also the tamper tool the runbook's proof uses on a real archive:

    python3 -B deploy/backup/test_rmbak.py tamper <archive> <case> <out dir>

which writes a damaged copy of <archive> into <out dir> and prints its path.
The cases are TAMPER's keys. A `resealed-` case rewrites the manifest's file
checksums to match the damage, so only the facts can catch it; `refacted-`
also replaces the recorded facts with the damaged store's own, so only the
invariants can.
"""

import gzip
import hashlib
import importlib.util
import io
import json
import os
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("rmbak", os.path.join(HERE, "rmbak.py"))
rmbak = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rmbak)

HAVE_REDIS = bool(shutil.which("redis-server") and shutil.which("redis-cli"))
COMMIT = "0123456789abcdef0123456789abcdef01234567"


# --- damaging an archive ---------------------------------------------------------


def unpack(archive, into):
    manifest = rmbak.verify_archive(archive, extract_to=into)
    return manifest


def repack(stage, manifest, out_dir, name, reseal):
    """Write `stage` back as an archive. With `reseal`, the manifest's file list
    is recomputed from the stage, so every checksum matches the damage."""
    if reseal:
        files = []
        for directory, dirs, names in os.walk(stage):
            dirs.sort()
            for entry in sorted(names):
                path = os.path.join(directory, entry)
                rel = os.path.relpath(path, stage).replace(os.sep, "/")
                files.append({"path": rel, "size": os.path.getsize(path), "sha256": rmbak.sha256_file(path),
                              "mode": f"{stat.S_IMODE(os.stat(path).st_mode):04o}"})
        manifest["files"] = sorted(files, key=lambda row: row["path"])
    final = os.path.join(out_dir, name)
    # write_archive verifies what it wrote; a deliberately unsealed archive is
    # written by hand below instead.
    if reseal:
        rmbak.write_archive(stage, manifest, final)
        return final
    manifest_bytes = json.dumps(manifest, indent=1, sort_keys=True).encode() + b"\n"
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w|", format=tarfile.PAX_FORMAT) as tar:
        def add(member, data):
            info = tarfile.TarInfo(member)
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
        add("manifest.json", manifest_bytes)
        for row in manifest["files"]:
            path = os.path.join(stage, row["path"])
            if os.path.exists(path):
                with open(path, "rb") as handle:
                    add(row["path"], handle.read())
        for extra in sorted(os.listdir(stage)):
            if extra.startswith("unlisted"):
                with open(os.path.join(stage, extra), "rb") as handle:
                    add(extra, handle.read())
        add("manifest.sha256", f"{hashlib.sha256(manifest_bytes).hexdigest()}  manifest.json\n".encode())
    if final.endswith(".zst"):
        with open(final, "wb") as out:
            subprocess.run(["zstd", "-q", "-c"], input=raw.getvalue(), stdout=out, check=True)
    else:
        with gzip.open(final, "wb") as out:
            out.write(raw.getvalue())
    return final


def with_store(stage, manifest, change):
    """Run `change(client)` against the archive's snapshot and write it back."""
    rdb = os.path.join(stage, "redis", "dump.rdb")
    conf = os.path.join(stage, "config", f"redis-{manifest['instance']}mush.conf")
    scratch = rmbak.ScratchRedis(rdb, rmbak.loadmodule_args(conf))
    with scratch as client:
        change(client)
        client.call("SAVE")
        shutil.copyfile(os.path.join(scratch.dir, "dump.rdb"), rdb + ".new")
    os.replace(rdb + ".new", rdb)


def journal_keys(client):
    keys = [k for k in client.call("KEYS", "mod:economy:log:journal:*")]
    return sorted(keys, key=lambda k: int(k.rsplit(b":", 1)[1]))


def set_doc(client, key, change):
    """Rewrite one JSON document, whichever way this store holds it."""
    kind = client.call("TYPE", key)
    text = client.call("JSON.GET", key) if kind == b"ReJSON-RL" else client.call("GET", key)
    doc = json.loads(text)
    change(doc)
    text = json.dumps(doc, separators=(",", ":"))
    if kind == b"ReJSON-RL":
        client.call("JSON.SET", key, "$", text)
    else:
        client.call("SET", key, text)


def bump_balance(client):
    key = sorted(client.call("KEYS", "mod:economy:account:*"))[0]
    set_doc(client, key, lambda doc: doc.__setitem__("posted_balance", doc["posted_balance"] + 1000))


def drop_middle_chunk(client):
    keys = journal_keys(client)
    client.call("DEL", keys[len(keys) // 2])


def drop_tail_entry(client):
    set_doc(client, journal_keys(client)[-1], lambda doc: doc["entries"].pop())


def add_account(client):
    client.call("SET", "account:intruder", '{"name":"intruder"}')


def rewind_clock(client):
    def change(doc):
        doc["checkpoint"]["ic_unix"] -= 86400
    set_doc(client, "mod:rustmush_kernel_time:world_ic/state", change)


def flip_byte(path, offset_from_middle=0):
    with open(path, "r+b") as handle:
        handle.seek(os.path.getsize(path) // 2 + offset_from_middle)
        byte = handle.read(1)
        handle.seek(-1, os.SEEK_CUR)
        handle.write(bytes([byte[0] ^ 0x5A]))


def first_key_file(stage):
    for directory, _, names in os.walk(os.path.join(stage, "data")):
        for name in sorted(names):
            if name.endswith(".hex"):
                return os.path.join(directory, name)
    return os.path.join(stage, "data", "launch.json")


def drop_archive_segment(stage):
    for directory, _, names in os.walk(os.path.join(stage, "data", "archive")):
        for name in sorted(names):
            if name.endswith(".seg"):
                os.unlink(os.path.join(directory, name))
                return
    raise SystemExit("this archive's ledger archive tier holds no segment")


def flip_archive_segment(stage):
    for directory, _, names in os.walk(os.path.join(stage, "data", "archive")):
        for name in sorted(names):
            if name.endswith(".seg"):
                flip_byte(os.path.join(directory, name))
                return
    raise SystemExit("this archive's ledger archive tier holds no segment")


# case -> (what it does, damage(stage, manifest), reseal, refact, the words the refusal must carry)
TAMPER = {
    "flip-byte": ("one byte of the compressed file flipped", None, False, False, None),
    "truncate": ("the last third of the file cut off", None, False, False, None),
    "data-file": ("one byte of a key file changed, the manifest untouched",
                  lambda stage, m: flip_byte(first_key_file(stage)), False, False, "does not match its manifest entry"),
    "missing-file": ("the store's dump.rdb left out",
                     lambda stage, m: os.unlink(os.path.join(stage, "redis", "dump.rdb")), False, False,
                     "listed member(s) are missing"),
    "unlisted-file": ("an extra file slipped in",
                      lambda stage, m: open(os.path.join(stage, "unlisted-extra"), "w").write("x"), False, False,
                      "which the manifest does not list"),
    "resealed-balance": ("one account's balance raised by 1000 in the snapshot, every checksum made to match",
                         lambda stage, m: with_store(stage, m, bump_balance), True, False, "does not conserve"),
    "resealed-chunk": ("a middle chunk of the money journal deleted from the snapshot, every checksum made to match",
                       lambda stage, m: with_store(stage, m, drop_middle_chunk), True, False,
                       "in neither the store nor the archive tier"),
    "resealed-tail": ("the journal's last entry deleted from the snapshot, every checksum made to match",
                      lambda stage, m: with_store(stage, m, drop_tail_entry), True, False, "ledger economy/journal"),
    "resealed-account": ("a login added to the snapshot, every checksum made to match",
                         lambda stage, m: with_store(stage, m, add_account), True, False, "accounts is"),
    "resealed-clock": ("the IC clock's checkpoint moved back a day, every checksum made to match",
                       lambda stage, m: with_store(stage, m, rewind_clock), True, False, "clock is"),
    "resealed-key": ("one byte of a key file changed, every checksum made to match (a key is opaque: only the "
                     "boot can tell)", lambda stage, m: flip_byte(first_key_file(stage)), True, False, None),
    "resealed-segment-gone": ("a segment of the ledger archive tier removed, every checksum made to match",
                              lambda stage, m: drop_archive_segment(stage), True, False, "archive tier"),
    "resealed-segment-flip": ("one byte of a ledger archive segment flipped, every checksum made to match",
                              lambda stage, m: flip_archive_segment(stage), True, False, "archive tier"),
    "refacted-balance": ("the balance raised AND the manifest's facts replaced by the damaged store's own",
                         lambda stage, m: with_store(stage, m, bump_balance), True, True, "does not conserve"),
    "refacted-chunk": ("the chunk deleted AND the manifest's facts replaced by the damaged store's own",
                       lambda stage, m: with_store(stage, m, drop_middle_chunk), True, True,
                       "in neither the store nor the archive tier"),
}


def tamper(archive, case, out_dir):
    what, damage, reseal, refact, _ = TAMPER[case]
    name = os.path.basename(archive)
    os.makedirs(out_dir, exist_ok=True)
    target = os.path.join(out_dir, name)
    if damage is None:
        shutil.copyfile(archive, target)
        if case == "flip-byte":
            flip_byte(target)
        else:
            os.truncate(target, os.path.getsize(target) * 2 // 3)
        return target
    stage = tempfile.mkdtemp(prefix="rmbak-tamper-", dir=out_dir)
    try:
        manifest = unpack(archive, stage)
        damage(stage, manifest)
        if refact:
            conf = os.path.join(stage, "config", f"redis-{manifest['instance']}mush.conf")
            facts = rmbak.facts_of_rdb(os.path.join(stage, "redis", "dump.rdb"), os.path.join(stage, "data"),
                                       rmbak.loadmodule_args(conf))
            facts["problems"] = []
            manifest["facts"] = facts
        try:
            return repack(stage, manifest, out_dir, name, reseal)
        except rmbak.Fail:
            raise
    finally:
        shutil.rmtree(stage, ignore_errors=True)


# --- a tiny instance ----------------------------------------------------------------


def chunk(number, first_seq, count, sealed):
    return {"chunk": number, "sealed": sealed,
            "entries": [{"seq": first_seq + i, "value": {"tx": first_seq + i, "amount": 5}} for i in range(count)]}


def archive_tier(root, ns, log_id, chunks, purged):
    """Write chunks into the ledger archive tier's on-disk form (mush-store/src/archive.rs)."""
    directory = os.path.join(root, ns, log_id)
    os.makedirs(directory)
    numbers = sorted(chunks)
    segment = f"{numbers[0]:012d}-{numbers[-1]:012d}.seg"
    index, blob = {"ns": ns, "log_id": log_id, "chunks": {}}, b""
    for number in numbers:
        raw = json.dumps(chunks[number], separators=(",", ":")).encode()
        member = gzip.compress(raw, mtime=0)
        index["chunks"][str(number)] = {"segment": segment, "offset": len(blob), "len": len(member),
                                        "raw_len": len(raw), "sha256": hashlib.sha256(raw).hexdigest(),
                                        "archived_at": 1, "purged": purged}
        blob += member
    with open(os.path.join(directory, segment), "wb") as handle:
        handle.write(blob)
    with open(os.path.join(directory, segment[:-4] + ".idx.json"), "w") as handle:
        json.dump(index, handle)


class Instance:
    """An installed instance's files under a sysroot, and its Redis."""

    def __init__(self, root, name="t1"):
        self.name, self.sysroot = name, os.path.join(root, "sys")
        self.home = f"{self.sysroot}/opt/rustmush/{name}"
        self.data = f"{self.home}/data"
        build = f"{self.home}/build"
        for directory in (self.data, f"{build}/target/release", f"{build}/docs/help", f"{build}/deploy/instance",
                          f"{build}/web/dist", f"{build}/scripts/tests", f"{self.sysroot}/etc/redis", f"{self.sysroot}/etc/systemd/system",
                          f"{self.data}/offer-keys", f"{self.data}/logs", f"{self.data}/quarantine",
                          os.path.join(root, "live")):
            os.makedirs(directory)
        self.sock = os.path.join(root, "live", "r.sock")
        files = {
            f"{self.home}/instance.env": f"INSTANCE={name}\nDATA_DIR=/opt/rustmush/{name}/data\n"
                                         f"BUILD_DIR=/opt/rustmush/{name}/build\nREDIS_URL=redis+unix://{self.sock}\n",
            f"{self.home}/mushd.toml": '[server]\nname = "t1"\n',
            f"{build}/.source-commit": COMMIT + "\n",
            f"{build}/web/dist/.commit": COMMIT + "\n",
            f"{build}/web/dist/index.html": "<html></html>\n",
            f"{build}/scripts/check-web-bundle.sh": "#\n",
            f"{build}/scripts/tests/not-shipped.sh": "#\n",
            f"{build}/scripts/trade-audit.py": "#\n",
            f"{build}/target/release/mush-server": "server binary\n",
            f"{build}/target/release/mush-gateway": "gateway binary\n",
            f"{build}/docs/help/time.md": "help\n",
            f"{build}/deploy/instance/render.sh": "#\n",
            f"{self.sysroot}/etc/redis/redis-{name}mush.conf": "port 0\n",
            f"{self.sysroot}/etc/systemd/system/{name}mush-server.service": "[Unit]\n",
            f"{self.data}/ssh_host_ed25519": "host key\n",
            f"{self.data}/offer-keys/key-0000000001.hex": "aa" * 32 + "\n",
            f"{self.data}/launch.json": "{}\n",
            f"{self.data}/logs/server.log.2026-11-23": "a log line\n",
        }
        for path, text in files.items():
            with open(path, "w") as handle:
                handle.write(text)
        self.proc = subprocess.Popen(
            ["redis-server", "--port", "0", "--unixsocket", self.sock, "--dir", os.path.join(root, "live"),
             "--save", "", "--appendonly", "no", "--daemonize", "no"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        end = time.monotonic() + 30
        while True:
            try:
                self.client = rmbak.Redis(unix=self.sock)
                self.client.call("PING")
                break
            except OSError:
                if time.monotonic() > end:
                    raise
                time.sleep(0.1)
        c = self.client
        # A replica's snapshot waits this long for more replicas before it forks
        # (5 s by default): not something a test needs twelve times.
        c.call("CONFIG", "SET", "repl-diskless-sync-delay", "0")
        c.call("SET", "meta:next_id", "9")
        c.call("SET", "meta:schema_version", "3")
        for number in range(4):
            c.call("SET", f"obj:{number}", json.dumps({"id": number, "name": f"thing {number}"}))
        c.call("SET", "account:keeper", '{"name":"keeper"}')
        c.call("SET", "token:abc", "session", "EX", 3600)
        c.call("XADD", "mod:comsys:hist:1", "1-1", "line", "hello")
        for number, balance in ((1, -500), (2, 300), (3, 200)):
            c.call("SET", f"mod:economy:account:{number}",
                   json.dumps({"id": number, "instrument": "tacsim.credit", "posted_balance": balance}))
        # A journal of five chunks: 0 and 1 only in the archive tier (purged from
        # the store), 2 in both, 3 sealed in the store, 4 the open tail.
        chunks = {n: chunk(n, n * 3, 3, n < 4) for n in range(5)}
        for number in (2, 3, 4):
            c.call("SET", f"mod:economy:log:journal:{number}", json.dumps(chunks[number]))
        archive_tier(f"{self.data}/archive", "economy", "journal", {n: chunks[n] for n in (0, 1, 2)}, True)
        c.call("SET", "mod:rustmush_kernel_time:world_ic/state", json.dumps(
            {"schema_version": 1, "reanchors": [], "checkpoint": {"real_unix": 1795000000, "ic_unix": 74226209808,
                                                                  "generation": 1}}))

    def close(self):
        try:
            self.client.call("SHUTDOWN", "NOSAVE")
        except (OSError, rmbak.RedisError):
            pass
        self.client.close()
        self.proc.wait(timeout=30)


def conf_file(root, name="backup.conf", **values):
    path = os.path.join(root, name)
    with open(path, "w") as handle:
        for key, value in values.items():
            handle.write(f"{key}={value}\n")
    return path


# --- tests ---------------------------------------------------------------------------


class Pure(unittest.TestCase):
    def test_archive_names(self):
        good = "dev1-20261123T160000Z-ebe5840f.rmbak.tar.zst"
        self.assertTrue(rmbak.NAME_RE.match(good))
        for bad in ("../" + good, "dev1-20261123T160000Z-ebe5840f.tar.zst", "Dev1-20261123T160000Z-ebe5840f.rmbak.tar.zst",
                    good + ".x", "dev1-2026-ebe5840f.rmbak.tar.gz"):
            self.assertFalse(rmbak.NAME_RE.match(bad), bad)

    def test_chunk_keys(self):
        self.assertEqual(rmbak.parse_chunk_key("log:journal:12"), ("journal", 12))
        self.assertEqual(rmbak.parse_chunk_key("log:scene:42/x:0"), ("scene:42/x", 0))
        for bad in ("log:journal:01", "log::3", "account:1", "log:journal:x"):
            self.assertIsNone(rmbak.parse_chunk_key(bad), bad)

    def test_ic_date_is_the_game_calendar(self):
        self.assertEqual(rmbak.ic_date(0), "0000-01-01 00:00:00")
        self.assertEqual(rmbak.ic_date(74226209808), "2353-09-12 15:36:48")
        self.assertEqual(rmbak.ic_date(365 * 86400 - 1), "0000-12-31 23:59:59")

    def test_ledger_gaps_and_disagreements_are_problems(self):
        whole = {("economy", "journal"): {0: chunk(0, 0, 2, True), 1: chunk(1, 2, 2, False)}}
        problems = []
        facts = rmbak.ledger_facts(whole, {}, problems)
        self.assertEqual(problems, [])
        self.assertEqual((facts["economy/journal"]["first_seq"], facts["economy/journal"]["last_seq"]), (0, 3))
        cases = {
            "in neither": {("economy", "journal"): {0: chunk(0, 0, 2, True), 2: chunk(2, 4, 2, False)}},
            "is followed by": {("economy", "journal"): {0: chunk(0, 0, 2, True), 1: chunk(1, 3, 2, False)}},
            "is not sealed": {("economy", "journal"): {0: chunk(0, 0, 2, False), 1: chunk(1, 2, 2, False)}},
        }
        for words, store in cases.items():
            problems = []
            rmbak.ledger_facts(store, {}, problems)
            self.assertTrue(any(words in p for p in problems), (words, problems))
        problems = []
        other = chunk(0, 0, 2, True)
        other["entries"][0]["value"] = "changed"
        rmbak.ledger_facts(whole, {("economy", "journal"): {0: (other, False)}}, problems)
        self.assertTrue(any("differs between the store and the archive tier" in p for p in problems), problems)

    def test_a_prefix_digest_ignores_later_entries(self):
        short = {("e", "j"): {0: chunk(0, 0, 3, False)}}
        longer = {("e", "j"): {0: chunk(0, 0, 3, True), 1: chunk(1, 3, 3, False)}}
        want = rmbak.ledger_facts(short, {}, [])["e/j"]["digest"]
        self.assertEqual(rmbak.ledger_facts(longer, {}, [], upto={"e/j": 2})["e/j"]["digest"], want)
        self.assertNotEqual(rmbak.ledger_facts(longer, {}, [])["e/j"]["digest"], want)

    def test_toml_edit(self):
        text = '[server]\nname = "a"\n[web]\nlisten = "1.2.3.4:5"\ntls = { cert = "c" }\n[llm]\nenabled = true\n'
        out, missing = rmbak.toml_edit(text, {("web", "listen"): '"127.0.0.1:9"', ("llm", "enabled"): "false",
                                              ("ipc", "listen"): '"x"'}, drop={("web", "tls")})
        self.assertIn('listen = "127.0.0.1:9"', out)
        self.assertIn("enabled = false", out)
        self.assertNotIn("tls", out)
        self.assertEqual(missing, {("ipc", "listen")})

    def test_prune_keeps_the_newest_of_one_instance(self):
        with tempfile.TemporaryDirectory() as root:
            names = [f"dev1-2026112{d}T000000Z-aaaaaaaa.rmbak.tar.zst" for d in range(1, 6)]
            names.append("dev2-20261120T000000Z-aaaaaaaa.rmbak.tar.zst")
            for name in names + ["notes.txt"]:
                open(os.path.join(root, name), "w").close()
            removed = rmbak.prune(root, "dev1", 2)
            self.assertEqual(removed, names[:3])
            self.assertEqual(sorted(os.listdir(root)), sorted(names[3:] + ["notes.txt"]))

    def test_conf_refuses_what_it_does_not_know(self):
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaises(rmbak.Fail):
                rmbak.load_conf(conf_file(root, KEEEP="3"))
            with self.assertRaises(rmbak.Fail):
                rmbak.load_conf(conf_file(root, KEEP="0"))
            with self.assertRaises(rmbak.Fail):
                rmbak.load_conf(conf_file(root, SECRETS="maybe"))
            self.assertEqual(rmbak.load_conf(conf_file(root, KEEP="7"))["KEEP"], "7")

    def test_the_example_conf_is_the_defaults(self):
        example = rmbak.load_conf(os.path.join(HERE, "backup.conf.example"))
        self.assertEqual(example, rmbak.CONF_DEFAULTS)
        with open(os.path.join(HERE, "backup.conf.example"), encoding="utf-8") as handle:
            written = {line.split("=", 1)[0] for line in handle if "=" in line and not line.startswith("#")}
        self.assertEqual(written, set(rmbak.CONF_DEFAULTS), "every key is written out, with its default")

    def test_the_timer_is_every_four_hours_and_clear_of_the_archive_pass(self):
        with open(os.path.join(HERE, "rustmush-backup.timer"), encoding="utf-8") as handle:
            lines = [line.strip() for line in handle if line.startswith("OnCalendar=")]
        self.assertEqual(lines, ["OnCalendar=*-*-* 00/4:20:00"], "one OnCalendar line: the interval's one place")
        with open(os.path.join(HERE, "rustmush-backup.service"), encoding="utf-8") as handle:
            unit = handle.read()
        for wanted in ("Type=oneshot", "rmbak.py run", "PrivateNetwork=true", "ReadWritePaths=/var/backups/rustmush"):
            self.assertIn(wanted, unit)


@unittest.skipUnless(HAVE_REDIS, "redis-server and redis-cli are not on PATH: the backup cycle is NOT tested here")
class Cycle(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.root = tempfile.mkdtemp(prefix="rmbak-test-")
        os.environ["RMBAK_SYSROOT"] = os.path.join(cls.root, "sys")
        cls.instance = Instance(cls.root)
        cls.dest = os.path.join(cls.root, "backups")
        cls.conf = rmbak.load_conf(conf_file(cls.root, BACKUP_DIR=cls.dest, INSTANCES="t1", KEEP="2"))
        cls.archive = rmbak.do_backup("t1", cls.conf)
        cls.manifest = rmbak.verify_archive(cls.archive)

    @classmethod
    def tearDownClass(cls):
        cls.instance.close()
        os.environ.pop("RMBAK_SYSROOT", None)
        shutil.rmtree(cls.root, ignore_errors=True)

    def deep(self, archive):
        """`verify --deep`: the differences, or raise what verify raises."""
        stage = tempfile.mkdtemp(prefix="deep-", dir=self.root)
        try:
            manifest = rmbak.verify_archive(archive, extract_to=stage)
            facts = rmbak.facts_of_rdb(os.path.join(stage, "redis", "dump.rdb"), os.path.join(stage, "data"), [])
            return rmbak.compare_facts(manifest["facts"], facts, "loaded") + facts["problems"]
        finally:
            shutil.rmtree(stage, ignore_errors=True)

    def test_the_archive_is_whole_private_and_named(self):
        self.assertTrue(rmbak.NAME_RE.match(os.path.basename(self.archive)))
        self.assertEqual(stat.S_IMODE(os.stat(self.archive).st_mode) & 0o007, 0)
        left = [n for n in os.listdir(self.dest) if n.startswith((".stage-", ".partial-", ".verify-"))]
        self.assertEqual(left, [])
        paths = {row["path"] for row in self.manifest["files"]}
        for wanted in ("redis/dump.rdb", "data/ssh_host_ed25519", "data/offer-keys/key-0000000001.hex",
                       "data/launch.json", "config/instance.env", "config/mushd.toml", "config/redis-t1mush.conf",
                       "config/units/t1mush-server.service", "build/.source-commit",
                       "build/target/release/mush-server", "build/docs/help/time.md",
                       "build/web/dist/index.html", "build/scripts/check-web-bundle.sh",
                       "data/archive/economy/journal/000000000000-000000000002.seg"):
            self.assertIn(wanted, paths)
        self.assertFalse(any(p.startswith("data/logs/") for p in paths), "logs are never archived")
        self.assertFalse(any(p.startswith(("build/scripts/tests/", "build/scripts/trade")) for p in paths))
        self.assertIn("data/quarantine", [row["path"] for row in self.manifest["dirs"]], "an empty directory is kept")

    def test_tree_lays_what_a_deploy_reads_and_only_where_there_is_none(self):
        class Args:
            pass
        Args.archive, Args.to = self.archive, os.path.join(self.root, "laid", "build")
        rmbak.cmd_tree(Args)
        for wanted in (".source-commit", "web/dist/.commit", "web/dist/index.html", "scripts/check-web-bundle.sh",
                       "deploy/instance/render.sh", "docs/help/time.md", "target/release/mush-gateway"):
            self.assertTrue(os.path.isfile(os.path.join(Args.to, wanted)), wanted)
        self.assertTrue(os.stat(os.path.join(Args.to, "target/release/mush-server")).st_mode & 0o100)
        self.assertFalse(os.path.exists(os.path.join(Args.to, "data")), "only the build slice is laid")
        with self.assertRaises(rmbak.Fail):
            rmbak.cmd_tree(Args)

    def test_the_facts_are_the_snapshots(self):
        facts = self.manifest["facts"]
        self.assertEqual(facts["problems"], [])
        self.assertEqual((facts["objects"], facts["accounts"], facts["next_id"]), (4, 1, 9))
        self.assertEqual(facts["volatile_keys"], 1)
        journal = facts["ledgers"]["economy/journal"]
        self.assertEqual((journal["first_seq"], journal["last_seq"], journal["chunks"]), (0, 14, 5))
        self.assertEqual((journal["chunks_in_store"], journal["chunks_only_in_archive"]), (3, 2))
        self.assertEqual(facts["economy"]["instruments"]["tacsim.credit"], {"accounts": 3, "sum_posted": 0})
        self.assertEqual(self.manifest["snapshot"]["ic_checkpoint"], "2353-09-12 15:36:48")
        self.assertEqual(self.deep(self.archive), [])

    def test_every_damage_is_caught(self):
        out = os.path.join(self.root, "tampered")
        for case, (what, _, reseal, _, words) in sorted(TAMPER.items()):
            with self.subTest(case=case):
                shutil.rmtree(out, ignore_errors=True)
                damaged = tamper(self.archive, case, out)
                if not reseal:
                    with self.assertRaises(rmbak.Fail) as caught:
                        rmbak.verify_archive(damaged)
                    if words:
                        self.assertIn(words, str(caught.exception))
                    continue
                rmbak.verify_archive(damaged)  # its checksums all match: only the facts can tell
                found = self.deep(damaged)
                if words is None:
                    # A key file is opaque bytes: nothing short of the boot reads it.
                    self.assertEqual(found, [], case)
                else:
                    self.assertTrue(any(words in line for line in found), (case, what, found))

    def test_secrets_kept_apart_are_listed_not_carried(self):
        dest = os.path.join(self.root, "apart")
        conf = dict(self.conf, SECRETS="apart", BINARIES="no")
        archive = rmbak.do_backup("t1", conf, dest)
        manifest = rmbak.verify_archive(archive)
        paths = {row["path"] for row in manifest["files"]}
        self.assertNotIn("data/ssh_host_ed25519", paths)
        self.assertNotIn("data/offer-keys/key-0000000001.hex", paths)
        self.assertNotIn("build/target/release/mush-server", paths)
        apart = {row["path"]: row for row in manifest["key_files"]["kept_apart"]}
        self.assertEqual(set(apart), {"data/ssh_host_ed25519", "data/offer-keys/key-0000000001.hex"})
        self.assertEqual(apart["data/ssh_host_ed25519"]["sha256"], hashlib.sha256(b"host key\n").hexdigest())
        self.assertFalse(manifest["binaries"]["mush-server"]["in_archive"])
        # The files come back from a directory kept elsewhere, and only the right ones.
        stage, target = os.path.join(self.root, "st"), os.path.join(self.root, "placed")
        os.makedirs(stage)
        rmbak.verify_archive(archive, extract_to=stage)
        with self.assertRaises(rmbak.Fail):
            rmbak.place_data(stage, manifest, target, None, False)
        wrong = os.path.join(self.root, "wrong-secrets")
        shutil.copytree(self.instance.data, wrong)
        with open(os.path.join(wrong, "ssh_host_ed25519"), "w") as handle:
            handle.write("another key\n")
        with self.assertRaises(rmbak.Fail):
            rmbak.place_data(stage, manifest, target, wrong, False)
        self.assertFalse(os.path.exists(target), "nothing is placed before the secrets are checked")
        rmbak.place_data(stage, manifest, target, self.instance.data, False)
        with open(os.path.join(target, "ssh_host_ed25519")) as handle:
            self.assertEqual(handle.read().strip(), "host key")

    def test_a_cut_is_taken_again_when_the_files_moved(self):
        real, calls = rmbak.snapshot_store, []

        def moving(env, stage):
            calls.append(1)
            if len(calls) == 1:
                # A ledger archive pass lands a file between the fork and the copy.
                with open(os.path.join(self.instance.data, "archive", "landed.tmp"), "w") as handle:
                    handle.write("x")
            return real(env, stage)
        rmbak.snapshot_store, sleep = moving, time.sleep
        time.sleep = lambda _: None
        try:
            manifest = rmbak.verify_archive(rmbak.do_backup("t1", self.conf, os.path.join(self.root, "moved")))
        finally:
            rmbak.snapshot_store, time.sleep = real, sleep
            os.unlink(os.path.join(self.instance.data, "archive", "landed.tmp"))
        self.assertEqual(manifest["snapshot"]["cut_attempts"], 2)
        self.assertIn({"path": "data/archive/landed.tmp", "why": "a writer's temp file"}, manifest["skipped"])

    def test_one_backup_at_a_time(self):
        import fcntl
        dest = os.path.join(self.root, "locked")
        os.makedirs(dest)
        with open(os.path.join(dest, ".lock"), "w") as held:
            fcntl.flock(held, fcntl.LOCK_EX)
            with self.assertRaises(rmbak.Fail) as caught:
                rmbak.do_backup("t1", self.conf, dest)
        self.assertIn("one backup at a time", str(caught.exception))
        self.assertEqual([n for n in os.listdir(dest) if n != ".lock"], [])

    def test_a_snapshot_that_breaks_an_invariant_fails_loudly_and_is_still_written(self):
        dest = os.path.join(self.root, "broken")
        self.instance.client.call("SET", "mod:economy:account:9",
                                  json.dumps({"id": 9, "instrument": "tacsim.credit", "posted_balance": 7}))
        try:
            with self.assertRaises(rmbak.Fail) as caught:
                rmbak.do_backup("t1", self.conf, dest)
        finally:
            self.instance.client.call("DEL", "mod:economy:account:9")
        self.assertIn("breaks 1 invariant", str(caught.exception))
        self.assertEqual(len(rmbak.archives_in(dest)), 1)

    def test_an_incident_is_evidence_and_never_prunes_a_good_backup(self):
        dest = os.path.join(self.root, "incident")

        class Args:
            pass
        Args.conf = conf_file(self.root, "incident.conf", BACKUP_DIR=dest, INSTANCES="t1", KEEP="1")
        rmbak.cmd_run(Args)
        good = rmbak.archives_in(dest)
        self.assertEqual(len(good), 1)
        # A record in the quarantine spool: the instance is in an integrity incident.
        record = os.path.join(self.instance.data, "quarantine", "seal-1.json")
        with open(record, "w") as handle:
            handle.write("{}")
        time.sleep(1.1)
        try:
            with self.assertRaises(rmbak.Fail):
                rmbak.cmd_run(Args)
        finally:
            os.unlink(record)
        have = rmbak.archives_in(dest)
        self.assertEqual(len(have), 2, "KEEP is 1, and nothing is pruned behind a failed backup")
        self.assertIn(good[0], have)
        with open(os.path.join(dest, "status.json")) as handle:
            self.assertIn("integrity incident", json.load(handle)["instances"]["t1"]["error"])
        time.sleep(1.1)
        rmbak.cmd_run(Args)  # healthy again: pruning resumes
        self.assertEqual(len(rmbak.archives_in(dest)), 1)

    def test_run_prunes_and_says_how_it_went(self):
        dest = os.path.join(self.root, "run")
        conf = conf_file(self.root, "run.conf", BACKUP_DIR=dest, INSTANCES="t1 ghost", KEEP="2")

        class Args:
            pass
        Args.conf = conf
        for _ in range(3):
            with self.assertRaises(rmbak.Fail) as caught:  # `ghost` is not installed: loud, and t1 still runs
                rmbak.cmd_run(Args)
            self.assertIn("ghost", str(caught.exception))
            time.sleep(1.1)
        self.assertEqual(len(rmbak.archives_in(dest, "t1")), 2)
        with open(os.path.join(dest, "status.json")) as handle:
            status = json.load(handle)
        self.assertTrue(status["instances"]["t1"]["ok"])
        self.assertEqual(status["instances"]["t1"]["kept"], 2)
        self.assertFalse(status["instances"]["ghost"]["ok"])
        self.assertIn("no instance.env", status["instances"]["ghost"]["error"])

    def test_serve_and_pull(self):
        serve = os.path.join(HERE, "backup-serve.sh")
        served = os.path.join(self.root, "served")
        os.makedirs(served)
        name = os.path.basename(self.archive)
        shutil.copyfile(self.archive, os.path.join(served, name))
        with open(os.path.join(served, "status.json"), "w") as handle:
            json.dump({"instances": {"t1": {"ok": True}}}, handle)
        with open(os.path.join(served, "secret.txt"), "w") as handle:
            handle.write("not an archive")
        conf = conf_file(self.root, "serve.conf", BACKUP_DIR=served)

        def ask(request):
            env = dict(os.environ, SSH_ORIGINAL_COMMAND=request, RUSTMUSH_BACKUP_CONF=conf)
            return subprocess.run(["bash", serve], env=env, capture_output=True)
        listing = ask("list")
        self.assertEqual(listing.stdout.decode().split(), [str(os.path.getsize(self.archive)), name])
        with open(self.archive, "rb") as handle:
            self.assertEqual(ask(f"fetch {name}").stdout, handle.read())
        for request in ("", "fetch secret.txt", "fetch ../sys/etc/redis/redis-t1mush.conf", f"fetch {name} extra",
                        "fetch /etc/passwd", "list x", "sh -c id", f"fetch {name};id", "scp -f /etc/passwd"):
            refused = ask(request)
            self.assertEqual((refused.returncode, refused.stdout), (2, b""), request)
        # The pull, through the same script.
        shim = os.path.join(self.root, "remote")
        with open(shim, "w") as handle:
            handle.write(f'#!/bin/sh\nSSH_ORIGINAL_COMMAND="$*" RUSTMUSH_BACKUP_CONF={conf} exec bash {serve}\n')
        os.chmod(shim, 0o755)
        pulled = os.path.join(self.root, "pulled")

        class Args:
            pass
        Args.conf = conf_file(self.root, "pull.conf", PULL_SSH=shim, PULL_DIR=pulled, PULL_KEEP="5", MAX_AGE_HOURS="9")
        rmbak.cmd_pull(Args)
        self.assertEqual(rmbak.archives_in(pulled), [name])
        rmbak.cmd_pull(Args)  # nothing new: quiet
        # A far side that failed, or went quiet, is loud here.
        with open(os.path.join(served, "status.json"), "w") as handle:
            json.dump({"instances": {"t1": {"ok": False, "error": "disk full"}}}, handle)
        with self.assertRaises(rmbak.Fail) as caught:
            rmbak.cmd_pull(Args)
        self.assertIn("disk full", str(caught.exception))
        old = "t1-20200101T000000Z-01234567.rmbak.tar." + name.rsplit(".", 1)[1]
        os.rename(os.path.join(pulled, name), os.path.join(pulled, old))
        os.rename(os.path.join(served, name), os.path.join(served, "gone"))
        with open(os.path.join(served, "status.json"), "w") as handle:
            json.dump({"instances": {"t1": {"ok": True}}}, handle)
        with self.assertRaises(rmbak.Fail) as caught:
            rmbak.cmd_pull(Args)
        self.assertIn("h old", str(caught.exception))
        # A damaged transfer is never kept.
        os.unlink(os.path.join(pulled, old))
        shutil.copyfile(tamper(self.archive, "flip-byte", os.path.join(self.root, "flipped")),
                        os.path.join(served, name))
        with self.assertRaises(rmbak.Fail):
            rmbak.cmd_pull(Args)
        self.assertEqual(rmbak.archives_in(pulled), [])


if __name__ == "__main__":
    if len(sys.argv) >= 2 and sys.argv[1] == "tamper":
        if len(sys.argv) != 5 or sys.argv[3] not in TAMPER:
            print("usage: test_rmbak.py tamper <archive> <case> <out dir>\ncases:", file=sys.stderr)
            for key, row in sorted(TAMPER.items()):
                print(f"  {key:24} {row[0]}", file=sys.stderr)
            sys.exit(2)
        print(tamper(os.path.abspath(sys.argv[2]), sys.argv[3], os.path.abspath(sys.argv[4])))
        sys.exit(0)
    if not HAVE_REDIS:
        print("test_rmbak: redis-server and redis-cli are not on PATH: the backup cycle is NOT tested here",
              file=sys.stderr)
    unittest.main()
