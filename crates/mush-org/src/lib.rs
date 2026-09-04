//! `mush-org` (E1a) — the pure organization-identity + scoped-authorization
//! library the economy (E2+) owns accounts and vaults through.
//!
//! This crate is **world-free**: it defines the id vocabulary, the record model,
//! the authoritative [`OrgState`] snapshot, the pure [`authorize`] engine, and the
//! closed [`OrgMutationPlan`] validate-then-apply — all unit-tested on hand-built
//! state, exactly the shipped M31/M32 pure-library shape. The `OrgModule` that
//! attaches this to the world (persistence, boot, commands, the destroy scrub,
//! the wiring-time capability + predicate registries) is **E1b**.
//!
//! ## Design spine (reviewed)
//!
//! - **Authority is a pure projection re-derived from live records each call** —
//!   the F3 primary guard; a destroyed object grants nothing because its records
//!   are scrubbed ([`OrgState`] scrub), and per-record `revision` +
//!   [`ExpectedRevision`] are only *secondary* staleness guards.
//! - **Default-deny, explicit-deny-wins**, with the total resolution
//!   `authorized = (grant OR policy_allow) AND NOT policy_deny` ([`authorize`]).
//! - **Delegation fully re-resolves its live source** each call (never a
//!   `source_revision` equality — an expired source bumps no revision).
//! - **Disclosure-safe**: [`Decision::Deny`] carries a staff-safe reason; a hidden
//!   org/membership/standing is never leaked, and the caller-facing layer collapses
//!   "denied" with "nonexistent".
//! - **`PrincipalId::Object` is clean** — no embedded generation (ids are never
//!   recycled).

// E1a — the pure library.
mod authorize;
mod ids;
mod plan;
mod record;
mod state;

// E1b — the world-attached module (persistence, boot, commands, the destroy
// scrub, the frozen predicate table, and the public authorize/submit seam).
mod api;
mod boot;
mod commands;
mod module;
mod wiring;

/// The maximum org scope-tree depth (root = 0). Enforced at scope creation, so a
/// containment walk always terminates; a check that would exceed it fails closed.
pub const MAX_SCOPE_DEPTH: u8 = 8;

/// The maximum number of scopes one organization may hold (E3b2 §4.8.3, doc 08b
/// §9). Enforced at [`OrgMutationOp::CreateScope`]; a scope tree is walked per
/// authorization and rendered whole by `@org/show`, so its size is a cost input.
pub const MAX_SCOPES_PER_ORG: usize = 64;

pub use authorize::{
    AuthRequest, AuthWitness, Decision, DenyReason, NoPredicates, PredicateEval, authorize,
};
pub use ids::{
    CapabilityId, DelegationId, GrantId, OrgId, OrgScopeId, PolicyId, PrincipalId, RoleId,
    SystemPrincipalId,
};
pub use plan::{EntityKey, ExpectedRevision, OrgError, OrgMutationOp, OrgMutationPlan};
pub use record::{
    AccessPolicy, ApprovalPolicy, CapabilityGrant, Delegation, DelegationStatus, GrantLimit,
    GrantTemplate, MembershipStatus, OrgMembership, OrgRole, OrgScope, OrgStatus, Organization,
    PolicyCondition, PolicyEffect, PolicyRule, PolicySubject, RulesValue, ScopeSelector,
    ScopeStatus, Visibility,
};
pub use state::{OrgState, ScrubReport};

// E1b public surface.
pub use api::{
    AUDIT_LOG, META_KEY, NS, OrgAction, OrgAuditEntry, audit, authorize_in_world, delegation_key,
    grant_key, member_key, org_key, policy_key, role_key, scope_key, submit,
};
pub use module::{OrgModule, OrgStore};
pub use wiring::{
    CapabilityVocabulary, OrgPredicateProvider, ScopeUseProvider, ScopeUseProviders,
};
