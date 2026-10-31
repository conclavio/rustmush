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

## 2026-10-31 — W-clock: the checkpoint is written (the outage policy arms on a live world) — PR #245

Doc 33 GG.5 found, on both boxes the day they were switched to the gateway generations, that
every boot — the first on each wiped store and the hot restart one to four minutes later —
logged `boot_outage=NoPriorCheckpoint`, and that neither store held a world-timeline state
key afterwards. The IC clock itself was never wrong (`ic = epoch + rate × real`); what was
inert was the whole of W-clock / Q-R4: no restart could be measured, so no restart could
freeze the world or re-anchor it.

**What was inert, and why.** Two writers in `crates/mush-server/src/timeline.rs`, neither of
which wrote on a live world:

- `apply_boot_outage_policy` evaluated the hydrated timeline against the boot and persisted
  the result only when state was already on the store or a freeze had been decided
  (`had_persisted_state || OutageFreeze || DeliberateFreeze`). A fresh store takes the
  `NoPriorCheckpoint` branch, which records the first checkpoint in memory — and was the one
  branch the condition excluded. So the world seeded from `[game]` never wrote its first
  checkpoint, and every later restart found an empty state and was a first boot again.
- `persist_world_clock_checkpoint`, the inline writer meant to move the checkpoint forward
  while the server runs, had no caller outside its own two tests. Even a world with persisted
  state would therefore have measured an outage from its previous *boot*, not from the moment
  it went down: a world up for a day and restarted in two seconds would have read a day lost.

**What now writes.**

- The boot persists its evaluation unconditionally: one small durable `WriteBatch`
  (`serial: BOOT_SERIAL`, one `world_timeline_state_op`), the shape the conditional write
  already had. Every evaluation records the boot as the checkpoint (`record_checkpoint`,
  `reanchor_from_checkpoint`, `resume_at` and `freeze_for_outage` all set it; a clock that did
  not advance is the one exception, and re-writing the unchanged state is harmless), so on a
  fresh store the write is the world's first durable checkpoint. The `had_persisted_state`
  test is gone.
- Every authoritative batch carries the checkpoint of its own second. `flush_dirty`
  (`crates/mush-server/src/world_task/commit_pipeline.rs`) asks `world_clock_checkpoint_rider`
  (`world_task.rs`) for it once the batch is known to be non-empty: when the world's `now` is
  past the timeline's last checkpoint and the clock is not frozen, the rider records the
  checkpoint at `now` and the batch gets the timeline state op *first*, ahead of the writes it
  covers. One op per second at most; none within the checkpoint's own second, none while
  frozen, none for a world with no checkpoint (a rig that never ran the boot policy). The
  batch is one `MULTI`/`EXEC`, so the store's checkpoint is never earlier than any IC stamp
  the store holds, and a restart re-anchors the IC clock at or after every persisted write —
  the invariant the first CI run showed to be missing (below).
- The world task checkpoints an idle world every `[game].world_clock_checkpoint_s` (new;
  default **10 s**, `mush_config::config::DEFAULT_WORLD_CLOCK_CHECKPOINT_S`; `0` is treated as
  `1`): `WorldClockCheckpointCadence` and `step_world_clock_checkpoint` in
  `crates/mush-server/src/world_task.rs`. The due time is derived from the timeline's own
  checkpoint — a full cadence after whichever writer set it last: the boot, a batch, the step,
  a resume — so a world that is writing runs no periodic checkpoints at all. The step runs
  once per loop iteration, right after the loop stamps `engine.set_now`, only where a timer
  may write (`admission_for(…, Timer) == Allow`: never while booting, recovering,
  commit-pending, store-read-only or integrity-frozen) and never while the clock is frozen — a
  frozen timeline must not advance its checkpoint, because the checkpoint is the pre-outage
  moment the countdown holds the world at. When due it records the checkpoint, queues one
  `Deferred::Persist` of the timeline state op (the path the countdown's unfreeze already
  uses) and drains it into the engine's module ops with `finish_external_step`, so the op
  rides that iteration's foot-of-loop batch whatever the wake was. The due time is folded into
  the loop's timer wake (`next_due = min(sched.next_due(), checkpoint due)`), so an idle world
  still writes it on the cadence; the cost is one comparison per iteration and one small JSON
  document (the timeline state) per cadence. An overdue checkpoint — after a long turn — is
  written on the first eligible iteration, and the next is a full cadence after that write.
  `persist_world_clock_checkpoint` stays as the boot-path tests' inline planter, and its doc
  now says so.
- The cadence travels the way `[llm].decision_workers` does: `BootedEngine::
  world_clock_checkpoint_s`, read once in `build_engine`, into
  `spawn_persist_with_gateway_production` (and `mushd`'s `spawn_persist_with_decision_workers`)
  as a parameter, through `spawn_persist_with_gateway_mode` into `run`. The plain spawns
  (`spawn_persist_with_gateway`, `…_and_outbox`, `…_and_decisions`) use the default, so no test
  rig changed shape but the handful that call the production or mode spawn directly.
  `ServerDeps` is untouched: forty-odd hand-built literals in the `mushd` stories would have
  had to grow a field.

**Why 10 s and not 60.** The next boot measures its outage from the last checkpoint, so an
idle world's measured downtime over-reads by up to one cadence (a world that was writing
measures from its last batch). Under the default threshold of 30 s a 60 s cadence would have
turned roughly half of all two-second hot restarts (`SERVER_RESTART=1 deploy.sh` on either
box) of a quiet world into five-minute freezes. At 10 s a restart reads as at most ten seconds
longer than it was. `build_engine` warns when the cadence is not below the threshold
(`[game].world_clock_checkpoint_s is not below outage_freeze_threshold_s`), and the boot's
`world timeline installed` line now carries `checkpoint_cadence_s`. `mushd.toml.example`,
`docs/ops.md` and doc 27a §7 document the three knobs together.

**The first CI run: the clock rewound (fixed here, same PR).** The K8 matrix's split-process
story `relay_queue_and_frozen_leg_survive_process_death_redis_gated`
(`crates/mushd/tests/durable_process/durability_e2e.rs`) boots one Redis store three times with
a hard kill between boots, each run a few seconds long. On the first commit the second boot
re-anchored silently (`lost_secs: 6`), ran seven seconds, and the third boot (`lost_secs: 9`)
died in module boot with `invalid or over-bound relay service state`. The relay was the
witness, not the cause: the boot re-anchors the IC clock *at* the last checkpoint, and with a
10 s cadence and runs shorter than that, the last checkpoint was always the previous boot's —
so every restart set the IC clock back to the previous boot's second, and every IC stamp
written since lay in the future. The second run transmitted the envelope the first had queued
behind a jammed node: its receipt read `queued` (the first run's IC second) later than
`transmitted` (the second run's rewound one), and `mush_propagation::service::State::validate`
refuses exactly that at the next load. The rewind was latent in "measure from the last
checkpoint" with any cadence at all — a crash always lands between checkpoints — and could
not show before this PR only because the policy never ran.

The fix is the batch rider above: the checkpoint is exact for every persisted write, so the
re-anchor can never land before a stamp the store holds; the cadence is only the idle floor.
The regression test is
`w_clock_a_batch_carries_the_checkpoint_of_its_second_so_a_restart_never_rewinds_the_clock`
(`world_task/tests.rs`): through the real `flush_dirty`, a write at 1 007 s puts the timeline
op first in its batch, a second batch in the same second carries none, a boot at 1 019 s
after a crash reads `SilentReanchor { lost_secs: 12 }` and resumes the IC clock exactly at the
write's IC second (without the rider: 19 s lost, and 28 IC seconds *before* the write), and a
frozen clock's batch carries none. The periodic test now also covers the derived due time:
after a resume the step is due a cadence later, not at once, since the resume is a checkpoint.

**Verified.** `rustfmt --edition 2024 --check` on every changed `.rs` file; `cargo clippy -p
mush-config -p mush-server -p mushd -p mush-bot --all-targets -- -D warnings` clean (every
test target of the four crates, so every caller of the changed spawn signatures compiles);
`RUSTDOCFLAGS=-D warnings cargo doc --no-deps -p mush-config -p mush-server -p mushd` clean.
`cargo test -p mush-server --lib -- w_clock timeline:: h2a_the_production_spawn`: 12 passed, 1
ignored (the Redis round trip, as always without `REDIS_URL`) — the three new stories among
them, the live-loop one in well under a second. `cargo test -p mush-server --test
split_boundary -- boot_parity`: 23 passed, `the_engine_knobs_reach_the_booted_world` now
asserting `[game].world_clock_checkpoint_s = 3` reaches `BootedEngine`.
`live_hard_modes_block_every_mutating_execution_seam` (the one test that calls the mode spawn
directly) passed. `cargo test -p mush-config --lib -- defaults_when_missing` passed with the
new default and the cadence-under-threshold assertion. The four `mush-contract-tests` guards
`feature_matrix_inventory`, `panic_budget`, `restriction_lints` and `ci_gated_test_inventory`:
31 passed; no row moved, because the new code adds no panic needle, no feature and no lint
allowance. Not run: the `mushd` stories (clippy compiled them; their spawn calls use the default
cadence and no behaviour of theirs changed), and anything on a box.

**Verified, the rider commit.** `rustfmt --edition 2024` on the three changed `.rs` files; `cargo
clippy -p mush-config -p mush-server --all-targets -- -D warnings` clean (the renamed type and the
rider live in mush-server; no spawn signature changed); `RUSTDOCFLAGS=-D warnings cargo doc
--no-deps -p mush-config -p mush-server` clean. `cargo test -p mush-server --lib -- w_clock
timeline:: flush_dirty h2a_the_production_spawn`: 14 passed, 1 ignored (the rider test and the
reworked periodic test among them). `--test split_boundary -- boot_parity`: 23 passed again (the
production boot and flush path, now with the rider). The four contract guards: 31 passed. The
failing K8 story itself, `cargo test -p mushd --test durable_process
durability_e2e::relay_queue_and_frozen_leg_survive_process_death_redis_gated -- --exact
--test-threads=1`, on conclav.dev (`cfg(unix)`, a throwaway clone of `2df965e8` plus this diff, a
private Redis 8.10 on port 6391): 1 passed in 12.3 s at the first attempt; the two sibling stories
of the same CI step, `kill9_mid_play_world_survives_redis_gated` and
`clean_shutdown_drains_and_snapshots`, pass there too. Not run: the gateway half of that step
(`split_halves authoritative_process_kill::`, the CI's), and anything on a box's live instance.

**Not done here, and why.**

- **No checkpoint at a clean shutdown of an idle world.** The shutdown's final batch carries
  the checkpoint of its second like any other, so a world that was writing measures a clean
  restart exactly; one whose last second wrote nothing still measures "the real downtime plus
  up to one cadence". With a 10 s cadence under a 30 s threshold the difference never changes
  the decision, so a checkpoint forced into the shutdown tail is a follow-up.
- **The K3 prepared-action lane carries no checkpoint.** `flush_prepared_action` sends a
  closed, self-hashed `AuthoritativeBundle` (typed `ReplayWrite`s, no module ops), so a crash
  between an authoritative action commit and the next legacy batch is still measured from the
  last checkpoint — the cadence bound, as before this cut — and any IC stamp inside that
  bundle may lie up to one cadence ahead of the re-anchored clock. Threading the timeline op
  into the bundle, or a checkpoint-only legacy batch ahead of it in the writer's FIFO, is a
  change to the exact-token lane's invariants and is not made here.
- **A restart during a countdown still restarts the countdown in full**, as doc 27a §7
  states: the checkpoint does not advance while frozen, so a hot restart two minutes into the
  five is measured from the pre-outage checkpoint and read as a further outage. That is the
  documented rule; whether a sub-threshold restart mid-countdown should instead resume the
  running countdown is the owner's ruling, not this cut's.
- **No deploy.** Both boxes run `d4d81c89` on fresh stores whose namespaces hold no state
  key, so the first boot on this code logs `NoPriorCheckpoint` once more — and writes it; every
  boot after that measures a real gap.
