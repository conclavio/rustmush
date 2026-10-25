#!/usr/bin/env python3
"""Interleave serial/parallel arena harness runs on one otherwise quiet host.

Build with cargo test --release -p tacsim --test harness --no-run first, then
pass its executable. Writes every report, log, and the comparison summary.
"""
import argparse
import json
import os
from pathlib import Path
import re
import statistics
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binary", type=Path)
parser.add_argument("output", type=Path)
parser.add_argument("--rounds", type=int, default=5)
parser.add_argument("--workers", type=int, default=4)
args = parser.parse_args()
assert args.rounds > 0 and 1 < args.workers <= 4
root = Path(__file__).resolve().parents[1]
binary, output = args.binary.resolve(), args.output.resolve()
output.mkdir(parents=True, exist_ok=True)
source = (root / "crates/tacsim/tests/harness.rs").read_text()
paths = []
for constant in ("DETERMINISTIC_OBJECTS", "DETERMINISTIC_LEAVES"):
    body = re.search(r"const " + constant + r".*?= \[(.*?)\];", source, re.S).group(1)
    paths.extend(re.findall(r'"([^"\n]+)"', body))
paths.append("frame_execution.state_hash")

def get(report, path):
    for key in path.split("."):
        report = report[key]
    return report

reports = {cohort: {1: [], args.workers: []} for cohort in ("ci", "authored")}
for round_no in range(args.rounds):
    order = (1, args.workers) if round_no % 2 == 0 else (args.workers, 1)
    for workers in order:
        for cohort in reports:
            stem = output / f"{cohort}-r{round_no + 1}-w{workers}"
            test = "tacsim_harness_arena_step" if cohort == "ci" else "tacsim_harness_arena_authored_reach"
            env = os.environ.copy()
            env.update(TACSIM_FRAME_WORKERS=str(workers), TACSIM_JSON=str(stem) + ".json", TACSIM_ASSERT="0")
            with stem.with_suffix(".log").open("w") as log:
                subprocess.run([str(binary), test, "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                               cwd=root / "crates/tacsim", env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
            suffix = "-arena.json" if cohort == "ci" else "-arena-authored.json"
            report = json.loads(Path(str(stem) + suffix).read_text())
            reports[cohort][workers].append(report)
            print(f"{cohort} round {round_no + 1} workers {workers}: wake p50={report['tempos'][0]['p50_ms']:.3f} p99={report['tempos'][0]['p99_ms']:.3f}", flush=True)

summary = {"rounds": args.rounds, "workers": args.workers, "deterministic_equal": True, "cohorts": {}}
for cohort, by_worker in reports.items():
    reference = by_worker[1][0]
    for rows in by_worker.values():
        for report in rows:
            for path in paths:
                assert get(report, path) == get(reference, path), (cohort, path)
            assert report["snapshot"]["hash_ok"]
    metrics = {}
    for name, read in {
        "wake_p50_ms": lambda r: r["tempos"][0]["p50_ms"],
        "wake_p99_ms": lambda r: r["tempos"][0]["p99_ms"],
        "window_p50_ms": lambda r: r["window"]["ms_p50"],
        "window_p99_ms": lambda r: r["window"]["ms_p99"],
    }.items():
        values = {w: [read(r) for r in rows] for w, rows in by_worker.items()}
        medians = {w: statistics.median(v) for w, v in values.items()}
        metrics[name] = {"samples": values, "median": medians,
                         "change_percent": 100 * (medians[args.workers] / medians[1] - 1)}
    summary["cohorts"][cohort] = metrics
(output / "comparison.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps(summary, indent=2))
