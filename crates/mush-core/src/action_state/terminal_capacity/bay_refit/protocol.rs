//! Closed registration for the paid bay refit. No caller-supplied targets.
use super::*;
use crate::action::ActionProviderRegistry;
use crate::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry,
    DefinitionRegistryBuilder,
};
/// The four kernel-pinned actions, in order. Industry's Initialize and Offer
/// write no protected family and are not pinned.
pub const ACTIONS: [&str; 4] = [
    "accept_bay_refit",
    "advance_bay_refit",
    "install_bay_refit",
    "cancel_bay_refit",
];
/// The participants, in order: the Industry bay owner, Economy's paid
/// filing and the Tactics mount-matched refit (doc 31 UD.4b PR-A).
const PARTICIPANTS: [(&str, &str, u32); 3] = [
    ("industry", "bay_refit", 1),
    ("economy", "initial_filing", 1),
    ("tactics", "component_refit", 1),
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
            return Err(refused("bay refit declaring owner"));
        }
        for (r, (o, id, version)) in participants.iter().zip(PARTICIPANTS) {
            if r.key().owner.as_str() != o
                || r.key().kind.as_str() != "owner_participant"
                || r.key().id.as_str() != id
                || r.version() != version
            {
                return Err(refused("bay refit exact participant"));
            }
        }
        for (a, id) in actions.iter().zip(ACTIONS) {
            if a.key().owner != *owner
                || a.key().kind.as_str() != "action"
                || a.key().id.as_str() != id
                || a.version() != 1
            {
                return Err(refused("bay refit action order"));
            }
        }
        if actions[2] != action_reference(&Value::Null, 2)?
            || actions[3] != action_reference(&Value::Null, 3)?
        {
            return Err(refused("bay refit closed install/cancel"));
        }
        Ok(Self {
            participants,
            actions,
        })
    }
    pub(in crate::action_state::terminal_capacity) fn admission_action(&self) -> &DefinitionRef {
        &self.actions[0]
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
            .ok_or_else(|| refused("bay refit current action"))
    }
    pub(in crate::action_state) fn validate_families(
        &self,
        registry: &StateFamilyRegistry,
    ) -> Result<(), ActionStateError> {
        let industry = owner_id("industry")?;
        for (owner, name, _) in FAMILIES.into_iter().chain([
            ("industry", DEPLOYMENT, 0),
            ("inventory", PART_HEAD, 0),
            ("inventory", PART_CENSUS, 0),
        ]) {
            let o = owner_id(owner)?;
            let id = StateFamilyId::new(name).map_err(bad)?;
            let actual = registry
                .entries
                .get(&(o.clone(), id.clone()))
                .ok_or_else(|| refused("bay refit missing family"))?;
            if actual.descriptor().canonical_value() != descriptor(name)?.canonical_value() {
                return Err(refused("bay refit family descriptor"));
            }
            if owner == "inventory" && !registry.delegates_family(&o, &id, &industry) {
                return Err(refused("bay refit Inventory delegation absent"));
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
        for (i, action) in self.actions.iter().enumerate() {
            if action != &action_reference(policy, i)? {
                return Err(refused("bay refit full action recipe"));
            }
            definitions.get_exact(action).map_err(bad)?;
            let provider = providers.get_exact(action).map_err(bad)?;
            if provider.owner().as_str() != "industry" {
                return Err(refused("bay refit provider owner"));
            }
            // Accept and Cancel hold and release the fee and the slot;
            // Advance is Industry's alone; Install captures the fee and
            // refits the hull.
            let expected: &[usize] = match i {
                0 | 3 => &[0, 1],
                1 => &[0],
                2 => &[1, 2],
                _ => return Err(refused("bay refit action index")),
            };
            for &index in expected {
                if !participants.grants_exact(&self.participants[index], provider.owner(), action) {
                    return Err(refused("bay refit exact owner grant"));
                }
            }
        }
        for p in &self.participants {
            definitions.get_exact(p).map_err(bad)?;
        }
        definitions.get_exact(&recipe(policy)?).map_err(bad)?;
        Ok(())
    }
}
/// The bay policy's recipe: `bay_refit` version 1, with a non-zero digest.
pub(super) fn recipe(policy: &Value) -> Result<DefinitionRef, ActionStateError> {
    let recipe: DefinitionRef = parsed(field(policy, "recipe")?)?;
    if recipe.key().kind.as_str() != "recipe"
        || recipe.key().id.as_str() != "bay_refit"
        || recipe.version() != 1
        || recipe.digest().as_bytes() == &[0; 32]
    {
        return Err(refused("bay refit recipe identity"));
    }
    Ok(recipe)
}
/// The bays the policy admits: a non-empty, strictly ascending list of
/// facility objects.
pub(super) fn facilities(policy: &Value) -> Result<Vec<ObjId>, ActionStateError> {
    let all: Vec<ObjId> = parsed(field(policy, "facilities")?)?;
    if all.is_empty() || all.len() > 64 || all.windows(2).any(|w| w[0] >= w[1]) {
        return Err(refused("bay refit facilities"));
    }
    Ok(all)
}
/// The policy's exact closed shape.
pub(super) fn validate_policy(policy: &Value) -> Result<(), ActionStateError> {
    strict(policy, &["format", "recipe", "facilities"])?;
    if number(policy, "format")? != 1 {
        return Err(refused("bay refit policy format"));
    }
    recipe(policy)?;
    facilities(policy)?;
    Ok(())
}
pub fn action_descriptor(
    policy: &Value,
    index: usize,
) -> Result<DefinitionDescriptor, ActionStateError> {
    let id = *ACTIONS
        .get(index)
        .ok_or_else(|| refused("bay refit action index"))?;
    let (payload, refs) = match index {
        0 => {
            validate_policy(policy)?;
            (
                json!({"format":1,"protocol":"bay_refit_v1","policy":policy,"reservation_ic_seconds":RESERVATION_SECONDS,"part":"inventory_lot_qty1","supplier_consent":"acknowledged_exact_offer_v1"}),
                vec![recipe(policy)?],
            )
        }
        1 => {
            validate_policy(policy)?;
            (
                json!({"format":1,"protocol":"bay_refit_v1","policy":policy,"kind":id,"cadence":CADENCE,"maximum_work_units":MAX_WORK_UNITS}),
                vec![recipe(policy)?],
            )
        }
        2 => (
            json!({"format":1,"protocol":"bay_refit_install_v1","authority":"actual_acknowledged_requester","effect":"capture_fee_consume_part_refit_hull","current_recipe_required":false}),
            vec![],
        ),
        _ => (
            json!({"format":1,"protocol":"bay_refit_cancel_v1","authority":"actual_acknowledged_requester","effect":"release_exact_fee_part_and_slot","current_recipe_required":false}),
            vec![],
        ),
    };
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(bad)?,
        DefinitionId::new(id).map_err(bad)?,
        1,
        payload,
    )
    .map_err(bad)?
    .with_references(refs))
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
    key: &str,
    value: &str,
    decode: &str,
    validate: &str,
    canonical: &str,
) -> Result<StateFamilyDescriptor, ActionStateError> {
    let cid = |part: &str| CanonicalId::new(format!("inventory.{part}")).map_err(bad);
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
/// The exact descriptor of every family the protocol names. Industry's and
/// Inventory's registrations must equal these.
pub fn descriptor(name: &str) -> Result<StateFamilyDescriptor, ActionStateError> {
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
        LOT => names(
            name,
            "lot_id.v1",
            "lot.v1",
            "lot.decode_json.v1",
            "lot.invariants.v1",
            "lot.json.v1",
        ),
        CENSUS => names(
            name,
            "endpoint_id.v1",
            "lot_census.v1",
            "lot_census.decode_json.v1",
            "lot_census.bounds.v1",
            "lot_census.json.v1",
        ),
        ORDER | HULL | JOB | DEPLOYMENT | PART | PART_HEAD | PART_CENSUS => {
            let owner = family_owner(name);
            let c =
                |suffix: &str| CanonicalId::new(format!("{owner}.{name}.{suffix}.v1")).map_err(bad);
            let ceiling = match name {
                ORDER => ORDER_CEILING,
                HULL | PART | PART_HEAD | DEPLOYMENT => 4096,
                _ => 2048,
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
            .and_then(|d| d.with_limits(64, ceiling))
            .map_err(bad)
        }
        _ => Err(refused("bay refit family")),
    }
}
