//! ---- Doc 18 cut IC10: selling into a standing bid, as gates ----
//!
//! The market's half of the trade run's selling leg (doc 18 §4.3 "Bids that take real
//! lots", D-18-26, §4.1 "The share", D-18-24): the **lot sell arm**
//! (`+market/sell <listing>=<qty>` → [`mush_market::sell_into_bid`]), the fifth op of
//! the one lot action (`deliver`, whose prepare is inventory's own
//! `InventoryFamilies::transfer_marked`), the contract gated on
//! `Fact{inventory, inventory.transferred, <marker>}` so **money never moves before the
//! matter**, the game-neutral proceeds seam ([`mush_market::ProceedsPolicy`]), the
//! re-stage ([`mush_market::restage_deliver`] behind `+market/accept <quote>`) and the
//! exhausted bid that stands at quantity 0.
//!
//! The rig is IC7's production composition in miniature: `mush-org`, `mush-economy`
//! with an EMPTY frozen party-cap table and a test party-cap *source*, the market's
//! and inventory's fact providers, the market's observer, `mush-inventory` delegating
//! quantity writes to `"market"` (and `ic10probe`), and `mush-market` with one
//! lot-represented good, one instrument-represented good, a test presence gate and a
//! test proceeds policy. A tiny `ic10probe` module runs a queued closure in a command
//! context of its own, so a test can call `sell_into_bid(.., Presence::Ship(hull))`
//! the way IC11's executor will, or `post_wanted` with a spec no verb builds.
//!
//! **The rig rule for every test with more than one sale** (reviewer F10):
//! `Ctx::authoritative_pending` refuses a second sale `SELL_BUSY` while the first
//! submission is still queued, so each leg is taken (or landed) with `take`/`land`
//! before the next sell — taking stands in for the driver picking it up.
//!
//! Every test asserts the economy's ladder holds no `SettlementRefused` page.

use std::sync::{Arc, Mutex};

use mush_core::module::{GameModule, ModError, Registrar};
use mush_core::{
    ArgShape, Audience, AuthoritativeSubmit, CmdError, CommandSpec, CommitToken, Ctx, GOD,
    GameClock, IdempotencyKey, IdempotencyKeyText, IdempotencyScope, LIMBO, ObjId, ObjKind,
    OfferExecutionContext, OfferSubmission, PerceivedRef, ProjectedLegalAction,
    ProjectedParameterKind, RequestUid, ScopedIdempotencyKey,
};
use mush_economy::{
    AccountId, Amount, CatalogRef, ContractId, ContractState, EconomyStore, EscalationKind,
    EscalationSubject, ExternalRef, FactProvider, PartyCap, PartyCapSource, VaultId,
};
use mush_inventory::{
    EndpointRef, Grade, InventoryError, InventoryFactProvider, InventoryFamilies,
    InventoryHandleCell, InventoryHandles, InventoryModule, LotId,
};
use mush_market::{
    BidSaleRequest, CatalogEntry, CatalogSource, DELIVER_TOO_LATE, DRAW_SETTLE_MARGIN_SECS,
    DRAW_WINDOW_SECS, ListingId, MarketError, MarketFactProvider, MarketId, MarketStore,
    NO_SUCH_LISTING, PaidKind, Presence, Proceeds, ProceedsCut, ProceedsPolicy, ProceedsRefusal,
    QuoteId, SaleTerms, SettledSale, VendorId, VenueGate, VenueRefusal, WantedSpec,
};
use mush_org::{OrgId, PrincipalId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use serde_json::json;
use smol_str::SmolStr;

// ---- the test seams ---------------------------------------------------------

const PACK: &str = "ic10.test";
const CREDIT: &str = "credit";
const SCRIP: &str = "scrip";
const T0: i64 = 1_700_000_000;
/// Each seller's account, in minor units (they pay nothing to sell).
const FUNDS: i64 = 10_000;
/// The house's account.
const HOUSE_FUNDS: i64 = 1_000_000;
/// The bid, per unit (28.00cr), and the policy's fixed reference (20.00cr).
const BID: i64 = 2_800;
const REFERENCE: i64 = 2_000;
/// The bid's authored intake.
const INTAKE: u64 = 100;
/// Each seller's rooted iron.
const HELD: u64 = 50;
/// The port's house: the first org a fresh world allocates.
const VENDOR_ORG: OrgId = OrgId(0);

#[derive(Debug)]
struct TestCatalog;

impl CatalogSource for TestCatalog {
    fn pack(&self, pack: &str) -> Option<Result<Vec<CatalogEntry>, String>> {
        if pack != PACK {
            return None;
        }
        let entry = |code: &str, name: &str, baseline: i64, representation: &str| {
            json!({
                "catalog": { "ruleset": "ic10", "code": code },
                "pack": PACK,
                "definition_version": 1,
                "name": name,
                "baseline": baseline,
                "legality": "legal",
                "representation": representation,
                "stackable": true,
                "tags": ["test"],
            })
        };
        Some(
            [
                entry("iron", "Iron, pooled", 700, "lot"),
                entry("grain", "Grain, sacked", 100, "instrument"),
            ]
            .into_iter()
            .map(|value| serde_json::from_value::<CatalogEntry>(value).map_err(|e| e.to_string()))
            .collect(),
        )
    }
}

/// The party-cap source: organizations accept.
#[derive(Debug, Default)]
struct TestCapSource;

impl PartyCapSource for TestCapSource {
    fn has_cap(&self, _w: &mush_core::World, principal: &PrincipalId, cap: PartyCap) -> bool {
        cap == PartyCap::Accept && matches!(principal, PrincipalId::Organization(_))
    }
}

/// What the test gate knows, set after the world exists.
#[derive(Debug, Default)]
struct GateCfg {
    /// The one counter room of venue `luna`.
    room: Option<ObjId>,
    /// Each seller's governed hold.
    holds: Vec<(ObjId, EndpointRef)>,
    /// The one hull a `Presence::Ship` is admitted for.
    hull: Option<ObjId>,
    /// The org the gate calls `luna`'s port seller.
    port_seller: Option<OrgId>,
    /// The last presence `admit` saw.
    last: Option<Presence>,
}

fn luna() -> MarketId {
    MarketId::new("luna").expect("venue")
}

/// The gate's refusal line for an actor away from the counter.
const AWAY: &str = "You are not at Luna's counter.";

#[derive(Debug)]
struct TestGate(Arc<Mutex<GateCfg>>);

impl VenueGate for TestGate {
    fn venue_of_room(&self, _w: &mush_core::World, room: ObjId) -> Option<MarketId> {
        (self.0.lock().expect("cfg").room == Some(room)).then(luna)
    }
    fn endpoint_for(
        &self,
        _w: &mush_core::World,
        actor: ObjId,
        venue: &MarketId,
        _presence: Presence,
    ) -> Option<EndpointRef> {
        if *venue != luna() {
            return None;
        }
        let cfg = self.0.lock().expect("cfg");
        cfg.holds.iter().find(|(a, _)| *a == actor).map(|(_, h)| *h)
    }
    fn admit(
        &self,
        w: &mush_core::World,
        actor: ObjId,
        venue: &MarketId,
        _endpoint: Option<EndpointRef>,
        presence: Presence,
    ) -> Result<(), VenueRefusal> {
        let mut cfg = self.0.lock().expect("cfg");
        cfg.last = Some(presence);
        if *venue != luna() {
            return Ok(());
        }
        let admitted = match presence {
            Presence::Here => w.get(actor).and_then(|o| o.location) == cfg.room,
            Presence::Ship(hull) => Some(hull) == cfg.hull,
        };
        if admitted {
            Ok(())
        } else {
            Err(VenueRefusal {
                line: AWAY.to_owned(),
            })
        }
    }
    fn governed(&self, _w: &mush_core::World, endpoint: EndpointRef) -> bool {
        self.0
            .lock()
            .expect("cfg")
            .holds
            .iter()
            .any(|(_, h)| *h == endpoint)
    }
    fn port_seller(&self, _w: &mush_core::World, venue: &MarketId, seller: &PrincipalId) -> bool {
        *venue == luna()
            && self
                .0
                .lock()
                .expect("cfg")
                .port_seller
                .is_some_and(|org| *seller == PrincipalId::Organization(org))
    }
}

/// How the test proceeds policy answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// One cut of 250‰ of the margin over `REFERENCE` to the payee (none at 0).
    Share,
    /// Two cuts — one payee too many.
    TwoCuts,
    /// One cut over the whole sale.
    OverGross,
    /// One cut to an account in another instrument.
    OtherInstrument,
    /// A refusal in the policy's own words.
    Refuse,
}

const POLICY_REFUSES: &str = "The test policy will not price that sale.";

#[derive(Debug)]
struct ProceedsCfg {
    mode: Mode,
    payee: AccountId,
    other: AccountId,
    /// Every `settled` call: (quote, quantity, basis, Σ cuts).
    settled: Vec<(QuoteId, u64, i64, i64)>,
}

#[derive(Debug)]
struct TestProceeds(Arc<Mutex<ProceedsCfg>>);

impl ProceedsPolicy for TestProceeds {
    fn proceeds(
        &self,
        _w: &mush_core::World,
        sale: &SaleTerms<'_>,
    ) -> Result<Proceeds, ProceedsRefusal> {
        let cfg = self.0.lock().expect("cfg");
        let qty = i64::try_from(sale.quantity).expect("qty");
        let basis = (sale.price.minor() - REFERENCE).max(0) * qty;
        let share = basis * 250 / 1_000;
        let cut = |to: AccountId, amount: i64| ProceedsCut {
            to,
            amount: Amount::from_minor(amount),
            label: SmolStr::new_static("test share"),
        };
        let cuts = match cfg.mode {
            Mode::Share if share > 0 => vec![cut(cfg.payee, share)],
            Mode::Share => Vec::new(),
            Mode::TwoCuts => vec![cut(cfg.payee, 1), cut(cfg.payee, 1)],
            Mode::OverGross => vec![cut(cfg.payee, sale.gross.minor() + 1)],
            Mode::OtherInstrument => vec![cut(cfg.other, 1)],
            Mode::Refuse => {
                return Err(ProceedsRefusal {
                    line: POLICY_REFUSES.to_owned(),
                });
            }
        };
        let total: i64 = cuts.iter().map(|c| c.amount.minor()).sum();
        Ok(Proceeds {
            line: Some(format!(
                "sold {}: gross {}, share {}, net {}",
                sale.quantity,
                sale.gross.minor(),
                total,
                sale.gross.minor() - total
            )),
            cuts,
            basis: Amount::from_minor(basis),
            memo: None,
        })
    }

    fn settled(&self, _ctx: &mut Ctx<'_>, sale: &SettledSale<'_>) {
        let cuts: i64 = sale.cuts.iter().map(|c| c.amount.minor()).sum();
        self.0.lock().expect("cfg").settled.push((
            sale.quote,
            sale.quantity,
            sale.basis.minor(),
            cuts,
        ));
    }
}

/// `ic10probe`: one command that runs a queued closure in its own command context.
type ProbeJob = Box<dyn FnOnce(&mut Ctx<'_>) + Send>;

#[derive(Clone, Default)]
struct ProbeQueue(Arc<Mutex<Vec<ProbeJob>>>);

struct Probe;

impl GameModule for Probe {
    fn name(&self) -> &'static str {
        "ic10probe"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(ProbeQueue::default());
        reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
        reg.command(
            CommandSpec {
                name: "+ic10probe".into(),
                shape: ArgShape::Raw,
                help: "+ic10probe -- test: run the queued closure in a command context.".into(),
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
    tw.run(actor, "+ic10probe");
    slot.lock().expect("lock").take().expect("the probe ran")
}

// ---- the rig ---------------------------------------------------------------

#[derive(Clone, Copy)]
struct Opts {
    /// Compose `InventoryFactProvider` (the "no inventory namespace" roster leaves it
    /// out).
    inventory_facts: bool,
    /// What the house's account is funded with.
    house_funds: i64,
    /// The bid's intake.
    intake: u64,
    /// The bid's grade floor.
    min_grade: u8,
    /// How long the bid stands, in real seconds.
    bid_ttl_secs: i64,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts {
            inventory_facts: true,
            house_funds: HOUSE_FUNDS,
            intake: INTAKE,
            min_grade: 0,
            bid_ttl_secs: 86_400,
        }
    }
}

struct Rig {
    tw: TestWorld,
    families: InventoryFamilies,
    pat: ObjId,
    pat_account: AccountId,
    pat_hold: EndpointRef,
    quinn: ObjId,
    quinn_account: AccountId,
    quinn_hold: EndpointRef,
    shop: EndpointRef,
    vendor: VendorId,
    vendor_account: AccountId,
    trust_account: AccountId,
    root_scope: u64,
    faucet: AccountId,
    bid: ListingId,
    cfg: Arc<Mutex<GateCfg>>,
    policy: Arc<Mutex<ProceedsCfg>>,
    corridor: ObjId,
    hull: ObjId,
}

fn rig_with(opts: Opts) -> Rig {
    let cfg: Arc<Mutex<GateCfg>> = Arc::default();
    let policy = Arc::new(Mutex::new(ProceedsCfg {
        mode: Mode::Share,
        payee: AccountId(0),
        other: AccountId(0),
        settled: Vec::new(),
    }));
    let cell: InventoryHandleCell = Arc::default();
    let mut facts: Vec<Arc<dyn FactProvider>> = vec![Arc::new(MarketFactProvider)];
    if opts.inventory_facts {
        facts.push(Arc::new(InventoryFactProvider));
    }
    let economy = mush_economy::EconomyModule::new()
        .with_party_caps(Vec::new())
        .with_party_cap_source(Arc::new(TestCapSource))
        // Doc 18 cut IC14 row 7: the production composition's dispute veto.
        .with_dispute_guard(Arc::new(mush_market::SaleDisputeGuard))
        .with_observers(vec![Arc::new(mush_market::MarketObserver)])
        .with_facts(facts);
    let market = mush_market::MarketModule::new()
        .with_inventory(Arc::clone(&cell))
        .with_catalog_sources(vec![Arc::new(TestCatalog)])
        .with_venue_gate(Arc::new(TestGate(Arc::clone(&cfg))))
        .with_proceeds_policy(Arc::new(TestProceeds(Arc::clone(&policy))));
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(economy),
        Box::new(
            InventoryModule::new()
                .with_quantity_delegates(vec!["market", "ic10probe"])
                .with_handle_cell(Arc::clone(&cell)),
        ),
        Box::new(market),
        Box::new(Probe),
    ]);
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
    ok(&mut tw, GOD, &format!("@instrument/register {SCRIP}=0"));
    ok(&mut tw, GOD, &format!("@account/system faucet={CREDIT}"));
    let faucet = economy_of(&tw)
        .snapshot()
        .accounts()
        .find(|a| matches!(a.owner, PrincipalId::System(_)))
        .expect("the faucet")
        .id;

    // The house, its storefront, and its account, funded.
    ok(&mut tw, GOD, "@org/create Ares");
    let org = latest_org(&tw);
    assert_eq!(org, VENDOR_ORG);
    let root_scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().org(org).map(|o| o.root_scope))
        .expect("the root scope")
        .0;
    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:#{}={CREDIT}", org.0),
    );
    let vendor_account = account_in(&tw, &PrincipalId::Organization(org), CREDIT);
    if opts.house_funds > 0 {
        ok(
            &mut tw,
            GOD,
            &format!("@econ/mint {faucet}={vendor_account} {}", opts.house_funds),
        );
    }
    let backing = tw
        .world_mut()
        .create(ObjKind::Thing, "AresStore", GOD, None)
        .expect("a container");
    let before: Vec<VaultId> = economy_of(&tw).snapshot().vaults().map(|v| v.id).collect();
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vault/create AresStore=container:#{} owner=org:#{} scope=#{root_scope}",
            backing.0, org.0
        ),
    );
    let shop = EndpointRef(
        economy_of(&tw)
            .snapshot()
            .vaults()
            .map(|v| v.id)
            .find(|id| !before.contains(id))
            .expect("the vault exists"),
    );
    ok(&mut tw, GOD, &format!("@market/catalog load {PACK}"));
    ok(
        &mut tw,
        GOD,
        &format!(
            "@market/vendor create #{}=#{} name=Ares market=luna buy=300",
            org.0,
            shop.vault().0
        ),
    );
    let vendor = market_of(&tw).vendor_of_org(org).expect("registered").id;

    // The payee: a second org's account in the same instrument, and one in another.
    ok(&mut tw, GOD, "@org/create Trust");
    let trust = latest_org(&tw);
    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:#{}={CREDIT}", trust.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:#{}={SCRIP}", trust.0),
    );
    let trust_account = account_in(&tw, &PrincipalId::Organization(trust), CREDIT);
    let trust_scrip = account_in(&tw, &PrincipalId::Organization(trust), SCRIP);
    {
        let mut p = policy.lock().expect("policy");
        p.payee = trust_account;
        p.other = trust_scrip;
    }

    // The sellers, standing at Luna's counter (LIMBO), each with a governed hold of
    // rooted iron.
    let (pat, pat_account, pat_hold) = seller(&mut tw, "Pat", faucet);
    let (quinn, quinn_account, quinn_hold) = seller(&mut tw, "Quinn", faucet);
    seed_rooted_pool(&mut tw, &families, pat_hold, HELD, LotId(0xa1));
    seed_rooted_pool(&mut tw, &families, quinn_hold, HELD, LotId(0xa2));
    let corridor = tw
        .world_mut()
        .create(ObjKind::Room, "Corridor", GOD, None)
        .expect("a room");
    let hull = tw
        .world_mut()
        .create(ObjKind::Thing, "Dray", GOD, None)
        .expect("a hull");
    {
        let mut c = cfg.lock().expect("cfg");
        c.room = Some(LIMBO);
        c.holds = vec![(pat, pat_hold), (quinn, quinn_hold)];
        c.hull = Some(hull);
        c.port_seller = Some(org);
    }

    // The house's standing bid, posted as the IC8 applier posts it.
    let spec = WantedSpec {
        vendor,
        catalog: iron(),
        price: Amount::from_minor(BID),
        remaining: opts.intake,
        vault: shop.vault(),
        min_grade: opts.min_grade,
        ttl_secs: opts.bid_ttl_secs,
    };
    let bid = with_probe(&mut tw, GOD, move |ctx| mush_market::post_wanted(ctx, spec))
        .expect("the bid posts");
    assert!(tw.engine_mut().take_authoritative_submit().is_none());
    Rig {
        tw,
        families,
        pat,
        pat_account,
        pat_hold,
        quinn,
        quinn_account,
        quinn_hold,
        shop,
        vendor,
        vendor_account,
        trust_account,
        root_scope,
        faucet,
        bid,
        cfg,
        policy,
        corridor,
        hull,
    }
}

fn rig() -> Rig {
    rig_with(Opts::default())
}

fn latest_org(tw: &TestWorld) -> OrgId {
    tw.world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().orgs().map(|o| o.id).max())
        .expect("an org exists")
}

fn seller(tw: &mut TestWorld, name: &str, faucet: AccountId) -> (ObjId, AccountId, EndpointRef) {
    let who = tw.spawn_player(name);
    ok(tw, GOD, &format!("@account/open #{}={CREDIT}", who.0));
    let account = account_in(tw, &PrincipalId::Object(who), CREDIT);
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
    let hold = EndpointRef(
        economy_of(tw)
            .snapshot()
            .vaults()
            .find(|v| v.owner == PrincipalId::Object(who))
            .expect("the hold")
            .id,
    );
    (who, account, hold)
}

fn iron() -> CatalogRef {
    CatalogRef {
        ruleset: SmolStr::new_static("ic10"),
        code: SmolStr::new_static("iron"),
    }
}

// ---- helpers ---------------------------------------------------------------

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

fn economy_of(tw: &TestWorld) -> &EconomyStore {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("the economy store")
}

fn market_of(tw: &TestWorld) -> &MarketStore {
    tw.world()
        .module_state::<MarketStore>()
        .expect("the market store")
}

fn account_in(tw: &TestWorld, owner: &PrincipalId, instrument: &str) -> AccountId {
    economy_of(tw)
        .snapshot()
        .accounts()
        .find(|a| &a.owner == owner && a.instrument.as_str() == instrument)
        .expect("the account exists")
        .id
}

fn posted(tw: &TestWorld, account: AccountId) -> i64 {
    economy_of(tw)
        .snapshot()
        .account(account)
        .expect("the account exists")
        .posted_balance
        .minor()
}

fn held(tw: &TestWorld, account: AccountId) -> i64 {
    economy_of(tw)
        .snapshot()
        .account(account)
        .expect("the account exists")
        .held_amount
        .minor()
}

fn available(tw: &TestWorld, account: AccountId) -> i64 {
    posted(tw, account) - held(tw, account)
}

fn no_settlement_pages(tw: &TestWorld) {
    let pages: Vec<String> = economy_of(tw)
        .escalations()
        .filter(|e| matches!(e.kind, EscalationKind::SettlementRefused))
        .map(|e| e.context.clone())
        .collect();
    assert!(
        pages.is_empty(),
        "the ladder holds a SettlementRefused page: {pages:?}"
    );
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

fn seed_rooted_pool(
    tw: &mut TestWorld,
    families: &InventoryFamilies,
    at: EndpointRef,
    amount: u64,
    lot: LotId,
) {
    apply(tw, families, |txn| {
        families
            .mint_origin(
                txn,
                at,
                &commodity("iron"),
                amount,
                Grade::new(0).expect("grade"),
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

/// The iron pool's balance at an endpoint.
fn pool(tw: &mut TestWorld, families: &InventoryFamilies, at: EndpointRef) -> u64 {
    read(tw, families, |txn| {
        families
            .pool(txn, at, &commodity("iron"))
            .ok()
            .flatten()
            .map_or(0, |p| p.quantity)
    })
}

fn bid_qty(tw: &TestWorld, id: ListingId) -> Option<u64> {
    market_of(tw).listing(id).map(|l| l.quantity)
}

fn state_of(tw: &TestWorld, c: ContractId) -> Option<ContractState> {
    economy_of(tw).snapshot().contract(c).map(|c| c.state)
}

/// The seller's newest sale quote.
fn last_sale(tw: &TestWorld, seller: ObjId) -> mush_market::Quote {
    market_of(tw)
        .quotes_of_seller(&PrincipalId::Object(seller))
        .last()
        .cloned()
        .expect("a sale quote")
}

/// Take the one queued authoritative submission.
fn take(tw: &mut TestWorld) -> AuthoritativeSubmit {
    *tw.engine_mut()
        .take_authoritative_submit()
        .expect("a submission was staged")
}

/// **Drive one submission the way the world task does** (IC7's `land_submit`).
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

fn land(tw: &mut TestWorld, token: u8) -> Result<(), String> {
    let submit = take(tw);
    land_submit(tw, &submit, token)
}

/// Move the real clock to `at` and fire whatever is due.
fn at(tw: &mut TestWorld, at: i64) {
    tw.set_now(at);
    let _ = tw.fire_due(at * 1000);
}

/// Fire the sweeps (inventory and market 60 s, economy 30 s) until the contract settles.
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

/// The `market_lot` offer `actor` holds right now.
fn lot_offer(tw: &TestWorld, actor: ObjId) -> Option<ProjectedLegalAction> {
    let engine = tw.engine();
    let action = mush_market::lot_action_ref(&engine.registry)?;
    let query = engine
        .legal_action_query(
            mush_core::PrincipalId::Object { id: actor },
            actor,
            PerceivedRef::SelfActor,
        )
        .ok()?;
    let offers = engine.project_legal_actions(&query).ok()?;
    offers.projected_for_action(&action).cloned()
}

fn choices(offer: &ProjectedLegalAction, id: &str) -> Option<Vec<String>> {
    offer
        .parameters()
        .parameters()
        .iter()
        .find(|p| p.id.as_str() == id)
        .and_then(|p| match &p.kind {
            ProjectedParameterKind::Choice { choices } => Some(choices.clone()),
            _ => None,
        })
}

/// A hand-built `market_lot` submission, as an untrusted caller could queue one.
fn forged(tw: &TestWorld, actor: ObjId, parameters: serde_json::Value) -> AuthoritativeSubmit {
    AuthoritativeSubmit {
        actor,
        action: mush_market::lot_action_ref(&tw.engine().registry).expect("the lot action"),
        subject: PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters,
        key: IdempotencyKeyText::parse_canonical("market.lot.forged").expect("key"),
    }
}

/// What "nothing happened" is measured against.
#[derive(Debug, PartialEq, Eq)]
struct Books {
    contracts: usize,
    house_available: i64,
    house_held: i64,
    quotes: usize,
    bid: Option<u64>,
}

fn books(r: &Rig) -> Books {
    Books {
        contracts: economy_of(&r.tw).snapshot().contracts().count(),
        house_available: available(&r.tw, r.vendor_account),
        house_held: held(&r.tw, r.vendor_account),
        quotes: market_of(&r.tw).quotes().count(),
        bid: bid_qty(&r.tw, r.bid),
    }
}

/// A refusal before any contract: the line, and nothing written anywhere.
fn refused_before_any_contract(r: &mut Rig, who: ObjId, line: &str, want: &str) {
    let before = books(r);
    let said = err(&mut r.tw, who, line);
    assert!(said.contains(want), "`{line}`: {said}");
    assert_eq!(books(r), before, "`{line}` wrote something");
    assert!(
        r.tw.engine_mut().take_authoritative_submit().is_none(),
        "`{line}` queued a submission"
    );
}

/// [`refused_before_any_contract`], with its arguments evaluated before the rig is
/// borrowed.
macro_rules! refused {
    ($r:ident, $who:expr, $line:expr, $want:expr $(,)?) => {{
        let who = $who;
        let line = $line;
        let want = $want;
        refused_before_any_contract(&mut $r, who, &line, want);
    }};
}

/// The settlement transaction's `(account, delta)` lines.
fn settlement_lines(tw: &TestWorld, contract: ContractId) -> Vec<(AccountId, i64)> {
    let Some(ContractState::Settled { transaction }) = state_of(tw, contract) else {
        panic!("contract #{contract} is not settled");
    };
    let mut lines: Vec<(AccountId, i64)> = economy_of(tw)
        .transaction(transaction)
        .expect("the settlement transaction")
        .lines
        .iter()
        .map(|l| (l.account, l.delta.minor()))
        .collect();
    lines.sort();
    lines
}

// ---- 1: the sale, end to end --------------------------------------------------

/// **A sale into a bid settles only after the marker**: the house's payment is escrowed
/// and held through any number of sweeps until the deliver leg lands, and the contract
/// settles only once inventory's mirror has seen the marker. The journal splits the
/// gross exactly; the pools and lineage move once; the bid's intake drops by the sale.
#[test]
fn ic10_a_sale_into_a_bid_settles_only_after_the_marker() {
    let mut r = rig();
    let start_house = posted(&r.tw, r.vendor_account);
    let said = ok(&mut r.tw, r.pat, &format!("+market/sell {}=20", r.bid));
    let (gross, share) = (BID * 20, (BID - REFERENCE) * 20 * 250 / 1_000);
    let net = gross - share;
    let line = format!("sold 20: gross {gross}, share {share}, net {net}");
    assert!(said.lines().any(|l| l == line), "the policy's line: {said}");
    let q = last_sale(&r.tw, r.pat);
    let contract = q.contract.expect("the contract");
    let lot = q.lot.clone().expect("the frozen terms");
    assert_eq!(lot.kind, PaidKind::Deliver);
    assert_eq!(lot.source.0, r.pat_hold);
    assert_eq!(lot.destination.0, r.shop);
    assert_eq!(q.buyer, PrincipalId::Organization(VENDOR_ORG));
    assert_eq!(q.cuts.len(), 1);
    assert_eq!(q.basis, Amount::from_minor((BID - REFERENCE) * 20));
    assert!(q.activated);
    assert_eq!(bid_qty(&r.tw, r.bid), Some(INTAKE - 20));
    assert!(matches!(
        state_of(&r.tw, contract),
        Some(ContractState::Active)
    ));
    let name = economy_of(&r.tw)
        .snapshot()
        .contract(contract)
        .map(|c| c.name.to_string())
        .expect("named");
    assert_eq!(name, format!("market sale #{} to bid #{}", q.id, r.bid));

    // The queued leg: the seller's own `deliver`, keyed by the contract.
    let leg = take(&mut r.tw);
    assert_eq!(leg.actor, r.pat);
    assert_eq!(
        leg.key.as_str(),
        format!("market.lot.deliver.{:x}", contract.0)
    );
    assert_eq!(leg.parameters["op"], json!("deliver"));
    assert_eq!(leg.parameters["contract"], json!(contract.0.to_string()));
    assert_eq!(
        leg.parameters["destination"],
        json!(r.shop.vault().0.to_string())
    );
    assert_eq!(
        leg.parameters["endpoint"],
        json!(r.pat_hold.vault().0.to_string())
    );

    // Money never moves before the matter: three economy sweeps, nothing paid.
    let now = r.tw.world().now();
    for i in 1..=3 {
        at(&mut r.tw, now + i * 31);
    }
    assert!(matches!(
        state_of(&r.tw, contract),
        Some(ContractState::Active)
    ));
    assert_eq!(posted(&r.tw, r.pat_account), FUNDS);
    assert_eq!(posted(&r.tw, r.trust_account), 0);
    assert_eq!(posted(&r.tw, r.vendor_account), start_house);
    assert_eq!(held(&r.tw, r.vendor_account), gross, "the escrow is held");

    // The leg lands; until inventory's mirror sweeps, the fact answers nothing.
    land_submit(&mut r.tw, &leg, 0x11).expect("the deliver lands");
    assert_eq!(pool(&mut r.tw, &r.families, r.pat_hold), HELD - 20);
    assert_eq!(pool(&mut r.tw, &r.families, r.shop), 20);
    let marker = lot.marker.clone().expect("the marker");
    assert!(
        InventoryFactProvider
            .resolve(
                r.tw.world(),
                mush_inventory::FACT_TRANSFERRED,
                &marker,
                None
            )
            .is_none(),
        "the mirror has not swept"
    );
    assert!(matches!(
        state_of(&r.tw, contract),
        Some(ContractState::Active)
    ));
    settle(&mut r.tw, contract);

    // The journal: house −gross, seller +net, payee +share, summing to zero.
    let mut want = vec![
        (r.vendor_account, -gross),
        (r.pat_account, net),
        (r.trust_account, share),
    ];
    want.sort();
    assert_eq!(settlement_lines(&r.tw, contract), want);
    assert_eq!(want.iter().map(|(_, d)| d).sum::<i64>(), 0);
    assert_eq!(posted(&r.tw, r.vendor_account), start_house - gross);
    assert_eq!(held(&r.tw, r.vendor_account), 0);
    assert_eq!(posted(&r.tw, r.pat_account), FUNDS + net);
    assert_eq!(posted(&r.tw, r.trust_account), share);

    // Lineage: the storefront root inherits the hold's, carrying 20; the marker names
    // the move.
    let families = r.families.clone();
    let (hold, shop) = (r.pat_hold, r.shop);
    let (source_root, dest_root, record) = read(&mut r.tw, &families, |txn| {
        (
            families
                .tranche_for(txn, hold, &commodity("iron"))
                .expect("reads")
                .expect("the hold's root"),
            families
                .tranche_for(txn, shop, &commodity("iron"))
                .expect("reads")
                .expect("the storefront's root"),
            txn.get(&families.applied, &marker)
                .expect("reads")
                .cloned()
                .expect("the marker"),
        )
    });
    assert_eq!(dest_root.1.inherited(), &[source_root.0]);
    assert_eq!(dest_root.1.quantity(), 20);
    assert_eq!(record.kind(), mush_inventory::FACT_TRANSFERRED);
    assert_eq!(
        (
            record.from(),
            record.to(),
            record.quantity(),
            record.actor()
        ),
        (Some(hold), Some(shop), 20, r.pat)
    );
    assert_eq!(bid_qty(&r.tw, r.bid), Some(INTAKE - 20));
    let calls = r.policy.lock().expect("policy").settled.clone();
    assert_eq!(calls, vec![(q.id, 20, (BID - REFERENCE) * 20, share)]);
    assert!(market_of(&r.tw).indexes_agree());
    no_settlement_pages(&r.tw);
}

// ---- 2: the split ----------------------------------------------------------------

/// **The proceeds split is exact and one payee only**: two cuts, cuts over the gross and
/// a payee in another instrument are each refused `UNSETTLEABLE` before any contract.
#[test]
fn ic10_the_proceeds_split_is_exact_and_one_payee_only() {
    let mut r = rig();
    for mode in [Mode::TwoCuts, Mode::OverGross, Mode::OtherInstrument] {
        r.policy.lock().expect("policy").mode = mode;
        let line = format!("+market/sell {}=20", r.bid);
        refused!(r, r.pat, line, mush_market::UNSETTLEABLE);
    }
    r.policy.lock().expect("policy").mode = Mode::Share;
    ok(&mut r.tw, r.pat, &format!("+market/sell {}=20", r.bid));
    no_settlement_pages(&r.tw);
}

// ---- 3: every refusal before any contract ------------------------------------------

/// **Every refusal comes before any contract**: no new contract, no hold on the house,
/// no queued submission, no quote row, the bid's quantity unchanged.
#[test]
fn ic10_every_refusal_comes_before_any_contract() {
    let mut r = rig();
    let bid = r.bid;
    // Over-selling: more than the hold holds.
    refused!(
        r,
        r.pat,
        format!("+market/sell {bid}=51"),
        "You have 50 of that there to sell, not 51, so nothing was sold.",
    );
    // Over-intake.
    refused!(
        r,
        r.pat,
        format!("+market/sell {bid}=101"),
        "That house wants only 100 more, so nothing was sold.",
    );
    // A stranger's hold named with from=.
    refused!(
        r,
        r.pat,
        format!("+market/sell {bid}=5 from={}", r.quinn_hold.vault().0),
        NO_SUCH_LISTING,
    );
    // A non-bid listing.
    ok(
        &mut r.tw,
        GOD,
        &format!(
            "@market/vendor stock #{}=ic10:grain price=100 qty=5",
            r.vendor.0
        ),
    );
    let ask = market_of(&r.tw)
        .live_listings()
        .filter(|l| !l.source.is_wanted())
        .map(|l| l.id)
        .max()
        .expect("the ask");
    refused!(
        r,
        r.pat,
        format!("+market/sell {ask}=1"),
        mush_market::SELLS_NOT_BUYS,
    );
    // A policy refusal, in the policy's own words.
    r.policy.lock().expect("policy").mode = Mode::Refuse;
    refused!(r, r.pat, format!("+market/sell {bid}=5"), POLICY_REFUSES,);
    r.policy.lock().expect("policy").mode = Mode::Share;

    // A queued first sale: a second one is busy, not short.
    ok(&mut r.tw, r.pat, &format!("+market/sell {bid}=30"));
    let first = books(&r);
    let said = err(&mut r.tw, r.pat, &format!("+market/sell {bid}=10"));
    assert!(said.contains(mush_market::SELL_BUSY), "{said}");
    assert_eq!(books(&r), first);
    // The first sale's leg taken but not landed: the in-flight tonnes are not sellable.
    let leg = take(&mut r.tw);
    assert_eq!(leg.parameters["op"], json!("deliver"));
    refused!(
        r,
        r.pat,
        format!("+market/sell {bid}=30"),
        "You have 20 of that there to sell, not 30, so nothing was sold.",
    );
    // Landed, the pool itself is 20 and nothing is subtracted twice.
    land_submit(&mut r.tw, &leg, 0x31).expect("the first leg lands");
    ok(&mut r.tw, r.pat, &format!("+market/sell {bid}=20"));
    let _ = take(&mut r.tw);

    // The lapsed bid.
    let deadline = market_of(&r.tw)
        .listing(bid)
        .map(|l| l.expires.at)
        .expect("the bid");
    r.tw.set_now(deadline);
    refused!(
        r,
        r.quinn,
        format!("+market/sell {bid}=1"),
        mush_market::BID_LAPSED,
    );
    no_settlement_pages(&r.tw);

    // An unfunded house.
    let mut r = rig_with(Opts {
        house_funds: 10_000,
        ..Opts::default()
    });
    let line = format!("+market/sell {}=20", r.bid);
    refused!(r, r.pat, line, mush_market::UNFUNDED);
    // A graded bid.
    let mut r = rig_with(Opts {
        min_grade: 1,
        ..Opts::default()
    });
    let line = format!("+market/sell {}=20", r.bid);
    refused!(r, r.pat, line, mush_market::GRADED_BID);
    // No `inventory` fact namespace on the roster.
    let mut r = rig_with(Opts {
        inventory_facts: false,
        ..Opts::default()
    });
    let line = format!("+market/sell {}=20", r.bid);
    refused!(r, r.pat, line, mush_market::UNSETTLEABLE);
    no_settlement_pages(&r.tw);
}

// ---- 4: the race --------------------------------------------------------------------

/// **Two sellers racing the last intake**: the second is refused `OVER_INTAKE` before
/// any contract.
#[test]
fn ic10_two_sellers_racing_the_last_intake() {
    let mut r = rig_with(Opts {
        intake: 30,
        ..Opts::default()
    });
    let bid = r.bid;
    ok(&mut r.tw, r.pat, &format!("+market/sell {bid}=20"));
    let _ = take(&mut r.tw);
    refused!(
        r,
        r.quinn,
        format!("+market/sell {bid}=20"),
        "That house wants only 10 more, so nothing was sold.",
    );
    ok(&mut r.tw, r.quinn, &format!("+market/sell {bid}=10"));
    assert_eq!(bid_qty(&r.tw, bid), Some(0));
    no_settlement_pages(&r.tw);
}

// ---- 5: an unlanded leg ----------------------------------------------------------------

/// **An unlanded leg refunds and restores the bid**: past the cutoff the re-stage is too
/// late while the contract still stands; past the window the contract expires, the
/// house is refunded, the seller is unpaid, the hold is untouched and the bid comes back
/// within one market sweep; `settled` is never called.
#[test]
fn ic10_an_unlanded_leg_refunds_and_restores_the_bid() {
    let mut r = rig();
    let start_house = posted(&r.tw, r.vendor_account);
    ok(&mut r.tw, r.pat, &format!("+market/sell {}=20", r.bid));
    let _dropped = take(&mut r.tw);
    let q = last_sale(&r.tw, r.pat);
    let contract = q.contract.expect("contract");
    let accepted_at = q.lot.as_ref().map(|l| l.accepted_at).expect("terms");

    r.tw.set_now(accepted_at + DRAW_WINDOW_SECS - DRAW_SETTLE_MARGIN_SECS + 1);
    let late = err(&mut r.tw, r.pat, &format!("+market/accept {}", q.id));
    assert!(late.contains(DELIVER_TOO_LATE), "{late}");
    assert!(r.tw.engine_mut().take_authoritative_submit().is_none());
    assert!(matches!(
        state_of(&r.tw, contract),
        Some(ContractState::Active)
    ));

    at(&mut r.tw, accepted_at + DRAW_WINDOW_SECS + 1);
    at(&mut r.tw, accepted_at + DRAW_WINDOW_SECS + 62);
    assert!(matches!(
        state_of(&r.tw, contract),
        Some(ContractState::Expired)
    ));
    assert_eq!(posted(&r.tw, r.vendor_account), start_house);
    assert_eq!(held(&r.tw, r.vendor_account), 0, "the house is refunded");
    assert_eq!(posted(&r.tw, r.pat_account), FUNDS, "the seller is unpaid");
    assert_eq!(posted(&r.tw, r.trust_account), 0);
    assert_eq!(pool(&mut r.tw, &r.families, r.pat_hold), HELD);
    assert_eq!(bid_qty(&r.tw, r.bid), Some(INTAKE), "the bid is restored");
    assert!(
        market_of(&r.tw).quote(q.id).is_none(),
        "the sale is forgotten"
    );
    assert!(r.policy.lock().expect("policy").settled.is_empty());
    no_settlement_pages(&r.tw);
}

// ---- 6: the cutoff and the lapsed paid sale --------------------------------------------

/// **Past `draw_by` the deliver entry is never offered**, so a stale leg is refused at
/// resolution; and **a landed leg whose contract lapses unpaid** (the inventory mirror
/// starved past the deadline) restores nothing and pages once.
#[test]
fn ic10_a_landed_leg_past_the_cutoff_is_never_offered_and_a_lapsed_paid_sale_pages() {
    let mut r = rig();
    ok(&mut r.tw, r.pat, &format!("+market/sell {}=20", r.bid));
    let stale = take(&mut r.tw);
    let q = last_sale(&r.tw, r.pat);
    let contract = q.contract.expect("contract");
    let offer = lot_offer(&r.tw, r.pat).expect("offered");
    assert_eq!(
        choices(&offer, "contract"),
        Some(vec![contract.0.to_string()])
    );
    assert!(choices(&offer, "op").is_some_and(|ops| ops.contains(&"deliver".to_owned())));
    let accepted_at = q.lot.as_ref().map(|l| l.accepted_at).expect("terms");
    r.tw.set_now(accepted_at + DRAW_WINDOW_SECS - DRAW_SETTLE_MARGIN_SECS + 1);
    let offer = lot_offer(&r.tw, r.pat).expect("still offered reserve/release");
    assert!(choices(&offer, "contract").is_none());
    assert!(choices(&offer, "op").is_some_and(|ops| !ops.contains(&"deliver".to_owned())));
    let refusal = land_submit(&mut r.tw, &stale, 0x61).expect_err("past the cutoff");
    assert!(refusal.starts_with("resolve:"), "{refusal}");
    assert_eq!(pool(&mut r.tw, &r.families, r.pat_hold), HELD);
    no_settlement_pages(&r.tw);

    // A landed leg, and no sweep until past the deadline.
    let mut r = rig();
    let start_house = posted(&r.tw, r.vendor_account);
    ok(&mut r.tw, r.pat, &format!("+market/sell {}=20", r.bid));
    land(&mut r.tw, 0x62).expect("the deliver lands");
    let q = last_sale(&r.tw, r.pat);
    let contract = q.contract.expect("contract");
    let accepted_at = q.lot.as_ref().map(|l| l.accepted_at).expect("terms");
    at(&mut r.tw, accepted_at + DRAW_WINDOW_SECS + 1);
    at(&mut r.tw, accepted_at + DRAW_WINDOW_SECS + 62);
    at(&mut r.tw, accepted_at + DRAW_WINDOW_SECS + 123);
    assert!(matches!(
        state_of(&r.tw, contract),
        Some(ContractState::Expired)
    ));
    assert_eq!(bid_qty(&r.tw, r.bid), Some(INTAKE - 20), "not restored");
    assert_eq!(posted(&r.tw, r.vendor_account), start_house, "refunded");
    assert_eq!(
        posted(&r.tw, r.pat_account),
        FUNDS,
        "the seller was not paid"
    );
    assert_eq!(
        pool(&mut r.tw, &r.families, r.shop),
        20,
        "the house holds it"
    );
    let pages: Vec<String> = economy_of(&r.tw)
        .escalations()
        .filter(|e| matches!(e.kind, EscalationKind::BootReconciliation))
        .filter(|e| matches!(e.subject, EscalationSubject::Contract(c) if c == contract))
        .map(|e| e.context.clone())
        .collect();
    assert_eq!(
        pages,
        vec![format!(
            "market sale quote #{}'s goods reached the house but contract #{contract} lapsed \
             unpaid, so the bid was not restored. The seller was not paid; settle it by hand.",
            q.id
        )]
    );
    assert!(r.policy.lock().expect("policy").settled.is_empty());
    no_settlement_pages(&r.tw);
}

// ---- 7: the replay guard ------------------------------------------------------------

/// **A replayed deliver moves nothing twice**: the same parameters under a fresh request
/// uid are refused by the marker.
#[test]
fn ic10_a_replayed_deliver_moves_nothing_twice() {
    let mut r = rig();
    ok(&mut r.tw, r.pat, &format!("+market/sell {}=20", r.bid));
    let leg = take(&mut r.tw);
    land_submit(&mut r.tw, &leg, 0x71).expect("the deliver lands");
    let refusal = land_submit(&mut r.tw, &leg, 0x72).expect_err("the marker refuses");
    assert!(refusal.starts_with("prepare:"), "{refusal}");
    assert_eq!(pool(&mut r.tw, &r.families, r.pat_hold), HELD - 20);
    assert_eq!(pool(&mut r.tw, &r.families, r.shop), 20);
    let contract = last_sale(&r.tw, r.pat).contract.expect("contract");
    settle(&mut r.tw, contract);
    assert_eq!(r.policy.lock().expect("policy").settled.len(), 1);
    no_settlement_pages(&r.tw);
}

// ---- 8: only the seller --------------------------------------------------------------

/// **Only the seller is offered the deliver leg**: the house's officer, a stranger and
/// a buyer of something else see no `deliver` op and no such contract choice, and a
/// forged `deliver` naming another's contract fails resolution.
#[test]
fn ic10_only_the_seller_is_offered_the_deliver_leg() {
    let mut r = rig();
    ok(&mut r.tw, r.pat, &format!("+market/sell {}=20", r.bid));
    let leg = take(&mut r.tw);
    let contract = last_sale(&r.tw, r.pat).contract.expect("contract");
    let pat_offer = lot_offer(&r.tw, r.pat).expect("offered");
    assert_eq!(
        choices(&pat_offer, "op"),
        Some(vec![
            "reserve".to_owned(),
            "release".to_owned(),
            "deliver".to_owned()
        ])
    );

    // The house's officer: endpoints of the org, and no deliver.
    let officer = r.tw.spawn_player("Officer");
    for capability in ["economy.account.post", "vault.withdraw"] {
        ok(
            &mut r.tw,
            GOD,
            &format!(
                "@org/grant #{}=Ares {capability} scope=#{}",
                officer.0, r.root_scope
            ),
        );
    }
    // A stranger with a hold of her own.
    let (sam, _sam_account, _sam_hold) = seller(&mut r.tw, "Sam", r.faucet);
    // A buyer of something else: Quinn holds a paid Draw of her own.
    seed_rooted_pool(&mut r.tw, &r.families.clone(), r.shop, 40, LotId(0xa3));
    ok(
        &mut r.tw,
        GOD,
        &format!(
            "@market/vendor stock #{}=ic10:iron price=700 qty=10",
            r.vendor.0
        ),
    );
    let ask = market_of(&r.tw)
        .live_listings()
        .filter(|l| !l.source.is_wanted())
        .map(|l| l.id)
        .max()
        .expect("the Draw ask");
    let said = ok(&mut r.tw, r.quinn, &format!("+market/quote {ask}=5"));
    let qq = said
        .split("Quote #")
        .nth(1)
        .and_then(|rest| rest.split(':').next())
        .and_then(|n| n.trim().parse::<u64>().ok())
        .expect("a quote id");
    ok(&mut r.tw, r.quinn, &format!("+market/accept {qq}"));
    let quinn_draw = take(&mut r.tw);
    assert_eq!(quinn_draw.parameters["op"], json!("draw"));
    let quinn_contract = quinn_draw.parameters["contract"].clone();

    for (who, label) in [(officer, "officer"), (sam, "stranger"), (r.quinn, "buyer")] {
        let offer = lot_offer(&r.tw, who).expect("an endpoint holder is offered the action");
        let ops = choices(&offer, "op").expect("ops");
        assert!(!ops.contains(&"deliver".to_owned()), "{label}: {ops:?}");
        let contracts = choices(&offer, "contract").unwrap_or_default();
        assert!(
            !contracts.contains(&contract.0.to_string()),
            "{label}: {contracts:?}"
        );
    }
    assert_eq!(
        choices(&lot_offer(&r.tw, r.quinn).expect("offered"), "contract"),
        Some(vec![quinn_contract.as_str().unwrap_or_default().to_owned()])
    );

    // A forged deliver of Pat's contract, from Quinn's own hold.
    let mut stolen = leg.parameters.clone();
    stolen["endpoint"] = json!(r.quinn_hold.vault().0.to_string());
    stolen["source"] = json!(r.quinn_hold.vault().0.to_string());
    let forgery = forged(&r.tw, r.quinn, stolen);
    let refusal = land_submit(&mut r.tw, &forgery, 0x81).expect_err("not her contract");
    assert!(refusal.starts_with("resolve:"), "{refusal}");
    assert!(refusal.contains("InvalidParameters"), "{refusal}");
    assert_eq!(pool(&mut r.tw, &r.families, r.quinn_hold), HELD);
    // The seller's own leg, forged (IC10 fixer, review T6): the destination, the
    // quantity, and the source and endpoint are each tied to the server's entry, so a
    // seller can neither divert the goods to a vault of their own, nor hand over more
    // or fewer than the contract names, nor hand over out of another endpoint — each
    // is refused, nothing moves and no marker is written (the contract's Fact never
    // turns true for goods the house did not receive).
    let marker = last_sale(&r.tw, r.pat)
        .lot
        .and_then(|l| l.marker)
        .expect("the sale's marker");
    let mut forgeries: Vec<(&str, serde_json::Value)> = Vec::new();
    for (label, destination) in [
        ("to Quinn's hold", r.quinn_hold.vault().0),
        ("to Pat's own hold", r.pat_hold.vault().0),
    ] {
        let mut p = leg.parameters.clone();
        p["destination"] = json!(destination.to_string());
        forgeries.push((label, p));
    }
    for (label, quantity) in [("fewer tonnes", 19_u64), ("more tonnes", 21)] {
        let mut p = leg.parameters.clone();
        p["quantity"] = json!(quantity);
        forgeries.push((label, p));
    }
    let mut p = leg.parameters.clone();
    p["endpoint"] = json!(r.quinn_hold.vault().0.to_string());
    p["source"] = json!(r.quinn_hold.vault().0.to_string());
    forgeries.push(("out of Quinn's hold", p));
    let mut p = leg.parameters.clone();
    p["source"] = json!(r.shop.vault().0.to_string());
    forgeries.push(("a source that is not the endpoint", p));
    let families = r.families.clone();
    for (n, (label, parameters)) in forgeries.into_iter().enumerate() {
        let forgery = forged(&r.tw, r.pat, parameters);
        let token = 0x90 + u8::try_from(n).expect("a few forgeries");
        let refusal = land_submit(&mut r.tw, &forgery, token);
        assert!(refusal.is_err(), "{label}: the forged leg landed");
        assert_eq!(pool(&mut r.tw, &families, r.pat_hold), HELD, "{label}");
        assert_eq!(pool(&mut r.tw, &families, r.quinn_hold), HELD, "{label}");
        assert_eq!(pool(&mut r.tw, &families, r.shop), 40, "{label}");
        let written = read(&mut r.tw, &families, |txn| {
            txn.get(&families.applied, &marker)
                .expect("the applied family reads")
                .is_some()
        });
        assert!(!written, "{label}: a marker was written");
    }
    // The genuine legs still land.
    land_submit(&mut r.tw, &leg, 0x82).expect("Pat's deliver lands");
    land_submit(&mut r.tw, &quinn_draw, 0x83).expect("Quinn's draw lands");
    no_settlement_pages(&r.tw);
}

// ---- 9: the gate ---------------------------------------------------------------------

/// **The sell gate, `Here` and `Ship`** (IC7's untested gate): away from the counter the
/// sale is refused in the gate's words before any write; at it, it sells; through the
/// probe, `Presence::Ship(hull)` with no source sells for the admitted hull and is
/// refused for another; and the **object arm** from outside the counter room is refused
/// by the gate before any escrow.
#[test]
fn ic10_the_sell_gate_here_and_ship() {
    let mut r = rig();
    let bid = r.bid;
    r.tw.world_mut()
        .move_to(r.pat, r.corridor)
        .expect("walk out");
    refused!(r, r.pat, format!("+market/sell {bid}=5"), AWAY);

    // The object arm, from the corridor: the gate refuses before any escrow.
    let thing =
        r.tw.world_mut()
            .create(ObjKind::Thing, "Trinket", r.pat, Some(r.pat))
            .expect("a carried thing");
    refused!(
        r,
        r.pat,
        format!("+market/sell #{}={}", thing.0, r.vendor.0),
        AWAY,
    );

    // Back at the counter, it sells.
    r.tw.world_mut().move_to(r.pat, LIMBO).expect("walk in");
    ok(&mut r.tw, r.pat, &format!("+market/sell {bid}=5"));
    let _ = take(&mut r.tw);

    // A queued order on its hull, away from the counter, with no source named.
    r.tw.world_mut()
        .move_to(r.pat, r.corridor)
        .expect("walk out");
    let hull = r.hull;
    let req = BidSaleRequest {
        listing: bid,
        quantity: 5,
        source: None,
        presence: Presence::Ship(hull),
    };
    let sold = with_probe(&mut r.tw, r.pat, move |ctx| {
        mush_market::sell_into_bid(ctx, &req).map_err(|e| format!("{e:?}"))
    })
    .expect("admitted on its hull");
    assert_eq!(r.cfg.lock().expect("cfg").last, Some(Presence::Ship(hull)));
    assert_eq!(sold.gross, Amount::from_minor(BID * 5));
    let leg = take(&mut r.tw);
    assert_eq!(leg.parameters["op"], json!("deliver"));
    let other = r.corridor;
    let req = BidSaleRequest {
        listing: bid,
        quantity: 5,
        source: None,
        presence: Presence::Ship(other),
    };
    let before = books(&r);
    let refused = with_probe(&mut r.tw, r.pat, move |ctx| {
        mush_market::sell_into_bid(ctx, &req).map_err(|e| format!("{e:?}"))
    })
    .expect_err("another hull is refused");
    assert!(refused.contains(AWAY), "{refused}");
    assert_eq!(books(&r), before);
    no_settlement_pages(&r.tw);
}

// ---- 10: the exhausted bid -----------------------------------------------------------------

/// **An exhausted bid stands at zero and reopens on a refund**: listed " — not buying"
/// before its deadline, refusing a further sale, kept by the sweep past its deadline,
/// blocking a second bid, restored by an expired sale of its last tonnes, and deleted
/// only by a withdrawal — after which a new bid posts.
#[test]
fn ic10_an_exhausted_bid_stands_at_zero_and_reopens_on_a_refund() {
    let mut r = rig_with(Opts {
        intake: 30,
        ..Opts::default()
    });
    let bid = r.bid;
    ok(&mut r.tw, r.pat, &format!("+market/sell {bid}=20"));
    land(&mut r.tw, 0xa1).expect("Pat's leg lands");
    // Settled before any clock jump below (IC10 fixer, review T2): a landed sale left
    // unswept past its own deadline would lapse unpaid instead.
    let pat_first = last_sale(&r.tw, r.pat).contract.expect("contract");
    settle(&mut r.tw, pat_first);
    // Quinn takes the last ten; her leg is dropped.
    ok(&mut r.tw, r.quinn, &format!("+market/sell {bid}=10"));
    let _dropped = take(&mut r.tw);
    let quinn_sale = last_sale(&r.tw, r.quinn);
    assert_eq!(bid_qty(&r.tw, bid), Some(0), "the bid stands at zero");
    assert!(market_of(&r.tw).indexes_agree());
    let listed = ok(&mut r.tw, r.pat, "+market/list");
    let row = listed
        .lines()
        .find(|l| l.starts_with(&format!("#{bid} ")))
        .unwrap_or_else(|| panic!("the zero bid is listed: {listed}"));
    assert!(row.ends_with(" — not buying"), "{row}");
    refused!(
        r,
        r.pat,
        format!("+market/sell {bid}=1"),
        mush_market::NOT_BUYING,
    );
    // A second bid for the same good beside the zero one is refused.
    let spec = WantedSpec {
        vendor: r.vendor,
        catalog: iron(),
        price: Amount::from_minor(BID),
        remaining: 10,
        vault: r.shop.vault(),
        min_grade: 0,
        ttl_secs: 86_400,
    };
    let again = spec.clone();
    let refused = with_probe(&mut r.tw, GOD, move |ctx| {
        mush_market::post_wanted(ctx, again)
    });
    assert_eq!(
        refused,
        Err(MarketError::bad(
            "That vendor already posts a standing bid for that good."
        ))
    );
    // The applier's revise to nothing left writes the exhausted marker too.
    let revised = with_probe(&mut r.tw, GOD, move |ctx| {
        mush_market::revise_wanted(ctx, bid, Amount::from_minor(BID), 0)
    });
    assert_eq!(revised, Ok(()));
    assert_eq!(bid_qty(&r.tw, bid), Some(0));

    // Quinn's sale expires unlanded **before** the bid's own deadline (its window is an
    // hour, the bid's a day): the refund restores its ten. (A refund landing past the
    // bid's deadline is the last case below, owner item O4(b).)
    let deadline = market_of(&r.tw)
        .listing(bid)
        .map(|l| l.expires.at)
        .expect("the bid");
    let accepted_at = quinn_sale
        .lot
        .as_ref()
        .map(|l| l.accepted_at)
        .expect("terms");
    assert!(accepted_at + DRAW_WINDOW_SECS < deadline);
    at(&mut r.tw, accepted_at + DRAW_WINDOW_SECS + 1);
    at(&mut r.tw, accepted_at + DRAW_WINDOW_SECS + 62);
    assert_eq!(bid_qty(&r.tw, bid), Some(10), "the refund re-opens it");
    // Pat sells the ten again (landed), exhausting it; past its deadline the sweep keeps
    // it.
    ok(&mut r.tw, r.pat, &format!("+market/sell {bid}=10"));
    land(&mut r.tw, 0xa2).expect("lands");
    assert_eq!(bid_qty(&r.tw, bid), Some(0));
    let pat_second = last_sale(&r.tw, r.pat).contract.expect("contract");
    settle(&mut r.tw, pat_second);
    at(&mut r.tw, deadline + 1);
    at(&mut r.tw, deadline + 62);
    assert_eq!(bid_qty(&r.tw, bid), Some(0), "the sweep never deletes it");
    assert!(market_of(&r.tw).indexes_agree());
    // Withdrawn by hand, it is deleted, and a new bid posts.
    ok(&mut r.tw, GOD, &format!("@market/vendor withdraw {bid}"));
    assert!(market_of(&r.tw).listing(bid).is_none());
    let fresh = with_probe(&mut r.tw, GOD, move |ctx| {
        mush_market::post_wanted(ctx, spec)
    });
    let fresh = fresh.expect("a new bid posts beside nothing");

    // Owner item O4(b): Quinn takes the new bid's whole ten shortly before its deadline
    // and her leg is dropped; past the deadline the sweep keeps the zero bid while her
    // sale stands, the refund past the deadline restores its ten, and the sweep then
    // deletes it as lapsed (the port's clerk re-posts a lapsed bid).
    let fresh_deadline = market_of(&r.tw)
        .listing(fresh)
        .map(|l| l.expires.at)
        .expect("the new bid");
    at(&mut r.tw, fresh_deadline - 100);
    ok(&mut r.tw, r.quinn, &format!("+market/sell {fresh}=10"));
    let _dropped = take(&mut r.tw);
    let late = last_sale(&r.tw, r.quinn);
    let late_accepted = late.lot.as_ref().map(|l| l.accepted_at).expect("terms");
    assert_eq!(bid_qty(&r.tw, fresh), Some(0));
    at(&mut r.tw, fresh_deadline + 1);
    at(&mut r.tw, fresh_deadline + 62);
    assert_eq!(
        bid_qty(&r.tw, fresh),
        Some(0),
        "the zero bid stands past its deadline while the sale stands"
    );
    at(&mut r.tw, late_accepted + DRAW_WINDOW_SECS + 1);
    at(&mut r.tw, late_accepted + DRAW_WINDOW_SECS + 62);
    at(&mut r.tw, late_accepted + DRAW_WINDOW_SECS + 123);
    assert!(matches!(
        state_of(&r.tw, late.contract.expect("contract")),
        Some(ContractState::Expired)
    ));
    assert!(
        market_of(&r.tw).listing(fresh).is_none(),
        "restored past its deadline, the bid lapses and the sweep deletes it"
    );
    assert!(market_of(&r.tw).indexes_agree());
    // Every landed sale settled; nothing paged of any kind.
    let pages: Vec<String> = economy_of(&r.tw)
        .escalations()
        .map(|e| e.context.clone())
        .collect();
    assert!(pages.is_empty(), "no escalation of any kind: {pages:?}");
}

// ---- 10b: a disputed sale (IC10 fixer, review F1) ------------------------------------

/// **A disputed sale keeps its tonnes committed past the cutoff**: the seller is a
/// party (the issuer) and may dispute the sale, which freezes the house's escrow and
/// the bid's intake and stops the leg being offered — so past the cutoff the frozen
/// sale's tonnes still count as in flight, and the same goods cannot be sold a second
/// time beside it. The dispute pages staff (the economy's own SettlementRefused
/// record). A refund resolution restores the bid and frees the tonnes.
#[test]
fn ic10_a_disputed_sale_keeps_its_tonnes_committed_past_the_cutoff() {
    let mut r = rig();
    let bid = r.bid;
    ok(&mut r.tw, r.pat, &format!("+market/sell {bid}=30"));
    let _dropped = take(&mut r.tw);
    let q = last_sale(&r.tw, r.pat);
    let contract = q.contract.expect("contract");
    let accepted_at = q.lot.as_ref().map(|l| l.accepted_at).expect("terms");
    // Doc 18 cut IC14 row 7: the seller may no longer dispute their own sale, so the
    // freeze this test needs is staff's (an admin's dispute is never vetoed).
    ok(&mut r.tw, GOD, &format!("@contract/dispute {contract}"));
    assert!(matches!(
        state_of(&r.tw, contract),
        Some(ContractState::Disputed)
    ));
    assert_eq!(
        held(&r.tw, r.vendor_account),
        30 * BID,
        "the escrow is frozen"
    );
    assert_eq!(bid_qty(&r.tw, bid), Some(INTAKE - 30));
    let paged = economy_of(&r.tw)
        .escalations()
        .filter(|e| matches!(e.kind, EscalationKind::SettlementRefused))
        .filter(|e| matches!(e.subject, EscalationSubject::Contract(c) if c == contract))
        .count();
    assert_eq!(paged, 1, "the dispute pages staff");

    // Past the cutoff: the leg is not offered and the tonnes stay committed.
    r.tw.set_now(accepted_at + DRAW_WINDOW_SECS - DRAW_SETTLE_MARGIN_SECS + 1);
    let offer = lot_offer(&r.tw, r.pat).expect("still offered reserve/release");
    assert!(choices(&offer, "op").is_some_and(|ops| !ops.contains(&"deliver".to_owned())));
    refused!(
        r,
        r.pat,
        format!("+market/sell {bid}=30"),
        "You have 20 of that there to sell, not 30, so nothing was sold.",
    );
    // Long past the contract's own deadline it stays frozen and committed.
    r.tw.set_now(accepted_at + 10 * DRAW_WINDOW_SECS);
    refused!(
        r,
        r.pat,
        format!("+market/sell {bid}=21"),
        "You have 20 of that there to sell, not 21, so nothing was sold.",
    );
    assert_eq!(pool(&mut r.tw, &r.families.clone(), r.pat_hold), HELD);

    // Staff refund it: the bid comes back on the next market sweep and the tonnes are
    // the seller's to sell again.
    ok(
        &mut r.tw,
        GOD,
        &format!("@contract/resolve {contract}=refund"),
    );
    assert!(matches!(
        state_of(&r.tw, contract),
        Some(ContractState::Cancelled)
    ));
    let now = r.tw.world().now();
    at(&mut r.tw, now + 61);
    at(&mut r.tw, now + 122);
    assert_eq!(bid_qty(&r.tw, bid), Some(INTAKE), "the bid is restored");
    assert_eq!(held(&r.tw, r.vendor_account), 0, "the house is refunded");
    ok(&mut r.tw, r.pat, &format!("+market/sell {bid}=30"));
}

/// **Doc 18 cut IC14 row 1 (review F1): past the marker horizon an absent marker is
/// unconfirmed, never "not moved".** A sale disputed for longer than
/// `MARKER_RETENTION_SECS` of real time: by then inventory may have retired its marker
/// from the hold's ring (32 newer moves out of the hold and a day gone), so a refund
/// resolution cannot read absence as "the goods never left". Here the hand-over never
/// landed at all, which is exactly the case it cannot tell from a retired marker: the
/// house is refunded and the bid is **not** restored — under-listing is the safe side
/// (lead ruling L5). (Inside the horizon the same refund restores it:
/// `ic10_a_disputed_sale_keeps_its_tonnes_committed_past_the_cutoff`.)
#[test]
fn ic14_a_refund_past_the_marker_horizon_never_restores_the_bid() {
    // The bid stands three real days, so it is still there when the refund lands.
    let mut r = rig_with(Opts {
        bid_ttl_secs: 3 * 86_400,
        ..Opts::default()
    });
    let bid = r.bid;
    ok(&mut r.tw, r.pat, &format!("+market/sell {bid}=30"));
    let _dropped = take(&mut r.tw);
    let q = last_sale(&r.tw, r.pat);
    let contract = q.contract.expect("contract");
    let accepted_at = q.lot.as_ref().map(|l| l.accepted_at).expect("terms");
    ok(&mut r.tw, GOD, &format!("@contract/dispute {contract}"));
    assert!(matches!(
        state_of(&r.tw, contract),
        Some(ContractState::Disputed)
    ));
    assert_eq!(bid_qty(&r.tw, bid), Some(INTAKE - 30));
    // A day and more on, staff refund it.
    r.tw.set_now(accepted_at + mush_inventory::MARKER_RETENTION_SECS + 60);
    ok(
        &mut r.tw,
        GOD,
        &format!("@contract/resolve {contract}=refund"),
    );
    assert!(matches!(
        state_of(&r.tw, contract),
        Some(ContractState::Cancelled)
    ));
    let now = r.tw.world().now();
    at(&mut r.tw, now + 61);
    at(&mut r.tw, now + 122);
    assert_eq!(held(&r.tw, r.vendor_account), 0, "the house is refunded");
    assert_eq!(
        bid_qty(&r.tw, bid),
        Some(INTAKE - 30),
        "the bid is not restored over goods that may have reached the house"
    );
    // The page: the economy pages a contract once, so the lapse's "could not be
    // confirmed" record folds into the dispute's standing one — one record on this
    // contract.
    let pages = economy_of(&r.tw)
        .escalations()
        .filter(|e| matches!(e.subject, EscalationSubject::Contract(c) if c == contract))
        .count();
    assert_eq!(pages, 1, "one standing record on the contract");
}

// ---- 11: the grammar -------------------------------------------------------------------------

/// **The lot command grammar**: `<n>=<q>`, `<n>=<q> from=<v>`, `<n>=<q> from=#<v>` take
/// the lot arm; a stray word is the usage line; a non-numeric left side and every
/// `#`-prefixed one are the object arm — even when an ask listing shares the dbref's
/// number (reviewer F4); and a bare number naming no viewable listing reads as the
/// object arm's unknown-object answer.
#[test]
fn ic10_the_lot_command_grammar() {
    let mut r = rig();
    let bid = r.bid;
    let hold = r.pat_hold.vault().0;
    for (token, line) in [
        format!("+market/sell {bid}=1"),
        format!("+market/sell {bid}=1 from={hold}"),
        format!("+market/sell {bid}=1 from=#{hold}"),
    ]
    .into_iter()
    .enumerate()
    {
        let said = ok(&mut r.tw, r.pat, &line);
        assert!(said.contains("sold 1:"), "`{line}`: {said}");
        let token = 0xb0 + u8::try_from(token).expect("token");
        land(&mut r.tw, token).expect("the leg lands");
    }
    refused!(
        r,
        r.pat,
        format!("+market/sell {bid}=1 foo"),
        mush_market::SELL_USAGE,
    );
    // A non-numeric left side: the object arm.
    let said = err(
        &mut r.tw,
        r.pat,
        &format!("+market/sell nothing={}", r.vendor.0),
    );
    assert!(said.contains("No such asset."), "{said}");
    // A bare number naming no listing: the object arm's unknown-object answer.
    let said = err(
        &mut r.tw,
        r.pat,
        &format!("+market/sell 999={}", r.vendor.0),
    );
    assert!(said.contains("No such asset."), "{said}");

    // `#<d>=<v>` with an ask listing numbered `d`: the object arm, and the object sale is
    // offered.
    let thing =
        r.tw.world_mut()
            .create(ObjKind::Thing, "Sack", r.pat, Some(r.pat))
            .expect("a carried thing");
    ok(
        &mut r.tw,
        GOD,
        &format!("@asset/title #{}=#{} catalog=ic10:grain", thing.0, r.pat.0),
    );
    // Publish ask listings until one carries the thing's number.
    for _ in 0..64 {
        let top = market_of(&r.tw)
            .live_listings()
            .map(|l| l.id.0)
            .max()
            .unwrap_or(0);
        if top >= u64::from(thing.0) {
            break;
        }
        ok(
            &mut r.tw,
            GOD,
            &format!(
                "@market/vendor stock #{}=ic10:grain price=100 qty=1",
                r.vendor.0
            ),
        );
    }
    let same = market_of(&r.tw)
        .listing(ListingId(u64::from(thing.0)))
        .expect("an ask listing shares the thing's number");
    assert!(!same.source.is_wanted());
    let said = ok(
        &mut r.tw,
        r.pat,
        &format!("+market/sell #{}={}", thing.0, r.vendor.0),
    );
    assert!(said.contains("Offered. Contract #"), "{said}");
    assert!(!said.contains(mush_market::SELLS_NOT_BUYS), "{said}");
    no_settlement_pages(&r.tw);
}

// ---- 13: the re-stage --------------------------------------------------------------------------

/// **A dropped leg is asked for again and settles once** (reviewer F5): the seller's
/// `+market/accept <quote>` re-stages the byte-identical leg; it lands and settles once;
/// a re-stage after the landing replays nothing; the house's officer and a stranger get
/// the no-such-quote answer.
#[test]
fn ic10_a_dropped_leg_is_asked_for_again_and_settles_once() {
    let mut r = rig();
    ok(&mut r.tw, r.pat, &format!("+market/sell {}=20", r.bid));
    let dropped = take(&mut r.tw);
    let q = last_sale(&r.tw, r.pat);
    let contract = q.contract.expect("contract");
    let again = ok(&mut r.tw, r.pat, &format!("+market/accept {}", q.id));
    let line = format!(
        "Asking again for the hand-over on contract #{contract}. It settles once the goods have \
         moved."
    );
    assert!(again.lines().any(|l| l == line), "{again}");
    let restaged = take(&mut r.tw);
    assert!(r.tw.engine_mut().take_authoritative_submit().is_none());
    assert_eq!(restaged.key, dropped.key);
    assert_eq!(restaged.parameters, dropped.parameters);
    assert_eq!(
        restaged, dropped,
        "the whole submission is the same request"
    );
    land_submit(&mut r.tw, &restaged, 0xd1).expect("it lands");

    // Asked again after the landing, before the settlement: doc 18 cut IC14 row 3 —
    // the re-stage reads the marker first and says the goods are in; nothing is
    // staged and nothing moves twice.
    let said = err(&mut r.tw, r.pat, &format!("+market/accept {}", q.id));
    assert!(said.contains(mush_market::DELIVER_LANDED), "{said}");
    assert!(
        r.tw.engine_mut().take_authoritative_submit().is_none(),
        "nothing re-staged after the landing"
    );
    assert_eq!(pool(&mut r.tw, &r.families, r.pat_hold), HELD - 20);
    assert_eq!(pool(&mut r.tw, &r.families, r.shop), 20);
    settle(&mut r.tw, contract);
    assert_eq!(r.policy.lock().expect("policy").settled.len(), 1);

    // Not theirs: the existing no-such-quote answer.
    let officer = r.tw.spawn_player("Officer");
    for capability in ["economy.account.post", "vault.withdraw"] {
        ok(
            &mut r.tw,
            GOD,
            &format!(
                "@org/grant #{}=Ares {capability} scope=#{}",
                officer.0, r.root_scope
            ),
        );
    }
    let (sam, _, _) = seller(&mut r.tw, "Sam", r.faucet);
    for who in [officer, sam] {
        let said = err(&mut r.tw, who, &format!("+market/accept {}", q.id));
        assert!(said.contains(mush_market::NO_SUCH_QUOTE), "{said}");
    }
    let _ = r.quinn_account;
    no_settlement_pages(&r.tw);
}

// ---- doc 18 cut IC14: the money-safety residuals --------------------------------------

/// Queue `n` authoritative submissions for `n` stand-in actors that are none of the
/// rig's (the engine's queue keys only by actor; these are never driven).
fn fill_engine_queue(tw: &mut TestWorld, n: u32) {
    for i in 0..n {
        let submit = forged(tw, ObjId(900_000 + i), json!({}));
        tw.engine_mut()
            .queue_authoritative_submit(Box::new(submit))
            .expect("the queue has room");
    }
}

/// **Doc 18 cut IC14 row 2: a full engine queue refuses the sale before the escrow.**
/// With 64 other actors' submissions queued the deliver leg could not be staged, so
/// `+market/sell` answers `SELL_QUEUE_FULL` (not `SELL_BUSY`: the seller has nothing
/// pending) and writes nothing: no contract, no quote, the house's balance and the
/// bid's intake untouched, and nothing of the seller's queued.
#[test]
fn ic14_a_full_queue_refuses_the_sale_before_the_escrow() {
    let mut r = rig();
    fill_engine_queue(&mut r.tw, 64);
    let before = books(&r);
    let said = err(&mut r.tw, r.pat, &format!("+market/sell {}=20", r.bid));
    assert!(said.contains(mush_market::SELL_QUEUE_FULL), "{said}");
    assert!(!said.contains(mush_market::SELL_BUSY), "{said}");
    assert_eq!(books(&r), before, "the sale wrote nothing");
    assert!(!r.tw.engine().has_authoritative_submit_for(r.pat));
    assert_eq!(r.tw.engine().queued_authoritative_actors().count(), 64);
    no_settlement_pages(&r.tw);
}

/// **Doc 18 cut IC14 row 2 (reviewer F2): two sales in one `fire_due` on a nearly
/// full queue escrow once.** 63 queued; two `@wait` lines fire in the same slice, one
/// sale each. The first escrows and its leg is admitted at the drain (the 64th place);
/// the second counts the first's deferred leg, is refused `SELL_QUEUE_FULL` before
/// any escrow, and no seller ever reads the engine's "Too many actions" line.
#[test]
fn ic14_two_same_fire_sells_on_a_nearly_full_queue_escrow_once() {
    let mut r = rig();
    fill_engine_queue(&mut r.tw, 63);
    let before = books(&r);
    // The engine's `@wait` is not composed in this rig, so the probe schedules the two
    // lines exactly as `@wait 5=…` would (one `Deferred::ScheduleIn` of a `WaitCmd`
    // each, the same due second).
    let line = format!("+market/sell {}=20", r.bid);
    let (pat, quinn) = (r.pat, r.quinn);
    with_probe(&mut r.tw, GOD, move |ctx| {
        for actor in [pat, quinn] {
            ctx.out.defer(mush_core::Deferred::ScheduleIn {
                delay_ms: 5_000,
                kind: mush_core::ScheduledKind::WaitCmd {
                    actor,
                    line: line.clone(),
                },
            });
        }
    });
    let t = r.tw.world().now() + 5;
    r.tw.set_now(t);
    let events = r.tw.fire_due(t * 1000);
    let said = texts(&events);
    assert!(
        !said.contains("Too many actions are committing"),
        "no seller reads the engine's back-pressure line: {said}"
    );
    assert_eq!(
        said.matches(mush_market::SELL_QUEUE_FULL).count(),
        1,
        "exactly one sale refused: {said}"
    );
    let after = books(&r);
    assert_eq!(
        after.contracts,
        before.contracts + 1,
        "one contract: {said}"
    );
    assert_eq!(after.quotes, before.quotes + 1);
    assert_eq!(after.house_held, before.house_held + BID * 20, "one escrow");
    assert_eq!(after.bid, before.bid.map(|q| q - 20));
    let queued: Vec<ObjId> = [r.pat, r.quinn]
        .into_iter()
        .filter(|who| r.tw.engine().has_authoritative_submit_for(*who))
        .collect();
    assert_eq!(
        queued.len(),
        1,
        "the seller who escrowed has their leg queued"
    );
    no_settlement_pages(&r.tw);
}

/// An actual observed selector and a real sale share one timer drain. The
/// observed handler does not move money; the later sale must count its deferred
/// actor before the house escrows. No runtime completion or certificate is mocked.
#[test]
fn m7_ic14_observed_work_counts_before_same_fire_escrow_and_preserves_custody() {
    observed_then_sale(false);
}

/// An observed attempt for an already queued actor takes no additional place.
/// The earlier selector survives backpressure; only this duplicate attempt is
/// NotDispatched, and the other seller can use the actual remaining queue slot.
#[test]
fn m7_ic14_observed_duplicate_actor_counts_once_before_same_fire_escrow() {
    observed_then_sale(true);
}

fn observed_then_sale(duplicate_actor: bool) {
    use mush_core::submission_observation::{SubmissionDisposition, SubmissionObservation};

    let mut r = rig();
    fill_engine_queue(&mut r.tw, if duplicate_actor { 62 } else { 63 });
    if duplicate_actor {
        let incumbent = forged(&r.tw, r.pat, json!({"incumbent": true}));
        r.tw.engine_mut()
            .queue_authoritative_submit(Box::new(incumbent))
            .expect("the actor's incumbent takes the 63rd place");
    }
    assert_eq!(r.tw.engine().queued_authoritative_actors().count(), 63);
    let before = books(&r);
    let captured: Arc<Mutex<Option<SubmissionObservation>>> = Arc::new(Mutex::new(None));
    let observed = forged(&r.tw, r.pat, json!({"attempt": "observed"}));
    let (pat, quinn) = (r.pat, r.quinn);
    let sale = format!("+market/sell {}=20", r.bid);
    with_probe(&mut r.tw, GOD, move |ctx| {
        for (actor, line) in [(pat, "+ic10probe".to_owned()), (quinn, sale)] {
            ctx.out.defer(mush_core::Deferred::ScheduleIn {
                delay_ms: 5_000,
                kind: mush_core::ScheduledKind::WaitCmd { actor, line },
            });
        }
    });
    // The next real probe dispatch is the first timer above, as Pat. Leave the
    // immutable actor capture to Ctx; do not construct a deferred observation.
    let sink = Arc::clone(&captured);
    let jobs =
        r.tw.world()
            .module_state::<ProbeQueue>()
            .expect("probe")
            .clone();
    jobs.0
        .lock()
        .expect("probe queue")
        .push(Box::new(move |ctx| {
            let observation = ctx
                .submit_authoritative_observed(
                    observed.action,
                    observed.subject,
                    observed.targets,
                    observed.parameters,
                    observed.key,
                )
                .expect("actual observed dispatch");
            *sink.lock().expect("observation slot") = Some(observation);
        }));
    let t = r.tw.world().now() + 5;
    r.tw.set_now(t);
    let said = texts(&r.tw.fire_due(t * 1_000));
    let mut observation = captured
        .lock()
        .expect("observation slot")
        .take()
        .expect("the actual timer captured one observation");
    assert!(!said.contains("Too many actions are committing"), "{said}");
    assert_eq!(r.tw.engine().queued_authoritative_actors().count(), 64);
    assert!(r.tw.engine().has_authoritative_submit_for(pat));
    if duplicate_actor {
        assert!(!said.contains(mush_market::SELL_QUEUE_FULL), "{said}");
        let after = books(&r);
        assert_eq!(after.contracts, before.contracts + 1);
        assert_eq!(after.quotes, before.quotes + 1);
        assert_eq!(after.house_held, before.house_held + BID * 20);
        assert_eq!(after.bid, before.bid.map(|quantity| quantity - 20));
        assert!(r.tw.engine().has_authoritative_submit_for(quinn));
        assert_eq!(
            observation.take_terminal(),
            Some(SubmissionDisposition::NotDispatched)
        );
        assert_eq!(observation.take_terminal(), None, "consumed once");
    } else {
        assert!(said.contains(mush_market::SELL_QUEUE_FULL), "{said}");
        assert_eq!(
            books(&r),
            before,
            "no contract, quote, intake or escrow change"
        );
        assert!(!r.tw.engine().has_authoritative_submit_for(quinn));
        assert_eq!(
            observation.take_terminal(),
            None,
            "queued custody is unresolved"
        );
    }
    let mut selected = None;
    while let Some(driven) = r.tw.engine_mut().take_observed_authoritative_submit() {
        if driven.request().actor == pat {
            assert!(selected.is_none(), "one selector for Pat");
            selected = Some(driven);
        }
    }
    let selected = selected.expect("the queued selector survived capacity inspection");
    assert_eq!(selected.is_observed(), !duplicate_actor);
    assert_eq!(
        selected.request().parameters,
        if duplicate_actor {
            json!({"incumbent": true})
        } else {
            json!({"attempt": "observed"})
        }
    );
    assert_eq!(
        observation.take_terminal(),
        None,
        "driving is not absence proof"
    );
    no_settlement_pages(&r.tw);
}

/// **Doc 18 cut IC14 row 2: the sale names its remedy** — a second line after the
/// policy's sale line, naming `+market/accept <quote>` with the sale's quote id.
#[test]
fn ic14_the_sale_names_its_remedy() {
    let mut r = rig();
    let said = ok(&mut r.tw, r.pat, &format!("+market/sell {}=20", r.bid));
    let q = last_sale(&r.tw, r.pat);
    let remedy = mush_market::SELL_REMEDY.replace("{quote}", &q.id.0.to_string());
    assert_eq!(
        remedy,
        format!(
            "If the hand-over stalls, +market/accept {} asks again.",
            q.id.0
        )
    );
    let lines: Vec<&str> = said.lines().collect();
    let sold = lines
        .iter()
        .position(|l| l.starts_with("sold 20:"))
        .expect("the policy's sale line");
    assert_eq!(lines.get(sold + 1), Some(&remedy.as_str()), "{said}");
}

/// **Doc 18 cut IC14 row 3: a re-stage after the landing says the goods are in.** The
/// leg lands; past `draw_by` and before the mirror sweeps, `+market/accept <quote>`
/// answers `DELIVER_LANDED`, never `DELIVER_TOO_LATE` ("…Nothing moved."), stages
/// nothing, and the contract then settles inside its settle margin.
#[test]
fn ic14_a_restage_after_the_landing_says_the_goods_are_in() {
    let mut r = rig();
    ok(&mut r.tw, r.pat, &format!("+market/sell {}=20", r.bid));
    land(&mut r.tw, 0xe1).expect("the deliver lands");
    let q = last_sale(&r.tw, r.pat);
    let contract = q.contract.expect("contract");
    let draw_by = q.lot.as_ref().and_then(|l| l.draw_by).expect("a cutoff");
    r.tw.set_now(draw_by + 1);
    let said = err(&mut r.tw, r.pat, &format!("+market/accept {}", q.id));
    assert!(said.contains(mush_market::DELIVER_LANDED), "{said}");
    assert!(!said.contains(DELIVER_TOO_LATE), "{said}");
    assert!(r.tw.engine_mut().take_authoritative_submit().is_none());
    settle(&mut r.tw, contract);
    assert_eq!(r.policy.lock().expect("policy").settled.len(), 1);
    no_settlement_pages(&r.tw);
}

/// **Doc 18 cut IC14 row 3 (reviewer F1): the D1 guard holds inside the horizon.**
/// The leg lands, then 40 staff `@inv/move`s leave the same hold (40 more markers in
/// its ring, past the floor of 32, all younger than a real day), then
/// `+market/accept <quote>`: still `DELIVER_LANDED`, nothing staged, and the hold
/// moved exactly the sale's tonnes once plus the staff moves.
#[test]
fn ic14_a_restage_after_40_later_moves_from_the_hold_is_still_a_hit() {
    let mut r = rig();
    ok(&mut r.tw, r.pat, &format!("+market/sell {}=5", r.bid));
    land(&mut r.tw, 0xe2).expect("the deliver lands");
    let q = last_sale(&r.tw, r.pat);
    for n in 0..40_u8 {
        let t = r.tw.world().now() + 30;
        r.tw.set_now(t);
        ok(
            &mut r.tw,
            GOD,
            &format!(
                "@inv/move {}/iron:1={}",
                r.pat_hold.vault().0,
                r.quinn_hold.vault().0
            ),
        );
        land(&mut r.tw, 0x80 + n).expect("a staff move lands");
    }
    let ring = read(&mut r.tw, &r.families, |txn| {
        txn.get(&r.families.marker_ring, &r.pat_hold)
            .expect("the ring reads")
            .map_or(0, mush_inventory::MarkerRing::len)
    });
    assert_eq!(ring, 41, "the sale's marker and the 40 moves, none evicted");
    let said = err(&mut r.tw, r.pat, &format!("+market/accept {}", q.id));
    assert!(said.contains(mush_market::DELIVER_LANDED), "{said}");
    assert!(r.tw.engine_mut().take_authoritative_submit().is_none());
    assert_eq!(pool(&mut r.tw, &r.families, r.pat_hold), HELD - 5 - 40);
    assert_eq!(pool(&mut r.tw, &r.families, r.shop), 5, "moved once");
}

/// **Doc 18 cut IC14 row 7 (lead ruling L6): a seller cannot dispute their own sale.**
/// `@contract/dispute` by the seller answers `SALE_DISPUTE_REFUSED`; the contract
/// stays Active, the house's escrow is not frozen, no dispute page is opened, and the
/// sale settles once the marker lands.
#[test]
fn ic14_a_seller_cannot_dispute_their_own_sale() {
    let mut r = rig();
    ok(&mut r.tw, r.pat, &format!("+market/sell {}=20", r.bid));
    let q = last_sale(&r.tw, r.pat);
    let contract = q.contract.expect("contract");
    let said = err(&mut r.tw, r.pat, &format!("@contract/dispute {contract}"));
    assert!(said.contains(mush_market::SALE_DISPUTE_REFUSED), "{said}");
    assert!(matches!(
        state_of(&r.tw, contract),
        Some(ContractState::Active)
    ));
    assert_eq!(
        held(&r.tw, r.vendor_account),
        20 * BID,
        "escrowed, not frozen"
    );
    no_settlement_pages(&r.tw);
    land(&mut r.tw, 0xe5).expect("the deliver lands");
    settle(&mut r.tw, contract);
    assert_eq!(r.policy.lock().expect("policy").settled.len(), 1);
    no_settlement_pages(&r.tw);
}

/// **Doc 18 cut IC14 row 7: staff and other contracts dispute as before.** An admin's
/// dispute of a sale still freezes it (and pages); the veto answers nothing for the
/// sale's other party (the house) or for a contract that is not a sale into a bid.
#[test]
fn ic14_staff_and_other_contracts_dispute_as_before() {
    use mush_economy::DisputeGuard;
    let mut r = rig();
    ok(&mut r.tw, r.pat, &format!("+market/sell {}=20", r.bid));
    let q = last_sale(&r.tw, r.pat);
    let contract = q.contract.expect("contract");
    let record = economy_of(&r.tw)
        .snapshot()
        .contract(contract)
        .cloned()
        .expect("the contract");
    let guard = mush_market::SaleDisputeGuard;
    assert_eq!(
        guard.refusal(r.tw.world(), &record, &q.seller),
        Some(mush_market::SALE_DISPUTE_REFUSED)
    );
    assert_eq!(guard.refusal(r.tw.world(), &record, &q.buyer), None);
    let mut other = record.clone();
    other.id = ContractId(contract.0 + 1_000_000);
    assert_eq!(
        guard.refusal(r.tw.world(), &other, &q.seller),
        None,
        "not a sale into a bid"
    );
    ok(&mut r.tw, GOD, &format!("@contract/dispute {contract}"));
    assert!(matches!(
        state_of(&r.tw, contract),
        Some(ContractState::Disputed)
    ));
    assert_eq!(
        held(&r.tw, r.vendor_account),
        20 * BID,
        "the escrow is frozen"
    );
}
