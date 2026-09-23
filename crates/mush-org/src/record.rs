//! The `mush-org` records (E1a) — the authoritative data model the authorization
//! engine reads and the mutation plan writes. All persistence + boot is E1b; these
//! are plain serializable structs.

use std::collections::BTreeSet;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::ids::{
    CapabilityId, DelegationId, GrantId, OrgId, OrgScopeId, PolicyId, PrincipalId, RoleId,
};

/// An opaque, schema-tagged ruleset payload — the shipped `RulesValue` pattern
/// (each module defines its own; there is no shared core type). A ruleset can
/// version + migrate its own `data` without `mush-org` interpreting it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulesValue {
    /// The ruleset's schema name for `data`.
    pub schema: SmolStr,
    /// The schema version.
    pub version: u32,
    /// The opaque payload.
    pub data: serde_json::Value,
}

/// An organization — a first-class owning entity, distinct from a faction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Organization {
    /// Identity (never reused).
    pub id: OrgId,
    /// Display name.
    pub name: SmolStr,
    /// Alternate names (case-insensitive lookup, authored).
    pub aliases: Vec<SmolStr>,
    /// Lifecycle state (closing is a workflow, never a delete).
    pub status: OrgStatus,
    /// A hidden org must not leak its existence to non-members (§12.2).
    pub hidden: bool,
    /// Free-form charter/description.
    pub charter: String,
    /// The root of this org's scope tree.
    pub root_scope: OrgScopeId,
    /// Bumped on every mutation; the optimistic-concurrency + staleness stamp.
    pub revision: u64,
    /// Opaque ruleset extension.
    pub ruleset_ext: RulesValue,
}

/// Organization lifecycle.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrgStatus {
    /// Normal.
    Active,
    /// Temporarily suspended (no new authority granted).
    Suspended,
    /// A close workflow is in progress (resolving accounts/assets/memberships).
    Dissolving,
    /// Closed; the id is retired, never reused.
    Closed,
}

/// A named node in an org's scope tree — what accounts, vaults, and grants bind
/// to. Never a hard-coded "wallet division"; the tree is authored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgScope {
    /// Identity.
    pub id: OrgScopeId,
    /// Owning org.
    pub org: OrgId,
    /// Parent scope (`None` for the org's root scope). Depth is capped at
    /// creation ([`crate::MAX_SCOPE_DEPTH`]) so containment walks terminate.
    pub parent: Option<OrgScopeId>,
    /// Authored name.
    pub name: SmolStr,
    /// Lifecycle.
    pub status: ScopeStatus,
    /// Depth from the root (root = 0); enforced ≤ [`crate::MAX_SCOPE_DEPTH`].
    pub depth: u8,
    /// The owning org's `revision` when this scope was created — the creation
    /// stamp a [`ScopeSelector::SubtreeFrozen`] compares against so a scope added
    /// after the snapshot is NOT silently included.
    pub created_rev: u64,
    /// Staleness stamp.
    pub revision: u64,
}

/// Scope lifecycle.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScopeStatus {
    /// Normal.
    Active,
    /// Retired (no new binding); existing bindings resolve/transfer on close.
    Retired,
}

/// Ground-truth membership of an object (character/NPC) in an org. Titles are
/// presentation; roles are templates, not final authority; the level scalar and
/// its thresholds stay ruleset/faction-side (E1c keeps faction's `level`, this
/// carries only the org-membership facts).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgMembership {
    /// Owning org.
    pub org: OrgId,
    /// The member object.
    pub member: ObjId,
    /// Lifecycle.
    pub status: MembershipStatus,
    /// Cosmetic titles (never authority).
    pub titles: Vec<SmolStr>,
    /// Role templates assigned to this member.
    pub roles: BTreeSet<RoleId>,
    /// Who may see this membership.
    pub visibility: Visibility,
    /// Unix seconds since joining.
    pub since: i64,
    /// Staleness stamp.
    pub revision: u64,
}

/// Membership lifecycle.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MembershipStatus {
    /// Active member.
    Active,
    /// Suspended (retains the record; grants via roles do not resolve).
    Suspended,
}

/// Who may observe a membership (the disclosure gate; enforced at render, and the
/// engine never leaks a hidden membership to an unauthorized caller).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    /// Anyone who can see the org.
    Public,
    /// Only fellow members.
    MembersKnow,
    /// Only staff/authorized auditors — a covert membership.
    Hidden,
}

/// A reusable role template: a bundle of grant templates. The NAME ("Treasurer",
/// "Quartermaster") is ruleset-facing; the engine resolves the grants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgRole {
    /// Identity.
    pub id: RoleId,
    /// Owning org.
    pub org: OrgId,
    /// Ruleset-facing name.
    pub name: SmolStr,
    /// The grant templates this role confers on its holders.
    pub grants: Vec<GrantTemplate>,
    /// Staleness stamp.
    pub revision: u64,
}

/// A capability + scope + optional limit a role confers — no grantee (the grantee
/// is any member holding the role). Resolved lazily at [`crate::authorize()`] time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantTemplate {
    /// The capability conferred.
    pub capability: CapabilityId,
    /// Where it applies.
    pub scope: ScopeSelector,
    /// Optional amount/operation cap.
    pub limit: Option<GrantLimit>,
    /// Whether a holder may delegate this (one hop).
    pub delegable: bool,
}

/// A direct capability grant to a specific object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityGrant {
    /// Identity.
    pub id: GrantId,
    /// Owning org.
    pub org: OrgId,
    /// The object empowered.
    pub grantee: ObjId,
    /// The capability.
    pub capability: CapabilityId,
    /// Where it applies.
    pub scope: ScopeSelector,
    /// Optional amount/operation cap.
    pub limit: Option<GrantLimit>,
    /// Whether the grantee may delegate this (one hop).
    pub delegable: bool,
    /// Not valid before this unix time, if set.
    pub valid_from: Option<i64>,
    /// Not valid at/after this unix time, if set (an expiry bumps NO revision).
    pub valid_until: Option<i64>,
    /// Staleness stamp.
    pub revision: u64,
}

/// An amount/operation limit on a grant. In E1 the request carries an optional
/// amount and a grant with a limit but a request with **no** amount is DENIED
/// (fail-closed); the real spend accounting lands with the economy (E2).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantLimit {
    /// Maximum per-operation amount in minor units, if capped.
    pub max_amount: Option<i64>,
}

/// A one-hop delegation of a source grant, narrowed. It cannot outlive or widen
/// its source; the engine re-resolves the LIVE source each authorization (a
/// captured `source_revision` alone is insufficient — an expired source bumps no
/// revision).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delegation {
    /// Identity.
    pub id: DelegationId,
    /// Owning org.
    pub org: OrgId,
    /// Who delegated.
    pub grantor: ObjId,
    /// Who received.
    pub grantee: ObjId,
    /// The source grant this narrows.
    pub source_grant: GrantId,
    /// The capability (must equal the source's).
    pub capability: CapabilityId,
    /// The (narrowed) scope.
    pub scope: ScopeSelector,
    /// The (narrowed) limit.
    pub limit: Option<GrantLimit>,
    /// Delegation expiry.
    pub expires_at: i64,
    /// The source grant's revision at delegation time (a fast staleness hint).
    pub source_revision: u64,
    /// Lifecycle.
    pub status: DelegationStatus,
}

/// Delegation lifecycle.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DelegationStatus {
    /// Live.
    Active,
    /// Explicitly revoked.
    Revoked,
}

/// A resource access policy owned by a principal (an account/vault/asset ACL, or
/// an org-wide rule). Explicit entries + denies take precedence over broad rules.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessPolicy {
    /// Identity.
    pub id: PolicyId,
    /// Owning org this policy is scoped under (for boot freeze + audit).
    pub org: OrgId,
    /// The principal that owns the policy.
    pub owner: PrincipalId,
    /// Ordered rules (a matching Deny always wins over any Allow).
    pub rules: Vec<PolicyRule>,
    /// The effect when no rule matches (default-deny at the engine level regardless).
    pub default: PolicyEffect,
    /// Staleness stamp.
    pub revision: u64,
}

/// One policy rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRule {
    /// Allow or Deny.
    pub effect: PolicyEffect,
    /// Who it matches.
    pub subject: PolicySubject,
    /// The capability it governs.
    pub capability: CapabilityId,
    /// Where it applies.
    pub scope: ScopeSelector,
    /// Extra gates (time window, …).
    pub conditions: Vec<PolicyCondition>,
}

/// Who a policy rule matches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicySubject {
    /// A specific principal.
    Principal(PrincipalId),
    /// Any member of an org.
    OrganizationMember(OrgId),
    /// A member holding a specific role.
    OrganizationRole {
        /// The org.
        org: OrgId,
        /// The role.
        role: RoleId,
    },
    /// The whole org (as a principal).
    Organization(OrgId),
    /// A registered predicate resolved by an external provider (e.g. faction
    /// standing). Returns only allow/deny — never the underlying hidden value.
    RegisteredPredicate {
        /// The provider name (e.g. `"faction"`).
        provider: SmolStr,
        /// The predicate name.
        predicate: SmolStr,
        /// Opaque provider arguments.
        arguments: serde_json::Value,
    },
}

/// Allow or deny.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyEffect {
    /// Permit.
    Allow,
    /// Refuse (always wins over any Allow).
    Deny,
}

/// An extra condition on a policy rule.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyCondition {
    /// The rule applies only within `[from, until)` unix seconds (either bound
    /// optional).
    TimeWindow {
        /// Not before, if set.
        from: Option<i64>,
        /// Not at/after, if set.
        until: Option<i64>,
    },
}

/// A multi-approval policy for high-value actions. **Type only in E1** — the
/// approval-collection workflow (binding approvals to an immutable plan hash) is
/// E2, when there are economy plans to approve.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalPolicy {
    /// One authorized actor suffices.
    OneAuthorizedActor,
    /// N approvals, optionally restricted to a role.
    Threshold {
        /// Required approvals.
        approvals: u8,
        /// Restrict approvers to this role, if set.
        from_role: Option<RoleId>,
    },
    /// N approvals from a named set.
    NamedSet {
        /// Required approvals.
        approvals: u8,
        /// The eligible actors.
        actors: BTreeSet<ObjId>,
    },
}

/// How a grant/policy/delegation names the scope(s) it applies to. A closed enum
/// so scope-isolation is auditable (R-B1): `SubtreeLive`'s expansion to
/// newly-created children is an explicit, opt-in property of THAT variant, so
/// `Exact`/`SubtreeFrozen` never silently widen when a child scope is added.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScopeSelector {
    /// Exactly one scope.
    Exact(OrgScopeId),
    /// The subtree rooted at `root` as it existed at `snapshot_revision` — a
    /// scope created after that revision is NOT included (no silent widening).
    SubtreeFrozen {
        /// The subtree root.
        root: OrgScopeId,
        /// The org revision the subtree membership is frozen at.
        snapshot_revision: u64,
    },
    /// The CURRENT subtree rooted at `root`, including scopes created later. Its
    /// widening is the explicit, admin-visible semantics of this variant.
    SubtreeLive(OrgScopeId),
}

impl ScopeSelector {
    /// The root scope this selector is anchored at.
    #[must_use]
    pub fn root(&self) -> OrgScopeId {
        match self {
            ScopeSelector::Exact(s)
            | ScopeSelector::SubtreeFrozen { root: s, .. }
            | ScopeSelector::SubtreeLive(s) => *s,
        }
    }
}
