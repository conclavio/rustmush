# 42b. Cut notes — surrender

One `## SU.n` heading per cut of [42b-surrender.md](42b-surrender.md) §7 (`SU.0`–`SU.6`), in the order
they landed: what was built, the gate that proves it, where the PR departed from §7's row, and the cell's
summed-minutes delta ([doc 43](43-testing-strategy.md) rule 2.3 #14).

## SU.0 — the trait on the record

Q-42b-5. No behaviour a player can reach changes: nothing writes the field until SU.1's seed, and nothing
reads it until SU.3's check.

**The record** (`crates/tacsim/src/field/crew.rs`).
- `CrewRecord.resolve_permille: Option<u16>`, 0 to 1,000 per mille, `serde(default,
  skip_serializing_if = "Option::is_none")`. `None` never yields: every player record and every record
  written before SU. `CREW_FORMAT` stays 1, as `Extraction::forced` did.
- Carried unchanged by every writer. The three that build a record afresh take the prior record's
  value: `check_in`, `on_frames` (a pilot incapacitated in the hull) and the eject
  (`field/eject.rs`). `capture`, the handover, the release and the extraction already rebuild with
  `..record`.
- `status_line` is untouched: the trait is never shown.

**Gate.** The in-source test `su0_resolve_on_the_record_decodes_under_the_same_format`. An old record
reads `None` and re-encodes byte-for-byte. A set value round-trips. A capture carries it. An unknown
neighbour field is still refused (`deny_unknown_fields`). The existing CR.0 decode test is unchanged.

**Departures from §7's row.** §7 names only `field/crew.rs`. The eject's writer (`field/eject.rs`) and the
custody rig's literal (`tests/support/field/custody_rig.rs`) also build a `CrewRecord` field by field, so
each names the new field.

**Cell delta (rule #14).** None. The one new test is an in-source unit test in the tacsim lib; no domain
binary or shard cell changes.

## SU.1 — raider crews

Q-42b-1, Q-42b-2, Q-42b-6, Q-42b-7. Raider hulls now carry a character to capture. Nothing reads the
resolve yet (SU.3).

**The content** (`crates/tacsim/content/field/raiders.json`, schema 5).
- `resolve_permille` on each raider: Skulk raiders 400, Reach raiders 750 (provisional).
- An optional `resolve_permille` and `crew` (a name) on each unit. Neither shipped raider names its
  crew, so they read "Skulk One crew" and so on.
- `raiders()` takes schema 5 only. It refuses a resolve over 1,000 and a blank crew name. A missing
  value reads `DEFAULT_RESOLVE_PERMILLE` 500 (`RaiderDef::resolve_of`: the unit's, else the
  raider's, else 500).
- The brief grows its schema-5 sentence. The pack is in neither the lane nor the relay digest, so
  nothing is re-signed.

**The seed** (`field/raiders.rs`, `crew`). For each armed hull, never the carrier (Q-42b-3):
1. A fresh `ObjKind::Npc`, GOD-owned, created inside the hull. It goes where the hull goes: aboard,
   launched, wrecked.
2. A `cockpit` station on the hull (`CREW_STATION`, the cockpit channels, no console room), and the
   NPC assigned to it through `mush_sim::commands::assign_station`. `parties::seated` reads that
   seat, so the field admits the hull with a `Pilot` row. `admit` keeps an NPC seat's hull
   `Controller::Mind`, so the ROM seal flies it as before.
3. A crew record `Unit { unit: hull }`, `Fit`, the leader as organizer, the authored resolve.

A re-seed (refused while a hull stands) makes fresh crew. The director seeds through the same
`seed`, so director raids carry crew too.

**The check-in** (`field/crew.rs`). `check_in` now takes a player, or an NPC whose record carries a
resolve. The raider seed is the only writer of a resolve. A hired NPC pilot is still left to its
contract (Q-42b-2, Q-42-26), even after `on_frames` has written it incapacitated.

**Gate.**
- The in-source test `su1_schema_5_authors_each_crews_resolve` covers the shipped values, the
  fallbacks, the default name and the refusals.
- CF.6's raider story (`field_raiders::cf6_…`) now also asserts:
  - after the seed, each hull holds one seated NPC with a `Unit` record, the leader as organizer
    and resolve 400, and the carrier has none;
  - after the fight, each crew member checked in from their wreck with the resolve carried.
- `field_raiders::`, `field_director::` (both gc11 stories), `field_fr2::`, `field_gc10::`,
  `field_boarding::`, `testimony::` and `field_story::` are green with crew aboard.

**Departures from §7's row.**
- `on_frames` is not widened: it already writes any seated pilot it finds incapacitated, NPCs
  included. Since SU.0 it carries the prior record's resolve.
- The `check_in` widening keys on the resolve rather than on any NPC with a record, so a hired
  pilot's `on_frames` record stays inert. That is Q-42b-2's rule.

## SU.2 — the reach count

Q-42b-4's odds term and Q-42b-11's read. Nothing calls the read yet. SU.3's `lift_check` is its first
caller, so no behaviour a player can reach changes.

**The read** (`crates/tacsim/src/arena/state.rs`, beside `party_holds_armed_hostile_in_reach`).
- `Arena::armed_hostiles_in_reach(id: ObjId) -> u32` counts the contacts hull `id` holds that pass four
  tests. The contact is Hostile to the hull's party. Its target came armed or has fired. It is not read
  dead. It was seen inside the hull's own `fight_inner_cm`. The store keeps one contact id per target
  (`ShadowContacts::id_of`), so the count is of units.
- A wrecked hull, a hull with no reach and an id the arena does not hold all read `0`.
- It is a read, never a step: no state moves and nothing is hashed.
- The per-contact test is now one private helper, `armed_hostile_held_in_reach`. The party boolean and the
  count both call it, so the two cannot drift. The boolean's answer is unchanged: the same four tests run in
  the same order behind the same `reach != 0` and wreck filters.

**SU.3's odds term** subtracts the raider's own standing hulls from this count (Q-42b-4). That subtraction
is SU.3's to make, in `lift_check`, not the kernel's.

**The gate.**
- The in-source test `arena::state::tests::su2_the_reach_count_counts_armed_hostiles_in_the_hulls_own_reach`
  holds one RED line tank and five contacts. A teammate is not counted. A never-armed barge is counted only
  once it has fired. A plot past the reach is not counted. A contact read dead stops counting. A wreck, a
  gunless hull and a stranger read `0`. At every step the party boolean equals "some hull of the party
  counts more than none".
- The harness kind on conclav-dev (doc 43 rule 13), because the cut edits `arena/state.rs`. It was green at
  `a5348999`, run [42059596120](https://github.com/conclavio/rustmush/actions/runs/42059596120).
  `tacsim_harness_delta_against_the_committed_baseline` passed against `arena.linux.json` (blessed at `a2ef4075`)
  with every deterministic leaf equal and the four `minds` blocks equal. The timing gate is live on that host, and
  every compared timing stayed inside 20%.

**Doc 40 §4a's pins.** Expected unmoved, as Q-42b-11 says, and none moves. The harness baseline is unmoved (above),
and the pin tests run in ci.yml's tacsim shards on this PR:
- The cut adds no draw, no `UnitState` or snapshot row, no pack byte and no API bump.
- The only kernel behaviour touched is the boolean, and its body is the same predicate moved into a helper.
- No pin is re-blessed. This cut shares no batch with a rebless (Q-36-56). FR.5's rebless is merged, and
  no other is open.

**Departures.** None from §7's row.

**Cell delta.** One in-source test of about 0.2 s in the tacsim lib binary.
