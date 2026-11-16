# 49. Combat content — cut notes

One section a unit of [doc 49](49-combat-content.md) §7. A unit's session replaces its own "Not built." line with
what it built, under these headings: **Survey**, **Built**, **Tests**, **Divergences**, **Not built**.
Each session edits only its own section, so two units' notes never touch the same lines.

## CC.0

Tests and the bot's rig read content (Q-49-16). Built 2026-11-15 on `a5018131b` (PR #622's tip),
with main `72d5b7150` merged in.
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
| The same, in files outside CC.0's row (3) | `mush-gateway`'s `arena_story_split` (its own `LOAD = 40`); `domain_world/voyage.rs` gc7 (a rearm quote of 10, from 40 less 30); `domain_economy/yard_in9.rs` (`rearm cannon (40 rounds`, `0/2000 rounds`) | CC.0, on the cutting session's word: built below |
| Content pins (20) | listed test by test in **Not built** | CC.2's one re-bless |
| Fights on the shipped tank, by seed (5) | listed in **Not built**: the run must end in a kill, a ROM drop, or a pinned shot count | CC.2, with the corpus pins |

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
- **Three files outside the row**, fixed on the cutting session's word:
  - `mush-gateway/tests/split_halves/arena_story_split.rs`: `LOAD` became `load()` there too, read
    from `tacsim::builtin_definitions()`. Like the in-process twin, it asserts the cannon's and the
    coaxial's loads differ.
  - `domain_world/voyage.rs` gc7: the known repair is still ten rounds short. Its cannon cell is set
    to the content's load less ten, not to a literal 30. So the quote's rearm of 10 and its
    111.00 cr and 92.50 cr, which price ten rounds, hold at any load of ten or more.
  - `domain_economy/yard_in9.rs`: `escort_load(component)` reads the shipped `tv_escort`'s loads,
    and the `+bay` quote is matched as `rearm cannon (<load> rounds` and `0/<load> rounds`. The
    hold is still seeded with 40 and 20 units of the two rounds' goods. That covers today's loads
    and CC.2's (24 and 600); the assertion after the rearm is only that rounds were loaded.

**Tests.** All runs were on conclav.dev, in a throwaway clone with the cut's patch applied. The
first round was at `a5018131b`; the second at main `72d5b7150` (PR #623 merged in), after the
three outside files were fixed.
- **Today's values.** All passed:
  - the four binaries first touched (`tacsim`'s lib tests, `tacsim::domain_arena`,
    `mushd::sim_lenses` and `mush-bot::arena_lane`): 3,673 tests;
  - the three re-pinned outside tests (`arena_story_split`, `voyage` gc7, `yard_in9`).
- **The probe, again, with the whole cut applied** (second round). It ran 7,571 tests and 26
  failed, down from 45:
  - the 25 pins and fights listed in **Not built**;
  - CC.10's `a_steady_wire_learns_the_lattice`.

  Every other test the first probe broke now passes under it, including the three outside files
  and `full_rate_on_each_guns_own_cycle`. One story failed once and passed on its retry:
  `observer_rule::ae2_a_missile_in_flight…`, "four missiles … and not one hit". That is a dice
  outcome, not a load or a cycle, and it passed in the first round.
- **Guards**, at today's values. All passed:
  - `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` for `tacsim`, `mushd`,
    `mush-bot` and `mush-gateway`, on the final patch;
  - the CI lint job's M7 memory-size pass on `tacsim`;
  - `mush-contract-tests`' `ci_policy` guards (`feature_matrix_inventory`, `panic_budget`,
    `restriction_lints`: 24 passed);
  - `cargo doc --no-deps --document-private-items` with `-D warnings` for `tacsim`, `mushd` and
    `mush-bot`.

  The last three ran in the first round. Since then the cut has changed only test files and this
  note, none of which those three read.

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
- **Content pins and seeded fights, for CC.2's one re-bless.** The probe moved all 25: 20 content
  pins and five seeded fights. Each is a hash, a digest or a fight recorded on the shipped
  content, so it re-blesses rather than reads. Paths are under `crates/tacsim/`.

  | File | Test | What it pins |
  |---|---|---|
  | `tests/domain_arena/content.rs` | `content::the_pinned_digests_are_the_shipped_ones` | `DIGESTS`, the 15 packs |
  | `tests/domain_arena/extensions.rs` | `extensions::ae1_the_three_chassis_load_beside_the_stock_game_without_moving_it` | the shipped tank's definition digest |
  | `tests/domain_arena/extensions.rs` | `extensions::ae2_the_ordnance_loads_beside_the_stock_game_without_moving_it` | the `ap_30` pack digest, and the authored autocannon and rack values as one tuple (a content pin by design) |
  | `tests/domain_arena/extensions.rs` | `extensions::ae3_the_drone_loads_beside_the_stock_game_without_moving_it` | the shipped tank's definition digest |
  | `tests/domain_arena/extensions.rs` | `extensions::x1_the_laser_content_loads_beside_the_stock_game_without_moving_it` | the `laser_charge` pack digest |
  | `tests/domain_arena/extensions.rs` | `extensions::x2_the_biped_content_loads_beside_the_stock_game_without_moving_it` | the shipped tank's definition digest |
  | `tests/domain_arena/extensions.rs` | `extensions::x3_the_thermal_content_loads_beside_the_stock_game_without_moving_it` | the shipped tank's definition digest |
  | `tests/domain_arena/extensions.rs` | `extensions::x4_the_ecm_content_loads_beside_the_stock_game_without_moving_it` | the shipped tank's definition digest |
  | `tests/domain_arena/extensions.rs` | `extensions::x5_the_eccm_content_loads_beside_the_stock_game_without_moving_it` | the shipped tank's definition digest |
  | `tests/domain_arena/extensions.rs` | `extensions::x6_the_authored_heightfield_reaches_the_arena_verbatim` | the shipped tank's definition digest |
  | `src/match_/seal.rs` | `match_::seal::m5_tests::m5rules_default_seal_identities_are_pinned` | the default seal custody identities |
  | `src/minds/registry/trials/physical_tests.rs` | `minds::registry::trials::physical_tests::c4_a_registry_trial_ends_the_same_on_every_host` | `C4_TRIAL_PIN`: each trial's end, time and hashes |
  | `tests/domain_arena/motor.rs` | `motor::m1a1_the_shipped_duel_hashes_as_before` | the shipped duel's hash (`DUEL_HASH_AT_HEAD`; §3.6 guessed it would not move) |
  | `tests/domain_arena/motor.rs` | `motor::m1b0_the_production_line_hashes_do_not_move` | the shipped duel's ledger lines |
  | `tests/domain_arena/motor.rs` | `motor::gc1b_an_ooc_match_offered_a_join_keeps_its_practice_hashes` | the shipped duel's practice hash |
  | `tests/domain_arena/minds.rs` | `minds::m1a5_the_production_hash_does_not_move` | the production encoding pins |
  | `tests/domain_arena/twin.rs` | `twin::m2_gate_a_on_a_mixed_pool` | the mixed pool's think counts |
  | `tests/domain_arena/twin.rs` | `twin::m2_gate_a_over_the_authored_lance_field` | the lance field's think counts |
  | `tests/domain_arena/twin.rs` | `twin::m2_gate_a_over_the_pooled_lanes_matches` | the pooled matches' think counts |
  | `tests/domain_arena/twin.rs` | `twin::m4_5_the_stock_pool_over_fifteen_seeds_is_its_record` | `M4_5_POOLED_RECORD` |
  | `tests/domain_arena/report.rs` | `report::m5_the_mind_flies_no_wreck_on_the_arenas_own_tape` | a seeded fight: the winner destroys the loser |
  | `tests/domain_arena/report.rs` | `report::m5_the_minds_shots_are_counted_in_its_block_and_the_pilots_lines_are_unchanged` | a seeded fight's shot counts |
  | `tests/domain_arena/report.rs` | `report::m5_the_pilots_report_carries_their_own_mind_block_and_no_other` | a seeded fight: the loser drops to the ROM on seed 2,352 |
  | `tests/domain_arena/report.rs` | `report::v3_a_real_fight_folds_to_each_pilots_own_projection` | a seeded fight's report lines |
  | `tests/domain_arena/replay.rs` | `replay::i1_a_real_fight_folds_to_one_objective_picture` | a seeded fight: the loser destroyed on the tape |

  Two more pins are `#[ignore]`d, so the probe did not run them. CC.2 runs them by name:
  - the arena harness baseline, `tests/harness_baseline/arena.linux.json`, through `tests/harness.rs`;
  - `M2_CROSS_PLATFORM_PINS`, in `tests/domain_arena/minds_rhai.rs` (the D-UM-28 gate).
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

Slots and the stock mind's discipline (Q-49-8, Q-49-9, Q-49-20, Q-49-23, Q-49-29). Built
2026-11-15 on main `c85a8744f` (CC.0 and CC.6 merged in), with main `73d05678b` merged on
2026-11-16 (CC.9's API 10 among it). Every pin below was re-read on the merged tree.

**Survey.**
- **Slots stand (Q-49-20).** Nothing in the tree made a slot single-use, so nothing needed
  unwinding: an armed slot fires at each ready moment while its terms hold.
- **A player's slot is bounded by the Compact floor alone (Q-49-29).** No judgement-term rule
  exists in code:
  - `rules.rs`' `fire_rules` takes the odds term from the typed terms, else the match's floor;
  - `check_floor` refuses only an odds term under the floor;
  - `condition_is_lawful` checks the same floor when the arming lands.

  `FIRE cannon WHEN lock` arms at the floor.
- **The card.** `line()` held `min_odds_permille: 100` (the Compact floor's default) and
  `reserve_pct: 0`. The other five presets are written over Line, so they inherit its reserve.
- **"Dry"** was one test, `stock_mind::is_dry`. Six places read it: the withdraw edge, the
  expiry's restoration, the rally, the arming gate and both stands. The Rhai port read it
  once, into `p.dry`.
- **The record.** `note_fired_on` wrote `fired_on_by` only in a host-ended field, or on a
  practice hull a pilot had named a player for (doc 45 AP.10).
- **`spent()`** counted a unit spent only when every row was empty.
- **The Rhai budget.** `standard.rhai` holds 31 of its 32 functions. `minds/stock.rs` compiles
  a trusted copy (31 private functions plus six wrappers, under 40) and checks `arming_for`'s
  signature line verbatim.

**Built.**
- **The Line card** (`arena/card.rs`, Q-49-8). `line()` holds `LINE_MIN_ODDS_PERMILLE`, 350,
  and `LINE_RESERVE_PCT`, 25. Both are provisional.
  - Sentry, Skirmisher, Hunter and Rearguard keep Line's quarter; Sniper keeps its own 30%.
  - Hunter's 30% floor is its own, under Line's.
  - Skirmisher's 35% now equals Line's, and it still names itself.
- **The half-magazine rule** (Q-49-9). `StockCard::floor_for(rounds, rounds_max)` is:
  - the card's floor while a mount holds half its load or more;
  - `LOW_MAGAZINE_RISE_PERMILLE` (150) above it once the mount holds under half, capped at
    `RISEN_FLOOR_MAX_PERMILLE` (950, the slider's top) and never below the card's own.

  Line reads 50% there, Sniper 80%. `StockCard::keeps(rounds_max)` is the reserve in rounds.
  The arming, the withdrawal and `spent` share it.
- **The card's text.** On an armed hull, `read_back` prints a sixth line under the fire
  discipline: `under half a magazine min_odds 50%; the reserve answers fire and covers a
  withdrawal` (103 columns). `docs/help/minds.md`'s Line paragraph says the same, and says that
  a hull at its reserve on every mount breaks off. `card.rs`' help test reads both.
- **Sentence 1: dry for withdrawal is its own test** (`arena/stock_mind.rs`).
  - The new public `at_reserve(view, card)` is true when some row holds a load and every row
    holds at or under the card's reserve of it. On a card with no reserve it is `is_dry`.
  - The withdraw edge, the expiry's restoration and the rally read it.
  - The arming gate (`engage_orders`) and both stands still read `is_dry`, which is
    unchanged, as is the Rhai kernel of that name.
  - The edge takes it as the doctrine's one retreat, as it takes a hurt hull: see
    **Divergences**.
- **Sentence 2: answering fire is decided in the slot check** (`arena/conditions.rs`,
  `arena/absent.rs`).
  - In `Arena::warrant`'s third term, a slot at or under its `min_rounds_left` still fires
    when a round is left and the lock's target has fired on the hull.
  - Fire is counted as the absent gate counts it: the new `Arena::answers_fire` is
    `fired_on_by`, plus a guard's widening to its player's other units (Q-45-8).
  - Fire from any other unit opens nothing, and an empty magazine answers nobody.
  - A seat's own rounds term is a rule predicate, not `min_rounds_left`, so the exception
    reaches only a mind's arming (`Arming::Arm`: the stock shorthand and Rhai's `arm`).
- **The record in every match** (`arena/absent.rs`). `note_fired_on` writes `fired_on_by` at
  every impact in every match. This moves the arena hash of every match with an impact.
- **Sentence 3: a withdrawing mind arms with no reserve.** `arming_for` takes a fifth argument,
  `withdrawing` (the think ends in `Withdraw`), and arms with `min_rounds_left` 0 then. Its
  odds term is each mount's `floor_for`.
- **Sentence 4: `spent()` counts a unit at its reserve** (`arena/damage.rs`). The new public
  `Arena::reserve_kept(unit, rounds_max)` is the active stock card's `keeps` while a mind flies
  the unit, and 0 for a seat or a custom card. `spent()`'s third condition reads rounds above
  that.
- **The Rhai port** (`content/minds/standard.rhai`): still 31 functions, `stock: 3`, and `api: 10` from CC.9.
  - `standing_order` reads `p.low` beside `p.dry`, a loop over the rows. The test that a row
    is over its reserve is `rounds * 100 > reserve * rounds_max`, which is T1's rounded-down
    `rounds > keeps`. The card's schema holds `reserve_pct` to 100, so no product overflows.
  - `p.low` is read only where it has a reader: the edge of an unrallied hull, and a
    withdrawing hull's restoration and rally.
  - `edges`, the restoration and `withdraw_arm`'s rally read `p.low` where they read `p.dry`.
    The edge reads both, as T1's does.
  - `arming_for(view, card, ceiling, withdrawing)` arms with no reserve while withdrawing, and
    at the risen floor on a mount under half its load. `emit` passes `this.state == 4`, and the
    two power arms pass `false`.
- **The digest.** `STANDARD_SHA256` (`minds/content.rs`) and `tests/domain_arena/content.rs`'
  `STANDARD_DOCTRINE_SHA256` are re-blessed from CC.9's `c544bfbd…f9375f` to `eec3f668…154a91`.
  `MIND_ABI` stays CC.9's `rhai=1.26.0;api=10;stock=3`. The mind view gains no field, so there is no
  API bump.
- **Outside the row, on the lead's word:**
  - `minds/stock.rs`: the trusted copy's shape check names the new signature, and
    `stock::fire_discipline` passes `false`. The public helper surface is unchanged.
  - `minds/stations.rs`: one test builds its expected arming on a card at the script's literal
    terms (10%, no reserve).
  - `arena/weapons.rs`: the MW.7 and AP.10 tests assert the record is kept in a practice
    match. AP.10's opening fire moves to a clone, so the practice after it opens unfired on.
  - Doc comments in `arena/hash.rs` (sub-item 25) and `match_/practice.rs`.
  - `docs/help/minds.md`, the Line paragraph.

**Tests.** Every run was on conclav.dev, in a throwaway clone of `c85a8744f` with the unit's patch
applied.

- **New tests, each caught by a mutation of its ruling.** A mutation pass put each ruling's
  production line back the old way, one at a time, and ran the new tests against it. All ten
  passed unmutated, and every mutation failed at least one:

  | Mutation (the line put back) | Failed |
  |---|---|
  | Line at 10% and no reserve | the two `card.rs` tests, `damage.rs`' spent test, the `stock_mind.rs` and `content.rs` withdrawal pair, both `reflex.rs` stories |
  | No rise under half a magazine | `cc1_the_floor_rises_fifteen_points_under_half_a_magazine`, the withdrawal pair |
  | The withdraw edge on `is_dry` | the withdrawal pair, the answering story |
  | No answering under the reserve | the `conditions.rs` slot test, the answering story, AP.6's guard story |
  | The record kept only in host-ended fields | the slot test, the answering story |
  | No guard widening in the answer | AP.6's guard story |
  | A withdrawing hull keeps its reserve | the withdrawal pair |
  | `spent()` reads empty rows only | `damage.rs`' spent test, the two-minds story |
  | The reserve's break-off not the one retreat | the withdrawal pair, AP.6's guard story |

  The tests:
  - `arena::card`:
    - `cc1_line_holds_a_thirty_five_per_cent_floor_and_a_quarter_in_reserve`: Line, the presets,
      `keeps`;
    - `cc1_the_floor_rises_fifteen_points_under_half_a_magazine`.
  - `arena::stock_mind::cc1_a_hull_at_its_reserve_withdraws_by_its_own_test_and_covers_the_withdrawal`.
    It covers `at_reserve` against `is_dry` and the break-off with one mount over its reserve
    and with none. A rallied hull stays at its reserve and still breaks off empty. Cornered, it
    arms with no reserve at the low-magazine floor.
  - `minds::content::cc1_mirror_a_hull_at_its_reserve_withdraws_and_covers_the_withdrawal`: the
    same through both runners, then `arming_for` alone on every preset, low and full, withdrawing
    and not. Native and Rhai agree.
  - `arena::conditions::cc1_a_slot_under_its_reserve_answers_only_a_unit_that_fired_on_its_hull`.
  - `arena::damage::cc1_two_minds_on_their_reserve_on_every_mount_are_spent`.
  - The done-when stories, in `tests/domain_arena/reflex.rs`:
    - `cc1_two_minds_on_their_reserve_end_the_match_spent_with_rounds_left`: two minds on Line, two
      rounds over their reserves, end the simulator match `Spent` with rounds in both magazines.
    - `cc1_a_mind_at_its_reserve_answers_the_unit_that_fired_on_it_and_withdraws_at_it`: at its
      reserve the mind fires nothing for twenty seconds, then answers the seat that fired on it.
      Once at its reserve on every mount with rounds left, it breaks off by its own edge.
  - `field_ap6::ap6_a_guard_that_loses_its_last_fire_channel_goes_home`, unchanged. It caught
    the literal reading of Q-49-23 (see **Divergences**).
- **Already held, pinned.** `arena::conditions::cc1_a_players_slot_needs_no_judgement_term_and_stands`
  (Q-49-29, Q-49-20). A typed `FIRE <mount> WHEN` with the lock, a range or a rounds term alone
  arms at the Compact floor and is lawful. Only an odds term under the floor is refused, and a
  slot stays armed after its shot. This held on main, so no mutation fails it.
- **The probe.** First, the nine crates' tests with no fail-fast, as CC.0's probe ran them: 7,601
  tests, 42 failures, all in `tacsim` but one contract test. Every non-`tacsim` crate passed:
  `mushd`, `mush-bot` (its lanes included), `mush-gateway`, `mush-server`, `mush-net-ssh`,
  `mush-campaign`, `mush-ansi`. The 42 sort into:
  - the token `rhai` in a `card.rs` doc comment, which the arena library's contract bans;
  - CC.1's own tests on the first draft;
  - tests that put Line under a 20% floor, re-sized to 40%: two in `seal.rs`, one in
    `ic_sortie/execution/tests.rs` and three in `match_lifecycle.rs`;
  - texts quoting Line's 10% or its five read-back lines: `arena_bridge.rs`,
    `domain_arena/cockpit.rs`, `match_lifecycle.rs`' refusal sentence, `card.rs`' preset list and
    help test, `minds/stations.rs`;
  - the practice record (`arena/weapons.rs`' MW.7 and AP.10 tests);
  - fixtures whose mind's one round now sits under its reserve: `reflex.rs`' in-flight and
    three-cadence spent draws, sized to 11 of 40, the in-flight ones with the retreat spent;
  - AP.6's guard story and the 512-site field story (see **Divergences**);
  - the pins below.
- **Re-blessed by record** (every value read on the final tree):

  | File and test | Pin | What moved |
  |---|---|---|
  | `minds/content.rs`, `domain_arena/content.rs` `m2_the_standard_doctrine_source_is_pinned`, `match_/seal.rs`, `support/api4_identity.rs` | the Standard source digest | `c544bfbd…f9375f` (CC.9's) → `eec3f668…154a91` |
  | `arena/hash.rs` `m5_the_active_profile_folds_only_off_its_default` | the two-tanks hash | `0x63ea…ab0e` → `0xabec…b56d` (the card) |
  | `match_/seal.rs` `m5rules_default_seal_identities_are_pinned` | nine seal digests, and their nine API 4 projections | the card, and the source; the projections with the card alone |
  | `minds/registry/trials/physical_tests.rs` `c4_a_registry_trial_ends_the_same_on_every_host` | `C4_TRIAL_PINS` | each case ends in the same elimination and is filed `Loss` at 106,500 ms, not 58,500; run on Windows by hand at the same tree, equal |
  | `domain_arena/motor.rs` `m1a1_…`, `gc1b_…`, `m1b0_…` | `DUEL_HASH_API6_OBSERVED`, `DUEL_HASH_AT_HEAD`, `M1B0_DUEL_LINES` | hashes only: the duel fires no shot in its minute, and its 34 lines equal the old pins once the hash is out |
  | `domain_arena/reflex.rs` `m1a1_…`, `m1b0_…` | the (j) field's first and last hashes, `M1B0_J_LINES` | hashes only: 50 lines equal once the hash is out |
  | `domain_arena/weapons.rs` `m1b0_…` | `M1B0_SEAT_LINES` | hashes only: 30 lines equal once the hash is out |
  | `domain_arena/minds.rs` `m1a5_the_production_hash_does_not_move` | the three production encoding pins | with the duel's and the field's |
  | `domain_arena/minds.rs` `m1b5_…` ×3, `m2_the_two_t1_fixture_…` ×2 | `M1B5_TWO_T1_INTENTS`, `M1B5_TWO_T1_EVENTS`; Line's discipline in `two_t1_slots_and_citations` | one answer of sixty (the arming at 3,482 ms: 350 and 10 kept, where 100 and 0); the events in the seals' `source` and `card_digest` alone; the slot check reads `floor_for` and `keeps` |
  | `domain_arena/twin.rs` `m4_5_the_stock_pool_over_fifteen_seeds_is_its_record` | `M4_5_POOLED_RECORD` | matches run longer (seed 2350's 180,250 → 231,250 ms), the split 14–1 → 13–2, shots 1,488/863 → 1,058/936, `floor_held` 0 → 1,039, `unwarranted` 0 → 12 |
  | `domain_arena/twin.rs` Gate A's three lanes | `COVERAGE`, `POOLED_MATCHES_WITH_STAND_IN` | pooled 10,015/291 → 10,851/405, mixed 12,884/449 → 12,866/345, lance 7,260/126 → 7,142/159, 14 → 12; 0 divergences |
  | `domain_arena/minds_rhai.rs` `m2_the_twin_corpus_hashes_the_same_on_every_host` | `M2_CROSS_PLATFORM_PINS` | stopping times 180,250 / 177,500 / 170,000 → 231,250 / 197,500 / 167,000 (900,000 unmoved); the float probe's thinks and bitmask unmoved; read on Linux |
  | `tests/harness_baseline/arena.linux.json` | twelve deterministic leaves, by record (`cc1_deterministic_update`) | the minds lanes' four final hashes; `fill_authored`'s thinks, landings, condition fires and drops; the arena lane's ledger bytes. Timing gate live and inside it (p50 −1.6%, p99 +1.0%, window p99 +3.7%) |

- **The Rhai think's cost, and the lanes' bounds, set by record** (`tests/domain_arena/twin.rs`).
  CC.1 took the twin lanes' costliest Rhai think past the 1,200 operations-plus-fees STOP that
  17b cut M2.5 set on the mixed and pooled lanes. Max, with p99 in brackets, measured on
  conclav.dev:

  | Lane | main | CC.1 as first built | CC.1 after its cost pass | Bound |
  |---|---|---|---|---|
  | mixed | 1,168 (1,006) | 1,319 (1,158) | 1,275 (1,114) | 1,200 → **1,350** |
  | pooled | 1,195 (1,057) | 1,346 (1,202) | 1,302 (1,158) | 1,200 → **1,350** |
  | lance | 1,308 (1,246) | 1,468 (1,415) | 1,424 (1,371) | none → **1,450** |

  - **The cost pass** (behaviour unchanged, Gate A at 0 divergences on every lane, still 31
    functions):
    - the reserve read once a think, only where a reader can read it;
    - `arming_for`'s reserve read without its overflow clamp. The card's schema holds it to 100,
      so the clamp never changed a value. This saved about 60 a think.
    - A fully lazy read, at each reader, measured worse (lance 1,486), so it was not kept.
  - **Where the peak is.** A print of every think past 1,250 showed the costliest are Engage
    thinks under an Engage order, on 7- to 12-row views, taking a fresh lock and citing a shot.
    Fees are only 72 to 90 of each; the rest is script. The cornered withdraw stand is not among
    them. Its two `best_hostile` scans and two `line_to` traces are the line fallback's two
    distinct queries, and `entry_cell` runs once a think.
  - **Why lance stayed over 1,350.** CC.1's own code costs about 50 on that think, so lance would
    sit near 1,375 even at zero. The rest is the ruled behaviour: longer fights, more rows.
  - **The ruling.** The lead set the target at 1,350 (90% of class I's 1,500); lance could not
    reach it inside CC.1. The owner ruled on 2026-11-16, "Guard at 1,450": mixed and pooled
    bounded at 1,350 (`TWO_TANK_THINK_BOUND`), lance at 1,450 (`LANCE_THINK_BOUND`, its first
    bound).
  - **The follow-up unit CC.1b** cuts the Engage path's cost toward 1,350 on every lane.
- **Guards**, on the final tree:
  - `cargo fmt -p tacsim --check`;
  - `cargo clippy -p tacsim --all-targets -- -D warnings`;
  - the M7 memory-size pass;
  - `cargo doc -p tacsim --no-deps --document-private-items` with `-D warnings`.

  The contract tests (`engine_contracts`, `ci_policy`, the help-page lint) passed in the probe and
  in the whole-workspace sweep.
- **The whole-workspace sweep** (`gate.yml`, `kind=sweep` on conclav-dev, run 43825750857, before
  the bounds): 12,740 tests.
  - Three failed: the mixed and pooled twin lanes on the old 1,200 bound, and
    `mushd::sim_lenses` `observer_rule::m5_the_mind_object_names_nothing_but_the_pilots_own_seal`.
    That one pinned Line's 10% and now reads `LINE_MIN_ODDS_PERMILLE`.
  - The probe had failed it too, but its retry group's `TRY 2 FAIL` slipped past my filter.
  - `observer_rule::nothing_about_an_undetected_unit_crosses_on_any_surface` failed once and
    passed on retry, in the load-sensitive group.

**Divergences.**
- **The reserve's break-off is the doctrine's one retreat** (my reading of Q-49-23 with doc
  45's Q-45-9).
  - Q-49-23 says a unit at its reserve on every mount counts as dry for withdrawal. Taken
    literally, a unit on `GUARD` holding two rounds of 40 broke for home with rounds left.
    AP.6's story `ap6_a_guard_that_loses_its_last_fire_channel_goes_home` failed on it.
  - Doc 45 has a guard stay while it can still defend itself. At its reserve it can: it
    answers fire.
  - So the edge treats the reserve as it treats a hurt hull: it breaks off once, and not after
    it has rallied, nor where a posture spent the retreat (`seed_stay`, which every standing
    posture runs).
  - An empty hull breaks off whatever it has spent, as before.
  - The cost: a rallied stock mind at its reserve stays and holds. In a field, which never
    ends on `spent`, two such minds can sit facing each other until the host ends the field.
    The simulator still ends that match (sentence 4).
- **Answering fire counts a guard's widening** (sentence 2). Q-49-23 says "a unit that fired on
  this one, as the absent gate counts it". The gate's count includes the guard's answer for its
  player's other units, so the slot check reads the gate's own set (`answers_fire`), not
  `fired_on_by` alone. A guard on its last two rounds answers the intruder that fired on its
  neighbour.
- **In practice, the absent gate now counts fire from the match's open** (the lead asked for
  this to be named for the owner). AP.10 had the record start on a practising hull when its
  pilot named a player (`MIND AWAY`). The record is now kept in every match, so a practising
  hull answers a unit that fired on it before the pilot let go. It still never fires first.
  AP.10's tests were changed by record:
  - `ap10_a_practising_hull_in_a_match_is_held_to_the_one_rule` takes its opening fire on a
    clone;
  - the MW.7 test's last assertion now reads the record as kept.

  AP.10's stories (`tests/domain_arena/cockpit.rs`) pass unchanged.
- **A withdrawing mind covers its withdrawal against any cornering hostile.** "Covering a
  withdrawal" is the doctrine's cornered stand, which fires on the best hostile in reach, not
  only on a unit that fired. This showed in `reflex.rs`' in-flight spent fixture. That
  fixture's mind now has its one retreat spent, so its one round over the reserve is its last.
- **`spent()` reads the active stock card** (`active_card`), which is the ROM card once a mind
  is dropped. A custom card keeps no reserve there; its own terms are its own.
- **The 512-site field story's bystander moved** (`tests/support/field/grounds_rig.rs`,
  `URVARA_REACH`): from `(282, 294)`, about 250 m from the raider's ramp, to `(290, 284)`, about
  120 m.
  - The Line card's 35% floor never let a raider fire on it from there, even over 240 s.
  - A probe with Line at 10% and no reserve passed; at 35% with no reserve it failed. So the
    floor, not the reserve, was the cause.
  - The cell is still in the far quadrant, as S.2 asserts, and no other test reads it.

**Not built.**
- **CC.2's palette** moves every cycle and load again and re-blesses the same pins after this
  unit, as CC.0's list says.
- **Measured fights at the new discipline** (hits per load, match length, dry withdrawals, human
  against mind) are CC.2's to report. This unit's pins show the direction: matches run longer
  (the pooled record's seed 2350 from 180,250 to 231,250 ms).
- **The bot** (CC.10) still fires every gun at full rate. Its lanes passed unchanged in the
  probe.

## CC.2

Not built.

## CC.3

Not built.

## CC.4

Rounds on `STATUS` (Q-49-13, the ground half). Built 2026-11-15 on main `c85a8744f`, with CC.0
(#626) merged. It changes no content, no kernel and no pin.

**Survey.**
- `STATUS` (`weapons_fire.rs`, `status`) printed no rounds. The frame already carried each
  mount's rounds and full load (`frame.weapons`), which the mount status rows, the TUI cockpit and
  the pixel cockpit read.
- The widest shipped fill is the heavy (`heavy_mk1`). It carries three mounts: the cannon, the
  coaxial gun (`machine_gun`) and the missile rack (`missiles`). Every other armed unit carries
  two. Its row at full load is 58 columns today, and 56 under CC.2's magazines.
- The bot closes an open `STATUS` block at the first line it does not know as one of the block's
  rows (`picture.rs`, `is_status_form`). It reads every section row after that as nobody's. A row
  the bot could not classify would have cost it its own sections, so the row needed a `Line`
  variant of its own, not only a needle.
- Two help texts list what `STATUS` says: `help cockpit`'s row, and the in-game `help STATUS` line
  (`cockpit.rs`, `cockpit_help`).

**Built.**
- **The row**, `WeaponRow::rounds_line(rows)` in `weapons_fire.rs`:
  `  rounds cannon 39/40, machine_gun 1990/2000, missiles 6/6`.
  - Two spaces in, then each mount by its id with `left/full`, in component order, joined by
    commas. A platform with no mount prints no row, as one with no heat prints no heat row.
  - It hangs on `WeaponRow`, which the cockpit already exports, so `cockpit.rs`' export list is
    unchanged.
- **`STATUS`** prints it after the position and the nearest spot, and before the heat. It reads
  the held frame's mount rows (`frame_weapons`): the pilot's own tank, which the arena writes.
- **Help.**
  - `docs/help/cockpit.md`'s `STATUS` row says what the line holds, and gives its shape with
    placeholders. No shipped load is quoted (CC.0's rule).
  - `cockpit_help("status")` says it too. Its pinned clause, `, your pod's switch if you carry
    one`, is kept.
- **The bot** (`crates/mush-bot/`):
  - `lines.rs`: `Line::StatusRounds { mounts }`. `status_row` reads it before the section reading,
    so a section called `rounds` still reads as a section. The answer table lists it.
  - `picture.rs`: it is a row of the block (`is_status_form`), so the block stays open through it.
    It is read into `Own::rounds`, which nothing reads yet. It moves nothing in
    `Picture::weapons`: those rounds come off the mount status rows, which the stale-ready guard
    reads against the acks.
  - The scripted rig's `STATUS` (`tests/arena_lane.rs`) prints the row, from the same figures its
    mount rows print (`rounds_of`). Every lane test that reads the rig's sections now reads them
    through the row.

**Tests.**
- **`match_::cockpit::tests::cc4_the_rounds_row_at_its_widest_fill_is_under_120_columns`.** It
  takes every fielded unit type that instantiates, each mount at its load off the content. For
  each, the row names every mount as `load/load`, in order, under 120 columns. It also checks:
  - the widest fill is the heavy's;
  - a spent mount reads its own left;
  - a platform with no mount gets no row.
- **`domain_arena`'s
  `cockpit::cc4_status_shows_each_mounts_rounds_within_120_columns_at_its_widest_fill`.** A heavy
  is fielded with `SIM UNIT heavy_mk1` in a running match. It checks:
  - the frame carries the content's loads;
  - `STATUS` has exactly one rounds row, equal to the row built from the content, before the
    section rows;
  - every line of the reply is under 120 columns.

  It builds its expected row without the cockpit's formatter, so it compiles on the base tree and
  fails there.
- **mush-bot.**
  - `lines`: the row, a one-mount row, a section called `rounds`, and two rows that are not it.
  - `picture::the_rounds_row_is_a_row_of_the_status_block`: the sections after the row still
    apply, and a row outside a block changes nothing.
  - `arena_contract`'s parser test reads the formatter's row off the hand-built frame, through the
    120-column renderer, into `Own::rounds`.
  - `arena_contract::the_rounds_row_at_its_widest_fill_is_one_line_the_bot_reads` renders the
    heavy's content fill as one line and reads every mount back.
- **Runs**, on conclav.dev in a throwaway clone:
  - **On the base tree**, with only the `domain_arena` story applied, the story fails. Its check
    that the frame carries the content's loads passes, then it finds no rounds row in `STATUS`.
  - **With the cut:**
    - `tacsim`'s lib tests and `domain_arena`: 3,516 passed;
    - the bot job (`mush-bot` without the mining cohort and `arena_mind_isolation`): 786 passed,
      `m5_4_the_mind_lines_read_as_the_bot_reads_them` among them;
    - `mushd`'s sim lenses: 95 passed;
    - `mush-contract-tests`: 200 passed, the help lint and the `ci_policy` guards (census, panic
      budget, restriction lints) among them.
  - **Guards**, all passed:
    - `cargo fmt --check` for `tacsim` and `mush-bot`;
    - `cargo clippy --all-targets -- -D warnings` for both;
    - the CI lint job's M7 memory-size pass on `tacsim`;
    - `cargo doc --no-deps --document-private-items` with `-D warnings` for both.

  After the run, only two things changed: the help row's wording (it adds no command-shaped
  word) and this note.

**Divergences.**
- **A row of its own**, not a field added to another row. The head row carries the object's name,
  which can be any length, so a field there could not be bounded by content. A row of its own is
  the shape heat, the receiver and the pod switch already take. Its width depends only on mount
  ids and loads, which the test reads from content.
- **Mount ids, not short names.** `mg` is shorter, but a short name is a fact about the whole
  platform (`weapon_short_of`). The bot can read `STATUS` before any mount status reply has told
  it the short names. The id is what the mount status rows open with.
- **Not wrapped.** The facet runs wrap; this row does not, because a four-space continuation reads
  as a facet run to a line reader. The unit test fails on the day the content fields a unit whose
  row would reach 120 columns.
- **Outside the row's file**, on the brief's word for the help and the tests:
  - the in-game help line in `cockpit.rs`;
  - the tests, in `match_/cockpit/tests.rs` and `tests/domain_arena/cockpit.rs` (appended).

**Not built.**
- Space decision support: CC.9's.
- The bot does not act on `Own::rounds`. CC.10's discipline (Q-49-17) can read it.
- The TUI and web status panes are unchanged: they already show each mount's rounds off the frame.

## CC.6

Space content as data (Q-49-11, Q-49-24, Q-49-25). Built 2026-11-15 on
`claude/cc6-space-content-data-97a7d4`, cut from `a5018131b`. This unit is the format, not the
retune: every shipped number is the one the Rust table carried, and a test holds each fit to
it value by value. **(expensive to reverse)**: the schema is recorded below.

**Survey.**

- **Mounts.** Four Rust structs, each with one standard constructor (`space/weapons.rs`): twin
  torpedo tubes, a laser turret, a spinal gun, a point-defence rotary. Fits were a Rust `match`
  over six keys (`space/custody.rs`), with the Dray's fit as the last arm, which every other
  hull flew. `raider_heavy` and `corvette_interdictor` were overlay keys over the corvette's
  unit; only the second is still that.
- **The mind view** (`space/view.rs`) showed the first laser, the first tube family and the
  first spinal gun, with `.max(8)` and `.max(24)` as the maxima. The bubble fired the first tube
  family and the first spinal gun only (`space/bubble.rs`). The applier sent one command per
  family a think (`space/applier.rs`).
- **"Armed"** (`space/backup.rs`) counted any laser, whatever the modes.
- **Why the ground's `weapon` kind does not fit.** It caps a range band at 100 km
  (`definitions/weapon.rs`), and every mount fires a round family whose load is at least one,
  with no notion of modes, salvos, an envelope or a damage falloff.
- **What pins what.** The tape's fire commands are matched in doc 47's tests:
  - `tick_thinks_tests.rs` reads exactly one `FireOffensiveLasers` for the house default's
    answer;
  - `field_ap11.rs` and `mw21_completion.rs` scan for `FireTorpedoSalvo` and `FireRailgun`.

  The shipped doctrines name `laser_1` and `torp_bow` by id, and they read `rounds == rounds_max`
  on the tubes (the interdictor).

**The schema (recorded, expensive to reverse).** Two pack entry kinds,
`crates/tacsim/src/definitions/space_mount.rs`, in `rules_definition` packs. They are decoded
strictly (`deny_unknown_fields`, integers only), bounded at load, refused as duplicates across
packs, and **not minted**: no unit binding names them, so the definition registry and
`CONTENT_KINDS` are unmoved. They join `OWNED_KINDS`.

- **`space_mount`** (`SpaceMountDef`): `label`, `note`, and `family`, exactly one of four:
  - `tubes`: `salvo` (1 to 8), `reload_ms`, `accel_mps2`, `dv_mps`, `warhead_damage`;
  - `laser`: `base_damage`, `falloff_half_m` (damage at range R is base × R0² / (R0² + R²),
    in whole km), `max_range_m`, `defence_range_m`, `heat_per_shot_permille`, and `modes`,
    with `offence` and/or `defence`, each `{ cycle_ms }` (Q-49-24: the cycle belongs to the
    mode);
  - `spinal`: `muzzle_mps`, `max_range_m`, `round_damage`, `heat_per_shot_permille`,
    `cycle_ms`;
  - `point_defence`: `envelope_m`, `burst_rounds`, `cycle_ms`, `damage_per_burst`.
- **Bounds.** A reach is 1 m to 2,000 km (`MAX_SPACE_RANGE_M`). A cycle is 250 ms (one bubble
  substep, `SPACE_CYCLE_FLOOR_MS`) to an hour; the 5 s floor of Q-49-19 is the palette's, for
  CC.8. Damage is 1 to 100,000, heat 0 to 1,000 per-mille.
- **`space_fit`** (`SpaceFitDef`): `label`, `note`, `unit` (a `unit` entry some loaded pack
  declares), `pixel_model`, `decoys`, `rcs` (the eight docking-game figures), and `mounts`. Each
  mount carries:
  - `id`: its id on the hull, what `fire(id, slot)` and the commands name, unique in the fit;
  - `mount`: the `space_mount` row;
  - `magazine`: required for tubes, spinal and point defence, refused for a laser;
  - `modes`: for a laser only, the modes the fit allows, a subset of the row's (Q-49-24);
  - `mode`: for a laser only, the start mode, one of `modes`.

  `Catalog::build` checks every reference: the unit, the row, the magazine against its family's
  cap, the modes against the row.
- **Which fit a hull flies.** `space_fit_for_id` reads an overlay key `tacsim.unit.<name>` as
  `tacsim.space_fit.<name>`, after a fitted core's chassis is read as its chassis. A hull with no
  fit of her own flies `tacsim.space_fit.dray` (`DEFAULT_SPACE_FIT`), as the table's last arm
  did.
- **What the kernel carries from a fit.** The runtime structs gained fields, each defaulted and
  left out of a document when empty, so a ship saved before CC.6 reads as it did:
  - `TorpedoTube::fired_since_full` and `RailgunMount::fired_since_full`: rounds fired since
    the magazine was full, counted by the kernel. What is left plus this is what the fit
    stows (`magazine_max()`). It is absent until she fires, so a hull that has not fired saves
    the bytes she always did: a stored maximum pushed the ten-minute heave-to's saved space
    state from under 64 KiB to 66,042 B (`voyage.rs`'s `sfr10` story caps it at 64 KiB);
  - `LaserMount::allowed_modes`: both, when absent;
  - `LaserMount::defence_cooldown_ms`: `None` when the two modes share one cycle, as every
    shipped row does.
- **One new tape command**, `FireMount { shooter_id, target_id, mount_id, count }`, fires a tube
  family or spinal gun by id. The first of each family keeps the family's own command, so every
  shipped hull's tape is the tape it always was.

**Built.**

- **Content** (`crates/tacsim/content/`):
  - `space/mounts.json` (`tacsim.pack.space_mounts`): six rows. They are the twin tubes, the
    dual-mode turret laser, the corvette's short-reach heavy laser, the spinal gun, the raider's
    low-velocity spinal gun and the point-defence rotary. The two variants are rows of their
    own, so no fit overrides a number.
  - `space/fits.json` (`tacsim.pack.space_fits`): six fits, the Dray, the Dray-P, the corvette,
    the interdictor, the belt raider and Halcyon. It requires the mount pack only; a unit is
    checked by reference, so a unit pack's version bump does not re-sign the fits.
  - `units/raider_heavy.json` (`tacsim.pack.raider_heavy`): the belt raider's own unit file, the
    corvette's sections, facets, drive, stowage and interior unchanged.
  - `space/picket_cast.json`: the raider's `unit` names it.
  - All three packs are fielded last (`EXTENSIONS_RAIDER_HEAVY`, `EXTENSIONS_SPACE`).
- **`space/custody.rs`:** the overlay is built from the fit (`overlay_from_fit`). The Rust table
  is gone. `space_fit_for_id` and `space_mount_row` read the shipped catalogue, and the
  catalogue logs once if it does not build. The raider is built through the corvette's path,
  so her checked-out state is the overlaid corvette's.
- **`space/weapons.rs`:** a `from_def` constructor for each family. The `standard_*`
  constructors are test-only and read the shipped rows. New: `LaserModes`, `magazine_max()`,
  `defence_cycle_ms()`, `offence_allowed()`.
- **`space/bubble.rs`:**
  - `fire_torpedo_salvo_from` and `fire_railgun_from` take a mount id, and `fire_mount` and
    `FireMount` dispatch to them;
  - `set_laser_mode` refuses a mode the fit does not allow;
  - an offence pulse skips a laser whose fit allows no offence;
  - a defence pulse sets the defence mode's cycle;
  - `new_dray`'s inline mounts are gone: her fit lays them.
- **`space/view.rs`:**
  - one row for each laser her fit lets fire at a lock, each tube family and each spinal gun,
    in the fit's order within each family;
  - a magazine's maximum is `magazine_max()`, so Halcyon reads 16 and 32 and they do not
    shrink;
  - point defence has no row.
- **`space/backup.rs`:** `armed` and `can_defend` count a laser only when her fit allows
  offence (`has_offence_beam`).
- **`space/applier.rs`:** a `fire` carries its mount id. Each tube family and spinal gun gets
  one command a think (a family named twice fires a salvo of two, at most `SALVO_MAX`), the
  first of each family by the family's command.
- **`pack.rs`:** the two `PackEntry` kinds, their bounds, and a refusal in a campaign-seed
  pack. **`commands.rs`:** `@tacsim/pack show` describes them.
- **Q-49-25:** every shipped laser is fitted dual-mode, the Dray's and the Dray-P's included,
  starting in offence; the corvette's and the interdictor's second laser starts in defence.

**Tests.**

- `space::custody::content_tests`:
  - every fit (and the Dray fallback for a hull with none) equals the Rust table, value by
    value;
  - every shipped laser is dual-mode, and the Dray's start in offence;
  - a fitted core's chassis flies its chassis's fit;
  - the raider's unit file builds the state the overlaid corvette did.
- `space::view::cc6_tests`: one row a mount, with point defence left out; Halcyon's maxima are
  her fit's and do not shrink after a salvo and a round; a laser fitted defence-only has no row.
- `space::applier::tests::cc6_a_second_tube_family_and_spinal_gun_each_fire_by_their_id`: four
  commands for five `fire`s, the first of each family by the family's command, the magazines
  counted down per mount, and the tape replays to the same mounts.
- `space::bubble::cc6_tests`: a mode the fit does not allow is refused; each mode's cycle comes
  from its row.
- `space::backup::tests::cc6_a_laser_fitted_defence_only_does_not_arm_her`.
- `definitions::space_mount::tests`: the bounds, and every reference a fit can get wrong.
- Content pins (`tests/domain_arena/content.rs`): the three new signatures in `DIGESTS`, the
  fielded count (+3), and the raider in the core list. `definitions.rs` and `extensions.rs`
  count her among the fielded units and the capital class. `mush-contract-tests`' unit looks
  list her as lookless, as the corvette is.
- Doc 47's space stories, run unchanged: `space_ap9.rs`, `field_ap11.rs`, the
  `tick_thinks_tests.rs` return-fire tests and `voyage.rs`. Their one edit is `voyage.rs`'s
  cast assertion, which now reads the raider's unit file.

**Divergences.**

- **A laser fires by its id as part of her battery.** Naming any offence laser fires every
  ready laser in offence, one command a think, as before and as `+tac/beam` does. A doc 47 test
  reads that one command off the tape. Firing a laser alone belongs with the cycle per mode and
  the switch rules, which are CC.8's.
- **The defence cycle is read per mode**, but the switch rules (a switch never fires early) are
  CC.8's, as its row says.
- **A laser row reads 100 of 100** (`LASER_ROW_ROUNDS`): a laser keeps no magazine. Its row's
  odds, bands, heat per shot and a ready that ignores the current mode are as before: those are
  CC.9's decision support.
- **The kernel reads the compiled-in content**, as `unit_spec_for_id` already did. An operator
  pack's `space_fit` is checked by the world's catalogue but does not fly.
- **Files outside the unit's row**, each the least this needed:
  - `pack.rs` and `commands.rs`: the entry kinds;
  - `space/applier.rs`: the ids;
  - `content/space/picket_cast.json`: the raider's unit;
  - count and pin lines in `tests/domain_arena/{content,definitions,extensions}.rs` and
    `voyage.rs`, and the lookless row in `mush-contract-tests`;
  - `FireMount`'s arm in `tests/domain_field/mw21_completion.rs`, whose match names every
    tape command.

  The lead approved `pack.rs`, `commands.rs` (additive, the new arms only), `applier.rs`, and
  the raider's unit file with its pointer, pins, cast assertion and lookless row.
  `mw21_completion.rs`'s arm came with the new tape command.
- **No part goods** were added, so doc 48's PL.0 was not needed.

**Not built.**

- **The retune (CC.8):** the 5 s offence floor, short weak offence lasers, the defence cycle, the
  switch rules and the doctrines.
- **Persistent magazines (CC.7):** a ship is still rebuilt from her fit at each check-out. A
  berth rearm fills to `magazine_max()` and sets `fired_since_full` back to nought; a check-in
  that saves magazines saves the counter with them.
- **Decision support (CC.9):** `+tac/status`, the cockpit, and true odds, bands and heat in the
  mind's rows. The player's `+tac/torp` and `+tac/railgun` still fire the first of each family:
  `space/commands.rs` is CC.9's, and `FireMount` is ready for a mount argument.
- **The Catalog Studio's listing** of the two space packs (doc 34's CS.3 directories). The
  studio's check already builds them through `Catalog::build_fielded`.
- A torpedo's integrity at launch (100) and its flight cap (90 s) stay kernel constants.

## CC.9

Space decision support (Q-49-13, space half, as corrected by doc 48b's W-49-5). Built 2026-11-15
on main `c85a8744f`, with CC.6 (#627) merged. Space has no chance roll, so nothing here is odds:
every figure is one the kernel already uses to decide a shot.

**Survey.**

- **What a player saw.** `+tac/status` printed the lock, the point-defence switch, mount counts
  and heat. `+tac/scan` printed range, light-lag, signature and EMCON. The canopy's cowling
  printed hull, change of speed, heat, PDC mode and EMCON. Nothing showed rounds, a cycle, a
  laser's mode, the lock's range or which band of her hull faced the shooter. The TUI drew the
  ground's text panel under the space canopy, which in space read "not in a cockpit".
- **What a space mind read** (`space/view.rs`, as CC.6 left it): one row a mount it may fire,
  with `p_permille` 850 on every row, one band of 250 km, `heat_per_shot` 0, a `ready` that
  ignored the laser's mode, and `aspect` front on every contact.
- **What the kernel decides a shot by** (`space/bubble.rs`, read only):
  - an offence laser fires when its fit allows offence, it is in offence, its cycle is done,
    the pulse would not take her heat past 1,000 per mille, and the lock lies within its reach;
    it lands at once, `LaserMount::damage_at_range`;
  - the spinal gun fires on its cycle, its rounds, the same heat rule and its reach; its round
    flies `compute_railgun_lead`'s time;
  - the tubes fire on their cycle and magazine, with no reach and no heat;
  - the band struck is `SpaceShipState::select_facet_for_attack`: five bands, front, left,
    right, rear and top (a hull struck from below reads left or right).
- **The bot** reads no space line: its lanes never open a bubble (`mining_cohort.rs` notes the
  picket's words as ones the bot does not have). So it has no space sentence table to add to.

**Built.**

- **One reading, `space::view::fire_picture`.** For a hull in a bubble: every mount she carries
  (lasers, tube families, spinal guns, point-defence rotaries, each family in her fit's order),
  and her lock while her sensors hold it. Each mount (`MountFacts`) carries:
  - its family, a laser's mode, its rounds and what her fit stows (`magazine_max()`; a rotary's
    drum has no maximum the kernel counts back to, so it reads rounds left);
  - its cycle left, its reach and its heat a shot;
  - its state, the first that holds of `down`, `empty`, `cycling`, `too_hot`, `past_reach`,
    else `ready`. Every mount of a hull that is not operational is `down`;
  - against a held lock within its reach, what one laser pulse lands there or how long a
    spinal round flies there.

  The lock (`LockFacts`) carries the ship, her callsign, the range and her band of five that
  faces the shooter.
- **The same rows on every surface.** `mush_pixel::space_cockpit::fire_rows` prints the
  feed's `fire` object (`view::fire_feed`) as a head, a row a mount and a lock row:

  ```text
  Mount          Kind      Rounds/mode     Ready       At the lock
  laser_dorsal   laser     offence         ready       98 a pulse
  rail_spinal    spinal    31/32           in 2.4 s    flies 2.2 s
  pdc_port       point def 4000 left       ready
  Lock raider.7.1 (Belt Raider): 42.0 km, her top band faces you.
  ```

  - **Line mode:** `+tac/status` prints its head line (now lock, PDC switch and heat; the mount
    counts went, the rows replace them), then the rows two spaces in
    (`commands::fire_status_lines`).
  - **The cockpit feed:** `push_space_panels` adds `fire` to every `space.cockpit` keyframe
    from a bubble. `bubble.rs`'s `cockpit_feed` is unchanged.
  - **The pixel canopy** (the TUI's picture and the web's, through `mush-pixel-wasm`) draws
    the rows over the cowling, bottom left, at the font's own size, coloured by what each
    says.
  - **The TUI:** under the space canopy and the holotank, a MOUNTS column of the rows beside
    the comms (`cockpit_text::draw_space_panel`), in place of the ground's panel.
- **The mind reads the same (API 10).**
  - Its rows of `v.weapons` are built from the picture: `p_permille` is `()` (no odds;
    the collector drone's row too), `bands_cm` is the mount's one reach (none for the tubes),
    `heat_per_shot` is the kernel's, and `ready` is cycled, with rounds, and a laser in offence.
    `ready_in_ms` is still the cycle left, which every shipped doctrine reads, so none of them
    moves.
  - New kernel `space_mount(v, id)`, a read at fee 1: one mount's `family`, `mode`, `rounds`,
    `rounds_max`, `ready_in_ms`, `state`, `reach_m`, `heat_per_shot`, `lock_damage` and
    `lock_flight_ms`, for every mount, point defence among them.
  - `Contact.aspect` in a space bubble reads her band of five (`SpaceContactView::facet`);
    on the ground it is unchanged. `ViewContact::aspect` carries the nearest of the arena's
    four, front for top, and nothing in space reads it.
- **The ABI bump, by record.** `API_VERSION` 10, `MIND_ABI = "rhai=1.26.0;api=10;stock=3"`,
  `RETAINED_ABIS` gains api 9 (six rows). The stock, house and acceptance content's headers,
  manifests and envelopes say api 10; their digests moved with the bytes:
  - Standard Doctrine source `2bea2391…7b5fef` to `c544bfbd…f9375f` (`STANDARD_SHA256`, and
    every pin that quotes it or its 16-hex prefix);
  - the field pilot's source, and the three House envelopes' digests in their manifests;
  - **the picket cast:** `content/space/picket_cast.json` names each doctrine by its SHA-256,
    so four entries moved with their headers: the picket's, the raider's, the interdictor's and
    the drone's. `cast.rs` holds them to the files. Until they moved, every picket story failed
    with "the picket's bubble opens on her".
- **Re-pinned by record**, read on Linux (conclav.dev). Each pin moved in a hash alone: no
  fight, count, time or ledger line moved otherwise.

  | Pin | What moved |
  |---|---|
  | `match_/seal.rs` `m5rules_default_seal_identities_are_pinned` | the nine default seal identities; the API 4 projection still holds |
  | `tests/domain_arena/minds.rs` `m1a5_the_production_hash_does_not_move` | three state hashes |
  | `tests/domain_arena/motor.rs` `DUEL_HASH_API6_OBSERVED`, `M1B0_DUEL_LINES` | the duel's hash; its 34 ledger lines, each in `hash` alone |
  | `tests/domain_arena/reflex.rs` `m1a1_…`, `M1B0_J_LINES` | two substep hashes; the 50 ledger lines, each in `hash` alone |
  | `minds/registry/trials/physical_tests.rs` `C4_TRIAL_PINS` | each row's state, ledger and result digests; every case still ends in the same elimination at 58,500 ms |
  | `tests/domain_arena/minds_rhai.rs` `M2_CROSS_PLATFORM_PINS` (`#[ignore]`d, run by name) | the four rows' hashes; the stopping times, the 3,600 thinks and the float bitmask as they were |
  | `tests/harness_baseline/arena.linux.json` (`#[ignore]`d, run by name) | `ledger_bytes_streamed` (1,651,236 to 1,650,975: the hash digits) and the minds lane's four `final_substep_hash` leaves; the record is `cc9_deterministic_update` |

  The harness delta moved those five fields and no other field of its deterministic block. It
  read p50 -6.3%, p99 -2.2% and window p99 -9.5%. Unlike AP.3, I did not dump the ledger line by
  line. The evidence is the delta itself and the 84 arena ledger lines above, each of which
  moved in `hash` alone.
- **Help:** `docs/help/space.md`, "Reading your mounts".

**Tests.**

All runs were on conclav.dev, in a throwaway clone, with the unit's patch applied on
`c85a8744f`.

- **New tests:**
  - `space::view::cc9_tests`, five tests:
    - Halcyon's full fit, a row for each of its eight mounts; the kernel's damage at range and
      the spinal round's flight time;
    - the five bands, each through the picture, the mind's space row and `Contact.aspect`;
    - what holds a mount back: a cycle, defence mode, an empty magazine, a rotary that is down
      or out of rounds, a derelict hull, the heat ceiling (and the kernel refuses the same shot),
      and past reach;
    - a mind's rows: no odds, the true reach and heat, `ready` in defence; `space_mount` for
      every mount and `()` on the ground;
    - every `+tac/status` row within 120 columns at the widest fill on Halcyon's full fit.
  - `mush-pixel`, three tests: the rows' words and tones with no odds; the lock row's widest
    fill (a 32-character id, a cut callsign, `u64::MAX` metres); the canopy draws the rows inside
    its colour budget.
  - `mush-net-ssh`: the space panel at 120 columns prints every mount row whole, beside the
    comms, and says so when there is no fire object.
- **Suites, all passing:**
  - `tacsim`'s lib tests with `domain_arena`, `domain_world` and `domain_field`: 4,108 tests,
    every space story of doc 47 among them (`voyage.rs`, `space_ap9.rs`, `field_ap11.rs`,
    `mw13_ship_mind.rs`, `mw21_completion.rs`, the `tick_thinks_tests.rs` return-fire tests);
  - `mush-pixel` with `mush-net-ssh`: 632 tests;
  - `mush-bot`'s lib tests: 536 tests;
  - the bot job (`cargo nextest run -P ci -p mush-bot`, less its two long stories, and its doc
    tests): 784 tests;
  - `tacsim`'s harness binary's own tests, 24, `every_committed_baseline_agrees_on_the_deterministic_block`
    among them;
  - by name: `minds_rhai::m2_the_twin_corpus_hashes_the_same_on_every_host`,
    `twin::m2_whole_match_hashes_are_equal_between_the_runners_over_the_pool`, and the
    harness in release, its measurement and then its delta. On the re-blessed file the delta
    reads its deterministic block equal (106 fields) and the four minds blocks equal, with
    p50 +0.9%, p99 +3.4% and window p99 +4.5%.
- **Windows:** the C4 trial by hand (`cargo test -p tacsim --lib c4_a_registry_trial -j 2`)
  reads the same four rows as Linux. The cross-platform corpus's Windows run is ci.yml's step.
- **Guards, all passing:**
  - `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` on `tacsim`,
    `mush-pixel`, `mush-net-ssh` and `mush-bot`;
  - the lint job's M7 memory-size pass;
  - `mush-contract-tests`' `ci_policy` guards (`feature_matrix_inventory`, `panic_budget`,
    `restriction_lints`) and the help lint (`engine_contracts help_commands`);
  - rustdoc with `-D warnings`, with and without `--document-private-items`, on `tacsim`,
    `mush-pixel` and `mush-net-ssh`.

**Divergences.**

- **Point defence has no row in `v.weapons`.** A mind cannot fire it (it fires on its own), so
  "every mount" is read through `space_mount(v, id)`, which carries it, as CC.6 left the rows.
- **A rotary's rounds have no maximum.** `PdcMount` keeps no count since full, and
  `space/weapons.rs` is not this unit's file. The rows say `4000 left`, and `rounds_max` is `()`.
- **The lock row's figures sit on the mount rows.** Range and band are on the lock row; the
  damage a pulse lands and the spinal round's flight are on each laser's and spinal gun's own
  row, as `At the lock`, so two lasers of different rows each show their own.
- **`+tac/scan` is unchanged.** Its rows already carry range; the lock's band is on
  `+tac/status`.
- **Files outside the row**, each the least the ABI bump needs, which the brief called for:
  - the API: `minds/mod.rs` (the version, the ABI, the retained list, the two name pins),
    `kernels.rs` (`space_mount`), `api.rs` (`Contact.aspect`);
  - the content: the shipped minds' headers, manifests and envelopes, and
    `content/space/picket_cast.json`'s four digests;
  - the tests that name the API or a digest: `authoring.rs`' two retained-ABI tests,
    `acceptance.rs`, `content.rs`' digest, `tests/domain_arena/content.rs`,
    `tests/support/api4_identity.rs` and `mush-bot/src/mine/market.rs`' draft ABI;
  - the pins re-blessed by record (the table above): `match_/seal.rs`,
    `minds/registry/trials/physical_tests.rs`, `tests/domain_arena/{minds,minds_rhai,motor,reflex,weapons}.rs`
    and the harness baseline.
- **The bot sentence table:** none added, as the bot reads no space line (Survey).

**Not built.**

- `+tac/torp` and `+tac/railgun` still fire the first of each family; a mount argument through
  `FireMount` is open.
- Doc 17's amendment log (its §3, §13 and §16 record each API) does not record API 10: this unit
  writes only its own cut-note section.
- Doc 17's view table and the mind help pages do not list `space_mount`, as they do not list
  `space(v)`. `docs/help/space.md` names it.
- **Land order with CC.2:** CC.2 re-blesses the seal, motor, minds, twin, C4 and harness pins
  too. Whichever lands second re-pins on top of the other.

## CC.7

Space magazines persist (Q-49-12; Q-49-10's magazines). Part 1 was built 2026-11-16, stacked on
CC.9 (`claude/cc9-space-decision-support-ab821d`, PR #634). Part 2, the berth rearm, waits on doc
48's PL.3 (PR #635) and is listed under **Not built**.

**Survey.**

- **Check-out refilled every magazine.** `space_ship_from_tactical_unit_with_spec` builds a ship
  from her fit's overlay, every mount full. Check-in (`build_space_damage_mutation_plan`) wrote
  sections, facets, heat, conditions and the `tacsim.space.subsystems` capability (crits,
  stripped rotaries, `slow_pct`), but no magazine. So every fight began full.
- **A bay repair clears the subsystem record.** `port/bay_work/jobs.rs` revokes
  `tacsim.space.subsystems` when it mends a hull. Magazines kept there would have refilled at
  every repair.
- **The cast was already resupplied, by construction.**
  - `spawn_kirkwood_picket_ships_in_world` makes a fresh hull and unit for the corvette and the
    raider at each spawn (`create_stock_space_unit_in_world` refuses an object that already holds a
    unit).
  - After check-out, `configure_corvette_and_raider_ships` lays the cast's fit overlay anew.
  - An unclaimed cast hull is destroyed after its bubble (`destroy_unclaimed_npc_hull`).
- **The census.** EX.0 (#631, open) counts a round out of the census once a rearm loads it into
  a magazine (`loaded`). A magazine is not a holding, so keeping it across check-ins moves no
  census term; part 2's draws will.
- **Persistence.**
  - While a bubble runs, the ship lives in the saved bubble state. CC.6's `fired_since_full`
    already rides it, skip-when-default, so a restart mid-bubble keeps her rounds (`voyage.rs`'s
    sfr8 holds that state bit-identical across a restart).
  - Outside a bubble, her durable form is her `TacticalUnit` document in the unit store.

**Built.**

- **`tacsim.space.magazines` (`SPACE_MAGAZINES_CAPABILITY`),** a capability of her unit of its
  own: `{"rounds": {mount id: rounds left}}` for each tube family, spinal gun and point-defence
  rotary short of what her fit stows.
  - **Check-in** writes it (`spent_magazines`) and revokes it when every magazine is full. A hull
    that never fired gains nothing, so her unit is the bytes it always was.
  - **Check-out** loads it onto the ship her fit built (`kept_magazines`, `load_kept_magazines`).
    Each magazine keeps what was left and never more than her fit stows. A tube family's or
    spinal gun's `fired_since_full` becomes what the fit stows less that, so a mind still reads
    the fit's maximum. An id she no longer carries is ignored.
  - It is not a key of `tacsim.space.subsystems`, so a bay repair leaves it alone.
- **The cast:** `configure_corvette_and_raider_ships` now says why each spawn sails full, and a
  test holds it.
- **Help:** `docs/help/space.md`, "Magazines": what she fires stays fired, through a restart; a
  bay repair is not a rearm; a patrol is resupplied.

**Tests.**

Runs were on conclav.dev, with the unit's patch applied on CC.9's tip (`3d8d118b2`).

- **New tests**, all on Halcyon's stock unit (torpedoes, spinal rounds and point defence):
  - `space::custody::cc7_tests` (five):
    - check-in keeps exactly the magazines short of her fit, and check-out loads them: every
      round conserved, the fit's maximum still read, and a second check-in idempotent;
    - a full hull gains no capability, and a hull rearmed to full drops it;
    - a restart: her unit's serde round trip, its durable form, keeps the rounds;
    - a bay repair (revoking `tacsim.space.subsystems`) conserves them;
    - a kept count never loads past her fit, and an unknown mount id is ignored.
  - `space::picket::tests::cc7_a_cast_hull_sails_full_whatever_her_unit_kept`: a cast hull
    with spent magazines sails full after the cast's fit is laid.
  - `tests/domain_world/voyage.rs`, the story
    `cc7_a_hulls_magazines_survive_check_in_a_restart_and_check_out`:
    1. a chartered Dray is checked out and her point defence fires 300 rounds;
    2. she is checked in by the real `check_out_space_unit` and `check_in_space_unit`;
    3. the store is folded and the engine rebooted;
    4. her unit still keeps the magazine, and the next check-out loads it: the rounds are
       conserved, and nothing refilled her.
- **Suites, all passing:**
  - `tacsim`'s lib tests with `domain_world`, `domain_field` and `domain_economy`: 3,573 tests,
    the space stories among them. 3,572 passed on the first run. The new story failed: its rig
    folded the check-in's writes before the world's older ones, so the charter's unit document
    won. With the order fixed it passes, run on its own;
  - the bot job (`cargo nextest run -P ci -p mush-bot`, less its two long stories, and its doc
    tests): 784 tests.
- **Guards, all passing:**
  - `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` on `tacsim`;
  - the lint job's M7 memory-size pass;
  - `mush-contract-tests`' `ci_policy` guards and the help lint;
  - rustdoc with `-D warnings`, in both modes.

**Divergences.**

- **The fit's stow is read from the fit her hull flies** (`space_unit_overlay_for_id` of her
  `hull_class`), the one check-out builds her from. A retune that shrinks a magazine clamps what
  she kept to the new stow at her next check-out, so a content change can lose rounds. Rounds are
  conserved across play, not across a retune.
- **`port/bay_work/jobs.rs` and the census are untouched.** Keeping magazines in a capability of
  their own needed no change there.
- **Files outside the row:** `tests/domain_world/voyage.rs` gains the one story below, with a
  local helper that keeps a staff context's durable writes.

**Not built.**

- **Part 2, the berth rearm** (Q-49-12), waits on doc 48's PL.3 (#635), which adds the space
  munitions goods. Until it lands, **nothing refills a player's space magazines**: what a hull
  fires is gone. The rearm is a follow-up PR: a new file in `tacsim/src/port/` that draws
  torpedoes, spinal rounds and point-defence boxes from the hold or the port, at the goods' cost
  plus a labour fee, like the ground's `+bay/order rearm`. It fills to `magazine_max()`, sets
  `fired_since_full` to nought, and rewrites or revokes `tacsim.space.magazines`. Its draws are
  EX.0's `loaded`.
- **A refit refills.** `+coop/refit` re-instantiates the unit (`mush_tactics::module`'s
  re-instantiate route, `port/refit.rs`), which drops every capability, the kept magazines and
  the subsystem record among them. A refit therefore rearms for free. Fixing that belongs with
  the refit's owner.
- **The interdictor** is cast in `space/tick.rs`, which is not this unit's file. It sails full
  the same way: a fresh unit at each spawn, and its overlay laid anew after check-out. No test
  here holds it.

## CC.8

Not built.

## CC.10

Not built.
