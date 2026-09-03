//! E1b acceptance: the `OrgModule` attaches the pure E1a engine to a real world —
//! staff create/inspect, the world authorize seam, per-record persistence + reboot
//! round-trips, fail-closed boot (container-fatal / leaf-skippable / orphan drop),
//! the destroy scrub, monotonic allocators, and the audit trail. Driven through the
//! real Engine via `TestWorld`.

use std::collections::HashMap;

use mush_core::{
    Audience, Effects, GOD, ModuleBootData, ModuleWrite, ObjId, ObjKind, Outbox, WorldEvent,
};
use mush_org::{
    AuthRequest, CapabilityId, OrgId, OrgScopeId, OrgStore, authorize_in_world, grant_key,
};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

const NS: &str = "org";

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![Box::new(mush_org::OrgModule::new())])
}
fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events.iter().map(|(_, e)| text_of(e)).collect::<Vec<_>>().join("\n")
}
fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}
fn store(tw: &TestWorld) -> &OrgStore {
    tw.world().module_state::<OrgStore>().expect("org store")
}
/// The single org created in a test + its root scope.
fn only_org(tw: &TestWorld) -> (OrgId, OrgScopeId) {
    let o = store(tw).state().orgs().next().expect("one org");
    (o.id, o.root_scope)
}
fn cap(s: &str) -> CapabilityId {
    CapabilityId::new(s).unwrap()
}
fn req(actor: ObjId, capability: &str, scope: OrgScopeId, now: i64) -> AuthRequest {
    AuthRequest { actor, capability: cap(capability), scope, now, amount: None }
}
fn allow(tw: &TestWorld, actor: ObjId, capability: &str, scope: OrgScopeId) -> bool {
    authorize_in_world(tw.world(), &req(actor, capability, scope, tw.world().now())).is_allow()
}

/// Fold the org module's persist ops into a kv snapshot for a reboot.
fn fold_kv(tw: &mut TestWorld) -> Vec<(SmolStr, serde_json::Value)> {
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for op in tw.take_module_ops() {
        if op.ns != NS {
            continue;
        }
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| *k != key);
                kv.push((key, value));
            }
            ModuleWrite::Del { key } => kv.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv
}
/// Reboot into a fresh world. `players` are respawned (in order) BEFORE the module
/// boots, mirroring the real server where the object store restores objects before
/// modules load — object ids are deterministic, so a grantee keeps its id and its
/// grant survives the boot liveness reconciliation.
fn reboot(kv: Vec<(SmolStr, serde_json::Value)>, players: &[&str]) -> TestWorld {
    let mut tw = boot();
    for name in players {
        tw.spawn_player(name);
    }
    tw.modules_loaded(HashMap::from([(SmolStr::new(NS), ModuleBootData { kv, streams: HashMap::new() })]));
    tw
}
fn del_keys(tw: &mut TestWorld) -> Vec<String> {
    tw.take_module_ops()
        .iter()
        .filter_map(|o| match &o.op {
            ModuleWrite::Del { key } if o.ns == NS => Some(key.to_string()),
            _ => None,
        })
        .collect()
}
/// Load boot data through the Engine directly so we can observe a boot rejection
/// (TestWorld::modules_loaded panics on it).
fn try_boot(tw: &mut TestWorld, kv: Vec<(SmolStr, serde_json::Value)>) -> Result<(), String> {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut()
        .modules_loaded(
            HashMap::from([(SmolStr::new(NS), ModuleBootData { kv, streams: HashMap::new() })]),
            &mut out,
            &mut fx,
        )
        .map_err(|e| format!("{e:?}"))
}

// ----------------------------------------------------------------------

#[test]
fn create_persists_and_authorizes_through_the_world_seam() {
    let mut tw = boot();
    let joe = tw.spawn_player("Joe");
    let ev = tw.run(GOD, "@org/create Camarilla");
    assert!(!has_error(&ev), "create refused: {}", texts(&ev));
    let (org, root) = only_org(&tw);
    // The org + root scope + meta persisted under ns org.
    let ops = tw.take_module_ops();
    assert!(ops.iter().any(|o| o.ns == NS && matches!(&o.op, ModuleWrite::JsonSet { key, .. } if *key == mush_org::org_key(org))));
    assert!(ops.iter().any(|o| o.ns == NS && matches!(&o.op, ModuleWrite::JsonSet { key, .. } if *key == mush_org::scope_key(root))));
    assert!(ops.iter().any(|o| o.ns == NS && matches!(&o.op, ModuleWrite::JsonSet { key, .. } if key == "meta")));

    // No authority yet.
    assert!(!allow(&tw, joe, "org.view", root));
    // Grant it, then the world seam authorizes Joe on the org's scope tree.
    let ev = tw.run(GOD, &format!("@org/grant #{}={} org.view", joe.0, org));
    assert!(!has_error(&ev), "grant refused: {}", texts(&ev));
    assert!(allow(&tw, joe, "org.view", root), "granted actor authorized");
    // ...but not a different actor, nor a different capability.
    let jane = tw.spawn_player("Jane");
    assert!(!allow(&tw, jane, "org.view", root), "ungranted actor denied");
    assert!(!allow(&tw, joe, "org.transfer", root), "ungranted capability denied");
}

#[test]
fn org_and_grant_survive_reboot() {
    let mut tw = boot();
    let joe = tw.spawn_player("Joe");
    tw.run(GOD, "@org/create Camarilla");
    let (org, root) = only_org(&tw);
    tw.run(GOD, &format!("@org/grant #{}={} org.view", joe.0, org));
    assert!(allow(&tw, joe, "org.view", root));

    let kv = fold_kv(&mut tw);
    let tw2 = reboot(kv, &["Joe"]);
    // Same ids, org present, authority intact after reboot.
    let (org2, root2) = only_org(&tw2);
    assert_eq!((org, root), (org2, root2), "deterministic ids across reboot");
    assert!(allow(&tw2, joe, "org.view", root2), "grant authority restored");
}

#[test]
fn destroying_a_grantee_scrubs_authority_and_deletes_its_grant() {
    let mut tw = boot();
    let joe = tw.spawn_player("Joe");
    tw.run(GOD, "@org/create Camarilla");
    let (org, root) = only_org(&tw);
    tw.run(GOD, &format!("@org/grant #{}={} org.view", joe.0, org));
    assert!(allow(&tw, joe, "org.view", root));
    // The one grant's id (for the deletion assert).
    let grant_id = store(&tw).state().grants_in(org).next().expect("a grant").id;
    tw.take_module_ops();

    tw.world_mut().destroy(joe).unwrap();
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().emit_world_event(&WorldEvent::ObjDestroyed { obj: joe }, &mut out, &mut fx);

    assert!(!allow(&tw, joe, "org.view", root), "authority scrubbed on destroy");
    assert!(store(&tw).state().grants_in(org).next().is_none(), "grant removed from state");
    assert!(del_keys(&mut tw).contains(&grant_key(grant_id).to_string()), "grant doc deleted");
}

#[test]
fn boot_fails_on_a_corrupt_org_container() {
    let mut tw = boot();
    // A well-keyed org doc that does not parse: the container polarity FAILS boot.
    let kv = vec![(SmolStr::new("org:0"), serde_json::json!({ "not": "an org" }))];
    let res = try_boot(&mut tw, kv);
    assert!(res.is_err(), "corrupt org container must fail the boot: {res:?}");
    assert!(res.unwrap_err().contains("bad org doc"), "diagnostic names the bad doc");
}

#[test]
fn boot_skips_and_deletes_a_corrupt_leaf() {
    let mut tw = boot();
    tw.run(GOD, "@org/create Camarilla");
    let mut kv = fold_kv(&mut tw);
    // Inject an unparseable grant leaf next to the healthy org/scope/meta.
    kv.push((SmolStr::new("grant:999"), serde_json::json!("not a grant")));
    let tw2 = {
        let mut t = boot();
        t.modules_loaded(HashMap::from([(SmolStr::new(NS), ModuleBootData { kv, streams: HashMap::new() })]));
        t
    };
    // The org still loaded (leaf corruption is not fatal)...
    assert_eq!(store(&tw2).state().orgs().count(), 1, "healthy org survived a corrupt leaf");
    // ...and the corrupt leaf key was deleted at boot.
    let mut tw2 = tw2;
    assert!(del_keys(&mut tw2).contains(&"grant:999".to_string()), "corrupt leaf deleted by raw key");
}

#[test]
fn boot_drops_an_orphan_grant_whose_org_is_missing() {
    // A grant doc whose org was never loaded is an orphan: dropped + deleted, boot OK.
    let mut tw = boot();
    let grant = serde_json::json!({
        "id": 7, "org": 42, "grantee": 100, "capability": "org.view",
        "scope": { "SubtreeLive": 0 }, "limit": null, "delegable": false,
        "valid_from": null, "valid_until": null, "revision": 1
    });
    let kv = vec![(SmolStr::new("grant:7"), grant)];
    let res = try_boot(&mut tw, kv);
    assert!(res.is_ok(), "an orphan leaf must NOT fail boot: {res:?}");
    assert!(store(&tw).state().grant(mush_org::GrantId(7)).is_none(), "orphan grant dropped");
    assert!(del_keys(&mut tw).contains(&"grant:7".to_string()), "orphan grant doc deleted");
}

#[test]
fn allocators_resume_and_never_reuse_ids() {
    let mut tw = boot();
    tw.run(GOD, "@org/create Alpha");
    tw.run(GOD, "@org/create Beta");
    let ids_before: Vec<u64> = store(&tw).state().orgs().map(|o| o.id.0).collect();
    assert_eq!(ids_before, vec![0, 1], "monotonic org ids");

    let kv = fold_kv(&mut tw);
    let mut tw2 = reboot(kv, &["Joe"]);
    tw2.run(GOD, "@org/create Gamma");
    let ids_after: Vec<u64> = store(&tw2).state().orgs().map(|o| o.id.0).collect();
    assert_eq!(ids_after, vec![0, 1, 2], "third org gets a fresh id after reboot (no reuse)");
}

#[test]
fn commands_are_staff_only() {
    let mut tw = boot();
    tw.run(GOD, "@org/create Camarilla");
    let (org, _root) = only_org(&tw);
    let joe = tw.spawn_player("Joe"); // ordinary player
    assert!(has_error(&tw.run(joe, "@org/create Sabbat")), "create refused for non-staff");
    assert!(has_error(&tw.run(joe, "@org/list")), "list refused for non-staff");
    assert!(has_error(&tw.run(joe, &format!("@org/show #{org}"))), "show refused for non-staff");
    assert!(has_error(&tw.run(joe, &format!("@org/grant #{}={} org.view", joe.0, org))), "grant refused");
    assert_eq!(store(&tw).state().orgs().count(), 1, "no org created by a non-staff actor");
}

#[test]
fn audit_records_actions_and_survives_reboot() {
    let mut tw = boot();
    let joe = tw.spawn_player("Joe");
    tw.run(GOD, "@org/create Camarilla");
    let (org, _root) = only_org(&tw);
    tw.run(GOD, &format!("@org/grant #{}={} org.view", joe.0, org));
    // The audit shows both actions.
    let t = texts(&tw.run(GOD, &format!("@org/audit #{org}")));
    assert!(t.contains("org.create"), "audit has the create: {t}");
    assert!(t.contains("grant.create"), "audit has the grant: {t}");

    // Survives reboot (the ledger tail rehydrates).
    let kv = fold_kv(&mut tw);
    let mut tw2 = reboot(kv, &["Joe"]);
    let t2 = texts(&tw2.run(GOD, &format!("@org/audit #{org}")));
    assert!(t2.contains("org.create") && t2.contains("grant.create"), "audit restored after reboot: {t2}");
}

#[test]
fn show_lists_the_org_structure() {
    let mut tw = boot();
    let joe = tw.spawn_player("Joe");
    tw.run(GOD, "@org/create Camarilla");
    let (org, _root) = only_org(&tw);
    tw.run(GOD, &format!("@org/grant #{}={} org.view", joe.0, org));
    let t = texts(&tw.run(GOD, &format!("@org/show #{org}")));
    assert!(t.contains("Camarilla") && t.contains("scope #") && t.contains("grant #"), "{t}");
}

#[test]
fn boot_drops_a_delegation_whose_grantor_was_destroyed() {
    // The confirmed E1b verify hole: if the grantor object was destroyed while the
    // org observer did not run (module unloaded, or a programmatic destroy), its
    // grant + delegation docs linger. Boot must reconcile against the live world and
    // drop them, or the surviving grantee keeps delegated authority that died with
    // the grantor (`src.grantee == d.grantor`, both the same dead id, would pass).
    use mush_org::{
        CapabilityGrant, Delegation, DelegationId, DelegationStatus, GrantId, OrgScope, OrgStatus,
        Organization, RulesValue, ScopeSelector, ScopeStatus,
    };
    let mut tw = boot();
    let y = tw.spawn_player("Yara"); // live grantee
    let x = ObjId(9999); // grantor: never created => dead
    let org = OrgId(0);
    let root = OrgScopeId(0);
    let org_rec = Organization {
        id: org, name: "Camarilla".into(), aliases: vec![], status: OrgStatus::Active,
        hidden: false, charter: String::new(), root_scope: root, revision: 1, ruleset_ext: RulesValue::default(),
    };
    let root_rec = OrgScope {
        id: root, org, parent: None, name: "root".into(), status: ScopeStatus::Active,
        depth: 0, created_rev: 1, revision: 1,
    };
    let grant = CapabilityGrant {
        id: GrantId(0), org, grantee: x, capability: cap("org.view"),
        scope: ScopeSelector::SubtreeLive(root), limit: None, delegable: true,
        valid_from: None, valid_until: None, revision: 1,
    };
    let del = Delegation {
        id: DelegationId(0), org, grantor: x, grantee: y, source_grant: GrantId(0),
        capability: cap("org.view"), scope: ScopeSelector::Exact(root), limit: None,
        expires_at: i64::MAX, source_revision: 1, status: DelegationStatus::Active,
    };
    let kv = vec![
        (mush_org::org_key(org), serde_json::to_value(&org_rec).unwrap()),
        (mush_org::scope_key(root), serde_json::to_value(&root_rec).unwrap()),
        (grant_key(GrantId(0)), serde_json::to_value(&grant).unwrap()),
        (mush_org::delegation_key(DelegationId(0)), serde_json::to_value(&del).unwrap()),
    ];
    let res = try_boot(&mut tw, kv);
    assert!(res.is_ok(), "boot succeeds, dropping the stale bindings: {res:?}");
    assert!(store(&tw).state().grant(GrantId(0)).is_none(), "dead-grantee grant dropped");
    assert!(store(&tw).state().delegation(DelegationId(0)).is_none(), "dead-grantor delegation dropped");
    // The org itself survives; the surviving grantee Y holds NOTHING.
    assert_eq!(store(&tw).state().orgs().count(), 1, "the org is intact");
    assert!(!allow(&tw, y, "org.view", root), "surviving grantee gains no delegated authority");
    let dels = del_keys(&mut tw);
    assert!(dels.contains(&grant_key(GrantId(0)).to_string()), "stale grant doc deleted");
    assert!(dels.contains(&mush_org::delegation_key(DelegationId(0)).to_string()), "stale delegation doc deleted");
}

#[test]
fn a_new_object_reusing_no_id_is_unaffected_by_a_prior_scrub() {
    // Destroy a grantee, then an unrelated object destroy leaves state intact.
    let mut tw = boot();
    let joe = tw.spawn_player("Joe");
    tw.run(GOD, "@org/create Camarilla");
    let (org, root) = only_org(&tw);
    tw.run(GOD, &format!("@org/grant #{}={} org.view", joe.0, org));
    let rock = tw.world_mut().create(ObjKind::Thing, "Rock", GOD, None).unwrap();
    tw.take_module_ops();
    tw.world_mut().destroy(rock).unwrap();
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().emit_world_event(&WorldEvent::ObjDestroyed { obj: rock }, &mut out, &mut fx);
    assert!(allow(&tw, joe, "org.view", root), "unrelated destroy leaves authority intact");
    assert!(del_keys(&mut tw).is_empty(), "no org doc deleted for an unrelated object");
}

// ======================================================================
// E3b2 (§4.8) — scopes, the capability vocabulary, the scope-use binder
// ======================================================================

/// A binder that reports a fixed set of scopes as "still bound" — the shape
/// `mush-economy` fills in (org cannot see accounts/assets/vaults itself).
#[derive(Debug)]
struct StubBinder(Vec<OrgScopeId>);
impl mush_org::ScopeUseProvider for StubBinder {
    fn binder(&self) -> &str {
        "stub"
    }
    fn scope_in_use(&self, _w: &mush_core::World, scope: OrgScopeId) -> Option<String> {
        self.0.contains(&scope).then(|| format!("account #7 binds scope #{scope}"))
    }
}

fn vocab(caps: &[&str]) -> std::collections::BTreeSet<CapabilityId> {
    caps.iter().map(|c| CapabilityId::new(c).expect("cap")).collect()
}
fn boot_with_vocab(caps: &[&str]) -> TestWorld {
    TestWorld::with_modules(vec![Box::new(mush_org::OrgModule::new().with_capabilities(vocab(caps)))])
}

#[test]
fn scope_create_and_retire_through_the_command_surface() {
    let mut tw = boot();
    assert!(!has_error(&tw.run(GOD, "@org/create Camarilla")));
    let (org, root) = only_org(&tw);

    let ev = tw.run(GOD, "@org/scope create Camarilla=logistics");
    assert!(!has_error(&ev), "scope create refused: {}", texts(&ev));
    let child = store(&tw)
        .state()
        .scopes_in(org)
        .find(|s| s.name == "logistics")
        .expect("the new scope")
        .id;
    assert_eq!(store(&tw).state().scope(child).expect("scope").parent, Some(root));
    assert_eq!(store(&tw).state().scope(child).expect("scope").depth, 1);

    // A nested one under the child.
    let ev = tw.run(GOD, &format!("@org/scope create Camarilla=depot parent=#{child}"));
    assert!(!has_error(&ev), "nested scope create refused: {}", texts(&ev));
    let depot = store(&tw).state().scopes_in(org).find(|s| s.name == "depot").expect("depot").id;
    assert_eq!(store(&tw).state().scope(depot).expect("scope").depth, 2);

    // A grant on the child authorizes; retiring it takes the authority away.
    let joe = tw.spawn_player("Joe");
    assert!(!has_error(
        &tw.run(GOD, &format!("@org/grant #{}=Camarilla economy.account.post scope=#{child}", joe.0))
    ));
    assert!(allow(&tw, joe, "economy.account.post", child));
    let ev = tw.run(GOD, &format!("@org/scope retire Camarilla=#{child}"));
    assert!(!has_error(&ev), "retire refused: {}", texts(&ev));
    assert!(!allow(&tw, joe, "economy.account.post", child), "a retired scope still authorized");

    // Retire is not a delete: the scope and the grant are both still there.
    assert!(store(&tw).state().scope(child).is_some());
    assert_eq!(store(&tw).state().grants_in(org).count(), 1);

    // The root can never be retired.
    let ev = tw.run(GOD, &format!("@org/scope retire Camarilla=#{root}"));
    assert!(has_error(&ev) && texts(&ev).contains("root scope"), "{}", texts(&ev));
}

#[test]
fn retire_survives_reboot_and_stays_retired() {
    let mut tw = boot();
    tw.run(GOD, "@org/create Camarilla");
    tw.run(GOD, "@org/scope create Camarilla=logistics");
    let (org, _) = only_org(&tw);
    let child = store(&tw).state().scopes_in(org).find(|s| s.name == "logistics").expect("scope").id;
    assert!(!has_error(&tw.run(GOD, &format!("@org/scope retire Camarilla=#{child}"))));
    let kv = fold_kv(&mut tw);
    let tw2 = reboot(kv, &[]);
    assert_eq!(
        store(&tw2).state().scope(child).expect("scope survives").status,
        mush_org::ScopeStatus::Retired
    );
}

#[test]
fn scope_retire_is_refused_while_a_binder_still_names_it() {
    let mut tw = TestWorld::with_modules(vec![Box::new(
        mush_org::OrgModule::new()
            .with_scope_use(vec![std::sync::Arc::new(StubBinder(vec![OrgScopeId(1)]))]),
    )]);
    tw.run(GOD, "@org/create Camarilla");
    tw.run(GOD, "@org/scope create Camarilla=logistics");
    let (org, _) = only_org(&tw);
    let child = store(&tw).state().scopes_in(org).find(|s| s.name == "logistics").expect("scope").id;
    assert_eq!(child, OrgScopeId(1), "the stub binder names scope #1");

    let ev = tw.run(GOD, &format!("@org/scope retire Camarilla=#{child}"));
    assert!(has_error(&ev), "a bound scope must not retire");
    assert!(texts(&ev).contains("still in use"), "{}", texts(&ev));
    assert_eq!(store(&tw).state().scope(child).expect("scope").status, mush_org::ScopeStatus::Active);
}

#[test]
fn grant_takes_an_exact_scope_and_a_per_operation_limit() {
    let mut tw = boot();
    let joe = tw.spawn_player("Joe");
    tw.run(GOD, "@org/create Camarilla");
    tw.run(GOD, "@org/scope create Camarilla=logistics");
    tw.run(GOD, "@org/scope create Camarilla=operations");
    let (org, root) = only_org(&tw);
    let logistics = store(&tw).state().scopes_in(org).find(|s| s.name == "logistics").expect("s").id;
    let operations = store(&tw).state().scopes_in(org).find(|s| s.name == "operations").expect("s").id;

    let ev = tw.run(
        GOD,
        &format!("@org/grant #{}=Camarilla economy.account.post scope=#{logistics} limit=500", joe.0),
    );
    assert!(!has_error(&ev), "grant refused: {}", texts(&ev));
    let g = store(&tw).state().grants_in(org).next().expect("grant").clone();
    assert_eq!(g.scope, mush_org::ScopeSelector::Exact(logistics));
    assert_eq!(g.limit.and_then(|l| l.max_amount), Some(500));

    // The exact selector does not reach the sibling or the root...
    let within = |scope, amount| {
        authorize_in_world(tw.world(), &AuthRequest {
            actor: joe,
            capability: cap("economy.account.post"),
            scope,
            now: tw.world().now(),
            amount: Some(amount),
        })
        .is_allow()
    };
    assert!(within(logistics, 500), "at the limit must pass");
    assert!(!within(logistics, 501), "over the limit must fail");
    assert!(!within(operations, 100), "an Exact selector reached a sibling");
    assert!(!within(root, 100), "an Exact selector reached the root");
    // ...and a limited grant denies an amount-free request (fail-closed).
    assert!(!allow(&tw, joe, "economy.account.post", logistics));

    // A limit is a PER-OPERATION ceiling, not a budget (A12): ten under-limit
    // requests all pass, forever.
    for _ in 0..10 {
        assert!(within(logistics, 500));
    }

    // A scope from another org is refused.
    tw.run(GOD, "@org/create Tremere");
    let other_root = store(&tw).state().orgs().find(|o| o.name == "Tremere").expect("org").root_scope;
    let ev = tw.run(
        GOD,
        &format!("@org/grant #{}=Camarilla economy.account.post scope=#{other_root}", joe.0),
    );
    assert!(has_error(&ev) && texts(&ev).contains("another organization"), "{}", texts(&ev));
}

#[test]
fn a_limit_is_refused_on_a_non_amount_capability() {
    let mut tw = boot();
    let joe = tw.spawn_player("Joe");
    tw.run(GOD, "@org/create Camarilla");
    let (org, root) = only_org(&tw);
    // A13: `vault.withdraw` requests carry no amount, so a limit would answer
    // AmountRequired forever — the grant would be inert, not narrow.
    let ev = tw.run(GOD, &format!("@org/grant #{}=Camarilla vault.withdraw scope=#{root} limit=5", joe.0));
    assert!(has_error(&ev), "a limit on a non-amount capability must refuse");
    assert!(texts(&ev).contains("amount-carrying"), "{}", texts(&ev));
    assert_eq!(store(&tw).state().grants_in(org).count(), 0, "the refused grant was stored anyway");
}

#[test]
fn an_unknown_capability_is_refused_when_a_vocabulary_is_wired() {
    let mut tw = boot_with_vocab(&["economy.account.post", "org.role.manage"]);
    let joe = tw.spawn_player("Joe");
    tw.run(GOD, "@org/create Camarilla");
    let (org, _) = only_org(&tw);

    let ev = tw.run(GOD, &format!("@org/grant #{}=Camarilla economy.acount.post", joe.0));
    assert!(has_error(&ev), "a typo-ed capability must refuse");
    assert!(texts(&ev).contains("Unknown capability"), "{}", texts(&ev));
    assert_eq!(store(&tw).state().grants_in(org).count(), 0);

    // A capability inside the vocabulary is granted normally.
    assert!(!has_error(&tw.run(GOD, &format!("@org/grant #{}=Camarilla org.role.manage", joe.0))));
    assert_eq!(store(&tw).state().grants_in(org).count(), 1);
}

#[test]
fn no_vocabulary_accepts_everything_and_show_flags_a_stray_grant() {
    // Empty vocabulary = the E1 behavior: any well-formed capability id is granted.
    let mut tw = boot();
    let joe = tw.spawn_player("Joe");
    tw.run(GOD, "@org/create Camarilla");
    assert!(!has_error(&tw.run(GOD, &format!("@org/grant #{}=Camarilla legacy.capability", joe.0))));
    let kv = fold_kv(&mut tw);

    // Reboot the SAME data under a vocabulary that no longer lists it: the grant is
    // not deleted, it is flagged — a shrinking vocabulary must never eat records.
    let mut tw2 = boot_with_vocab(&["economy.account.post"]);
    tw2.spawn_player("Joe");
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new(NS),
        ModuleBootData { kv, streams: HashMap::new() },
    )]));
    let (org, _) = only_org(&tw2);
    assert_eq!(store(&tw2).state().grants_in(org).count(), 1, "the stray grant was dropped");
    let ev = tw2.run(GOD, "@org/show Camarilla");
    assert!(
        texts(&ev).contains("UNKNOWN CAPABILITY"),
        "@org/show must flag a stray grant: {}",
        texts(&ev)
    );
}

#[test]
fn a_frozen_subtree_never_widens_to_a_scope_created_later() {
    let mut tw = boot();
    let joe = tw.spawn_player("Joe");
    tw.run(GOD, "@org/create Camarilla");
    let (org, root) = only_org(&tw);
    // A grant frozen at the org's revision AS IT IS NOW — the snapshot a caller
    // reading `Organization.revision` would take.
    let snapshot_revision = store(&tw).state().org(org).expect("org").revision;
    let grant_id = tw.world_mut().module_state_mut::<OrgStore>().expect("store").alloc_grant();
    let frozen = mush_org::CapabilityGrant {
        id: grant_id,
        org,
        grantee: joe,
        capability: cap("economy.account.post"),
        scope: mush_org::ScopeSelector::SubtreeFrozen { root, snapshot_revision },
        limit: None,
        delegable: false,
        valid_from: None,
        valid_until: None,
        revision: 1,
    };
    let mut probe_kv: Vec<(SmolStr, serde_json::Value)> = fold_kv(&mut tw);
    probe_kv.push((mush_org::grant_key(grant_id), serde_json::to_value(&frozen).expect("grant")));
    let mut tw = reboot(probe_kv, &["Joe"]);
    assert!(allow(&tw, joe, "economy.account.post", root), "the frozen subtree covers its own root");

    // A scope created AFTER the snapshot must not fall inside it.
    assert!(!has_error(&tw.run(GOD, "@org/scope create Camarilla=logistics")));
    let (org, _) = only_org(&tw);
    let child = store(&tw).state().scopes_in(org).find(|s| s.name == "logistics").expect("scope").id;
    assert!(
        !allow(&tw, joe, "economy.account.post", child),
        "a frozen subtree silently widened to a scope created after its snapshot"
    );
    assert!(store(&tw).state().scope(child).expect("scope").created_rev > snapshot_revision);
}
