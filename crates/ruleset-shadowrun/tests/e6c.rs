//! E6c acceptance (08f §5.4): **the neutral market**.
//!
//! The verify axis of this cut is *market existence oracles, stale quotes,
//! market-maker insolvency*, so the rhythms here are:
//!
//! - **the gate first.** Two worlds differing only in state the viewer may not see
//!   publish byte-identical projections — hidden listings, a hidden vendor
//!   organization, and another buyer's open quotes, each on its own.
//! - **nothing is reserved.** A quote is a price; the listing's quantity moves only
//!   when a contract comes into force, and the loser of a race reads the same
//!   deterministic refusal a stale quote gets, with nothing charged.
//! - **every stale reason answers alike.** A lapsed quote, a re-priced listing, a
//!   bumped catalogue version, a halted maker and a retired vendor are one string.
//! - **funding is enumerated.** A maker's balance after N sales and M drips equals
//!   capitalization + sales + drips exactly, and every unenumerated route refuses.
//!
//! Everything runs through the real `Engine` via `TestWorld`, on the production
//! module composition (org → faction → economy → party → market → the ruleset), with
//! `MarketObserver` in the economy's frozen observer table and the Shadowrun content,
//! dice and delivery providers in the market's three.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use mush_core::{Audience, GOD, ModuleBootData, ModuleWrite, ObjId, ObjKind};
use mush_economy::{
    AccountId, ContractId, ContractState, DeclaredCadence, DeclaredCadenceClock, EconomyConfig,
    EconomySnapshot, EconomyStore, PartyCap, VaultId,
};
use mush_market::{
    ListingId, MarketId, MarketStore, Projection, QuoteId, Representation, VendorId,
};
use mush_org::{CapabilityId, OrgId, PrincipalId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use ruleset_shadowrun::{SrBoardObserver, SrEconomyConfig, SrEconomyStore};
use smol_str::SmolStr;

const PERIOD: i64 = 100;
/// The market the fixture trades in.
const VENUE: &str = "seattle";
/// A stocked, legal, asset-represented good from the `sr.core` starter pack.
const GUN: &str = "sr:colt.america";
/// A stocked, legal, commodity-represented good.
const AMMO: &str = "sr:ammo.regular";
/// A restricted good whose availability is an engine-resolved test.
const APDS: &str = "sr:ammo.apds";
/// A legal entitlement.
const LICENCE: &str = "sr:licence.firearms";
/// A legal installed augmentation.
const WARE: &str = "sr:ware.datajack";
/// A code carried by the writable test pack, so a test can bump its version.
const WIDGET: &str = "sr:test.widget";

// ---- rig ------------------------------------------------------------------

fn vocabulary() -> BTreeSet<CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| CapabilityId::new(c).expect("well-formed"))
        .collect()
}

/// A per-test directory `[shadowrun.economy].catalog_paths` points at, so the pack
/// tests drive the **real** `@market/catalog load` path over a real file — which is
/// also what makes `catalog_paths` stop being inert (B9).
fn pack_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("e6c-packs-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a pack directory");
    dir
}

/// Write one pack file into `dir`, and return the path it will be named by.
fn write_pack(dir: &std::path::Path, pack: &str, entries: &serde_json::Value) -> String {
    let path = dir.join(format!("{pack}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(entries).expect("json"))
        .expect("the pack writes");
    path.display().to_string()
}

/// One entry's JSON, as a pack file carries it (the loader stamps `pack`).
fn entry_json(code: &str, version: u32, rep: &str, baseline: i64) -> serde_json::Value {
    serde_json::json!({
        "catalog": { "ruleset": "sr", "code": code },
        "definition_version": version,
        "name": format!("Test {code}"),
        "baseline": baseline,
        "legality": "legal",
        "representation": rep,
        "stackable": true
    })
}

fn sr_config() -> SrEconomyConfig {
    SrEconomyConfig {
        enabled: true,
        // Composition data (DL-11): the org ids whose acceptance may be performed
        // programmatically. The fixture's orgs land in this range.
        vendor_orgs: (0..8).collect(),
        ..SrEconomyConfig::default()
    }
}

/// `modules.rs`'s own `vendor_caps`, so the rig freezes the table production does.
fn vendor_caps(cfg: &SrEconomyConfig) -> Vec<(PrincipalId, PartyCap)> {
    cfg.vendor_orgs
        .iter()
        .map(|id| (PrincipalId::Organization(OrgId(*id)), PartyCap::Accept))
        .collect()
}

fn market_issuer() -> mush_org::SystemPrincipalId {
    mush_org::SystemPrincipalId::new("sr.market").expect("well-formed")
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
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_faction::FactionModule::new()),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_config(econ)
                .with_observers(vec![
                    Arc::new(SrBoardObserver::new(cfg.clone())),
                    Arc::new(mush_market::MarketObserver),
                ])
                .with_title_issuers(vec![market_issuer()])
                .with_party_caps(vendor_caps(&cfg)),
        ),
        Box::new(mush_party::PartyModule::new()),
        // E6c: the market boots after the economy and before the ruleset, which
        // registers content, dice and delivery into its three frozen tables.
        Box::new(
            mush_market::MarketModule::new()
                .with_catalog_sources(vec![Arc::new(ruleset_shadowrun::SrCatalogSource::new(
                    &cfg,
                ))])
                .with_availability(vec![Arc::new(ruleset_shadowrun::SrAvailability::new())])
                .with_delivery(vec![Arc::new(ruleset_shadowrun::SrDelivery)])
                .with_issuer(market_issuer()),
        ),
        Box::new(mush_jobs::JobsModule::new()),
        Box::new(ruleset_shadowrun::ShadowrunRuleset::with_seed(7).economy(cfg)),
    ])
}

fn boot() -> TestWorld {
    let mut tw = boot_with(sr_config());
    ok(&mut tw, GOD, "@market/catalog load sr.core");
    tw
}

/// A world whose `catalog_paths` names `dir`.
fn boot_at(dir: &std::path::Path) -> TestWorld {
    let mut tw = boot_with(SrEconomyConfig {
        catalog_paths: vec![dir.display().to_string()],
        ..sr_config()
    });
    ok(&mut tw, GOD, "@market/catalog load sr.core");
    tw
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}
fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    matches_error(events)
}
fn matches_error(events: &[(Audience, OutputEvent)]) -> bool {
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
fn market(tw: &TestWorld) -> &MarketStore {
    tw.world()
        .module_state::<MarketStore>()
        .expect("market store")
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
fn venue() -> MarketId {
    MarketId::new(VENUE).expect("a venue")
}
fn catalog_of(code: &str) -> mush_economy::CatalogRef {
    let (r, c) = code.split_once(':').expect("ruleset:code");
    mush_economy::CatalogRef {
        ruleset: SmolStr::new(r),
        code: SmolStr::new(c),
    }
}

/// The projection of one good, as anybody sees it.
fn projection(tw: &TestWorld, code: &str) -> Projection {
    let snapshot = snap(tw).clone();
    let hidden: Vec<OrgId> = market(tw)
        .vendors()
        .map(|v| v.org)
        .filter(|o| mush_market::is_hidden_org(tw.world(), *o))
        .collect();
    let now = tw.world().now();
    market(tw).projection(&catalog_of(code), &venue(), &snapshot, &hidden, now)
}

/// A registered vendor: an organization with an unscoped nuyen account, a vault it
/// owns, and `funds` in the wallet.
struct Shop {
    org: OrgId,
    account: AccountId,
    vault: VaultId,
    id: VendorId,
}

fn vendor(tw: &mut TestWorld, name: &str, funds: i64) -> Shop {
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
        .expect("the shop wallet")
        .id;
    if funds > 0 {
        let f = faucet(tw, "sr.chargen");
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
        .expect("the shop vault")
        .id;
    ok(
        tw,
        GOD,
        &format!(
            "@market/vendor create #{}=#{} name={name} market={VENUE} buy=300",
            org.0, vault.0
        ),
    );
    let id = market(tw).vendor_of_org(org).expect("registered").id;
    Shop {
        org,
        account,
        vault,
        id,
    }
}

fn faucet(tw: &TestWorld, name: &str) -> AccountId {
    let owner = PrincipalId::System(mush_org::SystemPrincipalId::new(name).expect("well-formed"));
    snap(tw)
        .accounts()
        .find(|a| a.owner == owner)
        .expect("faucet account")
        .id
}

/// Publish one listing and return its id.
fn stock(
    tw: &mut TestWorld,
    shop: &Shop,
    code: &str,
    price: i64,
    qty: u64,
    extra: &str,
) -> ListingId {
    let before: BTreeSet<ListingId> = market(tw).live_listings().map(|l| l.id).collect();
    ok(
        tw,
        GOD,
        &format!(
            "@market/vendor stock #{}={code} price={price} qty={qty} {extra}",
            shop.id.0
        ),
    );
    market(tw)
        .live_listings()
        .map(|l| l.id)
        .find(|id| !before.contains(id))
        .expect("a new listing")
}

/// A world object nobody is carrying, for an asset listing to convey.
fn thing(tw: &mut TestWorld, name: &str) -> ObjId {
    tw.world_mut()
        .create(ObjKind::Thing, name, GOD, None)
        .expect("an object")
}

/// An **approved** character with a nuyen account and `funds` in it.
fn buyer(tw: &mut TestWorld, name: &str, funds: i64) -> ObjId {
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

fn account_of(tw: &TestWorld, who: ObjId) -> AccountId {
    sr(tw)
        .profile(who)
        .and_then(|p| p.account)
        .expect("a profiled account")
}

/// The only open quote a buyer holds.
fn only_quote(tw: &TestWorld, who: ObjId) -> QuoteId {
    let me = PrincipalId::Object(who);
    let ids: Vec<QuoteId> = market(tw).quotes_of_buyer(&me).map(|q| q.id).collect();
    assert_eq!(ids.len(), 1, "expected one quote, got {ids:?}");
    ids[0]
}

fn state(tw: &TestWorld, id: ContractId) -> ContractState {
    snap(tw).contract(id).expect("contract").state
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
                },
            )
        })
        .collect();
    tw.modules_loaded(boot);
}

fn patch(kv: &mut Kv, ns: &str, key: &str, f: impl Fn(&mut serde_json::Value)) {
    let entry = kv.get_mut(&SmolStr::new(ns)).expect("the namespace wrote");
    let mut seen = false;
    for (k, v) in entry.iter_mut() {
        if k.as_str() == key {
            f(v);
            seen = true;
        }
    }
    assert!(seen, "no such persisted key to patch: {ns}/{key}");
}

fn sweep_at(tw: &mut TestWorld, t: i64) -> Vec<(Audience, OutputEvent)> {
    tw.set_now(t);
    tw.fire_due(t.saturating_mul(1000))
}

/// Buy `qty` of `listing` as `who`, settle it, and return the contract.
fn buy(tw: &mut TestWorld, who: ObjId, listing: ListingId, qty: u64) -> ContractId {
    ok(tw, who, &format!("+market/quote #{}={qty}", listing.0));
    // The newest, not the only one: a settled quote **stays** as the trade-tape row
    // the projection folds, so a buyer accumulates rows as they shop.
    let q = market(tw)
        .quotes_of_buyer(&PrincipalId::Object(who))
        .filter(|q| q.listing == listing && q.settled.is_none())
        .map(|q| q.id)
        .max()
        .expect("the quote just issued");
    ok(tw, who, &format!("+market/accept #{}", q.0));
    let c = market(tw)
        .quote(q)
        .and_then(|q| q.contract)
        .expect("a contract");
    let now = tw.world().now();
    sweep_at(tw, now.saturating_add(60));
    c
}

// ===========================================================================
// The gate: the safe public projection (B1, §5.3, D-E6-14)
// ===========================================================================

/// **The gate, part 1.** Two worlds differing only in **hidden listings** publish
/// byte-identical projections: a hidden listing contributes nothing to band, depth,
/// volume or last trade — not even a rounding difference.
#[test]
fn e6c_b1_two_worlds_differing_only_in_hidden_listings_publish_the_same_projection() {
    let mut plain = boot();
    let shop = vendor(&mut plain, "Ammo", 0);
    stock(&mut plain, &shop, AMMO, 20, 50, "");
    let public = projection(&plain, AMMO);

    let mut with_hidden = boot();
    let shop = vendor(&mut with_hidden, "Ammo", 0);
    stock(&mut with_hidden, &shop, AMMO, 20, 50, "");
    stock(&mut with_hidden, &shop, AMMO, 999, 900, "hidden");
    let mixed = projection(&with_hidden, AMMO);

    assert_eq!(
        public, mixed,
        "a hidden listing moved the public projection"
    );
    assert!(
        market(&with_hidden).live_listings().count() == 2,
        "the hidden listing really is there"
    );
}

/// **The gate, part 2.** Two worlds differing only in a **hidden organization's**
/// listings publish byte-identical projections, and its listings are *absent* from
/// `+market/list` for an outsider rather than refused (G3).
#[test]
fn e6c_b1_a_hidden_vendor_orgs_listings_are_absent_from_the_projection_and_the_page() {
    let mut plain = boot();
    let shop = vendor(&mut plain, "Ammo", 0);
    stock(&mut plain, &shop, AMMO, 20, 50, "");
    let public = projection(&plain, AMMO);

    let mut with_hidden = boot();
    let shop = vendor(&mut with_hidden, "Ammo", 0);
    stock(&mut with_hidden, &shop, AMMO, 20, 50, "");
    let secret = vendor(&mut with_hidden, "Ghost", 0);
    stock(&mut with_hidden, &secret, AMMO, 5, 900, "");
    // B10's first write path for `Organization.hidden` — the reason E6a's D11
    // shipped preventively with no end-to-end test.
    ok(
        &mut with_hidden,
        GOD,
        &format!("@org/status #{}=active hidden=yes", secret.org.0),
    );
    let mixed = projection(&with_hidden, AMMO);
    assert_eq!(
        public, mixed,
        "a hidden organization's listings moved the public projection"
    );

    let outsider = buyer(&mut with_hidden, "Nobody", 100);
    let page = ok(&mut with_hidden, outsider, "+market/list");
    assert!(
        !page.contains("Ghost"),
        "a hidden vendor's listing was on an outsider's page: {page}"
    );
    assert!(
        page.contains("Ammo"),
        "the public one is still there: {page}"
    );
}

/// **The gate, part 3.** Two worlds differing only in **another buyer's open quotes**
/// publish byte-identical projections: every quote of every buyer contributes nothing.
#[test]
fn e6c_b1_another_buyers_open_quotes_move_no_public_byte() {
    let mut quiet = boot();
    let shop = vendor(&mut quiet, "Ammo", 0);
    let l = stock(&mut quiet, &shop, AMMO, 20, 50, "");
    let before = projection(&quiet, AMMO);

    let other = buyer(&mut quiet, "Rival", 10_000);
    ok(&mut quiet, other, &format!("+market/quote #{}=5", l.0));
    assert!(
        market(&quiet).quotes().count() == 1,
        "the rival's quote really exists"
    );
    assert_eq!(
        before,
        projection(&quiet, AMMO),
        "an open quote moved the public projection"
    );
}

/// **The gate, part 4.** `+market/depth` is byte-identical for a staffer, a member
/// and a stranger — because the projection function takes no viewer argument at all.
#[test]
fn e6c_b1_depth_is_byte_identical_for_a_staffer_and_a_stranger() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ammo", 0);
    stock(&mut tw, &shop, AMMO, 20, 50, "");
    stock(&mut tw, &shop, AMMO, 4_000, 900, "hidden");
    let stranger = buyer(&mut tw, "Stranger", 100);

    let staff_view = ok(&mut tw, GOD, &format!("+market/depth {AMMO}"));
    let stranger_view = ok(&mut tw, stranger, &format!("+market/depth {AMMO}"));
    assert_eq!(
        staff_view, stranger_view,
        "the public projection differed by viewer"
    );
    assert!(
        !staff_view.contains("4000"),
        "the hidden listing's price reached the public depth: {staff_view}"
    );
}

/// **The gate, part 5.** An unviewable listing renders `No such listing.` — the same
/// bytes a nonexistent one gets (08a decision 13).
#[test]
fn e6c_b1_an_unviewable_listing_and_a_nonexistent_one_read_alike() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ammo", 0);
    let hidden = stock(&mut tw, &shop, AMMO, 20, 50, "hidden");
    let stranger = buyer(&mut tw, "Stranger", 100);

    let unviewable = err(&mut tw, stranger, &format!("+market/show #{}", hidden.0));
    let nonexistent = err(&mut tw, stranger, "+market/show #99999");
    assert_eq!(unviewable, nonexistent);
    assert!(unviewable.contains("No such listing."), "{unviewable}");
}

/// **The gate, part 6.** A filter the actor may not use — or one this server does not
/// know — returns an **empty list**, never a refusal.
#[test]
fn e6c_b1_an_unusable_filter_returns_an_empty_list_and_not_a_refusal() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ammo", 0);
    stock(&mut tw, &shop, AMMO, 20, 50, "");
    let stranger = buyer(&mut tw, "Stranger", 100);

    let nothing = ok(&mut tw, stranger, "+market/list wardrobe=secret");
    assert!(
        nothing.contains("Nothing is for sale."),
        "an unknown filter should render an empty list: {nothing}"
    );
    let empty_class = ok(&mut tw, stranger, "+market/list legal=forbidden");
    assert!(
        empty_class.contains("Nothing is for sale."),
        "{empty_class}"
    );
    let real = ok(&mut tw, stranger, "+market/list legal=restricted");
    assert!(real.contains("Regular Ammunition"), "{real}");
}

/// **The gate, part 7.** A page is bounded on records **visited** and never exceeds
/// the row cap (D-LBK-5, 08i §9 answer 11).
#[test]
fn e6c_b1_the_page_is_bounded_on_records_visited_and_never_exceeds_the_row_cap() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ammo", 0);
    for _ in 0..40 {
        stock(&mut tw, &shop, AMMO, 20, 5, "hidden");
    }
    let stranger = buyer(&mut tw, "Stranger", 100);
    let page = mush_market::visible(tw.world(), stranger, None, mush_market::MARKET_PAGE, None);
    assert!(
        page.visited <= mush_market::MARKET_PAGE_VISIT,
        "the walk visited {} records",
        page.visited
    );
    assert!(page.ids.len() <= mush_market::MARKET_PAGE_MAX);
    assert!(
        page.ids.is_empty(),
        "a stranger matched a hidden listing: {:?}",
        page.ids
    );
    assert!(
        page.visited >= 40,
        "filtering happened before paging, so every record was visited: {}",
        page.visited
    );
    // The staffer's page walks exactly the same records — the cursor is the last
    // record VISITED, so a narrow viewer and a wide one page in step.
    let staff = mush_market::visible(tw.world(), GOD, None, mush_market::MARKET_PAGE, None);
    assert_eq!(staff.ids.len(), mush_market::MARKET_PAGE);
    assert!(staff.next.is_some(), "the page names its own cursor");
}

/// The whole disclosure axis in one: a hidden vendor's **existence** is unreachable
/// from every public surface, and a rival's purchase changes no byte a third party
/// can read until it settles publicly.
#[test]
fn e6c_b1_a_hidden_vendors_existence_is_unreachable_from_every_public_surface() {
    let mut tw = boot();
    let open = vendor(&mut tw, "Ammo", 0);
    stock(&mut tw, &open, AMMO, 20, 50, "");
    let secret = vendor(&mut tw, "Ghost", 0);
    let secret_listing = stock(&mut tw, &secret, AMMO, 5, 900, "");
    ok(
        &mut tw,
        GOD,
        &format!("@org/status #{}=active hidden=yes", secret.org.0),
    );
    let stranger = buyer(&mut tw, "Stranger", 10_000);

    let vendors = ok(&mut tw, stranger, "+market/vendors");
    assert!(!vendors.contains("Ghost"), "{vendors}");
    let list = ok(&mut tw, stranger, "+market/list");
    assert!(!list.contains("Ghost"), "{list}");
    let depth = ok(&mut tw, stranger, &format!("+market/depth {AMMO}"));
    assert!(
        depth.contains("band: 20..20"),
        "the public band is the public listing's alone: {depth}"
    );
    assert!(
        !depth.contains("900"),
        "the hidden vendor's quantity reached depth: {depth}"
    );
    let show = err(
        &mut tw,
        stranger,
        &format!("+market/show #{}", secret_listing.0),
    );
    assert_eq!(show, err(&mut tw, stranger, "+market/show #99999"));
    // The refusal for a hidden vendor by NAME reads the same as an unknown one, so
    // `+market/sell` is not a vendor-existence oracle either.
    let by_name = err(&mut tw, stranger, "+market/sell me=Ghost");
    let unknown = err(&mut tw, stranger, "+market/sell me=Nowhere");
    assert_eq!(by_name, unknown);
}

// ===========================================================================
// The catalog (B3, B9)
// ===========================================================================

/// A pack loads, **versions** and reloads idempotently, and `catalog_paths` is the
/// route it comes in by — the config knob E6a shipped parsed and inert.
#[test]
fn e6c_b3_a_pack_loads_versions_and_reloads_idempotently() {
    let dir = pack_dir("versions");
    write_pack(
        &dir,
        "mkt.test",
        &serde_json::json!([entry_json("test.widget", 1, "instrument", 100)]),
    );
    let mut tw = boot_at(&dir);
    let first = ok(&mut tw, GOD, "@market/catalog load mkt.test");
    assert!(first.contains("1 new"), "{first}");
    let entry = market(&tw)
        .entry_of(&catalog_of("sr:test.widget"))
        .expect("the entry loaded")
        .clone();
    assert_eq!(entry.definition_version, 1);
    assert_eq!(entry.pack.as_str(), "mkt.test");

    let again = ok(&mut tw, GOD, "@market/catalog load mkt.test");
    assert!(
        again.contains("0 new, 0 versioned, 0 dropped"),
        "a reload was not idempotent: {again}"
    );

    // Versioning in place, plus a dropped entry: the pack is the authority on what
    // it holds.
    write_pack(
        &dir,
        "mkt.test",
        &serde_json::json!([entry_json("test.widget", 2, "instrument", 120)]),
    );
    let bumped = ok(&mut tw, GOD, "@market/catalog load mkt.test");
    assert!(bumped.contains("1 versioned"), "{bumped}");
    assert_eq!(
        market(&tw)
            .entry_of(&catalog_of("sr:test.widget"))
            .expect("still there")
            .definition_version,
        2
    );
    let sr_core = market(&tw).entries_of_pack("sr.core").count();
    assert!(sr_core > 0, "versioning one pack disturbed another");
}

/// A bad entry fails **the pack** and names the file; the server keeps running and
/// the catalogue is exactly as it was.
#[test]
fn e6c_b3_a_bad_entry_fails_the_pack_and_names_it_and_the_server_boots() {
    let dir = pack_dir("bad");
    let mut bad = entry_json("test.bad", 1, "asset", 10);
    bad["name"] = serde_json::json!("");
    let path = write_pack(
        &dir,
        "mkt.bad",
        &serde_json::json!([entry_json("test.good", 1, "instrument", 10), bad]),
    );
    let mut tw = boot_at(&dir);
    let before = market(&tw).entries().count();

    let refused = err(&mut tw, GOD, "@market/catalog load mkt.bad");
    assert!(
        refused.contains(&path),
        "the refusal names no file: {refused}"
    );
    assert!(refused.contains("entry 2"), "{refused}");
    assert_eq!(
        market(&tw).entries().count(),
        before,
        "a bad entry wrote part of its pack"
    );
    // The server keeps running, and a pack nothing defines is named rather than
    // guessed at.
    assert!(!ok(&mut tw, GOD, "@market/catalog list").is_empty());
    let missing = err(&mut tw, GOD, "@market/catalog load nowhere.pack");
    assert!(missing.contains("nowhere.pack"), "{missing}");
}

/// `MAX_CATALOG_ENTRIES_PER_PACK` refuses, before anything is written.
#[test]
fn e6c_b3_a_pack_past_the_per_pack_cap_is_refused_whole() {
    let dir = pack_dir("cap");
    let entries: Vec<serde_json::Value> = (0..=mush_market::MAX_CATALOG_ENTRIES_PER_PACK)
        .map(|i| entry_json(&format!("bulk.{i}"), 1, "instrument", 1))
        .collect();
    write_pack(&dir, "mkt.bulk", &serde_json::Value::Array(entries));
    let mut tw = boot_at(&dir);
    let before = market(&tw).entries().count();
    let refused = err(&mut tw, GOD, "@market/catalog load mkt.bulk");
    assert!(refused.contains("per pack"), "{refused}");
    assert_eq!(market(&tw).entries().count(), before);
}

/// **OPEN 1**: a `Lot`-represented entry is refused at listing creation. The starter
/// pack ships none, and nothing in this cut defines a `Lot` record.
#[test]
fn e6c_b3_a_lot_represented_entry_is_refused_at_listing_creation() {
    let dir = pack_dir("lot");
    write_pack(
        &dir,
        "mkt.lots",
        &serde_json::json!([entry_json("ore.raw", 1, "lot", 10)]),
    );
    let mut tw = boot_at(&dir);
    ok(&mut tw, GOD, "@market/catalog load mkt.lots");
    let shop = vendor(&mut tw, "Ammo", 0);
    let refused = err(
        &mut tw,
        GOD,
        &format!(
            "@market/vendor stock #{}=sr:ore.raw price=10 qty=4",
            shop.id.0
        ),
    );
    assert!(
        refused.contains("not stockable"),
        "a lot-represented listing was allowed: {refused}"
    );
    assert!(
        !market(&tw)
            .entries_of_pack("sr.core")
            .any(|e| e.representation == Representation::Lot),
        "OPEN 1: the starter pack ships no Lot entry"
    );
}

// ===========================================================================
// Vendors and the buyer-as-issuer purchase (B5, B6)
// ===========================================================================

/// The whole path: quote → offer (buyer as issuer) → buyer's own escrow → the vendor
/// accepting **programmatically with no officer connected** (DL-11) → settlement, on
/// a real E4b2 contract. `TitleTransfer` moves the title, and it needs both
/// acceptances (D-E3-12).
#[test]
fn e6c_b6_a_stock_purchase_settles_through_a_real_contract_and_moves_title() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let gun = thing(&mut tw, "a Colt America L36");
    let listing = stock(&mut tw, &shop, GUN, 400, 1, &format!("object=#{}", gun.0));
    // Stocking minted the title, **stamped with its catalogue reference** (B4a).
    let asset = snap(&tw).asset(gun).expect("a titled asset").clone();
    assert_eq!(asset.title_owner, PrincipalId::Organization(shop.org));
    assert_eq!(
        asset.catalog.as_ref().map(|c| c.code.as_str()),
        Some("colt.america"),
        "stocking did not stamp the catalogue reference"
    );

    let who = buyer(&mut tw, "Kaz", 1_000);
    let wallet = account_of(&tw, who);
    ok(&mut tw, who, &format!("+market/quote #{}", listing.0));
    let q = only_quote(&tw, who);
    ok(&mut tw, who, &format!("+market/accept #{}", q.0));
    let contract = market(&tw)
        .quote(q)
        .and_then(|q| q.contract)
        .expect("contract");
    let c = snap(&tw).contract(contract).expect("the contract").clone();
    assert_eq!(
        c.issuer,
        PrincipalId::Object(who),
        "the buyer is the issuer"
    );
    assert_eq!(c.acceptances.len(), 2, "both parties accepted");
    assert_eq!(held(&tw, wallet), 400, "the buyer's own escrow stands");

    let now = tw.world().now();
    sweep_at(&mut tw, now + 60);
    assert!(
        matches!(state(&tw, contract), ContractState::Settled { .. }),
        "the purchase did not settle: {:?}",
        state(&tw, contract)
    );
    assert_eq!(bal(&tw, wallet), 600);
    assert_eq!(bal(&tw, shop.account), 400);
    let after = snap(&tw).asset(gun).expect("the asset").clone();
    assert_eq!(
        after.title_owner,
        PrincipalId::Object(who),
        "title did not move to the buyer"
    );
    assert_eq!(
        after.catalog.as_ref().map(|c| c.code.as_str()),
        Some("colt.america"),
        "the catalogue reference survived the transfer"
    );
}

/// A **procurement** listing carries its lead time as a `Before` deadline (§5.4), and
/// settles through the same ordinary contract.
#[test]
fn e6c_b5_a_procurement_carries_its_lead_time_as_a_before_deadline() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let listing = stock(&mut tw, &shop, AMMO, 30, 10, "lead=3600");
    let l = market(&tw).listing(listing).expect("the listing").clone();
    assert!(
        matches!(l.source, mush_market::Supply::Procurement { .. }),
        "not a procurement: {:?}",
        l.source
    );
    let who = buyer(&mut tw, "Kaz", 1_000);
    ok(&mut tw, who, &format!("+market/quote #{}=2", listing.0));
    let q = only_quote(&tw, who);
    ok(&mut tw, who, &format!("+market/accept #{}", q.0));
    let contract = market(&tw)
        .quote(q)
        .and_then(|q| q.contract)
        .expect("contract");
    let c = snap(&tw).contract(contract).expect("contract").clone();
    assert!(
        c.consent
            .conditions
            .iter()
            .any(|cond| matches!(cond, mush_economy::TypedCondition::Before { .. })),
        "a procurement's lead time is not a Before deadline: {:?}",
        c.consent.conditions
    );
}

/// A **retired** vendor stops quoting, and its live contracts still settle (B10).
#[test]
fn e6c_b5_a_retired_vendor_stops_quoting_and_its_live_contracts_still_settle() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let listing = stock(&mut tw, &shop, AMMO, 30, 10, "");
    let who = buyer(&mut tw, "Kaz", 1_000);
    ok(&mut tw, who, &format!("+market/quote #{}=2", listing.0));
    let q = only_quote(&tw, who);
    ok(&mut tw, who, &format!("+market/accept #{}", q.0));
    let contract = market(&tw)
        .quote(q)
        .and_then(|q| q.contract)
        .expect("contract");

    ok(
        &mut tw,
        GOD,
        &format!("@market/vendor retire #{}", shop.id.0),
    );
    let refused = err(&mut tw, who, &format!("+market/quote #{}=1", listing.0));
    assert!(refused.contains("No such listing."), "{refused}");

    let now = tw.world().now();
    sweep_at(&mut tw, now + 60);
    assert!(
        matches!(state(&tw, contract), ContractState::Settled { .. }),
        "retiring a vendor tore up its live agreement: {:?}",
        state(&tw, contract)
    );
}

/// Each shipped representation produces the right artefact on purchase: an
/// `EconomicAsset` with title **and its `CatalogRef`**, a `Commodity` account line, an
/// `Entitlement` line, and an `InstalledAugmentation` — the last of which is the
/// first production caller of `SrProfile::install` (E6b's M11 handoff).
#[test]
fn e6c_b6_each_shipped_representation_produces_its_artefact_on_purchase() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let who = buyer(&mut tw, "Kaz", 100_000);

    // The two account-line representations need their goods instruments registered
    // and the vendor holding stock in them; the market mints neither. Stock is
    // capitalization: an attributed staff mint from a registered system source.
    for code in [AMMO, LICENCE] {
        let id = format!("sr.{}", code.trim_start_matches("sr:"));
        ok(&mut tw, GOD, &format!("@instrument/register {id}=0"));
        ok(&mut tw, GOD, &format!("@account/system sr.goods={id}"));
        ok(
            &mut tw,
            GOD,
            &format!("@account/open org:#{}={id}", shop.org.0),
        );
        ok(&mut tw, GOD, &format!("@account/open #{}={id}", who.0));
        let source = snap(&tw)
            .accounts()
            .find(|a| {
                matches!(&a.owner, PrincipalId::System(s) if s.as_str() == "sr.goods")
                    && a.instrument.as_str() == id
            })
            .expect("the goods faucet")
            .id;
        let vendor_line = snap(&tw)
            .accounts()
            .find(|a| a.owner == PrincipalId::Organization(shop.org) && a.instrument.as_str() == id)
            .expect("the vendor's goods account")
            .id;
        ok(
            &mut tw,
            GOD,
            &format!("@econ/mint {source}={vendor_line} 100"),
        );
    }

    let ammo_listing = stock(&mut tw, &shop, AMMO, 30, 10, "");
    buy(&mut tw, who, ammo_listing, 3);
    let ammo_line = snap(&tw)
        .accounts()
        .find(|a| a.owner == PrincipalId::Object(who) && a.instrument.as_str() == "sr.ammo.regular")
        .expect("the buyer's commodity account")
        .id;
    assert_eq!(
        bal(&tw, ammo_line),
        3,
        "the commodity line was not delivered"
    );

    let licence_listing = stock(&mut tw, &shop, LICENCE, 200, 1, "");
    buy(&mut tw, who, licence_listing, 1);
    let licence_line = snap(&tw)
        .accounts()
        .find(|a| {
            a.owner == PrincipalId::Object(who) && a.instrument.as_str() == "sr.licence.firearms"
        })
        .expect("the buyer's entitlement account")
        .id;
    assert_eq!(bal(&tw, licence_line), 1);

    let ware_listing = stock(&mut tw, &shop, WARE, 900, 1, "");
    buy(&mut tw, who, ware_listing, 1);
    let profile = sr(&tw).profile(who).expect("a profile").clone();
    assert_eq!(
        profile.augmentations.len(),
        1,
        "settlement wrote no augmentation record"
    );
    assert_eq!(profile.augmentations[0].catalog.as_str(), "ware.datajack");

    let gun = thing(&mut tw, "a Colt");
    let gun_listing = stock(&mut tw, &shop, GUN, 400, 1, &format!("object=#{}", gun.0));
    buy(&mut tw, who, gun_listing, 1);
    let asset = snap(&tw).asset(gun).expect("the asset").clone();
    assert_eq!(asset.title_owner, PrincipalId::Object(who));
    assert!(
        asset.catalog.is_some(),
        "the artefact carries its CatalogRef"
    );
}

// ===========================================================================
// Quotes: the deterministic refusal (B6, D-E6-13)
// ===========================================================================

/// A quote's expiry declares its clock, and each declared window is honoured.
#[test]
fn e6c_b6_a_quote_declares_its_clock_and_each_window_is_honoured() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let listing = stock(&mut tw, &shop, AMMO, 30, 10, "");
    let who = buyer(&mut tw, "Kaz", 1_000);
    let now = tw.world().now();

    ok(&mut tw, who, &format!("+market/quote #{}=1", listing.0));
    let live = only_quote(&tw, who);
    let q = market(&tw).quote(live).expect("the quote").clone();
    assert_eq!(q.expires.clock, mush_economy::EconomicClock::RealUtc);
    assert_eq!(q.expires.at, now + mush_market::QUOTE_TTL_LIVE);

    // The remote window, on the same declared clock.
    tw.set_now(now + 1);
    ok(
        &mut tw,
        who,
        &format!("+market/quote #{} remote=1", listing.0),
    );
    let remote = market(&tw)
        .quotes_of_buyer(&PrincipalId::Object(who))
        .map(|q| q.id)
        .max()
        .expect("the remote quote");
    let r = market(&tw).quote(remote).expect("the quote").clone();
    assert_eq!(r.expires.at, now + 1 + mush_market::QUOTE_TTL_REMOTE);
}

/// **Scenario 29**: a lapsed quote, a re-priced listing and a bumped catalogue
/// definition version all refuse with the **same deterministic text**, and move no
/// money. A buyer who watched and waited gets the answer one who never looked gets.
#[test]
fn e6c_b6_every_stale_reason_refuses_identically_and_charges_nothing() {
    let dir = pack_dir("stale");
    write_pack(
        &dir,
        "mkt.test",
        &serde_json::json!([entry_json("test.widget", 1, "instrument", 100)]),
    );
    let mut tw = boot_at(&dir);
    ok(&mut tw, GOD, "@market/catalog load mkt.test");
    let shop = vendor(&mut tw, "Ares", 0);
    let who = buyer(&mut tw, "Kaz", 10_000);
    let wallet = account_of(&tw, who);
    let opening = bal(&tw, wallet);

    // (a) lapsed
    let l1 = stock(&mut tw, &shop, AMMO, 30, 10, "");
    ok(&mut tw, who, &format!("+market/quote #{}=1", l1.0));
    let q1 = only_quote(&tw, who);
    let now = tw.world().now();
    tw.set_now(now + mush_market::QUOTE_TTL_LIVE + 1);
    let lapsed = err(&mut tw, who, &format!("+market/accept #{}", q1.0));

    // (b) the listing moved under it
    let l2 = stock(&mut tw, &shop, AMMO, 30, 10, "");
    ok(&mut tw, who, &format!("+market/quote #{}=1", l2.0));
    let q2 = market(&tw)
        .quotes_of_buyer(&PrincipalId::Object(who))
        .filter(|q| q.listing == l2)
        .map(|q| q.id)
        .max()
        .expect("a quote");
    // Somebody else buys one, which is the only thing that moves a listing's
    // revision — and it is exactly what "sold out from under it" looks like.
    let rival = buyer(&mut tw, "Rival", 10_000);
    buy(&mut tw, rival, l2, 9);
    let moved = err(&mut tw, who, &format!("+market/accept #{}", q2.0));

    // (c) the catalogue version bumped under it
    let l3 = stock(&mut tw, &shop, WIDGET, 30, 10, "");
    ok(&mut tw, who, &format!("+market/quote #{}=1", l3.0));
    let q3 = market(&tw)
        .quotes_of_buyer(&PrincipalId::Object(who))
        .filter(|q| q.listing == l3)
        .map(|q| q.id)
        .max()
        .expect("a quote");
    write_pack(
        &dir,
        "mkt.test",
        &serde_json::json!([entry_json("test.widget", 2, "instrument", 100)]),
    );
    ok(&mut tw, GOD, "@market/catalog load mkt.test");
    let versioned = err(&mut tw, who, &format!("+market/accept #{}", q3.0));

    assert_eq!(lapsed, moved, "a lapse and a re-price read differently");
    assert_eq!(
        moved, versioned,
        "a re-price and a re-version read differently"
    );
    assert!(lapsed.contains("no longer stands"), "{lapsed}");
    assert_eq!(
        bal(&tw, wallet),
        opening,
        "a refused quote moved the buyer's money"
    );
    assert_eq!(
        held(&tw, wallet),
        0,
        "a refused quote left an escrow standing"
    );
}

/// **OPEN 4**: a quote reserves nothing, so a quote against a listing that sold out
/// first refuses at accept — and **no oversell is reachable**: the listing's quantity
/// decrements only at activation.
#[test]
fn e6c_b6_a_quote_reserves_nothing_and_no_interleaving_oversells() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let listing = stock(&mut tw, &shop, AMMO, 30, 1, "");
    let first = buyer(&mut tw, "First", 10_000);
    let second = buyer(&mut tw, "Second", 10_000);

    // Both hold a quote on the last unit; neither reserved anything.
    ok(&mut tw, first, &format!("+market/quote #{}=1", listing.0));
    ok(&mut tw, second, &format!("+market/quote #{}=1", listing.0));
    let qa = only_quote(&tw, first);
    let qb = only_quote(&tw, second);
    assert_eq!(
        market(&tw).listing(listing).expect("still there").quantity,
        1,
        "a quote reserved stock"
    );

    ok(&mut tw, first, &format!("+market/accept #{}", qa.0));
    assert!(
        market(&tw).listing(listing).is_none(),
        "the last unit did not leave the listing at activation"
    );
    let loser = err(&mut tw, second, &format!("+market/accept #{}", qb.0));
    assert!(
        loser.contains("No such quote.") || loser.contains("no longer stands"),
        "{loser}"
    );
    assert_eq!(
        held(&tw, account_of(&tw, second)),
        0,
        "the loser was charged"
    );
}

/// An availability test is **recorded on the quote** and binding on re-read; a
/// failure refuses **without charging**.
#[test]
fn e6c_b7_an_availability_test_is_recorded_and_a_failure_charges_nothing() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let listing = stock(&mut tw, &shop, APDS, 200, 10, "");
    // A buyer with no sheet skill rolls no dice at all — fail closed.
    let nobody = buyer(&mut tw, "Nobody", 10_000);
    let wallet = account_of(&tw, nobody);
    let opening = bal(&tw, wallet);
    let refused = err(&mut tw, nobody, &format!("+market/quote #{}=1", listing.0));
    assert!(refused.contains("lay hands on"), "{refused}");
    assert!(refused.contains("Nothing was charged."), "{refused}");
    assert_eq!(bal(&tw, wallet), opening);
    assert_eq!(
        market(&tw)
            .quotes_of_buyer(&PrincipalId::Object(nobody))
            .count(),
        0,
        "a failed availability test stored a quote"
    );

    // A buyer whose sheet carries the skill can pass, and the outcome is stored.
    let dealer = buyer(&mut tw, "Dealer", 10_000);
    let store = ruleset_shadowrun::ShadowrunRuleset::new().store();
    let mut doc = store.get_or_default(tw.world(), dealer);
    doc.set_number("charisma", 6);
    store
        .set_map_entry(
            &mut doc,
            ruleset_shadowrun::schema::ACTIVE_SKILLS_FIELD,
            "negotiation",
            serde_json::json!(6),
        )
        .expect("the skill map takes a rating");
    store
        .put_by(tw.world_mut(), dealer, &doc, GOD)
        .expect("the sheet writes");
    let mut recorded = None;
    for _ in 0..12 {
        let ev = tw.run(dealer, &format!("+market/quote #{}=1", listing.0));
        if !has_error(&ev) {
            recorded = market(&tw)
                .quotes_of_buyer(&PrincipalId::Object(dealer))
                .next()
                .and_then(|q| q.availability.clone());
            break;
        }
        let now = tw.world().now();
        tw.set_now(now + 1);
    }
    let outcome = recorded.expect("a passing availability test inside twelve tries");
    assert!(outcome.passed);
    assert!(outcome.threshold > 0);
    assert!(
        !outcome.note.is_empty(),
        "the outcome records its arithmetic"
    );
}

// ===========================================================================
// NPC market makers (B8, doc 10 7A, D-E6-15)
// ===========================================================================

/// A registered maker; it is an **`Organization`**, never a `System` principal.
struct Maker {
    id: mush_market::MarketSourceId,
    cash: AccountId,
    org: OrgId,
}

fn maker(tw: &mut TestWorld, shop: &Shop, volume: i64, loss: i64, funds: i64) -> Maker {
    if funds > 0 {
        let f = faucet(tw, "sr.chargen");
        ok(
            tw,
            GOD,
            &format!("@econ/mint {f}={} {funds}", shop.account.0),
        );
    }
    ok(
        tw,
        GOD,
        &format!(
            "@market/source create #{}=#{} market={VENUE} spread=0 volume={volume} loss={loss} \
             window=86400",
            shop.org.0, shop.vault.0
        ),
    );
    let id = market(tw).sources().map(|s| s.id).max().expect("a maker");
    Maker {
        id,
        cash: shop.account,
        org: shop.org,
    }
}

/// A maker stops quoting at **each** of its budgets, one at a time, and every budget
/// is asked **before the quote is issued** rather than at accept.
#[test]
fn e6c_b8_a_maker_stops_quoting_at_each_of_its_budgets() {
    // volume
    let mut tw = boot();
    let shop = vendor(&mut tw, "Maker", 0);
    let m = maker(&mut tw, &shop, 50, 1_000_000, 10_000);
    let listing = stock(&mut tw, &shop, AMMO, 30, 100, &format!("source={}", m.id.0));
    let who = buyer(&mut tw, "Kaz", 100_000);
    let over = err(&mut tw, who, &format!("+market/quote #{}=5", listing.0));
    assert!(over.contains("run out of volume"), "{over}");
    assert_eq!(
        market(&tw).quotes().count(),
        0,
        "the budget was asked after the quote was issued"
    );
    ok(&mut tw, who, &format!("+market/quote #{}=1", listing.0));

    // loss — selling below the catalogue baseline books the difference
    let mut tw = boot();
    let shop = vendor(&mut tw, "Maker", 0);
    let m = maker(&mut tw, &shop, 1_000_000, 5, 10_000);
    let listing = stock(&mut tw, &shop, AMMO, 1, 100, &format!("source={}", m.id.0));
    let who = buyer(&mut tw, "Kaz", 100_000);
    let over = err(&mut tw, who, &format!("+market/quote #{}=3", listing.0));
    assert!(over.contains("run out of loss"), "{over}");

    // cash — a maker with an empty wallet is out of the fourth budget
    let mut tw = boot();
    let shop = vendor(&mut tw, "Maker", 0);
    let m = maker(&mut tw, &shop, 1_000_000, 1_000_000, 0);
    let listing = stock(&mut tw, &shop, AMMO, 30, 100, &format!("source={}", m.id.0));
    let who = buyer(&mut tw, "Kaz", 100_000);
    let broke = err(&mut tw, who, &format!("+market/quote #{}=1", listing.0));
    assert!(broke.contains("run out of cash"), "{broke}");

    // halted — and an outstanding quote made before the halt settles nothing
    let mut tw = boot();
    let shop = vendor(&mut tw, "Maker", 0);
    let m = maker(&mut tw, &shop, 1_000_000, 1_000_000, 10_000);
    let listing = stock(&mut tw, &shop, AMMO, 30, 100, &format!("source={}", m.id.0));
    let who = buyer(&mut tw, "Kaz", 100_000);
    ok(&mut tw, who, &format!("+market/quote #{}=1", listing.0));
    let q = only_quote(&tw, who);
    ok(&mut tw, GOD, &format!("@market/source halt #{}", m.id.0));
    let halted_quote = err(&mut tw, who, &format!("+market/quote #{}=1", listing.0));
    assert!(halted_quote.contains("is not quoting"), "{halted_quote}");
    let settles_nothing = err(&mut tw, who, &format!("+market/accept #{}", q.0));
    assert!(
        settles_nothing.contains("no longer stands"),
        "{settles_nothing}"
    );
    assert_eq!(held(&tw, account_of(&tw, who)), 0);
    assert_eq!(
        bal(&tw, m.cash),
        10_000,
        "a halted maker's quote moved money"
    );
}

/// **A maker mints nothing.** Every funding route is enumerated — world-start
/// capitalization, sales, and a registered faucet's drip under the `[economy]` mint
/// cap — and its balance after N sales and M drips is exactly their sum. A maker is
/// never a `System` party, and a drip past the cap is refused.
#[test]
fn e6c_b8_a_makers_balance_is_exactly_capitalization_plus_sales_plus_drips() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Maker", 0);
    let m = maker(&mut tw, &shop, 1_000_000, 1_000_000, 2_000);
    assert_eq!(bal(&tw, m.cash), 2_000, "capitalization");

    // A sale.
    let listing = stock(&mut tw, &shop, AMMO, 30, 10, &format!("source={}", m.id.0));
    let who = buyer(&mut tw, "Kaz", 10_000);
    buy(&mut tw, who, listing, 2);
    assert_eq!(bal(&tw, m.cash), 2_060, "capitalization + one sale");

    // A drip, under the mint cap.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@market/vendor fund #{}=100 faucet=sr.chargen period=60",
            shop.id.0
        ),
    );
    let now = tw.world().now();
    sweep_at(&mut tw, now + 200);
    let after_drip = bal(&tw, m.cash);
    assert!(
        after_drip >= 2_160,
        "the enumerated drip did not arrive: {after_drip}"
    );

    // A drip **above** the mint cap is refused, and nothing else can add to it: the
    // maker is an Organization and `@econ/mint` from an unregistered source is not a
    // route at all.
    let over_cap = mush_economy::DEFAULT_MINT_OBLIGATION_CAP_MAJOR + 1;
    let refused = err(
        &mut tw,
        GOD,
        &format!(
            "@market/vendor fund #{}={over_cap} faucet=sr.chargen period=60",
            shop.id.0
        ),
    );
    assert!(!refused.is_empty());
    assert_eq!(
        bal(&tw, m.cash),
        after_drip,
        "an over-cap drip moved money anyway"
    );
    assert!(
        matches!(
            snap(&tw).account(m.cash).expect("account").owner,
            PrincipalId::Organization(_)
        ),
        "a maker is never a System party (D-E6-4)"
    );
    let _ = m.org;
}

// ===========================================================================
// Restart (B2)
// ===========================================================================

/// Every record re-enters through its constructor on load; a corrupt listing, quote
/// or vendor document is **skipped loudly** and the world boots; and the projection
/// after a reboot equals the projection before it.
#[test]
fn e6c_b2_the_world_reboots_and_the_projection_is_the_same_on_the_other_side() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let listing = stock(&mut tw, &shop, AMMO, 30, 10, "");
    let who = buyer(&mut tw, "Kaz", 10_000);
    buy(&mut tw, who, listing, 2);
    let before = projection(&tw, AMMO);

    let kv = fold_all(&mut tw);
    reboot(&mut tw, &kv);
    assert!(
        market(&tw).indexes_agree(),
        "an index drifted across the reboot"
    );
    assert_eq!(
        before,
        projection(&tw, AMMO),
        "the projection changed across a reboot"
    );

    // A corrupt document of each family is skipped loudly and the world still boots.
    let mut broken = kv.clone();
    let key = format!("market:listing:{}", listing.0);
    patch(&mut broken, "market", &key, |v| {
        *v = serde_json::json!({ "id": 9_999, "nonsense": true });
    });
    reboot(&mut tw, &broken);
    assert!(
        market(&tw).listing(listing).is_none(),
        "a corrupt listing loaded anyway"
    );
    assert!(
        market(&tw).vendor(shop.id).is_some(),
        "one corrupt document took the rest of the store down"
    );
    assert!(market(&tw).indexes_agree());
}

/// A stored record that fails its **wire** is skipped: the caps are re-run on load,
/// not merely on write (the E5a fold rule).
#[test]
fn e6c_b2_a_document_past_its_caps_never_loads() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let listing = stock(&mut tw, &shop, AMMO, 30, 10, "");
    let mut kv = fold_all(&mut tw);
    patch(
        &mut kv,
        "market",
        &format!("market:listing:{}", listing.0),
        |v| {
            v["quantity"] = serde_json::json!(mush_market::MAX_QUANTITY + 1);
        },
    );
    reboot(&mut tw, &kv);
    assert!(
        market(&tw).listing(listing).is_none(),
        "a listing past MAX_QUANTITY loaded"
    );
    assert!(market(&tw).vendor(shop.id).is_some(), "the world booted");
}

// ===========================================================================
// B10 — the minimal org lifecycle (DL-06)
// ===========================================================================

/// **D11's missing end-to-end test.** A hidden organization reads exactly as an
/// unknown one for an outsider — **by name and by id alike** — now that
/// `@org/status … hidden=yes` is the first thing in the shipped surface that writes
/// `Organization.hidden`.
#[test]
fn e6c_b10_a_hidden_org_reads_as_an_unknown_one_by_name_and_by_id() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ghost", 0);
    ok(
        &mut tw,
        GOD,
        &format!("@org/status #{}=active hidden=yes", shop.org.0),
    );
    assert!(
        mush_market::is_hidden_org(tw.world(), shop.org),
        "the flag did not write"
    );
    let stranger = buyer(&mut tw, "Stranger", 100);
    let trinket = tw
        .world_mut()
        .create(ObjKind::Thing, "a trinket", GOD, Some(stranger))
        .expect("an object in their hands");

    // By id and by name, against an unknown id and an unknown name: four refusals,
    // two pairs, byte-identical inside each pair.
    let by_id = err(
        &mut tw,
        stranger,
        &format!("+market/sell #{}=#{}", trinket.0, shop.id.0),
    );
    let unknown_id = err(
        &mut tw,
        stranger,
        &format!("+market/sell #{}=#9999", trinket.0),
    );
    assert_eq!(by_id, unknown_id, "a hidden org answered differently by id");
    let by_name = err(
        &mut tw,
        stranger,
        &format!("+market/sell #{}=Ghost", trinket.0),
    );
    let unknown_name = err(
        &mut tw,
        stranger,
        &format!("+market/sell #{}=Nowhere", trinket.0),
    );
    assert_eq!(
        by_name, unknown_name,
        "a hidden org answered differently by name"
    );
    assert_eq!(by_id, by_name, "the two routes answered differently");
}

/// `@org/status` moves the lifecycle, bumps the revision, and **`Closed` is
/// terminal**: it never re-opens.
#[test]
fn e6c_b10_org_status_moves_the_lifecycle_bumps_the_revision_and_closed_is_terminal() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@org/create Renraku");
    let org = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().orgs().map(|o| o.id).max())
        .expect("the org");
    let rev = |tw: &TestWorld| {
        tw.world()
            .module_state::<mush_org::OrgStore>()
            .and_then(|s| s.state().org(org).map(|o| o.revision))
            .expect("the org")
    };
    let before = rev(&tw);
    ok(&mut tw, GOD, &format!("@org/status #{}=suspended", org.0));
    assert!(rev(&tw) > before, "the revision did not move");
    ok(&mut tw, GOD, &format!("@org/status #{}=closed", org.0));
    let refused = err(&mut tw, GOD, &format!("@org/status #{}=active", org.0));
    assert!(refused.contains("terminal"), "{refused}");
    // A player may not move it at all.
    let who = buyer(&mut tw, "Kaz", 10);
    err(&mut tw, who, &format!("@org/status #{}=active", org.0));
}

// ===========================================================================
// B9 — the catalogue baseline supersedes the fence table
// ===========================================================================

/// **B9**: `CatalogEntry.baseline` is the fence's basis now, and a market purchase is
/// therefore fenceable the moment it settles — which is the whole reason B4 exists.
/// The `[shadowrun.economy.fence_baseline]` table survives only as an **override**.
#[test]
fn e6c_b9_the_catalogue_baseline_is_the_fences_and_a_purchase_can_then_be_fenced() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let who = buyer(&mut tw, "Kaz", 10_000);
    let gun = tw
        .world_mut()
        .create(ObjKind::Thing, "a Colt America L36", GOD, Some(who))
        .expect("an object");
    let listing = stock(&mut tw, &shop, GUN, 400, 1, &format!("object=#{}", gun.0));
    buy(&mut tw, who, listing, 1);
    assert_eq!(
        snap(&tw).asset(gun).expect("the asset").title_owner,
        PrincipalId::Object(who)
    );

    // A fence, over a **different** organization, with money to pay with.
    let pawn = vendor(&mut tw, "Pawn", 10_000);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@sr/fence #{}=#{} name=Pawn demand=0",
            pawn.org.0, pawn.vault.0
        ),
    );
    // No `fence_baseline` entry exists for `colt.america`; the catalogue's own 425 is
    // what prices it, at the configured immediate rate of 300 permille.
    ok(&mut tw, who, &format!("+fence/quote #0=#{}", gun.0));
    let quoted = sr(&tw)
        .quotes()
        .next()
        .expect("a fence quote against a market-bought gun")
        .clone();
    assert_eq!(quoted.catalog.as_str(), "colt.america");
    assert_eq!(
        quoted.offer.minor(),
        425 * 300 / 1_000,
        "the fence did not price from the catalogue baseline"
    );
}

/// **The replay hazard.** A quote that has already been taken up, one that lapsed and
/// one whose listing sold out all refuse a second accept, and none of them moves a
/// minor unit.
#[test]
fn e6c_b6_a_replayed_accept_refuses_and_moves_no_money() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let listing = stock(&mut tw, &shop, AMMO, 30, 10, "");
    let who = buyer(&mut tw, "Kaz", 10_000);
    let wallet = account_of(&tw, who);

    ok(&mut tw, who, &format!("+market/quote #{}=2", listing.0));
    let q = only_quote(&tw, who);
    ok(&mut tw, who, &format!("+market/accept #{}", q.0));
    let after_first = bal(&tw, wallet);
    let held_after = held(&tw, wallet);

    let replay = err(&mut tw, who, &format!("+market/accept #{}", q.0));
    assert!(replay.contains("already been taken up"), "{replay}");
    assert_eq!(bal(&tw, wallet), after_first, "a replay moved money");
    assert_eq!(
        held(&tw, wallet),
        held_after,
        "a replay placed a second escrow"
    );

    let now = tw.world().now();
    sweep_at(&mut tw, now + 60);
    let after_settle = bal(&tw, wallet);
    let replay_settled = err(&mut tw, who, &format!("+market/accept #{}", q.0));
    assert!(
        replay_settled.contains("already been taken up")
            || replay_settled.contains("No such quote."),
        "{replay_settled}"
    );
    assert_eq!(
        bal(&tw, wallet),
        after_settle,
        "a post-settlement replay paid twice"
    );
    // The settled quote is the trade-tape row, and the projection counts it exactly
    // once however often the accept is replayed.
    assert_eq!(projection(&tw, AMMO).volume.minor(), 60);
}

/// A **cancelled** deal puts the goods back and forgets its quote, so a
/// quote/cancel loop mints no permanent documents.
#[test]
fn e6c_b6_a_cancelled_deal_returns_the_stock_and_forgets_its_quote() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let listing = stock(&mut tw, &shop, AMMO, 30, 10, "");
    let who = buyer(&mut tw, "Kaz", 10_000);
    let wallet = account_of(&tw, who);
    let opening = bal(&tw, wallet);

    ok(&mut tw, who, &format!("+market/quote #{}=4", listing.0));
    let q = only_quote(&tw, who);
    ok(&mut tw, who, &format!("+market/accept #{}", q.0));
    let contract = market(&tw)
        .quote(q)
        .and_then(|q| q.contract)
        .expect("contract");
    assert_eq!(
        market(&tw).listing(listing).expect("still there").quantity,
        6,
        "the stock did not leave at activation"
    );

    ok(&mut tw, GOD, &format!("@contract/cancel #{}", contract.0));
    assert_eq!(
        market(&tw).listing(listing).expect("still there").quantity,
        10,
        "the stock did not come back"
    );
    assert!(market(&tw).quote(q).is_none(), "the dead quote was kept");
    assert_eq!(bal(&tw, wallet), opening, "the escrow was not refunded");
    assert_eq!(held(&tw, wallet), 0);
}

/// **A budget that is checked and never spent is not a budget.** A maker's window is
/// **consumed by use**: two sales inside one window exhaust a volume budget that
/// either one alone would have passed, and a cancelled deal gives the budget back.
#[test]
fn e6c_b8_a_makers_window_is_consumed_by_use_and_returned_by_a_cancellation() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Maker", 0);
    // Room for two sales of 60 and not a third.
    let m = maker(&mut tw, &shop, 150, 1_000_000, 10_000);
    let listing = stock(&mut tw, &shop, AMMO, 30, 100, &format!("source={}", m.id.0));
    let who = buyer(&mut tw, "Kaz", 100_000);

    buy(&mut tw, who, listing, 2);
    assert_eq!(
        market(&tw)
            .source(m.id)
            .expect("the maker")
            .volume_used
            .minor(),
        60,
        "the first sale spent nothing"
    );
    buy(&mut tw, who, listing, 2);
    assert_eq!(
        market(&tw)
            .source(m.id)
            .expect("the maker")
            .volume_used
            .minor(),
        120
    );
    let over = err(&mut tw, who, &format!("+market/quote #{}=2", listing.0));
    assert!(
        over.contains("run out of volume"),
        "two sales did not exhaust the window: {over}"
    );

    // A cancelled deal gives it back — the budget follows the deal, not the intent.
    ok(&mut tw, who, &format!("+market/quote #{}=1", listing.0));
    let q = market(&tw)
        .quotes_of_buyer(&PrincipalId::Object(who))
        .filter(|q| q.settled.is_none())
        .map(|q| q.id)
        .max()
        .expect("an open quote");
    ok(&mut tw, who, &format!("+market/accept #{}", q.0));
    assert_eq!(
        market(&tw)
            .source(m.id)
            .expect("the maker")
            .volume_used
            .minor(),
        150
    );
    let contract = market(&tw)
        .quote(q)
        .and_then(|q| q.contract)
        .expect("a contract");
    ok(&mut tw, GOD, &format!("@contract/cancel #{}", contract.0));
    assert_eq!(
        market(&tw)
            .source(m.id)
            .expect("the maker")
            .volume_used
            .minor(),
        120,
        "a cancelled deal did not return the budget"
    );
    // …and never below zero, however many cancellations arrive.
    assert!(
        market(&tw)
            .source(m.id)
            .expect("the maker")
            .volume_used
            .minor()
            >= 0
    );
}

/// **The gate, part 8.** `+market/depth` on a good that is traded **only** in hidden
/// listings is byte-identical to `+market/depth` on a good nobody trades at all.
///
/// The venue defaulting is the subtle half: reading every listing to pick a venue
/// would have let a hidden listing choose it, and answering a *different sentence*
/// for the empty case would have made the reply's shape an existence oracle — a good
/// with only hidden listings would have rendered an empty projection where a good
/// with none rendered "nothing trades in that". Both render the empty projection.
#[test]
fn e6c_b1_a_good_traded_only_in_hidden_listings_reads_as_one_nobody_trades() {
    let mut tw = boot();
    let open = vendor(&mut tw, "Ammo", 0);
    stock(&mut tw, &open, AMMO, 20, 50, "");
    let stranger = buyer(&mut tw, "Stranger", 100);

    // A good nothing has ever been listed for.
    let untraded = ok(&mut tw, stranger, &format!("+market/depth {WARE}"));

    // The same good, listed once — hidden.
    let secret = vendor(&mut tw, "Ghost", 0);
    stock(&mut tw, &secret, WARE, 4_000, 3, "hidden");
    let hidden_only = ok(&mut tw, stranger, &format!("+market/depth {WARE}"));
    assert_eq!(
        untraded, hidden_only,
        "a hidden-only good reads differently from an untraded one"
    );

    // …and again with the *organization* hidden rather than the listing.
    let mut tw = boot();
    let open = vendor(&mut tw, "Ammo", 0);
    stock(&mut tw, &open, AMMO, 20, 50, "");
    let stranger = buyer(&mut tw, "Stranger", 100);
    let untraded = ok(&mut tw, stranger, &format!("+market/depth {WARE}"));
    let secret = vendor(&mut tw, "Ghost", 0);
    stock(&mut tw, &secret, WARE, 4_000, 3, "");
    ok(
        &mut tw,
        GOD,
        &format!("@org/status #{}=active hidden=yes", secret.org.0),
    );
    let hidden_org_only = ok(&mut tw, stranger, &format!("+market/depth {WARE}"));
    assert_eq!(
        untraded, hidden_org_only,
        "a hidden organization's listing chose the venue, or changed the shape of the reply"
    );
    // A staffer sees exactly the same thing, because there is no privileged
    // projection to see anything else through.
    assert_eq!(
        hidden_org_only,
        ok(&mut tw, GOD, &format!("+market/depth {WARE}"))
    );
}
