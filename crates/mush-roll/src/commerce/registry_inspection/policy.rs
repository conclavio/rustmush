//! Trusted composition identifies a real Registry authority; no actor is created.
use super::*;
use mush_org::{OrgId, OrgScopeId, SystemPrincipalId};
use serde::de::{DeserializeSeed, SeqAccess, Visitor};
use serde::{Deserializer, de};
use std::marker::PhantomData;

/// Exact Registry scope and operators for inspection. Decoding this value is data, never a
/// grant; the live Org authorizer remains mandatory on every invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PolicyWire")]
pub struct RegistryInspectionPolicy {
    pub(crate) format: u16,
    pub(crate) issuer: SystemPrincipalId,
    pub(crate) organization: OrgId,
    pub(crate) scope: OrgScopeId,
    pub(crate) operators: Vec<ObjId>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyWire {
    format: u16,
    issuer: SystemPrincipalId,
    organization: OrgId,
    scope: OrgScopeId,
    #[serde(deserialize_with = "operators")]
    operators: Vec<ObjId>,
}
fn operators<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ObjId>, D::Error> {
    bounded::<D, ObjId, 32>(d)
}
/// The revocation policy's bounded row decoder, repeated (it is that module's own).
pub(super) fn bounded<'de, D: Deserializer<'de>, T: Deserialize<'de>, const N: usize>(
    d: D,
) -> Result<Vec<T>, D::Error> {
    struct Rows<T, const N: usize>(PhantomData<T>);
    struct Overflow;
    impl<'de> DeserializeSeed<'de> for Overflow {
        type Value = ();
        fn deserialize<D: Deserializer<'de>>(self, _: D) -> Result<(), D::Error> {
            Err(de::Error::custom("row bound"))
        }
    }
    impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for Rows<T, N> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "at most {N} rows")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            if seq.size_hint().is_some_and(|n| n > N) {
                return Err(de::Error::custom("row bound"));
            }
            let mut rows = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(N));
            while rows.len() < N {
                match seq.next_element()? {
                    Some(row) => rows.push(row),
                    None => return Ok(rows),
                }
            }
            // Refuse the next element before decoding its payload.
            let _ = seq.next_element_seed(Overflow)?;
            Ok(rows)
        }
    }
    d.deserialize_seq(Rows::<T, N>(PhantomData))
}
impl TryFrom<PolicyWire> for RegistryInspectionPolicy {
    type Error = PublicationError;
    fn try_from(w: PolicyWire) -> Result<Self, Self::Error> {
        let p = Self {
            format: w.format,
            issuer: w.issuer,
            organization: w.organization,
            scope: w.scope,
            operators: w.operators,
        };
        p.validate()?;
        Ok(p)
    }
}
impl RegistryInspectionPolicy {
    /// Construct one explicit scope. Operators must already be sorted and unique.
    ///
    /// # Errors
    /// An empty, oversized or unsorted operator list.
    pub fn new(
        issuer: SystemPrincipalId,
        organization: OrgId,
        scope: OrgScopeId,
        operators: Vec<ObjId>,
    ) -> Result<Self, ModError> {
        let p = Self {
            format: 1,
            issuer,
            organization,
            scope,
            operators,
        };
        p.validate().map_err(error)?;
        Ok(p)
    }
    pub(crate) fn validate(&self) -> Result<(), PublicationError> {
        if self.format != 1
            || self.operators.is_empty()
            || self.operators.len() > 32
            || self
                .operators
                .windows(2)
                .any(|pair| matches!(pair, [a, b] if a >= b))
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
    /// The configured operators, ascending.
    #[must_use]
    pub fn operators(&self) -> &[ObjId] {
        &self.operators
    }
    /// The Org scope the capability is authorized on.
    #[must_use]
    pub const fn scope(&self) -> OrgScopeId {
        self.scope
    }
    /// The Registry org (doc 17h cut M8d2: a seizure's claimant and a forfeit's recipient,
    /// Q-M8-4).
    #[must_use]
    pub const fn organization(&self) -> OrgId {
        self.organization
    }
    /// Exact action identity (tacsim's owner), independent of the current execution manifest.
    ///
    /// # Errors
    /// A policy that does not validate.
    pub fn action(&self) -> Result<DefinitionRef, ModError> {
        let [_, action, _] = self.definitions()?;
        DefinitionRegistryBuilder::new()
            .register(action_owner()?, action)
            .map_err(error)
    }
    /// Exact outcome identity (tacsim's owner).
    ///
    /// # Errors
    /// A policy that does not validate.
    pub fn outcome(&self) -> Result<DefinitionRef, ModError> {
        let [_, _, outcome] = self.definitions()?;
        DefinitionRegistryBuilder::new()
            .register(action_owner()?, outcome)
            .map_err(error)
    }
    /// Exact authority identity (the Roll's owner).
    ///
    /// # Errors
    /// A policy that does not validate.
    pub fn authority(&self) -> Result<DefinitionRef, ModError> {
        let [authority, _, _] = self.definitions()?;
        DefinitionRegistryBuilder::new()
            .register(owner()?, authority)
            .map_err(error)
    }
    /// The authority descriptor the Roll registers (`registry_authority` / `mind_inspection`).
    ///
    /// # Errors
    /// A policy that does not validate.
    pub fn authority_descriptor(&self) -> Result<DefinitionDescriptor, ModError> {
        let [authority, _, _] = self.definitions()?;
        Ok(authority)
    }
    /// The action and outcome descriptors tacsim registers under its own owner.
    ///
    /// # Errors
    /// A policy that does not validate.
    pub fn tacsim_descriptors(&self) -> Result<[DefinitionDescriptor; 2], ModError> {
        let [_, action, outcome] = self.definitions()?;
        Ok([action, outcome])
    }
    /// Doc 17h cut M8d D3: the fine's action and outcome descriptors, tacsim's, the action
    /// referencing the same authority.
    ///
    /// # Errors
    /// A policy that does not validate.
    pub fn fine_descriptors(&self) -> Result<[DefinitionDescriptor; 2], ModError> {
        let [authority, _, _] = self.definitions()?;
        let authority_ref = DefinitionRegistryBuilder::new()
            .register(owner()?, authority)
            .map_err(error)?;
        let action = descriptor(
            "action",
            "registry_mind_fine",
            json!({
                "format": 1,
                "protocol": "registry_inspection_fine_v1",
                "reads": ["unwarranted_inspection", "hull_title_holder"],
                "writes": "one_fine_record",
                "levies": "one_period_sanction_obligation",
                "capability": REGISTRY_FINE_CAPABILITY
            }),
        )?
        .with_references(vec![authority_ref]);
        let outcome = descriptor(
            "outcome",
            "registry_mind_fined",
            json!({"format": 1, "fields": ["hull", "seq", "amount", "operator", "summary"]}),
        )?;
        Ok([action, outcome])
    }
    /// Doc 17h cut M8d D3: the fine action's exact identity (tacsim's owner).
    ///
    /// # Errors
    /// A policy that does not validate.
    pub fn fine_action(&self) -> Result<DefinitionRef, ModError> {
        let [action, _] = self.fine_descriptors()?;
        DefinitionRegistryBuilder::new()
            .register(action_owner()?, action)
            .map_err(error)
    }
    /// Doc 17h cut M8d D3: the fine outcome's exact identity (tacsim's owner).
    ///
    /// # Errors
    /// A policy that does not validate.
    pub fn fine_outcome(&self) -> Result<DefinitionRef, ModError> {
        let [_, outcome] = self.fine_descriptors()?;
        DefinitionRegistryBuilder::new()
            .register(action_owner()?, outcome)
            .map_err(error)
    }
    /// Doc 17h cut M8d D4: the principal revocation's action and outcome descriptors,
    /// tacsim's, the action referencing the same authority.
    ///
    /// # Errors
    /// A policy that does not validate.
    pub fn principal_descriptors(&self) -> Result<[DefinitionDescriptor; 2], ModError> {
        let [authority, _, _] = self.definitions()?;
        let authority_ref = DefinitionRegistryBuilder::new()
            .register(owner()?, authority)
            .map_err(error)?;
        let action = descriptor(
            "action",
            "registry_principal_revocation",
            json!({
                "format": 1,
                "protocol": "registry_principal_revocation_v1",
                "reads": ["unwarranted_inspection"],
                "writes": "one_revoked_principal",
                "bars": "new_licences_for_the_term",
                "capability": super::super::registry_revocation::REGISTRY_REVOCATION_CAPABILITY
            }),
        )?
        .with_references(vec![authority_ref]);
        let outcome = descriptor(
            "outcome",
            "registry_principal_revoked",
            json!({"format": 1, "fields": ["principal", "until_ic", "operator", "summary"]}),
        )?;
        Ok([action, outcome])
    }
    /// Doc 17h cut M8d D4: the principal revocation action's exact identity.
    ///
    /// # Errors
    /// A policy that does not validate.
    pub fn principal_action(&self) -> Result<DefinitionRef, ModError> {
        let [action, _] = self.principal_descriptors()?;
        DefinitionRegistryBuilder::new()
            .register(action_owner()?, action)
            .map_err(error)
    }
    /// Doc 17h cut M8d D4: the principal revocation outcome's exact identity.
    ///
    /// # Errors
    /// A policy that does not validate.
    pub fn principal_outcome(&self) -> Result<DefinitionRef, ModError> {
        let [_, outcome] = self.principal_descriptors()?;
        DefinitionRegistryBuilder::new()
            .register(action_owner()?, outcome)
            .map_err(error)
    }
    /// **The quirk claim's action and outcome descriptors** (doc 17h cut M8g G3), tacsim's,
    /// the action referencing the same authority: an author files; no capability is asked.
    ///
    /// # Errors
    /// A policy that does not validate.
    pub fn claim_descriptors(&self) -> Result<[DefinitionDescriptor; 2], ModError> {
        let [authority, _, _] = self.definitions()?;
        let authority_ref = DefinitionRegistryBuilder::new()
            .register(owner()?, authority)
            .map_err(error)?;
        let action = descriptor(
            "action",
            "mind_quirk_claim",
            json!({
                "format": 1,
                "protocol": "mind_quirk_claim_v1",
                "reads": ["charter_title", "earned_contacts"],
                "writes": "one_claim_record",
                "confirms": "nothing"
            }),
        )?
        .with_references(vec![authority_ref]);
        let outcome = descriptor(
            "outcome",
            "mind_quirk_claimed",
            json!({"format": 1, "fields": ["claim", "author", "seq"]}),
        )?;
        Ok([action, outcome])
    }
    /// Doc 17h cut M8g G3: the claim action's exact identity (tacsim's owner).
    ///
    /// # Errors
    /// A policy that does not validate.
    pub fn claim_action(&self) -> Result<DefinitionRef, ModError> {
        let [action, _] = self.claim_descriptors()?;
        DefinitionRegistryBuilder::new()
            .register(action_owner()?, action)
            .map_err(error)
    }
    /// Doc 17h cut M8g G3: the claim outcome's exact identity (tacsim's owner).
    ///
    /// # Errors
    /// A policy that does not validate.
    pub fn claim_outcome(&self) -> Result<DefinitionRef, ModError> {
        let [_, outcome] = self.claim_descriptors()?;
        DefinitionRegistryBuilder::new()
            .register(action_owner()?, outcome)
            .map_err(error)
    }
    /// Doc 17h cut M8d2 S4/S5: `[seize action, seize outcome, forfeit action, forfeit
    /// outcome]`, tacsim's, each action referencing the same authority. The two action ids are
    /// the economy's closed seizure plan's (`mush_economy::registry_filing::seizure`).
    ///
    /// # Errors
    /// A policy that does not validate.
    pub fn seizure_descriptors(&self) -> Result<[DefinitionDescriptor; 4], ModError> {
        use mush_economy::registry_filing::seizure::{FORFEIT_ACTION_ID, SEIZE_ACTION_ID};
        let [authority, _, _] = self.definitions()?;
        let authority_ref = DefinitionRegistryBuilder::new()
            .register(owner()?, authority)
            .map_err(error)?;
        let seize = descriptor(
            "action",
            SEIZE_ACTION_ID,
            json!({
                "format": 1,
                "protocol": "registry_core_seizure_v1",
                "reads": ["newest_inspection", "fitted_core", "core_title"],
                "writes": ["one_seizure_record", "core_title_disputed"],
                "capability": super::REGISTRY_SEIZE_CAPABILITY
            }),
        )?
        .with_references(vec![authority_ref.clone()]);
        let seized = descriptor(
            "outcome",
            "registry_core_seized",
            json!({"format": 1, "fields": ["hull", "seq", "operator", "summary"]}),
        )?;
        let forfeit = descriptor(
            "action",
            FORFEIT_ACTION_ID,
            json!({
                "format": 1,
                "protocol": "registry_core_forfeit_v1",
                "reads": ["seizure_record", "seizure_hold"],
                "writes": ["seizure_record_forfeited", "core_title_to_registry"],
                "capability": super::REGISTRY_FORFEIT_CAPABILITY
            }),
        )?
        .with_references(vec![authority_ref]);
        let forfeited = descriptor(
            "outcome",
            "registry_core_forfeited",
            json!({"format": 1, "fields": ["hull", "seq", "operator", "summary"]}),
        )?;
        Ok([seize, seized, forfeit, forfeited])
    }
    /// Doc 17h cut M8d2: the four seizure definitions' exact identities (tacsim's owner), in
    /// [`Self::seizure_descriptors`]' order.
    ///
    /// # Errors
    /// A policy that does not validate.
    pub fn seizure_refs(&self) -> Result<[DefinitionRef; 4], ModError> {
        let owner = action_owner()?;
        let [a, b, c, d] = self.seizure_descriptors()?;
        let register = |descriptor: DefinitionDescriptor| {
            DefinitionRegistryBuilder::new()
                .register(owner.clone(), descriptor)
                .map_err(error)
        };
        Ok([register(a)?, register(b)?, register(c)?, register(d)?])
    }
    /// `[authority, action, outcome]`: the authority under the Roll's owner, the action and
    /// outcome under tacsim's ([`ACTION_OWNER`]); the action references the authority.
    pub(crate) fn definitions(&self) -> Result<[DefinitionDescriptor; 3], ModError> {
        self.validate().map_err(error)?;
        let authority = descriptor(
            "registry_authority",
            "mind_inspection",
            serde_json::to_value(self).map_err(error)?,
        )?;
        let authority_ref = DefinitionRegistryBuilder::new()
            .register(owner()?, authority.clone())
            .map_err(error)?;
        let action = descriptor(
            "action",
            "registry_mind_inspection",
            json!({
                "format": 1,
                "protocol": "registry_hull_inspection_v1",
                "reads": ["fitted_core", "provenance", "open_trip_seal", "licence_state"],
                "writes": "one_inspection_record",
                "grants": "nothing"
            }),
        )?
        .with_references(vec![authority_ref]);
        let outcome = descriptor(
            "outcome",
            "registry_mind_inspected",
            json!({"format": 1, "fields": ["inspection", "hull", "operator", "summary"]}),
        )?;
        Ok([authority, action, outcome])
    }
}
fn owner() -> Result<OwnerId, ModError> {
    OwnerId::new(crate::NS).map_err(error)
}
fn action_owner() -> Result<OwnerId, ModError> {
    OwnerId::new(ACTION_OWNER).map_err(error)
}
fn descriptor(
    kind: &str,
    id: &str,
    value: serde_json::Value,
) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(error)?,
        DefinitionId::new(id).map_err(error)?,
        1,
        value,
    )
    .map_err(error)
}
