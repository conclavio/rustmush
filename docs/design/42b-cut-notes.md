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
