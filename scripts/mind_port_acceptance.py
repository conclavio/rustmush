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
import zlib

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "crates/tacsim/content/minds/acceptance/field_pilot.rhai"
MANIFEST = SOURCE.with_suffix(".manifest.json")
ENVELOPE = SOURCE.with_suffix(".envelope.json")
MAX_FROZEN_GAME_BYTES = 16 * 1024 * 1024


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


def retained_blob(directory, filename, descriptor):
    raw = (directory / "author" / filename).read_bytes()
    require(0 < len(raw) <= 16 * 1024 * 1024 and len(raw) == descriptor["bytes"]
            and list(hashlib.sha256(raw).digest()) == descriptor["sha256"],
            f"retained {filename} bytes differ from their exact artifact")
    return json.loads(raw)


def selected_profile(seal, expected):
    selected = seal["cards"]["selected"]
    require(len(selected) == 1 and selected[0]["name"] == expected,
            f"actual sealed selection is not {expected}")
    index = selected[0]["key"]["index"]
    profiles = seal["cards"]["definition"]["profiles"]
    require(type(index) is int and 0 <= index < len(profiles)
            and profiles[index]["name"] == expected, "sealed profile binding differs")
    return profiles[index]


def frozen_game(row):
    """Independently decode one exact retained format-2 game, under a fixed cap."""
    require(type(row) is dict and set(row) == {
        "seed", "swapped", "match_id", "unit_count", "raw_bytes", "raw_sha256", "zlib"},
        "frozen game summary shape")
    require(type(row["seed"]) is int and 0 <= row["seed"] <= 0xffffffffffffffff
            and type(row["swapped"]) is bool
            and type(row["match_id"]) is int and 0 < row["match_id"] <= 0xffffffffffffffff
            and type(row["unit_count"]) is int and 2 <= row["unit_count"] <= 12
            and type(row["raw_bytes"]) is int and 0 < row["raw_bytes"] <= MAX_FROZEN_GAME_BYTES,
            "frozen game summary bounds")
    encoded, digest = row["zlib"], row["raw_sha256"]
    require(type(encoded) is str and 0 < len(encoded) <= 2 * MAX_FROZEN_GAME_BYTES
            and len(encoded) % 2 == 0 and re.fullmatch(r"[0-9a-f]+", encoded) is not None
            and type(digest) is str and re.fullmatch(r"[0-9a-f]{64}", digest) is not None,
            "frozen game canonical hex or byte bound")
    decoder = zlib.decompressobj()
    try:
        # Never flush an unbounded remainder. One excess byte proves a forged
        # declared length without allowing a compressed bomb to grow in memory.
        raw = decoder.decompress(bytes.fromhex(encoded), row["raw_bytes"] + 1)
    except zlib.error as error:
        raise ValueError("frozen game invalid zlib stream") from error
    require(len(raw) == row["raw_bytes"] and decoder.eof
            and not decoder.unused_data and not decoder.unconsumed_tail,
            "frozen game length, truncated stream or trailing bytes")
    require(hashlib.sha256(raw).hexdigest() == digest, "frozen game raw digest differs")
    game = json.loads(raw)
    require(type(game) is dict and set(game) == {
        "seed", "swapped", "format", "match_id", "limit", "engine", "opening", "clock",
        "seals", "sources"}, "frozen game body shape")
    require(type(game["seed"]) is int and game["seed"] == row["seed"]
            and type(game["swapped"]) is bool and game["swapped"] is row["swapped"]
            and type(game["match_id"]) is int and game["match_id"] == row["match_id"]
            and type(game["format"]) is int and game["format"] == 1
            and type(game["seals"]) is list and len(game["seals"]) == row["unit_count"],
            "frozen game body differs from summary")
    return game


def verify_proving_artifacts(directory, story, proving):
    start = read(directory / "author/proving-start-manifest.json")
    require(start["key"]["account"] == proving["key"]["account"]
            and start["key"]["run"] == proving["key"]["run"] == 1
            and start["key"]["part"] == "start" and proving["key"]["part"] == "result",
            "retained proving start and result belong to different runs")
    require(start["artifact"] == story["proving_start_artifact"]
            and proving["artifact"] == story["proving_result_artifact"],
            "story does not bind both retained proving artifacts")
    frozen = retained_blob(directory, "proving-start.json", start["artifact"])
    result = retained_blob(directory, "proving-result.json", proving["artifact"])
    require(frozen["context"] == result["context"] == proving["context"],
            "retained proving contexts differ")
    games = result.get("report", {}).get("games")
    require(type(games) is list and len(games) == 30,
            "thirty actual retained proving game reports are required")
    seal = story["first_seal"]
    require(seal["requested"]["origin"]["digest"] == story["first_definition"],
            "live seal is not the first saved definition")
    require(seal["source_sha256"] == seal["requested"]["source_sha256"],
            "live effective source differs from the requested source")
    envelope = read(ENVELOPE)
    hold = selected_profile(seal, "Hold")
    require(seal["cards"]["definition"]["schema"] == envelope["schema"]
            and hold["values"] == envelope["profiles"]["Hold"]
            and hold["rules"] == envelope["rules"].get("Hold", []),
            "live seal does not carry the exact accepted Hold card/rules")
    require(type(frozen.get("format")) is int and frozen["format"] == 2,
            "current frozen proving format is required")
    require(len(frozen["games"]) == 30, "thirty actual frozen starts are required")
    for index, encoded_game in enumerate(frozen["games"]):
        game = frozen_game(encoded_game)
        require(game["swapped"] is (index % 2 == 1)
                and game["seed"] == frozen["context"]["pair_seeds"][index // 2],
                "actual frozen seed/side pairing differs")
        actual_game = games[index]
        require(type(actual_game.get("seed")) is int
                and actual_game["seed"] == game["seed"]
                and actual_game.get("swapped") is game["swapped"],
                "actual proving game seed/side order differs from its frozen start")
        require(actual_game.get("outcome") in ("win", "loss", "draw", "timeout"),
                "actual proving game outcome is missing or unclassified")
        require(type(actual_game.get("faults")) is int and 0 <= actual_game["faults"] <= 0xffffffff,
                "actual proving game fault count must be a nonnegative integer")
        require(len(game["seals"]) == 2, "actual proving is not 1v1")
        candidates = [s for s in game["seals"] if s["requested"]["mind"] == "FieldPilot"]
        require(len(candidates) == 1, "one actual Field Pilot seal is required")
        candidate = candidates[0]
        require(candidate["engine"] == "scripted" and candidate["class"] == "II"
                and candidate["source_sha256"] == seal["source_sha256"]
                and candidate["requested"] == seal["requested"]
                and candidate["cards"] == seal["cards"]
                and candidate["card_digest"] == seal["card_digest"],
                "proving did not execute the first exact sealed revision/card/rules")
        opponents = [s for s in game["seals"] if s is not candidate]
        opponent = opponents[0]
        actual = {"class":opponent["class"],
                  "profile":selected_profile(opponent, "Line")["name"],
                  "source_sha256":opponent["source_sha256"]}
        require(actual == frozen["context"]["opponent"],
                "actual proving opponent differs from its reported identity")
    return games


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
    actual_games = verify_proving_artifacts(directory, story, proving)
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
    require(all(game["outcome"] == pairs[index // 2][2 + index % 2].lower()
                for index, game in enumerate(actual_games)),
            "actual proving outcomes disagree with the retained summary")
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
    all_mind_faults = sum(game["faults"] for game in actual_games)
    require(all_mind_faults == int(fault_rows[0][1]),
            "actual proving fault counts disagree with the retained summary")
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
        ("live-isolation", ["cargo","test","-p","mush-bot","--test","arena_mind_isolation","--","--nocapture"]),
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
