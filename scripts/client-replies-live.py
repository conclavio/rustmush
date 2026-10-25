#!/usr/bin/env python3
"""Real dev1 reply framing, asynchronous staff completion, and hot restart."""
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time

spec = importlib.util.spec_from_file_location("client_ssh", Path(__file__).with_name("client-ssh-live.py"))
ssh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ssh)
result = {"passed": False, "checks": [], "transcripts": []}
start, end = "__REPLY_BEGIN__", "__REPLY_END__"
term = None


def command(line, contains=None):
    term.line(line)
    raw, text = term.read(lambda raw, text: end in text)
    markers = re.findall(start + "|" + end, text)
    assert markers == [start, end], (line, markers, text[-1200:])
    if contains is not None:
        assert contains in text, (line, text[-1200:])
        assert text.index(start) < text.index(contains) < text.index(end), (line, text[-1200:])
    result["transcripts"].append({"command": line, "reply": text})
    return raw, text


try:
    ssh.wait_ready()
    term = ssh.Terminal(False)
    term.line("OUTPUTSUFFIX " + end)
    term.read(lambda raw, text: end in text)
    term.line("OUTPUTPREFIX " + start)
    _, text = term.read(lambda raw, text: end in text)
    assert start not in text, "a newly set prefix does not prefix its own command"
    command("think world-reply-probe", "world-reply-probe")
    command("@stats")
    command("@ps")
    command("@siteban/list")
    command("@sshkey/list")
    command("@dump", "snapshot acknowledged by the store")
    command("+act broken with []", "parameters")
    result["checks"].append("world, local staff, asynchronous store, and gateway action replies close after their bodies")

    token = "reply-history-" + str(time.time_ns())
    command("say " + token, token)
    command("@history 1", token)
    command("@pixel/encoder")
    command("@pixel/stats")
    command("@sounds off")
    command("@sounds on")
    raw, _ = command("@tui")
    assert b"\x1b[?1049h" in raw
    term.line("")
    raw, _ = command("@notui")
    assert b"\x1b[?1049l" in raw
    command("@history 1", token)
    result["checks"].append("SSH-local history, pixel, sound, and TUI transition replies are framed")

    term.line("@stats\r@siteban/list\rthink pipelined-last")
    _, text = term.read(lambda raw, text: text.count(end) >= 3)
    assert re.findall(start + "|" + end, text) == [start, end] * 3, text[-1800:]
    assert text.rfind(start) < text.index("pipelined-last") < text.rfind(end)
    result["transcripts"].append({"pipelined": text})
    result["checks"].append("three pipelined commands produce sequential, unnested pairs")

    gateway = int(subprocess.check_output(["systemctl", "show", "--property=MainPID", "--value", "dev1mush-gateway"]))
    children = subprocess.check_output(["pgrep", "-P", str(gateway), "-x", "mush-server"], text=True).split()
    assert len(children) == 1, children
    old_server = int(children[0])
    os.kill(old_server, signal.SIGKILL)
    term.read(lambda raw, text: "restarting" in text)
    ssh.wait_ready()
    command("think after-hot-restart", "after-hot-restart")
    assert int(subprocess.check_output(["systemctl", "show", "--property=MainPID", "--value", "dev1mush-gateway"])) == gateway
    result["checks"].append("same SSH connection retains both delimiters through a real server SIGKILL/relaunch")
    result["restart"] = {"gateway_pid": gateway, "old_server_pid": old_server}

    command("OUTPUTPREFIX")
    term.line("OUTPUTSUFFIX")
    term.line("think cleared-delimiters")
    _, text = term.read(lambda raw, text: "cleared-delimiters" in text)
    assert start not in text and end not in text
    result["checks"].append("clearing delimiters uses the old prefix and new suffix semantics")
    term.line("OUTPUTSUFFIX " + end)
    term.read(lambda raw, text: end in text)
    term.line("OUTPUTPREFIX " + start)
    term.read(lambda raw, text: end in text)
    command("QUIT", "Goodbye.")
    term.proc.wait(timeout=15)
    assert term.proc.returncode == 0
    result["checks"].append("QUIT farewell and suffix drain before clean channel close")
    result["passed"] = True
finally:
    if term is not None:
        term.close()
    (ssh.OUT / "reply-live.json").write_text(json.dumps(result, indent=2) + "\n")
print("CLIENT_REPLIES_LIVE_PASSED")
