//! E1a end-to-end tests — build an `OrgState` through the mutation plan, then
//! exercise the pure `authorize` engine against the reviewed exit gates.

use std::collections::BTreeSet;

use mush_core::ObjId;
use mush_org::{
    AccessPolicy, AuthRequest, CapabilityGrant, CapabilityId, Decision, Delegation,
    DelegationStatus, DenyReason, EntityKey, ExpectedRevision, GrantLimit, GrantTemplate,
    MembershipStatus, NoPredicates, OrgError, OrgId, OrgMembership, OrgMutationOp, OrgMutationPlan,
    OrgRole, OrgScope, OrgScopeId, OrgState, OrgStatus, Organization, PolicyEffect, PolicyRule,
    PolicySubject, PredicateEval, PrincipalId, RoleId, RulesValue, ScopeSelector, ScopeStatus,
    Visibility,
};

fn cap(s: &str) -> CapabilityId {
    CapabilityId::new(s).unwrap()
}

fn org_rec(id: u64, root: u64) -> Organization {
    Organization {
        id: OrgId(id),
        name: format!("org{id}").into(),
        aliases: vec![],
        status: OrgStatus::Active,
        hidden: false,
        charter: String::new(),
        root_scope: OrgScopeId(root),
        revision: 1,
        ruleset_ext: RulesValue::default(),
    }
}
fn scope_rec(id: u64, org: u64, parent: Option<u64>, depth: u8, created_rev: u64) -> OrgScope {
    OrgScope {
        id: OrgScopeId(id),
        org: OrgId(org),
        parent: parent.map(OrgScopeId),
        name: format!("scope{id}").into(),
        status: ScopeStatus::Active,
        depth,
        created_rev,
        revision: 1,
    }
}

/// Apply a plan, panicking on error (test setup convenience).
fn apply(state: &OrgState, ops: Vec<OrgMutationOp>) -> OrgState {
    OrgMutationPlan::of(ops).apply(state).expect("plan applies")
}

fn req(actor: u32, capability: &str, scope: u64, now: i64) -> AuthRequest {
    AuthRequest { actor: ObjId(actor), capability: cap(capability), scope: OrgScopeId(scope), now, amount: None }
}

// A tiny world: two orgs, each with a root scope + a child scope.
// org 1: root=10, child=11 ; org 2: root=20, child=21
fn two_orgs() -> OrgState {
    apply(&OrgState::new(), vec![
        OrgMutationOp::CreateOrg { org: org_rec(1, 10), root: scope_rec(10, 1, None, 0, 1) },
        OrgMutationOp::CreateScope { scope: scope_rec(11, 1, Some(10), 1, 1) },
        OrgMutationOp::CreateOrg { org: org_rec(2, 20), root: scope_rec(20, 2, None, 0, 1) },
        OrgMutationOp::CreateScope { scope: scope_rec(21, 2, Some(20), 1, 1) },
    ])
}

fn grant(id: u64, org: u64, grantee: u32, capability: &str, scope: ScopeSelector, delegable: bool) -> CapabilityGrant {
    CapabilityGrant {
        id: mush_org::GrantId(id),
        org: OrgId(org),
        grantee: ObjId(grantee),
        capability: cap(capability),
        scope,
        limit: None,
        delegable,
        valid_from: None,
        valid_until: None,
        revision: 1,
    }
}

#[test]
fn default_deny_with_no_grant() {
    let s = two_orgs();
    let d = mush_org::authorize(&s, &req(100, "economy.account.view", 11, 0), &NoPredicates);
    assert_eq!(d, Decision::Deny(DenyReason::NoAuthority));
}

#[test]
fn direct_grant_authorizes_only_its_scope() {
    let s = two_orgs();
    let s = apply(&s, vec![OrgMutationOp::PutGrant {
        grant: grant(1, 1, 100, "economy.account.view", ScopeSelector::Exact(OrgScopeId(11)), false),
    }]);
    // Authorized on the exact scope...
    assert!(mush_org::authorize(&s, &req(100, "economy.account.view", 11, 0), &NoPredicates).is_allow());
    // ...but NOT on its sibling/parent, and NOT for a different capability.
    assert!(!mush_org::authorize(&s, &req(100, "economy.account.view", 10, 0), &NoPredicates).is_allow());
    assert!(!mush_org::authorize(&s, &req(100, "economy.account.transfer", 11, 0), &NoPredicates).is_allow());
    // ...and NOT for a different actor.
    assert!(!mush_org::authorize(&s, &req(101, "economy.account.view", 11, 0), &NoPredicates).is_allow());
}

#[test]
fn subtree_live_covers_descendants_frozen_excludes_later_children() {
    let mut s = two_orgs();
    // Grant a LIVE subtree over org1 root (covers root + child 11).
    s = apply(&s, vec![OrgMutationOp::PutGrant {
        grant: grant(1, 1, 100, "org.role.manage", ScopeSelector::SubtreeLive(OrgScopeId(10)), false),
    }]);
    assert!(mush_org::authorize(&s, &req(100, "org.role.manage", 11, 0), &NoPredicates).is_allow());

    // A FROZEN subtree at snapshot_revision 1 covers 11 (created_rev 1) but NOT a
    // child created later at created_rev 5.
    s = apply(&s, vec![
        OrgMutationOp::CreateScope { scope: scope_rec(12, 1, Some(10), 1, 5) },
        OrgMutationOp::PutGrant {
            grant: grant(2, 1, 101, "org.role.manage", ScopeSelector::SubtreeFrozen { root: OrgScopeId(10), snapshot_revision: 1 }, false),
        },
    ]);
    assert!(mush_org::authorize(&s, &req(101, "org.role.manage", 11, 0), &NoPredicates).is_allow(), "frozen covers the pre-snapshot child");
    assert!(!mush_org::authorize(&s, &req(101, "org.role.manage", 12, 0), &NoPredicates).is_allow(), "frozen must NOT silently widen to a later child");
    // ...whereas the LIVE grant (actor 100) DOES reach the new child.
    assert!(mush_org::authorize(&s, &req(100, "org.role.manage", 12, 0), &NoPredicates).is_allow());
}

#[test]
fn identical_role_names_different_orgs_different_powers() {
    let mut s = two_orgs();
    // Both orgs have a role named "treasurer" but granting DIFFERENT capabilities.
    let role_a = OrgRole {
        id: RoleId(1), org: OrgId(1), name: "treasurer".into(), revision: 1,
        grants: vec![GrantTemplate { capability: cap("economy.account.transfer"), scope: ScopeSelector::SubtreeLive(OrgScopeId(10)), limit: None, delegable: false }],
    };
    let role_b = OrgRole {
        id: RoleId(2), org: OrgId(2), name: "treasurer".into(), revision: 1,
        grants: vec![GrantTemplate { capability: cap("economy.account.view"), scope: ScopeSelector::SubtreeLive(OrgScopeId(20)), limit: None, delegable: false }],
    };
    let mut roles_a = BTreeSet::new();
    roles_a.insert(RoleId(1));
    let mut roles_b = BTreeSet::new();
    roles_b.insert(RoleId(2));
    s = apply(&s, vec![
        OrgMutationOp::CreateRole { role: role_a },
        OrgMutationOp::CreateRole { role: role_b },
        OrgMutationOp::PutMembership { membership: OrgMembership { org: OrgId(1), member: ObjId(100), status: MembershipStatus::Active, titles: vec![], roles: roles_a, visibility: Visibility::Public, since: 0, revision: 1 } },
        OrgMutationOp::PutMembership { membership: OrgMembership { org: OrgId(2), member: ObjId(100), status: MembershipStatus::Active, titles: vec![], roles: roles_b, visibility: Visibility::Public, since: 0, revision: 1 } },
    ]);
    // In org1, actor 100 (treasurer) may transfer but not in org2 (treasurer there is view-only).
    assert!(mush_org::authorize(&s, &req(100, "economy.account.transfer", 11, 0), &NoPredicates).is_allow());
    assert!(!mush_org::authorize(&s, &req(100, "economy.account.transfer", 21, 0), &NoPredicates).is_allow());
    assert!(mush_org::authorize(&s, &req(100, "economy.account.view", 21, 0), &NoPredicates).is_allow());
}

#[test]
fn explicit_deny_beats_a_grant() {
    let mut s = two_orgs();
    s = apply(&s, vec![
        OrgMutationOp::PutGrant { grant: grant(1, 1, 100, "economy.account.transfer", ScopeSelector::SubtreeLive(OrgScopeId(10)), false) },
        OrgMutationOp::PutPolicy { policy: AccessPolicy {
            id: mush_org::PolicyId(1), org: OrgId(1), owner: PrincipalId::Organization(OrgId(1)), default: PolicyEffect::Deny, revision: 1,
            rules: vec![PolicyRule { effect: PolicyEffect::Deny, subject: PolicySubject::Principal(PrincipalId::Object(ObjId(100))), capability: cap("economy.account.transfer"), scope: ScopeSelector::Exact(OrgScopeId(11)), conditions: vec![] }],
        }},
    ]);
    // The grant covers scope 11, but the explicit Deny on 11 wins.
    assert_eq!(mush_org::authorize(&s, &req(100, "economy.account.transfer", 11, 0), &NoPredicates), Decision::Deny(DenyReason::ExplicitDeny));
    // The grant still authorizes the (un-denied) root scope 10.
    assert!(mush_org::authorize(&s, &req(100, "economy.account.transfer", 10, 0), &NoPredicates).is_allow());
}

#[test]
fn policy_allow_grants_with_no_capability_grant() {
    // The ACL-grant reading (R-D2): a policy Allow is a first-class allow source.
    let mut s = two_orgs();
    s = apply(&s, vec![OrgMutationOp::PutPolicy { policy: AccessPolicy {
        id: mush_org::PolicyId(1), org: OrgId(1), owner: PrincipalId::Organization(OrgId(1)), default: PolicyEffect::Deny, revision: 1,
        rules: vec![PolicyRule { effect: PolicyEffect::Allow, subject: PolicySubject::Principal(PrincipalId::Object(ObjId(100))), capability: cap("vault.view"), scope: ScopeSelector::Exact(OrgScopeId(11)), conditions: vec![] }],
    }}]);
    assert!(mush_org::authorize(&s, &req(100, "vault.view", 11, 0), &NoPredicates).is_allow());
    assert!(!mush_org::authorize(&s, &req(101, "vault.view", 11, 0), &NoPredicates).is_allow());
}

#[test]
fn delegation_narrows_and_re_resolves_the_live_source() {
    let mut s = two_orgs();
    // A delegable grant to 100 over the live subtree, valid only until t=100.
    let mut g = grant(1, 1, 100, "economy.account.transfer", ScopeSelector::SubtreeLive(OrgScopeId(10)), true);
    g.valid_until = Some(100);
    s = apply(&s, vec![OrgMutationOp::PutGrant { grant: g }]);
    // 100 delegates a NARROWED (exact scope 11) grant to 200, expiring at t=1000.
    let del = Delegation {
        id: mush_org::DelegationId(1), org: OrgId(1), grantor: ObjId(100), grantee: ObjId(200),
        source_grant: mush_org::GrantId(1), capability: cap("economy.account.transfer"),
        scope: ScopeSelector::Exact(OrgScopeId(11)), limit: None, expires_at: 1000, source_revision: 1, status: DelegationStatus::Active,
    };
    s = apply(&s, vec![OrgMutationOp::Delegate { delegation: del }]);
    // Before the source expires: 200 may transfer on 11 (delegated), but NOT on 10 (narrowed away).
    assert!(mush_org::authorize(&s, &req(200, "economy.account.transfer", 11, 50), &NoPredicates).is_allow());
    assert!(!mush_org::authorize(&s, &req(200, "economy.account.transfer", 10, 50), &NoPredicates).is_allow());
    // AFTER the SOURCE grant's valid_until (t=200) — which bumps NO revision —
    // the delegation must NOT authorize even though its own expires_at=1000 is future.
    assert!(!mush_org::authorize(&s, &req(200, "economy.account.transfer", 11, 200), &NoPredicates).is_allow(), "delegation cannot outlive an expired source (R-B2)");
    // The source holder 100 is likewise no longer authorized after expiry.
    assert!(!mush_org::authorize(&s, &req(100, "economy.account.transfer", 11, 200), &NoPredicates).is_allow());
}

#[test]
fn delegation_that_would_widen_is_refused_at_apply() {
    let mut s = two_orgs();
    // Non-delegable? make delegable but try to widen scope from Exact(11) to the subtree.
    let mut g = grant(1, 1, 100, "economy.account.transfer", ScopeSelector::Exact(OrgScopeId(11)), true);
    g.delegable = true;
    s = apply(&s, vec![OrgMutationOp::PutGrant { grant: g }]);
    let widen = Delegation {
        id: mush_org::DelegationId(1), org: OrgId(1), grantor: ObjId(100), grantee: ObjId(200),
        source_grant: mush_org::GrantId(1), capability: cap("economy.account.transfer"),
        scope: ScopeSelector::SubtreeLive(OrgScopeId(10)), // wider than Exact(11)!
        limit: None, expires_at: 1000, source_revision: 1, status: DelegationStatus::Active,
    };
    let err = OrgMutationPlan::of(vec![OrgMutationOp::Delegate { delegation: widen }]).apply(&s).unwrap_err();
    assert_eq!(err, OrgError::DelegationWiden);
}

#[test]
fn limit_requires_amount_and_enforces_ceiling() {
    let mut s = two_orgs();
    let mut g = grant(1, 1, 100, "economy.account.transfer", ScopeSelector::Exact(OrgScopeId(11)), false);
    g.limit = Some(GrantLimit { max_amount: Some(500) });
    s = apply(&s, vec![OrgMutationOp::PutGrant { grant: g }]);
    // No amount → AmountRequired (fail-closed).
    let no_amt = AuthRequest { actor: ObjId(100), capability: cap("economy.account.transfer"), scope: OrgScopeId(11), now: 0, amount: None };
    assert_eq!(mush_org::authorize(&s, &no_amt, &NoPredicates), Decision::Deny(DenyReason::AmountRequired));
    // Within limit → allow; over → LimitExceeded.
    let within = AuthRequest { amount: Some(500), ..no_amt.clone() };
    assert!(mush_org::authorize(&s, &within, &NoPredicates).is_allow());
    let over = AuthRequest { amount: Some(501), ..no_amt };
    assert_eq!(mush_org::authorize(&s, &over, &NoPredicates), Decision::Deny(DenyReason::LimitExceeded));
}

struct Standing(bool);
impl PredicateEval for Standing {
    fn eval(&self, provider: &str, predicate: &str, _: &serde_json::Value, _: ObjId) -> Option<bool> {
        if provider == "faction" && predicate == "friendly" { Some(self.0) } else { None }
    }
}

#[test]
fn registered_predicate_allows_denies_and_fails_closed() {
    let mut s = two_orgs();
    s = apply(&s, vec![OrgMutationOp::PutPolicy { policy: AccessPolicy {
        id: mush_org::PolicyId(1), org: OrgId(1), owner: PrincipalId::Organization(OrgId(1)), default: PolicyEffect::Deny, revision: 1,
        rules: vec![PolicyRule { effect: PolicyEffect::Allow, subject: PolicySubject::RegisteredPredicate { provider: "faction".into(), predicate: "friendly".into(), arguments: serde_json::Value::Null }, capability: cap("economy.account.view"), scope: ScopeSelector::Exact(OrgScopeId(11)), conditions: vec![] }],
    }}]);
    let r = req(100, "economy.account.view", 11, 0);
    // Friendly → allowed; not friendly → denied.
    assert!(mush_org::authorize(&s, &r, &Standing(true)).is_allow());
    assert!(!mush_org::authorize(&s, &r, &Standing(false)).is_allow());
    // Unresolvable provider (NoPredicates) → the Allow rule is SUPPRESSED (fail-closed).
    assert!(!mush_org::authorize(&s, &r, &NoPredicates).is_allow());
}

#[test]
fn unresolvable_predicate_satisfies_a_deny() {
    let mut s = two_orgs();
    s = apply(&s, vec![
        OrgMutationOp::PutGrant { grant: grant(1, 1, 100, "economy.account.view", ScopeSelector::Exact(OrgScopeId(11)), false) },
        OrgMutationOp::PutPolicy { policy: AccessPolicy {
            id: mush_org::PolicyId(1), org: OrgId(1), owner: PrincipalId::Organization(OrgId(1)), default: PolicyEffect::Deny, revision: 1,
            rules: vec![PolicyRule { effect: PolicyEffect::Deny, subject: PolicySubject::RegisteredPredicate { provider: "faction".into(), predicate: "hostile".into(), arguments: serde_json::Value::Null }, capability: cap("economy.account.view"), scope: ScopeSelector::Exact(OrgScopeId(11)), conditions: vec![] }],
        }},
    ]);
    // The grant would allow, but the unresolvable Deny predicate is SATISFIED → deny.
    assert_eq!(mush_org::authorize(&s, &req(100, "economy.account.view", 11, 0), &NoPredicates), Decision::Deny(DenyReason::ExplicitDeny));
}

#[test]
fn nonexistent_scope_denies_not_allows() {
    let s = two_orgs();
    // A scope that doesn't exist → Deny(NoSuchScope) (the module collapses this
    // with NoAuthority publicly — but the engine never ALLOWS it).
    assert_eq!(mush_org::authorize(&s, &req(100, "economy.account.view", 999, 0), &NoPredicates), Decision::Deny(DenyReason::NoSuchScope));
}

#[test]
fn scope_creation_refuses_excess_depth() {
    // A chain of scopes down to depth 8 is fine; depth 9 is refused.
    let mut s = OrgState::new();
    s = apply(&s, vec![OrgMutationOp::CreateOrg { org: org_rec(1, 100), root: scope_rec(100, 1, None, 0, 1) }]);
    let mut ops = vec![];
    for d in 1..=8u8 {
        ops.push(OrgMutationOp::CreateScope { scope: scope_rec(100 + u64::from(d), 1, Some(100 + u64::from(d) - 1), d, 1) });
    }
    s = apply(&s, ops);
    // depth 9 → refused.
    let err = OrgMutationPlan::of(vec![OrgMutationOp::CreateScope { scope: scope_rec(109, 1, Some(108), 9, 1) }]).apply(&s).unwrap_err();
    assert_eq!(err, OrgError::DepthExceeded);
}

#[test]
fn plan_is_atomic_and_stale_guarded() {
    let s = two_orgs();
    // A plan whose 2nd op fails (duplicate org) leaves the whole plan unapplied.
    let bad = OrgMutationPlan::of(vec![
        OrgMutationOp::CreateScope { scope: scope_rec(30, 1, Some(10), 1, 1) },
        OrgMutationOp::CreateOrg { org: org_rec(1, 10), root: scope_rec(10, 1, None, 0, 1) }, // dup!
    ]);
    assert!(bad.apply(&s).is_err());
    // The original state is untouched (we still don't have scope 30).
    assert!(s.scope(OrgScopeId(30)).is_none());

    // Stale guard: an expectation that doesn't match current revision refuses.
    let stale = OrgMutationPlan {
        expected: vec![ExpectedRevision { key: EntityKey::Org(OrgId(1)), revision: 99 }],
        ops: vec![OrgMutationOp::CreateScope { scope: scope_rec(31, 1, Some(10), 1, 1) }],
    };
    assert!(matches!(stale.apply(&s), Err(OrgError::Stale(_, 99, Some(1)))));
}

#[test]
fn destroy_scrub_removes_all_authority() {
    let mut s = two_orgs();
    let mut roles = BTreeSet::new();
    roles.insert(RoleId(1));
    s = apply(&s, vec![
        OrgMutationOp::CreateRole { role: OrgRole { id: RoleId(1), org: OrgId(1), name: "r".into(), revision: 1, grants: vec![GrantTemplate { capability: cap("economy.account.view"), scope: ScopeSelector::Exact(OrgScopeId(11)), limit: None, delegable: false }] } },
        OrgMutationOp::PutMembership { membership: OrgMembership { org: OrgId(1), member: ObjId(100), status: MembershipStatus::Active, titles: vec![], roles, visibility: Visibility::Public, since: 0, revision: 1 } },
        OrgMutationOp::PutGrant { grant: grant(1, 1, 100, "economy.account.transfer", ScopeSelector::Exact(OrgScopeId(11)), false) },
    ]);
    assert!(mush_org::authorize(&s, &req(100, "economy.account.view", 11, 0), &NoPredicates).is_allow());
    assert!(mush_org::authorize(&s, &req(100, "economy.account.transfer", 11, 0), &NoPredicates).is_allow());
    // Destroy the object → scrub → no authority remains, and no membership record.
    s.scrub_object(ObjId(100));
    assert!(!mush_org::authorize(&s, &req(100, "economy.account.view", 11, 0), &NoPredicates).is_allow());
    assert!(!mush_org::authorize(&s, &req(100, "economy.account.transfer", 11, 0), &NoPredicates).is_allow());
    assert!(s.membership(OrgId(1), ObjId(100)).is_none());
}

#[test]
fn org_subject_policy_authorizes_no_actor() {
    // Regression: `PolicySubject::Organization` is the org "as a principal" — an
    // OrgId, never an ObjId actor — so an org-subject Allow rule must match NOBODY
    // (it previously reduced to `org == org`, authorizing the whole playerbase).
    let mut s = two_orgs();
    let mut roles = BTreeSet::new();
    roles.insert(RoleId(1));
    s = apply(&s, vec![
        OrgMutationOp::CreateRole { role: OrgRole { id: RoleId(1), org: OrgId(1), name: "r".into(), revision: 1, grants: vec![] } },
        OrgMutationOp::PutMembership { membership: OrgMembership { org: OrgId(1), member: ObjId(100), status: MembershipStatus::Active, titles: vec![], roles, visibility: Visibility::Public, since: 0, revision: 1 } },
        OrgMutationOp::PutPolicy { policy: AccessPolicy {
            id: mush_org::PolicyId(1), org: OrgId(1), owner: PrincipalId::Organization(OrgId(1)), default: PolicyEffect::Deny, revision: 1,
            rules: vec![PolicyRule { effect: PolicyEffect::Allow, subject: PolicySubject::Organization(OrgId(1)), capability: cap("economy.account.transfer"), scope: ScopeSelector::SubtreeLive(OrgScopeId(10)), conditions: vec![] }],
        }},
    ]);
    // A non-member is denied...
    assert!(!mush_org::authorize(&s, &req(999, "economy.account.transfer", 11, 0), &NoPredicates).is_allow());
    // ...and so is an active MEMBER — "any member" is `OrganizationMember`, not this.
    assert!(!mush_org::authorize(&s, &req(100, "economy.account.transfer", 11, 0), &NoPredicates).is_allow());

    // For contrast, the same rule with `OrganizationMember` DOES gate on membership.
    s = apply(&s, vec![OrgMutationOp::PutPolicy { policy: AccessPolicy {
        id: mush_org::PolicyId(1), org: OrgId(1), owner: PrincipalId::Organization(OrgId(1)), default: PolicyEffect::Deny, revision: 2,
        rules: vec![PolicyRule { effect: PolicyEffect::Allow, subject: PolicySubject::OrganizationMember(OrgId(1)), capability: cap("economy.account.transfer"), scope: ScopeSelector::SubtreeLive(OrgScopeId(10)), conditions: vec![] }],
    }}]);
    assert!(mush_org::authorize(&s, &req(100, "economy.account.transfer", 11, 0), &NoPredicates).is_allow(), "member matches OrganizationMember");
    assert!(!mush_org::authorize(&s, &req(999, "economy.account.transfer", 11, 0), &NoPredicates).is_allow(), "non-member still denied");
}

#[test]
fn delegation_grantor_must_hold_the_source_grant() {
    let mut s = two_orgs();
    // A delegable grant held by 100.
    s = apply(&s, vec![OrgMutationOp::PutGrant {
        grant: grant(1, 1, 100, "economy.account.transfer", ScopeSelector::SubtreeLive(OrgScopeId(10)), true),
    }]);
    // 999 holds nothing; a delegation claiming to originate from 999 off 100's grant
    // is refused at apply (forged delegated authority).
    let forged = Delegation {
        id: mush_org::DelegationId(1), org: OrgId(1), grantor: ObjId(999), grantee: ObjId(999),
        source_grant: mush_org::GrantId(1), capability: cap("economy.account.transfer"),
        scope: ScopeSelector::Exact(OrgScopeId(11)), limit: None, expires_at: 1000, source_revision: 1, status: DelegationStatus::Active,
    };
    let err = OrgMutationPlan::of(vec![OrgMutationOp::Delegate { delegation: forged }]).apply(&s).unwrap_err();
    assert!(matches!(err, OrgError::Invalid(_)), "forged delegation refused at apply, got {err:?}");

    // Live re-check: a LEGIT delegation from 100, then the source grant is reassigned
    // away from 100 (PutGrant replace, same id, grantee 101). The delegatee 200 must
    // now be denied — the grantor no longer holds the source.
    let legit = Delegation {
        id: mush_org::DelegationId(2), org: OrgId(1), grantor: ObjId(100), grantee: ObjId(200),
        source_grant: mush_org::GrantId(1), capability: cap("economy.account.transfer"),
        scope: ScopeSelector::Exact(OrgScopeId(11)), limit: None, expires_at: 1000, source_revision: 1, status: DelegationStatus::Active,
    };
    s = apply(&s, vec![OrgMutationOp::Delegate { delegation: legit }]);
    assert!(mush_org::authorize(&s, &req(200, "economy.account.transfer", 11, 0), &NoPredicates).is_allow(), "delegated while 100 holds the source");
    // Reassign the source grant to 101.
    s = apply(&s, vec![OrgMutationOp::PutGrant {
        grant: grant(1, 1, 101, "economy.account.transfer", ScopeSelector::SubtreeLive(OrgScopeId(10)), true),
    }]);
    assert!(!mush_org::authorize(&s, &req(200, "economy.account.transfer", 11, 0), &NoPredicates).is_allow(), "delegation dies when the grantor loses the source (live re-check)");
}

#[test]
fn grant_replace_reassigns_authority_cleanly() {
    // Regression: a same-id grant replace whose grantee changes must move authority
    // wholesale — the old grantee keeps NOTHING (stale reverse-index bug).
    let mut s = two_orgs();
    s = apply(&s, vec![OrgMutationOp::PutGrant {
        grant: grant(1, 1, 100, "economy.account.transfer", ScopeSelector::Exact(OrgScopeId(11)), false),
    }]);
    assert!(mush_org::authorize(&s, &req(100, "economy.account.transfer", 11, 0), &NoPredicates).is_allow());
    // Replace grant 1: same id, new grantee 101.
    s = apply(&s, vec![OrgMutationOp::PutGrant {
        grant: grant(1, 1, 101, "economy.account.transfer", ScopeSelector::Exact(OrgScopeId(11)), false),
    }]);
    assert!(!mush_org::authorize(&s, &req(100, "economy.account.transfer", 11, 0), &NoPredicates).is_allow(), "old grantee loses authority on reassign");
    assert!(mush_org::authorize(&s, &req(101, "economy.account.transfer", 11, 0), &NoPredicates).is_allow(), "new grantee holds it");
}

#[test]
fn policy_replace_moving_orgs_leaves_the_old_org() {
    // Regression: a same-id policy replace whose org changes must stop being
    // evaluated under the old org (stale by-org index bug).
    let mut s = two_orgs();
    s = apply(&s, vec![OrgMutationOp::PutPolicy { policy: AccessPolicy {
        id: mush_org::PolicyId(1), org: OrgId(1), owner: PrincipalId::Organization(OrgId(1)), default: PolicyEffect::Deny, revision: 1,
        rules: vec![PolicyRule { effect: PolicyEffect::Allow, subject: PolicySubject::Principal(PrincipalId::Object(ObjId(100))), capability: cap("vault.view"), scope: ScopeSelector::Exact(OrgScopeId(11)), conditions: vec![] }],
    }}]);
    assert!(mush_org::authorize(&s, &req(100, "vault.view", 11, 0), &NoPredicates).is_allow());
    // Move the policy to org 2 (keep the same rule text). Requests on org1's scope 11
    // must no longer see it.
    s = apply(&s, vec![OrgMutationOp::PutPolicy { policy: AccessPolicy {
        id: mush_org::PolicyId(1), org: OrgId(2), owner: PrincipalId::Organization(OrgId(2)), default: PolicyEffect::Deny, revision: 2,
        rules: vec![PolicyRule { effect: PolicyEffect::Allow, subject: PolicySubject::Principal(PrincipalId::Object(ObjId(100))), capability: cap("vault.view"), scope: ScopeSelector::Exact(OrgScopeId(11)), conditions: vec![] }],
    }}]);
    assert!(!mush_org::authorize(&s, &req(100, "vault.view", 11, 0), &NoPredicates).is_allow(), "moved policy no longer evaluated under org1");
}

// ======================================================================
// E3b2 (§4.8) — inert statuses become real, RetireScope, the scope cap
// ======================================================================

#[test]
fn a_retired_scope_confers_no_authority() {
    let mut s = two_orgs();
    s = apply(&s, vec![OrgMutationOp::PutGrant {
        grant: grant(1, 1, 100, "economy.account.post", ScopeSelector::SubtreeLive(OrgScopeId(10)), false),
    }]);
    assert!(mush_org::authorize(&s, &req(100, "economy.account.post", 11, 0), &NoPredicates).is_allow());

    // Retiring the child kills authority ON it while leaving the rest of the tree
    // (and the grant record itself) untouched — retire is never a delete.
    s = apply(&s, vec![OrgMutationOp::RetireScope { scope: OrgScopeId(11) }]);
    assert_eq!(
        mush_org::authorize(&s, &req(100, "economy.account.post", 11, 0), &NoPredicates),
        Decision::Deny(DenyReason::Inactive)
    );
    assert!(mush_org::authorize(&s, &req(100, "economy.account.post", 10, 0), &NoPredicates).is_allow());
    assert!(s.grant(mush_org::GrantId(1)).is_some(), "retire deleted the grant");
    assert_eq!(s.scope(OrgScopeId(11)).expect("kept").status, ScopeStatus::Retired);
}

#[test]
fn a_non_active_org_confers_no_authority_on_any_scope() {
    let mut s = two_orgs();
    s = apply(&s, vec![OrgMutationOp::PutGrant {
        grant: grant(1, 1, 100, "economy.account.post", ScopeSelector::SubtreeLive(OrgScopeId(10)), false),
    }]);
    // A policy Allow must not out-vote the org status either.
    s = apply(&s, vec![OrgMutationOp::PutPolicy {
        policy: AccessPolicy {
            id: mush_org::PolicyId(1),
            org: OrgId(1),
            owner: PrincipalId::Organization(OrgId(1)),
            rules: vec![PolicyRule {
                effect: PolicyEffect::Allow,
                subject: PolicySubject::Principal(PrincipalId::Object(ObjId(100))),
                capability: cap("economy.account.post"),
                scope: ScopeSelector::SubtreeLive(OrgScopeId(10)),
                conditions: vec![],
            }],
            default: PolicyEffect::Deny,
            revision: 1,
        },
    }]);
    assert!(mush_org::authorize(&s, &req(100, "economy.account.post", 11, 0), &NoPredicates).is_allow());

    for status in [OrgStatus::Suspended, OrgStatus::Dissolving, OrgStatus::Closed] {
        let mut org = s.org(OrgId(1)).expect("org").clone();
        org.status = status;
        // `put_org` is crate-private; go through the plan alphabet (CreateOrg would
        // collide) by rebuilding the whole state with the wound-down org.
        let mut wound = OrgState::new();
        let mut root = scope_rec(10, 1, None, 0, 1);
        root.status = ScopeStatus::Active;
        wound = apply(&wound, vec![
            OrgMutationOp::CreateOrg { org, root },
            OrgMutationOp::CreateScope { scope: scope_rec(11, 1, Some(10), 1, 1) },
            OrgMutationOp::PutGrant {
                grant: grant(1, 1, 100, "economy.account.post", ScopeSelector::SubtreeLive(OrgScopeId(10)), false),
            },
        ]);
        assert_eq!(
            mush_org::authorize(&wound, &req(100, "economy.account.post", 11, 0), &NoPredicates),
            Decision::Deny(DenyReason::Inactive),
            "{status:?} org still authorized"
        );
    }
}

#[test]
fn retire_refuses_a_root_scope_and_a_missing_one() {
    let s = two_orgs();
    let e = OrgMutationPlan::of(vec![OrgMutationOp::RetireScope { scope: OrgScopeId(10) }])
        .apply(&s)
        .expect_err("a root scope must not retire");
    assert!(matches!(e, OrgError::Invalid(_)), "{e:?}");
    let e = OrgMutationPlan::of(vec![OrgMutationOp::RetireScope { scope: OrgScopeId(999) }])
        .apply(&s)
        .expect_err("a missing scope must not retire");
    assert!(matches!(e, OrgError::NotFound(_)), "{e:?}");
    // ...and the refusal left the state alone.
    assert_eq!(s.scope(OrgScopeId(10)).expect("root").status, ScopeStatus::Active);
}

#[test]
fn scope_creation_stops_at_the_per_org_cap() {
    let mut s = two_orgs();
    // org 1 already holds 2 scopes (root 10 + child 11); fill to the cap.
    let mut next = 100u64;
    for _ in 2..mush_org::MAX_SCOPES_PER_ORG {
        s = apply(&s, vec![OrgMutationOp::CreateScope { scope: scope_rec(next, 1, Some(10), 1, 1) }]);
        next += 1;
    }
    assert_eq!(s.scopes_in(OrgId(1)).count(), mush_org::MAX_SCOPES_PER_ORG);
    let e = OrgMutationPlan::of(vec![OrgMutationOp::CreateScope { scope: scope_rec(next, 1, Some(10), 1, 1) }])
        .apply(&s)
        .expect_err("the cap must refuse");
    assert_eq!(e, OrgError::TooManyScopes);
    // The other org is unaffected — the cap is per organization.
    assert!(
        OrgMutationPlan::of(vec![OrgMutationOp::CreateScope { scope: scope_rec(next, 2, Some(20), 1, 1) }])
            .apply(&s)
            .is_ok()
    );
}
