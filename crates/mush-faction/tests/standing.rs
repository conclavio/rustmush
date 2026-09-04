//! E1c acceptance: the faction-standing predicate provider bridges live faction
//! standing into org access policies — directly (the provider resolves member /
//! officer / intel against `FactionState`) and end-to-end (an org `AccessPolicy`
//! gated on `faction:officer` authorizes an officer through `authorize_in_world`,
//! and fails closed otherwise). The faction authorization model stays faction-side.

use std::collections::HashMap;
use std::sync::Arc;

use mush_core::{ModuleBootData, ObjId};
use mush_faction::{
    Faction, FactionId, FactionMembership, FactionModule, FactionStandingProvider, FactionState,
    Visibility, fac_key, member_key,
};
use mush_org::{
    AccessPolicy, AuthRequest, CapabilityId, OrgId, OrgModule, OrgPredicateProvider, OrgScope,
    OrgScopeId, OrgStatus, Organization, PolicyEffect, PolicyId, PolicyRule, PolicySubject,
    PrincipalId, RulesValue, ScopeSelector, ScopeStatus, authorize_in_world, org_key, policy_key,
    scope_key,
};
use mush_testkit::TestWorld;
use smol_str::SmolStr;

fn cap(s: &str) -> CapabilityId {
    CapabilityId::new(s).unwrap()
}
fn membership(who: ObjId, faction: FactionId, level: u8) -> FactionMembership {
    FactionMembership {
        character: who,
        faction,
        title: SmolStr::default(),
        level,
        visibility: Visibility::Public,
        since: 0,
    }
}

#[test]
fn provider_resolves_member_officer_intel_and_fails_closed() {
    let mut tw = TestWorld::with_modules(vec![Box::new(FactionModule::new())]);
    let officer = tw.spawn_player("Ophelia");
    let grunt = tw.spawn_player("Gary");
    let outsider = tw.spawn_player("Xena");
    let fid = {
        let st = tw
            .world_mut()
            .module_state_mut::<FactionState>()
            .expect("faction state");
        let fid = st.alloc_faction_id();
        let mut f = Faction::new(fid, "Camarilla");
        f.officer_level = 2;
        f.intel_level = 2;
        st.factions.insert(fid, f);
        st.members
            .insert((officer, fid), membership(officer, fid, 3)); // officer + intel
        st.members.insert((grunt, fid), membership(grunt, fid, 1)); // member only
        fid
    };
    let p = FactionStandingProvider;
    let arg = serde_json::json!(fid.0);
    assert_eq!(p.provider(), "faction");
    // member
    assert_eq!(p.eval(tw.world(), "member", &arg, officer), Some(true));
    assert_eq!(p.eval(tw.world(), "member", &arg, grunt), Some(true));
    assert_eq!(p.eval(tw.world(), "member", &arg, outsider), Some(false));
    // officer (level >= officer_level 2)
    assert_eq!(p.eval(tw.world(), "officer", &arg, officer), Some(true));
    assert_eq!(
        p.eval(tw.world(), "officer", &arg, grunt),
        Some(false),
        "level 1 < officer 2"
    );
    // intel (level >= intel_level 2)
    assert_eq!(p.eval(tw.world(), "intel", &arg, officer), Some(true));
    assert_eq!(
        p.eval(tw.world(), "intel", &arg, grunt),
        Some(false),
        "level 1 < intel 2"
    );
    // The Principal wire form ({"faction": id}) is also accepted.
    assert_eq!(
        p.eval(
            tw.world(),
            "officer",
            &serde_json::json!({ "faction": fid.0 }),
            officer
        ),
        Some(true)
    );
    // Fail-closed: unknown faction, unknown predicate, bad argument -> None.
    assert_eq!(
        p.eval(tw.world(), "officer", &serde_json::json!(999), officer),
        None,
        "unknown faction"
    );
    assert_eq!(
        p.eval(tw.world(), "sovereign", &arg, officer),
        None,
        "unknown predicate"
    );
    assert_eq!(
        p.eval(tw.world(), "member", &serde_json::json!("nope"), officer),
        None,
        "bad argument"
    );
}

#[test]
fn org_policy_gates_on_live_faction_standing_end_to_end() {
    // Both modules loaded; the OrgModule carries the faction-standing provider.
    let mut tw = TestWorld::with_modules(vec![
        Box::new(OrgModule::with_predicates(vec![Arc::new(
            FactionStandingProvider,
        )])),
        Box::new(FactionModule::new()),
    ]);
    let officer = tw.spawn_player("Ophelia");
    let outsider = tw.spawn_player("Xena");

    let fid = FactionId(0);
    let mut fac = Faction::new(fid, "Camarilla");
    fac.officer_level = 2;
    let faction_kv = vec![
        (fac_key(fid), serde_json::to_value(&fac).unwrap()),
        (
            member_key(officer, fid),
            serde_json::to_value(membership(officer, fid, 3)).unwrap(),
        ),
    ];

    // An org whose scope-11.. no: a root scope, with a policy allowing `org.view`
    // only to a `faction:officer` of faction 0.
    let (org, root) = (OrgId(0), OrgScopeId(0));
    let org_rec = Organization {
        id: org,
        name: "Ivory Tower".into(),
        aliases: vec![],
        status: OrgStatus::Active,
        hidden: false,
        charter: String::new(),
        root_scope: root,
        revision: 1,
        ruleset_ext: RulesValue::default(),
    };
    let root_rec = OrgScope {
        id: root,
        org,
        parent: None,
        name: "root".into(),
        status: ScopeStatus::Active,
        depth: 0,
        created_rev: 1,
        revision: 1,
    };
    let policy = AccessPolicy {
        id: PolicyId(0),
        org,
        owner: PrincipalId::Organization(org),
        default: PolicyEffect::Deny,
        revision: 1,
        rules: vec![PolicyRule {
            effect: PolicyEffect::Allow,
            subject: PolicySubject::RegisteredPredicate {
                provider: "faction".into(),
                predicate: "officer".into(),
                arguments: serde_json::json!(fid.0),
            },
            capability: cap("org.view"),
            scope: ScopeSelector::Exact(root),
            conditions: vec![],
        }],
    };
    let org_kv = vec![
        (org_key(org), serde_json::to_value(&org_rec).unwrap()),
        (scope_key(root), serde_json::to_value(&root_rec).unwrap()),
        (
            policy_key(PolicyId(0)),
            serde_json::to_value(&policy).unwrap(),
        ),
    ];

    tw.modules_loaded(HashMap::from([
        (
            SmolStr::new("faction"),
            ModuleBootData {
                kv: faction_kv,
                streams: HashMap::new(),
            },
        ),
        (
            SmolStr::new("org"),
            ModuleBootData {
                kv: org_kv,
                streams: HashMap::new(),
            },
        ),
    ]));

    let now = tw.world().now();
    let ask = |who: ObjId| AuthRequest {
        actor: who,
        capability: cap("org.view"),
        scope: root,
        now,
        amount: None,
    };
    // The officer is authorized through the faction-standing predicate...
    assert!(
        authorize_in_world(tw.world(), &ask(officer)).is_allow(),
        "faction officer authorized"
    );
    // ...the outsider (not a member, so not an officer) is denied.
    assert!(
        !authorize_in_world(tw.world(), &ask(outsider)).is_allow(),
        "non-officer denied"
    );
}
