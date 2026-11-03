# 40. Cut notes — the field, second round

One `## FR.n` heading per cut of [40-field-second-round.md](40-field-second-round.md) §6, in the order they
landed: what was built, the test that proves it, where the PR departed from the row, and the cell's
summed-minutes delta (doc 43 rule 2.3 #14).

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
