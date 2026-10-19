//! Closed declaration; no caller-selected families, keys, ceilings or verifier.
use super::*;
use crate::action::ActionProviderRegistry;
use crate::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry,
    DefinitionRegistryBuilder,
};

pub(super) const ACTIONS: [&str; 5] = [
    "accept_core_integration",
    "reserve_core_output",
    "advance_core_integration",
    "complete_core_integration",
    "cancel_core_integration",
];
#[derive(Clone, Debug, Serialize)]
pub(in crate::action_state) struct Protocol {
    pub(super) participants: [DefinitionRef; 2],
    pub(super) actions: [DefinitionRef; 5],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) delivery: Option<DefinitionRef>,
}
impl Protocol {
    pub(in crate::action_state) fn new(
        owner: &OwnerId,
        participants: [DefinitionRef; 2],
        actions: [DefinitionRef; 5],
    ) -> Result<Self, ActionStateError> {
        if owner.as_str() != "industry" {
            return Err(refused("core integration declaration owner"));
        }
        for (r, (owner, id, version)) in participants.iter().zip([
            ("industry", "core_production", 2),
            ("economy", "initial_filing", 1),
        ]) {
            if r.key().owner.as_str() != owner
                || r.key().kind.as_str() != "owner_participant"
                || r.key().id.as_str() != id
                || (r.version() != version && !(owner == "industry" && r.version() == 3))
            {
                return Err(refused("core integration participant identity"));
            }
        }
        for (a, id) in actions.iter().zip(ACTIONS) {
            if a.key().owner != *owner
                || a.key().kind.as_str() != "action"
                || a.key().id.as_str() != id
                || a.version() != 1
            {
                return Err(refused("core integration action identity/order"));
            }
        }
        if actions[4] != action_reference(&Value::Null, 4)? {
            return Err(refused("core integration stable cancellation identity"));
        }
        Ok(Self {
            participants,
            actions,
            delivery: None,
        })
    }
    pub(in crate::action_state) fn with_delivery(
        owner: &OwnerId,
        participants: [DefinitionRef; 2],
        actions: [DefinitionRef; 6],
    ) -> Result<Self, ActionStateError> {
        let [accept, reserve, advance, complete, cancel, delivery] = actions;
        let mut protocol = Self::new(
            owner,
            participants,
            [accept, reserve, advance, complete, cancel],
        )?;
        if delivery != super::delivery::action_reference()? {
            return Err(refused("core delivery exact action"));
        }
        protocol.delivery = Some(delivery);
        Ok(protocol)
    }
    pub(in crate::action_state::terminal_capacity) fn industry_participant(
        &self,
    ) -> &DefinitionRef {
        &self.participants[0]
    }
    pub(in crate::action_state::terminal_capacity) fn admission_action(&self) -> &DefinitionRef {
        &self.actions[0]
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
            .chain(self.delivery.iter())
            .position(|a| a == &identity.action)
            .ok_or_else(|| refused("core integration actual action differs"))
    }
    pub(in crate::action_state) fn validate_families(
        &self,
        registry: &StateFamilyRegistry,
    ) -> Result<(), ActionStateError> {
        // Version 3 is accepted only with this exact additional closed refit
        // declaration, never as a generic successor participant version.
        if self.participants[0].version() == 3
            && registry
                .core_refit
                .as_ref()
                .is_none_or(|p| p.industry_participant() != self.industry_participant())
        {
            return Err(refused(
                "core integration v3 requires the exact refit owner",
            ));
        }
        for (owner, name, _) in FAMILIES
            .into_iter()
            .chain(self.delivery.iter().flat_map(|_| super::delivery::FAMILIES))
        {
            let expected = descriptor(name)?;
            let owner = owner_id(owner)?;
            let id = StateFamilyId::new(name).map_err(bad)?;
            let actual = registry
                .entries
                .get(&(owner.clone(), id.clone()))
                .ok_or_else(|| refused("core integration family absent"))?;
            if actual.descriptor().canonical_value() != expected.canonical_value() {
                return Err(refused("core integration exact family descriptor"));
            }
            if owner.as_str() == "inventory"
                && !registry.delegates_family(&owner, &id, &owner_id("industry")?)
            {
                return Err(refused("core integration Inventory delegation absent"));
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
        if super::delivery::promised(policy)? != self.delivery.is_some() {
            return Err(refused("core delivery declaration/promise differs"));
        }
        for (index, action) in self.actions.iter().enumerate() {
            if *action != action_reference(policy, index)? {
                return Err(refused("core integration complete registered action"));
            }
            definitions.get_exact(action).map_err(bad)?;
            let provider = providers.get_exact(action).map_err(bad)?;
            if provider.owner().as_str() != "industry" {
                return Err(refused("core integration provider owner"));
            }
            // Reserve uses Core's closed World image. Other transitions prepare
            // the one actual Industry slot; only paid transitions use Economy.
            if index != 1
                && !participants.grants_exact(&self.participants[0], provider.owner(), action)
            {
                return Err(refused("core integration Industry grant absent"));
            }
            if matches!(index, 0 | 3 | 4)
                && !participants.grants_exact(&self.participants[1], provider.owner(), action)
            {
                return Err(refused("core integration Economy grant absent"));
            }
        }
        if let Some(action) = &self.delivery {
            if action != &super::delivery::action_reference()? {
                return Err(refused("core delivery action differs"));
            }
            definitions.get_exact(action).map_err(bad)?;
            let provider = providers.get_exact(action).map_err(bad)?;
            if provider.owner().as_str() != "industry"
                || !participants.grants_exact(&self.participants[0], provider.owner(), action)
            {
                return Err(refused("core delivery Industry grant absent"));
            }
        }
        for p in &self.participants {
            definitions.get_exact(p).map_err(bad)?;
        }
        let recipe: DefinitionRef = parsed(field(policy, "recipe")?)?;
        definitions.get_exact(&recipe).map_err(bad)?;
        Ok(())
    }
}
pub(super) fn action_descriptor(
    policy: &Value,
    index: usize,
) -> Result<DefinitionDescriptor, ActionStateError> {
    let id = *ACTIONS
        .get(index)
        .ok_or_else(|| refused("core integration action index"))?;
    let (payload, refs) = if index == 4 {
        (
            json!({"format":1,"protocol":"certified_core_integration_cancel_v1","authority":"actual_acknowledged_requester","effect":"release_exact_retained_input_slot_and_payment","current_recipe_required":false}),
            vec![],
        )
    } else {
        let recipe: DefinitionRef = parsed(field(policy, "recipe")?)?;
        if recipe.key().kind.as_str() != "recipe"
            || recipe.key().id.as_str() != "core_integrate"
            || recipe.version() != 1
        {
            return Err(refused("core integration recipe identity"));
        }
        (
            json!({"format":1,"protocol":if super::delivery::promised(policy)? { "certified_core_integration_delivery_v2" } else { "certified_core_integration_v1" },"policy":policy,"reservation_ic_seconds":604800,"supplier_consent":"acknowledged_exact_offer_v1","input":"certified_origin_qty1","title_recipient":"authenticated_personal_requester"}),
            vec![recipe],
        )
    };
    let d = DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(bad)?,
        DefinitionId::new(id).map_err(bad)?,
        1,
        payload,
    )
    .map_err(bad)?
    .with_references(refs);
    Ok(d)
}
pub(super) fn action_reference(
    policy: &Value,
    index: usize,
) -> Result<DefinitionRef, ActionStateError> {
    DefinitionRegistryBuilder::new()
        .register(owner_id("industry")?, action_descriptor(policy, index)?)
        .map_err(bad)
}
fn names(
    name: &str,
    prefix: &str,
    key: &str,
    value: &str,
    decode: &str,
    validate: &str,
    canonical: &str,
) -> Result<StateFamilyDescriptor, ActionStateError> {
    let cid = |part: &str| CanonicalId::new(format!("{prefix}{part}")).map_err(bad);
    StateFamilyDescriptor::new(
        StateFamilyId::new(name).map_err(bad)?,
        cid(key)?,
        cid(value)?,
        1,
        cid(decode)?,
        cid(validate)?,
        cid(canonical)?,
    )
    .map_err(bad)
}
pub(super) fn descriptor(name: &str) -> Result<StateFamilyDescriptor, ActionStateError> {
    match name {
        ORDER | JOB => {
            let c = |suffix: &str| {
                CanonicalId::new(format!("industry.{name}.{suffix}.v1")).map_err(bad)
            };
            StateFamilyDescriptor::new(
                StateFamilyId::new(name).map_err(bad)?,
                c("key")?,
                c("value")?,
                1,
                c("decode")?,
                c("validate")?,
                c("canonical")?,
            )
            .and_then(|d| d.with_limits(64, if name == ORDER { ORDER_CEILING } else { 2048 }))
            .map_err(bad)
        }
        OUTPUT | DEPLOYMENT => crate::core_output::descriptor(name).map_err(bad),
        INPUT => {
            let c = |suffix: &str| {
                CanonicalId::new(format!("inventory.{name}.{suffix}.v1")).map_err(bad)
            };
            StateFamilyDescriptor::new(
                StateFamilyId::new(name).map_err(bad)?,
                c("key")?,
                c("value")?,
                1,
                c("decode")?,
                c("validate")?,
                c("canonical")?,
            )
            .and_then(|d| d.with_limits(20, 16384))
            .map_err(bad)
        }
        LOT => names(
            name,
            "inventory.",
            "lot_id.v1",
            "lot.v1",
            "lot.decode_json.v1",
            "lot.invariants.v1",
            "lot.json.v1",
        ),
        CENSUS => names(
            name,
            "inventory.",
            "endpoint_id.v1",
            "lot_census.v1",
            "lot_census.decode_json.v1",
            "lot_census.bounds.v1",
            "lot_census.json.v1",
        ),
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
        BOOK => {
            crate::reservation_book_state_family_descriptor(StateFamilyId::new(name).map_err(bad)?)
                .map_err(bad)
        }
        SCHEDULE => {
            crate::durable_schedule_state_family_descriptor(StateFamilyId::new(name).map_err(bad)?)
                .map_err(bad)
        }
        super::delivery::CORE | super::delivery::INDUSTRY => super::delivery::descriptor(name),
        _ => Err(refused("core integration unknown family")),
    }
}
