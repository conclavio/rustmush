#!/usr/bin/env bash
#
# harness-weekly.sh — the TacSim scale harness by hand, the way the CI
# `tacsim-harness` job runs it (16b cut U.3; 16c T-37 and Q29; D-16-19).
#
# GitHub CI is billing-blocked, so the box that actually runs the weekly harness
# is the owner's. This is that run as one command, from anywhere in the tree:
#
#   scripts/harness-weekly.sh
#
# (or `bash scripts/harness-weekly.sh` where a checkout lost the executable bit).
#
# It runs every ignored lane of crates/tacsim/tests/harness.rs in release —
# the lance lane (tacsim_harness_lance, 16b cut Z.1: twelve machines a side in
# lances of four under a scripted commander on the proving ground) among them —
# then the delta against the committed crates/tacsim/tests/harness_baseline/
# arena.<os>.json, and stops non-zero at the first failure. The reports land at
# the workspace root as harness*.json.
#
# What asserts where:
#   * this script exports TACSIM_ASSERT=1, so every lane asserts its absolutes
#     (D-16-19: assert locally — this box is not a shared runner);
#   * the authored-reach lane asserts too since 16b cut J.3b (16c Q26): 12 ms
#     per wake, 20 ms per 500 ms window (17.5 at J.3b, 20 since J.3c), the same
#     per-pulse bounds;
#   * the lance lane (16b cut Z.1) asserts W8 alone — the span and the orders
#     per commander per minute bounded, the tape equal to the script's orders
#     and the opening round landed at any knob, and the span and the busiest
#     minute equal to the squads a side and (16b cut Z.2) each side's
#     `order_released` lines from the script's release at the ninetieth arena
#     second where its first squad still stands (16b cut AE.1: at least one,
#     red's; blue-1 is all wrecks by then), and the orders counted over the
#     scripted wakes a machine stood to take (seven, pinned), held to two
#     facts of the fight, only at the lane's defaults (seed 2350,
#     TACSIM_LANCE_UNITS=24, TACSIM_LANCE_ARENA_S=120, jitter 0; off them the
#     fight decides those) — and prints its timing beside 12 ms and 20 ms,
#     report-only;
#   * the delta fails on any difference in the deterministic block, and on a
#     timing figure more than 20 per cent slower on the host that blessed the
#     file — a timing-only failure is re-run once before it is believed;
#   * the CI job sets no TACSIM_ASSERT and fails on the delta alone for a
#     measurement; the sight differential (tacsim_harness_sight_differential,
#     16b cut J.1) is a correctness lane that asserts always, so a sight
#     mismatch fails the first step, here and in CI.
#
# TACSIM_BASELINE_BLESS=1 scripts/harness-weekly.sh re-blesses this platform's
# baseline from the fresh report; an ordinary run never writes it.
#
# NEVER run this on a host that is serving players: it competes with the game
# server for every core, and a pulse past `stall_ms` voids a live match.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

# The environment is where this script and the CI job differ: CI sets
# TACSIM_JSON alone; this sets TACSIM_JSON and TACSIM_ASSERT, and passes
# TACSIM_BASELINE_BLESS (and any TACSIM_* sizing knob) through if the caller
# set it.
export TACSIM_JSON=harness.json
export TACSIM_ASSERT=1
if [ -n "${TACSIM_BASELINE_BLESS:-}" ]; then
  export TACSIM_BASELINE_BLESS
fi

# `.github/workflows/ci.yml`'s `tacsim-harness` step "TacSim harness", verbatim.
# Keep the two lines equal.
cargo test --release -p tacsim --test harness -- --ignored --test-threads=1 --nocapture --skip tacsim_harness_delta_against_the_committed_baseline

# `.github/workflows/ci.yml`'s step "Compare with the committed baseline",
# verbatim. Keep the two lines equal.
cargo test --release -p tacsim --test harness tacsim_harness_delta_against_the_committed_baseline -- --exact --ignored --nocapture
