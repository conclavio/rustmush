#!/usr/bin/env python3
"""Doc 43 rule 2.3 #15: which nightly-, weekly- or dispatch-tier jobs a change should dispatch.

Reads .github/nightly-paths.txt and the files a change touches, and prints the jobs
whose paths it touches with the command that dispatches them. Advice only: the exit
status is 0 whatever it finds (the rule is the reviewer's, not a gate).

    python scripts/nightly_advice.py                # this branch against origin/main
    python scripts/nightly_advice.py <base> [<head>]  # any two commits (CI passes the PR's)

On GitHub Actions it also writes the advice as a warning annotation and to the
step summary.
"""
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


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


def changed(base, head):
    out = subprocess.run(["git", "diff", "--name-only", base, head], cwd=ROOT,
                         capture_output=True, text=True, check=True).stdout
    return [p for p in out.splitlines() if p]


def main():
    base = sys.argv[1] if len(sys.argv) > 1 else "origin/main..."
    head = sys.argv[2] if len(sys.argv) > 2 else "HEAD"
    if base.endswith("..."):
        base = subprocess.run(["git", "merge-base", base[:-3], head], cwd=ROOT,
                              capture_output=True, text=True, check=True).stdout.strip()
    files = changed(base, head)
    hits = {}
    for (job, tier), paths in sorted(load_map().items()):
        for f in files:
            if any(f == p or (p.endswith("/") and f.startswith(p)) for p in paths):
                hits.setdefault((job, tier), []).append(f)
    if not hits:
        print(f"Rule 2.3 #15: {len(files)} changed file(s) touch no nightly, weekly or dispatch job's paths.")
        return 0
    branch = os.environ.get("GITHUB_HEAD_REF") or subprocess.run(
        ["git", "rev-parse", "--abbrev-ref", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    lines = [f"Rule 2.3 #15: this change touches paths {len(hits)} non-PR job(s) cover. Dispatch them and "
             f"get them green before the PR is marked ready:",
             f"    gh workflow run CI --repo conclavio/rustmush --ref {branch}", ""]
    for (job, tier), fs in sorted(hits.items()):
        shown = ", ".join(fs[:3]) + (f" and {len(fs) - 3} more" if len(fs) > 3 else "")
        lines.append(f"  {job} ({tier}): {shown}")
    text = "\n".join(lines)
    print(text)
    if os.environ.get("GITHUB_ACTIONS") == "true":
        jobs = ", ".join(job for job, _ in sorted(hits))
        print(f"::warning title=Rule 2.3 #15: dispatch before ready::this PR touches paths of {jobs}; "
              f"run `gh workflow run CI --ref {branch}` and get them green before marking it ready")
        summary = os.environ.get("GITHUB_STEP_SUMMARY")
        if summary:
            with open(summary, "a", encoding="utf-8") as f:
                f.write("### Rule 2.3 #15: nightly jobs to dispatch\n\n```\n" + text + "\n```\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
