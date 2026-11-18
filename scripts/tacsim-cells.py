#!/usr/bin/env python3
"""tacsim's integration cells, balanced by measured test time.

`.github/tacsim-integration-shards.txt` names the domain binaries every
`tacsim-integration-N` cell builds (`partition:`), the whole targets a cell
runs first (`N: target ...`) and, in `weight:` lines, the measured cost of
the slow tests: `weight: <seconds> <binary>::<test>` for a test of a domain
binary, `weight: <seconds> <target>` for a whole target (its cost to its cell
in test-thread seconds: cost_lane takes the whole pool, so four times its
wall). This script turns those into each cell's share.

    plan N <manifest> <list.json>
        <list.json> is `cargo nextest list --message-format json` over the
        partitioned binaries. Every test that would run is placed by greedy
        longest-first (LPT): the weighted tests by their weight, the rest at
        the mean weight of an unweighted test (the `default-weight:` line),
        each onto the cell with the least load so far, ties to the lower
        cell, starting from the whole targets' weights. Every cell computes
        the same plan from the same list, so each test runs in exactly one
        cell. Prints cell N's nextest filterset on stdout and the plan (every
        cell's planned minutes, the list's digest) on stderr.

    weights <junit dir> ...
        Each <junit dir> is one CI run's downloaded `nextest-junit-tacsim-
        integration-*` artifacts (`gh run download <run> -p
        'nextest-junit-tacsim-integration-*' -D <dir>`). Prints the `weight:`
        and `default-weight:` lines for the manifest: the mean time of every
        partitioned test whose mean is at least THRESHOLD seconds, and the
        mean of the rest.
"""

import collections
import glob
import hashlib
import json
import os
import statistics
import sys
import xml.etree.ElementTree as ET

THRESHOLD = 60.0


def read_manifest(path):
    partition, cells, weights, default = None, {}, {}, None
    with open(path, encoding="utf-8") as fh:
        for raw in fh:
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            key, _, rest = line.partition(":")
            key, words = key.strip(), rest.split()
            if key == "partition":
                partition = words
            elif key == "weight":
                weights[words[1]] = float(words[0])
            elif key == "default-weight":
                default = float(words[0])
            else:
                cells[int(key)] = words
    if partition is None or default is None:
        sys.exit(f"{path}: no partition: or default-weight: line")
    return partition, cells, weights, default


def plan(cell, manifest, listing):
    partition, cells, weights, default = read_manifest(manifest)
    count = len(cells)
    if cell not in cells:
        sys.exit(f"no cell {cell} in {manifest}")
    with open(listing, encoding="utf-8") as fh:
        suites = json.load(fh)["rust-suites"]
    tests = []
    for suite in suites.values():
        binary = suite["binary-id"].split("::", 1)[-1]
        if binary not in partition:
            continue
        for name, case in suite["testcases"].items():
            if case["filter-match"]["status"] != "matches":
                continue
            tests.append((binary, name))
    if not tests:
        sys.exit("the listing has no tests of the partitioned binaries")
    tests.sort(key=lambda t: (-weights.get(f"{t[0]}::{t[1]}", default), t))
    load = {n: sum(weights.get(target, 0.0) for target in cells[n]) for n in cells}
    mine = collections.defaultdict(list)
    for binary, name in tests:
        n = min(load, key=lambda k: (load[k], k))
        load[n] += weights.get(f"{binary}::{name}", default)
        if n == cell:
            mine[binary].append(name)
    digest = hashlib.sha256(
        "\n".join(f"{b}::{n}" for b, n in sorted(tests)).encode()
    ).hexdigest()[:12]
    print(
        f"tacsim cells: {len(tests)} tests (list {digest}), planned test-minutes "
        + " ".join(f"{n}:{load[n] / 60:.1f}" for n in sorted(load))
        + f"; cell {cell} runs {sum(map(len, mine.values()))}",
        file=sys.stderr,
    )
    if not mine:
        sys.exit(f"cell {cell} was planned no tests")
    terms = []
    for binary in sorted(mine):
        names = " | ".join(f"test(={name})" for name in sorted(mine[binary]))
        terms.append(f"(binary_id(tacsim::{binary}) & ({names}))")
    print(" | ".join(terms))


def weights(dirs):
    per = collections.defaultdict(list)
    for d in dirs:
        for path in glob.glob(os.path.join(d, "**", "junit.xml"), recursive=True):
            for case in ET.parse(path).getroot().iter("testcase"):
                binary = case.get("classname").split("::", 1)[-1]
                per[f"{binary}::{case.get('name')}"].append(float(case.get("time", 0)))
    if not per:
        sys.exit("no junit.xml under the given directories")
    means = {k: statistics.mean(v) for k, v in per.items()}
    slow = sorted((k for k in means if means[k] >= THRESHOLD), key=lambda k: (-means[k], k))
    rest = [v for k, v in means.items() if means[k] < THRESHOLD]
    print(f"default-weight: {statistics.mean(rest):.1f}")
    for k in slow:
        print(f"weight: {means[k]:.0f} {k}")


def main(argv):
    if len(argv) == 4 and argv[0] == "plan":
        plan(int(argv[1]), argv[2], argv[3])
    elif len(argv) >= 2 and argv[0] == "weights":
        weights(argv[1:])
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
