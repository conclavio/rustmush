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
  | `tests/harness_baseline/arena.linux.json` | twelve deterministic leaves, by record (`cc1_deterministic_update`) | the minds lanes' four final hashes; `fill_authored`'s thinks, landings, condition fires and drops; the arena lane's ledger bytes. Timing gate live and inside it on the merged tree (p50 −2.2%, p99 +7.7%, window p99 +10.8%) |

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

## CC.1b

The Engage path's think cost (the owner's "Guard at 1,450" follow-up to CC.1). Built 2026-11-17
on main `8519e21c9` (CC.2, #649, and CC.2b, #651, merged). It changes the Standard Doctrine's
script and no behaviour: Gate A held every think of every lane equal to T1. No bound changed.
It also folds in the stale `config.rs` doc for `harness.max_path_cells_per_pulse` (CC.2b's).

**Survey.**
- **Where the cost was.** I profiled the twin lanes in a throwaway clone on conclav.dev.
  - **The probe.** A clone-only sandbox function, `cc1b_mark(label)`, recorded the operations and
    fees spent so far. A copy of the script carried 36 marks, one at the end of each section of the
    think.
  - **Running beside T2.** The twin ran each variant script beside T2 on every think. A variant
    sees the same views, since the twin flies T1's answer, so its costs compare think for think.
  - **The unit of cost.** A Rhai operation is one AST node evaluated: a variable, an operator, a
    call, a statement. An integer constant is free. A property read such as `view.wreck` costs
    three, and so does an index; a mark costs three.
- **The costliest thinks** (operations plus fees, CC.2's tree):

  | Lane | p50 | p99 | max | The costliest think |
  |---|---|---|---|---|
  | mixed | 808 | 1,097 | 1,263 | Engage, two rows, a fresh lock, citing the coaxial gun |
  | pooled | 868 | 1,110 | 1,263 | the same, via Approach's edge into Engage |
  | lance | 1,097 | 1,288 | 1,376 | Engage under an Engage order, eleven rows, citing the coaxial gun |

- **What each section cost**, on lance's costliest think (the mark's three taken out):
  - **The arming** (`arming_for`): about 70 to set up, and about 90 for each of the two mounts.
    Each mount computes four terms and compares the seven of the slot's view row.
  - **The plot stamps** (`stamps`): about 80.
  - **The edges**: about 70.
  - **The hostile edges**: about 75 before `best_hostile`, and about 80 in it with its fee.
  - **The standing order**: about 140 over its four parts.
  - **The notices**: 35 to 130, by the misses read.
  - **Engage's arm and `engage_orders`**: about 270.
  - **The emission's helm, lock and citation**: about 65.
- **What was waste.** Work the think did and threw away, or did twice:
  - the leash's arms called only to return at once;
  - the obeyed order's key formed twice (`standing_order`, then `obeyed`);
  - the objective read twice and an `ends` flag tested on every think;
  - `state == 1 || state == 2 || state == 3` written out;
  - three string compares for `may_pursue`, each with its own `&&`;
  - two gates in `engage_orders` no caller can reach it past;
  - a `min(.., 100)` that never changes a value;
  - a reserve flag (`loaded`) the loop can fold into one.

**Built** (`content/minds/standard.rhai`; still 31 functions, `api: 10`, `stock: 3`). Six cuts. Each
was measured as its own variant, on top of the ones before, on the same views:

| Cut | Saving at the lanes' max (mixed / pooled / lance) |
|---|---|
| 1. The leash's arms are called only under a leash; the station is read only there | 18 / 18 / 18 |
| 2. `standing_order` reads T1's `obeyed` off the one key it already formed, and reads the obeyed order's objective and early ends in one pass | 30 / 30 / 41 |
| 3. `hostile_edges` tests pursuit by ranges (`state < 1 \|\| state > 3`) and one `switch` on the three objectives that hold it | 13 / 13 / 19 |
| 4. The reserve reading folds `loaded` into `low` and reads `reserve_pct` as it is | 6 / 6 / 6 |
| 5. `engage_orders` drops its power and dry gates (see **Divergences**) | 9 / 9 / 9 |
| 6. `arming_for` drops its own test for no rows, reads `reserve_pct` as it is, and holds the ceiling to the mount's reach in one statement | 14 / 14 / 14 |

- **The result**, operations plus fees:

  | Lane | p50 | p99 | max | Bound | Every think, summed |
  |---|---|---|---|---|---|
  | mixed | 808 → 741 | 1,097 → 1,000 | 1,263 → **1,173** | 1,350 | −8.8% |
  | pooled | 868 → 795 | 1,110 → 1,014 | 1,263 → **1,173** | 1,350 | −9.0% |
  | lance | 1,097 → 1,001 | 1,288 → 1,185 | 1,376 → **1,269** | 1,450 | −9.3% |

  Over the 50 costliest thinks of each lane the six cuts save 94 (mixed), 92 (pooled) and 102
  (lance) on average.
- **Tried and dropped.** Reading a miss notice's launch cell before its reason saved nothing,
  because an engaged hull stands still, so its misses launch from its own cell. It cost lance's
  peak one operation.
- **Could the bounds come back down?** Yes. The owner rules on it; this unit changes no bound.
  - **Lance's bound could come down to 1,350**, the lead's first target and the two-tank lanes'
    bound. Its peak, 1,269, is 81 under that.
  - **The two-tank lanes' bound could come back to M2.5's 1,200**, with their peaks 27 under it.
    That margin is thin, and CC.3's ground laser (Q-49-6) will change the lanes' fights.
  - **My recommendation:** lance to 1,350 now. Leave the two-tank lanes at 1,350 until CC.3's
    fights are measured.
- **The `config.rs` doc** for `harness.max_path_cells_per_pulse` no longer says the bound is
  "never a throttle the arena applies". It says the motor holds the field to the same figure per
  `decide_ms` window, and names the test that holds the two figures equal.

**Tests.**
- **New:** `minds::content`'s `cc1b_mirror_a_dry_engaged_hull_and_an_ended_advance`, through both
  runners on every think:
  - a hull engaged under an Engage order whose every mount runs dry withdraws and says nothing of
    fire. This is the dry gate `engage_orders` no longer tests.
  - an immobile hull's Advance ends; its key holds over the next think on the same order; and a
    Hold under another key spends it and is obeyed. This is the one-key `obeyed`.
- **Gate A on every lane:** all 58,233 thinks of the mixed, pooled and lance lanes equal T1's,
  intent for intent and byte for byte, with no fault. The existing mirrors of the reshaped arms
  pass: the ignored or ended order, the supporter's leash, the screen, the reserve and the
  arming.
- **Mutations**, each caught on the mirrors and Gate A's lanes:
  - the one key read the wrong way round (`ignored` inverted): eleven tests, the new mirror,
    the order, leash and screen mirrors and the lance lane among them;
  - an early end that writes no key: the new mirror and the ignored-or-ended order's;
  - a Screen that pursues: the screen mirror and CF.6b's line fallback;
  - the reserve's `low` started at `true`: the unarmed hull's mirror and the mixed lane;
  - the arming held to a mount's first band, not its last: the arming mirror and the pooled
    lane.
- **Re-pinned by record**, read on Linux (conclav.dev). The source moved, a seal's digest covers
  its source, and the arena folds every seal into its state hash, so every pin that holds a state
  hash moved. Nothing else moved: no think, fight, count or time.

  | Pin | What moved |
  |---|---|
  | `minds/content.rs` `STANDARD_SHA256`, `domain_arena/content.rs` `STANDARD_DOCTRINE_SHA256`, `match_/seal.rs`, `support/api4_identity.rs` | the source digest, `eec3f668…154a91` → `e9f1a863…02ed15` |
  | `match_/seal.rs` `m5rules_default_seal_identities_are_pinned` | the nine default seal identities; the API 4 projection still holds, so no custody moved |
  | `domain_arena/minds.rs` `m1a5_the_production_hash_does_not_move` | three state hashes |
  | `domain_arena/motor.rs` `DUEL_HASH_API6_OBSERVED`, `M1B0_DUEL_LINES` | the duel's hash; its 34 ledger lines, each in `hash` alone (the script checks it line by line) |
  | `domain_arena/reflex.rs` `m1a1_…`, `M1B0_J_LINES` | two substep hashes; the 50 ledger lines, each in `hash` alone |
  | `domain_arena/minds.rs` `M1B5_TWO_T1_EVENTS`, `domain_arena/weapons.rs` `M1B0_SEAT_LINES` | the `mind_sealed` lines' `source` prefix |
  | `minds/registry/trials/physical_tests.rs` `C4_TRIAL_PINS` | each row's state, ledger and result digests; every case still ends in the same elimination, filed `Loss` at 241,500 ms |
  | `domain_arena/minds_rhai.rs` `M2_CROSS_PLATFORM_PINS` (`#[ignore]`d, run by name) | the four rows' hashes; the stopping times (262,250, 321,750, 250,500 and 900,000), the probe's 3,600 thinks and the 369098751 bitmask as they were |
  | `tests/harness_baseline/arena.linux.json` (`#[ignore]`d, run by name) | the record `cc1b_deterministic_update`, nine leaves: the ledger stream's bytes (2,692,001 to 2,692,554, the hash digits) and chunks (168 to 169, one boundary crossed), the minds lane's four `final_substep_hash` leaves, and three leaves no path compares (`frame_execution`'s state hash, the snapshot's two sizes) |

  The harness delta moved six compared fields and no other field of its deterministic block.
  Its timings read p50 −9.0%, p99 −8.3% and window p99 −7.7% against TS.21's. On the re-blessed
  file, after a fresh run of the lanes, the delta reads its deterministic block equal (106
  fields) and the four minds blocks equal, with p50 −5.3%, p99 −8.7% and window p99 −5.1%.
- **Sweep:** `tacsim`'s whole suite on the first tree, 4,831 tests. All passed but the 8 hash pins
  above, which were then re-pinned. On the re-pinned tree those 8 pass, and so do the `#[ignore]`d
  corpus pins and Gate A on whole-match hashes, by name. The contract tests passed: 200.
- **Windows:** the C4 trial by hand (`cargo test -p tacsim --lib c4_a_registry_trial -j 2`)
  reads the same four rows as Linux. The cross-platform corpus's Windows run is ci.yml's step.
- **Guards**, on the final tree:
  - `cargo fmt -p tacsim --check`;
  - `cargo clippy -p tacsim --all-targets -- -D warnings`;
  - the M7 memory-size pass;
  - `cargo doc -p tacsim --no-deps --document-private-items` with `-D warnings`;
  - the bot job, `mush-bot`'s whole suite (815 tests, its two long stories among them);
  - the harness binary's own tests;
  - the contract tests, in the sweep.

**Divergences.**
- **Two of T1's gates are not ported** (cut 5). `engage_orders` no longer tests the power and
  dryness that T1's tests, because no caller reaches it past them:
  - the think returns from its power arms on `off` and `starting`, the view's only words but
    `online`;
  - a dry hull is in Withdraw from the edges, and its withdraw stand refuses it.

  `hold_fire` stays. A future caller of `engage_orders` before the power switch, or with a dry
  hull, would need the gates back. The comment at the gate says so.
- **`arming_for` has no test of its own for a hull with no rows** (cut 6). The disarm scan reads
  no row, and only `engage_orders` records a ceiling, past its own return on no rows. D-UM-48 still
  holds: such a hull says nothing.

**Not built.**
- **A deeper cut needs a kernel**, which is a mind ABI bump: the arming's seven compares per mount
  are D-UM-4's on-change parity, and the plot stamps' write and Approach's `line_to` (fee 20) are
  T1's. A kernel that answered "this mount's view row holds these terms" would take about 60
  operations off the Engage path. It is not in this unit's scope.
- **The bounds** are the owner's (above).

## CC.2

The ground palette and the floor (Q-49-2, Q-49-3, Q-49-19, Q-49-22). Built 2026-11-16 and
2026-11-17 on CC.1's tip (`74ca06740`, PR #642), with main `45b50f303` merged in once #642 had
landed (`ce65da267`). Every pin below was read on conclav.dev on this branch.

**Survey.**
- **The floor.** `definitions/weapon.rs` held W14's `CYCLE_FLOOR_MS`, 1,000 ms, for every mount,
  refused on load. No mount could be marked as automatic defence.
- **The shipped ground palette**, before and as ruled:

  | Mount | Cycle | Load (its round's pack) |
  |---|---|---|
  | cannon (`cannon_105`, `he_105`) | 6,000 → **15,000** ms | 40 → **24** |
  | coaxial gun (`mg_coax`, `ball_762`) | 1,000 → **5,000** ms | 2,000 → **600** |
  | autocannon (`autocannon_30`, `ap_30`) | 3,000 → **10,000** ms | 240 → **120** |
  | missile rack (`missile_lr`, `missile_round`) | 10,000 → **30,000** ms | 6 → **4** |
  | pulse laser (`laser_mk1`, `laser_charge`) | 2,000 → **5,000** ms | 500, kept (Q-49-28) |

  No unit pack names a cycle or a load, so `content/units/` did not move.
- **The probe.** CC.2's packs and floor over CC.1's tip, re-signed, and the nine crates' tests
  with no fail-fast, as CC.0 and CC.1 ran them: 7,678 tests, 42 failures, and the D-UM-28 pin
  by name. They sort into:
  - CC.0's list of content pins and seeded fights, every one of which moved (25);
  - a scripted fight timed on the old cannon (`extensions::q6_…`) and a registry trial's
    terminal (`physical_cycle_trial_…`);
  - ten field stories, which run on the wall clock (see **Divergences**);
  - five of the bot's lane tests (see **The bot's windows**).

**Built.**
- **The 5 s floor and the automatic-defence class** (`definitions/weapon.rs`, Q-49-2, Q-49-19).
  - `CYCLE_FLOOR_MS` is 5,000. Every mount a player or a mind fires is held to it, and the
    loader refuses a cycle under it, naming "the 5 s floor".
  - `AUTOMATIC_DEFENCE_FLOOR_MS` keeps W14's 1,000 for a mount authored `automatic_defence`
    (Q-49-10's point defence, a laser in its defence mode), refused under it as "the
    automatic-defence floor". `WeaponDef::cycle_floor_ms()` names the floor a mount is held to.
  - The flag is written only when set, so every shipped pack signs as it did. No ground mount
    ships in the class yet.
- **The ruled palette** (`content/weapons/`, `content/ammunition/`, Q-49-3, Q-49-22): the nine
  packs in the table above, each note rewritten for its numbers, each re-signed with
  `pack_digest` (the tool re-sorts the keys, so only its digest was carried over).
- **The harness window** (`tests/harness.rs`). `DEFAULT_ARENA_S` is 90 where it was 30: six
  cannon cycles and three of the rack's, where 30 seconds held two cannon rounds a hull.
- **The help.** `docs/help/simulator.md`'s autocannon and rack sentences give the new cycles and
  loads; `definitions/content.rs`' two doc comments say the same.
- **The measured fights, by hand** (`tests/domain_arena/twin.rs`,
  `cc2_the_ruled_palette_measured`, `#[ignore]`d; a `by-hand` row in `test-tiers.toml` and a
  recipe comment in `ci.yml`, which the gated-test inventory requires). Sixty one-against-one
  fights on the proving ground at seeds 2,350 to 2,379, to the end or the 900 s cap:
  - thirty of the stock mind against itself;
  - thirty against a seated pilot who starts up, holds the muster, locks the nearest hostile and
    fires every mount that is ready, loaded and in reach, at any odds and to the last round
    (`trigger_happy`, a stand-in for a player pressing `FIRE` on every ready row).

**The measured fights** (run at `ce65da267`, conclav.dev; the same figures at `81c38ea22`, before
the merge):

| | Mind against mind | Pilot (red) against mind (blue) |
|---|---|---|
| Ends | 30 eliminations: red 13, blue 17 | 30 eliminations: the pilot 22, the mind 8 |
| Length | 189,250 to 294,750 ms, median 247,000 (about a quarter of the 900 s cap) | 293,750 to 400,750 ms, median 339,750 |
| Cannon rounds a hull fired | 7.1 of 24 on average, 11 at most | pilot 8.6, mind 7.7 |
| Cannon rounds that hit | 64% (273 of 424): about 15 hits for a full load | pilot 62%, mind 70% |
| Coaxial rounds a hull fired | 194 of 600 on average | pilot 238, mind 1.3 |
| Coaxial rounds that hit | 39% (4,493 of 11,630): about 230 for a full belt | pilot 34%, mind 55% (22 of 40) |
| Withdrawals | none | none |

- **No fight runs to the cap**, and none comes near a load: no hull fired more than 11 cannon
  rounds of its 24, so no mind reached its reserve and none withdrew, dry or hurt. Fights end on
  the first hull lost, at about a third of the cannon's load.
- **The pilot who fires on every ready row beats the stock mind 22 to 8.** The mind, held to the
  Line card's 35% floor, fights from cannon range and almost never fires its coaxial gun (40
  rounds in thirty fights), while the pilot's coaxial puts some 80 rounds a fight on the mind.
- **The lanes agree.** The stock pool (two against two, `M4_5_POOLED_RECORD`) ends between
  248,500 and 412,750 ms. In Gate A's mixed lane (one armed and one unarmed hull a side) eight of
  the fifteen matches end in an elimination, between 303,500 and 546,750 ms, and seven reach the
  cap.

**Tests.**
- **New:**
  - `definitions::cc2_the_floor_is_five_seconds_and_automatic_defence_keeps_the_engines`. The
    constants; the shipped coaxial gun loads at 5,000 ms and is refused at 4,999, naming the
    floor; marked `automatic_defence`, it loads at 1,000 and is refused at 999.
  - `definitions::cc2_the_ground_palette_is_the_ruled_one`: the five cycles and five loads as
    shipped, a content pin by design.
- **Mutations** (each on conclav.dev, against `domain_arena`'s definitions tests):

  | Mutation | Caught by |
  |---|---|
  | `CYCLE_FLOOR_MS` back at W14's 1,000 | `cc2_the_floor_is_five_seconds_and_automatic_defence_keeps_the_engines` |
  | `cycle_floor_ms` ignoring `automatic_defence` | the same |

  A retuned cycle or load in a pack fails `cc2_the_ground_palette_is_the_ruled_one` and the
  digest pins by construction.
- **Re-blessed by record** (every value read on conclav.dev; paths under `crates/tacsim/`):

  | File and test | Pin | What moved |
  |---|---|---|
  | `tests/domain_arena/content.rs` `the_pinned_digests_are_the_shipped_ones` | `DIGESTS` | the cannon's, the coaxial gun's and their rounds' four packs |
  | `tests/domain_arena/extensions.rs` `ae1`, `ae3`, `x2` to `x6` | `LINE_TANK_DIGEST` | the tank's definition carries its mounts' cycles and loads: `c85d03ac…` → `3fc31438…` |
  | `tests/domain_arena/extensions.rs` `ae2`, `x1` | `DIGESTS_ORDNANCE`, the laser's row; the autocannon's and the rack's authored tuples | the four ordnance packs and `laser_mk1`; `(3, 10_000, 20, 120, …)` and `(1, 30_000, 350, 4, …)` |
  | `src/match_/seal.rs` `m5rules_default_seal_identities_are_pinned` | the API 4 projections and the seal identities | the line tank's and the heavy's six of each (their captured mount facts); the target's three unmoved |
  | `src/minds/registry/trials/physical_tests.rs` `c4_a_registry_trial_ends_the_same_on_every_host` | `C4_TRIAL_PINS` | every case ends in the same elimination, filed `Loss`, at 241,500 ms where it was 106,500; run on Windows by hand too, the four rows equal |
  | the same file, `physical_cycle_trial_…` | `cancelled_uncalled` | 1 → 0 (see **Divergences**) |
  | `tests/domain_arena/motor.rs` `m1a1`, `gc1b`, `m1b0` | `DUEL_HASH_API6_OBSERVED`, `DUEL_HASH_AT_HEAD`, `M1B0_DUEL_LINES` | hashes only: the duel fires no shot in its minute, and its 34 lines equal CC.1's once the hash is out. `DUEL_HASH_AT_HEAD` moves too, as CC.0 found |
  | `tests/domain_arena/minds.rs` `m1a5_the_production_hash_does_not_move` | the shipped duel's pin | `0xcaae…6143` → `0xdeb8…5f67`; the field's two, on the rig's own mounts, unmoved |
  | `tests/domain_arena/minds_rhai.rs` `m2_the_twin_corpus_hashes_the_same_on_every_host` | `M2_CROSS_PLATFORM_PINS` | stopping times 231,250 / 197,500 / 167,000 → 262,250 / 321,750 / 250,500 (900,000 unmoved); the probe's thinks and bitmask unmoved |
  | `tests/domain_arena/twin.rs` `m4_5_…` | `M4_5_POOLED_RECORD` | ends 167,000–240,500 → 248,500–412,750 ms, the split 13–2 → 10–5, shots 1,058/936 → 670/531, thinks 11,256 → 16,502, `floor_held` 1,039 → 4,437 |
  | `tests/domain_arena/twin.rs` Gate A's three lanes | `COVERAGE`, `POOLED_MATCHES_WITH_STAND_IN` | pooled 10,851/405 → 16,066/436, mixed 12,866/345 → 33,734/534 (at the 900 s cap), lance 7,142/159 → 8,433/133; 12 → 8; no divergence on any lane |
  | `tests/domain_arena/report.rs`, `replay.rs` | `FIGHT_SEED`; the winner's counts | 2,352 → 2,351; seven cannon rounds and 190 of the coaxial gun's |
  | `tests/harness_baseline/arena.linux.json` | 121 leaves, by record (`cc2_deterministic_update`) | see **The harness** |

  The Rhai think's cost moved down with the slower fire: the pooled and mixed lanes peak at
  1,263 operations and fees (CC.1: 1,302 and 1,275), under `TWO_TANK_THINK_BOUND`'s 1,350; the
  lance lane at 1,376 (CC.1: 1,424), under `LANCE_THINK_BOUND`'s 1,450. Gate A held on every
  lane: no divergence.
- **The harness** (`tests/harness.rs`, `tests/harness_baseline/arena.linux.json`). At the 90 s
  window every deterministic leaf moved (118 differences in the delta), so the fresh run's
  non-timing leaves are carried onto main's file by record, with every earlier record kept and
  TS.21's timing leaves and host kept. The record is `cc2_deterministic_update`, with each moved
  leaf's previous value. Verified at `0c3e429ae` on conclav.dev: the delta reads the
  deterministic block equal (106 fields), the four minds blocks equal, and the timing gate live
  and inside it (wake p50 −6.5%, p99 −5.4%, window p99 −4.0%). A first reading on a box other
  sessions were loading had the window p99 at +21.1%; it was the load.
  - **A known breach, on the lead's ruling.** Under `TACSIM_ASSERT=1` (TS.21's procedure A), the
    CI cohort's `motor.peak_per_pulse.path_cells` reads **50,425** against
    `harness.max_path_cells_per_pulse` **48,128**. It is at seed 2350, in the wake ending at arena
    101,500 ms (the 90 s window's last): seven path queries in one wake, every one found, none
    failed, about 7,200 cells each. It is several hulls routing at once, not one search sealed in
    a pocket. The 30 s window never reached it. Every other bound and lens holds, and the
    authored cohort peaks at 20,318. **The bound is kept, so the weekly asserted run
    (`scripts/harness-weekly.sh`) is red on this figure until CC.2b lands.** The CI job's delta
    does not assert absolutes and compares the bound as an equal threshold, so it is green.
    CC.2b, stacked on this branch, is a per-wake path budget in the motor (`arena/motor.rs`).
- **The bot's windows** (`crates/mush-bot/tests/arena_lane.rs`, test code only, on the lead's
  word): windows widened for the 15 s cannon; CC.10 owns the discipline. Each reads the cannon's
  cycle through `Server::cycle_ms`, which the rig reads from the shipped content; the only
  literals are the windows' old figures.
  - `a_due_fire_beats_a_due_poll`: three shots, the first shot's 20 s window plus two cycles.
  - `a_due_gun_keeps_its_slot_from_a_poll`: the crowded minute's learning window plus one cycle,
    so a cycle the first shot's read missed is learned on the second.
  - `a_full_width_channel_line_never_swallows_an_expected_answer`: 15 s plus one cycle for the
    next shot. It also tries the next shot when the cannon's `FIRE` shares its tick with a poll
    (see **Not built**: that case is a finding).
  - `a_withdraw_on_a_failing_hull_holds_then_lapses`: reads the stop from the hold's own end, the
    tick it lapsed in. Under the 15 s cannon the `SPEED 0` goes in that tick (42,200 ms), one
    tick before the test's old window opened (42,250).
  - `an_empty_gun_is_not_fired_again`: the old 20 s or two cycles, whichever is longer.
- **Field stories** (see **Divergences** for each): `field_host` `cf2`, `field_carrier` `cf3`,
  `field_crash` `cf_review`, `field_gc3` `gc3`, `escort_lease` `cf6c`, `field_story` `gc0` and
  `s2`, and the five yielding `surrender` stories.
- **The sweep**, on conclav.dev, the nine crates' tests with no fail-fast, then by name the
  D-UM-28 pin and the measured fights:
  - at `e842bb416`, CC.2 over CC.1's tip: 7,678 tests, all passing (65 skipped);
  - at `ce65da267`, main `45b50f303` merged in (CC.7 part 2 and EX.3a among it): 7,685 tests,
    all passing, and `cargo clippy -p tacsim -p mush-bot --all-targets -- -D warnings` clean.
- **Guards**, at `e842bb416` on conclav.dev, all passing:
  - `cargo fmt --check` for `tacsim`, `mush-bot` and `mush-contract-tests`;
  - `cargo clippy --all-targets -- -D warnings` for `tacsim` and `mush-bot`;
  - the CI lint job's M7 memory-size pass;
  - `cargo doc --no-deps --document-private-items` with `-D warnings` for `tacsim` and
    `mush-bot`;
  - `mush-contract-tests` whole (the census, the panic budget, the restriction lints, the help
    page lint and the gated-test inventory);
  - `mush-bot`'s tests whole (the bot job), and the harness binary's own tests.

**Divergences.**
- **The field rigs' quiet interval reads the content** (`support/field/field_host_rig.rs`,
  `quiet_s`). A field disengages after `[tacsim.field].disengage_s` with no hostile fire, and
  the rigs held 15 s. Under the 15 s cannon that closed a field between two rounds of one gun:
  in `cf2` and `cf3` the field ended at 15,301 ms, the millisecond the cannon was ready again.
  `quiet_s` is the slowest cycle of the line tank's and the escort's mounts, read from the
  shipped content, plus five seconds: 20 s. `field_story`'s and `field_gc3`'s 15 s overrides are
  gone.
- **The field story writes a summary every pulse** (`field_story.rs`,
  `cfg.arena.ledger.summary_ms = 100`, as `field_ap1`'s rig does). The black box closes a chunk
  only when it is full (16 KiB), and the story reads its Fired and SectionDestroyed lines off
  closed chunks. With a coaxial burst every 5 s, not every second, the raiders' fire alone fills
  a chunk in minutes: a print of the raiders' thinks showed them firing from 5 s into the field
  while no Fired line reached the story inside its 60 s.
- **The yielding surrender stories wait 300 s for the demotion** (`support/field/surrender_rig.rs`,
  was 150). After the first crew yields, the crew that holds fights on at the stranger's hulls,
  and the field demotes a quiet interval after its last shot: 278 to 279 s on all five cases at
  256 and 512, measured with a 600 s wait. The wait returns as the field settles, so each story
  takes as long as its fight. (Emptying the holding hull's magazines was tried first and is wrong:
  a hull that was armed still counts in `hostile_in_weapons_range` when it has spent its rounds,
  so it held the field open for good.)
- **The mixed Gate A lane runs to the match's own 900 s** (`twin.rs`, `MIXED_CAP_S`, was 300).
  At 300 s no match of the fifteen made a turret kill, so the lane's coverage (a mind dropped to
  the ROM) was empty and its assertion failed by name.
- **The mixed lane counts a withdrawal of either kind.** Its "some withdraws" claim was the
  unarmed hulls'. Under the ruled palette an unarmed hull loses sections to heavier, rarer rounds
  and is disabled before it wears down to the withdraw edge: none of the fifteen withdrew, even at
  900 s. The lane now counts the armed hulls' withdrawals too (they withdraw at their reserve,
  CC.1's edge, or hurt: one of the fifteen did), prints both, and asserts their sum. The unarmed arm is
  held by `minds::content`'s `m2_mirror_the_withdraw_dwell_and_the_rally`.
- **The seeded real fight moved to seed 2,351** (`report.rs`, `replay.rs`). At 2,352 the dry
  tank's turret now goes first (a drop to the ROM at 71,470 ms), and it is never wrecked. A scan
  of seeds 2,350 to 2,409 found 2,351 the first whose fight ends in the loser's wreck with no
  drop (101,418 ms). Ten of the twenty seeds 2,350 to 2,369 end in a wreck; the porter measured
  eight at 17b cut M4.5a.
- **The scripted leg fight steps by the cannon's cycle** (`extensions::q6_…`). It fired every
  10 s, which was "every time the cannon is ready" at 6 s. It now steps by the cycle read from the
  catalog, rounded up to the coarsest cadence (15 s), and reads its halfway point off the first
  leg's loss.
- **A registry trial's final wreck has no uncalled suffix** (`physical_tests.rs`,
  `physical_cycle_trial_…`, `cancelled_uncalled` 1 → 0 by record). Under the slower cycles the
  last wreck lands with no think of the wrecked unit drawn and uncalled.
  `meter::physical::tests::wreck_terminal` still builds the suffix of one on purpose.
- **Two files outside the row**: `ci.yml` gains the measured fights' recipe comment, and
  `test-tiers.toml` its `by-hand` row. The gated-test inventory requires both.

**Not built.**
- **No count changed.** The magazines are the ruled 24 / 600 / 120 / 4. What the measured fights
  say about them is for the owner (Q-49-22).
- **CC.2b**, the motor's per-wake path budget: until it lands the weekly asserted harness run is
  red on the CI cohort's path cells (above).
- **A bot finding, for CC.10.** When the cannon's `FIRE` shares its tick with a poll (`RANGE`), a
  full-width channel line before the ack takes the ack: the shot is counted only off the next
  `WEAPONSTATUS`, 1,670 ms later (in the lane's rig, the first acked shot, typed at 40,750 ms). The
  old 6 s cannon never put its first acked shot on a poll's tick. AM.1's test keeps its own case,
  a `FIRE` alone in its tick.
- **No ground mount ships as automatic defence.** The class exists for point defence and the
  laser's defence mode (Q-49-10); later units author it.
- **The quiet interval's floor** (`config.rs`, `FIELD_DISENGAGE_S`, 10 to 3,600 s) sits under
  the cannon's 15 s cycle. The shipped default, 120 s, is well over it, but an operator who set
  10 or 15 would see a field disengage between two rounds of one gun, as the rigs did. A
  follow-up could floor it at the longest shipped cycle.
- **A quiet field's black box reaches the world late.** Chunks close at 16 KiB, so with sparse
  fire the world reads a shot's lines minutes after it. Durability is unchanged (the snapshot
  rides behind every closed chunk); only the lag grew.
- **The heavy's unit note** (`content/units/heavy_mk1.json`) still says version 2 added "a
  six-round missile rack". It is the version's history; re-signing the heavy's pack for it would
  move that pack's pin for a note.

## CC.2b

The motor's path cells per window (the lead's follow-up to CC.2's harness breach). Built
2026-11-17 on CC.2's branch (PR #649, main `45b50f303` merged), stacked on it. Scope:
`tacsim/src/arena/motor.rs` and its tests, and on the lead's word the snapshot format
(`arena/snapshot.rs`) and its shape pin.

**Survey.**
- **The breach.** Under `TACSIM_ASSERT=1` at CC.2's 90 s window, the harness CI cohort's
  `motor.peak_per_pulse.path_cells` read 50,425 against `harness.max_path_cells_per_pulse`
  48,128: seed 2350, the wake ending at arena 101,500 ms, seven searches, every one found.
- **The bound was a threshold, never applied.** `config.rs`' doc for the bound says so: "a
  threshold the harness asserts, never a throttle the arena applies", because a controller that
  stopped searching when a per-wake budget ran out would make a route a function of the tempo
  (D-16-2).
- **The motor already budgets in arena time.** `MotorTable::may_search` holds the field to
  `reflex.max_path_queries` searches per `reflex.decide_ms` window of arena time (1 s), for the
  same reason; a hull it refuses keeps the path it has and searches in a later window.
- **What a search costs past its budget.** `PathScratch::last_expanded` counts the expansion
  that breaks a `Budget` and, past `POCKET_PROBE_CELLS` expansions, the goal-side probe's flood:
  at most `POCKET_PROBE_CELLS + 2` (1,026) more than the A* budget handed in.

**Built** (`arena/motor.rs`).
- **`PATH_CELLS_PER_WINDOW`**, 48,128: the harness's figure, now the motor's budget per
  `decide_ms` window of arena time, beside the query budget. A wake lies inside one window, so a
  window held to it holds every wake to it, at every tempo.
- **`MotorTable` counts the window's cells** (a new field, `cells`), reset when the window
  turns. Like the window and the query spend beside it, it is not folded in the state hash, and a
  snapshot carries it, so a match resumed mid-window defers as an unbroken one does.
- **`SNAPSHOT_FORMAT` 20** (`arena/snapshot.rs`): the motor table's `cells`. Format 19 is
  refused by name before state decode, as every earlier bump; pre-launch, in-flight matches
  restart rather than migrating.
- **Each search is handed what the window has left**, less 1,026 (`SEARCH_OVERHEAD_CELLS`), and
  never more than `reflex.path_max_cells`. So the window's sum never passes 48,128.
- **No search starts** once less than `POCKET_PROBE_CELLS` (1,024, the expansions that close most
  routes) is left, after the window's first: the hull keeps its path and searches in a later
  window, as one the query budget refused does.
- **A search that runs out of what others left is deferred, not failed**: its `searched_ms` and
  `searched_from` are put back, so it searches again at its next tick in a later window, and
  `path_deferred` counts it. A search that runs out of a window it opened found no route a window
  can pay for, which is the `unreachable` edge's `Budget` as before.

**Tests.**
- `motor::tests::cc2b_the_window_holds_its_path_cells`: the figure equals
  `HarnessConfig::default().max_path_cells_per_pulse`; a fresh window hands 47,102; a later search
  gets what is left; under 1,024 left, no search; the next window opens with nothing spent; a
  smaller `path_max_cells` still caps.
- `motor::tests::cc2b_a_burst_of_routes_holds_the_window_and_every_hull_routes`: eight hulls on a
  200 by 200 window, each sent across a wall with one gap at its far end, so each search floods the
  near side (about 19,500 cells); all eight followed at one arena millisecond, a window at a time.
  Per window, (cells, deferred, failures): (47,103, 6, 0), (47,103, 4, 0), (47,103, 2, 0),
  (39,099, 0, 0). Two hulls route in each window, all eight by the fourth, none fails, and the run
  is the same twice.
- `motor::tests::cc2b_a_match_resumed_mid_window_defers_as_an_unbroken_one` (the lead's): the
  burst followed over two windows, once unbroken and once with the arena encoded and decoded after
  the first three hulls of the first window, when the window has spent most of its cells. The
  decoded table holds the same cells, and every hull's route, every counter and the table's
  window, searches and cells come out the same.
- **Mutations**, each caught:

  | Mutation | Caught by |
  |---|---|
  | No cell budget (every search handed `path_max_cells`, none refused) | both tests |
  | A deferred search failed as `unreachable` | the burst test |
  | `cells` not carried by the snapshot (`#[serde(skip)]`) | the resume test |

- **The harness, the done-when** (`tests/harness.rs`, at the 90 s window, conclav.dev). Under
  `TACSIM_ASSERT=1` every lane passes, `harness.max_path_cells_per_pulse` unchanged at 48,128:
  the CI cohort's busiest wake (seed 2350, arena 101,500 ms) now expands **47,985** path cells
  where it expanded 50,425; the authored cohort's peak stays 20,318. The weekly asserted run is
  green again.
- **Re-blessed by record:**

  | File and test | Pin | What moved |
  |---|---|---|
  | `tests/harness_baseline/arena.linux.json` | 12 leaves (`cc2b_deterministic_update`) | the motor's: peak path cells 50,425 → 47,985, total 135,684 → 133,244, one more deferral (590 → 591), the slope and corner refusals the deferred search no longer made; the `stock_ci` minds lane's final substep hash; uncompared, the snapshot's bytes (+12, the new field) and `frame_execution`'s state hash |
  | `tests/domain_arena/arena.rs` `af1_the_snapshot_shape_is_pinned` | `AF1_SHAPE_DIGEST`, `AF1_SHAPE_LINES` | one path, `state.motors.cells: number`: 739 → 740 |
  | `arena.rs` (four), `conditions.rs`, `deliberation.rs` (two), `minds/resimulation.rs` | the literal format each round trip checks | 19 → 20 |

  The delta against the re-blessed file reads the deterministic block equal (106 fields) and the
  four minds blocks equal. The timing gate, on a box other sessions were loading (load 5 to 8),
  read the window p99 from +9% to +34% over three runs. So CC.2's and CC.2b's harness binaries
  were run back to back, alternately, twice each (wake p50 / p99 / window p99, ms):

  | Run | CC.2 | CC.2b |
  |---|---|---|
  | first | 4.92 / 5.89 / 11.42 | 5.24 / 9.00 / 16.47 |
  | second | 5.14 / 6.76 / 13.21 | 5.05 / 6.06 / 12.50 |

  The budget costs nothing measurable; the spread is the box. Against TS.21's baseline, CC.2b's
  second run reads −7.6%, −8.1% and −2.2%, inside the 20% gate.

- **The sweep** (conclav.dev, the nine crates with no fail-fast, then by name the D-UM-28 pin and
  the measured fights): 7,688 tests at `93756fb46`, all passing but the eight format literals
  above, which the bump moved and `762711899` and `e19c48e5f` re-pin; those eight, every snapshot test and the
  CC.2b tests then pass at `e19c48e5f` (69 tests), with three more literals there (the
  refused-format errors' `expected: 20`). The D-UM-28 pin and CC.2's measured fights pass,
  their figures unchanged.
- **Guards**: `cargo fmt --check`, the M7 memory-size pass and rustdoc with `-D warnings` for
  `tacsim` at `93756fb46`; `cargo clippy -p tacsim --all-targets -- -D warnings` at
  `762711899`, clean (a type alias for the resume test's follows, which `type_complexity` asked for).

**Divergences.**
- **A window, not a wake.** The lead's done-when names a per-wake budget. The budget is per
  `decide_ms` window of arena time, as the query budget is, so a route never depends on the
  tempo (D-16-2); a wake lies inside one window, so the per-wake figure the harness asserts is
  bounded by it. A deferred hull searches at its next tick, one `decide_ms` later (four wakes at the
  `Engaged` tempo): in the burst above, the eighth hull waited three windows.
- **Not oldest first, but tick order.** Within a window the searches run in the order the hulls'
  motor ticks fall, and a deferred hull competes again at its next tick. The burst shows the wait
  is what the burst costs over what a window pays, and no hull starves while windows keep
  refilling.
- **One search's ceiling is 47,102 cells, not `path_max_cells`' 65,536.** A search that needs more
  than a whole window can pay is the `unreachable` edge's `Budget`. AI.1's pocket probe answers the
  floods that used to reach that size.

**Not built.**
- **`config.rs`' doc for `harness.max_path_cells_per_pulse`** still says the bound is never a
  throttle the arena applies. It is now the motor's window budget too; the doc is outside this
  unit's scope (motor.rs and its tests) and is the lead's to rule on.

## CC.3

The ground laser (Q-49-6, Q-49-27, Q-49-28). Built 2026-11-17 on CC.1b's tip (`f380cf615`,
PR #661), stacked on it. The defect PR it waited on (#623, a yard-built laser keeps its charge)
had merged.

**Survey.**
- **Against armour.** A charge did 25 against everything: five charges dropped a drone, the same
  as four coaxial bursts. A charge heavy enough to drop drones quickly would have out-damaged the
  cannon against a tank's armour per minute, which Q-49-6 forbids.
- **Heat.** A shot that would have crossed the ceiling was capped at it, and only a platform
  already at the ceiling was refused. At the 5 s cycle the cooling always made room, so heat never
  held a shot back.
- **The mounts.** `line_tank_laser` was the line tank with its 105 swapped for the laser, and
  `biped_mk2` carried the laser and a machine gun. Neither carried a main gun.
- **The ripple.** An agent surveyed every reader of the two units and the laser packs outside
  `content/`:
  - the bay refit and its family rule (the cannon must stay ahead of the machine gun);
  - the bare-FIRE rule (a mount called `cannon` wins it);
  - the hull bills, the pixel looks, mushd's `sim_units` story, the help page;
  - the harness cohorts and the lance lane, which field the laser tank.

**Built.**
- **The combat kernel against armour** (`arena/damage.rs`, Q-49-27). A beam spends a facet's armour
  at `BEAM_ARMOR_PERMILLE` (200, a fifth) of its damage, and stops there while the facet holds.
  Once the facet goes down, what is left of the beam reaches the internals in full. A projectile
  spends armour point for point, as before. The round records that it is a beam at acceptance
  (`Shot.beam`), as it records its damage, so a turret shot off in flight still lands its beam.
- **Heat refuses** (`arena/heat.rs`, Q-49-28). A shot whose heat would carry the platform past its
  ceiling is refused (`UnitState::refuses_heat`). The same rule is read by:
  - the arena's FIRE;
  - the frame's reading (`FrameHeat::refuses`), and through it the cockpit's FIRE and
    `WEAPONSTATUS`;
  - the mind's `would_overheat`, which T1 and T2 share.

  The status line's OVERHEATED still means "at the ceiling".
- **The laser content** (Q-49-6):
  - the charge is 120 where it was 25: against armour a fifth of that is a fifth of a 105 shell,
    and a drone falls in three;
  - the laser keeps X.1's heat: a 60 ceiling, 20 a pulse, 3 a second of cooling;
  - `line_tank_laser` version 5 carries the 105 back as its main gun, listed first, with the laser
    and the coaxial gun beside it, so Class I's two slots arm the cannon and the laser;
  - `biped_mk2` version 3 carries the 105 in its right arm, the laser in its left and the machine
    gun in its torso;
  - the notes are rewritten under the 1,024-byte bound, and "what you wear a tank down with" is
    gone.
- **Replays:** `REPLAY_ENGINE` is `tacsim-engine-cc3-v1`, so a tape recorded under the old rules is
  refused.
- **Help:** `docs/help/simulator.md`'s laser tank, Mk2 and bare-FIRE entries.

**Measured.** All counts are charges that landed:
- **A drone falls in three**, from whatever face they strike: ten seconds at the laser's cycle.
  It took five.
- **A line tank is never wrecked by seven.** Its cheapest wreck, through its rear hull or rear
  turret, takes eight: at least 35 s at the laser's cycle, before any heat lockout. Three 105
  rounds can wreck it from the rear.
- **Sustained fire locks out.** From cold the laser fires nine charges at its cycle. After that a
  charge waits about 1.7 s for the platform to cool to 40, so it fires about one every 6.7 s where
  it fired every 5.
- **The lance lane** (twin, Gate A at 0 divergences): its thinks cost p50 865, p99 1,185, max 1,257
  (CC.1b: 1,001, 1,185, 1,269), under `LANCE_THINK_BOUND`'s 1,450 and the two-tank lanes' 1,350.

**Tests.**
- **New:**
  - `arena::damage` `cc3_a_beam_spends_armour_at_a_fifth_and_reaches_the_internals_in_full`: a
    facet that holds, the beam that takes it down, a beam too weak to dent it, and a facet already
    down;
  - `arena::heat` `cc3_a_shot_that_would_cross_the_ceiling_is_refused_and_one_onto_it_is_not`:
    the unit and the frame agree, and the status line keeps "at the ceiling";
  - `domain_arena` `extensions::cc3_a_laser_drops_a_drone_quickly_and_not_a_tank`: the drone in
    three and the tank surviving seven. Rounds are conserved: every charge fired is one off the
    store, and a charge refused for heat is none.
- **Mutations**, each caught:
  - a beam spending armour as a round does: the beam test, the acceptance test and both X.1
    tests;
  - a beam leaking its remainder into the internals while its facet holds: the beam test;
  - heat refusing only at the ceiling again: the heat test, the no-heat mirror, X.1's heat test
    and the hot-gun reflex test;
  - the frame reading the old rule, so the mind does too: the heat test and the no-heat
    mirror.
- **Re-pinned by record:**

  | File and test | What moved |
  |---|---|
  | `extensions.rs` `DIGESTS`, `DIGESTS_CHASSIS` | the laser charge, the laser, the laser tank and the Mk2 |
  | `content.rs` `m4_…`, `extensions.rs` `ae1_…`, `ae2_…` | the versions: the laser tank 4 → 5, the Mk2 2 → 3 |
  | `extensions.rs` `x1_heat_rises_…` | at 54 the laser is held (it crossed back in at 54 until CC.3); it fires at 39 |
  | `extensions.rs` `x1_a_laser_pulse_…` | the claim is a beam's share against armour |
  | `stock_mind.rs` `d2_…`, `reflex.rs` `t1_an_overheated_machine…` | a shot that would cross the ceiling is held by the mind, as the engine refuses it |
  | `reflex.rs` `t1_an_unreachable_order_and_a_hot_gun…` | its one shot starts at 40 and lands on the ceiling |
  | `conditions.rs` `m1b2_t1_…` | its field takes the laser tank's 105 off, so the heat story stays on the laser and the coaxial (S1, S2); its sink comes back at ten times the authored rate, so the platform is under 40 at every ready after the cooling and the laser keeps the lattice the re-lock's citation lands on |
  | `minds.rs` `m1b0_…` (two), `minds_rhai.rs` `m3_…` (three) | their hand-built beams: five for the one point of armour (was one), 2,000 to shoot a turret off (was 1,000) |
  | bare FIRE: `weapons.rs` `ae2_…`, `content.rs` `e1_…`'s list, `cockpit/tests.rs` `ae2_…` | the laser machines' bare FIRE is the 105, and a dry 105 falls to the laser |
  | `cockpit/tests.rs` `cc4_…` | the widest rounds row is the Mk2's (cannon, laser, machine gun), under 120 columns |
  | `refit.rs` `ud4_…`, `hardware/flow/bay.rs` `ud4b_…` | the laser goes in beside the 105 and comes off alone; no cannon comes back to the bay |
  | `content/industry/hulls-v2.json`, `port/bill.rs` | the laser tank's bill 21 / 7 → 19 / 7 and the Mk2's 39 / 17 → 37 / 16 (the designs' own bills), re-signed |
  | `domain_economy/yard_in9.rs` | the yard-built laser tank's parts take the 105 |
  | `twin.rs` `COVERAGE` | the lance lane, 8,433 / 133 → 8,105 / 177 |
  | `resimulation.rs` `m7_…` | the replay engine's `cc3` |
  | `mushd` `sim_units.rs` | the laser tank shows its 105, and its pilot names the laser |
  | `tests/harness_baseline/arena.linux.json` (`#[ignore]`d, run by name) | the record `cc3_deterministic_update`, 89 leaves: both cohorts field the laser tank, whose ROM arms its first mount, now the 105, so the fights moved (shots 1,691 → 1,362, the ledger 2,692,554 → 2,500,995 bytes), and the busiest wake's path cells read 48,128, exactly CC.2b's window budget. Timing p50 −11.1%, p99 −14.6%, window p99 −12.7% against TS.21's |

- **Sweep**, on the final tree:
  - `tacsim`'s whole suite: 4,834 tests, all passing but `m1b2_t1`, whose cooling was then
    raised (above) and which passes, run again on Windows;
  - mushd's `sim_lenses`, 95 tests;
  - `mush-pixel`, `mush-core`, `mush-economy`, `mush-industry` and the contract tests: 2,649
    tests;
  - the bot job: 815 tests.
- **The harness**, in release: every lane under `TACSIM_ASSERT=1`, the path-cell bound included.
  Then the delta on the re-blessed file: its deterministic block equal (106 fields), and the four
  minds blocks equal, with p50 −9.4%, p99 −9.1% and window p99 −6.1%.
- **Guards**, on the final tree:
  - `cargo fmt -p tacsim -p mushd --check`;
  - `cargo clippy -p tacsim -p mushd --all-targets -- -D warnings`;
  - the M7 memory-size pass;
  - `cargo doc -p tacsim --no-deps --document-private-items` with `-D warnings`.

**Divergences.**
- **A beam's share is a kernel constant**, not a field on the round. Q-49-27 makes it a rule of the
  combat kernel for every beam, and the ground ships one beam.
- **No snapshot format bump.** `Shot.beam` is skipped when false, so no shape moves, and a shot
  written before CC.3 decodes as a projectile, which it was.
- **The heat figures are X.1's.** Under the refusal they lock sustained fire out after nine
  charges, the "now and then" Q-49-28 asks for. No pack's heat moved.
- **The Mk2's machine gun moved to its torso**, so its arms carry the 105 (the Mk1's arm) and the
  laser.

**Not built.**
- **The space lasers** (Q-49-7) are the space kernel's, in CC.8.
- **The charge store stays 500** (Q-49-28), and Q-41-7's free recharge away from port stands.

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

### Part 2, the berth rearm (2026-11-16)

Built on main `73d05678b`: CC.9, CC.7 part 1, doc 48's PL.3 (the space munitions goods) and
EX.0 (the census's `loaded`). The lead ruled the shape: the bay order, not a verb of its own.

**Survey.**

- **The ground's rearm is a bay order** (`port/bay_work`), and it already gives:
  - the tariff's labour per round and time per mount at the operator's multiplier;
  - the fee escrowed at the take and captured at completion;
  - the IC clock;
  - one all-or-nothing draw of the order's goods from a hold;
  - EX.0's count of the draw's munitions into the hull's `loaded_kg`.
- **A ship docked at a port with a workshop is already a bay subject.** `facility_of`'s second
  case makes her her own workshop subject, drawing on her own hold.
- **PL.3's goods** name the ammunition they load: `torpedo` (one round a unit), `spinal_round`
  (one) and `pd_box` (500 rounds a box). The space mount rows name no ammunition.
- **Stock and asks.** Stickney Yards' storefront stocks all three, listed in the yard board's
  asks, and a listing holds its quantity. Ceres' workshop operator (Occator) stocks none.

**Built.**

- **`port/bay_work/magazines.rs`** (new), the space part:
  - each family's ammunition, as constants: the twin tubes `tacsim.ammunition.torpedo`, the
    spinal gun `tacsim.ammunition.spinal_round`, the rotary `tacsim.ammunition.pd_rotary`;
  - a unit's magazines: her fit's stows, read against her kept record;
  - the quote's jobs; the hold-then-stores split and its price; the load;
  - the op that carries the record at completion;
  - the quote's and the reply's words.
- **`BayJob::Magazine { mount, rounds, good, units }`**, a new job kind under the selector word
  `rearm`, a rearm and not a repair, so a lessee may order it.
  - **The quote:** for each magazine short of her fit, the whole units of its good that fit in
    what it is short. The labour is `cost_per_ammo_round` a round and `duration_ic_rearm_component`
    a mount, as the ground's.
  - **Whole units only, so rounds are conserved.** A box is 500 rounds, so a rotary short by less
    than a box waits. The ground's rearm rounds a short magazine up to a whole unit and loads only
    what fits.
- **Where the rounds come from** (at the take):
  - her hold first, as many units as are free there;
  - the shortfall from the workshop's stores, the operator's storefront, as a replaced part is
    supplied. Its price is the good's catalogue price (`reference_major`), on the job's
    `part_minor`, so the escrowed fee holds it beside the labour.
  - Too few anywhere is refused before any escrow (`magazines::SHORT_OF_ROUNDS`).
- **The draw** (`OwedKind::Draw`) gains `store` and `store_at`: the bought units, debited from the
  storefront in the same draw as the hold's, all or nothing. `BayOrder` gains `store_goods` and
  `store_vault`. All four are left out of a document while empty, so every order written before
  reads as it did.
- **Completion:** the bay's condition plan (`ic_condition_plan`) carries cells and conditions
  only, so the magazine record rides as its own op beside it (`magazines::record_op`).
- **The census:** a landed draw counts the bought units' kilograms into `loaded_kg` with the
  hold's.
- **`space/custody.rs`:** `magazine_op` is now one function, used by check-in and the bay.
- **The surfaces:**
  - `+bay <ship>` lists each magazine's rearm and a line for where its rounds come from: what
    comes from her hold, what is bought, and its price;
  - the order's reply names what is bought;
  - `BAY_HELP` gains the ship case, and `docs/help/space.md`'s "Magazines" says how to rearm;
  - the bot's `+bay` reader (`mush-bot/src/mine/salvage.rs`) gains `Answer::ShortOfRounds`, its
    needle held to tacsim's constant at its widest fill, 101 columns.

**Tests.**

Runs were on conclav.dev, with the patch applied on `73d05678b`.

- **New stories,** `tests/domain_field/cc7b_berth_rearm.rs`, at Ceres through
  `+bay/order <ship>=rearm`. Each fails on main, where a ship's quote has no rearm.
  - **A Dray's point defence:** one rotary is short three boxes and the other less than one. One
    box comes from her hold and two are bought.
    - The fee is the labour plus two boxes.
    - The rotary rearms full, and the short one waits.
    - Her hold ends with 0 boxes and the stores with 3.
    - The census's `loaded` moves by three boxes' kilograms, and its balance holds.
  - **A corvette's tubes and spinal gun** (her unit refitted to the corvette): four torpedoes come
    from her hold, and two torpedoes and four spinal rounds are bought.
    - The fee is the labour plus the rounds bought.
    - Every magazine ends full and her record is dropped.
    - The census's `loaded` moves by their kilograms, and its balance holds.
  - **Too few anywhere:** refused with `SHORT_OF_ROUNDS` before any escrow, and nothing moves.
- **Suites, all passing:**
  - `tacsim`'s lib tests with `domain_field`, `domain_economy` and `domain_world`: 3,621 tests;
  - `mush-bot`'s lib tests (the reader's contract check among them): 547 tests.
- **One pin re-pinned, by record:** `mill::sm12a_only_the_clerk_the_foundry_and_the_mill_create_matter`
  counts `.debit(` in `port/bay_work/act.rs`, and the count moves from one to two: the draw's
  stores debit.
- **Guards, all passing:**
  - fmt and clippy `-D warnings` on `tacsim` and `mush-bot`;
  - the M7 memory-size pass;
  - `ci_policy`'s guards and the help lint;
  - rustdoc in both modes.

**Divergences.**

- **`port/bay_work` is extended, not a file of its own** (the lead's ruling): `jobs.rs` (the job,
  the quote, the work), `mod.rs` (the take's split, the draw, the census, the completion's op),
  `act.rs` (the draw's second source), `record.rs` (the order's two fields) and `command.rs` (the
  quote, the reply, the help). The `.debit(` pin in `tests/domain_economy/mill.rs` and the bot's
  reader (`mush-bot/src/mine/salvage.rs`) move with them. `space/custody.rs` gives up its record op to one function.
  EX.3b may touch `mod.rs` for scrap; these changes stay on the rearm paths.
- **The ammunition is named in `magazines.rs`, by family.** The space mount schema has no
  ammunition field, and the space packs are CC.8's.
- **Whole units**, not the ground's round-up (above).
- **No hold on the bought units.** A replaced part's lot is held from the take to the draw. The
  bought munitions are fungible and are not held: if the stores sell out meanwhile, the draw is
  refused, and after its tries the order is cancelled and the fee released.

**Not built.**

- **The yard's listed stock.** The bay sells only free units. At Stickney Yards every space round
  is listed in the yard board's asks, which hold them, so the bay there sells none: a player buys
  them on the board into her hold (`+market`) and rearms from it. Selling listed stock through
  the bay needs the stock clerk to withdraw an ask's quantity on a claim, as it does for a part's
  lot (`port/stock.rs`).
- **A finding in the bay's completion.** `jobs::work`'s clean-up revokes
  `tacsim.space.subsystems` (a space hull's crits) when it repairs her. But `complete` writes the
  work through `ic_condition_plan`, which drops capability ops, so the revoke never lands, and a
  bay repair leaves her crits. (Part 1's survey above read the intent, not the landing.) The
  magazine record rides its own op for that reason.
- **`+coop/refit` still refills for free** (part 1).

## CC.8

The space palette and doctrines (Q-49-7, Q-49-10, Q-49-19, Q-49-24, Q-49-25, Q-49-26). Built
2026-11-16 on main `b4254e826`, with CC.6 (#627), CC.9 (#634) and CC.7's first part (#640) merged.

**Survey.**

- **What a lone shooter did to a screen, on the values CC.6 carried** (measured, see "The screen
  measurement" below): every stock screen stopped all eight torpedoes of a lone corvette's
  magazine, at 60, 150 and 300 km standing off and on a 6 km/s closing pass. Two rotaries alone
  stopped both rounds of every salvo, so no defence cycle could make a hauler's screen leak while
  their 15 km envelope stood. That is doc 48b's W-49-4, confirmed.
- **The cycles.** One cooldown served both of a laser's modes: a switch changed the mode and
  nothing else, so a laser idle in defence fired the moment it was switched to offence.
- **The doctrines** fired as §3.4 (corrected) says: the picket its laser, the interdictor its laser
  and at most one torpedo a check-out, the raider her opener and then lasers and her spinal gun.
  Every one of them leaned on a 500 km, 110-a-pulse laser on a 1 s cycle.
- **The stories leaned on it too.** Doc 35's SF.R stories in `voyage.rs` had a chartered Dray
  beam the interdictor to a derelict in four seconds, and the picket's corvette laser the raider
  to one in twenty-four. On the ruled palette no story hull can do either; see Divergences.
- **The geometry doc 47's stories fly.** The raider ambushes from 120 km, closing at 20 m/s
  (`content/space/picket_cast.json`), and AP.9's and CO.2's stories assert that the Dray's
  answer *arrives* on her. That sets the offence laser's reach (Divergences).
- **`speed(n)` in space is a standing burn**, not a speed goal: after her opener the raider
  burned flat out for the rest of the fight, past 1 km/s, so she never held a course.

**Built.**

- **The palette** (`content/space/mounts.json`, `fits.json`, re-signed; `DIGESTS` re-pinned):

  | Mount | Cycle | Reach | What it does | Note |
  |---|---|---|---|---|
  | Twin torpedo tubes | 8 s reload, a salvo of two | to 500 km | 180 a round | as before; magazines 8 (16 on Halcyon), persisting (CC.7) |
  | Turret laser, offence | **5 s** (the floor, Q-49-19) | **150 km** | **30** close in, 15 at 25 km, 7 at 45 km, 1 past 100 km | was 110 at 1 s to 500 km |
  | Customs corvette's heavier laser, offence | **5 s** | **100 km** | **45** close in, 22 at 25 km, 2 at 100 km | was 180 at 1 s to 160 km |
  | Either laser, defence | **4 s, automatic** (provisional) | 45 km | stops a torpedo | Q-49-10, Q-49-24 |
  | Spinal gun (and the raider's slower one) | **6 s** | 250 km | 145 a round | 24 rounds (32 on Halcyon) |
  | Point-defence rotary | 0.5 s, automatic | **4 km** (provisional) | two bursts stop a torpedo | was 15 km |

  Every laser stays dual-mode (Q-49-25). The Dray and Dray-P start both in offence, to answer
  fire; the corvette, the interdictor and now the belt raider start their second in defence; Halcyon
  starts both in offence and screens with four rotaries.
- **The cycle per mode and the switch rules** (`space/bubble.rs`'s `set_laser_mode`, through
  `LaserMount::switch_mode` and `cool` in `space/weapons.rs`):
  - each mode fires on its own cycle: an offence pulse starts the 5 s cycle, a torpedo stopped in
    defence starts the defence cycle;
  - a switch never fires early: the new mode's cycle starts from the switch, so a laser switched
    to defence stops nothing for 4 s and one switched to offence waits 5 s;
  - her offence cycle keeps running behind a switch to defence (`offence_left_ms`, absent from her
    document when nought), so switching back within one offence cycle keeps what was left of it,
    neither a fresh cycle nor none: a switch is no way round the floor;
  - the fit names each laser's modes, and a mode it does not allow is refused (CC.6's rule, kept).
  - `fire_picture`'s states needed no change: they read the cycle left in her current mode.
- **The three doctrines, rewritten** (`content/minds/`; `picket_cast.json`'s three digests moved
  with them). Each reads `space_mount(v, id)` and fires only what can land:
  - a spinal round only at a hull that has held her course since the doctrine's last think (it
    remembers her velocity), since a hull that burns is not where the round was aimed;
  - a laser in offence only while her lock lies within its reach (`lock_damage` is not `()`);
  - a salvo of two torpedoes at each reload, at a hull whose screen lets them through.

  Per doctrine:
  - **Kirkwood picket:** the spinal gun and the laser at a hostile (the raider), where it fired
    its laser alone; its tubes only at a hull that ran the picket, a hauler, since the raider's
    warship's screen stops a lone salvo.
  - **Interdictor:** the laser and the spinal gun at any hull that will not heave to, and the
    tubes only at a hull radiating for a fight. It closes until it is closing at 50 m/s, then
    holds its drive: `speed(n)` left standing carried it past her ever faster, and the closing
    speed it piled up left no Dray able to heave to. Its hail and heave-to logic is unchanged,
    and so is what MW.9's and MW.10's stories read (it closes on a hull that will not stop).
  - **Belt raider:** her opener as before; then her torch dies (`hold()`) and she presses from
    where she is. She prefers a prey radiating for a fight to one that is not, among those customs
    have finished with, and presses only the former, as before.
- **Help:** `docs/help/space.md`, `+tac/beam` and "Reading your mounts": the reach and the falloff
  in words, the per-mode cycles, the switch rules, the rotary's envelope.

**The screen measurement** (Q-49-26). A lone corvette fires her whole tube magazine, a salvo of
two at each reload, at each stock hull as her fit lays her screen ("as fitted"), and again with
every laser in defence ("braced"). Ranges: 30, 60, 150 and 300 km standing off, and a pass from
200 km closing at 6 km/s. The rig that printed this table and swept the defence cycle and the
envelope was a one-off and is not kept; `cc8_a_warships_screen_stops_a_lone_salvo_and_a_haulers_leaks`
holds its outcomes at 150 km, 300 km and on the closing pass.

| Stock screen | As fitted, 30 and 60 km | As fitted, 150 km, 300 km and the closing pass | Braced |
|---|---|---|---|
| Dray, Dray-P (haulers) | stops all eight | **lets one round of each salvo through**: four of eight land, and the fourth disables her | stops all eight; she has nothing left to answer with |
| Customs corvette, interdictor, belt raider | stops all eight | stops all eight | stops all eight |
| Halcyon | stops all eight | stops all eight (four rotaries) | stops all eight |

- **The defence cycle, 4 s, provisional.** Swept at 3, 4, 5 and 6 s: at 3 s and 4 s every warship's
  screen stops a lone salvo at every range measured; at 5 s and 6 s a warship lets one round
  through on the closing pass. 4 s is the longest that holds.
- **The rotary's envelope, 4 km, provisional.** At 15 km a hauler's two rotaries stop everything
  whatever the defence cycle, which is the review's finding. At 5 km a hauler leaks only at
  300 km; at 3 km and 4 km she leaks from 150 km out. Close in her screen holds, since slow rounds
  give the rotaries time.
- **The spinal gun's dodge stays, priced in her change of speed** (24 rounds at a Dray): a Dray
  that coasts takes every round at every range; one that burns across the line while a round
  flies dodges all 24 from 60 km out (none at 30 km), and spends 71 m/s (at 60 km) to 107 m/s of
  her change of speed doing it.
- **The offence laser** at the raider's ambush range of 120 km lands a point a pulse: the Dray's
  answer arrives, and does little.
- **Decoys are not counted.** A decoy is the pilot's to launch (`+tac/decoy`) and spoofs one round
  each: a Dray carries four.

**The story** (`space::tick::thinks_tests::cc8_a_corvette_disables_a_dray_that_dodges`): the
interdictor, flown by its shipped doctrine, takes a Dray 160 km off who is radiating for a fight
and will not heave to. Her pilot burns across the line of every spinal round in flight at her.
Every round is dodged, each dodge costs her change of speed, and the doctrine fires only when she
has held her course; her hauler's screen lets the torpedoes through, and she is disabled. The
interdictor takes no harm.

**Tests.**

All runs were on conclav.dev, in a throwaway clone, with the unit's patch applied on main
`73d05678b` (main `b4254e826` merged forward with PL.3, PL.4, EX.0 and EX.2).

- **New tests** (`space::bubble::cc8_tests`, `src/space/bubble_cc8_tests.rs`):
  - `cc8_every_shipped_fit_flies_the_ruled_palette`: every fit's cycles (8, 6, 5, 4 and 0.5 s), the
    spinal magazines, every laser dual-mode, and the start modes;
  - `cc8_an_offence_laser_is_short_and_weak_against_a_hull`;
  - `cc8_a_switch_starts_the_new_modes_cycle_and_never_fires_early`,
    `cc8_switching_back_within_one_offence_cycle_keeps_the_offence_cooldown`,
    `cc8_each_mode_fires_on_its_own_cycle`;
  - `cc8_a_warships_screen_stops_a_lone_salvo_and_a_haulers_leaks`: the measurement's outcomes,
    held at 150 km, 300 km and on the closing pass;
  - `cc8_the_spinal_guns_dodge_stays_and_costs_her_change_of_speed`.
- **The story**: `space::tick::thinks_tests::cc8_a_corvette_disables_a_dray_that_dodges`.
- **Retired**: `space::custody::content_tests::cc6_every_shipped_fit_carries_the_rust_tables_values`,
  for `cc6_a_hull_with_no_fit_of_her_own_flies_the_drays`.
- **Suites, all passing:**
  - `cargo nextest run -P ci -p tacsim`: 4,795 tests. Doc 47's space stories are among them:
    `space_ap9.rs`, `field_ap11.rs`, the `tick_thinks_tests.rs` return-fire tests, MW.9, MW.11,
    MW.13 and MW.21, and every `voyage.rs` story;
  - the bot job (`cargo nextest run -P ci -p mush-bot`, less its two long stories): 796 tests.
- **Guards, all passing**:
  - `cargo fmt -p tacsim --check` and `cargo clippy -p tacsim --all-targets -- -D warnings`;
  - the lint job's M7 memory-size pass;
  - `mush-contract-tests`' `ci_policy` guards: census, panic budget, restriction lints and the
    fiat budget;
  - `engine_contracts` (the help lint and the space story lints);
  - rustdoc with `-D warnings`, in both modes.
- **Pins moved, by record**:
  - the two space pack signatures in `DIGESTS`;
  - the three doctrine digests in `picket_cast.json`;
  - the story re-pins in the table above.
- **The arena** was not touched. One `domain_arena` cockpit test failed once, under load, and
  passed on its own and in the final run.

**Divergences.**

- **The rotary's envelope moved**, beyond the defence cycle Q-49-26 names: measured, no defence
  cycle can make a hauler's screen leak while two rotaries alone stop every salvo (above).
- **The offence laser's effective reach is 25 km**: inside it a pulse does at least half its
  close-range damage (15 of 30 on the turret, 22 of 45 on the corvette's heavier laser). Past it a
  pulse still lands out to a full reach of 150 km (100 km on the heavier laser), but as a token:
  a point at 120 km. The reason is gameplay. A Dray's answer to a raider ambushing from 120 km is
  a token, but a real one: R-47-2 and R-47-3 hold, and the raider knows she has been answered.
  Every hurt at range comes from the tubes and the spinal gun, which the rewritten doctrines use
  (Q-49-26). The lead accepted this reading of Q-49-7 (a short reach, little to a hull) and put it
  to the owner, who may veto it. No story asserts more than a token answer at range.
- **The belt raider's second laser starts in defence**, so her screen is a warship's.
- **Halcyon's spinal gun stows 32**, its own fit's figure, beside the palette's 24.
- **A laser's `cooldown_remaining_ms` is left out of her document when nought.** The per-mode
  cycle put `defence_cooldown_ms` on every laser in every copy of a bubble's saved state, and a
  ten-minute heave-to's document grew past the 64 KiB budget `voyage.rs`'s sfr10 holds it to
  (65,891 B). Leaving out an idle laser's cooldown wins the bytes back; it reads back as nought.
  A binary from before this unit cannot read a laser saved without it (pre-launch: no document
  needs it).
- **The doctrines changed behaviour beyond the lasers**, each for a reason the stories showed:
  - the raider's torch dies after her opener: a raider that burns flat out can never be hurt by a
    lone corvette, since the dodge stays;
  - the raider prefers a fighting prey: once a quiet carrier was the nearest cleared, she pressed
    nobody, and an escort she had opened on sailed away (sfr18);
  - the picket keeps its tubes for a runner: its salvos at the raider were all stopped, and their
    thirty-second flights kept every docking under fire long after the raider was down;
  - the interdictor caps its closing speed at 50 m/s.

  The picket's corvette now disables the raider in about fifty seconds with spinal rounds, where
  it took about twenty-four with the old laser.
- **Doc 35's SF.R stories changed how they reach their precondition**, because no story hull can
  now make an interdictor derelict by play (measured: four minutes of close laser fight leave a
  jinking Dray and the interdictor both standing), and the story lint forbids writing bubble state
  off the command path. Each story still makes its doc 35 assertion at full strength, but one
  (sfr7, below). Every precondition is reached by play, with no staff fiat a story did not already
  have. The helpers are `heave_to_until_stood_down` (MW.9's recipe: burn against her drift until
  the interdictor stands her down), and `picket_until_the_raider_is_down` /
  `convoy_until_the_raider_is_down` (the stocked lane; clear the Kirkwood picket's customs; the
  corvette's spinal rounds disable the raider once she coasts). In a picket's bubble the raider
  holds the bubble's interdictor slot, so `interdictor_hull` reads her.

  | Story | What it tests | Old precondition | New precondition | Re-pinned |
  |---|---|---|---|---|
  | sfr1 | promotion, the paused leg timer, the wall-clock bubble, resume to dock | a Dray beams the interdictor to a derelict in 4 s | she fires on it, then heaves to until it stands her down | `steps` and `wall_elapsed_ms`: the 4 one-second steps and the capped 30 s step, plus the seconds she spent heaving to, read from the run |
  | sfr2 (1) | custody check-out, damage persisted on check-in, the NPC derelict's open epoch and no claim before `+helm/tow` | the interdictor's volleys hurt her; her beams disable it | stocked lane: the raider's opening round finds her stern; the corvette disables the raider | the derelict's life support read as its 48 hours less the seconds the bubble ran on |
  | sfr2 (3) | a carrier disabled in a bubble checks in `Immobile` with a 48-hour window | the interdictor disables her in 16 s | the same, stern on, so her drive goes before her hull | the window, 16 → 120 s |
  | sfr6 | the picket triad end to end, the dodge, the fine | the corvette lasers the raider down in 24 s | the corvette's spinal rounds, once the raider coasts | the window, 24 s → until the bubble closes |
  | sfr7 | the docking session, forged tapes, the bounce, clamp, tow, dock at the port | a derelict interdictor 18 km off | stocked lane, the raider's derelict; the far `+helm/dock` is tried once the corvette's last rounds are done | the approach, 3 ticks → until flown; **one assertion changed**: "the bubble does not step while she docks" became "she is held, her drive cut, and the bubble stays open" (SF.R25: a picket's bubble keeps its 1 s cadence for the corvette; the interdiction bubble had gone to its cruise cadence). Put to the lead |
  | sfr9 | the docking and `+helm/run` gates; a derelict carrier's journey fails | beams disable the interdictor; it disables a Dray in 16 s | heave to; the same Dray, given 120 s | the window, 16 → 120 s |
  | sfr11 | the one propellant ledger, no refill on re-entry | beams disable the interdictor | heave to | none (its figures are read from the run) |
  | sfr14 | authored packs, picket only at Kirkwood, drone minds | beams end the 2-ship interdiction | heave to | none |
  | sfr15 (2) | a stripped rotary opens a gap in the screen | torpedoes 5 km out, inside the 15 km envelope | 3.5 km out, inside the 4 km envelope | the start distance |
  | sfr15 (3) | rear damage, clamp, tow, dock, bay repair to full | the interdictor's volleys; its derelict | stocked lane: the raider's opening round on her stern; the raider's derelict | the damage line, `drive 172/250, hull 524/600` → `drive 207/250, hull 558/600`, by record |
  | sfr17 (C) | a 30 s late tick; one carrier's salvage hold does not hold the other; the hold expires | beams disable the interdictor | a two-carrier convoy at the picket; the raider's derelict | the second carrier resumes once the corvette's last rounds are done: next tick → within 20 |
  | sfr18 | per-carrier inspection, a disabled escort settled, stand-down | the raider's lasers disable the escort | unchanged; the raider now presses the escort, who shows fight, before the quiet smuggler | none |
  | sfr20 (5) | the rear split, a stripped rotary persisting through check-in and check-out | ticks timed to 1 s volleys; beams end it | bow on until a rotary is stripped, stern on until the drive is hurt; then heave to | the tick counts, now read from the fight |
  | sfr21 | prepared bubble input, standing orders, the per-entity store, the approach debit, an `OutOfGate` session | beams disable the interdictor | stocked lane; she shows herself again and clears customs; the raider's derelict | none |

  The picket's own doctrine test (`space::picket::tests`) reads the new doctrine: the corvette no
  longer fires torpedoes at the raider.

  **Runtime.** The fights are stepped a wall second at a time while they last; each helper stops
  when its condition holds, and no story strides through an idle wait. Measured on conclav.dev,
  the 37 SF.R, MW.9, MW.10, CR.6 and RS.8 stories ran 286 s on main and 320 s on this branch.
  The branch run shared the box with 350 other tests: stories this unit did not touch (sfr13,
  sfr26b) rose by as much as the rewritten ones (about a second each), so part of the 34 s is load.
- **Files outside the row**, each the least this needed. The lead approved them, and the SF.R
  stories' change of premise:
  - `space/weapons.rs`: the per-mode cycle's state and the switch;
  - `space/custody.rs`: CC.6's `table_before_cc6` test retired, as CC.6's hand-over asked;
  - `space/view.rs`, `space/picket.rs`: a test each whose premise the palette moved;
  - `space/tick_thinks_tests.rs`: the story;
  - `content/space/picket_cast.json`: the three digests;
  - `tests/domain_arena/content.rs`: the two pack signatures;
  - `tests/domain_world/voyage.rs`: the SF.R stories above;
  - `docs/help/space.md`.

**Not built.**

- **Firing one laser alone.** Naming any laser still fires every ready laser in offence, as one
  battery (CC.6's divergence).
- **A mind cannot switch a laser's mode**: the mind API has no verb for it, so the switch rules
  bind players (`+tac/pdc point|offense`). The house default's Dray answers in offence.
- **Collector drones are not soft.** A collector drone is a Dray's hull section (600 internals), so
  an offence laser takes minutes to bring one down, though Q-49-7 makes lasers the drones' answer.
  That is mining content.
- **The Δv price of a dodge is real but small**: these hulls carry far more change of speed than
  any fight spends (a Dray's, on her 6,000 km/s exhaust, runs past a thousand kilometres a
  second), so a dodge's few metres a
  second never bind. A raider that burned flat out could not be hurt by a lone corvette at all;
  the doctrine, not the price, keeps her from it. Pricing the dodge so it binds is the owner's
  call, and not this unit's.
- **CC.7's second part** (the berth rearm) is untouched.

## CC.10

The bot (Q-49-17). Built 2026-11-17 on CC.2's tip (`929ca6790`, PR #649). Once #649 and CC.2b
(#651) had landed, it was rebased onto main `c847b5dd2`; the runs below are at `929ca6790`. It
changes only `crates/mush-bot/` and the bot's guide (`docs/bot/guide.md`). No game crate, content
or pin moves.

**Survey.**
- **The fire gate** (`pilot.rs`, `fireable`) fired every gun that was ready, had rounds left and
  did not read 0%. So it fired at any chance over nothing, down to the last round: the full-rate
  client Q-49-17 names.
- **The bot cannot read content.** It is an outside client (`crates/mush-bot/AGENTS.md`: game
  crates only in its tests). So "from content" means two things:
  - what its own screens tell it: each gun's cycle, learned from the `cycling (N s)` row read
    just after its own `FIRE`, and each gun's magazine, from its `WEAPONSTATUS` row;
  - what the help says: `help minds` gives the Line card's discipline in words ("fires at 35% or
    better on any face, 50% once a gun is under half its magazine. It keeps a quarter of each
    gun's rounds back, and spends them only to answer a unit that has fired on it, or to cover
    its way home").
- **The windows.** Cut AN.2's exchange (`EXCHANGE_WINDOW_MS`, 20 s) and the halt's longest
  (`HALT_MAX_MS`, 60 s) were literals tuned on the 6 s cannon and the 1 s coaxial gun.
  - Under CC.2's palette, 20 s holds about one cannon round and four of the coaxial gun's.
    `EXCHANGE_MIN_TAKEN`'s eight hits could not be reached.
  - `HALT_STILL_MS` reads the scope's 50 m rounding against a machine's speed. No gun moves it.
- **The lane's poll reservation** (`lane.rs`, `reserve_ok`) kept a line free for every gun "due",
  using the old gate's test. A gun the new gate holds is due at every tick, so it would have held
  back every poll that can wait.
- **The commander's system prompt** told the model that the pilot fires every ready gun on its
  cycle.

**Built.**
- **The discipline** (`pilot.rs`, `disciplined`; Q-49-17 with CC.1's card):
  - `FLOOR_PERCENT` (35), and `LOW_MAGAZINE_FLOOR_PERCENT` (50) once a gun holds under half its
    magazine (`floor_percent`).
  - `RESERVE_PERCENT` (25), rounded down as the card's `keeps` rounds (`reserve_rounds`).
  - A gun fires only at its floor or better, and only over its reserve. Below the reserve it
    fires only when the lock is a unit the pilot blames for a hit taken this match, or while its
    own fall back or a `withdraw` flies. Then any round left goes, still at the floor.
  - **The chance must be read under the lock it holds**: the gun's own row since the last lock
    line, or for the main gun the lock line's own figure (the game's lock line prints the chance
    of the gun a bare `FIRE` fires). With no chance read, the gun waits. The old gate fired on a
    chance it had not read.
- **Answering fire** (`Pilot::fired_on`, `take_fired_on`, `blamed`). The hit line says nothing of
  who fired, so the blame uses the threat turn's reading:
  - the nearest live hostile, held young, whose row's relative bearing lies in the struck facet's
    45-degree face (front included);
  - else the lock.

  The blame follows a rename (`was`). It is kept for the match, as the game's `fired_on_by` is.
- **Covering** (`Pilot::covering`): the pilot's own fall back (`withdraw_until_ms`) or a doctrine
  `withdraw`, read at each decision.
- **The windows, counted in the main gun's learned cycle** (`Pilot::main_cycle_ms`: the cannon's,
  else the first gun's):
  - `EXCHANGE_WINDOW_CYCLES` (4) replaces `EXCHANGE_WINDOW_MS`. It is 60 s on the ruled 15 s
    cannon and 24 s on the old 6 s one. It is tunable 2..=5, which spans the old 10 to 30 s.
  - `HALT_MAX_CYCLES` (10) replaces `HALT_MAX_MS`. It is exactly the old 60 s on the 6 s cannon,
    and 150 s now.
  - Until the main gun's cycle is learned, the exchange reads `(0, 0)` and is not judged, and the
    halt has no cap of its own. The halt still ends on its other three ends (a hit, the band, its
    hostile stopped coming on), each of which comes.
  - `EXCHANGE_MIN_TAKEN` and `EXCHANGE_MARGIN` stay; see **Not built**.
- **The lane's reservation** keeps a line only for a gun `Pilot::may_fire`, the same predicate
  the pilot fires on.
- **Text.**
  - The pilot's module head gives the discipline among the help sentences it flies on.
  - The commander's prompt (`COMMANDER_POLICY`) states the floor and the reserve where it said
    "fires every ready" gun.
  - The bot's guide gives the discipline, the halt's cap and the exchange's window in cycles.
    Its example note now reads `… in 60 s`.

**Tests.**
- **New, in `pilot.rs`:** `cc10_a_gun_fires_only_at_its_floor_and_keeps_its_reserve`. It checks,
  through the names:
  - the floor and its rise;
  - the reserve, and one round over it;
  - answering a hit on the front face, and following a rename;
  - a hit from the other arc, which blames the hostile there and not the lock;
  - covering under a `withdraw` and under the fall back;
  - the chance read under the lock, the lock line standing in for the main gun only;
  - `may_fire`.
- **Changed, in `pilot.rs`:**
  - The halt test now also checks that the halt still runs one millisecond before ten cycles, and
    that with no cycle learned it has no cap of its own.
  - The exchange and halt tests count in a learned 15 s cannon (`learn_main`).
  - The test fixture `weapon()` is a gun clear of the discipline (30 of 40, 62% read). It was 10
    of 40 with no chance read.
- **New, in `tests/arena_lane.rs`**, on the scripted server with a new `chance` knob (62% unless
  set):
  - `cc10_a_gun_under_its_floor_never_fires_and_one_at_it_does`: at 34% for two cannon cycles,
    no `FIRE` and no round spent; at 35%, both guns fire;
  - `cc10_a_magazine_is_fired_to_its_floor_and_its_reserve_and_no_further`: eight rounds a gun.
    At 35% each gun stops at half its magazine. At 50% it fires on to its reserve and keeps it for
    four cycles more. Nothing is ever refused as empty.
  - `cc10_the_reserve_answers_the_unit_that_fired_on_the_tank`: fired down to its reserve, the
    tank holds for two cycles. Then a hit on its left face, where BA's row stands, and both guns
    fire their reserve to the last round.
- **Changed, in `tests/arena_lane.rs`:**
  - The two re-approach stories read the window from the lane's own pilot (`exchange_window`).
    The helper asserts that window is the shipped cannon's cycle times `EXCHANGE_WINDOW_CYCLES`.
  - The crowded minute's own copy of the reservation rule calls `Pilot::may_fire`.
  - `full_rate`'s doc says what it measures now: the clock, with both guns kept clear of the
    discipline.
  - **Three scenes counted in the cannon's cycles**, which failed on the first run. The coaxial
    gun's first shot now waits for a chance read under the lock, so both cycles are learned at
    12 s, not 26 s, and every later coincidence moves. Under CC.2's palette a gun is due inside a
    given second about a fifth as often as with the 1 s coaxial gun these scenes were tuned on.
    Each span was measured on conclav.dev before it was set:
    - **The crowded minute** (`a_due_gun_keeps_its_slot_from_a_poll`) asserts that a `STATUS`
      was held back for a due gun.
      - Over 60 s, no notice phase from 0 to 990 ms held one back, and no start delay from 0 to
        30 s did either.
      - Over 120 s, one delay of twelve did. Over 180 s, every delay did.
      - The scene now runs `DUE_SCENE_CYCLES` (12) of the cannon's cycles, 180 s. That is 72 s
        on the 6 s cannon the minute was tuned on.
      - The machine gun's floor scales with the span. The reservation's own bound keeps its
        tuned figure, at least one held tick per 250 of a minute (5).
    - **The doubt's search** (`the_doubts_weaponstatus_skips_the_poll_gap`) found its moment at
      180 s and 240 s, and not at 60, 90 or 120 s. It now searches the same twelve cycles.
    - **The lattice rigs** (`the_lattice_at_the_live_edge`) run `LATTICE_SPAN_CYCLES` (20) of
      the cannon's cycles: 120 s on the 6 s cannon, 300 s now. At 120 s the ruled cannon fired
      eight lines, so one refusal broke the one-in-ten bound: on read 1 of 8 where off read 2
      of 9. At 300 s, on reads 1 of 20 against off's 3 of 22, and the machine gun 2 of 56
      against 9 of 63.

      The same test at CC.2's base read 0 of 7. So the bound held there by a margin of one
      line.
- **New, in `tests/arena_contract.rs`:** `the_pilots_floor_and_reserve_are_the_line_cards`. It
  holds the bot's figures to `tacsim::arena::card`'s Line card: `LINE_MIN_ODDS_PERMILLE`,
  `low_magazine_floor`, `LINE_RESERVE_PCT`. It also checks `floor_for` and `keeps` against the
  bot's `floor_percent` and `reserve_rounds` for every magazine to 600 and every round in it.
- **New, in `commander.rs`:** `the_policy_names_the_floor_and_the_reserve`.
- **Mutations** (on conclav.dev). Each ruling's production line was put back the old way, one
  at a time, and the new and changed tests were run against it. All passed unmutated, and every
  mutation failed at least one:

  | Mutation (the line put back) | Failed |
  |---|---|
  | No floor (any chance over nothing) | the pilot test; the lane's floor test and magazine test |
  | No rise under half a magazine | the pilot test; the contract test; the lane's magazine test |
  | No reserve | the pilot test; the lane's magazine test and answering test |
  | The reserve never answers fire | the pilot test; the lane's answering test |
  | The reserve never covers a withdrawal | the pilot test |
  | A chance read before the lock line counts | the pilot test |
  | The halt's longest a literal 60 s | the halt test |
  | The exchange's window a literal 20 s | the exchange-count test; both lane re-approach stories |
- **Runs**, on conclav.dev, each in a throwaway clone of `929ca6790` with the unit's patch:
  - **The first round** found the three scenes above failing (799 of 802 in the bot job). It also
    ran the guards and the mutation pass.
  - **The final patch:**
    - the bot job (`mush-bot` without `mining_cohort` and `arena_mind_isolation`, as CI's bot
      shard runs it): 802 of 802;
    - `arena_mind_isolation` (CI's bot-stories shard): passed in the first round;
    - the doc tests: passed;
    - `mush-contract-tests` whole: 200, with the census, the panic budget, the restriction lints
      and the help-page lint among them;
    - `cargo fmt -p mush-bot --check`, `cargo clippy -p mush-bot --all-targets -- -D warnings`,
      and `cargo doc -p mush-bot --no-deps --document-private-items` with `-D warnings`: clean;
    - the CI lint job's M7 memory-size pass: clean. It covers six game crates and no file this
      unit changed.
  - **MW.19's minds lane** (`scripts/bot-minds-lane.sh`, the nightly `bot-minds-lane` job):
    passed in 3,763 s on the final patch. The rating took 2,875 s on a box other sessions were
    loading (MW.19 measured about 25 minutes), and sent the author no line it had not asked for.
    It drives `src/mine/`, which this unit does not touch; it shares only `arena::lines`, which
    this unit leaves unchanged.
  - `mining_cohort` was not run. It flies the mine module only, and CI's bot-stories shard runs
    it.

**Divergences.**
- **The windows count in the main gun's cycle, not the fastest gun's.**
  - Twenty of the fastest gun's cycles would also have given back the old 20 s exactly.
  - But the pilot learns a gun's cycle only by firing it, and the floor keeps the coaxial gun
    quiet at range (CC.2's mind fired 1.3 coaxial rounds a fight at 35%).
  - Keyed to the fastest gun it had learned, the window would have been 300 s of the cannon most
    of the time.
- **The blame is a reading, not the game's record.** The game's `fired_on_by` counts every unit
  whose round reached the hull. The bot reads one hostile per decision off the newest hit's face.
  Two shooters in one face between two decisions blame the nearer one.
- **Covering is the bot's two withdrawals.** The re-approach's and the return's back-offs are
  tactics, not the way home, and keep the reserve.
- **No dry withdrawal.** Q-49-23's "at its reserve on every mount counts as dry for withdrawal" is
  the stock mind's edge. The bot's own withdrawal rule (a failing hull) is unchanged. A bot at its
  reserve holds it until it is fired on.

**Not built.**
- **The exchange's counts** (`EXCHANGE_MIN_TAKEN` 8, `EXCHANGE_MARGIN` 4) were tuned on the 1 s
  coaxial gun.
  - In 60 s a single disciplined machine lands about three cannon rounds, so a re-approach now
    needs two machines' fire.
  - The re-approach ships off (cut AO.3), so the default bot is unaffected.
  - Sizing the counts to the palette is a measured round's work, one constant at a time.
- **CC.2's channel-line finding is a rig artifact, not a lane defect.**
  - In the game, `Firing {w}.` is the cockpit's own reply, said as `FIRE` returns
    (`weapons_fire.rs`, after `forward_intent`). So one session's answers come back in the order
    it typed them.
  - The rig's `fire_answer_ms = 20` delays the `FIRE`'s answer alone, which lets a `RANGE` typed
    in the same tick be answered first. The ledger then pops the `FIRE` with it, as answers in
    order would. That is the only way the ack loses its place.
  - AM.1's test keeps skipping shared ticks, and the lane is unchanged.
- **The win-rate lanes** (`arena_win_rate`, `arena_muster_rate`; weekly and on dispatch,
  report-only) were not run. They fly this bot against the stock mind and would measure the
  discipline in fights.
