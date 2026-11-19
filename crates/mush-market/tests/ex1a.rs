//! ---- Doc 48 cut EX.1a: the seller, in the market ----
//!
//! Before this cut only a staff-registered vendor organization sold on the market. These
//! gates hold what EX.1a built (doc 48 §7.1's row; Q-48-2, -5, -44, -49, -50, -54):
//!
//! - **a player lists an exact lot** from a vault they may take from, and another player
//!   buys it while the seller types nothing: the listing is the seller's **standing
//!   acceptance**, which `mush_market::ListingAcceptance` checks against the very sale
//!   contract before the economy accepts for them;
//! - **a player's bid escrows** `price × quantity` at posting, and each sale into it draws
//!   the escrow in **one plan**: the bid's hold released, the sale's escrow placed, the
//!   rest of the bid placed again, so the money is reserved throughout and once;
//! - **a private offer** is refused to anyone but its buyer, at quote and at accept;
//! - **a listing outlives seven real hours** at 24:1 (Q-48-54's first point: an exact
//!   lot's hold has no clock, so the listing is no longer cut to the reservation's);
//! - **the caps**: 32 listings a seller, the store's 1,024 ceiling under 32 sellers'
//!   load, and the economy's 100-a-party and 16-an-issuer contract caps for player
//!   traders.
//!
//! The composition is production's in miniature: `mush-org`, `mush-economy` with the
//! market's observer, its dispute guard and its standing acceptance, the inventory and
//! market facts, `mush-inventory` delegating quantity writes to `"market"`, and
//! `mush-market` with one vendor in the venue so the venue has a currency.

use std::sync::Arc;
use std::time::Instant;

use mush_core::{
    AuthoritativeSubmit, CommitToken, GOD, GameClock, IdempotencyKey, IdempotencyScope, ObjId,
    ObjKind, OfferExecutionContext, OfferSubmission, RequestUid, ScopedIdempotencyKey,
};
use mush_economy::{
    AccountId, ContractId, ContractState, EconomyStore, EscalationKind, ExternalRef, FactProvider,
    HoldId, PartyCap, StandingAcceptance,
};
use mush_inventory::{
    EndpointRef, Grade, InventoryError, InventoryFactProvider, InventoryFamilies,
    InventoryHandleCell, InventoryHandles, InventoryModule, LotId,
};
use mush_market::{
    CatalogEntry, CatalogSource, ListingAcceptance, ListingId, MAX_LISTINGS,
    MAX_LISTINGS_PER_SELLER, MarketFactProvider, MarketStore, NO_SUCH_LISTING, NO_VENDOR, QuoteId,
};
use mush_org::{OrgId, PrincipalId};
use mush_testkit::TestWorld;
use mush_testkit::fixtures::{err, ok};
use serde_json::json;

// ---- the rig ---------------------------------------------------------------

const PACK: &str = "ex1a.test";
const CREDIT: &str = "credit";
const T0: i64 = 1_700_000_000;
const FUNDS: i64 = 100_000;
const VENDOR_ORG: OrgId = OrgId(0);
/// The pack's extra goods, one bid each a seller may post (a bidder holds one bid a good).
const GOODS: usize = 32;

fn good(n: usize) -> String {
    format!("g{n:02}")
}

#[derive(Debug)]
struct TestCatalog;

impl CatalogSource for TestCatalog {
    fn pack(&self, pack: &str) -> Option<Result<Vec<CatalogEntry>, String>> {
        if pack != PACK {
            return None;
        }
        let entry = |code: String, name: String| {
            json!({
                "catalog": { "ruleset": "ex1a", "code": code },
                "pack": PACK,
                "definition_version": 1,
                "name": name,
                "baseline": 700,
                "legality": "legal",
                "representation": "lot",
                "stackable": true,
                "tags": ["test"],
            })
        };
        let mut all = vec![entry("iron".to_owned(), "Iron, graded".to_owned())];
        all.extend((0..GOODS).map(|n| entry(good(n), format!("Good {n}"))));
        Some(
            all.into_iter()
                .map(|v| serde_json::from_value::<CatalogEntry>(v).map_err(|e| e.to_string()))
                .collect(),
        )
    }
}

fn modules(cell: &InventoryHandleCell) -> Vec<Box<dyn mush_core::module::GameModule>> {
    let facts: Vec<Arc<dyn FactProvider>> = vec![
        Arc::new(MarketFactProvider),
        Arc::new(InventoryFactProvider),
    ];
    vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(
            mush_economy::EconomyModule::new()
                .with_party_caps(vec![(
                    PrincipalId::Organization(VENDOR_ORG),
                    PartyCap::Accept,
                )])
                .with_observers(vec![Arc::new(mush_market::MarketObserver)])
                .with_dispute_guard(Arc::new(mush_market::SaleDisputeGuard))
                // Doc 48 cut EX.1a: the production composition's one implementor.
                .with_standing_acceptance(Arc::new(ListingAcceptance))
                .with_facts(facts),
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
    ]
}

/// One trader: a person with a funded account and a vault of their own.
#[derive(Clone, Copy, Debug)]
struct Trader {
    who: ObjId,
    account: AccountId,
    vault: EndpointRef,
}

struct Rig {
    tw: TestWorld,
    families: InventoryFamilies,
    faucet: AccountId,
    sal: Trader,
    bea: Trader,
    cy: Trader,
}

fn economy(tw: &TestWorld) -> &EconomyStore {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("the economy store")
}

fn market(tw: &TestWorld) -> &MarketStore {
    tw.world()
        .module_state::<MarketStore>()
        .expect("the market store")
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

fn held(tw: &TestWorld, account: AccountId) -> i64 {
    economy(tw)
        .snapshot()
        .account(account)
        .expect("the account")
        .held_amount
        .minor()
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

fn read<T>(
    tw: &mut TestWorld,
    families: &InventoryFamilies,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> T,
) -> T {
    let mut txn = tw
        .engine_mut()
        .action_transaction(&families.pools)
        .expect("inventory transaction");
    body(&mut txn)
}

/// Mint one exact lot of `quantity` `id` into `at` (it stays a lot: not dissolved).
fn mint_lot(
    tw: &mut TestWorld,
    families: &InventoryFamilies,
    at: EndpointRef,
    id: &str,
    quantity: u64,
    lot: LotId,
) {
    apply(tw, families, |txn| {
        families
            .mint_origin(
                txn,
                at,
                &commodity(id),
                quantity,
                Grade::new(2).expect("grade"),
                ExternalRef {
                    provider: "test".into(),
                    kind: "seed".into(),
                    id: lot.0.to_string().into(),
                    revision: None,
                },
                lot,
            )
            .map(|_| ())
    })
    .expect("a lot mints");
}

/// Put `amount` of `id` into the endpoint's pool through a lot (the pool's lineage root).
fn seed_pool(
    tw: &mut TestWorld,
    families: &InventoryFamilies,
    at: EndpointRef,
    id: &str,
    amount: u64,
    lot: LotId,
) {
    mint_lot(tw, families, at, id, amount, lot);
    apply(tw, families, |txn| {
        families.dissolve(txn, at, lot).map(|_| ())
    })
    .expect("the seed lot dissolves into the pool");
}

/// The pool balance of `id` at an endpoint.
fn pool(tw: &mut TestWorld, families: &InventoryFamilies, at: EndpointRef, id: &str) -> u64 {
    read(tw, families, |txn| {
        families
            .pool(txn, at, &commodity(id))
            .ok()
            .flatten()
            .map_or(0, |p| p.quantity)
    })
}

/// Where a lot is now, if it exists.
fn lot_at(tw: &mut TestWorld, families: &InventoryFamilies, lot: LotId) -> Option<EndpointRef> {
    read(tw, families, |txn| {
        txn.get(&families.lots, &lot)
            .ok()
            .flatten()
            .map(mush_inventory::Lot::endpoint)
    })
}

fn trader(tw: &mut TestWorld, name: &str, faucet: AccountId) -> Trader {
    let who = tw.spawn_player(name);
    ok(tw, GOD, &format!("@account/open #{}={CREDIT}", who.0));
    let account = account_of(tw, &PrincipalId::Object(who));
    ok(tw, GOD, &format!("@econ/mint {faucet}={account} {FUNDS}"));
    let crate_ = tw
        .world_mut()
        .create(ObjKind::Thing, &format!("{name}'s crate"), who, Some(who))
        .expect("a container");
    ok(
        tw,
        GOD,
        &format!(
            "@vault/create Hold=container:#{} owner=#{}",
            crate_.0, who.0
        ),
    );
    let vault = EndpointRef(
        economy(tw)
            .snapshot()
            .vaults()
            .find(|v| v.owner == PrincipalId::Object(who))
            .expect("the vault")
            .id,
    );
    Trader {
        who,
        account,
        vault,
    }
}

fn rig_at(ic_ratio: u32) -> Rig {
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
        ic_ratio,
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
    // The venue's one vendor, so luna trades in credits.
    ok(&mut tw, GOD, "@org/create Ares");
    let shop = PrincipalId::Organization(VENDOR_ORG);
    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:#{}={CREDIT}", VENDOR_ORG.0),
    );
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
    let shop_vault = economy(&tw)
        .snapshot()
        .vaults()
        .find(|v| v.owner == shop)
        .expect("the shop's vault")
        .id;
    ok(&mut tw, GOD, &format!("@market/catalog load {PACK}"));
    ok(
        &mut tw,
        GOD,
        &format!(
            "@market/vendor create #{}=#{} name=Ares market=luna",
            VENDOR_ORG.0, shop_vault.0
        ),
    );
    let sal = trader(&mut tw, "Sal", faucet);
    let bea = trader(&mut tw, "Bea", faucet);
    let cy = trader(&mut tw, "Cy", faucet);
    Rig {
        tw,
        families,
        faucet,
        sal,
        bea,
        cy,
    }
}

fn rig() -> Rig {
    rig_at(1)
}

fn take(tw: &mut TestWorld) -> AuthoritativeSubmit {
    *tw.engine_mut()
        .take_authoritative_submit()
        .expect("a submission was staged")
}

/// **Drive one submission the way the world task does** (IC10's `land_submit`).
fn land_submit(tw: &mut TestWorld, submit: &AuthoritativeSubmit, token: u8) -> Result<(), String> {
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
            .map_err(|error| format!("resolve: {error:?}"))?;
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
        .map_err(|error| format!("prepare: {error:?}"))?;
    tw.engine_mut()
        .install_prepared_action(prepared.into_parts().1)
        .map_err(|error| format!("{error:?}"))?;
    Ok(())
}

fn land(tw: &mut TestWorld, token: u8) {
    let submit = take(tw);
    land_submit(tw, &submit, token).expect("the leg lands");
}

/// Move the real clock to `at` and fire whatever is due.
fn at(tw: &mut TestWorld, at: i64) {
    tw.set_now(at);
    let _ = tw.fire_due(at * 1000);
}

fn state_of(tw: &TestWorld, c: ContractId) -> Option<ContractState> {
    economy(tw).snapshot().contract(c).map(|c| c.state)
}

/// Fire the sweeps until the contract settles.
fn settle(tw: &mut TestWorld, contract: ContractId) {
    let start = tw.world().now();
    for i in 1..=12 {
        at(tw, start + i * 31);
        if matches!(state_of(tw, contract), Some(ContractState::Settled { .. })) {
            return;
        }
    }
    panic!(
        "contract #{contract} did not settle on the sweep: {:?}",
        state_of(tw, contract)
    );
}

fn no_settlement_pages(tw: &TestWorld) {
    let pages: Vec<String> = economy(tw)
        .escalations()
        .filter(|e| matches!(e.kind, EscalationKind::SettlementRefused))
        .map(|e| e.context.clone())
        .collect();
    assert!(pages.is_empty(), "the ladder holds a page: {pages:?}");
}

/// The listing a line like "Listed for sale as #12: …" or "Bid #12: …" names.
fn listed_id(said: &str, head: &str) -> ListingId {
    let rest = said
        .split(head)
        .nth(1)
        .unwrap_or_else(|| panic!("no `{head}` in: {said}"));
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    ListingId(digits.parse().expect("a listing number"))
}

/// Sal lists one exact lot, and its hold lands.
fn post_lot(r: &mut Rig, lot: LotId, price: i64, extra: &str, token: u8) -> ListingId {
    let sal = r.sal.who;
    let said = ok(
        &mut r.tw,
        sal,
        &format!("+market/post {lot} price={price} market=luna{extra}"),
    );
    let id = listed_id(&said, "Listed for sale as #");
    land(&mut r.tw, token);
    id
}

/// A buyer quotes `listing` and accepts into their own vault: the contract.
fn buy(r: &mut Rig, buyer: Trader, listing: ListingId, qty: u64) -> ContractId {
    let quoted = ok(
        &mut r.tw,
        buyer.who,
        &format!("+market/quote {listing}={qty}"),
    );
    let quote = quoted
        .split("Quote #")
        .nth(1)
        .and_then(|rest| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse::<u64>()
                .ok()
        })
        .map(QuoteId)
        .expect("a quote number");
    let agreed = ok(
        &mut r.tw,
        buyer.who,
        &format!("+market/accept {quote}={}", buyer.vault.vault().0),
    );
    assert!(agreed.contains("Agreed."), "{agreed}");
    market(&r.tw)
        .quote(quote)
        .and_then(|q| q.contract)
        .expect("the accept drafted a contract")
}

/// The live holds on an account, by id.
fn live_holds(tw: &TestWorld, account: AccountId) -> Vec<mush_economy::Hold> {
    economy(tw)
        .snapshot()
        .holds()
        .filter(|h| h.account == account && h.status.is_held())
        .cloned()
        .collect()
}

// ---- 1: an exact lot, sold while its seller is away ------------------------------

/// **A player lists an exact lot from a vault they may take from, and another player buys
/// it while the seller types nothing** (Q-48-2, -49, -50). The listing is personal (no
/// vendor row), the seller's own account is paid, the lot itself — the same id — reaches
/// the buyer's vault, and the sale contract records the seller's acceptance, given by the
/// listing's standing.
#[test]
fn ex1a_a_player_lists_an_exact_lot_and_another_buys_it_while_the_seller_is_away() {
    let mut r = rig();
    let lot = LotId(0x5a1);
    let (sal, bea) = (r.sal, r.bea);
    mint_lot(&mut r.tw, &r.families, sal.vault, "iron", 10, lot);
    let vendors = market(&r.tw).vendors().count();

    let said = ok(
        &mut r.tw,
        sal.who,
        &format!("+market/post {lot} price=700 market=luna"),
    );
    assert!(said.contains("Listed for sale as #"), "{said}");
    assert!(said.contains("10 Iron, graded at 7.00cr each."), "{said}");
    assert!(said.contains("It trades while you are away."), "{said}");
    let id = listed_id(&said, "Listed for sale as #");
    land(&mut r.tw, 0x11);
    let listing = market(&r.tw).listing(id).cloned().expect("listed");
    assert!(
        listing.is_personal(),
        "a personal seller is not a vendor row"
    );
    assert_eq!(listing.vendor, NO_VENDOR);
    assert_eq!(listing.account, Some(sal.account));
    assert_eq!(listing.seller, PrincipalId::Object(sal.who));
    assert_eq!(listing.source.exact_lot(), Some(lot));
    assert_eq!(
        market(&r.tw).vendors().count(),
        vendors,
        "no vendor row was made"
    );
    assert_eq!(market(&r.tw).listings_of_seller(&listing.seller), 1);

    // From here Sal types nothing.
    let contract = buy(&mut r, bea, id, 10);
    let sale = economy(&r.tw)
        .snapshot()
        .contract(contract)
        .cloned()
        .expect("the contract");
    assert!(
        sale.acceptances
            .iter()
            .any(|a| a.party == PrincipalId::Object(sal.who)),
        "the seller's acceptance stands on the contract: {:?}",
        sale.acceptances
    );
    land(&mut r.tw, 0x22);
    settle(&mut r.tw, contract);
    assert_eq!(posted(&r.tw, sal.account), FUNDS + 7_000);
    assert_eq!(posted(&r.tw, bea.account), FUNDS - 7_000);
    assert_eq!(
        lot_at(&mut r.tw, &r.families, lot),
        Some(bea.vault),
        "the lot itself reached the buyer"
    );
    assert!(
        market(&r.tw).listing(id).is_none(),
        "the sale took the listing"
    );
    no_settlement_pages(&r.tw);
}

/// **A listing outlives seven real hours** at 24:1 (Q-48-54's first point): an exact lot's
/// hold has no clock of its own, so neither a personal listing nor a vendor's exact-lot
/// listing is cut to the inventory reservation's seven IC days. Before EX.1a both lapsed at
/// 25,200 real seconds.
#[test]
fn ex1a_a_listing_outlives_seven_real_hours() {
    let mut r = rig_at(24);
    let (sal, bea) = (r.sal, r.bea);
    let lot = LotId(0x5a2);
    mint_lot(&mut r.tw, &r.families, sal.vault, "iron", 5, lot);
    let id = post_lot(&mut r, lot, 700, "", 0x11);
    // A vendor's exact lot, stocked by staff.
    let shop = economy(&r.tw)
        .snapshot()
        .vaults()
        .find(|v| v.owner == PrincipalId::Organization(VENDOR_ORG))
        .expect("the shop")
        .id;
    let shop_lot = LotId(0x5a3);
    mint_lot(
        &mut r.tw,
        &r.families,
        EndpointRef(shop),
        "iron",
        3,
        shop_lot,
    );
    let vendor = market(&r.tw).vendor_of_org(VENDOR_ORG).expect("vendor").id;
    let stocked = ok(
        &mut r.tw,
        GOD,
        &format!(
            "@market/vendor stock #{}=ex1a:iron price=900 qty=3 endpoint={} lot={shop_lot}",
            vendor.0, shop.0
        ),
    );
    assert!(stocked.contains("published"), "{stocked}");
    land(&mut r.tw, 0x12);
    let vendor_listing = market(&r.tw)
        .listings_for(
            &mush_economy::CatalogRef {
                ruleset: "ex1a".into(),
                code: "iron".into(),
            },
            &mush_market::MarketId::new("luna").expect("venue"),
        )
        .find(|l| !l.is_personal())
        .map(|l| l.id)
        .expect("the vendor's listing");
    let seven_hours = mush_market::hold_ttl_real_secs(r.tw.world()).expect("a running clock");
    assert_eq!(seven_hours, 7 * 3_600, "seven IC days at 24:1");

    // Eight real hours on, with every sweep fired.
    for step in 1..=16 {
        at(&mut r.tw, T0 + step * 1_800);
    }
    assert!(
        market(&r.tw).listing(id).is_some(),
        "the personal listing stands"
    );
    assert!(
        market(&r.tw).listing(vendor_listing).is_some(),
        "the vendor's exact-lot listing stands"
    );
    let contract = buy(&mut r, bea, id, 5);
    land(&mut r.tw, 0x22);
    settle(&mut r.tw, contract);
    assert_eq!(lot_at(&mut r.tw, &r.families, lot), Some(bea.vault));
    let late = buy(&mut r, bea, vendor_listing, 3);
    land(&mut r.tw, 0x23);
    settle(&mut r.tw, late);
    assert_eq!(lot_at(&mut r.tw, &r.families, shop_lot), Some(bea.vault));
}

// ---- 2: an escrowed bid ----------------------------------------------------------

/// **A player's bid escrows at posting and fills** (Q-48-50), and **its hold converts to
/// escrow exactly, in one plan**: right after a sale into it the bidder's held amount is
/// unchanged (the sale's escrow plus the rest of the bid), both new holds come from one
/// placing plan, the bid's old hold is released by that same plan, and the listing names
/// the new hold. Each settlement moves money once; the last fill takes the listing and
/// leaves nothing held.
#[test]
fn ex1a_a_bid_escrows_at_posting_and_fills_in_one_plan() {
    let mut r = rig();
    let (sal, bea) = (r.sal, r.bea);
    seed_pool(&mut r.tw, &r.families, sal.vault, "iron", 20, LotId(0xb0));

    let said = ok(
        &mut r.tw,
        bea.who,
        &format!(
            "+market/bid ex1a:iron qty=10 price=500 into={} market=luna",
            bea.vault.vault().0
        ),
    );
    assert!(
        said.contains("10 Iron, graded at 5.00cr each; 50.00cr is held from your account."),
        "{said}"
    );
    let bid = listed_id(&said, "Bid #");
    assert_eq!(
        held(&r.tw, bea.account),
        5_000,
        "the bid is escrowed at posting"
    );
    let first = market(&r.tw)
        .listing(bid)
        .and_then(|l| l.escrow)
        .expect("an escrow");
    assert_eq!(
        live_holds(&r.tw, bea.account)
            .iter()
            .map(|h| (h.id, h.amount.minor()))
            .collect::<Vec<_>>(),
        vec![(first, 5_000)]
    );

    // Sal sells four into it; Bea types nothing.
    let sold = ok(
        &mut r.tw,
        sal.who,
        &format!("+market/sell {bid}=4 from={}", sal.vault.vault().0),
    );
    assert!(sold.contains("Sold 4"), "{sold}");
    let holds = live_holds(&r.tw, bea.account);
    assert_eq!(
        held(&r.tw, bea.account),
        5_000,
        "no gap and no double hold: {holds:?}"
    );
    assert_eq!(holds.len(), 2, "{holds:?}");
    let escrow = holds
        .iter()
        .find(|h| matches!(h.purpose, mush_economy::HoldPurpose::Escrow { .. }))
        .expect("the sale's escrow");
    let rest = holds
        .iter()
        .find(|h| matches!(h.purpose, mush_economy::HoldPurpose::Reservation { .. }))
        .expect("the rest of the bid");
    assert_eq!(escrow.amount.minor(), 2_000);
    assert_eq!(rest.amount.minor(), 3_000);
    assert_eq!(escrow.holder, rest.holder, "one placing plan");
    let old = economy(&r.tw)
        .snapshot()
        .hold(first)
        .cloned()
        .expect("the old hold");
    assert!(
        matches!(old.status, mush_economy::HoldStatus::Released),
        "{old:?}"
    );
    assert_eq!(
        old.last_journal_seq, rest.last_journal_seq,
        "released by the same plan that placed the rest"
    );
    let listing = market(&r.tw).listing(bid).cloned().expect("the bid stands");
    assert_eq!(listing.quantity, 6);
    assert_eq!(listing.escrow, Some(rest.id), "the bid names its new hold");

    let contract = market(&r.tw)
        .quotes_of_seller(&PrincipalId::Object(sal.who))
        .last()
        .and_then(|q| q.contract)
        .expect("the sale's contract");
    land(&mut r.tw, 0x31);
    settle(&mut r.tw, contract);
    assert_eq!(posted(&r.tw, sal.account), FUNDS + 2_000);
    assert_eq!(posted(&r.tw, bea.account), FUNDS - 2_000);
    assert_eq!(held(&r.tw, bea.account), 3_000);
    assert_eq!(pool(&mut r.tw, &r.families, bea.vault, "iron"), 4);

    // The last six: the bid is drawn to nothing and goes.
    ok(
        &mut r.tw,
        sal.who,
        &format!("+market/sell {bid}=6 from={}", sal.vault.vault().0),
    );
    assert!(
        market(&r.tw).listing(bid).is_none(),
        "an exhausted personal bid goes"
    );
    let last = market(&r.tw)
        .quotes_of_seller(&PrincipalId::Object(sal.who))
        .last()
        .and_then(|q| q.contract)
        .expect("the last sale");
    land(&mut r.tw, 0x32);
    settle(&mut r.tw, last);
    assert_eq!(held(&r.tw, bea.account), 0);
    assert_eq!(posted(&r.tw, bea.account), FUNDS - 5_000);
    assert_eq!(pool(&mut r.tw, &r.families, bea.vault, "iron"), 10);
    no_settlement_pages(&r.tw);
}

/// **A bid releases what it holds at withdrawal and at expiry** (Q-48-50).
#[test]
fn ex1a_a_bid_releases_its_escrow_at_withdrawal_and_expiry() {
    let mut r = rig();
    let bea = r.bea;
    let said = ok(
        &mut r.tw,
        bea.who,
        &format!(
            "+market/bid ex1a:iron qty=10 price=500 into={} market=luna",
            bea.vault.vault().0
        ),
    );
    let bid = listed_id(&said, "Bid #");
    assert_eq!(held(&r.tw, bea.account), 5_000);
    let gone = ok(&mut r.tw, bea.who, &format!("+market/withdraw {bid}"));
    assert!(gone.contains("what it still held is yours again"), "{gone}");
    assert_eq!(held(&r.tw, bea.account), 0);
    assert!(market(&r.tw).listing(bid).is_none());

    // A bid left to lapse: its hold ages out at the listing's own deadline.
    let said = ok(
        &mut r.tw,
        bea.who,
        &format!(
            "+market/bid ex1a:iron qty=2 price=500 into={} market=luna days=1",
            bea.vault.vault().0
        ),
    );
    let lapsing = listed_id(&said, "Bid #");
    assert_eq!(held(&r.tw, bea.account), 1_000);
    for step in 1..=30 {
        at(&mut r.tw, T0 + 86_400 + step * 60);
    }
    assert_eq!(
        held(&r.tw, bea.account),
        0,
        "the escrow lapsed with the bid"
    );
    assert!(market(&r.tw).listing(lapsing).is_none());
}

// ---- 3: a private offer --------------------------------------------------------

/// **A private offer is refused to a third party at quote** (Q-48-5): Cy cannot see it,
/// show it or quote it; staff can see it and cannot buy it; Bea, its one buyer, buys it.
#[test]
fn ex1a_a_private_offer_is_refused_to_a_third_party_at_quote() {
    let mut r = rig();
    let (sal, bea, cy) = (r.sal, r.bea, r.cy);
    let lot = LotId(0x5b1);
    mint_lot(&mut r.tw, &r.families, sal.vault, "iron", 2, lot);
    let said = ok(
        &mut r.tw,
        sal.who,
        &format!("+market/post {lot} price=700 market=luna to=Bea"),
    );
    assert!(said.contains(" (a private offer to Bea)."), "{said}");
    let id = listed_id(&said, "Listed for sale as #");
    land(&mut r.tw, 0x11);
    let listing = market(&r.tw).listing(id).cloned().expect("listed");
    assert_eq!(listing.private_to, Some(PrincipalId::Object(bea.who)));
    assert_eq!(listing.visibility, mush_market::MarketVisibility::Hidden);

    let page = ok(&mut r.tw, cy.who, "+market/list market=luna");
    assert!(!page.contains(&format!("#{id} ")), "{page}");
    let shown = err(&mut r.tw, cy.who, &format!("+market/show {id}"));
    assert!(shown.contains(NO_SUCH_LISTING), "{shown}");
    let quoted = err(&mut r.tw, cy.who, &format!("+market/quote {id}=2"));
    assert!(quoted.contains(NO_SUCH_LISTING), "{quoted}");
    let staff = err(&mut r.tw, GOD, &format!("+market/quote {id}=2"));
    assert!(
        staff.contains(mush_market::seller::PRIVATE_TO_OTHER),
        "{staff}"
    );
    let bea_page = ok(&mut r.tw, bea.who, "+market/list market=luna");
    assert!(
        bea_page.contains(mush_market::seller::PRIVATE_MARK),
        "{bea_page}"
    );

    let contract = buy(&mut r, bea, id, 2);
    land(&mut r.tw, 0x22);
    settle(&mut r.tw, contract);
    assert_eq!(lot_at(&mut r.tw, &r.families, lot), Some(bea.vault));
    assert_eq!(posted(&r.tw, cy.account), FUNDS, "Cy paid nothing");
}

/// **A private offer is refused to a third party at accept**, whatever quote they hold
/// (doc 48a A-14): a quote document made Cy's by a hand edit across a reboot is refused at
/// `+market/accept` in the offer's own words, with nothing drafted or charged.
#[test]
fn ex1a_a_private_offer_is_refused_to_a_third_party_at_accept() {
    let mut r = rig();
    let (sal, bea, cy) = (r.sal, r.bea, r.cy);
    let lot = LotId(0x5b2);
    mint_lot(&mut r.tw, &r.families, sal.vault, "iron", 2, lot);
    let id = post_lot(&mut r, lot, 700, " to=Bea", 0x11);
    let quoted = ok(&mut r.tw, bea.who, &format!("+market/quote {id}=2"));
    assert!(quoted.contains("Quote #0"), "{quoted}");
    // Cy cannot take up Bea's quote as it stands…
    let theirs = err(
        &mut r.tw,
        cy.who,
        &format!("+market/accept 0={}", cy.vault.vault().0),
    );
    assert!(theirs.contains(mush_market::NO_SUCH_QUOTE), "{theirs}");
    // …nor once the document says it is theirs.
    let ops = r.tw.take_module_ops();
    let mut boot = mush_testkit::fold_module_ops(ops);
    {
        let docs = boot.get_mut(mush_market::NS).expect("market documents");
        let mut found = false;
        for (key, value) in &mut docs.kv {
            if key.as_str() == "market:quote:0" {
                value["buyer"] = json!({ "Object": cy.who.0 });
                found = true;
            }
        }
        assert!(found, "the quote document was folded");
    }
    r.tw.modules_loaded(boot);
    assert_eq!(
        market(&r.tw).quote(QuoteId(0)).map(|q| q.buyer.clone()),
        Some(PrincipalId::Object(cy.who))
    );
    let contracts = economy(&r.tw).snapshot().contracts().count();
    let refused = err(
        &mut r.tw,
        cy.who,
        &format!("+market/accept 0={}", cy.vault.vault().0),
    );
    assert!(
        refused.contains(mush_market::seller::PRIVATE_TO_OTHER),
        "{refused}"
    );
    assert_eq!(economy(&r.tw).snapshot().contracts().count(), contracts);
    assert_eq!(posted(&r.tw, cy.account), FUNDS);
    assert_eq!(held(&r.tw, cy.account), 0);
    assert!(
        market(&r.tw).listing(id).is_some(),
        "the offer still stands"
    );
}

/// **A private bid is sold into only by its named seller** (Q-48-5, every kind).
#[test]
fn ex1a_a_private_bid_takes_only_its_named_seller() {
    let mut r = rig();
    let (sal, bea, cy) = (r.sal, r.bea, r.cy);
    seed_pool(&mut r.tw, &r.families, cy.vault, "iron", 5, LotId(0xc0));
    seed_pool(&mut r.tw, &r.families, sal.vault, "iron", 5, LotId(0xc1));
    let said = ok(
        &mut r.tw,
        bea.who,
        &format!(
            "+market/bid ex1a:iron qty=5 price=500 into={} market=luna to=Sal",
            bea.vault.vault().0
        ),
    );
    let bid = listed_id(&said, "Bid #");
    // A stranger cannot see it, so the bare number reads as no such thing at all.
    let refused = err(
        &mut r.tw,
        cy.who,
        &format!("+market/sell {bid}=1 from={}", cy.vault.vault().0),
    );
    assert!(refused.contains("No such asset."), "{refused}");
    assert_eq!(pool(&mut r.tw, &r.families, cy.vault, "iron"), 5);
    ok(
        &mut r.tw,
        sal.who,
        &format!("+market/sell {bid}=1 from={}", sal.vault.vault().0),
    );
}

// ---- 4: the standing acceptance's own guards -----------------------------------

/// **The persisted documents, kept across hand edits**: every op drained since the rig
/// booted, folded into one boot map, so each edit reboots the modules over the whole store
/// (a fold of only the ops since the last reboot would boot an almost empty one).
struct Docs {
    boot: std::collections::HashMap<smol_str::SmolStr, mush_core::ModuleBootData>,
}

impl Docs {
    fn new(tw: &mut TestWorld) -> Docs {
        Docs {
            boot: mush_testkit::fold_module_ops(tw.take_module_ops()),
        }
    }

    /// Set `path` (dot-separated) of the market document `key` to `value`, and reboot.
    fn edit(&mut self, tw: &mut TestWorld, key: &str, path: &str, value: serde_json::Value) {
        mush_testkit::fold_module_ops_into(&mut self.boot, tw.take_module_ops());
        let docs = self
            .boot
            .get_mut(mush_market::NS)
            .expect("market documents");
        let doc = docs
            .kv
            .iter_mut()
            .find(|(k, _)| k.as_str() == key)
            .map(|(_, doc)| doc)
            .unwrap_or_else(|| panic!("{key} was not folded"));
        let mut at = doc;
        let parts: Vec<&str> = path.split('.').collect();
        for part in &parts[..parts.len() - 1] {
            at = &mut at[*part];
        }
        at[*parts.last().expect("a field")] = value;
        tw.modules_loaded(self.boot.clone());
    }

    /// Put a whole market document back, and reboot.
    fn put(&mut self, tw: &mut TestWorld, key: &str, value: serde_json::Value) {
        mush_testkit::fold_module_ops_into(&mut self.boot, tw.take_module_ops());
        let docs = self.boot.entry(mush_market::NS.into()).or_default();
        docs.kv.retain(|(k, _)| k.as_str() != key);
        docs.kv.push((key.into(), value));
        tw.modules_loaded(self.boot.clone());
    }
}

/// What `ListingAcceptance` answers for `contract` (the stored one, or `with` it changed).
fn stands_for(
    tw: &TestWorld,
    contract: ContractId,
    party: ObjId,
    against: Option<HoldId>,
    with: impl FnOnce(&mut mush_economy::EconomicContract),
) -> bool {
    let mut c = economy(tw)
        .snapshot()
        .contract(contract)
        .cloned()
        .expect("the contract");
    with(&mut c);
    ListingAcceptance.stands(tw.world(), &c, &PrincipalId::Object(party), against)
}

/// **A bid's standing acceptance says yes for exactly its terms** (Q-48-50), probed on a
/// real sale into it: refused as the bid stands right after the sale (a stale listing
/// revision); then, the quote's stamp brought level by a hand edit, yes for the bidder
/// against the bid's own escrow, and no for a third party, the other party, no backing hold,
/// a hold that is not the bid's, a listing at another price, a listing short of the quote's
/// quantity, and a withdrawn listing.
#[test]
fn ex1a_the_standing_acceptance_holds_a_bid_to_its_terms() {
    let mut r = rig();
    let (sal, bea, cy) = (r.sal, r.bea, r.cy);
    seed_pool(&mut r.tw, &r.families, sal.vault, "iron", 10, LotId(0xd0));
    let said = ok(
        &mut r.tw,
        bea.who,
        &format!(
            "+market/bid ex1a:iron qty=10 price=500 into={} market=luna",
            bea.vault.vault().0
        ),
    );
    let bid = listed_id(&said, "Bid #");
    ok(
        &mut r.tw,
        sal.who,
        &format!("+market/sell {bid}=4 from={}", sal.vault.vault().0),
    );
    let _ = r.tw.engine_mut().take_authoritative_submit();
    let quote = market(&r.tw)
        .quotes_of_seller(&PrincipalId::Object(sal.who))
        .last()
        .cloned()
        .expect("the sale's quote");
    let contract = quote.contract.expect("the contract");
    let escrow = market(&r.tw).listing(bid).and_then(|l| l.escrow);
    let stands = |tw: &TestWorld, party: ObjId, against: Option<HoldId>| {
        stands_for(tw, contract, party, against, |_| {})
    };
    assert!(
        !stands(&r.tw, bea.who, escrow),
        "the listing moved on (a stale revision): no standing"
    );

    let mut docs = Docs::new(&mut r.tw);
    let quote_key = format!("market:quote:{}", quote.id.0);
    let listing_key = format!("market:listing:{}", bid.0);
    let revision = market(&r.tw).listing(bid).map(|l| l.revision).expect("bid");
    docs.edit(&mut r.tw, &quote_key, "observed.listing", json!(revision));
    assert!(
        stands(&r.tw, bea.who, escrow),
        "the bidder, against the bid's own escrow"
    );
    assert!(!stands(&r.tw, cy.who, escrow), "a third party");
    assert!(!stands(&r.tw, sal.who, escrow), "the other party");
    assert!(!stands(&r.tw, bea.who, None), "no backing hold");
    let other = economy(&r.tw)
        .snapshot()
        .holds()
        .map(|h| h.id)
        .find(|h| Some(*h) != escrow)
        .expect("another hold");
    assert!(
        !stands(&r.tw, bea.who, Some(other)),
        "a hold that is not the bid's"
    );
    assert!(
        !stands_for(&r.tw, contract, bea.who, escrow, |c| {
            c.consent.terms.consideration[0].amount = mush_economy::Amount::from_minor(1_999);
        }),
        "a consideration that is not the quote's"
    );
    // Doc 48e (exchange G26; 48d M-07): the payouts are the ones the market builds.
    assert!(
        !stands_for(&r.tw, contract, bea.who, escrow, |c| {
            c.consent.terms.outcomes[0].payouts[0].share =
                mush_economy::Share::Exact(mush_economy::Amount::from_minor(1_999));
        }),
        "an outcome that pays the seller other than the market built"
    );
    assert!(
        !stands_for(&r.tw, contract, bea.who, escrow, |c| {
            let to = c.consent.terms.outcomes[0].payouts[0].to;
            c.consent.terms.outcomes[0]
                .payouts
                .push(mush_economy::PayoutShare {
                    to,
                    share: mush_economy::Share::Exact(mush_economy::Amount::from_minor(1)),
                });
        }),
        "an outcome with a payee the market did not build"
    );
    docs.edit(&mut r.tw, &listing_key, "price", json!(501));
    assert!(
        !stands(&r.tw, bea.who, escrow),
        "a listing at another price"
    );
    docs.edit(&mut r.tw, &listing_key, "price", json!(500));
    assert!(stands(&r.tw, bea.who, escrow));
    docs.edit(&mut r.tw, &listing_key, "quantity", json!(3));
    assert!(
        !stands(&r.tw, bea.who, escrow),
        "a listing short of the quote's quantity"
    );
    docs.edit(&mut r.tw, &listing_key, "quantity", json!(6));
    assert!(stands(&r.tw, bea.who, escrow));
    ok(&mut r.tw, bea.who, &format!("+market/withdraw {bid}"));
    assert!(!stands(&r.tw, bea.who, escrow), "a withdrawn listing");
}

/// **An ask's standing acceptance says yes for exactly its terms** (Q-48-50), probed on a
/// real purchase with its listing put back as it stood: yes for the seller with no backing
/// hold; no with a backing hold, for the buyer, for a third party, and for a contract that
/// pays the seller's account anything but the whole price, takes less from the buyer, has
/// another issuer or asks something of the seller.
#[test]
fn ex1a_the_standing_acceptance_holds_an_ask_to_its_terms() {
    let mut r = rig();
    let (sal, bea, cy) = (r.sal, r.bea, r.cy);
    let lot = LotId(0x5d1);
    mint_lot(&mut r.tw, &r.families, sal.vault, "iron", 4, lot);
    let id = post_lot(&mut r, lot, 700, "", 0x11);
    let listing =
        serde_json::to_value(market(&r.tw).listing(id).cloned().expect("listed")).expect("json");
    let mut docs = Docs::new(&mut r.tw);
    let contract = buy(&mut r, bea, id, 4);
    let _ = r.tw.engine_mut().take_authoritative_submit();
    // Doc 48e (exchange G2): the sale lapses the listing (its hold is the Consume's).
    let now = r.tw.world().now();
    assert!(
        market(&r.tw)
            .listing(id)
            .is_none_or(|l| mush_market::listing_lapsed(l, now)),
        "the sale took the listing off the board"
    );
    assert!(
        !stands_for(&r.tw, contract, sal.who, None, |_| {}),
        "a sold listing no longer stands"
    );
    docs.put(&mut r.tw, &format!("market:listing:{}", id.0), listing);
    let stands = |tw: &TestWorld, party: ObjId, against: Option<HoldId>| {
        stands_for(tw, contract, party, against, |_| {})
    };
    assert!(stands(&r.tw, sal.who, None), "the seller, owing nothing");
    let any = economy(&r.tw)
        .snapshot()
        .holds()
        .map(|h| h.id)
        .next()
        .expect("a hold");
    assert!(
        !stands(&r.tw, sal.who, Some(any)),
        "an ask draws on no hold"
    );
    assert!(
        !stands(&r.tw, bea.who, None),
        "the buyer has no standing on an ask"
    );
    assert!(!stands(&r.tw, cy.who, None), "a third party");
    let changed = |with: fn(&mut mush_economy::EconomicContract, AccountId, ObjId)| {
        stands_for(&r.tw, contract, sal.who, None, |c| {
            with(c, cy.account, cy.who)
        })
    };
    assert!(
        !changed(|c, cy, _| c.consent.terms.outcomes[0].payouts[0].to = cy),
        "paying someone else"
    );
    assert!(
        !changed(|c, _, _| {
            c.consent.terms.outcomes[0].payouts[0].share =
                mush_economy::Share::Exact(mush_economy::Amount::from_minor(2_799));
        }),
        "paying the seller less than the price"
    );
    assert!(
        !changed(|c, _, _| {
            c.consent.terms.consideration[0].amount = mush_economy::Amount::from_minor(2_799);
        }),
        "taking less from the buyer"
    );
    assert!(
        !changed(|c, _, cy| c.issuer = PrincipalId::Object(cy)),
        "another issuer"
    );
    let sal_account = sal.account;
    assert!(
        !stands_for(&r.tw, contract, sal.who, None, |c| {
            c.consent
                .terms
                .consideration
                .push(mush_economy::Consideration {
                    from: PrincipalId::Object(sal.who),
                    account: sal_account,
                    amount: mush_economy::Amount::from_minor(1),
                });
        }),
        "asking something of the seller"
    );
}

/// **A listing whose terms moved under a standing quote is refused at accept** (Q-48-50):
/// the listing's price changed by a hand edit that left its revision alone passes the
/// quote's own checks, and the seller's standing acceptance, asked of the very contract,
/// refuses it; the contract is cancelled and nothing is charged.
#[test]
fn ex1a_a_listing_whose_terms_moved_under_its_quote_is_refused_at_accept() {
    let mut r = rig();
    let (sal, bea) = (r.sal, r.bea);
    let lot = LotId(0x5d2);
    mint_lot(&mut r.tw, &r.families, sal.vault, "iron", 2, lot);
    let id = post_lot(&mut r, lot, 700, "", 0x11);
    ok(&mut r.tw, bea.who, &format!("+market/quote {id}=2"));
    let mut docs = Docs::new(&mut r.tw);
    docs.edit(
        &mut r.tw,
        &format!("market:listing:{}", id.0),
        "price",
        json!(800),
    );
    let contracts = economy(&r.tw).snapshot().contracts().count();
    let refused = err(
        &mut r.tw,
        bea.who,
        &format!("+market/accept 0={}", bea.vault.vault().0),
    );
    assert!(
        refused.contains(mush_market::seller::SELLER_GONE),
        "{refused}"
    );
    let live = economy(&r.tw)
        .snapshot()
        .contracts()
        .filter(|c| !c.state.is_terminal())
        .count();
    assert_eq!(live, 0, "the drafted contract was canceled");
    assert!(economy(&r.tw).snapshot().contracts().count() >= contracts);
    assert_eq!(held(&r.tw, bea.account), 0);
    assert_eq!(posted(&r.tw, bea.account), FUNDS);
    assert_eq!(lot_at(&mut r.tw, &r.families, lot), Some(sal.vault));
}

/// **A backing hold that is not the bidder's own bid is refused** (Q-48-50, the economy's
/// half): a bid whose document names a hold Bea placed by hand (`@hold/place`, a manual
/// reservation) passes the market's own checks and is refused by
/// `accept_contract_standing`, so the sale is cancelled with nothing moved.
#[test]
fn ex1a_a_backing_hold_that_is_not_the_bid_is_refused() {
    let mut r = rig();
    let (sal, bea) = (r.sal, r.bea);
    seed_pool(&mut r.tw, &r.families, sal.vault, "iron", 10, LotId(0xe0));
    let said = ok(
        &mut r.tw,
        bea.who,
        &format!(
            "+market/bid ex1a:iron qty=10 price=500 into={} market=luna",
            bea.vault.vault().0
        ),
    );
    let bid = listed_id(&said, "Bid #");
    let placed = ok(
        &mut r.tw,
        bea.who,
        &format!("@hold/place {}=6000", bea.account.0),
    );
    let manual = placed
        .split("hold #")
        .nth(1)
        .and_then(|rest| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse::<u64>()
                .ok()
        })
        .map(HoldId)
        .expect("a hold number");
    let mut docs = Docs::new(&mut r.tw);
    docs.edit(
        &mut r.tw,
        &format!("market:listing:{}", bid.0),
        "escrow",
        json!(manual.0),
    );
    assert_eq!(
        market(&r.tw).listing(bid).and_then(|l| l.escrow),
        Some(manual)
    );
    let live = |tw: &TestWorld| {
        economy(tw)
            .snapshot()
            .contracts()
            .filter(|c| !c.state.is_terminal())
            .count()
    };
    let before = live(&r.tw);
    let refused = err(
        &mut r.tw,
        sal.who,
        &format!("+market/sell {bid}=4 from={}", sal.vault.vault().0),
    );
    assert!(
        refused.contains(mush_market::seller::BIDDER_NOT_BUYING),
        "{refused}"
    );
    assert_eq!(live(&r.tw), before, "the sale was canceled");
    let manual_now = economy(&r.tw)
        .snapshot()
        .hold(manual)
        .cloned()
        .expect("the manual hold");
    assert!(
        manual_now.status.is_held(),
        "the hand-placed hold was not touched"
    );
    assert_eq!(posted(&r.tw, sal.account), FUNDS);
    assert_eq!(pool(&mut r.tw, &r.families, sal.vault, "iron"), 10);
}

/// **A sale into a bid whose goods never move refunds, and the bid is not restored**
/// (doc 48 cut EX.1a): the refund returns the sale's escrow to the bidder's available
/// balance, not to the bid's hold, so the intake it would restore is no longer backed. The
/// bid stands at what its escrow still covers.
#[test]
fn ex1a_an_unlanded_sale_into_a_bid_refunds_and_the_bid_is_not_restored() {
    let mut r = rig();
    let (sal, bea) = (r.sal, r.bea);
    seed_pool(&mut r.tw, &r.families, sal.vault, "iron", 10, LotId(0xe1));
    let said = ok(
        &mut r.tw,
        bea.who,
        &format!(
            "+market/bid ex1a:iron qty=10 price=500 into={} market=luna",
            bea.vault.vault().0
        ),
    );
    let bid = listed_id(&said, "Bid #");
    ok(
        &mut r.tw,
        sal.who,
        &format!("+market/sell {bid}=4 from={}", sal.vault.vault().0),
    );
    let _ = r.tw.engine_mut().take_authoritative_submit();
    let contract = market(&r.tw)
        .quotes_of_seller(&PrincipalId::Object(sal.who))
        .last()
        .and_then(|q| q.contract)
        .expect("the sale's contract");
    let rest = market(&r.tw).listing(bid).and_then(|l| l.escrow);
    let window = mush_market::DRAW_WINDOW_SECS;
    for step in 1..=((window + 900) / 30) {
        at(&mut r.tw, T0 + step * 30);
    }
    assert!(
        matches!(
            state_of(&r.tw, contract),
            Some(ContractState::Expired | ContractState::Cancelled)
        ),
        "{:?}",
        state_of(&r.tw, contract)
    );
    assert!(
        market(&r.tw)
            .quotes_of_seller(&PrincipalId::Object(sal.who))
            .next()
            .is_none(),
        "the sale was released"
    );
    let listing = market(&r.tw).listing(bid).cloned().expect("the bid stands");
    assert_eq!(
        listing.quantity, 6,
        "not restored to an intake no hold backs"
    );
    assert_eq!(listing.escrow, rest);
    assert_eq!(
        held(&r.tw, bea.account),
        3_000,
        "the bid's hold, and nothing else"
    );
    assert_eq!(
        posted(&r.tw, bea.account),
        FUNDS,
        "the sale's escrow came back"
    );
    assert_eq!(pool(&mut r.tw, &r.families, sal.vault, "iron"), 10);
}

// ---- 5: the caps --------------------------------------------------------------

/// **At most 32 open listings a seller** (Q-48-44): asks and bids together; the 33rd is
/// refused with the cap's own line, and a withdrawal makes room again.
#[test]
fn ex1a_a_seller_holds_at_most_32_listings() {
    let mut r = rig();
    let sal = r.sal;
    let mut first = None;
    for n in 0..MAX_LISTINGS_PER_SELLER {
        let lot = LotId(0x1000 + u64::try_from(n).expect("n"));
        mint_lot(&mut r.tw, &r.families, sal.vault, "iron", 1, lot);
        let id = post_lot(&mut r, lot, 100, "", 0x40);
        first.get_or_insert(id);
    }
    assert_eq!(
        market(&r.tw).listings_of_seller(&PrincipalId::Object(sal.who)),
        MAX_LISTINGS_PER_SELLER
    );
    let refused = err(
        &mut r.tw,
        sal.who,
        &format!(
            "+market/bid ex1a:iron qty=1 price=100 into={} market=luna",
            sal.vault.vault().0
        ),
    );
    assert!(
        refused.contains("You hold 32 listings, as many as a seller may."),
        "{refused}"
    );
    let mine = ok(&mut r.tw, sal.who, "+market/mine");
    assert!(
        mine.contains("You hold 32 of the 32 listings a seller may."),
        "{mine}"
    );
    let first = first.expect("listed");
    ok(&mut r.tw, sal.who, &format!("+market/withdraw {first}"));
    land(&mut r.tw, 0x41);
    ok(
        &mut r.tw,
        sal.who,
        &format!(
            "+market/bid ex1a:iron qty=1 price=100 into={} market=luna",
            sal.vault.vault().0
        ),
    );
}

/// **The store's 1,024 ceiling under 32 sellers' load** (Q-48-44, doc 48b G-5): 32 sellers
/// — more than the 30-hull harness cohort — each at the 32-listing cap fill
/// [`MAX_LISTINGS`] exactly; the next listing is refused "The market is full"; the page,
/// the projection and a sweep stay bounded and the indexes agree; trade goes on at the
/// ceiling (a sale frees a place, and the place is taken again).
#[test]
fn ex1a_the_store_ceiling_holds_under_32_sellers_load() {
    let mut r = rig();
    let sellers = MAX_LISTINGS / MAX_LISTINGS_PER_SELLER;
    assert_eq!(sellers, 32);
    let faucet = r.faucet;
    let mut traders = Vec::new();
    let started = Instant::now();
    for s in 0..sellers {
        let t = trader(&mut r.tw, &format!("Seller{s:02}"), faucet);
        for g in 0..GOODS {
            let said = ok(
                &mut r.tw,
                t.who,
                &format!(
                    "+market/bid ex1a:{} qty=1 price=1 into={} market=luna",
                    good(g),
                    t.vault.vault().0
                ),
            );
            assert!(said.contains("Bid #"), "{said}");
        }
        traders.push(t);
    }
    let filled = started.elapsed();
    assert_eq!(market(&r.tw).listing_count(), MAX_LISTINGS);
    let late = r.bea;
    let full = err(
        &mut r.tw,
        late.who,
        &format!(
            "+market/bid ex1a:iron qty=1 price=1 into={} market=luna",
            late.vault.vault().0
        ),
    );
    assert!(full.contains("The market is full; try later."), "{full}");
    let lot = LotId(0x7777);
    mint_lot(&mut r.tw, &r.families, r.sal.vault, "iron", 1, lot);
    let full_ask = err(
        &mut r.tw,
        r.sal.who,
        &format!("+market/post {lot} price=100 market=luna"),
    );
    assert!(
        full_ask.contains("The market is full; try later."),
        "{full_ask}"
    );
    assert!(market(&r.tw).indexes_agree());

    // The page and a sweep at the ceiling.
    let t = Instant::now();
    let page = ok(&mut r.tw, late.who, "+market/list market=luna");
    let listed = t.elapsed();
    let rows = page.lines().filter(|l| l.starts_with('#')).count();
    assert!(rows <= mush_market::MARKET_PAGE_MAX, "{rows}");
    let t = Instant::now();
    at(&mut r.tw, T0 + 61);
    let swept = t.elapsed();
    eprintln!(
        "ex1a load: {MAX_LISTINGS} listings by {sellers} sellers in {filled:?}; \
         +market/list {listed:?}; one sweep fire {swept:?}"
    );

    // Trade goes on at the ceiling: a seller withdraws, a place opens and is taken.
    let first = traders[0];
    let theirs = market(&r.tw)
        .personal_listings_of(&PrincipalId::Object(first.who))
        .next()
        .map(|l| l.id)
        .expect("a bid");
    ok(&mut r.tw, first.who, &format!("+market/withdraw {theirs}"));
    ok(
        &mut r.tw,
        late.who,
        &format!(
            "+market/bid ex1a:iron qty=1 price=1 into={} market=luna",
            late.vault.vault().0
        ),
    );
    assert_eq!(market(&r.tw).listing_count(), MAX_LISTINGS);
    assert!(market(&r.tw).indexes_agree());
}

/// **The economy's 100-a-party cap binds a player seller** (Q-48-44): with 100 of Sal's
/// sales live (each bought, none settled), the next buyer's purchase is refused at the offer
/// with the cap's own words, and nothing is escrowed. A personal listing is not a contract,
/// so the hundred listings Sal posted on the way counted against nothing.
#[test]
fn ex1a_the_party_cap_binds_a_player_seller() {
    let mut r = rig();
    let sal = r.sal;
    let faucet = r.faucet;
    let cap = mush_economy::MAX_LIVE_CONTRACTS_PER_PARTY;
    let per_buyer = mush_market::MAX_OPEN_QUOTES_PER_BUYER;
    let mut buyers: Vec<Trader> = Vec::new();
    let mut sold = 0usize;
    let mut next_lot = 0x2000u64;
    while sold < cap {
        if sold.is_multiple_of(per_buyer) {
            let n = buyers.len();
            buyers.push(trader(&mut r.tw, &format!("Buyer{n:02}"), faucet));
        }
        let buyer = *buyers.last().expect("a buyer");
        let lot = LotId(next_lot);
        next_lot += 1;
        mint_lot(&mut r.tw, &r.families, sal.vault, "iron", 1, lot);
        let id = post_lot(&mut r, lot, 100, "", 0x50);
        buy(&mut r, buyer, id, 1);
        // The consume is left unlanded: the contract is live either way.
        let _ = r.tw.engine_mut().take_authoritative_submit();
        sold += 1;
    }
    let live = economy(&r.tw)
        .snapshot()
        .contracts()
        .filter(|c| c.state.is_live() && c.consent.parties.contains(&PrincipalId::Object(sal.who)))
        .count();
    assert_eq!(live, cap);
    let lot = LotId(next_lot);
    mint_lot(&mut r.tw, &r.families, sal.vault, "iron", 1, lot);
    let id = post_lot(&mut r, lot, 100, "", 0x51);
    let late = trader(&mut r.tw, "Late", faucet);
    ok(&mut r.tw, late.who, &format!("+market/quote {id}=1"));
    let quote = market(&r.tw)
        .quotes_of_buyer(&PrincipalId::Object(late.who))
        .last()
        .map(|q| q.id)
        .expect("a quote");
    let refused = err(
        &mut r.tw,
        late.who,
        &format!("+market/accept {quote}={}", late.vault.vault().0),
    );
    assert!(
        refused.contains("a party is already on the most live contracts allowed"),
        "{refused}"
    );
    assert_eq!(held(&r.tw, late.account), 0);
    assert!(market(&r.tw).listing(id).is_some(), "the listing stands");
}

/// **The economy's 16-an-issuer cap binds a player selling into bids** (Q-48-44): the seller
/// is each sale's issuer, so with 16 sales open across four players' bids (four a bid, the
/// per-bid cap), the 17th is refused at the offer and nothing is escrowed.
#[test]
fn ex1a_the_issuer_cap_binds_a_player_selling_into_bids() {
    let mut r = rig();
    let sal = r.sal;
    let faucet = r.faucet;
    seed_pool(&mut r.tw, &r.families, sal.vault, "iron", 100, LotId(0xf0));
    let per_bid = mush_market::MAX_OPEN_SELLS_PER_VENDOR;
    let cap = mush_economy::MAX_OPEN_CONTRACTS_PER_ISSUER;
    let mut bids = Vec::new();
    for n in 0..=(cap / per_bid) {
        let bidder = trader(&mut r.tw, &format!("Bidder{n}"), faucet);
        let said = ok(
            &mut r.tw,
            bidder.who,
            &format!(
                "+market/bid ex1a:iron qty=10 price=10 into={} market=luna",
                bidder.vault.vault().0
            ),
        );
        bids.push((bidder, listed_id(&said, "Bid #")));
    }
    let mut sold = 0;
    'outer: for (_, bid) in &bids {
        for _ in 0..per_bid {
            if sold == cap {
                break 'outer;
            }
            ok(
                &mut r.tw,
                sal.who,
                &format!("+market/sell {bid}=1 from={}", sal.vault.vault().0),
            );
            // The deliver leg is left unlanded: the sale stays open.
            let _ = r.tw.engine_mut().take_authoritative_submit();
            sold += 1;
        }
    }
    let (last_bidder, last_bid) = *bids.last().expect("a fifth bid");
    let before = held(&r.tw, last_bidder.account);
    let refused = err(
        &mut r.tw,
        sal.who,
        &format!("+market/sell {last_bid}=1 from={}", sal.vault.vault().0),
    );
    assert!(
        refused.contains("that issuer already has the most open contracts allowed"),
        "{refused}"
    );
    assert_eq!(held(&r.tw, last_bidder.account), before, "nothing escrowed");
    assert_eq!(
        market(&r.tw).listing(last_bid).map(|l| l.quantity),
        Some(10)
    );
}

// ---- 6: the refusals --------------------------------------------------------------

/// The personal verbs' refusals, each before any write: a lot you may not take reads as
/// none; your own listing is not yours to buy; a bulk bid twice; a lot already listed; a
/// player with no account in the venue's currency; another player's listing is not yours
/// to withdraw.
#[test]
fn ex1a_the_personal_verbs_refuse_before_any_write() {
    let mut r = rig();
    let (sal, bea, cy) = (r.sal, r.bea, r.cy);
    let lot = LotId(0x5c1);
    mint_lot(&mut r.tw, &r.families, sal.vault, "iron", 3, lot);
    let stranger = err(
        &mut r.tw,
        cy.who,
        &format!("+market/post {lot} price=700 market=luna"),
    );
    assert!(
        stranger.contains(mush_market::seller::NO_SUCH_LOT),
        "{stranger}"
    );
    let usage = err(
        &mut r.tw,
        sal.who,
        &format!("+market/post {lot} market=luna"),
    );
    assert!(usage.contains("Usage: +market/post"), "{usage}");
    let nowhere = err(&mut r.tw, sal.who, &format!("+market/post {lot} price=700"));
    assert!(
        nowhere.contains(mush_market::seller::NAME_VENUE),
        "{nowhere}"
    );
    let id = post_lot(&mut r, lot, 700, "", 0x11);
    let twice = err(
        &mut r.tw,
        sal.who,
        &format!("+market/post {lot} price=700 market=luna"),
    );
    assert!(twice.contains(mush_market::seller::LOT_HELD), "{twice}");
    let own = err(&mut r.tw, sal.who, &format!("+market/quote {id}=1"));
    assert!(own.contains("That is your own listing"), "{own}");
    let not_yours = err(&mut r.tw, bea.who, &format!("+market/withdraw {id}"));
    assert!(not_yours.contains(NO_SUCH_LISTING), "{not_yours}");
    let bid = |r: &mut Rig| {
        err(
            &mut r.tw,
            bea.who,
            &format!(
                "+market/bid ex1a:iron qty=2 price=100 into={} market=luna",
                bea.vault.vault().0
            ),
        )
    };
    ok(
        &mut r.tw,
        bea.who,
        &format!(
            "+market/bid ex1a:iron qty=2 price=100 into={} market=luna",
            bea.vault.vault().0
        ),
    );
    let again = bid(&mut r);
    assert!(again.contains("You already bid for that good"), "{again}");
    let not_mine = err(
        &mut r.tw,
        bea.who,
        &format!(
            "+market/bid ex1a:g00 qty=2 price=100 into={} market=luna",
            sal.vault.vault().0
        ),
    );
    assert!(
        not_mine.contains(mush_market::seller::NO_SUCH_VAULT),
        "{not_mine}"
    );
    let broke = err(
        &mut r.tw,
        bea.who,
        &format!(
            "+market/bid ex1a:g01 qty=1000 price=100000 into={} market=luna",
            bea.vault.vault().0
        ),
    );
    assert!(broke.contains("available to hold for that bid"), "{broke}");
    // Selling into one's own bid, even from a second vault, is refused in its own words.
    let own_bid = market(&r.tw)
        .personal_listings_of(&PrincipalId::Object(bea.who))
        .find(|l| l.source.is_wanted())
        .map(|l| l.id)
        .expect("Bea's bid");
    let spare =
        r.tw.world_mut()
            .create(ObjKind::Thing, "Bea's spare crate", bea.who, Some(bea.who))
            .expect("a container");
    ok(
        &mut r.tw,
        GOD,
        &format!(
            "@vault/create Spare=container:#{} owner=#{}",
            spare.0, bea.who.0
        ),
    );
    let second = EndpointRef(
        economy(&r.tw)
            .snapshot()
            .vaults()
            .filter(|v| v.owner == PrincipalId::Object(bea.who))
            .map(|v| v.id)
            .max()
            .expect("the spare vault"),
    );
    assert_ne!(second, bea.vault);
    seed_pool(&mut r.tw, &r.families, second, "iron", 2, LotId(0x5c9));
    let into_own = err(
        &mut r.tw,
        bea.who,
        &format!("+market/sell {own_bid}=1 from={}", second.vault().0),
    );
    assert!(into_own.contains("That is your own listing"), "{into_own}");
    let withdrawn = ok(&mut r.tw, sal.who, &format!("+market/withdraw {id}"));
    assert!(
        withdrawn.contains("its goods are yours to use again"),
        "{withdrawn}"
    );
}

/// Every new player sentence is under 120 columns as the verbs fill it.
#[test]
fn ex1a_every_new_sentence_fits_120_columns() {
    use mush_market::seller as s;
    let filled = [
        s::ASK_POSTED
            .replace("{listing}", "1023")
            .replace("{qty}", "100000")
            .replace("{name}", "Crated instrument boards")
            .replace("{price}", "1,234,567.00cr")
            .replace("{to}", &s::FOR_ONLY.replace("{who}", "Bartholomew")),
        s::POSTED_AWAY.replace("{listing}", "1023"),
        s::BID_POSTED
            .replace("{listing}", "1023")
            .replace("{qty}", "100000")
            .replace("{name}", "Crated instrument boards")
            .replace("{price}", "12,345.00cr")
            .replace("{total}", "1,234,500,000.00cr")
            .replace("{to}", ""),
        s::WITHDRAWN.replace("{listing}", "1023"),
        s::BID_WITHDRAWN.replace("{listing}", "1023"),
        s::MINE_COUNT.replace("{n}", "32").replace("{cap}", "32"),
        s::SELLER_FULL.replace("{cap}", "32"),
        s::OWN_LISTING.replace("{listing}", "1023"),
        s::BID_TWICE.replace("{listing}", "1023"),
        s::BID_CANNOT_COVER.replace("{total}", "1,234,500,000.00cr"),
        s::POST_USAGE.to_owned(),
        s::BID_USAGE.to_owned(),
        s::WITHDRAW_USAGE.to_owned(),
        s::BID_INTO_HOLD.to_owned(),
        s::PRIVATE_TO_OTHER.to_owned(),
        s::BIDDER_NOT_BUYING.to_owned(),
        s::SELLER_GONE.to_owned(),
        s::BID_NEEDS_APPROVAL.to_owned(),
    ];
    for line in filled {
        assert!(
            line.chars().count() < 120,
            "{} columns: {line}",
            line.chars().count()
        );
    }
}

/// **Doc 48e, exchange G1 (48c #3, 48d C-01): an exact lot sells whole.** A quote for part of one
/// is refused before anything is written; a bare quote prices the whole lot. Before the fix a
/// one-unit quote (the default) was accepted: the buyer paid for one unit, the Consume of the
/// whole lot refused at prepare, the contract settled on its window anyway, and the seller's lot
/// stayed held under a listing the accept had deleted.
#[test]
fn g1_an_exact_lot_is_quoted_whole_or_not_at_all() {
    let mut r = rig();
    let lot = LotId(0x5a2);
    let bea = r.bea;
    mint_lot(&mut r.tw, &r.families, r.sal.vault, "iron", 10, lot);
    let id = post_lot(&mut r, lot, 700, "", 0x13);
    for part in ["1", "9"] {
        let refused = err(&mut r.tw, bea.who, &format!("+market/quote {id}={part}"));
        assert!(
            refused.contains(&mush_market::WHOLE_LOT.replace("{n}", "10")),
            "{refused}"
        );
    }
    assert_eq!(
        market(&r.tw)
            .quotes_of_buyer(&PrincipalId::Object(bea.who))
            .count(),
        0,
        "a refused quote writes nothing"
    );
    // A bare quote is the whole lot.
    let quoted = ok(&mut r.tw, bea.who, &format!("+market/quote {id}"));
    let quote = market(&r.tw)
        .quotes_of_buyer(&PrincipalId::Object(bea.who))
        .next()
        .cloned()
        .unwrap_or_else(|| panic!("a quote: {quoted}"));
    assert_eq!(quote.quantity, 10, "{quoted}");
    // And the whole lot is what is sold.
    let contract = buy(&mut r, bea, id, 10);
    land(&mut r.tw, 0x23);
    settle(&mut r.tw, contract);
    assert_eq!(lot_at(&mut r.tw, &r.families, lot), Some(bea.vault));
}

/// **Doc 48e, exchange G2 (48c #1, 48d C-02): a hold-backed sale settles only on proof the lot
/// moved.** A sale whose Consume never lands pays the seller nothing: it stands unsettled, its
/// window passes, and the buyer's money comes back. The listing is off the board, and once the
/// sale has refunded its hold is the seller's to release. Before the fix the sale's one condition
/// was its window, so it settled at the window and paid the seller for a lot the buyer never got.
#[test]
fn g2_a_hold_backed_sale_with_no_consume_pays_nothing_and_refunds() {
    let mut r = rig();
    let lot = LotId(0x5a3);
    let (sal, bea) = (r.sal, r.bea);
    mint_lot(&mut r.tw, &r.families, sal.vault, "iron", 10, lot);
    let id = post_lot(&mut r, lot, 700, "", 0x14);
    let contract = buy(&mut r, bea, id, 10);
    // The Consume is dropped: it never lands.
    let _ = take(&mut r.tw);
    let start = r.tw.world().now();
    for i in 1..=4 {
        at(&mut r.tw, start + i * 31);
    }
    assert!(
        !matches!(
            state_of(&r.tw, contract),
            Some(ContractState::Settled { .. })
        ),
        "no proof the lot moved, no settlement: {:?}",
        state_of(&r.tw, contract)
    );
    assert_eq!(
        posted(&r.tw, sal.account),
        FUNDS,
        "the seller is paid nothing"
    );
    // Past its window the sale refunds.
    at(
        &mut r.tw,
        start + mush_market::DRAW_WINDOW_SECS + mush_economy::SWEEP_PERIOD_SECS + 1,
    );
    at(
        &mut r.tw,
        start + mush_market::DRAW_WINDOW_SECS + 2 * mush_economy::SWEEP_PERIOD_SECS + 2,
    );
    assert!(
        matches!(state_of(&r.tw, contract), Some(ContractState::Expired)),
        "{:?}",
        state_of(&r.tw, contract)
    );
    assert_eq!(
        posted(&r.tw, bea.account),
        FUNDS,
        "the buyer's money is back"
    );
    assert_eq!(posted(&r.tw, sal.account), FUNDS);
    // The lot never left the seller.
    assert_eq!(lot_at(&mut r.tw, &r.families, lot), Some(sal.vault));
    // The sale is over, so its hold is free to release: it is no longer in force.
    assert!(
        !mush_market::sale_in_force(r.tw.world(), id),
        "a refunded sale holds nothing"
    );
}

/// **Doc 48e, exchange G21 (48d M-01): the hold reconcile reaches every lot listing.** Thirty-two
/// healthy listings stand ahead of one whose hold was never placed. The reconcile checks at most
/// 32 a fire; before the fix it took the first 32 every fire, so the unheld listing was never
/// reached and stood on the board advertising stock nothing held. With the cursor it is reached
/// on the next lap and withdrawn.
#[test]
fn g21_the_hold_reconcile_reaches_a_listing_past_the_first_lap() {
    let mut r = rig();
    let (sal, bea) = (r.sal, r.bea);
    for n in 0..mush_market::MAX_LOT_RECONCILED_PER_FIRE as u64 {
        let lot = LotId(0x7000 + n);
        mint_lot(&mut r.tw, &r.families, sal.vault, "iron", 1, lot);
        post_lot(&mut r, lot, 100, "", 0x60);
    }
    // Bea's listing: its reserve is staged and dropped, so nothing holds it.
    let lot = LotId(0x7fff);
    mint_lot(&mut r.tw, &r.families, bea.vault, "iron", 1, lot);
    let said = ok(
        &mut r.tw,
        bea.who,
        &format!("+market/post {lot} price=100 market=luna"),
    );
    let unheld = listed_id(&said, "Listed for sale as #");
    let _ = take(&mut r.tw);
    let start = r.tw.world().now();
    for i in 1..=12 {
        at(&mut r.tw, start + i * 61);
        if market(&r.tw).listing(unheld).is_none() {
            break;
        }
    }
    assert!(
        market(&r.tw).listing(unheld).is_none(),
        "the listing past the first lap is reached and withdrawn"
    );
    assert_eq!(
        market(&r.tw).listings_of_seller(&PrincipalId::Object(sal.who)),
        mush_market::MAX_LOT_RECONCILED_PER_FIRE,
        "the healthy listings stand"
    );
}

/// **Doc 48e, exchange G22 (48d M-02): one live bid for a good at each venue.** Bea bids for
/// iron at Luna and then at Mars: both stand. A second bid for iron at Luna is still refused.
/// Before the fix the Mars bid was refused as a second bid.
#[test]
fn g22_a_bidder_bids_for_one_good_at_two_venues() {
    let mut r = rig();
    let bea = r.bea;
    let line = |venue: &str| {
        format!(
            "+market/bid ex1a:iron qty=2 price=100 into={} market={venue}",
            bea.vault.vault().0
        )
    };
    // A second venue, Mars, with a vendor of its own, so it trades in credits.
    ok(&mut r.tw, GOD, "@org/create Hermes");
    let hermes = OrgId(1);
    ok(
        &mut r.tw,
        GOD,
        &format!("@account/open org:#{}={CREDIT}", hermes.0),
    );
    let strongbox =
        r.tw.world_mut()
            .create(ObjKind::Thing, "Hermes strongbox", GOD, None)
            .expect("a container");
    ok(
        &mut r.tw,
        GOD,
        &format!(
            "@vault/create Hermes=container:#{} owner=org:#{}",
            strongbox.0, hermes.0
        ),
    );
    let vault = economy(&r.tw)
        .snapshot()
        .vaults()
        .find(|v| v.owner == PrincipalId::Organization(hermes))
        .expect("the Hermes vault")
        .id;
    ok(
        &mut r.tw,
        GOD,
        &format!(
            "@market/vendor create #{}=#{} name=Hermes market=mars",
            hermes.0, vault.0
        ),
    );
    let luna = ok(&mut r.tw, bea.who, &line("luna"));
    assert!(luna.contains("Bid #"), "{luna}");
    let mars = ok(&mut r.tw, bea.who, &line("mars"));
    assert!(mars.contains("Bid #"), "a bid at another venue: {mars}");
    let again = err(&mut r.tw, bea.who, &line("luna"));
    assert!(
        again.contains("You already bid for that good here"),
        "{again}"
    );
}

/// **Doc 48e, exchange G18 (48d H-11): a lot ask posted while the last is still landing is
/// refused before anything is charged.** Sal posts one lot; before its hold lands she posts a
/// second: refused with [`mush_market::seller::POST_BUSY`], and no second listing stands. Before
/// the fix the second was listed, its hold's submission voided, and its fee kept.
#[test]
fn g18_a_post_while_the_last_is_landing_is_refused() {
    let mut r = rig();
    let sal = r.sal;
    mint_lot(&mut r.tw, &r.families, sal.vault, "iron", 4, LotId(0xd8));
    mint_lot(&mut r.tw, &r.families, sal.vault, "iron", 4, LotId(0xd9));
    let first = ok(
        &mut r.tw,
        sal.who,
        &format!("+market/post {} price=100 market=luna", LotId(0xd8)),
    );
    assert!(first.contains("Listed for sale as #"), "{first}");
    let second = err(
        &mut r.tw,
        sal.who,
        &format!("+market/post {} price=100 market=luna", LotId(0xd9)),
    );
    assert!(second.contains(mush_market::seller::POST_BUSY), "{second}");
    assert_eq!(
        market(&r.tw)
            .personal_listings_of(&PrincipalId::Object(sal.who))
            .count(),
        1
    );
}

/// **Doc 48e, exchange G29 (48d H-27): `+market/mine` shows what the actor listed for an
/// organization.** Sal, granted Ares' posting right, lists a lot `as=Ares`: her page shows it
/// under "Listed as". Before the fix the page said she had nothing listed.
#[test]
fn g29_mine_shows_an_orgs_listing_the_actor_posted() {
    let mut r = rig();
    let sal = r.sal;
    ok(
        &mut r.tw,
        GOD,
        &format!("@org/grant #{}=Ares economy.account.post", sal.who.0),
    );
    mint_lot(&mut r.tw, &r.families, sal.vault, "iron", 4, LotId(0xda));
    let said = ok(
        &mut r.tw,
        sal.who,
        &format!("+market/post {} price=100 market=luna as=Ares", LotId(0xda)),
    );
    assert!(said.contains("Listed for sale as #"), "{said}");
    let page = ok(&mut r.tw, sal.who, "+market/mine");
    assert!(page.contains("Listed as "), "{page}");
    assert!(page.contains("Ares"), "{page}");
}
