//! **Whose side a hull flies for in the Isidis operation** (doc 50 cut OP.6; Q-50-26, Q-50-61).
//!
//! Only while the operation is inside its window (`crate::isidis_op::status`, phase `Running`)
//! and its legal-target list names carriers does a hull fly for a side. Then a hull flies for a
//! side when:
//! - **its flag is that side's polity** (`crate::concession::hull_flag`, Q-50-61): the polity of
//!   the organization holding its title; a side's polity is its commanding organization's; or
//! - **it carries a live job of that side's arm**: a haul of the Freeport arm's taken or aboard,
//!   or an escort of one taken (`crate::stockpile::arm`). The Republic has no arm posting jobs.
//!
//! A hull that flies for two sides, or none, flies for none. Two hulls flying for different sides
//! are **operation foes**: in a bubble they are hostile to each other (`super::tick`), and either
//! may break off (`super::tick::BREAK_OFF_M`). Outside the window every carrier is friendly to
//! every other, as before.

use mush_core::{ObjId, World};

use crate::operations::{OperationPhase, OperationStatus};

/// The legal-target class that makes carriers foes (OP.5's `isidis_op::LEGAL_TARGETS`).
pub const CARRIER_TARGETS: &str = "carrier";

/// **The operation, when its window makes carriers foes**: running, with carriers among its legal
/// targets.
#[must_use]
pub fn carriers_contested(w: &World) -> Option<OperationStatus> {
    crate::isidis_op::status(w).filter(|status| {
        status.phase == OperationPhase::Running
            && status.legal_targets.iter().any(|t| t == CARRIER_TARGETS)
    })
}

/// **The side `hull` flies for** in `status`, by its index in `status.sides`; `None` for none (or
/// for two).
#[must_use]
pub fn side_of(w: &World, status: &OperationStatus, hull: ObjId) -> Option<usize> {
    let flag = crate::concession::hull_flag(w, hull);
    let by_flag = status.sides.iter().position(|(_, principal)| {
        matches!(principal, mush_org::PrincipalId::Organization(org)
            if flag.is_some() && crate::polity::polity_of_org(w, *org) == flag)
    });
    let by_job = arm_job_side(w, status, hull);
    match (by_flag, by_job) {
        (Some(a), Some(b)) if a != b => None,
        (Some(side), _) | (None, Some(side)) => Some(side),
        (None, None) => None,
    }
}

/// The side whose arm's live job `hull` carries: a Freeport arm haul taken or aboard, or an
/// escort of one taken.
fn arm_job_side(w: &World, status: &OperationStatus, hull: ObjId) -> Option<usize> {
    use crate::services::escort::EscortStage;
    use crate::services::haul::HaulStage;
    let arm = crate::stockpile::arm(w)?;
    let carries = arm.posts.iter().any(|post| {
        let haul = crate::services::haul::haul_of(w, post.haul).is_some_and(|job| {
            job.hull == Some(hull) && matches!(job.stage, HaulStage::Taken | HaulStage::Aboard)
        });
        let escort = post
            .escort
            .and_then(|e| crate::services::escort::escort_of(w, e))
            .is_some_and(|job| {
                job.with == Some(hull)
                    && matches!(job.stage, EscortStage::Taken | EscortStage::Watching)
            });
        haul || escort
    });
    if !carries {
        return None;
    }
    let (org, _) = crate::polity::lane_org_account(
        w,
        &mush_core::CanonicalId::new(crate::stockpile::ARM_ORG).ok()?,
    )?;
    status
        .sides
        .iter()
        .position(|(_, principal)| *principal == mush_org::PrincipalId::Organization(org))
}

/// **Whether `a` and `b` are operation foes**: the window is open and they fly for different
/// sides.
#[must_use]
pub fn foes(w: &World, a: ObjId, b: ObjId) -> bool {
    let Some(status) = carriers_contested(w) else {
        return false;
    };
    match (side_of(w, &status, a), side_of(w, &status, b)) {
        (Some(x), Some(y)) => x != y,
        _ => false,
    }
}
