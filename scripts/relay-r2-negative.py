#!/usr/bin/env python3
"""R2 fault injection on the owner's disposable dev1 only; always restore records."""
import copy
import json
import pathlib
import subprocess
import time

ROOT = pathlib.Path("/opt/rustmush/dev1")
assert ROOT.resolve() == ROOT
OUT = ROOT / "r2-evidence"
STATE = "mod:propagation:relay_transport"


def call(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, capture_output=True, **kwargs).stdout.strip()


def get(key):
    raw = call("redis-cli", "-p", "6788", "--raw", "JSON.GET", key)
    return json.loads(raw) if raw else None


def put(key, doc):
    if doc is None:
        call("redis-cli", "-p", "6788", "DEL", key)
    else:
        call("redis-cli", "-p", "6788", "-x", "JSON.SET", key, "$", input=json.dumps(doc))


def unit(action):
    call("systemctl", action, "dev1mush-gateway")


unit("stop")
good = get(STATE)
assert good and good["version"] == 1
keys = call("redis-cli", "-p", "6788", "--scan", "--pattern", "mod:propagation:envelope:*").splitlines()
record_key = next(k for k in keys if get(k).get("transit", {}).get("history"))
original = get(record_key)
(OUT / "pre-negative-transport.json").write_text(json.dumps(good, indent=2) + "\n")
rows = []
try:
    for case in ["invalid_capacity", "missing_capacity", "invalid_cursor", "invalid_clock"]:
        key = STATE if "capacity" in case else record_key
        bad = copy.deepcopy(good if key == STATE else original)
        if case == "invalid_capacity":
            bad["version"] = 999
        elif case == "missing_capacity":
            bad = None
        elif case == "invalid_cursor":
            bad["transit"]["next"] = 999
        else:
            # A completed conventional history stamp may not change generation.
            bad["transit"]["history"][0]["arrived"]["clock"]["generation"] = 999
        put(key, bad)
        cursor = call("journalctl", "-u", "dev1mush-gateway", "-n", "1", "--show-cursor",
                      "--no-pager", "-o", "cat").rsplit("-- cursor: ", 1)[1].strip()
        unit("start")
        diagnostic = []
        for _ in range(40):
            journal = call("journalctl", "-u", "dev1mush-gateway", "--after-cursor", cursor,
                           "--no-pager", "-o", "cat")
            if key == STATE:
                diagnostic = [s for s in journal.splitlines()
                              if "relay transport" in s or "relay_transport missing" in s]
            else:
                diagnostic = [s for s in journal.splitlines()
                              if "held, not deleted" in s and record_key.removeprefix("mod:propagation:") in s]
            if diagnostic:
                break
            time.sleep(.5)
        unit("stop")
        assert diagnostic, "missing diagnostic: " + case
        assert get(key) == bad, "injected evidence was overwritten: " + case
        rows.append({"case": case, "passed": True, "diagnostic": diagnostic[-1]})
        (OUT / "negative-cases.json").write_text(json.dumps(rows, indent=2) + "\n")
        put(key, good if key == STATE else original)
finally:
    unit("stop")
    put(STATE, good)
    put(record_key, original)
    unit("start")
print(json.dumps({"passed": len(rows) == 4, "cases": len(rows)}))
