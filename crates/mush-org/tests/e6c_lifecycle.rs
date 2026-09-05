//! **B10 — the minimal org lifecycle** (E6c; DL-06 / 08g §3 #3, filed "before E6c"
//! and "critical for E6c").
//!
//! `OrgStatus::{Suspended, Dissolving, Closed}` have existed since E1a and **nothing
//! moved them**: closing was documented as a workflow with no op and no verb.
//! `Organization.hidden` has been read by the economy's existence-hiding renderer
//! since E6a's D11 and **written by nothing in the shipped surface**, which is why
//! that fold shipped preventively with no end-to-end test.
//!
//! This is the narrow slice that closes both — [`OrgMutationOp::SetOrgStatus`] and
//! [`OrgMutationOp::PutRole`], the revision bump, and `@org/status`. The **close
//! workflow** that resolves the accounts, assets and memberships a `Dissolving`
//! organization still holds, and the membership producer, stay E7's (OPEN 5).

use std::collections::BTreeSet;

use mush_core::ObjId;
use mush_org::{
    CapabilityId, EntityKey, ExpectedRevision, MembershipStatus, OrgId, OrgMembership,
    OrgMutationOp, OrgMutationPlan, OrgRole, OrgScope, OrgScopeId, OrgState, OrgStatus,
    Organization, RoleId, RulesValue, ScopeStatus,
};

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

fn scope_rec(id: u64, org: u64) -> OrgScope {
    OrgScope {
        id: OrgScopeId(id),
        org: OrgId(org),
        parent: None,
        name: "root".into(),
        status: ScopeStatus::Active,
        depth: 0,
        created_rev: 1,
        revision: 1,
    }
}

fn base() -> OrgState {
    OrgMutationPlan::of(vec![OrgMutationOp::CreateOrg {
        org: org_rec(1, 10),
        root: scope_rec(10, 1),
    }])
    .apply(&OrgState::default())
    .expect("the org is created")
}

fn role(id: u64, org: u64, caps: &[&str]) -> OrgRole {
    OrgRole {
        id: RoleId(id),
        org: OrgId(org),
        name: format!("role{id}").into(),
        grants: caps
            .iter()
            .map(|c| mush_org::GrantTemplate {
                capability: CapabilityId::new(c).expect("well-formed"),
                scope: mush_org::ScopeSelector::Exact(OrgScopeId(10)),
                limit: None,
                delegable: false,
            })
            .collect(),
        revision: 1,
    }
}

/// `SetOrgStatus` moves the lifecycle and **bumps the revision**, which is the
/// staleness stamp every `ExpectedRevision` guard reads. It touches nothing else.
#[test]
fn e6c_b10_set_org_status_moves_the_lifecycle_and_bumps_the_revision() {
    let state = base();
    let before = state.org(OrgId(1)).expect("the org").revision;
    let next = OrgMutationPlan::of(vec![OrgMutationOp::SetOrgStatus {
        org: OrgId(1),
        status: Some(OrgStatus::Suspended),
        hidden: None,
    }])
    .apply(&state)
    .expect("the status moves");
    let o = next.org(OrgId(1)).expect("the org");
    assert_eq!(o.status, OrgStatus::Suspended);
    assert!(o.revision > before, "the revision did not move");
    assert!(!o.hidden, "an untouched field moved");
    assert_eq!(
        next.scopes_in(OrgId(1)).count(),
        1,
        "the scope tree was disturbed"
    );

    // A stale plan is refused against the bumped revision, which is what the bump is
    // for.
    let stale = OrgMutationPlan {
        expected: vec![ExpectedRevision {
            key: EntityKey::Org(OrgId(1)),
            revision: before,
        }],
        ops: vec![OrgMutationOp::SetOrgStatus {
            org: OrgId(1),
            status: Some(OrgStatus::Active),
            hidden: None,
        }],
    };
    assert!(stale.apply(&next).is_err(), "a stale plan applied anyway");
}

/// `hidden` gets its **first write path** — the reason E6a's D11 shipped
/// preventively with no end-to-end test — and it moves independently of the status.
#[test]
fn e6c_b10_hidden_has_a_write_path_and_moves_independently_of_the_status() {
    let state = base();
    let next = OrgMutationPlan::of(vec![OrgMutationOp::SetOrgStatus {
        org: OrgId(1),
        status: None,
        hidden: Some(true),
    }])
    .apply(&state)
    .expect("hidden writes");
    let o = next.org(OrgId(1)).expect("the org");
    assert!(o.hidden, "the flag did not write");
    assert_eq!(o.status, OrgStatus::Active, "the status moved with it");

    let back = OrgMutationPlan::of(vec![OrgMutationOp::SetOrgStatus {
        org: OrgId(1),
        status: None,
        hidden: Some(false),
    }])
    .apply(&next)
    .expect("hidden clears");
    assert!(!back.org(OrgId(1)).expect("the org").hidden);
}

/// **`Closed` is terminal.** A closed organization does not re-open: the id is
/// retired, and re-opening it would hand back authority every grantee was told had
/// ended.
#[test]
fn e6c_b10_a_closed_organization_never_re_opens() {
    let closed = OrgMutationPlan::of(vec![OrgMutationOp::SetOrgStatus {
        org: OrgId(1),
        status: Some(OrgStatus::Closed),
        hidden: None,
    }])
    .apply(&base())
    .expect("the org closes");
    for want in [
        OrgStatus::Active,
        OrgStatus::Suspended,
        OrgStatus::Dissolving,
    ] {
        let e = OrgMutationPlan::of(vec![OrgMutationOp::SetOrgStatus {
            org: OrgId(1),
            status: Some(want),
            hidden: None,
        }])
        .apply(&closed)
        .expect_err("a closed org re-opened");
        assert!(format!("{e}").contains("terminal"), "{e}");
    }
    // Even hiding it is refused, because the op is refused as a whole — refuse, never
    // clamp (the E1a rule).
    assert!(
        OrgMutationPlan::of(vec![OrgMutationOp::SetOrgStatus {
            org: OrgId(1),
            status: None,
            hidden: Some(true),
        }])
        .apply(&closed)
        .is_err()
    );
}

/// `SetOrgStatus` on an org that does not exist is `NotFound`, and nothing is
/// written — the refuse-not-clamp rule.
#[test]
fn e6c_b10_set_org_status_on_an_unknown_org_writes_nothing() {
    let state = base();
    assert!(
        OrgMutationPlan::of(vec![OrgMutationOp::SetOrgStatus {
            org: OrgId(99),
            status: Some(OrgStatus::Suspended),
            hidden: None,
        }])
        .apply(&state)
        .is_err()
    );
    assert_eq!(
        state.org(OrgId(1)).expect("the org").status,
        OrgStatus::Active
    );
}

/// `PutRole` is the **upsert** `CreateRole` is not: a role whose capability set was
/// wrong can be corrected rather than abandoned, and correcting it bumps the owning
/// organization's revision.
#[test]
fn e6c_b10_put_role_upserts_where_create_role_refuses() {
    let state = OrgMutationPlan::of(vec![OrgMutationOp::CreateRole {
        role: role(20, 1, &["economy.account.post"]),
    }])
    .apply(&base())
    .expect("the role is created");
    // `CreateRole` refuses the same id a second time — which is what made a role
    // immutable once written.
    assert!(
        OrgMutationPlan::of(vec![OrgMutationOp::CreateRole {
            role: role(20, 1, &["contract.draft"]),
        }])
        .apply(&state)
        .is_err()
    );
    let before = state.org(OrgId(1)).expect("the org").revision;
    let next = OrgMutationPlan::of(vec![OrgMutationOp::PutRole {
        role: role(20, 1, &["contract.draft"]),
    }])
    .apply(&state)
    .expect("the role is replaced");
    let r = next.role(RoleId(20)).expect("the role");
    assert_eq!(r.grants.len(), 1);
    assert_eq!(r.grants[0].capability.as_str(), "contract.draft");
    assert!(
        next.org(OrgId(1)).expect("the org").revision > before,
        "an upsert did not bump the owning org's revision"
    );

    // A role cannot change organization, and an unknown organization is refused.
    let mut moved = role(20, 1, &["contract.draft"]);
    moved.org = OrgId(2);
    assert!(
        OrgMutationPlan::of(vec![OrgMutationOp::PutRole { role: moved }])
            .apply(&next)
            .is_err()
    );
}

/// A membership naming a role `PutRole` replaced still resolves — the upsert keeps
/// the id, which is what makes it a correction rather than a new role.
#[test]
fn e6c_b10_a_membership_survives_a_role_correction() {
    let state = OrgMutationPlan::of(vec![
        OrgMutationOp::CreateRole {
            role: role(20, 1, &["economy.account.post"]),
        },
        OrgMutationOp::PutMembership {
            membership: OrgMembership {
                org: OrgId(1),
                member: ObjId(7),
                roles: BTreeSet::from([RoleId(20)]),
                status: MembershipStatus::Active,
                titles: Vec::new(),
                visibility: mush_org::Visibility::Public,
                since: 0,
                revision: 1,
            },
        },
    ])
    .apply(&base())
    .expect("the membership is written");
    let next = OrgMutationPlan::of(vec![OrgMutationOp::PutRole {
        role: role(20, 1, &["contract.draft"]),
    }])
    .apply(&state)
    .expect("the role is corrected");
    let m = next
        .membership(OrgId(1), ObjId(7))
        .expect("the membership survived");
    assert!(m.roles.contains(&RoleId(20)));
}
