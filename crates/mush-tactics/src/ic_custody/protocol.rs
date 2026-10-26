//! Fixed participant identity for the four-owner IC custody composition.
//! Definition data is not a grant, host event, or permission to publish.
use mush_core::{DefinitionDescriptor, DefinitionId, DefinitionKind, ModError};
pub const MAX_OPEN: usize = 1024;
pub const MAX_HISTORY: usize = 8192;
pub const MAX_UNIT_BYTES: usize = 1048576;
pub const MAX_HISTORY_BYTES: usize = 8192;
pub fn participant_descriptor() -> Result<DefinitionDescriptor, ModError> {
    let bad = |e: &dyn std::fmt::Display| ModError::msg(e.to_string());
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_participant").map_err(|e| bad(&e))?,
        DefinitionId::new("ic_custody").map_err(|e| bad(&e))?,
        1,
        serde_json::json!({"format": 1, "protocol": "ic_custody_v1",
            "max_open": MAX_OPEN,
            "max_history": MAX_HISTORY,
            "max_unit_bytes": MAX_UNIT_BYTES,
            "max_history_bytes": MAX_HISTORY_BYTES
        }),
    )
    .map_err(|e| bad(&e))
}
