#!/usr/bin/env python3
"""Doc 51 §2: the census of waits in test code.

Finds every wait primitive in the Rust test code under crates/ and says, for each
one, its shape (how it waits) and its class (what it waits for). The counts in doc
51 §2 are this script's output; rerun it to check them or to see them move.

    python scripts/wait_audit.py                 # the tables of doc 51 §2
    python scripts/wait_audit.py --dump          # one TSV row a site, for reading
    python scripts/wait_audit.py --class clock   # the sites of one class
    python scripts/wait_audit.py --literals      # the literal-span census
    python scripts/wait_audit.py --budget        # what the wait_budget gate counts, a row a file
    python scripts/wait_audit.py --src           # ...with the test code under src/ (minutes)

What is scanned. Every .rs file under crates/<pkg>/tests/ (the integration
binaries and their rigs). With --src it also scans crates/<pkg>/src/: a file in a
`tests` directory or named tests.rs or *_tests.rs is test code whole, and any other
file is test code from its first `#[cfg(test)]` to its end (the house layout puts
the unit-test module last). Comments and string literals are blanked first, so a
needle in prose is not a site.

What is a site. A call of one of: sleep( (tokio's, std's or bare), timeout(,
timeout_at(, sleep_until(, recv_timeout(, wait_timeout(. A method call .timeout(
is a client's configuration, not a wait, and is not counted.

--budget prints what doc 51's ratchet counts (the `wait_budget` gate in
crates/mush-contract-tests/tests/ci_policy/wait_budget.rs): in story files only,
outside any `start_paused` test, the raw waits above (`wait`) and the wall-clock
reads unix_now_s(, unix_now_ms( and SystemTime::now( (`clock`). One row a file
and counter, `<counter> <count> <crate>/<path>`, then the totals by crate. The
gate's budget files must hold the same counts. For the crates of the gate's
`src` scope (SRC_SCOPE below, doc 51 cut FL.6) it also counts the test code under
src/ by the gate's exact rule, which is narrower than --src's: a file in a
`tests` directory, named tests.rs or *_tests.rs, or fenced by a leading
#![cfg(test)] is test code whole; in any other file only the #[cfg(test)] items
count, brace-counted from the attribute to its close.

Shape, mechanical:
  fixed    a sleep outside any loop: the test waits a span and goes on
  poll     a sleep inside a loop: the loop asks again until something holds
  bound    a timeout around a future: the future is the wait, the span its limit
A site inside a `#[tokio::test(... start_paused = true ...)]` function is marked
`paused`: its span is virtual time the test drives, not wall time.

Class, by rule (the RULES table below; first match wins). The rules read the
site's own statement, the lines around it and the function it is in:
  figure   a span or distance that restates a game figure (a cycle, a window)
  clock    a clock the test does not drive (the arena's wake, a world tick, a
           tokio timer in the code under test)
  stream   a reply read from a stream (SSH channel, web socket, pipe, child's
           stdout), where the wait ends on a needle or on silence
  process  a real process or listener starting, stopping or binding
  event    already bounded by an event: a channel, a join handle, a notify, a
           future of the code under test; the span is only the failure limit
  driven   virtual time on a paused clock
A site no rule names takes its shape's default: a bound is `event` (the future
it wraps is the event), a poll or a bare sleep is `clock`. "By rule" at the end of
the report says how many sites each rule and each default carried. The rules are
heuristics over text. They size the classes and find their members; they are not
a proof about any one site.

Rig or story. A file with /support/ in its path, or named harness.rs, is a rig;
every other test file is a story. Doc 51's ratchet counts stories only.

--literals counts `Duration::from_*(<number>)` in test code: the spans a test
writes as a literal. `in a wait` is one on a wait site's own lines; `elsewhere`
is one bound to a constant, passed to a rig helper or set in a config.
"""
import argparse
import os
import re
import sys
from collections import Counter, defaultdict

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# The dot is its own alternative, so `x.timeout(` matches at the dot whatever comes
# before it. (Until doc 51 cut FL.1 the look-behind stood before the optional dot,
# and `x.timeout(` was read as a bare `timeout(` and counted; the tree held none.)
NEEDLE = re.compile(
    r"(?:(?P<dot>\.)|(?<![A-Za-z0-9_]))(?P<name>sleep_until|sleep|timeout_at|timeout|recv_timeout|wait_timeout)\s*\("
)
# A wall-clock read (doc 51 §3.9). The empty `dot` group keeps scan_file's shape.
CLOCK = re.compile(r"(?<![A-Za-z0-9_])(?P<dot>)(?P<name>unix_now_s|unix_now_ms|SystemTime\s*::\s*now)\s*\(")
LITERAL = re.compile(r"Duration::from_(secs|millis|micros|secs_f32|secs_f64)\(\s*([0-9][0-9_\.]*)\s*\)")
FN = re.compile(r"\bfn\s+([A-Za-z0-9_]+)")
LOOP = re.compile(r"(?<![A-Za-z0-9_])(loop|while|for)(?![A-Za-z0-9_])")


def blank(source):
    """Comments and string/char literals replaced by spaces, newlines kept."""
    out = []
    i, n = 0, len(source)
    while i < n:
        c = source[i]
        two = source[i : i + 2]
        if two == "//":
            j = source.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
        elif two == "/*":
            depth, j = 1, i + 2
            while j < n and depth:
                if source[j : j + 2] == "/*":
                    depth, j = depth + 1, j + 2
                elif source[j : j + 2] == "*/":
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            out.append("".join(ch if ch == "\n" else " " for ch in source[i:j]))
            i = j
        elif c == '"' or (c == "r" and re.match(r'r#*"', source[i:]) and not (i and (source[i - 1].isalnum() or source[i - 1] == "_"))):
            if c == "r":
                m = re.match(r'r(#*)"', source[i:])
                close = '"' + m.group(1)
                j = source.find(close, i + len(m.group(0)))
                j = n if j < 0 else j + len(close)
            else:
                j = i + 1
                while j < n and source[j] != '"':
                    j += 2 if source[j] == "\\" else 1
                j = min(n, j + 1)
            out.append('"' + "".join(ch if ch == "\n" else " " for ch in source[i + 1 : j - 1]) + '"' if j - i >= 2 else source[i:j])
            i = j
        elif c == "'":
            m = re.match(r"'(\\.[^']*|[^'\\])'", source[i:])
            if m:
                out.append("' '" + " " * (len(m.group(0)) - 3) if len(m.group(0)) >= 3 else m.group(0))
                i += len(m.group(0))
            else:
                out.append(c)
                i += 1
        else:
            out.append(c)
            i += 1
    return "".join(out)


class Site:
    __slots__ = ("crate", "path", "line", "name", "shape", "paused", "fn", "text", "ctx", "cls", "rule", "scope")


def scan_file(crate, rel, source, start_line=0, scope="tests", pattern=NEEDLE):
    """The sites of one file. `start_line`: ignore sites above it (src files)."""
    code = blank(source)
    raw_lines = source.split("\n")
    sites = []
    # Block stack: (kind, fn_name, paused). kind is "loop", "fn" or "".
    stack = []
    header_start = 0
    line_of = [0]
    for m in re.finditer("\n", code):
        line_of.append(m.end())

    def line_no(pos):
        lo, hi = 0, len(line_of) - 1
        while lo < hi:
            mid = (lo + hi + 1) // 2
            if line_of[mid] <= pos:
                lo = mid
            else:
                hi = mid - 1
        return lo + 1

    needles = {m.start(): m for m in pattern.finditer(code)}
    i, n = 0, len(code)
    while i < n:
        c = code[i]
        if c == "{":
            header = code[header_start:i]
            fn = FN.findall(header)
            kind, name, paused = "", None, False
            if fn:
                kind, name = "fn", fn[-1]
                paused = "start_paused" in header
            elif LOOP.search(header.split(";")[-1]):
                kind = "loop"
            stack.append((kind, name, paused))
            header_start = i + 1
        elif c == "}":
            if stack:
                stack.pop()
            header_start = i + 1
        elif c == ";":
            # A `#[tokio::test(...)]` attribute sits before `fn`, with no `;` between.
            header_start = i + 1
        if i in needles:
            m = needles[i]
            ln = line_no(i)
            if not m.group("dot") or m.group("name") != "timeout":
                if ln > start_line:
                    s = Site()
                    s.crate, s.path, s.line, s.name, s.scope = crate, rel, ln, m.group("name"), scope
                    in_loop = False
                    s.fn, s.paused = "", False
                    for kind, name, paused in reversed(stack):
                        if kind == "loop":
                            in_loop = True
                        if kind == "fn":
                            s.fn, s.paused = name, paused
                            break
                    # A closure or async block in a loop still polls; one in a fn does not leave it.
                    if s.name in ("sleep", "sleep_until"):
                        s.shape = "poll" if in_loop else "fixed"
                    else:
                        s.shape = "bound"
                    s.text = " ".join(raw_lines[ln - 1 : ln + 2]).strip()
                    s.ctx = "\n".join(raw_lines[max(0, ln - 9) : ln + 6])
                    sites.append(s)
        i += 1
    return sites


def test_start(source):
    """Line of the first `#[cfg(test)]` in a src file, or None."""
    m = re.search(r"^\s*#\[cfg\((all\()?test\b", source, re.M)
    return source.count("\n", 0, m.start()) if m else None


def walk(include_src):
    crates = os.path.join(ROOT, "crates")
    for crate in sorted(os.listdir(crates)):
        base = os.path.join(crates, crate)
        for sub in ("tests", "src") if include_src else ("tests",):
            top = os.path.join(base, sub)
            for dirpath, _dirs, files in os.walk(top):
                for f in sorted(files):
                    if not f.endswith(".rs"):
                        continue
                    full = os.path.join(dirpath, f)
                    rel = os.path.relpath(full, ROOT).replace("\\", "/")
                    with open(full, encoding="utf-8", errors="replace") as fh:
                        source = fh.read()
                    if sub == "tests":
                        yield crate, rel, source, 0, "tests"
                    else:
                        parts = rel.split("/")
                        whole = "tests" in parts[3:-1] or f == "tests.rs" or f.endswith("_tests.rs")
                        if whole:
                            yield crate, rel, source, 0, "src"
                        else:
                            at = test_start(source)
                            if at is not None:
                                yield crate, rel, source, at, "src"


# --- The class rules -------------------------------------------------------------
# (class, rule name, where, pattern). `where` is "fn" (the function the site is
# in), "text" (the site's statement and the two lines after) or "near" (three lines
# before the site to two after). First match wins, so the order is the precedence.
def R(cls, name, where, pattern):
    return (cls, name, where, re.compile(pattern, re.S))


RULES = [
    # A real process, a server task or a listener: start, bind, exit.
    R("process", "bind/exit bound", "text", r"\b(BIND_TIMEOUT|EXIT_TIMEOUT|BOOT_TIMEOUT|SPAWN_TIMEOUT|START_TIMEOUT|STOP_TIMEOUT)\b"),
    R(
        "process",
        "child or listener",
        "near",
        r"\.try_wait\(\)|child\.wait\(\)|\.kill\(\)|wait_with_output|\bexited\b|\b(ssh|web|ipc|bound|addr|ready)_rx\b"
        r"|TcpStream::connect|UnixStream::connect|connect_async|\.accept\(\)",
    ),
    R(
        "process",
        "boot/stop helper",
        "fn",
        r"^(boot|boot_.*|start|start_.*|spawn_.*|launch.*|stop|restart|wait_exit|wait_for_exit|wait_for_file"
        r"|wait_for_stop|await_log|.*_helper)$",
    ),
    # A reply read from a stream.
    R(
        "stream",
        "stream read",
        "near",
        r"channel\.wait\(\)|\.read_line\(|\.read_exact\(|\.read_buf\(|\.read\(&mut|read_frame|next_frame|frames\.read"
        r"|\.next_line\(\)|lines\.next\(\)|\b(ws|socket|stream|sink|client|conn)\.next\(\)|\.read_to_end\(|recv_text"
        r"|next_text|read_message|\.seen\b|\.readable\(\)|io::copy",
    ),
    R(
        "stream",
        "reader helper",
        "fn",
        r"^(until|expect|expect_.*|read_.*|next_.*|pump|idle|drain.*|recv_.*|await_line|try_await_line|watch|say"
        r"|answer|ask|ask_.*|collect.*|wait_for_text|wait_text|until_line|event|state)$",
    ),
    R("stream", "retry on busy", "near", r"TRANSIENTS|RETRY_BUDGET|please retry|still pending|ENGINE_BUSY|is_transient|(?i:sentinel)"),
    # A game or product figure restated as a span.
    R(
        "figure",
        "named figure",
        "near",
        r"BOARD_MS|DISCONNECT_GRACE|\bgrace\b|(?i:linkdead)|(?i:\bcycle)|cannon|\brack\b|hold_ms|disengage|quiet_s"
        r"|deadline \*|MAX_DURATION|(?i:match_wait)|STORY_BUDGET|(?i:lockout)|(?i:write_timeout)|(?i:idle_timeout)",
    ),
    # Already bounded by an event.
    R(
        "event",
        "channel/handle/notify",
        "text",
        r"\.recv\(\)|\.notified\(\)|\.changed\(\)|\.join\(\)|\brx\b|_rx\b|\bhandle\b|_handle\b|\.task\b|\btask\b|oneshot"
        r"|\bdone\b|\.acquire\(|barrier|\.closed\(\)|_join\b|\bjoin\b",
    ),
    R("event", "std channel", "text", r"recv_timeout|wait_timeout"),
]


def classify(s):
    if s.paused:
        s.cls, s.rule = "driven", "start_paused test"
        return
    for cls, name, where, pat in RULES:
        hay = {"path": s.path, "fn": s.fn, "text": s.text, "near": s.ctx}[where]
        if pat.search(hay):
            s.cls, s.rule = cls, name
            return
    if s.shape == "bound":
        # A timeout around a future of the code under test: the future is the event.
        s.cls, s.rule = "event", "bound on a future (default)"
    elif s.shape == "poll":
        s.cls, s.rule = "clock", "poll on state (default)"
    else:
        # A bare sleep on a running clock gives something the test does not drive time to happen.
        s.cls, s.rule = "clock", "fixed sleep (default)"


CLASSES = ["figure", "clock", "stream", "process", "event", "driven"]
SHAPES = ["fixed", "poll", "bound"]


def table(rows, cols, cell, total_label="total"):
    widths = [max(len(str(r)) for r in rows + [total_label])] + [max(len(c), 5) for c in cols] + [5]
    line = lambda items: "  ".join(str(x).rjust(w) if k else str(x).ljust(w) for k, (x, w) in enumerate(zip(items, widths)))
    print(line([""] + cols + ["all"]))
    sums = Counter()
    for r in rows:
        vals = [cell(r, c) for c in cols]
        if not any(vals):
            continue
        for c, v in zip(cols, vals):
            sums[c] += v
        print(line([r] + vals + [sum(vals)]))
    print(line([total_label] + [sums[c] for c in cols] + [sum(sums.values())]))


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--src", action="store_true", help="also scan test code under crates/*/src")
    ap.add_argument("--dump", action="store_true", help="one TSV row a site")
    ap.add_argument("--class", dest="cls", choices=CLASSES, help="list the sites of one class")
    ap.add_argument("--literals", action="store_true", help="the literal-duration census")
    ap.add_argument("--budget", action="store_true", help="what the wait_budget gate counts, one row a file")
    args = ap.parse_args()

    if args.budget:
        return budget()

    sites, literals, files = [], Counter(), 0
    for crate, rel, source, start, scope in walk(args.src):
        files += 1
        found = scan_file(crate, rel, source, start, scope)
        for s in found:
            classify(s)
        sites.extend(found)
        lines = blank(source).split("\n")
        support = "/support/" in rel or rel.endswith("/harness.rs")
        waits = set()
        for s in found:
            waits.update(range(s.line, s.line + 3))
        for k, text in enumerate(lines[start:], start + 1):
            for _m in LITERAL.finditer(text):
                where = "rig" if support else ("in a wait" if k in waits else "elsewhere")
                literals[(crate, where)] += 1

    if args.dump:
        print("crate\tscope\tpath\tline\tfn\tneedle\tshape\tclass\trule\ttext")
        for s in sites:
            print("\t".join([s.crate, s.scope, s.path, str(s.line), s.fn, s.name, s.shape, s.cls, s.rule, s.text[:160]]))
        return
    if args.cls:
        for s in sites:
            if s.cls == args.cls:
                print(f"{s.path}:{s.line}\t{s.fn}\t{s.shape}\t{s.rule}\t{s.text[:140]}")
        return
    if args.literals:
        crates = sorted({k[0] for k in literals})
        print("Literal `Duration::from_*(<number>)` in test code, by crate; the last two columns are stories\n")
        table(crates, ["rig", "in a wait", "elsewhere"], lambda r, c: literals[(r, c)])
        return

    crates = sorted({s.crate for s in sites})
    with_site = len({s.path for s in sites})
    print(f"{len(sites)} wait sites in {with_site} files ({files} test files scanned)\n")
    print("By class and crate\n")
    by = Counter((s.crate, s.cls) for s in sites)
    table(crates, CLASSES, lambda r, c: by[(r, c)])
    print("\nBy shape and crate (wall-clock sites only: `driven` left out)\n")
    wall = [s for s in sites if s.cls != "driven"]
    by = Counter((s.crate, s.shape) for s in wall)
    table(crates, SHAPES, lambda r, c: by[(r, c)])
    print("\nBy class and shape\n")
    by = Counter((s.cls, s.shape) for s in sites)
    table(CLASSES, SHAPES, lambda r, c: by[(r, c)])
    print("\nRig or story (rig = tests/support/ and harness.rs)\n")
    rig = lambda s: "rig" if "/support/" in s.path or s.path.endswith("/harness.rs") else "story"
    by = Counter((s.crate, rig(s)) for s in wall)
    table(crates, ["rig", "story"], lambda r, c: by[(r, c)])
    print("\nBy rule\n")
    for (cls, rule), k in sorted(Counter((s.cls, s.rule) for s in sites).items()):
        print(f"  {cls:9} {k:4}  {rule}")


# The crates whose test code under src/ the wait_budget gate counts (its SRC_SCOPE).
SRC_SCOPE = ("mush-server",)
CFG_TEST = "#[cfg(test)]"


def cfg_test_lines(code):
    """The 1-based line ranges of a blanked file's #[cfg(test)] items, as the
    gate's scan::blank_cfg_test_items finds them: from the attribute to the close
    of the item's first brace, or nothing when a `;` comes first."""
    spans, search = [], 0
    while True:
        at = code.find(CFG_TEST, search)
        if at < 0:
            return spans
        cursor = at + len(CFG_TEST)
        while cursor < len(code) and code[cursor] not in "{;":
            cursor += 1
        if cursor >= len(code) or code[cursor] == ";":
            search = at + len(CFG_TEST)
            continue
        depth, end = 0, cursor
        while end < len(code):
            if code[end] == "{":
                depth += 1
            elif code[end] == "}":
                depth -= 1
                if depth == 0:
                    break
            end += 1
        spans.append((code.count("\n", 0, at) + 1, code.count("\n", 0, end) + 1))
        search = end + 1


def src_whole(rel, code):
    """Whether a src/ file is test code whole, by the gate's rule."""
    parts = rel.split("/")
    name = parts[-1]
    if "tests" in parts[3:-1] or name == "tests.rs" or name.endswith("_tests.rs"):
        return True
    head = code.lstrip()
    return head.startswith("#![cfg(test)]")


def src_budget_files():
    """(crate, rel, source, line filter) for the src/ files of SRC_SCOPE."""
    for crate in SRC_SCOPE:
        top = os.path.join(ROOT, "crates", crate, "src")
        for dirpath, _dirs, files in os.walk(top):
            for f in sorted(files):
                if not f.endswith(".rs"):
                    continue
                full = os.path.join(dirpath, f)
                rel = os.path.relpath(full, ROOT).replace("\\", "/")
                with open(full, encoding="utf-8", errors="replace") as fh:
                    source = fh.read()
                code = blank(source)
                if src_whole(rel, code):
                    yield crate, rel, source, None
                else:
                    spans = cfg_test_lines(code)
                    if spans:
                        yield crate, rel, source, spans


def budget():
    """The wait_budget gate's counts: story files, outside paused tests."""
    rows = Counter()
    for crate, rel, source, start, scope in walk(False):
        if "/support/" in rel or rel.endswith("/harness.rs") or crate == "mush-storykit":
            continue
        path = rel.split("/", 2)[2]
        for counter, pattern in (("wait", NEEDLE), ("clock", CLOCK)):
            n = sum(1 for s in scan_file(crate, rel, source, start, scope, pattern) if not s.paused)
            if n:
                rows[(crate, path, counter)] = n
    for crate, rel, source, spans in src_budget_files():
        path = rel.split("/", 2)[2]
        inside = lambda line: spans is None or any(a <= line <= b for a, b in spans)
        for counter, pattern in (("wait", NEEDLE), ("clock", CLOCK)):
            n = sum(1 for s in scan_file(crate, rel, source, 0, "src", pattern) if not s.paused and inside(s.line))
            if n:
                rows[(crate, path, counter)] = n
    order = {"wait": 0, "clock": 1}
    for (crate, path, counter), n in sorted(rows.items(), key=lambda kv: (kv[0][0], kv[0][1], order[kv[0][2]])):
        print(f"{counter} {n} {crate}/{path}")
    print()
    by = Counter()
    for (crate, _path, counter), n in rows.items():
        by[(crate, counter)] += n
    table(sorted({k[0] for k in by}), ["wait", "clock"], lambda r, c: by[(r, c)])


if __name__ == "__main__":
    sys.exit(main())
