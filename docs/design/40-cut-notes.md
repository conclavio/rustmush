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

