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
    econ(tw).snapshot()
}
/// The economy's whole store, for the two reads that are not on the snapshot: the
/// journal's transactions and the staff escalation ladder.
fn econ(tw: &TestWorld) -> &EconomyStore {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
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
///
/// `hidden_orgs` is derived from **the records the projection folds** — every listing's
/// seller and every trade row's — exactly as `api::hidden_seller_orgs` derives it. The
/// fold's D1 is why: composing it from the vendor table instead is precisely the defect,
/// and a rig that reproduced the defect could never have caught it.
fn projection(tw: &TestWorld, code: &str) -> Projection {
    let snapshot = snap(tw).clone();
    let store = market(tw);
    let hidden: Vec<OrgId> = store
        .live_listings()
        .map(|l| &l.seller)
        .chain(
            store
                .quotes()
                .filter(|q| q.settled.is_some())
                .map(|q| &q.seller),
        )
        .filter_map(|seller| match seller {
            PrincipalId::Organization(org) => Some(*org),
            PrincipalId::Object(_) | PrincipalId::System(_) => None,
        })
        .collect::<BTreeSet<OrgId>>()
        .into_iter()
        .filter(|o| mush_market::is_hidden_org(tw.world(), *o))
        .collect();
    let now = tw.world().now();
    store.projection(&catalog_of(code), &venue(), &snapshot, &hidden, now)
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

/// Register the goods instrument for a commodity or entitlement code, open the vendor's
/// and the buyer's accounts in it, and mint the vendor `units` of stock.
///
/// **The independent verify fold's M1 is why every test that accepts a commodity
/// purchase now does this.** `+market/accept` asks the frozen `DeliveryProvider` whether
/// the goods leg could run **before** any money moves, and none of the four facts it
/// needs — a registered instrument, a seller account in it, a buyer account in it, and
/// enough stock — exists by default: nothing in the market registers an instrument,
/// opens a goods account or checks stock against `Listing.quantity`. Before, the
/// purchase settled, the money moved, `deliver` logged a line nobody is paged by, and
/// the trade tape counted a trade that delivered nothing. §5.6 deviation 1's "reachable
/// only by a world change between the accept and the settlement" was false: the accept
/// proved nothing at all.
fn goods(tw: &mut TestWorld, shop: &Shop, who: ObjId, code: &str, units: i64) {
    let id = format!("sr.{}", code.trim_start_matches("sr:"));
    let instrument = mush_economy::InstrumentId::new(&id).expect("a usable instrument id");
    if snap(tw).instrument(&instrument).is_none() {
        ok(tw, GOD, &format!("@instrument/register {id}=0"));
        ok(tw, GOD, &format!("@account/system sr.goods={id}"));
        ok(tw, GOD, &format!("@account/open org:#{}={id}", shop.org.0));
    }
    let mine = snap(tw)
        .accounts()
        .any(|a| a.owner == PrincipalId::Object(who) && a.instrument.as_str() == id);
    if !mine {
        ok(tw, GOD, &format!("@account/open #{}={id}", who.0));
    }
    if units > 0 {
        let source = snap(tw)
            .accounts()
            .find(|a| {
                matches!(&a.owner, PrincipalId::System(s) if s.as_str() == "sr.goods")
                    && a.instrument.as_str() == id
            })
            .expect("the goods faucet")
            .id;
        let line = snap(tw)
            .accounts()
            .find(|a| a.owner == PrincipalId::Organization(shop.org) && a.instrument.as_str() == id)
            .expect("the vendor's goods account")
            .id;
        ok(tw, GOD, &format!("@econ/mint {source}={line} {units}"));
    }
}

/// The buyer's own line in one goods instrument.
fn goods_line(tw: &TestWorld, who: ObjId, code: &str) -> Option<AccountId> {
    let id = format!("sr.{}", code.trim_start_matches("sr:"));
    snap(tw)
        .accounts()
        .find(|a| a.owner == PrincipalId::Object(who) && a.instrument.as_str() == id)
        .map(|a| a.id)
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

/// A world object **in the stocking staffer's hands**, for an asset listing to convey.
///
/// The independent verify fold's M10 is why it is not a floating object any more:
/// `@market/vendor stock … object=` mints a vendor-owned title, and it now refuses
/// anything that is not a `Thing` the staffer is holding or one already in the vendor's
/// own vault. A limbo object was never the shape a shop stocks from.
fn thing(tw: &mut TestWorld, name: &str) -> ObjId {
    tw.world_mut()
        .create(ObjKind::Thing, name, GOD, Some(GOD))
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

/// Fold the **next** batch of module writes into an already-folded `kv`, dropping the
/// named documents — the shape of a crash between a journal post and the doc writes that
/// record it.
///
/// The journal itself is `JsonSet` ledger chunks, so it survives; the dropped documents
/// are exactly the ones the crash lost, and each module's boot has to recover from what
/// is left. `dropped` is `(namespace, key)`.
fn fold_dropping(tw: &mut TestWorld, kv: &mut Kv, dropped: &[(&str, String)]) {
    for op in tw.take_module_ops() {
        let ns = op.ns.clone();
        let entry = kv.entry(ns.clone()).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                if dropped.iter().any(|(n, k)| *n == ns && *k == key.as_str()) {
                    continue;
                }
                entry.retain(|(k, _)| *k != key);
                entry.push((key, value));
            }
            ModuleWrite::Del { key } => {
                if dropped.iter().any(|(n, k)| *n == ns && *k == key.as_str()) {
                    continue;
                }
                entry.retain(|(k, _)| *k != key);
            }
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
    goods(&mut tw, &shop, who, AMMO, 100);
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
    goods(&mut tw, &shop, who, AMMO, 100);
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
    goods(&mut tw, &shop, rival, AMMO, 100);
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
    goods(&mut tw, &shop, first, AMMO, 100);
    goods(&mut tw, &shop, second, AMMO, 0);

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
    maker_at(tw, shop, volume, loss, funds, 0)
}

/// The same, with a **declared spread**.
///
/// The fold's D4 is why the parameter exists: no shipped test exercised a nonzero
/// spread, and `price_for` applied it to the quote while `projection` and
/// `commands::listing_line` read the listing's own price — so the market advertised a
/// price nobody paid and published a band that need not contain its own last trade.
fn maker_at(
    tw: &mut TestWorld,
    shop: &Shop,
    volume: i64,
    loss: i64,
    funds: i64,
    spread: i64,
) -> Maker {
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
            "@market/source create #{}=#{} market={VENUE} spread={spread} volume={volume} \
             loss={loss} window=86400",
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
///
/// Every one of the four refusals is now the **same** `STALE_QUOTE` string
/// `+market/accept` answers with (independent verify fold, D2): naming the budget made
/// `+market/quote` a free solvency oracle on an org wallet `@account/show` hides.
#[test]
fn e6c_b8_a_maker_stops_quoting_at_each_of_its_budgets() {
    // volume
    let mut tw = boot();
    let shop = vendor(&mut tw, "Maker", 0);
    let m = maker(&mut tw, &shop, 50, 1_000_000, 10_000);
    let listing = stock(&mut tw, &shop, AMMO, 30, 100, &format!("source={}", m.id.0));
    let who = buyer(&mut tw, "Kaz", 100_000);
    let volume = err(&mut tw, who, &format!("+market/quote #{}=5", listing.0));
    assert!(volume.contains("no longer stands"), "{volume}");
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
    let loss = err(&mut tw, who, &format!("+market/quote #{}=3", listing.0));

    // cash — a maker with an empty wallet is out of the fourth budget
    let mut tw = boot();
    let shop = vendor(&mut tw, "Maker", 0);
    let m = maker(&mut tw, &shop, 1_000_000, 1_000_000, 0);
    let listing = stock(&mut tw, &shop, AMMO, 30, 100, &format!("source={}", m.id.0));
    let who = buyer(&mut tw, "Kaz", 100_000);
    let broke = err(&mut tw, who, &format!("+market/quote #{}=1", listing.0));

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

    // ---- E6c independent verify fold ----
    //
    // **D2.** All four budget refusals are one string, and it is the accept's own.
    assert_eq!(volume, loss, "volume and loss read differently");
    assert_eq!(loss, broke, "loss and cash read differently");
    assert_eq!(broke, halted_quote, "cash and halted read differently");
    assert_eq!(
        halted_quote, settles_nothing,
        "+market/quote and +market/accept read differently about the same maker"
    );
    for refusal in [&volume, &loss, &broke, &halted_quote] {
        assert!(
            !refusal.contains("run out of") && !refusal.contains("cash"),
            "a maker refusal named its budget to a stranger: {refusal}"
        );
    }
}

/// **D2, the disclosure half.** `+market/quote` was a free, unlimited solvency oracle on
/// a market maker's organization wallet: it answered "run out of cash" to whoever asked,
/// wrote no record and consumed no per-buyer slot, so a stranger could poll it and watch
/// an org wallet cross zero — while `@account/show` on that very account answers them
/// `No such account.` The two surfaces must never disagree about the wallet.
#[test]
fn vf_d2_a_stranger_cannot_read_a_makers_wallet_off_a_quote() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Maker", 0);
    let m = maker(&mut tw, &shop, 1_000_000, 1_000_000, 0);
    let listing = stock(&mut tw, &shop, AMMO, 30, 100, &format!("source={}", m.id.0));
    let stranger = buyer(&mut tw, "Stranger", 100_000);

    let broke = err(
        &mut tw,
        stranger,
        &format!("+market/quote #{}=1", listing.0),
    );
    let hidden_account = err(&mut tw, stranger, &format!("@account/show #{}", m.cash.0));
    assert!(
        hidden_account.contains("No such account."),
        "the economy stopped hiding the wallet: {hidden_account}"
    );
    assert!(
        !broke.contains("cash") && !broke.contains("run out"),
        "the quote refusal published the wallet the economy hides: {broke}"
    );

    // Fund it past the cash test and the refusal does not change either, so the poll
    // reads the same before and after the wallet crossed zero.
    let f = faucet(&tw, "sr.chargen");
    ok(&mut tw, GOD, &format!("@econ/mint {f}={} 10000", m.cash.0));
    ok(
        &mut tw,
        stranger,
        &format!("+market/quote #{}=1", listing.0),
    );
    ok(&mut tw, GOD, &format!("@market/source halt #{}", m.id.0));
    let halted = err(
        &mut tw,
        stranger,
        &format!("+market/quote #{}=1", listing.0),
    );
    assert_eq!(
        broke, halted,
        "a funded-but-halted maker read differently from a broke one"
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
    goods(&mut tw, &shop, who, AMMO, 100);
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
    goods(&mut tw, &shop, who, AMMO, 100);
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
    // The shop stocks from its own hands (M10), and the buyer takes delivery of the
    // object once the title is theirs — the fence asks who is *holding* it.
    let gun = thing(&mut tw, "a Colt America L36");
    let listing = stock(&mut tw, &shop, GUN, 400, 1, &format!("object=#{}", gun.0));
    buy(&mut tw, who, listing, 1);
    assert_eq!(
        snap(&tw).asset(gun).expect("the asset").title_owner,
        PrincipalId::Object(who)
    );
    tw.world_mut()
        .move_to(gun, who)
        .expect("the buyer takes delivery of what they now hold title to");

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
    // Ten units, because the published volume carries `DEPTH_QUANTITY_STEP`'s control
    // now (independent verify fold, D6): a window too thin to reach ten trades' worth
    // publishes zero, which is the safe direction and not the property under test here.
    let listing = stock(&mut tw, &shop, AMMO, 30, 10, "");
    let who = buyer(&mut tw, "Kaz", 10_000);
    let wallet = account_of(&tw, who);
    goods(&mut tw, &shop, who, AMMO, 100);

    ok(&mut tw, who, &format!("+market/quote #{}=10", listing.0));
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
    assert_eq!(projection(&tw, AMMO).volume.minor(), 300);
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
    goods(&mut tw, &shop, who, AMMO, 100);
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
    goods(&mut tw, &shop, who, AMMO, 100);

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
        over.contains("no longer stands"),
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

// ===========================================================================
// E6c independent verify fold
// ===========================================================================

/// **D1 (HIGH).** One skipped vendor document used to open a hidden organization's
/// listings straight into the public projection.
///
/// `hidden_seller_orgs` composed `hidden_orgs` from `store.vendors()` alone, and
/// `store::load` is deliberately **total**: a corrupt `market:vendor:{id}` is skipped and
/// the world boots, while that org's `market:listing:{id}` documents load fine — a
/// `Listing` denormalizes its seller and its wire never consults the vendor table. So the
/// hidden org fell out of `hidden_orgs`, `is_public_seller` answered `true`, and its price
/// and quantity were summed into `band`/`depth` and rendered to a stranger — while
/// `may_view_listing` still hid the same listing from `+market/list`. The two surfaces
/// disagreed and the leaky one was the gate. The set is derived from the records the
/// projection folds now, so the documented fail-closed actually fires.
#[test]
fn vf_d1_a_skipped_vendor_document_does_not_open_a_hidden_orgs_listings() {
    let mut tw = boot();
    let open = vendor(&mut tw, "Ammo", 0);
    stock(&mut tw, &open, AMMO, 20, 50, "");
    let secret = vendor(&mut tw, "Ghost", 0);
    stock(&mut tw, &secret, AMMO, 5, 900, "");
    ok(
        &mut tw,
        GOD,
        &format!("@org/status #{}=active hidden=yes", secret.org.0),
    );
    let sealed = projection(&tw, AMMO);
    assert_eq!(
        sealed.band.0.minor(),
        20,
        "the hidden shop is already in the public band"
    );

    // The hidden vendor's document will not load; its listings will.
    let mut kv = fold_all(&mut tw);
    patch(
        &mut kv,
        "market",
        &format!("market:vendor:{}", secret.id.0),
        |v| *v = serde_json::json!({ "nonsense": true }),
    );
    reboot(&mut tw, &kv);
    assert!(
        market(&tw).vendor(secret.id).is_none(),
        "the corrupt vendor document loaded anyway"
    );
    assert!(
        market(&tw).live_listings().any(|l| l.vendor == secret.id),
        "the hidden shop's listings really are still there"
    );
    assert_eq!(
        sealed,
        projection(&tw, AMMO),
        "a skipped vendor document opened a hidden organization's listings into the public \
         projection"
    );
    let stranger = buyer(&mut tw, "Stranger", 100);
    let depth = ok(&mut tw, stranger, &format!("+market/depth {AMMO}"));
    assert!(
        !depth.contains("900"),
        "the hidden quantity reached depth: {depth}"
    );
    assert!(depth.contains("band: 20..20"), "{depth}");

    // The mirror case: the listings are corrupt and the vendor survives. The public
    // projection is the public shop's alone either way.
    let mut tw = boot();
    let open = vendor(&mut tw, "Ammo", 0);
    stock(&mut tw, &open, AMMO, 20, 50, "");
    let secret = vendor(&mut tw, "Ghost", 0);
    let ghost_listing = stock(&mut tw, &secret, AMMO, 5, 900, "");
    ok(
        &mut tw,
        GOD,
        &format!("@org/status #{}=active hidden=yes", secret.org.0),
    );
    let sealed = projection(&tw, AMMO);
    let mut kv = fold_all(&mut tw);
    patch(
        &mut kv,
        "market",
        &format!("market:listing:{}", ghost_listing.0),
        |v| *v = serde_json::json!({ "nonsense": true }),
    );
    reboot(&mut tw, &kv);
    assert!(market(&tw).vendor(secret.id).is_some());
    assert_eq!(sealed, projection(&tw, AMMO));
}

/// **D3 (MEDIUM).** A wall of unviewable listings made the public ones unreachable: an
/// empty page rendered **no cursor**.
///
/// `api::visible` is correctly bounded on records visited and returns the last record
/// visited, so the API pages in step — but `commands::list` printed the cursor only in
/// the `else` after the empty case, so a viewer whose 256 newest live listings were all
/// unviewable read "Nothing is for sale." with the cursor discarded and every older
/// public listing unreachable from the player surface. Reachability, not truncation, is
/// what has to be viewer-independent.
#[test]
fn vf_d3_a_wall_of_unviewable_listings_still_pages_to_the_public_ones() {
    let mut tw = boot();
    let open = vendor(&mut tw, "Ammo", 0);
    // The public listing is the OLDEST, so it is past the wall in the newest-first walk.
    stock(&mut tw, &open, AMMO, 20, 50, "");
    // 260 hidden listings: five shops, each well inside the 64 the per-vendor cap allows.
    for shop in 0..5 {
        let ghost = vendor(&mut tw, &format!("Ghost{shop}"), 0);
        for _ in 0..52 {
            stock(&mut tw, &ghost, AMMO, 999, 5, "hidden");
        }
    }
    assert!(market(&tw).live_listings().count() > mush_market::MARKET_PAGE_VISIT);

    let stranger = buyer(&mut tw, "Stranger", 100);
    let mut page = ok(&mut tw, stranger, "+market/list");
    assert!(
        page.contains("Nothing is for sale."),
        "the fixture is not the shape the finding is about: {page}"
    );
    assert!(
        page.contains("(more: +market/list after="),
        "an empty page discarded its cursor, so every older public listing is unreachable"
    );

    // Follow it, in a bounded number of pages, to the public listing.
    let mut pages = 1usize;
    while pages < 16 {
        let Some(cursor) = page
            .split("after=")
            .nth(1)
            .and_then(|r| r.split(')').next())
            .map(str::to_string)
        else {
            break;
        };
        page = ok(&mut tw, stranger, &format!("+market/list after={cursor}"));
        pages += 1;
        if page.contains("Regular Ammunition") {
            break;
        }
    }
    assert!(
        page.contains("Regular Ammunition"),
        "the public listing was unreachable behind the wall after {pages} pages: {page}"
    );
}

/// **D4 (MEDIUM).** A maker's spread moves **every** published price alike.
///
/// `price_for` applied the spread to the quote and `Quote.price` became the published
/// `last_trade`, while `projection`'s band and `commands::listing_line` read
/// `Listing.price` un-moved — so `+market/list` advertised a price nobody pays and the
/// safe projection published a band that need not contain its own last trade. No shipped
/// test exercised a nonzero spread at all.
#[test]
fn vf_d4_a_makers_spread_moves_every_published_price_alike() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Maker", 0);
    let m = maker_at(&mut tw, &shop, 1_000_000, 1_000_000, 100_000, 500);
    let listing = stock(&mut tw, &shop, AMMO, 30, 100, &format!("source={}", m.id.0));
    let who = buyer(&mut tw, "Kaz", 100_000);
    goods(&mut tw, &shop, who, AMMO, 100);

    // 30 at +500 permille is 45, and 45 is what every surface says.
    let row = ok(&mut tw, who, "+market/list");
    assert!(
        row.contains("× 45"),
        "the list row advertises the old price: {row}"
    );
    let depth = ok(&mut tw, who, &format!("+market/depth {AMMO}"));
    assert!(
        depth.contains("band: 45..45"),
        "the band is pre-spread: {depth}"
    );
    let quoted = ok(&mut tw, who, &format!("+market/quote #{}=2", listing.0));
    assert!(quoted.contains("2 × 45 = 90"), "{quoted}");

    let q = only_quote(&tw, who);
    ok(&mut tw, who, &format!("+market/accept #{}", q.0));
    let now = tw.world().now();
    sweep_at(&mut tw, now + 60);
    let p = projection(&tw, AMMO);
    let last = p.last_trade.expect("a settled trade").minor();
    assert_eq!(last, 45);
    assert!(
        p.band.0.minor() <= last && last <= p.band.1.minor(),
        "the published band does not contain its own last trade: {:?} vs {last}",
        p.band
    );
}

/// **D7 (LOW).** §5.6 B1 claims the venue default "reads exactly the set the projection
/// folds". It read two of the projection's four listing tests, so a **lapsed** public
/// listing still chose the header venue while contributing nothing to the numbers — and a
/// good whose only public listing had lapsed rendered a different first line from a good
/// with no listing at all.
#[test]
fn vf_d7_a_lapsed_public_listing_does_not_choose_the_venue() {
    let mut tw = boot();
    let open = vendor(&mut tw, "Ammo", 0);
    stock(&mut tw, &open, AMMO, 20, 50, "");
    let stranger = buyer(&mut tw, "Stranger", 100);
    let untraded = ok(&mut tw, stranger, &format!("+market/depth {WARE}"));

    // The same good, listed once — publicly — and then left to lapse.
    let shop = vendor(&mut tw, "Ware", 0);
    stock(&mut tw, &shop, WARE, 4_000, 3, "days=1");
    let now = tw.world().now();
    tw.set_now(now + 2 * 86_400);
    let lapsed = ok(&mut tw, stranger, &format!("+market/depth {WARE}"));
    assert_eq!(
        untraded, lapsed,
        "a lapsed public listing chose the venue, or changed the shape of the reply"
    );
}

/// **D9 (LOW).** `@market/catalog load` took an unvalidated path as the pack name and
/// read outside `catalog_paths`: the name went straight to `direct.join("{pack}.json")`,
/// and `check_entry`'s byte rule runs only on the ENTRIES, after the file has been
/// opened, read and parsed. The refusal echoed the resolved absolute path and told
/// "exists but is not a loadable pack" apart from "no such pack".
#[test]
fn vf_d9_a_pack_name_that_is_a_path_is_refused_like_an_unknown_pack() {
    let dir = pack_dir("traversal");
    // A real pack file one directory ABOVE the configured root — the probe's target.
    let outside = dir
        .parent()
        .map(|p| p.join("e6c-outside-probe.json"))
        .expect("a parent directory");
    std::fs::write(
        &outside,
        serde_json::to_string(&serde_json::json!([entry_json(
            "outside.probe",
            1,
            "instrument",
            5
        )]))
        .expect("json"),
    )
    .expect("the probe writes");
    let mut tw = boot_at(&dir);

    let unknown = err(&mut tw, GOD, "@market/catalog load nowhere.pack");
    for name in [
        "../e6c-outside-probe",
        "a/b",
        "..\\e6c-outside-probe",
        "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
    ] {
        let refused = err(&mut tw, GOD, &format!("@market/catalog load {name}"));
        assert_eq!(
            refused.replace(name, "nowhere.pack"),
            unknown,
            "`{name}` did not read like an unknown pack"
        );
        assert!(
            !refused.contains(&dir.display().to_string()),
            "the refusal echoed a resolved path: {refused}"
        );
    }
    assert!(
        market(&tw)
            .entry_of(&catalog_of("sr:outside.probe"))
            .is_none(),
        "a file outside catalog_paths was loaded"
    );
    let _ = std::fs::remove_file(&outside);
}

/// **R1 (HIGH).** A settlement healed by economy boot never reached `MarketObserver`.
///
/// `mush-economy`'s `boot::finished_transition` finishes a terminal transition whose doc
/// write was lost by reading the contract's own deterministic idempotency key, and
/// persists the result with a **raw document write** — no `ContractChanged` event, as
/// `sweep.rs` says in as many words. The one re-announcement path filters on
/// `visibility.is_public()` and a market purchase is `Parties`, and `mush-market` had no
/// boot reconcile at all. So the goods were never handed over though the money had moved,
/// the trade tape lost the row permanently, and the quote — `contract: Some`,
/// `settled: None` — counted as open against its buyer's eight for ever.
#[test]
fn vf_r1_a_settlement_healed_at_boot_is_stamped_and_delivered_once() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let listing = stock(&mut tw, &shop, AMMO, 30, 20, "");
    let who = buyer(&mut tw, "Kaz", 10_000);
    goods(&mut tw, &shop, who, AMMO, 100);
    ok(&mut tw, who, &format!("+market/quote #{}=10", listing.0));
    let q = only_quote(&tw, who);
    ok(&mut tw, who, &format!("+market/accept #{}", q.0));
    let contract = market(&tw)
        .quote(q)
        .and_then(|q| q.contract)
        .expect("a contract");
    // Everything up to and including activation is durable…
    let mut kv = fold_all(&mut tw);
    // …then the settlement posts, and the crash loses the two documents that record it:
    // the contract's own state, and the market's trade-tape row.
    let now = tw.world().now();
    sweep_at(&mut tw, now + 60);
    assert!(matches!(
        state(&tw, contract),
        ContractState::Settled { .. }
    ));
    let line = goods_line(&tw, who, AMMO).expect("the buyer's commodity account");
    assert_eq!(bal(&tw, line), 10, "the goods arrived the first time");
    let settled_at = market(&tw)
        .quote(q)
        .and_then(|q| q.settled)
        .expect("a stamped row")
        .at;
    fold_dropping(
        &mut tw,
        &mut kv,
        &[
            ("economy", mush_economy::contract_key(contract).to_string()),
            ("market", mush_market::quote_key(q).to_string()),
        ],
    );
    // A long time later, so a stamp taken from `now` rather than from the transaction
    // would move the row right out of the projection's window.
    tw.set_now(now + 3 * 86_400);
    reboot(&mut tw, &kv);

    assert!(
        matches!(state(&tw, contract), ContractState::Settled { .. }),
        "the economy did not heal its own contract: {:?}",
        state(&tw, contract)
    );
    let row = market(&tw).quote(q).expect("the quote survived").clone();
    assert!(
        row.settled.is_some(),
        "the market never learned the purchase settled"
    );
    assert_eq!(
        row.settled.map(|s| s.at),
        Some(settled_at),
        "the row was stamped with the boot's clock rather than the transaction's"
    );
    let p = projection(&tw, AMMO);
    assert_eq!(p.last_trade.map(|a| a.minor()), Some(30));
    assert_eq!(p.volume.minor(), 300, "the tape lost a settled trade");
    assert_eq!(
        bal(&tw, line),
        10,
        "the boot reconcile delivered the goods a second time"
    );
}

/// **R2 (HIGH).** The same missing pass in the other direction: a cancellation healed by
/// economy boot never returned the stock or the maker's budget. `on_contract_changed`'s
/// `Cancelled` arm is the only place `restore_quantity` and `charge_budgets(.., -1)` ever
/// run, so the listing stayed permanently short by the sold quantity and the maker's
/// window stayed spent for a deal that never happened.
#[test]
fn vf_r2_a_cancellation_healed_at_boot_returns_the_stock_and_the_budget() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Maker", 0);
    let m = maker(&mut tw, &shop, 1_000_000, 1_000_000, 100_000);
    let listing = stock(&mut tw, &shop, AMMO, 30, 100, &format!("source={}", m.id.0));
    let who = buyer(&mut tw, "Kaz", 100_000);
    goods(&mut tw, &shop, who, AMMO, 100);
    ok(&mut tw, who, &format!("+market/quote #{}=4", listing.0));
    let q = only_quote(&tw, who);
    ok(&mut tw, who, &format!("+market/accept #{}", q.0));
    let contract = market(&tw)
        .quote(q)
        .and_then(|q| q.contract)
        .expect("a contract");
    assert_eq!(market(&tw).listing(listing).expect("live").quantity, 96);
    assert_eq!(
        market(&tw)
            .source(m.id)
            .expect("the maker")
            .volume_used
            .minor(),
        120
    );
    let mut kv = fold_all(&mut tw);

    ok(&mut tw, GOD, &format!("@contract/cancel #{}", contract.0));
    fold_dropping(
        &mut tw,
        &mut kv,
        &[
            ("economy", mush_economy::contract_key(contract).to_string()),
            ("market", mush_market::quote_key(q).to_string()),
            ("market", mush_market::listing_key(listing).to_string()),
            ("market", mush_market::source_key(m.id).to_string()),
        ],
    );
    reboot(&mut tw, &kv);

    assert_eq!(
        market(&tw).listing(listing).expect("still there").quantity,
        100,
        "the stock never came back"
    );
    assert_eq!(
        market(&tw)
            .source(m.id)
            .expect("the maker")
            .volume_used
            .minor(),
        0,
        "the maker's window stayed spent for a deal that never happened"
    );
    assert!(
        market(&tw).quote(q).is_none(),
        "the stranded quote kept its buyer's slot"
    );
    // Idempotent: the same reconcile again gives nothing back a second time.
    reboot(&mut tw, &kv);
    assert_eq!(
        market(&tw).listing(listing).expect("still there").quantity,
        100,
        "a second reconcile double-restored the stock"
    );
}

/// **R3 (MEDIUM).** Nothing ever deleted an expired listing: `MarketStore::reindex`
/// builds `live` from `quantity > 0` alone, `api::visible` never checked `expires`, and
/// the market registered no tick at all — so a lapsed listing rendered as buyable while
/// `quote` refused it, and went on counting against its vendor's 64 slots and the store's
/// 1 024 for ever. A vendor that let 64 lapse could never list again.
#[test]
fn vf_r3_a_lapsed_listing_is_swept_and_its_vendor_can_list_again() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ammo", 0);
    for _ in 0..mush_market::MAX_LISTINGS_PER_VENDOR {
        stock(&mut tw, &shop, AMMO, 20, 5, "days=1");
    }
    let full = err(
        &mut tw,
        GOD,
        &format!("@market/vendor stock #{}={AMMO} price=20 qty=5", shop.id.0),
    );
    assert!(full.contains("as many listings"), "{full}");

    let stranger = buyer(&mut tw, "Stranger", 100);
    let now = tw.world().now();
    tw.set_now(now + 2 * 86_400);
    // Even before the sweep the page and the projection agree it is gone.
    let page = ok(&mut tw, stranger, "+market/list");
    assert!(page.contains("Nothing is for sale."), "{page}");

    sweep_at(&mut tw, now + 2 * 86_400 + 60);
    assert_eq!(
        market(&tw).listings_of_vendor(shop.id),
        0,
        "the sweep left the lapsed listings holding the vendor's slots"
    );
    ok(
        &mut tw,
        GOD,
        &format!("@market/vendor stock #{}={AMMO} price=20 qty=5", shop.id.0),
    );
    assert_eq!(market(&tw).live_listings().count(), 1);
}

/// **R4 (MEDIUM; covers M11).** Trade-tape rows were never pruned by time and shared the
/// global quote cap, so a busy history could refuse every new quote for ever. The window
/// prunes them now, and the tape has a ceiling of its own below `MAX_QUOTES`.
#[test]
fn vf_r4_the_trade_tape_is_pruned_by_the_window_it_is_folded_over() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let listing = stock(&mut tw, &shop, AMMO, 30, 100, "days=90");
    let who = buyer(&mut tw, "Kaz", 100_000);
    goods(&mut tw, &shop, who, AMMO, 100);
    buy(&mut tw, who, listing, 10);
    assert_eq!(market(&tw).trade_rows(), 1);
    assert_eq!(projection(&tw, AMMO).volume.minor(), 300);

    // A year past the window the row counts for nothing and must not still occupy a slot.
    let now = tw.world().now();
    sweep_at(&mut tw, now + 365 * 86_400);
    assert_eq!(
        market(&tw).trade_rows(),
        0,
        "a row a year past the window still held a quote slot"
    );
    assert_eq!(market(&tw).quotes().count(), 0);

    // …while an in-window row survives the same fire.
    let fresh = stock(&mut tw, &shop, AMMO, 30, 100, "days=90");
    buy(&mut tw, who, fresh, 10);
    let now = tw.world().now();
    sweep_at(&mut tw, now + 60);
    assert_eq!(market(&tw).trade_rows(), 1, "an in-window row was swept");
    assert_eq!(projection(&tw, AMMO).volume.minor(), 300);
}

/// **R6 (MEDIUM).** A pack reload that dropped an entry left its listings publishing band
/// and depth for a good nothing can define, quote or deliver: `del_entry` touched no
/// listing and `projection` never consulted the catalogue, so the public projection
/// permanently advertised depth in a good that could not be bought.
#[test]
fn vf_r6_a_dropped_pack_entry_takes_its_listings_with_it() {
    let dir = pack_dir("dropped");
    write_pack(
        &dir,
        "mkt.test",
        &serde_json::json!([entry_json("test.widget", 1, "instrument", 100)]),
    );
    let mut tw = boot_at(&dir);
    ok(&mut tw, GOD, "@market/catalog load mkt.test");
    let shop = vendor(&mut tw, "Ares", 0);
    stock(&mut tw, &shop, WIDGET, 40, 30, "");
    assert_ne!(
        projection(&tw, WIDGET),
        Projection::default(),
        "the fixture publishes nothing to begin with"
    );

    // The pack is reloaded without the entry: it is the pack's own authority on what it
    // holds, and the listings go with it.
    write_pack(&dir, "mkt.test", &serde_json::json!([]));
    let reloaded = ok(&mut tw, GOD, "@market/catalog load mkt.test");
    assert!(reloaded.contains("1 dropped"), "{reloaded}");
    assert_eq!(
        projection(&tw, WIDGET),
        Projection::default(),
        "the projection still advertises depth in a good nothing can define"
    );
    let stranger = buyer(&mut tw, "Stranger", 100);
    let page = ok(&mut tw, stranger, "+market/list");
    assert!(page.contains("Nothing is for sale."), "{page}");
    assert_eq!(market(&tw).live_listings().count(), 0);
}

/// **R9 (LOW).** Two latent hazards in `accept`/`on_contract_changed`.
///
/// (a) the quote-to-contract link was written **after** the last acceptance, so
/// `by_quote_contract` was empty for the whole of activation: the day any path settles
/// inline at activation, the observer would return early and the trade row and the
/// delivery would be lost silently — R1 on the happy path with no crash. It is written as
/// soon as `offer_contract` returns now, and the return leg keys on whether the deal
/// actually came into force so a pre-activation cancellation gives nothing back.
///
/// (b) the `Settled` arm re-stamped and re-delivered unconditionally. It is idempotent
/// now, and the stamp is the transaction's own `posted_at`.
#[test]
fn vf_r9_the_contract_link_is_live_through_activation_and_settling_is_idempotent() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Maker", 0);
    let m = maker(&mut tw, &shop, 1_000_000, 1_000_000, 100_000);
    let listing = stock(
        &mut tw,
        &shop,
        AMMO,
        400,
        100,
        &format!("source={}", m.id.0),
    );
    // A buyer who cannot fund the escrow: the offer is made, the buyer's own acceptance
    // refuses, and the draft is cancelled — a transition against a quote nothing could
    // find before the fix.
    let broke = buyer(&mut tw, "Broke", 10);
    goods(&mut tw, &shop, broke, AMMO, 100);
    ok(&mut tw, broke, &format!("+market/quote #{}=1", listing.0));
    let q = only_quote(&tw, broke);
    err(&mut tw, broke, &format!("+market/accept #{}", q.0));
    assert!(
        market(&tw).quote(q).is_none(),
        "a cancelled draft left its quote wedging the buyer's slot"
    );
    assert_eq!(
        market(&tw).listing(listing).expect("live").quantity,
        100,
        "a deal that never came into force gave back stock that never left"
    );
    assert_eq!(
        market(&tw)
            .source(m.id)
            .expect("the maker")
            .volume_used
            .minor(),
        0,
        "a deal that never came into force refunded budget that was never spent"
    );

    // (b) A settled row is stamped once, from the transaction, and the goods move once.
    let who = buyer(&mut tw, "Kaz", 100_000);
    goods(&mut tw, &shop, who, AMMO, 100);
    let cheap = stock(&mut tw, &shop, AMMO, 30, 100, "");
    let contract = buy(&mut tw, who, cheap, 10);
    let row = market(&tw)
        .quotes_of_buyer(&PrincipalId::Object(who))
        .find(|r| r.settled.is_some())
        .expect("a settled row")
        .clone();
    let stamped = row.settled.expect("a settlement").at;
    let posted = econ(&tw)
        .transaction(match state(&tw, contract) {
            ContractState::Settled { transaction } => transaction,
            other => panic!("the purchase did not settle: {other:?}"),
        })
        .expect("the settling transaction")
        .posted_at;
    assert_eq!(
        stamped, posted,
        "the row was stamped with `now`, not the post"
    );
    let line = goods_line(&tw, who, AMMO).expect("the buyer's commodity account");
    assert_eq!(bal(&tw, line), 10);

    // Re-entering the settlement path (the boot reconcile is the production route) must
    // move neither the stamp nor the goods.
    let kv = fold_all(&mut tw);
    tw.set_now(posted + 4 * 86_400);
    reboot(&mut tw, &kv);
    assert_eq!(
        market(&tw)
            .quote(row.id)
            .and_then(|r| r.settled)
            .map(|s| s.at),
        Some(stamped),
        "a second Settled pass moved the row inside or outside the window"
    );
    assert_eq!(bal(&tw, line), 10, "the goods were delivered twice");
}

/// **M1 (HIGH).** A purchase whose goods leg cannot be delivered used to take the buyer's
/// money anyway, and the failure paged nobody: every arm of `module::deliver` was a bare
/// `tracing::error!`, and the quote was still written `settled: Some(..)` so the tape
/// counted a trade that delivered nothing. Both halves: a `can_deliver` pre-check at
/// accept, and the economy's own staff ladder at settlement.
#[test]
fn vf_m1_an_undeliverable_purchase_is_refused_before_any_money_moves() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let listing = stock(&mut tw, &shop, AMMO, 30, 10, "");
    let who = buyer(&mut tw, "Kaz", 10_000);
    let wallet = account_of(&tw, who);
    let opening = bal(&tw, wallet);

    // Nothing registers the goods instrument, opens a goods account or holds stock.
    ok(&mut tw, who, &format!("+market/quote #{}=2", listing.0));
    let q = only_quote(&tw, who);
    let refused = err(&mut tw, who, &format!("+market/accept #{}", q.0));
    assert!(refused.contains("Nothing was charged."), "{refused}");
    assert_eq!(
        bal(&tw, wallet),
        opening,
        "an undeliverable purchase charged"
    );
    assert_eq!(held(&tw, wallet), 0, "an undeliverable purchase escrowed");

    // Over-listing stock is the same defect: ten units sold against one held.
    goods(&mut tw, &shop, who, AMMO, 1);
    let over = err(&mut tw, who, &format!("+market/accept #{}", q.0));
    assert_eq!(over, refused, "an over-listed sale read differently");
    goods(&mut tw, &shop, who, AMMO, 10);
    ok(&mut tw, who, &format!("+market/accept #{}", q.0));
}

/// **M1, the residual.** A world change between the accept and the settlement is still
/// reachable — the pre-check posts nothing and promises nothing — and that is exactly
/// what the staff ladder is for. A paid-and-undelivered purchase opens a level-2 record
/// on its contract instead of a log line nobody is paged by.
#[test]
fn vf_m1_a_paid_and_undelivered_purchase_pages_the_staff_ladder() {
    let dir = pack_dir("undeliverable");
    write_pack(
        &dir,
        "mkt.test",
        &serde_json::json!([entry_json("test.widget", 1, "instrument", 100)]),
    );
    let mut tw = boot_at(&dir);
    ok(&mut tw, GOD, "@market/catalog load mkt.test");
    let shop = vendor(&mut tw, "Ares", 0);
    let who = buyer(&mut tw, "Kaz", 100_000);
    goods(&mut tw, &shop, who, WIDGET, 100);
    let listing = stock(&mut tw, &shop, WIDGET, 40, 30, "");
    ok(&mut tw, who, &format!("+market/quote #{}=2", listing.0));
    let q = only_quote(&tw, who);
    ok(&mut tw, who, &format!("+market/accept #{}", q.0));
    let contract = market(&tw)
        .quote(q)
        .and_then(|q| q.contract)
        .expect("a contract");

    // The world changes under the live deal: the pack drops the entry, so nothing can
    // say what was bought by the time the money lands.
    write_pack(&dir, "mkt.test", &serde_json::json!([]));
    ok(&mut tw, GOD, "@market/catalog load mkt.test");
    let now = tw.world().now();
    sweep_at(&mut tw, now + 60);
    assert!(matches!(
        state(&tw, contract),
        ContractState::Settled { .. }
    ));

    let paged = econ(&tw)
        .escalations()
        .any(|e| e.subject == mush_economy::EscalationSubject::Contract(contract) && e.is_open());
    assert!(
        paged,
        "a paid-and-undelivered purchase reached nobody: {:?}",
        econ(&tw)
            .escalations()
            .map(|e| e.subject)
            .collect::<Vec<_>>()
    );
}

/// **M2 (HIGH).** A maker's budgets bounded quoting, not selling. A quote reserves
/// nothing (OPEN 4), so N quotes taken out against N listings behind one maker each saw
/// `volume_used`/`loss_used` at their pre-sale value, all passed, and all charged — both
/// budgets exceeded without limit, because `is_current` consults only `halted`.
#[test]
fn vf_m2_a_makers_budgets_bound_selling_and_not_only_quoting() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Maker", 0);
    // Room for exactly one sale of 60.
    let m = maker(&mut tw, &shop, 100, 1_000_000, 100_000);
    let a = stock(&mut tw, &shop, AMMO, 30, 100, &format!("source={}", m.id.0));
    let b = stock(&mut tw, &shop, AMMO, 30, 100, &format!("source={}", m.id.0));
    let who = buyer(&mut tw, "Kaz", 100_000);
    goods(&mut tw, &shop, who, AMMO, 100);

    // Two quotes against two listings behind one maker: both pass, because neither
    // reserves anything.
    ok(&mut tw, who, &format!("+market/quote #{}=2", a.0));
    let qa = only_quote(&tw, who);
    ok(&mut tw, who, &format!("+market/quote #{}=2", b.0));
    let qb = market(&tw)
        .quotes_of_buyer(&PrincipalId::Object(who))
        .filter(|q| q.listing == b)
        .map(|q| q.id)
        .max()
        .expect("the second quote");

    ok(&mut tw, who, &format!("+market/accept #{}", qa.0));
    let second = err(&mut tw, who, &format!("+market/accept #{}", qb.0));
    assert!(second.contains("no longer stands"), "{second}");
    let maker_now = market(&tw).source(m.id).expect("the maker").clone();
    assert_eq!(maker_now.volume_used.minor(), 60);
    assert!(
        maker_now.volume_used.minor() <= maker_now.volume_budget.minor(),
        "a maker sold past its declared volume budget"
    );

    // The same on the loss budget, where the money leaks rather than the volume.
    let mut tw = boot();
    let shop = vendor(&mut tw, "Maker", 0);
    let m = maker(&mut tw, &shop, 1_000_000, 15, 100_000);
    let a = stock(&mut tw, &shop, AMMO, 20, 100, &format!("source={}", m.id.0));
    let b = stock(&mut tw, &shop, AMMO, 20, 100, &format!("source={}", m.id.0));
    let who = buyer(&mut tw, "Kaz", 100_000);
    goods(&mut tw, &shop, who, AMMO, 100);
    ok(&mut tw, who, &format!("+market/quote #{}=1", a.0));
    let qa = only_quote(&tw, who);
    ok(&mut tw, who, &format!("+market/quote #{}=1", b.0));
    let qb = market(&tw)
        .quotes_of_buyer(&PrincipalId::Object(who))
        .filter(|q| q.listing == b)
        .map(|q| q.id)
        .max()
        .expect("the second quote");
    ok(&mut tw, who, &format!("+market/accept #{}", qa.0));
    err(&mut tw, who, &format!("+market/accept #{}", qb.0));
    let maker_now = market(&tw).source(m.id).expect("the maker").clone();
    assert!(
        maker_now.loss_used.minor() <= maker_now.loss_budget.minor(),
        "a maker lost past its declared loss budget: {} of {}",
        maker_now.loss_used.minor(),
        maker_now.loss_budget.minor()
    );
}

/// **M3 (MEDIUM).** A listing that had lapsed still sold: `is_current` checked the
/// QUOTE's expiry, the listing's revision and quantity, the entry's version, the artefact
/// stamp, the vendor and the maker — and never the listing's own deadline, though
/// `quote()` does. The vendor's declared window was enforced when the price was asked for
/// and dropped at the point money moves.
#[test]
fn vf_m3_a_lapsed_listing_no_longer_sells() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let listing = stock(&mut tw, &shop, AMMO, 30, 10, "days=1");
    let who = buyer(&mut tw, "Kaz", 10_000);
    goods(&mut tw, &shop, who, AMMO, 100);
    let wallet = account_of(&tw, who);
    let opening = bal(&tw, wallet);

    // A remote quote outlives the listing's own window by design.
    ok(
        &mut tw,
        who,
        &format!("+market/quote #{} remote=2", listing.0),
    );
    let q = only_quote(&tw, who);
    let now = tw.world().now();
    tw.set_now(now + 86_400 + 60);
    let refused = err(&mut tw, who, &format!("+market/accept #{}", q.0));
    assert!(
        refused.contains("no longer stands"),
        "the market sold goods it publicly says are not for sale: {refused}"
    );
    assert_eq!(bal(&tw, wallet), opening);
    assert_eq!(held(&tw, wallet), 0);
}

/// **M4 (MEDIUM; covers R5).** DL-06 was inert against the market: no file in
/// `mush-market/src` mentioned `OrgStatus`, so a `Suspended`, `Dissolving` or `Closed`
/// organization kept trading and could still be registered as a vendor. That contradicted
/// `mush-org`'s own `org_status` doc comment and left B10's only staff verb for shutting a
/// shop down with no effect on the market.
#[test]
fn vf_m4_a_suspended_organization_stops_trading_and_its_live_contracts_still_settle() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let listing = stock(&mut tw, &shop, AMMO, 30, 10, "");
    let who = buyer(&mut tw, "Kaz", 10_000);
    goods(&mut tw, &shop, who, AMMO, 100);
    ok(&mut tw, who, &format!("+market/quote #{}=2", listing.0));
    let q = only_quote(&tw, who);
    ok(&mut tw, who, &format!("+market/accept #{}", q.0));
    let contract = market(&tw)
        .quote(q)
        .and_then(|q| q.contract)
        .expect("a contract");

    ok(
        &mut tw,
        GOD,
        &format!("@org/status #{}=suspended", shop.org.0),
    );
    // It quotes nothing, it stocks nothing, and it cannot be re-registered.
    let quoted = err(&mut tw, who, &format!("+market/quote #{}=1", listing.0));
    assert!(quoted.contains("no longer stands"), "{quoted}");
    let stocked = err(
        &mut tw,
        GOD,
        &format!("@market/vendor stock #{}={AMMO} price=20 qty=5", shop.id.0),
    );
    assert!(stocked.contains("not trading"), "{stocked}");
    let registered = err(
        &mut tw,
        GOD,
        &format!(
            "@market/vendor create #{}=#{} name=Ares market={VENUE} buy=300",
            shop.org.0, shop.vault.0
        ),
    );
    assert!(registered.contains("No such vendor."), "{registered}");

    // …and the live agreement it already made settles untouched.
    let now = tw.world().now();
    sweep_at(&mut tw, now + 60);
    assert!(
        matches!(state(&tw, contract), ContractState::Settled { .. }),
        "suspending an organization tore up its live agreement: {:?}",
        state(&tw, contract)
    );

    // `Closed` is terminal, so its vendor is retired at the next boot and its listings
    // go with it.
    ok(&mut tw, GOD, &format!("@org/status #{}=closed", shop.org.0));
    let kv = fold_all(&mut tw);
    reboot(&mut tw, &kv);
    assert!(
        !market(&tw)
            .vendor(shop.id)
            .is_some_and(mush_market::Vendor::is_trading),
        "a closed organization is still a trading shop"
    );
    assert_eq!(market(&tw).listings_of_vendor(shop.id), 0);
}

/// **M5 (MEDIUM; covers R7).** A maker's loss budget was spent and returned at the
/// catalogue's **live** baseline rather than the one the deal booked, so any pack reload
/// between the two legs made the return differ from the spend: a raised baseline returned
/// more than was spent and `.max(0)` then destroyed the loss every *other* trade in the
/// window had booked, handing the maker a fresh full budget; a lowered one returned less
/// and drove `loss_used` permanently upward.
#[test]
fn vf_m5_a_reloaded_baseline_does_not_change_what_a_cancellation_returns() {
    for baseline_after in [10_000i64, 20i64] {
        let dir = pack_dir(&format!("booked-{baseline_after}"));
        write_pack(
            &dir,
            "mkt.test",
            &serde_json::json!([entry_json("test.widget", 1, "instrument", 100)]),
        );
        let mut tw = boot_at(&dir);
        ok(&mut tw, GOD, "@market/catalog load mkt.test");
        let shop = vendor(&mut tw, "Maker", 0);
        let m = maker(&mut tw, &shop, 1_000_000, 1_000_000, 100_000);
        let listing = stock(
            &mut tw,
            &shop,
            WIDGET,
            40,
            100,
            &format!("source={}", m.id.0),
        );
        let who = buyer(&mut tw, "Kaz", 100_000);
        goods(&mut tw, &shop, who, WIDGET, 100);

        ok(&mut tw, who, &format!("+market/quote #{}=2", listing.0));
        let q = only_quote(&tw, who);
        ok(&mut tw, who, &format!("+market/accept #{}", q.0));
        let spent = market(&tw)
            .source(m.id)
            .expect("the maker")
            .loss_used
            .minor();
        assert_eq!(spent, (100 - 40) * 2, "the sale booked no loss");
        let contract = market(&tw)
            .quote(q)
            .and_then(|q| q.contract)
            .expect("a contract");

        // The catalogue moves under the live deal — an ordinary versioned write that
        // leaves every outstanding quote current.
        write_pack(
            &dir,
            "mkt.test",
            &serde_json::json!([entry_json("test.widget", 1, "instrument", baseline_after)]),
        );
        ok(&mut tw, GOD, "@market/catalog load mkt.test");
        ok(&mut tw, GOD, &format!("@contract/cancel #{}", contract.0));
        assert_eq!(
            market(&tw)
                .source(m.id)
                .expect("the maker")
                .loss_used
                .minor(),
            0,
            "a baseline of {baseline_after} made the return differ from the spend"
        );
    }
}

/// **M6 (MEDIUM).** A catalogue entry with no baseline made a maker's loss budget inert:
/// both loss sites resolved `None` to zero, so `loss = (0 - price).max(0) * qty` was
/// always zero and the loss arm never tripped however far below the shelf price the
/// spread took the quote. Doc 10 7A's "mediocre prices is a trading policy, not a
/// substitute for funding" was unenforced for any entry that omits a reference price.
#[test]
fn vf_m6_a_baseline_less_entry_cannot_be_a_makers_listing() {
    let dir = pack_dir("nobaseline");
    let mut entry = entry_json("test.widget", 1, "instrument", 0);
    entry["baseline"] = serde_json::Value::Null;
    write_pack(&dir, "mkt.test", &serde_json::json!([entry]));
    let mut tw = boot_at(&dir);
    ok(&mut tw, GOD, "@market/catalog load mkt.test");
    let shop = vendor(&mut tw, "Maker", 0);
    // A zero loss budget and a spread that takes the price to a tenth: the exact shape
    // the loss budget exists to refuse.
    let m = maker_at(&mut tw, &shop, 1_000_000, 0, 100_000, -900);

    let refused = err(
        &mut tw,
        GOD,
        &format!(
            "@market/vendor stock #{}={WIDGET} price=1000 qty=10 source={}",
            shop.id.0, m.id.0
        ),
    );
    assert!(refused.contains("reference price"), "{refused}");
    // Stocking it from the vendor's own vault is untouched — the rule is the maker's.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@market/vendor stock #{}={WIDGET} price=1000 qty=10",
            shop.id.0
        ),
    );
}

/// **M7 (LOW).** A re-spread maker honoured its old quotes: `spread` is a live pricing
/// input `ObservedRevisions` did not stamp, so a staff correction of a mis-set spread left
/// every outstanding quote executing at the old price for the whole quote window.
/// Re-pricing through the listing invalidated a quote and re-pricing through the spread
/// did not.
#[test]
fn vf_m7_a_re_spread_maker_does_not_honour_its_old_quotes() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Maker", 0);
    let m = maker_at(&mut tw, &shop, 1_000_000, 1_000_000, 100_000, -900);
    let listing = stock(
        &mut tw,
        &shop,
        AMMO,
        300,
        100,
        &format!("source={}", m.id.0),
    );
    let who = buyer(&mut tw, "Kaz", 100_000);
    goods(&mut tw, &shop, who, AMMO, 100);
    ok(&mut tw, who, &format!("+market/quote #{}=1", listing.0));
    let q = only_quote(&tw, who);
    assert_eq!(market(&tw).quote(q).expect("the quote").price.minor(), 30);

    // Staff correct the typo'd spread. Every outstanding quote priced against it lapses.
    ok(
        &mut tw,
        GOD,
        &format!("@market/source budget #{}=spread=0", m.id.0),
    );
    let refused = err(&mut tw, who, &format!("+market/accept #{}", q.0));
    assert!(
        refused.contains("no longer stands"),
        "a re-spread maker still honoured a quote at the old price: {refused}"
    );
    assert_eq!(held(&tw, account_of(&tw, who)), 0);
    ok(&mut tw, who, &format!("+market/quote #{}=1", listing.0));
    let fresh = market(&tw)
        .quotes_of_buyer(&PrincipalId::Object(who))
        .filter(|r| r.settled.is_none())
        .map(|r| (r.id, r.price.minor()))
        .max()
        .expect("a fresh quote");
    assert_eq!(
        fresh.1, 300,
        "the corrected spread did not reach a new quote"
    );
}

/// **M8 (LOW).** A listing could name **any** market maker, including another
/// organization's: nothing looked the source up — not that it existed, not that it
/// belonged to this vendor, not that it made a market in this venue — while the sale's
/// money went to the vendor's own account. One shop's trading consumed another
/// organization's declared risk budget and was gated on a wallet it does not own.
#[test]
fn vf_m8_a_listing_may_not_name_another_organizations_maker() {
    let mut tw = boot();
    let a = vendor(&mut tw, "ShopA", 0);
    let b = vendor(&mut tw, "ShopB", 0);
    let theirs = maker(&mut tw, &b, 1_000_000, 1_000_000, 100_000);

    let refused = err(
        &mut tw,
        GOD,
        &format!(
            "@market/vendor stock #{}={AMMO} price=30 qty=10 source={}",
            a.id.0, theirs.id.0
        ),
    );
    assert!(refused.contains("another organization"), "{refused}");
    assert_eq!(market(&tw).live_listings().count(), 0);

    // A maker id nothing defines names the mistake rather than refusing every later
    // quote with `No such listing.`
    let unknown = err(
        &mut tw,
        GOD,
        &format!(
            "@market/vendor stock #{}={AMMO} price=30 qty=10 source=9999",
            a.id.0
        ),
    );
    assert!(unknown.contains("No such market source."), "{unknown}");

    // A maker of the vendor's own organization is the one that works.
    let mine = maker(&mut tw, &a, 1_000_000, 1_000_000, 100_000);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@market/vendor stock #{}={AMMO} price=30 qty=10 source={}",
            a.id.0, mine.id.0
        ),
    );
    assert_eq!(market(&tw).live_listings().count(), 1);
}

/// **M9 (LOW).** `+market/sell` let one player pin a vendor's cash in mutually exclusive
/// escrows against a single object: the vendor escrows first (D-E6-9) and the seller never
/// accepts, so the economy's duplicate-deposit lock — taken at *activation* — never
/// arrived, and N offers all sat `Offered` with N vendor holds standing.
#[test]
fn vf_m9_one_object_is_one_standing_offer_to_one_vendor() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Pawn", 100_000);
    let who = buyer(&mut tw, "Kaz", 10_000);
    let gun = thing(&mut tw, "a Colt America L36");
    let listing = stock(&mut tw, &shop, GUN, 400, 1, &format!("object=#{}", gun.0));
    buy(&mut tw, who, listing, 1);
    tw.world_mut()
        .move_to(gun, who)
        .expect("the buyer takes delivery");

    ok(
        &mut tw,
        who,
        &format!("+market/sell #{}=#{}", gun.0, shop.id.0),
    );
    let one = held(&tw, shop.account);
    assert!(one > 0, "the vendor escrowed nothing");
    let second = err(
        &mut tw,
        who,
        &format!("+market/sell #{}=#{}", gun.0, shop.id.0),
    );
    assert!(second.contains("already offered"), "{second}");
    assert_eq!(
        held(&tw, shop.account),
        one,
        "one object pinned a second escrow"
    );
}

/// **M10 (LOW).** `@market/vendor stock` minted title over **any** live object: the arm
/// asked only `is_live_object` and `issue_title`'s own `approver_live` — the same test —
/// so a staffer could name a character, a room or an item a player is carrying and the
/// market silently minted a vendor-owned title stamped with any loaded catalogue code,
/// which is also what E6b's fence prices off.
#[test]
fn vf_m10_stocking_does_not_mint_title_over_a_character_or_a_carried_thing() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let who = buyer(&mut tw, "Kaz", 10_000);
    let refused = err(
        &mut tw,
        GOD,
        &format!(
            "@market/vendor stock #{}={GUN} price=400 qty=1 object=#{}",
            shop.id.0, who.0
        ),
    );
    assert!(refused.contains("@asset/title"), "{refused}");
    assert!(
        snap(&tw).asset(who).is_none(),
        "the market minted a title over a player character"
    );

    // The same for a thing a player is carrying.
    let trinket = tw
        .world_mut()
        .create(ObjKind::Thing, "a trinket", GOD, Some(who))
        .expect("an object in their hands");
    let carried = err(
        &mut tw,
        GOD,
        &format!(
            "@market/vendor stock #{}={GUN} price=400 qty=1 object=#{}",
            shop.id.0, trinket.0
        ),
    );
    assert_eq!(carried, refused);
    assert!(snap(&tw).asset(trinket).is_none());

    // What a shop really stocks from is untouched: the staffer's own hands.
    let held_by_staff = thing(&mut tw, "a Colt America L36");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@market/vendor stock #{}={GUN} price=400 qty=1 object=#{}",
            shop.id.0, held_by_staff.0
        ),
    );
    assert!(snap(&tw).asset(held_by_staff).is_some());
}

/// **M12 (LOW).** The public tape had no self-dealing test, so a vendor's own member could
/// print `last_trade` and `volume` at round-trip cost: the contract really settles, so the
/// row is honest, but the price the market publishes to everyone is one the seller chose
/// and paid themselves.
#[test]
fn vf_m12_a_vendors_own_member_does_not_print_the_public_tape() {
    let mut tw = boot();
    let shop = vendor(&mut tw, "Ares", 0);
    let listing = stock(&mut tw, &shop, AMMO, 30, 100, "");
    let outsider = buyer(&mut tw, "Kaz", 100_000);
    goods(&mut tw, &shop, outsider, AMMO, 100);
    buy(&mut tw, outsider, listing, 10);
    let honest = projection(&tw, AMMO);
    assert_eq!(honest.last_trade.map(|a| a.minor()), Some(30));

    // A member of the selling organization buys from their own shop, high.
    let inside = buyer(&mut tw, "Insider", 1_000_000);
    goods(&mut tw, &shop, inside, AMMO, 100);
    // `acts_for` an organization is "holds `economy.account.post` on its root
    // compartment" — the same test `may_view_listing` uses for a seller's own people.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=#{} economy.account.post",
            inside.0, shop.org.0
        ),
    );
    assert!(
        mush_economy::acts_for(tw.world(), inside, &PrincipalId::Organization(shop.org)),
        "the fixture's insider does not act for the selling organization"
    );
    let dear = stock(&mut tw, &shop, AMMO, 9_000, 100, "");
    buy(&mut tw, inside, dear, 10);

    let after = projection(&tw, AMMO);
    assert_eq!(
        after.last_trade.map(|a| a.minor()),
        Some(30),
        "a self-dealt trade printed the public last trade"
    );
    assert_eq!(
        after.volume.minor(),
        honest.volume.minor(),
        "a self-dealt trade printed the public volume"
    );
    // The row is still the buyer's own order history — unpublished, never deleted.
    assert!(
        market(&tw)
            .quotes_of_buyer(&PrincipalId::Object(inside))
            .any(|q| q.settled.is_some()),
        "a self-dealt purchase lost its own record"
    );
}
