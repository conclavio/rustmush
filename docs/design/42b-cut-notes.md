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

**Cell delta (rule #14).** No story is added. `field_raiders::cf6_…` gains two read-only assertion loops
over the seeded crew (no extra pulses), and the tacsim lib binary gains one in-source test of well under a
second. No domain binary's summed minutes move beyond run-to-run variance.
