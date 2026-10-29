//! Data contract for reading the actual Tactics movement custody owner.
//! These values are observations, never placement, custody or title capabilities.

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::read_service::{OwnerReadHandle, OwnerReadService};
use mush_core::{
    ActionError, ActionErrorCode, ActionTxn, DefinitionRegistryBuilder, OwnerId, PrepareCtx,
    Registrar,
};
use mush_core::{
    DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKind, ModError, ObjId,
};
use serde::Serialize;
use serde_json::json;

pub const MAX_MOVEMENT_CUSTODY_OBJECTS: usize = 512;
pub const MAX_MOVEMENT_CUSTODY_RECORDS: usize = 4096;
pub const MAX_MOVEMENT_CUSTODY_BYTES: usize = 128 * 1024;

#[derive(Clone, Serialize)]
pub struct MovementCustodyRequest {
    pub objects: BTreeSet<ObjId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MovementCustodyFact {
    pub object: ObjId,
    pub active_unit: bool,
    pub held_record: bool,
    pub ooc_deployed: bool,
    pub ic_custodied: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MovementCustodyFacts {
    /// One fact for every requested object, in strictly increasing id order.
    pub facts: Vec<MovementCustodyFact>,
    pub digest: DefinitionDigest,
}

pub(crate) trait CustodyRead: Send + Sync {
    fn retain(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        objects: &BTreeSet<ObjId>,
    ) -> Result<(), ActionError>;
}
pub(crate) type CustodyReadFactory =
    Arc<dyn Fn(&mut Registrar<'_>) -> Result<Arc<dyn CustodyRead>, ModError> + Send + Sync>;
struct RegisteredRead<S: OwnerReadService> {
    handle: OwnerReadHandle<S>,
}

/// Only this typed adapter may be configured by the Sim builder. A root cannot
/// substitute a callback claiming that it checked the foreign owner: Core must
/// resolve the actual registered service, state slot, grant and readiness.
pub(crate) fn factory<S>() -> CustodyReadFactory
where
    S: OwnerReadService<Request = MovementCustodyRequest, Output = MovementCustodyFacts>,
{
    Arc::new(|reg| {
        let definition = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("tactics").map_err(|e| ModError::msg(e.to_string()))?,
                descriptor()?,
            )
            .map_err(|e| ModError::msg(e.to_string()))?;
        let handle = reg.request_owner_read::<S>(definition)?;
        Ok(Arc::new(RegisteredRead { handle }))
    })
}
impl<S> CustodyRead for RegisteredRead<S>
where
    S: OwnerReadService<Request = MovementCustodyRequest, Output = MovementCustodyFacts>,
{
    fn retain(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        objects: &BTreeSet<ObjId>,
    ) -> Result<(), ActionError> {
        let refused = || {
            crate::tick::err(
                ActionErrorCode::Unavailable,
                "movement custody owner unavailable, held or changed",
            )
        };
        if objects.is_empty() || objects.len() > MAX_MOVEMENT_CUSTODY_OBJECTS {
            return Err(refused());
        }
        let result = ctx
            .read_owner_preimage(
                txn,
                &self.handle,
                MovementCustodyRequest {
                    objects: objects.clone(),
                },
            )
            .map_err(|_| refused())?;
        if result.digest.as_bytes() == &[0; 32] || result.facts.len() != objects.len() {
            return Err(refused());
        }
        for (fact, expected) in result.facts.iter().zip(objects) {
            if fact.object != *expected
                || fact.held_record
                || fact.ooc_deployed
                || fact.ic_custodied
            {
                return Err(refused());
            }
        }
        Ok(())
    }
}

/// Fixed Tactics-owned service contract. Registration and a genuine Core read
/// handle are still required; knowing this descriptor grants no read or write.
pub fn descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(|e| ModError::msg(e.to_string()))?,
        DefinitionId::new("movement_custody_facts").map_err(|e| ModError::msg(e.to_string()))?,
        1,
        json!({"state":"UnitStore","purpose":"movement_custody_preimage_only",
            "fields":["active_unit","held_record","ooc_deployed","ic_custodied"],
            "max_objects":MAX_MOVEMENT_CUSTODY_OBJECTS,"max_records":MAX_MOVEMENT_CUSTODY_RECORDS,
            "max_snapshot_bytes":MAX_MOVEMENT_CUSTODY_BYTES}),
    )
    .map_err(|e| ModError::msg(e.to_string()))
}
