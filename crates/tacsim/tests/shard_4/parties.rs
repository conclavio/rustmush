//! **Parties in the kernel** (doc 32 cut CF.0; Q-CF-46, the engine gate run on
//! the OOC range before any IC host code exists).
//!
//! The OOC range is a field whose host happens to name two parties red and
//! blue. These tests field a third — slots 3 and 4 of blue's pods admitted as
//! party `G` — on the shipped proving ground with the production fill's
//! machines, and pin what CF.0 has to make true of the kernel:
//!
//! * **a three-party practice runs and resolves**: every party marked hostile
//!   to every other by the host, the field ends by the host's elimination with
//!   at most one party standing, and every callsign is a party letter and a
//!   drawn unit letter;
//! * **relations are beliefs held per party** (Q-CF-5 to Q-CF-7): a leader's
//!   Hostile mark is private to its own book, fire makes the victim's party
//!   hold the shooter hostile, a third party nobody marked is fired on by
//!   nobody, and a Friendly mark cancels the hostility fire produced;
//! * **the end is the host's** (Q-CF-3): the same field under
//!   [`EndCondition::Host`] runs on past the moment elimination ended it.

use crate::support::{Muster, booted_world, install_sealed, run_until, sealed_sources};
use mush_testkit::TestWorld;
use tacsim::arena::state::{Arena, Classification, PartyId};
use tacsim::arena::{EndCondition, EndReason, FieldHost, Mark, Relation};
use tacsim::config::{ArenaConfig, MatchConfig, SensorsConfig};

/// The match seed every run here opens on.
const SEED: u64 = 0x00c0_ffee;

/// The third party: the seventh letter of the alphabet, `G`.
fn green() -> PartyId {
    PartyId::nth(6).expect("the seventh party letter")
}

/// The proving ground with two production-fill machines a party: red's slots
/// 1 and 2, blue's 1 and 2, and blue's 3 and 4 admitted as [`green`], under
/// `host`.
fn three_parties(world: &TestWorld, host: FieldHost) -> Arena {
    let mut custody = crate::support::shipped_custody_seeded(
        world,
        &[
            &[Muster::fill(PartyId::RED, 1), Muster::fill(PartyId::RED, 2)],
            &[
                Muster::fill(PartyId::BLUE, 1),
                Muster::fill(PartyId::BLUE, 2),
            ],
            &[
                Muster::fill(PartyId::BLUE, 3),
                Muster::fill(PartyId::BLUE, 4),
            ],
        ],
        SEED,
    );
    for unit in custody.units.iter_mut().skip(4) {
        unit.team = green();
        unit.reseal(&tacsim::match_::SealTerms::production_fixture())
            .expect("the production fill's terms seal");
    }
    custody.host = host;
    let sources = sealed_sources(&custody.units);
    let mut arena = Arena::new(
        custody,
        &ArenaConfig::default(),
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("the three-party field checks out");
    install_sealed(&mut arena, &sources);
    arena
}

/// The parties with a unit still in action.
fn standing(arena: &Arena) -> Vec<PartyId> {
    let mut parties: Vec<PartyId> = arena
        .units
        .iter()
        .filter(|unit| !arena.out_of_action(unit.id))
        .map(|unit| unit.team)
        .collect();
    parties.dedup();
    parties.sort_unstable();
    parties.dedup();
    parties
}

#[test]
fn a_three_party_practice_runs_and_resolves() {
    let world = booted_world();
    let mut arena = three_parties(&world, FieldHost::all_hostile());
    let parties: Vec<PartyId> = arena.relations.parties().collect();
    assert_eq!(parties, vec![PartyId::BLUE, green(), PartyId::RED]);
    for viewer in &parties {
        for other in &parties {
            let expected = if viewer == other {
                Relation::Own
            } else {
                Relation::Hostile
            };
            assert_eq!(
                arena.relation(*viewer, *other),
                expected,
                "{viewer} of {other}"
            );
        }
    }
    // Q-CF-11: the party letter, then a drawn unit letter, distinct within the
    // party; the third party is lettered `G`.
    let mut names: Vec<String> = arena
        .units
        .iter()
        .map(|unit| unit.callsign.to_string())
        .collect();
    for (unit, name) in arena.units.iter().zip(&names) {
        assert_eq!(name.len(), 2, "{name}");
        assert!(
            name.starts_with(unit.team.letter()),
            "{name} on {}",
            unit.team
        );
    }
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 6, "six distinct callsigns");

    let _ = run_until(&mut arena, 900_000, 1_000, |_, _| {});
    let end = arena
        .ended
        .expect("the practice resolves inside its duration");
    let left = standing(&arena);
    match end.reason {
        EndReason::Elimination {
            winner: Some(winner),
        } => {
            assert!(left.contains(&winner), "the winner stands: {left:?}");
        }
        EndReason::Elimination { winner: None } | EndReason::Spent => {}
        other => panic!("the practice ended as {other:?} at {} ms", end.t_ms),
    }
    assert!(end.t_ms < 900_000, "resolved before the duration bound");
}

#[test]
fn a_mark_is_private_fire_makes_a_belief_and_a_friendly_mark_cancels_it() {
    let world = booted_world();
    // No opening marks and a host end: three neutral parties.
    let host = FieldHost {
        end: EndCondition::Host,
        ..FieldHost::default()
    };
    let mut arena = three_parties(&world, host);
    assert_eq!(arena.relation(PartyId::RED, green()), Relation::Neutral);
    // Red's leader marks green hostile: red's book only.
    arena.mark_party(PartyId::RED, green(), Some(Mark::Hostile));
    assert_eq!(arena.relation(PartyId::RED, green()), Relation::Hostile);
    assert_eq!(
        arena.relation(green(), PartyId::RED),
        Relation::Neutral,
        "private"
    );
    assert_eq!(arena.relation(PartyId::BLUE, green()), Relation::Neutral);

    // Run until green has been fired on by red and so holds red hostile.
    let mut seen_hostile_row = false;
    let mut seen_neutral_row = false;
    let _ = run_until(&mut arena, 600_000, 1_000, |arena, _| {
        for unit in arena.units.iter().filter(|unit| unit.team == PartyId::RED) {
            for row in arena.frame_contacts(unit.id) {
                match row.class {
                    Classification::Classified { party } if party == green() => {
                        assert_eq!(row.relation, Relation::Hostile);
                        seen_hostile_row = true;
                    }
                    Classification::Classified { party } if party == PartyId::BLUE => {
                        assert_eq!(row.relation, Relation::Neutral);
                        seen_neutral_row = true;
                    }
                    Classification::Unknown => assert_eq!(row.relation, Relation::Unknown),
                    Classification::Classified { .. } => {}
                }
            }
        }
    });
    assert!(seen_hostile_row, "a red frame classified a green hull");
    assert!(seen_neutral_row, "a red frame classified a blue hull");
    assert!(
        arena.ended.is_none()
            || arena
                .ended
                .is_some_and(|end| end.reason == EndReason::Timeout),
        "the host's end: no elimination ({:?})",
        arena.ended
    );
    assert_eq!(
        arena.relation(green(), PartyId::RED),
        Relation::Hostile,
        "green saw red's fire"
    );
    assert_eq!(
        arena.relation(PartyId::BLUE, PartyId::RED),
        Relation::Neutral,
        "blue was never fired on"
    );
    let blue_book = arena.relations.book(PartyId::BLUE).expect("blue's book");
    assert!(
        blue_book.fired.is_empty(),
        "nobody fired on a party nobody marked"
    );

    // Green's leader cancels the fire-derived hostility.
    arena.mark_party(green(), PartyId::RED, Some(Mark::Friendly));
    assert_eq!(arena.relation(green(), PartyId::RED), Relation::Friend);
    assert_eq!(
        arena.relation(PartyId::RED, green()),
        Relation::Hostile,
        "red's mark stands"
    );
    // And clearing the mark leaves the fire's belief underneath it.
    arena.mark_party(green(), PartyId::RED, None);
    assert_eq!(arena.relation(green(), PartyId::RED), Relation::Hostile);
}

#[test]
fn the_end_is_the_hosts() {
    let world = booted_world();
    let mut eliminated = three_parties(&world, FieldHost::all_hostile());
    let _ = run_until(&mut eliminated, 900_000, 1_000, |_, _| {});
    let end = eliminated.ended.expect("the elimination host ends it");
    assert!(
        matches!(end.reason, EndReason::Elimination { .. } | EndReason::Spent),
        "{end:?}"
    );
    // The same field, the same seed, under a host that ends it itself: the
    // kernel runs on past the moment elimination ended the first.
    let host = FieldHost {
        end: EndCondition::Host,
        ..FieldHost::all_hostile()
    };
    let mut hosted = three_parties(&world, host);
    let past = end.t_ms.saturating_add(10_000);
    let _ = run_until(&mut hosted, past, 1_000, |_, _| {});
    assert!(hosted.ended.is_none(), "{:?}", hosted.ended);
    assert!(hosted.now_ms >= past);
}

/// **The window side is the region's** (doc 32 Q-CF-51): nothing in the kernel
/// names 256. The same production field checks out and fights on a 512-a-side
/// window — doc 30 Q-30-6's biggest authored map — and the definition's own
/// width is what it reads.
#[test]
fn the_kernel_runs_on_a_window_of_the_regions_own_side() {
    let world = booted_world();
    let mut custody = crate::support::shipped_custody_seeded(
        &world,
        &[
            &[Muster::fill(PartyId::RED, 1)],
            &[Muster::fill(PartyId::BLUE, 1)],
        ],
        SEED,
    );
    let side = tacsim::definitions::arena::MAX_ARENA_SIDE;
    assert_eq!(side, 512);
    assert_ne!(side, tacsim::definitions::arena::DEFAULT_WINDOW_SIDE);
    let ground = tacsim::arena::state::TerrainCell {
        class: mush_atlas::TerrainClass::Ground,
        elev_cm: 0,
        move_cost: 100,
    };
    custody.definition.terrain = tacsim::arena::state::ArenaTerrain::uniform(
        side,
        side,
        custody.definition.terrain.cell_cm(),
        ground,
    )
    .expect("a 512 window");
    let sources = sealed_sources(&custody.units);
    let mut arena = Arena::new(
        custody,
        &ArenaConfig::default(),
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("a 512-a-side field checks out");
    install_sealed(&mut arena, &sources);
    assert_eq!(arena.definition.terrain.width(), side);
    let _ = run_until(&mut arena, 30_000, 1_000, |_, _| {});
    assert_eq!(arena.now_ms, 30_000);
}
