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
# lances of four under a scripted commander on the proving ground) and the
# minds lane (tacsim_harness_arena_minds, 17b cut M4.2: the relief cohort as
# unit minds in four variants, fill first) among them — then the delta against
# the committed crates/tacsim/tests/harness_baseline/arena.<os>.json, and stops
# non-zero at the first failure. The reports land at the workspace root as
# harness*.json.
#
# Since 17b cut M4.5 every lane fields unit minds at the ROM (Class 0, the
# native stock mind) where it fielded the reflex controller: the arena
# report's `reflex` object is `motor` (no `ticks`, no `traces`) and its state
# histogram is `minds.states_at_end`, the minds' thinks and landings are what
# "the field decided something" reads, and the baseline is arena.linux.json
# (the owner's ruling of 2026-10-08), blessed on a Linux host by the
# non-asserting path docs/ops.md's *Blessing* gives (the lanes without
# TACSIM_ASSERT, then the delta with TACSIM_BASELINE_BLESS=1 on the same build)
# under a host name no later run carries, ccr-m4.5-linux-bless — the cloud
# containers all read /etc/hostname as `vm`, and a blessed host a later run
# matches makes the timing gate live there — so the timing gate was dormant
# everywhere. Since 16c T-109 (the owner, 2026-10-09) the lenses are the
# production VPS's (conclav.io) and the baseline is blessed THERE under the
# box's own host name, so the timing gate is live on the VPS and dormant on
# every other host, ALIEN18 included; run this script on the VPS with dev1's
# centre idle.
#
# What asserts where:
#   * this script exports TACSIM_ASSERT=1, so every lane asserts its absolutes
#     (D-16-19: assert locally — this box is not a shared runner);
#   * the lenses since 16c T-109: 20 ms per wake on both cohorts, 22 ms per
#     500 ms window on the CI cohort and 32 ms on the authored one, under the
#     40 ms engaged_p99_ms ceiling (ALIEN18's 12, 12, 20 and 25 before);
#   * the authored-reach lane asserts too since 16b cut J.3b (16c Q26): the
#     per-wake lens, its own per-window lens (17.5 at J.3b, 20 from J.3c to
#     T-109, 32 since), the same per-pulse bounds;
#   * the lance lane (16b cut Z.1) asserts W8 alone — the span and the orders
#     per commander per minute bounded, the tape equal to the script's orders,
#     the opening round landed and the lances fired at any knob, and the
#     busiest minute the squads a side and the span one fewer (17b cut M4.5,
#     the fight at the ROM: no end inside the window — twelve hulls wrecked,
#     neither side out — each first squad released, the second and third
#     squads' orders live on standing machines), and (16b cut Z.2) each
#     side's `order_released` lines from the script's release at the
#     ninetieth arena second where its first squad still stands (both sides
#     since 17b cut M4.5: red's three and blue's two, a line a standing
#     machine), and the orders counted over the scripted wakes a machine
#     stood to take (eight since 17b cut M4.5, pinned), held to facts of the
#     fight, only at the lane's defaults (seed 2350, TACSIM_LANCE_UNITS=24,
#     TACSIM_LANCE_ARENA_S=120, jitter 0; off them the fight decides those) —
#     and prints its timing beside the authored lenses, report-only. It fights at
#     the ROM since 17b cut M4.5c's row cut (a mind's view keeps every
#     contact not known friendly before a friend, so a hull among its own
#     lance still reads the enemy) and holds under TACSIM_ASSERT;
#   * the minds lane (17b cut M4.2) collects every check into one
#     harness_minds_verdict line per variant and panics ONCE at the end: `held=`
#     is exactly the plan's listed asserts (wake p99 the per-wake lens over the
#     unjittered wakes since 16c T-108 (a jittered wake steps two substeps; its
#     p99 is printed beside as wake_p99_other_ms; a run with no unjittered
#     wake does not hold), window p99 the
#     cohort's lens counting inline think time, faults nought on every variant,
#     ops nought on the stock variants (on every native-sealed variant since
#     the fill decision, the fill included), ops per window and per substep
#     within the pool, the load mind's ops p1 at least 4,300, the computed
#     CPU lens),
#     `sound=` the lane's integrity (a failed batch, a refused snapshot, a cold
#     field — a `sound=no` run is re-run, never counted). The fill variant's
#     `held=` over THREE idle-host runs of this script was the decision
#     `tacsim::match_::PRODUCTION_FILL_ENGINE` records (`held=yes` three times
#     keeps `scripted`, any `held=no` moves it to `native`) — TAKEN on
#     2026-10-09 from the owner's three ALIEN18 runs, all `held=no`: the fill
#     is `native`, so the fill variant now measures the production fill on
#     the native stock mind (no Rhai, ops nought) and its `held=` word is the
#     owner's capacity reading, no longer a decision input; a `stock_ci`
#     `held=no` was to be the owner's STOP before 17b
#     cut M4.5 — waived by the owner on 2026-10-08, the three runs still the
#     owner's. Its four harness-minds-<variant>.json reports carry a
#     deterministic `minds` block the delta compares against the baseline's
#     `minds.<variant>` (folded by 17b cut M4.5's bless, which refuses to bless
#     without all four reports fresh from this build);
#   * the delta fails on any difference in the deterministic block, and on a
#     timing figure more than 20 per cent slower on the host that blessed the
#     file — a timing-only failure is re-run once before it is believed;
#   * the CI job sets no TACSIM_ASSERT and fails on the delta alone for a
#     measurement; the sight differential (tacsim_harness_sight_differential,
#     16b cut J.1) is a correctness lane that asserts always, so a sight
#     mismatch fails the first step, here and in CI.
#
# TACSIM_BASELINE_BLESS=1 scripts/harness-weekly.sh re-blesses this platform's
# baseline from the fresh reports — the arena report and the four minds
# reports, all from this build — once every lane above has held: the delta is
# the second cargo line and `set -e` stops the script at the first, so a lane
# that fails under TACSIM_ASSERT on this host (a lens it misses, a minds
# verdict `held=no`) blesses nothing, and a bless needs a run in which every
# lane held here. Run the script again without the variable afterwards: the
# delta then reads the blessed file equal. An ordinary run never writes it.
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
