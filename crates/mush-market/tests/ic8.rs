//! ---- Doc 18 cut IC8: the market's half of finite stock and port commerce ----
//!
//! Three market changes cut IC8 makes for the trade run's ports, gated here on **the
//! production composition in miniature** (IC0's rig shape: `mush-org`, `mush-economy`
//! with the vendor organization in its frozen party-cap table and the market's
//! observer, `mush-inventory` delegating quantity writes to `"market"`, `mush-market`
//! with one lot-represented catalogue entry and one instrument-represented one), plus
//! a tiny test module, `ic8probe`, that runs a queued closure in a command context of
//! its own so a test can call an API no verb reaches:
//!
//! - **`Supply::Wanted`, the standing bid as a document** (lead rulings L14, L21): it
//!   round-trips, it is refused on both buy paths with `WANTED_IS_A_BID` before any
//!   write, `list_goods` refuses it first, `+market/list` renders it as demand with "not
//!   buying" when the bidder cannot fund a unit, and the projection never counts it;
//! - **the lapsed-hold rule** (lead ruling L7): the GOD sweep keeps a lapsed hold-backed
//!   listing while its hold is Live, Lapsed or unreadable, never spends its budget on
//!   one, and deletes it once the hold reads Gone;
//! - **the release entry** (lead ruling L20): `lapsed_holds` names exactly the lapsed
//!   hold-backed listings, and `release_lapsed_hold` stages the holder's own release and
//!   refuses anyone who cannot take from the endpoint.
//!
//! Selling into a bid is cut IC10's; nothing here sells.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mush_core::module::{GameModule, ModError, Registrar};
use mush_core::{
    ArgShape, Audience, CmdError, CommandSpec, CommitToken, Ctx, GOD, GameClock, IdempotencyKey,
    IdempotencyScope, ObjId, ObjKind, OfferExecutionContext, OfferSubmission, RequestUid,
    ScopedIdempotencyKey,
};
use mush_economy::{
    AccountId, Amount, CatalogRef, EconomicClock, EconomyStore, EscalationKind, ExternalRef,
    PartyCap, VaultId,
};
use mush_inventory::{
    EndpointRef, Grade, InventoryError, InventoryFamilies, InventoryHandleCell, InventoryHandles,
    InventoryModule, LotId, RESERVATION_TTL_SECS,
};
use mush_market::{
    CatalogEntry, CatalogSource, ListingId, ListingSpec, MarketError, MarketStore,
    MarketVisibility, QuoteId, Supply, VendorId, WANTED_IS_A_BID, WantedSpec,
};
use mush_org::{OrgId, PrincipalId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use serde_json::json;
use smol_str::SmolStr;

// ---- the rig ---------------------------------------------------------------

const PACK: &str = "ic8.test";
const GOOD: &str = "ic8:iron";
const CREDIT: &str = "credit";
const T0: i64 = 1_700_000_000;
/// The vendor's organization, pre-wired into the frozen party-cap table (IC0's rule).
const VENDOR_ORG: OrgId = OrgId(0);

#[derive(Debug)]
struct TestCatalog;

impl CatalogSource for TestCatalog {
    fn pack(&self, pack: &str) -> Option<Result<Vec<CatalogEntry>, String>> {
        if pack != PACK {
            return None;
        }
        let entry = |code: &str, name: &str, representation: &str| {
            json!({
                "catalog": { "ruleset": "ic8", "code": code },
                "pack": PACK,
                "definition_version": 1,
                "name": name,
                "baseline": 700,
                "legality": "legal",
                "representation": representation,
                "stackable": true,
                "tags": ["test"],
            })
        };
        // `ore0`..`ore64`: one lot-represented good per kept listing of the sweep-budget
        // test (one live listing per pool).
        let ores = (0..65).map(|n| entry(&format!("ore{n}"), &format!("Ore {n}"), "lot"));
        Some(
            [
                entry("iron", "Iron, graded", "lot"),
                entry("grain", "Grain, sacked", "instrument"),
            ]
            .into_iter()
            .chain(ores)
            .map(|value| serde_json::from_value::<CatalogEntry>(value).map_err(|e| e.to_string()))
            .collect(),
        )
    }
}

/// `ic8probe`: one command that runs a queued closure in its own command context.
type ProbeJob = Box<dyn FnOnce(&mut Ctx<'_>) + Send>;

#[derive(Clone, Default)]
struct ProbeQueue(Arc<Mutex<Vec<ProbeJob>>>);

struct Probe;

impl GameModule for Probe {
    fn name(&self) -> &'static str {
        "ic8probe"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(ProbeQueue::default());
        reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
        reg.command(
            CommandSpec {
                name: "+ic8probe".into(),
                shape: ArgShape::Raw,
                help: "+ic8probe -- test: run the queued closure in a command context.".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let job = ctx
                    .state::<ProbeQueue>()
                    .map(|q| Arc::clone(&q.0))
                    .and_then(|q| q.lock().expect("lock").pop());
                if let Some(job) = job {
                    job(ctx);
                }
                Ok(())
            }),
        )?;
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        Ok(())
    }
}

fn with_probe<T: Send + 'static>(
    tw: &mut TestWorld,
    actor: ObjId,
    f: impl FnOnce(&mut Ctx<'_>) -> T + Send + 'static,
) -> T {
    let slot: Arc<Mutex<Option<T>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&slot);
    let queue = tw
        .world()
        .module_state::<ProbeQueue>()
        .expect("the probe module")
        .clone();
    queue.0.lock().expect("lock").push(Box::new(move |ctx| {
        *sink.lock().expect("lock") = Some(f(ctx));
    }));
    tw.run(actor, "+ic8probe");
    slot.lock().expect("lock").take().expect("the probe ran")
}

fn modules(cell: &InventoryHandleCell) -> Vec<Box<dyn GameModule>> {
    vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(
            mush_economy::EconomyModule::new()
                .with_party_caps(vec![(
                    PrincipalId::Organization(VENDOR_ORG),
                    PartyCap::Accept,
                )])
                .with_observers(vec![Arc::new(mush_market::MarketObserver)]),
        ),
        Box::new(
            InventoryModule::new()
                .with_quantity_delegates(["market"])
                .with_handle_cell(Arc::clone(cell)),
        ),
        Box::new(
            mush_market::MarketModule::new()
                .with_inventory(Arc::clone(cell))
                .with_catalog_sources(vec![Arc::new(TestCatalog)]),
        ),
        Box::new(Probe),
    ]
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
    let events = tw.run(actor, line);
    assert!(
        !has_error(&events),
        "`{line}` was refused: {}",
        texts(&events)
    );
    texts(&events)
}

fn err(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let events = tw.run(actor, line);
    assert!(
        has_error(&events),
        "`{line}` should have been refused: {}",
        texts(&events)
    );
    texts(&events)
}

fn economy(tw: &TestWorld) -> &EconomyStore {
    tw.world().module_state::<EconomyStore>().expect("economy")
}

fn market(tw: &TestWorld) -> &MarketStore {
    tw.world().module_state::<MarketStore>().expect("market")
}

fn account_of(tw: &TestWorld, owner: &PrincipalId) -> AccountId {
    economy(tw)
        .snapshot()
        .accounts()
        .find(|a| &a.owner == owner)
        .expect("the account exists")
        .id
}

fn posted(tw: &TestWorld, account: AccountId) -> i64 {
    economy(tw)
        .snapshot()
        .account(account)
        .expect("the account")
        .posted_balance
        .minor()
}

fn vault_of(tw: &TestWorld, owner: &PrincipalId) -> VaultId {
    economy(tw)
        .snapshot()
        .vaults()
        .find(|v| &v.owner == owner)
        .expect("the vault exists")
        .id
}

fn refused_settlements(tw: &TestWorld) -> usize {
    economy(tw)
        .escalations()
        .filter(|e| matches!(e.kind, EscalationKind::SettlementRefused))
        .count()
}

fn commodity(id: &str) -> mush_core::CanonicalId {
    mush_core::CanonicalId::new(id).expect("test commodity id")
}

fn apply<T>(
    tw: &mut TestWorld,
    families: &InventoryFamilies,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> Result<T, InventoryError>,
) -> Result<T, InventoryError> {
    let engine = tw.engine_mut();
    let (result, prepared) = {
        let mut txn = engine
            .action_transaction(&families.pools)
            .expect("inventory transaction");
        let result = body(&mut txn);
        match result {
            Ok(value) => (Ok(value), txn.prepare().ok()),
            Err(error) => (Err(error), None),
        }
    };
    if let Some(prepared) = prepared {
        let (_, installation) = prepared.into_parts();
        engine
            .install_prepared_action(installation)
            .expect("the fixture installs");
    }
    result
}

fn seed_rooted_pool(
    tw: &mut TestWorld,
    families: &InventoryFamilies,
    at: EndpointRef,
    amount: u64,
    lot: LotId,
) {
    seed(tw, families, at, "iron", amount, lot);
}

/// Put `amount` of `good` into the endpoint's pool through one origin lot (so the pool
/// has the lineage root a lot listing needs).
fn seed(
    tw: &mut TestWorld,
    families: &InventoryFamilies,
    at: EndpointRef,
    good: &str,
    amount: u64,
    lot: LotId,
) {
    let good = commodity(good);
    apply(tw, families, |txn| {
        families
            .mint_origin(
                txn,
                at,
                &good,
                amount,
                Grade::new(2).expect("grade"),
                ExternalRef {
                    provider: "test".into(),
                    kind: "seed".into(),
                    id: "1".into(),
                    revision: None,
                },
                lot,
            )
            .map(|_| ())
    })
    .expect("seed mints one origin lot");
    apply(tw, families, |txn| {
        families.dissolve(txn, at, lot).map(|_| ())
    })
    .expect("the seed lot dissolves");
}

/// Drive the one queued authoritative submission the way the world task does
/// (IC0's `land`, line for line).
fn land(tw: &mut TestWorld, token: u8) -> Result<ObjId, String> {
    let submit = tw
        .engine_mut()
        .take_authoritative_submit()
        .ok_or_else(|| "nothing was staged".to_owned())?;
    let (action, resolved) = {
        let engine = tw.engine();
        let query = engine
            .legal_action_query(
                mush_core::PrincipalId::Object { id: submit.actor },
                submit.actor,
                submit.subject.clone(),
            )
            .map_err(|error| error.to_string())?;
        let offers = engine
            .project_legal_actions(&query)
            .map_err(|error| error.to_string())?;
        let projected = offers
            .projected_for_action(&submit.action)
            .ok_or_else(|| "that action is not currently offered".to_owned())?;
        let submission = OfferSubmission {
            offer: projected.offer(),
            subject: projected.subject().clone(),
            targets: Vec::new(),
            parameters: submit.parameters.clone(),
        };
        let execution = OfferExecutionContext {
            request: RequestUid::from_bytes([token; 16]),
            principal: mush_core::PrincipalId::Object { id: submit.actor },
            actor: submit.actor,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: mush_core::PrincipalId::Object { id: submit.actor },
                    owner: submit.action.key().owner.clone(),
                    namespace: mush_core::CanonicalId::new(submit.action.key().id.as_str())
                        .map_err(|_| "namespace".to_owned())?,
                },
                1,
                IdempotencyKey::Request(RequestUid::from_bytes([token; 16])),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        };
        let resolved = engine
            .resolve_legal_action(&offers, execution, submission)
            .map_err(|error| format!("{error:?}"))?;
        let (action, resolved, _guards) = resolved.into_authoritative_parts();
        (action, resolved)
    };
    let (prepared, _) = tw
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(
            &action,
            &resolved,
            CommitToken::from_bytes([token; 16]),
        )
        .map_err(|error| format!("{error:?}"))?;
    tw.engine_mut()
        .install_prepared_action(prepared.into_parts().1)
        .map_err(|error| format!("{error:?}"))?;
    Ok(submit.actor)
}

/// Fire everything due at `at` (unix seconds): the market's sweep among it.
fn sweep_at(tw: &mut TestWorld, at: i64) {
    tw.set_now(at);
    let _ = tw.fire_due(at * 1000);
}

struct Rig {
    tw: TestWorld,
    families: InventoryFamilies,
    pat: ObjId,
    pat_account: AccountId,
    pat_hold: EndpointRef,
    shop_vault: VaultId,
    vendor: VendorId,
    vendor_account: AccountId,
    faucet: AccountId,
}

/// The rig: a funded buyer with a hold, a vendor with a storefront holding a rooted
/// pool of 100 iron, and no listing yet. The vendor's own account holds nothing.
fn rig() -> Rig {
    let cell: InventoryHandleCell = Arc::default();
    let mut tw = TestWorld::with_modules(modules(&cell));
    let families = tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory publishes its handles")
        .0
        .clone();
    tw.engine_mut().set_offer_keys(
        mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(1, [0x41; 32]).expect("an offer secret"),
            [],
        )
        .expect("an offer key ring"),
    );
    tw.world_mut().set_game_clock(GameClock {
        ic_ratio: 1,
        epoch_offset: 0,
    });
    tw.set_now(T0);
    ok(
        &mut tw,
        GOD,
        &format!("@instrument/register {CREDIT}=2 display=cr:suffix"),
    );
    ok(&mut tw, GOD, &format!("@account/system faucet={CREDIT}"));
    let faucet = economy(&tw)
        .snapshot()
        .accounts()
        .find(|a| matches!(a.owner, PrincipalId::System(_)))
        .expect("the faucet")
        .id;
    ok(&mut tw, GOD, "@org/create Ares");
    let shop = PrincipalId::Organization(VENDOR_ORG);
    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:#{}={CREDIT}", VENDOR_ORG.0),
    );
    let vendor_account = account_of(&tw, &shop);
    let strongbox = tw
        .world_mut()
        .create(ObjKind::Thing, "Ares strongbox", GOD, None)
        .expect("a container");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vault/create Ares=container:#{} owner=org:#{}",
            strongbox.0, VENDOR_ORG.0
        ),
    );
    let shop_vault = vault_of(&tw, &shop);
    ok(&mut tw, GOD, &format!("@market/catalog load {PACK}"));
    ok(
        &mut tw,
        GOD,
        &format!(
            "@market/vendor create #{}=#{} name=Ares market=luna buy=300",
            VENDOR_ORG.0, shop_vault.0
        ),
    );
    let vendor = market(&tw)
        .vendor_of_org(VENDOR_ORG)
        .expect("registered")
        .id;
    let pat = tw.spawn_player("Pat");
    ok(&mut tw, GOD, &format!("@account/open #{}={CREDIT}", pat.0));
    let pat_account = account_of(&tw, &PrincipalId::Object(pat));
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {faucet}={pat_account} 100000"),
    );
    let crate_ = tw
        .world_mut()
        .create(ObjKind::Thing, "Pat's crate", pat, Some(pat))
        .expect("a container");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vault/create Hold=container:#{} owner=#{}",
            crate_.0, pat.0
        ),
    );
    let pat_hold = EndpointRef(vault_of(&tw, &PrincipalId::Object(pat)));
    seed_rooted_pool(
        &mut tw,
        &families,
        EndpointRef(shop_vault),
        100,
        LotId(0xb1),
    );
    Rig {
        tw,
        families,
        pat,
        pat_account,
        pat_hold,
        shop_vault,
        vendor,
        vendor_account,
        faucet,
    }
}

fn iron() -> CatalogRef {
    CatalogRef {
        ruleset: SmolStr::new_static("ic8"),
        code: SmolStr::new_static("iron"),
    }
}

fn grain() -> CatalogRef {
    CatalogRef {
        ruleset: SmolStr::new_static("ic8"),
        code: SmolStr::new_static("grain"),
    }
}

fn wanted(r: &Rig, price: i64, remaining: u64) -> WantedSpec {
    WantedSpec {
        vendor: r.vendor,
        catalog: iron(),
        price: Amount::from_minor(price),
        remaining,
        vault: r.shop_vault,
        min_grade: 0,
        ttl_secs: 86_400,
    }
}

fn post(r: &mut Rig, spec: WantedSpec) -> Result<ListingId, MarketError> {
    with_probe(&mut r.tw, GOD, move |ctx| {
        mush_market::post_wanted(ctx, spec)
    })
}

/// Stock a hold-backed listing of `qty` iron out of the shop and land its hold.
fn stock_hold(r: &mut Rig, qty: u64, days: i64) -> ListingId {
    let said = ok(
        &mut r.tw,
        GOD,
        &format!(
            "@market/vendor stock #{}={GOOD} price=700 qty={qty} days={days} endpoint={}",
            r.vendor.0, r.shop_vault.0
        ),
    );
    let id = said
        .split("Listing #")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|n| n.parse::<u64>().ok())
        .map(ListingId)
        .unwrap_or_else(|| panic!("a listing id in: {said}"));
    land(&mut r.tw, 0x10).expect("the hold lands");
    id
}

// ---- Supply::Wanted -----------------------------------------------------------

/// **A standing bid round-trips, and both buy paths refuse it before any write** (lead
/// ruling L14): the quote with `WANTED_IS_A_BID` and no quote written; the accept of a
/// quote document naming it (hand-made through a reboot of the market's documents)
/// with the same line, no contract and the buyer's money untouched.
#[test]
fn ic8_wanted_round_trips_and_buy_paths_refuse() {
    let mut r = rig();
    let id = {
        let spec = wanted(&r, 500, 20);
        post(&mut r, spec)
    }
    .expect("the bid posts");
    let listing = market(&r.tw).listing(id).expect("the bid stands").clone();
    assert_eq!(listing.seller, PrincipalId::Organization(VENDOR_ORG));
    assert_eq!(listing.quantity, 20);
    assert_eq!(listing.price.minor(), 500);
    assert_eq!(listing.revision, 1);
    assert_eq!(listing.visibility, MarketVisibility::Public);
    assert_eq!(listing.expires.clock, EconomicClock::RealUtc);
    assert!(matches!(
        &listing.source,
        Supply::Wanted { vault, commodity, min_grade: 0 }
            if *vault == r.shop_vault && commodity.0.as_str() == "iron"
    ));
    let wire = serde_json::to_value(&listing).expect("encodes");
    let back: mush_market::Listing = serde_json::from_value(wire.clone()).expect("decodes");
    assert_eq!(back, listing, "the bid round-trips on the wire");
    let mut bad = wire;
    bad["source"]["wanted"]["min_grade"] = json!(101);
    assert!(
        serde_json::from_value::<mush_market::Listing>(bad).is_err(),
        "a grade floor past MAX_GRADE is refused on the wire"
    );
    assert!(
        market(&r.tw).lot_matter(&listing).is_none(),
        "not a lot sale"
    );
    assert!(listing.source.lot().is_none());
    assert!(
        market(&r.tw)
            .lot_listing_of(EndpointRef(r.shop_vault), &commodity("iron"))
            .is_none(),
        "a bid sits on no pool"
    );

    let pat = r.pat;
    let quotes = market(&r.tw).quotes().count();
    let refused = err(&mut r.tw, pat, &format!("+market/quote {id}=1"));
    assert!(refused.contains(WANTED_IS_A_BID), "{refused}");
    assert_eq!(
        market(&r.tw).quotes().count(),
        quotes,
        "no quote was written"
    );

    // A quote document naming the bid, made by editing a real one through a reboot.
    let held = stock_hold(&mut r, 10, 3);
    ok(&mut r.tw, pat, &format!("+market/quote {held}=10"));
    let ops = r.tw.take_module_ops();
    let mut boot: HashMap<SmolStr, mush_core::ModuleBootData> = mush_testkit::fold_module_ops(ops);
    let docs = boot.get_mut(mush_market::NS).expect("market documents");
    for (key, value) in &mut docs.kv {
        if key.as_str() == "market:quote:0" {
            value["listing"] = json!(id.0);
        }
    }
    r.tw.modules_loaded(boot);
    assert_eq!(
        market(&r.tw).quote(QuoteId(0)).map(|q| q.listing),
        Some(id),
        "the quote now names the bid"
    );
    let refused = err(
        &mut r.tw,
        pat,
        &format!("+market/accept 0={}", r.pat_hold.vault().0),
    );
    assert!(refused.contains(WANTED_IS_A_BID), "{refused}");
    assert!(
        market(&r.tw)
            .quote(QuoteId(0))
            .is_some_and(|q| q.contract.is_none()),
        "no contract was drafted"
    );
    assert_eq!(posted(&r.tw, r.pat_account), 100_000, "nothing was charged");
    assert!(r.tw.engine_mut().take_authoritative_submit().is_none());
    assert_eq!(refused_settlements(&r.tw), 0);
}

/// **`list_goods` refuses a `Supply::Wanted` first** (reviewer F7): on a lot entry and
/// on a non-lot entry alike — the latter would otherwise fall to the counter arm and
/// publish — with no listing written.
#[test]
fn ic8_list_goods_refuses_a_wanted_source() {
    let mut r = rig();
    for catalog in [iron(), grain()] {
        let spec = ListingSpec {
            vendor: r.vendor,
            catalog,
            price: Amount::from_minor(500),
            quantity: 5,
            source: Supply::Wanted {
                vault: r.shop_vault,
                commodity: mush_market::LotCommodity(commodity("iron")),
                min_grade: 0,
            },
            object: None,
            visibility: MarketVisibility::Public,
            ttl_secs: 86_400,
        };
        let before = market(&r.tw).live_listings().count();
        let refused = with_probe(&mut r.tw, GOD, move |ctx| {
            mush_market::list_goods(ctx, spec)
        });
        assert_eq!(
            refused,
            Err(MarketError::bad(
                "A standing bid is posted with post_wanted, never listed."
            ))
        );
        assert_eq!(
            market(&r.tw).live_listings().count(),
            before,
            "nothing listed"
        );
    }
}

/// **The `+market/list` row, and "not buying"** (lead ruling L14): demand, rendered as
/// such; " — not buying" while the bidder's own account cannot fund one unit, and the
/// plain row once it can. `+market/show` names it a standing bid.
#[test]
fn ic8_the_list_row_and_not_buying() {
    let mut r = rig();
    let id = {
        let spec = wanted(&r, 500, 20);
        post(&mut r, spec)
    }
    .expect("the bid posts");
    let pat = r.pat;
    let list = ok(&mut r.tw, pat, "+market/list");
    assert!(
        list.contains(&format!(
            "#{id} Iron, graded — wanted 20 × 5.00cr [luna] by Ares — not buying"
        )),
        "{list}"
    );
    let (faucet, account) = (r.faucet, r.vendor_account);
    ok(
        &mut r.tw,
        GOD,
        &format!("@econ/mint {faucet}={account} 499"),
    );
    let list = ok(&mut r.tw, pat, "+market/list");
    assert!(list.contains("— not buying"), "499 cannot fund 500: {list}");
    ok(&mut r.tw, GOD, &format!("@econ/mint {faucet}={account} 1"));
    let list = ok(&mut r.tw, pat, "+market/list");
    assert!(
        list.contains(&format!(
            "#{id} Iron, graded — wanted 20 × 5.00cr [luna] by Ares"
        )) && !list.contains("not buying"),
        "{list}"
    );
    let show = ok(&mut r.tw, pat, &format!("+market/show {id}"));
    assert!(
        show.contains("  supply: a standing bid; the house buys this"),
        "{show}"
    );
}

/// **The projection ignores a standing bid**: no band, no depth, no default venue.
#[test]
fn ic8_the_projection_ignores_a_wanted() {
    let mut r = rig();
    let _bid = {
        let spec = wanted(&r, 500, 20);
        post(&mut r, spec)
    }
    .expect("the bid posts");
    let store = market(&r.tw);
    let projection = store.projection(
        &iron(),
        &mush_market::MarketId::new("luna").expect("venue"),
        economy(&r.tw).snapshot(),
        &[],
        T0,
    );
    assert_eq!(projection.band, (Amount::ZERO, Amount::ZERO), "no band");
    assert!(projection.depth.iter().all(|b| b.quantity == 0), "no depth");
    let pat = r.pat;
    let depth = ok(&mut r.tw, pat, "+market/depth ic8:iron");
    assert!(!depth.contains("5.00cr"), "a bid is not depth: {depth}");
}

/// **`post_wanted` and `revise_wanted` refuse what they must**, writing nothing —
/// each case by its own answer (fixer T14), so no case passes on another check's
/// refusal; the ttl is clamped to `60..=MAX_LISTING_TTL_SECS`; the vendor's listing cap
/// holds; a retired vendor posts no bid.
#[test]
#[allow(clippy::too_many_lines)]
fn ic8_post_and_revise_wanted_refusals() {
    let mut r = rig();
    let refuses = |r: &mut Rig, spec: WantedSpec, want: MarketError| {
        let before = market(&r.tw).live_listings().count();
        let answer = post(r, spec);
        assert_eq!(answer, Err(want));
        assert_eq!(market(&r.tw).live_listings().count(), before);
    };
    let bad = MarketError::bad;
    let base = wanted(&r, 500, 20);
    refuses(
        &mut r,
        WantedSpec {
            catalog: grain(),
            ..base.clone()
        },
        bad("A standing bid buys a lot-represented good."),
    );
    refuses(
        &mut r,
        WantedSpec {
            price: Amount::ZERO,
            ..base.clone()
        },
        bad("A standing bid offers a positive price."),
    );
    for remaining in [0, mush_market::MAX_QUANTITY + 1] {
        refuses(
            &mut r,
            WantedSpec {
                remaining,
                ..base.clone()
            },
            bad("That is not a quantity this market buys."),
        );
    }
    refuses(
        &mut r,
        WantedSpec {
            min_grade: 101,
            ..base.clone()
        },
        bad("That grade floor is out of range."),
    );
    let pat_vault = r.pat_hold.vault();
    refuses(
        &mut r,
        WantedSpec {
            vault: pat_vault,
            ..base.clone()
        },
        bad("A standing bid receives into the vendor's own storefront vault."),
    );
    refuses(
        &mut r,
        WantedSpec {
            vendor: VendorId(99),
            ..base.clone()
        },
        MarketError::NoStanding(mush_market::NO_SUCH_VENDOR),
    );
    let id = post(&mut r, base.clone()).expect("the bid posts");
    refuses(
        &mut r,
        base.clone(),
        bad("That vendor already posts a standing bid for that good."),
    );
    // Revise: in place, the revision bumped; a non-bid and bad figures refused.
    let revised = with_probe(&mut r.tw, GOD, move |ctx| {
        mush_market::revise_wanted(ctx, id, Amount::from_minor(600), 30)
    });
    assert_eq!(revised, Ok(()));
    let listing = market(&r.tw).listing(id).expect("the bid").clone();
    assert_eq!(
        (listing.price.minor(), listing.quantity, listing.revision),
        (600, 30, 2)
    );
    // Doc 18 cut IC10: an intake of 0 is no longer refused here — it writes the
    // exhausted marker (`tests/ic10.rs` pins it) — so the `(600, 0, ..)` row left.
    for (price, remaining, want) in [
        (0, 30, "A standing bid offers a positive price."),
        (
            600,
            mush_market::MAX_QUANTITY + 1,
            "That is not a quantity this market buys.",
        ),
    ] {
        let answer = with_probe(&mut r.tw, GOD, move |ctx| {
            mush_market::revise_wanted(ctx, id, Amount::from_minor(price), remaining)
        });
        assert_eq!(answer, Err(bad(want)), "{price} {remaining}");
    }
    let unchanged = market(&r.tw).listing(id).expect("the bid").clone();
    assert_eq!(unchanged, listing, "a refused revision writes nothing");
    let held = stock_hold(&mut r, 10, 3);
    let answer = with_probe(&mut r.tw, GOD, move |ctx| {
        mush_market::revise_wanted(ctx, held, Amount::from_minor(600), 30)
    });
    assert_eq!(
        answer,
        Err(MarketError::NoStanding(mush_market::NO_SUCH_LISTING))
    );
    // A withdrawn bid is deleted and stages nothing.
    with_probe(&mut r.tw, GOD, move |ctx| {
        mush_market::withdraw_listing(ctx, id)
    })
    .expect("withdrawn");
    assert!(market(&r.tw).listing(id).is_none());
    assert!(r.tw.engine_mut().take_authoritative_submit().is_none());
    // The ttl is clamped at both ends.
    let ore = |n: usize| CatalogRef {
        ruleset: SmolStr::new_static("ic8"),
        code: SmolStr::new(format!("ore{n}")),
    };
    let now = r.tw.world().now();
    for (n, ttl, lasts) in [(0, 1, 60), (1, i64::MAX, mush_market::MAX_LISTING_TTL_SECS)] {
        let id = post(
            &mut r,
            WantedSpec {
                catalog: ore(n),
                ttl_secs: ttl,
                ..base.clone()
            },
        )
        .expect("the bid posts");
        assert_eq!(
            market(&r.tw).listing(id).expect("the bid").expires.at,
            now + lasts,
            "ttl {ttl}"
        );
    }
    // The vendor's listing cap.
    let mut n = 2;
    while market(&r.tw).listings_of_vendor(r.vendor) < mush_market::MAX_LISTINGS_PER_VENDOR {
        post(
            &mut r,
            WantedSpec {
                catalog: ore(n),
                ..base.clone()
            },
        )
        .expect("under the cap");
        n += 1;
    }
    refuses(
        &mut r,
        WantedSpec {
            catalog: ore(n),
            ..base.clone()
        },
        bad("That vendor already holds as many listings as the market allows."),
    );
    // A retired vendor posts no bid.
    let vendor = r.vendor;
    with_probe(&mut r.tw, GOD, move |ctx| {
        mush_market::retire_vendor(ctx, vendor)
    })
    .expect("retired");
    refuses(
        &mut r,
        WantedSpec {
            catalog: ore(64),
            ..base.clone()
        },
        bad("That vendor is not trading, so it posts no bid."),
    );
}

// ---- the lapsed-hold rule ------------------------------------------------------

/// **A lapsed hold-backed listing survives the sweep until its hold is gone** (lead
/// ruling L7): kept while the hold is Live (real deadline passed, IC hold not), kept
/// while it is Lapsed (IC time past the hold), and deleted on the sweep after the
/// holder's release lands.
#[test]
fn ic8_a_lapsed_hold_listing_survives_the_sweep_until_its_hold_is_gone() {
    let mut r = rig();
    let id = stock_hold(&mut r, 10, 1);
    let expires = market(&r.tw).listing(id).expect("listed").expires.at;
    sweep_at(&mut r.tw, expires + 61);
    assert!(
        market(&r.tw).listing(id).is_some(),
        "kept while its hold is Live"
    );
    assert_eq!(
        mush_market::lapsed_holds(r.tw.world(), 64)
            .iter()
            .map(|h| h.listing)
            .collect::<Vec<_>>(),
        vec![id]
    );
    r.tw.world_mut().set_game_clock(GameClock {
        ic_ratio: 1,
        epoch_offset: RESERVATION_TTL_SECS + 1,
    });
    sweep_at(&mut r.tw, expires + 122);
    assert!(
        market(&r.tw).listing(id).is_some(),
        "kept while its hold is Lapsed"
    );
    // The holder (staff may take from the org's storefront) releases it.
    with_probe(&mut r.tw, GOD, move |ctx| {
        mush_market::release_lapsed_hold(ctx, id)
    })
    .expect("the release stages");
    assert!(
        market(&r.tw).listing(id).is_some(),
        "the release deletes nothing"
    );
    land(&mut r.tw, 0x21).expect("the release lands");
    sweep_at(&mut r.tw, expires + 183);
    assert!(
        market(&r.tw).listing(id).is_none(),
        "deleted once the hold is Gone"
    );
    assert_eq!(refused_settlements(&r.tw), 0);
}

/// **Kept hold listings never spend the sweep budget** (reviewer F5): 65 kept
/// hold-backed lapsed listings and one lapsed counter listing with the highest id — the
/// counter listing is deleted on the first sweep. (Before IC8's filter-then-take, the
/// first 64 lapsed listings in id order were taken whether or not they were deleted.)
/// Two vendors carry them, because one vendor holds at most `MAX_LISTINGS_PER_VENDOR`;
/// the pools are the `ore<n>` goods at two endpoints, inside the actor's endpoint offer.
#[test]
fn ic8_kept_hold_listings_never_spend_the_sweep_budget() {
    let mut r = rig();
    let families = r.families.clone();
    // A second vendor, Bora, with a storefront of its own.
    ok(&mut r.tw, GOD, "@org/create Bora");
    let bora = OrgId(1);
    ok(
        &mut r.tw,
        GOD,
        &format!("@account/open org:#{}={CREDIT}", bora.0),
    );
    let chest =
        r.tw.world_mut()
            .create(ObjKind::Thing, "Bora chest", GOD, None)
            .expect("a container");
    ok(
        &mut r.tw,
        GOD,
        &format!(
            "@vault/create Bora=container:#{} owner=org:#{}",
            chest.0, bora.0
        ),
    );
    let bora_vault = vault_of(&r.tw, &PrincipalId::Organization(bora));
    ok(
        &mut r.tw,
        GOD,
        &format!(
            "@market/vendor create #{}=#{} name=Bora market=luna buy=300",
            bora.0, bora_vault.0
        ),
    );
    let bora_vendor = market(&r.tw).vendor_of_org(bora).expect("registered").id;
    for n in 0..65_u64 {
        let (vendor, vault) = if n < 33 {
            (r.vendor, r.shop_vault)
        } else {
            (bora_vendor, bora_vault)
        };
        let ore = format!("ore{n}");
        seed(
            &mut r.tw,
            &families,
            EndpointRef(vault),
            &ore,
            1,
            LotId(0x1000 + n),
        );
        let said = ok(
            &mut r.tw,
            GOD,
            &format!(
                "@market/vendor stock #{}=ic8:{ore} price=700 qty=1 days=1 endpoint={}",
                vendor.0, vault.0
            ),
        );
        assert!(said.contains("published"), "{said}");
        let token = u8::try_from(n).expect("small").wrapping_add(0x40);
        land(&mut r.tw, token).expect("the hold lands");
    }
    // One counter listing (instrument-represented), the highest id, lapsing too.
    ok(
        &mut r.tw,
        GOD,
        &format!(
            "@market/vendor stock #{}=ic8:grain price=100 qty=5 days=1",
            bora_vendor.0
        ),
    );
    let counter = market(&r.tw)
        .live_listings()
        .find(|l| l.catalog == grain())
        .expect("the counter listing")
        .id;
    assert!(
        market(&r.tw).live_listings().all(|l| l.id <= counter),
        "the counter listing has the highest id"
    );
    let latest = market(&r.tw)
        .live_listings()
        .map(|l| l.expires.at)
        .max()
        .expect("listings");
    sweep_at(&mut r.tw, latest + 61);
    assert!(
        market(&r.tw).listing(counter).is_none(),
        "the counter listing went"
    );
    assert_eq!(
        mush_market::lapsed_holds(r.tw.world(), 128).len(),
        65,
        "every hold-backed listing was kept"
    );
}

/// **A release by someone who cannot take from the endpoint is refused** — the same
/// answer as no listing at all — and stages nothing.
#[test]
fn ic8_release_by_a_non_holder_is_refused() {
    let mut r = rig();
    let id = stock_hold(&mut r, 10, 1);
    // Not lapsed yet: refused for the holder too, and nothing staged.
    let answer = with_probe(&mut r.tw, GOD, move |ctx| {
        mush_market::release_lapsed_hold(ctx, id)
    });
    assert!(matches!(answer, Err(MarketError::Bad(_))), "{answer:?}");
    assert!(r.tw.engine_mut().take_authoritative_submit().is_none());
    let expires = market(&r.tw).listing(id).expect("listed").expires.at;
    r.tw.set_now(expires + 1);
    let pat = r.pat;
    let answer = with_probe(&mut r.tw, pat, move |ctx| {
        mush_market::release_lapsed_hold(ctx, id)
    });
    assert_eq!(
        answer,
        Err(MarketError::NoStanding(mush_market::NO_SUCH_LISTING))
    );
    let answer = with_probe(&mut r.tw, pat, move |ctx| {
        mush_market::release_lapsed_hold(ctx, ListingId(999))
    });
    assert_eq!(
        answer,
        Err(MarketError::NoStanding(mush_market::NO_SUCH_LISTING))
    );
    assert!(r.tw.engine_mut().take_authoritative_submit().is_none());
}

/// **`lapsed_holds` lists only lapsed hold-backed listings**: not a live one, not a
/// lapsed counter listing, not a standing bid.
#[test]
fn ic8_lapsed_holds_lists_only_lapsed_hold_listings() {
    let mut r = rig();
    let lapsing = stock_hold(&mut r, 10, 1);
    let bid = {
        let spec = wanted(&r, 500, 20);
        post(&mut r, spec)
    }
    .expect("the bid posts");
    ok(
        &mut r.tw,
        GOD,
        &format!(
            "@market/vendor stock #{}=ic8:grain price=100 qty=5 days=1",
            r.vendor.0
        ),
    );
    let expires = market(&r.tw).listing(lapsing).expect("listed").expires.at;
    assert!(
        mush_market::lapsed_holds(r.tw.world(), 64).is_empty(),
        "nothing lapsed yet"
    );
    r.tw.set_now(expires + 1);
    let holds = mush_market::lapsed_holds(r.tw.world(), 64);
    assert_eq!(holds.len(), 1, "{holds:?}");
    let hold = &holds[0];
    assert_eq!(hold.listing, lapsing);
    assert_eq!(hold.endpoint, EndpointRef(r.shop_vault));
    assert_eq!(hold.commodity.as_str(), "iron");
    assert_eq!(hold.quantity, 10);
    assert_eq!(hold.name, "Iron, graded");
    assert_ne!(hold.listing, bid);
    assert!(
        mush_market::lapsed_holds(r.tw.world(), 0).is_empty(),
        "the limit bounds it"
    );
}

/// **Doc 18 cut IC14 row 15: a non-holder hears `NO_SUCH_LISTING` for any listing.**
/// `release_lapsed_hold` checks custody before it checks whether the listing is
/// hold-backed, so a stranger's answer for a Draw (storefront) listing and for a
/// hold-backed listing is the one byte-identical refusal — whether a listing holds
/// stock is not something a stranger learns. The holder still hears the specific line.
#[test]
fn ic14_a_non_holder_hears_no_such_listing_for_any_listing() {
    let mut r = rig();
    let held = stock_hold(&mut r, 10, 1);
    // The Draw listing on another good's storefront pool (one live listing per pool).
    let families = r.families.clone();
    seed(
        &mut r.tw,
        &families,
        EndpointRef(r.shop_vault),
        "ore0",
        5,
        LotId(0xd0),
    );
    let said = ok(
        &mut r.tw,
        GOD,
        &format!(
            "@market/vendor stock #{}=ic8:ore0 price=700 qty=1",
            r.vendor.0
        ),
    );
    let drawn = market(&r.tw)
        .live_listings()
        .map(|l| l.id)
        .max()
        .expect("the Draw listing");
    assert_ne!(drawn, held, "{said}");
    let pat = r.pat;
    let mut answers = Vec::new();
    for id in [drawn, held] {
        let answer = with_probe(&mut r.tw, pat, move |ctx| {
            mush_market::release_lapsed_hold(ctx, id)
        });
        answers.push(answer);
    }
    for answer in &answers {
        assert_eq!(
            answer,
            &Err(MarketError::NoStanding(mush_market::NO_SUCH_LISTING)),
            "{answers:?}"
        );
    }
    assert!(r.tw.engine_mut().take_authoritative_submit().is_none());
    // The holder (staff) still hears why a Draw listing has nothing to release.
    let answer = with_probe(&mut r.tw, GOD, move |ctx| {
        mush_market::release_lapsed_hold(ctx, drawn)
    });
    assert_eq!(
        answer,
        Err(MarketError::bad(
            "That listing holds no stock, so there is nothing to release."
        ))
    );
}

/// **Doc 18 cut IC14 row 16: a kept lapsed hold is read once per ten sweeps.** On a
/// roster without a releaser the sweep keeps a lapsed hold-backed listing until its
/// hold reads `Gone`; the read now backs off per listing — 20 sweeps make two reads —
/// while a holder's staged release lifts the backoff, so its landing is seen on the
/// next sweep.
#[test]
fn ic14_a_kept_lapsed_hold_is_read_once_per_ten_sweeps() {
    let mut r = rig();
    let id = stock_hold(&mut r, 10, 1);
    let expires = market(&r.tw).listing(id).expect("listed").expires.at;
    let start = market(&r.tw).hold_reads();
    for n in 1..=20 {
        sweep_at(&mut r.tw, expires + 61 * n);
    }
    assert!(market(&r.tw).listing(id).is_some(), "kept: the hold stands");
    assert_eq!(
        market(&r.tw).hold_reads() - start,
        2,
        "two reads in 20 sweeps"
    );
    // The holder releases the hold: a staged release is read on every sweep for a
    // backoff window, so the landing deletes the listing on the very next sweep.
    let answer = with_probe(&mut r.tw, GOD, move |ctx| {
        mush_market::release_lapsed_hold(ctx, id)
    });
    assert_eq!(answer, Ok(()));
    land(&mut r.tw, 0xc4).expect("the release lands");
    let base = expires + 61 * 20;
    let mut gone_after = None;
    for n in 1..=mush_market::HOLD_RECHECK_SWEEPS {
        sweep_at(&mut r.tw, base + 61 * i64::try_from(n).expect("small"));
        if market(&r.tw).listing(id).is_none() {
            gone_after = Some(n);
            break;
        }
    }
    assert_eq!(
        gone_after,
        Some(1),
        "a released hold's listing is deleted on the next sweep"
    );
}
