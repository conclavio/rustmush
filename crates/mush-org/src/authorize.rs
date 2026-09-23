//! The pure authorization engine (E1a).
//!
//! [`authorize`] answers one question — *may this actor exercise this capability
//! on this scope, right now?* — over an [`OrgState`] snapshot, with an injected
//! [`PredicateEval`] for cross-module policy subjects (faction standing, …). It is
//! pure and world-free (the M32 `unit::apply` shape), unit-tested on hand-built
//! state.
//!
//! ## The resolution (R-D2, default-deny)
//!
//! ```text
//! authorized = ( grant_stage_match  OR  policy_allow )  AND  NOT policy_deny
//! ```
//!
//! - **grant stage**: direct grants ∪ grants via the actor's active-membership
//!   roles ∪ one-hop delegations — each checked for capability, scope containment
//!   (depth-capped), validity window, and limit;
//! - **policy stage**: [`crate::AccessPolicy`] rules whose subject matches the actor;
//! - a matching **Deny** always wins; an unresolvable registered predicate
//!   **suppresses an Allow and satisfies a Deny** (fail-closed).
//!
//! ## Live re-resolution (R-B2)
//!
//! A delegation is authorized only by FULLY re-resolving its live source grant
//! each call — existence, current validity window, still-`delegable`, capability
//! match, and scope containment in the source's CURRENT scope — never a
//! `source_revision` equality (an expired source bumps no revision).
//!
//! ## Disclosure (R-R3)
//!
//! [`Decision::Deny`] carries a staff-safe [`DenyReason`]; it never reveals a
//! hidden org/membership/standing. The caller-facing renderer (E1b) collapses
//! "denied" and "nonexistent" so an unauthorized actor cannot tell them apart.
//! [`AuthWitness`] is server-internal — returned for the deferred-action staleness
//! re-check, never handed to the actor.

use mush_core::ObjId;
use smol_str::SmolStr;

use crate::MAX_SCOPE_DEPTH;
use crate::ids::{CapabilityId, OrgId, OrgScopeId};
use crate::record::{
    Delegation, GrantLimit, GrantTemplate, PolicyCondition, PolicyEffect, PolicySubject,
    ScopeSelector,
};
use crate::state::OrgState;

/// One authorization request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthRequest {
    /// The acting object.
    pub actor: ObjId,
    /// The capability sought.
    pub capability: CapabilityId,
    /// The concrete target scope (its org is derived from the scope).
    pub scope: OrgScopeId,
    /// The current unix time (for validity windows / conditions).
    pub now: i64,
    /// The operation amount in minor units, if the action carries one. A grant
    /// that carries a [`GrantLimit`] authorizes only when an amount is present and
    /// within the limit (fail-closed).
    pub amount: Option<i64>,
}

/// The engine's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Permitted; carries the server-internal witness (never shown to the actor).
    Allow(AuthWitness),
    /// Refused; carries a staff-safe reason.
    Deny(DenyReason),
}

impl Decision {
    /// Whether the decision permits the action.
    #[must_use]
    pub fn is_allow(&self) -> bool {
        matches!(self, Decision::Allow(_))
    }
}

/// A staff-safe denial reason. The caller-facing layer (E1b) MUST collapse the
/// existence-revealing variants (`NoSuchScope`, `Inactive`) with `NoAuthority` for
/// an unauthorized actor, so "denied" and "nonexistent" are indistinguishable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DenyReason {
    /// No grant or policy allowed the action (the default-deny outcome).
    NoAuthority,
    /// A Deny policy rule matched.
    ExplicitDeny,
    /// A grant would authorize but the action carries no amount to check against
    /// its limit.
    AmountRequired,
    /// The action amount exceeds a matching grant's limit.
    LimitExceeded,
    /// The target scope does not exist. **Collapse with `NoAuthority` publicly.**
    NoSuchScope,
    /// The target scope is `Retired`, or its organization is not `Active` — an
    /// inert compartment grants nothing, however good the grant is (E3b2 §4.8.1).
    /// **Collapse with `NoAuthority` publicly** (it reveals org lifecycle).
    Inactive,
}

/// The server-internal record of an allow decision: the revisions that
/// contributed, for the E2 deferred-action staleness hint (the authoritative
/// re-check re-runs `authorize` live — R-D4). Never returned to the actor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthWitness {
    /// When the decision was made (unix seconds).
    pub at: i64,
    /// `(record-tag, revision)` pairs the decision depended on — a fast "did
    /// anything change" hint, not the whole authorization check.
    pub source_revisions: Vec<(SmolStr, u64)>,
}

/// Resolves cross-module policy predicates (e.g. faction standing) without
/// `mush-org` depending on the provider. Frozen at wiring time (E1b).
pub trait PredicateEval {
    /// Does `actor` satisfy the predicate? `Some(true)`/`Some(false)` when the
    /// provider resolved it; `None` when the provider/predicate is unregistered or
    /// unresolvable — the engine then **suppresses an Allow and satisfies a Deny**.
    /// The provider returns only a boolean + never leaks the underlying value.
    fn eval(
        &self,
        provider: &str,
        predicate: &str,
        arguments: &serde_json::Value,
        actor: ObjId,
    ) -> Option<bool>;
}

/// A [`PredicateEval`] that resolves nothing — every registered predicate is
/// unresolvable (Allow suppressed, Deny satisfied). Useful for tests + a default.
pub struct NoPredicates;
impl PredicateEval for NoPredicates {
    fn eval(&self, _: &str, _: &str, _: &serde_json::Value, _: ObjId) -> Option<bool> {
        None
    }
}

/// Authorize (or refuse) one request against a state snapshot. Pure + total.
#[must_use]
pub fn authorize(state: &OrgState, req: &AuthRequest, predicates: &dyn PredicateEval) -> Decision {
    let Some(target) = state.scope(req.scope) else {
        return Decision::Deny(DenyReason::NoSuchScope);
    };
    let org = target.org;
    // E3b2 §4.8.1: a retired compartment, or one under an organization that is not
    // `Active` (suspended / dissolving / closed / — fail-closed — missing), confers
    // NO authority. Both statuses were inert before this cut, so a grant on a
    // wound-down org still authorized spending; this is the whole-org kill switch
    // the economy's status rules (§4.4) lean on. Checked before any grant/policy
    // work, so it cannot be out-voted by a policy Allow.
    let org_active = state
        .org(org)
        .is_some_and(|o| o.status == crate::OrgStatus::Active);
    if !org_active || target.status != crate::ScopeStatus::Active {
        return Decision::Deny(DenyReason::Inactive);
    }
    let mut revs: Vec<(SmolStr, u64)> = Vec::new();
    // The most-specific denial reason seen while failing grants (amount/limit),
    // used only when no grant/policy authorizes and no explicit deny fired.
    let mut near: Option<DenyReason> = None;

    // ---- grant stage --------------------------------------------------------
    let mut grant_ok = false;

    // Direct grants to the actor.
    for g in state.grants_to(req.actor) {
        if g.org != org || g.capability != req.capability {
            continue;
        }
        match grant_authorizes(state, &g.scope, g.valid_from, g.valid_until, g.limit, req) {
            GrantCheck::Ok => {
                grant_ok = true;
                revs.push((SmolStr::new("grant"), g.revision));
            }
            GrantCheck::Fail(reason) => near = tighten(near, reason),
            GrantCheck::NoMatch => {}
        }
    }

    // Grants via the actor's active-membership roles.
    for role_id in state.active_roles(org, req.actor) {
        let Some(role) = state.role(*role_id) else {
            continue;
        };
        for gt in &role.grants {
            if gt.capability != req.capability {
                continue;
            }
            match template_authorizes(state, gt, req) {
                GrantCheck::Ok => {
                    grant_ok = true;
                    revs.push((SmolStr::new("role"), role.revision));
                }
                GrantCheck::Fail(reason) => near = tighten(near, reason),
                GrantCheck::NoMatch => {}
            }
        }
    }

    // One-hop delegations to the actor — FULLY re-resolve the live source (R-B2).
    for d in state.delegations_to(req.actor) {
        if let Some(rev) = delegation_authorizes(state, d, org, req) {
            grant_ok = true;
            revs.push((SmolStr::new("delegation"), rev));
        }
    }

    // ---- policy stage -------------------------------------------------------
    let mut policy_allow = false;
    let mut policy_deny = false;
    for p in state.policies_of(org) {
        for rule in &p.rules {
            if rule.capability != req.capability {
                continue;
            }
            if !scope_contains(state, &rule.scope, req.scope) {
                continue;
            }
            if !conditions_pass(&rule.conditions, req.now) {
                continue;
            }
            let matches = match &rule.subject {
                PolicySubject::Principal(crate::PrincipalId::Object(o)) => *o == req.actor,
                PolicySubject::Principal(_) => false, // org/system principals never act directly
                // An org "as a principal" is an OrgId; `req.actor` is always an
                // ObjId, so an org-subject rule can never match a real actor. It
                // must fail closed (never `*o == org`, which authorizes everyone) —
                // "any member of org o" is `OrganizationMember`, actor-gated below.
                PolicySubject::Organization(_) => false,
                PolicySubject::OrganizationMember(o) => {
                    *o == org
                        && state
                            .membership(org, req.actor)
                            .is_some_and(|m| m.status == crate::MembershipStatus::Active)
                }
                PolicySubject::OrganizationRole { org: o, role } => {
                    *o == org && state.active_roles(org, req.actor).contains(role)
                }
                PolicySubject::RegisteredPredicate {
                    provider,
                    predicate,
                    arguments,
                } => {
                    match predicates.eval(provider, predicate, arguments, req.actor) {
                        Some(b) => b,
                        // Unresolvable: suppress an Allow, satisfy a Deny.
                        None => matches!(rule.effect, PolicyEffect::Deny),
                    }
                }
            };
            if !matches {
                continue;
            }
            match rule.effect {
                PolicyEffect::Allow => {
                    policy_allow = true;
                    revs.push((SmolStr::new("policy"), p.revision));
                }
                PolicyEffect::Deny => policy_deny = true,
            }
        }
    }

    // ---- resolve (R-D2) -----------------------------------------------------
    if policy_deny {
        return Decision::Deny(DenyReason::ExplicitDeny);
    }
    if grant_ok || policy_allow {
        return Decision::Allow(AuthWitness {
            at: req.now,
            source_revisions: revs,
        });
    }
    Decision::Deny(near.unwrap_or(DenyReason::NoAuthority))
}

/// The result of checking whether one grant/template authorizes a request.
enum GrantCheck {
    /// Authorizes.
    Ok,
    /// Matched the capability but a gate failed for the given reason.
    Fail(DenyReason),
    /// Did not apply (scope/time did not match at all).
    NoMatch,
}

fn grant_authorizes(
    state: &OrgState,
    scope: &ScopeSelector,
    valid_from: Option<i64>,
    valid_until: Option<i64>,
    limit: Option<GrantLimit>,
    req: &AuthRequest,
) -> GrantCheck {
    if !time_valid(valid_from, valid_until, req.now) {
        return GrantCheck::NoMatch;
    }
    if !scope_contains(state, scope, req.scope) {
        return GrantCheck::NoMatch;
    }
    limit_ok(limit, req)
}

fn template_authorizes(state: &OrgState, gt: &GrantTemplate, req: &AuthRequest) -> GrantCheck {
    if !scope_contains(state, &gt.scope, req.scope) {
        return GrantCheck::NoMatch;
    }
    limit_ok(gt.limit, req)
}

/// A grant with a limit authorizes only when the request carries an amount within
/// it (fail-closed): no amount ⇒ [`DenyReason::AmountRequired`], over ⇒
/// [`DenyReason::LimitExceeded`].
fn limit_ok(limit: Option<GrantLimit>, req: &AuthRequest) -> GrantCheck {
    match (limit.and_then(|l| l.max_amount), req.amount) {
        (None, _) => GrantCheck::Ok, // unlimited grant
        (Some(_), None) => GrantCheck::Fail(DenyReason::AmountRequired),
        (Some(max), Some(a)) if a <= max => GrantCheck::Ok,
        (Some(_), Some(_)) => GrantCheck::Fail(DenyReason::LimitExceeded),
    }
}

/// Fully re-resolve a delegation against its LIVE source (R-B2). Returns the
/// source grant's revision if it authorizes.
fn delegation_authorizes(
    state: &OrgState,
    d: &Delegation,
    org: OrgId,
    req: &AuthRequest,
) -> Option<u64> {
    if d.org != org
        || d.status != crate::DelegationStatus::Active
        || d.capability != req.capability
        || d.expires_at <= req.now
    {
        return None;
    }
    // The delegation's own (narrowed) scope + limit must authorize the request.
    if !scope_contains(state, &d.scope, req.scope) {
        return None;
    }
    if !matches!(limit_ok(d.limit, req), GrantCheck::Ok) {
        return None;
    }
    // Re-resolve the LIVE source grant — existence, validity NOW, delegable,
    // capability, and that the delegation's scope is contained in the source's
    // CURRENT scope. A captured source_revision is not trusted.
    let src = state.grant(d.source_grant)?;
    if src.org != org || !src.delegable || src.capability != d.capability {
        return None;
    }
    // The delegation's authority is only as good as the grantor's live hold on the
    // source grant. If the source has since been reassigned away from the grantor
    // (or the grantor never held it), the delegation confers nothing — a captured
    // link is not trusted, exactly as the scope/validity re-resolution below.
    if src.grantee != d.grantor {
        return None;
    }
    if !time_valid(src.valid_from, src.valid_until, req.now) {
        return None; // source expired — bumps no revision, so equality alone would miss it
    }
    if !selector_subsumes(state, &src.scope, &d.scope) {
        return None; // delegation escaped the source's current scope
    }
    // The source's own limit must also admit the amount.
    if !matches!(limit_ok(src.limit, req), GrantCheck::Ok) {
        return None;
    }
    Some(src.revision)
}

/// Whether `now` lies within `[from, until)` (either bound optional).
fn time_valid(from: Option<i64>, until: Option<i64>, now: i64) -> bool {
    from.is_none_or(|f| now >= f) && until.is_none_or(|u| now < u)
}

fn conditions_pass(conds: &[PolicyCondition], now: i64) -> bool {
    conds.iter().all(|c| match c {
        PolicyCondition::TimeWindow { from, until } => time_valid(*from, *until, now),
    })
}

/// Does `selector` cover the concrete `target` scope? Depth-capped so the walk
/// always terminates.
fn scope_contains(state: &OrgState, selector: &ScopeSelector, target: OrgScopeId) -> bool {
    match selector {
        ScopeSelector::Exact(s) => *s == target,
        ScopeSelector::SubtreeLive(root) => is_descendant_or_self(state, *root, target),
        ScopeSelector::SubtreeFrozen {
            root,
            snapshot_revision,
        } => {
            is_descendant_or_self(state, *root, target)
                && state
                    .scope(target)
                    .is_some_and(|t| t.created_rev <= *snapshot_revision)
        }
    }
}

/// Whether `target` is `root` or a descendant of it — walking `target`'s parent
/// chain, bounded by [`MAX_SCOPE_DEPTH`] hops (depth is capped at creation, so a
/// well-formed tree never exceeds it; the bound is a fail-closed backstop).
fn is_descendant_or_self(state: &OrgState, root: OrgScopeId, target: OrgScopeId) -> bool {
    let mut cur = target;
    for _ in 0..=usize::from(MAX_SCOPE_DEPTH) {
        if cur == root {
            return true;
        }
        match state.scope(cur).and_then(|s| s.parent) {
            Some(p) => cur = p,
            None => return false,
        }
    }
    false // exceeded the depth cap → fail closed
}

/// Whether `outer` covers every scope `inner` covers (delegation narrow-only).
/// Conservative + fail-closed.
pub(crate) fn selector_subsumes(
    state: &OrgState,
    outer: &ScopeSelector,
    inner: &ScopeSelector,
) -> bool {
    // The inner anchor must sit inside the outer selector's set.
    if !scope_contains(state, outer, inner.root()) {
        return false;
    }
    match (outer, inner) {
        // A live outer contains anything rooted inside it.
        (ScopeSelector::SubtreeLive(_), _) => true,
        // A frozen outer may narrow to an exact scope, or a no-later frozen subtree,
        // but NOT to a live subtree (which could widen past the frozen snapshot).
        (
            ScopeSelector::SubtreeFrozen {
                snapshot_revision: os,
                ..
            },
            inner,
        ) => match inner {
            ScopeSelector::Exact(_) => true,
            ScopeSelector::SubtreeFrozen {
                snapshot_revision: is,
                ..
            } => is <= os,
            ScopeSelector::SubtreeLive(_) => false,
        },
        // An exact outer can only cover the identical exact scope.
        (ScopeSelector::Exact(_), ScopeSelector::Exact(_)) => true,
        (ScopeSelector::Exact(_), _) => false,
    }
}

/// Pick the more-informative (more specific) of two near-miss deny reasons.
fn tighten(cur: Option<DenyReason>, new: DenyReason) -> Option<DenyReason> {
    fn rank(r: &DenyReason) -> u8 {
        match r {
            DenyReason::LimitExceeded => 3,
            DenyReason::AmountRequired => 2,
            _ => 1,
        }
    }
    match cur {
        Some(c) if rank(&c) >= rank(&new) => Some(c),
        _ => Some(new),
    }
}
