//! Real owner-read preimages for movement's separate journey state slot.
//!
//! The existing movement capture decides admission. This read retains exactly
//! its journey predicates through final publication, without copying routes,
//! replaying a service or treating an observation as a lifecycle capability.

use std::collections::BTreeSet;

use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{
    ActionError, ActionErrorCode, ActionTxn, DefinitionDescriptor, DefinitionDigest, DefinitionId,
    DefinitionKind, DefinitionRegistryBuilder, ModError, ObjId, OwnerId, PrepareCtx, Registrar,
};
use serde::Serialize;
use serde_json::json;

use crate::{JourneyBook, NS};

const MAX_ROOTS: usize = crate::personnel_capture::MAX_DEPENDENCIES;
const MAX_SNAPSHOT_BYTES: usize = 64 * 1024;
pub(super) struct JourneyReader;
pub(super) type JourneyRead = OwnerReadHandle<JourneyReader>;

#[derive(Serialize)]
pub(super) struct JourneyRequest {
    roots: BTreeSet<ObjId>,
}

fn bad(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}
pub(super) fn register(reg: &mut Registrar<'_>) -> Result<JourneyRead, ModError> {
    let descriptor = DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(bad)?,
        DefinitionId::new("movement_journey_preimage").map_err(bad)?,
        1,
        json!({"state":"JourneyBook","purpose":"movement_preimage_only",
            "fields":["held_empty","root","record_present","under_way"],
            "max_roots":MAX_ROOTS,"max_snapshot_bytes":MAX_SNAPSHOT_BYTES}),
    )
    .map_err(bad)?;
    let owner = OwnerId::new(NS).map_err(bad)?;
    let expected = DefinitionRegistryBuilder::new()
        .register(owner.clone(), descriptor.clone())
        .map_err(bad)?;
    reg.definition_validator(
        descriptor.kind().clone(),
        move |ctx: &mush_core::DefinitionValidationContext<'_>| {
            if ctx.definition().reference() == &expected && !ctx.definition().is_current() {
                Ok(())
            } else {
                Err("unknown Sim movement read definition".into())
            }
        },
    )?;
    let definition = reg.definition(descriptor)?;
    reg.owner_read_service(
        definition.clone(),
        BTreeSet::from([owner]),
        BTreeSet::new(),
        JourneyReader,
    )?;
    reg.request_owner_read::<JourneyReader>(definition)
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
    type Request = JourneyRequest;
    type Output = DefinitionDigest;

    fn read(
        &self,
        _: &JourneyBook,
        ctx: &OwnerReadContext<'_>,
        request: JourneyRequest,
    ) -> Result<Self::Output, OwnerReadError> {
        if ctx.requester().as_str() != NS {
            return Err(OwnerReadError::Authority);
        }
        ctx.guard_state::<JourneyBook, _>(move |book| fingerprint(book, &request.roots))
    }
}

pub(super) fn retain_journeys(
    ctx: &mut PrepareCtx<'_>,
    txn: &mut ActionTxn<'_>,
    reader: &JourneyRead,
    nodes: &BTreeSet<ObjId>,
) -> Result<(), ActionError> {
    let unavailable = || {
        crate::tick::err(
            ActionErrorCode::Unavailable,
            "movement journey preimage unavailable",
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
    ctx.read_owner_preimage(txn, reader, JourneyRequest { roots })
        .map(|_| ())
        .map_err(|_| unavailable())
}

#[cfg(test)]
#[path = "movement_reads_tests.rs"]
mod tests;
