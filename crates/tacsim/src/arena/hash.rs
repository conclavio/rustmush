//! **The per-substep state hash** (16b cut A.1; doc 16 §3.1 "Determinism", §4
//! "Ledger events").
//!
//! *Each substep produces a state hash recorded with its pulse.* That one line
//! is what turns rate independence from a claim into a test: the cross-tempo
//! property test (16b A7) replays one scripted intent and control log at 5,000,
//! 1,000 and 500 ms and asserts an identical **sequence of these hashes**, so a
//! tempo that changed an outcome is caught at the substep it diverged in rather
//! than at the end of the match.
//!
//! For that to work the hash must be a function of the *quantised* state and of
//! nothing else — no addresses, no iteration order that depends on a hash map's
//! seed, no wall time, no floating point. Every byte folded in below is an
//! integer read in unit-id order, and the mix is written out here so that the
//! answer does not move when the toolchain does.

use crate::arena::state::Arena;
use crate::deliberation::order::Objective;

/// FNV-1a 64's offset basis.
pub const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a 64's prime.
pub const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Fold `bytes` into an FNV-1a 64 accumulator.
///
/// Public because [`crate::arena::rng::draw`] keys on the same primitive: one
/// mix, written once, testable on its own.
#[must_use]
pub fn fnv1a64_update(state: u64, bytes: &[u8]) -> u64 {
    let mut hash = state;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
thread_local! {
    /// **How many times this thread has taken [`state_hash`]** (the M.3
    /// review's witness for 16c `T-23`), in test builds only: the lazy phase-4
    /// slot (`Arena::action_hash`) is closed again before a pulse returns, so
    /// whether phase 4 took a hash for nothing is visible only as a count. No
    /// outcome reads it, and it does not exist outside `cfg(test)`.
    pub(crate) static STATE_HASHES_TAKEN: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };
}

/// **The state hash** of an arena, as the ledger records it.
///
/// FNV-1a 64 over these bytes, in this order, every multi-byte integer
/// **little-endian**:
///
/// 1. For each unit, **in unit-id order** (the order [`Arena::units`] is kept
///    in):
///    1. the object number, `u32`;
///    2. `pose.x_cm`, `i64`;
///    3. `pose.y_cm`, `i64`;
///    4. `pose.z_cm`, `i32`;
///    5. `pose.heading_mdeg`, `u32`;
///    6. `speed_cms`, `i32`;
///    7. the power byte (`0` off, `1` starting, `2` online);
///    8. **the five limit bytes** (16b cut M.4, 16c `B-47`; four until then, when
///       one byte named at most one closed side): `slow_pct`; `no_turn_left`
///       (`0`/`1`); `no_turn_right` (`0`/`1`); `motion_disabled` (`0`/`1`);
///       `wreck` (`0`/`1`);
///    9. **what it is held against** (16b cut M.3, 16c `T-04`):
///       [`crate::arena::state::UnitState::held`] as a presence byte, then the
///       held cell's `x` and `y` as `u16` and the cause as one byte
///       ([`crate::arena::motion::BlockedCause::code`]: `0` impassable, `1`
///       occupied, `2` edge, `3` too steep) — `[0]` alone for none;
///    10. **the lock** (16b cut W.1): the contact id as `u64`, or `0` for none, which
///        is a value [`crate::arena::rng::draw_contact_id`] never mints;
///    11. **the weapon cells** (16b cut W.1), in the two `BTreeMap`s' own key order:
///        the count of `ready_at_ms` rows as `u64`, then per row the weapon id's
///        bytes and its `i64`; then the same for `ammo` with its `u32`. The id's
///        bytes go in because a row is a `(weapon, value)` pair and folding only the
///        values would make two weapons that swapped cycles one state;
///    12. **the controller byte** (16b cut N.1): `0` pilot, `1` reflex;
///    13. **`grade_pct`** (16b cut X.6), `u16` — the uphill slope of the edge the
///        unit entered its cell by;
///    14. **the traffic hold** (16b cut E.3): a presence byte, then the held
///        cell's two `u16` and the `i64` arena millisecond it opened at;
///    15. **the motion carries** (16b cut Q.6), in field order: `carry.turn`
///        as `u32`, `carry.speed` as `u32`, `carry.east` as `i64` and
///        `carry.north` as `i64`; then (16b cut M.4, 16c `T-16`) the two sign
///        bytes `carry.turn_side` and `carry.speed_rate` in one call — which way
///        the turn carry was banked and by which rate the speed carry was, so a
///        replay about to zero a carry on a flip differs from one that is not;
///    16. **the platform's own receiver** (16b cut D.4): a presence byte, then
///        `budget` and `budget_max` as **one byte each** (the fold writes
///        `[1, budget, budget_max]` in one call), then `reach_permille` as `u16`
///        (16b cut M.3 corrected this line, which typed all three `u16`; 16c
///        `T-80`);
///    17. **the seat-order rate** (16b cut M.1, `T-82`): `orders_window_s` as
///        `i64`, then `orders_in_window` as `u32`. Persistent state that decides a
///        refusal notice is hashed: a replay whose pilot's order landed in a
///        different arena second would refuse a different later one;
///    18. **heat** (16b cut M.3, 16c `T-06`): a presence byte — `1` when
///        [`crate::arena::state::UnitState::heat_now`] answers — then that heat
///        as `u32` and [`crate::arena::state::HeatState::carry_milli`] as `u32`;
///        `[0]` alone for a unit that carries no heat, which is every stock tank;
///    19. **the pod's switch** (16b cut O.2, 16c N12): one byte,
///        [`crate::arena::state::UnitState::ecm_on`] (`1` radiating, `0`
///        silent), written for every unit whether it carries a pod or not. It
///        decides what every sight inside the pod's bubble sees — detection and
///        both ends of the mesh — and it is authoritative state with one writer
///        ([`crate::arena::intents::resolve_emcon`]), so a replay whose pilot's
///        switch landed a pulse later must not compare equal even before the
///        next pass shows the difference in a contact.
/// 2. **The spent-section window** (16b cut D.2), in the map's own key order: the
///    count of entries as `u64`, then per entry the platform told's `u32`, the
///    target's `u32`, the section id's bytes and the arena millisecond as `i64`.
/// 3. **The reflex machines** (16b cut N.1), in unit-id order — the map's own, so
///    there is no iteration order to diverge on: per machine the unit's `u32`, the
///    state byte, the goal cell's two `u16` (`u16::MAX` twice for none), the
///    waypoint index as `u16` (`u16::MAX` for none), the route's length as `u64`
///    and the target unit's `u32` (`0` for none); then the standing order (16b
///    cut N.2: `[0]` alone for none, else the objective's code byte, the `i64`
///    expiry and the `hold_fire` byte, then — **16b cut Y.1** — the issuer's
///    `u32`, `0` for none; then — **16b cut Z.2** — the objective's payload for
///    the two squad roles, a `Screen`'s `facing_mdeg` as `u32` and a `Support`'s
///    charge `of` as its `u32`, and nothing more for the other four arms; the
///    order's `spot` is deliberately not folded, since
///    it steers nothing) and, from **16b cut Q.7**, the avoided cells — their count as
///    `u64`, then per entry the cell's two `u16` and the stop's `i64` stamp — and
///    `stand_until_ms`, `quiet_since_ms` (each a presence byte then an `i64`) and
///    the `rallied` byte; then, from **16b cut E.4**, `last_plot` (a presence
///    byte, then the cell's two `u16` and the plot's `i64` stamp),
///    `unheld_since_ms` (a presence byte then an `i64`) and `hunt` (a presence
///    byte then the cell's two `u16`); then, from **16b cut O.1**,
///    `hold_retry_ms` (a presence byte then an `i64`); then, from **16b cut
///    AI.1**, `searched_from` (a presence byte, then the cell's two `u16`) and
///    `refused` (`[0]` alone for none; else `[1]`, the stand cell's and the
///    refused cell's two `u16` each, the cause's code byte, the goal's two
///    `u16` and the `i64` it lapses at) — decisions with no line of their own
///    that gate a later search, the O.1 argument once more.
///
///    **Since 17b cut M1a.1** (doc 17 §4, D-UM-3) the goal, the waypoint, the
///    route, the avoided cells, `hold_retry_ms`, `searched_from` and `refused`
///    are read off the hull's [`crate::arena::motor::MotorRow`] on
///    [`Arena::motors`] — a row the table does not hold folding as
///    `MotorRow::default()`, which is exactly what a fresh reflex row's own
///    fields folded (`u16::MAX` twice for no goal, `u16::MAX` for no waypoint, a
///    `0` route length and avoid count, `[0]` for each absent stamp, cell and
///    stop) — in the order above, interleaved with the machine's own fields as
///    they always were. So a reflex row folds the bytes it folded before the
///    split, which two constants read at `32a1cc23` pin
///    (`crates/tacsim/tests/reflex.rs`'
///    `m1a1_the_moved_motor_folds_the_same_bytes`,
///    `crates/tacsim/tests/motor.rs`' `m1a1_the_shipped_duel_hashes_as_before`).
///
///    **Item 3b, the motor rows of hulls with no machine** (17b cut M1a.1),
///    is folded right after the last of them: after the
///    reflex machines, every motor row whose unit has **no** reflex row — a
///    pilot's `DRIVE` (17b cut M1a.4) or a mind's (M1a.5) — in unit-id order:
///    the unit's `u32`, the goal's two `u16` (`u16::MAX` twice for none), the
///    waypoint as `u16` (`u16::MAX` for none), the route's length as `u64`, the
///    avoided cells (their count as `u64`, then per entry the cell's two `u16`
///    and the `i64` stamp), `hold_retry_ms` (a presence byte then an `i64`),
///    `searched_from` and `refused` — each in item 3's own encoding. Per row and
///    with **no count prefix**, so an arena with no such row — every match this
///    cut can field — adds no byte at all, and the hash of every existing match
///    is unmoved. A goal is a decision that reaches a pose only substeps later,
///    which is the whole reason item 3 folds one; a hull steered by something
///    other than the reflex controller is folded for the same reason.
///
///    **Item 3c, the unit minds** (17b cut M1a.5; doc 17 §3, §6, §8), right
///    after item 3b: every row of [`Arena::minds`], in unit-id order and with
///    **no count prefix** (so an arena with no mind — every match that exists
///    — adds no byte): the unit's `u32`; `think_seq` as `u64`; `pending` as
///    `[0]`, or `[1]` then its `think_seq` `u64` and its `t_ms` `i64`; the
///    memory's length as `u64` then its FNV-1a 64 digest as `u64`; the
///    notices' count as `u64`, then per notice its **canonical
///    `serde_json::to_vec` bytes** — every field, so two buffers that would
///    drive two different decisions (two `Miss` notices differing only in
///    their reason, a `Struck`'s figures, a `was`, a section) never fold the
///    same; the range memory's count as `u64`, then per entry the
///    `ContactId`'s `u64` and the figure's `u64`; the order as `[0]`, or
///    `[1]`, the objective's code byte (`MindObjective::code`) and its payload
///    (`Advance`'s two `u16`, `Engage`'s target as `u64` or `0`, `Screen`'s
///    facing as `u32`, `Support`'s charge as `u64` or `0`, nothing for the
///    other two), the `hold_fire` byte, the band byte (`0` none, `1` close, `2`
///    effective, `3` far), `expires_ms` as `i64` and the squad word's bytes;
///    then `strikes`, one byte. Every field decides a later intent — a notice
///    a mind reads, a range it holds, an order it obeys — and none of them has
///    a ledger line of its own, so a replay that differed in one would
///    otherwise compare equal until the difference reached a pose.
///
///    **Since 17b cut M1b.1** each row ends, after `strikes`, with **the
///    card's bytes** ([`crate::arena::card::StockCard`], doc 17 §11): its
///    fields in declaration order — an enum as its code byte, a bool as one
///    byte, an integer little-endian, no separator — because the card is what
///    a mind thinks under and a later sub-cut's runner reads it.
///
///    **Carried from 17b cut M3.2, folded from 17b cut M3.6** (the cut's one
///    re-bless, which moves `M2_CROSS_PLATFORM_PINS`): the pending think's
///    ANSWER (`PendingThink`'s `epochs`, `intents` and `memory`, D-UM-23 as
///    amended), the row's `dropped` (`DropCause`) and its `seal_digest` are
///    carried by the snapshot and **not yet folded** — `pending` still folds
///    `[1]`, `think_seq`, `t_ms` (or `[0]`), M2's bytes exactly, and two
///    arenas differing only in one of them hash equal (the in-source probe
///    `m3_the_carried_answer_drop_seal_and_classes_are_not_yet_folded` pins
///    it; 17b cut M3.6 inverts it with the per-field probes). This is a
///    **declared gap**, not a covered one: the answer is NOT a function of
///    the folded state — the think read notices staged and drained inside
///    one substep, which no substep hash ever folds (doc 17 §6; 17b cut
///    M3.2's gate fixture
///    `m3_a_resume_between_n_and_n_plus_one_reproduces_the_hash_sequence_at_three_cadences`
///    turns on exactly that) — so until 17b cut M3.6 two runs differing only
///    in a pending answer hash equal at every substep until its landing
///    writes the memory or lands an intent, and D-UM-23's "hashed whole"
///    closes the gap there. A drop on strikes IS a function of the folded
///    `strikes` (the third strike writes both, and a clean think resets
///    `strikes` only on a row not dropped).
///
///    **Never folded**: [`crate::arena::state::UnitState::seal`] (17b cut
///    M3.2's `SealRef` — a per-match constant, the floor's argument below,
///    whose `closed` moves only with a ledger line, 17b cut M3.6's
///    `sortie_closed`, which the ledger's sequence folds) and
///    [`Arena::classes`] (17b cut M3.1's class table, captured at check-out
///    and carried from 17b cut M3.2: a per-match constant, likewise).
///
///    **Item 3d, the condition slots** (17b cut M1b.1; doc 17 §5, D-UM-16),
///    right after item 3c: every row of [`Arena::conditions`], in unit-id
///    order and with **no count prefix** (an arena with no slot — every
///    production match — adds no byte): the unit's `u32`, then per ARMED slot
///    its 1-based number `u8`, the weapon id's bytes, the target as `[0]`
///    (lock), `[1]` then the contact's `u64` (contact) or `[2]` (best hostile),
///    `min_permille` `u16`, the aspect mask `u8`, `min_rounds_left` `u32`,
///    `min_range_m` `u32` and `max_range_m` `u32`. A slot decides a shot at a
///    later millisecond with no ledger line of its own until it fires.
///
///    **Item 3e, the held-shot rows** (17b cut M1b.1), right after 3d: every
///    entry of [`Arena::held_said`] in key order, per row with no count prefix
///    — the unit's `u32`, the slot `u8`, the contact's `u64` and the stamp
///    `i64` — because a stamp decides whether a later held sentence is said.
///
///    **[`Arena::odds_floor_permille`] is NOT folded** (17b cut M1b.1): it is a
///    per-match constant captured at check-out, carried by the snapshot, and
///    read by nothing a production match runs before 17b cut M4 — folding it
///    would move the production hash of every match for a value no production
///    match reads. Two arenas differing only in the floor hash equal (the
///    in-source probe pins it).
/// 4. **The shadow contact store** (16b cut S.1), in its own iteration order —
///    viewer object number ascending, then contact id ascending, which is what
///    the two `BTreeMap`s give: per contact the viewer's `u32`, the contact id's
///    `u64`, the target's `u32`, the class byte (`0` unknown, `1` classified red,
///    `2` classified blue), `first_seen_ms` and `last_seen_ms` as `i64`; then the
///    plot's five integers, the own sighting (a presence byte, its stamp, its own
///    plot and **its reading**) and **the reading the platform holds** — a reading
///    being a presence byte, the look's `i64` stamp, the packed bands as `u32` and
///    the packed section count as `u8` (16b cut Q.2); then **whether this hold was
///    opened under a provisional number** as one byte (16b cut D.4).
/// 5. **The designator book** (16b cut Q.1; a digest since 16b cut M.3, 16c
///    `T-27`): the book's running digest
///    ([`crate::arena::state::DesignatorBook::digest`]) as `u64`, then the count
///    of issued numbers as `u64`. Until M.3 this walked both of the book's maps
///    row by row in every substep hash — a monotonic table, so the walk only ever
///    grew. The digest is an FNV-1a 64 fold of every *mint* in the order it was
///    made (the viewer's `u32`, the target's `u32`, the number's `u16`), so it
///    is a function of the whole sequence of mints: two books that minted the
///    same numbers for the same pairs in the same order agree, and (short of a
///    64-bit collision) any two that did not disagree. That tells apart
///    everything the walk could — the book is append-only, a re-assignment of a
///    held pair mints nothing, and the `next` counters are themselves a function
///    of the mints — and one thing more, the interleaving of two viewers' mints,
///    which is itself a function of arena time (the passes that mint run in
///    `(t_ms, seq)` order), so no replay the walk called equal is told apart
///    here by anything but the order the arena made its numbers in. The count
///    beside it is a cheap second witness.
/// 6. **The held losses** (16b cut M.2), in the map's own key order: the count of
///    entries as `u64`, then per entry the viewer's `u32`, the contact id's `u64`
///    and the deadline as `i64`. The designator in the value is not folded.
/// 7. The tempo byte (`0` quiet, `1` alert, `2` engaged).
/// 8. `now_ms`, `i64`.
/// 9. The due-event count, `u64`.
/// 10. **The in-flight shot count** (16b cut W.1), `u64`.
/// 11. The ledger's event sequence, `u64`.
/// 12. **The mode and the Hold objective** (16b cut O.1; 16c N11): the mode's
///     code byte ([`crate::arena::state::MatchMode::code`]: `0` skirmish, `1`
///     hold, `2` range — 16b cut O.3's arm rides this one byte and adds none:
///     a range carries no objective, so its hold is `[0]`); then the hold as a
///     presence byte, the spot's `x` and `y` as `u16`,
///     `radius_cells` as `u16` and `hold_ms` as `u32`, then `held_by` as a
///     presence byte and the team byte (`0` red, `1` blue) in one call, then the
///     `i64` arena millisecond it opened at — `[0]` alone for no hold, and `[0]`
///     alone for a hold nobody holds.
///
/// **Why the Hold clock is in it** (16b cut O.1), under cut D.4's one rule —
/// *persistent state that decides an end is hashed*. `held_by` moves with **no
/// ledger line of its own** (opening, dropping and re-opening the clock journal
/// nothing: it reaches no pilot), and it decides the match: a replay that saw
/// the sole occupant one substep earlier would win one substep earlier, and
/// until then nothing else in here would differ. Folding it catches that at the
/// substep it happened. The `HoldSettles` event it schedules is folded by the
/// due count (item 9). The machines' `hold_retry_ms` (item 3) is the same
/// argument read for a controller: a decision with no line of its own that
/// decides a later goal.
///
/// **Why the contacts are in it** (16b cut S.1). Doc 16 §3.1 makes the arena the
/// single writer for *unit poses, unit condition cells, contacts, orders,
/// scheduled impacts, the match ledger*, and the hash exists to catch a tempo that
/// changed an outcome. Detection, classification and loss are journalled, so the
/// ledger sequence already moves with each of them — but a contact's `last_seen_ms`
/// advances on every refresh with no event at all, and a replay that refreshed a
/// contact one pass earlier would otherwise compare equal. The store is folded in
/// its map order, which is deterministic by construction, so there is no iteration
/// order for a replay to diverge on.
///
/// **What is deliberately absent.** The *ordered* desired heading and speed are
/// not folded in: they are an input, and a pulse at one tempo may have applied a
/// helm order that a pulse at another has not yet — the replay compares what the
/// world *became*, not what it was told. The opaque `TacticalUnit` is not folded
/// in either; its truth is M32's and its every change is journalled there, so
/// hashing it would make this function a second, weaker copy of that journal.
/// (One cell of it is, since 16b cut M.3: the `heat` cell, item 1.18, because it
/// gates a refusal and moves every substep — see cut M.3's extensions below.)
///
/// **The staff tempo floor is not in it either** (16b cut I.3; 16c N10). It is
/// a view of how often the task wakes — `crate::arena::clock::Clock`'s
/// `floor_ms` and the floor at the previous wake — and the tempo decides no
/// outcome (D-16-2), so neither does a floor under it; every floored pulse is
/// quantised to the substep lattice so the positions a due event is applied
/// against never depend on it. **But its `tempo_floor` line moves the ledger's
/// sequence, item 11, and so every hash after it**, exactly as every line does:
/// a floored run hash-equals another run floored at the same arena moment, and
/// never an unfloored one. A staff `@arena/place` adds no hashed field either:
/// it moves a pose, which items 1.2 to 1.4 already fold, and its `placed` or
/// `place_refused` line moves item 11 like any other.
///
/// **Why the machines are in it** (16b cut N.1). The controller writes no ledger
/// event of its own — what leaves it is the orders a pilot's verbs would have
/// staged, and those journal what they always did — so a replay whose machine
/// chose a different waypoint would compare equal for as many substeps as it took
/// the divergence to reach a pose. Folding the machine in catches it at the
/// substep it happened, which is the whole reason the hash sequence and not only
/// the final state is compared. What is folded is the *decision* and not the
/// route's cells: the cells are a function of the goal and the ground, and hashing
/// them would hash the window once per unit.
///
/// **Cut W.1's extension**, which that last sentence promised: the lock, the two
/// weapon cells and the in-flight shot count. Each is a fact the arena is the single
/// writer of and each moves without an event of its own — a cycle that has run down,
/// a lock cleared because its contact expired — so a replay that fired half a pulse
/// earlier would otherwise compare equal. The shot *record* is not folded in beyond
/// its count: the shot's own fields are a function of the poses and the range at
/// acceptance, which are already in here, and folding a predicted point would hash
/// the same numbers twice.
///
/// **Cut X.6's extension**: `grade_pct`, for the same argument. It is
/// authoritative state with exactly one writer
/// ([`crate::arena::motion::integrate`]'s accepted crossing), it moves without a
/// ledger event, and a unit can sit in one cell for many substeps after the
/// crossing that set it — so a replay that took a ramp a substep earlier would
/// compare equal until the difference in speed had accumulated into a whole
/// centimetre of pose. The *limit* beside it is deliberately not folded: it is
/// read once at check-out and never moves, and every refusal it causes is a pose
/// that is already in here.
///
/// **Cut Q.1's extension**: the designator book. It is the same argument the
/// contact store makes, one step further. A number is issued when a platform's
/// own sensors first insert an `Unknown` contact — and that insert journals an
/// `EventKind::Detected` which is **byte for byte the shape a mesh arrival
/// journals**, so the ledger's sequence moves identically whether a number was
/// issued or not. A replay whose blip was found by its own sight where the
/// original took it off the mesh would otherwise compare equal, and the two runs
/// would then show the pilot different names for the same tank. Folding the book
/// catches it at the substep it happened, which is the argument cut F.3 made for
/// the plot.
///
/// **Cut Q.2's extension**: the two readings on every contact. They are the
/// contact store's own argument again — a close look is taken on a pass and
/// journals nothing, a share hands one over and journals nothing, and the band a
/// gunner watched come apart is written in beside a `section_destroyed` that is
/// about the *victim* and would be byte-identical either way. A replay that got
/// close a pass earlier, or whose share arrived from a different look, would
/// otherwise compare equal and then show the two pilots different `SCAN`s.
///
/// The *unit's own* [`crate::arena::state::UnitState::condition`] is deliberately
/// **not** folded, and that is the other half of the same argument: it is a pure
/// function of the `TacticalUnit` beside it, which M32 journals, so folding it
/// would make this function a second and weaker copy of that journal — the reason
/// the opaque unit itself is absent.
///
/// **Cut Q.6's extension**: the four motion carries. They are the *fractional
/// part* of three quantities this hash already folds — the heading, the speed and
/// the pose — so leaving them out would be folding a number and hiding its low
/// bits. Each is authoritative state with exactly one writer
/// ([`crate::arena::motion::integrate`]), each moves with no ledger event of its
/// own, and two replays whose carries differ are two replays that are about to
/// disagree about a whole centimetre — several substeps later, when the drift has
/// accumulated, which is precisely the divergence the per-substep sequence exists
/// to catch at the substep it happened. **16b cut M.4** (16c `T-16`) adds the two
/// sign bytes beside them, `turn_side` and `speed_rate`: a carry is now zeroed on
/// the substep that turns the other way or changes rate, so which way it was
/// banked decides the next substep's heading and speed.
///
/// **What a shot in flight carries is still not folded** beyond the count, and
/// that includes 16b cut M.4's `Resolution::beyond` (16c `B-52`): it is a
/// function of the range at acceptance and the weapon's authored bands, both
/// fixed when the shot was accepted, exactly as the rest of the recorded
/// resolution is. So is **16b cut AE.2**'s launch point (`from_x_cm`,
/// `from_y_cm`, `from_cell`, `from_height_cm`): the shooter's pose and eye at
/// acceptance, which this hash folded as the shooter's own pose at the substep
/// the shot was accepted in, and which nothing writes again — the impact reads
/// it and the shot leaves the table.
///
/// The *callsign* on each unit is deliberately **not** folded: it is a pure
/// function of the check-out message, resolved once by `Arena::new` and never
/// moved, so two replays of one custody message agree on it by construction —
/// the same argument `slope_limit_pct` makes.
///
/// Neither is the transient `lost_designators` table, and the reason is not that
/// it is empty here: the record below is taken **before** the staging scan that
/// drains it, so in a substep where a contact expired the table is holding that
/// row's name at this very line. It is not folded because its contents are a
/// function of state this hash already folds — the expiry that filled it is the
/// contact store's own change, and the name it carries is read off the book and
/// the callsign — and because it never survives the substep it was filled in, so
/// nothing it holds can reach a later one. Folding it would add no information
/// and would make the hash a function of *where* the drain sits.
///
/// **Cut D.2's `spent_hits` is absent on exactly that argument**, and it is the
/// one other table the argument really does fit. The shots in it are a function
/// of state this hash already folds — the struck section's two cells, and the
/// `hit` line the ledger sequence counts — and it never survives the substep
/// `crate::arena::damage::Arena::apply_hit` filled it in, because the staging
/// scan drains it. It carries the same caveat word for word: the record below is
/// taken *before* that scan, so in a substep where a round found a spent section
/// the table is holding that shot at this very line, and that costs nothing.
///
/// **Cut U.2's `jam_fold` is absent too, on a plainer argument** (16c `T-29`):
/// [`crate::arena::state::Arena::jam_fold`] is a memo of
/// [`crate::arena::Arena::jamming_at`], a pure function of the poses, the power
/// bytes and the wreck bytes item 1 already folds, and it is cleared at every
/// point inside a substep where one of those moves. Folding a cache would add no
/// information and would make the hash a function of which folds had been asked
/// for by the time the record was taken.
///
/// **Cut J.2's horizon index is absent on the same argument, and a plainer one
/// still**: it lives inside [`crate::arena::state::ArenaTerrain`] as a function
/// of the cells the definition digest already covers, built with them and never
/// written, so folding it would fold the terrain a second time.
///
/// **Cut Y.4's `at_release` and `pending_catchup` are absent on `last_frames`'
/// argument**: they are what a pilot will be told about a drop, written only by
/// a control that names a pilot and read only by the frame builder, so folding
/// them would make the hash a function of who was connected — and nothing that
/// steers reads them, so they cannot move a later substep either.
///
/// **Cut I.4's `watched` and `last_observer_s` are absent beside
/// `last_frames`**: a fact about who is watching and not about the match. They
/// gate the staff picture phase 5 builds at most once an arena second, are
/// written only by the task's `Watch` and by that gate, and are read by nothing
/// that steers, so folding them would make the hash a function of whether a
/// staffer was looking — and a watched match must hash-equal an unwatched one
/// (`crates/tacsim/tests/arena.rs`, `i4_a_watched_match_is_the_same_match`).
///
/// **Cut D.2's `spent_said` is the opposite case, and it IS folded.** It persists
/// for [`crate::arena::state::SPENT_NOTICE_MS`] across substeps and pulses and it
/// decides what a *later* substep says, so `lost_designators`' argument is simply
/// false of it: a replay whose first spent round landed one substep earlier would
/// suppress a different later one, and the two runs would then show the two
/// pilots different lines. Folding it catches that at the substep it happened,
/// which is the argument cut Q.1 made for the designator book. Two rules keep the
/// fold rate-independent, and both are `Arena::spent_said`'s: the decision and
/// the write happen whether or not anybody is seated and whether or not a frame
/// is ever sent, and entries are pruned only on touch, against the event's own
/// timestamp, never by a per-pulse sweep. Because the staging scan runs **after**
/// the substep's hash is recorded, a write at substep *k* is first folded at
/// *k + 1*.
///
/// **Cut D.4's extension**: two more, under the same one rule — *persistent state
/// that decides a notice is hashed*.
///
/// [`crate::arena::state::ShadowContact::numbered`] moves with **no
/// distinguishable ledger event**: a contact opened by this platform's own
/// sensors and a contact handed over by the mesh both journal an
/// `EventKind::Detected` of byte-identical shape, so a replay whose blip was
/// found by its own sight where the original took it off the mesh would compare
/// equal — and the two runs would then say the `identified` line in one and not
/// the other. That is cut Q.1's own argument for the designator book, one hold
/// further in.
///
/// [`crate::arena::state::UnitState::receiver`] persists between passes and
/// decides a notice, and it can move with no ledger line at all: a platform that
/// stops running passes (a wreck, a shot-off sight) writes `None` silently, which
/// is a deliberate silence and not an omission. Folding it catches that at the
/// pass's own timestamp. The `jammer` and `eccm` beside it stay out for the
/// reason they always did: those are facts about the unit's fit that nothing in a
/// match moves.
///
/// **Cut E.2's extension**, under the same one rule.
/// [`crate::arena::state::ShadowContact::known_dead`] is what the shooter's own
/// machine reads to drop a target it killed, and it moves with **no ledger line
/// that names the holder**: a `destroyed` event carries the victim and the
/// shooter, never the contact row the fact was written onto, so a replay that
/// credited one shooter's kill to another platform's hold would compare equal
/// and the two runs would then steer two different tanks. Folding it catches
/// that at the substep it happened. Because the staging scan runs *after* the
/// substep's hash is recorded, a kill at substep *k* is first folded at
/// *k + 1*, exactly as `spent_said`'s writes are.
///
/// [`crate::arena::state::UnitState::fatal_sections`] beside it is deliberately
/// **not** folded: it is laid down once by [`Arena::new`] off the check-out
/// record and never moves, which is `slope_limit_pct`'s and `jammer`'s argument
/// word for word.
///
/// **Cut E.3's extension**: [`crate::arena::state::UnitState::occupied_hold`], on
/// `grade_pct`'s argument exactly. It is authoritative state with one writer
/// ([`crate::arena::motion::integrate`]), it decides a *later* refusal — whether
/// the next crossing this unit is refused is a hold or doc 16 §3.4 step 4's stop
/// — and it moves with **no ledger line at all**, because a hold is by
/// construction the substep that journals nothing. A replay that opened its hold
/// one substep earlier would stop one substep earlier two seconds later, and
/// nothing else in here would differ until the difference had accumulated into a
/// whole centimetre of pose. The cell and the stamp both go in: a hold against a
/// different cell is a different hold, and a hold that started earlier matures
/// earlier.
///
/// **Cut M.2's extension** (16c `T-83`): [`Arena::held_losses`], under cut D.4's
/// one rule. It was kept out while a sweep at the end of every pulse said the
/// expired entries, because folding a table a pulse boundary moved would have
/// made the hash a function of the tempo; since the expiry is a
/// [`crate::arena::state::DueKind::LossSettles`] at the deadline's own
/// millisecond, nothing but an arena timestamp moves it, and it decides whether a
/// later substep says `lost` — a replay whose contact was re-detected a pass
/// later would otherwise compare equal until the two pilots were shown
/// different lines. It is written only in the staging scan (the `lost` arm
/// inserts, the `detected` arm cancels, the settle removes), which runs after
/// the substep's hash is recorded, so an entry written at substep *k* — and the
/// due event scheduled beside it, which the due count folds — is first folded at
/// *k + 1*, exactly as `spent_said`'s writes are. The designator in the value is
/// not folded: it is a name derived from the book and the callsign, both of
/// which are already in here.
///
/// **Cut M.3's extensions** (16c `T-04`, `T-06`), under cut D.4's one rule.
///
/// [`crate::arena::state::UnitState::held`] decides, since that cut, the
/// ledger's `blocked` line, the pilot's `Held:` notice and a machine's avoid
/// list — all three fire on its new edge — and it moves with no line of its own
/// when it clears (a reverse off the bank, a crossing into another cell, the
/// order coming off). A replay that cleared it a substep earlier would journal
/// a second stop where the original said nothing, so the cell and the cause
/// both go in: a different cell or a different cause is a different condition,
/// and a new edge. The per-substep `blocked` flag beside it stays out — it
/// journals nothing now, and it is what *this* substep did rather than state a
/// later one reads.
///
/// **Heat** gates firing (a platform at its ceiling refuses `FIRE`) and moves on
/// every substep with no ledger line at all — the dissipation is a rate over
/// arena time, carried in thousandths ([`crate::arena::heat`]'s head) — which is
/// cut Q.6's argument for the motion carries, word for word: a replay that shed
/// its heat a substep sooner would compare equal until the difference refused a
/// shot. The cell's value and the carry both go in, because the carry is the
/// cell's fractional part.
///
/// **What the staging scan writes after the record** (16c `T-09`, written down
/// by 16b cut M.3). The record below is taken before the substep's staging scan,
/// and the scan writes hashed state of its own — among it: `spent_said` and
/// `known_dead` (their paragraphs above); a gunner's observed band — when a
/// classified target's section is destroyed by the gunner's own round, the
/// scan's `section_destroyed` arm writes that section's band `destroyed` into the
/// reading the gunner's platform holds
/// ([`crate::arena::state::ShadowContact::seen_condition`], through
/// `Arena::observe_section_lost`); the lock a `lost` contact clears (the `lost`
/// arm sets the viewer's `lock` — per-unit item 10 — to none); and
/// `held_losses` (item 6), with the `LossSettles` due event the `lost` arm
/// schedules beside an entry, which the due count (item 9) folds — the M.2
/// paragraph above. Every one of them is first folded at *k + 1*, and each is
/// rate-independent for the reason `spent_said` is: the scan runs at the same
/// point of every substep at every cadence, off the journal entries of that
/// substep. The list is not a checklist: a write added to the scan is folded a
/// substep late by construction, whether or not it is named here.
///
/// **What a snapshot keeps of all this** (16b cut AF.1; 16c N16;
/// `crate::arena::snapshot`). What the snapshot carries is every field this
/// function folds and every field that steers; what it re-derives is a
/// function of the definition (whose digest the document records) and of
/// compiled code (the locomotion table, like the slope rule — the document
/// records its digest too), or a cache of state it carries (the index, the
/// jamming memo, the cheapest cell per row); what it resets is a view or
/// per-pulse scratch. The decode's digest and hash checks and
/// `crates/tacsim/tests/arena.rs`' every-pulse resume are the proofs; a field
/// added to the state must go in one of the three lists in `snapshot.rs`, whose
/// exhaustive destructuring will not compile until it does. No fold here moved
/// for it.
#[must_use]
pub fn state_hash(arena: &Arena) -> u64 {
    #[cfg(test)]
    STATE_HASHES_TAKEN.with(|taken| taken.set(taken.get().saturating_add(1)));
    let mut hash = FNV_OFFSET_BASIS;
    for unit in &arena.units {
        hash = fnv1a64_update(hash, &unit.id.0.to_le_bytes());
        hash = fnv1a64_update(hash, &unit.pose.x_cm.to_le_bytes());
        hash = fnv1a64_update(hash, &unit.pose.y_cm.to_le_bytes());
        hash = fnv1a64_update(hash, &unit.pose.z_cm.to_le_bytes());
        hash = fnv1a64_update(hash, &unit.pose.heading_mdeg.to_le_bytes());
        hash = fnv1a64_update(hash, &unit.speed_cms.to_le_bytes());
        hash = fnv1a64_update(hash, &[unit.power.code()]);
        hash = fnv1a64_update(
            hash,
            &[
                unit.limits.slow_pct,
                // 16b cut M.4 (`B-47`): the two sides are two bytes.
                u8::from(unit.limits.no_turn_left),
                u8::from(unit.limits.no_turn_right),
                u8::from(unit.limits.motion_disabled),
                u8::from(unit.limits.wreck),
            ],
        );
        // **16b cut M.3** (`T-04`): what it is held against — its own item after
        // the limit bytes (five since 16b cut M.4).
        match unit.held {
            Some(held) => {
                hash = fnv1a64_update(hash, &[1_u8]);
                hash = fnv1a64_update(hash, &held.x.to_le_bytes());
                hash = fnv1a64_update(hash, &held.y.to_le_bytes());
                hash = fnv1a64_update(hash, &[held.cause.code()]);
            }
            None => hash = fnv1a64_update(hash, &[0_u8]),
        }
        // 16b cut W.1: the lock and the two weapon cells.
        hash = fnv1a64_update(
            hash,
            &unit.lock.map_or(0_u64, |contact| contact.0).to_le_bytes(),
        );
        hash = fnv1a64_update(hash, &(unit.ready_at_ms.len() as u64).to_le_bytes());
        for (weapon, ready_at_ms) in &unit.ready_at_ms {
            hash = fnv1a64_update(hash, weapon.as_bytes());
            hash = fnv1a64_update(hash, &ready_at_ms.to_le_bytes());
        }
        hash = fnv1a64_update(hash, &(unit.ammo.len() as u64).to_le_bytes());
        for (weapon, rounds) in &unit.ammo {
            hash = fnv1a64_update(hash, weapon.as_bytes());
            hash = fnv1a64_update(hash, &rounds.to_le_bytes());
        }
        // 16b cut N.1: who is flying it.
        hash = fnv1a64_update(hash, &[unit.controller.code()]);
        // 16b cut X.6: the slope it is standing on.
        hash = fnv1a64_update(hash, &unit.grade_pct.to_le_bytes());
        // **16b cut E.3**: and what it is standing *against* — see this function's
        // head for why a hold is hashed where the slope limit beside it is not.
        match unit.occupied_hold {
            Some(hold) => {
                hash = fnv1a64_update(hash, &[1_u8]);
                hash = fnv1a64_update(hash, &hold.cell.0.to_le_bytes());
                hash = fnv1a64_update(hash, &hold.cell.1.to_le_bytes());
                hash = fnv1a64_update(hash, &hold.since_ms.to_le_bytes());
            }
            None => hash = fnv1a64_update(hash, &[0_u8]),
        }
        // 16b cut Q.6: the fractions of a turn, a speed change and a centimetre
        // that the last substep did not spend.
        hash = fnv1a64_update(hash, &unit.carry.turn.to_le_bytes());
        hash = fnv1a64_update(hash, &unit.carry.speed.to_le_bytes());
        hash = fnv1a64_update(hash, &unit.carry.east.to_le_bytes());
        hash = fnv1a64_update(hash, &unit.carry.north.to_le_bytes());
        // 16b cut M.4 (`T-16`): which way each of the first two was banked.
        hash = fnv1a64_update(hash, &[unit.carry.turn_side, unit.carry.speed_rate]);
        // **16b cut D.4**: what this platform's own last pass ran under.
        match unit.receiver {
            Some(receiver) => {
                hash = fnv1a64_update(hash, &[1_u8, receiver.budget, receiver.budget_max]);
                hash = fnv1a64_update(hash, &receiver.reach_permille.to_le_bytes());
            }
            None => hash = fnv1a64_update(hash, &[0_u8]),
        }
        // **16b cut M.1** (`T-82`): the seat-order rate's window and count — they
        // persist between pulses and decide a `queue_full` refusal.
        hash = fnv1a64_update(hash, &unit.orders_window_s.to_le_bytes());
        hash = fnv1a64_update(hash, &unit.orders_in_window.to_le_bytes());
        // **16b cut M.3** (`T-06`): heat, which gates firing and moves every
        // substep with no line — the cell's value and the carry that is its
        // fractional part, under one presence byte.
        match (unit.heat_now(), unit.heat.as_ref()) {
            (Some(now), Some(heat)) => {
                hash = fnv1a64_update(hash, &[1_u8]);
                hash = fnv1a64_update(hash, &now.to_le_bytes());
                hash = fnv1a64_update(hash, &heat.carry_milli.to_le_bytes());
            }
            _ => hash = fnv1a64_update(hash, &[0_u8]),
        }
        // **16b cut O.2** (16c N12): the pod's switch, which decides what every
        // sight inside its bubble sees.
        hash = fnv1a64_update(hash, &[u8::from(unit.ecm_on)]);
    }
    // **16b cut D.2**: the spent-section window, in the map's own key order.
    hash = fnv1a64_update(hash, &(arena.spent_said.len() as u64).to_le_bytes());
    for ((told, target, section), at_ms) in &arena.spent_said {
        hash = fnv1a64_update(hash, &told.0.to_le_bytes());
        hash = fnv1a64_update(hash, &target.0.to_le_bytes());
        hash = fnv1a64_update(hash, section.as_bytes());
        hash = fnv1a64_update(hash, &at_ms.to_le_bytes());
    }
    // 16b cut N.1: the controller's own machines, in unit-id order.
    //
    // **17b cut M1a.1**: the driving fields are the motor row's now, read back
    // here in the order this loop always folded them; a unit the table holds no
    // row for folds a default row, which is what its own fields folded before
    // the split (item 3). Borrowed, not cloned: a hash is taken every substep
    // and a route is a vector, and an empty default allocates nothing.
    let fresh = crate::arena::motor::MotorRow::default();
    for (unit, row) in arena.reflex.rows() {
        let motor = arena.motors.row(*unit).unwrap_or(&fresh);
        hash = fnv1a64_update(hash, &unit.0.to_le_bytes());
        hash = fnv1a64_update(hash, &[row.state.code()]);
        let (gx, gy) = motor.goal.unwrap_or((u16::MAX, u16::MAX));
        hash = fnv1a64_update(hash, &gx.to_le_bytes());
        hash = fnv1a64_update(hash, &gy.to_le_bytes());
        hash = fnv1a64_update(hash, &motor.waypoint.unwrap_or(u16::MAX).to_le_bytes());
        hash = fnv1a64_update(hash, &(motor.route.len() as u64).to_le_bytes());
        hash = fnv1a64_update(
            hash,
            &row.target
                .map_or(0_u32, |(_, target)| target.0)
                .to_le_bytes(),
        );
        // **16b cut N.2**: the standing order — what it says and when it stops
        // saying it. Both are functions of arena time and of nothing else, so two
        // cadences of the same scripted match fold the same bytes; the order's
        // `issued_ms` is a *world* millisecond and is deliberately NOT folded, for
        // exactly that reason.
        match &row.order {
            Some(standing) => {
                hash = fnv1a64_update(hash, &[standing.order.objective.code()]);
                hash = fnv1a64_update(hash, &standing.expires_ms.to_le_bytes());
                hash = fnv1a64_update(hash, &[u8::from(standing.order.constraints.hold_fire)]);
                // **16b cut Y.1** (the cut's one hash move): whose scope the hint
                // is written in, `0` for the squad's own commander. Two orders
                // that differ only in the issuer point at different objects, so
                // they are two states. `spot` is NOT folded: it is the
                // commander's word for the cell, and nothing that steers reads it.
                hash = fnv1a64_update(
                    hash,
                    &standing.order.issuer.map_or(0_u32, |id| id.0).to_le_bytes(),
                );
                // **16b cut Z.2** (the cut's one hash move): the two squad roles
                // steer by their payload — the arc a screen faces, the unit a
                // support keeps station on — so two orders that differ only
                // there are two states. The other four arms fold nothing more:
                // `Advance`'s cell and `Engage`'s hint reach the hash through the
                // goal and the target they write, as they always have.
                match standing.order.objective {
                    Objective::Screen { facing_mdeg } => {
                        hash = fnv1a64_update(hash, &facing_mdeg.to_le_bytes());
                    }
                    Objective::Support { of } => {
                        hash = fnv1a64_update(hash, &of.0.to_le_bytes());
                    }
                    Objective::Hold
                    | Objective::Advance { .. }
                    | Objective::Engage { .. }
                    | Objective::Withdraw => {}
                }
            }
            None => hash = fnv1a64_update(hash, &[0_u8]),
        }
        // **16b cut Q.7**: the cells it was stopped at, with the stamps that age
        // them out, and the three fields that make `Withdraw` something other than
        // a parking space. Every one is written from an arena timestamp and read
        // at an arena timestamp, so two cadences of one scripted match fold the
        // same bytes — and a replay whose tank has already rallied must not
        // compare equal to one that has not.
        hash = fold_avoid(hash, &motor.avoid);
        hash = fold_stamp(hash, row.stand_until_ms);
        hash = fold_stamp(hash, row.quiet_since_ms);
        hash = fnv1a64_update(hash, &[u8::from(row.rallied)]);
        // **16b cut E.4**: the three fields that make `Patrol` something other
        // than a fixed cycle — where the machine last saw something and when,
        // how long it has held nothing at all, and the far entry cell it is
        // crossing to. Each is written from an arena timestamp and read at an
        // arena timestamp, so two cadences of one scripted match fold the same
        // bytes; each is a *decision* the controller made and would otherwise
        // reach a pose several substeps later, which is the whole reason the
        // machines are folded at all.
        match row.last_plot {
            Some(((x, y), at_ms)) => {
                hash = fnv1a64_update(hash, &[1_u8]);
                hash = fnv1a64_update(hash, &x.to_le_bytes());
                hash = fnv1a64_update(hash, &y.to_le_bytes());
                hash = fnv1a64_update(hash, &at_ms.to_le_bytes());
            }
            None => hash = fnv1a64_update(hash, &[0_u8]),
        }
        hash = fold_stamp(hash, row.unheld_since_ms);
        match row.hunt {
            Some((x, y)) => {
                hash = fnv1a64_update(hash, &[1_u8]);
                hash = fnv1a64_update(hash, &x.to_le_bytes());
                hash = fnv1a64_update(hash, &y.to_le_bytes());
            }
            None => hash = fnv1a64_update(hash, &[0_u8]),
        }
        // **16b cut O.1**: when a Hold spot no route reached may be tried again —
        // a decision the controller made, written and read at arena timestamps,
        // exactly as the hunt's clock beside it is.
        hash = fold_stamp(hash, motor.hold_retry_ms);
        // **16b cut AI.1** (16c `B-84`): the cell the last search started from,
        // and the stop the machine remembers. Neither has a line of its own, and
        // each gates a later search — `searched_from` whether a refusal is
        // recorded, `refused` whether a search is spent — so each would reach a
        // pose several substeps after the decision. Written at the tick's `t_ms`
        // or the `blocked` line's own millisecond, read at a tick.
        hash = fold_search(hash, motor);
    }
    // **17b cut M1a.1** (item 3b): the motor rows of hulls the controller is not
    // flying — a pilot's `DRIVE`, a mind's — in unit-id order, each in item 3's
    // own encoding and with no count prefix, so a table that holds none (every
    // match before M1a.4) folds no byte here.
    for (unit, motor) in arena.motors.rows() {
        if arena.reflex.unit(unit).is_some() {
            continue;
        }
        hash = fnv1a64_update(hash, &unit.0.to_le_bytes());
        let (gx, gy) = motor.goal.unwrap_or((u16::MAX, u16::MAX));
        hash = fnv1a64_update(hash, &gx.to_le_bytes());
        hash = fnv1a64_update(hash, &gy.to_le_bytes());
        hash = fnv1a64_update(hash, &motor.waypoint.unwrap_or(u16::MAX).to_le_bytes());
        hash = fnv1a64_update(hash, &(motor.route.len() as u64).to_le_bytes());
        hash = fold_avoid(hash, &motor.avoid);
        hash = fold_stamp(hash, motor.hold_retry_ms);
        hash = fold_search(hash, motor);
    }
    // **17b cut M1a.5** (item 3c): the unit minds, per row with no count
    // prefix — an arena with none (every production match) folds no byte here.
    for (unit, mind) in &arena.minds {
        hash = fold_mind(hash, *unit, mind);
    }
    // **17b cut M1b.1** (item 3d): every hull's condition slots, per row with
    // no count prefix — an arena with none (every production match) folds no
    // byte here.
    for (unit, slots) in arena.conditions.rows() {
        hash = fnv1a64_update(hash, &unit.0.to_le_bytes());
        for (at, slot) in slots.iter().enumerate() {
            let Some(cond) = slot else {
                continue;
            };
            let number = u8::try_from(at.saturating_add(1)).unwrap_or(u8::MAX);
            hash = fnv1a64_update(hash, &[number]);
            hash = fnv1a64_update(hash, cond.weapon.as_bytes());
            hash = fnv1a64_update(hash, &[cond.target.code()]);
            if let crate::arena::conditions::SlotTarget::Contact { id } = cond.target {
                hash = fnv1a64_update(hash, &id.0.to_le_bytes());
            }
            hash = fnv1a64_update(hash, &cond.min_permille.to_le_bytes());
            hash = fnv1a64_update(hash, &[cond.aspects]);
            hash = fnv1a64_update(hash, &cond.min_rounds_left.to_le_bytes());
            hash = fnv1a64_update(hash, &cond.min_range_m.to_le_bytes());
            hash = fnv1a64_update(hash, &cond.max_range_m.to_le_bytes());
        }
    }
    // **17b cut M1b.1** (item 3e): when each slot last said why it held, per row
    // with no count prefix — empty in every production match.
    for ((unit, slot, contact), at_ms) in &arena.held_said {
        hash = fnv1a64_update(hash, &unit.0.to_le_bytes());
        hash = fnv1a64_update(hash, &[*slot]);
        hash = fnv1a64_update(hash, &contact.0.to_le_bytes());
        hash = fnv1a64_update(hash, &at_ms.to_le_bytes());
    }
    // **17b cut M1b.1**: `arena.odds_floor_permille` is deliberately NOT folded
    // (this function's head says why).
    for (viewer, id, held) in arena.contacts.all() {
        hash = fnv1a64_update(hash, &viewer.0.to_le_bytes());
        hash = fnv1a64_update(hash, &id.0.to_le_bytes());
        hash = fnv1a64_update(hash, &held.target.0.to_le_bytes());
        hash = fnv1a64_update(hash, &[held.class.code()]);
        hash = fnv1a64_update(hash, &held.first_seen_ms.to_le_bytes());
        hash = fnv1a64_update(hash, &held.last_seen_ms.to_le_bytes());
        // **16b cut F.3**: the plot and the provenance are authoritative state,
        // folded exactly as the stamps beside them are. A replay whose share
        // arrived from a different sighting, or whose plot is a metre away, must
        // not compare equal — the plot is what every reader of a contact's
        // position now reads, and the own sighting is what decides whether the
        // mesh may carry it a hop further.
        hash = fold_plot(hash, &held.seen_at);
        match held.own_seen {
            Some(own) => {
                hash = fnv1a64_update(hash, &[1_u8]);
                hash = fnv1a64_update(hash, &own.at_ms.to_le_bytes());
                hash = fold_plot(hash, &own.plot);
                // **16b cut Q.2**: what this platform's OWN sensors read, which
                // is what it may put on the mesh.
                hash = fold_reading(hash, own.condition);
            }
            None => hash = fnv1a64_update(hash, &[0_u8]),
        }
        // **16b cut Q.2**: and what it holds, which may be a teammate's. Both are
        // knowledge that moves with no event of its own — a close look taken one
        // pass earlier, a share that arrived, a band the gunner watched go — so a
        // replay that read the same hull a pass sooner must not compare equal.
        hash = fold_reading(hash, held.seen_condition);
        // **16b cut D.4**: whether this hold was opened under a number, which is
        // what decides the `identified` line and which moves with no
        // distinguishable event of its own — see this function's head.
        //
        // **16b cut E.2** (`B-45`): and whether this platform watched the target
        // die, beside it and for the same reason — the `destroyed` line that
        // moves it names the victim and the shooter and never the **holder**,
        // so the fold is the only thing that tells two replays apart.
        hash = fnv1a64_update(hash, &[u8::from(held.numbered), u8::from(held.known_dead)]);
    }
    // **16b cut Q.1**: the provisional numbers — since **16b cut M.3** (`T-27`)
    // as the book's running digest of every mint and the issued count, in place
    // of a walk of both maps that grew with the match (this function's head).
    hash = fnv1a64_update(hash, &arena.designators.digest().to_le_bytes());
    hash = fnv1a64_update(hash, &(arena.designators.issued_len() as u64).to_le_bytes());
    // **16b cut M.2** (16c `T-83`): the held losses, in the map's own key order —
    // the deadline decides whether a later substep says `lost`. The designator
    // beside it is not folded: it is read off the book and the callsign, both
    // already in here.
    hash = fnv1a64_update(hash, &(arena.held_losses.len() as u64).to_le_bytes());
    for ((viewer, contact), (deadline, _)) in &arena.held_losses {
        hash = fnv1a64_update(hash, &viewer.0.to_le_bytes());
        hash = fnv1a64_update(hash, &contact.0.to_le_bytes());
        hash = fnv1a64_update(hash, &deadline.to_le_bytes());
    }
    hash = fnv1a64_update(hash, &[arena.clock.tempo().code()]);
    hash = fnv1a64_update(hash, &arena.now_ms.to_le_bytes());
    hash = fnv1a64_update(hash, &(arena.due.len() as u64).to_le_bytes());
    hash = fnv1a64_update(hash, &(arena.shots.len() as u64).to_le_bytes());
    hash = fnv1a64_update(hash, &arena.ledger.seq().to_le_bytes());
    // **16b cut O.1** (item 12): the mode, and the Hold objective with its clock.
    hash = fnv1a64_update(hash, &[arena.mode.code()]);
    match arena.hold.as_ref() {
        Some(hold) => {
            hash = fnv1a64_update(hash, &[1_u8]);
            hash = fnv1a64_update(hash, &hold.spot.0.to_le_bytes());
            hash = fnv1a64_update(hash, &hold.spot.1.to_le_bytes());
            hash = fnv1a64_update(hash, &hold.radius_cells.to_le_bytes());
            hash = fnv1a64_update(hash, &hold.hold_ms.to_le_bytes());
            match hold.held_by {
                Some((team, since_ms)) => {
                    hash = fnv1a64_update(
                        hash,
                        &[
                            1_u8,
                            match team {
                                crate::arena::state::Team::Red => 0_u8,
                                crate::arena::state::Team::Blue => 1_u8,
                            },
                        ],
                    );
                    hash = fnv1a64_update(hash, &since_ms.to_le_bytes());
                }
                None => hash = fnv1a64_update(hash, &[0_u8]),
            }
        }
        None => hash = fnv1a64_update(hash, &[0_u8]),
    }
    hash
}

/// **One unit mind's row** (17b cut M1a.5; item 3c of [`state_hash`]).
fn fold_mind(mut hash: u64, unit: mush_core::ObjId, mind: &crate::arena::mind::MindState) -> u64 {
    use crate::arena::perception::MindObjective;
    use crate::deliberation::order::RangeBand;
    hash = fnv1a64_update(hash, &unit.0.to_le_bytes());
    hash = fnv1a64_update(hash, &mind.think_seq.to_le_bytes());
    // 17b cut M3.2: the answer the pending row carries (`epochs`, `intents`,
    // `memory`), `dropped` and `seal_digest` are NOT folded here until 17b cut
    // M3.6 — this head's item 3c says why.
    match mind.pending.as_ref() {
        Some(pending) => {
            hash = fnv1a64_update(hash, &[1_u8]);
            hash = fnv1a64_update(hash, &pending.think_seq.to_le_bytes());
            hash = fnv1a64_update(hash, &pending.t_ms.to_le_bytes());
        }
        None => hash = fnv1a64_update(hash, &[0_u8]),
    }
    hash = fnv1a64_update(hash, &(mind.memory.len() as u64).to_le_bytes());
    hash = fnv1a64_update(
        hash,
        &fnv1a64_update(FNV_OFFSET_BASIS, &mind.memory).to_le_bytes(),
    );
    hash = fnv1a64_update(hash, &(mind.notices.len() as u64).to_le_bytes());
    for notice in &mind.notices {
        // Canonical bytes: a derive over closed enums, integers and words, so
        // the same notice is the same bytes on every run. A notice that does not
        // serialise cannot exist (every payload is plain data); it would fold
        // as no bytes rather than take the arena down.
        hash = fnv1a64_update(hash, &serde_json::to_vec(notice).unwrap_or_default());
    }
    hash = fnv1a64_update(hash, &(mind.ranges.len() as u64).to_le_bytes());
    for (contact, range_m) in &mind.ranges {
        hash = fnv1a64_update(hash, &contact.0.to_le_bytes());
        hash = fnv1a64_update(hash, &range_m.to_le_bytes());
    }
    match mind.order.as_ref() {
        Some(order) => {
            hash = fnv1a64_update(hash, &[1_u8, order.objective.code()]);
            match order.objective {
                MindObjective::Advance { to } => {
                    hash = fnv1a64_update(hash, &to.0.to_le_bytes());
                    hash = fnv1a64_update(hash, &to.1.to_le_bytes());
                }
                MindObjective::Engage { target } => {
                    hash = fnv1a64_update(hash, &target.map_or(0, |id| id.0).to_le_bytes());
                }
                MindObjective::Screen { facing_mdeg } => {
                    hash = fnv1a64_update(hash, &facing_mdeg.to_le_bytes());
                }
                MindObjective::Support { of } => {
                    hash = fnv1a64_update(hash, &of.map_or(0, |id| id.0).to_le_bytes());
                }
                MindObjective::Hold | MindObjective::Withdraw => {}
            }
            let band = match order.max_range_band {
                None => 0_u8,
                Some(RangeBand::Close) => 1,
                Some(RangeBand::Effective) => 2,
                Some(RangeBand::Far) => 3,
            };
            hash = fnv1a64_update(hash, &[u8::from(order.hold_fire), band]);
            hash = fnv1a64_update(hash, &order.expires_ms.to_le_bytes());
            hash = fnv1a64_update(hash, order.squad.as_bytes());
        }
        None => hash = fnv1a64_update(hash, &[0_u8]),
    }
    hash = fnv1a64_update(hash, &[mind.strikes]);
    // **17b cut M1b.1**: the card, its fields in declaration order — an enum as
    // its code byte, a bool as one byte, an integer little-endian.
    fold_card(hash, &mind.card)
}

/// **One mind's card** (17b cut M1b.1; inside item 3c of [`state_hash`], after
/// `strikes`).
fn fold_card(mut hash: u64, card: &crate::arena::card::StockCard) -> u64 {
    hash = fnv1a64_update(hash, &[card.stance.code()]);
    hash = fnv1a64_update(hash, &card.engage_band_permille.to_le_bytes());
    hash = fnv1a64_update(hash, &card.close_m.to_le_bytes());
    hash = fnv1a64_update(
        hash,
        &[card.orbit_sense.code(), u8::from(card.turn_into_hits)],
    );
    hash = fnv1a64_update(hash, &card.turn_s.to_le_bytes());
    hash = fnv1a64_update(hash, &card.screen_bearing.to_le_bytes());
    hash = fnv1a64_update(hash, &card.screen_arc.to_le_bytes());
    hash = fnv1a64_update(hash, &[card.opening.code()]);
    hash = fnv1a64_update(hash, &card.min_odds_permille.to_le_bytes());
    hash = fnv1a64_update(hash, &[card.aspect.code()]);
    hash = fnv1a64_update(hash, &card.reserve_pct.to_le_bytes());
    hash = fnv1a64_update(hash, &card.reserve_odds_pct.to_le_bytes());
    hash = fnv1a64_update(hash, &[card.prefer.code()]);
    hash = fnv1a64_update(hash, &card.weight_range.to_le_bytes());
    hash = fnv1a64_update(hash, &card.weight_facet.to_le_bytes());
    hash = fnv1a64_update(hash, &card.weight_health.to_le_bytes());
    hash = fnv1a64_update(hash, &card.switch_m.to_le_bytes());
    hash = fnv1a64_update(hash, &card.withdraw_permille.to_le_bytes());
    hash = fnv1a64_update(hash, &card.stand_min_permille.to_le_bytes());
    hash = fnv1a64_update(hash, &card.rally_ms.to_le_bytes());
    hash = fnv1a64_update(hash, &card.search_ms.to_le_bytes());
    hash = fnv1a64_update(hash, &card.hunt_after_ms.to_le_bytes());
    fnv1a64_update(hash, &[u8::from(card.stop_unseen)])
}

/// **A motor row's avoided cells** (16b cut Q.7's encoding; one function since
/// 17b cut M1a.1 so items 3 and 3b cannot fold them differently): their count as
/// `u64`, then per entry the cell's two `u16` and the stop's `i64` stamp.
fn fold_avoid(mut hash: u64, avoid: &[((u16, u16), i64)]) -> u64 {
    hash = fnv1a64_update(hash, &(avoid.len() as u64).to_le_bytes());
    for ((x, y), at_ms) in avoid {
        hash = fnv1a64_update(hash, &x.to_le_bytes());
        hash = fnv1a64_update(hash, &y.to_le_bytes());
        hash = fnv1a64_update(hash, &at_ms.to_le_bytes());
    }
    hash
}

/// **A motor row's search start and remembered stop** (16b cut AI.1's encoding;
/// one function since 17b cut M1a.1 so items 3 and 3b cannot fold them
/// differently): `searched_from` as a presence byte then the cell's two `u16`,
/// and `refused` as `[0]` alone for none, else `[1]`, the stand cell's and the
/// refused cell's two `u16` each, the cause's code byte, the goal's two `u16`
/// and the `i64` it lapses at.
fn fold_search(mut hash: u64, motor: &crate::arena::motor::MotorRow) -> u64 {
    match motor.searched_from {
        Some((x, y)) => {
            hash = fnv1a64_update(hash, &[1_u8]);
            hash = fnv1a64_update(hash, &x.to_le_bytes());
            hash = fnv1a64_update(hash, &y.to_le_bytes());
        }
        None => hash = fnv1a64_update(hash, &[0_u8]),
    }
    match motor.refused {
        Some(refusal) => {
            hash = fnv1a64_update(hash, &[1_u8]);
            hash = fnv1a64_update(hash, &refusal.at.0.to_le_bytes());
            hash = fnv1a64_update(hash, &refusal.at.1.to_le_bytes());
            hash = fnv1a64_update(hash, &refusal.cell.0.to_le_bytes());
            hash = fnv1a64_update(hash, &refusal.cell.1.to_le_bytes());
            hash = fnv1a64_update(hash, &[refusal.cause.code()]);
            hash = fnv1a64_update(hash, &refusal.goal.0.to_le_bytes());
            hash = fnv1a64_update(hash, &refusal.goal.1.to_le_bytes());
            hash = fnv1a64_update(hash, &refusal.until_ms.to_le_bytes());
        }
        None => hash = fnv1a64_update(hash, &[0_u8]),
    }
    hash
}

/// **One contact's reading** (16b cut Q.2): a presence byte, then the look's own
/// stamp, the packed bands and how many sections are packed.
///
/// A contact carries two of them — what this platform's own sensors read and what
/// it holds, which may be a teammate's — and they are folded by one function so
/// they cannot fold differently, exactly as [`fold_plot`]'s two plots are.
fn fold_reading(hash: u64, reading: Option<crate::arena::condition::Reading>) -> u64 {
    match reading {
        Some(reading) => {
            let hash = fnv1a64_update(hash, &[1_u8]);
            let hash = fnv1a64_update(hash, &reading.at_ms.to_le_bytes());
            let hash = fnv1a64_update(hash, &reading.condition.bands.to_le_bytes());
            fnv1a64_update(hash, &[reading.condition.sections])
        }
        None => fnv1a64_update(hash, &[0_u8]),
    }
}

/// **One optional arena timestamp** (16b cut Q.7): a presence byte, then the
/// `i64`.
///
/// `Withdraw`'s two stamps are folded by one function so they cannot fold
/// differently, for the reason [`fold_plot`] and [`fold_reading`] are one
/// function each.
fn fold_stamp(hash: u64, at_ms: Option<i64>) -> u64 {
    match at_ms {
        Some(at_ms) => {
            let hash = fnv1a64_update(hash, &[1_u8]);
            fnv1a64_update(hash, &at_ms.to_le_bytes())
        }
        None => fnv1a64_update(hash, &[0_u8]),
    }
}

/// **One contact plot's five integers** (16b cut F.3), in field order.
///
/// A contact carries two of them — the plot every reader reads, and the plot of
/// this platform's own last sighting — and they are folded by one function so
/// they cannot fold differently.
fn fold_plot(hash: u64, plot: &crate::arena::state::SeenPose) -> u64 {
    let hash = fnv1a64_update(hash, &plot.x_cm.to_le_bytes());
    let hash = fnv1a64_update(hash, &plot.y_cm.to_le_bytes());
    let hash = fnv1a64_update(hash, &plot.z_cm.to_le_bytes());
    let hash = fnv1a64_update(hash, &plot.heading_mdeg.to_le_bytes());
    fnv1a64_update(hash, &plot.speed_cms.to_le_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::clock::{Tempo, TempoSignals};
    use crate::arena::state::{DueKind, Power, Team};
    use crate::arena::test_support::{arena_of, pose, tank};
    use crate::config::TempoConfig;
    use mush_core::ObjId;

    fn two_tanks() -> Arena {
        arena_of(vec![
            tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
            tank(ObjId(9), Team::Blue, pose(6_500, 6_500)),
        ])
    }

    #[test]
    fn fnv_folds_bytes_in_order() {
        assert_eq!(fnv1a64_update(FNV_OFFSET_BASIS, b""), FNV_OFFSET_BASIS);
        // Order matters, which is what makes the length-prefix-free field list
        // above safe only because every field has a fixed width.
        assert_ne!(
            fnv1a64_update(FNV_OFFSET_BASIS, b"ab"),
            fnv1a64_update(FNV_OFFSET_BASIS, b"ba")
        );
        // Chaining is the same as one pass.
        let split = fnv1a64_update(fnv1a64_update(FNV_OFFSET_BASIS, b"ab"), b"cd");
        assert_eq!(split, fnv1a64_update(FNV_OFFSET_BASIS, b"abcd"));
    }

    #[test]
    fn the_hash_is_stable_for_one_state() {
        let arena = two_tanks();
        assert_eq!(state_hash(&arena), state_hash(&arena));
        // And identical for an independently built identical arena.
        assert_eq!(state_hash(&arena), state_hash(&two_tanks()));
    }

    #[test]
    fn the_hash_changes_when_a_pose_changes() {
        let mut arena = two_tanks();
        let before = state_hash(&arena);
        if let Some(unit) = arena.unit_mut(ObjId(4)) {
            unit.pose.x_cm += 1;
        }
        let moved = state_hash(&arena);
        assert_ne!(before, moved);
        // One centimetre back is the same state again.
        if let Some(unit) = arena.unit_mut(ObjId(4)) {
            unit.pose.x_cm -= 1;
        }
        assert_eq!(state_hash(&arena), before);
        // Each pose field separately.
        for mutate in [
            (|unit: &mut crate::arena::state::UnitState| unit.pose.y_cm += 1) as fn(&mut _),
            |unit| unit.pose.z_cm += 1,
            |unit| unit.pose.heading_mdeg += 1,
            |unit| unit.speed_cms += 1,
            |unit| unit.power = Power::Online,
            |unit| unit.limits.slow_pct = 50,
            |unit| unit.limits.no_turn_left = true,
            |unit| unit.limits.no_turn_right = true,
            |unit| unit.limits.motion_disabled = true,
            |unit| unit.limits.wreck = true,
        ] {
            let mut probe = two_tanks();
            if let Some(unit) = probe.unit_mut(ObjId(9)) {
                mutate(unit);
            }
            assert_ne!(state_hash(&probe), before, "a change went unhashed");
        }
    }

    #[test]
    fn the_hash_folds_the_tempo_the_clock_the_schedule_and_the_ledger() {
        let base = state_hash(&two_tanks());

        let mut tempo = two_tanks();
        tempo.clock.evaluate(
            0,
            &TempoSignals {
                shot_fired: true,
                ..TempoSignals::quiet()
            },
            &TempoConfig::default(),
        );
        assert_eq!(tempo.clock.tempo(), Tempo::Engaged);
        assert_ne!(state_hash(&tempo), base);

        let mut later = two_tanks();
        later.now_ms = 1;
        assert_ne!(state_hash(&later), base);

        let mut scheduled = two_tanks();
        scheduled.due.schedule(1_000, DueKind::PositionSummary);
        assert_ne!(state_hash(&scheduled), base);

        let mut journalled = two_tanks();
        let _ = journalled.ledger.append(
            0,
            crate::arena::ledger::EventKind::Startup { unit: ObjId(4) },
            Vec::new(),
            0,
        );
        assert_ne!(state_hash(&journalled), base);
    }

    /// **16b cut W.1's extension**: the lock, the two weapon cells and the in-flight
    /// shot count are each a fact that moves without an event of its own, so each has
    /// to move the hash.
    #[test]
    fn the_hash_folds_the_lock_the_weapon_cells_and_the_shots_in_flight() {
        let base = state_hash(&two_tanks());
        for mutate in [
            (|unit: &mut crate::arena::state::UnitState| {
                unit.lock = Some(crate::arena::state::ContactId(7));
            }) as fn(&mut _),
            |unit| {
                unit.ready_at_ms
                    .insert(smol_str::SmolStr::new("cannon"), 1_000);
            },
            |unit| {
                unit.ammo.insert(smol_str::SmolStr::new("cannon"), 40);
            },
        ] {
            let mut probe = two_tanks();
            if let Some(unit) = probe.unit_mut(ObjId(9)) {
                mutate(unit);
            }
            assert_ne!(state_hash(&probe), base, "a weapon cell went unhashed");
        }
        // Two weapons that swapped cycles are not one state, which is why the id's
        // bytes are folded beside the value.
        let mut swapped = two_tanks();
        if let Some(unit) = swapped.unit_mut(ObjId(9)) {
            unit.ready_at_ms
                .insert(smol_str::SmolStr::new("cannon"), 1_000);
            unit.ready_at_ms
                .insert(smol_str::SmolStr::new("machine_gun"), 2_000);
        }
        let mut other = two_tanks();
        if let Some(unit) = other.unit_mut(ObjId(9)) {
            unit.ready_at_ms
                .insert(smol_str::SmolStr::new("cannon"), 2_000);
            unit.ready_at_ms
                .insert(smol_str::SmolStr::new("machine_gun"), 1_000);
        }
        assert_ne!(state_hash(&swapped), state_hash(&other));
    }

    /// **16b cut X.6's extension**: the slope a unit entered its cell by is a
    /// fact with one writer that moves without an event, so it has to move the
    /// hash. The limit beside it never moves, so it deliberately does not.
    #[test]
    fn the_hash_folds_the_slope_a_unit_entered_its_cell_by() {
        let base = state_hash(&two_tanks());
        let mut climbed = two_tanks();
        if let Some(unit) = climbed.unit_mut(ObjId(9)) {
            unit.grade_pct = 30;
        }
        assert_ne!(state_hash(&climbed), base, "a grade went unhashed");
        // Back to the flat is the same state again.
        if let Some(unit) = climbed.unit_mut(ObjId(9)) {
            unit.grade_pct = 0;
        }
        assert_eq!(state_hash(&climbed), base);
        // The limit is a fact about the fit, not about the match.
        let mut limited = two_tanks();
        if let Some(unit) = limited.unit_mut(ObjId(9)) {
            unit.slope_limit_pct = Some(45);
        }
        assert_eq!(state_hash(&limited), base);
        // **16b cut AE.1**: and so is the row it pays, for the same reason.
        let mut walker = two_tanks();
        if let Some(unit) = walker.unit_mut(ObjId(9)) {
            unit.cost_row = crate::arena::state::CostRow::Legged;
        }
        assert_eq!(state_hash(&walker), base, "the cost row is a fit constant");
    }

    /// **16b cut E.3's extension**: a traffic hold is a fact with one writer that
    /// decides a later refusal and moves with no ledger line at all, so it has to
    /// move the hash — and the cell and the stamp each on their own, because a
    /// hold against a different cell is a different hold and a hold that opened
    /// earlier matures earlier.
    #[test]
    fn the_hash_folds_the_cell_a_unit_is_held_against_and_since_when() {
        use crate::arena::state::OccupiedHold;

        let base = state_hash(&two_tanks());
        let mut held = two_tanks();
        if let Some(unit) = held.unit_mut(ObjId(9)) {
            unit.occupied_hold = Some(OccupiedHold {
                cell: (4, 3),
                since_ms: 2_000,
            });
        }
        let first = state_hash(&held);
        assert_ne!(first, base, "a hold went unhashed");
        // The cell alone.
        if let Some(unit) = held.unit_mut(ObjId(9)) {
            unit.occupied_hold = Some(OccupiedHold {
                cell: (3, 4),
                since_ms: 2_000,
            });
        }
        assert_ne!(state_hash(&held), first, "the held cell went unhashed");
        // The stamp alone.
        if let Some(unit) = held.unit_mut(ObjId(9)) {
            unit.occupied_hold = Some(OccupiedHold {
                cell: (4, 3),
                since_ms: 2_250,
            });
        }
        assert_ne!(state_hash(&held), first, "the hold's stamp went unhashed");
        // And letting it go is the state it was in before.
        if let Some(unit) = held.unit_mut(ObjId(9)) {
            unit.occupied_hold = None;
        }
        assert_eq!(state_hash(&held), base);
    }

    /// **16b cut Q.1's extension**: a provisional number moves without a
    /// distinguishable ledger event — a mesh `detected` and a sensor `detected`
    /// are one shape — so the book has to move the hash. The unit's own callsign
    /// does not: it is a function of the check-out and never moves.
    #[test]
    fn the_hash_folds_the_designator_book_and_not_the_callsigns() {
        let base = state_hash(&two_tanks());
        let mut issued = two_tanks();
        assert_eq!(issued.designators.assign(ObjId(4), ObjId(9)), 1);
        assert_ne!(state_hash(&issued), base, "a number went unhashed");
        // Two viewers that issued the same number for different targets are not
        // one state.
        let mut other = two_tanks();
        assert_eq!(other.designators.assign(ObjId(9), ObjId(4)), 1);
        assert_ne!(state_hash(&other), state_hash(&issued));
        // The callsign is a fact about the check-out, not about the match.
        let mut renamed = two_tanks();
        if let Some(unit) = renamed.unit_mut(ObjId(9)) {
            unit.callsign = crate::arena::state::Designator::Slot {
                team: Team::Blue,
                slot: 9,
            };
        }
        assert_eq!(state_hash(&renamed), base);
    }

    /// **16b cut Q.6's extension**: each of the four motion carries is the
    /// fractional part of a quantity already in the hash, so each has to move it
    /// on its own — a replay whose carries differ is a replay that is about to
    /// disagree about a whole centimetre.
    #[test]
    fn the_hash_folds_each_of_the_four_motion_carries() {
        let base = state_hash(&two_tanks());
        for mutate in [
            (|unit: &mut crate::arena::state::UnitState| unit.carry.turn = 1) as fn(&mut _),
            |unit| unit.carry.speed = 1,
            |unit| unit.carry.east = 1,
            |unit| unit.carry.north = 1,
            // 16b cut M.4 (`T-16`): the two sign bytes, each on its own.
            |unit| unit.carry.turn_side = 1,
            |unit| unit.carry.speed_rate = 2,
        ] {
            let mut probe = two_tanks();
            if let Some(unit) = probe.unit_mut(ObjId(9)) {
                mutate(unit);
            }
            assert_ne!(state_hash(&probe), base, "a carry went unhashed");
        }
        // And the two sign bytes are not one number either: a left turn's
        // carry and an accelerating one are different states.
        let mut left = two_tanks();
        if let Some(unit) = left.unit_mut(ObjId(9)) {
            unit.carry.turn_side = 1;
        }
        let mut accel = two_tanks();
        if let Some(unit) = accel.unit_mut(ObjId(9)) {
            unit.carry.speed_rate = 1;
        }
        assert_ne!(state_hash(&left), state_hash(&accel));
        // The four are separate fields and not one number: a carry moved from one
        // to another is a different state.
        let mut east = two_tanks();
        if let Some(unit) = east.unit_mut(ObjId(9)) {
            unit.carry.east = 7;
        }
        let mut north = two_tanks();
        if let Some(unit) = north.unit_mut(ObjId(9)) {
            unit.carry.north = 7;
        }
        assert_ne!(state_hash(&east), state_hash(&north));
        // And back to nothing carried is the same state again.
        if let Some(unit) = east.unit_mut(ObjId(9)) {
            unit.carry.east = 0;
        }
        assert_eq!(state_hash(&east), base);
    }

    /// **16b cut D.2's extension**: the spent-section window is persistent state
    /// that decides what a later substep says, so it has to move the hash — and
    /// the scratch beside it is not, so it must not.
    #[test]
    fn the_hash_folds_the_spent_section_window_and_not_the_spent_shot_scratch() {
        let base = state_hash(&two_tanks());
        let mut said = two_tanks();
        said.spent_said.insert(
            (ObjId(4), ObjId(9), smol_str::SmolStr::new("turret")),
            5_000,
        );
        assert_ne!(
            state_hash(&said),
            base,
            "one window entry alone went unhashed"
        );
        // The stamp is part of it: two runs that said the same line at different
        // arena times are about to suppress different later ones.
        let mut later = two_tanks();
        later.spent_said.insert(
            (ObjId(4), ObjId(9), smol_str::SmolStr::new("turret")),
            5_001,
        );
        assert_ne!(state_hash(&later), state_hash(&said));
        // And so is each of the three key parts.
        for key in [
            (ObjId(9), ObjId(9), smol_str::SmolStr::new("turret")),
            (ObjId(4), ObjId(4), smol_str::SmolStr::new("turret")),
            (ObjId(4), ObjId(9), smol_str::SmolStr::new("hull")),
        ] {
            let mut probe = two_tanks();
            probe.spent_said.insert(key, 5_000);
            assert_ne!(state_hash(&probe), state_hash(&said), "a key part is lost");
        }
        // Taking it out again is the same state.
        said.spent_said.clear();
        assert_eq!(state_hash(&said), base);
        // The scratch is deliberately absent: it is a function of state this hash
        // already folds and it never survives its own substep.
        let mut scratch = two_tanks();
        scratch.spent_hits.insert(7);
        assert_eq!(state_hash(&scratch), base, "the scratch was folded");
    }

    /// **16b cut D.4's extension**: the two facts that move with no
    /// distinguishable ledger event of their own — whether a hold was opened
    /// under a number, and what the platform's own receiver is running under.
    #[test]
    fn the_hash_folds_whether_a_hold_was_numbered_and_the_platforms_own_receiver() {
        use crate::arena::perception::Receiver;
        use crate::arena::state::{Classification, ContactId, SeenPose, ShadowContact};

        let base = state_hash(&two_tanks());

        // `numbered` alone: two arenas identical but for the one byte. A mesh
        // arrival and a contact this platform's own sight opened journal the same
        // `detected` line, so this is the only thing that tells them apart.
        let hold = |numbered: bool| {
            let mut probe = two_tanks();
            probe.contacts.insert(
                ObjId(4),
                ContactId(7),
                ShadowContact {
                    numbered,
                    ..ShadowContact::sighted(
                        ObjId(9),
                        Classification::Unknown,
                        1_000,
                        SeenPose::default(),
                    )
                },
            );
            state_hash(&probe)
        };
        assert_ne!(hold(false), base, "the contact itself went unhashed");
        assert_ne!(hold(true), hold(false), "`numbered` alone went unhashed");

        // `receiver` alone, and each of its three integers on its own: two
        // stacked pods and one pod are not one state, and neither is a filter
        // that recovered half the reach.
        let receiver = Receiver {
            budget: 15,
            budget_max: 40,
            reach_permille: 600,
        };
        let with = |receiver: Option<Receiver>| {
            let mut probe = two_tanks();
            if let Some(unit) = probe.unit_mut(ObjId(9)) {
                unit.receiver = receiver;
            }
            state_hash(&probe)
        };
        assert_ne!(with(Some(receiver)), base, "a receiver went unhashed");
        for moved in [
            Receiver {
                budget: 0,
                ..receiver
            },
            Receiver {
                budget_max: 100,
                ..receiver
            },
            Receiver {
                reach_permille: 601,
                ..receiver
            },
        ] {
            assert_ne!(
                with(Some(moved)),
                with(Some(receiver)),
                "a receiver field went unhashed"
            );
        }
        // And clean again is the same state: the fold is a function of the value
        // and not of how it got there.
        assert_eq!(with(None), base);
    }

    /// **16b cut E.2's extension** (`B-45`): whether the platform holding a
    /// contact watched its target die. The `destroyed` line that moves it names
    /// the victim and the shooter and never the holder, so nothing but this fold
    /// tells two replays apart — and the static mask beside it never moves, so
    /// it must not be folded at all.
    #[test]
    fn the_hash_folds_whether_a_hold_knows_its_target_is_dead_and_not_the_fatal_mask() {
        use crate::arena::state::{Classification, ContactId, SeenPose, ShadowContact};

        let base = state_hash(&two_tanks());
        let hold = |known_dead: bool| {
            let mut probe = two_tanks();
            probe.contacts.insert(
                ObjId(4),
                ContactId(7),
                ShadowContact {
                    known_dead,
                    ..ShadowContact::sighted(
                        ObjId(9),
                        Classification::Unknown,
                        1_000,
                        SeenPose::default(),
                    )
                },
            );
            state_hash(&probe)
        };
        assert_ne!(hold(false), base, "the contact itself went unhashed");
        assert_ne!(hold(true), hold(false), "`known_dead` alone went unhashed");

        // The mask is a fact about the unit's fit, not about the match.
        let mut fitted = two_tanks();
        if let Some(unit) = fitted.unit_mut(ObjId(9)) {
            unit.fatal_sections = 0b101;
        }
        assert_eq!(state_hash(&fitted), base, "the fatal mask was folded");
    }

    /// **16b cut M.3's extension** (16c `T-04`): what a unit is held against
    /// moves the hash — its presence, and each of the cell and the cause on its
    /// own, because a different cell or a different cause is a new edge — and
    /// letting it go is the state before. The per-substep flag beside it does not.
    #[test]
    fn m3_the_hash_folds_what_a_unit_is_held_against_cell_and_cause() {
        use crate::arena::motion::{BlockedCause, Held};

        let base = state_hash(&two_tanks());
        let held = |x: u16, y: u16, cause: BlockedCause| {
            let mut probe = two_tanks();
            if let Some(unit) = probe.unit_mut(ObjId(9)) {
                unit.held = Some(Held { x, y, cause });
            }
            state_hash(&probe)
        };
        let first = held(4, 3, BlockedCause::Impassable);
        assert_ne!(first, base, "a held unit went unhashed");
        assert_ne!(held(3, 3, BlockedCause::Impassable), first, "the column");
        assert_ne!(held(4, 4, BlockedCause::Impassable), first, "the row");
        for cause in [
            BlockedCause::Occupied,
            BlockedCause::Edge,
            BlockedCause::TooSteep,
        ] {
            assert_ne!(
                held(4, 3, cause),
                first,
                "a different cause at the same cell went unhashed: {cause:?}"
            );
        }
        // The flag is not the condition, and is not folded.
        let mut flagged = two_tanks();
        if let Some(unit) = flagged.unit_mut(ObjId(9)) {
            unit.blocked = true;
        }
        assert_eq!(
            state_hash(&flagged),
            base,
            "the per-substep flag was folded"
        );
        // Letting go is the state it was in before.
        let mut released = two_tanks();
        if let Some(unit) = released.unit_mut(ObjId(9)) {
            unit.held = Some(Held {
                x: 4,
                y: 3,
                cause: BlockedCause::Impassable,
            });
            unit.held = None;
        }
        assert_eq!(state_hash(&released), base);
    }

    /// **16b cut M.3's extension** (16c `T-06`), the carry probe's shape: heat
    /// gates firing and moves every substep with no line, so the heat cell and
    /// its carry each move the hash on their own.
    #[test]
    fn m3_the_hash_folds_the_heat_cell_and_the_heat_carry() {
        use crate::arena::test_support::laser_tank;

        let hot = || {
            arena_of(vec![
                tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
                laser_tank(ObjId(9), Team::Blue, pose(6_500, 6_500)),
            ])
        };
        let base = state_hash(&hot());
        assert_eq!(state_hash(&hot()), base, "the fixture is stable");
        // The heat cell, through the one writer the arena uses for it: one
        // second's rise — a shot's worth — as `raise_heat` would apply it.
        let mut raised = hot();
        let addr = raised
            .unit(ObjId(9))
            .and_then(|unit| unit.heat.as_ref())
            .and_then(|heat| heat.addr(crate::definitions::unit::HEAT_CELL))
            .unwrap_or_else(|| panic!("the laser tank carries a heat cell"));
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetCell { addr, value: 7 }],
            None,
            None,
        )
        .unwrap_or_else(|_| panic!("a one-op plan"));
        let post = raised
            .unit(ObjId(9))
            .map(|unit| mush_tactics::apply(&unit.unit, &plan, 0))
            .and_then(Result::ok)
            .map(|(post, _)| post)
            .unwrap_or_else(|| panic!("the plan applies"));
        raised.set_unit(ObjId(9), post, false);
        assert_eq!(
            raised.unit(ObjId(9)).and_then(|unit| unit.heat_now()),
            Some(7)
        );
        assert_ne!(state_hash(&raised), base, "the heat cell went unhashed");
        // The carry alone: the fraction of a heat unit the last substep did not
        // shed.
        let mut carried = hot();
        if let Some(heat) = carried
            .unit_mut(ObjId(9))
            .and_then(|unit| unit.heat.as_mut())
        {
            heat.carry_milli = 1;
        }
        assert_ne!(state_hash(&carried), base, "the heat carry went unhashed");
        assert_ne!(state_hash(&carried), state_hash(&raised));
        // And back to cold with nothing carried is the same state again.
        if let Some(heat) = carried
            .unit_mut(ObjId(9))
            .and_then(|unit| unit.heat.as_mut())
        {
            heat.carry_milli = 0;
        }
        assert_eq!(state_hash(&carried), base);
    }

    /// **16b cut M.1's extension** (`T-82`): the seat-order rate's window and
    /// count each move the hash on their own — they decide a later `queue_full`.
    #[test]
    fn the_hash_folds_the_seat_order_window_and_its_count() {
        let base = state_hash(&two_tanks());
        let mut windowed = two_tanks();
        if let Some(unit) = windowed.unit_mut(ObjId(9)) {
            unit.orders_window_s = 3;
        }
        assert_ne!(state_hash(&windowed), base, "the window went unhashed");
        let mut counted = two_tanks();
        if let Some(unit) = counted.unit_mut(ObjId(9)) {
            unit.orders_in_window = 3;
        }
        assert_ne!(state_hash(&counted), base, "the count went unhashed");
        assert_ne!(state_hash(&counted), state_hash(&windowed));
        if let Some(unit) = counted.unit_mut(ObjId(9)) {
            unit.orders_in_window = 0;
        }
        assert_eq!(state_hash(&counted), base);
    }

    /// **16b cut M.2's extension** (`T-83`): a held loss moves the hash — its
    /// viewer, its contact and its deadline each on their own — its expiry moves
    /// it back, and the designator carried beside the deadline is not folded.
    #[test]
    fn m2_the_hash_folds_a_held_loss_and_its_expiry_moves_it_back() {
        use crate::arena::state::{ContactId, Designator};

        let base = state_hash(&two_tanks());
        let held = |viewer: u32, contact: u64, deadline: i64, n: u16| {
            let mut probe = two_tanks();
            probe.held_losses.insert(
                (ObjId(viewer), ContactId(contact)),
                (deadline, Designator::Provisional(n)),
            );
            probe
        };
        let first = state_hash(&held(4, 7, 2_004, 1));
        assert_ne!(first, base, "a held loss went unhashed");
        assert_ne!(state_hash(&held(9, 7, 2_004, 1)), first, "the viewer");
        assert_ne!(state_hash(&held(4, 8, 2_004, 1)), first, "the contact");
        assert_ne!(state_hash(&held(4, 7, 2_005, 1)), first, "the deadline");
        assert_eq!(
            state_hash(&held(4, 7, 2_004, 2)),
            first,
            "the designator was folded"
        );
        // The expiry takes it out again: the same state as before the loss.
        let mut expired = held(4, 7, 2_004, 1);
        expired.held_losses.remove(&(ObjId(4), ContactId(7)));
        assert_eq!(state_hash(&expired), base);
    }

    #[test]
    fn the_hash_does_not_fold_the_orders_only_their_effect() {
        // A pulse that has accepted a helm order but not yet integrated it is
        // the same world as one that has not: the replay compares what the
        // world became.
        let mut ordered = two_tanks();
        if let Some(unit) = ordered.unit_mut(ObjId(4)) {
            unit.desired_heading_mdeg = 90_000;
            unit.desired_speed_cms = 500;
        }
        assert_eq!(state_hash(&ordered), state_hash(&two_tanks()));
    }

    #[test]
    fn two_units_swapping_positions_is_not_one_state() {
        // The guard against a hash that folds a multiset rather than a sequence.
        let swapped = arena_of(vec![
            tank(ObjId(4), Team::Red, pose(6_500, 6_500)),
            tank(ObjId(9), Team::Blue, pose(1_500, 1_500)),
        ]);
        assert_ne!(state_hash(&swapped), state_hash(&two_tanks()));
    }

    /// **16b cut O.1's extension** (item 12): the mode and the Hold clock move
    /// the hash — two arenas differing only in `held_by`, or only in its `since`,
    /// or only in the mode, are two states — and so does a machine's
    /// `hold_retry_ms`.
    #[test]
    fn o1_the_hash_folds_the_mode_the_hold_clock_and_the_hold_retry() {
        use crate::arena::state::{HoldState, MatchMode};
        let hold = HoldState {
            spot: (4, 4),
            letter: Some('A'),
            radius_cells: 1,
            hold_ms: 10_000,
            held_by: None,
        };
        let with = |mode: MatchMode, held: Option<HoldState>| {
            let mut arena = two_tanks();
            arena.mode = mode;
            arena.hold = held;
            state_hash(&arena)
        };
        let base = state_hash(&two_tanks());
        assert_eq!(with(MatchMode::Skirmish, None), base);
        let open = with(MatchMode::Hold, Some(hold));
        assert_ne!(open, base, "the mode and the objective went unhashed");
        assert_ne!(with(MatchMode::Hold, None), base, "the mode alone");
        let red = with(
            MatchMode::Hold,
            Some(HoldState {
                held_by: Some((Team::Red, 250)),
                ..hold
            }),
        );
        assert_ne!(red, open, "`held_by` went unhashed");
        let blue = with(
            MatchMode::Hold,
            Some(HoldState {
                held_by: Some((Team::Blue, 250)),
                ..hold
            }),
        );
        assert_ne!(blue, red, "the holding side went unhashed");
        let later = with(
            MatchMode::Hold,
            Some(HoldState {
                held_by: Some((Team::Red, 500)),
                ..hold
            }),
        );
        assert_ne!(later, red, "the clock's `since` went unhashed");
        let wider = with(
            MatchMode::Hold,
            Some(HoldState {
                radius_cells: 2,
                ..hold
            }),
        );
        assert_ne!(wider, open, "the radius went unhashed");

        // A machine's hold retry.
        let mut machine = two_tanks();
        machine.adopt_reflex(ObjId(4), crate::arena::reflex::ReflexState::Patrol);
        let adopted = state_hash(&machine);
        if let Some(row) = machine.motors.row_mut(ObjId(4)) {
            row.hold_retry_ms = Some(90_000);
        }
        assert_ne!(
            state_hash(&machine),
            adopted,
            "`hold_retry_ms` went unhashed"
        );
        if let Some(row) = machine.motors.row_mut(ObjId(4)) {
            row.hold_retry_ms = None;
        }
        assert_eq!(state_hash(&machine), adopted);
    }

    /// **16b cut AI.1's extension** (item 3): the cell a machine's last search
    /// started from, and the stop it remembers, each move the hash — and every
    /// field of the stop does, so a fold that wrote only its presence would be
    /// caught — and resetting either is the same state again.
    #[test]
    fn ai1_the_hash_folds_the_search_start_and_the_refusal_memory() {
        use crate::arena::motion::BlockedCause;
        use crate::arena::reflex::{ReflexState, Refusal};
        let mut machine = two_tanks();
        machine.adopt_reflex(ObjId(4), ReflexState::Patrol);
        let adopted = state_hash(&machine);

        if let Some(row) = machine.motors.row_mut(ObjId(4)) {
            row.searched_from = Some((1, 1));
        }
        let from = state_hash(&machine);
        assert_ne!(from, adopted, "`searched_from` went unhashed");
        if let Some(row) = machine.motors.row_mut(ObjId(4)) {
            row.searched_from = Some((1, 2));
        }
        assert_ne!(
            state_hash(&machine),
            from,
            "`searched_from`'s cell went unhashed"
        );
        if let Some(row) = machine.motors.row_mut(ObjId(4)) {
            row.searched_from = None;
        }
        assert_eq!(state_hash(&machine), adopted);

        let base = Refusal {
            at: (1, 1),
            cell: (2, 2),
            cause: BlockedCause::Impassable,
            goal: (6, 4),
            until_ms: 31_000,
        };
        let mut seen: Vec<u64> = vec![adopted];
        for refusal in [
            base,
            Refusal { at: (1, 2), ..base },
            Refusal {
                cell: (2, 1),
                ..base
            },
            Refusal {
                cause: BlockedCause::Edge,
                ..base
            },
            Refusal {
                goal: (6, 5),
                ..base
            },
            Refusal {
                until_ms: 31_001,
                ..base
            },
        ] {
            if let Some(row) = machine.motors.row_mut(ObjId(4)) {
                row.refused = Some(refusal);
            }
            let hash = state_hash(&machine);
            assert!(
                !seen.contains(&hash),
                "`refused` {refusal:?} folded the same bytes as an earlier state"
            );
            seen.push(hash);
        }
        if let Some(row) = machine.motors.row_mut(ObjId(4)) {
            row.refused = None;
        }
        assert_eq!(state_hash(&machine), adopted);
    }

    /// **16b cut O.2** (16c N12), item 1.19: two arenas differing only in one
    /// unit's pod switch differ, and switching it back is the same state again.
    #[test]
    fn o2_the_hash_folds_the_pods_switch() {
        let base = state_hash(&two_tanks());
        let mut silent = two_tanks();
        if let Some(unit) = silent.unit_mut(ObjId(9)) {
            assert!(unit.ecm_on, "every unit checks out radiating");
            unit.ecm_on = false;
        }
        assert_ne!(state_hash(&silent), base, "the switch went unhashed");
        if let Some(unit) = silent.unit_mut(ObjId(9)) {
            unit.ecm_on = true;
        }
        assert_eq!(state_hash(&silent), base);
    }

    /// **17b cut M1b.1** (items 3d, 3e, the card inside 3c): every new field
    /// moves the hash, and an arena with none folds no byte for them — the
    /// floor folds nothing at all.
    #[test]
    fn m1b1_slots_held_rows_and_the_card_move_the_hash_and_the_floor_does_not() {
        use crate::arena::conditions::{ASPECTS_ANY, FireCondition, SlotTarget};
        use crate::arena::state::ContactId;
        use smol_str::SmolStr;
        let base_arena = two_tanks();
        let base = state_hash(&base_arena);
        // The floor is a per-match constant: not folded.
        let mut floor = base_arena.clone();
        floor.odds_floor_permille = 50;
        assert_eq!(state_hash(&floor), base, "the floor folds nothing");
        // Item 3d: a slot, and each of its terms.
        let cond = FireCondition {
            weapon: SmolStr::new("cannon"),
            target: SlotTarget::Lock,
            min_permille: 100,
            aspects: ASPECTS_ANY,
            min_rounds_left: 0,
            min_range_m: 0,
            max_range_m: u32::MAX,
        };
        let armed_with = |cond: FireCondition| {
            let mut arena = base_arena.clone();
            let _ = arena.conditions.arm(ObjId(4), cond, 2);
            state_hash(&arena)
        };
        let armed = armed_with(cond.clone());
        assert_ne!(armed, base, "a slot moves the hash");
        let variants = [
            FireCondition {
                weapon: SmolStr::new("laser"),
                ..cond.clone()
            },
            FireCondition {
                target: SlotTarget::BestHostile,
                ..cond.clone()
            },
            FireCondition {
                target: SlotTarget::Contact { id: ContactId(1) },
                ..cond.clone()
            },
            FireCondition {
                target: SlotTarget::Contact { id: ContactId(2) },
                ..cond.clone()
            },
            FireCondition {
                min_permille: 110,
                ..cond.clone()
            },
            FireCondition {
                aspects: 0b0110,
                ..cond.clone()
            },
            FireCondition {
                min_rounds_left: 1,
                ..cond.clone()
            },
            FireCondition {
                min_range_m: 1,
                ..cond.clone()
            },
            FireCondition {
                max_range_m: 1,
                ..cond.clone()
            },
        ];
        let mut seen = vec![base, armed];
        for variant in variants {
            let hash = armed_with(variant.clone());
            assert!(!seen.contains(&hash), "{variant:?} folds as another slot");
            seen.push(hash);
        }
        // The slot's number is folded: the same terms in slot 2.
        let mut second = base_arena.clone();
        let _ = second.conditions.arm(
            ObjId(4),
            FireCondition {
                weapon: SmolStr::new("other"),
                ..cond.clone()
            },
            2,
        );
        let _ = second.conditions.arm(ObjId(4), cond.clone(), 2);
        let _ = second.conditions.disarm(ObjId(4), Some("other"));
        assert_ne!(state_hash(&second), armed, "slot 2 is not slot 1");
        // Item 3e: each field of a held-shot row.
        let held_with = |key: (ObjId, u8, ContactId), at: i64| {
            let mut arena = base_arena.clone();
            arena.held_said.insert(key, at);
            state_hash(&arena)
        };
        let row = held_with((ObjId(4), 1, ContactId(7)), 1_000);
        assert_ne!(row, base);
        for other in [
            held_with((ObjId(9), 1, ContactId(7)), 1_000),
            held_with((ObjId(4), 2, ContactId(7)), 1_000),
            held_with((ObjId(4), 1, ContactId(8)), 1_000),
            held_with((ObjId(4), 1, ContactId(7)), 1_001),
        ] {
            assert_ne!(other, row);
        }
        // The card, inside a mind's row: each section's terms move it.
        let mut minded = base_arena.clone();
        minded
            .minds
            .insert(ObjId(4), crate::arena::mind::MindState::default());
        let card_base = state_hash(&minded);
        // Every one of the card's 24 fields, each edit hashing apart from the
        // base and from every other edit (a dropped or a doubled field folds
        // two cards as one).
        use crate::arena::card::{AspectWord, Opening, OrbitSense, Prefer, Stance};
        let edits: [fn(&mut crate::arena::card::StockCard); 24] = [
            |card| card.stance = Stance::Orbit,
            |card| card.engage_band_permille += 1,
            |card| card.close_m += 1,
            |card| card.orbit_sense = OrbitSense::Cw,
            |card| card.turn_into_hits = !card.turn_into_hits,
            |card| card.turn_s += 1,
            |card| card.screen_bearing += 1,
            |card| card.screen_arc += 1,
            |card| card.opening = Opening::Search,
            |card| card.min_odds_permille += 1,
            |card| card.aspect = AspectWord::Rear,
            |card| card.reserve_pct += 1,
            |card| card.reserve_odds_pct += 1,
            |card| card.prefer = Prefer::BestOdds,
            |card| card.weight_range += 1,
            |card| card.weight_facet += 1,
            |card| card.weight_health += 1,
            |card| card.switch_m += 1,
            |card| card.withdraw_permille += 1,
            |card| card.stand_min_permille += 1,
            |card| card.rally_ms += 1,
            |card| card.search_ms += 1,
            |card| card.hunt_after_ms += 1,
            |card| card.stop_unseen = !card.stop_unseen,
        ];
        let mut seen = vec![card_base];
        for (at, edit) in edits.into_iter().enumerate() {
            let mut edited = minded.clone();
            if let Some(state) = edited.minds.get_mut(&ObjId(4)) {
                edit(&mut state.card);
                assert_ne!(
                    state.card,
                    minded.minds[&ObjId(4)].card,
                    "edit {at} changed nothing"
                );
            }
            let hash = state_hash(&edited);
            assert!(
                !seen.contains(&hash),
                "card edit {at} folds as another card"
            );
            seen.push(hash);
        }
    }

    /// **17b cut M3.2: carried, not yet folded** (this file's head, item 3c).
    /// Two arenas differing ONLY in a pending think's answer — its `intents`,
    /// its `memory`, any one of its three `epochs` — or in a mind row's
    /// `dropped` or `seal_digest`, hash EQUAL: M2's fold stands byte for byte
    /// until 17b cut M3.6 folds them with the per-field probes (and inverts
    /// this test). And two differing only in a unit's `seal` (every field of
    /// the `SealRef`) or in the arena's class table hash equal for good: per-
    /// match constants, never folded. The presence, `think_seq` and `t_ms` of
    /// the pending think still move it, as they did.
    #[test]
    fn m3_the_carried_answer_drop_seal_and_classes_are_not_yet_folded() {
        use crate::arena::core::{ClassTable, CoreClass};
        use crate::arena::intents::IntentKind;
        use crate::arena::mind::{
            DropCause, MindState, PendingThink, SealEngine, SealRef, SortieId,
        };
        use crate::arena::state::ContactId;
        let mut base = two_tanks();
        base.minds.insert(
            ObjId(4),
            MindState {
                pending: Some(PendingThink {
                    think_seq: 2,
                    t_ms: 2_000,
                    epochs: [0; 3],
                    intents: Vec::new(),
                    memory: Vec::new(),
                }),
                ..MindState::default()
            },
        );
        let before = state_hash(&base);
        let with_mind = |change: &dyn Fn(&mut MindState)| {
            let mut arena = base.clone();
            if let Some(state) = arena.minds.get_mut(&ObjId(4)) {
                change(state);
            }
            assert_ne!(
                arena.minds.get(&ObjId(4)),
                base.minds.get(&ObjId(4)),
                "the probe changed the row"
            );
            state_hash(&arena)
        };
        let pending = |change: &dyn Fn(&mut PendingThink)| {
            with_mind(&|state: &mut MindState| {
                if let Some(pending) = state.pending.as_mut() {
                    change(pending);
                }
            })
        };
        // Carried, not yet folded (17b cut M3.6 folds them).
        for (field, hash) in [
            (
                "pending.intents",
                pending(&|p| {
                    p.intents = vec![IntentKind::Lock {
                        contact: ContactId(7),
                    }];
                }),
            ),
            ("pending.memory", pending(&|p| p.memory = vec![1, 2, 3])),
            ("pending.epochs (helm)", pending(&|p| p.epochs = [1, 0, 0])),
            (
                "pending.epochs (sensors)",
                pending(&|p| p.epochs = [0, 1, 0]),
            ),
            ("pending.epochs (fire)", pending(&|p| p.epochs = [0, 0, 1])),
            (
                "dropped",
                with_mind(&|s| s.dropped = Some(DropCause::Strikes)),
            ),
            (
                "dropped (core)",
                with_mind(&|s| s.dropped = Some(DropCause::Core)),
            ),
            ("seal_digest", with_mind(&|s| s.seal_digest = [7; 32])),
        ] {
            assert_eq!(hash, before, "`{field}` is folded before 17b cut M3.6");
        }
        // What M2 folds still moves it.
        assert_ne!(pending(&|p| p.t_ms = 2_250), before, "pending.t_ms");
        assert_ne!(pending(&|p| p.think_seq = 3), before, "pending.think_seq");
        assert_ne!(
            with_mind(&|s| s.pending = None),
            before,
            "pending's presence"
        );

        // Never folded: the seal reference, every field of it.
        let sealed = |change: &dyn Fn(&mut SealRef)| {
            let mut arena = base.clone();
            if let Some(unit) = arena.unit_mut(ObjId(4)) {
                change(&mut unit.seal);
                assert_ne!(unit.seal, SealRef::none(), "the probe changed the seal");
            }
            state_hash(&arena)
        };
        for (field, hash) in [
            (
                "seal.sortie",
                sealed(&|s| s.sortie = SortieId::in_match(7, ObjId(4))),
            ),
            ("seal.digest", sealed(&|s| s.digest = [9; 32])),
            ("seal.core_class", sealed(&|s| s.core_class = CoreClass::II)),
            ("seal.class", sealed(&|s| s.class = CoreClass::I)),
            ("seal.engine", sealed(&|s| s.engine = SealEngine::Scripted)),
            ("seal.governed", sealed(&|s| s.governed = false)),
            ("seal.closed", sealed(&|s| s.closed = true)),
        ] {
            assert_eq!(hash, before, "`{field}` is folded");
        }
        // Never folded: the class table.
        let mut classes = base.clone();
        let stock = ClassTable::STOCK;
        let mut wire = serde_json::to_value(stock).expect("the table encodes");
        wire["II"]["slots"] = serde_json::json!(5);
        classes.classes = serde_json::from_value(wire).expect("a lawful table");
        assert_ne!(classes.classes, stock, "the probe changed the table");
        assert_eq!(state_hash(&classes), before, "the class table is folded");
    }
}
