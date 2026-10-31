//! Focused calls to the actual private status producer; no new public helper.
#![cfg(test)]
use super::*;
use crate::deliberation::LanceLeader;

#[test]
fn r14_leaders_row_covers_unbooked_and_booked_empty_records() {
    let (tw, _, _) = dropped_world(crate::TacsimConfig::default());
    let absent = MatchId(u64::MAX);
    assert!(
        tw.world()
            .module_state::<crate::match_::MatchBook>()
            .unwrap()
            .get(absent)
            .is_none()
    );
    assert_eq!(
        super::super::leaders_row(tw.world(), absent),
        "  leaders: none"
    );
    assert!(
        tw.world()
            .module_state::<crate::match_::MatchBook>()
            .unwrap()
            .get(DROP_MATCH)
            .unwrap()
            .leaders
            .is_empty()
    );
    assert_eq!(
        super::super::leaders_row(tw.world(), DROP_MATCH),
        "  leaders: none"
    );
}

#[test]
fn r14_leaders_row_names_live_character_and_marks_actual_swept_character() {
    let (mut tw, ada, bo) = dropped_world(crate::TacsimConfig::default());
    let red = SquadId {
        team: PartyId::RED,
        index: 1,
    };
    let blue = SquadId {
        team: PartyId::BLUE,
        index: 1,
    };
    tw.world_mut()
        .module_state_mut::<crate::match_::MatchBook>()
        .unwrap()
        .get_mut(DROP_MATCH)
        .unwrap()
        .leaders = vec![
        LanceLeader {
            squad: red,
            by: CharacterRef::Character { id: ada },
        },
        LanceLeader {
            squad: blue,
            by: CharacterRef::Character { id: bo },
        },
    ];
    assert_eq!(
        super::super::leaders_row(tw.world(), DROP_MATCH),
        "  leaders: red-1 Ada, blue-1 Bo"
    );
    assert!(tw.world_mut().destroy(bo).unwrap().contains(&bo));
    assert!(tw.world_mut().sweep().contains(&bo));
    assert!(tw.world().get(bo).is_none());
    assert_eq!(
        super::super::leaders_row(tw.world(), DROP_MATCH),
        "  leaders: red-1 Ada, blue-1 ?"
    );
    // Read-only presentation retains the absent character's exact attribution.
    assert_eq!(
        tw.world()
            .module_state::<crate::match_::MatchBook>()
            .unwrap()
            .get(DROP_MATCH)
            .unwrap()
            .leaders
            .get(1)
            .expect("second leader")
            .by,
        CharacterRef::Character { id: bo }
    );
}
