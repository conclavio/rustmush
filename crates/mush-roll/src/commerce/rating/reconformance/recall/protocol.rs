use super::*;
use crate::commerce::FiledRevisionId;
use mush_core::action::{ActionInvocation, PerceivedRef, PrincipalId};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
};
use mush_core::{DefinitionRef, GOD};
use serde::{Deserialize, Serialize};
use serde_json::json;
pub(crate) fn definitions(manifest: &DefinitionRef) -> Result<[DefinitionDescriptor; 2], ModError> {
    super::super::super::registration::ReconformanceDefinitions::new(manifest.clone())?;
    let root = crate::commerce::authored_retirement::retention_reference()?;
    let mut refs = vec![manifest.clone(), root];
    refs.sort();
    let make = |kind: &str, id: &str| {
        DefinitionDescriptor::new(
            DefinitionKind::new(kind).map_err(error)?,
            DefinitionId::new(id).map_err(error)?,
            1,
            json!({"format":1,"protocol":"registry_measured_recall_v1","service_actor":GOD}),
        )
        .map(|d| d.with_references(refs.clone()))
        .map_err(error)
    };
    Ok([
        make("action", "recall_nonconforming_mind")?,
        make("outcome", "measured_recall_committed")?,
    ])
}
pub(crate) fn action(manifest: &DefinitionRef) -> Result<DefinitionRef, ModError> {
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new(crate::NS).map_err(error)?,
            definitions(manifest)?
                .into_iter()
                .next()
                .ok_or_else(|| error("recall action"))?,
        )
        .map_err(error)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) filed: FiledRevisionId,
}
impl Request {
    pub(super) fn parse(inv: &ActionInvocation) -> Result<Self, PublicationError> {
        if inv.actor() != GOD
            || inv.principal() != &(PrincipalId::Object { id: GOD })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner.as_str() != crate::NS
        {
            return Err(PublicationError::Invalid);
        }
        serde_json::from_value(inv.parameters().clone()).map_err(bad)
    }
}
