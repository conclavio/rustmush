//! ---- Doc 18 cut IC0: the market's engine blockers, as gates ----
//!
//! The trade run (doc 18 §3 beat E) found four things wrong with a lot sale before any
//! content existed to sell: a settled lot sale paged `SettlementRefused` with an empty
//! delivery table; a listing stood fourteen real days against a seven-IC-day hold that
//! nothing read, so a late accept took the money and delivered nothing; every price
//! line printed minor units; and the economist answered the room with what it had read
//! as the asker. This file is the gate for all four, on **the production composition
//! in miniature**: `mush-org`, `mush-economy` with the vendor organization in its
//! composition-frozen party-cap table, `mush-inventory` delegating quantity writes to
//! `"market"`, and `mush-market` holding the same handle cell with one lot-represented
//! catalogue entry.
//!
//! **Why the party cap is wired here and not asserted on the real roster.** A vendor
//! organization accepts a purchase contract programmatically through
//! `accept_contract_as`, which refuses any principal the composition did not name in
//! `with_party_caps`; the game's root passes `Vec::new()` today
//! (`tacsim/src/composition/mod.rs`), so on the real roster the money leg stops at
//! "that vendor cannot complete the sale". Doc 18 cut IC5's `with_party_cap_source`
//! is what opens it there, and the stock → quote → accept → settle gate on the
//! production roster is IC5's own verify lens. What IC0 can and does pin is the
//! engine: with a vendor that may accept, the loop settles and pages nothing.

use std::sync::Arc;

use mush_core::{
    Audience, CommitToken, GOD, GameClock, IdempotencyKey, IdempotencyScope,
    InterfaceAbilityProvider, ObjId, ObjKind, OfferExecutionContext, OfferSubmission, RequestUid,
    ScopedIdempotencyKey,
};
use mush_economy::{AccountId, EconomyStore, EscalationKind, ExternalRef, PartyCap, VaultId};
use mush_inventory::{
    EndpointRef, Grade, InventoryError, InventoryFamilies, InventoryHandleCell, InventoryHandles,
    InventoryModule, LotId, RESERVATION_TTL_SECS,
};
use mush_market::interface::{MARKET_INTERFACE, MarketInterface};
use mush_market::{CatalogEntry, CatalogSource, ListingId, MarketStore, QuoteId, STALE_QUOTE};
use mush_org::{OrgId, PrincipalId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use serde_json::{Value, json};

// ---- the rig ---------------------------------------------------------------

/// The test pack and its one lot-represented good, `<ruleset>:<code>`.
const PACK: &str = "ic0.test";
const GOOD: &str = "ic0:iron";

/// The vendor's organization, **pre-wired** into the economy's party-cap table: the
/// table is composition-frozen and `@org/create` allocates ids from zero upward on a
/// fresh world, so the rig names the id the shop will get and asserts it got it.
const VENDOR_ORG: OrgId = OrgId(0);

/// The settlement currency: two decimals, `cr` after the digits, so a 700-minor price
/// reads `7.00cr` through the economy's one render.
const CREDIT: &str = "credit";

/// One lot-represented catalogue entry, as a composition-frozen source.
#[derive(Debug)]
struct TestCatalog;

impl CatalogSource for TestCatalog {
    fn pack(&self, pack: &str) -> Option<Result<Vec<CatalogEntry>, String>> {
        if pack != PACK {
            return None;
        }
        let value = json!({
            "catalog": { "ruleset": "ic0", "code": "iron" },
            "pack": PACK,
            "definition_version": 1,
            "name": "Iron, graded",
            "baseline": 700,
            "legality": "legal",
            "representation": "lot",
            "stackable": true,
            "tags": ["test"],
        });
        Some(
            serde_json::from_value::<CatalogEntry>(value)
                .map(|entry| vec![entry])
                .map_err(|error| error.to_string()),
        )
    }
}

fn boot() -> (TestWorld, InventoryFamilies) {
    let cell: InventoryHandleCell = Arc::default();
    let tw = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(
            mush_economy::EconomyModule::new()
                .with_party_caps(vec![(
                    PrincipalId::Organization(VENDOR_ORG),
                    PartyCap::Accept,
                )])
                // The market's ear, as the game's root wires it: a settlement
                // becomes the trade-tape row and the hand-over `deliver` runs.
                .with_observers(vec![Arc::new(mush_market::MarketObserver)]),
        ),
        Box::new(
            InventoryModule::new()
                .with_quantity_delegates(["market"])
                .with_handle_cell(Arc::clone(&cell)),
        ),
        Box::new(
            mush_market::MarketModule::new()
                .with_inventory(Arc::clone(&cell))
                .with_catalog_sources(vec![Arc::new(TestCatalog)]),
        ),
    ]);
    let families = tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory publishes its handles")
        .0
        .clone();
    (tw, families)
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
        .expect("the account exists")
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

/// Every `SettlementRefused` page the economy's ladder holds, by its context line.
fn refused_settlements(tw: &TestWorld) -> Vec<String> {
    economy(tw)
        .escalations()
        .filter(|e| matches!(e.kind, EscalationKind::SettlementRefused))
        .map(|e| e.context.clone())
        .collect()
}

fn commodity(id: &str) -> mush_core::CanonicalId {
    mush_core::CanonicalId::new(id).expect("test commodity id")
}

/// `crates/mush-market/tests/e7r2.rs`'s rig verbs: an owner-stamped transaction
/// installed exactly as an exact receipt would be.
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

/// Put `amount` of `id` into the endpoint's pool **through a lot**, so the pool has
/// the lineage root a lot listing is required to have.
fn seed_rooted_pool(
    tw: &mut TestWorld,
    families: &InventoryFamilies,
    at: EndpointRef,
    id: &str,
    amount: u64,
    lot: LotId,
) {
    apply(tw, families, |txn| {
        families
            .mint_origin(
                txn,
                at,
                &commodity(id),
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
    .expect("the seed lot dissolves into the endpoint's pool");
}

/// The pool's balance and availability at an endpoint.
fn pool(tw: &mut TestWorld, families: &InventoryFamilies, at: EndpointRef, id: &str) -> (u64, u64) {
    read(tw, families, |txn| {
        let key = mush_inventory::pool_key(at, &commodity(id)).expect("pool key");
        let scalar = txn.get(&families.pools, &key).ok().flatten().copied();
        let book = txn.get(&families.books, &key).ok().flatten().cloned();
        match (scalar, book) {
            (Some(scalar), Some(book)) => (
                u64::try_from(scalar.current()).unwrap_or_default(),
                book.checked_available(scalar).unwrap_or_default(),
            ),
            _ => (0, 0),
        }
    })
}

/// How many live lots an endpoint holds, from the census the bundle writes.
fn live_lots(tw: &mut TestWorld, families: &InventoryFamilies, at: EndpointRef) -> u64 {
    read(tw, families, |txn| {
        txn.get(&families.lot_census, &at)
            .ok()
            .flatten()
            .map_or(0, |census| u64::from(census.get()))
    })
}

/// The production composition in miniature, stocked: a funded buyer with a hold of
/// her own, a vendor organization with a storefront and a rooted pool of 100 iron at
/// it, and one lot listing of ten at 7.00cr.
struct Rig {
    tw: TestWorld,
    families: InventoryFamilies,
    pat: ObjId,
    pat_account: AccountId,
    pat_hold: EndpointRef,
    shop_endpoint: EndpointRef,
    vendor_account: AccountId,
    listing: ListingId,
}

const T0: i64 = 1_700_000_000;

fn rig_at(ic_ratio: u32, days: Option<i64>) -> Rig {
    let (mut tw, families) = boot();
    // The durable offer-key ring a legal-action offer is signed with — the world
    // task installs it at boot; a rig that lands submissions needs one too.
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

    // The shop.
    ok(&mut tw, GOD, "@org/create Ares");
    let org = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().orgs().map(|o| o.id).max())
        .expect("the org exists");
    assert_eq!(
        org, VENDOR_ORG,
        "the rig pre-wires the shop's party cap by id, so the shop must get that id"
    );
    let shop = PrincipalId::Organization(org);
    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:#{}={CREDIT}", org.0),
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
            strongbox.0, org.0
        ),
    );
    let shop_vault = vault_of(&tw, &shop);
    let shop_endpoint = EndpointRef(shop_vault);
    ok(&mut tw, GOD, &format!("@market/catalog load {PACK}"));
    ok(
        &mut tw,
        GOD,
        &format!(
            "@market/vendor create #{}=#{} name=Ares market=luna buy=300",
            org.0, shop_vault.0
        ),
    );
    let vendor = market(&tw).vendor_of_org(org).expect("registered").id;

    // The buyer.
    let pat = tw.spawn_player("Pat");
    ok(&mut tw, GOD, &format!("@account/open #{}={CREDIT}", pat.0));
    let pat_account = account_of(&tw, &PrincipalId::Object(pat));
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {faucet}={pat_account} 100000"),
    );
    assert_eq!(posted(&tw, pat_account), 100_000);
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

    // The stock, rooted, and the listing over it.
    seed_rooted_pool(&mut tw, &families, shop_endpoint, "iron", 100, LotId(0xb1));
    assert_eq!(pool(&mut tw, &families, shop_endpoint, "iron"), (100, 100));
    let days = days.map_or_else(String::new, |d| format!(" days={d}"));
    let said = ok(
        &mut tw,
        GOD,
        &format!(
            "@market/vendor stock #{}={GOOD} price=700 qty=10 endpoint={}{days}",
            vendor.0, shop_vault.0
        ),
    );
    assert!(said.contains("Listing #0 published."), "{said}");
    let listing = ListingId(0);
    land(&mut tw, 0x11).expect("the listing's reserve lands");
    assert_eq!(
        pool(&mut tw, &families, shop_endpoint, "iron"),
        (100, 90),
        "the listing's hold is placed: balance unmoved, availability down by the ten"
    );
    Rig {
        tw,
        families,
        pat,
        pat_account,
        pat_hold,
        shop_endpoint,
        vendor_account,
        listing,
    }
}

fn rig() -> Rig {
    rig_at(1, None)
}

/// **Drive the one queued authoritative submission the way the world task does**
/// (`mush-industry/tests/e7c3.rs`'s `land`, line for line): regenerate the actor's
/// own offer, resolve the queued parameters against it, prepare and install. A
/// command only *queues* its lot leg (`Queued.`); this is the rest of the lane.
fn land(tw: &mut TestWorld, token: u8) -> Result<(), String> {
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
    Ok(())
}

/// Fire the economy's coarse sweep until the contract settles, bounded.
fn settle(tw: &mut TestWorld, contract: mush_economy::ContractId) {
    for i in 1..=5 {
        let at = T0 + i * (mush_economy::SWEEP_PERIOD_SECS + 1);
        tw.set_now(at);
        let _ = tw.fire_due(at * 1000);
        let state = economy(tw).snapshot().contract(contract).map(|c| c.state);
        if matches!(state, Some(mush_economy::ContractState::Settled { .. })) {
            return;
        }
    }
    let state = economy(tw).snapshot().contract(contract).map(|c| c.state);
    panic!("contract #{contract} did not settle on the sweep: {state:?}");
}

// ---- item 1 + item 7: the gate ----------------------------------------------

/// **Stock → quote → accept → settle, and no `SettlementRefused` page.**
///
/// Before IC0, `module::deliver` walked the delivery table for a `'lot'` purchase on
/// settlement, found no provider and paged the ladder — about a buyer who held exactly
/// what she paid for, because the matter leg had already moved at activation. The gate
/// drives the whole loop through the verbs a player types and asserts the ladder is
/// empty, the money moved once, and the lot is in her hold.
#[test]
fn ic0_a_lot_sale_settles_and_pages_nothing() {
    let Rig {
        mut tw,
        families,
        pat,
        pat_account,
        pat_hold,
        shop_endpoint,
        vendor_account,
        listing,
    } = rig();

    let quoted = ok(&mut tw, pat, &format!("+market/quote {listing}=10"));
    assert!(quoted.contains("Quote #0"), "{quoted}");
    let agreed = ok(
        &mut tw,
        pat,
        &format!("+market/accept 0={}", pat_hold.vault().0),
    );
    assert!(agreed.contains("Contract #"), "{agreed}");
    let contract = market(&tw)
        .quote(QuoteId(0))
        .and_then(|q| q.contract)
        .expect("the accept drafted a contract");
    // The matter leg, staged at activation and landed here as the world task would.
    land(&mut tw, 0x22).expect("the consume lands");

    settle(&mut tw, contract);

    // The quote is the trade-tape row now, and the ladder holds nothing about it.
    assert!(
        market(&tw)
            .quote(QuoteId(0))
            .is_some_and(|q| q.settled.is_some()),
        "the quote records its settlement"
    );
    let pages = refused_settlements(&tw);
    assert!(
        pages.is_empty(),
        "a settled lot sale paged the ladder (doc 18 cut IC0, item 1): {pages:?}"
    );
    // The money moved exactly once: 10 × 7.00cr.
    assert_eq!(posted(&tw, pat_account), 100_000 - 7_000);
    assert_eq!(posted(&tw, vendor_account), 7_000);
    // The matter moved exactly once: the shop's pool lost the hold, the buyer's hold
    // gained one live lot, and the listing — exhausted by its one sale — is gone.
    assert_eq!(pool(&mut tw, &families, shop_endpoint, "iron"), (90, 90));
    assert_eq!(live_lots(&mut tw, &families, pat_hold), 1);
    assert!(market(&tw).listing(listing).is_none());
    let hold = ok(&mut tw, pat, &format!("@inv/lots {}", pat_hold.vault().0));
    assert!(
        hold.contains("iron") && hold.contains("10"),
        "the buyer reads her own lot: {hold}"
    );
}

// ---- item 2: the listing cannot outlive its hold -----------------------------

/// **A lot listing's advertised window is cut to the hold's**, at the world's rate.
#[test]
fn ic0_a_lot_listing_stands_no_longer_than_its_hold() {
    // Rate 1: seven IC days are seven real days; `days=30` is cut to them.
    let r = rig_at(1, Some(30));
    let listing = market(&r.tw).listing(r.listing).expect("listed").clone();
    assert_eq!(listing.expires.at - T0, RESERVATION_TTL_SECS);
    // Rate 4: the hold lapses in seven IC days, which is 1.75 real days, and so
    // does the listing.
    let r = rig_at(4, None);
    let listing = market(&r.tw).listing(r.listing).expect("listed").clone();
    assert_eq!(listing.expires.at - T0, RESERVATION_TTL_SECS / 4);
    assert_eq!(
        mush_market::hold_ttl_real_secs(r.tw.world()),
        Some(RESERVATION_TTL_SECS / 4)
    );
}

/// **A late accept is refused with nothing charged**, because `is_current` reads the
/// hold's own deadline — and so is a late quote.
///
/// The IC calendar is re-phased seven days forward while real time stands still (a
/// staff `@tacsim/time`-shaped act on the one clock generation), so the listing and
/// the quote are both still inside their real-time windows and the **only** thing that
/// lapsed is the hold. Before IC0 this accept escrowed the money and the matter leg
/// refused inside the settling bundle.
#[test]
fn ic0_a_quote_against_a_lapsed_hold_is_refused_before_money_moves() {
    let Rig {
        mut tw,
        families,
        pat,
        pat_account,
        pat_hold,
        shop_endpoint,
        listing,
        ..
    } = rig();
    ok(&mut tw, pat, &format!("+market/quote {listing}=10"));
    let quote = market(&tw).quote(QuoteId(0)).expect("quoted").clone();
    assert!(quote.expires.at > T0, "the quote is inside its own window");

    tw.world_mut().set_game_clock(GameClock {
        ic_ratio: 1,
        epoch_offset: RESERVATION_TTL_SECS + 1,
    });
    assert!(
        tw.world().now() < quote.expires.at,
        "real time did not move: only the IC clock did"
    );

    let refused = err(
        &mut tw,
        pat,
        &format!("+market/accept 0={}", pat_hold.vault().0),
    );
    assert!(refused.contains(STALE_QUOTE), "{refused}");
    assert_eq!(posted(&tw, pat_account), 100_000, "nothing was charged");
    assert!(
        market(&tw)
            .quote(QuoteId(0))
            .is_some_and(|q| q.contract.is_none()),
        "no contract was drafted"
    );
    assert!(
        tw.engine_mut().take_authoritative_submit().is_none(),
        "no matter leg was staged"
    );
    assert!(refused_settlements(&tw).is_empty());
    assert_eq!(
        pool(&mut tw, &families, shop_endpoint, "iron"),
        (100, 90),
        "the hold itself is untouched: the seller withdraws it"
    );
    // A fresh quote reads the same hold and refuses the same way.
    let refused = err(&mut tw, pat, &format!("+market/quote {listing}=10"));
    assert!(refused.contains(STALE_QUOTE), "{refused}");
}

// ---- item 4: major units on every price line -------------------------------

/// **Every price line reads in the instrument's scale and symbol.** The parse side is
/// still minor units (`price=700`), and the three player reads — the list, the quote
/// and the orders — all say `7.00cr`.
#[test]
fn ic0_every_price_line_reads_in_major_units() {
    let Rig {
        mut tw,
        pat,
        pat_hold,
        listing,
        ..
    } = rig();
    let list = ok(&mut tw, pat, "+market/list");
    assert!(
        list.contains("10 × 7.00cr"),
        "the list renders the unit price in major units: {list}"
    );
    assert!(!list.contains(" 700 "), "{list}");
    let quoted = ok(&mut tw, pat, &format!("+market/quote {listing}=10"));
    assert!(
        quoted.contains("10 × 7.00cr = 70.00cr"),
        "the quote renders price and total in major units: {quoted}"
    );
    let depth = ok(&mut tw, pat, "+market/depth ic0:iron market=luna");
    assert!(
        depth.contains("band: 7.00cr..7.00cr"),
        "the projection's band renders in major units: {depth}"
    );
    let shown = ok(&mut tw, GOD, "@market/catalog show ic0:iron");
    assert!(
        shown.contains("never charged): 7.00cr"),
        "the catalogue baseline renders in major units: {shown}"
    );
    ok(
        &mut tw,
        pat,
        &format!("+market/accept 0={}", pat_hold.vault().0),
    );
    land(&mut tw, 0x22).expect("the consume lands");
    let orders = ok(&mut tw, pat, "+market/orders");
    assert!(
        orders.contains("10 × 7.00cr = 70.00cr"),
        "the order history renders in major units: {orders}"
    );
}

// ---- item 5: the economist answers the asker -------------------------------

fn invoke(
    tw: &mut TestWorld,
    asker: ObjId,
    name: &str,
    input: Value,
) -> Result<mush_core::InterfaceReply, mush_core::InterfaceError> {
    use mush_core::{Outbox, ParsedArgs, SwitchSet};
    let engine = tw.engine_mut();
    let provider = engine
        .registry
        .interface_provider(MARKET_INTERFACE)
        .expect("the market interface");
    let mut out = Outbox::new();
    let mut ctx = mush_core::Ctx {
        w: &mut engine.world,
        actor: asker,
        switches: SwitchSet::default(),
        args: ParsedArgs::default(),
        out: &mut out,
        reg: &engine.registry,
        module: Some(smol_str::SmolStr::new("npc")),
        typed_state: mush_core::ModuleActionState::unavailable(),
    };
    provider.invoke(
        &mut ctx,
        mush_core::AbilityCall { npc: GOD, asker },
        name,
        &input,
    )
}

/// **`market.list` and `market.quote` are the asker's alone.** The list was filtered
/// as the asker and the quote is her record; before IC0 both were said to the room.
#[test]
fn ic0_the_economist_answers_the_asker_and_not_the_room() {
    let Rig {
        mut tw,
        pat,
        listing,
        ..
    } = rig();
    let names: Vec<_> = MarketInterface
        .abilities()
        .into_iter()
        .map(|a| a.name)
        .collect();
    assert_eq!(names.len(), 2);
    let list = invoke(&mut tw, pat, "market.list", json!({})).expect("a list");
    assert_eq!(list.audience, mush_core::ReplyAudience::Asker);
    assert!(list.text.contains("10 × 7.00cr"), "{}", list.text);
    let quote = invoke(
        &mut tw,
        pat,
        "market.quote",
        json!({ "listing": listing.0, "quantity": 2 }),
    )
    .expect("a quote");
    assert_eq!(quote.audience, mush_core::ReplyAudience::Asker);
    assert!(
        quote.text.contains("2 × 7.00cr = 14.00cr"),
        "the economist's quote is in major units: {}",
        quote.text
    );
}
