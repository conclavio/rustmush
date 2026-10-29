//! Bounded, exact unit-authority records. These records confer nothing without
//! the resolver's current title, organization, parent and mission checks.

use std::collections::BTreeSet;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};

use crate::{CapabilityId, GrantId, OrgId, OrgScopeId, PrincipalId, RoleId, ScopeSelector};

/// A registry deliberately retains revoked identities rather than reusing them.
pub const MAX_UNIT_GRANTS: usize = 1024;
/// Finite scopes and wildcard exceptions are bounded on write and decode.
pub const MAX_UNIT_SCOPE_ITEMS: usize = 256;
/// Maximum number of unit-grant rows in a live proof, including organization loans.
pub const MAX_UNIT_GRANT_DEPTH: usize = 8;
/// Shared by the entire request, including every unit in a finite admission.
/// Also bounds branching organization loans and direct organization searches.
pub const MAX_UNIT_PROOF_VISITS: usize = 128;

/// Never reused within the durable unit-grant registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnitGrantId(pub u64);

/// The immutable journal position which established this title. Routine asset
/// effects leave it unchanged; free transfers and title reissues replace it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TitleLineage {
    pub journal_seq: u64,
    pub effect_index: u32,
}

/// Rights remain separate. Delegating a right is a different action from using it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitCapability {
    OperateCrew,
    Deploy,
    CommandAssignedAi,
    RecoverLoad,
    MaintainRefit,
    TransferAbandonTitle,
}

impl UnitCapability {
    /// Organization capability vocabulary; no generic `use` or financial right.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OperateCrew => "unit.operate_crew",
            Self::Deploy => "unit.deploy",
            Self::CommandAssignedAi => "unit.command_assigned_ai",
            Self::RecoverLoad => "unit.recover_load",
            Self::MaintainRefit => "unit.maintain_refit",
            Self::TransferAbandonTitle => "unit.transfer_abandon_title",
        }
    }

    pub(crate) fn capability_id(self) -> Option<CapabilityId> {
        CapabilityId::new(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitAction {
    Exercise(UnitCapability),
    Delegate(UnitCapability),
}

impl UnitAction {
    #[must_use]
    pub fn capability(self) -> UnitCapability {
        match self {
            Self::Exercise(capability) | Self::Delegate(capability) => capability,
        }
    }

    #[must_use]
    pub fn requires_delegation(self) -> bool {
        matches!(self, Self::Delegate(_))
    }
}

/// The accepted assignment, not the mutable draft or a cached leader identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionBinding {
    pub mission: u64,
    pub detachment: u64,
    pub roster_revision: u64,
    pub accepted_roster_hash: [u8; 32],
}

impl MissionBinding {
    pub fn validate(&self) -> Result<(), UnitGrantError> {
        if self.mission == 0 || self.detachment == 0 || self.roster_revision == 0 {
            return Err(UnitGrantError::Malformed("zero mission identity"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum UnitGrantee {
    Character(ObjId),
    /// The recipient's compartment, independent of the lender's title scope.
    Organization {
        organization: OrgId,
        scope: OrgScopeId,
    },
    CurrentLeader(MissionBinding),
}

/// A wildcard follows current effective control, never physical custody. An
/// exclusion remains effective through every child in this lineage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UnitScopeWire", into = "UnitScopeWire")]
pub enum UnitScope {
    Units {
        units: BTreeSet<ObjId>,
    },
    OrganizationControlled {
        organization: OrgId,
        scope: ScopeSelector,
        excluded: BTreeSet<ObjId>,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum UnitScopeWire {
    Units {
        units: Vec<ObjId>,
    },
    OrganizationControlled {
        organization: OrgId,
        #[serde(deserialize_with = "strict_scope_selector")]
        scope: ScopeSelector,
        excluded: Vec<ObjId>,
    },
}

fn strict_scope_selector<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<ScopeSelector, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    enum Wire {
        Exact(OrgScopeId),
        SubtreeFrozen {
            root: OrgScopeId,
            snapshot_revision: u64,
        },
        SubtreeLive(OrgScopeId),
    }
    Ok(match Wire::deserialize(deserializer)? {
        Wire::Exact(scope) => ScopeSelector::Exact(scope),
        Wire::SubtreeFrozen {
            root,
            snapshot_revision,
        } => ScopeSelector::SubtreeFrozen {
            root,
            snapshot_revision,
        },
        Wire::SubtreeLive(scope) => ScopeSelector::SubtreeLive(scope),
    })
}

fn canonical_units(units: Vec<ObjId>, empty: bool) -> Result<BTreeSet<ObjId>, UnitGrantError> {
    if units.len() > MAX_UNIT_SCOPE_ITEMS || (!empty && units.is_empty()) {
        return Err(UnitGrantError::Malformed("unit scope bound"));
    }
    if units.windows(2).any(|pair| match pair {
        [left, right] => left >= right,
        _ => false,
    }) {
        return Err(UnitGrantError::Malformed(
            "unit scope must be sorted and unique",
        ));
    }
    Ok(units.into_iter().collect())
}

impl TryFrom<UnitScopeWire> for UnitScope {
    type Error = UnitGrantError;

    fn try_from(wire: UnitScopeWire) -> Result<Self, Self::Error> {
        Ok(match wire {
            UnitScopeWire::Units { units } => Self::Units {
                units: canonical_units(units, false)?,
            },
            UnitScopeWire::OrganizationControlled {
                organization,
                scope,
                excluded,
            } => Self::OrganizationControlled {
                organization,
                scope,
                excluded: canonical_units(excluded, true)?,
            },
        })
    }
}

impl From<UnitScope> for UnitScopeWire {
    fn from(scope: UnitScope) -> Self {
        match scope {
            UnitScope::Units { units } => Self::Units {
                units: units.into_iter().collect(),
            },
            UnitScope::OrganizationControlled {
                organization,
                scope,
                excluded,
            } => Self::OrganizationControlled {
                organization,
                scope,
                excluded: excluded.into_iter().collect(),
            },
        }
    }
}

impl UnitScope {
    pub fn validate(&self) -> Result<(), UnitGrantError> {
        let (len, empty_allowed) = match self {
            Self::Units { units } => (units.len(), false),
            Self::OrganizationControlled { excluded, .. } => (excluded.len(), true),
        };
        if len > MAX_UNIT_SCOPE_ITEMS || (len == 0 && !empty_allowed) {
            return Err(UnitGrantError::Malformed("unit scope bound"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum UnitGrantParent {
    PersonalTitle {
        unit: ObjId,
        owner: ObjId,
        lineage: TitleLineage,
    },
    OrganizationGrant {
        organization: OrgId,
        grant: GrantId,
    },
    OrganizationRoleTemplate {
        organization: OrgId,
        role: RoleId,
        role_revision: u64,
        template_index: u16,
    },
    UnitGrant {
        id: UnitGrantId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitGrantStatus {
    Active,
    Revoked,
}

/// Consent which DT.3 may copy into an authenticated, actually acquired custody
/// permit. A grant record alone never supplies retained custody authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryPolicy {
    None,
    ReturnOnly,
}

/// A single capability and its exact source. The runtime supplies identities,
/// grantor and time; no field here is accepted as a cached authorization answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UnitGrantWire", into = "UnitGrantWire")]
pub struct UnitGrant {
    pub id: UnitGrantId,
    pub revision: u64,
    pub grantor: ObjId,
    pub grantee: UnitGrantee,
    pub capability: UnitCapability,
    pub scope: UnitScope,
    pub valid_from: Option<i64>,
    pub valid_until: Option<i64>,
    pub delegable: bool,
    pub recovery: RecoveryPolicy,
    pub parent: UnitGrantParent,
    pub status: UnitGrantStatus,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnitGrantWire {
    id: UnitGrantId,
    revision: u64,
    grantor: ObjId,
    grantee: UnitGrantee,
    capability: UnitCapability,
    scope: UnitScope,
    #[serde(deserialize_with = "required_option")]
    valid_from: Option<i64>,
    #[serde(deserialize_with = "required_option")]
    valid_until: Option<i64>,
    delegable: bool,
    recovery: RecoveryPolicy,
    parent: UnitGrantParent,
    status: UnitGrantStatus,
}

fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

impl UnitGrant {
    pub fn validate(&self) -> Result<(), UnitGrantError> {
        if self.id.0 == 0 || self.revision == 0 {
            return Err(UnitGrantError::Malformed("zero grant identity"));
        }
        self.scope.validate()?;
        if self
            .valid_from
            .zip(self.valid_until)
            .is_some_and(|(start, end)| start >= end)
        {
            return Err(UnitGrantError::Malformed("empty validity window"));
        }
        if let UnitGrantee::CurrentLeader(binding) = &self.grantee {
            binding.validate()?;
            if self.capability != UnitCapability::CommandAssignedAi || self.delegable {
                return Err(UnitGrantError::Malformed(
                    "mission role is nondelegable command only",
                ));
            }
            if !matches!(self.scope, UnitScope::Units { .. }) {
                return Err(UnitGrantError::Malformed(
                    "mission role requires finite accepted units",
                ));
            }
        }
        if matches!(self.parent, UnitGrantParent::UnitGrant { id } if id == self.id) {
            return Err(UnitGrantError::Cycle);
        }
        if matches!(
            self.parent,
            UnitGrantParent::OrganizationRoleTemplate {
                role_revision: 0,
                ..
            }
        ) {
            return Err(UnitGrantError::Malformed("zero role revision"));
        }
        Ok(())
    }
}

impl TryFrom<UnitGrantWire> for UnitGrant {
    type Error = UnitGrantError;
    fn try_from(w: UnitGrantWire) -> Result<Self, Self::Error> {
        let grant = Self {
            id: w.id,
            revision: w.revision,
            grantor: w.grantor,
            grantee: w.grantee,
            capability: w.capability,
            scope: w.scope,
            valid_from: w.valid_from,
            valid_until: w.valid_until,
            delegable: w.delegable,
            recovery: w.recovery,
            parent: w.parent,
            status: w.status,
        };
        grant.validate()?;
        Ok(grant)
    }
}

impl From<UnitGrant> for UnitGrantWire {
    fn from(g: UnitGrant) -> Self {
        Self {
            id: g.id,
            revision: g.revision,
            grantor: g.grantor,
            grantee: g.grantee,
            capability: g.capability,
            scope: g.scope,
            valid_from: g.valid_from,
            valid_until: g.valid_until,
            delegable: g.delegable,
            recovery: g.recovery,
            parent: g.parent,
            status: g.status,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionCommand {
    pub binding: MissionBinding,
    pub term: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitAuthRequest {
    pub actor: ObjId,
    pub unit: ObjId,
    pub action: UnitAction,
    pub now: i64,
    pub mission: Option<MissionCommand>,
}

/// Trusted current title projection; a stale economic doc is not evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitAssetFacts {
    pub unit: ObjId,
    pub owner: PrincipalId,
    pub lineage: TitleLineage,
    pub scope: Option<OrgScopeId>,
    pub active: bool,
}

/// Optional exact contractual control, already resolved by its owning module.
/// An assignment or lease marker alone must never produce this evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrgControlFacts {
    pub organization: OrgId,
    pub unit: ObjId,
    pub scope: OrgScopeId,
    pub capability: UnitCapability,
    pub delegable: bool,
    pub recovery: RecoveryPolicy,
    pub valid_from: Option<i64>,
    pub valid_until: Option<i64>,
    /// Stable contract/control identity and its current revision, for audit only.
    pub source: u64,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissionLeader {
    Player(ObjId),
    Npc(ObjId),
}

impl MissionLeader {
    #[must_use]
    pub fn character(self) -> ObjId {
        match self {
            Self::Player(actor) | Self::Npc(actor) => actor,
        }
    }
}

/// The mission owner has verified acceptance, human identity, embarkation and
/// eligibility. The resolver still checks the exact binding, unit and term.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionFacts {
    pub binding: MissionBinding,
    pub active: bool,
    pub units: BTreeSet<ObjId>,
    pub leader: Option<MissionLeader>,
    pub term: u64,
}

pub trait UnitAuthorityFacts {
    fn principal_exists(&self, actor: ObjId) -> bool;
    fn asset(&self, unit: ObjId) -> Option<UnitAssetFacts>;
    fn organization_control(
        &self,
        _organization: OrgId,
        _unit: ObjId,
        _capability: UnitCapability,
        _now: i64,
    ) -> Option<OrgControlFacts> {
        None
    }
    fn mission(&self, _binding: &MissionBinding) -> Option<MissionFacts> {
        None
    }
    /// Only an actual custody owner may attest this. No producer is installed
    /// by DT.2a; an old grant, seat claim or carrier location is insufficient.
    fn recovery_custody(&self, _unit: ObjId, _operator: ObjId) -> Option<RecoveryCustodyFacts> {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryAction {
    SafeStop,
    Disengage,
    ReturnToRecordedOrigin { origin: ObjId },
    RecoverIntoRecordedBerth { berth: ObjId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryCustodyFacts {
    pub unit: ObjId,
    pub operator: ObjId,
    pub hull_epoch: u64,
    pub origin: ObjId,
    pub berth: ObjId,
    pub policy: RecoveryPolicy,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryRequest {
    pub actor: ObjId,
    pub unit: ObjId,
    pub hull_epoch: u64,
    pub action: RecoveryAction,
}

/// An audit/staleness witness, never a capability token. Re-run live resolution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitGrantWitness {
    pub at: i64,
    pub sources: Vec<UnitAuthoritySource>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnitAuthoritySource {
    Title {
        unit: ObjId,
        lineage: TitleLineage,
    },
    Grant {
        id: UnitGrantId,
        revision: u64,
    },
    OrganizationGrant {
        grant: GrantId,
        revision: u64,
    },
    OrganizationRole {
        role: RoleId,
        revision: u64,
        template_index: u16,
    },
    OrganizationControl {
        organization: OrgId,
        source: u64,
        revision: u64,
    },
    Mission {
        binding: MissionBinding,
        term: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnitDecision {
    Allow(UnitGrantWitness),
    Deny(UnitGrantError),
}

impl UnitDecision {
    #[must_use]
    pub fn is_allow(&self) -> bool {
        matches!(self, Self::Allow(_))
    }
}

/// Server-side reasons. A public adapter must collapse existence/lineage details.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UnitGrantError {
    #[error("unit authority is unavailable")]
    NoAuthority,
    #[error("unit authority was explicitly excluded")]
    Excluded,
    #[error("unit authority is no longer active")]
    Inactive,
    #[error("unit authority source changed")]
    Stale,
    #[error("unit grant widens its source")]
    Widening,
    #[error("unit grant lineage contains a cycle")]
    Cycle,
    #[error("unit grant proof exceeds its bound")]
    Capacity,
    #[error("invalid unit grant: {0}")]
    Malformed(&'static str),
}
