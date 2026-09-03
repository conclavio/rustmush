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

use crate::MAX_SCOPE_DEPTH;
use crate::ids::{DelegationId, GrantId, OrgId, OrgScopeId, PolicyId, RoleId};
use crate::record::{
    AccessPolicy, CapabilityGrant, Delegation, OrgMembership, OrgRole, OrgScope, Organization,
};
use crate::state::OrgState;

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
    /// Create a role template.
    CreateRole {
        /// The new role.
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
    /// A delegation is not a strict narrowing of its source grant.
    #[error("delegation does not narrow its source grant")]
    DelegationWiden,
}

impl OrgMutationPlan {
    /// A plan with the given ops and no staleness expectations.
    #[must_use]
    pub fn of(ops: Vec<OrgMutationOp>) -> OrgMutationPlan {
        OrgMutationPlan { expected: Vec::new(), ops }
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
                return Err(OrgError::Invalid("scope depth must be parent depth + 1".into()));
            }
            if scope.depth > MAX_SCOPE_DEPTH {
                return Err(OrgError::DepthExceeded);
            }
            state.put_scope(scope.clone());
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
                return Err(OrgError::DuplicateId(format!("delegation {}", delegation.id)));
            }
            let src = state
                .grant(delegation.source_grant)
                .ok_or_else(|| OrgError::NotFound(format!("source grant {}", delegation.source_grant)))?;
            if !src.delegable {
                return Err(OrgError::Invalid("source grant is not delegable".into()));
            }
            if src.grantee != delegation.grantor {
                // Only the actual holder of the source grant may delegate it —
                // otherwise delegated authority is forgeable from any delegable
                // grant regardless of who holds it (authorize re-checks this live).
                return Err(OrgError::Invalid("grantor does not hold the source grant".into()));
            }
            if src.org != delegation.org || src.capability != delegation.capability {
                return Err(OrgError::Invalid("delegation does not match its source".into()));
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
fn limit_narrows(src: Option<crate::record::GrantLimit>, del: Option<crate::record::GrantLimit>) -> bool {
    match (src.and_then(|l| l.max_amount), del.and_then(|l| l.max_amount)) {
        (None, _) => true,                    // unlimited source admits any delegated limit
        (Some(_), None) => false,             // a limited source cannot delegate an unlimited grant
        (Some(s), Some(d)) => d <= s,
    }
}
