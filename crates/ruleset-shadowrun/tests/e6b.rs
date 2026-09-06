//! E6b acceptance (08f §4.5): **outcomes, fences, custody.**
//!
//! The verify axis of this cut is *forged seals, prose settlement, fence laundering,
//! consignment custody races*, so the rhythms here are:
//!
//! - **escrow strictly before custody.** Every fence deal asserts the fence's
//!   `held_amount` before the goods move, and `+fence/deliver` is refused until the
//!   contract is in force (D-E6-9).
//! - **a fence buys custody, never title.** Every settlement asserts the seller's
//!   title record and any `Disputed` claim are byte-unchanged afterwards (D-E6-10).
//! - **a hidden fact contributes nothing.** Two worlds differing only in hidden state
//!   quote identically (D-E6-11).
//! - **payment never edits the sheet.** `sheet:shadowrun` is asserted unchanged
//!   across every test in the file (D-E6-16).
//!
//! Everything runs through the real `Engine` on the production module composition,
//! extended with E6b's wiring: `scene.outcome` as the one producer-declared outcome
//! kind, and `[shadowrun.economy].vendor_orgs` frozen into `with_party_caps`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use mush_core::{Audience, GOD, ModuleBootData, ModuleWrite, ObjId, ObjKind};
use mush_economy::{
    AccountId, ContractId, ContractState, DeclaredCadence, DeclaredCadenceClock, EconomyConfig,
    EconomySnapshot, EconomyStore, PartyCap, VaultId,
};
use mush_org::{CapabilityId, OrgId, PrincipalId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use ruleset_shadowrun::{SrBoardObserver, SrEconomyConfig, SrEconomyStore};
use smol_str::SmolStr;

const NS: &str = "shadowrun";
const PERIOD: i64 = 100;
/// The catalogue code the fixture prices against, and its source-controlled baseline.
const CODE: &str = "ares.predator";
const BASELINE: u64 = 1_000;

// ---- rig ------------------------------------------------------------------

fn vocabulary() -> BTreeSet<CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| CapabilityId::new(c).expect("well-formed"))
        .collect()
}

fn sr_config() -> SrEconomyConfig {
    SrEconomyConfig {
        enabled: true,
        fence_baseline: BTreeMap::from([(SmolStr::new(CODE), BASELINE)]),
        // Composition data (DL-11): the org ids whose acceptance may be performed
        // programmatically. The fixture's orgs land in this range.
        vendor_orgs: vec![0, 1, 2, 3],
        ..SrEconomyConfig::default()
    }
}

/// `modules.rs`'s own `vendor_caps`, so the rig freezes the same table production
/// does rather than a looser one.
fn vendor_caps(cfg: &SrEconomyConfig) -> Vec<(PrincipalId, PartyCap)> {
    cfg.vendor_orgs
        .iter()
        .map(|id| (PrincipalId::Organization(OrgId(*id)), PartyCap::Accept))
        .collect()
}

fn boot_with(cfg: SrEconomyConfig) -> TestWorld {
    let econ = EconomyConfig {
        cadences: BTreeMap::from([(
            SmolStr::new("sr.month"),
            DeclaredCadence {
                clock: DeclaredCadenceClock::WorldIc,
                period_secs: PERIOD,
                catch_up: ruleset_shadowrun::SR_MONTH_CATCH_UP,
            },
        )]),
        ..EconomyConfig::default()
    };
    TestWorld::with_modules(vec![
        Box::new(mush_canon::CanonModule::new()),
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_faction::FactionModule::new()),
        Box::new(mush_scenes::ScenesModule::new()),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_config(econ)
                .with_observers(vec![Arc::new(SrBoardObserver::new(cfg.clone()))])
                .with_canon_outcome_kinds(vec![SmolStr::new_static(
                    mush_scenes::KIND_SCENE_OUTCOME,
                )])
                .with_party_caps(vendor_caps(&cfg)),
        ),
        Box::new(mush_party::PartyModule::new()),
        Box::new(mush_jobs::JobsModule::new()),
        Box::new(ruleset_shadowrun::ShadowrunRuleset::with_seed(7).economy(cfg)),
    ])
}

fn boot() -> TestWorld {
    boot_with(sr_config())
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
fn sr(tw: &TestWorld) -> &SrEconomyStore {
    tw.world()
        .module_state::<SrEconomyStore>()
        .expect("sr store")
}
fn bal(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw)
        .account(id)
        .expect("account")
        .posted_balance
        .minor()
}
fn held(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw).account(id).expect("account").held_amount.minor()
}
fn state(tw: &TestWorld, id: ContractId) -> ContractState {
    snap(tw).contract(id).expect("contract").state
}
fn account_of(tw: &TestWorld, who: ObjId) -> AccountId {
    sr(tw)
        .profile(who)
        .and_then(|p| p.account)
        .expect("a profiled account")
}
fn newest_contract(tw: &TestWorld) -> ContractId {
    snap(tw)
        .contracts()
        .map(|c| c.id)
        .max()
        .expect("a contract")
}

/// An **approved** character with a nuyen account and `funds` in it — the e6a rig's
/// own adoption path, so the fixture exercises what `@sr/migrate commit` promises.
fn runner(tw: &mut TestWorld, name: &str, funds: i64) -> ObjId {
    let who = tw.spawn_player(name);
    let store = ruleset_shadowrun::ShadowrunRuleset::new().store();
    let mut doc = store.get_or_default(tw.world(), who);
    doc.set_number("nuyen", funds);
    doc.approved = mush_ruleset::ApprovalState::Approved { by: GOD, at: 10 };
    store
        .put_by(tw.world_mut(), who, &doc, GOD)
        .expect("the sheet writes");
    ok(tw, GOD, &format!("@sr/migrate commit #{}", who.0));
    who
}

/// The whole `sheet:shadowrun` document for `who`, as JSON — the D-E6-16 witness.
fn sheet(tw: &TestWorld, who: ObjId) -> serde_json::Value {
    let store = ruleset_shadowrun::ShadowrunRuleset::new().store();
    serde_json::to_value(store.get_or_default(tw.world(), who)).expect("the sheet serializes")
}

type Kv = BTreeMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>;

fn fold_all(tw: &mut TestWorld) -> Kv {
    let mut kv: Kv = BTreeMap::new();
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
    kv
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

/// A registered fence: an organization with an unscoped nuyen account, a vault it
/// owns, and `funds` in the wallet.
struct Fence {
    org: OrgId,
    account: AccountId,
    vault: VaultId,
    id: u64,
}

fn fence(tw: &mut TestWorld, name: &str, funds: i64, demand: i64) -> Fence {
    ok(tw, GOD, &format!("@org/create {name}"));
    let org = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().orgs().map(|o| o.id).max())
        .expect("the org exists");
    ok(tw, GOD, &format!("@account/open org:#{}=sr.nuyen", org.0));
    let account = snap(tw)
        .accounts()
        .find(|a| a.owner == PrincipalId::Organization(org))
        .expect("the fence wallet")
        .id;
    let f = snap(tw)
        .accounts()
        .find(|a| {
            a.owner
                == PrincipalId::System(
                    mush_org::SystemPrincipalId::new("sr.chargen").expect("well-formed"),
                )
        })
        .expect("faucet")
        .id;
    if funds > 0 {
        ok(tw, GOD, &format!("@econ/mint {f}={account} {funds}"));
    }
    let backing = tw
        .world_mut()
        .create(ObjKind::Thing, &format!("{name} strongbox"), GOD, None)
        .expect("a container");
    ok(
        tw,
        GOD,
        &format!(
            "@vault/create {name}=container:#{} owner=org:#{}",
            backing.0, org.0
        ),
    );
    let vault = snap(tw)
        .vaults()
        .find(|v| v.owner == PrincipalId::Organization(org))
        .expect("the fence vault")
        .id;
    ok(
        tw,
        GOD,
        &format!(
            "@sr/fence #{}=#{} name={name} demand={demand}",
            org.0, vault.0
        ),
    );
    let id = sr(tw).fence_of_org(org).expect("the fence registered").id.0;
    Fence {
        org,
        account,
        vault,
        id,
    }
}

/// A titled object in `who`'s hands, catalogued so a fence can price it.
fn goods(tw: &mut TestWorld, who: ObjId, name: &str, catalogued: bool) -> ObjId {
    let obj = tw
        .world_mut()
        .create(ObjKind::Thing, name, GOD, Some(who))
        .expect("the object");
    let catalog = if catalogued {
        format!(" catalog=sr:{CODE}")
    } else {
        String::new()
    };
    ok(
        tw,
        GOD,
        &format!("@asset/title #{}=#{}{catalog}", obj.0, who.0),
    );
    obj
}

// ===========================================================================
// B9 — the fence: quote, escrow, deliver, settle
// ===========================================================================

/// The immediate sale, end to end: the quote is deterministic, the fence's escrow is
/// placed **before** the goods move (answer 15), delivery settles it at once, and the
/// seller's **title survives** (D-E6-10).
#[test]
fn e6b_b9_an_immediate_fence_sale_escrows_first_delivers_second_and_leaves_title_alone() {
    let mut tw = boot();
    let seller = runner(&mut tw, "Jax", 0);
    let f = fence(&mut tw, "Ares", 10_000, 0);
    let obj = goods(&mut tw, seller, "predator", true);
    let sheet_before = sheet(&tw, seller);

    ok(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{}", f.id, obj.0),
    );
    let q = sr(&tw).quotes().next().expect("a quote").clone();
    // 300 permille of a 1000 baseline, and the basis is the whole arithmetic.
    assert_eq!(q.offer.minor(), 300);
    assert_eq!(q.basis.len(), 3);

    // The fence escrows at `+fence/accept`; nothing has moved yet.
    ok(&mut tw, seller, &format!("+fence/accept {}", q.id.0));
    let contract = newest_contract(&tw);
    assert_eq!(state(&tw, contract), ContractState::Offered);
    assert_eq!(held(&tw, f.account), 300, "the fence did not escrow");
    assert_eq!(
        tw.world().get(obj).and_then(|o| o.location),
        Some(seller),
        "the goods moved before the money was committed"
    );

    // D-E6-9: delivery is refused while the contract is `Offered`.
    let refusal = err(&mut tw, seller, &format!("+fence/deliver {}", q.id.0));
    assert!(refusal.contains("not in force"), "{refusal}");

    // The seller binds, and only then delivers.
    ok(&mut tw, seller, &format!("@contract/accept {contract}"));
    assert_eq!(state(&tw, contract), ContractState::Active);
    ok(&mut tw, seller, &format!("+fence/deliver {}", q.id.0));
    // The deposit is journal authority; the sweep is what turns a satisfied
    // deterministic condition into a settlement.
    let now = tw.world().now();
    tw.set_now(now + 60);
    tw.fire_due((now + 60) * 1_000);
    assert!(
        matches!(state(&tw, contract), ContractState::Settled { .. }),
        "the immediate sale did not settle on delivery: {:?}",
        state(&tw, contract)
    );
    assert_eq!(bal(&tw, account_of(&tw, seller)), 300);

    // **Title never moved** (D-E6-10): the fence bought custody.
    let asset = snap(&tw).asset(obj).expect("the title record");
    assert_eq!(asset.title_owner, PrincipalId::Object(seller));
    // D-E6-16: payment wrote no sheet.
    assert_eq!(sheet(&tw, seller), sheet_before);
}

/// The consignment: it pays **more**, it shows its delay in the offer, and it matures
/// only after that delay — the `After { condition: <deposit>, delay }` shape.
#[test]
fn e6b_b9_a_consignment_pays_more_and_matures_only_after_its_delay() {
    let mut tw = boot();
    let seller = runner(&mut tw, "Jax", 0);
    let f = fence(&mut tw, "Ares", 10_000, 0);
    let obj = goods(&mut tw, seller, "predator", true);
    let sheet_before = sheet(&tw, seller);

    let line = ok(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{} consign", f.id, obj.0),
    );
    assert!(
        line.contains("consignment in"),
        "the delay is shown: {line}"
    );
    let q = sr(&tw).quotes().next().expect("a quote").clone();
    assert_eq!(q.offer.minor(), 600, "a consignment pays more");

    ok(&mut tw, seller, &format!("+fence/accept {}", q.id.0));
    let contract = newest_contract(&tw);
    ok(&mut tw, seller, &format!("@contract/accept {contract}"));
    ok(&mut tw, seller, &format!("+fence/deliver {}", q.id.0));
    assert_eq!(
        state(&tw, contract),
        ContractState::Active,
        "a consignment settled before its delay"
    );
    assert_eq!(bal(&tw, account_of(&tw, seller)), 0);

    // A sweep *before* the delay still settles nothing.
    let now = tw.world().now();
    tw.set_now(now + 60);
    tw.fire_due((now + 60) * 1_000);
    assert_eq!(state(&tw, contract), ContractState::Active);
    // Past the delay — but inside the deal's own window — the sweep settles it.
    let delay = SrEconomyConfig::default().fence_delay();
    tw.set_now(now + delay + 120);
    tw.fire_due((now + delay + 120) * 1_000);
    assert!(
        matches!(state(&tw, contract), ContractState::Settled { .. }),
        "the consignment never matured: {:?}",
        state(&tw, contract)
    );
    assert_eq!(bal(&tw, account_of(&tw, seller)), 600);
    assert_eq!(sheet(&tw, seller), sheet_before);
}

/// **D-E6-11.** Two worlds differing only in **hidden** state — a covert theft report
/// and a `Disputed` claim on the very object — produce byte-identical quotes. The
/// pricing function never reads any of it.
#[test]
fn e6b_b9_two_worlds_differing_only_in_hidden_state_quote_identically() {
    let quote_in = |stolen: bool| -> (i64, Vec<(String, i64)>) {
        let mut tw = boot();
        let seller = runner(&mut tw, "Jax", 0);
        let f = fence(&mut tw, "Ares", 10_000, 0);
        let obj = goods(&mut tw, seller, "predator", true);
        if stolen {
            ok(
                &mut tw,
                GOD,
                &format!(
                    "@asset/stolen #{}=covert lifted from a Renraku courier",
                    obj.0
                ),
            );
            assert!(
                snap(&tw).asset(obj).expect("the asset").stolen.is_some(),
                "the fixture must actually differ"
            );
        }
        ok(
            &mut tw,
            seller,
            &format!("+fence/quote {}=#{}", f.id, obj.0),
        );
        let q = sr(&tw).quotes().next().expect("a quote").clone();
        (
            q.offer.minor(),
            q.basis
                .iter()
                .map(|b| (b.label.to_string(), b.permille))
                .collect(),
        )
    };
    assert_eq!(
        quote_in(false),
        quote_in(true),
        "a hidden fact moved the price, so a low quote is an oracle"
    );
}

/// **OPEN 1.** An object whose asset record carries no `catalog` **cannot be
/// quoted** — refused, never guessed from what somebody paid for it.
#[test]
fn e6b_b9_an_object_with_no_catalogue_cannot_be_quoted() {
    let mut tw = boot();
    let seller = runner(&mut tw, "Jax", 0);
    let f = fence(&mut tw, "Ares", 10_000, 0);
    let plain = goods(&mut tw, seller, "brick", false);
    let refusal = err(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{}", f.id, plain.0),
    );
    assert!(refusal.contains("no catalogue entry"), "{refusal}");
    assert_eq!(sr(&tw).quotes().count(), 0);
}

/// Scenario 29: a quote whose declared window has passed, or whose bound revisions
/// have moved, is refused **deterministically** rather than re-priced.
#[test]
fn e6b_b9_an_expired_or_restated_quote_is_refused_and_never_repriced() {
    let mut tw = boot();
    let seller = runner(&mut tw, "Jax", 0);
    let f = fence(&mut tw, "Ares", 10_000, 0);
    let obj = goods(&mut tw, seller, "predator", true);

    ok(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{}", f.id, obj.0),
    );
    let q = sr(&tw).quotes().next().expect("a quote").id.0;
    let now = tw.world().now();
    tw.set_now(now + ruleset_shadowrun::FENCE_QUOTE_TTL_SECS + 1);
    let refusal = err(&mut tw, seller, &format!("+fence/accept {q}"));
    assert!(refusal.contains("lapsed"), "{refusal}");

    // And a fence that re-declares its terms invalidates its own outstanding quotes.
    // The clock is deliberately NOT rewound: E6b's independent verify fold (M1)
    // refuses a second quote on an object that still has a live one, and D4's sweep
    // clears the lapsed one this walk passes.
    ok(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{}", f.id, obj.0),
    );
    let q2 = sr(&tw)
        .quotes()
        .map(|x| x.id.0)
        .max()
        .expect("a second quote");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@sr/fence #{}=#{} name=Ares demand=-100",
            f.org.0, f.vault.0
        ),
    );
    let refusal = err(&mut tw, seller, &format!("+fence/accept {q2}"));
    assert!(refusal.contains("lapsed"), "{refusal}");
}

/// **M15/DL-11.** The fence accepts with **no officer connected at all** — it has no
/// member and no grantee — and the wiring-frozen `with_party_caps` is the only thing
/// that lets it. It confers no money authority: a fence with an empty wallet is
/// refused on its own account's authority, which is how "a fence that runs out stops
/// quoting" is enforced (M19, D-E6-15).
#[test]
fn e6b_b4_the_fence_accepts_with_no_officer_and_a_broke_one_cannot() {
    let mut tw = boot();
    let seller = runner(&mut tw, "Jax", 0);
    let rich = fence(&mut tw, "Ares", 10_000, 0);
    let broke = fence(&mut tw, "Aztech", 0, 0);
    assert!(
        tw.world()
            .module_state::<mush_org::OrgStore>()
            .is_some_and(|s| s.state().members_in(rich.org).next().is_none()),
        "the fixture must have no officer at all"
    );
    let a = goods(&mut tw, seller, "predator", true);
    let b = goods(&mut tw, seller, "predator two", true);

    ok(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{}", rich.id, a.0),
    );
    let q = sr(&tw).quotes().map(|x| x.id.0).max().expect("a quote");
    ok(&mut tw, seller, &format!("+fence/accept {q}"));
    assert_eq!(held(&tw, rich.account), 300);

    ok(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{}", broke.id, b.0),
    );
    let q2 = sr(&tw).quotes().map(|x| x.id.0).max().expect("a quote");
    let refusal = err(&mut tw, seller, &format!("+fence/accept {q2}"));
    assert!(refusal.contains("cannot pay"), "{refusal}");
    assert_eq!(held(&tw, broke.account), 0);
}

/// **G3, scenario 12.** A hidden fence organization renders as "an organization" to a
/// non-member wherever the economy renders it, and its **listings are absent** from
/// `+fence/list` rather than refused — the economy owns the render, the ruleset owns
/// the absence.
#[test]
fn e6b_b1_a_hidden_fence_renders_as_an_organization_and_its_listings_are_absent() {
    let mut tw = boot();
    let outsider = runner(&mut tw, "Jax", 0);
    let f = fence(&mut tw, "Ares", 10_000, 0);
    let visible = ok(&mut tw, outsider, "+fence/list");
    assert!(visible.contains("Ares"), "{visible}");

    // Nothing in the shipped surface writes `Organization.hidden` (E6a fold, D11),
    // so the covert compartment is made the way an operator would: on the document,
    // read back at boot.
    let mut kv = fold_all(&mut tw);
    let orgs = kv
        .get_mut(&SmolStr::new("org"))
        .expect("the org namespace wrote");
    let key = SmolStr::new(format!("org:{}", f.org.0));
    let mut seen = false;
    for (k, v) in orgs.iter_mut() {
        if *k == key {
            v["hidden"] = serde_json::json!(true);
            seen = true;
        }
    }
    assert!(seen, "no org document to hide");
    reboot(&mut tw, &kv);
    let hidden = ok(&mut tw, outsider, "+fence/list");
    assert!(
        !hidden.contains("Ares"),
        "a hidden fence's listing is absent, not refused: {hidden}"
    );
    assert!(
        hidden.contains("No fence is buying"),
        "the absence is silence, not a refusal: {hidden}"
    );
    // And staff, who may be told it exists, still see it named.
    let staff = ok(&mut tw, GOD, "+fence/list");
    assert!(staff.contains("Ares"), "{staff}");
    // And the economy's own renderer agrees wherever it names the org: the run
    // board's notice renders a hidden payer as "an organization" too.
    let _ = outsider;
}

/// The fence's two document families survive a reboot, and a corrupt one is skipped
/// loudly while the world boots (the `store.rs` trade).
#[test]
fn e6b_b9_the_fence_records_survive_a_reboot_and_a_corrupt_quote_is_skipped() {
    let mut tw = boot();
    let seller = runner(&mut tw, "Jax", 0);
    let f = fence(&mut tw, "Ares", 10_000, 0);
    let obj = goods(&mut tw, seller, "predator", true);
    ok(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{}", f.id, obj.0),
    );
    let q = sr(&tw).quotes().next().expect("a quote").clone();

    let mut kv = fold_all(&mut tw);
    reboot(&mut tw, &kv);
    assert_eq!(sr(&tw).quotes().count(), 1);
    assert_eq!(sr(&tw).quote(q.id).expect("the quote").offer, q.offer);
    assert!(sr(&tw).fence(ruleset_shadowrun::FenceId(f.id)).is_some());

    // A quote document that fails its own wire is skipped; the fence stays.
    let entry = kv
        .get_mut(&SmolStr::new(NS))
        .expect("the ruleset namespace wrote");
    for (k, v) in entry.iter_mut() {
        if k.as_str() == format!("sr:quote:{}", q.id.0) {
            v["catalog"] = serde_json::json!("");
        }
    }
    reboot(&mut tw, &kv);
    assert_eq!(sr(&tw).quotes().count(), 0, "a corrupt quote was trusted");
    assert!(
        sr(&tw).fence(ruleset_shadowrun::FenceId(f.id)).is_some(),
        "one bad quote took the fence with it"
    );
}

// ===========================================================================
// B2 — `@asset/stolen`
// ===========================================================================

/// D-E6-18 / answer 14: the **title owner**, an organization holding `asset.insure`
/// on the asset's compartment, and **staff** may file. A bystander may not, and the
/// refusal is the existence-hiding one.
#[test]
fn e6b_b2_the_owner_an_insurer_and_staff_may_file_and_a_bystander_may_not() {
    let mut tw = boot();
    let owner = runner(&mut tw, "Jax", 0);
    let bystander = runner(&mut tw, "Nosy", 0);
    let obj = goods(&mut tw, owner, "predator", true);

    let refusal = err(
        &mut tw,
        bystander,
        &format!("@asset/stolen #{}=public taken", obj.0),
    );
    assert_eq!(refusal.trim(), "No such asset.");
    assert!(snap(&tw).asset(obj).expect("the asset").stolen.is_none());

    ok(
        &mut tw,
        owner,
        &format!("@asset/stolen #{}=public taken at gunpoint", obj.0),
    );
    let asset = snap(&tw).asset(obj).expect("the asset");
    let report = asset.stolen.clone().expect("a theft report");
    assert_eq!(report.visibility, mush_economy::StolenVisibility::Public);
    assert_eq!(report.by, PrincipalId::Object(owner));
    // The plain `Disputed` claim insurance and salvage consume is there too.
    assert!(
        asset
            .encumbrances
            .iter()
            .any(|e| matches!(e, mush_economy::Encumbrance::Disputed { .. })),
        "no Disputed claim was recorded: {:?}",
        asset.encumbrances
    );
    // Written once.
    err(
        &mut tw,
        owner,
        &format!("@asset/stolen #{}=public again", obj.0),
    );
}

/// A **covert** report is invisible to the thief even while they hold the object, and
/// visible to its reporter and to staff. That is the whole point of the visibility
/// field living beside the claim rather than inside it.
#[test]
fn e6b_b2_a_covert_report_is_invisible_to_the_thief() {
    let mut tw = boot();
    let owner = runner(&mut tw, "Jax", 0);
    let thief = runner(&mut tw, "Rat", 0);
    let obj = goods(&mut tw, owner, "predator", true);
    ok(
        &mut tw,
        owner,
        &format!("@asset/stolen #{}=covert lifted", obj.0),
    );
    // The thief takes possession — and still learns nothing.
    tw.world_mut().move_to(obj, thief).expect("the move");
    let seen = tw.run(thief, &format!("@asset/show #{}", obj.0));
    assert!(
        !texts(&seen).contains("REPORTED STOLEN"),
        "a covert claim leaked to the thief: {}",
        texts(&seen)
    );
    let owner_view = ok(&mut tw, owner, &format!("@asset/show #{}", obj.0));
    assert!(owner_view.contains("REPORTED STOLEN"), "{owner_view}");
    let staff_view = ok(&mut tw, GOD, &format!("@asset/show #{}", obj.0));
    assert!(staff_view.contains("REPORTED STOLEN"), "{staff_view}");
}

/// **The laundering hazard.** A fence sale of a reported-stolen object generates
/// evidence and clears nothing: no fence template carries `TitleTransfer`, the title
/// and the `Disputed` claim survive, and a re-quote of the same object still shows
/// the claim to anyone entitled to see it.
#[test]
fn e6b_b9_a_fence_cannot_launder_a_stolen_title() {
    let mut tw = boot();
    let seller = runner(&mut tw, "Jax", 0);
    let f = fence(&mut tw, "Ares", 10_000, 0);
    let obj = goods(&mut tw, seller, "predator", true);
    ok(
        &mut tw,
        seller,
        &format!("@asset/stolen #{}=public it is mine", obj.0),
    );
    let before = snap(&tw).asset(obj).expect("the asset").clone();

    ok(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{}", f.id, obj.0),
    );
    let q = sr(&tw).quotes().map(|x| x.id.0).max().expect("a quote");
    ok(&mut tw, seller, &format!("+fence/accept {q}"));
    let contract = newest_contract(&tw);
    // No fence template carries a title transfer, at all.
    let c = snap(&tw).contract(contract).expect("the contract").clone();
    assert!(
        !c.consent
            .terms
            .deliverables
            .iter()
            .any(|d| matches!(d, mush_economy::Deliverable::TitleTransfer { .. })),
        "a fence template carried a title transfer: {:?}",
        c.consent.terms.deliverables
    );
    ok(&mut tw, seller, &format!("@contract/accept {contract}"));
    ok(&mut tw, seller, &format!("+fence/deliver {q}"));
    let now = tw.world().now();
    tw.set_now(now + 60);
    tw.fire_due((now + 60) * 1_000);
    assert!(matches!(
        state(&tw, contract),
        ContractState::Settled { .. }
    ));

    let after = snap(&tw).asset(obj).expect("the asset");
    assert_eq!(after.title_owner, before.title_owner, "title moved");
    assert_eq!(
        after.encumbrances, before.encumbrances,
        "the claim was lost"
    );
    assert_eq!(after.stolen, before.stolen, "the report was cleared");
    // A later purchase is evidence, not guilt: the settlement stamped provenance.
    assert!(
        after.acquisition.is_none() || after.acquisition != before.acquisition,
        "the settlement changed nothing at all about provenance"
    );
}

// ===========================================================================
// B8 — the other three settlement routes
// ===========================================================================

/// `+run/post` now admits a `before=` row, `ContractSpec.deterministic` follows the
/// routes, and the board's `notice.kinds` stops being the constant `["attest"]`.
#[test]
fn e6b_b8_a_deterministic_route_sets_the_notice_kinds_and_the_deterministic_flag() {
    let mut tw = boot();
    let payer = runner(&mut tw, "Johnson", 10_000);
    // E6b independent verify fold (M2): a run that routes one row and leaves another
    // on an attestation must name the adjudicator whose word settles that row --
    // otherwise the payer settles it on their own.
    let arbiter = runner(&mut tw, "Arbiter", 0);
    let deadline = tw.world().now() + 10_000;
    ok(
        &mut tw,
        payer,
        &format!(
            "+run/post Milkrun=1000 outcomes=success,failure route=failure:before:{deadline} adjudicator=#{}",
            arbiter.0
        ),
    );
    let run = sr(&tw).runs().next().expect("a listing").clone();
    assert_eq!(
        run.notice.kinds,
        vec![SmolStr::new("attest"), SmolStr::new("before")],
        "the notice still says the constant"
    );
    let c = snap(&tw).contract(run.contract).expect("contract").clone();
    assert!(
        c.consent
            .evidence
            .contains(&mush_economy::EvidenceKind::Before),
        "the permitted evidence does not name the route it uses: {:?}",
        c.consent.evidence
    );
}

/// A **composite**: every gating condition plus one row. The gate's index lands in
/// every row's `when`, which is what makes it gating rather than decorative.
#[test]
fn e6b_b8_a_composite_needs_every_gate_plus_one_row() {
    let mut tw = boot();
    let payer = runner(&mut tw, "Johnson", 10_000);
    // M2 again: every row here is an attestation, so the surface names the
    // adjudicator whose word ends the run.
    let arbiter = runner(&mut tw, "Arbiter", 0);
    let deadline = tw.world().now() + 10_000;
    ok(
        &mut tw,
        payer,
        &format!(
            "+run/post Milkrun=1000 outcomes=success,failure gate=before:{deadline} adjudicator=#{}",
            arbiter.0
        ),
    );
    let run = sr(&tw).runs().next().expect("a listing").clone();
    assert_eq!(run.gates, vec![2], "the gate index is not after the rows");
    let c = snap(&tw).contract(run.contract).expect("contract").clone();
    assert_eq!(c.consent.conditions.len(), 3);
    for row in &c.consent.terms.outcomes {
        assert!(
            row.when.contains(&2),
            "a row does not name the gate: {:?}",
            row.when
        );
    }
}

/// A `canon=` route names its own attestor set and **no person adjudicator**, so the
/// crew check has nothing to find and the run is crewable.
#[test]
fn e6b_b8_a_canon_routed_run_names_no_person_adjudicator() {
    let mut tw = boot();
    let payer = runner(&mut tw, "Johnson", 10_000);
    ok(
        &mut tw,
        payer,
        "+run/post Extraction=1000 outcomes=success,failure \
         route=success:canon:scene.outcome:run:self|failure:canon:scene.outcome:run:self",
    );
    let run = sr(&tw).runs().next().expect("a listing").clone();
    let c = snap(&tw).contract(run.contract).expect("contract").clone();
    assert!(
        c.consent.adjudicator.is_none(),
        "a routed run defaulted a person adjudicator over its canon set"
    );
    assert!(
        matches!(
            c.consent.conditions.first(),
            Some(mush_economy::TypedCondition::Attestation {
                by: mush_economy::AttestorSet::Canon { .. }
            })
        ),
        "the canon route did not become a canon set: {:?}",
        c.consent.conditions
    );
    assert!(run.notice.kinds.contains(&SmolStr::new("canon")));
}

/// An **unregistered** canon kind is refused at offer, and the refusal leaves no
/// listing and no escrow behind (M17's shape).
#[test]
fn e6b_b8_an_unregistered_canon_kind_is_refused_at_post() {
    let mut tw = boot();
    let payer = runner(&mut tw, "Johnson", 10_000);
    err(
        &mut tw,
        payer,
        "+run/post Extraction=1000 route=success:canon:scene.concluded:run:1",
    );
    assert_eq!(sr(&tw).runs().count(), 0);
    assert_eq!(held(&tw, account_of(&tw, payer)), 0);
}

// ===========================================================================
// B10 — the augmentation record
// ===========================================================================

/// D-E6-16: the record is bounded, it deserializes through its wire, and **it is not
/// the sheet**. Nothing E6b ships writes `sheet:shadowrun`.
#[test]
fn e6b_b10_the_augmentation_record_is_bounded_and_is_never_the_sheet() {
    let mut tw = boot();
    let who = runner(&mut tw, "Jax", 0);
    let before = sheet(&tw, who);

    let aug: ruleset_shadowrun::InstalledAugmentation = serde_json::from_value(serde_json::json!({
        "object": null, "catalog": "wired.reflexes.1", "grade": "standard",
        "essence_cost": 200, "clinic": "DocWagon", "installed_at": 1,
    }))
    .expect("a well-formed installation loads");
    assert_eq!(aug.essence_cost, 200);

    // An impossible Essence cost, and an unnamed catalogue entry, never load.
    for bad in [
        serde_json::json!({ "catalog": "x", "essence_cost": 9_000 }),
        serde_json::json!({ "catalog": "", "essence_cost": 10 }),
    ] {
        assert!(serde_json::from_value::<ruleset_shadowrun::InstalledAugmentation>(bad).is_err());
    }

    // Recording one bumps the profile and touches no sheet.
    let mut profile = sr(&tw).profile(who).expect("a profile").clone();
    assert!(profile.install(aug));
    assert_eq!(profile.augmentations.len(), 1);
    assert_eq!(
        sheet(&tw, who),
        before,
        "recording an augmentation wrote the sheet"
    );
}

// ===========================================================================
// B7/B8 — the sealed outcome settles a run, end to end
// ===========================================================================

/// The whole board loop on the canon route: post naming **this** run
/// (`run:self`), crew it, seal the outcome, and watch E4 pay the row it selects.
#[test]
fn e6b_b7_a_sealed_outcome_selects_the_matching_row_and_pays_it() {
    let mut tw = boot();
    let payer = runner(&mut tw, "Johnson", 10_000);
    let crew = runner(&mut tw, "Jax", 0);
    let room = tw
        .world_mut()
        .create(ObjKind::Room, "The Meet", GOD, None)
        .expect("a room");
    tw.world_mut().move_to(crew, room).expect("the move");
    tw.world_mut().move_to(GOD, room).expect("the move");
    tw.world_mut().set_connected(crew, true);
    tw.world_mut().set_live_conns(crew, 1);

    // **One row**, decided by the seal alone. A bare `Before` row would be
    // satisfied the moment the last runner accepted (D-E6-20), which is exactly the
    // ambiguity the next test is about — not this one.
    ok(
        &mut tw,
        payer,
        "+run/post Extraction=1000 outcomes=success \
         route=success:canon:scene.outcome:run:self",
    );
    let run = sr(&tw).runs().next().expect("a listing").clone();
    let contract = run.contract;
    assert!(run.notice.kinds.contains(&SmolStr::new("canon")));

    // Crew it: the party applies, the payer accepts, the runner binds.
    ok(&mut tw, crew, "+party/create Wetwork");
    let party = mush_party::parties_of(tw.world(), crew)
        .first()
        .expect("a live party")
        .0;
    ok(&mut tw, crew, &format!("+run/apply {}={party}", run.id.0));
    ok(&mut tw, payer, &format!("+run/crew {}=accept", run.id.0));
    ok(&mut tw, crew, &format!("@contract/accept {contract}"));
    assert_eq!(state(&tw, contract), ContractState::Active);
    let before = bal(&tw, account_of(&tw, crew));

    // Confer, seal, settle.
    ok(&mut tw, crew, "+timestop");
    let scene = tw
        .world()
        .module_state::<mush_scenes::ScenesState>()
        .and_then(|st| st.latest_scene_at(room).map(|s| s.id))
        .expect("a scene");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@scene/facilitate {}=#{} run={}",
            scene.0, crew.0, contract.0
        ),
    );
    ok(
        &mut tw,
        crew,
        &format!("+scene/outcome success=Jax run={contract}"),
    );
    let c = snap(&tw).contract(contract).expect("contract").clone();
    assert!(
        matches!(state(&tw, contract), ContractState::Settled { .. }),
        "the seal did not settle the run: {:?} / conditions {:?} / satisfied {:?} / rows {:?}",
        state(&tw, contract),
        c.consent.conditions,
        c.satisfied,
        c.consent.terms.outcomes
    );
    assert!(
        bal(&tw, account_of(&tw, crew)) > before,
        "the selected row paid nothing"
    );
}

/// **Two rows satisfied at once reads `Undecidable` and disputes, never freezes** —
/// the money stays reserved and the parties are paged rather than the world stopping.
///
/// Since E6b's own verify fold (M5) a canon source names its **row**, so two canon
/// rows on one run can no longer both match; the ambiguity that remains is the real
/// one D-E6-20 already named — a bare `Before` row is ready the moment the last
/// runner accepts, so a seal on another row makes two rows ready together.
#[test]
fn e6b_b7_two_rows_satisfied_at_once_disputes_rather_than_freezing() {
    let mut tw = boot();
    let payer = runner(&mut tw, "Johnson", 10_000);
    let crew = runner(&mut tw, "Jax", 0);
    let room = tw
        .world_mut()
        .create(ObjKind::Room, "The Meet", GOD, None)
        .expect("a room");
    tw.world_mut().move_to(crew, room).expect("the move");
    tw.world_mut().move_to(GOD, room).expect("the move");
    tw.world_mut().set_connected(crew, true);
    tw.world_mut().set_live_conns(crew, 1);

    // The seal answers `success`; the `failure` row is a bare `Before`, satisfied
    // from the moment the last runner accepts (D-E6-20). Both rows are therefore
    // ready at once and the contract cannot say which one happened.
    let deadline = tw.world().now() + 1_000_000;
    ok(
        &mut tw,
        payer,
        &format!(
            "+run/post Extraction=1000 outcomes=success,failure \
             route=success:canon:scene.outcome:run:self|failure:before:{deadline}"
        ),
    );
    let run = sr(&tw).runs().next().expect("a listing").clone();
    let contract = run.contract;
    ok(&mut tw, crew, "+party/create Wetwork");
    let party = mush_party::parties_of(tw.world(), crew)
        .first()
        .expect("a live party")
        .0;
    ok(&mut tw, crew, &format!("+run/apply {}={party}", run.id.0));
    ok(&mut tw, payer, &format!("+run/crew {}=accept", run.id.0));
    ok(&mut tw, crew, &format!("@contract/accept {contract}"));
    let before = bal(&tw, account_of(&tw, crew));

    ok(&mut tw, crew, "+timestop");
    let scene = tw
        .world()
        .module_state::<mush_scenes::ScenesState>()
        .and_then(|st| st.latest_scene_at(room).map(|s| s.id))
        .expect("a scene");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@scene/facilitate {}=#{} run={}",
            scene.0, crew.0, contract.0
        ),
    );
    ok(
        &mut tw,
        crew,
        &format!("+scene/outcome success=Jax run={contract}"),
    );
    let now = tw.world().now();
    tw.set_now(now + 60);
    tw.fire_due((now + 60) * 1_000);
    assert_eq!(
        state(&tw, contract),
        ContractState::Disputed,
        "an ambiguous outcome should dispute, not settle or freeze: {:?}",
        state(&tw, contract)
    );
    assert_eq!(
        bal(&tw, account_of(&tw, crew)),
        before,
        "an undecidable outcome paid somebody"
    );
    // The world did not stop: the economy is still posting.
    assert!(
        !tw.world()
            .module_state::<EconomyStore>()
            .is_some_and(EconomyStore::is_frozen)
    );
}

// ===========================================================================
// B5 — C16, bounded per-actor refusal telemetry
// ===========================================================================

/// C16: the existence-hiding renderer is where a probe becomes indistinguishable
/// from a typo — which is exactly why staff need to see the *shape* of one. The
/// tally is bounded, RAM-only, and read by nothing that decides anything.
#[test]
fn e6b_b5_existence_hiding_refusals_are_counted_and_surfaced_to_staff() {
    let mut tw = boot();
    let owner = runner(&mut tw, "Jax", 0);
    let prober = runner(&mut tw, "Nosy", 0);
    let obj = goods(&mut tw, owner, "predator", true);
    for _ in 0..3 {
        err(
            &mut tw,
            prober,
            &format!("@asset/stolen #{}=public mine now", obj.0),
        );
    }
    let tallies = tw
        .world()
        .module_state::<EconomyStore>()
        .expect("economy store")
        .refusal_tallies();
    let (who, tally) = tallies
        .iter()
        .find(|(w, _)| *w == prober)
        .expect("the prober was counted");
    assert_eq!(*who, prober);
    assert!(
        tally.hidden >= 3,
        "hidden refusals were not counted: {tally:?}"
    );
    let alerts = ok(&mut tw, GOD, "@econ/alerts");
    assert!(
        alerts.contains("Refusals"),
        "the tally is not surfaced by @econ/alerts: {alerts}"
    );
}

// ===========================================================================
// E6b verify fold — three lenses, one regression each
// ===========================================================================

/// **D1.** A **covert** theft report must not move a quote's state. It did:
/// `ReportStolen` bumps `EconomicAsset.revision` like every asset op, the quote was
/// bound to that revision, and the holder's outstanding quote flipped from `open` to
/// `lapsed` where they could read it. The price was identical in both worlds and the
/// *rendered state* was not, which is the same oracle D-E6-11 forbids one step later.
#[test]
fn vf_d1_a_covert_report_does_not_lapse_the_holders_quote() {
    let mut tw = boot();
    let owner = runner(&mut tw, "Jax", 0);
    let f = fence(&mut tw, "Ares", 10_000, 0);
    let obj = goods(&mut tw, owner, "predator", true);
    ok(&mut tw, owner, &format!("+fence/quote {}=#{}", f.id, obj.0));
    let q = sr(&tw).quotes().next().expect("a quote").id.0;
    let before = ok(&mut tw, owner, &format!("+fence/show {q}"));
    assert!(before.contains("open"), "{before}");

    ok(
        &mut tw,
        GOD,
        &format!("@asset/stolen #{}=covert lifted from a courier", obj.0),
    );
    let after = ok(&mut tw, owner, &format!("+fence/show {q}"));
    assert!(
        after.contains("open"),
        "a covert report lapsed the quote, so its state is an oracle: {after}"
    );
    // And the deal still closes, on the same number.
    ok(&mut tw, owner, &format!("+fence/accept {q}"));
    assert_eq!(held(&tw, f.account), 300);
}

/// **D2.** The **count** must not disclose what the line below it hides: a covert
/// report records a `Disputed` claim like any other, and `encumbrances: {n}` sat one
/// line above the gated report.
#[test]
fn vf_d2_the_encumbrance_count_does_not_disclose_a_covert_claim() {
    let mut tw = boot();
    let owner = runner(&mut tw, "Jax", 0);
    let watcher = runner(&mut tw, "Nosy", 0);
    let obj = goods(&mut tw, owner, "predator", true);
    // The watcher is the assignee: they may view the asset, and may not be told
    // about a covert claim on it.
    ok(
        &mut tw,
        GOD,
        &format!("@asset/assign #{}=#{} minding it", obj.0, watcher.0),
    );
    ok(
        &mut tw,
        owner,
        &format!("@asset/stolen #{}=covert lifted", obj.0),
    );
    let seen = ok(&mut tw, watcher, &format!("@asset/show #{}", obj.0));
    assert!(
        seen.contains("encumbrances: 0"),
        "the count leaked a covert claim: {seen}"
    );
    assert!(!seen.contains("REPORTED STOLEN"), "{seen}");
    // The owner, who filed it, sees both.
    let owner_view = ok(&mut tw, owner, &format!("@asset/show #{}", obj.0));
    assert!(owner_view.contains("encumbrances: 1"), "{owner_view}");
}

/// **D3.** `+fence/list` filters **then** pages: taking the page before the
/// hidden-org filter made a member's and a non-member's page truncate differently,
/// which is a difference a non-member can read.
#[test]
fn vf_d3_the_fence_page_filters_before_it_pages() {
    let mut tw = boot();
    let outsider = runner(&mut tw, "Jax", 0);
    let hidden = fence(&mut tw, "Ares", 10_000, 0);
    let open = fence(&mut tw, "Aztech", 10_000, 0);
    let mut kv = fold_all(&mut tw);
    let orgs = kv
        .get_mut(&SmolStr::new("org"))
        .expect("the org namespace wrote");
    let key = SmolStr::new(format!("org:{}", hidden.org.0));
    for (k, v) in orgs.iter_mut() {
        if *k == key {
            v["hidden"] = serde_json::json!(true);
        }
    }
    reboot(&mut tw, &kv);
    let seen = ok(&mut tw, outsider, "+fence/list");
    assert!(!seen.contains("Ares"), "{seen}");
    assert!(
        seen.contains("Aztech"),
        "the visible fence fell off the page behind the hidden one: {seen}"
    );
    let _ = open;
}

/// **D6.** A record cap is enforced on **load** as well as on write: a store carrying
/// more quotes than the ceiling boots with the ceiling and says so, rather than
/// quietly keeping whatever the documents held.
#[test]
fn vf_d6_the_quote_cap_is_enforced_on_load() {
    let mut tw = boot();
    let seller = runner(&mut tw, "Jax", 0);
    let f = fence(&mut tw, "Ares", 10_000, 0);
    let obj = goods(&mut tw, seller, "predator", true);
    ok(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{}", f.id, obj.0),
    );
    let mut kv = fold_all(&mut tw);
    let ns = kv
        .get_mut(&SmolStr::new(NS))
        .expect("the ruleset namespace wrote");
    let template = ns
        .iter()
        .find(|(k, _)| k.as_str().starts_with("sr:quote:"))
        .map(|(_, v)| v.clone())
        .expect("a quote document");
    for id in 1..=(ruleset_shadowrun::MAX_QUOTES as u64 + 8) {
        let mut doc = template.clone();
        doc["id"] = serde_json::json!(id);
        ns.push((SmolStr::new(format!("sr:quote:{id}")), doc));
    }
    reboot(&mut tw, &kv);
    assert_eq!(
        sr(&tw).quotes().count(),
        ruleset_shadowrun::MAX_QUOTES,
        "the load path kept more quotes than the cap allows"
    );
}

/// **D8.** C16's tally exists so staff can see the shape of an id-space walk, and
/// `FenceId`/`QuoteId` are two brand-new enumerable id spaces — a refusal built
/// straight into a `CmdError` never reached it.
#[test]
fn vf_d8_a_fence_id_walk_is_counted_by_the_refusal_telemetry() {
    let mut tw = boot();
    let prober = runner(&mut tw, "Nosy", 0);
    for id in 0..4u64 {
        err(&mut tw, prober, &format!("+fence/quote {id}=nothing"));
    }
    let tallies = tw
        .world()
        .module_state::<EconomyStore>()
        .expect("economy store")
        .refusal_tallies();
    let (_, tally) = tallies
        .iter()
        .find(|(w, _)| *w == prober)
        .expect("the fence walk was counted");
    assert!(tally.hidden >= 4, "{tally:?}");
}

/// **R8.** The allocator floors come from every document's **parsed key**, loaded or
/// not: a corrupt `sr:fence:{n}` plus a lost `sr:meta` used to re-mint id `n` and
/// `JsonSet` straight over the evidence.
#[test]
fn vf_r8_a_corrupt_document_still_claims_its_id() {
    let mut tw = boot();
    let f = fence(&mut tw, "Ares", 10_000, 0);
    let mut kv = fold_all(&mut tw);
    let ns = kv
        .get_mut(&SmolStr::new(NS))
        .expect("the ruleset namespace wrote");
    // Corrupt the fence document and lose the allocator.
    for (k, v) in ns.iter_mut() {
        if k.as_str() == format!("sr:fence:{}", f.id) {
            v["demand"] = serde_json::json!(9_000);
        }
    }
    ns.retain(|(k, _)| k.as_str() != "sr:meta");
    reboot(&mut tw, &kv);
    assert!(
        sr(&tw).fence(ruleset_shadowrun::FenceId(f.id)).is_none(),
        "the corrupt document was trusted"
    );
    // The id is still claimed: the next mint is past it, so nothing overwrites the
    // evidence and no live quote is silently re-pointed at a different organization.
    let next = sr(&tw).next_fence_id();
    assert!(next > f.id, "id {next} re-minted over a corrupt document");
}

/// **M5.** A canon source names its **row**, so a seal selects the outcome it names.
/// With one source per run, one seal filled every canon row's slot, and with a
/// single row the row name the sealer typed decided nothing at all.
#[test]
fn vf_m5_a_canon_route_names_its_row_so_the_seal_selects_it() {
    let mut tw = boot();
    let payer = runner(&mut tw, "Johnson", 10_000);
    ok(
        &mut tw,
        payer,
        "+run/post Extraction=1000 outcomes=success,failure \
         route=success:canon:scene.outcome:run:self|failure:canon:scene.outcome:run:self",
    );
    let run = sr(&tw).runs().next().expect("a listing").clone();
    let c = snap(&tw).contract(run.contract).expect("contract").clone();
    let sources: Vec<String> = c
        .consent
        .conditions
        .iter()
        .filter_map(|con| match con {
            mush_economy::TypedCondition::Attestation {
                by: mush_economy::AttestorSet::Canon { source, .. },
            } => source.as_ref().map(std::string::ToString::to_string),
            _ => None,
        })
        .collect();
    assert_eq!(
        sources,
        vec![
            format!("run:{}#success", run.contract.0),
            format!("run:{}#failure", run.contract.0),
        ],
        "two canon rows share a source, so one seal answers both"
    );
}

/// **M11.** A `canon:` route names **this** run. Written free-hand it let one listing
/// piggyback on another's seal.
#[test]
fn vf_m11_a_canon_route_naming_another_run_is_refused_at_post() {
    let mut tw = boot();
    let payer = runner(&mut tw, "Johnson", 10_000);
    let refusal = err(
        &mut tw,
        payer,
        "+run/post Extraction=1000 route=success:canon:scene.outcome:run:99|failure:canon:scene.outcome:run:99",
    );
    assert!(refusal.contains("names its own run"), "{refusal}");
    assert_eq!(sr(&tw).runs().count(), 0);
    assert_eq!(held(&tw, account_of(&tw, payer)), 0);
}

// ===========================================================================
// E6b independent verify fold
// ===========================================================================

/// **D1.** The `canon:` source check was a **denylist** — it refused only a
/// `run:<n>` naming another run — so every other prefix listed happily. A run could
/// cite `scene:{uid}`, which is exactly the source a scene bound to **no** run seals
/// under: staff's "chronicle only" conferral was therefore a live settlement
/// authority for any listing that named the uid. It is an allowlist now: after the
/// `run:self` substitution a canon source is exactly `run:{this}#{one of its rows}`.
#[test]
fn vf2_d1_a_canon_route_is_allowlisted_to_this_runs_own_rows() {
    let mut tw = boot();
    let payer = runner(&mut tw, "Johnson", 10_000);

    // The probe: a scene source. A `scene:{uid}` is what an unbound scene's seal
    // carries, so this listing would have settled on a chronicle-only seal.
    let refusal = err(
        &mut tw,
        payer,
        "+run/post Extraction=1000 route=success:canon:scene.outcome:scene:1|\
         failure:canon:scene.outcome:scene:1",
    );
    assert!(refusal.contains("names its own run"), "{refusal}");

    // Another run's id, and a row this run does not carry, are refused the same way.
    for spec in [
        "route=success:canon:scene.outcome:run:99|failure:canon:scene.outcome:run:99",
        "route=success:canon:scene.outcome:run:self|failure:canon:scene.outcome:run:0#nosuchrow",
    ] {
        let refusal = err(&mut tw, payer, &format!("+run/post Extraction=1000 {spec}"));
        assert!(refusal.contains("names its own run"), "{spec}: {refusal}");
    }
    assert_eq!(sr(&tw).runs().count(), 0, "a refused post left a listing");
    assert_eq!(
        held(&tw, account_of(&tw, payer)),
        0,
        "it left an escrow too"
    );

    // And `run:self` still lists, resolved to this run and this row.
    ok(
        &mut tw,
        payer,
        "+run/post Extraction=1000 route=success:canon:scene.outcome:run:self|\
         failure:canon:scene.outcome:run:self",
    );
    let run = sr(&tw).runs().next().expect("a listing").clone();
    let c = snap(&tw).contract(run.contract).expect("contract").clone();
    let sources: Vec<String> = c
        .consent
        .conditions
        .iter()
        .filter_map(|cond| match cond {
            mush_economy::TypedCondition::Attestation {
                by: mush_economy::AttestorSet::Canon { source, .. },
            } => source.as_ref().map(ToString::to_string),
            _ => None,
        })
        .collect();
    assert_eq!(
        sources,
        vec![
            format!("run:{}#success", run.contract.0),
            format!("run:{}#failure", run.contract.0),
        ]
    );
}

/// **M3.** A `canon:` **gate** could never be satisfied: `+run/post` rewrote a
/// gate's `run:self` to a bare `run:{next}`, while the producer's seal always emits
/// `run:{contract}#{row}`. A crewed run carrying one froze its escrow to the
/// deadline with nothing anywhere saying why. Canon evidence is per row.
#[test]
fn vf2_m3_a_canon_gate_is_refused_at_post() {
    let mut tw = boot();
    let payer = runner(&mut tw, "Johnson", 10_000);
    let refusal = err(
        &mut tw,
        payer,
        "+run/post Extraction=1000 gate=canon:scene.outcome:run:self",
    );
    assert!(
        refusal.contains("A gate cannot be a canon route"),
        "{refusal}"
    );
    assert_eq!(sr(&tw).runs().count(), 0);
    assert_eq!(held(&tw, account_of(&tw, payer)), 0);
}

/// **M2.** `parse_routes` defaults every unnamed row to `attest`, and a deterministic
/// surface names **no person adjudicator** — so `consent.adjudicator` was `None`, and
/// `@contract/attest`'s `None` arm fell back to the condition's own set, which is
/// `Party(payer)`. The payer could settle the unrouted row on their own word while
/// the crew check read the surface as `Impersonal`. Either every row is routed, or
/// the adjudicator is named.
#[test]
fn vf2_m2_a_partly_routed_run_must_name_the_adjudicator_of_the_rest() {
    let mut tw = boot();
    let payer = runner(&mut tw, "Johnson", 10_000);
    let arbiter = runner(&mut tw, "Arbiter", 0);
    let deadline = tw.world().now() + 10_000;

    let refusal = err(
        &mut tw,
        payer,
        &format!("+run/post Extraction=1000 route=failure:before:{deadline}"),
    );
    assert!(refusal.contains("settle the unrouted row"), "{refusal}");
    assert_eq!(sr(&tw).runs().count(), 0);
    assert_eq!(held(&tw, account_of(&tw, payer)), 0);

    // Named, and the surface hands the row to the adjudicator rather than the payer.
    ok(
        &mut tw,
        payer,
        &format!(
            "+run/post Extraction=1000 route=failure:before:{deadline} adjudicator=#{}",
            arbiter.0
        ),
    );
    let run = sr(&tw).runs().next().expect("a listing").clone();
    let c = snap(&tw).contract(run.contract).expect("contract").clone();
    assert_eq!(
        c.consent.adjudicator,
        Some(mush_economy::AttestorSet::Party(PrincipalId::Object(
            arbiter
        ))),
        "the adjudicator is not the named one: {:?}",
        c.consent.adjudicator
    );
    // The payer's own attest of the unrouted row is refused: the adjudicator
    // replaces every condition's set.
    let refusal = err(
        &mut tw,
        payer,
        &format!("@contract/attest {}=0", run.contract),
    );
    assert!(refusal.contains("names an adjudicator"), "{refusal}");
}

/// **M1.** `TypedCondition::DepositInto` is a pure state predicate, and nothing
/// refused a second quote — or a second contract — naming the same object. Eight
/// quotes on one item were eight escrows, all satisfied by the single hand-over that
/// satisfies the first: one object, one delivery, N payments.
#[test]
fn vf2_m1_one_object_is_one_open_quote_and_one_live_deposit() {
    let mut tw = boot();
    let seller = runner(&mut tw, "Jax", 0);
    let f = fence(&mut tw, "Ares", 10_000, 0);
    let other = fence(&mut tw, "Aztech", 10_000, 0);
    let obj = goods(&mut tw, seller, "predator", true);

    ok(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{}", f.id, obj.0),
    );
    // A second quote on the same object, at the same fence and at another one.
    for id in [f.id, other.id] {
        let refusal = err(&mut tw, seller, &format!("+fence/quote {id}=#{}", obj.0));
        assert!(refusal.contains("already under an open quote"), "{refusal}");
    }
    assert_eq!(sr(&tw).quotes().count(), 1);

    // And with the deal live, the economy refuses a second contract naming the same
    // deposit — the layer under the fence, and the one a different producer would
    // otherwise walk straight past.
    let q = sr(&tw).quotes().next().expect("a quote").id.0;
    ok(&mut tw, seller, &format!("+fence/accept {q}"));
    let contract = newest_contract(&tw);
    ok(&mut tw, seller, &format!("@contract/accept {contract}"));
    assert_eq!(state(&tw, contract), ContractState::Active);

    // The second contract names a vault the **seller** owns, so the vault-owner rule
    // the earlier fold added is not what refuses: the deposit lock is.
    let backing = tw
        .world_mut()
        .create(ObjKind::Thing, "footlocker", GOD, None)
        .expect("a container");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vault/create Locker=container:#{} owner=#{}",
            backing.0, seller.0
        ),
    );
    let mine = snap(&tw).vaults().map(|v| v.id).max().expect("a vault");
    ok(&mut tw, seller, "@contract/draft Poach");
    let second = newest_contract(&tw);
    let s_acct = account_of(&tw, seller);
    ok(
        &mut tw,
        seller,
        &format!("@contract/pay {second}={s_acct} 1"),
    );
    ok(
        &mut tw,
        seller,
        &format!("@contract/remainder {second}={s_acct}"),
    );
    ok(
        &mut tw,
        seller,
        &format!("@contract/condition {second}=deposit #{} {}", obj.0, mine.0),
    );
    let refusal = err(&mut tw, seller, &format!("@contract/offer {second}"));
    assert!(refusal.contains("already names that object"), "{refusal}");
    assert_eq!(held(&tw, f.account), 300, "a second escrow was placed");
}

/// **M5.** `+fence/accept` re-checked the expiry and the revisions but **not
/// custody**, so a seller could be quoted, hand the goods away, and still make the
/// fence escrow the whole price against goods they demonstrably do not have — the
/// fence's liquidity locked to the deal's deadline for nothing.
#[test]
fn vf2_m5_accept_re_asserts_custody_before_the_fence_escrows() {
    let mut tw = boot();
    let seller = runner(&mut tw, "Jax", 0);
    let confederate = runner(&mut tw, "Kez", 0);
    let f = fence(&mut tw, "Ares", 10_000, 0);
    let obj = goods(&mut tw, seller, "predator", true);

    ok(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{}", f.id, obj.0),
    );
    let q = sr(&tw).quotes().next().expect("a quote").id.0;
    tw.world_mut()
        .move_to(obj, confederate)
        .expect("the goods move");

    let refusal = err(&mut tw, seller, &format!("+fence/accept {q}"));
    assert!(refusal.contains("not holding that any more"), "{refusal}");
    assert_eq!(held(&tw, f.account), 0, "the fence escrowed anyway");
    assert_eq!(snap(&tw).contracts().count(), 0, "a contract was offered");
}

/// **D4.** `close_quote` runs only from the contract observer, so a quote that simply
/// **lapsed** was never deleted — and the per-seller count included it. Eight lapses
/// wedged a seller out of the fence for ever, behind a refusal telling them to let
/// one lapse. The count is of live quotes, and the walk clears the dead ones.
#[test]
fn vf2_d4_lapsed_quotes_are_swept_and_do_not_wedge_their_seller() {
    let mut tw = boot();
    let seller = runner(&mut tw, "Jax", 0);
    let f = fence(&mut tw, "Ares", 10_000, 0);
    let cap = ruleset_shadowrun::MAX_OPEN_QUOTES_PER_SELLER;
    let mut objs = Vec::new();
    for i in 0..cap {
        let obj = goods(&mut tw, seller, &format!("predator {i}"), true);
        ok(
            &mut tw,
            seller,
            &format!("+fence/quote {}=#{}", f.id, obj.0),
        );
        objs.push(obj);
    }
    assert_eq!(sr(&tw).quotes().count(), cap);
    // At the cap, a ninth is refused — that part is unchanged.
    let ninth = goods(&mut tw, seller, "ninth", true);
    let refusal = err(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{}", f.id, ninth.0),
    );
    assert!(refusal.contains("as many open quotes"), "{refusal}");

    // Let every one of them lapse.
    let now = tw.world().now();
    tw.set_now(now + ruleset_shadowrun::FENCE_QUOTE_TTL_SECS + 1);
    ok(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{}", f.id, ninth.0),
    );
    assert_eq!(
        sr(&tw).quotes().count(),
        1,
        "the lapsed quotes are still on the store: {:?}",
        sr(&tw).quotes().map(|q| q.id.0).collect::<Vec<_>>()
    );
    assert!(
        sr(&tw).quotes().all(|q| q.object == ninth),
        "the surviving quote is not the new one"
    );
}

/// **D5.** The whole-store quote ceiling was enforced on **load** only, while the
/// write path checked the per-seller eight — so many sellers put the store past it
/// legitimately and the next boot silently dropped the overflow, contract-bound
/// quotes included. The write path refuses now, and the loader prefers a quote whose
/// deal is standing over a re-askable price.
#[test]
fn vf2_d5_the_quote_ceiling_is_enforced_on_write_and_the_loader_keeps_the_bound_ones() {
    let mut tw = boot();
    let seller = runner(&mut tw, "Jax", 0);
    let f = fence(&mut tw, "Ares", 10_000, 0);
    let obj = goods(&mut tw, seller, "predator", true);
    ok(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{}", f.id, obj.0),
    );
    let template = sr(&tw).quotes().next().expect("a quote").clone();

    // A store that already carries the ceiling, filled with somebody else's quotes
    // so the per-seller cap is not what answers.
    let mut kv = fold_all(&mut tw);
    let quotes = kv.entry(SmolStr::new(ruleset_shadowrun::NS)).or_default();
    quotes.retain(|(k, _)| !k.starts_with("sr:quote:"));
    for n in 0..ruleset_shadowrun::MAX_QUOTES as u64 {
        let mut q = template.clone();
        q.id = ruleset_shadowrun::QuoteId(1_000 + n);
        q.seller = GOD;
        q.object = ObjId(90_000 + u32::try_from(n).expect("small"));
        quotes.push((
            ruleset_shadowrun::quote_key(q.id),
            serde_json::to_value(&q).expect("a quote serializes"),
        ));
    }
    reboot(&mut tw, &kv);
    assert_eq!(sr(&tw).quotes().count(), ruleset_shadowrun::MAX_QUOTES);

    let more = goods(&mut tw, seller, "one too many", true);
    let refusal = err(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{}", f.id, more.0),
    );
    assert!(refusal.contains("fences are busy"), "{refusal}");

    // And the loader half: past the ceiling, a quote **bound to a live contract** is
    // not a re-askable price, so it displaces an unbound one instead of being
    // dropped. Six bound quotes, all of them past the cap in id order.
    let mut docs: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for n in 0..ruleset_shadowrun::MAX_QUOTES as u64 {
        let mut q = template.clone();
        q.id = ruleset_shadowrun::QuoteId(n);
        q.contract = None;
        q.object = ObjId(90_000 + u32::try_from(n).expect("small"));
        docs.push((
            ruleset_shadowrun::quote_key(q.id),
            serde_json::to_value(&q).expect("serializes"),
        ));
    }
    let bound: Vec<ruleset_shadowrun::QuoteId> = (0..6)
        .map(|n| ruleset_shadowrun::QuoteId(ruleset_shadowrun::MAX_QUOTES as u64 + n))
        .collect();
    for (i, id) in bound.iter().enumerate() {
        let mut q = template.clone();
        q.id = *id;
        q.contract = Some(ContractId(500 + i as u64));
        q.object = ObjId(80_000 + u32::try_from(i).expect("small"));
        docs.push((
            ruleset_shadowrun::quote_key(*id),
            serde_json::to_value(&q).expect("serializes"),
        ));
    }
    let loaded = ruleset_shadowrun::load_store(&docs, sr_config());
    assert_eq!(loaded.quotes().count(), ruleset_shadowrun::MAX_QUOTES);
    for id in &bound {
        assert!(
            loaded.quote(*id).is_some(),
            "a contract-bound quote was dropped at the cap: {}",
            id.0
        );
    }
}

/// **M9.** A live `DepositInto` names the fence's vault by id and is **frozen** into
/// the consent surface, so re-registering the fence onto another vault left every
/// standing deal pointing at the old one — the seller's delivery lands where the
/// condition still says, or the deal simply never settles. Refused while anything is
/// standing, with what has to end first.
#[test]
fn vf2_m9_a_fences_vault_does_not_move_under_a_live_deal() {
    let mut tw = boot();
    let seller = runner(&mut tw, "Jax", 0);
    let f = fence(&mut tw, "Ares", 10_000, 0);
    let obj = goods(&mut tw, seller, "predator", true);
    ok(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{}", f.id, obj.0),
    );
    let q = sr(&tw).quotes().next().expect("a quote").id.0;
    ok(&mut tw, seller, &format!("+fence/accept {q}"));

    let backing = tw
        .world_mut()
        .create(ObjKind::Thing, "second strongbox", GOD, None)
        .expect("a container");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vault/create Ares2=container:#{} owner=org:#{}",
            backing.0, f.org.0
        ),
    );
    let other = snap(&tw)
        .vaults()
        .map(|v| v.id)
        .max()
        .expect("the second vault");
    let refusal = err(
        &mut tw,
        GOD,
        &format!("@sr/fence #{}=#{} name=Ares", f.org.0, other.0),
    );
    assert!(refusal.contains("still standing"), "{refusal}");
    assert_eq!(
        sr(&tw)
            .fence(ruleset_shadowrun::FenceId(f.id))
            .map(|x| x.vault),
        Some(f.vault),
        "the vault moved anyway"
    );
    // Everything else about the fence is still editable, and bumping its revision
    // lapses the outstanding quote deterministically.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@sr/fence #{}=#{} name=Ares demand=-100",
            f.org.0, f.vault.0
        ),
    );
}

/// **M7.** `CatalogRef` was writable only by `AssetOp::Title`, and `Title` refuses an
/// object that already has a live record — so every title minted before E6b could
/// never be catalogued, and every catalogue-keyed rule was unreachable for it. The
/// fence is the shipped consequence: an uncatalogued object cannot be quoted at all.
/// `@asset/catalog` is the missing verb, and it bumps the revision, which is what
/// makes D-E6-13's stale-revision refusal reachable.
#[test]
fn vf2_m7_an_existing_title_can_be_catalogued_and_the_change_lapses_a_quote() {
    let mut tw = boot();
    let seller = runner(&mut tw, "Jax", 0);
    let f = fence(&mut tw, "Ares", 10_000, 0);
    // A pre-E6b shaped title: no catalogue reference at all.
    let obj = goods(&mut tw, seller, "predator", false);
    let refusal = err(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{}", f.id, obj.0),
    );
    assert!(refusal.contains("no catalogue entry"), "{refusal}");

    // Staff catalogue it, and it becomes quotable.
    ok(
        &mut tw,
        GOD,
        &format!("@asset/catalog #{}=sr:{CODE}", obj.0),
    );
    assert_eq!(
        snap(&tw)
            .asset(obj)
            .and_then(|a| a.catalog.as_ref().map(|c| c.code.to_string())),
        Some(CODE.to_string())
    );
    ok(
        &mut tw,
        seller,
        &format!("+fence/quote {}=#{}", f.id, obj.0),
    );
    let q = sr(&tw).quotes().next().expect("a quote").id.0;

    // Re-cataloguing is a **declared** change to what the thing is, so the
    // outstanding quote lapses rather than being re-priced.
    ok(
        &mut tw,
        GOD,
        &format!("@asset/catalog #{}=sr:other.code", obj.0),
    );
    let refusal = err(&mut tw, seller, &format!("+fence/accept {q}"));
    assert!(refusal.contains("lapsed"), "{refusal}");
    // …and a player is not staff.
    let refusal = err(
        &mut tw,
        seller,
        &format!("@asset/catalog #{}=sr:{CODE}", obj.0),
    );
    assert!(!refusal.is_empty(), "a non-staff catalogue was accepted");
}
