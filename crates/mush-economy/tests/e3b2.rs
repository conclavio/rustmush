//! E3b2 acceptance (doc 08b §4.4–§4.8, §4.11 scenarios 8–12): the authorization /
//! approval half of the live economy module.
//!
//! The verify axis of this cut is **fail-open authorization**, so almost every test
//! here is a refusal that must hold and a disclosure that must not happen. Three
//! rhythms recur:
//!
//! - *the sibling division*: authority on one compartment is authority on exactly
//!   that compartment, and everything about the other one renders as though it did
//!   not exist (§4.5's renderer-level existence hiding);
//! - *live re-qualification*: an approval, a delegation, and a role are all
//!   re-derived at the moment of posting, never captured when they were granted;
//! - *nothing moved*: every refusal is asserted against balances, world state, and
//!   the journal, not just against the message.
//!
//! Everything runs through the real `Engine` via `TestWorld`, like `e2b.rs`/`e3b.rs`.
//! Two seams have no command surface (roles/memberships/delegations, and a
//! hand-built `EconomyPlan`), so a tiny in-test `@probe` module reaches them the way
//! a real module would — through `mush_org::submit` and `mush_economy::post`.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use mush_core::module::{ModError, Registrar};
use mush_core::{
    ArgShape, Audience, CmdError, CommandSpec, Ctx, GOD, GameModule, LIMBO, ModuleBootData,
    ModuleWrite, ObjId, ObjKind,
};
use mush_economy::{
    AccountId, AccountStatus, ApprovalStatus, CustodyOp, EconomicCause,
    EconomyError, EconomyPlan, EconomySnapshot, EconomyStore, IdempotencyKey, PostOutcome, VaultId,
};
use mush_org::{
    CapabilityGrant, CapabilityId, Delegation, DelegationStatus, GrantTemplate, MembershipStatus,
    OrgAction, OrgId, OrgMembership, OrgMutationOp, OrgMutationPlan, OrgRole, OrgScope, OrgScopeId,
    OrgStatus, OrgStore, Organization, PrincipalId, RulesValue, ScopeSelector, ScopeStatus,
    Visibility,
};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

// ---- rig ------------------------------------------------------------------

/// The capability vocabulary the server freezes at wiring (§4.10), reproduced here
/// so the tests exercise the real configuration rather than the empty default.
fn vocabulary() -> BTreeSet<CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| CapabilityId::new(c).expect("well-formed"))
        .collect()
}

fn modules(probe: &Probe) -> Vec<Box<dyn GameModule>> {
    vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_economy::EconomyModule::with_custody(vec![])),
        Box::new(ProbeMod(probe.clone())),
    ]
}

fn boot() -> (TestWorld, Probe) {
    let probe = Probe::default();
    let tw = TestWorld::with_modules(modules(&probe));
    (tw, probe)
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events.iter().map(|(_, e)| text_of(e)).collect::<Vec<_>>().join("\n")
}
fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}
/// Run a command that must succeed, returning what the actor saw.
fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(!has_error(&ev), "`{line}` was refused: {}", texts(&ev));
    texts(&ev)
}
/// Run a command that must be refused, returning what the actor saw.
fn err(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(has_error(&ev), "`{line}` should have been refused, got: {}", texts(&ev));
    texts(&ev)
}

fn store(tw: &TestWorld) -> &EconomyStore {
    tw.world().module_state::<EconomyStore>().expect("economy store")
}
fn snap(tw: &TestWorld) -> &EconomySnapshot {
    store(tw).snapshot()
}
fn org_state(tw: &TestWorld) -> &mush_org::OrgState {
    tw.world().module_state::<OrgStore>().expect("org store").state()
}
fn bal(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw).account(id).expect("account").posted_balance.minor()
}
fn thing(tw: &mut TestWorld, name: &str) -> ObjId {
    tw.world_mut().create(ObjKind::Thing, name, GOD, Some(LIMBO)).expect("create")
}
fn journal_len(tw: &TestWorld) -> usize {
    store(tw).transactions().count()
}
fn requests(tw: &TestWorld) -> Vec<mush_economy::ApprovalRequest> {
    snap(tw).approvals().cloned().collect()
}
/// The single live request (the tests that call this expect exactly one).
fn only_request(tw: &TestWorld) -> mush_economy::ApprovalRequest {
    let live: Vec<_> = requests(tw)
        .into_iter()
        .filter(|r| r.status == ApprovalStatus::Pending)
        .collect();
    assert_eq!(live.len(), 1, "expected exactly one live request, got {live:?}");
    live.into_iter().next().expect("checked")
}

/// Both namespaces, folded — an org reboot and an economy reboot are one event in
/// the real server, and rebooting only one would wipe the other's state.
type Kv = HashMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>;

fn fold_kv(tw: &mut TestWorld, acc: &mut Kv) {
    for op in tw.take_module_ops() {
        let rows = acc.entry(op.ns.clone()).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                rows.retain(|(k, _)| *k != key);
                rows.push((key, value));
            }
            ModuleWrite::Del { key } => rows.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
}
/// Reboot the modules in place, so the world (and its objects) survive.
fn reboot(tw: &mut TestWorld, kv: &Kv) {
    let boot: HashMap<SmolStr, ModuleBootData> = kv
        .iter()
        .map(|(ns, rows)| (ns.clone(), ModuleBootData { kv: rows.clone(), streams: HashMap::new() }))
        .collect();
    tw.modules_loaded(boot);
}

// ---- the probe: the two seams with no command surface ---------------------

#[derive(Default)]
struct ProbeState {
    econ: Option<EconomyPlan>,
    org: Option<OrgMutationPlan>,
    outcome: Option<Result<PostOutcome, EconomyError>>,
}

#[derive(Clone, Default)]
struct Probe(Arc<Mutex<ProbeState>>);

impl Probe {
    fn stage_econ(&self, p: EconomyPlan) {
        self.0.lock().expect("probe lock").econ = Some(p);
    }
    fn stage_org(&self, ops: Vec<OrgMutationOp>) {
        self.0.lock().expect("probe lock").org = Some(OrgMutationPlan::of(ops));
    }
    fn outcome(&self) -> Result<PostOutcome, EconomyError> {
        self.0.lock().expect("probe lock").outcome.clone().expect("an outcome was recorded")
    }
}

struct ProbeMod(Probe);

impl GameModule for ProbeMod {
    fn name(&self) -> &'static str {
        "probe"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let st = self.0.clone();
        reg.command(
            CommandSpec {
                name: "@probe".into(),
                shape: ArgShape::EqPair,
                switches: ["post", "org"].iter().copied().map(SmolStr::new).collect(),
                help: "test probe: post a staged EconomyPlan / submit a staged OrgMutationPlan".into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                if ctx.switches.has("post") {
                    let plan = st.0.lock().expect("probe lock").econ.take().expect("stage a plan first");
                    // `post` is the last fallible call, exactly as a real handler
                    // must treat it (§4.2 step 6).
                    let outcome = mush_economy::post(ctx, &plan);
                    st.0.lock().expect("probe lock").outcome = Some(outcome);
                    return Ok(());
                }
                let plan = st.0.lock().expect("probe lock").org.take().expect("stage an org plan first");
                mush_org::submit(ctx, &plan, OrgAction {
                    action: SmolStr::new_static("probe"),
                    org: None,
                    detail: serde_json::json!({}),
                })
                .map_err(|e| CmdError::msg(e.to_string()))
            }),
        )?;
        Ok(())
    }
}

fn submit_org(tw: &mut TestWorld, probe: &Probe, ops: Vec<OrgMutationOp>) {
    probe.stage_org(ops);
    ok(tw, GOD, "@probe/org");
}

// ---- the divisional fixture (scenario 8's world) --------------------------

struct Div {
    org: OrgId,
    root: OrgScopeId,
    ops: OrgScopeId,
    log: OrgScopeId,
    faucet: AccountId,
    ops_acct: AccountId,
    log_acct: AccountId,
    pat_acct: AccountId,
    ops_vault: VaultId,
    log_vault: VaultId,
    quinn: ObjId,
    pat: ObjId,
}

fn scope_of(tw: &TestWorld, org: OrgId, name: &str) -> OrgScopeId {
    org_state(tw).scopes_in(org).find(|s| s.name == name).expect("scope").id
}
fn account_on(tw: &TestWorld, scope: OrgScopeId) -> AccountId {
    snap(tw).accounts().find(|a| a.scope == Some(scope)).expect("account on scope").id
}
fn account_of(tw: &TestWorld, owner: &PrincipalId) -> AccountId {
    snap(tw).accounts().find(|a| a.owner == *owner).expect("account of owner").id
}
fn vault_on(tw: &TestWorld, scope: OrgScopeId) -> VaultId {
    snap(tw).vaults().find(|v| v.scope == Some(scope)).expect("vault on scope").id
}

/// Two divisions of one org, each with its own wallet and vault, plus a faucet and
/// an outside payee. **No grants**: each test says exactly what authority it hands
/// out, because that is what these tests are about.
fn divisions(tw: &mut TestWorld) -> Div {
    let quinn = tw.spawn_player("Quinn");
    let pat = tw.spawn_player("Pat");
    ok(tw, GOD, "@instrument/register gold=0");
    ok(tw, GOD, "@org/create Ironhold");
    let org = org_state(tw).orgs().next().expect("the org").id;
    let root = org_state(tw).org(org).expect("org").root_scope;
    ok(tw, GOD, "@org/scope create Ironhold=operations");
    ok(tw, GOD, "@org/scope create Ironhold=logistics");
    let ops = scope_of(tw, org, "operations");
    let log = scope_of(tw, org, "logistics");

    ok(tw, GOD, "@account/system faucet=gold");
    ok(tw, GOD, &format!("@account/open org:Ironhold/{}=gold", ops.0));
    ok(tw, GOD, &format!("@account/open org:Ironhold/{}=gold", log.0));
    ok(tw, GOD, &format!("@account/open #{}=gold", pat.0));
    let faucet = account_of(tw, &PrincipalId::System(mush_org::SystemPrincipalId::new("faucet").expect("id")));
    let ops_acct = account_on(tw, ops);
    let log_acct = account_on(tw, log);
    let pat_acct = account_of(tw, &PrincipalId::Object(pat));
    ok(tw, GOD, &format!("@econ/mint {faucet}={ops_acct} 1000"));
    ok(tw, GOD, &format!("@econ/mint {faucet}={log_acct} 1000"));

    let ops_chest = thing(tw, "ops chest");
    let log_chest = thing(tw, "log chest");
    ok(tw, GOD, &format!("@vault/create opsvault=container:#{} owner=org:Ironhold scope=#{}", ops_chest.0, ops.0));
    ok(tw, GOD, &format!("@vault/create logvault=container:#{} owner=org:Ironhold scope=#{}", log_chest.0, log.0));
    let ops_vault = vault_on(tw, ops);
    let log_vault = vault_on(tw, log);

    Div { org, root, ops, log, faucet, ops_acct, log_acct, pat_acct, ops_vault, log_vault, quinn, pat }
}

/// `@org/grant <who>=Ironhold <cap> scope=<scope>`.
fn grant(tw: &mut TestWorld, who: ObjId, capability: &str, scope: OrgScopeId) {
    ok(tw, GOD, &format!("@org/grant #{}=Ironhold {capability} scope=#{}", who.0, scope.0));
}
fn grant_id(tw: &TestWorld, who: ObjId, capability: &str) -> mush_org::GrantId {
    let cap = CapabilityId::new(capability).expect("cap");
    org_state(tw)
        .orgs()
        .flat_map(|o| org_state(tw).grants_in(o.id))
        .find(|g| g.grantee == who && g.capability == cap)
        .expect("the grant")
        .id
}

// ======================================================================
// Scenario 8 — the quartermaster and the sibling division
// ======================================================================

#[test]
fn scenario_8_the_quartermaster_pays_from_logistics_and_never_from_operations() {
    let (mut tw, _p) = boot();
    let d = divisions(&mut tw);
    grant(&mut tw, d.quinn, "economy.account.post", d.log);
    grant(&mut tw, d.quinn, "economy.account.view", d.log);

    // Logistics: authorized, and the money really moves.
    ok(&mut tw, d.quinn, &format!("@econ/pay {}={} 250", d.log_acct, d.pat_acct));
    assert_eq!(bal(&tw, d.log_acct), 750);
    assert_eq!(bal(&tw, d.pat_acct), 250);

    // Operations: the same capability, one compartment over — refused, nothing moved,
    // and (no view standing there) indistinguishable from an account that is not
    // there at all.
    let before = journal_len(&tw);
    let seen = err(&mut tw, d.quinn, &format!("@econ/pay {}={} 1", d.ops_acct, d.pat_acct));
    assert!(seen.contains("No such account."), "the sibling wallet leaked: {seen}");
    assert_eq!(bal(&tw, d.ops_acct), 1000, "the refused payment moved money");
    assert_eq!(journal_len(&tw), before, "the refused payment posted a transaction");
}

#[test]
fn scenario_8_every_verb_against_the_sibling_renders_as_nonexistent() {
    let (mut tw, _p) = boot();
    let d = divisions(&mut tw);
    for c in ["economy.account.post", "economy.account.view", "vault.view", "vault.deposit", "vault.withdraw"] {
        grant(&mut tw, d.quinn, c, d.log);
    }
    let widget = thing(&mut tw, "widget");
    tw.world_mut().move_to(widget, d.quinn).expect("Quinn carries it");

    // What Quinn MAY see, so the contrast is real and not just blanket blindness.
    let seen = ok(&mut tw, d.quinn, &format!("@account/show {}", d.log_acct));
    assert!(seen.contains("Ironhold/logistics"), "the visible wallet did not render: {seen}");
    let seen = ok(&mut tw, d.quinn, &format!("@vault/show {}", d.log_vault));
    assert!(seen.contains("logvault"), "the visible vault did not render: {seen}");

    // ...and the identical text for every route to the sibling.
    for (line, expected) in [
        (format!("@account/show {}", d.ops_acct), "No such account."),
        (format!("@account/close {}", d.ops_acct), "No such account."),
        (format!("@vault/show {}", d.ops_vault), "No such vault."),
        (format!("@vault/deposit #{}={}", widget.0, d.ops_vault), "No such vault."),
        (format!("@vault/withdraw #{}={}", widget.0, d.ops_vault), "No such vault."),
        (format!("@econ/approve {}", 0), "No such approval request."),
        (format!("@econ/deny {}", 0), "No such approval request."),
    ] {
        let seen = err(&mut tw, d.quinn, &line);
        assert!(seen.contains(expected), "`{line}` leaked: {seen}");
        assert!(!seen.contains("operations"), "`{line}` named the sibling: {seen}");
    }

    // The lists agree with the refusals: nothing about operations is discoverable.
    let seen = ok(&mut tw, d.quinn, "@account/list");
    assert!(seen.contains("Ironhold/logistics"));
    assert!(!seen.contains("Ironhold/operations"), "@account/list leaked the sibling: {seen}");
    let seen = ok(&mut tw, d.quinn, "@vault/list");
    assert!(seen.contains("logvault") && !seen.contains("opsvault"), "@vault/list leaked: {seen}");
}

#[test]
fn scenario_8_a_non_staff_debit_of_a_system_account_is_refused_at_both_layers() {
    let (mut tw, probe) = boot();
    let d = divisions(&mut tw);
    grant(&mut tw, d.quinn, "economy.account.post", d.log);
    let before = bal(&tw, d.faucet);

    // The command layer refuses it (a faucet is neither owned nor scoped)...
    let seen = err(&mut tw, d.quinn, &format!("@econ/pay {}={} 100", d.faucet, d.pat_acct));
    assert!(seen.contains("No such account."), "the faucet leaked: {seen}");

    // ...and so does the BRIDGE, which is the lock that matters: A6's whole point is
    // that the command guard is not the only thing standing between a player and
    // minting money. Same plan, straight through `post`.
    let plan = EconomyPlan::new(
        IdempotencyKey::new("test", "mint-attempt"),
        PrincipalId::Object(d.quinn),
        EconomicCause::Command { serial: 0 },
        vec![
            mush_economy::RequestedLine {
                account: d.faucet,
                delta: mush_economy::Amount::from_minor(-100),
                memo: SmolStr::default(),
            },
            mush_economy::RequestedLine {
                account: d.pat_acct,
                delta: mush_economy::Amount::from_minor(100),
                memo: SmolStr::default(),
            },
        ],
    );
    probe.stage_econ(plan);
    ok(&mut tw, d.quinn, "@probe/post");
    assert_eq!(probe.outcome(), Err(EconomyError::Unauthorized), "the bridge let a player mint");
    assert_eq!(bal(&tw, d.faucet), before, "the faucet moved");
}

// ======================================================================
// Scenario 9 — a role NAME is not authority; its grants are
// ======================================================================

#[test]
fn scenario_9_a_role_name_confers_nothing_and_swapping_its_grants_refuses_the_next_post() {
    let (mut tw, probe) = boot();
    let d = divisions(&mut tw);
    let (r1, r2) = {
        let s = tw.world_mut().module_state_mut::<OrgStore>().expect("org store");
        (s.alloc_role(), s.alloc_role())
    };
    // Two roles with the SAME ruleset-facing name. One carries the grant template;
    // the other is a rename with nothing behind it.
    let template = GrantTemplate {
        capability: CapabilityId::new("economy.account.post").expect("cap"),
        scope: ScopeSelector::Exact(d.log),
        limit: None,
        delegable: false,
    };
    let member = |roles: BTreeSet<mush_org::RoleId>| OrgMembership {
        org: d.org,
        member: d.quinn,
        status: MembershipStatus::Active,
        titles: vec![SmolStr::new_static("Quartermaster")],
        roles,
        visibility: Visibility::Public,
        since: 0,
        revision: 1,
    };
    submit_org(&mut tw, &probe, vec![
        OrgMutationOp::CreateRole {
            role: OrgRole { id: r1, org: d.org, name: "quartermaster".into(), grants: vec![template], revision: 1 },
        },
        OrgMutationOp::CreateRole {
            role: OrgRole { id: r2, org: d.org, name: "quartermaster".into(), grants: vec![], revision: 1 },
        },
        OrgMutationOp::PutMembership { membership: member(BTreeSet::from([r1])) },
    ]);

    // The role's grant template authorizes the post (no direct grant exists).
    ok(&mut tw, d.quinn, &format!("@econ/pay {}={} 100", d.log_acct, d.pat_acct));
    assert_eq!(bal(&tw, d.log_acct), 900);

    // Adding the identically-NAMED, grantless role changes nothing: the name is
    // presentation, the grants are authority.
    submit_org(&mut tw, &probe, vec![OrgMutationOp::PutMembership {
        membership: member(BTreeSet::from([r1, r2])),
    }]);
    ok(&mut tw, d.quinn, &format!("@econ/pay {}={} 100", d.log_acct, d.pat_acct));
    assert_eq!(bal(&tw, d.log_acct), 800, "a cosmetic role changed the authority");

    // Replace the role behind the title with the grantless one — same title, same
    // membership, no authority. Re-derived live, so the very next post is refused.
    submit_org(&mut tw, &probe, vec![OrgMutationOp::PutMembership {
        membership: member(BTreeSet::from([r2])),
    }]);
    err(&mut tw, d.quinn, &format!("@econ/pay {}={} 100", d.log_acct, d.pat_acct));
    assert_eq!(bal(&tw, d.log_acct), 800, "the refused payment moved money");
    assert_eq!(
        org_state(&tw).membership(d.org, d.quinn).expect("member").titles[0].as_str(),
        "Quartermaster",
        "the cosmetic title is untouched by any of this"
    );
}

// ======================================================================
// Scenario 10 — a delegation is only as live as its source
// ======================================================================

/// A delegable source grant with an expiry, plus a one-hop delegation narrowed to
/// one scope. Returns the moment the SOURCE stops being valid.
fn delegated(
    tw: &mut TestWorld,
    probe: &Probe,
    d: &Div,
    capability: &str,
    grantor: ObjId,
    grantee: ObjId,
) -> i64 {
    let now = tw.world().now();
    let source_expiry = now + 3_600;
    let (gid, did) = {
        let s = tw.world_mut().module_state_mut::<OrgStore>().expect("org store");
        (s.alloc_grant(), s.alloc_delegation())
    };
    let cap = CapabilityId::new(capability).expect("cap");
    submit_org(tw, probe, vec![
        OrgMutationOp::PutGrant {
            grant: CapabilityGrant {
                id: gid,
                org: d.org,
                grantee: grantor,
                capability: cap.clone(),
                scope: ScopeSelector::SubtreeLive(d.root),
                limit: None,
                delegable: true,
                valid_from: None,
                valid_until: Some(source_expiry),
                revision: 1,
            },
        },
        OrgMutationOp::Delegate {
            delegation: Delegation {
                id: did,
                org: d.org,
                grantor,
                grantee,
                source_grant: gid,
                capability: cap,
                // Narrowed to logistics, and outliving its source on paper — the
                // delegation's OWN expiry is deliberately far away, so the only
                // thing that can stop it is live re-resolution of the source.
                scope: ScopeSelector::Exact(d.log),
                limit: None,
                expires_at: now + 1_000_000,
                source_revision: 1,
                status: DelegationStatus::Active,
            },
        },
    ]);
    source_expiry
}

#[test]
fn scenario_10_a_narrowed_delegation_dies_with_its_source() {
    let (mut tw, probe) = boot();
    let d = divisions(&mut tw);
    let ada = tw.spawn_player("Ada");
    let cy = tw.spawn_player("Cy");
    let expiry = delegated(&mut tw, &probe, &d, "economy.account.post", ada, cy);

    // The delegate spends inside the narrowed scope...
    ok(&mut tw, cy, &format!("@econ/pay {}={} 100", d.log_acct, d.pat_acct));
    assert_eq!(bal(&tw, d.log_acct), 900);
    // ...and never outside it, even though the SOURCE covers the whole tree.
    err(&mut tw, cy, &format!("@econ/pay {}={} 100", d.ops_acct, d.pat_acct));
    assert_eq!(bal(&tw, d.ops_acct), 1000);

    // The source expires. Nothing is revoked, no revision moves, no record changes —
    // and the delegation confers nothing, because it is re-resolved against the LIVE
    // source at every post.
    tw.set_now(expiry + 1);
    err(&mut tw, cy, &format!("@econ/pay {}={} 100", d.log_acct, d.pat_acct));
    assert_eq!(bal(&tw, d.log_acct), 900, "an expired source still authorized a spend");
    assert_eq!(
        org_state(&tw).delegations_in(d.org).next().expect("delegation").status,
        DelegationStatus::Active,
        "the delegation record itself is untouched — expiry is derived, not stamped"
    );
}

#[test]
fn scenario_10_a_delegate_approver_stops_counting_when_the_source_expires() {
    let (mut tw, probe) = boot();
    let d = divisions(&mut tw);
    let ada = tw.spawn_player("Ada");
    let cy = tw.spawn_player("Cy");
    grant(&mut tw, d.quinn, "economy.account.post", d.log);
    grant(&mut tw, d.quinn, "economy.account.view", d.log);
    let expiry = delegated(&mut tw, &probe, &d, "economy.account.approve", ada, cy);
    ok(&mut tw, GOD, &format!("@account/approval {}=100 threshold:1", d.log_acct));

    // Payment A: the delegate's approval qualifies, so the re-submission posts.
    let pay_a = format!("@econ/pay {}={} 500", d.log_acct, d.pat_acct);
    ok(&mut tw, d.quinn, &pay_a);
    let r = only_request(&tw);
    ok(&mut tw, cy, &format!("@econ/approve {}", r.id));
    ok(&mut tw, d.quinn, &pay_a);
    assert_eq!(bal(&tw, d.log_acct), 500, "the approved payment did not post");

    // Payment B: same approver, same approval — but the source grant has expired by
    // the time it is re-qualified, so the collected approval no longer satisfies.
    let pay_b = format!("@econ/pay {}={} 400", d.log_acct, d.pat_acct);
    ok(&mut tw, d.quinn, &pay_b);
    let r2 = only_request(&tw);
    ok(&mut tw, cy, &format!("@econ/approve {}", r2.id));
    tw.set_now(expiry + 1);
    let seen = ok(&mut tw, d.quinn, &pay_b);
    assert!(seen.contains("Approval required"), "an expired delegate still satisfied: {seen}");
    assert_eq!(bal(&tw, d.log_acct), 500, "the unapproved payment posted");
    assert_eq!(
        snap(&tw).approval(r2.id).expect("request").approvals.len(),
        1,
        "the recorded approval is still ON the request — it simply stopped counting"
    );
}

// ======================================================================
// Scenario 11 — approvals end to end
// ======================================================================

/// The scenario-11 world: Quinn spends from logistics, Ada and Bo may approve there,
/// and anything over 100 escalates to two approvals.
struct Appr {
    d: Div,
    ada: ObjId,
    bo: ObjId,
}

fn approvals_fixture(tw: &mut TestWorld, policy: &str) -> Appr {
    let d = divisions(tw);
    let ada = tw.spawn_player("Ada");
    let bo = tw.spawn_player("Bo");
    grant(tw, d.quinn, "economy.account.post", d.log);
    grant(tw, d.quinn, "economy.account.view", d.log);
    grant(tw, ada, "economy.account.approve", d.log);
    grant(tw, bo, "economy.account.approve", d.log);
    ok(tw, GOD, &format!("@account/approval {}=100 {policy}", d.log_acct));
    Appr { d, ada, bo }
}

#[test]
fn scenario_11_two_approvals_post_the_plan_and_consume_the_request() {
    let (mut tw, _p) = boot();
    let a = approvals_fixture(&mut tw, "threshold:2");
    let pay = format!("@econ/pay {}={} 500", a.d.log_acct, a.d.pat_acct);

    // Above the threshold: nothing moves, a request is minted.
    let seen = ok(&mut tw, a.d.quinn, &pay);
    assert!(seen.contains("Approval required"), "{seen}");
    assert_eq!(bal(&tw, a.d.log_acct), 1000, "an unapproved plan moved money");
    let r = only_request(&tw);

    // One approval is not two.
    ok(&mut tw, a.ada, &format!("@econ/approve {}", r.id));
    let seen = ok(&mut tw, a.d.quinn, &pay);
    assert!(seen.contains("Approval required"), "one approval satisfied a two-approval policy: {seen}");
    assert_eq!(bal(&tw, a.d.log_acct), 1000);

    // Two are. The re-submission posts, and the request is spent — once.
    ok(&mut tw, a.bo, &format!("@econ/approve {}", r.id));
    ok(&mut tw, a.d.quinn, &pay);
    assert_eq!(bal(&tw, a.d.log_acct), 500);
    assert_eq!(bal(&tw, a.d.pat_acct), 500);
    let consumed = snap(&tw).approval(r.id).expect("request").status;
    let ApprovalStatus::Consumed { transaction } = consumed else {
        panic!("the spent request is {consumed:?}, not Consumed");
    };
    assert_eq!(
        store(&tw).transaction(transaction).expect("tx").consumed_approval,
        Some(r.id),
        "the journal entry does not name the request it spent"
    );
    // The audit trail kept both decisions.
    assert_eq!(snap(&tw).approval(r.id).expect("request").decisions.len(), 2);
}

#[test]
fn scenario_11_a_second_identical_submission_mints_no_second_request() {
    let (mut tw, _p) = boot();
    let a = approvals_fixture(&mut tw, "threshold:2");
    let pay = format!("@econ/pay {}={} 500", a.d.log_acct, a.d.pat_acct);
    ok(&mut tw, a.d.quinn, &pay);
    let first = only_request(&tw).id;
    for _ in 0..3 {
        let seen = ok(&mut tw, a.d.quinn, &pay);
        assert!(seen.contains(&format!("request #{first}")), "a re-submission changed the id: {seen}");
    }
    assert_eq!(requests(&tw).len(), 1, "one live request per plan hash (A1)");
}

#[test]
fn scenario_11_editing_the_amount_needs_its_own_request() {
    let (mut tw, _p) = boot();
    let a = approvals_fixture(&mut tw, "threshold:1");
    ok(&mut tw, a.d.quinn, &format!("@econ/pay {}={} 500", a.d.log_acct, a.d.pat_acct));
    let r = only_request(&tw);
    ok(&mut tw, a.ada, &format!("@econ/approve {}", r.id));

    // A different amount is a different plan, and the approval is bound to the plan
    // — not to the payer, the account, or the day.
    let seen = ok(&mut tw, a.d.quinn, &format!("@econ/pay {}={} 501", a.d.log_acct, a.d.pat_acct));
    assert!(seen.contains("Approval required"), "an approval authorized a different plan: {seen}");
    assert_eq!(bal(&tw, a.d.log_acct), 1000);
    assert_eq!(requests(&tw).len(), 2, "the edited plan minted its own request");
    // ...and the original still posts on its own terms.
    ok(&mut tw, a.d.quinn, &format!("@econ/pay {}={} 500", a.d.log_acct, a.d.pat_acct));
    assert_eq!(bal(&tw, a.d.log_acct), 500);
}

#[test]
fn scenario_11_revoking_an_approvers_capability_unsatisfies_the_request() {
    let (mut tw, probe) = boot();
    let a = approvals_fixture(&mut tw, "threshold:2");
    let pay = format!("@econ/pay {}={} 500", a.d.log_acct, a.d.pat_acct);
    ok(&mut tw, a.d.quinn, &pay);
    let r = only_request(&tw);
    ok(&mut tw, a.ada, &format!("@econ/approve {}", r.id));
    ok(&mut tw, a.bo, &format!("@econ/approve {}", r.id));

    // Both approvals are recorded and would post — but Ada loses the capability
    // first. Approvals are re-qualified LIVE (A5), so hers stops counting.
    let gid = grant_id(&tw, a.ada, "economy.account.approve");
    submit_org(&mut tw, &probe, vec![OrgMutationOp::RevokeGrant { grant: gid }]);
    let seen = ok(&mut tw, a.d.quinn, &pay);
    assert!(seen.contains("Approval required"), "a revoked approver still counted: {seen}");
    assert_eq!(bal(&tw, a.d.log_acct), 1000);
    assert_eq!(snap(&tw).approval(r.id).expect("request").approvals.len(), 2, "the record still holds both");

    // Restore it and the very same request satisfies again — nothing was re-collected.
    grant(&mut tw, a.ada, "economy.account.approve", a.d.log);
    ok(&mut tw, a.d.quinn, &pay);
    assert_eq!(bal(&tw, a.d.log_acct), 500);
}

#[test]
fn scenario_11_the_requesters_own_approval_never_counts() {
    let (mut tw, _p) = boot();
    let a = approvals_fixture(&mut tw, "threshold:2");
    // Quinn may approve OTHER people's requests here...
    grant(&mut tw, a.d.quinn, "economy.account.approve", a.d.log);
    let pay = format!("@econ/pay {}={} 500", a.d.log_acct, a.d.pat_acct);
    ok(&mut tw, a.d.quinn, &pay);
    let r = only_request(&tw);

    // ...but not their own: four-eyes. Quinn + Ada is one qualifying approval.
    let seen = ok(&mut tw, a.d.quinn, &format!("@econ/approve {}", r.id));
    assert!(seen.contains("does not count"), "the self-approval was not called out: {seen}");
    ok(&mut tw, a.ada, &format!("@econ/approve {}", r.id));
    assert_eq!(snap(&tw).approval(r.id).expect("request").approvals.len(), 2);
    let seen = ok(&mut tw, a.d.quinn, &pay);
    assert!(seen.contains("Approval required"), "the requester's own approval counted: {seen}");
    assert_eq!(bal(&tw, a.d.log_acct), 1000);

    // A second real approver, and it posts.
    ok(&mut tw, a.bo, &format!("@econ/approve {}", r.id));
    ok(&mut tw, a.d.quinn, &pay);
    assert_eq!(bal(&tw, a.d.log_acct), 500);
}

#[test]
fn scenario_11_a_denial_cancels_and_a_consumed_request_survives_reboot() {
    let (mut tw, _p) = boot();
    let a = approvals_fixture(&mut tw, "threshold:1");
    let pay = format!("@econ/pay {}={} 500", a.d.log_acct, a.d.pat_acct);

    // A denial is recorded and cancels the request; the plan needs a fresh one.
    ok(&mut tw, a.d.quinn, &pay);
    let denied = only_request(&tw).id;
    ok(&mut tw, a.ada, &format!("@econ/deny {denied}"));
    assert_eq!(snap(&tw).approval(denied).expect("request").status, ApprovalStatus::Cancelled);
    let seen = ok(&mut tw, a.d.quinn, &pay);
    assert!(seen.contains("Approval required") && !seen.contains(&format!("#{denied}")), "{seen}");
    let live = only_request(&tw).id;
    assert_ne!(live, denied);

    ok(&mut tw, a.ada, &format!("@econ/approve {live}"));
    ok(&mut tw, a.d.quinn, &pay);
    assert_eq!(bal(&tw, a.d.log_acct), 500);

    let mut kv: Kv = HashMap::new();
    fold_kv(&mut tw, &mut kv);
    reboot(&mut tw, &kv);
    assert!(
        matches!(snap(&tw).approval(live).expect("survives").status, ApprovalStatus::Consumed { .. }),
        "a spent request must not come back pending"
    );
    assert_eq!(snap(&tw).approval(denied).expect("survives").status, ApprovalStatus::Cancelled);
    assert_eq!(bal(&tw, a.d.log_acct), 500, "the balance changed across the reboot");
    // The allocator never re-mints a used id.
    ok(&mut tw, a.d.quinn, &format!("@econ/pay {}={} 400", a.d.log_acct, a.d.pat_acct));
    let fresh = only_request(&tw).id;
    assert!(fresh > live, "the approval allocator re-minted {live} as {fresh}");
}

#[test]
fn scenario_11_a_consumed_request_heals_from_the_journal_when_its_doc_lags() {
    let (mut tw, _p) = boot();
    let a = approvals_fixture(&mut tw, "threshold:1");
    let pay = format!("@econ/pay {}={} 500", a.d.log_acct, a.d.pat_acct);
    ok(&mut tw, a.d.quinn, &pay);
    let r = only_request(&tw).id;
    ok(&mut tw, a.ada, &format!("@econ/approve {r}"));
    let mut kv: Kv = HashMap::new();
    fold_kv(&mut tw, &mut kv); // …the state BEFORE the consuming post
    ok(&mut tw, a.d.quinn, &pay);

    // Drop the request's own doc update, keeping the journal (C3: heal by ID, never
    // by hash). The entry that spent it is the authority.
    let mut kv2 = kv.clone();
    let econ = kv2.get_mut("economy").expect("economy rows").clone();
    let mut fresh: Kv = HashMap::new();
    fold_kv(&mut tw, &mut fresh);
    let key = mush_economy::approval_key(r);
    for (k, v) in fresh.get("economy").expect("rows") {
        if *k == key {
            continue; // the lagging doc: keep the OLD, pending copy
        }
        let rows = kv2.entry(SmolStr::new("economy")).or_default();
        rows.retain(|(rk, _)| rk != k);
        rows.push((k.clone(), v.clone()));
    }
    assert!(econ.iter().any(|(k, _)| *k == key), "the pending doc is in the pre-image");
    reboot(&mut tw, &kv2);
    assert!(
        matches!(snap(&tw).approval(r).expect("request").status, ApprovalStatus::Consumed { .. }),
        "a Pending doc the journal proves was spent must heal to Consumed"
    );
}

#[test]
fn scenario_11_an_expired_request_is_not_consumable() {
    let (mut tw, _p) = boot();
    let a = approvals_fixture(&mut tw, "threshold:1");
    let pay = format!("@econ/pay {}={} 500", a.d.log_acct, a.d.pat_acct);
    ok(&mut tw, a.d.quinn, &pay);
    let r = only_request(&tw);
    ok(&mut tw, a.ada, &format!("@econ/approve {}", r.id));

    // Past the TTL the collected approval is worthless: the remedy is a NEW request,
    // not a refusal — the plan itself was never wrong.
    tw.set_now(r.expires_at + 1);
    let seen = err(&mut tw, a.ada, &format!("@econ/approve {}", r.id));
    assert!(seen.contains("expired"), "{seen}");
    let seen = ok(&mut tw, a.d.quinn, &pay);
    assert!(seen.contains("Approval required"), "an expired request posted: {seen}");
    assert_eq!(bal(&tw, a.d.log_acct), 1000);
    assert_eq!(snap(&tw).approval(r.id).expect("request").status, ApprovalStatus::Expired);
    let fresh = only_request(&tw);
    assert_ne!(fresh.id, r.id, "the stale request was reused");
    assert!(fresh.approvals.is_empty(), "the new request inherited approvals");
}

#[test]
fn scenario_11_the_pending_request_caps_refuse_beyond_the_limit() {
    let (mut tw, _p) = boot();
    let a = approvals_fixture(&mut tw, "threshold:1");
    for i in 0..mush_economy::MAX_PENDING_PER_REQUESTER {
        let amount = 500 + i;
        ok(&mut tw, a.d.quinn, &format!("@econ/pay {}={} {amount}", a.d.log_acct, a.d.pat_acct));
    }
    assert_eq!(requests(&tw).len(), mush_economy::MAX_PENDING_PER_REQUESTER);
    let seen = err(&mut tw, a.d.quinn, &format!("@econ/pay {}={} 999", a.d.log_acct, a.d.pat_acct));
    assert!(seen.contains("too many pending approval requests"), "{seen}");
    assert_eq!(requests(&tw).len(), mush_economy::MAX_PENDING_PER_REQUESTER, "the cap leaked a request");
    assert_eq!(bal(&tw, a.d.log_acct), 1000);
}

#[test]
fn scenario_11_boot_collects_long_dead_requests() {
    let (mut tw, _p) = boot();
    let a = approvals_fixture(&mut tw, "threshold:1");
    ok(&mut tw, a.d.quinn, &format!("@econ/pay {}={} 500", a.d.log_acct, a.d.pat_acct));
    let dead = only_request(&tw);
    ok(&mut tw, a.ada, &format!("@econ/deny {}", dead.id));

    // Much later — well past the dead request's collection window — a second one is
    // raised, to prove the GC is selective rather than a periodic wipe.
    let later = dead.expires_at + mush_economy::APPROVAL_GC_AFTER_SECS + 2 * mush_economy::APPROVAL_TTL_SECS;
    tw.set_now(later);
    ok(&mut tw, a.d.quinn, &format!("@econ/pay {}={} 600", a.d.log_acct, a.d.pat_acct));
    let recent = only_request(&tw).id;

    let mut kv: Kv = HashMap::new();
    fold_kv(&mut tw, &mut kv);
    tw.set_now(later + 10);
    reboot(&mut tw, &kv);

    assert!(snap(&tw).approval(dead.id).is_none(), "a long-dead request survived the GC");
    let deleted = tw
        .take_module_ops()
        .into_iter()
        .any(|o| o.ns == "economy" && matches!(&o.op, ModuleWrite::Del { key } if *key == mush_economy::approval_key(dead.id)));
    assert!(deleted, "the collected request's doc was not deleted");
    // ...while the recent one is untouched and still live.
    assert_eq!(snap(&tw).approval(recent).expect("kept").status, ApprovalStatus::Pending);
    // ...and the allocator is still floored past every id it ever minted, collected
    // ones included (C4): a re-minted id would silently inherit a dead request's
    // history.
    ok(&mut tw, a.d.quinn, &format!("@econ/pay {}={} 700", a.d.log_acct, a.d.pat_acct));
    let fresh = requests(&tw).into_iter().map(|r| r.id).max().expect("a request");
    assert!(fresh > recent && fresh > dead.id, "the allocator re-minted a collected id ({fresh})");
}

#[test]
fn scenario_11_splitting_a_payment_under_the_threshold_posts_each_part() {
    let (mut tw, _p) = boot();
    let a = approvals_fixture(&mut tw, "threshold:2");
    // INTENDED and pinned (A12): an ApprovalRule is a per-transaction escalation,
    // not a budget. Ten payments of 100 pass a threshold of 100 — holds are E4.
    for _ in 0..8 {
        ok(&mut tw, a.d.quinn, &format!("@econ/pay {}={} 100", a.d.log_acct, a.d.pat_acct));
    }
    assert_eq!(bal(&tw, a.d.log_acct), 200, "the split payments did not all post");
    assert!(requests(&tw).is_empty(), "an at-threshold payment escalated");
    // ...and one hair over it still escalates.
    let seen = ok(&mut tw, a.d.quinn, &format!("@econ/pay {}={} 101", a.d.log_acct, a.d.pat_acct));
    assert!(seen.contains("Approval required"), "{seen}");
}

#[test]
fn a_named_set_approver_may_view_the_requests_accounts_while_it_is_live() {
    let (mut tw, _p) = boot();
    let d = divisions(&mut tw);
    let ada = tw.spawn_player("Ada");
    grant(&mut tw, d.quinn, "economy.account.post", d.log);
    ok(&mut tw, GOD, &format!("@account/approval {}=100 named:1:#{}", d.log_acct, ada.0));

    // Ada holds no capability at all — being named IS the authorization — and with
    // no live request she cannot see the account either.
    let seen = err(&mut tw, ada, &format!("@account/show {}", d.log_acct));
    assert!(seen.contains("No such account."), "{seen}");

    ok(&mut tw, d.quinn, &format!("@econ/pay {}={} 500", d.log_acct, d.pat_acct));
    let r = only_request(&tw);
    let seen = ok(&mut tw, ada, &format!("@account/show {}", d.log_acct));
    assert!(seen.contains("Ironhold/logistics"), "a named approver cannot see what she is signing: {seen}");
    let seen = ok(&mut tw, ada, "@econ/approvals");
    assert!(seen.contains(&format!("#{}", r.id)), "{seen}");

    ok(&mut tw, ada, &format!("@econ/approve {}", r.id));
    ok(&mut tw, d.quinn, &format!("@econ/pay {}={} 500", d.log_acct, d.pat_acct));
    assert_eq!(bal(&tw, d.log_acct), 500);

    // The request is spent, so the borrowed view ends with it.
    let seen = err(&mut tw, ada, &format!("@account/show {}", d.log_acct));
    assert!(seen.contains("No such account."), "the view outlived the request: {seen}");
}

// ======================================================================
// Scenario 12 — a hidden org is never named
// ======================================================================

#[test]
fn scenario_12_a_hidden_org_is_never_named_to_an_outsider() {
    let (mut tw, probe) = boot();
    let d = divisions(&mut tw);
    let (oid, sid) = {
        let s = tw.world_mut().module_state_mut::<OrgStore>().expect("org store");
        (s.alloc_org(), s.alloc_scope())
    };
    submit_org(&mut tw, &probe, vec![OrgMutationOp::CreateOrg {
        org: Organization {
            id: oid,
            name: "Umbra".into(),
            aliases: vec![],
            status: OrgStatus::Active,
            hidden: true,
            charter: String::new(),
            root_scope: sid,
            revision: 1,
            ruleset_ext: RulesValue::default(),
        },
        root: OrgScope {
            id: sid,
            org: oid,
            parent: None,
            name: "root".into(),
            status: ScopeStatus::Active,
            depth: 0,
            created_rev: 1,
            revision: 1,
        },
    }]);
    ok(&mut tw, GOD, "@account/open org:Umbra=gold");
    let hidden_acct = account_on(&tw, sid);
    let cache = thing(&mut tw, "cache");
    ok(&mut tw, GOD, &format!("@vault/create umbravault=container:#{} owner=org:Umbra scope=#{}", cache.0, sid.0));
    let hidden_vault = vault_on(&tw, sid);

    // A crate Pat owns but Umbra holds title to — the destroy gate's hardest case.
    let stash = thing(&mut tw, "stash");
    ok(&mut tw, GOD, &format!("@chown #{}=Pat", stash.0));
    tw.world_mut().move_to(stash, d.pat).expect("hand it over");
    ok(&mut tw, GOD, &format!("@asset/title #{}=org:Umbra scope=#{}", stash.0, sid.0));

    let mut seen = Vec::new();
    seen.push(ok(&mut tw, d.pat, "@account/list"));
    seen.push(ok(&mut tw, d.pat, "@vault/list"));
    seen.push(ok(&mut tw, d.pat, "@asset/list owner=org:Umbra"));
    seen.push(ok(&mut tw, d.pat, &format!("@asset/list scope=#{}", sid.0)));
    seen.push(err(&mut tw, d.pat, &format!("@account/show {hidden_acct}")));
    seen.push(err(&mut tw, d.pat, &format!("@vault/show {hidden_vault}")));
    seen.push(err(&mut tw, d.pat, &format!("@asset/show #{}", stash.0)));
    // The destroy gate: neutral for non-staff, and it must not explain itself.
    let gate = err(&mut tw, d.pat, &format!("@destroy #{}", stash.0));
    assert!(gate.contains("You cannot destroy that right now."), "{gate}");
    seen.push(gate);

    for s in &seen {
        let lower = s.to_lowercase();
        assert!(!lower.contains("umbra"), "the hidden org was named: {s}");
        assert!(!lower.contains("cache") && !lower.contains("stash"), "a hidden holding leaked: {s}");
    }
    assert!(tw.world().get(stash).is_some(), "the gated object was destroyed anyway");
    // Staff, by contrast, get the detail — the hiding is a disclosure rule, not a
    // hole in the record.
    let staff = ok(&mut tw, GOD, "@account/list");
    assert!(staff.contains("Umbra"), "staff cannot see the hidden org's wallet: {staff}");
}

// ======================================================================
// Org status + scope lifecycle (§4.4, §4.8)
// ======================================================================

/// Rewrite an org's persisted status and reboot — the only route to a wound-down
/// org, since the mutation alphabet has no status op (see the report).
fn set_org_status(tw: &mut TestWorld, kv: &mut Kv, org: OrgId, status: &str) {
    fold_kv(tw, kv);
    let key = mush_org::org_key(org);
    for (k, v) in kv.get_mut("org").expect("org rows") {
        if *k == key {
            v["status"] = serde_json::json!(status);
        }
    }
    reboot(tw, kv);
}

#[test]
fn a_retired_scope_refuses_debits_while_its_sibling_keeps_working() {
    let (mut tw, probe) = boot();
    let d = divisions(&mut tw);
    // Authority over the WHOLE tree, so the refusal can only be about the scope.
    ok(&mut tw, GOD, &format!("@org/grant #{}=Ironhold economy.account.post", d.quinn.0));
    ok(&mut tw, GOD, &format!("@org/grant #{}=Ironhold economy.account.view", d.quinn.0));
    ok(&mut tw, d.quinn, &format!("@econ/pay {}={} 100", d.ops_acct, d.pat_acct));

    // `submit` is authorization-free and ungated by design, so the probe can retire a
    // scope the COMMAND would refuse (an account still binds it) — which is exactly
    // the stranded-compartment state the money rules have to survive.
    submit_org(&mut tw, &probe, vec![OrgMutationOp::RetireScope { scope: d.ops }]);
    let seen = err(&mut tw, d.quinn, &format!("@econ/pay {}={} 100", d.ops_acct, d.pat_acct));
    assert!(seen.contains("unauthorized") || seen.contains("No such account."), "{seen}");
    assert_eq!(bal(&tw, d.ops_acct), 900, "a retired compartment still authorized a debit");
    // The sibling is untouched.
    ok(&mut tw, d.quinn, &format!("@econ/pay {}={} 100", d.log_acct, d.pat_acct));
    assert_eq!(bal(&tw, d.log_acct), 900);
}

#[test]
fn a_dissolving_org_settles_but_a_suspended_one_takes_nothing() {
    let (mut tw, _p) = boot();
    let d = divisions(&mut tw);
    ok(&mut tw, GOD, &format!("@org/grant #{}=Ironhold economy.account.post", d.quinn.0));
    let mut kv: Kv = HashMap::new();

    // Dissolving: credits land (final settlement is the whole point) and debits stop.
    set_org_status(&mut tw, &mut kv, d.org, "Dissolving");
    ok(&mut tw, GOD, &format!("@econ/mint {}={} 250", d.faucet, d.log_acct));
    assert_eq!(bal(&tw, d.log_acct), 1250, "a dissolving org could not be settled with");
    err(&mut tw, d.quinn, &format!("@econ/pay {}={} 10", d.log_acct, d.pat_acct));
    assert_eq!(bal(&tw, d.log_acct), 1250, "a dissolving org authorized a spend");

    // Suspended: nothing moves in either direction, staff included — a suspension
    // that money can still flow through is not a suspension.
    set_org_status(&mut tw, &mut kv, d.org, "Suspended");
    err(&mut tw, GOD, &format!("@econ/mint {}={} 250", d.faucet, d.log_acct));
    assert_eq!(bal(&tw, d.log_acct), 1250);
    err(&mut tw, d.quinn, &format!("@econ/pay {}={} 10", d.log_acct, d.pat_acct));

    // Closed likewise.
    set_org_status(&mut tw, &mut kv, d.org, "Closed");
    err(&mut tw, GOD, &format!("@econ/mint {}={} 250", d.faucet, d.log_acct));
    assert_eq!(bal(&tw, d.log_acct), 1250);
}

#[test]
fn a_scope_retires_only_after_the_economy_lets_go_of_it() {
    let (mut tw, _p) = boot();
    let d = divisions(&mut tw);

    // An open wallet blocks it...
    let seen = err(&mut tw, GOD, &format!("@org/scope retire Ironhold=#{}", d.ops.0));
    assert!(seen.contains("still in use") && seen.contains("account"), "{seen}");

    // ...so drain and close the wallet. The vault still holds the scope.
    ok(&mut tw, GOD, &format!("@econ/pay {}={} 1000", d.ops_acct, d.pat_acct));
    ok(&mut tw, GOD, &format!("@account/close {}", d.ops_acct));
    assert_eq!(snap(&tw).account(d.ops_acct).expect("account").status, AccountStatus::Closed);
    let seen = err(&mut tw, GOD, &format!("@org/scope retire Ironhold=#{}", d.ops.0));
    assert!(seen.contains("vault"), "{seen}");

    ok(&mut tw, GOD, &format!("@vault/close {}", d.ops_vault));
    ok(&mut tw, GOD, &format!("@org/scope retire Ironhold=#{}", d.ops.0));
    assert_eq!(org_state(&tw).scope(d.ops).expect("scope").status, ScopeStatus::Retired);
    // And a closed account cannot be posted to afterwards, retired scope or not.
    err(&mut tw, GOD, &format!("@econ/mint {}={} 10", d.faucet, d.ops_acct));
}

#[test]
fn an_org_wallet_opens_only_on_a_live_scope_of_its_own_org() {
    let (mut tw, probe) = boot();
    let d = divisions(&mut tw);
    ok(&mut tw, GOD, "@org/create Tremere");
    let other = org_state(&tw).orgs().find(|o| o.name == "Tremere").expect("org").root_scope;

    // Invariant 18 at open: the scope must belong to the named org...
    err(&mut tw, GOD, &format!("@account/open org:Ironhold/{}=gold", other.0));
    // ...and be live.
    submit_org(&mut tw, &probe, vec![OrgMutationOp::RetireScope { scope: d.ops }]);
    let seen = err(&mut tw, GOD, &format!("@account/open org:Ironhold/{}=gold", d.ops.0));
    assert!(seen.contains("not active") || seen.contains("does not belong"), "{seen}");
    // The live sibling still opens.
    ok(&mut tw, GOD, &format!("@account/open org:Ironhold/{}=gold", d.log.0));
    assert_eq!(snap(&tw).accounts().filter(|a| a.scope == Some(d.log)).count(), 2);
}

// ======================================================================
// The rest of §4.4–§4.7
// ======================================================================

#[test]
fn the_grant_command_refuses_an_unknown_capability_and_a_pointless_limit() {
    let (mut tw, _p) = boot();
    let d = divisions(&mut tw);
    let before = org_state(&tw).grants_in(d.org).count();

    let seen = err(&mut tw, GOD, &format!("@org/grant #{}=Ironhold economy.acount.post", d.quinn.0));
    assert!(seen.contains("Unknown capability"), "{seen}");
    // A13: `vault.withdraw` requests carry no amount, so a limit denies forever.
    let seen = err(&mut tw, GOD, &format!("@org/grant #{}=Ironhold vault.withdraw scope=#{} limit=5", d.quinn.0, d.log.0));
    assert!(seen.contains("amount-carrying"), "{seen}");
    assert_eq!(org_state(&tw).grants_in(d.org).count(), before, "a refused grant was stored");

    // The amount-carrying one takes a limit, and it bites per operation.
    ok(&mut tw, GOD, &format!("@org/grant #{}=Ironhold economy.account.post scope=#{} limit=100", d.quinn.0, d.log.0));
    ok(&mut tw, d.quinn, &format!("@econ/pay {}={} 100", d.log_acct, d.pat_acct));
    err(&mut tw, d.quinn, &format!("@econ/pay {}={} 101", d.log_acct, d.pat_acct));
    assert_eq!(bal(&tw, d.log_acct), 900);
}

#[test]
fn an_approval_rule_is_never_a_shield_against_a_reversal() {
    let (mut tw, _p) = boot();
    let d = divisions(&mut tw);
    ok(&mut tw, GOD, &format!("@account/approval {}=100 threshold:2", d.log_acct));
    ok(&mut tw, GOD, &format!("@econ/mint {}={} 500", d.faucet, d.log_acct));
    let tx = store(&tw).transactions().last().expect("the payment").id;
    assert_eq!(bal(&tw, d.log_acct), 1500);

    // The clawback nets a 500 debit on a rule-carrying account, far above its
    // threshold — and posts anyway (A7): a reversal is not a new spend, and a rule
    // that could block one would be a way to keep stolen money.
    ok(&mut tw, GOD, &format!("@econ/reverse {tx}"));
    assert_eq!(bal(&tw, d.log_acct), 1000, "the reversal did not post");
    assert!(requests(&tw).is_empty(), "a reversal minted an approval request");
}

#[test]
fn the_module_layer_refuses_a_plan_carrying_two_custody_ops() {
    let (mut tw, probe) = boot();
    let d = divisions(&mut tw);
    let a = thing(&mut tw, "widget a");
    let b = thing(&mut tw, "widget b");
    tw.world_mut().move_to(a, GOD).expect("carry a");
    tw.world_mut().move_to(b, GOD).expect("carry b");
    let before = journal_len(&tw);

    // The pure engine is happy with this (one custody op per OBJECT), but the module
    // applies custody one provider call at a time: a refusal on the second move would
    // strand the first in the world with nothing journaled. Refused before any
    // provider is touched.
    let mut plan = EconomyPlan::new(
        IdempotencyKey::new("test", "two-custody"),
        PrincipalId::Object(GOD),
        EconomicCause::Command { serial: 0 },
        Vec::new(),
    );
    plan.custody_ops = vec![
        CustodyOp::Deposit { object: a, vault: d.log_vault },
        CustodyOp::Deposit { object: b, vault: d.log_vault },
    ];
    probe.stage_econ(plan);
    ok(&mut tw, GOD, "@probe/post");
    assert_eq!(
        probe.outcome(),
        Err(EconomyError::TooLarge("one custody op per plan at the module layer"))
    );
    assert_eq!(tw.world().get(a).expect("a").location, Some(GOD), "widget a moved");
    assert_eq!(tw.world().get(b).expect("b").location, Some(GOD), "widget b moved");
    assert_eq!(journal_len(&tw), before, "the refused plan posted");

    // One op is fine, through the ordinary command.
    ok(&mut tw, GOD, &format!("@vault/deposit #{}={}", a.0, d.log_vault));
}

#[test]
fn account_reads_are_hidden_and_field_limited_for_non_staff() {
    let (mut tw, _p) = boot();
    let d = divisions(&mut tw);
    ok(&mut tw, GOD, &format!("@econ/mint {}={} 40", d.faucet, d.pat_acct));

    // Pat reads their own account: the §4.5 field set, and nothing beyond it.
    let seen = ok(&mut tw, d.pat, &format!("@account/show {}", d.pat_acct));
    assert!(seen.contains("gold") && seen.contains("40") && seen.contains("Active"), "{seen}");
    assert!(!seen.contains("owner") && !seen.contains("revision") && !seen.contains("scope"), "field set leaked: {seen}");
    // Staff see the rest.
    let seen = ok(&mut tw, GOD, &format!("@account/show {}", d.pat_acct));
    assert!(seen.contains("owner") && seen.contains("revision"), "{seen}");

    // An account Pat has no standing on is simply not there.
    let seen = err(&mut tw, d.pat, &format!("@account/show {}", d.log_acct));
    assert!(seen.contains("No such account."), "{seen}");
    // A nonexistent id renders identically — that is the whole point.
    assert_eq!(seen, err(&mut tw, d.pat, "@account/show 9999"));
    // The list shows exactly what the shows do.
    let seen = ok(&mut tw, d.pat, "@account/list");
    assert!(seen.contains("Pat") && !seen.contains("Ironhold") && !seen.contains("faucet"), "{seen}");

    // An approval rule is part of the non-staff field set (you can see what will
    // escalate on you), and it renders on the list too.
    ok(&mut tw, GOD, &format!("@account/approval {}=25 threshold:1", d.pat_acct));
    let seen = ok(&mut tw, d.pat, &format!("@account/show {}", d.pat_acct));
    assert!(seen.contains("approval above 25"), "{seen}");
    let seen = ok(&mut tw, d.pat, "@account/list");
    assert!(seen.contains("approval rule"), "{seen}");
}

#[test]
fn approval_rules_refuse_the_shapes_that_could_never_gate_anything() {
    let (mut tw, _p) = boot();
    let d = divisions(&mut tw);

    // A11: the one authorized actor is the payer, and four-eyes bars them.
    let seen = err(&mut tw, GOD, &format!("@account/approval {}=100 one", d.log_acct));
    assert!(seen.contains("never be satisfied"), "{seen}");
    // A zero-approval policy escalates and then waves the plan through.
    err(&mut tw, GOD, &format!("@account/approval {}=100 threshold:0", d.log_acct));
    // D-E3-11: a role-based policy needs a role that actually exists in the org.
    let seen = err(&mut tw, GOD, &format!("@account/approval {}=100 threshold:1:treasurer", d.log_acct));
    assert!(seen.contains("No such role"), "{seen}");
    // A named set smaller than the approvals it demands is unsatisfiable too.
    err(&mut tw, GOD, &format!("@account/approval {}=100 named:2:#{}", d.log_acct, d.pat.0));
    assert!(snap(&tw).account(d.log_acct).expect("account").approval.is_none(), "a refused rule was stored");

    // A real one sets, and `none` clears.
    ok(&mut tw, GOD, &format!("@account/approval {}=100 threshold:1", d.log_acct));
    assert!(snap(&tw).account(d.log_acct).expect("account").approval.is_some());
    ok(&mut tw, GOD, &format!("@account/approval {}=none", d.log_acct));
    assert!(snap(&tw).account(d.log_acct).expect("account").approval.is_none());
}

#[test]
fn a_role_based_policy_needs_the_capability_and_a_real_membership() {
    let (mut tw, probe) = boot();
    let d = divisions(&mut tw);
    let ada = tw.spawn_player("Ada");
    let bo = tw.spawn_player("Bo");
    let cy = tw.spawn_player("Cy");
    grant(&mut tw, d.quinn, "economy.account.post", d.log);
    grant(&mut tw, d.quinn, "economy.account.view", d.log);
    grant(&mut tw, ada, "economy.account.approve", d.log);
    grant(&mut tw, bo, "economy.account.approve", d.log);
    let role = tw.world_mut().module_state_mut::<OrgStore>().expect("org store").alloc_role();
    let member = |who: ObjId| OrgMembership {
        org: d.org,
        member: who,
        status: MembershipStatus::Active,
        titles: vec![],
        roles: BTreeSet::from([role]),
        visibility: Visibility::Public,
        since: 0,
        revision: 1,
    };
    submit_org(&mut tw, &probe, vec![
        OrgMutationOp::CreateRole {
            role: OrgRole { id: role, org: d.org, name: "treasurer".into(), grants: vec![], revision: 1 },
        },
        // Ada has the capability AND the role; Cy has only the role.
        OrgMutationOp::PutMembership { membership: member(ada) },
        OrgMutationOp::PutMembership { membership: member(cy) },
    ]);
    ok(&mut tw, GOD, &format!("@account/approval {}=100 threshold:1:treasurer", d.log_acct));

    let pay = format!("@econ/pay {}={} 500", d.log_acct, d.pat_acct);
    ok(&mut tw, d.quinn, &pay);
    let r = only_request(&tw);

    // `from_role` is a CONJUNCTION, and standing is exactly "would this count" — so
    // neither half alone can even record a decision: the id reads as unknown.
    let unknown = err(&mut tw, bo, &format!("@econ/approve {}", r.id)); // capability, no role
    assert!(unknown.contains("No such approval request."), "{unknown}");
    assert_eq!(unknown, err(&mut tw, cy, &format!("@econ/approve {}", r.id))); // role, no capability
    assert_eq!(snap(&tw).approval(r.id).expect("request").decisions.len(), 0, "a half-qualified actor voted");

    // Ada holds both, through a REAL membership (D-E3-11).
    ok(&mut tw, ada, &format!("@econ/approve {}", r.id));
    ok(&mut tw, d.quinn, &pay);
    assert_eq!(bal(&tw, d.log_acct), 500);

    // Suspending the membership takes the role away without deleting anything.
    let pay2 = format!("@econ/pay {}={} 400", d.log_acct, d.pat_acct);
    ok(&mut tw, d.quinn, &pay2);
    let r2 = only_request(&tw);
    ok(&mut tw, ada, &format!("@econ/approve {}", r2.id));
    let mut suspended = member(ada);
    suspended.status = MembershipStatus::Suspended;
    suspended.revision = 2;
    submit_org(&mut tw, &probe, vec![OrgMutationOp::PutMembership { membership: suspended }]);
    let seen = ok(&mut tw, d.quinn, &pay2);
    assert!(seen.contains("Approval required"), "a suspended member still satisfied from_role: {seen}");
    assert_eq!(bal(&tw, d.log_acct), 500);
}

#[test]
fn close_and_unfreeze_are_doc_writes_that_leave_the_journal_alone() {
    let (mut tw, _p) = boot();
    let d = divisions(&mut tw);

    // Closing needs an empty, active account and the authority to spend from it.
    ok(&mut tw, GOD, &format!("@econ/mint {}={} 50", d.faucet, d.pat_acct));
    let seen = err(&mut tw, d.pat, &format!("@account/close {}", d.pat_acct));
    assert!(seen.contains("still holds a balance"), "{seen}");
    ok(&mut tw, d.pat, &format!("@econ/pay {}={} 50", d.pat_acct, d.log_acct));
    // The revision the JOURNAL left behind; the close must not move it.
    let before = snap(&tw).account(d.pat_acct).expect("account").clone();
    ok(&mut tw, d.pat, &format!("@account/close {}", d.pat_acct));
    let after = snap(&tw).account(d.pat_acct).expect("account").clone();
    assert_eq!(after.status, AccountStatus::Closed);
    assert_eq!(after.revision, before.revision, "a doc-authority write bumped the journal revision");

    // An org wallet needs post authority, not merely a view of it.
    grant(&mut tw, d.quinn, "economy.account.view", d.log);
    err(&mut tw, d.quinn, &format!("@account/close {}", d.log_acct));
    grant(&mut tw, d.quinn, "economy.account.post", d.log);
    ok(&mut tw, GOD, &format!("@econ/pay {}={} 1050", d.log_acct, d.ops_acct));
    ok(&mut tw, d.quinn, &format!("@account/close {}", d.log_acct));

    // Unfreeze is staff-only and only clears a freeze.
    err(&mut tw, d.pat, &format!("@account/unfreeze {}", d.pat_acct));
    err(&mut tw, GOD, &format!("@account/unfreeze {}", d.pat_acct));
}

#[test]
fn a_frozen_account_recovers_through_unfreeze_without_disturbing_the_journal() {
    let (mut tw, _p) = boot();
    let d = divisions(&mut tw);
    // Freeze Pat's account the way the destruction backstop does, then recover it.
    let joe = tw.spawn_player("Joe");
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", joe.0));
    let joe_acct = account_of(&tw, &PrincipalId::Object(joe));
    ok(&mut tw, GOD, &format!("@econ/mint {}={} 100", d.faucet, joe_acct));
    let revision = snap(&tw).account(joe_acct).expect("account").revision;
    tw.world_mut().destroy(joe).expect("destroy");
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    tw.engine_mut().emit_world_event(&mush_core::WorldEvent::ObjDestroyed { obj: joe }, &mut out, &mut fx);
    assert_eq!(snap(&tw).account(joe_acct).expect("account").status, AccountStatus::Frozen);

    ok(&mut tw, GOD, &format!("@account/unfreeze {joe_acct}"));
    let a = snap(&tw).account(joe_acct).expect("account").clone();
    assert_eq!(a.status, AccountStatus::Active);
    assert_eq!(a.posted_balance.minor(), 100, "the recovery touched the money");
    assert_eq!(a.revision, revision + 1, "only the backstop's own bump is on the record");

    // ...and the recovered account survives a reboot as Active.
    let mut kv: Kv = HashMap::new();
    fold_kv(&mut tw, &mut kv);
    reboot(&mut tw, &kv);
    assert_eq!(snap(&tw).account(joe_acct).expect("account").status, AccountStatus::Active);
    assert!(!store(&tw).is_frozen(), "the doc write read as drift at the next boot");
}

#[test]
fn an_approval_id_the_actor_cannot_act_on_is_simply_unknown() {
    let (mut tw, _p) = boot();
    let a = approvals_fixture(&mut tw, "threshold:1");
    ok(&mut tw, a.d.quinn, &format!("@econ/pay {}={} 500", a.d.log_acct, a.d.pat_acct));
    let r = only_request(&tw);

    // Pat holds nothing on logistics: the live id and a nonexistent one read the
    // same, and neither `@econ/approvals` nor a denial admits the request exists.
    let unknown = err(&mut tw, a.d.pat, &format!("@econ/approve {}", r.id));
    assert_eq!(unknown, err(&mut tw, a.d.pat, "@econ/approve 4242"));
    assert_eq!(unknown, err(&mut tw, a.d.pat, &format!("@econ/deny {}", r.id)));
    let seen = ok(&mut tw, a.d.pat, "@econ/approvals");
    assert!(seen.contains("No approval requests await you."), "{seen}");
    assert_eq!(snap(&tw).approval(r.id).expect("request").decisions.len(), 0, "an outsider recorded a decision");

    // Ada sees it and can act.
    let seen = ok(&mut tw, a.ada, "@econ/approvals");
    assert!(seen.contains(&format!("#{}", r.id)), "{seen}");
    ok(&mut tw, a.ada, &format!("@econ/approve {}", r.id));
    // Approving twice is a no-op, not a second vote.
    ok(&mut tw, a.ada, &format!("@econ/approve {}", r.id));
    assert_eq!(snap(&tw).approval(r.id).expect("request").approvals.len(), 1);
}

#[test]
fn the_per_account_pending_cap_holds_across_requesters() {
    let (mut tw, _p) = boot();
    let a = approvals_fixture(&mut tw, "threshold:1");
    // The per-requester cap (4) fires first for one payer, so filling the
    // per-account cap (16) takes several — which is exactly the shape that matters:
    // one account cannot be buried under other people's pending requests either.
    let payers: Vec<ObjId> = (0..5)
        .map(|i| {
            let p = tw.spawn_player(&format!("Payer{i}"));
            grant(&mut tw, p, "economy.account.post", a.d.log);
            grant(&mut tw, p, "economy.account.view", a.d.log);
            p
        })
        .collect();
    let mut amount = 200;
    let mut minted = 0usize;
    for p in &payers {
        for _ in 0..mush_economy::MAX_PENDING_PER_REQUESTER {
            if minted == mush_economy::MAX_PENDING_PER_ACCOUNT {
                break;
            }
            amount += 1;
            ok(&mut tw, *p, &format!("@econ/pay {}={} {amount}", a.d.log_acct, a.d.pat_acct));
            minted += 1;
        }
    }
    assert_eq!(requests(&tw).len(), mush_economy::MAX_PENDING_PER_ACCOUNT);

    // A fresh payer, well under their own cap, is still refused — and told which
    // cap they hit.
    amount += 1;
    let seen = err(&mut tw, payers[4], &format!("@econ/pay {}={} {amount}", a.d.log_acct, a.d.pat_acct));
    assert!(seen.contains("too many pending approval requests"), "{seen}");
    assert_eq!(requests(&tw).len(), mush_economy::MAX_PENDING_PER_ACCOUNT, "the cap leaked a request");
    assert_eq!(bal(&tw, a.d.log_acct), 1000, "nothing posted along the way");
}
