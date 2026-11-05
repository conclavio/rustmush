# 40. Cut notes — the field's second round

One `## FR.n` heading per cut of [40-field-second-round.md](40-field-second-round.md) §6, in the order they
landed: what was built, the story that proves it, where the PR departed from §6's row, and the cell's
summed-minutes delta (doc 43 rule 2.3 #14).

## FR.2 — durable standing (Q-38-24, Q-40-15..18)

**Built.**

- **The tally on the live record** (Q-40-15). `FieldLive.tally: Vec<FireRow { shooter, victim, rounds }>`
  (`#[serde(default)]`, `FIELD_LIVE_FORMAT` 1 kept). `standing::note_chunk` counts into the host's RAM book
  (`FieldBook.tally`) as before; `field::on_snapshot` lays it on the live record in the snapshot's own batch, and
  the boot's resume seeds the book from it, so a resumed field's tally is the snapshot's plus the pulses it
  re-fights. `FireTally` is gone. Admission clears a site's tally; a failed resume forgets it.
- **The window on the grant** (Q-40-16). mush-faction's `Grant` carries `window: Option<VoidWindow { opened_at,
  until_ic }>` (serde default; the store's schema 1 kept), written once by `record_voidable_standing`, which takes
  it as a new last argument; an identical retry keeps the first window (it is not in the conflict compare).
  `void_standing` refuses a windowed grant past `until_ic`. `StandingReceipt.window` reports it, and
  `voidable_causes(w, prefix, by)` lists the open grants a voider may still void. `VoidOffers`, the
  `tacsim.field.standing` tick and `STANDING_TICK` are gone.
- **The synchronous close** (Q-40-17). mush-core's `SessionStat.prev_input: Option<i64>`: `None` from
  `session_start` (a fresh session or a reconnect), `Some(0)` after the first input (nothing before it), then the
  second of the input before the latest. `+field/void` voids every `field:` grant naming the actor whose window
  is open: inside `until_ic`, and `prev_input <= opened_at`. A void typed after another command is refused at
  once; a void typed first after a reconnect is forgiven (named in doc 40 §4.3). The window stays 600 IC s.
- **The director's auto-void** (Q-40-18). `FieldParty.director: bool` (serde default), set at admission or the
  join from `Entrant.director`: a fixture raider's hull whose carrier carries the director's `raid` key then. It is
  fixed at admission because the director clears the key when the carrier lifts, which may be before the field
  demotes. At demotion, a director party's friendly-fire grants are written and then voided in the same batch by
  its leader, with `DIRECTOR_VOID_REASON`; nobody is offered them. A staff raider's leader is offered as before.

**Stories** (`crates/tacsim/tests/shard_2/field_fr2.rs`, shard_2 per doc 43 §10 amendment 2):

- `fr2_ahuna_the_tally_survives_a_restart_and_the_void_is_accepted` and
  `fr2_urvara_the_tally_survives_a_restart_and_the_void_is_accepted`: GC.10's void gate at 256 and 512, with a
  `crash` once a snapshot holds the round; the resumed live record's tally is at least the pre-crash one, the
  demotion writes the one change with its window, and A's `+field/void` with no command between is accepted.
- `fr2_ahuna_a_void_after_another_command_is_refused` and `fr2_urvara_a_void_after_another_command_is_refused`:
  the same field; A types `+field`, then `+field/void`, which is refused and the change stands.
- `fr2_a_director_raids_friendly_fire_is_voided_with_no_hand`: GC.11's tempting site with the prospector and the
  hired guard in the Skulk Syndicate; the director lands, declares and fights; at demotion every change of the
  raid leader's is voided by that leader, with no command typed.
- Unit: `standing::tests` (the window's close on the next input, the reconnect's forgiveness, the tally's
  round trip), `world.rs` `session_stats_track_connect_and_input_times` (prev_input), and the mush-faction
  reputation suite on the new signature.

**Divergences from §6's row.**

- The row names `FieldLive.fire`; that field already holds the pending opening shot (`PendingFire`, doc 32
  §15), so the tally is `FieldLive.tally`.
- The director flag rides the party (`FieldParty.director`), not a read of the carrier's `raid` key at
  demotion: the key goes when the raid is over (`surface::raid::over`), which can precede the demotion.

- The void gates run `[tacsim.arena.ledger].summary_ms = 100`: the black box's 16 KiB chunks close only when full
  (or at the end), and at the default 5 s summary a four-unit field never closes one mid-field, so the round
  would reach the host only after the restart and the gate would not exercise the durable tally. That is also a
  production fact worth naming: the tally on the live record holds what reached the host in closed chunks; rounds
  in the chunk still open at a snapshot are counted when that chunk closes after the resume.

**Cost** (rule 14): five new stories in shard_2, summed locally (Windows, nextest, `-j 8`) at 42 + 42 + 39 + 44 + 67 s
= **+3.9 summed minutes** on shard_2's cell; the CI figure is the PR run's JUnit for `shard_2`.

## FR.6 — the seam (Q-36-48, Q-40-22)

**Built.** `SpacePartyId` is deleted: every ship, constant and parameter in `crates/tacsim/src/space/*`
(bubble, commands, custody, mod, picket, tick) and `tests/shard_10/voyage.rs` holds the arena's `PartyId`.
The space constants are lettered by the arena through `space::admitted_party(n)` (`PartyId::nth`, Q-CF-11):
`CARRIER_PARTY` is `A`, `INTERDICTOR_PARTY` and `CUSTOMS_CORVETTE_PARTY` `B`, `BELT_RAIDER_PARTY` `C`, the
letters the old `to_party_id` mapping already produced. `SpaceRelation` is deleted:

- `LocalSpaceBubble::set_party_relation(a, b, Option<Mark>)` marks both books (the bubble's standing stays
  symmetric, as before); `None` clears both marks to neutral.
- `party_relation(viewer, other)` returns the arena's `Relation` (`Own` for one's own party, where the old type
  said `Friendly`); `canonical_party_relation` merged into it.
- `Inspecting` is a bubble-side flag on the picket's party (`inspecting`, `set_inspecting`, `is_inspecting`),
  read where the old fifth relation was read: a screening party guards a neutral it screens from incoming
  ordnance. The old `Inspecting` arm there was dead code (`party_relation` never returned it); in the shipped
  picket the corvette already guarded the carrier because the raider is hostile to it, so no shipped scenario
  changes.
- The holotank and cockpit feeds read `relation_feed_word(Relation)`: `friend`, `neutral`, `hostile` as before.
- The `+tac/fire` refusals keep their two texts (`Own`/`Friend` → friendly, `Neutral`/`Unknown` → non-hostile).

**No persisted document moved.** Doc 40 §4.6 says the space track persists none yet; that is wrong since SF.R8
(`SpacePersistedState` under `SPACE_STATE_KEY`, as doc 35c §7 item 7 notes). The seam therefore keeps the wire:
`SpaceShipState.party` serializes through `bubble::party_wire` as the admission number the documents already hold
(`A` = 1, `B` = 2, …; a stored 0 reads as `A`, as `SpacePartyId(0)` did). The bubble's `Relations` books were
already `PartyId`-keyed, and `state_hash` folds no party, so no hash moves. Every existing document decodes and
re-encodes byte-identically; no wipe and no migration is needed.

**The one additive key.** The inspecting flag is persisted as `inspecting` with
`skip_serializing_if = "BTreeSet::is_empty"`: absent from every document unless a picket is screening, when it
holds the picket party's wire word (`["blue"]` for `B`). The old `Inspecting` was never persisted at all (it
collapsed to no mark), so the flag cannot be faithful without this key. A first try kept it `#[serde(skip)]` and
re-derived it from the picket; `sfr8`'s bit-identical restore of a mid-picket book failed on it, and the bubble
copies in the tape's snapshots cannot be re-derived, so the key stays. `step_kirkwood_picket` keeps the flag in
step with `KirkwoodPicketState::is_screening` (hailed or hove-to) before and after every step.

**Not in this cut.** §4.6's last clause, the stance table feeding a bubble's opening marks as it feeds
`FieldHost`, needs FR.1's table; the bubble's `set_party_relation` now takes the arena's `Mark`, which is the
seam that reader needs. Doc 35c's Q-35c-1 (whether SF.R18 absorbs FR.6): the owner had
SF.R18 wait on this cut (2026-11-03), so FR.6 lands first as doc 40 rules it and SF.R18's per-carrier parties
build on it.

**Tests.** `space::tests::fr6_space_parties_are_arena_letters_on_an_unmoved_wire` (the letters `A`/`B`/`C`, the
ship's `party` wire `1`/`2`/`3` both ways, symmetric marks, `None` clears, `Own`, no `inspecting` key while unset and a round trip while set).
`voyage::sfr13_…` letters its two parties by `PartyId::nth` and asserts `A`/`B`; `sfr6_…` asserts the triad's
relations as `Relation`; `sfr15_…` uses `PartyId::nth(0)`. The space lib tests and the shard 10 `sfr*` stories
pass on Windows (16/16 `voyage::sfr*`, including `sfr8`'s restart). No field story at 256 or 512 touches the space track, so neither size's story moves.

**Cell delta** (rule #14). About zero, and every move the sums show is runner noise. The figures come from
the JUnit of #363's green CI run `41728581154` (head `eb9b7359`), set against main's last CI run before it,
`41715071762` at `f1b2a1e4`; main's later merges up to #363's base `ac525cc6` were docs-only and ran no CI.
They are cross-checked against main's run at the #363 merge, `41733188761` (`fe212f4a`). Summed minutes,
main → PR → merge:

| Cell | Tests | Main | PR | Merge | Read |
|---|---|---|---|---|---|
| tacsim-lib | 2113 → 2114 | 21.3 | 37.4 | 36.1 | The new test is 0.0 s, and the 14 `space::` tests sum to 0.0 min on all three runs. The +16 min is in the other 2,099 tests (21.2 → 37.4 → 36.1). |
| tacsim-integration-10 (shard 10) | 357 | 27.5 | 47.9 | 36.2 | The 16 `voyage::sfr*` stories FR.6 touched went 2.3 → 4.1 → 3.1. The 341 tests it did not touch went 25.2 → 43.8 → 33.1, the same factor (about 1.75× on the PR run, 1.3× on the merge run). |
| Every other cell | — | 549.4 | 557.8 | 553.7 | Untouched by FR.6. Cells moved both ways by up to 26 min on the PR run (rest-server −25.8, shard 3 +11.8) and up to 29 min on the merge run (shard 4 −28.6, shard 5 +22.2). |
| **All cells** | | 598.2 | 643.1 | 626.0 | |

No cell's wall is set by this cut, and no story moves to another shard.

## FR.1 (2026-11-03) — stances (§4.2; Q-38-23, Q-40-11..14, round 2's 40-3)

Built against doc 40 Draft 0.4 (§4.2, §5.2 and the FR.1 row of §6), one PR.

- **The table** (`crates/mush-faction/src/stance.rs`): `StanceSide::{Org, Faction}` (the field's
  `Affiliation`s), `Stance::{Allied, Neutral, Hostile}`, and `StanceRow { from, to, stance,
  declared_by, at }` (`deny_unknown_fields`), one row a direction, in
  `FactionState.stances` beside the edges — the "future relationship matrix" `data.rs` reserved.
  Persisted whole under `mod:faction:stance:<from>:<to>` (`f<id>` / `o<id>`), write-through, no
  receipt; Neutral deletes the row. `FactionState::stance`, `::mutually_allied`,
  `::stances_naming`. The boot loads the rows as leaves (a bad or key-disagreeing row is skipped
  loudly) and drops a row naming a missing faction; an org side stays (org ids are never reused).
  `@faction/destroy` deletes the destroyed faction's rows.
- **The verbs** (`stance_commands.rs`): `+faction/stance <yours>=<other>/<allied|neutral|hostile>`
  for a faction's principals and members at `officer_level` and an organization's holders of
  `org.stance.declare` on its root scope; `@faction/stance <from>=<to>/<stance>` for staff. A side
  is a faction first, else an organization; a hidden one the viewer may not know reads as missing.
  A declaration is a line to the other side's officers. `+faction <name>` shows the rows
  ("Stance toward X: Hostile (mutual).", "X stands Allied toward it (unreciprocated)."), each naming
  a side the viewer may know; `+faction <org>` shows an organization's rows. `+faction` moves from
  the read-only registration group to legacy-post-image with its new switch (allowlist stanza
  refreshed, family `faction.stance`; `PINNED_BASELINE` unmoved); `@faction`'s stanza notes the
  switch.
- **The capability**: `mush_org::STANCE_DECLARE_CAPABILITY` = `org.stance.declare`, in tacsim's
  built-in vocabulary beside `org.role.manage` and in the server's vocabulary pin.
- **The field** (`crates/tacsim/src/field/`): `parties::opening_stances` reads the table over the
  parties' open affiliations — `(by, of)` Hostile where an organization of `by` declared Hostile on
  one of `of`'s, and every mutually Allied pair (Q-40-12); a fixture raider's standing marks ride
  the same rows. `FieldParty.stance_hostile` (`#[serde(default)]`, skipped when empty, so no record
  format moves) holds them on the live record: written at admission (`parties::admit_stances`),
  extended in `carrier::joining` for the pairs a new party opens only, and `FieldHost.marks` is
  built from those rows (`parties::standing_marks`) at both. Allied pairs join `FieldHost.friends`.
  `mark_in_field`'s clear of a party on its row sends `Some(Mark::Hostile)` and says "cleared to
  the standing Hostile" (`PARTY_CLEARED_TO_STANDING`) — so a raider's clear now behaves the same.
  `friends_of` (the void's friendly-fire test) counts a mutual Allied pair (Q-40-14).
  `friends_on_ground` returns the Allied pairs and each party's `stance_hostile` as admission
  would write them. `field/carrier.rs` is touched in `joining` only. No kernel change.
- **The story** (`tests/shard_2/field_fr1.rs`, `fr1_declared_stances_at_256` / `_at_512`, Ahuna
  Flank and Urvara Reach): three co-ops as factions (Concord ↔ Basin Hostile both ways, Concord ↔
  Halo Allied both ways, Halo → Basin Allied one-sided; Concord's leader also an open Basin
  member); a stance opens nothing (Q-40-13); Concord's `+field/mark` promotes; each picture holds
  the other Hostile from the first classification; Halo is Concord's Friend both ways, Basin is
  not Halo's; the clear of Basin returns to Hostile (not the shared member's Friend), a Friendly
  mark replaces it, the next clear returns to it; Halo's mid-field Neutral changes nothing; a
  Basin captain landing mid-field joins with Concord's Hostile on it in Concord's book and its own
  on Concord; everyone stands down and the field disengages; the next field reads Halo's Neutral
  and admits the arrival under the Hostile. The org declare path (`@org/create`, `@org/grant …
  org.stance.declare`) is exercised by the same story; mush-org has no membership verb, so an
  organization's stance has no field effect through a command path yet — the factions carry the
  field half. `mush-faction/tests/faction.rs` covers the refusals, the row round-trip through the
  store and the destroy cascade.
- **The raider's clear changed, as §4.2 rules**: CF.7's field story (`shard_9/field_story.rs`) had
  the raider's leader `=clear` every party but the bystander's to aim the raid; a clear now returns
  to the raider's standing Hostile, so the story marks them `=friendly` instead (two lines, the
  same intent). No other test clears a standing mark.
- **A refused join's letter** (found merging main): `on_joined` takes a refused arrival's party off
  the live record but would have left its letter on the other parties' `stance_hostile`; `joining`
  now drops a newly lettered party from every standing row before extending them, so the letter's
  next holder opens under its own stances only.
- **The story's host carrier sails home** before the field: after main's FR.2 and S.4b-T merged,
  the 512 run under parallel load met doc 36 cut GC.1b's known race (the kernel refuses a join
  while a mind's think is in flight at its encode: "a think awaits its answer on #395"), with the
  neutral captain's carrier the only mind on the ground. With no mind on the ground the join is the
  story's, not the race's; three parallel runs green at both sizes.
- **Grounds**: the sites are FR.0's `grounds_rig` (`AHUNA_FLANK`, `URVARA_REACH`), taken once FR.0
  landed; the three hulls' cells are the story's own `Stand` table (from `field_gc10` and `field_story`).
  `driving_rig::en_route_at` is the one support addition (`en_route` for any site).
- **Shard** (doc 43 rule #3): shard_2, the cell with the room (11 summed minutes in doc 43 §10).
- **Summed-minutes delta** (rule #14): **+2.9 min** on `tacsim-integration-2` (the two stories 84.8 s + 86.5 s on the PR run 41730885044 JUnit); the cell now sums 23.0 min (shard_2 14.2 + cost_lane), its wall 17 m 0 s, under the twenty-minute line.

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

**The delta is one new test of 0.03 s in the `bot` cell; no other cell gained or lost a test.** Measured from
the JUnit of #364's last green CI run, [41747090228](https://github.com/conclavio/rustmush/actions/runs/41747090228)
on head `857a61bf`, against main's run just before it,
[41746078482](https://github.com/conclavio/rustmush/actions/runs/41746078482) on `f299aaf0`, which is the main
that `857a61bf` merged. So the two runs differ only by this cut.

- **New tests**: `mine::lines::tests::fr0_every_pocket_of_a_512_site_is_an_address` (`bot`, 0.03 s).
- **No CI time from the rest**:
  - The Urvara smoke is `#[ignore]` and runs by name only.
  - The words' cases extend existing tests.
  - The hoist moves no test.
  - §4a's inventory ran as gate dispatches, not in CI.
- **Each cell's test count**: the same in both runs except `bot` (735 → 736).
- **Summed minutes per cell, main → PR**: `bot` 11.10 → 11.83; `bot-stories` 27.18 → 28.29;
  `tacsim-integration-9` (`field_story`'s shard) 50.82 → 30.73; `tacsim-integration-1` 78.86 → 56.57;
  `tacsim-integration-6` 34.24 → 44.04; `tacsim-integration-7` 37.17 → 46.51; `rest-server-boundary` 36.90 →
  19.21; total 665.46 → 564.79.
- **These movements are host noise, not this cut.** They run both ways, from −22.3 to +9.8 minutes, in cells
  whose tests are identical in both runs. They are led by long single tests:
  - `mill::sm12a_a_month_of_milling…` −61 s;
  - `field_story::s2_the_field_story_at_512` −38 s, a story whose only change here is where its `Ground` table
    is declared.

No cell's wall moved because of FR.0, so nothing moves shard (rule #3).


## FR.4 — segments (Q-38-28, Q-40-21)

Row: §6 FR.4. Base `a2ef4075`.

**Built.**

- **`Ledger::roll_segment(segment, t_ms, hash)`** (`arena/ledger.rs`): `bytes_total` → 0, so the summaries' 90%
  and the `Timeout` at 100% are a segment's, then one `segment` line (`EventKind::Segment { segment }`, the
  segment it opens, from 2) — the first byte the new segment counts. `seq`, `digest`, `shipped` and the chunk
  numbering run on, so the segments are one chain. The ledger carries its segment (`Ledger::segment`, 1 until
  a roll); `LedgerState.segment` is `serde(default = 1, skip_serializing_if = 1)`, so every OOC snapshot and
  every field's first segment encodes byte for byte as before. `Ledger::segment_full` is the threshold: at
  least three quarters of `max_bytes` (48 MiB of 64, provisional).
- **The live roll** (`arena_task.rs`, `Task::snapshot_if_due`): past the due check, a task under the field
  host's terms whose segment is full closes the open chunk and queues every closed one, rolls, and closes
  and queues the line as a chunk of its own; the period's snapshot is then encoded and queued behind it, so
  it is the new segment's head with nothing open. An OOC task (no `FieldTerms`) never rolls.
- **The host's half** (`field/segment.rs`, `note_chunk`, called at the head of the bridge's `on_chunk`): a
  chunk whose first line is a `segment` line for a later segment than the live record's moves
  `FieldLive.segment` onto it and writes the live record in the chunk's own batch, before the chunk is
  filed. So the line's chunk is the new log's first, and the head snapshot that follows it is written
  beside a live record already on the new segment.
- **The resume's roll** (`field/boot.rs`): right after the snapshot decodes, `live.segment = max(live,
  snapshot's ledger) + 1` and the ledger rolls onto it at the snapshot's millisecond; the line is the
  resume's first. It replaces the old `segment += 1`. The max keeps every opening on a fresh log even when
  the store holds a live record a roll moved past its last snapshot.
- `FieldLive.segment`'s doc now says it counts rolls as well as openings; the staff report spells the line
  `segment segment=<n>`.

**Divergences from §6's row and §4.5.**

- §4.5 says the host bumps `FieldLive.segment` "in the host's batch" of the snapshot. The host reads the
  roll off the tape instead (the `segment` line's own chunk): the task's queue keeps at most one waiting
  snapshot and a newer one replaces it at the back, behind chunks the new segment closed, so a snapshot-borne
  bump could file those chunks under the old log. The line rides ahead of the head in the same queue, so the
  head's batch still finds the live record on the new segment.
- The segment line is a chunk of its own on a live roll (one small chunk a roll). That makes every chunk one
  segment's and leaves the head with no open bytes, so a resume from the head files nothing of the last
  segment under the next. A resume's roll leaves the snapshot's open chunk open: those bytes went to the new
  segment's log before this cut too.

**The story** (`tests/shard_10/field_fr4.rs`; shard_10, "the room for growth" in the shard list):
`fr4_ahuna_a_field_rolls_resumes_from_the_head_and_keeps_every_chunk` and
`fr4_urvara_a_field_rolls_resumes_from_the_head_and_keeps_every_chunk`, on `grounds_rig`'s `AHUNA_FLANK` and
`URVARA_REACH`. A's `+field/fire E` at B's hull opens a field with `[tacsim.arena.ledger].max_bytes` lowered to
48 KiB (the test knob), `summary_ms` 100 and `snapshot_ms` 2,000. The story asserts:

- the field rolls in its first minute, and the live record is on segment 2 at the roll's millisecond (the head);
- the world restarts there, and the field resumes on segment 3, its `segment` line at the head's millisecond with
  the head's sequence plus one;
- the world holds `field.<site>.<ordinal>.1`, `.2` and `.3` of the one field: segment 1's chunks from 1 with no
  segment line, segment 2 opening on the next chunk with the line alone, segment 3 on the chunk after it;
- read in chunk order, every chunk is numbered once and the lines are one sequence with no gap.

Unit tests: `arena::ledger::tests::fr4_a_roll_resets_the_segments_bytes_and_runs_the_chain_on` (the threshold,
the reset, the chain, no `segment` key before a roll and a round trip after) and
`field::segment::tests::fr4_a_rolled_chunk_opens_its_segment_and_no_other_chunk_does`.

**§4a's OOC pins.** None moves, against FR.0's inventory above: OOC tasks never roll, and a ledger that has not
rolled encodes no new key, so `SNAPSHOT_FORMAT` 19 and `FIELD_LIVE_FORMAT` 1 stand. The new `EventKind` variant
is written only by a roll. The draw-label set is untouched. The pin rows were re-run on this branch (below).

### Pins run (§4a, rule #13)

- **The pin rows**: gate sweep [41761095972](https://github.com/conclavio/rustmush/actions/runs/41761095972) on
  conclav.dev, head `dda59126`, with FR.0 run A's filter widened to run C's rows (`arena::`, `arena::rng::`,
  `match_lifecycle::m3_start_writes…`) and `field_fr4::`. **1,617 tests, 0 failed, 2,275 s.** Each §4a row's test
  passed:
  - `twin::m4_5_the_stock_pool…`, `motor::m1a1_the_shipped_duel_hashes_as_before` and `motor::gc1b_…`;
  - `reflex::m3_the_j_fields…`, `minds::m2_the_memory_codec…`;
  - `the_draw_label_set_is_pinned`, `adding_a_label_shifts_no_existing_draw`;
  - `content::the_pinned_digests_are_the_shipped_ones`, `m2_the_standard_doctrine_source_is_pinned` and
    `m5rules_default_seal_identities_are_pinned`;
  - the 16 `af1_` snapshot tests and `field::record::`;
  - the field gates: `field_gc2::` (2), `field_gc4::` (6), `field_gc10::` (2) and `field_story::` (4).

  `M2_CROSS_PLATFORM_PINS` row 0–3 is the `tacsim-minds` cell's by-name step on the PR run (green).
- **The harness kind**: [41761096467](https://github.com/conclavio/rustmush/actions/runs/41761096467) on
  conclav.io, 4 of 8 failed. Three are timing lenses:
  - `arena_minds`: wake p99 23.3 ms against 20 ms, window p99 45.3 ms against 32 ms;
  - `arena_step`: window p99 33.9 ms against 22 ms;
  - `arena_authored_reach`: wake p99 26.4 ms against 20 ms.

  The fourth is `lance` W8 (span 3 where 2), which TS.21's own harness runs on conclav.dev (41756575489,
  41760419368) also fail. Every variant reads `sound=true` and every snapshot `hash_ok=true`. No harness path rolls
  (an OOC task has no `FieldTerms`).
  **Main fails the same four on the same box**, with the same assertions and similar figures:
  [41763746775](https://github.com/conclavio/rustmush/actions/runs/41763746775), `main` at `ea602323`, `authored_reach`
  wake p99 22.3 ms, `arena_step` window p99 33.7 ms, the minds verdict 0 of 4, and `lance` W8. So the kind's red
  is the box's timing and the known W8 read, both TS.21's to re-bless (doc 43 amendment 6), not this cut's.

### Cell minutes (rule #14)

**+1.6 summed minutes on `tacsim-integration-10`**: the two stories, 45.0 s (Ahuna) + 51.9 s (Urvara), on the PR's
CI run [41761064794](https://github.com/conclavio/rustmush/actions/runs/41761064794) (head `dda59126`). That run is
set against main's [41754225803](https://github.com/conclavio/rustmush/actions/runs/41754225803) at `c0d123a9`;
main's two merges after it, up to this branch's base `a2ef4075`, were docs only.

- **New tests**: the two stories, plus two unit tests in `tacsim-lib` (0.0 s each).
- **Test counts**: no other cell's count moved.
- **The cell's sum** read 50.2 → 28.3 min and its Test step 16 m 25 s → 9 m 42 s: the host, not the cut.
- **Every other cell** moved both ways by up to 26 min with identical tests (`tacsim-integration-1` −25.9,
  `rest-server` +16.4, `tacsim-lib` +17.0); the total read 637.1 → 604.0.
- **Shards**: no cell's wall is set by this cut, and nothing moves shard.

## FR.5 — carried in: the join-during-think await (found 2026-11-04, before the cut)

Built by FR.5 (below, item 1). This recorded a finding so FR.5 did not rediscover it: doc 38 §9.3's "the join-during-think await",
with the evidence from the GC.1b shard-10 flake.

- **What fails.** `field_carrier::gc1b_a_refused_middle_party_never_lends_its_letter_to_the_next_arrival` failed at
  `field_carrier.rs:725` on PR #389's runs 41764030040 and 41767085311, and passed alone. `gc1_…` fails at line 420
  the same way. Under load the kernel answers both arrivals' joins for real with `match recovery capacity refused: a
  think awaits its answer on #395: the batch was not delivered before the encode`.
- **Why.** `arena_task.rs` takes `waiting.joins` in (`Task::join`, then `Arena::join`'s recovery-capacity encode)
  before `Task::pulse`, and only the pulse's head runs `await_thinks`. A join that lands while the last pulse's think
  batch is outstanding meets `SnapshotError::Awaiting`. Every unseated hull on the field thinks (the carrier on the
  pad, a rig out). On a loaded cell each late pulse steps more arena time, so more pulses carry a batch.
- **FR.5's item.** Await the outstanding batch before the joins are taken in (`arena/*` is FR.5's alone, doc 38
  §9.2). The other half of the failure is FR.3's: `carrier::on_joined`'s refusal drops the kept arrival and puts it
  in `join_refused`, never to be asked again. FR.3 builds the retry.
- **Evidence** (conclav.dev; 3–5 processes of the shard_10 binary at once, `field_carrier::`, `--test-threads=6`,
  `taskset` to 4 cores):
  - beat-counted waits: 4 of 18 failed, while the base passed 15 of 15 on the same rig. No wait length helps.
  - every core class at `think_ms` 5000 (the config ceiling): 2 of 30 failed, against 3 of 30 for the unchanged
    control run beside it.
- **Stopgap.** `.config/nextest.toml` retries the gc1b story twice (a pass on retry reports FLAKY). Remove it when
  FR.3's retry lands.

## FR.3 — reinforcement (§4.4; Q-38-25, Q-38-26, Q-40-19, Q-40-20, Q-40-24; doc 23 Q-23-21, D-23-23)

Row: §6 FR.3. Base `c0801a8a` (S.4b-1 merged). This cut is the unlicensed half. Its join reads `RigRecord.claim`,
and an `Open` claim waits for doc 23 S.4b-2's install (see "Owed" below).

**Built.**

- **`FIELD_LAUNCH` is retired** (`detachment/sortie/prepare.rs`, `launch`). A `+sortie/launch` from a carrier on a
  promoted site's pad now passes two checks:
  - **The launch cycle** (Q-40-19): `[tacsim.field].launch_cycle_s`, 60 wall seconds (provisional), banded 10–600
    as `FIELD_LAUNCH_CYCLE_S`. It is turned into IC seconds at the world's rate (`FieldConfig::launch_cycle_ic`).
    - A carrier puts one hull down a cycle.
    - A launch of several slots opens one cycle per slot, in slot order: slot *k*'s `due` is its cycle's start
      plus `LAUNCH_S`.
    - A second launch inside the cycle is refused with `site_record::LAUNCH_CYCLE`, which names the field, the
      site, the carrier and the wall seconds left.
    - Only launches accepted at or after the promotion open a cycle (`SiteRecord::cycle_free_at`). A launch from
      before the field opened is no pour into it.
  - **The cap over the one count** (Q-CF-21's CF.2 line), now counting what is in flight.
- **The one count** (Q-40-24): `SiteRecord.pending: Vec<PendingLaunch { hull, carrier, accepted_ic }>`.
  - The field is serde default and skipped while empty, so every earlier record is byte-identical. It is bounded
    by `FIELD_MAX_UNITS`, kept in hull order and checked by `SiteRecord::check`.
  - **Written** by the launch's own transaction for every new slot, whether or not a field is open. The DT
    `Snapshot`'s `site_records` now insert a record that did not exist (`fresh_record`, at the site's side).
  - **Counted** by `field::parties::site_population`: each live row whose hull is not on the ground, plus each
    launch the field sent back aboard. So the prepare, the rig ramp's launch half and admission all see a launch
    in flight. A hull whose row is already live is not counted twice by its own launch.
  - **Retired** by the abort: `+sortie/cancel-launch` and an `+sortie/end` that cancels a departure drop the row
    in the same commit (`prepare::drop_pending`), which frees the cycle too (§4.4 step 5).
  - **Stale**: a row is live until its landing (`accepted_ic + LAUNCH_S`) and one cycle have passed
    (`SiteRecord::pending_live`). The count and the cycle then ignore it, and the record's next launch drops it.
- **The join** (§4.4 steps 3 and 4; `field/carrier.rs`). `join_launches` runs beside `join_arrivals` in the field
  tick, one new line in `admit.rs`'s `tick`. Each **pending entrant** (`carrier::launches`) is a non-carrier hull
  whose DT flight landed on this site (`Phase::Field`, epoch open, not closed) and that the running field does
  not hold:
  - not on the live record, not owed an exit, not checked out, not held out, not towed, not out at the ramp and
    not leaving;
  - or one the field sent back aboard.

  Each entrant is checked out under the field's mark and offered as `ArenaMsg::Join` where it stands at the ramp,
  with the director's spread (`raiders::LAUNCH_SPREAD_CELLS`, 4). That goes through the shared `carrier::join`,
  now `join(ctx, site, hull, pose, spread)`, which the arrivals use at their pad pose with `PAD_SPREAD_CELLS`.
  - On the kernel's `Joined` Ok, the site hears `LAUNCHED_IN_FIELD`. The launch's pilot, still in its seat, is
    bound to the task as an admission binds one (`seat_joined_pilot`). Without that the hull joined and flew as
    its mind, and its pilot got no frames.
  - **Step 3's seal**: `launch_seal` reads `RigRecord.claim`. `Open` is `LaunchSeal::Claim(open)`; anything else
    is the ROM, as every entrant.
- **A refused join** (Q-40-20; step 4). `carrier::on_joined` tells a launch from an arrival by
  `FieldBook.launching`. A refused launch is checked back in, its row leaves the live record (and its roster
  party, if nobody else holds it), and it is put back aboard into its bay berth (`place_interior`). Its operator
  hears `LAUNCH_BACK_ABOARD`, and it is noted in `FieldBook.relaunch`. The next tick puts it down at its ramp
  again and offers the join, until it is taken or the field demotes.
  - The flights' tick (`detachment/sortie/tick.rs`, one guard) skips a `Phase::Field` flight in `relaunch`, so a
    bay that is no return does not close the epoch `RecoveredToBase`.
  - The relaunch set is RAM, cleared when the field is gone. A restart while a hull is aboard loses it, and the
    flights' tick then reads the hull home as recovered: an honest close, not a hull on the ground outside the
    field.
- **GC.1b's refused arrival is retried** (Q-40-20): `FieldBook.join_refused` is gone. A refused arrival is asked
  again at the next tick, as the director's raid is. `ARRIVED_IN_FIELD` now says the carrier launches into the
  field one hull a cycle, which closes Q-36-5's half state for an F-6 arrival.
- **Config**: `launch_cycle_s` in `[tacsim.field]` (`mush-config` with its table row, `config_ext`, tacsim's
  `FieldConfig` and its validation).

**Divergences from §4.4's text.**

- **The row is retired by the abort and by staleness, not by the join's batch.** The field tick runs over a `Ctx`
  with no write into the site family. Retiring there would need a new host-owed `tacsim_field` op, as `exit` is.
  Instead the count takes the union of the ground and the live rows, by hull. A landed hull is counted by the
  ground, a refused one by `relaunch`, and a row past its window by nobody. The count is the same one §4.4 asks
  for, written in one place.
- **The rig ramp's `+tv/<n> launch` into a running field is still refused** (CF.1's "every act on a promoted site",
  `FIELD_OPEN_CODE`). The unlicensed story is the DT `+sortie/launch`, as the row says. Opening the rig ramp's
  launch is the licensed half's: S.4b-2's `go` queues it behind the claim.
- **An `Open` claim waits rather than joining.** Until S.4b-2's `custody_unit` seals under the claim's own seal, a
  join would seal the ROM over an open claim. No world composes the claim path yet, so nobody can reach it.

**Owed (named follow-up): FR.3b, the licensed reinforcement**, on top of doc 23 S.4b-2 (`claude/s4b2-claim-arm`,
not yet up as a PR when this cut was opened). It needs:

- `join_launches`' `LaunchSeal::Claim` arm to drop its wait, once `custody_unit` seals under
  `minds::ic_sortie::claim_seal`;
- the rig ramp's launch opened into a field behind the claim, with its pending row;
- the shared acceptance story of D-23-23 at 256 and 512: a second cored rig launched `mind=` after the first shot,
  its claim opened in the bay, joined by the next tick under its own seal and thinking.

The row's other unbuilt acceptance items are an F-6 arrival that joins and then launches, and two *different*
carriers one short of the cap. Both read the same code as the stories below (the arrival's carrier is an ordinary
carrier once joined, and the count is per site, not per carrier). Neither has a story of its own yet.

**The story** (`mush-server/tests/split_boundary/dt_preparation_plans.rs`). The tacsim world harness lands no DT
activation, so the DT field stories live in the split-boundary harness. Its shared helpers (`pulse`'s clerk,
`staff_place_on_ahuna`, `promoted_by`, `until_hosted`, `two_pilot_fixture`) now read the ground and the field's cap
and cycle from a `tokio::task_local!` `Story` scope. Outside a scope it is Ahuna Flank with the shipped knobs, so
every earlier test reads as before.

- `fr3_a_second_escort_launches_into_the_running_field_on_ahuna` and `…_on_urvara` (256 and 512): two players fly
  one operation, and the activation puts the first escort down. Its `+field/fire E` at a bystander opens the
  field with the second escort still aboard. Then, through the command paths:
  - the organizer's `+sortie/launch 1:1=2` is accepted, with its pending row naming the hull and the carrier;
  - a second launch inside the cycle is refused with the seconds left;
  - `+sortie/cancel-launch 1:1=2` drops the row and frees the cycle, and the launch again is taken;
  - the hull lands on the IC clock, and the next tick joins it. Its pilot's cockpit frame is the hull's, in the
    first escort's party, deployed, within the spread of its ramp.
- `fr3_the_pending_row_counts_a_launch_still_aboard` (the one count, at Ahuna; it reads no ground): with the cap
  at two and only the carrier down, slot 1's launch is accepted and slot 2's, a moment later, is refused with
  CF.2's cap line while slot 1 is still aboard. Before the row, both went down.
- Unit: `surface::site_record::tests::fr3_pending_launches_count_cycle_and_go_stale`.
- `tacsim --test shard_10`: `field_carrier::gc1b_a_refused_middle_party_never_lends_its_letter_to_the_next_arrival`
  steps (3)–(4) now read the retry. The refused arrival's retry takes the freed letter, the third takes the next
  free one, and the restart keeps every letter. This is the test that failed under load (the gc1b-flake session's
  diagnosis: the kernel refuses a join for real while a think is outstanding, FR.5's await, and the arrival was
  never asked again). The cut drops PR #398's `retries = 2` stopgap override for it from `.config/nextest.toml`.
  The gc1b-flake session's loaded stress was run on conclav.dev at this branch's `237bd386`: 5 copies × 6 rounds of
  `field_carrier::` on `taskset -c 0-3`, `--test-threads=6`. **0 of 30 failed**, against 4 of 18 (beat-counted
  waits) and 3 of 30 (the unchanged control) on the same rig without the retry.

**§4a's pins.** None of §4a's pinned files is touched: no kernel file, no snapshot or live-record format, no
content, no seal. The live record gains no field (`launching` and `relaunch` are RAM), so `FIELD_LIVE_FORMAT` 1
stands. `SiteRecord.pending` is skipped when empty. Rule #13's harness kind is therefore not required for this
cut, and FR.5 moves no hash it would not have moved before it.

### Cell minutes (rule #14)

To be read from the PR run's JUnit and recorded here before the PR is marked ready.

## FR.5 — the rebless (§4.1, §4a; Q-38-17, Q-38-27, Q-36-56, Q-40-2..10, Q-40-25)

PR #410. One kernel rebless, built in §4.1's order. The base is `4390f04c`.

### What was built

1. **The await** (Q-40-10). `arena_loop` awaits the outstanding think batch (`Task::await_thinks`) before
   `Task::apply_field_acts`, which applies the pulse head's marks, tow releases, withdrawals, homes and joins.
   A join now encodes an arena whose answers have landed. The pulse's own await then finds nothing in flight.
   An OOC match has no field acts, so its order is unchanged.
   - Gate: `arena_task::tests::fr5_a_join_is_taken_in_after_the_batch_in_flight_lands`. It drives GC.1b's
     order, where the join is refused by name with "a think awaits its answer", then the new order, where the
     join is taken in.
   - FR.3's retry stays as the guard.
2. **`note_fire`** (Q-40-9). A third party that holds the shooter a Friend, by standing or by mark, is no
   longer told. The victim is moved as before. Gate: `party::tests::fr5_a_friend_of_both_is_not_told`.
3. **`line_to` over wrecks** (Q-40-8, no API bump).
   - `ThinkJob::wrecks` carries the wreck cells (`Arena::mind_wrecks`), captured at collection beside the
     map. The view is unchanged: a mind reads no wreck.
   - The stock mind and the Rhai `line_to` kernel trace `stock_mind::Ground`, which is the map plus those
     cells, over `WithWrecks` with the line's two ends left out, as the fire trace does.
   - The twin's `line` counter reads the same ground.
4. **The component effects** (Q-40-2..6).
   - `ComponentEffects` is compiled beside `COMPONENT_HITS`. An optional `component_effects` block may
     override it, on the resolution pack's def and per unit (`UnitSpec`). It is absent from every shipped
     pack, so no digest moves.
   - `Arena::apply_part_effects` (`arena/part_effects.rs`) runs in `admit_unit`, which also serves a join,
     and after `part_struck` lands. What it does, kind by kind:
     - Sensor: off when jammed. When damaged, the range is 500 ‰ of `UnitState.sensor_authored`, so it never
       compounds. `sensor_authored` is folded into the hash only when present.
     - Pod and filter: off when jammed or damaged.
     - Plant, jammed: `Power::Off` and the pending `StartupComplete` dropped. STARTUP is refused by name with
       `Refusal::Plant`.
     - Plant, damaged: the heat ceiling and the dissipation at 500 ‰, read-side.
     - Core: one class down, read-side (`read_core_under`), with the slots truncated.
     - Mount: as GC.2 built it. `part_works` is retired into the derivation.
   - The relay is the host's (`field/orders.rs`): an order is held while the relay is jammed and delivered
     10 s late while it is damaged.
   - No new draw.
5. **The coupled tow** (Q-40-7, Q-40-25).
   - Kernel (`arena/tow.rs`):
     - `UnitState.coupled_to` is folded only when present.
     - `FieldHost.tows` is coupled in `Arena::new`. A stacked casualty is first moved off its tug's cell by
       the join's ring rule.
     - The trailer step: after the tug's `integrate` and `index.move_unit`, its casualty takes the cell the
       tug left. A coupled unit never integrates and never fires.
     - The drop: a destroyed or withdrawn tug leaves its casualty standing.
     - `ArenaMsg::Release` / `ArenaOut::Released` is a field act, IC only, answered behind the ledger.
     - The coupled withdraw: a casualty is judged by its tug at the ramp and is withdrawn before it.
   - Host: `field/admit.rs` lifts the `carried` filter. A towed casualty is an entrant on its tug's party
     (`coupled_entrants`), the pair rides `FieldHost.tows`, and the casualty is never counted.
   - `surface/tow.rs`'s half went last on that file, after RS.5. It is **FR.5b** (PR #433), split from #410 so
     that #410 could merge first.

**The join's install of a scripted seal** (doc 23 S.4b-3's need). Carried here as the kernel half, agreed with
the S.4b session on 2026-11-04.
- `ArenaMsg::Join` carries `source: Option<String>`, the retained doctrine source keyed by the seal's digest.
- `Arena::join_installing` installs the runner on the join's clone by the fresh check-out's rule
  (`minds::install_sealed_runner`).
- A scripted seal with no source, or one that does not install, is refused by name and never voids the field
  (D-23-17).
- Every sender passes `None` today. S.4b-3's host half fills it in.

**Not taken here: doc 23's D-23-18, the carried memory.** S.4b-3 (#411) left its arena side to FR.5. D-23-18
(doc 23 §17.7) has two halves:
- the return: `UnitReturn` carries the mind's memory, `think_seq` and drop cause at every check-in;
- the seed: the next install on that epoch seeds the row from `OpenSortie.mind`.

The seed needs the carried mind on `CustodyUnit`, a struct that dozens of test files build literally, beside the
S.4b host record. Neither half is useful alone. FR.5 therefore leaves both to one later cut on doc 23's side, which
owns `arena/*` once FR.5 merges.

### §4a's pins, re-pinned once (rule #13)

Each row is set against FR.0's inventory and was read on conclav.dev.

| §4a row | FR.0's value | FR.5's | Why |
|---|---|---|---|
| `M4_5_POOLED_RECORD` | as committed | re-recorded (21 lines) | items 3 and 4. The divergence counters return toward nought, `line` 28 → 5 and `no_line` 31 → 12. Thinks 10,603 → 10,306. The 14–1 split and `first_divergence_ms` (101,377) stand. The one blue win moves from seed 2360 to seed 2355, a close finish the effects now decide. Nothing is `unlawful`. |
| `COVERAGE` | pooled `(10_295, 308)`, mixed `(13_230, 502)`, lance `(7_157, 130)` | `(10_015, 291)`, `(12_884, 449)`, `(7_260, 126)` | the same two items; no divergence on any lane |
| `POOLED_MATCHES_WITH_STAND_IN` | 12 | 14 | a jammed or damaged core reads a class down, so more hulls finish on the ROM |
| `M2_CROSS_PLATFORM_PINS` | rows 0–3 as FR.0 recorded | rows 0 and 1 re-pinned (stops 176,250 → 180,250 and 235,250 → 177,500) | items 3 and 4. Row 2 is unmoved byte for byte. Row 3, the float probe, is unmoved (3,600 thinks, bitmask 369098751). Windows is ci.yml's step on the PR. |
| `DUEL_HASH_API6_OBSERVED`, `DUEL_HASH_AT_HEAD` | `16080728793682240297`, `0xd15b_cccc_3546_150f` | **unmoved** | every new row folds into the hash only when present |
| The (j) field, the memory digests, the draw-label set, `DIGESTS`, `POWER_STATES_SCHEMA_DIGEST`, `STANDARD_SHA256`, `API_VERSION` 7, the seal identities, `SNAPSHOT_FORMAT` 19, `FIELD_LIVE_FORMAT` 1, the four pack digests | as FR.0 recorded | **unmoved** | no new draw, no shipped pack byte, no API bump (Q-40-8), and every new field decodes by serde default |
| The field gates | as FR.0 recorded | `field_gc2::` (2) and `field_story::` (4) unmoved; `field_gc10` keeps `idle_demote_s` (see the departures); story (a) moves with item 5 (below) | |
| The harness's arena baseline (`arena.linux.json`) | blessed at `a2ef4075` | 65 deterministic leaves re-blessed by record, `fr5_deterministic_update` | items 3 and 4. One hull ends a minds lane in `startup`, because a warm-up hit jammed its plant. The warm-up's cold check now reads a jammed or destroyed plant as the ruling's `Off` (`plant_down`). The timing block is unmoved. |
| The lance lane's W8 fact pins | `blue-1` the squad held by a wreck alone; standings `(3, 2, 3, 3)` | `red-1`; `(3, 3, 4, 3)`; six releases | re-pinned by record, items 3 and 4. The span (3), the end (none, 131,500 ms) and the busiest minute stand. |
| `field_scale` at 100 | Engaged p99 6.375 ms (GC.12) | Engaged p99 **7.831 ms**, worst 500 ms window 14.851 ms (lattice); 9 units: 0.275 ms | the effect derivation in the pulse. Measured by hand in release on the owner's Windows box, because the harness held conclav.dev at the time. The 40 ms budget stands. |
| The contract guards | 34 files green | green (163 tests) | |

**The harness kind on conclav-dev** (rule #13): green at `ef63b21f`, run
[41860284848](https://github.com/conclavio/rustmush/actions/runs/41860284848). Three runs before it found the moves
above, in order:
- [41848864284](https://github.com/conclavio/rustmush/actions/runs/41848864284): the cold check and W8.
- [41855932681](https://github.com/conclavio/rustmush/actions/runs/41855932681): the delta.

### Departures

- **`field_gc10`'s void gate still demotes on `idle_demote_s`, not on `disengage_s` as §4a expected.** Probed on
  conclav.dev with the books and every armed viewer's hostile contacts printed each two arena seconds, the cause
  is not a friend of both:
  - B's hull is its own unit party (`f`) and was the victim, so its book holds A's hull (`e`) fired.
  - B's `+field/mark <e>=friendly` writes the book of B's roster party (`blue -> e: Friend`), never `f`'s.
  - `f` is armed and holds `e` in reach, so the tempo is Engaged every pulse.

  `note_fire`'s fix does not touch this, so the gate keeps its 8 s idle bound, with the reason in its comment.
  **Ruled by the owner (2026-11-04): yes.** A leader's `+field/mark <hull>=friendly|hostile` also writes the
  books of the unit parties whose hulls that leader seats, so one mark covers every hull they command. It is
  host-side (GC.10/FR.1's mark path) and is built as its own small PR after FR.7. When that PR lands,
  `field_gc10`'s void gate moves to `disengage_s`, as §4a expected, and its 8 s idle-bound comment goes.
  **Built** (the leader's mark PR, after FR.5b):
  - `field::commands::mark_in_field` also sends `ArenaMsg::Mark` for every unit party whose hull the leader seats
    (`seated_parties`, by the live record's pilot or the seat as it stands now). Each book keeps its own standing
    Hostile on a clear.
  - `gc10_a_leader_voids_friendly_fire_standing` now demotes on `disengage_s` (5 s, with the idle bound at 600 s),
    in 28.9 s.
  - `field_fr1::`, `field_fr2::`, `field_story::` (both sizes), `field_raiders::` and `review_36a::` are unmoved:
    23 of 23.
- **The relay** reads the carrier's world record. A relay damaged by a hit inside the running field therefore
  takes effect from the next field. The seam is named in `carrier_relay`'s doc comment: a host mirror of the
  kernel's part conditions would replace that read.
- **A core below its sealed class** lowers the mind's class read-side and truncates its slots, as `DegradeCore`
  does. It does not re-seat the mind on the ROM, because `DegradeCore` does not either.
- **A coupled casualty's "no fire"** answers the existing refusal for a hull with nothing to fire, so no new refusal word is added.
  After a release the casualty keeps its damage limits but may fire again. A casualty destroyed while coupled
  stays coupled, since a wreck is still towed.

**After merge: the field seed and the Q-40-4 JAM path (2026-11-04, #451).** gc0 (`shard_9
field_story::gc0_the_raiders_barge_lifts…`) and cf6c (`shard_8 escort_lease::cf6c_a_leased_escort_flies_out…`)
went red on main once IN.5 (#414) merged after this cut.

- **The cause** is not IN.5's content. A field's arena seed was `match_::seed_for(ordinal, world_now, the
  process's salt)`, and IN.5's Freeport Works vendor row at Ceres (bisected on conclav.dev: stripping it alone
  turns gc0 green) moves `world_now` and the salt at admission. Under the new seed gc0's scripted engine hit
  draws this cut's JAM on the escort's plant: Q-40-4, power off, STARTUP refused. A probe read the escort's
  frame Online before the hit and Off after. cf6c's escort just lost its dice exchange. Both stories passed or
  failed on the seed's luck.
- **The knob**: `FieldConfig::seed: Option<u64>`, a test and replay knob, not in `[tacsim.field]`. `None` (the
  default, and production's) keeps `seed_for`. `Some(seed)` seeds every field a world admits, read once at
  admission (`field/admit.rs`). The field rigs pin it in `driving_rig::rig_config` (`FIELD_SEED`,
  `0x5EED_0002`), so a scripted field story's hits, jams and misses no longer hang on lane content or the
  world clock.
- **The stories**: gc0 names a jammed escort plant in its failure, since a tender's `unjam` is the only way
  back and the story has no tender.
- **cf6c's cause** (found after #460): no balance or effects bug. The escort is `tv_escort`, a line tank
  with the line tank's own sections, armour and guns, facing its intruder front to front, so the fight is a
  duel between identical hulls that the dice settle in about 30 s of arena time (cannon HE at 120 a round on
  a 6 s cycle). Neither side's content has changed since IN.1; FR.5's part effects and IN.5/IN.6's part lots
  touch neither. The defect was the story's reading of the fight: it ended the exchange on the black box's
  `hit` lines, which reach the host in batches. A probe saw the escort's first hit at arena 409 ms and the
  story learn of it only at about 33 s, by which time the duel was over. The exchange now ends on the
  escort's own cockpit frame, which shows it struck within a pulse (armour below its maximum). The intruder
  ceases fire with the escort, and the escort's own fired and landed rounds are asserted once the field has
  closed and the black box has caught up. cf6c is back on the PR tier.
- **Long term**: doc 43c's clock seam (TS.10) is this knob's home, with a world clock a test can pin.
- **cf6** (`shard_1 field_raiders::cf6_a_seeded_raider_fights_with_nobody_connected…`), also red on main
  without the pin, is the same luck in another form. The guard and a raider hull lost each other, the carrier
  fell, and the field demoted on the rig's 10 s disengage with that hull alive and healthy. A healthy hull
  leaves no row on the site record. The story's point is a fight to the raider's destruction, so its rig alone
  raises `disengage_s` to 60 (#460).
- **The ramp exit's boarding window** (built, PR #468, 2026-11-04). Before it, a
  raider's withdrawal was let go by the arena at its carrier's ramp and checked in beside it, and
  `raiders::board` ran only once the record named the `exit`, with no carrier check. A carrier wrecked inside
  that window, or before the release, left the hull out of the field "aboard" a wreck. The owner's ruling
  (2026-11-04, "back in the field"): a withdrawal counts only once the raider is aboard; the pad is never an
  exit. As built:
  - **Aboard in the batch that lets it go.** `exit::on_withdrawn` checks a raider's hull in and boards it
    (`raiders::board`: the WITHDRAW spent, off the ground) in the same batch. The record's `exit` is still owed
    and written as before; once it lands, the continuation for `Then::Raider` runs only the carrier's
    `lift_check`.
  - **A carrier lost before the boarding: the kernel keeps the hull.** `ArenaMsg::Withdraw` gains
    `carrier: Option<ObjId>`, named only for `Then::Raider`. `Arena::withdraw_to` refuses
    `WithdrawRefusal::CarrierLost` while that carrier is a wreck or not in the field, wherever the hull
    stands. It decides on the arena's live facts at the pulse head that would have let the hull go, so no
    wreck can land between the check and the release. The host strands the hull (`raiders::strand`): it sends
    the leader's order at sequence 2 with no directive, which lifts the WITHDRAW in the kernel so the doctrine
    fights on (to the death, the earlier ruling), then spends the hull's `withdraw` mark so the host stops
    asking. **Why this shape:** the arena never let the hull go, so nothing has to be undone or re-joined.
    The host alone could only read the carrier's condition off the last snapshot (a 300 ms decode, up to
    `snapshot_ms` stale), and a re-admit after a release would need a join, a home and a fresh row. A busy
    field leaves the mark set, and the next ask's refusal strands the hull again. A restart that resumes a
    snapshot from before the cancel would bring the WITHDRAW back with the mark already spent. That hull
    holds at its ramp, firing only when cornered, until the field ends; nothing exercises this.
  - **A carrier wrecked after the boarding takes its hulls with it.** In `close::demote_units`, a raider's
    fixture carrier checked in a wreck runs `raiders::wreck_aboard`: each hull of its raider aboard it (off the
    ground, launched from it, in no field, not remains) gets the arena's wreck mark on its own record
    (`carrier_damage::wreck_plan`), as GC.3 wrecks a programme carrier's bay rigs. It stays aboard, under the
    raider's mark naming the carrier, and nothing deletes it. `land_and_launch` never launches a wreck again.
    **A deviation from the brief's wording:** the brief asked for "a `Wreck` row on the site record". The site
    record's unit index holds only hulls on the ground (`keep_held_out` drops a row whose hull is off it), and
    GC.3's bay rigs carry no rows, so the hull is a Wreck on its own record and stays in the bay, not on the
    index. For Q-41-29's laden count, `tow::carried_kg` now weighs a fixture carrier's hulls aboard
    (`raiders::aboard_kg`, dry mass, wrecks included), since the carrier has no programme bay for
    `voyage::basis` to read. Salvage of the carrier's bay contents is doc 41's and is not touched here.
  - **Tests** (`shard_10 field_boarding::`, doc 43 #3, the shard with room):
    `boarding_a_raider_whose_carrier_is_lost_stays_in_the_field` (the carrier wrecked by staff hand before the
    field) was red before the fix, with both hulls out of the field and off the ground, and is green after (18
    s). `boarding_a_raider_aboard_a_carrier_wrecked_later_is_a_wreck_in_the_bay` (a grounded carrier, one hull
    withdrawn and one aboard, then B's fire takes the carrier's hull) was red before at "aboard in the batch
    that let it go", with the hull standing on the site's ground out of the field, and is green after (26 s).
    `field_raiders::cf6b…` and `field_story::gc0…` wait for the record's exit as well as the boarding, because
    the boarding now comes first. `field_raiders::`, `field_carrier::`, `field_fr5::`, `field_gc4::` and the
    lib's `field::` and `withdraw` tests pass.
  - **The player's hull at the ramp has the same window. Not fixed here.** `+tv/<n> recover`
    (`Then::Rig`), `+sortie/recover` (`Then::Sortie`) and FR.5b's coupled load (`Then::TowLoad`, via
    `surface/tow.rs` `field_load_cmd`) are all let go with no carrier check. Their recover or load runs as a
    later step, once the record's exit lands. A player's carrier wrecked before the release, or inside that
    window, leaves the rig beside the ramp, out of the field. The recover act does not refuse a wrecked
    carrier, so the rig then goes into a wreck's bay, and GC.3 wrecks it only if the carrier checks in later.
    The player keeps agency, since the record names the exit and their own recover passes. The fix is not the
    same small change: the recover and the load are full surface acts with their own refusals and durations,
    so they cannot run inside `on_withdrawn`'s batch. Naming the carrier on the ask would close only the
    before-release half, and it would change a player-facing refusal with no ruling. The question is the
    owner's.

## FR.5b — the coupled tow's surface half (§4.1 item 5; Q-40-7, Q-40-25)

PR #433, off `main` after #410 and RS.5 (#415). #410 merged with story (a)'s stale expectation
(`field_gc4.rs:317`: "the casualty rides as its tug's load"), so main was red on `tacsim-integration-1` until this
PR.

### What was built

- **The trailer.** While a field runs, the kernel trails the casualty. `tow::follow` does nothing while the tug or its
  casualty is in a running field, and the casualty checks in at its own row's pose.
- **The drop.** `tow::set_down` sets the casualty down at its own pose and never on its tug's cell. Lane 3's P0-8
  (the tug falls irrecoverable) stays green.
- **Release as a field act.** `+tv/<n> release go` on a tug in a field goes to `field_release_cmd` (`port/tv.rs`):
  - It sends `offer_field_release` and answers `FIELD_RELEASE_ORDERED`, or `FIELD_BUSY` when the queue is full.
    The preview line is `FIELD_RELEASE_PREVIEW`.
  - `exit::on_released`, on `Ok`, submits `TowOrder::Release { site, field: true }` in the same batch. The tow
    retires on the site record and the pilot gets the ordinary `RELEASED` line.
  - On `Err` it tells `FIELD_RELEASE_REFUSED`.
  - A re-attach inside a field still waits (Q-CF-30).
- **The coupled withdraw at the ramp** (Q-40-25). `+tv/<n> load go` on a tug in a field goes to `field_load_cmd`:
  - It checks the bay's room first, then `exit::request_coupled` sends `ArenaMsg::Withdraw` for the casualty and
    the tug with the ramp's position.
  - Two new `Then` arms, `Towed` and `TowLoad`, follow the pair out. A refusal is told once, by the tug's answer;
    `NotAtRamp` reads `tow::NOT_AT_RAMP`.
  - Once both exits are on the record, the exit tick runs `tow::load_cmd` as the player. `TowOrder::Load`/`Loaded`
    then run as on open ground, and `complete_load` is untouched.
  - `+tv/<n> recover` and `+sortie/recover` still refuse a tug in tow.

### Stories (conclav.dev)

`tests/shard_10/field_fr5.rs`, new, each at Ahuna Flank (256) and Urvara Reach (512):
- the casualty struck while towed, then released in the field by `+tv/<n> release go` (109 s each);
- the tug falls and the casualty is dropped where it stands (50 s each);
- a coupled pair withdrawn at the ramp by `+tv/<n> load go` under fire and loaded into the bay (69 s and 54 s).

Story (a), `field_gc4::gc4_the_wingman_drags…`, keeps its claims. Three asserts move, each commented "Doc 40 cut
FR.5": the casualty is in the field, it stands beside its tug rather than on it, and it is indexed at its own cell.
It ran in 204 s.

`field_fr5`, `field_gc4`, `field_carrier` and `field_gc2` passed 20 of 20. `mush-contract-tests` and `tacsim --lib`
passed 2,208.

### Departures

- **Own carrier only.** The coupled load in a field offers only the rig's own carrier's ramp. The host cannot read
  the tug's live pose against a Friend's carrier.
- **A fixture instead of a first field.** The new stories lay the casualty on the site's index with the staff fixture
  (GC.5's precedent) instead of fighting a first field. Story (a) still fights its first field.
- **Hits read from the ledger.** The stories set `[tacsim.arena.ledger].summary_ms` to 100 (FR.4's knob), so the
  hit lines reach the host while the field runs.
- **The tug-falls story checks at demotion.** Once the tug was destroyed, the field closed within seconds in every
  run, so the drop is read at demotion. The kernel gate `fr5_a_tug_destroyed_leaves_its_casualty_standing` holds
  the uncoupling itself.
- **One public reader.** `tow::field_stand(w, site, unit)` reads a unit's cell and `coupled_to` from the field's
  last snapshot, for the stories.
- **The release story's placement.** At Ahuna the powerless casualty was never seen at ramp + 3 from the stranger's
  hull, so it stands at ramp + 4 and its tug at ramp + 5.

## FR.7 — the round's story (Q-38-29; §4.7, §6's FR.7 row)

PR #420, stacked on FR.5 (#410). One script, `crates/mush-bot/tests/mining_cohort/fr7_round.rs`, run as
`fr7_round::fr7_the_rounds_story_at_256` (Ahuna Flank) and `…_at_512` (Urvara Reach), over the two `Ground`s of
`crates/tacsim/tests/support/field/grounds_rig.rs`, which the bot's `mining_cohort` binary declares by path (two
one-constant shim modules give it the `driving_rig::AHUNA` and `field_raiders_rig::SKULKS` it names).

### What the script does

The production root is booted in the test binary (`mine::boot_with`, FR.0's smoke's rig). Every player line goes
over that player's own SSH session in line mode, after the bot's own closed vocabulary admits it:
`mine::lane::field::in_vocabulary` for the captain, `…::escort::in_vocabulary` for the Basin pilots. Answers are
read by the bot's parsers where it has them: `own_party`, `field_contact`, `void_offered`, `void_answer`, and the
arena classifier's weapon rows.

- **The cast.** Concord's captain is the bot's own hired-crew lane (`Lane::with_detachment`), driven in-process over
  its session exactly as `--mine-detachment` plays it. It kits up at Ceres, hires Ilya Mendes for rig 1, takes rig 2's
  cab, drafts, accepts, readies and activates the operation, and hops to the site. The script stops the lane where
  it would type `+sortie/launch <op>=all` and types the captain's lines from there (FR.0's precedent). Basin's raider
  and wing each fly an armed hull on the ground.
- **Beat 1, the raid of a declared-Hostile co-op (FR.1).** The raider types `+faction/stance Basin=Concord/hostile`.
  The captain's `+sortie/launch <op>=1` puts rig 1 down the ramp, read through `+sortie/operation` ("last confirmed on
  the surface"). The raider's `+field/mark W=hostile` on open ground opens the field. The live record holds both Basin
  parties' `stance_hostile` naming Concord's operation party from admission, and the raider's `CONTACTS`/`LOCK`/`FIRE
  machine_gun` lands on Concord.
- **Beat 2, reinforcement from the pad (FR.3).** The captain's `+sortie/launch <op>=2` into the running field. Rig 2,
  the captain in its cab, joins Concord's party on the live record, and the captain's `+field` reads that party.
- **Beat 3, the tow (FR.5): pending**, below.
- **Beat 4, a jammed sight (FR.5 item 4).** The wing flies the fixture design `fr7.unit.jamming_tank`, worn to its
  armour floor: the shipped line tank under its own pack and unit id with `component_hits` at a jam every survivable
  hit (`jam_permille` 1000, no damage). The raider's machine gun on it jams the turret's parts one a hit until the sight
  is one. The field's snapshot (`tacsim:field:snapshot:<site>`, read from the store) then holds the wing's
  `UnitState.sensor` null.
- **Beat 6, a segment rolled (FR.4).** `[tacsim.arena.ledger].max_bytes` is set to 1 MiB and `summary_ms` to 1 s, the
  production floors: `ArenaConfig::validate` refuses less at boot, so FR.4's 48 KiB harness knob is not available to
  a production root. After the jam the raider types nothing, and the field waits on its idle bound (`idle_demote_s` =
  `disengage_s` = 3,300 s) until the live record reads segment 2.
- **Beat 5, friendly fire voided at once (FR.2).** Raider and wing are both Basin, so the raider's rounds on the wing
  are friendly fire. The field demotes on its idle bound, and the raider, who has typed nothing since the jam, is
  offered the void. Its next line, `+field/void`, is answered `Voided as friendly fire`.

**What is a fixture** (as FR.1's and FR.2's): at boot, before the world task, a staff hand makes, places and seats
the two Basin hulls and wears the wing. `@faction/create` and `@faction/add` make and staff the two factions. The
jamming design is a test pack on `[tacsim].pack_paths`, signed with the digest `tacsim::pack::load_pack` computes.
Everything after boot is a player's command over SSH. The world is only read, from the store (the live record, the
field snapshot, the chunk count).

### The tow beat's seam (FR.5's `surface/tow.rs` half, PR #415)

Two async functions in `fr7_round.rs`, each marked `// FR.5 tow half pending:` and each answering `"pending"` today:

- `tow_struck_before_field`, called after rig 1 is on the surface and before the raider's mark. It is to type a tug's
  `+tv/<n> tow go` on a casualty beside the ramp (read through `lines::tow_line`) and assert that admission carries
  the pair coupled: the casualty an entrant on its tug's party, in `FieldHost.tows`, not counted.
- `tow_released_under_fire`, called after the reinforcement joins. It is to have the raider `LOCK`/`FIRE` on the
  towed casualty until a round lands, then the tug's pilot types `+tv/<n> release go` (admitted by
  `field::round_word`), reads the `Released` line, and asserts the casualty stands uncoupled where it was dropped.

What it needs from FR.5: the open-ground tow coupling into the field (`surface/tow.rs`), and a rig the story can disable
by the production path to be the casualty. Nothing else in the script depends on the seam.

### Runs (conclav.dev, by hand as `ghrunner`, throwaway clone, `CARGO_TARGET_DIR=/home/ghrunner/fr7-target`)

**Green at both sizes on head `6ccaf77`**, run r4, the two sizes side by side, 2026-11-04 (logs
`/home/ghrunner/fr7-256-r4.log` and `fr7-512-r4.log` on the box):

| Beat | Ahuna Flank (256) | Urvara Reach (512) |
|---|---|---|
| The captain's lane at its launch (kit-up, hire, operation, hop; rate lifted to 168 after enrolment) | +342 s | +352 s |
| 1. `Basin now stands Hostile toward Concord`; the mark opens field 1; both Basin parties' `stance_hostile` = `["a"]` (Concord's operation party) at admission; raider's fire lands on Concord | green | green |
| 2. `+sortie/launch 1:1=2` into the field; rig 2 joins party A; the captain's `+field` reads `Your party is A` | green | green |
| 3. the tow | pending (the seam) | pending (the seam) |
| 4. the wing's snapshot `sensor` null | after 10 shots, +405 s | after 13 shots, +427 s |
| 6. live record on segment 2 (48 / 47 chunks shipped) | +3528 s | +3576 s |
| 5. field settles; the raider's first line, `+field/void`, answers `Voided as friendly fire: 1 standing change(s)` | +3703 s | +3726 s |
| `test result: ok` | 3,704 s | 3,727 s |

How the earlier runs failed, and what changed:

- **r1** (`ceec1323`): beats 1, 2 and 4 green at both sizes. The roll never came, because the field demoted about
  four minutes after it opened, once the rigs were wrecked. FR.4's 48 KiB knob is below the production floor, so the
  roll needs about forty field minutes.
- **r2** (`ca529e1`): showed the fill rate, about one 16 KiB chunk a minute for this five-hull field. It was stopped
  by hand, then re-run with a 3,300 s idle bound.
- **r3**: killed at +780 s by another session's `pkill` of `mining_cohort` processes on the shared box. Since then,
  `fr7-run.sh` runs a renamed copy of the test binary (`/home/ghrunner/fr7-bin/fr7story-<size>`).

Contract guards, `cargo clippy -j 8 -p mush-bot --all-targets -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc
-p mush-bot --no-deps` and `cargo nextest run -p mush-contract-tests` (163 passed) were run on conclav.dev in a
separate target dir. The Windows box's build slot was held for FR.5 throughout.

**Not run on gate.yml.** §6's acceptance names gate.yml. These runs are by hand on the same box, because gate.yml's
sweep would build the whole workspace to run two hour-long ignored tests. Once TS.2's manifest lands, a
`--extra "--run-ignored only -E 'test(/fr7_the_rounds_story/)'" --packages mush-bot` dispatch is the recorded way
to run it.

### Tier

#420 merged before TS.2's manifest reached its branch, so it merged without rows; FR.7b (#454) adds them, on the
by-hand tier (see FR.7b). The story is `#[ignore]`d, run by name, about an hour a size, and the two sizes run side by
side:

`cargo test -j 8 -p mush-bot --test mining_cohort -- --ignored fr7_ --nocapture`

### Cell minutes (rule #14)

**+0 summed minutes on every CI cell.** Both tests are `#[ignore]`d, and no PR cell runs ignored `mining_cohort`
tests. The binary gains one module and no default test. The dispatch costs about 60 minutes of wall time per size on
conclav.dev.

## FR.7b — the tow beat, and FR.7's tier rows

PR #454, off `main` after FR.7 (#420, merged at `6ccaf77e` with the tow seam still pending and without its tier
rows) and FR.5b (#433).

**The rows** (`crates/mush-contract-tests/test-tiers.toml`). `mush-bot/tests::fr7_round::fr7_the_rounds_story_at_256`
and `_512` are on the by-hand tier at the revision of their last green run. Notes on the rows:
- The id is the module path. The inventory refuses a `mining_cohort::` id.
- They are not on the dispatch tier because the manifest accepts dispatch only for a dispatch-only ci.yml job or a
  gate.yml kind with a `via` script. Every ci.yml job also runs on the schedule, and the `sweep` kind has no `via`.
- Moving them to dispatch means a gate kind and its script. That is not built here.
- With the rows in, `ci_gated_test_inventory` on conclav.dev names only IN.5's two `in5_…` rows, which belong to
  IN.5's session.

**The tow beat** (`fr7_round.rs`, `tow_struck_before_field` and `tow_released_under_fire`, the seams FR.7 marked):

- **The casualty** is a stock rig (`tacsim.unit.tv_prospector`, "Stranded Rig") placed at boot one cell east of the
  ramp with its power destroyed. Staff then index it as a casualty on the site's record with `@tacsim/site <site>
  leave #<unit> casualty <x>,<y>` over God's SSH session. This is GC.5's fixture, as `field_fr5` uses it. No player
  path disables a rig outside a field, which is what this fixture stands in for.
- **Struck before the field.** The captain's `+tv/1 tow go` (the bot's GC.4 tow line, admitted by
  `escort::in_vocabulary`) has rig 1, which came down the ramp, take the casualty in tow. `lines::tow_line` reads it
  as `Attached`: "Rig 1 has the Stranded Rig in tow, at 1.6 m/s."
- **Admitted coupled.** After the raider's mark, the field's snapshot holds the casualty `coupled_to` rig 1
  (Q-40-7).
- **Struck in tow.** The raider locks Concord's contacts in turn and fires until the snapshot reads the casualty's
  `own_last_hit_ms` while it is still coupled.
- **Released under fire.** After one more burst, the captain types `+tv/1 release go` (`field::round_word`) and reads
  "Rig 1 lets go of the Stranded Rig at field 1's next pulse." The snapshot then stands the casualty with no
  `coupled_to`, within two cells of where it was struck.

**What moved from FR.7, and why:**

- **The release comes before beat 2's launch.** Once rig 2 flies, the captain sits at its helm, and at a helm `+tv/<n>`
  names only that rig (`port::tv::select_word`), so `+tv/1 release go` answers "There is no rig 1 in the … bay".
- **The lane's launch is held back.** The captain lane's `+sortie/launch <op>=all` could open and be sent in one
  tick, past FR.7's check of the open step. When that happened, rig 2 left with slot 1, and the beat-2 launch only
  re-confirmed a pending departure. The driver now refuses to send that line. The opening asserts that rig 2 is not
  yet in the field.
- **The raider's approach depends on the ground** (`approach`). At Ahuna Flank the ramp's row is out of the raider's
  sight from the east: across every FR.7 run there it saw only the carrier. It now comes from twelve cells south of
  the ramp, facing north, and marks `N`. Four approaches were probed on conclav.dev in parallel: from the south,
  from the west, and east-north-east all saw the casualty; from the east-south-east, the raider saw only its wing.
  Urvara keeps the east approach (`W`).
- **The raider no longer fires while rig 2 comes down.** The field's idle bound holds it open, and the extra rounds
  had wrecked rig 1, the tug.

**Runs** (conclav.dev, head `c969ec3f`; the code is `37276a81`'s, with docs and the nextest override on top). Both sizes
are green by hand and through gate.yml.

| Beat | Ahuna Flank (256), by hand | Urvara Reach (512), by hand |
|---|---|---|
| The lane's own `=all` held back | +343 s | +352 s |
| 3a. rig 1 takes the Stranded Rig in tow | green | green |
| 3b. coupled at admission; struck in tow; released under fire, uncoupled where struck | (129, 124) | (257, 252) |
| 4. the wing's sight dark | after 8 shots, +419 s | after 6 shots, +420 s |
| 6. segment 2 | +3290 s | +3229 s |
| 5. `Voided as friendly fire` | green | green |
| `test result: ok` | 3,719 s | 3,720 s |

- **Through gate.yml**: run [41904925552](https://github.com/conclavio/rustmush/actions/runs/41904925552), dispatched
  with `scripts/vps-gate.sh --runner conclav-dev --packages mush-bot --extra "--run-ignored only -E
  'test(/fr7_/)'" --no-push`. Both tests passed under the `ci-bot` profile, 256 in 3,712 s and 512 in 3,742 s. The
  run is a success, which meets §6's "green on gate.yml".
- **The profile override.** `.config/nextest.toml` gains a `ci-bot` override for `fr7_round::`
  (`terminate-after = 100`). Without it, the profile's 20-minute limit would end an hour-long story.
- **The runs before these** (r5–r7 and the four Ahuna approach probes) found the release-at-the-helm refusal, the
  lane's own launch, the Ahuna sight line and the fire that wrecked the tug, each fixed as listed above.
- **Contract test.** `mush-contract-tests` on conclav.dev fails only on IN.5's two missing rows.

**Cell minutes (rule #14): +0.** The tests stay `#[ignore]`d, and no PR cell runs them.


### FR.7c — the ledger floor (PR #461; the owner's approval of a test-only floor)

- **The seam.** `tacsim::store::lower_ledger_floor_for_test(w, max_bytes, summary_ms)` sets `[tacsim.arena.ledger]`
  on the layer's frozen block after the boot has validated it. Every arena started afterwards reads that block
  (`match_::lobby::configs`).
- **How it is gated:**
  - It compiles only under `cfg(test)` or tacsim's new `test-ledger-floor` feature. The feature is off by default,
    and `mush-bot`'s `[dev-dependencies]` edge alone turns it on (`feature_matrix_inventory`: census 16, allowlist
    row). No shipped binary carries it.
  - No config key reaches it. Only Rust code holding the booted world calls it.
  - `ArenaConfig::validate`'s floors (1 MiB, 1,000 ms), its refusal of a production config below them, and the
    roll threshold (three quarters of `max_bytes`) are all unchanged.
  - It is a free function rather than a `TacsimStore` method, so the test file writes no other module's state
    slot (`state_slot_owners`, `fiat_budget`).
- **The story.** FR.7's rig calls the seam in its `prepare` closure with FR.4's 48 KiB and a summary every 100 ms.
  The idle bound falls from 3,300 s to 180 s, and the story's timeout from 100 to 30 minutes.
- **Runs** (conclav.dev, by hand, renamed binaries, head `a93f3d2d`; the seam's move to a free function after it
  changes no behaviour):

  | | Ahuna Flank (256) | Urvara Reach (512) |
  |---|---|---|
  | Lane at its launch | +340 s | +351 s |
  | Sight jammed | after 7 shots, +405 s | after 1 shot, +401 s |
  | Live record's segment by then | 4 | 3 |
  | Result | green, 585 s | green, 580 s |
  | Re-run at the PR head `28e0a372` (the free-function seam) | green, 599 s | green, 626 s |

  That is about ten minutes a size, down from 3,719 s, and about 340 s of it is the captain lane's clock-bound
  kit-up and hop.
- **The tier.** Both rows move from by-hand to **nightly**, on a new ci.yml job, `fr7-round-story`. The job runs
  `cargo nextest run -P ci-bot -p mush-bot --test mining_cohort --run-ignored only --test-threads 2` with
  `-E` naming both tests, on the schedule and on dispatch, `timeout-minutes: 90`. It is also in `nightly-report`'s
  `needs`, and the ci-bot override from #454 still lets each test outlive twenty minutes.
- **Cell minutes (rule #14).** +0 on every PR cell. The nightly gains one job: the mush-bot test build plus about
  ten minutes with the two sizes side by side.
