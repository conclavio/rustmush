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
///    8. the four limit bytes: `slow_pct`; `no_turn_toward` (`0` none, `1` left,
///       `2` right); `motion_disabled` (`0`/`1`); `wreck` (`0`/`1`);
///    9. **the lock** (16b cut W.1): the contact id as `u64`, or `0` for none, which
///       is a value [`crate::arena::rng::draw_contact_id`] never mints;
///    10. **the weapon cells** (16b cut W.1), in the two `BTreeMap`s' own key order:
///        the count of `ready_at_ms` rows as `u64`, then per row the weapon id's
///        bytes and its `i64`; then the same for `ammo` with its `u32`. The id's
///        bytes go in because a row is a `(weapon, value)` pair and folding only the
///        values would make two weapons that swapped cycles one state;
///    11. **the controller byte** (16b cut N.1): `0` pilot, `1` reflex;
///    12. **`grade_pct`** (16b cut X.6), `u16` — the uphill slope of the edge the
///        unit entered its cell by.
/// 2. **The reflex machines** (16b cut N.1), in unit-id order — the map's own, so
///    there is no iteration order to diverge on: per machine the unit's `u32`, the
///    state byte, the goal cell's two `u16` (`u16::MAX` twice for none), the
///    waypoint index as `u16` (`u16::MAX` for none), the route's length as `u64`
///    and the target unit's `u32` (`0` for none).
/// 3. **The shadow contact store** (16b cut S.1), in its own iteration order —
///    viewer object number ascending, then contact id ascending, which is what
///    the two `BTreeMap`s give: per contact the viewer's `u32`, the contact id's
///    `u64`, the target's `u32`, the class byte (`0` unknown, `1` classified red,
///    `2` classified blue), `first_seen_ms` and `last_seen_ms` as `i64`; then the
///    plot's five integers, the own sighting (a presence byte, its stamp, its own
///    plot and **its reading**) and **the reading the platform holds** — a reading
///    being a presence byte, the look's `i64` stamp, the packed bands as `u32` and
///    the packed section count as `u8` (16b cut Q.2).
/// 4. **The designator book** (16b cut Q.1), in its own two `BTreeMap`s' order:
///    the count of `next` rows as `u64`, then per row the viewer's `u32` and the
///    number as `u16`; then the count of issued numbers as `u64`, then per issued
///    number the viewer's `u32`, the target's `u32` and the number as `u16`.
/// 5. The tempo byte (`0` quiet, `1` alert, `2` engaged).
/// 6. `now_ms`, `i64`.
/// 7. The due-event count, `u64`.
/// 8. **The in-flight shot count** (16b cut W.1), `u64`.
/// 9. The ledger's event sequence, `u64`.
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
#[must_use]
pub fn state_hash(arena: &Arena) -> u64 {
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
                match unit.limits.no_turn_toward {
                    None => 0,
                    Some(crate::arena::state::Side::Left) => 1,
                    Some(crate::arena::state::Side::Right) => 2,
                },
                u8::from(unit.limits.motion_disabled),
                u8::from(unit.limits.wreck),
            ],
        );
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
    }
    // 16b cut N.1: the controller's own machines, in unit-id order.
    for (unit, row) in arena.reflex.rows() {
        hash = fnv1a64_update(hash, &unit.0.to_le_bytes());
        hash = fnv1a64_update(hash, &[row.state.code()]);
        let (gx, gy) = row.goal.unwrap_or((u16::MAX, u16::MAX));
        hash = fnv1a64_update(hash, &gx.to_le_bytes());
        hash = fnv1a64_update(hash, &gy.to_le_bytes());
        hash = fnv1a64_update(hash, &row.waypoint.unwrap_or(u16::MAX).to_le_bytes());
        hash = fnv1a64_update(hash, &(row.route.len() as u64).to_le_bytes());
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
            }
            None => hash = fnv1a64_update(hash, &[0_u8]),
        }
    }
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
    }
    // **16b cut Q.1**: the provisional numbers, in the book's own map order.
    hash = fnv1a64_update(
        hash,
        &(arena.designators.counters_len() as u64).to_le_bytes(),
    );
    for (viewer, next) in arena.designators.counters() {
        hash = fnv1a64_update(hash, &viewer.0.to_le_bytes());
        hash = fnv1a64_update(hash, &next.to_le_bytes());
    }
    hash = fnv1a64_update(hash, &(arena.designators.issued_len() as u64).to_le_bytes());
    for (viewer, target, n) in arena.designators.issued() {
        hash = fnv1a64_update(hash, &viewer.0.to_le_bytes());
        hash = fnv1a64_update(hash, &target.0.to_le_bytes());
        hash = fnv1a64_update(hash, &n.to_le_bytes());
    }
    hash = fnv1a64_update(hash, &[arena.clock.tempo().code()]);
    hash = fnv1a64_update(hash, &arena.now_ms.to_le_bytes());
    hash = fnv1a64_update(hash, &(arena.due.len() as u64).to_le_bytes());
    hash = fnv1a64_update(hash, &(arena.shots.len() as u64).to_le_bytes());
    hash = fnv1a64_update(hash, &arena.ledger.seq().to_le_bytes());
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
    use crate::arena::state::{DueKind, Power, Side, Team};
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
            |unit| unit.limits.no_turn_toward = Some(Side::Left),
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
            None,
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
}
