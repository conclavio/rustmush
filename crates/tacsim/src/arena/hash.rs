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
///    11. **the controller byte** (16b cut N.1): `0` pilot, `1` reflex.
/// 2. **The reflex machines** (16b cut N.1), in unit-id order — the map's own, so
///    there is no iteration order to diverge on: per machine the unit's `u32`, the
///    state byte, the goal cell's two `u16` (`u16::MAX` twice for none), the
///    waypoint index as `u16` (`u16::MAX` for none), the route's length as `u64`
///    and the target unit's `u32` (`0` for none).
/// 3. **The shadow contact store** (16b cut S.1), in its own iteration order —
///    viewer object number ascending, then contact id ascending, which is what
///    the two `BTreeMap`s give: per contact the viewer's `u32`, the contact id's
///    `u64`, the target's `u32`, the class byte (`0` unknown, `1` classified red,
///    `2` classified blue), `first_seen_ms` and `last_seen_ms` as `i64`.
/// 4. The tempo byte (`0` quiet, `1` alert, `2` engaged).
/// 5. `now_ms`, `i64`.
/// 6. The due-event count, `u64`.
/// 7. **The in-flight shot count** (16b cut W.1), `u64`.
/// 8. The ledger's event sequence, `u64`.
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
    }
    hash = fnv1a64_update(hash, &[arena.clock.tempo().code()]);
    hash = fnv1a64_update(hash, &arena.now_ms.to_le_bytes());
    hash = fnv1a64_update(hash, &(arena.due.len() as u64).to_le_bytes());
    hash = fnv1a64_update(hash, &(arena.shots.len() as u64).to_le_bytes());
    hash = fnv1a64_update(hash, &arena.ledger.seq().to_le_bytes());
    hash
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
