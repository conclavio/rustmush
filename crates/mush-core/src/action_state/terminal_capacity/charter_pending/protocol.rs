use super::*;
#[derive(Clone, Debug, Serialize)]
pub(in crate::action_state) struct Protocol {
    pub(super) participants: [DefinitionRef; 2],
    pub(super) actions: [DefinitionRef; 4],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) registry_revocation: Option<DefinitionRef>,
}
impl Protocol {
    pub(in crate::action_state) fn new(
        owner: &OwnerId,
        participants: [DefinitionRef; 2],
        actions: [DefinitionRef; 4],
        registry_revocation: Option<DefinitionRef>,
    ) -> Result<Self, ActionStateError> {
        if owner.as_str() != "roll" {
            return Err(refused("pending declaration owner"));
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
                return Err(refused("pending participant identity"));
            }
        }
        for (i, a) in actions.iter().enumerate() {
            if *a != action_reference(&participants[1], i)? {
                return Err(refused("pending full action identity or order"));
            }
        }
        if registry_revocation.as_ref().is_some_and(|a| {
            a.key().owner.as_str() != "roll"
                || a.key().kind.as_str() != "action"
                || a.key().id.as_str() != "registry_term_revocation"
                || a.version() != 1
        }) {
            return Err(refused("pending Registry action identity"));
        }
        Ok(Self {
            participants,
            actions,
            registry_revocation,
        })
    }
    pub(in crate::action_state::terminal_capacity) fn admission_action(&self) -> &DefinitionRef {
        &self.actions[1]
    }
    pub(in crate::action_state::terminal_capacity) fn action_index(
        &self,
        identity: &RegisteredTxnIdentity,
    ) -> Result<usize, ActionStateError> {
        self.actions
            .iter()
            .chain(self.registry_revocation.iter())
            .position(|a| a == &identity.action)
            .ok_or_else(|| refused("pending actual action differs"))
    }
    pub(in crate::action_state) fn validate_families(
        &self,
        registry: &StateFamilyRegistry,
    ) -> Result<(), ActionStateError> {
        for (family, max) in FAMILIES.into_iter().chain(
            self.registry_revocation
                .iter()
                .flat_map(|_| [(SANCTION, 32768), (STATUS, 4096)]),
        ) {
            let expected = descriptor(family, max)?;
            let id = expected.id().clone();
            let actual = registry
                .entries
                .get(&(
                    OwnerId::new("roll").map_err(|_| refused("pending owner"))?,
                    id,
                ))
                .ok_or_else(|| refused("pending family unavailable"))?;
            if actual.descriptor().canonical_value() != expected.canonical_value() {
                return Err(refused("pending descriptor differs"));
            }
        }
        // 17g C2f: the Offer and Fund write the buyer-offer row.
        owner_index::validate_families(registry, &[owner_index::PENDING_OFFERS])
    }
    pub(in crate::action_state) fn validate_composition(
        &self,
        definitions: &DefinitionRegistry,
        providers: &ActionProviderRegistry,
        participants: &crate::participant::ParticipantRegistry,
    ) -> Result<(), ActionStateError> {
        for (i, a) in self
            .actions
            .iter()
            .chain(self.registry_revocation.iter())
            .enumerate()
        {
            let def = definitions
                .get_exact(a)
                .map_err(|_| refused("pending action unavailable"))?;
            let provider = providers
                .get_exact(a)
                .map_err(|_| refused("pending provider unavailable"))?;
            if provider.owner().as_str() != "roll" {
                return Err(refused("pending provider owner"));
            }
            if i < 4 {
                if def.payload() != &action_payload(i)
                    || def.references() != [self.participants[1].clone()]
                {
                    return Err(refused("pending descriptor payload or references"));
                }
            } else {
                let refs = def.references();
                if def.payload() != &revocation_payload()
                    || refs.len() != 2
                    || !refs.contains(&self.participants[1])
                {
                    return Err(refused("pending Registry descriptor"));
                }
                let authority = refs
                    .iter()
                    .find(|r| r.key().kind.as_str() == "registry_authority")
                    .ok_or_else(|| refused("pending Registry authority reference"))?;
                if authority.key().owner.as_str() != "roll"
                    || authority.key().id.as_str() != "mind_revocation"
                    || authority.version() != 1
                {
                    return Err(refused("pending Registry authority identity"));
                }
                let authority = definitions
                    .get_exact(authority)
                    .map_err(|_| refused("pending Registry authority unavailable"))?;
                if *a != revocation_reference(authority.payload(), &self.participants[1])? {
                    return Err(refused("pending Registry full reference"));
                }
            }
            for r in self.participants.iter().skip(if i == 0 { 1 } else { 0 }) {
                definitions
                    .get_exact(r)
                    .map_err(|_| refused("pending participant unavailable"))?;
                if !participants.grants_exact(r, provider.owner(), a) {
                    return Err(refused("pending participant grant"));
                }
            }
        }
        Ok(())
    }
}
pub(super) fn descriptor(
    family: &str,
    max: usize,
) -> Result<StateFamilyDescriptor, ActionStateError> {
    let c = |s: &str| {
        CanonicalId::new(format!("{family}.{s}.v1")).map_err(|_| refused("pending descriptor id"))
    };
    StateFamilyDescriptor::new(
        StateFamilyId::new(family).map_err(|_| refused("pending family"))?,
        c("key")?,
        c("value")?,
        1,
        c("strict_decode")?,
        c("invariants")?,
        c("canonical")?,
    )
    .and_then(|d| d.with_limits(192, max))
    .map_err(|_| refused("pending descriptor"))
}
pub(super) fn action_payload(i: usize) -> Value {
    match i {
        0 => {
            json!({"format":1,"protocol":"personal_charter_pending_v1","authority":"current_personal_seller","named_buyer":true,"source_free":true,"settlement":"earlier_of_term_end_and_explicit_delay","no_automatic_acceptance":true})
        }
        1 => {
            json!({"format":1,"protocol":"personal_charter_pending_v1","authority":"named_buyer","hold":"single_real_escrow","title":"unchanged","future_terminal_reserved":true})
        }
        2 => {
            json!({"format":1,"protocol":"personal_charter_pending_v1","authority":"registered_private_service_intent","trigger":"last_owned_claim_close_or_same_clock_deadline","old_claims":"retained"})
        }
        _ => {
            json!({"format":1,"protocol":"personal_charter_pending_v1","authority":"registered_private_service_intent","causes":["registry_revoked","clock_changed","recipient_unavailable"],"source_rights":"not_required"})
        }
    }
}
pub(super) fn action_reference(
    retention: &DefinitionRef,
    i: usize,
) -> Result<DefinitionRef, ActionStateError> {
    let id = ACTIONS
        .get(i)
        .ok_or_else(|| refused("pending action index"))?;
    let d = DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(|_| refused("pending kind"))?,
        DefinitionId::new(*id).map_err(|_| refused("pending id"))?,
        if i == 0 { 2 } else { 1 },
        action_payload(i),
    )
    .map_err(|_| refused("pending descriptor"))?
    .with_references(vec![retention.clone()]);
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("roll").map_err(|_| refused("pending owner"))?,
            d,
        )
        .map_err(|_| refused("pending reference"))
}
pub(super) fn revocation_payload() -> Value {
    json!({"format":1,"protocol":"registry_exact_term_revocation_v1","reason":"registry_review","scope":"one_accepted_term_generation","duration":"original_grace_end","later_terms":"fresh_admission","shared_rating":"unchanged"})
}
pub(super) fn revocation_reference(
    authority: &Value,
    retention: &DefinitionRef,
) -> Result<DefinitionRef, ActionStateError> {
    let owner = OwnerId::new("roll").map_err(|_| refused("pending owner"))?;
    let d = |kind: &str, id: &str, payload: Value| {
        DefinitionDescriptor::new(
            DefinitionKind::new(kind).map_err(|_| refused("pending kind"))?,
            DefinitionId::new(id).map_err(|_| refused("pending id"))?,
            1,
            payload,
        )
        .map_err(|_| refused("pending descriptor"))
    };
    let authority = DefinitionRegistryBuilder::new()
        .register(
            owner.clone(),
            d("registry_authority", "mind_revocation", authority.clone())?,
        )
        .map_err(|_| refused("pending authority"))?;
    DefinitionRegistryBuilder::new()
        .register(
            owner,
            d("action", "registry_term_revocation", revocation_payload())?
                .with_references(vec![authority, retention.clone()]),
        )
        .map_err(|_| refused("pending Registry reference"))
}
