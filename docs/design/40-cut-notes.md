# 40. Cut notes — the field's second round

One `## FR.n` heading per cut of [40-field-second-round.md](40-field-second-round.md) §6 (`FR.0`–`FR.7`), in
the order they landed: what was built, the test that proves it, and where the PR departed from the plan's row.
Each note records its cell's summed-minutes delta from its PR run's JUnit ([doc 43](43-testing-strategy.md)
§2.3 rule #14).

## FR.0 — the inventory and the bot's words

Row: §6 FR.0; Q-38-29, Q-40-23, doc 38 §8. Base commit `535efcb6` (the inventory ran at `8d983ce2`, main when
it was dispatched: the one commit between is doc 43 Draft 0.4, docs only, so the code under test is the
base's).

### §4a's inventory, at the base commit

Two gate dispatches on conclav.dev (`scripts/vps-gate.sh --ref main --runner conclav-dev`), both green:

- **Run A** [41721272740](https://github.com/conclavio/rustmush/actions/runs/41721272740): `-p tacsim -p
  mush-contract-tests`, filter `package(mush-contract-tests) | (package(tacsim) & (twin:: | minds_rhai:: |
  reflex:: | minds:: | content:: | motor:: | field_gc2:: | field_gc4:: | field_gc10:: | field_story:: |
  m3_start_writes_one_seal_per_unit_and_the_live_record_carries_them | seal:: | snapshot:: | field::record::))`:
  **866 tests, 0 failed, 0 flaky, 1,978 s of test time**.
- **Run B** [41723958565](https://github.com/conclavio/rustmush/actions/runs/41723958565): `--run-ignored all`,
  `minds_rhai::m2_the_twin_corpus_hashes_the_same_on_every_host` by name, as ci.yml's `Cross-platform mind
  hashes` step runs it: **1 test, passed** (6 s). (A first dispatch with `test(=m2_the_twin…)` selected
  nothing — nextest names carry the module, `minds_rhai::` — and is not counted.)
- **Run C** [41724925720](https://github.com/conclavio/rustmush/actions/runs/41724925720): the three rows run A's filter missed (its `test(=m3_start…)` matched nothing for the same
  reason, and `arena::rng` and `shard_10/arena.rs` were outside it): **14 tests, 0 failed** (6 s; `arena::rng::tests::*` 10, the three snapshot tests, `match_lifecycle::m3_start_writes…`).

Each row of §4a, what it read at the base, and the test that holds it (every test named passed in run A, B or C):

| §4a row | At the base | Held by |
|---|---|---|
| `M4_5_POOLED_RECORD` (21 lines), `COVERAGE`, `POOLED_MATCHES_WITH_STAND_IN` | `COVERAGE` pooled `(10_295, 308)`, mixed `(13_230, 502)`, lance `(7_157, 130)`; `POOLED_MATCHES_WITH_STAND_IN` 12; the record as committed in `shard_2/twin.rs` | `twin::m4_5_the_stock_pool_over_fifteen_seeds_is_its_record` (9.1 s), `twin::m2_gate_a_over_the_pooled_lanes_matches`, `…_on_a_mixed_pool`, `…_over_the_authored_lance_field` and the other seven of `twin::` (11 tests, 80.4 s) |
| `M2_CROSS_PLATFORM_PINS` rows 0–3 | row 0 `(2350, 176250, 12465543414996090189, 20f549f1…, 6349d78a…)`; row 1 `(2351, 235250, 6776313200826027730, a019a970…, 1c1fd95e…)`; row 2 `(2352, 170000, 4593528509461668393, 469d2a99…, 6ff3a5a8…)`; row 3 the float probe `(2350, 900000, 10559587572615102858, 37d56f2b…, 4ea397ba…)` | run B (Linux, conclav.dev); Windows is ci.yml's same step on the PR |
| `DUEL_HASH_API6_OBSERVED`, `DUEL_HASH_AT_HEAD` | `16080728793682240297`; `0xd15b_cccc_3546_150f` | `motor::m1a1_the_shipped_duel_hashes_as_before`, `motor::gc1b_an_ooc_match_offered_a_join_keeps_its_practice_hashes` (of `motor::`'s 43, 41.4 s) |
| The (j) field's substep hashes and ledger lines | as committed in `shard_2/reflex.rs` | `reflex::m3_the_j_fields_sortie_lines_are_one_list_at_three_cadences`, `reflex::m1a1_the_moved_motor_folds_the_same_bytes` (of `reflex::`'s 85, 20.6 s) |
| `M2_TWO_T1_MEMORY_DIGESTS`, `M2_UNARMED_MEMORY_DIGESTS` | as committed in `shard_2/minds.rs` | `minds::m2_the_memory_codec_writes_the_native_bytes` (of the shard's `minds::` 90, 48.7 s) |
| The draw-label set | seven kinds: `hit`, `facet`, `burst`, `classify`, `contact`, `mind`, `component` | `arena_library::the_draw_label_set_is_pinned` (run A); tacsim's `arena::rng::tests::adding_a_label_shifts_no_existing_draw` (run C) |
| `DIGESTS` (14 packs), `POWER_STATES_SCHEMA_DIGEST` | 14 rows as committed; `7b126afa…` | `content::the_pinned_digests_are_the_shipped_ones`, `content::m3_every_shipped_chassis_binds_its_core` (the power-states digest) (of `content::`'s 37, 13.2 s) |
| `STANDARD_SHA256`, `API_VERSION`, the default seal identities | `3837c345…a316`; 7; as committed | `content::m2_the_standard_doctrine_source_is_pinned`, `match_::seal::m5_tests::m5rules_default_seal_identities_are_pinned` (run A); `match_lifecycle::m3_start_writes_one_seal_per_unit_and_the_live_record_carries_them` (run C) |
| `SNAPSHOT_FORMAT` | 19 | `arena::snapshot::tests::af1_*` (5, run A); `shard_10/arena.rs`'s three `assert_eq!(SNAPSHOT_FORMAT, 19)` tests, `m1a1_a_snapshot_carries_the_motors_and_refuses_a_format_two_document`, `m1a4_a_snapshot_carries_a_motor_tick_and_a_pilots_motor_row`, `m3_a_snapshot_carries_the_minds_with_their_pending_answers` (run C) |
| `FIELD_LIVE_FORMAT` | 1 | `field::record::tests::*` (3) |
| `ARENA_V4_DIGEST`, `OOC_V6_DIGEST`, `STAFF_LOBBY_V4_DIGEST`, `ATLAS_V1_DIGEST` | `fd5860b5…`; `64f6cf88…`; `8d791c31…`; `b0bcabf3…` | `content::` (the pack rows) |
| The field gates | GC.2: `gc2_an_idle_declared_field_demotes` (26.7 s), `gc2_an_unarmed_barge_does_not_hold_a_field_engaged` (31.8 s) — the file holds two tests carrying the four gates; GC.4 story (a) `gc4_the_wingman_drags_the_casualty_to_the_ramp_and_loads_it_before_the_lift` (203.5 s) and five more (834.4 s for six); GC.10 `gc10_a_leader_voids_friendly_fire_standing` (26.2 s, demotes on `idle_demote_s`), `gc10_two_rosters_in_one_organization_are_friends`; the field story at 256 `gc0_the_raiders_barge_lifts_and_the_field_demotes_without_a_friendly_mark` (237.0 s) and at 512 `s2_the_field_story_at_512` (183.0 s), with `cf7_…` and `s2_a_field_pulse_is_measured_at_both_sides` | run A |
| `field_scale` at 100 | not re-run: GC.12's hand run stands as the base figure (Engaged p99 6.375 ms at 100 against the 40 ms budget, doc 40 §4.7); FR.5 re-measures once after the batch | — |
| The contract guards | all 34 files of `mush-contract-tests` green (panic budget 10, restriction lints 11, feature matrix 3, state-slot owners 5, legacy-handler allowlist 5) | run A |

FR.5 reads this table: a pin it moves is re-pinned once with the reason, against the value recorded here.

### The four-digit addresses (Q-40-23)

`mine::lines::is_addr` took a block letter and 1–3 digits, so nothing past `P256` read: a 512 site's
sounding dropped every pocket numbered 1000 and over, and the lane's own vocabulary gate (`lane.rs`'s `+tv/1
drive <addr> go`) refused the drive. It now takes a letter `A`–`P` and a number 1 to `MAX_POCKET_NUMBER`
(1,024, `Scheme::WIDEST`'s `pockets_in_block`) with no leading zero, the game's own strict
`Scheme::parse_pocket_address` at the widest site (case aside: the game prints upper case). The one parser
serves the sounding (`sounding_pockets`), the drive's gate and `has_addr`'s arrival reading; doc 39's IN.1 and
doc 41's RS.0 add none (doc 39 Draft 0.5's rule: the first of FR.0 and RS.0 to land carries it for all three).
The gate `lines::tests::fr0_every_pocket_of_a_512_site_is_an_address` walks every pocket of both schemes
through `Scheme::pocket_address` into `is_addr`, refuses what the game refuses (`A0`, `A01`, `C1025`), reads a
wrapped 512 sounding of 34 four-digit-heavy pockets whole, and reads `G1000`'s arrival without reading `G100`.

The bot also names Urvara Reach: `Site::Urvara` (route word `urvara`, `--mine urvara`), so the lane plans,
lifts and flies there.

### The round's words

`mine::lane::field::round_word`, admitted by the captain's (`field::in_vocabulary`) and the escort's
(`escort::in_vocabulary`) closed vocabularies, each in closed form:

| Word | Form admitted | Cut that gives it meaning |
|---|---|---|
| `+field/void` | exactly that | GC.10 today; FR.2 |
| `+sortie/launch` into a field | `+sortie/launch <op>=<n>` or `=all`, `<op>` up to three `:`-joined numbers, `<n>` 1–99 | FR.3 (Q-40-19) |
| `+tv/<n> release` | `+tv/<n> release` and `+tv/<n> release go`, `<n>` 1–99 | FR.5 (a field act) |
| `+faction/stance` | `+faction/stance <mine>=<other>/<stance>`, `<stance>` one of `allied`, `neutral`, `hostile`; each side a `#` dbref or a name of letters, digits, spaces, `-`, `'`, `.`, `_` (1–64) | FR.1 (Q-40-11) |

`@faction/stance` (staff) is refused. The answers to `+field/void` read through `lines::void_answer`
(`VOIDED` taken, `NOTHING_TO_VOID` nothing) and the offer through `lines::void_offered`, each held to
`tacsim::field::standing`'s own constants in `field_lines_read_the_games_own_sentences`. The words are
vocabulary only: no lane plays them yet (FR.7's story does, over SSH on conclav.dev).

### The two grounds, hoisted

`field_story.rs`'s `Ground` table (`Placed`, `Ground`, `AHUNA_FLANK`, `URVARA_REACH`, `Ground::site`) moved
unchanged to `crates/tacsim/tests/support/field/grounds_rig.rs`, every item `pub(crate)`, declared by path in
all ten shard roots beside `field_raiders_rig`, so a later cut's story at both sizes reads it from any shard.
`field_story` imports it back; its tests and their minutes are unchanged.

### The smoke

`fr0_the_bot_drives_to_a_four_digit_pocket_on_urvara_and_types_void` in `crates/mush-bot/tests/mining_cohort.rs`,
`#[ignore]`d and run by name (`cargo test -j 8 -p mush-bot --test mining_cohort -- --ignored fr0_ --nocapture`):
the production root booted in the test binary, the bot's own lane at `Site::Urvara` over its own SSH session in
line mode, the canary's two phases around the rate lift (168). The lane kits up at Ceres, bunkers, lifts to
Urvara Reach, prints the witness map, launches and scans; the test hears the lane's sounding out, then types
`+tv/1 drive G1000 go` (admitted by `lane::in_vocabulary`, the first four-digit pocket east of the pad's `G993`),
reads `Rig 1 arrives at G1000` with `has_addr`, and types `+field/void` (admitted by `field::in_vocabulary`),
read by `void_answer` as nothing to void. **Passed on the owner's Windows box, 2026-11-03: scanned at +329 s,
arrived at `G1000` at +349 s, `+field/void` answered "Nothing your party's fire cost is yours to void now."
at +349 s; 350 s in all.** The lane's own Urvara sounding around `K1` read `G929 fair` and `G993 rich`
volatiles whole.

A first run typed the drive straight after the scan and was refused (`Rig 1 is sounding`: the lane files its
sounding with the scan), so the test now hears the sounding out first.

### Divergences

- The smoke types the drive and the void through the bot's own SSH session and closed vocabularies after the
  bot's own lane has played Ceres to Urvara and the scan; the lane does not choose `G1000` itself (its pocket
  is its sounding's best, which a fixed seed need not put past 999). The drive is the lane's own line form
  and is admitted by the lane's own gate.
- `+faction/stance`, the launch into a field and `release` in a field are admitted before the game has them
  (FR.1, FR.3, FR.5): the word's form is doc 40's ruled one (Q-40-11, §4.4, §4.1 item 5).

### Cell minutes

CELL_MINUTES
