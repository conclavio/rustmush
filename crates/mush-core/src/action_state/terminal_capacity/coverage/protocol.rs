use super::*;
use crate::action::ActionProviderRegistry;
use crate::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry,
    DefinitionRegistryBuilder,
};

const ACTIONS: [&str; 3] = [
    "bind_hull_coverage",
    "claim_hull_coverage",
    "release_hull_coverage",
];
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Captured {
    format: u16,
    manifest: DefinitionRef,
    participants: [DefinitionRef; 2],
}
impl Captured {
    pub(super) fn parse(v: &Value) -> Result<Self, ActionStateError> {
        let p: Self = parsed(v)?;
        if p.format != 1
            || !locator(&p.manifest, "tacsim", "registry_manifest", "combat", 1)
            || !locator(
                &p.participants[0],
                "economy",
                "owner_participant",
                "initial_filing",
                1,
            )
            || p.participants[1] != roll_participant()?
        {
            return Err(refused("coverage exact captured protocol"));
        }
        Ok(p)
    }
    pub(super) fn action(&self, index: usize) -> Result<DefinitionRef, ActionStateError> {
        let id = ACTIONS
            .get(index)
            .ok_or_else(|| refused("coverage action index"))?;
        let payload = match index {
            0 => {
                json!({"format":1,"protocol":"hull_coverage_bind_v1","authority":"buyer_and_private_withheld_departure","start":"actual_world_ic","premium":"same_ack","liability":"pre_premium_funded_hold","retention":"same_ack","future_terminal_reserved":true})
            }
            1 => {
                json!({"format":1,"protocol":"hull_coverage_claim_v1","authority":"actual_private_host_loss","loss":"recorded_ic_destruction","window":"captured_original_clock_interval","delay":"captured_loss_plus_delay","wreck_title":"same_ack_insurer"})
            }
            2 => {
                json!({"format":1,"protocol":"hull_coverage_release_v1","authority":"actual_private_close_and_no_covered_loss","causes":["clean_close","cancelled","expired_clean"],"premium":"retained","collateral":"released_same_ack"})
            }
            _ => return Err(refused("coverage action index")),
        };
        let mut refs = vec![
            self.manifest.clone(),
            self.participants[0].clone(),
            self.participants[1].clone(),
        ];
        refs.sort();
        DefinitionRegistryBuilder::new()
            .register(
                owner("tacsim")?,
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").map_err(bad)?,
                    DefinitionId::new(*id).map_err(bad)?,
                    1,
                    payload,
                )
                .map_err(bad)?
                .with_references(refs),
            )
            .map_err(bad)
    }
}
fn locator(r: &DefinitionRef, owner: &str, kind: &str, id: &str, version: u32) -> bool {
    r.key().owner.as_str() == owner
        && r.key().kind.as_str() == kind
        && r.key().id.as_str() == id
        && r.version() == version
}
fn roll_participant() -> Result<DefinitionRef, ActionStateError> {
    DefinitionRegistryBuilder::new()
        .register(
            owner("roll")?,
            DefinitionDescriptor::new(
                DefinitionKind::new("owner_participant").map_err(bad)?,
                DefinitionId::new("commercial_roots").map_err(bad)?,
                1,
                json!({"format":1,"protocol":"commercial_retention_v1"}),
            )
            .map_err(bad)?,
        )
        .map_err(bad)
}

#[derive(Clone, Debug, Serialize)]
pub(in crate::action_state) struct Protocol {
    captured: Captured,
    actions: [DefinitionRef; 3],
}
impl Protocol {
    pub(in crate::action_state) fn new(
        declaring: &OwnerId,
        manifest: DefinitionRef,
        participants: [DefinitionRef; 2],
        actions: [DefinitionRef; 3],
    ) -> Result<Self, ActionStateError> {
        if declaring.as_str() != "tacsim" {
            return Err(refused("coverage declaring owner"));
        }
        let captured =
            Captured::parse(&json!({"format":1,"manifest":manifest,"participants":participants}))?;
        for (i, a) in actions.iter().enumerate() {
            if a != &captured.action(i)? {
                return Err(refused("coverage closed action recipe"));
            }
        }
        Ok(Self { captured, actions })
    }
    pub(super) fn action_index(
        &self,
        id: &RegisteredTxnIdentity,
    ) -> Result<usize, ActionStateError> {
        self.actions
            .iter()
            .position(|a| a == &id.action)
            .ok_or_else(|| refused("coverage current action"))
    }
    pub(super) fn captured(&self) -> &Captured {
        &self.captured
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
    pub(in crate::action_state) fn required_images(
        &self,
        identity: &RegisteredTxnIdentity,
    ) -> Option<Box<RequiredImages>> {
        self.actions.contains(&identity.action).then(|| {
            Box::new(RequiredImages {
                participants: self.captured.participants.clone(),
                action: identity.action.clone(),
                commit: identity.commit,
            })
        })
    }
    pub(in crate::action_state) fn validate_families(
        &self,
        registry: &StateFamilyRegistry,
    ) -> Result<(), ActionStateError> {
        for (o, f, ceiling) in FAMILIES.iter().chain(&PREREQUISITES) {
            let entry = registry
                .entries
                .get(&(owner(o)?, StateFamilyId::new(f).map_err(bad)?))
                .ok_or_else(|| refused("coverage missing typed family"))?;
            if entry.descriptor().canonical_value() != descriptor(o, f, *ceiling)?.canonical_value()
            {
                return Err(refused("coverage exact family descriptor"));
            }
        }
        if !registry.delegates_family(
            &owner("roll")?,
            &StateFamilyId::new(RETENTION).map_err(bad)?,
            &owner("tacsim")?,
        ) {
            return Err(refused("coverage retention delegation"));
        }
        Ok(())
    }
    pub(in crate::action_state) fn validate_composition(
        &self,
        definitions: &DefinitionRegistry,
        providers: &ActionProviderRegistry,
        participants: &crate::participant::ParticipantRegistry,
    ) -> Result<(), ActionStateError> {
        definitions
            .get_exact(&self.captured.manifest)
            .map_err(bad)?;
        for p in &self.captured.participants {
            definitions.get_exact(p).map_err(bad)?;
        }
        for action in &self.actions {
            definitions.get_exact(action).map_err(bad)?;
            let provider = providers.get_exact(action).map_err(bad)?;
            if provider.owner().as_str() != "tacsim"
                || self
                    .captured
                    .participants
                    .iter()
                    .any(|p| !participants.grants_exact(p, provider.owner(), action))
            {
                return Err(refused("coverage exact native grants"));
            }
        }
        Ok(())
    }
}
#[derive(Debug)]
pub(in crate::action_state) struct RequiredImages {
    participants: [DefinitionRef; 2],
    action: DefinitionRef,
    commit: CommitToken,
}
impl RequiredImages {
    pub(in crate::action_state) fn check(
        &self,
        images: &crate::participant::OwnerImages,
    ) -> Result<(), crate::participant::ParticipantError> {
        images.require_exact_exchange(&self.participants, &self.action, self.commit)
    }
}
pub(super) fn descriptor(
    o: &str,
    family: &str,
    ceiling: usize,
) -> Result<StateFamilyDescriptor, ActionStateError> {
    let c = |suffix: &str| CanonicalId::new(format!("{family}.{suffix}.v1")).map_err(bad);
    StateFamilyDescriptor::new(
        StateFamilyId::new(family).map_err(bad)?,
        c("key")?,
        c("value")?,
        1,
        c(if o == "roll" {
            "strict_decode"
        } else {
            "decode"
        })?,
        c(if o == "roll" {
            "invariants"
        } else {
            "validate"
        })?,
        c("canonical")?,
    )
    .and_then(|d| d.with_limits(if o == "roll" { 192 } else { 32 }, ceiling))
    .map_err(bad)
}
