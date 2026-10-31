# 27a. Cut notes — field unit survival and towing

One dated `##` heading per cut or correction of
[27a-field-unit-survival-and-towing.md](27a-field-unit-survival-and-towing.md), in the order
they land: what landed, what it deferred and why, how it was verified.

## 2026-10-31 — W-clock: one implementation; the `mush-tactics` field clock twin retired

The tree held the W-clock (doc 27 §5, FO-11 / Q-H11 / Q-R4 / Q-F3 / P-4) twice. This cut
leaves one.

**What was duplicated.** The kernel implementation, PR #90 (merged 2026-10-29), is the live
one: `mush_core::clock::WorldTimeline` (the durable checkpoint, `freeze_for_outage`,
`resume_at`, `reanchor_from_checkpoint`, `WorldClockFreezeKind::{Deliberate, OutageCountdown}`,
persisted under `WORLD_TIMELINE_STATE_KEY`), the `mush-server` boot policy
(`timeline.rs` `evaluate_boot_outage` / `apply_boot_outage_policy`, wired in `boot.rs`), the
world task's countdown step (5m / 2m / 1m / unfreeze broadcasts, `WorldClockResumed`), the
`[game]` knobs `outage_freeze_threshold_s` 30 and `outage_countdown_s` 300, the module events
`WorldClockFrozen` / `WorldClockResumed`, and the deliberate freeze verb
`@tacsim/time freeze|resume`. Its consumers already hang off the events: the economy's
`contract_live::{pause,resume}_real_time_deadlines` (Q-F3 on contract windows) and the
`tacsim` field module's boot / admit / tick holds (doc 32 CF.2). PR #137 (merged the same day)
added a second copy under `mush_tactics::field_ops::clock`: `FieldClockCoordinator` with its
own `OutageFreezeConfig` (its own 30 / 300 defaults), `evaluate_restart_gap`, `step_countdown`,
`OutageBroadcastMilestone`, `OutageReconnectBanner`, a `Deliberate` freeze, the P-4 unit
quarantine and the Q-F3 salvage-claim pause/extension, plus two bridges in
`tacsim::field_ops` (`resolve_world_outage_with_field_clock`,
`step_world_outage_countdown_with_field_clock`) that evaluated the outage gap a second time
and pushed the result into the kernel `WorldTimeline`. A grep before this cut confirmed that
the coordinator, its config and the two bridges had no caller outside `#[cfg(test)]`
(`mush-tactics/src/field_ops/mod.rs` tests, `tacsim/src/field_ops.rs` tests): no module state
held a coordinator and the server never called a bridge, so the twin's quarantine and
deadline behaviour existed only in those two tests. Doc 27a §7 described the twin and was
stale against it (it said threshold 120 s and milestones 5m/3m/1m/30s/10s; the code had 30 s
and 5m/2m/1m), and doc 32 attributed the W-clock to PR #137.

**What was deleted.** `crates/mush-tactics/src/field_ops/clock.rs` whole (900 lines):
`FieldClockCoordinator`, `OutageFreezeConfig` and its two `DEFAULT_OUTAGE_*` constants
(nothing live read them; the kernel's knobs are `mush-config`'s), `ActiveFieldFreeze`,
`FieldOutageResolution`, `OutageBroadcastMilestone`, `OutageReconnectBanner`,
`OutageUnfreezeReport`, `ClockFreezeError`. In `crates/tacsim/src/field_ops.rs`: the two
bridge functions, the `WorldOutageCountdownStep` alias, the `FieldOpsError::{Clock,
WorldTimeline}` variants that existed for them, the `mush_core::clock` import, and the bridge
test `w_clock_world_timeline_outage_freeze_countdown_and_salvage_claim_adjustment` (the
kernel's own tests in `mush-server`'s `timeline.rs` and `world_task/tests.rs` cover the outage
path it re-told). The twin's re-exports from both crates. The per-reconnect banner is not
carried anywhere: the kernel broadcasts the milestones to connected players and a connect-time
line would be the kernel's to add.

**What moved onto the kernel events.** The two behaviours the kernel does not have are
re-expressed as consumers of `WorldClockFrozen` / `WorldClockResumed`, the way
`contract_live.rs` extends contract deadlines:

- **P-4 / Q-H11 quarantine** → new `crates/mush-tactics/src/field_ops/quarantine.rs`:
  `OutageQuarantineBook` (`QuarantinedUnitRecord`, `MAX_QUARANTINED_UNITS` 512,
  `QuarantineError`). `audit_site_units_on_world_clock_resumed(event, site_book, ledger,
  units)` audits a site's units for a `WorldClockResumed` event, stamping each record from the
  event's `resumed_real_unix` / `ic_unix` (the twin computed the IC stamp from its own
  checkpoint — a second clock), and ignores every other event; the per-unit audit, the
  `ensure_unit_not_quarantined` gate and `release_quarantined_unit` are the twin's, unchanged.
  The twin's `ensure_unfrozen` gate is gone: the gate is `World::world_timeline().is_frozen()`,
  which the field module already uses.
- **Q-F3 salvage-claim deadline extension** → `SalvageClaimBook::apply_world_clock_event`
  in `salvage.rs`, over the book's existing `pause_claims_for_outage` /
  `resume_claims_after_outage` (which the twin merely called); `AdjustedSalvageClaimDeadline`
  moves into `salvage.rs`. `WorldClockFrozen` pauses at `real_unix`; `WorldClockResumed`
  pauses any pending claim the freeze event never reached at `frozen_real_unix` and resumes
  all at `resumed_real_unix`, so a claim is extended by the same `unavailable_secs` the
  economy applies to a window it finds open at the freeze. **The economy's Q-F3 path does not
  cover these deadlines**: `pause_real_time_deadlines` walks `EconomyStore` contracts with a
  `RealUtc` window; a `SalvageClaim` is a `mush-tactics` record with its own
  `deadline_real_s` / `paused_at_real_s`, never an `EconomicContract` (the economy test's
  contract is merely *named* "SalvageContest"). So the adjustment stays, as a consumer.
- Neither consumer is subscribed: nothing live holds a `SalvageClaimBook` or an
  `OutageQuarantineBook` (doc 32 §3: the field-ops library has one live caller,
  `assess_tacsim_unit`), which is the same situation the twin was in. The subscription is one
  `reg.subscribe(...)` line in whichever module comes to own the books.

**Docs.** Doc 27a: the `W-clock` row of the milestone table and §7 now describe the kernel
(30 s / 300 s, milestones 5m/2m/1m, no reconnect banner, who gates on the freeze) and the two
consumers. Doc 32: the §2 outage row and the §3 tree fact name PR #90 and the kernel, and
record the twin's retirement. The `field_ops` module docs in both crates say the same.

**Verified.** `rustfmt --edition 2024 --check` on the four changed `.rs` files; `cargo clippy -p mush-tactics -p tacsim --all-targets -- -D warnings` clean (3m 09s); `RUSTDOCFLAGS=-D warnings cargo doc -p mush-tactics -p tacsim --no-deps` clean. `cargo test -p mush-tactics`: 207 passed, 0 failed (the lib's 73, the new `w_clock_events_quarantine_mismatched_units_and_extend_salvage_claim_deadlines` among them, and the nine integration binaries; no doc-tests). `cargo test -p tacsim --lib -- field_ops`: 11 passed. `cargo test -p mush-server --lib -- timeline`: 6 passed, 1 ignored (the Redis round trip, as always without `REDIS_URL`), the kernel's two `w_clock_restart_*` stories among them. The four `mush-contract-tests` guards `panic_budget`, `restriction_lints`, `feature_matrix_inventory` and `ci_gated_test_inventory`: 31 passed; no row moved, because the deleted code carried no panic needle and the new code adds none.

**Not done here, and why.**

- The kernel's checkpoint persistence (`persist_world_clock_checkpoint` has no live caller; the
  first checkpoint is never persisted on a fresh store) is a sibling task's; `timeline.rs`'s
  persistence logic is untouched here.
- No live holder for either book, so no subscription: that is the field-ops wiring question
  doc 32 §3 records, not this cut's.
- No deploy.
