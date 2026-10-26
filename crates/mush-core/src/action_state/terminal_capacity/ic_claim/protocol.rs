//! Fixed four-owner protocol; no module supplies targets or verifier callbacks.
use super::*;
use crate::action::ActionProviderRegistry;
use crate::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry,
    DefinitionRegistryBuilder,
};

pub(super) const ACTIONS: [&str; 2] = ["open_ic_mind", "close_ic_mind"];
pub(super) const OWNERS: [&str; 4] = ["roll", "tacsim", "tactics", "sim"];
const IDS: [&str; 4] = [
    "commercial_roots",
    "ic_sortie",
    "ic_custody",
    "ic_exclusion",
];

pub(super) fn participant_descriptors()
-> Result<[(OwnerId, DefinitionDescriptor); 4], ActionStateError> {
    let payloads = [
        json!({"format":1,"protocol":"commercial_retention_v1"}),
        json!({"format":1,"protocol":"ic_sortie_v1","max_open":1024,"max_history":8192,"max_record_bytes":1048576,"max_total_bytes":67108864}),
        json!({"format":1,"protocol":"ic_custody_v1","max_open":1024,"max_history":8192,"max_unit_bytes":1048576,"max_history_bytes":8192}),
        json!({"format":1,"protocol":"ic_exclusion_v1","max_open":1024,"max_history":8192,"max_record_bytes":8192}),
    ];
    OWNERS
        .into_iter()
        .zip(IDS)
        .zip(payloads)
        .map(|((owner, id), payload)| {
            Ok((
                owner_id(owner)?,
                DefinitionDescriptor::new(
                    DefinitionKind::new("owner_participant").map_err(bad)?,
                    DefinitionId::new(id).map_err(bad)?,
                    1,
                    payload,
                )
                .map_err(bad)?,
            ))
        })
        .collect::<Result<Vec<_>, ActionStateError>>()?
        .try_into()
        .map_err(|_| refused("IC participant count"))
}
pub(super) fn participant_refs() -> Result<[DefinitionRef; 4], ActionStateError> {
    participant_descriptors()?
        .into_iter()
        .map(|(owner, descriptor)| {
            DefinitionRegistryBuilder::new()
                .register(owner, descriptor)
                .map_err(bad)
        })
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| refused("IC participant count"))
}
fn check_manifest(reference: &DefinitionRef) -> Result<(), ActionStateError> {
    if reference.key().owner.as_str() != "tacsim"
        || reference.key().kind.as_str() != "registry_manifest"
        || reference.key().id.as_str() != "combat"
        || reference.version() != 1
    {
        return Err(refused("IC manifest identity"));
    }
    Ok(())
}
pub(super) fn action_descriptor(
    manifest: &DefinitionRef,
    participants: &[DefinitionRef; 4],
    index: usize,
) -> Result<DefinitionDescriptor, ActionStateError> {
    check_manifest(manifest)?;
    if participants != &participant_refs()? {
        return Err(refused("IC fixed participant identity"));
    }
    let mut refs = vec![manifest.clone()];
    refs.extend(participants.iter().cloned());
    let id = ACTIONS.get(index).ok_or_else(|| refused("IC transition"))?;
    let transition = ["open", "close"]
        .get(index)
        .copied()
        .ok_or_else(|| refused("IC transition"))?;
    DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(bad)?,
        DefinitionId::new(*id).map_err(bad)?,
        1,
        json!({"format":1,"protocol":"ic_mind_custody_v1","transition":transition}),
    )
    .map(|row| row.with_references(refs))
    .map_err(bad)
}
pub(super) fn action_reference(
    manifest: &DefinitionRef,
    participants: &[DefinitionRef; 4],
    index: usize,
) -> Result<DefinitionRef, ActionStateError> {
    DefinitionRegistryBuilder::new()
        .register(
            owner_id("tacsim")?,
            action_descriptor(manifest, participants, index)?,
        )
        .map_err(bad)
}

#[derive(Clone, Debug, Serialize)]
pub(in crate::action_state) struct Protocol {
    pub(super) manifest: DefinitionRef,
    pub(super) participants: [DefinitionRef; 4],
    pub(super) actions: [DefinitionRef; 2],
}
impl Protocol {
    pub(in crate::action_state) fn new(
        owner: &OwnerId,
        manifest: DefinitionRef,
        participants: [DefinitionRef; 4],
        actions: [DefinitionRef; 2],
    ) -> Result<Self, ActionStateError> {
        if owner.as_str() != "tacsim" {
            return Err(refused("IC declaration owner"));
        }
        for (index, action) in actions.iter().enumerate() {
            if *action != action_reference(&manifest, &participants, index)? {
                return Err(refused("IC complete action identity"));
            }
        }
        Ok(Self {
            manifest,
            participants,
            actions,
        })
    }
    pub(in crate::action_state) fn enable_participant_limits(
        &self,
        registry: &mut crate::participant::ParticipantRegistry,
    ) -> Result<(), crate::participant::ParticipantError> {
        registry.enable_ic_claim(self.actions.clone(), self.participants.clone())
    }
    pub(in crate::action_state) fn require_owner_images(
        &self,
        identity: &RegisteredTxnIdentity,
        images: &crate::participant::OwnerImages,
    ) -> Result<(), crate::participant::ParticipantError> {
        if self.actions.contains(&identity.action) {
            images.require_exact_ic(&self.participants, &identity.action, identity.commit)?;
        }
        Ok(())
    }
    pub(in crate::action_state) fn admission_action(&self) -> &DefinitionRef {
        &self.actions[0]
    }
    pub(in crate::action_state) fn recognizes_historical(&self, effect: &DefinitionRef) -> bool {
        effect.key() == self.admission_action().key() && effect.version() == 1
    }
    pub(super) fn action_index(
        &self,
        identity: &RegisteredTxnIdentity,
    ) -> Result<usize, ActionStateError> {
        self.actions
            .iter()
            .position(|a| a == &identity.action)
            .ok_or_else(|| refused("IC actual action differs"))
    }
    pub(in crate::action_state) fn validate_families(
        &self,
        registry: &StateFamilyRegistry,
    ) -> Result<(), ActionStateError> {
        let expected = family_descriptor()?;
        let owner = owner_id("roll")?;
        let entry = registry
            .entries
            .get(&(owner.clone(), expected.id().clone()))
            .ok_or_else(|| refused("IC claim family missing"))?;
        if entry.descriptor().canonical_value() != expected.canonical_value()
            || !registry.delegates_family(&owner, expected.id(), &owner_id("tacsim")?)
        {
            return Err(refused("IC claim descriptor or delegation differs"));
        }
        Ok(())
    }
    pub(in crate::action_state) fn validate_composition(
        &self,
        definitions: &DefinitionRegistry,
        providers: &ActionProviderRegistry,
        participants: &crate::participant::ParticipantRegistry,
    ) -> Result<(), ActionStateError> {
        definitions.get_exact(&self.manifest).map_err(bad)?;
        for action in &self.actions {
            let definition = definitions.get_exact(action).map_err(bad)?;
            let provider = providers.get_exact(action).map_err(bad)?;
            let refs = definition.references();
            if provider.owner().as_str() != "tacsim"
                || refs.len() != 5
                || !refs.contains(&self.manifest)
            {
                return Err(refused("IC exact provider or reference set"));
            }
            for reference in &self.participants {
                definitions.get_exact(reference).map_err(bad)?;
                if !refs.contains(reference)
                    || !participants.grants_exact(reference, provider.owner(), action)
                {
                    return Err(refused("IC actual participant request grant missing"));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapturedParticipants {
    retention: DefinitionRef,
    seal: DefinitionRef,
    custody: DefinitionRef,
    exclusion: DefinitionRef,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapturedProtocol {
    manifest: DefinitionRef,
    participants: CapturedParticipants,
}
impl CapturedProtocol {
    pub(super) fn current(protocol: &Protocol) -> Self {
        let [retention, seal, custody, exclusion] = protocol.participants.clone();
        Self {
            manifest: protocol.manifest.clone(),
            participants: CapturedParticipants {
                retention,
                seal,
                custody,
                exclusion,
            },
        }
    }
    pub(super) fn action(&self, index: usize) -> Result<DefinitionRef, ActionStateError> {
        action_reference(
            &self.manifest,
            &[
                self.participants.retention.clone(),
                self.participants.seal.clone(),
                self.participants.custody.clone(),
                self.participants.exclusion.clone(),
            ],
            index,
        )
    }
}
