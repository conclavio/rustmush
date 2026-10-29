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
        for r in policy_references(policy)? {
            definitions.get_exact(&r).map_err(bad)?;
        }
        Ok(())
    }
}
/// **Every definition a refit policy names** (doc 17h cut M8b): the chartered recipe, the
/// ungoverning recipe when composed, and every root — the line family, the further families,
/// the twins and the coreless bases — each with a non-zero digest; sorted, deduplicated. A
/// line-only policy names what it always did.
pub(super) fn policy_references(policy: &Value) -> Result<Vec<DefinitionRef>, ActionStateError> {
    let recipe_of = |value: &Value, id: &str| -> Result<DefinitionRef, ActionStateError> {
        let recipe: DefinitionRef = parsed(value)?;
        if recipe.key().kind.as_str() != "recipe"
            || recipe.key().id.as_str() != id
            || recipe.version() != 1
        {
            return Err(refused("core refit recipe identity"));
        }
        Ok(recipe)
    };
    let mut refs = vec![recipe_of(field(policy, "recipe")?, "core_refit")?];
    let roots: [DefinitionRef; 3] = parsed(field(policy, "fitted_units")?)?;
    refs.extend(roots);
    if let Some(families) = policy.get("families") {
        let families: Vec<[DefinitionRef; 3]> = parsed(families)?;
        refs.extend(families.into_iter().flatten());
    }
    if let Some(ungovern) = policy.get("ungovern") {
        refs.push(recipe_of(field(ungovern, "recipe")?, "core_ungovern")?);
        let twins: Vec<[DefinitionRef; 3]> = parsed(field(ungovern, "twins")?)?;
        refs.extend(twins.into_iter().flatten());
    }
    if let Some(coreless) = policy.get("coreless") {
        let bases: Vec<Value> = parsed(coreless)?;
        for base in bases {
            refs.push(parsed(field(&base, "base")?)?);
        }
    }
    if refs.iter().any(|r| r.digest().as_bytes() == &[0; 32]) {
        return Err(refused("core refit zero digest"));
    }
    refs.sort();
    refs.dedup();
    Ok(refs)
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
            let refs = policy_references(policy)?;
            // `fee_minor` is `core_refit`'s fee; the ungoverning recipe's is the policy's
            // own `ungovern.fee_minor` (doc 17h cut M8b G3), inside `policy`.
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

#[cfg(test)]
mod m8b_tests {
    //! Doc 17h cut M8b (G11, G12): the policy's references and the slot's facility.
    use super::*;
    use crate::{CommitToken, mechanics::DefinitionDigest};

    fn reference(kind: &str, id: &str) -> DefinitionRef {
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("tacsim").expect("an owner"),
                DefinitionDescriptor::new(
                    DefinitionKind::new(kind).expect("a kind"),
                    DefinitionId::new(id).expect("an id"),
                    1,
                    json!({"fixture": id}),
                )
                .expect("a descriptor"),
            )
            .expect("registers")
    }
    fn policy(ungovern_recipe: &str) -> Value {
        let units = |ids: [&str; 3]| ids.map(|id| reference("unit", id));
        let fitted = units(["a", "b", "c"]);
        let rig = units(["d", "e", "f"]);
        let line_twins = units(["g", "h", "i"]);
        let rig_twins = units(["j", "k", "l"]);
        let recipe = reference("recipe", "core_refit");
        let ungovern = reference("recipe", ungovern_recipe);
        let base = reference("unit", "m");
        json!({
            "recipe": recipe,
            "fitted_units": fitted,
            "families": [rig],
            "ungovern": {
                "recipe": ungovern,
                "factory": 20,
                "twins": [line_twins, rig_twins],
            },
            "coreless": [{"base": base, "family": 1}],
            "factory": 10,
        })
    }

    #[test]
    fn m8b_every_root_and_both_recipes_are_named_and_a_wrong_recipe_is_refused() {
        let refs = policy_references(&policy("core_ungovern")).expect("names them");
        assert_eq!(refs.len(), 2 + 13, "two recipes, twelve roots and the base");
        assert!(policy_references(&policy("core_refit")).is_err());
    }

    #[test]
    fn m8b_the_slot_is_the_recipes_own_facility() {
        let row = |offer: Value| super::super::wire::Row {
            raw: Value::Null,
            order: 1,
            accepted: CommitToken::from_bytes([1; 16]),
            action: reference("action", "accept_core_refit"),
            intent: Value::Null,
            offer,
            policy: policy("core_ungovern"),
            fingerprint: DefinitionDigest::from_bytes([2; 32]),
        };
        let at = |offer: Value| {
            row(offer)
                .resource()
                .map(|r| format!("{r:?}"))
                .map_err(|e| format!("{e:?}"))
        };
        let chartered = at(json!({})).expect("the chartered slot");
        let ungoverned = at(json!({"kind": "core_ungovern"})).expect("the shop's slot");
        assert!(chartered.contains("f10") && ungoverned.contains("f20"));
        assert!(at(json!({"kind": "core_refit"})).is_err(), "one encoding");
        assert!(row(json!({})).displaced().expect("reads").is_none());
    }
}
