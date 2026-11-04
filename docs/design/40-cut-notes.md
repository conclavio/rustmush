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

**Cell delta.** One lib unit test added, no story added or lengthened: the tacsim lib cell and shard 10 are
unchanged within noise (to be read from the PR run's JUnit).

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
- **Grounds**: FR.0's hoist of `field_story.rs`'s grounds into `tests/support/field` had not
  landed, so the story carries its own two-row table (cells from `field_gc10` and `field_story`).
  `driving_rig::en_route_at` is the one support addition (`en_route` for any site).
- **Shard** (doc 43 rule #3): shard_2, the cell with the room (11 summed minutes in doc 43 §10).
- **Summed-minutes delta** (rule #14): **+2.9 min** on `tacsim-integration-2` (the two stories 84.8 s + 86.5 s on the PR run 41730885044 JUnit); the cell now sums 23.0 min (shard_2 14.2 + cost_lane), its wall 17 m 0 s, under the twenty-minute line.
