#!/usr/bin/env python3
"""Run or compare fresh M6.10 acceptance. No benchmark result is manufactured.

`run NEW_DIRECTORY` owns the Cargo build slot for the duration. `compare DIR`
only reads the two independently produced artifacts and writes comparison.json.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "crates/tacsim/content/minds/acceptance/field_pilot.rhai"
MANIFEST = SOURCE.with_suffix(".manifest.json")


def read(path):
    return json.loads(path.read_text(encoding="utf-8"))


def require(condition, message):
    if not condition:
        raise ValueError(message)


def clean_revision(expected=None):
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    require(expected is None or revision == expected,
            "the checked-out revision changed during acceptance; rerun from one clean revision")
    dirty = subprocess.check_output(
        ["git", "status", "--porcelain", "--untracked-files=normal"], cwd=ROOT, text=True)
    require(not dirty,
            "commit the exact tested source first; acceptance requires a clean tree throughout "
            "(keep output outside the checkout or under ignored target/)")
    return revision


def wilson(wins, total):
    require(0 <= wins <= total and total > 0, "invalid sample counts")
    z = 1.959963984540054
    n = float(total)
    p = wins / n
    d = 1 + z * z / n
    center = (p + z * z / (2 * n)) / d
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return [max(0, center - half), min(1, center + half)]


def counts(row):
    values = [row[k] for k in ("wins", "losses", "draws", "timeouts")]
    require(all(type(v) is int and v >= 0 for v in values), "nonnegative integer outcomes")
    require(sum(values) == 30, "all thirty outcomes, including draws/timeouts, must be retained")
    return dict(zip(("wins", "losses", "draws", "timeouts"), values), samples=30,
                wilson_95=wilson(values[0], 30))


def compare(directory):
    run = read(directory / "acceptance-run.json")
    clean_revision(run["revision"])
    external = read(directory / "external/external-interval.json")
    story = read(directory / "author/author-story.json")
    proving_bytes = (directory / "author/proving-manifest.json").read_bytes()
    proving = json.loads(proving_bytes)
    require(hashlib.sha256(proving_bytes).hexdigest() == story["proving_manifest_sha256"],
            "proving manifest is not the exact result retained by this author story")
    source = read(MANIFEST)
    require(hashlib.sha256(SOURCE.read_bytes()).hexdigest() == source["source_sha256"],
            "source manifest does not describe the current port")
    for artifact in (external, story):
        require(artifact["revision"] == run["revision"] and artifact["run_id"] == run["run_id"],
                "artifacts must belong to the same fresh acceptance invocation and revision")
        require(run["started_unix_ms"] <= artifact["started_unix_ms"] <= artifact["finished_unix_ms"],
                "artifact predates this acceptance invocation")
        require(artifact["finished_unix_ms"] - run["started_unix_ms"] <= 24 * 60 * 60 * 1000,
                "acceptance artifacts span more than one day; rerun them together")
    require(story["first_seal"]["requested"]["source_sha256"] == source["source_sha256"],
            "live story did not seal this exact Field Pilot source")
    require(story["first_seal"]["class"] == "II", "live story did not run Class II")
    checks = story["checks"]
    require(len(checks) == 6 and sum(not r["armed"] for r in checks) == 3,
            "all six production pictures, including three unarmed cases, are required")
    require(all(r["fault"] is None and not r["station_refusals"] and not r["class_violations"]
                and r["operations"] + r["fees"] <= 3000 for r in checks),
            "production six-picture check failed or exceeded Class II")
    context = proving["context"]
    for key in ("scenario", "scenario_content_sha256", "ground", "ground_sha256", "config_sha256",
                "chassis", "startup_ms", "max_duration_ms", "opponent"):
        require(context[key] == external[key], f"scenario mismatch: {key}")
    require(context["scenario"].lower() == "muster" and external["per_side"] == 1,
            "acceptance requires unposed 1v1 MUSTER")
    require(context["candidate"]["class"] == "II" and context["candidate"]["profile"] == "Hold"
            and context["candidate"]["source_sha256"] == source["source_sha256"],
            "proving candidate is not the exact default Field Pilot")
    require(0 < context["native_substep_ms"] <= external["native_substep_max_ms"],
            "proving cadence exceeds the shared configured native ceiling")
    require(len(external["sides"]) == 2
            and {s["side"] for s in external["sides"]} == {"red", "blue"}
            and all(s["samples"] == 15 for s in external["sides"]), "external sides are not 15+15")
    for name in ("wins", "losses", "draws", "timeouts"):
        require(sum(s[name] for s in external["sides"]) == external[name], "external totals disagree")
    require(len(external["matches"]) == 30 and len({(m["side"], m["match"]) for m in external["matches"]}) == 30,
            "external interval lacks thirty distinct actual match records")
    require(external["samples"] == 30, "external sample total is not thirty")
    require(all(m["side"] in ("red", "blue") and type(m["match"]) is int and m["match"] > 0
                for m in external["matches"]), "invalid actual match identity")
    for side in external["sides"]:
        records = [m for m in external["matches"] if m["side"] == side["side"]]
        require(len(records) == 15, "each side needs fifteen actual match records")
        observed = dict.fromkeys(("wins", "losses", "draws", "timeouts"), 0)
        for match in records:
            outcome = match["outcome"]
            require(outcome in ("red", "blue", "draw", "timeout"),
                    "unclassified actual match outcome must not disappear")
            category = ("wins" if outcome == side["side"] else "losses") if outcome in ("red", "blue") \
                else {"draw": "draws", "timeout": "timeouts"}[outcome]
            observed[category] += 1
        require(all(type(side[name]) is int and side[name] == value for name, value in observed.items()),
                "external side totals disagree with actual match outcomes")
    require(all(m["ground_sha256"] == context["ground_sha256"] and m["opponent"] == context["opponent"]
                and m["ledger"]["digest"] for m in external["matches"]), "external match evidence differs")
    pattern = re.compile(r"Won (\d+); lost (\d+); drew (\d+); timed out (\d+)\.")
    totals = [pattern.fullmatch(line) for line in proving["summary"]]
    totals = [m for m in totals if m]
    require(len(totals) == 1, "exact retained proving outcome summary is required")
    paired = re.compile(r"Seed (\d+): normal (Win|Loss|Draw|Timeout); swapped Some\((Win|Loss|Draw|Timeout)\)\.")
    pairs = [paired.fullmatch(line) for line in proving["summary"]]
    pairs = [m for m in pairs if m]
    require(len(pairs) == 15 and [int(m[1]) for m in pairs] == context["pair_seeds"],
            "proving needs fifteen actual paired seeds with both outcomes")
    embedded = dict(zip(("wins", "losses", "draws", "timeouts"), map(int, totals[0].groups())))
    outcome_names = {"Win":"wins", "Loss":"losses", "Draw":"draws", "Timeout":"timeouts"}
    for outcome, name in outcome_names.items():
        require(sum(m[i] == outcome for m in pairs for i in (2, 3)) == embedded[name],
                "paired outcomes disagree with proving totals")
    faults_pattern = re.compile(
        r"Rounds fired \d+; candidate rounds left \d+; rules fired \d+, held \d+; faults (\d+)\.")
    fault_rows = [faults_pattern.fullmatch(line) for line in proving["summary"]]
    fault_rows = [m for m in fault_rows if m]
    require(len(fault_rows) == 1, "exact retained proving fault total is required")
    # Metrics.motor currently counts every mind in the game. Do not relabel this
    # as candidate-only or quietly discard an opponent's failures.
    all_mind_faults = int(fault_rows[0][1])
    embedded, external_counts = counts(embedded), counts(external)
    left, right = embedded["wilson_95"], external_counts["wilson_95"]
    separate = left[1] < right[0] or right[1] < left[0]
    point_rate = embedded["wins"] / embedded["samples"]
    inside_external_interval = right[0] <= point_rate <= right[1]
    failures = []
    if not inside_external_interval:
        failures.append("Field Pilot pooled point win rate is outside the fresh external bot's 95% Wilson interval")
    if all_mind_faults:
        failures.append(f"actual proving recorded {all_mind_faults} mind faults across both sides")
    result = {"format":1, "run_id":run["run_id"], "revision":run["revision"], "context":context,
              "source_sha256":source["source_sha256"], "embedded_rhai":embedded,
              "fresh_external_bot":external_counts, "intervals_do_not_overlap":separate,
              "field_pilot_point_win_rate":point_rate,
              "field_pilot_rate_in_external_wilson_95":inside_external_interval,
              "proving_faults":{"all_minds":all_mind_faults, "candidate":None, "opponent":None,
                                "scope":"The retained report counts both sides; separate candidate/opponent totals are unavailable."},
              "acceptance_passed":not failures, "acceptance_failures":failures,
              "acceptance_criterion":"Field Pilot pooled point win rate lies inside the fresh external bot's 95% Wilson interval, and actual proving records zero mind faults.",
              "interpretation":"95% intervals do not overlap" if separate else "Intervals overlap; no difference called",
              "sampling":"Each pilot is pooled across both sides. The external live seeds and embedded paired seeds are independent samples."}
    (directory / "comparison.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2))
    # Preserve the complete measured comparison even when its behavioral gate
    # fails. Interval overlap remains the separate difference-report rule.
    require(not failures, "; ".join(failures))


def run(directory):
    directory.mkdir(parents=False, exist_ok=False)
    revision = clean_revision()
    manifest = {"format":1, "run_id":str(uuid.uuid4()), "revision":revision,
                "started_unix_ms":time.time_ns() // 1_000_000}
    (directory / "acceptance-run.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    env = dict(os.environ, M6_ACCEPTANCE_REVISION=revision, M6_ACCEPTANCE_RUN_ID=manifest["run_id"],
               M6_AUTHOR_STORY_OUT=str(directory / "author"), M6_BOT_INTERVAL_OUT=str(directory / "external"))
    clean_revision(revision)
    tree = subprocess.check_output(["cargo", "tree", "-p", "mush-bot", "--edges", "normal,build", "--target", "all",
                                    "--prefix", "none", "--format", "{p}"], cwd=ROOT, env=env, text=True)
    (directory / "bot-runtime-dependencies.txt").write_text(tree, encoding="utf-8")
    clean_revision(revision)
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=ROOT, env=env, text=True))
    workspace = {p["name"] for p in metadata["packages"] if p["id"] in metadata["workspace_members"]}
    runtime = {line.split()[0] for line in tree.splitlines() if line.strip()}
    require((workspace & runtime) == {"mush-bot"}, "standalone bot gained a RustMUSH runtime/build dependency")
    commands = [
        ("picture-check", ["cargo","test","-p","tacsim","--lib","field_pilot_","--","--nocapture"]),
        ("author-story", ["cargo","test","-p","mush-bot","--test","arena_mind_author_story","--","--ignored","--nocapture"]),
        ("external-interval", ["cargo","test","-p","mush-bot","--test","arena_mind_port_rate","--","--ignored","--nocapture"]),
    ]
    for name, command in commands:
        clean_revision(revision)
        print("Running", name, flush=True)
        with (directory / f"{name}.log").open("w", encoding="utf-8") as log:
            subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    manifest["finished_unix_ms"] = time.time_ns() // 1_000_000
    (directory / "acceptance-run.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    compare(directory)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("run", "compare"))
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    {"run":run, "compare":compare}[args.action](args.directory.resolve())
