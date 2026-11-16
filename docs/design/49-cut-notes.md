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

Not built.

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

Not built.

## CC.7

Not built.

## CC.8

Not built.

## CC.10

Not built.
