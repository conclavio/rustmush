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
2026-11-15 on main `c85a8744f` (CC.0 and CC.6 merged in).

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
- **The Rhai port** (`content/minds/standard.rhai`): still 31 functions, `api: 9`, `stock: 3`.
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
  `STANDARD_DOCTRINE_SHA256` are re-blessed from `2bea2391…7b5fef` to `fd73e14b…ed1c3b`.
  `MIND_ABI` stays `rhai=1.26.0;api=9;stock=3`. The mind view gains no field, so there is no
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
  | `minds/content.rs`, `domain_arena/content.rs` `m2_the_standard_doctrine_source_is_pinned`, `match_/seal.rs`, `support/api4_identity.rs` | the Standard source digest | `2bea2391…7b5fef` → `fd73e14b…ed1c3b` |
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

CC1_BOUND_PLACEHOLDER
- **Guards**, on the final tree:
  - `cargo fmt -p tacsim --check`;
  - `cargo clippy -p tacsim --all-targets -- -D warnings`;
  - the M7 memory-size pass;
  - `cargo doc -p tacsim --no-deps --document-private-items` with `-D warnings`.

  The contract tests (`engine_contracts`, `ci_policy`, the help-page lint) passed in the probe and
  re-ran in the final sweep.

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

Not built.

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

Not built.

## CC.7

Not built.

## CC.8

Not built.

## CC.10

Not built.
