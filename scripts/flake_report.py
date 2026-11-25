#!/usr/bin/env python3
"""Which tests are flaky, how often, and whether each is on the known list.

Doc 51 unit FL.2 (§4.2). nextest writes a JUnit report for every CI shard and
every soak pass. This script reads them, applies docs/flaky-tests.md's own
rule for what a flake is, and prints one table. It only reports: it writes
nothing to the repository unless `--write` is given, and it changes nothing
on GitHub.

Sources, any mix:

    --ci-days D     ci.yml's completed runs of the last D days
    --ci-runs N     ci.yml's last N completed runs
    --run ID        one run; the table is then about the tests that failed in
                    it ("has this test failed anywhere else?"). With no other
                    CI source it is read against the last 14 days.
    --junit DIR     a directory of reports from one head: a soak's (every
                    *.xml under it is one pass of one package, soak.json names
                    the head) or one downloaded run's. Repeatable.

A CI run's reports are its `nextest-junit-<shard>` artifacts, every attempt's.
By default only the shards with a failed job are downloaded, from runs that
failed or were rerun: that is where a failure, and a pass at the same head,
can be. `--all-shards` reads every shard of every run, which also finds the
passes on retry (`<flakyFailure>`) in green runs, at about twenty downloads a
run. Downloads are kept under `--cache` (default target/flake-report) and
never fetched twice.

What counts as flaky (docs/flaky-tests.md):

    retry     a pass on retry: nextest's <flakyFailure>
    same head the test failed and passed at one head, across attempts, runs
              or soak passes
    branches  the test failed on two or more branches in the window. This
              one is a lead, not a finding: a red main shows the same way, and
              so does one bug on two stacked branches. The script cannot tell
              branches apart, so the row is printed and marked "to read", and
              `--check` does not fail on it alone.

A failure on one branch only, with no pass at that head, is that branch's and
is not in the table. A soak test that failed every pass is broken, not flaky,
and is listed apart.

    --check         exit 1 on a flaky test (by retry or same head) that is not
                    on the known list, on a listed row past its expiry, or on
                    a soak test that failed every pass. Exit 2 means the
                    sources could not be read (gh, the network), not a finding.
    --write [PATH]  write the table to PATH (default docs/flaky-observed.md),
                    the generated file ruling Q-51-8 asks a session to refresh
                    in the pull request that adds or closes a row
    --known PATH    the list (default crates/mush-contract-tests/flaky-known.toml)
    --repo O/N      the repository (default: $GITHUB_REPOSITORY, else gh's)

Needs Python 3.11 (tomllib) and, for the CI sources, an authenticated `gh`.
"""

import argparse
import collections
import datetime
import io
import json
import os
import pathlib
import subprocess
import sys
import time
import tomllib
import xml.etree.ElementTree as ET
import zipfile

UTC = datetime.timezone.utc
ROOT = pathlib.Path(__file__).resolve().parent.parent
KNOWN = ROOT / "crates" / "mush-contract-tests" / "flaky-known.toml"
RECORD = ROOT / "docs" / "flaky-tests.md"
OBSERVED = ROOT / "docs" / "flaky-observed.md"
ARTIFACT = "nextest-junit-"
# A report larger than this is not one of ours.
MAX_REPORT = 256 * 1024 * 1024

Seen = collections.namedtuple("Seen", "test outcome head branch when label link kind")
# outcome: "pass", "fail" or "retry" (failed, then passed on nextest's retry).


def parse_time(text):
    if not text:
        return None
    try:
        when = datetime.datetime.fromisoformat(text.replace("Z", "+00:00"))
    except ValueError:
        return None
    return when if when.tzinfo else when.replace(tzinfo=UTC)


def read_junit(data, where):
    """Yield (test, outcome, when) for every testcase of one report."""
    if len(data) > MAX_REPORT or b"<!ENTITY" in data or b"<!DOCTYPE" in data:
        print(f"skipped {where}: not a nextest report", file=sys.stderr)
        return
    try:
        root = ET.fromstring(data)
    except ET.ParseError as error:
        print(f"skipped {where}: {error}", file=sys.stderr)
        return
    stamp = parse_time(root.get("timestamp"))
    for case in root.iter("testcase"):
        binary = case.get("classname") or "?"
        test = (binary, case.get("name") or "?")
        tags = {child.tag for child in case}
        if tags & {"failure", "error"}:
            outcome = "fail"
        elif "flakyFailure" in tags:
            outcome = "retry"
        else:
            outcome = "pass"
        yield test, outcome, parse_time(case.get("timestamp")) or stamp


# --- the known list ----------------------------------------------------------


def read_known(path):
    with open(path, "rb") as fh:
        return tomllib.load(fh).get("flaky", [])


def row_for(rows, test):
    binary, name = test
    for row in rows:
        if row.get("binary") != binary:
            continue
        if row.get("test") == name:
            return row
        if row.get("module") and name.startswith(row["module"] + "::"):
            return row
        if row.get("whole_binary"):
            return row
    return None


def row_scope(row):
    if row.get("test"):
        return f"`{row['test']}`"
    if row.get("module"):
        return f"`{row['module']}::*`"
    return "(the whole binary)"


def expired(row, today):
    try:
        return datetime.date.fromisoformat(row["expires"]) < today
    except (KeyError, ValueError):
        return True


def read_record(path=RECORD):
    """The table rows of docs/flaky-tests.md: [(number, line)]."""
    rows = []
    try:
        text = pathlib.Path(path).read_text(encoding="utf-8")
    except OSError:
        return rows
    for line in text.splitlines():
        cells = line.split("|")
        if len(cells) > 2 and cells[1].strip().isdigit():
            rows.append((cells[1].strip(), line))
    return rows


# The tests `phases!` makes of one story (mush-storykit, doc 51 §3.6): the test is
# `<story>::<phase>`, and the record names the story.
PHASE_TESTS = ("at_the_epoch", "before_a_days_turn", "mid_month")


def record_row(record, test):
    parts = test[1].split("::")
    short = parts[-1]
    if short in PHASE_TESTS and len(parts) > 1:
        short = parts[-2]
    return next((number for number, line in record if short in line), None)


# --- sources -----------------------------------------------------------------


def from_dir(path):
    """A directory of reports from one head: every file is one pass."""
    path = pathlib.Path(path)
    meta = {}
    if (path / "soak.json").is_file():
        meta = json.loads((path / "soak.json").read_text(encoding="utf-8"))
    head = meta.get("head") or f"dir:{path.name}"
    link = meta.get("link") or str(path)
    files = sorted(path.rglob("*.xml"))
    if not files:
        print(f"{path}: no *.xml report", file=sys.stderr)
    for file in files:
        label = f"{path.name}/{file.relative_to(path).as_posix()}"
        for test, outcome, when in read_junit(file.read_bytes(), file):
            when = when or datetime.datetime.fromtimestamp(file.stat().st_mtime, UTC)
            yield Seen(test, outcome, head, meta.get("branch", "soak"), when, label, link, "soak")


class Github:
    def __init__(self, repo, cache):
        self.repo = repo
        self.cache = pathlib.Path(cache)
        self.calls = 0

    def api(self, path, jq=None, paginate=False, binary=False):
        command = ["gh", "api", f"repos/{self.repo}/{path}"]
        if paginate:
            command.append("--paginate")
        if jq:
            command += ["--jq", jq]
        # GitHub answers a 502 now and then; a rate limit or a 404 is not retried.
        for wait in (2, 8, 30, None):
            self.calls += 1
            done = subprocess.run(command, capture_output=True, check=False)
            if done.returncode == 0:
                break
            why = done.stderr.decode(errors="replace").strip()
            transient = any(
                mark in why for mark in ("HTTP 5", "timeout", "connection", "EOF", "stream error")
            )
            if wait is None or not transient:
                raise RuntimeError(f"gh api {path}: {why}")
            time.sleep(wait)
        if binary:
            return done.stdout
        return [json.loads(line) for line in done.stdout.decode().splitlines() if line.strip()]

    def runs(self, days=None, count=None):
        fields = "{id, head_sha, head_branch, event, conclusion, run_attempt, created_at, html_url}"
        jq = f".workflow_runs[] | {fields}"
        base = "actions/workflows/ci.yml/runs?status=completed&per_page=100"
        if days is not None:
            since = (datetime.datetime.now(UTC) - datetime.timedelta(days=days)).strftime("%Y-%m-%dT%H:%M:%SZ")
            return self.api(f"{base}&created=%3E%3D{since}", jq, paginate=True)
        found, page = [], 1
        while len(found) < count:
            batch = self.api(f"{base}&page={page}", jq)
            if not batch:
                break
            found += batch
            page += 1
        return found[:count]

    def run(self, run_id):
        fields = "{id, head_sha, head_branch, event, conclusion, run_attempt, created_at, html_url}"
        return self.api(f"actions/runs/{run_id}", fields)[0]

    def reports(self, run, every_shard):
        """Yield (attempt, shard, report bytes) for one run, cached on disk."""
        home = self.cache / str(run["id"])
        mode = "all" if every_shard else "failed"
        index = home / "index.json"
        if index.is_file():
            kept = json.loads(index.read_text(encoding="utf-8"))
            if kept["attempt"] == run["run_attempt"] and (kept["mode"] == "all" or kept["mode"] == mode):
                for entry in kept["reports"]:
                    data = (home / entry["file"]).read_bytes()
                    yield entry["attempt"], entry["shard"], data
                return
        failed_or_rerun = run["conclusion"] == "failure" or run["run_attempt"] > 1
        entries = []
        if every_shard or failed_or_rerun:
            jobs = self.api(
                f"actions/runs/{run['id']}/jobs?filter=all&per_page=100",
                ".jobs[] | {name, run_attempt, conclusion, started_at, completed_at}",
                paginate=True,
            )
            wanted = {shard_of(job["name"]) for job in jobs if job["conclusion"] == "failure"}
            artifacts = self.api(
                f"actions/runs/{run['id']}/artifacts?per_page=100",
                ".artifacts[] | {id, name, created_at, expired}",
                paginate=True,
            )
            home.mkdir(parents=True, exist_ok=True)
            for artifact in artifacts:
                name = artifact["name"]
                if not name.startswith(ARTIFACT) or artifact["expired"]:
                    continue
                shard = name[len(ARTIFACT):]
                if not every_shard and shard not in wanted:
                    continue
                blob = self.api(f"actions/artifacts/{artifact['id']}/zip", binary=True)
                attempt = attempt_of(artifact, shard, jobs)
                with zipfile.ZipFile(io.BytesIO(blob)) as archive:
                    for at, member in enumerate(archive.infolist()):
                        if not member.filename.endswith(".xml") or member.file_size > MAX_REPORT:
                            continue
                        file = f"{artifact['id']}-{at}.xml"
                        (home / file).write_bytes(archive.read(member))
                        entries.append({"file": file, "attempt": attempt, "shard": shard})
        home.mkdir(parents=True, exist_ok=True)
        index.write_text(
            json.dumps({"attempt": run["run_attempt"], "mode": mode, "reports": entries}),
            encoding="utf-8",
        )
        for entry in entries:
            yield entry["attempt"], entry["shard"], (home / entry["file"]).read_bytes()

    def seen(self, run, every_shard):
        when_run = parse_time(run["created_at"])
        for attempt, shard, data in self.reports(run, every_shard):
            label = f"{run['id']} a{attempt} {shard}"
            link = f"{run['html_url']}/attempts/{attempt}"
            for test, outcome, when in read_junit(data, label):
                yield Seen(test, outcome, run["head_sha"], run["head_branch"] or "?", when or when_run, label, link, "ci")


def shard_of(job_name):
    """`test (rest-mushd)` -> `rest-mushd`."""
    if job_name.startswith("test (") and job_name.endswith(")"):
        return job_name[len("test ("):-1]
    return job_name


def attempt_of(artifact, shard, jobs):
    """The attempt whose job of this shard was running when the artifact was made."""
    made = parse_time(artifact["created_at"])
    mine = [job for job in jobs if shard_of(job["name"]) == shard]
    for job in mine:
        start, end = parse_time(job["started_at"]), parse_time(job["completed_at"])
        if made and start and end and start <= made <= end + datetime.timedelta(minutes=2):
            return job["run_attempt"]
    return max((job["run_attempt"] for job in mine), default=1)


# --- the rule ----------------------------------------------------------------


class Tally:
    """What the rule needs, without keeping every passing testcase."""

    def __init__(self):
        self.bad = collections.defaultdict(list)  # test -> the failures and retries
        self.passed = collections.defaultdict(set)  # test -> the heads it passed at
        self.soak = collections.Counter()  # test -> the soak reports that ran it
        self.reports = set()

    def add(self, one):
        self.reports.add(one.label)
        if one.kind == "soak":
            self.soak[one.test] += 1
        if one.outcome != "fail":
            self.passed[one.test].add(one.head)
        if one.outcome != "pass":
            self.bad[one.test].append(one)


def judge(tally):
    """test -> the facts the table prints, for every test with a failure or a retry."""
    facts = {}
    for test, bad in tally.bad.items():
        retries = [s for s in bad if s.outcome == "retry"]
        fails = [s for s in bad if s.outcome == "fail"]
        same_head = sorted({s.head for s in fails} & tally.passed[test])
        branches = sorted({s.branch for s in fails if s.kind == "ci"})
        soak_bad = [s for s in bad if s.kind == "soak"]
        soak_runs = tally.soak[test]
        why = []
        if retries:
            why.append(f"retry x{len(retries)}")
        if same_head:
            why.append(f"same head x{len(same_head)}")
        firm = bool(why)
        if len(branches) >= 2:
            why.append(f"{len(branches)} branches" + ("" if firm else " (to read)"))
        facts[test] = {
            "bad": sorted(bad, key=lambda s: s.when),
            "why": why,
            "firm": firm,
            "branches": branches,
            "soak": (len(soak_bad), soak_runs),
            "broken": soak_runs >= 2 and len(soak_bad) == soak_runs and not retries,
        }
    return facts


def render(facts, rows, record, sources, now, focus=None):
    today = now.date()
    out = []
    flaky = {t: f for t, f in facts.items() if f["why"] and not f["broken"]}
    broken = {t: f for t, f in facts.items() if f["broken"]}
    theirs = {t: f for t, f in facts.items() if not f["why"] and not f["broken"]}
    if focus is not None:
        flaky = {t: f for t, f in flaky.items() if t in focus}
        broken = {t: f for t, f in broken.items() if t in focus}
        theirs = {t: f for t, f in theirs.items() if t in focus}

    def listing(test):
        row = row_for(rows, test)
        if row is None:
            return "", "**NOT LISTED**"
        note = f"row {row['row']}" if row.get("row") else "carried"
        if expired(row, today):
            note += f", **EXPIRED {row.get('expires')}**"
        else:
            note += f", to {row.get('expires')}"
        return row.get("class", ""), note

    def within(bad, days):
        return sum(1 for s in bad if now - s.when <= datetime.timedelta(days=days))

    def links(bad):
        last, seen_links = [], set()
        for s in reversed(bad):
            if s.link not in seen_links:
                seen_links.add(s.link)
                last.append(f"[{s.label}]({s.link})" if s.link.startswith("http") else f"`{s.label}`")
            if len(last) == 3:
                break
        return ", ".join(last)

    out.append(f"Sources: {'; '.join(sources)}.")
    out.append("")
    if flaky:
        out.append("| Test | Binary | Class | Known list | Record | First seen | Last seen | 14 d | 30 d | Soak | Why | Last three |")
        out.append("|---|---|---|---|---|---|---|---:|---:|---|---|---|")
        order = sorted(flaky, key=lambda t: (-len(flaky[t]["bad"]), t))
        for test in order:
            f = flaky[test]
            bad = f["bad"]
            klass, known = listing(test)
            failed, passes = f["soak"]
            soak = f"{failed}/{passes}" if passes else ""
            number = record_row(record, test)
            in_record = f"row {number}" if number else "none"
            out.append(
                f"| `{test[1]}` | `{test[0]}` | {klass} | {known} | {in_record} | {bad[0].when:%Y-%m-%d} | "
                f"{bad[-1].when:%Y-%m-%d} | {within(bad, 14)} | {within(bad, 30)} | {soak} | "
                f"{', '.join(f['why'])} | {links(bad)} |"
            )
    else:
        out.append("No test met the rule for a flake in these sources.")
    if broken:
        out += ["", "**Failed every soak pass (broken, not flaky):**", ""]
        for test in sorted(broken):
            failed, passes = broken[test]["soak"]
            out.append(f"- `{test[0]}` `{test[1]}`: {failed} of {passes} passes; {links(broken[test]['bad'])}")
    if theirs and focus is not None:
        out += ["", "**Failed on one branch only, with no pass at that head (the branch's own, by the rule):**", ""]
        for test in sorted(theirs):
            klass, known = listing(test)
            where = ", ".join(theirs[test]["branches"]) or "one head"
            out.append(f"- `{test[0]}` `{test[1]}`: {where}; known list: {known}; {links(theirs[test]['bad'])}")
    elif theirs:
        out += ["", f"{len(theirs)} more test(s) failed on one branch or one head only and are that branch's own by the rule; `--run <id>` names them for a run."]
    quiet = [row for row in rows if not any(row_for([row], test) for test in flaky)]
    stale = [row for row in rows if expired(row, today)]
    if focus is None and rows:
        out += ["", f"**The known list:** {len(rows)} row(s), {len(rows) - len(quiet)} seen in these sources, {len(stale)} past expiry."]
        if quiet:
            out += ["", "Listed and not seen (no evidence here to renew on):", ""]
            out += [f"- `{row['binary']}` {row_scope(row)}, {row.get('class')}, expires {row.get('expires')}" for row in quiet]
    unlisted = sorted(t for t in flaky if flaky[t]["firm"] and row_for(rows, t) is None)
    over = sorted(t for t in flaky if (row := row_for(rows, t)) is not None and expired(row, today))
    return out, unlisted, over, sorted(broken), stale


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--ci-days", type=float)
    parser.add_argument("--ci-runs", type=int)
    parser.add_argument("--run", type=int, action="append", default=[])
    parser.add_argument("--junit", "--soak", action="append", default=[], dest="junit")
    parser.add_argument("--all-shards", action="store_true")
    parser.add_argument("--cache", default=str(ROOT / "target" / "flake-report"))
    parser.add_argument("--known", default=str(KNOWN))
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY"))
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--write", nargs="?", const=str(OBSERVED))
    args = parser.parse_args()
    if not (args.ci_days or args.ci_runs or args.run or args.junit):
        parser.error("name a source: --ci-days, --ci-runs, --run or --junit")

    now = datetime.datetime.now(UTC)
    rows = read_known(args.known)
    tally, sources, focus = Tally(), [], None
    for directory in args.junit:
        before = len(tally.reports)
        for one in from_dir(directory):
            tally.add(one)
        sources.append(f"`{pathlib.Path(directory).name}`: {len(tally.reports) - before} report(s)")

    if args.run and not (args.ci_days or args.ci_runs):
        args.ci_days = 14
    if args.ci_days or args.ci_runs or args.run:
        repo = args.repo
        if not repo:
            done = subprocess.run(
                ["gh", "repo", "view", "--json", "nameWithOwner", "--jq", ".nameWithOwner"],
                capture_output=True, text=True, check=False,
            )
            repo = done.stdout.strip()
        if not repo:
            sys.exit("no repository: pass --repo OWNER/NAME")
        github = Github(repo, args.cache)
        runs = {}
        if args.ci_days:
            for run in github.runs(days=args.ci_days):
                runs[run["id"]] = run
            sources.append(f"ci.yml, the last {args.ci_days:g} day(s)")
        if args.ci_runs:
            for run in github.runs(count=args.ci_runs):
                runs[run["id"]] = run
            sources.append(f"ci.yml, the last {args.ci_runs} run(s)")
        focus = set() if args.run else None
        for run_id in args.run:
            run = runs.setdefault(run_id, github.run(run_id))
            sources.append(f"run {run_id} ({run['head_branch']})")
        read = 0
        for run in runs.values():
            before = len(tally.reports)
            for one in github.seen(run, args.all_shards):
                tally.add(one)
                if focus is not None and run["id"] in args.run and one.outcome != "pass":
                    focus.add(one.test)
            read += len(tally.reports) > before
        which = "every shard" if args.all_shards else "the shards with a failed job"
        sources.append(f"{len(runs)} run(s) listed, {read} with reports read ({which})")
        print(f"{github.calls} API call(s); downloads are kept under {args.cache}", file=sys.stderr)

    lines, unlisted, over, broken, stale = render(judge(tally), rows, read_record(), sources, now, focus)
    title = "# Flaky tests observed" if focus is None else "# Failed in the named run(s)"
    body = "\n".join([title, ""] + lines) + "\n"
    print(body)

    if args.write:
        header = (
            "<!-- GENERATED by scripts/flake_report.py --write. Do not edit by hand.\n"
            "     Refresh it in the pull request that adds or closes a row of\n"
            "     crates/mush-contract-tests/flaky-known.toml (doc 51, ruling Q-51-8). -->\n\n"
        )
        intro = (
            f"\n*Generated {now:%Y-%m-%d %H:%M} UTC. What was seen, by the rule of "
            "[flaky-tests.md](flaky-tests.md), which stays the hand-written record of causes. "
            "A test here whose Record is `none` is the work queue. The columns: Class and Known "
            "list are the list's (crates/mush-contract-tests/flaky-known.toml); Record is the row "
            "of flaky-tests.md that names the test; 14 d and 30 d count the runs or passes in "
            "which the test failed or passed on retry; Soak is failed passes over passes; a Why "
            "of branches alone is a lead to read, not a finding.*\n\n"
        )
        text = header + body.replace(title + "\n", title + "\n" + intro, 1)
        pathlib.Path(args.write).write_text(text, encoding="utf-8", newline="\n")
        print(f"wrote {args.write}", file=sys.stderr)

    if args.check:
        problems = []
        problems += [f"flaky and not on the known list: {b} {n}" for b, n in unlisted]
        problems += [f"flaky and its row is past expiry: {b} {n}" for b, n in over]
        problems += [f"failed every soak pass (broken, not flaky): {b} {n}" for b, n in broken]
        problems += [
            f"a listed row is past its expiry ({row.get('expires')}): {row['binary']} {row_scope(row)}"
            for row in stale
        ]
        for problem in problems:
            print(f"::error title=flake report::{problem}")
        if problems:
            sys.exit(1)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError) as error:
        print(f"::warning title=flake report::could not read the sources: {error}")
        sys.exit(2)
