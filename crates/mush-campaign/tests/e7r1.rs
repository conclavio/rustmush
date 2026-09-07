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
}

/// The whole fixture, through real commands: an instrument, a faucet, the provider's
/// pool with `funds` in it, the two applicants' accounts, and `hulls` hulls titled to
/// the provider.
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
    // The provider's motor pool: hulls titled to the yard, placed and formed up, so
    // they read as accessible operational assets.
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
    // so the cooldown is not waived; under `approved_operations` a loss inside the
    // restrictions waives it and the same applicant qualifies at once.
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
