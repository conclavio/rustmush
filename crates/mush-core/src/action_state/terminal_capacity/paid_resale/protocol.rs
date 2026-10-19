use super::*;
#[derive(Clone, Debug, Serialize)]
pub(in crate::action_state) struct Protocol {
    pub(super) participants: [DefinitionRef; 2],
    pub(super) actions: [DefinitionRef; 5],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) registry_revocation: Option<DefinitionRef>,
}
impl Protocol {
    pub(in crate::action_state) fn new(
        owner: &OwnerId,
        participants: [DefinitionRef; 2],
        actions: [DefinitionRef; 5],
        registry_revocation: Option<DefinitionRef>,
    ) -> Result<Self, ActionStateError> {
        if owner.as_str() != "market" {
            return Err(refused("resale declaration owner"));
        }
        for (r, (owner, id)) in participants
            .iter()
            .zip([("economy", "initial_filing"), ("roll", "commercial_roots")])
        {
            if r.key().owner.as_str() != owner
                || r.key().kind.as_str() != "owner_participant"
                || r.key().id.as_str() != id
                || r.version() != 1
            {
                return Err(refused("resale participant identity"));
            }
        }
        let retention = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("roll").map_err(|_| refused("resale owner"))?,
                DefinitionDescriptor::new(
                    DefinitionKind::new("owner_participant").map_err(|_| refused("resale kind"))?,
                    DefinitionId::new("commercial_roots").map_err(|_| refused("resale id"))?,
                    1,
                    json!({"format":1,"protocol":"commercial_retention_v1"}),
                )
                .map_err(|_| refused("resale descriptor"))?,
            )
            .map_err(|_| refused("resale retention reference"))?;
        if participants[1] != retention {
            return Err(refused("resale retention differs"));
        }
        for (index, action) in actions.iter().enumerate() {
            if *action != action_reference(&retention, index)? {
                return Err(refused("resale full action identity"));
            }
        }
        if registry_revocation.as_ref().is_some_and(|a| {
            a.key().owner.as_str() != "roll"
                || a.key().kind.as_str() != "action"
                || a.key().id.as_str() != "registry_term_revocation"
                || a.version() != 1
        }) {
            return Err(refused("resale Registry action identity"));
        }
        Ok(Self {
            participants,
            actions,
            registry_revocation,
        })
    }
    pub(in crate::action_state::terminal_capacity) fn admission_action(&self) -> &DefinitionRef {
        &self.actions[2]
    }
    pub(super) fn action_index(
        &self,
        identity: &RegisteredTxnIdentity,
    ) -> Result<usize, ActionStateError> {
        self.actions
            .iter()
            .chain(self.registry_revocation.iter())
            .position(|a| a == &identity.action)
            .ok_or_else(|| refused("resale actual action differs"))
    }
    pub(in crate::action_state) fn validate_families(
        &self,
        registry: &StateFamilyRegistry,
    ) -> Result<(), ActionStateError> {
        if registry.original_purchase.is_none()
            && registry.entitlement_purchase.is_none()
            && registry.house_purchase.is_none()
        {
            return Err(refused("resale requires actual immutable birth audit"));
        }
        for (owner, family, max) in FAMILIES.into_iter().chain(
            self.registry_revocation
                .iter()
                .flat_map(|_| [("roll", SANCTION, 32768), ("roll", STATUS, 4096)]),
        ) {
            let owner = OwnerId::new(owner).map_err(|_| refused("resale owner"))?;
            let expected = descriptor(&owner, family, max)?;
            let actual = registry
                .entries
                .get(&(owner.clone(), expected.id().clone()))
                .ok_or_else(|| refused("resale family unavailable"))?;
            if actual.descriptor().canonical_value() != expected.canonical_value() {
                return Err(refused("resale descriptor differs"));
            }
            if owner.as_str() == "roll"
                && family != SANCTION
                && family != STATUS
                && !registry.delegates_family(
                    &owner,
                    expected.id(),
                    &OwnerId::new("market").map_err(|_| refused("resale owner"))?,
                )
            {
                return Err(refused("resale Roll family delegation"));
            }
            if self.registry_revocation.is_some()
                && owner.as_str() == "market"
                && family != OFFER
                && !registry.delegates_family(
                    &owner,
                    expected.id(),
                    &OwnerId::new("roll").map_err(|_| refused("resale owner"))?,
                )
            {
                return Err(refused("resale compound Market family delegation"));
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
        for (i, action) in self
            .actions
            .iter()
            .chain(self.registry_revocation.iter())
            .enumerate()
        {
            let def = definitions
                .get_exact(action)
                .map_err(|_| refused("resale action unavailable"))?;
            let provider = providers
                .get_exact(action)
                .map_err(|_| refused("resale provider unavailable"))?;
            if provider.owner().as_str() != if i == 5 { "roll" } else { "market" } {
                return Err(refused("resale provider owner"));
            }
            if i < 5 {
                if def.payload() != &action_payload(i)?
                    || def.references() != [self.participants[1].clone()]
                {
                    return Err(refused("resale descriptor payload or refs"));
                }
            } else {
                if def.payload() != &revocation_payload()
                    || def.references().len() != 2
                    || !def.references().contains(&self.participants[1])
                {
                    return Err(refused("resale Registry descriptor"));
                }
                let authority = def
                    .references()
                    .iter()
                    .find(|r| r.key().kind.as_str() == "registry_authority")
                    .ok_or_else(|| refused("resale Registry authority"))?;
                if authority.key().owner.as_str() != "roll"
                    || authority.key().id.as_str() != "mind_revocation"
                    || authority.version() != 1
                {
                    return Err(refused("resale Registry authority identity"));
                }
                let declared = definitions
                    .get_exact(authority)
                    .map_err(|_| refused("resale Registry authority missing"))?;
                if *action != revocation_reference(declared.payload(), &self.participants[1])? {
                    return Err(refused("resale Registry exact reference"));
                }
            }
            for owner in self.participants.iter().skip(if i == 0 { 1 } else { 0 }) {
                definitions
                    .get_exact(owner)
                    .map_err(|_| refused("resale participant missing"))?;
                if !participants.grants_exact(owner, provider.owner(), action) {
                    return Err(refused("resale participant grant"));
                }
            }
        }
        Ok(())
    }
}
pub(super) fn descriptor(
    owner: &OwnerId,
    family: &str,
    max: usize,
) -> Result<StateFamilyDescriptor, ActionStateError> {
    let c = |s: &str| {
        CanonicalId::new(format!("{family}.{s}.v1")).map_err(|_| refused("resale descriptor id"))
    };
    let (decode, validate, key) = if owner.as_str() == "roll" {
        ("strict_decode", "invariants", 192)
    } else {
        ("decode", "validate", 32)
    };
    StateFamilyDescriptor::new(
        StateFamilyId::new(family).map_err(|_| refused("resale family"))?,
        c("key")?,
        c("value")?,
        1,
        c(decode)?,
        c(validate)?,
        c("canonical")?,
    )
    .and_then(|d| d.with_limits(key, max))
    .map_err(|_| refused("resale descriptor"))
}
pub(super) fn action_payload(i: usize) -> Result<Value, ActionStateError> {
    Ok(match i {
        0 => {
            json!({"format":1,"protocol":"personal_paid_resale_v1","authority":"current_personal_paid_holder","named_buyer":true,"source_free":true,"existing_term":"unchanged","no_automatic_acceptance":true})
        }
        1 => {
            json!({"format":1,"protocol":"personal_paid_resale_v1","authority":"named_buyer","claims":"exact_zero","payment":"immediate_exact_gross_split","holder":"same_ack","term":"unchanged"})
        }
        2 => {
            json!({"format":1,"protocol":"personal_paid_resale_v1","authority":"named_buyer","claims":"positive","hold":"single_real_gross_escrow","holder":"unchanged","future_terminal_reserved":true})
        }
        3 => {
            json!({"format":1,"protocol":"personal_paid_resale_v1","authority":"registered_private_service_intent","trigger":"last_paid_claim_close_or_same_clock_deadline","old_claims":"retained","term":"unchanged"})
        }
        4 => {
            json!({"format":1,"protocol":"personal_paid_resale_v1","authority":"registered_private_service_intent","causes":["registry_revoked","clock_changed","recipient_unavailable","monetary_binding_changed"],"source_rights":"not_required"})
        }
        _ => return Err(refused("resale action index")),
    })
}
pub(super) fn action_reference(
    retention: &DefinitionRef,
    i: usize,
) -> Result<DefinitionRef, ActionStateError> {
    let id = ACTIONS
        .get(i)
        .ok_or_else(|| refused("resale action index"))?;
    let d = DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(|_| refused("resale kind"))?,
        DefinitionId::new(*id).map_err(|_| refused("resale action id"))?,
        1,
        action_payload(i)?,
    )
    .map_err(|_| refused("resale descriptor"))?
    .with_references(vec![retention.clone()]);
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("market").map_err(|_| refused("resale owner"))?,
            d,
        )
        .map_err(|_| refused("resale reference"))
}

pub(super) fn revocation_payload() -> Value {
    json!({"format":1,"protocol":"registry_exact_term_revocation_v1","reason":"registry_review","scope":"one_accepted_term_generation","duration":"original_grace_end","later_terms":"fresh_admission","shared_rating":"unchanged"})
}
pub(super) fn revocation_reference(
    authority: &Value,
    retention: &DefinitionRef,
) -> Result<DefinitionRef, ActionStateError> {
    let owner = OwnerId::new("roll").map_err(|_| refused("resale owner"))?;
    let d = |kind: &str, id: &str, payload: Value| {
        DefinitionDescriptor::new(
            DefinitionKind::new(kind).map_err(|_| refused("resale kind"))?,
            DefinitionId::new(id).map_err(|_| refused("resale id"))?,
            1,
            payload,
        )
        .map_err(|_| refused("resale descriptor"))
    };
    let authority = DefinitionRegistryBuilder::new()
        .register(
            owner.clone(),
            d("registry_authority", "mind_revocation", authority.clone())?,
        )
        .map_err(|_| refused("resale authority"))?;
    DefinitionRegistryBuilder::new()
        .register(
            owner,
            d("action", "registry_term_revocation", revocation_payload())?
                .with_references(vec![authority, retention.clone()]),
        )
        .map_err(|_| refused("resale Registry reference"))
}
