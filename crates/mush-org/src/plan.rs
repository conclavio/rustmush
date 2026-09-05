//! The closed org-mutation alphabet + atomic validate-then-apply (E1a) — the M32
//! `UnitMutationPlan` shape: a plan is validated in full against a **clone**, and
//! either applies whole or refuses whole with zero mutation (refuse-not-clamp).
//!
//! IDs for created records are **pre-allocated by the caller** (the E1b module
//! owns the monotonic allocator); `apply` validates structure + staleness, never
//! mints ids. Revisions are the caller's to set (the module owns the bookkeeping);
//! `apply` enforces them only through the optimistic [`ExpectedRevision`] guard —
//! the async/deferred staleness check (a plan built against an old snapshot and
//! applied later is refused if the named entity moved).

use mush_core::ObjId;

use crate::ids::{DelegationId, GrantId, OrgId, OrgScopeId, PolicyId, RoleId};
use crate::record::{
    AccessPolicy, CapabilityGrant, Delegation, OrgMembership, OrgRole, OrgScope, Organization,
};
use crate::state::OrgState;
use crate::{MAX_SCOPE_DEPTH, MAX_SCOPES_PER_ORG};

/// A closed, typed org mutation. Creates carry pre-allocated ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrgMutationOp {
    /// Create an organization together with its root scope (an org must have one).
    CreateOrg {
        /// The new org (its `root_scope` must equal `root.id`).
        org: Organization,
        /// The org's root scope (`parent: None`, `depth: 0`).
        root: OrgScope,
    },
    /// Create a child scope under an existing parent.
    CreateScope {
        /// The new scope.
        scope: OrgScope,
    },
    /// Retire a non-root scope (E3b2): it stops conferring authority
    /// ([`DenyReason::Inactive`](crate::DenyReason)) and takes no new binding. Never
    /// a delete — existing records keep naming it so staff can resolve them. A
    /// **root** scope is refused: an org must always have a live compartment.
    RetireScope {
        /// The scope to retire.
        scope: OrgScopeId,
    },
    /// Create a role template.
    CreateRole {
        /// The new role.
        role: OrgRole,
    },
    /// **Set an organization's lifecycle state and/or its hidden flag** (E6c B10,
    /// DL-06 / 08g §3 #3, the minimal slice).
    ///
    /// `OrgStatus::{Suspended, Dissolving, Closed}` have existed since E1a and
    /// **nothing moved them**: closing was documented as a workflow with no op and no
    /// verb, so a retired vendor could not be retired and a suspended organization
    /// could not be suspended. This is that write, and no more of it: the full close
    /// workflow — resolving the accounts, assets and memberships a `Dissolving`
    /// organization still holds — stays E7's, as does the membership producer
    /// (`@org/member`).
    ///
    /// `hidden` is the same story from the other side. `Organization.hidden` has been
    /// read by the economy's existence-hiding renderer since E6a's D11 and **written
    /// by nothing in the shipped surface**, which is why that fold shipped
    /// preventively with no end-to-end test and E6b's scenario 12 had to reach into
    /// the store. This is its first write path.
    ///
    /// Both fields are optional so a staffer can move one without touching the other,
    /// and either move bumps the org's `revision` — the staleness stamp every
    /// [`ExpectedRevision`] guard reads.
    SetOrgStatus {
        /// Which organization.
        org: OrgId,
        /// The new lifecycle state, or `None` to leave it.
        status: Option<crate::record::OrgStatus>,
        /// Whether it hides its existence from non-members, or `None` to leave it.
        hidden: Option<bool>,
    },
    /// **Add or replace a role template** (E6c B10).
    ///
    /// [`OrgMutationOp::CreateRole`] refuses an id that already exists, which makes a
    /// role template immutable once written — so a role whose capability set was
    /// wrong could never be corrected, only abandoned. This is the upsert, and it
    /// bumps the owning organization's revision like every other mutation.
    PutRole {
        /// The role.
        role: OrgRole,
    },
    /// Add (or replace) a membership.
    PutMembership {
        /// The membership.
        membership: OrgMembership,
    },
    /// Remove a membership.
    RemoveMembership {
        /// The org.
        org: OrgId,
        /// The member.
        member: ObjId,
    },
    /// Add (or replace) a direct capability grant.
    PutGrant {
        /// The grant.
        grant: CapabilityGrant,
    },
    /// Revoke a direct grant.
    RevokeGrant {
        /// The grant id.
        grant: GrantId,
    },
    /// Create a one-hop delegation.
    Delegate {
        /// The delegation (must narrow its source).
        delegation: Delegation,
    },
    /// Revoke a delegation.
    RevokeDelegation {
        /// The delegation id.
        delegation: DelegationId,
    },
    /// Add (or replace) an access policy.
    PutPolicy {
        /// The policy.
        policy: AccessPolicy,
    },
}

/// An optimistic-concurrency expectation: entity `key` must currently be at
/// `revision`, else the plan is stale (the async/deferred-action guard).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedRevision {
    /// Which entity.
    pub key: EntityKey,
    /// Its expected current revision.
    pub revision: u64,
}

/// Names an entity for an [`ExpectedRevision`] check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntityKey {
    /// An org.
    Org(OrgId),
    /// A scope.
    Scope(OrgScopeId),
    /// A role.
    Role(RoleId),
    /// A grant.
    Grant(GrantId),
    /// A policy.
    Policy(PolicyId),
    /// A membership.
    Membership(OrgId, ObjId),
}

/// A validate-then-apply plan.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct OrgMutationPlan {
    /// Optimistic staleness expectations checked before any op.
    pub expected: Vec<ExpectedRevision>,
    /// The ops, applied in order (all-or-nothing).
    pub ops: Vec<OrgMutationOp>,
}

/// What can go wrong applying a plan. A failure leaves the input state untouched.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OrgError {
    /// An [`ExpectedRevision`] did not match current state (stale plan).
    #[error("stale plan: {0:?} expected revision {1}, found {2:?}")]
    Stale(EntityKey, u64, Option<u64>),
    /// A created id already exists.
    #[error("duplicate id: {0}")]
    DuplicateId(String),
    /// A referenced record does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// A structural invariant was violated.
    #[error("invalid: {0}")]
    Invalid(String),
    /// A scope would exceed the maximum tree depth.
    #[error("scope depth exceeds the maximum ({MAX_SCOPE_DEPTH})")]
    DepthExceeded,
    /// The organization already holds [`MAX_SCOPES_PER_ORG`] scopes.
    #[error("the organization already has the maximum number of scopes ({MAX_SCOPES_PER_ORG})")]
    TooManyScopes,
    /// A delegation is not a strict narrowing of its source grant.
    #[error("delegation does not narrow its source grant")]
    DelegationWiden,
}

impl OrgMutationPlan {
    /// A plan with the given ops and no staleness expectations.
    #[must_use]
    pub fn of(ops: Vec<OrgMutationOp>) -> OrgMutationPlan {
        OrgMutationPlan {
            expected: Vec::new(),
            ops,
        }
    }

    /// Validate + apply against a clone of `state`. Returns the new state on
    /// success, or the first error (leaving `state` conceptually untouched — the
    /// caller keeps its original).
    pub fn apply(&self, state: &OrgState) -> Result<OrgState, OrgError> {
        // Staleness first (against the pre-image).
        for e in &self.expected {
            let cur = current_revision(state, &e.key);
            if cur != Some(e.revision) {
                return Err(OrgError::Stale(e.key.clone(), e.revision, cur));
            }
        }
        let mut next = state.clone();
        for op in &self.ops {
            apply_one(&mut next, op)?;
        }
        Ok(next)
    }
}

fn current_revision(state: &OrgState, key: &EntityKey) -> Option<u64> {
    match key {
        EntityKey::Org(id) => state.org(*id).map(|o| o.revision),
        EntityKey::Scope(id) => state.scope(*id).map(|s| s.revision),
        EntityKey::Role(id) => state.role(*id).map(|r| r.revision),
        EntityKey::Grant(id) => state.grant(*id).map(|g| g.revision),
        EntityKey::Policy(id) => state.policy(*id).map(|p| p.revision),
        EntityKey::Membership(org, m) => state.membership(*org, *m).map(|mm| mm.revision),
    }
}

fn apply_one(state: &mut OrgState, op: &OrgMutationOp) -> Result<(), OrgError> {
    match op {
        OrgMutationOp::CreateOrg { org, root } => {
            if state.org(org.id).is_some() {
                return Err(OrgError::DuplicateId(format!("org {}", org.id)));
            }
            if state.scope(root.id).is_some() {
                return Err(OrgError::DuplicateId(format!("scope {}", root.id)));
            }
            if org.root_scope != root.id
                || root.org != org.id
                || root.parent.is_some()
                || root.depth != 0
            {
                return Err(OrgError::Invalid("org/root-scope mismatch".into()));
            }
            state.put_org(org.clone());
            state.put_scope(root.clone());
        }
        OrgMutationOp::CreateScope { scope } => {
            if state.scope(scope.id).is_some() {
                return Err(OrgError::DuplicateId(format!("scope {}", scope.id)));
            }
            if state.org(scope.org).is_none() {
                return Err(OrgError::NotFound(format!("org {}", scope.org)));
            }
            let parent_id = scope
                .parent
                .ok_or_else(|| OrgError::Invalid("non-root scope needs a parent".into()))?;
            let parent = state
                .scope(parent_id)
                .ok_or_else(|| OrgError::NotFound(format!("parent scope {parent_id}")))?;
            if parent.org != scope.org {
                return Err(OrgError::Invalid("scope parent in a different org".into()));
            }
            if scope.depth != parent.depth + 1 {
                return Err(OrgError::Invalid(
                    "scope depth must be parent depth + 1".into(),
                ));
            }
            if scope.depth > MAX_SCOPE_DEPTH {
                return Err(OrgError::DepthExceeded);
            }
            // Boundedness (E3b2 §4.8.3): a scope tree is walked per authorization and
            // listed whole by `@org/show`, so its size is a real cost input.
            if state.scopes_in(scope.org).count() >= MAX_SCOPES_PER_ORG {
                return Err(OrgError::TooManyScopes);
            }
            state.put_scope(scope.clone());
        }
        OrgMutationOp::RetireScope { scope } => {
            let mut s = state
                .scope(*scope)
                .cloned()
                .ok_or_else(|| OrgError::NotFound(format!("scope {scope}")))?;
            if s.parent.is_none() {
                return Err(OrgError::Invalid("a root scope cannot be retired".into()));
            }
            s.status = crate::ScopeStatus::Retired;
            s.revision += 1;
            state.put_scope(s);
        }
        OrgMutationOp::CreateRole { role } => {
            if state.role(role.id).is_some() {
                return Err(OrgError::DuplicateId(format!("role {}", role.id)));
            }
            if state.org(role.org).is_none() {
                return Err(OrgError::NotFound(format!("org {}", role.org)));
            }
            state.put_role(role.clone());
        }
        OrgMutationOp::SetOrgStatus {
            org,
            status,
            hidden,
        } => {
            let mut o = state
                .org(*org)
                .cloned()
                .ok_or_else(|| OrgError::NotFound(format!("org {org}")))?;
            // `Closed` is terminal and the id is retired: an organization that has
            // closed does not re-open, and a re-opened one would hand back authority
            // every grantee was told had ended.
            if o.status == crate::record::OrgStatus::Closed {
                return Err(OrgError::Invalid(
                    "a closed organization is terminal".into(),
                ));
            }
            if let Some(s) = status {
                o.status = *s;
            }
            if let Some(h) = hidden {
                o.hidden = *h;
            }
            o.revision = o.revision.saturating_add(1);
            state.put_org(o);
        }
        OrgMutationOp::PutRole { role } => {
            if state.org(role.org).is_none() {
                return Err(OrgError::NotFound(format!("org {}", role.org)));
            }
            if let Some(existing) = state.role(role.id)
                && existing.org != role.org
            {
                return Err(OrgError::Invalid(
                    "a role cannot change organization".into(),
                ));
            }
            state.put_role(role.clone());
            if let Some(mut o) = state.org(role.org).cloned() {
                o.revision = o.revision.saturating_add(1);
                state.put_org(o);
            }
        }
        OrgMutationOp::PutMembership { membership } => {
            if state.org(membership.org).is_none() {
                return Err(OrgError::NotFound(format!("org {}", membership.org)));
            }
            for role in &membership.roles {
                let r = state
                    .role(*role)
                    .ok_or_else(|| OrgError::NotFound(format!("role {role}")))?;
                if r.org != membership.org {
                    return Err(OrgError::Invalid("membership role from another org".into()));
                }
            }
            state.put_membership(membership.clone());
        }
        OrgMutationOp::RemoveMembership { org, member } => {
            if state.membership(*org, *member).is_none() {
                return Err(OrgError::NotFound(format!("membership {org}/{member}")));
            }
            state.remove_membership(*org, *member);
        }
        OrgMutationOp::PutGrant { grant } => {
            if state.org(grant.org).is_none() {
                return Err(OrgError::NotFound(format!("org {}", grant.org)));
            }
            let root = grant.scope.root();
            let sc = state
                .scope(root)
                .ok_or_else(|| OrgError::NotFound(format!("scope {root}")))?;
            if sc.org != grant.org {
                return Err(OrgError::Invalid("grant scope in another org".into()));
            }
            state.put_grant(grant.clone());
        }
        OrgMutationOp::RevokeGrant { grant } => {
            if state.grant(*grant).is_none() {
                return Err(OrgError::NotFound(format!("grant {grant}")));
            }
            state.remove_grant(*grant);
        }
        OrgMutationOp::Delegate { delegation } => {
            if state.delegation(delegation.id).is_some() {
                return Err(OrgError::DuplicateId(format!(
                    "delegation {}",
                    delegation.id
                )));
            }
            let src = state.grant(delegation.source_grant).ok_or_else(|| {
                OrgError::NotFound(format!("source grant {}", delegation.source_grant))
            })?;
            if !src.delegable {
                return Err(OrgError::Invalid("source grant is not delegable".into()));
            }
            if src.grantee != delegation.grantor {
                // Only the actual holder of the source grant may delegate it —
                // otherwise delegated authority is forgeable from any delegable
                // grant regardless of who holds it (authorize re-checks this live).
                return Err(OrgError::Invalid(
                    "grantor does not hold the source grant".into(),
                ));
            }
            if src.org != delegation.org || src.capability != delegation.capability {
                return Err(OrgError::Invalid(
                    "delegation does not match its source".into(),
                ));
            }
            if !crate::authorize::selector_subsumes(state, &src.scope, &delegation.scope) {
                return Err(OrgError::DelegationWiden);
            }
            if !limit_narrows(src.limit, delegation.limit) {
                return Err(OrgError::DelegationWiden);
            }
            state.put_delegation(delegation.clone());
        }
        OrgMutationOp::RevokeDelegation { delegation } => {
            // Revoke by marking the record Revoked (kept for audit); if absent, error.
            let mut d = state
                .delegation(*delegation)
                .cloned()
                .ok_or_else(|| OrgError::NotFound(format!("delegation {delegation}")))?;
            d.status = crate::DelegationStatus::Revoked;
            state.put_delegation(d);
        }
        OrgMutationOp::PutPolicy { policy } => {
            if state.org(policy.org).is_none() {
                return Err(OrgError::NotFound(format!("org {}", policy.org)));
            }
            state.put_policy(policy.clone());
        }
    }
    Ok(())
}

/// A delegation's limit must be no wider than its source's.
fn limit_narrows(
    src: Option<crate::record::GrantLimit>,
    del: Option<crate::record::GrantLimit>,
) -> bool {
    match (
        src.and_then(|l| l.max_amount),
        del.and_then(|l| l.max_amount),
    ) {
        (None, _) => true,        // unlimited source admits any delegated limit
        (Some(_), None) => false, // a limited source cannot delegate an unlimited grant
        (Some(s), Some(d)) => d <= s,
    }
}
