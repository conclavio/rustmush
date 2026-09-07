//! Shared fixtures for the in-crate unit tests. Compiled only under `cfg(test)`.

use mush_core::{ClockRef, ClockStamp, ObjId};
use mush_economy::{AccountId, Amount, EconomicVisibility};
use mush_org::PrincipalId;
use smol_str::SmolStr;

use crate::ids::OperationId;
use crate::record::{
    AudienceSpec, Bond, Consequence, ObjectiveKind, ObjectiveSpec, OperationTerms, SideSpec,
};

/// A `(notice, (open, close))` triple on one world-IC generation.
#[must_use]
pub(crate) fn window_at(notice: i64, lead: i64, generation: u64) -> (ClockStamp, ClockStamp) {
    let clock = ClockRef::WorldIc { generation };
    (
        ClockStamp {
            clock: clock.clone(),
            at: notice.saturating_add(lead),
        },
        ClockStamp {
            clock,
            at: notice.saturating_add(lead).saturating_add(3_600),
        },
    )
}

/// A complete, valid set of declared terms — bonded, with one front stake and one
/// contract stake, so every consumer arm is reachable from it.
#[must_use]
pub(crate) fn terms_fixture(operation: OperationId) -> OperationTerms {
    let notice = ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: 0,
    };
    OperationTerms {
        operation,
        attacker: PrincipalId::Object(ObjId(100)),
        scope: None,
        objective: ObjectiveSpec {
            kind: ObjectiveKind::WindowElapsed,
            summary: SmolStr::new_static("hold the refinery through the window"),
        },
        legal_targets: vec![SmolStr::new_static("structure")],
        stakes: vec![
            Consequence::Front {
                region: 4,
                control: SmolStr::new_static("de_facto"),
            },
            Consequence::Outcome {
                contract: 9,
                row: SmolStr::new_static("success"),
            },
        ],
        bond: Some(Bond {
            account: AccountId(1),
            amount: Amount::from_minor(500),
            forfeit_to: None,
            hold: None,
        }),
        sides: vec![SideSpec {
            name: SmolStr::new_static("attack"),
            principal: PrincipalId::Object(ObjId(100)),
        }],
        eligible_defenders: AudienceSpec {
            members: vec![PrincipalId::Object(ObjId(200))],
        },
        notice: notice.clone(),
        window: window_at(notice.at, crate::record::OPERATION_NOTICE_MIN, 1),
        visibility: EconomicVisibility::Audience {
            members: vec![PrincipalId::Object(ObjId(200))],
            announce_issuer: false,
        },
        consent_hash: [0; 32],
    }
}
