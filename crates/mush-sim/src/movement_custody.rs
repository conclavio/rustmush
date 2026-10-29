//! Data contract for reading the actual Tactics movement custody owner.
//! These values are observations, never placement, custody or title capabilities.

use std::collections::BTreeSet;

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
    pub facts: Vec<MovementCustodyFact>,
    pub digest: DefinitionDigest,
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
