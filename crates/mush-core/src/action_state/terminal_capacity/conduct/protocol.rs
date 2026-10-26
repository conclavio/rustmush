use super::*;
use crate::action::ActionProviderRegistry;
use crate::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry,
    DefinitionRegistryBuilder,
};

pub(super) const ACTIONS: [&str; 5] = [
    "conduct_trial_admit",
    "conduct_trial_grant",
    "conduct_trial_settle",
    "conduct_trial_cancel",
    "conduct_trial_recover",
];
const TRANSITIONS: [&str; 5] = ["admit", "grant", "settle", "cancel", "recover"];

pub(super) fn manifest(r: &DefinitionRef) -> Result<(), ActionStateError> {
    if r.key().owner.as_str() != "tacsim"
        || r.key().kind.as_str() != "conduct_manifest"
        || r.key().id.as_str() != "safe_conduct"
        || r.version() != 1
    {
        return Err(refused("Conduct exact manifest"));
    }
    Ok(())
}
pub(super) fn action(m: &DefinitionRef, index: usize) -> Result<DefinitionRef, ActionStateError> {
    manifest(m)?;
    let name = ACTIONS
        .get(index)
        .ok_or_else(|| refused("Conduct action index"))?;
    let payload = if index < 3 {
        json!({"format":1,"protocol":"roll.conduct_trial.v1","transition":TRANSITIONS[index],"manifest":m})
    } else {
        json!({"format":1,"protocol":"roll.conduct_trial.v1","transition":TRANSITIONS[index],"cleanup":true})
    };
    let descriptor = DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(bad)?,
        DefinitionId::new(*name).map_err(bad)?,
        1,
        payload,
    )
    .map_err(bad)?;
    DefinitionRegistryBuilder::new()
        .register(
            owner("roll")?,
            if index < 3 {
                descriptor.with_references(vec![m.clone()])
            } else {
                descriptor
            },
        )
        .map_err(bad)
}
pub(super) fn action_index(
    m: &DefinitionRef,
    a: &DefinitionRef,
) -> Result<usize, ActionStateError> {
    for i in 0..5 {
        if action(m, i)? == *a {
            return Ok(i);
        }
    }
    Err(refused("Conduct full action reference"))
}
pub(super) fn participants() -> Result<[DefinitionRef; 3], ActionStateError> {
    [
        ("roll", "commercial_roots", "commercial_retention_v1"),
        ("tacsim", "conduct_quota", "conduct_quota_v1"),
        ("tacsim", "conduct_artifacts", "conduct_artifacts_v1"),
    ]
    .into_iter()
    .map(|(o, id, protocol)| {
        DefinitionRegistryBuilder::new()
            .register(
                owner(o)?,
                DefinitionDescriptor::new(
                    DefinitionKind::new("owner_participant").map_err(bad)?,
                    DefinitionId::new(id).map_err(bad)?,
                    1,
                    json!({"format":1,"protocol":protocol}),
                )
                .map_err(bad)?,
            )
            .map_err(bad)
    })
    .collect::<Result<Vec<_>, _>>()?
    .try_into()
    .map_err(|_| refused("Conduct three owners"))
}

#[derive(Clone, Debug, Serialize)]
pub(in crate::action_state) struct Protocol {
    manifest: DefinitionRef,
    participants: [DefinitionRef; 3],
    actions: [DefinitionRef; 5],
}
impl Protocol {
    pub(in crate::action_state) fn new(
        declaring: &OwnerId,
        manifest_ref: DefinitionRef,
        owners: [DefinitionRef; 3],
        actions: [DefinitionRef; 5],
    ) -> Result<Self, ActionStateError> {
        manifest(&manifest_ref)?;
        if declaring.as_str() != "roll" || owners != participants()? {
            return Err(refused("Conduct declaring owner or exact roster"));
        }
        for (i, a) in actions.iter().enumerate() {
            if *a != action(&manifest_ref, i)? {
                return Err(refused("Conduct closed actions"));
            }
        }
        Ok(Self {
            manifest: manifest_ref,
            participants: owners,
            actions,
        })
    }
    pub(in crate::action_state::terminal_capacity) fn admission_action(&self) -> &DefinitionRef {
        &self.actions[0]
    }
    pub(in crate::action_state::terminal_capacity) fn recognizes_historical(
        &self,
        a: &DefinitionRef,
    ) -> bool {
        a.key() == self.actions[0].key() && a.version() == 1
    }
    pub(super) fn current(&self, job: &wire::Job, index: usize) -> Result<(), ActionStateError> {
        if index < 3 && job.manifest != self.manifest {
            return Err(refused("Conduct current worker manifest differs"));
        }
        Ok(())
    }
    pub(super) fn action_index(
        &self,
        id: &RegisteredTxnIdentity,
    ) -> Result<usize, ActionStateError> {
        self.actions
            .iter()
            .position(|a| a == &id.action)
            .ok_or_else(|| refused("Conduct actual configured action"))
    }
    pub(in crate::action_state) fn required_images(
        &self,
        id: &RegisteredTxnIdentity,
    ) -> Option<Box<RequiredImages>> {
        self.actions.contains(&id.action).then(|| {
            Box::new(RequiredImages {
                participants: self.participants.clone(),
                action: id.action.clone(),
                commit: id.commit,
            })
        })
    }
    pub(in crate::action_state) fn validate_families(
        &self,
        registry: &StateFamilyRegistry,
    ) -> Result<(), ActionStateError> {
        for (family, bytes) in FAMILIES {
            let entry = registry
                .entries
                .get(&(owner("roll")?, StateFamilyId::new(family).map_err(bad)?))
                .ok_or_else(|| refused("Conduct missing family"))?;
            if entry.descriptor().canonical_value() != descriptor(family, bytes)?.canonical_value()
            {
                return Err(refused("Conduct exact family descriptor"));
            }
        }
        Ok(())
    }
    pub(in crate::action_state) fn validate_composition(
        &self,
        definitions: &DefinitionRegistry,
        providers: &ActionProviderRegistry,
        registry: &crate::participant::ParticipantRegistry,
    ) -> Result<(), ActionStateError> {
        definitions.get_exact(&self.manifest).map_err(bad)?;
        for p in &self.participants {
            definitions.get_exact(p).map_err(bad)?;
        }
        for a in &self.actions {
            definitions.get_exact(a).map_err(bad)?;
            let provider = providers.get_exact(a).map_err(bad)?;
            if provider.owner().as_str() != "roll"
                || self
                    .participants
                    .iter()
                    .any(|p| !registry.grants_exact(p, provider.owner(), a))
            {
                return Err(refused("Conduct exact native grants"));
            }
        }
        Ok(())
    }
}
#[derive(Debug)]
pub(in crate::action_state) struct RequiredImages {
    participants: [DefinitionRef; 3],
    action: DefinitionRef,
    commit: CommitToken,
}
impl RequiredImages {
    pub(in crate::action_state) fn check(
        &self,
        images: &crate::participant::OwnerImages,
    ) -> Result<(), crate::participant::ParticipantError> {
        images.require_exact_conduct(&self.participants, &self.action, self.commit)
    }
}
pub(super) fn descriptor(
    family: &str,
    bytes: usize,
) -> Result<StateFamilyDescriptor, ActionStateError> {
    let c = |part: &str| CanonicalId::new(format!("{family}.{part}.v1")).map_err(bad);
    StateFamilyDescriptor::new(
        StateFamilyId::new(family).map_err(bad)?,
        c("key")?,
        c("value")?,
        1,
        c("decode")?,
        c("validate")?,
        c("canonical")?,
    )
    .and_then(|d| d.with_limits(32, bytes))
    .map_err(bad)
}
