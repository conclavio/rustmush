//! Real owner-read preimages for movement's separate Sim slots.
//!
//! The existing movement capture decides admission. This read retains exactly
//! its owner predicates through final publication, without copying routes,
//! replaying a service or treating an observation as a lifecycle capability.

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{
    ActionError, ActionErrorCode, ActionTxn, DefinitionDescriptor, DefinitionDigest, DefinitionId,
    DefinitionKind, DefinitionRegistryBuilder, ModError, ObjId, OwnerId, PrepareCtx, Registrar,
};
use serde::Serialize;
use serde_json::json;

use crate::{FocusState, JourneyBook, NS};

const MAX_ROOTS: usize = crate::personnel_capture::MAX_DEPENDENCIES;
const MAX_SNAPSHOT_BYTES: usize = 64 * 1024;
const MAX_PROJECTION_MARKS: usize = 4096;
const MAX_PROJECTION_BYTES: usize = 1024 * 1024;
const MAX_PROJECTION_TEXT: usize = 64 * 1024;
const MAX_PROJECTION_FIELD: usize = 1024;
pub(super) struct JourneyReader;
struct FocusReader;
struct IcExclusionReader;
struct DeploymentProjectionReader;
struct ReadHandles {
    journeys: OwnerReadHandle<JourneyReader>,
    focus: OwnerReadHandle<FocusReader>,
    ic_exclusion: OwnerReadHandle<IcExclusionReader>,
    deployment: OwnerReadHandle<DeploymentProjectionReader>,
    tactics: Option<Arc<dyn crate::movement_custody::CustodyRead>>,
}
#[derive(Clone)]
pub(super) struct MovementReads(Arc<ReadHandles>);

#[derive(Clone, Serialize)]
pub(super) struct MovementReadRequest {
    roots: BTreeSet<ObjId>,
}

fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}
pub(super) fn register(
    reg: &mut Registrar<'_>,
    tactics: Option<Arc<dyn crate::movement_custody::CustodyRead>>,
) -> Result<MovementReads, ModError> {
    let descriptor = DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(bad)?,
        DefinitionId::new("movement_journey_preimage").map_err(bad)?,
        1,
        json!({"state":"JourneyBook","purpose":"movement_preimage_only",
            "fields":["held_empty","root","record_present","under_way"],
            "max_roots":MAX_ROOTS,"max_snapshot_bytes":MAX_SNAPSHOT_BYTES}),
    )
    .map_err(bad)?;
    let focus_descriptor = DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(bad)?,
        DefinitionId::new("movement_focus_preimage").map_err(bad)?,
        1,
        json!({"state":"FocusState","purpose":"movement_preimage_only",
            "fields":["object","has_focus_on","focus_of_actor_present"],
            "max_roots":MAX_ROOTS,"max_snapshot_bytes":MAX_SNAPSHOT_BYTES}),
    )
    .map_err(bad)?;
    let owner = OwnerId::new(NS).map_err(bad)?;
    let ic_descriptor = DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(bad)?,
        DefinitionId::new("movement_ic_exclusion_preimage").map_err(bad)?,
        1,
        json!({"state":"IcExclusions","purpose":"movement_preimage_only",
            "fields":["loaded","generation","unit","identity_present"],
            "max_roots":MAX_ROOTS,"max_snapshot_bytes":MAX_SNAPSHOT_BYTES}),
    )
    .map_err(bad)?;
    let expected = DefinitionRegistryBuilder::new()
        .register(owner.clone(), descriptor.clone())
        .map_err(bad)?;
    let deployment_descriptor = DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(bad)?,
        DefinitionId::new("movement_deployment_projection").map_err(bad)?,
        1,
        json!({"state":"DeployedUnits","purpose":"projection_preimage_only",
            "fields":["unit","match_id","arena"],"scope":"complete_bounded_index",
            "max_marks":MAX_PROJECTION_MARKS,"max_snapshot_bytes":MAX_PROJECTION_BYTES,
            "max_field_text_bytes":MAX_PROJECTION_FIELD,"max_text_bytes":MAX_PROJECTION_TEXT}),
    )
    .map_err(bad)?;
    let expected_deployment = DefinitionRegistryBuilder::new()
        .register(owner.clone(), deployment_descriptor.clone())
        .map_err(bad)?;
    let expected_focus = DefinitionRegistryBuilder::new()
        .register(owner.clone(), focus_descriptor.clone())
        .map_err(bad)?;
    let expected_ic = DefinitionRegistryBuilder::new()
        .register(owner.clone(), ic_descriptor.clone())
        .map_err(bad)?;
    reg.definition_validator(
        descriptor.kind().clone(),
        move |ctx: &mush_core::DefinitionValidationContext<'_>| {
            if (ctx.definition().reference() == &expected
                || ctx.definition().reference() == &expected_focus
                || ctx.definition().reference() == &expected_ic
                || ctx.definition().reference() == &expected_deployment)
                && !ctx.definition().is_current()
            {
                Ok(())
            } else {
                Err("unknown Sim movement read definition".into())
            }
        },
    )?;
    let definition = reg.definition(descriptor)?;
    let focus_definition = reg.definition(focus_descriptor)?;
    let ic_definition = reg.definition(ic_descriptor)?;
    let deployment_definition = reg.definition(deployment_descriptor)?;
    reg.owner_read_service(
        definition.clone(),
        BTreeSet::from([owner.clone()]),
        BTreeSet::new(),
        JourneyReader,
    )?;
    reg.owner_read_service(
        focus_definition.clone(),
        BTreeSet::from([owner.clone()]),
        BTreeSet::new(),
        FocusReader,
    )?;
    reg.owner_read_service(
        ic_definition.clone(),
        BTreeSet::from([owner.clone()]),
        BTreeSet::new(),
        IcExclusionReader,
    )?;
    reg.owner_read_service(
        deployment_definition.clone(),
        BTreeSet::from([owner]),
        BTreeSet::new(),
        DeploymentProjectionReader,
    )?;
    let journeys = reg.request_owner_read::<JourneyReader>(definition)?;
    let focus = reg.request_owner_read::<FocusReader>(focus_definition)?;
    let ic_exclusion = reg.request_owner_read::<IcExclusionReader>(ic_definition)?;
    let deployment = reg.request_owner_read::<DeploymentProjectionReader>(deployment_definition)?;
    Ok(MovementReads(Arc::new(ReadHandles {
        journeys,
        focus,
        ic_exclusion,
        deployment,
        tactics,
    })))
}

fn fingerprint(
    book: &JourneyBook,
    roots: &BTreeSet<ObjId>,
) -> Result<DefinitionDigest, OwnerReadError> {
    if roots.is_empty() || roots.len() > MAX_ROOTS {
        return Err(OwnerReadError::Limit);
    }
    if !book.held.is_empty() {
        return Err(OwnerReadError::Refused);
    }
    let rows: Vec<_> = roots
        .iter()
        .map(|root| (*root, book.get(*root).is_some(), book.under_way(*root)))
        .collect();
    DefinitionDigest::for_canonical_value_with_limit(
        "sim.movement.journey_preimage.v1",
        &json!(rows),
        MAX_SNAPSHOT_BYTES,
    )
    .map_err(|_| OwnerReadError::Limit)
}
impl OwnerReadService for JourneyReader {
    type State = JourneyBook;
    type Request = MovementReadRequest;
    type Output = DefinitionDigest;

    fn read(
        &self,
        _: &JourneyBook,
        ctx: &OwnerReadContext<'_>,
        request: MovementReadRequest,
    ) -> Result<Self::Output, OwnerReadError> {
        if ctx.requester().as_str() != NS {
            return Err(OwnerReadError::Authority);
        }
        ctx.guard_state::<JourneyBook, _>(move |book| fingerprint(book, &request.roots))
    }
}

fn focus_fingerprint(
    focus: &FocusState,
    roots: &BTreeSet<ObjId>,
) -> Result<DefinitionDigest, OwnerReadError> {
    if roots.is_empty() || roots.len() > MAX_ROOTS {
        return Err(OwnerReadError::Limit);
    }
    let rows: Vec<_> = roots
        .iter()
        .map(|root| {
            (
                *root,
                focus.has_focus_on(*root),
                focus.focus_of_actor(*root).is_some(),
            )
        })
        .collect();
    DefinitionDigest::for_canonical_value_with_limit(
        "sim.movement.focus_preimage.v1",
        &json!(rows),
        MAX_SNAPSHOT_BYTES,
    )
    .map_err(|_| OwnerReadError::Limit)
}
impl OwnerReadService for FocusReader {
    type State = FocusState;
    type Request = MovementReadRequest;
    type Output = DefinitionDigest;

    fn read(
        &self,
        _: &FocusState,
        ctx: &OwnerReadContext<'_>,
        request: MovementReadRequest,
    ) -> Result<Self::Output, OwnerReadError> {
        if ctx.requester().as_str() != NS {
            return Err(OwnerReadError::Authority);
        }
        ctx.guard_state::<FocusState, _>(move |focus| focus_fingerprint(focus, &request.roots))
    }
}

fn ic_fingerprint(
    state: &crate::ic_exclusion::IcExclusions,
    roots: &BTreeSet<ObjId>,
) -> Result<DefinitionDigest, OwnerReadError> {
    if roots.is_empty() || roots.len() > MAX_ROOTS {
        return Err(OwnerReadError::Limit);
    }
    let image = state
        .movement_preimage(roots)
        .map_err(|_| OwnerReadError::Integrity)?;
    DefinitionDigest::for_canonical_value_with_limit(
        "sim.movement.ic_exclusion_preimage.v1",
        &json!(image),
        MAX_SNAPSHOT_BYTES,
    )
    .map_err(|_| OwnerReadError::Limit)
}
impl OwnerReadService for IcExclusionReader {
    type State = crate::ic_exclusion::IcExclusions;
    type Request = MovementReadRequest;
    type Output = DefinitionDigest;

    fn read(
        &self,
        _: &Self::State,
        ctx: &OwnerReadContext<'_>,
        request: MovementReadRequest,
    ) -> Result<Self::Output, OwnerReadError> {
        if ctx.requester().as_str() != NS {
            return Err(OwnerReadError::Authority);
        }
        ctx.guard_state::<Self::State, _>(move |state| ic_fingerprint(state, &request.roots))
    }
}

fn deployment_fingerprint(
    index: &crate::deployed::DeployedUnits,
) -> Result<DefinitionDigest, OwnerReadError> {
    if index.len() > MAX_PROJECTION_MARKS {
        return Err(OwnerReadError::Limit);
    }
    let mut text = MAX_PROJECTION_TEXT;
    let mut rows = Vec::with_capacity(index.len());
    for (unit, mark) in index.iter() {
        if mark.arena.len() > MAX_PROJECTION_FIELD {
            return Err(OwnerReadError::Limit);
        }
        text = text
            .checked_sub(mark.arena.len())
            .ok_or(OwnerReadError::Limit)?;
        rows.push((unit, mark.match_id, mark.arena.as_str()));
    }
    DefinitionDigest::for_canonical_value_with_limit(
        "sim.movement.deployment_projection.v1",
        &json!(rows),
        MAX_PROJECTION_BYTES,
    )
    .map_err(|_| OwnerReadError::Limit)
}
impl OwnerReadService for DeploymentProjectionReader {
    type State = crate::deployed::DeployedUnits;
    type Request = ();
    type Output = DefinitionDigest;
    fn read(
        &self,
        _: &Self::State,
        ctx: &OwnerReadContext<'_>,
        _: (),
    ) -> Result<Self::Output, OwnerReadError> {
        if ctx.requester().as_str() != NS {
            return Err(OwnerReadError::Authority);
        }
        ctx.guard_state::<Self::State, _>(deployment_fingerprint)
    }
}

pub(super) fn retain(
    ctx: &mut PrepareCtx<'_>,
    txn: &mut ActionTxn<'_>,
    reads: &MovementReads,
    nodes: &BTreeSet<ObjId>,
) -> Result<(), ActionError> {
    let unavailable = || {
        crate::tick::err(
            ActionErrorCode::Unavailable,
            "movement owner preimage unavailable",
        )
    };
    let roots = {
        let world = ctx.world().map_err(|_| unavailable())?;
        let state = crate::state(world).ok_or_else(unavailable)?;
        let mut roots = nodes.clone();
        if roots.is_empty() || roots.len() > MAX_ROOTS {
            return Err(unavailable());
        }
        for node in nodes {
            roots.extend(crate::graph::ancestors(state, *node).map_err(|_| unavailable())?);
            if roots.len() > MAX_ROOTS {
                return Err(unavailable());
            }
        }
        roots
    };
    let request = MovementReadRequest { roots };
    ctx.read_owner_preimage(txn, &reads.0.journeys, request.clone())
        .map_err(|_| unavailable())?;
    ctx.read_owner_preimage(txn, &reads.0.focus, request.clone())
        .map_err(|_| unavailable())?;
    ctx.read_owner_preimage(txn, &reads.0.ic_exclusion, request.clone())
        .map_err(|_| unavailable())?;
    ctx.read_owner_preimage(txn, &reads.0.deployment, ())
        .map_err(|_| unavailable())?;
    if let Some(tactics) = &reads.0.tactics {
        tactics.retain(ctx, txn, &request.roots)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "movement_reads_tests.rs"]
mod tests;
