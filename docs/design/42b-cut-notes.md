# 42b. Cut notes — surrender

One `## SU.n` heading per cut of [42b-surrender.md](42b-surrender.md) §7 (`SU.0`–`SU.6`), in the order they
landed: what was built, the gate that proves it, where the PR departed from §7's row, and the cell's
summed-minutes delta ([doc 43](43-testing-strategy.md) rule 2.3 #14).

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
- The harness kind on conclav-dev (doc 43 rule 13), because the cut edits `arena/state.rs`. See below.

**Doc 40 §4a's pins.** Expected unmoved, as Q-42b-11 says, and none moves:
- The cut adds no draw, no `UnitState` or snapshot row, no pack byte and no API bump.
- The only kernel behaviour touched is the boolean, and its body is the same predicate moved into a helper.
- No pin is re-blessed. This cut shares no batch with a rebless (Q-36-56). FR.5's rebless is merged, and
  no other is open.

**Departures.** None from §7's row.

**Cell delta.** One in-source test of about 0.2 s in the tacsim lib binary.
