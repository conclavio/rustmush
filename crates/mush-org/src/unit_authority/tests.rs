#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use mush_core::ObjId;

use super::*;
use crate::{
    AccessPolicy, CapabilityGrant, GrantId, GrantTemplate, MembershipStatus, NoPredicates, OrgId,
    OrgMembership, OrgRole, OrgScope, OrgScopeId, OrgState, OrgStatus, Organization,
    PolicyCondition, PolicyEffect, PolicyId, PolicyRule, PolicySubject, PrincipalId, RoleId,
    RulesValue, ScopeSelector, ScopeStatus, Visibility,
};

const OWNER: ObjId = ObjId(1);
const LENDER: ObjId = ObjId(2);
const PILOT: ObjId = ObjId(3);
const UNIT: ObjId = ObjId(20);
const OTHER: ObjId = ObjId(21);
const CAP: UnitCapability = UnitCapability::OperateCrew;

#[derive(Clone, Default)]
struct Facts {
    people: BTreeSet<ObjId>,
    assets: BTreeMap<ObjId, UnitAssetFacts>,
    missions: BTreeMap<MissionBinding, MissionFacts>,
    custody: Option<RecoveryCustodyFacts>,
}

impl UnitAuthorityFacts for Facts {
    fn principal_exists(&self, actor: ObjId) -> bool {
        self.people.contains(&actor)
    }
    fn asset(&self, unit: ObjId) -> Option<UnitAssetFacts> {
        self.assets.get(&unit).cloned()
    }
    fn mission(&self, binding: &MissionBinding) -> Option<MissionFacts> {
        self.missions.get(binding).cloned()
    }
    fn recovery_custody(&self, _: ObjId, _: ObjId) -> Option<RecoveryCustodyFacts> {
        self.custody.clone()
    }
}

fn lineage(seq: u64) -> TitleLineage {
    TitleLineage {
        journal_seq: seq,
        effect_index: 0,
    }
}

fn facts() -> Facts {
    Facts {
        people: (1..=16).map(ObjId).collect(),
        assets: [UNIT, OTHER]
            .into_iter()
            .map(|unit| {
                (
                    unit,
                    UnitAssetFacts {
                        unit,
                        owner: PrincipalId::Object(OWNER),
                        lineage: lineage(10),
                        scope: None,
                        active: true,
                    },
                )
            })
            .collect(),
        ..Facts::default()
    }
}

fn finite(unit: ObjId) -> UnitScope {
    UnitScope::Units {
        units: [unit].into_iter().collect(),
    }
}

fn loan(id: u64, recipient: ObjId) -> UnitGrant {
    UnitGrant {
        id: UnitGrantId(id),
        revision: 1,
        grantor: OWNER,
        grantee: UnitGrantee::Character(recipient),
        capability: CAP,
        scope: finite(UNIT),
        valid_from: Some(0),
        valid_until: Some(100),
        delegable: true,
        recovery: RecoveryPolicy::ReturnOnly,
        parent: UnitGrantParent::PersonalTitle {
            unit: UNIT,
            owner: OWNER,
            lineage: lineage(10),
        },
        status: UnitGrantStatus::Active,
    }
}

fn child(id: u64, grantor: ObjId, recipient: ObjId, parent: u64) -> UnitGrant {
    UnitGrant {
        grantor,
        parent: UnitGrantParent::UnitGrant {
            id: UnitGrantId(parent),
        },
        ..loan(id, recipient)
    }
}

fn book(grants: impl IntoIterator<Item = UnitGrant>) -> BTreeMap<UnitGrantId, UnitGrant> {
    grants.into_iter().map(|grant| (grant.id, grant)).collect()
}

fn request(actor: ObjId, unit: ObjId) -> UnitAuthRequest {
    UnitAuthRequest {
        actor,
        unit,
        action: UnitAction::Exercise(CAP),
        now: 10,
        mission: None,
    }
}

fn allowed(
    orgs: &OrgState,
    grants: &BTreeMap<UnitGrantId, UnitGrant>,
    facts: &Facts,
    req: &UnitAuthRequest,
) -> bool {
    authorize_unit(orgs, grants, facts, &NoPredicates, req).is_allow()
}

fn orgs() -> OrgState {
    let mut state = OrgState::new();
    for (organization, root, child) in [(1, 10, 11), (2, 20, 21)] {
        state.put_org(Organization {
            id: OrgId(organization),
            name: format!("org{organization}").into(),
            aliases: vec![],
            status: OrgStatus::Active,
            hidden: false,
            charter: String::new(),
            root_scope: OrgScopeId(root),
            revision: 1,
            ruleset_ext: RulesValue::default(),
        });
        for (id, parent, depth) in [(root, None, 0), (child, Some(OrgScopeId(root)), 1)] {
            state.put_scope(OrgScope {
                id: OrgScopeId(id),
                org: OrgId(organization),
                parent,
                name: format!("scope{id}").into(),
                status: ScopeStatus::Active,
                depth,
                created_rev: 1,
                revision: 1,
            });
        }
    }
    state
}

fn org_grant(id: u64, capability: UnitCapability, delegable: bool) -> CapabilityGrant {
    CapabilityGrant {
        id: GrantId(id),
        org: OrgId(1),
        grantee: OWNER,
        capability: capability.capability_id().expect("fixed capability"),
        scope: ScopeSelector::SubtreeLive(OrgScopeId(10)),
        limit: None,
        delegable,
        valid_from: Some(0),
        valid_until: Some(100),
        revision: 1,
    }
}

fn org_owned(facts: &mut Facts, unit: ObjId, scope: u64) {
    let row = facts.assets.get_mut(&unit).expect("asset");
    row.owner = PrincipalId::Organization(OrgId(1));
    row.scope = Some(OrgScopeId(scope));
}

fn wildcard(id: u64, recipient: ObjId) -> UnitGrant {
    UnitGrant {
        recovery: RecoveryPolicy::None,
        scope: UnitScope::OrganizationControlled {
            organization: OrgId(1),
            scope: ScopeSelector::SubtreeLive(OrgScopeId(10)),
            excluded: BTreeSet::new(),
        },
        parent: UnitGrantParent::OrganizationGrant {
            organization: OrgId(1),
            grant: GrantId(1),
        },
        ..loan(id, recipient)
    }
}

#[test]
fn personal_owner_and_two_hop_loans_revalidate_live_lineage() {
    let orgs = OrgState::new();
    let mut facts = facts();
    let mut grants = book([loan(1, LENDER), child(2, LENDER, PILOT, 1)]);
    assert!(allowed(&orgs, &grants, &facts, &request(PILOT, UNIT)));
    assert!(!allowed(&orgs, &grants, &facts, &request(PILOT, OTHER)));
    let mut onward = request(PILOT, UNIT);
    onward.action = UnitAction::Delegate(CAP);
    assert!(allowed(&orgs, &grants, &facts, &onward));
    grants.get_mut(&UnitGrantId(2)).expect("child").delegable = false;
    assert!(!allowed(&orgs, &grants, &facts, &onward));
    assert!(allowed(&orgs, &grants, &facts, &request(PILOT, UNIT)));
    // A routine asset edit is deliberately not an authority-fact field.
    assert_eq!(facts.asset(UNIT).expect("asset").lineage, lineage(10));
    facts.assets.get_mut(&UNIT).expect("asset").owner = PrincipalId::Object(ObjId(4));
    assert!(!allowed(&orgs, &grants, &facts, &request(PILOT, UNIT)));
    let asset = facts.assets.get_mut(&UNIT).expect("asset");
    asset.owner = PrincipalId::Object(OWNER);
    asset.lineage = lineage(12); // even a free title away-and-back is a new root
    assert!(!allowed(&orgs, &grants, &facts, &request(PILOT, UNIT)));
    assert!(allowed(&orgs, &grants, &facts, &request(OWNER, UNIT)));
}

#[test]
fn expiry_revocation_and_live_parent_narrowing_do_not_depend_on_revision() {
    let facts = facts();
    let orgs = OrgState::new();
    let original = book([loan(1, LENDER), child(2, LENDER, PILOT, 1)]);
    let mut end = request(PILOT, UNIT);
    end.now = 100;
    assert!(!allowed(&orgs, &original, &facts, &end));
    for edit in 0..5 {
        let mut grants = original.clone();
        let parent = grants.get_mut(&UnitGrantId(1)).expect("parent");
        match edit {
            0 => parent.status = UnitGrantStatus::Revoked,
            1 => parent.delegable = false,
            2 => parent.valid_until = Some(50),
            3 => parent.grantee = UnitGrantee::Character(ObjId(4)),
            _ => parent.scope = finite(OTHER),
        }
        assert!(
            !allowed(&orgs, &grants, &facts, &request(PILOT, UNIT)),
            "edit {edit}"
        );
    }
}

#[test]
fn admission_rejects_capability_scope_time_and_recovery_amplification() {
    let orgs = OrgState::new();
    let facts = facts();
    let grants = book([loan(1, LENDER)]);
    let valid = child(2, LENDER, PILOT, 1);
    assert!(validate_unit_grant(&orgs, &grants, &facts, &NoPredicates, &valid, 10).is_ok());
    for change in 0..4 {
        let mut wider = valid.clone();
        match change {
            0 => wider.capability = UnitCapability::TransferAbandonTitle,
            1 => wider.scope = finite(OTHER),
            2 => wider.valid_until = None,
            _ => wider.valid_from = None,
        }
        assert!(validate_unit_grant(&orgs, &grants, &facts, &NoPredicates, &wider, 10).is_err());
    }
    let mut without_policy = grants;
    without_policy
        .get_mut(&UnitGrantId(1))
        .expect("parent")
        .recovery = RecoveryPolicy::None;
    assert!(
        validate_unit_grant(&orgs, &without_policy, &facts, &NoPredicates, &valid, 10).is_err()
    );
}

#[test]
fn cycles_and_depth_are_closed_even_in_supplied_records() {
    let orgs = OrgState::new();
    let facts = facts();
    let mut grants = book([loan(1, ObjId(2))]);
    for id in 2..=9 {
        let grantor = ObjId(u32::try_from(id).expect("small"));
        let recipient = ObjId(u32::try_from(id + 1).expect("small"));
        grants.insert(UnitGrantId(id), child(id, grantor, recipient, id - 1));
    }
    assert!(
        allowed(&orgs, &grants, &facts, &request(ObjId(9), UNIT)),
        "eight rows fit"
    );
    assert!(
        !allowed(&orgs, &grants, &facts, &request(ObjId(10), UNIT)),
        "ninth row refuses"
    );
    grants.get_mut(&UnitGrantId(1)).expect("first").parent =
        UnitGrantParent::UnitGrant { id: UnitGrantId(2) };
    grants.get_mut(&UnitGrantId(2)).expect("second").grantee = UnitGrantee::Character(OWNER);
    assert!(!allowed(&orgs, &grants, &facts, &request(ObjId(2), UNIT)));
}

#[test]
fn exact_org_source_cannot_be_substituted_by_policy_or_another_grant() {
    let mut orgs = orgs();
    let mut facts = facts();
    org_owned(&mut facts, UNIT, 11);
    orgs.put_grant(org_grant(1, CAP, false));
    orgs.put_grant(org_grant(2, CAP, true));
    let grant = wildcard(1, PILOT);
    assert!(
        validate_unit_grant(&orgs, &BTreeMap::new(), &facts, &NoPredicates, &grant, 10).is_err()
    );
    let mut correct = grant.clone();
    correct.parent = UnitGrantParent::OrganizationGrant {
        organization: OrgId(1),
        grant: GrantId(2),
    };
    assert!(
        validate_unit_grant(&orgs, &BTreeMap::new(), &facts, &NoPredicates, &correct, 10).is_ok()
    );
    orgs.remove_grant(GrantId(2));
    orgs.put_policy(AccessPolicy {
        id: PolicyId(1),
        org: OrgId(1),
        owner: PrincipalId::Organization(OrgId(1)),
        rules: vec![PolicyRule {
            effect: PolicyEffect::Allow,
            subject: PolicySubject::Principal(PrincipalId::Object(OWNER)),
            capability: CAP.capability_id().expect("cap"),
            scope: ScopeSelector::SubtreeLive(OrgScopeId(10)),
            conditions: vec![],
        }],
        default: PolicyEffect::Deny,
        revision: 1,
    });
    assert!(
        crate::authorize(
            &orgs,
            &crate::AuthRequest {
                actor: OWNER,
                capability: CAP.capability_id().expect("cap"),
                scope: OrgScopeId(11),
                now: 10,
                amount: None
            },
            &NoPredicates
        )
        .is_allow()
    );
    assert!(!allowed(
        &orgs,
        &book([correct]),
        &facts,
        &request(PILOT, UNIT)
    ));
}

#[test]
fn exact_roles_require_live_membership_and_template_revision_but_direct_grants_do_not() {
    let mut orgs = orgs();
    let mut facts = facts();
    org_owned(&mut facts, UNIT, 11);
    let role = OrgRole {
        id: RoleId(1),
        org: OrgId(1),
        name: "operator".into(),
        revision: 1,
        grants: vec![GrantTemplate {
            capability: CAP.capability_id().expect("cap"),
            scope: ScopeSelector::SubtreeLive(OrgScopeId(10)),
            limit: None,
            delegable: true,
        }],
    };
    orgs.put_role(role.clone());
    orgs.put_membership(OrgMembership {
        org: OrgId(1),
        member: OWNER,
        status: MembershipStatus::Active,
        titles: vec![],
        roles: [RoleId(1)].into_iter().collect(),
        visibility: Visibility::Public,
        since: 0,
        revision: 1,
    });
    let mut grant = wildcard(1, PILOT);
    grant.parent = UnitGrantParent::OrganizationRoleTemplate {
        organization: OrgId(1),
        role: RoleId(1),
        role_revision: 1,
        template_index: 0,
    };
    let grants = book([grant]);
    assert!(allowed(&orgs, &grants, &facts, &request(PILOT, UNIT)));
    orgs.put_role(OrgRole {
        revision: 2,
        ..role
    });
    assert!(!allowed(&orgs, &grants, &facts, &request(PILOT, UNIT)));
    orgs.remove_membership(OrgId(1), OWNER);
    assert!(!allowed(&orgs, &grants, &facts, &request(PILOT, UNIT)));
    orgs.put_grant(org_grant(1, CAP, true));
    assert!(
        allowed(
            &orgs,
            &book([wildcard(2, PILOT)]),
            &facts,
            &request(PILOT, UNIT)
        ),
        "direct grantees need not be members"
    );
}

#[test]
fn wildcard_follows_current_control_and_future_units_but_not_custody() {
    let mut orgs = orgs();
    orgs.put_grant(org_grant(1, CAP, true));
    let mut facts = facts();
    org_owned(&mut facts, UNIT, 11);
    let grants = book([wildcard(1, PILOT)]);
    assert!(allowed(&orgs, &grants, &facts, &request(PILOT, UNIT)));
    assert!(
        !allowed(&orgs, &grants, &facts, &request(PILOT, OTHER)),
        "personal title aboard supplies nothing"
    );
    org_owned(&mut facts, OTHER, 11);
    assert!(allowed(&orgs, &grants, &facts, &request(PILOT, OTHER)));
    facts.assets.get_mut(&UNIT).expect("asset").owner = PrincipalId::Organization(OrgId(2));
    assert!(
        !allowed(&orgs, &grants, &facts, &request(PILOT, UNIT)),
        "stale org scope cannot preserve control"
    );
}

#[test]
fn exclusions_override_overlapping_wildcards_and_cannot_be_removed_by_children() {
    let mut orgs = orgs();
    orgs.put_grant(org_grant(1, CAP, true));
    let mut facts = facts();
    org_owned(&mut facts, UNIT, 11);
    let mut excluded = wildcard(1, LENDER);
    if let UnitScope::OrganizationControlled { excluded, .. } = &mut excluded.scope {
        excluded.insert(UNIT);
    }
    let mut narrowed = child(3, LENDER, PILOT, 1);
    narrowed.recovery = RecoveryPolicy::None;
    let grants = book([excluded, wildcard(2, LENDER), narrowed.clone()]);
    assert!(!allowed(&orgs, &grants, &facts, &request(LENDER, UNIT)));
    assert!(!allowed(&orgs, &grants, &facts, &request(PILOT, UNIT)));
    let mut base = grants;
    base.remove(&UnitGrantId(3));
    assert!(validate_unit_grant(&orgs, &base, &facts, &NoPredicates, &narrowed, 10).is_err());
    base.get_mut(&UnitGrantId(1)).expect("excluded").status = UnitGrantStatus::Revoked;
    assert!(allowed(&orgs, &base, &facts, &request(LENDER, UNIT)));
}

#[test]
fn exclusion_in_a_disjoint_compartment_does_not_veto_other_control() {
    let mut orgs = orgs();
    orgs.put_grant(org_grant(1, CAP, true));
    orgs.put_scope(OrgScope {
        id: OrgScopeId(12),
        org: OrgId(1),
        parent: Some(OrgScopeId(10)),
        name: "sibling".into(),
        status: ScopeStatus::Active,
        depth: 1,
        created_rev: 1,
        revision: 1,
    });
    let mut facts = facts();
    org_owned(&mut facts, UNIT, 11);
    let mut exclusion = wildcard(1, PILOT);
    exclusion.scope = UnitScope::OrganizationControlled {
        organization: OrgId(1),
        scope: ScopeSelector::Exact(OrgScopeId(12)),
        excluded: [UNIT].into_iter().collect(),
    };
    let grants = book([exclusion, wildcard(2, PILOT)]);
    assert!(allowed(&orgs, &grants, &facts, &request(PILOT, UNIT)));
    facts.assets.get_mut(&UNIT).expect("asset").scope = Some(OrgScopeId(12));
    assert!(!allowed(&orgs, &grants, &facts, &request(PILOT, UNIT)));
}

#[test]
fn proof_budget_is_shared_across_finite_units_and_empty_memberships() {
    let mut orgs = orgs();
    orgs.put_grant(org_grant(1, CAP, true));
    let mut facts = facts();
    let units: BTreeSet<_> = (100..164).map(ObjId).collect();
    for unit in &units {
        facts.assets.insert(
            *unit,
            UnitAssetFacts {
                unit: *unit,
                owner: PrincipalId::Organization(OrgId(1)),
                lineage: lineage(10),
                scope: Some(OrgScopeId(11)),
                active: true,
            },
        );
    }
    let mut candidate = wildcard(1, PILOT);
    candidate.scope = UnitScope::Units {
        units: units.iter().copied().take(63).collect(),
    };
    assert!(
        validate_unit_grant(
            &orgs,
            &BTreeMap::new(),
            &facts,
            &NoPredicates,
            &candidate,
            10
        )
        .is_ok()
    );
    candidate.scope = UnitScope::Units { units };
    assert_eq!(
        validate_unit_grant(
            &orgs,
            &BTreeMap::new(),
            &facts,
            &NoPredicates,
            &candidate,
            10
        ),
        Err(UnitGrantError::Capacity)
    );
    for id in 100..229 {
        orgs.put_membership(OrgMembership {
            org: OrgId(id),
            member: PILOT,
            status: MembershipStatus::Active,
            titles: vec![],
            roles: BTreeSet::new(),
            visibility: Visibility::Public,
            since: 0,
            revision: 1,
        });
    }
    assert_eq!(
        authorize_unit(
            &orgs,
            &BTreeMap::new(),
            &facts,
            &NoPredicates,
            &request(PILOT, UNIT)
        ),
        UnitDecision::Deny(UnitGrantError::Capacity)
    );
}

#[test]
fn policy_records_rules_and_conditions_share_the_proof_work_budget() {
    let mut facts = facts();
    org_owned(&mut facts, UNIT, 11);
    for mode in 0..3 {
        let mut orgs = orgs();
        orgs.put_grant(org_grant(1, CAP, true));
        let mut rule = PolicyRule {
            effect: PolicyEffect::Deny,
            subject: PolicySubject::Principal(PrincipalId::Object(ObjId(4))),
            capability: CAP.capability_id().expect("cap"),
            scope: ScopeSelector::SubtreeLive(OrgScopeId(10)),
            conditions: vec![],
        };
        if mode == 2 {
            rule.conditions = vec![
                PolicyCondition::TimeWindow {
                    from: None,
                    until: None
                };
                129
            ];
        }
        let rules = if mode == 1 {
            vec![rule; 129]
        } else if mode == 2 {
            vec![rule]
        } else {
            vec![]
        };
        for id in 1..=if mode == 0 { 129 } else { 1 } {
            orgs.put_policy(AccessPolicy {
                id: PolicyId(id),
                org: OrgId(1),
                owner: PrincipalId::Organization(OrgId(1)),
                rules: rules.clone(),
                default: PolicyEffect::Deny,
                revision: 1,
            });
        }
        assert_eq!(
            authorize_unit(
                &orgs,
                &book([wildcard(1, PILOT)]),
                &facts,
                &NoPredicates,
                &request(PILOT, UNIT)
            ),
            UnitDecision::Deny(UnitGrantError::Capacity),
            "mode {mode}"
        );
    }
}

#[test]
fn personal_loan_to_organization_uses_recipient_compartment_and_live_parent_rights() {
    let mut orgs = orgs();
    orgs.put_grant(org_grant(1, CAP, true));
    let facts = facts();
    let mut organizational_loan = loan(1, LENDER);
    organizational_loan.grantee = UnitGrantee::Organization {
        organization: OrgId(1),
        scope: OrgScopeId(11),
    };
    let onward = wildcard(2, PILOT);
    let grants = book([organizational_loan.clone(), onward]);
    assert!(allowed(&orgs, &grants, &facts, &request(PILOT, UNIT)));
    assert_eq!(
        facts.asset(UNIT).expect("asset").owner,
        PrincipalId::Object(OWNER)
    );
    for change in 0..3 {
        let mut grants = grants.clone();
        let loan = grants.get_mut(&UnitGrantId(1)).expect("loan");
        match change {
            0 => loan.delegable = false,
            1 => loan.status = UnitGrantStatus::Revoked,
            _ => {
                loan.grantee = UnitGrantee::Organization {
                    organization: OrgId(1),
                    scope: OrgScopeId(21),
                }
            }
        }
        assert!(
            !allowed(&orgs, &grants, &facts, &request(PILOT, UNIT)),
            "change {change}"
        );
    }
    organizational_loan.parent = UnitGrantParent::OrganizationGrant {
        organization: OrgId(1),
        grant: GrantId(1),
    };
    organizational_loan.recovery = RecoveryPolicy::None;
    let circular = book([organizational_loan, wildcard(2, PILOT)]);
    assert!(
        !allowed(&orgs, &circular, &facts, &request(PILOT, UNIT)),
        "org cannot derive its own control from itself"
    );
}

#[test]
fn org_policy_vetoes_every_grantor_and_final_recipient() {
    let mut orgs = orgs();
    orgs.put_grant(org_grant(1, CAP, true));
    let mut facts = facts();
    org_owned(&mut facts, UNIT, 11);
    let mut second = child(2, LENDER, PILOT, 1);
    second.recovery = RecoveryPolicy::None;
    let grants = book([wildcard(1, LENDER), second]);
    assert!(allowed(&orgs, &grants, &facts, &request(PILOT, UNIT)));
    for subject in [OWNER, LENDER, PILOT] {
        orgs.put_policy(AccessPolicy {
            id: PolicyId(1),
            org: OrgId(1),
            owner: PrincipalId::Organization(OrgId(1)),
            rules: vec![PolicyRule {
                effect: PolicyEffect::Deny,
                subject: PolicySubject::Principal(PrincipalId::Object(subject)),
                capability: CAP.capability_id().expect("cap"),
                scope: ScopeSelector::Exact(OrgScopeId(11)),
                conditions: vec![],
            }],
            default: PolicyEffect::Deny,
            revision: 1,
        });
        assert!(
            !allowed(&orgs, &grants, &facts, &request(PILOT, UNIT)),
            "denied {subject:?}"
        );
    }
}

#[test]
fn mission_grant_follows_human_succession_only_for_exact_accepted_binding_and_term() {
    let orgs = OrgState::new();
    let mut facts = facts();
    let binding = MissionBinding {
        mission: 4,
        detachment: 5,
        roster_revision: 2,
        accepted_roster_hash: [7; 32],
    };
    facts.missions.insert(
        binding.clone(),
        MissionFacts {
            binding: binding.clone(),
            active: true,
            units: [UNIT].into_iter().collect(),
            leader: Some(MissionLeader::Player(PILOT)),
            term: 1,
        },
    );
    let mut grant = loan(1, PILOT);
    grant.capability = UnitCapability::CommandAssignedAi;
    grant.delegable = false;
    grant.grantee = UnitGrantee::CurrentLeader(binding.clone());
    let grants = book([grant]);
    let mut req = request(PILOT, UNIT);
    req.action = UnitAction::Exercise(UnitCapability::CommandAssignedAi);
    req.mission = Some(MissionCommand {
        binding: binding.clone(),
        term: 1,
    });
    assert!(allowed(&orgs, &grants, &facts, &req));
    let mission = facts.missions.get_mut(&binding).expect("mission");
    mission.leader = Some(MissionLeader::Npc(LENDER));
    mission.term = 2;
    assert!(!allowed(&orgs, &grants, &facts, &req));
    req.actor = LENDER;
    req.mission = Some(MissionCommand {
        binding: binding.clone(),
        term: 2,
    });
    assert!(allowed(&orgs, &grants, &facts, &req));
    facts
        .missions
        .get_mut(&binding)
        .expect("mission")
        .binding
        .accepted_roster_hash = [8; 32];
    assert!(!allowed(&orgs, &grants, &facts, &req));
    facts.missions.get_mut(&binding).expect("mission").binding = binding.clone();
    facts.missions.get_mut(&binding).expect("mission").leader = None;
    assert!(!allowed(&orgs, &grants, &facts, &req));
}

#[test]
fn record_decode_is_canonical_bounded_and_requires_explicit_windows_and_policy() {
    let grant = loan(1, PILOT);
    let value = serde_json::to_value(&grant).expect("encode");
    assert_eq!(
        serde_json::from_value::<UnitGrant>(value.clone()).expect("decode"),
        grant
    );
    for key in ["valid_from", "valid_until", "recovery"] {
        let mut damaged = value.clone();
        damaged.as_object_mut().expect("object").remove(key);
        assert!(
            serde_json::from_value::<UnitGrant>(damaged).is_err(),
            "missing {key}"
        );
    }
    assert!(serde_json::from_str::<UnitScope>(r#"{"Units":{"units":[20,20]}}"#).is_err());
    assert!(serde_json::from_str::<UnitScope>(r#"{"Units":{"units":[21,20]}}"#).is_err());
    assert!(serde_json::from_str::<UnitScope>(r#"{"Units":{"units":[],"extra":true}}"#).is_err());
    let oversized = serde_json::json!({"Units":{"units":(1..=257).collect::<Vec<_>>()}});
    assert!(serde_json::from_value::<UnitScope>(oversized).is_err());
    let unknown_selector = serde_json::json!({"OrganizationControlled":{"organization":1,
        "scope":{"SubtreeFrozen":{"root":10,"snapshot_revision":1,"extra":true}},"excluded":[]}});
    assert!(serde_json::from_value::<UnitScope>(unknown_selector).is_err());
}

#[test]
fn retained_recovery_needs_real_exact_custody_and_never_comes_from_a_grant() {
    let mut facts = facts();
    let req = RecoveryRequest {
        actor: PILOT,
        unit: UNIT,
        hull_epoch: 4,
        action: RecoveryAction::SafeStop,
    };
    assert!(!authorize_recovery(&facts, &req));
    facts.custody = Some(RecoveryCustodyFacts {
        unit: UNIT,
        operator: PILOT,
        hull_epoch: 4,
        origin: ObjId(80),
        berth: ObjId(81),
        policy: RecoveryPolicy::ReturnOnly,
        active: true,
    });
    assert!(authorize_recovery(&facts, &req));
    assert!(!authorize_recovery(
        &facts,
        &RecoveryRequest {
            hull_epoch: 5,
            ..req.clone()
        }
    ));
    assert!(!authorize_recovery(
        &facts,
        &RecoveryRequest {
            action: RecoveryAction::ReturnToRecordedOrigin { origin: ObjId(82) },
            ..req.clone()
        }
    ));
    assert!(authorize_recovery(
        &facts,
        &RecoveryRequest {
            action: RecoveryAction::RecoverIntoRecordedBerth { berth: ObjId(81) },
            ..req
        }
    ));
}
