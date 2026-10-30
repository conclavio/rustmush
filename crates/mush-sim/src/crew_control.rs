//! Read the actual K6 holder for a bounded field roster, without granting control.
use mush_core::read_service::{OwnerReadContext, OwnerReadError, OwnerReadService};
use mush_core::{
    ControlLeaseKey, ControlLeaseState, DefinitionDescriptor, DefinitionId, DefinitionKind,
    ModError, ObjId, OwnerId, PrincipalId, Registrar, StateFamilyHandle,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrewControlQuery {
    pub unit: ObjId,
    pub operator: ObjId,
}

pub struct CrewControlReader {
    leases: StateFamilyHandle<ControlLeaseKey, ControlLeaseState>,
}
pub fn definition_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(bad)?,
        DefinitionId::new("field_crew_control").map_err(bad)?,
        1,
        json!({"purpose":"field_roster_actual_helm","max_units":6}),
    )
    .map_err(bad)
}
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    leases: StateFamilyHandle<ControlLeaseKey, ControlLeaseState>,
) -> Result<(), ModError> {
    let definition = reg.definition(definition_descriptor()?)?;
    reg.owner_read_service(
        definition,
        BTreeSet::from([OwnerId::new("tacsim").map_err(bad)?]),
        BTreeSet::new(),
        CrewControlReader { leases },
    )
}
impl OwnerReadService for CrewControlReader {
    type State = crate::SpatialState;
    type Request = Vec<CrewControlQuery>;
    type Output = Vec<bool>;
    fn read(
        &self,
        _state: &Self::State,
        ctx: &OwnerReadContext<'_>,
        query: Self::Request,
    ) -> Result<Self::Output, OwnerReadError> {
        if ctx.requester().as_str() != "tacsim"
            || query.len() > 6
            || query.iter().any(|q| q.unit.0 == 0 || q.operator.0 == 0)
        {
            return Err(OwnerReadError::Refused);
        }
        let channel = mush_core::ChannelId::new("helm").ok_or(OwnerReadError::Integrity)?;
        query
            .into_iter()
            .map(|q| {
                let lease =
                    ctx.action_state(&self.leases, &ControlLeaseKey::new(q.unit, channel.clone()))?;
                Ok(lease
                    .is_some_and(|l| l.holder() == Some(&PrincipalId::Object { id: q.operator })))
            })
            .collect()
    }
}
