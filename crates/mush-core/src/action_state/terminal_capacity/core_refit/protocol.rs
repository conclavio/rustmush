//! Closed registration for paid Install/Replace. No caller-supplied targets.
use super::*;
use crate::action::ActionProviderRegistry;
use crate::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry,
    DefinitionRegistryBuilder,
};
const ACTIONS: [&str; 4] = [
    "accept_core_refit",
    "advance_core_refit",
    "install_core_component",
    "cancel_core_refit",
];
#[derive(Clone, Debug, Serialize)]
pub(in crate::action_state) struct Protocol {
    participants: [DefinitionRef; 3],
    actions: [DefinitionRef; 4],
}
impl Protocol {
    pub(in crate::action_state) fn new(
        owner: &OwnerId,
        participants: [DefinitionRef; 3],
        actions: [DefinitionRef; 4],
    ) -> Result<Self, ActionStateError> {
        if owner.as_str() != "industry" {
            return Err(refused("core refit declaring owner"));
        }
        for (r, (o, id, version)) in participants.iter().zip([
            ("industry", "core_production", 3),
            ("economy", "initial_filing", 1),
            ("tactics", "component_replacement", 1),
        ]) {
            if r.key().owner.as_str() != o
                || r.key().kind.as_str() != "owner_participant"
                || r.key().id.as_str() != id
                || r.version() != version
            {
                return Err(refused("core refit exact participant"));
            }
        }
        for (a, id) in actions.iter().zip(ACTIONS) {
            if a.key().owner != *owner
                || a.key().kind.as_str() != "action"
                || a.key().id.as_str() != id
                || a.version() != 1
            {
                return Err(refused("core refit action order"));
            }
        }
        if actions[2] != action_reference(&Value::Null, &Value::Null, 2)?
            || actions[3] != action_reference(&Value::Null, &Value::Null, 3)?
        {
            return Err(refused("core refit closed install/cancel"));
        }
        Ok(Self {
            participants,
            actions,
        })
    }
    pub(in crate::action_state::terminal_capacity) fn admission_action(&self) -> &DefinitionRef {
        &self.actions[0]
    }
    pub(in crate::action_state::terminal_capacity) fn industry_participant(
        &self,
    ) -> &DefinitionRef {
        &self.participants[0]
    }
    pub(super) fn terminal_participants(&self) -> [DefinitionRef; 2] {
        [self.participants[1].clone(), self.participants[2].clone()]
    }
    pub(in crate::action_state::terminal_capacity) fn recognizes_historical(
        &self,
        effect: &DefinitionRef,
    ) -> bool {
        effect.key() == self.actions[0].key() && effect.version() == 1
    }
    pub(super) fn action_index(
        &self,
        identity: &RegisteredTxnIdentity,
    ) -> Result<usize, ActionStateError> {
        self.actions
            .iter()
            .position(|a| a == &identity.action)
            .ok_or_else(|| refused("core refit current action"))
    }
    pub(in crate::action_state) fn validate_families(
        &self,
        registry: &StateFamilyRegistry,
    ) -> Result<(), ActionStateError> {
        if registry
            .core_integration
            .as_ref()
            .is_none_or(|p| p.industry_participant() != self.industry_participant())
        {
            return Err(refused("core refit exact manufacture owner prerequisite"));
        }
        for (name, _) in FAMILIES {
            let actual = registry
                .entries
                .get(&(owner_id()?, StateFamilyId::new(name).map_err(bad)?))
                .ok_or_else(|| refused("core refit missing family"))?;
            if actual.descriptor().canonical_value() != descriptor(name)?.canonical_value() {
                return Err(refused("core refit family descriptor"));
            }
        }
        Ok(())
    }
    pub(in crate::action_state) fn validate_composition(
        &self,
        definitions: &DefinitionRegistry,
        providers: &ActionProviderRegistry,
        participants: &crate::participant::ParticipantRegistry,
    ) -> Result<(), ActionStateError> {
        let admission = definitions.get_exact(&self.actions[0]).map_err(bad)?;
        let policy = field(admission.payload(), "policy")?;
        let advance = definitions.get_exact(&self.actions[1]).map_err(bad)?;
        let component = field(advance.payload(), "component")?;
        for (i, action) in self.actions.iter().enumerate() {
            if action != &action_reference(policy, component, i)? {
                return Err(refused("core refit full action recipe"));
            }
            definitions.get_exact(action).map_err(bad)?;
            let provider = providers.get_exact(action).map_err(bad)?;
            if provider.owner().as_str() != "industry" {
                return Err(refused("core refit provider owner"));
            }
            let expected: &[usize] = match i {
                0 | 3 => &[0, 1],
                1 => &[0],
                2 => &[1, 2],
                _ => return Err(refused("core refit action index")),
            };
            for &index in expected {
                if !participants.grants_exact(&self.participants[index], provider.owner(), action) {
                    return Err(refused("core refit exact owner grant"));
                }
            }
        }
        for p in &self.participants {
            definitions.get_exact(p).map_err(bad)?;
        }
        let recipe: DefinitionRef = parsed(field(policy, "recipe")?)?;
        definitions.get_exact(&recipe).map_err(bad)?;
        let roots: [DefinitionRef; 3] = parsed(field(policy, "fitted_units")?)?;
        for r in roots {
            definitions.get_exact(&r).map_err(bad)?;
        }
        Ok(())
    }
}
pub(super) fn action_reference(
    policy: &Value,
    component: &Value,
    index: usize,
) -> Result<DefinitionRef, ActionStateError> {
    let id = *ACTIONS
        .get(index)
        .ok_or_else(|| refused("core refit action index"))?;
    let d = if index == 2 {
        crate::core_exchange::action_descriptor().map_err(bad)?
    } else {
        let (payload, refs) = if index == 3 {
            (
                json!({"format":1,"protocol":"certified_core_refit_cancel_v1","authority":"actual_acknowledged_requester","effect":"release_exact_fee_and_slot","current_recipe_required":false}),
                vec![],
            )
        } else {
            let recipe: DefinitionRef = parsed(field(policy, "recipe")?)?;
            if recipe.key().kind.as_str() != "recipe"
                || recipe.key().id.as_str() != "core_refit"
                || recipe.version() != 1
            {
                return Err(refused("core refit recipe identity"));
            }
            let roots: [DefinitionRef; 3] = parsed(field(policy, "fitted_units")?)?;
            let mut refs = vec![recipe];
            refs.extend(roots);
            refs.sort();
            refs.dedup();
            let payload = if index == 0 {
                json!({"format":1,"protocol":"certified_core_refit_v1","policy":policy,"fee_minor":10000,"reservation_ic_seconds":604800,"incoming":"delivered_certified_core","supplier_consent":"acknowledged_exact_offer_v1"})
            } else {
                json!({"format":1,"protocol":"certified_core_refit_v1","policy":policy,"component":component,"kind":id,"cadence":3600,"maximum_work_units":167})
            };
            (payload, refs)
        };
        DefinitionDescriptor::new(
            DefinitionKind::new("action").map_err(bad)?,
            DefinitionId::new(id).map_err(bad)?,
            1,
            payload,
        )
        .map_err(bad)?
        .with_references(refs)
    };
    DefinitionRegistryBuilder::new()
        .register(owner_id()?, d)
        .map_err(bad)
}
pub(super) fn descriptor(name: &str) -> Result<StateFamilyDescriptor, ActionStateError> {
    match name {
        BOOK => {
            crate::reservation_book_state_family_descriptor(StateFamilyId::new(name).map_err(bad)?)
                .map_err(bad)
        }
        SLOTS => StateFamilyDescriptor::new(
            StateFamilyId::new(name).map_err(bad)?,
            CanonicalId::new("reservation_book_id.v1").map_err(bad)?,
            CanonicalId::new("industry.facility_slots.v1").map_err(bad)?,
            1,
            CanonicalId::new("industry.facility_slots.decode_json.v1").map_err(bad)?,
            CanonicalId::new("industry.facility_slots.bounds.v1").map_err(bad)?,
            CanonicalId::new("industry.facility_slots.json.v1").map_err(bad)?,
        )
        .map_err(bad),
        SCHEDULE => {
            crate::durable_schedule_state_family_descriptor(StateFamilyId::new(name).map_err(bad)?)
                .map_err(bad)
        }
        ORDER | ASSET | HULL | EXCHANGE | JOB => {
            let c = |suffix: &str| {
                CanonicalId::new(format!("industry.{name}.{suffix}.v1")).map_err(bad)
            };
            let ceiling = FAMILIES
                .iter()
                .find(|(f, _)| *f == name)
                .ok_or_else(|| refused("core refit target"))?
                .1;
            StateFamilyDescriptor::new(
                StateFamilyId::new(name).map_err(bad)?,
                c("key")?,
                c("value")?,
                1,
                c("decode")?,
                c("validate")?,
                c("canonical")?,
            )
            .and_then(|d| d.with_limits(if name == EXCHANGE { 16 } else { 64 }, ceiling))
            .map_err(bad)
        }
        _ => Err(refused("core refit family")),
    }
}
