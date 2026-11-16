# 49. Combat content — cut notes

One section a unit of [doc 49](49-combat-content.md) §7. A unit's session replaces its own "Not built." line with
what it built, under these headings: **Survey**, **Built**, **Tests**, **Divergences**, **Not built**.
Each session edits only its own section, so two units' notes never touch the same lines.

## CC.0

Tests and the bot's rig read content (Q-49-16). Built 2026-11-15 on `a5018131b` (PR #622's tip).
It changes only test files. It makes no content or kernel change, and no pin is re-blessed.

**Survey.** §3.6's list was a grep. To find what a retune really breaks, I ran a probe in a
throwaway clone on conclav.dev. It moved every ground mount's cycle and every round's load at
once:
- cycles: cannon 6,000 to 7,000 ms, coaxial gun 1,000 to 2,000, autocannon 3,000 to 4,000,
  laser 2,000 to 3,000, missile rack 10,000 to 11,000;
- loads: 40 to 36, 2,000 to 1,800, 240 to 216, 500 to 450, 6 to 5.

It re-signed the ten packs with `cargo run -p mush-content --example pack_digest` and ran the
tests of nine crates with no fail-fast. The crates were `tacsim`, `mushd`, `mush-bot`,
`mush-gateway`, `mush-server`, `mush-net-ssh`, `mush-campaign`, `mush-ansi` and
`mush-contract-tests`. The probe left out `mush-bot`'s mining cohort and `tacsim`'s cost lane,
which have no ground mount.

The probe ran 7,567 tests and 45 failed. They fall into four groups:

| Group | Tests | Whose |
|---|---|---|
| Literal loads and cycles, and timelines built on them (17) | the sim lenses' `arena_story`, `sim_rematch` (2), `sim_units` (4), `sim_weapons`, `observer_rule` (2); `match_::cockpit::tests::ae2_…`; `domain_arena`'s `definitions::a_value_past_a_bound…`, `extensions::ae2_the_autocannon…` and `ae2_the_missile…`, `weapons::ae2_one_missile…`, `conditions::m3_a_third_slot…` and `m1b2_t1_…` | CC.0: built below |
| The same, in files outside CC.0's row (3) | `mush-gateway`'s `arena_story_split` (its own `LOAD = 40`); `domain_world/voyage.rs` gc7 (a rearm quote of 10, from 40 less 30); `domain_economy/yard_in9.rs` (`rearm cannon (40 rounds`, `0/2000 rounds`) | see **Not built** |
| Content pins (20) | see **Not built**: CC.2's one re-bless | CC.2 |
| Fights on the shipped tank, by seed (5) | `report.rs`' `m5_…` (3) and `v3_…`, `replay.rs`' `i1_…`: the run must end in a kill, a ROM drop, or a pinned shot count | CC.2, with the corpus pins |

The brief named some places that needed no change:
- `docs/help/cockpit.md` quotes no shipped cycle or load. Its `Still cycling: 4 s to go.` and
  `machine_gun (mg) cycling (3 s)` are readings of hand-built scenes, held there by
  `ap3_watch_lines_are_the_three_lines_within_120_columns` and the `fixture::frame()` tests. Each
  is still a possible reading under CC.2's palette.
- The web cockpit fixtures (`web/src/lib/fixtures/cockpit/`) are hand-built too: a cannon at 14 of
  20 rounds with 1 km and 3 km bands, and a coaxial at 380 of 500 that is 900 ms from ready. No pack
  feeds them, so a retune moves none of them.
- Of the "cycling (" files, every one the probe did not break formats or parses hand-built rows:
  - the bot's parser fixtures in `mush-bot/src/arena/` (CC.10's);
  - `cockpit/tests.rs`' renderer cases;
  - `domain_arena`'s own fixture mounts (`weapons.rs`' `cannon()` at 4,000 ms and 40 rounds,
    `support/rig.rs`, the `count_per_load: 40` fixtures).

**Built.**
- **`crates/mushd/tests/sim_lenses/shipped.rs`** (a helper module of the group binary):
  - `mount(unit, component)` gives a shipped mount's `burst` and load. It reads them from
    `tacsim::builtin_definitions()`, the catalog `SIM UNIT` binds through. No lens asserts a
    cycle's length: each reads `cycling (` and nothing after it.
  - `Mount::full()` gives `N of N round(s)`, and `Mount::left(spent)` gives the same with `spent`
    rounds gone.
- **The sim lenses** read every load and spent count through it:
  - `arena_story`'s and `sim_rematch`'s `LOAD` and `sim_units`' at-spec `LOAD` became `load()`.
  - `arena_story`'s row parser tells the cannon's row from the coaxial's by the load alone, so
    `load()` now asserts the two loads differ.
  - In `sim_units`, the car's, the heavy's and the rack's rows, the one-burst spend and the burst
    on the tape come from the helper.
  - `sim_weapons`' full and one-down rows, and `observer_rule`'s spent-round needles, come from the
    helper. `observer_rule` also asserts one round a pull.
- **`match_::cockpit::tests`**:
  - `fielded_summaries` and `fielded_mount(entry, id)` read the fielded content.
  - `ae2_the_bare_fire_is_the_authors_choice_by_component_order` builds its expected rows from
    the loads. The rack's cycling row comes from the rack's own cycle and load.
- **`domain_arena`**:
  - `definitions`: the bound test edits the coaxial's shipped cycle, read from the pack, to one
    under `CYCLE_FLOOR_MS`. It no longer replaces the literal `"cycle_ms": 1000`, so it follows
    CC.2's floor as well.
  - `extensions`: the autocannon and rack tests read each mount's load and cycle off the unit's
    own mount rows.
  - `weapons`: the three-burst run spaces its pulls by the car's cycle, rounded up to the coarsest
    cadence (5,000 ms at today's 3,000), and expects the load less nine.
  - `conditions::m3_…`: the run is sized from the heavy's cannon and rack cycles. It ends on a pulse
    at every cadence, past the cannon's third shot and the rack's second, and compares shots before
    the end.
  - `conditions`' laser field: red's two cycles are the field's own, `LASER_FIELD_CYCLES` (2,000
    and 1,000 ms). `COOLS_AT` and `UNLOCKS_AT` are built on those cycles. See **Divergences**.
- **The bot's scripted rig** (`mush-bot/tests/arena_lane.rs`):
  - `Server::cycle_ms` reads the line tank's mount cycles from the content (`shipped_cycles`).
    This replaces the hard-coded 6,000 ms for the cannon and 1,000 ms for the rest.
  - `cycle_left` asks for a gun's last shot before its cycle, so a gun the tank does not carry
    never reaches the content lookup.
  - The lattice tests' bounds (`1_000 + 2 * LATTICE_MS`, `6_000 + 2 * LATTICE_MS`) read the
    cycle.
  - The lead's full-cadence bounds keep their margins over the cycle: 200 ms for the coaxial and
    500 ms for the cannon (1,200 and 6,500 ms today).
  - The lane itself still learns each cycle from the rows the rig prints, as it does live.
    `AGENTS.md`'s rule holds: game crates appear only in the fixture.

**Tests.** All runs were on conclav.dev, in a throwaway clone at `a5018131b`, with the cut's patch
applied:
- **Today's values.** The four touched binaries, `tacsim`'s lib tests, `tacsim::domain_arena`,
  `mushd::sim_lenses` and `mush-bot::arena_lane`, ran 3,673 tests and all passed.
- **The probe, again, with the cut applied.** It ran the same 7,567 tests and 30 failed, down
  from 45. Every one of the 30 is in **Not built**:
  - 20 content pins;
  - five seeded fights;
  - three tests in files outside the row;
  - two bot lane tests. One of them, `full_rate_on_each_guns_own_cycle`, failed on the lead's
    literal bounds, which this cut then made read the cycle. Rerun under the probe,
    `mush-bot::arena_lane` passes 86 of its 87; the one left is CC.10's (below). So 29 remain
    under the probe.

  Every CC.0-group test above passes under the probe, among them `observer_rule`'s
  `nothing_of_the_shooting_crosses…`.
- **Guards**, at today's values. All passed:
  - `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` for `tacsim`, `mushd` and
    `mush-bot`, on the final patch;
  - the CI lint job's M7 memory-size pass on `tacsim`;
  - `mush-contract-tests`' `ci_policy` guards (`feature_matrix_inventory`, `panic_budget`,
    `restriction_lints`: 24 passed);
  - `cargo doc --no-deps --document-private-items` with `-D warnings` for the three crates.

  The last three ran one edit earlier. That edit took an unused field out of the mushd helper,
  which none of the three reads.

The probe's edits never reached this worktree. It ran in the clone, and the clone was deleted.

**Divergences.**
- **The laser field holds its own cycles** instead of reading them. Its timeline is a chain of
  coincidences on the 2,000 ms laser:
  - the laser reaches its ceiling by `COOLS_AT`;
  - `UNLOCKS_AT` falls so that the fresh lock's think finds the laser ready.

  With a 3,000 ms laser, the think cites the coaxial instead. Reading the cycle would mean a new
  timeline every retune. The test is about T1's arming and heat, not the shipped laser's cycle, so
  the fixture keeps its own (Q-49-16's aim, that a retune moves only data, holds).
- **The burst is read too**, where a load's arithmetic needs it (one burst spent). Damage and
  bands stay literal: they are not a cycle or a load.
- **The bot lane's lead bounds** are full-cadence assertions, which Q-49-17 hands to CC.10 to make
  discipline assertions. CC.0 only reads the cycle under them.

**Not built.**
- **Three files outside CC.0's row** still pin the cannon's load:
  - `crates/mush-gateway/tests/split_halves/arena_story_split.rs` has `LOAD = 40`. It is
    `arena_story`'s gateway twin, and `load()` and `crate::shipped`'s pattern fit it as they stand.
  - `crates/tacsim/tests/domain_world/voyage.rs` gc7 asserts a rearm quote of 10.
  - `crates/tacsim/tests/domain_economy/yard_in9.rs` asserts `rearm cannon (40 rounds` and
    `0/2000 rounds`.

  CC.0 asked the session that cut it before touching them and had no answer by the PR. Each is
  a few lines, and CC.2 meets all three at its retune.
- **Content pins, for CC.2's one re-bless.** The probe moved all of these:
  - `domain_arena/content.rs`' `DIGESTS`;
  - `extensions.rs`' pins of the shipped tank's definition (`ae1`, `ae3`, `x2` to `x6`), and its
    ordnance and laser pack pins (`ae2_the_ordnance_loads…` and `x1_…`). The ordnance test also
    spells out the authored autocannon and rack values as a tuple, which is a content pin by
    design;
  - `match_::seal`'s `m5rules_default_seal_identities_are_pinned`;
  - the registry's `C4_TRIAL_PIN` (`minds::registry::trials`);
  - `motor.rs`' `m1a1_the_shipped_duel_hashes_as_before`, `m1b0_…` and `gc1b_…`. §3.6 guessed
    `DUEL_HASH_AT_HEAD` would not move; the shipped duel does;
  - `minds.rs`' `m1a5_the_production_hash_does_not_move`;
  - `twin.rs`' three `m2_gate_a_…` tests and `M4_5_POOLED_RECORD`.

  The arena harness baseline is an `#[ignore]`d job, so the probe did not run it.
- **Seeded fights on the shipped tank**, for CC.2 with the corpus pins: `report.rs`' three
  `m5_…` tests and `v3_…`, and `replay.rs`' `i1_…`. Each runs a fight by seed and pins its end: a
  kill on the tape, the loser's drop to the ROM, or a shot count. A retune changes the fight, so
  they re-bless like the corpus, not like a readout.
- **The bot, for CC.10.** `a_steady_wire_learns_the_lattice` does not learn the cannon's cycle under
  the probe. The lane's `WEAPONSTATUS` read after a cannon shot lands 1,020 ms after the send, past
  the bot's own `CYCLE_READ_WITHIN_MS` (1,000). That is the lane's polling against a 2,000 ms
  coaxial, not the rig. The rig's magazine knob (`Some(200)`, else `30 of 40`) is the rig's own
  and stays.
- **Premises CC.2's palette will test.** Every palette cycle (15, 5, 10 and 30 s) is a multiple
  of the 5,000 ms cadence:
  - Each `WeaponReady` after a 5,000 ms arming then lands on a pulse end at that cadence. The
    mid-pulse cases in `conditions::m3_…` and its siblings lose their mid-pulse instant, though
    the cross-cadence equality still holds.
  - `arena_story`'s `MAX_SHOTS` is 30, above a 24-round load.

## CC.1

Not built.

## CC.2

Not built.

## CC.3

Not built.

## CC.4

Not built.

## CC.6

Not built.

## CC.9

Not built.

## CC.7

Not built.

## CC.8

Not built.

## CC.10

Not built.
