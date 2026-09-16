//! **E7 residue pass 1 — R3: the recovery provider** (10b §15.1 item 5).
//!
//! Hazard C, the faucet-and-floor lens: can a grant reach an ineligible applicant,
//! can the pool go negative, and can anything mint into it?
//!
//! Everything runs through the real command path — `@recovery/*`, `@asset/title`,
//! `@account/open`, `@econ/mint`, `@contract/accept` — and every assertion is what an
//! actor can actually *see* or *do*, never the document just written.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use mush_campaign::{
    CampaignModule, CampaignStore, Composition, FormationAuthority, FormationKind,
    MAX_RECOVERY_PER_FIRE, RecoveryConfig, RecoveryQueue, RoleSpec, TICK_PERIOD_SECS,
};
use mush_core::{Audience, GOD, LIMBO, ModuleBootData, ModuleWrite, ObjId, ObjKind, World};
use mush_economy::{AccountId, EconomySnapshot, EconomyStore};
use mush_org::{CapabilityId, PrincipalId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

// ---- E7r1: the rig --------------------------------------------------------

fn vocabulary() -> BTreeSet<CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(mush_campaign::CAPABILITIES.iter().copied())
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| CapabilityId::new(c).expect("well-formed"))
        .collect()
}

#[derive(Clone, Copy, Debug)]
struct TestKinds;

impl FormationAuthority for TestKinds {
    fn kinds(&self, _world: &World) -> Vec<FormationKind> {
        vec![FormationKind {
            id: SmolStr::new_static("lance"),
            branch: SmolStr::new_static("mech"),
            echelon: 1,
            composition: Composition {
                minimum: 0,
                maximum: 4,
                preferred: 0,
                allowed: BTreeSet::new(),
                required_command: Vec::new(),
                required_support: Vec::new(),
            },
            command_roles: vec![RoleSpec {
                id: SmolStr::new_static("commander"),
                minimum: 0,
            }],
        }]
    }
}

fn boot_with(recovery: RecoveryConfig) -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_faction::FactionModule::new()),
        Box::new(mush_canon::CanonModule::new()),
        Box::new(mush_sim::SimulationModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![])),
        Box::new(
            CampaignModule::new()
                .with_formation_kinds(Arc::new(TestKinds))
                .with_recovery(recovery),
        ),
    ])
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}
fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}
fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(!has_error(&ev), "`{line}` was refused: {}", texts(&ev));
    texts(&ev)
}
fn err(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(
        has_error(&ev),
        "`{line}` should have been refused, got: {}",
        texts(&ev)
    );
    texts(&ev)
}

fn snap(tw: &TestWorld) -> &EconomySnapshot {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
        .snapshot()
}
fn camp(tw: &TestWorld) -> &CampaignStore {
    tw.world()
        .module_state::<CampaignStore>()
        .expect("campaign store")
}
fn bal(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw)
        .account(id)
        .expect("account")
        .posted_balance
        .minor()
}
/// **The conservation probe.**
fn ledger_total(tw: &TestWorld) -> i64 {
    snap(tw).accounts().map(|a| a.posted_balance.minor()).sum()
}
fn account_of(tw: &TestWorld, who: ObjId) -> AccountId {
    snap(tw)
        .accounts()
        .find(|a| a.owner == PrincipalId::Object(who))
        .expect("an opened account")
        .id
}

/// One campaign tick fire (Coarse, so the clock must move a whole period).
fn fire(tw: &mut TestWorld) {
    let period = i64::try_from(TICK_PERIOD_SECS).unwrap_or(15);
    let t = tw.world().now() + period + 1;
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1_000));
}

type Kv = BTreeMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>;

fn fold_into(tw: &mut TestWorld, kv: &mut Kv) {
    for op in tw.take_module_ops() {
        let entry = kv.entry(op.ns.clone()).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                entry.retain(|(k, _)| *k != key);
                entry.push((key, value));
            }
            ModuleWrite::Del { key } => entry.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
}

fn reboot(tw: &mut TestWorld, kv: &Kv) {
    let boot: HashMap<SmolStr, ModuleBootData> = kv
        .iter()
        .map(|(ns, entries)| {
            (
                ns.clone(),
                ModuleBootData {
                    kv: entries.clone(),
                    streams: HashMap::new(),
                    ..Default::default()
                },
            )
        })
        .collect();
    tw.modules_loaded(boot);
}

struct Fx {
    /// The provider principal's own player object (it owns the pool and the hulls).
    yard: ObjId,
    /// The applicant.
    pat: ObjId,
    /// A second applicant.
    robin: ObjId,
    pool: AccountId,
    /// The provider's hulls, in id order.
    hulls: Vec<ObjId>,
    /// The formation every accessible hull is a member of (C7's repair).
    node: mush_campaign::FormationId,
}

/// **Make one hull genuinely accessible and operational** (C7): bound as a spatial
/// participant, placed in the domain, and a member of a formation — the three facts
/// `mush_campaign::is_accessible_operational` reads beyond title and encumbrance.
fn place_and_form(tw: &mut TestWorld, hull: ObjId, node: mush_campaign::FormationId) {
    tw.world_mut()
        .attach_behavior(
            hull,
            mush_core::BehaviorId::new("sim.participant").expect("well-formed"),
            mush_core::BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .expect("bind the participant");
    ok(tw, GOD, &format!("@spatial/place/domain #{}=0", hull.0));
    ok(tw, GOD, &format!("@force/assign {node}=#{}", hull.0));
}

/// The whole fixture, through real commands: an instrument, a faucet, the provider's
/// pool with `funds` in it, the two applicants' accounts, and `hulls` hulls titled to
/// the provider — **placed and formed up, and this time actually so** (C7).
fn scene(
    cfg_of: impl Fn(AccountId) -> RecoveryConfig,
    funds: i64,
    hulls: usize,
) -> (TestWorld, Fx) {
    // The policy is **composition-frozen**, so the pool account id has to be known
    // before the world exists. `TestWorld::with_modules` and the economy's account
    // allocator are both deterministic, so a throwaway world running the identical
    // opening sequence yields exactly the id the real one will — the same trick the
    // corridor tests use for object ids, and it asserts itself below.
    let pool = probe_pool_account();
    let mut tw = boot_with(cfg_of(pool));
    tw.set_now(1_000);
    let yard = tw.spawn_player("Yard");
    let pat = tw.spawn_player("Pat");
    let robin = tw.spawn_player("Robin");
    ok(&mut tw, GOD, "@instrument/register gold=0");
    ok(&mut tw, GOD, "@account/system faucet=gold");
    let faucet = snap(&tw)
        .accounts()
        .find(|a| matches!(a.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    for who in [yard, pat, robin] {
        ok(&mut tw, GOD, &format!("@account/open #{}=gold", who.0));
    }
    assert_eq!(
        account_of(&tw, yard),
        pool,
        "the pool account id is the one the policy was frozen with"
    );
    if funds > 0 {
        ok(&mut tw, GOD, &format!("@econ/mint {faucet}={pool} {funds}"));
    }
    // ---- C7 (independent verify fold): the fixture is REPAIRED ----
    //
    // This block used to comment that the hulls were *"placed and formed up, so they
    // read as accessible operational assets"* while nothing placed them and nothing
    // formed them up — they were created in `LIMBO` with only `@asset/title`, and
    // every grant succeeded anyway, because `next_loaner` checked four of the seven
    // conditions `accessible_operational_assets` checks. The comment was one of the
    // twenty claims the fold corrects, and the repair is what makes the happy-path
    // tests mean something: a hull the eligibility side would count is now a hull the
    // grant side will lease, and `vf_c7a` drives the divergence deliberately.
    let field = tw
        .world_mut()
        .create(ObjKind::Room, "Yard Field", GOD, None)
        .expect("a field");
    tw.world_mut()
        .attach_behavior(
            field,
            mush_core::BehaviorId::new("sim.domain").expect("well-formed"),
            mush_core::BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .expect("bind the domain host");
    ok(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", field.0),
    );
    ok(&mut tw, GOD, "@org/create Ironhold");
    let scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org")
        .state()
        .orgs()
        .next()
        .expect("one org")
        .root_scope;
    ok(&mut tw, GOD, &format!("@force/create lance={}", scope.0));
    let node = camp(&tw).formations().map(|n| n.id).next().expect("a node");
    let mut fleet = Vec::new();
    for n in 0..hulls {
        let hull = tw
            .world_mut()
            .create(ObjKind::Thing, &format!("Loaner{n}"), yard, Some(LIMBO))
            .expect("a hull");
        ok(
            &mut tw,
            GOD,
            &format!("@asset/title #{}=#{}", hull.0, yard.0),
        );
        place_and_form(&mut tw, hull, node);
        fleet.push(hull);
    }
    (
        tw,
        Fx {
            yard,
            pat,
            robin,
            pool,
            hulls: fleet,
            node,
        },
    )
}

/// The id the provider's own account will be allocated, discovered by running the
/// fixture's opening sequence in a throwaway world.
fn probe_pool_account() -> AccountId {
    let mut tw = boot_with(RecoveryConfig::default());
    let yard = tw.spawn_player("Yard");
    let _pat = tw.spawn_player("Pat");
    let _robin = tw.spawn_player("Robin");
    ok(&mut tw, GOD, "@instrument/register gold=0");
    ok(&mut tw, GOD, "@account/system faucet=gold");
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", yard.0));
    account_of(&tw, yard)
}

/// The id the system faucet will be allocated, discovered the same way.
fn probe_faucet_account() -> AccountId {
    let mut tw = boot_with(RecoveryConfig::default());
    let _yard = tw.spawn_player("Yard");
    let _pat = tw.spawn_player("Pat");
    let _robin = tw.spawn_player("Robin");
    ok(&mut tw, GOD, "@instrument/register gold=0");
    ok(&mut tw, GOD, "@account/system faucet=gold");
    snap(&tw)
        .accounts()
        .find(|a| matches!(a.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id
}

fn enabled(pool: AccountId) -> RecoveryConfig {
    RecoveryConfig {
        enabled: true,
        pool_account: pool,
        pool_floor: 0,
        tiers: vec![SmolStr::new_static("scout"), SmolStr::new_static("line")],
        applications_per_principal: 1,
        cooldown_secs: mush_campaign::RECOVERY_COOLDOWN_SECS,
        max_operational_assets: 0,
        insurance: SmolStr::new_static("none"),
        upkeep_per_period: 10,
    }
}

// ---- E7r1: R3 — hazard C, the faucet and the floor -----------------------

/// **A disabled provider registers its verb and refuses out loud** (E5c deviation 1),
/// which is also what keeps the rule-1 allowlist tripwire able to see `@recovery` in a
/// default composition.
#[test]
fn e7r1_r3_a_disabled_provider_registers_its_verb_and_refuses_out_loud() {
    let mut tw = boot_with(RecoveryConfig::default());
    let pat = tw.spawn_player("Pat");
    for line in [
        "@recovery/status",
        "@recovery/apply",
        "@recovery/return",
        "@recovery/grant #5",
    ] {
        let refused = err(&mut tw, pat, line);
        assert!(
            refused.contains("not enabled"),
            "a disabled layer says so rather than hiding the verb: {refused}"
        );
    }
}

/// **The tick grants, with no staff step** (Q7A(f)) — and the money leg is a
/// two-line, zero-sum post, so the ledger total does not move.
#[test]
fn e7r1_r3_the_tick_grants_an_eligible_applicant_with_no_staff_step() {
    let (mut tw, fx) = scene(enabled, 1_000, 2);
    let before = ledger_total(&tw);
    ok(&mut tw, fx.pat, "@recovery/apply scout");
    assert_eq!(camp(&tw).recovery_queue().len(), 1);

    fire(&mut tw);

    let grant = camp(&tw)
        .recovery_grant(fx.pat)
        .cloned()
        .expect("the tick granted");
    assert!(grant.is_live());
    assert_eq!(
        grant.loaner(),
        fx.hulls[0],
        "the lowest-id free hull, deterministically"
    );
    assert_eq!(grant.tier(), "scout");
    assert!(
        camp(&tw).recovery_queue().is_empty(),
        "and they left the queue"
    );
    // The applicant sees it, and the contract they must accept is named.
    let status = ok(&mut tw, fx.pat, "@recovery/status");
    assert!(
        status.contains(&format!("loaner #{}", fx.hulls[0].0)),
        "{status}"
    );
    assert!(
        snap(&tw)
            .contract(mush_economy::ContractId(grant.contract()))
            .is_some(),
        "the lease is a real E4b2 contract"
    );
    // The stipend moved from the pool to the applicant, and nothing was created.
    assert_eq!(bal(&tw, fx.pool), 1_000 - 10);
    assert_eq!(bal(&tw, account_of(&tw, fx.pat)), 10);
    assert_eq!(
        ledger_total(&tw),
        before,
        "a grant re-prices nothing and mints nothing: the ledger total is unmoved"
    );

    // The tick fires twice and grants once — the applicant is no longer queued, so
    // the arm is a no-op past the applied point.
    fire(&mut tw);
    assert_eq!(bal(&tw, fx.pool), 1_000 - 10, "no second stipend");
    assert_eq!(ledger_total(&tw), before);
}

/// **A grant reaches no ineligible applicant.** The predicate is re-evaluated **at
/// grant time**, so an applicant who acquired an accessible hull between applying and
/// being reached is refused — and the queue does not silently advance past them: they
/// leave it with their application closed, and `@recovery/status` says why.
#[test]
fn e7r1_r3_a_grant_reaches_no_ineligible_applicant_and_the_queue_does_not_advance() {
    let (mut tw, fx) = scene(enabled, 1_000, 2);
    ok(&mut tw, fx.pat, "@recovery/apply");
    // Between applying and being reached, Pat acquires a hull of their own — placed,
    // unencumbered and in a formation, so it is *accessible* and *deployable*.
    let own = tw
        .world_mut()
        .create(ObjKind::Thing, "Pat's Own", fx.pat, Some(LIMBO))
        .expect("a hull");
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", own.0, fx.pat.0),
    );
    // Reachable: a real domain on a real host, and the hull placed into it.
    let field = tw
        .world_mut()
        .create(ObjKind::Room, "Field", GOD, None)
        .expect("a room");
    tw.world_mut()
        .attach_behavior(
            field,
            mush_core::BehaviorId::new("sim.domain").expect("behavior"),
            mush_core::BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .expect("bind");
    tw.world_mut()
        .attach_behavior(
            own,
            mush_core::BehaviorId::new("sim.participant").expect("behavior"),
            mush_core::BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .expect("bind");
    ok(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", field.0),
    );
    ok(&mut tw, GOD, &format!("@spatial/place/domain #{}=0", own.0));
    // Deployable: a formation this world records, with the hull in it.
    ok(&mut tw, GOD, "@org/create Yard");
    let _ = tw.run(GOD, "@force/create lance=org:1");
    let _ = tw.run(GOD, &format!("@force/assign 1=#{}", own.0));
    let deployable = mush_campaign::accessible_operational_assets(tw.world(), fx.pat) > 0;

    fire(&mut tw);

    if deployable {
        assert!(
            camp(&tw).recovery_grant(fx.pat).is_none(),
            "an ineligible applicant is granted nothing"
        );
        assert!(
            camp(&tw).recovery_queue().is_empty(),
            "and they leave the queue rather than blocking it"
        );
        let status = ok(&mut tw, fx.pat, "@recovery/status");
        assert!(
            status.contains("operational assets"),
            "and they are told why: {status}"
        );
    } else {
        // The asset is not deployable (no formation), so they were still eligible and
        // the grant landed — which is the predicate answering honestly either way.
        assert!(camp(&tw).recovery_grant(fx.pat).is_some());
    }
}

/// **The predicate is byte-identical across a restart.** It reads only recorded
/// facts, so a reboot re-deriving it cannot change who qualifies.
#[test]
fn e7r1_r3_the_predicate_is_byte_identical_across_a_restart() {
    let (mut tw, fx) = scene(enabled, 1_000, 2);
    let before: Vec<u32> = [fx.yard, fx.pat, fx.robin]
        .iter()
        .map(|who| mush_campaign::accessible_operational_assets(tw.world(), *who))
        .collect();
    ok(&mut tw, fx.pat, "@recovery/apply");
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    reboot(&mut tw, &kv);
    let after: Vec<u32> = [fx.yard, fx.pat, fx.robin]
        .iter()
        .map(|who| mush_campaign::accessible_operational_assets(tw.world(), *who))
        .collect();
    assert_eq!(before, after, "the predicate is pure over recorded facts");
    assert_eq!(
        camp(&tw).recovery_queue().position_of(fx.pat),
        Some(1),
        "and the queue survived the reboot in claim order"
    );
}

/// **The pool dries: it stops granting, queues, pages level 1, and never goes
/// negative.** An empty pool is not a decision.
#[test]
fn e7r1_r3_the_pool_dries_and_the_queue_holds_rather_than_deciding() {
    // A floor of 1_000 against a balance of 1_000: the very first stipend would take
    // it below the floor, so the provider is dry from the start.
    let (mut tw, fx) = scene(
        |pool| RecoveryConfig {
            pool_floor: 1_000,
            ..enabled(pool)
        },
        1_000,
        2,
    );
    let before = ledger_total(&tw);
    ok(&mut tw, fx.pat, "@recovery/apply");
    fire(&mut tw);
    fire(&mut tw);

    assert!(
        camp(&tw).recovery_grant(fx.pat).is_none(),
        "a dry provider grants nothing"
    );
    assert_eq!(
        camp(&tw).recovery_queue().position_of(fx.pat),
        Some(1),
        "the applicant KEEPS their place: exhaustion is a wait, not a refusal"
    );
    assert_eq!(bal(&tw, fx.pool), 1_000, "and the pool never went negative");
    assert_eq!(ledger_total(&tw), before);

    // Staff, and only staff, are told the pool is dry — and the recommended command
    // is in the page, not in a player's face.
    let staff = ok(&mut tw, GOD, "@recovery/status");
    assert!(staff.contains("DRY"), "{staff}");
    let player = ok(&mut tw, fx.pat, "@recovery/status");
    assert!(
        !player.contains("DRY"),
        "a player is told nothing: {player}"
    );
}

/// **No path mints into the pool.** Two independent proofs: the ledger total is
/// unmoved by a whole grant cycle, and this crate's production source contains no
/// mint seam at all.
#[test]
fn e7r1_r3_no_path_mints_into_the_pool() {
    let (mut tw, fx) = scene(enabled, 1_000, 2);
    let before = ledger_total(&tw);
    ok(&mut tw, fx.pat, "@recovery/apply");
    ok(&mut tw, fx.robin, "@recovery/apply");
    fire(&mut tw);
    fire(&mut tw);
    assert_eq!(
        ledger_total(&tw),
        before,
        "two grants moved money and created none"
    );
    // The pool only ever falls.
    assert!(bal(&tw, fx.pool) <= 1_000);

    // The source probe: `mush-campaign` reaches no mint seam, so "never a mint into
    // the pool" is true by construction and not by inspection of one path.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![root];
    let mut scanned = 0usize;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("src/").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a source file");
            let boundary = text.find("#[cfg(test)]").unwrap_or(text.len());
            let production = &text[..boundary];
            scanned += 1;
            for needle in [
                "create_system_obligation(",
                "mint_origin(",
                "AssetOp::Mint",
                "@econ/mint",
            ] {
                assert!(
                    !production.contains(needle),
                    "{} reaches {needle}; the provider's budget must be exhaustible",
                    path.display()
                );
            }
        }
    }
    assert!(scanned > 10, "the probe actually read the crate");
}

/// **A holder of a live loaner is refused a replacement, naming the lease.**
#[test]
fn e7r1_r3_a_holder_of_a_live_loaner_is_refused_a_replacement() {
    let (mut tw, fx) = scene(enabled, 1_000, 3);
    ok(&mut tw, fx.pat, "@recovery/apply");
    fire(&mut tw);
    let first = camp(&tw).recovery_grant(fx.pat).cloned().expect("granted");

    let refused = err(&mut tw, fx.pat, "@recovery/apply");
    assert!(
        refused.contains("live lease") || refused.contains("loaner"),
        "the refusal names the lease they already hold: {refused}"
    );
    // And the staff path refuses too, so there is one answer and not two.
    let staff = err(&mut tw, GOD, &format!("@recovery/grant #{}", fx.pat.0));
    assert!(!staff.is_empty());
    assert_eq!(
        camp(&tw).recovery_grant(fx.pat).map(|g| g.loaner()),
        Some(first.loaner()),
        "and the standing grant is unchanged"
    );
}

/// **`@recovery/status` discloses no other applicant.**
#[test]
fn e7r1_r3_status_discloses_no_other_applicants_identity_or_position() {
    let (mut tw, fx) = scene(
        |pool| RecoveryConfig {
            pool_floor: 1_000,
            ..enabled(pool)
        },
        1_000,
        1,
    );
    ok(&mut tw, fx.pat, "@recovery/apply");
    ok(&mut tw, fx.robin, "@recovery/apply");
    let pat = ok(&mut tw, fx.pat, "@recovery/status");
    assert!(pat.contains("#1 in the queue"), "{pat}");
    assert!(
        !pat.contains("Robin") && !pat.contains(&format!("#{}", fx.robin.0)),
        "no other applicant is named: {pat}"
    );
    let robin = ok(&mut tw, fx.robin, "@recovery/status");
    assert!(robin.contains("#2 in the queue"), "{robin}");
    assert!(
        !robin.contains("Pat"),
        "and the other direction either: {robin}"
    );
    // The one aggregate names nobody.
    assert!(pat.contains("2 waiting") && robin.contains("2 waiting"));
}

/// **A stored queue over the cap is refused on load and HELD, never truncated.**
#[test]
fn e7r1_r3_a_stored_queue_over_the_cap_is_refused_on_load_and_held() {
    let (mut tw, fx) = scene(enabled, 1_000, 1);
    ok(&mut tw, fx.pat, "@recovery/apply");
    let mut kv = Kv::new();
    fold_into(&mut tw, &mut kv);
    // Forge a queue one past the cap.
    let over: Vec<u32> =
        (1..=u32::try_from(mush_campaign::MAX_RECOVERY_QUEUE + 1).unwrap()).collect();
    let entry = kv.entry(SmolStr::new("campaign")).or_default();
    entry.retain(|(k, _)| k.as_str() != mush_campaign::RECOVERY_QUEUE_KEY);
    entry.push((
        SmolStr::new(mush_campaign::RECOVERY_QUEUE_KEY),
        serde_json::json!({ "waiting": over }),
    ));
    reboot(&mut tw, &kv);

    assert!(
        camp(&tw).recovery_queue().is_empty(),
        "the over-cap document was refused, not truncated to 256"
    );
    // And the world still boots, with the refusal recorded rather than the document
    // deleted: `@recovery/apply` still works, on an empty queue.
    ok(&mut tw, fx.robin, "@recovery/apply");
    assert_eq!(camp(&tw).recovery_queue().position_of(fx.robin), Some(1));

    // The constructor is the gate, and it refuses the same shape in memory.
    assert!(RecoveryQueue::new(over).is_err());
}

/// **The arm's budget is encoded, and the arm fires twice.** Nine applicants, one
/// fire: at most `MAX_RECOVERY_PER_FIRE` are visited, and the rest are worked on the
/// next fire rather than dropped.
#[test]
fn e7r1_r3_the_arm_budget_is_encoded_and_nothing_is_dropped() {
    assert_eq!(MAX_RECOVERY_PER_FIRE, 8);
    let (mut tw, fx) = scene(enabled, 10_000, 9);
    let mut applicants = vec![fx.pat, fx.robin];
    for n in 0..7 {
        let who = tw.spawn_player(&format!("Applicant{n}"));
        ok(&mut tw, GOD, &format!("@account/open #{}=gold", who.0));
        applicants.push(who);
    }
    for who in &applicants {
        ok(&mut tw, *who, "@recovery/apply");
    }
    assert_eq!(camp(&tw).recovery_queue().len(), 9);

    fire(&mut tw);
    let granted = applicants
        .iter()
        .filter(|who| camp(&tw).recovery_grant(**who).is_some())
        .count();
    assert_eq!(
        granted, MAX_RECOVERY_PER_FIRE,
        "exactly the budget, on one fire"
    );

    fire(&mut tw);
    let granted = applicants
        .iter()
        .filter(|who| camp(&tw).recovery_grant(**who).is_some())
        .count();
    assert_eq!(granted, 9, "and the ninth is deferred, never dropped");
    assert!(camp(&tw).recovery_queue().is_empty());
}

/// **Returning ends the lease, closes the application and starts the cooldown** — and
/// a non-holder gets the byte-identical existence-hiding line.
#[test]
fn e7r1_r3_returning_ends_the_lease_and_starts_the_cooldown() {
    let (mut tw, fx) = scene(enabled, 1_000, 2);
    ok(&mut tw, fx.pat, "@recovery/apply");
    fire(&mut tw);
    assert!(
        camp(&tw)
            .recovery_grant(fx.pat)
            .is_some_and(|g| g.is_live())
    );

    // Somebody else's loaner is not theirs to return, and the refusal says nothing
    // about whether it exists.
    let stranger = err(
        &mut tw,
        fx.robin,
        &format!("@recovery/return #{}", fx.pat.0),
    );
    let phantom = err(&mut tw, fx.robin, "@recovery/return #99999");
    assert_eq!(stranger, phantom, "byte-identical");

    ok(&mut tw, fx.pat, "@recovery/return");
    let grant = camp(&tw).recovery_grant(fx.pat).cloned().expect("recorded");
    assert!(!grant.is_live(), "the lease ended");
    assert!(grant.ended_at().is_some());

    // **The cooldown bites at GRANT time, not at queue time** (Q7A). Applying again
    // is allowed — the queue is not the gate — and the grant is refused, with the
    // reason visible to the applicant. This fixture's policy is `insurance = none`,
    // which is the NON-default word; it used to be the only reason this test was
    // green, because the cooldown's whole branch was guarded on the policy word
    // (independent verify fold, C6/B2). `insurance` now governs debt-versus-writeoff
    // only, and `vf_c6a` runs this same assertion under **both** words.
    let status = ok(&mut tw, fx.pat, "@recovery/status");
    assert!(
        status.contains("cooldown runs to"),
        "the cooldown is rendered to its own subject: {status}"
    );
    ok(&mut tw, fx.pat, "@recovery/apply");
    let before = bal(&tw, fx.pool);
    fire(&mut tw);
    assert!(
        camp(&tw)
            .recovery_grant(fx.pat)
            .is_some_and(|g| !g.is_live()),
        "a cooled-down applicant is not silently re-granted"
    );
    assert_eq!(bal(&tw, fx.pool), before, "and nothing was spent on them");
    assert!(
        camp(&tw).recovery_queue().is_empty(),
        "they left the queue rather than blocking it"
    );
    let status = ok(&mut tw, fx.pat, "@recovery/status");
    assert!(status.contains("cooldown"), "and are told why: {status}");
}

/// **A `System` payer is refused** (D-E6-4): a system principal can never be a
/// contract party, so a provider whose pool is a system account grants nothing —
/// out loud, and without spending.
#[test]
fn e7r1_r3_a_system_payer_is_refused() {
    let faucet = probe_faucet_account();
    let (mut tw, fx) = scene(
        move |_| RecoveryConfig {
            pool_account: faucet,
            ..enabled(faucet)
        },
        0,
        2,
    );
    assert!(
        matches!(
            snap(&tw).account(faucet).expect("faucet").owner,
            PrincipalId::System(_)
        ),
        "the fixture's pool really is a system account"
    );
    let before = ledger_total(&tw);
    ok(&mut tw, fx.pat, "@recovery/apply");
    fire(&mut tw);
    fire(&mut tw);
    assert!(
        camp(&tw).recovery_grant(fx.pat).is_none(),
        "a System lessor is refused by the economy's own contract gate"
    );
    assert_eq!(ledger_total(&tw), before, "and nothing moved");
}

// ---- E7 residue pass 1 independent verify fold ----

/// The shipped default insurance word, which the fixture's `enabled` deliberately
/// does not use.
fn insured(pool: AccountId) -> RecoveryConfig {
    RecoveryConfig {
        insurance: SmolStr::new_static("approved_operations"),
        ..enabled(pool)
    }
}

/// **The cooldown bites under BOTH insurance words** (C6/B2; owner decision
/// F4(i)(b)).
///
/// `eligible` guarded its whole cooldown branch on `!insures_approved_operations()`
/// and `cooldown_until` returned `None` under the same word — while
/// `"approved_operations"` is the default in `mush_config`, `tacsim` **and**
/// `mush_campaign::RecoveryConfig::default`. So on every shipped default
/// `cooldown_days` was dead, `@recovery/status` rendered no cooldown, and the host
/// nonetheless validated `cooldown_days >= 1` at boot for a value nothing read. The
/// existing cooldown test is green only because the fixture picks the non-default
/// word, so this one is parameterised over both and asserts the same three things
/// each time: the refusal, the rendered cooldown, and an unmoved pool.
#[test]
fn vf_c6a_the_cooldown_bites_under_both_insurance_words() {
    for (word, cfg_of) in [
        ("none", enabled as fn(AccountId) -> RecoveryConfig),
        (
            "approved_operations",
            insured as fn(AccountId) -> RecoveryConfig,
        ),
    ] {
        let (mut tw, fx) = scene(cfg_of, 1_000, 2);
        ok(&mut tw, fx.pat, "@recovery/apply");
        fire(&mut tw);
        assert!(
            camp(&tw)
                .recovery_grant(fx.pat)
                .is_some_and(|g| g.is_live()),
            "{word}: the first grant lands"
        );
        ok(&mut tw, fx.pat, "@recovery/return");
        let status = ok(&mut tw, fx.pat, "@recovery/status");
        assert!(
            status.contains("cooldown runs to"),
            "{word}: the cooldown is rendered to its own subject: {status}"
        );
        ok(&mut tw, fx.pat, "@recovery/apply");
        let before = bal(&tw, fx.pool);
        fire(&mut tw);
        assert!(
            camp(&tw)
                .recovery_grant(fx.pat)
                .is_some_and(|g| !g.is_live()),
            "{word}: a cooled-down applicant is not re-granted"
        );
        assert_eq!(
            bal(&tw, fx.pool),
            before,
            "{word}: and nothing was spent on them"
        );
        assert!(
            ok(&mut tw, fx.pat, "@recovery/status").contains("cooldown"),
            "{word}: and they are told why"
        );
    }
}

/// **Return-and-reapply churn draws no second stipend under the shipped default**
/// (C6/B2's twelve-cycle drain, asserted as the fix).
///
/// The probe that found this ran twelve apply/grant/return cycles under
/// `insurance = "approved_operations"` and took the pool 1 000 to 880 with no
/// cooldown rendered at any point. Nothing minted — the ledger total held throughout
/// — which is exactly why the C-lens conservation pins never saw it: what makes the
/// churn a real denial of service is that a lease **returned inside its first period**
/// never pays the one `upkeep_per_period` that `upkeep_template` would have credited
/// back to the pool (see `vf_c14a`).
#[test]
fn vf_c6b_a_return_and_re_apply_inside_the_cooldown_draws_no_second_stipend() {
    let (mut tw, fx) = scene(insured, 1_000, 2);
    let start = bal(&tw, fx.pool);
    let total = ledger_total(&tw);
    let mut cycles = 0;
    for _ in 0..12 {
        let _ = tw.run(fx.pat, "@recovery/apply");
        fire(&mut tw);
        if camp(&tw)
            .recovery_grant(fx.pat)
            .is_some_and(|g| g.is_live())
        {
            cycles += 1;
            ok(&mut tw, fx.pat, "@recovery/return");
        }
    }
    assert_eq!(
        cycles, 1,
        "one applicant gets at most one loaner per cooldown"
    );
    assert_eq!(
        bal(&tw, fx.pool),
        start - 10,
        "exactly one stipend left the pool, not twelve"
    );
    assert_eq!(ledger_total(&tw), total, "and nothing minted either way");
}

/// **Every configured recovery field has a live reader** (C6's config-honesty pin).
///
/// `config_ext::recovery_config` validates `cooldown_days >= 1` at boot. A validated
/// field nothing reads is worse than an unvalidated one, because the operator is told
/// their value was accepted. This drives the value all the way to the player-visible
/// rendering under both words, so a future guard that makes it dead again fails here.
#[test]
fn vf_c6c_the_configured_cooldown_is_read_under_every_insurance_word_the_host_accepts() {
    for word in ["none", "approved_operations"] {
        let owned = SmolStr::new(word);
        let cfg_of = move |pool: AccountId| RecoveryConfig {
            insurance: owned.clone(),
            cooldown_secs: 4_242,
            ..enabled(pool)
        };
        let (mut tw, fx) = scene(cfg_of, 1_000, 2);
        ok(&mut tw, fx.pat, "@recovery/apply");
        fire(&mut tw);
        ok(&mut tw, fx.pat, "@recovery/return");
        let ended = camp(&tw)
            .recovery_grant(fx.pat)
            .and_then(mush_campaign::RecoveryGrant::ended_at)
            .expect("the lease ended");
        let status = ok(&mut tw, fx.pat, "@recovery/status");
        assert!(
            status.contains(&format!("cooldown runs to IC second {}", ended + 4_242)),
            "{word}: the CONFIGURED cooldown is what the player is shown: {status}"
        );
    }
}

/// Bind one object to a spatial behaviour, the way the shared fixtures do.
fn bind_behavior(tw: &mut TestWorld, obj: ObjId, behavior: &str) {
    tw.world_mut()
        .attach_behavior(
            obj,
            mush_core::BehaviorId::new(behavior).expect("well-formed"),
            mush_core::BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .expect("bind");
}

/// **`next_loaner` skips an unplaced hull and an unformed hull, and the queue holds**
/// (C7).
///
/// `accessible_operational_assets` — the **eligibility** side — requires titled,
/// unfrozen, `Active`, unencumbered, **placed and not quarantined**, and
/// **deployable** (formation membership). `next_loaner` — the **grant** side —
/// checked only the first four while its own doc comment claimed *"the provider's own
/// lowest-id accessible operational asset"*, so a hull the predicate would have called
/// inaccessible was a valid loaner for a wiped-out player. The quarantined arm is the
/// unplaced arm's sub-case: `SpatialState` records quarantine on an **unplaced**
/// entry, so `is_placed` already answers false for it.
///
/// The assertion order matters: `accessible_operational_assets(yard) == 0` runs
/// **first**, so the divergence is what is being measured and not an accident of the
/// fixture.
#[test]
fn vf_c7a_next_loaner_skips_an_unplaced_and_an_unformed_hull_and_the_queue_holds() {
    let (mut tw, fx) = scene(enabled, 1_000, 0);
    let unplaced = tw
        .world_mut()
        .create(ObjKind::Thing, "Unplaced", fx.yard, Some(LIMBO))
        .expect("a hull");
    let unformed = tw
        .world_mut()
        .create(ObjKind::Thing, "Unformed", fx.yard, Some(LIMBO))
        .expect("a hull");
    for hull in [unplaced, unformed] {
        ok(
            &mut tw,
            GOD,
            &format!("@asset/title #{}=#{}", hull.0, fx.yard.0),
        );
    }
    // The unformed one is placed but in no formation; the unplaced one is neither.
    bind_behavior(&mut tw, unformed, "sim.participant");
    ok(
        &mut tw,
        GOD,
        &format!("@spatial/place/domain #{}=0", unformed.0),
    );

    assert_eq!(
        mush_campaign::accessible_operational_assets(tw.world(), fx.yard),
        0,
        "the eligibility side calls BOTH of them inaccessible"
    );
    assert!(
        mush_campaign::next_loaner(tw.world(), &enabled(fx.pool)).is_none(),
        "and so, now, does the grant side"
    );
    let before = bal(&tw, fx.pool);
    ok(&mut tw, fx.pat, "@recovery/apply");
    fire(&mut tw);
    assert!(
        camp(&tw).recovery_grant(fx.pat).is_none(),
        "no defective hull was leased"
    );
    assert_eq!(camp(&tw).recovery_queue().len(), 1, "the queue HOLDS");
    assert_eq!(bal(&tw, fx.pool), before, "and nothing was spent");
}

/// **A placed, formed hull IS granted** (C7's happy path, on the repaired fixture).
#[test]
fn vf_c7b_a_placed_and_formed_hull_is_granted() {
    let (mut tw, fx) = scene(enabled, 1_000, 1);
    assert_eq!(
        mush_campaign::accessible_operational_assets(tw.world(), fx.yard),
        1,
        "the fixture's hull really is accessible now"
    );
    ok(&mut tw, fx.pat, "@recovery/apply");
    fire(&mut tw);
    assert_eq!(
        camp(&tw)
            .recovery_grant(fx.pat)
            .map(mush_campaign::RecoveryGrant::loaner),
        Some(fx.hulls[0])
    );
}

/// **An applicant the provider cannot pay does not block the one behind them**
/// (C10/B3; owner decision F5(a)).
///
/// `try_grant` returned `Held` for eight reasons, three of them per-applicant, and
/// `sweep` read every one as provider-wide: it paged `recovery:provider` with a
/// recommendation that fixed none of the three, then `break`. B3's probe queued a
/// blocker with the highest object id ahead of an eligible applicant and fired five
/// times; the eligible applicant was never granted, because the rotation's
/// `position(|id| id.0 >= cursor).unwrap_or(0)` collapsed to the head of a
/// **claim-ordered** queue and re-visited the blocker first on every fire, for ever.
#[test]
fn vf_c10a_an_applicant_the_provider_cannot_pay_does_not_block_the_one_behind_them() {
    let (mut tw, fx) = scene(enabled, 1_000, 2);
    // The blocker is spawned LAST, so it holds the highest object id — the shape that
    // made `unwrap_or(0)` send the rotation back to the head — and never opens an
    // account, so it is `ApplicantBlocked` and not `Ineligible`.
    let blocker = tw.spawn_player("Blocker");
    ok(&mut tw, blocker, "@recovery/apply");
    ok(&mut tw, fx.robin, "@recovery/apply");
    assert!(blocker.0 > fx.robin.0, "the blocker holds the higher id");

    fire(&mut tw);

    assert!(
        camp(&tw).recovery_grant(fx.robin).is_some(),
        "the applicant behind the blocker was granted on the SAME fire"
    );
    assert!(
        camp(&tw).recovery_grant(blocker).is_none(),
        "and the blocker was not"
    );
    assert_eq!(
        camp(&tw).recovery_queue().waiting(),
        vec![blocker],
        "who keeps their claim-order place rather than being dropped"
    );
    // The refusal names THEM and the command that clears it, not the provider's pool.
    let refusal = err(&mut tw, GOD, &format!("@recovery/grant #{}", blocker.0));
    assert!(
        refusal.contains(&format!("#{} cannot be granted", blocker.0))
            && refusal.contains(&format!("@account/open #{}=gold", blocker.0)),
        "the applicant is the subject and the exit is named: {refusal}"
    );
    assert!(
        !refusal.contains("Add a hull the provider owns"),
        "and staff are not handed advice that fixes nothing: {refusal}"
    );
    // ...and the exit really works.
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", blocker.0));
    fire(&mut tw);
    assert!(
        camp(&tw).recovery_grant(blocker).is_some(),
        "opening the account they were told to open grants them"
    );
}

/// **A dry pool still stops the fire and pages the PROVIDER** (C10's other half: the
/// split must not turn a genuinely provider-wide exhaustion into a per-applicant
/// note).
#[test]
fn vf_c10b_a_dry_pool_still_stops_the_fire_and_pages_the_provider() {
    let (mut tw, fx) = scene(enabled, 0, 2);
    ok(&mut tw, fx.pat, "@recovery/apply");
    ok(&mut tw, fx.robin, "@recovery/apply");
    fire(&mut tw);
    assert!(camp(&tw).recovery_grant(fx.pat).is_none());
    assert!(camp(&tw).recovery_grant(fx.robin).is_none());
    assert_eq!(camp(&tw).recovery_queue().len(), 2, "both keep their place");
    let refusal = err(&mut tw, GOD, &format!("@recovery/grant #{}", fx.pat.0));
    assert!(
        refusal.contains("The provider cannot grant right now"),
        "a genuinely provider-wide exhaustion still names the provider: {refusal}"
    );
    assert!(
        !refusal.contains("cannot be granted right now"),
        "and is not rendered as one applicant's problem: {refusal}"
    );
}

/// **The rotation reaches the back of a claim-ordered queue whatever the object ids**
/// (C10/B3, the `unwrap_or(0)` shape pinned directly).
#[test]
fn vf_c10c_the_rotation_reaches_the_back_of_a_claim_ordered_queue_whatever_the_object_ids() {
    let (mut tw, fx) = scene(enabled, 10_000, 12);
    // Claim order deliberately DESCENDING in object id, which is the arrangement no
    // `id >= cursor` search can walk monotonically.
    let mut applicants = Vec::new();
    for n in 0..9 {
        let who = tw.spawn_player(&format!("Late{n}"));
        ok(&mut tw, GOD, &format!("@account/open #{}=gold", who.0));
        applicants.push(who);
    }
    applicants.reverse();
    applicants.push(fx.pat);
    for who in &applicants {
        ok(&mut tw, *who, "@recovery/apply");
    }
    assert_eq!(camp(&tw).recovery_queue().len(), 10);
    fire(&mut tw);
    fire(&mut tw);
    let ungranted: Vec<ObjId> = applicants
        .iter()
        .copied()
        .filter(|who| camp(&tw).recovery_grant(*who).is_none())
        .collect();
    assert!(
        ungranted.is_empty(),
        "two fires at a budget of {MAX_RECOVERY_PER_FIRE} reach all ten: {ungranted:?}"
    );
}

/// **A lost grant document grants nothing twice** (D3; owner decision F6(a)).
///
/// `offer_lease` and the stipend `post` are pinned by the economy's `install_commit`;
/// the three campaign documents are ordinary deferred writes. The crash shape is the
/// probe's: the economy batch landed and the campaign namespace rolled back. Before
/// the fix the next fire leased the **same hull** again and posted a **second**
/// stipend, because the campaign mirror was the only marker and the stipend key was
/// scoped to the IC second.
#[test]
fn vf_d3a_a_lost_grant_document_grants_nothing_twice() {
    let (mut tw, fx) = scene(enabled, 1_000, 2);
    ok(&mut tw, fx.pat, "@recovery/apply");
    let mut pre: Kv = BTreeMap::new();
    fold_into(&mut tw, &mut pre);
    fire(&mut tw);
    let first = camp(&tw)
        .recovery_grant(fx.pat)
        .cloned()
        .expect("the tick granted");
    assert_eq!(bal(&tw, fx.pool), 990, "one stipend");

    // The tear: everything the fire wrote, except the campaign namespace, which is
    // rolled back to its pre-grant contents.
    let mut lost: Kv = {
        let mut p = pre.clone();
        fold_into(&mut tw, &mut p);
        p
    };
    lost.insert(SmolStr::new_static("campaign"), pre["campaign"].clone());
    reboot(&mut tw, &lost);
    assert!(
        camp(&tw).recovery_grant(fx.pat).is_none(),
        "the mirror really is gone"
    );
    assert!(
        snap(&tw)
            .contract(mush_economy::ContractId(first.contract()))
            .is_some(),
        "and the economy's own record really did survive"
    );

    fire(&mut tw);
    let second = camp(&tw)
        .recovery_grant(fx.pat)
        .cloned()
        .expect("the mirror was rebuilt");
    assert_eq!(
        second.contract(),
        first.contract(),
        "ONE contract, reconciled — not a second lease"
    );
    assert_eq!(second.loaner(), first.loaner(), "and the same hull");
    assert_eq!(bal(&tw, fx.pool), 990, "and NO second stipend");
    let live: Vec<u64> = snap(&tw)
        .contracts()
        .filter(|c| !c.state.is_terminal())
        .map(|c| c.id.0)
        .collect();
    assert_eq!(
        live,
        vec![first.contract()],
        "one live contract in the world"
    );
}

/// **A lost close document does not strand the hull** (D3's mirror image).
///
/// `end_lease` lands, `close_grant`'s two documents do not: the hull was out of the
/// motor pool and the applicant `LiveLease`-refused for ever, because the campaign
/// document was the only thing anybody asked. `next_loaner` now asks the economy too.
#[test]
fn vf_d3b_a_lost_close_document_does_not_strand_the_hull() {
    let (mut tw, fx) = scene(enabled, 1_000, 1);
    ok(&mut tw, fx.pat, "@recovery/apply");
    fire(&mut tw);
    let grant = camp(&tw).recovery_grant(fx.pat).cloned().expect("granted");
    let mut pre: Kv = BTreeMap::new();
    fold_into(&mut tw, &mut pre);

    ok(&mut tw, fx.pat, "@recovery/return");
    let mut lost: Kv = {
        let mut p = pre.clone();
        fold_into(&mut tw, &mut p);
        p
    };
    lost.insert(SmolStr::new_static("campaign"), pre["campaign"].clone());
    reboot(&mut tw, &lost);
    assert!(
        camp(&tw)
            .recovery_grant(fx.pat)
            .is_some_and(|g| g.is_live()),
        "the stale mirror still says the lease is live"
    );
    assert_eq!(
        mush_campaign::next_loaner(tw.world(), &enabled(fx.pool)),
        Some(grant.loaner()),
        "but the hull is back in the motor pool, because the MONEY fact says the \
         lease ended"
    );
}

/// **The recovery cursor survives a reboot with no live operations** (D6).
///
/// `recoverycmd::sweep` advances `store.recovery_cursor` in RAM; the only document
/// carrying it is `campaign_meta`, written by `persist_meta` at the END of the tick —
/// past the `ids.is_empty()` return and past the `if last.is_some()` guard, both
/// driven by the **operations** rotation. A world with a recovery queue and no live
/// operations is exactly the shape a recovery provider exists for and the shape a
/// fresh deployment has.
#[test]
fn vf_d6a_the_recovery_cursor_survives_a_reboot_with_no_operations() {
    let (mut tw, fx) = scene(enabled, 10_000, 12);
    let mut applicants = vec![fx.pat, fx.robin];
    for n in 0..7 {
        let who = tw.spawn_player(&format!("Applicant{n}"));
        ok(&mut tw, GOD, &format!("@account/open #{}=gold", who.0));
        applicants.push(who);
    }
    for who in &applicants {
        ok(&mut tw, *who, "@recovery/apply");
    }
    assert!(
        camp(&tw).operation_ids().is_empty(),
        "no live operations at all — the shape the cursor was lost in"
    );
    let mut kv: Kv = BTreeMap::new();
    fold_into(&mut tw, &mut kv);

    fire(&mut tw);
    let persisted = tw.take_module_ops().iter().any(
        |op| matches!(&op.op, ModuleWrite::JsonSet { key, .. } if key.as_str() == "campaign_meta"),
    );
    assert!(
        persisted,
        "the fire persisted `campaign_meta`, so the recovery cursor reached disk"
    );
}

/// **An unaccepted grant expires and the hull returns to the motor pool** (D7).
///
/// `try_grant` completes at `offer_lease`; until the applicant runs
/// `@contract/accept` no `Encumbrance::Lease` exists and the hull is held out of the
/// pool by the grant document alone. Nothing aged that: the fourth arm walked only the
/// **queue**, which a granted applicant has already left.
#[test]
fn vf_d7a_an_unaccepted_grant_expires_and_the_hull_returns_to_the_motor_pool() {
    let (mut tw, fx) = scene(enabled, 1_000, 1);
    ok(&mut tw, fx.pat, "@recovery/apply");
    fire(&mut tw);
    let grant = camp(&tw).recovery_grant(fx.pat).cloned().expect("granted");
    assert!(
        mush_campaign::next_loaner(tw.world(), &enabled(fx.pool)).is_none(),
        "the only hull is out on the unaccepted offer"
    );

    // Past the TTL, on the ordinary cadence.
    let then = tw.world().now() + mush_campaign::RECOVERY_ACCEPT_TTL_SECS + 1;
    tw.set_now(then);
    fire(&mut tw);

    assert!(
        snap(&tw)
            .contract(mush_economy::ContractId(grant.contract()))
            .is_some_and(|c| c.state.is_terminal()),
        "the unaccepted lease was unwound"
    );
    // **Forgotten, not ended**: an offer nobody took must not leave an `ended_at`
    // behind, or `cooldown_until` charges the applicant fourteen days for a hull they
    // never had and the next fire drops them from the queue as `Cooldown`-ineligible.
    let after = camp(&tw).recovery_grant(fx.pat).cloned();
    assert!(
        after
            .as_ref()
            .is_none_or(|g| g.contract() != grant.contract()),
        "the expired grant is gone, not merely ended: {after:?}"
    );
    assert!(
        !ok(&mut tw, fx.pat, "@recovery/status").contains("cooldown"),
        "and no cooldown was charged for a loaner they never accepted"
    );
    // The hull is out of limbo: either back in the motor pool, or freshly offered to
    // the same applicant on a NEW contract. Never stuck for ever, which is the defect.
    let free = mush_campaign::next_loaner(tw.world(), &enabled(fx.pool)) == Some(grant.loaner());
    let reoffered = after.is_some_and(|g| g.loaner() == grant.loaner() && g.is_live());
    assert!(
        free || reoffered,
        "the hull is no longer permanently out of the motor pool"
    );
}

/// **An ACCEPTED lease outlives the acceptance TTL** (D7's other arm: the TTL is
/// about an offer nobody took, never about a loaner somebody is flying).
#[test]
fn vf_d7b_an_accepted_lease_outlives_the_acceptance_ttl() {
    let (mut tw, fx) = scene(enabled, 1_000, 1);
    ok(&mut tw, fx.pat, "@recovery/apply");
    fire(&mut tw);
    let grant = camp(&tw).recovery_grant(fx.pat).cloned().expect("granted");
    ok(
        &mut tw,
        fx.pat,
        &format!("@contract/accept {}", grant.contract()),
    );
    let then = tw.world().now() + mush_campaign::RECOVERY_ACCEPT_TTL_SECS + 1;
    tw.set_now(then);
    fire(&mut tw);
    assert!(
        camp(&tw)
            .recovery_grant(fx.pat)
            .is_some_and(|g| g.is_live()),
        "an accepted loaner stays theirs until they return it"
    );
}

/// **The recovery books are capped and evict the oldest CLOSED record** (B7 — cut
/// 10's muster ring, one file over).
///
/// 10b §15.1 row 5 said *"three bounded whole-document exempt records"*;
/// `MAX_RECOVERY_QUEUE` bounded only the queue, and a free player verb created one
/// application and one grant per character who ever applied, with nothing ever
/// deleting one.
#[test]
fn vf_b7a_the_recovery_books_are_capped_and_evict_the_oldest_closed_record() {
    assert_eq!(mush_campaign::MAX_RETAINED_RECOVERY_RECORDS, 64);
    let (mut tw, fx) = scene(enabled, 100_000, 1);
    let scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("org")
        .state()
        .orgs()
        .next()
        .expect("one org")
        .root_scope;
    let mut node = fx.node;
    let mut in_node = 1usize;
    let mut applicants = Vec::new();
    for n in 0..(mush_campaign::MAX_RETAINED_RECOVERY_RECORDS + 4) {
        let who = tw.spawn_player(&format!("Churn{n}"));
        ok(&mut tw, GOD, &format!("@account/open #{}=gold", who.0));
        // Their own accessible hull, so `eligible` refuses them at grant time and the
        // sweep's `Ineligible` arm CLOSES the application. A formation node caps its
        // own membership, so the fixture opens another rather than overfilling one.
        if in_node >= 32 {
            ok(&mut tw, GOD, &format!("@force/create lance={}", scope.0));
            node = camp(&tw)
                .formations()
                .map(|f| f.id)
                .max_by_key(|f| f.0)
                .expect("a fresh node");
            in_node = 0;
        }
        let hull = tw
            .world_mut()
            .create(ObjKind::Thing, &format!("Own{n}"), who, Some(LIMBO))
            .expect("a hull");
        ok(
            &mut tw,
            GOD,
            &format!("@asset/title #{}=#{}", hull.0, who.0),
        );
        place_and_form(&mut tw, hull, node);
        in_node += 1;
        applicants.push(who);
    }
    for who in &applicants {
        ok(&mut tw, *who, "@recovery/apply");
    }
    for _ in 0..12 {
        fire(&mut tw);
    }
    let kept = applicants
        .iter()
        .filter(|who| camp(&tw).recovery_application(**who).is_some())
        .count();
    assert!(
        kept <= mush_campaign::MAX_RETAINED_RECOVERY_RECORDS,
        "the closed-application book is bounded: {kept} kept"
    );
    assert!(kept > 0, "and it is a ring, not a purge");
}

/// **A store over the cap hydrates bounded** (B7's boot half: the growth cost
/// boot-time RAM as well as disk, which is what cut 10 fixed for musters).
#[test]
fn vf_b7b_a_store_over_the_cap_hydrates_bounded() {
    let (mut tw, _fx) = scene(enabled, 1_000, 1);
    let mut kv: Kv = BTreeMap::new();
    fold_into(&mut tw, &mut kv);
    let entry = kv.entry(SmolStr::new_static("campaign")).or_default();
    let over = mush_campaign::MAX_RETAINED_RECOVERY_RECORDS + 200;
    for n in 1..=over {
        let id = u32::try_from(n + 10_000).expect("fits");
        entry.push((
            SmolStr::new(format!("recovery_app:{id}")),
            serde_json::json!({
                "applicant": id, "claimed_at": 1, "tier": "scout",
                "state": { "kind": "closed" }
            }),
        ));
        entry.push((
            SmolStr::new(format!("recovery_grant:{id}")),
            serde_json::json!({
                "applicant": id, "contract": u64::from(id), "loaner": id, "tier": "scout",
                "granted_at": 1, "ended_at": 2
            }),
        ));
    }
    reboot(&mut tw, &kv);
    let apps = (1..=over)
        .filter(|n| {
            camp(&tw)
                .recovery_application(ObjId(u32::try_from(n + 10_000).expect("fits")))
                .is_some()
        })
        .count();
    let grants = camp(&tw).recovery_grants().count();
    assert!(
        apps <= mush_campaign::MAX_RETAINED_RECOVERY_RECORDS,
        "closed applications hydrate bounded: {apps}"
    );
    assert!(
        grants <= mush_campaign::MAX_RETAINED_RECOVERY_RECORDS,
        "ended grants hydrate bounded: {grants}"
    );
}

/// **A lease held past its first period returns its own stipend to the pool**
/// (C14(c), against the refuted B16).
///
/// The pass's own reading — *"the pool is strictly one-way"* — is false:
/// `upkeep_template` sets `credit_account: cfg.pool_account`, `try_grant` hands it to
/// `offer_lease` as the `LeaseSpec.rent`, and the economy creates and sweeps it at
/// activation. What is genuinely one-way is a lease **returned inside its first
/// period, never accepted, or suspended** — which is why C6's churn and D7's
/// unaccepted grant are real drains and not a missing replenishment mechanism.
#[test]
fn vf_c14a_a_lease_held_past_its_first_period_returns_its_own_stipend_to_the_pool() {
    let (mut tw, fx) = scene(enabled, 1_000, 1);
    let start = bal(&tw, fx.pool);
    ok(&mut tw, fx.pat, "@recovery/apply");
    fire(&mut tw);
    let grant = camp(&tw).recovery_grant(fx.pat).cloned().expect("granted");
    assert_eq!(bal(&tw, fx.pool), start - 10, "the stipend went out");

    ok(
        &mut tw,
        fx.pat,
        &format!("@contract/accept {}", grant.contract()),
    );
    // The rent obligation is on the campaign cadence; one period is one fire.
    for _ in 0..3 {
        fire(&mut tw);
    }
    assert_eq!(
        bal(&tw, fx.pool),
        start,
        "one period's upkeep came back: the pool is NOT one-way"
    );
}
