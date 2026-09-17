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
///       `2` right); `motion_disabled` (`0`/`1`); `wreck` (`0`/`1`).
/// 2. The tempo byte (`0` quiet, `1` alert, `2` engaged).
/// 3. `now_ms`, `i64`.
/// 4. The due-event count, `u64`.
/// 5. The ledger's event sequence, `u64`.
///
/// **What is deliberately absent.** The *ordered* desired heading and speed are
/// not folded in: they are an input, and a pulse at one tempo may have applied a
/// helm order that a pulse at another has not yet — the replay compares what the
/// world *became*, not what it was told. The opaque `TacticalUnit` is not folded
/// in either; its truth is M32's and its every change is journalled there, so
/// hashing it would make this function a second, weaker copy of that journal.
/// Cut W adds the cells it needs as an explicit, documented extension.
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
    }
    hash = fnv1a64_update(hash, &[arena.clock.tempo().code()]);
    hash = fnv1a64_update(hash, &arena.now_ms.to_le_bytes());
    hash = fnv1a64_update(hash, &(arena.due.len() as u64).to_le_bytes());
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
