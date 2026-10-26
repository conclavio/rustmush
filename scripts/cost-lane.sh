#!/usr/bin/env bash
#
# cost-lane.sh — the surface-mining cost lane by hand (doc 20 cut SM.12c-lane; doc 20 §11, §13's
# SM.12 row; doc 20a §5.3; the owner's O-28 default (i), provisional).
#
# No CI job runs the cost lane: not on push, not on a dispatch, not on the daily schedule. The
# `.github/workflows/ci.yml` comment block "Doc 20 cut SM.12c-lane" names both tests and carries
# the two cargo lines below verbatim (crates/tacsim/tests/cost_lane.rs's C5 pin keeps them equal).
# This is the run as one command, from anywhere in the tree:
#
#   scripts/cost-lane.sh
#
# (or `bash scripts/cost-lane.sh` where a checkout lost the executable bit).
#
# It runs, in release and in this order:
#   * sm12c_the_surface_tick_is_measured — the surface tick's gate on the shipped lane: ten TVs
#     (four pilots of two rigs and the Occator yard's two), 1,800 one-second fires (12 IC hours at
#     world rate 24), five runs from fresh boots; the minimum of the five runs' surface-fire p99s
#     under 5,000 us (doc 20 §11's Tick row), always asserted in release. CI runs this test in
#     debug only, with generous ceilings, so its 5 ms line has its one machine-run home here;
#   * sm12c_cost_lane_cohort_over_an_ic_month — the #[ignore]d month: TACSIM_COHORT_TVS TVs (30
#     here: 15 Dray-P hulls of two rigs; 64 is the test's default and the content bounds' reach),
#     a test-only seed variant of the shipped lane that edits only existing levers, over the
#     lane's six sites, every act cycling, Occator's yard and both mills running, 30 IC days
#     at world rate 24 in one-second fires, one reboot at IC day 15. It writes cost-lane.json at
#     the workspace root once an IC day, asserts its deterministic invariants always (every gated
#     act kind within its write ceiling, the money total, the frames under 256 KiB, every
#     Ceres-site pilot looping), and its timing absolute (the month's surface-fire p99 under
#     5,000 us) only under TACSIM_ASSERT=1, which this script exports. It STOPs (cost-lane.json
#     first, then a panic naming it) only when the applied-marker mirror reads incomplete or comes
#     within 3,900 of 4,096 (O-27); the 3,072 crossing is a figure, never a halt.
#
# The knobs, passed through when set:
#   * TACSIM_COHORT_TVS — the cohort's TVs, even, 2..=64; this script sets 30 when unset, the
#     cohort the recorded month was taken at. The test's own default, 64, kits up since the owner
#     raised the economy's per-party cap of live contracts (mush_economy::MAX_LIVE_CONTRACTS_PER_PARTY)
#     from 32 to 100 on 2026-10-26 (O-19c); a one-IC-day smoke at 64 ran green that day;
#   * TACSIM_COHORT_IC_DAYS — the IC days, 1..=30 (default 30; 5 is the smoke the month's order
#     runs first, doc 20a §5.3.9).
#
# The report: cost-lane.json at the workspace root (ignored by .gitignore's /cost-lane*.json).
#
# The rule for the box: run it after scripts/harness-weekly.sh, never beside it, and never on a
# host serving players. The month competes with everything else on the box for its cores, and its
# timing figures are the box's own (D-H-8, D-16-19: assert locally, never on a shared runner).

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

# The environment: the absolutes asserted on this box, the report at the root, the sizing knobs
# passed through if the caller set them.
export TACSIM_ASSERT=1
export TACSIM_COST_JSON=cost-lane.json
export TACSIM_COHORT_TVS="${TACSIM_COHORT_TVS:-30}"
if [ -n "${TACSIM_COHORT_IC_DAYS:-}" ]; then
  export TACSIM_COHORT_IC_DAYS
fi

# `.github/workflows/ci.yml`'s "Doc 20 cut SM.12c-lane" comment block, verbatim. Keep the two
# lines equal.
cargo test --release -p tacsim --test cost_lane sm12c_the_surface_tick_is_measured -- --exact --nocapture
cargo test --release -p tacsim --test cost_lane sm12c_cost_lane_cohort_over_an_ic_month -- --ignored --exact --test-threads=1 --nocapture
