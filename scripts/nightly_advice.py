#!/usr/bin/env python3
"""Doc 43 rule 2.3 #15: which nightly-, weekly- or dispatch-tier jobs a change should dispatch.

Reads .github/nightly-paths.txt and the files a change touches, and prints the jobs
whose paths it touches with the command that dispatches them at the FAST pace: the
CI dispatch with `mind_pulse_ms` empty, the M7 stories' 50-ms pulse. That run is the
gate; the PR is marked ready once it is green. The real-pace run (production's
1,000-ms pulse, three hours and more) is not a gate (owner, 2026-11-05): it is the
next nightly's, or it is fired and not waited on, and its result reaches the owner
through the nightly report.

    python scripts/nightly_advice.py                       # this branch against origin/main
    python scripts/nightly_advice.py <base> [<head>]       # any two commits (CI passes the PR's)
    python scripts/nightly_advice.py --dispatch            # ...and fire the fast run for this branch
    python scripts/nightly_advice.py --dispatch --real-pace-too   # ...and the real-pace run, unwaited

Without --dispatch it is advice only and exits 0 whatever it finds. On GitHub Actions
it also writes the advice as a warning annotation and to the step summary.
"""
import argparse
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REPO = "conclavio/rustmush"


def load_map():
    jobs = {}
    with open(os.path.join(ROOT, ".github", "nightly-paths.txt"), encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            job, tier, path = line.split()
            jobs.setdefault((job, tier), []).append(path)
    return jobs


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("base", nargs="?", default="origin/main...")
    parser.add_argument("head", nargs="?", default="HEAD")
    parser.add_argument("--dispatch", action="store_true",
                        help="fire the fast-pace CI dispatch for this branch when any job matches")
    parser.add_argument("--real-pace-too", action="store_true",
                        help="with --dispatch, also fire the real-pace run (not a gate; not waited on)")
    args = parser.parse_args()
    base, head = args.base, args.head
    if base.endswith("..."):
        base = git("merge-base", base[:-3], head)
    files = [p for p in git("diff", "--name-only", base, head).splitlines() if p]
    hits = {}
    for (job, tier), paths in sorted(load_map().items()):
        for f in files:
            if any(f == p or (p.endswith("/") and f.startswith(p)) for p in paths):
                hits.setdefault((job, tier), []).append(f)
    if not hits:
        print(f"Rule 2.3 #15: {len(files)} changed file(s) touch no nightly, weekly or dispatch job's paths.")
        return 0
    branch = os.environ.get("GITHUB_HEAD_REF") or git("rev-parse", "--abbrev-ref", "HEAD")
    fast = ["gh", "workflow", "run", "CI", "--repo", REPO, "--ref", branch]
    real = fast + ["-f", "mind_pulse_ms=1000"]
    lines = [f"Rule 2.3 #15: this change touches paths {len(hits)} non-PR job(s) cover. Run them at the FAST "
             f"pace and get them green before the PR is marked ready:",
             "    " + " ".join(fast) + "        # the gate (mind_pulse_ms empty = the 50-ms pulse)",
             "  Optional, not a gate, never waited on (the nightly report carries its result):",
             "    " + " ".join(real), ""]
    for (job, tier), fs in sorted(hits.items()):
        shown = ", ".join(fs[:3]) + (f" and {len(fs) - 3} more" if len(fs) > 3 else "")
        lines.append(f"  {job} ({tier}): {shown}")
    text = "\n".join(lines)
    print(text)
    if os.environ.get("GITHUB_ACTIONS") == "true":
        jobs = ", ".join(job for job, _ in sorted(hits))
        print(f"::warning title=Rule 2.3 #15: dispatch before ready::this PR touches paths of {jobs}; "
              f"run `gh workflow run CI --ref {branch}` (fast pace) and get them green before marking it ready")
        summary = os.environ.get("GITHUB_STEP_SUMMARY")
        if summary:
            with open(summary, "a", encoding="utf-8") as f:
                f.write("### Rule 2.3 #15: nightly jobs to dispatch\n\n```\n" + text + "\n```\n")
    if args.dispatch:
        subprocess.run(fast, cwd=ROOT, check=True)
        print(f"Dispatched the fast-pace run for {branch}; it gates the PR.")
        if args.real_pace_too:
            subprocess.run(real, cwd=ROOT, check=True)
            print("Dispatched the real-pace run too; do not wait on it, the nightly report carries its result.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
