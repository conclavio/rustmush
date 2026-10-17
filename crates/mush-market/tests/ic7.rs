//! ---- Doc 18 cut IC7: market seams for content ports, as gates ----
//!
//! Doc 18 §4.3/§4.4 name six market/economy seams a content port needs before any port
//! exists to use them: a content-resolved party-cap source, asks that **draw** from a
//! storefront pool, a presence gate, a capacity hook, a Draw that refuses at its prepare
//! without taking the buyer's money, and paid lot legs keyed by their contract. This file
//! is the gate for all six, on **the production composition in miniature** (IC0's rig
//! shape): `mush-org`, `mush-economy` with an EMPTY frozen party-cap table and a test
//! party-cap *source*, the market's fact provider and observer, `mush-inventory`
//! delegating quantity writes to `"market"`, and `mush-market` with one lot-represented
//! catalogue entry, a test presence gate and (where a test says so) a test capacity
//! provider. The tacsim implementations of the three seams are cut IC8's; nothing here
//! names a game.
//!
//! A tiny test module, `ic7probe`, runs a queued closure in a command context of its
//! own, so a test can call `mush_market::accept_at(.., Presence::Ship(hull))` the way
//! IC11's executor will, or `list_goods` with a spec no verb builds. Whether it is a
//! quantity delegate is the difference between tests 7 and 8.
//!
//! Every test ends asserting the economy's ladder holds no `SettlementRefused` page.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mush_core::module::{GameModule, ModError, Registrar};
use mush_core::{
    ArgShape, Audience, AuthoritativeSubmit, CmdError, CommandSpec, CommitToken, Ctx, GOD,
    GameClock, IdempotencyKey, IdempotencyKeyText, IdempotencyScope, LIMBO, ObjId, ObjKind,
    OfferExecutionContext, OfferSubmission, PerceivedRef, ProjectedLegalAction,
    ProjectedParameterKind, RequestUid, ScopedIdempotencyKey,
};
use mush_economy::{
    AccountId, ContractId, ContractState, EconomyStore, EscalationKind, EscalationSubject,
    ExternalRef, PartyCap, PartyCapSource, VaultId,
};
use mush_inventory::{
    EndpointRef, Grade, InventoryError, InventoryFamilies, InventoryHandleCell, InventoryHandles,
    InventoryModule, LotId,
};
use mush_market::{
    CAPACITY_UNREADABLE, CapacityPlan, CapacityProvider, CapacityReadError, CapacityRefusal,
    CatalogEntry, CatalogSource, DRAW_SETTLE_MARGIN_SECS, DRAW_TOO_LATE, DRAW_WINDOW_SECS,
    ListingId, MarketFactProvider, MarketId, MarketStore, PORT_ONLY_ENDPOINT, PaidKind, Presence,
    QuoteId, STALE_QUOTE, VendorId, VenueGate, VenueRefusal,
};
use mush_org::{OrgId, PrincipalId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use serde_json::json;

// ---- the test seams ---------------------------------------------------------

const PACK: &str = "ic7.test";
const GOOD: &str = "ic7:iron";
const CREDIT: &str = "credit";
const T0: i64 = 1_700_000_000;
/// What each buyer is funded with, in minor units.
const FUNDS: i64 = 200_000;
/// The port's own institution: the first org a fresh world allocates.
const VENDOR_ORG: OrgId = OrgId(0);

#[derive(Debug)]
struct TestCatalog;

impl CatalogSource for TestCatalog {
    fn pack(&self, pack: &str) -> Option<Result<Vec<CatalogEntry>, String>> {
        if pack != PACK {
            return None;
        }
        let value = json!({
            "catalog": { "ruleset": "ic7", "code": "iron" },
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

/// The party-cap source: answers from a switch the test sets, and counts its calls.
#[derive(Debug, Default)]
struct TestCapSource {
    on: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
}

impl PartyCapSource for TestCapSource {
    fn has_cap(&self, _w: &mush_core::World, principal: &PrincipalId, cap: PartyCap) -> bool {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.on.load(Ordering::SeqCst)
            && cap == PartyCap::Accept
            && matches!(principal, PrincipalId::Organization(_))
    }
}

/// What the test gate knows, set after the world exists.
#[derive(Debug, Default)]
struct GateCfg {
    /// The one counter room of venue `luna`.
    room: Option<ObjId>,
    /// The buyer's hold: the gate's endpoint, and governed.
    hold: Option<EndpointRef>,
    /// The one hull a `Presence::Ship` is admitted for.
    hull: Option<ObjId>,
    /// The org the gate calls `luna`'s port seller.
    port_seller: Option<OrgId>,
    /// The last presence `admit` saw.
    last: Option<Presence>,
    /// What the capacity provider's last `check` answered (reviewer T3: a refusal for
    /// volume and an unreadable pool are one line to the actor, so a test that cares which
    /// one it was reads it here).
    last_check: Option<Result<(), CapacityRefusal>>,
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
        _actor: ObjId,
        venue: &MarketId,
        _presence: Presence,
    ) -> Option<EndpointRef> {
        if *venue == luna() {
            self.0.lock().expect("cfg").hold
        } else {
            None
        }
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
        self.0.lock().expect("cfg").hold == Some(endpoint)
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

/// The capacity provider: plans for the buyer's hold only, `{"limit_units": N, "litres":
/// {"iron": 1}}`, and refuses past the limit with a line naming the packed volume.
#[derive(Debug)]
struct TestCapacity {
    cfg: Arc<Mutex<GateCfg>>,
    limit: u64,
}

fn packed_line(used: u64, limit: u64, add: u64) -> String {
    format!(
        "The hold is packed: {used} of {limit} units of volume are taken and that needs {add} more."
    )
}

impl CapacityProvider for TestCapacity {
    fn plan(&self, _w: &mush_core::World, endpoint: EndpointRef) -> Option<CapacityPlan> {
        (self.cfg.lock().expect("cfg").hold == Some(endpoint))
            .then(|| CapacityPlan(json!({ "limit_units": self.limit, "litres": { "iron": 1 } })))
    }
    fn check(
        &self,
        plan: &CapacityPlan,
        commodity: &mush_core::CanonicalId,
        quantity: u64,
        read_pool: &mut dyn FnMut(&mush_core::CanonicalId) -> Result<u64, CapacityReadError>,
    ) -> Result<(), CapacityRefusal> {
        let answer = self.answer(plan, commodity, quantity, read_pool);
        self.cfg.lock().expect("cfg").last_check = Some(answer.clone());
        answer
    }
}

impl TestCapacity {
    fn answer(
        &self,
        plan: &CapacityPlan,
        commodity: &mush_core::CanonicalId,
        quantity: u64,
        read_pool: &mut dyn FnMut(&mush_core::CanonicalId) -> Result<u64, CapacityReadError>,
    ) -> Result<(), CapacityRefusal> {
        let limit = plan.0["limit_units"].as_u64().unwrap_or(0);
        let mut used = 0u64;
        let mut per_unit = 1u64;
        if let Some(litres) = plan.0["litres"].as_object() {
            for (code, l) in litres {
                let l = l.as_u64().unwrap_or(1);
                let id = mush_core::CanonicalId::new(code).expect("code");
                let held = read_pool(&id).map_err(|_| CapacityRefusal::Unreadable)?;
                used += held * l;
                if id == *commodity {
                    per_unit = l;
                }
            }
        }
        let add = quantity * per_unit;
        if used + add > limit {
            return Err(CapacityRefusal::Refused {
                fits: limit.saturating_sub(used) / per_unit.max(1),
                line: packed_line(used, limit, add),
            });
        }
        Ok(())
    }
}

/// `ic7probe`: one command that runs a queued closure in its own command context.
type ProbeJob = Box<dyn FnOnce(&mut Ctx<'_>) + Send>;

#[derive(Clone, Default)]
struct ProbeQueue(Arc<Mutex<Vec<ProbeJob>>>);

struct Probe;

impl GameModule for Probe {
    fn name(&self) -> &'static str {
        "ic7probe"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(ProbeQueue::default());
        reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
        reg.command(
            CommandSpec {
                name: "+ic7probe".into(),
                shape: ArgShape::Raw,
                help: "+ic7probe -- test: run the queued closure in a command context.".into(),
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
    tw.run(actor, "+ic7probe");
    slot.lock().expect("lock").take().expect("the probe ran")
}

// ---- the rig ---------------------------------------------------------------

#[derive(Clone, Copy)]
struct Opts {
    /// Install `MarketFactProvider` (test 14 leaves it out).
    facts: bool,
    /// Name the vendor in the frozen party-cap table too (test 11's third rig).
    table_vendor: bool,
    /// The capacity provider's unit limit, when one is installed.
    capacity: Option<u64>,
    /// Whether `ic7probe` is a quantity delegate.
    probe_delegate: bool,
    /// The storefront pool, and the Draw listing's quantity (`None`: no listing).
    pool: u64,
    draw: Option<u64>,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts {
            facts: true,
            table_vendor: false,
            capacity: None,
            probe_delegate: false,
            pool: 100,
            draw: Some(100),
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
    depot: EndpointRef,
    vendor: VendorId,
    vendor_account: AccountId,
    root_scope: u64,
    faucet: AccountId,
    listing: Option<ListingId>,
    cfg: Arc<Mutex<GateCfg>>,
    source_on: Arc<AtomicBool>,
    source_calls: Arc<AtomicUsize>,
    corridor: ObjId,
    hull: ObjId,
}

fn rig_with(opts: Opts) -> Rig {
    let cfg: Arc<Mutex<GateCfg>> = Arc::default();
    let source_on = Arc::new(AtomicBool::new(true));
    let source_calls = Arc::new(AtomicUsize::new(0));
    let cell: InventoryHandleCell = Arc::default();
    let mut economy = mush_economy::EconomyModule::new()
        .with_party_caps(if opts.table_vendor {
            vec![(PrincipalId::Organization(VENDOR_ORG), PartyCap::Accept)]
        } else {
            Vec::new()
        })
        .with_party_cap_source(Arc::new(TestCapSource {
            on: Arc::clone(&source_on),
            calls: Arc::clone(&source_calls),
        }))
        .with_observers(vec![Arc::new(mush_market::MarketObserver)]);
    if opts.facts {
        economy = economy.with_facts(vec![Arc::new(MarketFactProvider)]);
    }
    let delegates: Vec<&str> = if opts.probe_delegate {
        vec!["market", "ic7probe"]
    } else {
        vec!["market"]
    };
    let mut market = mush_market::MarketModule::new()
        .with_inventory(Arc::clone(&cell))
        .with_catalog_sources(vec![Arc::new(TestCatalog)])
        .with_venue_gate(Arc::new(TestGate(Arc::clone(&cfg))));
    if let Some(limit) = opts.capacity {
        market = market.with_capacity_provider(Arc::new(TestCapacity {
            cfg: Arc::clone(&cfg),
            limit,
        }));
    }
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(economy),
        Box::new(
            InventoryModule::new()
                .with_quantity_delegates(delegates)
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
    ok(&mut tw, GOD, &format!("@account/system faucet={CREDIT}"));
    let faucet = economy_of(&tw)
        .snapshot()
        .accounts()
        .find(|a| matches!(a.owner, PrincipalId::System(_)))
        .expect("the faucet")
        .id;

    // The port's institution, its storefront and a depot, both on its root scope so an
    // officer granted `vault.withdraw` there may take from them.
    ok(&mut tw, GOD, "@org/create Ares");
    let org = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().orgs().map(|o| o.id).max())
        .expect("the org exists");
    assert_eq!(org, VENDOR_ORG);
    let root_scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().org(org).map(|o| o.root_scope))
        .expect("the root scope")
        .0;
    let shop_principal = PrincipalId::Organization(org);
    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:#{}={CREDIT}", org.0),
    );
    let vendor_account = account_of(&tw, &shop_principal);
    let vault_for = |tw: &mut TestWorld, name: &str| -> EndpointRef {
        let backing = tw
            .world_mut()
            .create(ObjKind::Thing, name, GOD, None)
            .expect("a container");
        let before: Vec<VaultId> = economy_of(tw).snapshot().vaults().map(|v| v.id).collect();
        ok(
            tw,
            GOD,
            &format!(
                "@vault/create {name}=container:#{} owner=org:#{} scope=#{root_scope}",
                backing.0, org.0
            ),
        );
        EndpointRef(
            economy_of(tw)
                .snapshot()
                .vaults()
                .map(|v| v.id)
                .find(|id| !before.contains(id))
                .expect("the vault exists"),
        )
    };
    let shop = vault_for(&mut tw, "AresStore");
    let depot = vault_for(&mut tw, "AresDepot");
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

    // The buyers, standing at Luna's counter (LIMBO), each with a hold of her own.
    let (pat, pat_account, pat_hold) = buyer(&mut tw, "Pat", faucet);
    let (quinn, quinn_account, quinn_hold) = buyer(&mut tw, "Quinn", faucet);
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
        c.hold = Some(pat_hold);
        c.hull = Some(hull);
        c.port_seller = Some(org);
    }

    // The stock, rooted.
    seed_rooted_pool(&mut tw, &families, shop, opts.pool, LotId(0xb1));
    seed_rooted_pool(&mut tw, &families, depot, 50, LotId(0xb2));
    let listing = opts.draw.map(|qty| {
        let said = ok(
            &mut tw,
            GOD,
            &format!(
                "@market/vendor stock #{}={GOOD} price=700 qty={qty}",
                vendor.0
            ),
        );
        let id = market_of(&tw)
            .live_listings()
            .map(|l| l.id)
            .max()
            .expect("the listing");
        assert!(
            said.contains(&format!("Listing #{id} published.")),
            "{said}"
        );
        id
    });
    assert!(
        tw.engine_mut().take_authoritative_submit().is_none(),
        "a drawn listing reserves nothing, so it stages nothing"
    );
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
        depot,
        vendor,
        vendor_account,
        root_scope,
        faucet,
        listing,
        cfg,
        source_on,
        source_calls,
        corridor,
        hull,
    }
}

fn rig() -> Rig {
    rig_with(Opts::default())
}

fn buyer(tw: &mut TestWorld, name: &str, faucet: AccountId) -> (ObjId, AccountId, EndpointRef) {
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

fn account_of(tw: &TestWorld, owner: &PrincipalId) -> AccountId {
    economy_of(tw)
        .snapshot()
        .accounts()
        .find(|a| &a.owner == owner)
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

fn refused_settlements(tw: &TestWorld) -> Vec<String> {
    economy_of(tw)
        .escalations()
        .filter(|e| matches!(e.kind, EscalationKind::SettlementRefused))
        .map(|e| e.context.clone())
        .collect()
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

/// One more vault of the port's institution on its root scope (the rig's own two are
/// the storefront and the depot).
fn org_vault(tw: &mut TestWorld, root_scope: u64, name: &str) -> EndpointRef {
    let backing = tw
        .world_mut()
        .create(ObjKind::Thing, name, GOD, None)
        .expect("a container");
    let before: Vec<VaultId> = economy_of(tw).snapshot().vaults().map(|v| v.id).collect();
    ok(
        tw,
        GOD,
        &format!(
            "@vault/create {name}=container:#{} owner=org:#{} scope=#{root_scope}",
            backing.0, VENDOR_ORG.0
        ),
    );
    EndpointRef(
        economy_of(tw)
            .snapshot()
            .vaults()
            .map(|v| v.id)
            .find(|id| !before.contains(id))
            .expect("the vault exists"),
    )
}

/// The iron pool's (balance, availability) at an endpoint.
fn pool(tw: &mut TestWorld, families: &InventoryFamilies, at: EndpointRef) -> (u64, u64) {
    read(tw, families, |txn| {
        let key = mush_inventory::pool_key(at, &commodity("iron")).expect("pool key");
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

fn live_lots(tw: &mut TestWorld, families: &InventoryFamilies, at: EndpointRef) -> u64 {
    read(tw, families, |txn| {
        txn.get(&families.lot_census, &at)
            .ok()
            .flatten()
            .map_or(0, |census| u64::from(census.get()))
    })
}

fn listing_qty(tw: &TestWorld, id: ListingId) -> Option<u64> {
    market_of(tw).listing(id).map(|l| l.quantity)
}

/// `+market/quote <listing>=<qty>` as `who`; the new quote's id.
fn quote(tw: &mut TestWorld, who: ObjId, listing: ListingId, qty: u64) -> QuoteId {
    let said = ok(tw, who, &format!("+market/quote {listing}={qty}"));
    let id = said
        .split("Quote #")
        .nth(1)
        .and_then(|rest| rest.split(':').next())
        .and_then(|n| n.trim().parse::<u64>().ok())
        .unwrap_or_else(|| panic!("no quote id in {said}"));
    QuoteId(id)
}

fn contract_of(tw: &TestWorld, q: QuoteId) -> ContractId {
    market_of(tw)
        .quote(q)
        .and_then(|q| q.contract)
        .expect("the accept drafted a contract")
}

fn state_of(tw: &TestWorld, c: ContractId) -> Option<ContractState> {
    economy_of(tw).snapshot().contract(c).map(|c| c.state)
}

fn receipt(tw: &TestWorld, c: ContractId) -> Option<mush_market::DrawReceipt> {
    let handle = mush_market::draw_receipts(tw.world()).expect("the receipt family");
    tw.engine()
        .action_state()
        .get(&handle, &c)
        .ok()
        .flatten()
        .cloned()
}

/// Take the one queued authoritative submission.
fn take(tw: &mut TestWorld) -> AuthoritativeSubmit {
    *tw.engine_mut()
        .take_authoritative_submit()
        .expect("a submission was staged")
}

/// **Drive one submission the way the world task does** (IC0's `land`, over a given
/// submission): regenerate the actor's own offer, resolve the parameters against it,
/// prepare and install.
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

/// Fire the market (60 s) and economy (30 s) sweeps until the contract settles.
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

fn no_settlement_pages(tw: &TestWorld) {
    let pages = refused_settlements(tw);
    assert!(
        pages.is_empty(),
        "the ladder holds a SettlementRefused page: {pages:?}"
    );
}

// ---- 1: the Draw, end to end -------------------------------------------------

/// **A Draw-backed ask moves the pool and the money exactly once**, and the vendor
/// accepts through the party-cap *source* (the frozen table is empty).
#[test]
fn ic7_a_draw_backed_ask_moves_the_pool_and_the_money_once() {
    let mut r = rig();
    let listing = r.listing.expect("listed");
    let q = quote(&mut r.tw, r.pat, listing, 10);
    let agreed = ok(
        &mut r.tw,
        r.pat,
        &format!("+market/accept {}={}", q.0, r.pat_hold.vault().0),
    );
    assert!(agreed.contains("Contract #"), "{agreed}");
    let contract = contract_of(&r.tw, q);
    // The counter decrements in the accept command, before the leg lands.
    assert_eq!(listing_qty(&r.tw, listing), Some(90));
    assert!(
        r.source_calls.load(Ordering::SeqCst) > 0,
        "the vendor accepted through the content source"
    );
    let staged = take(&mut r.tw);
    assert_eq!(
        staged.key.as_str(),
        format!("market.lot.draw.{:x}", contract.0),
        "a paid leg is keyed by its contract"
    );
    land_submit(&mut r.tw, &staged, 0x21).expect("the draw lands");
    assert!(
        receipt(&r.tw, contract).is_some(),
        "the draw wrote its receipt"
    );

    settle(&mut r.tw, contract);
    assert_eq!(pool(&mut r.tw, &r.families, r.shop), (90, 90));
    assert_eq!(pool(&mut r.tw, &r.families, r.pat_hold), (10, 10));
    assert_eq!(
        live_lots(&mut r.tw, &r.families, r.pat_hold),
        0,
        "a Draw delivers pool quantity, never a lot"
    );
    assert_eq!(posted(&r.tw, r.pat_account), FUNDS - 7_000);
    assert_eq!(posted(&r.tw, r.vendor_account), 7_000);
    let row = market_of(&r.tw).quote(q).expect("the trade row").clone();
    assert!(row.drawn_at.is_some(), "the mirror saw the receipt");
    assert!(row.settled.is_some());
    assert_eq!(row.lot.map(|l| l.kind), Some(PaidKind::Draw));
    no_settlement_pages(&r.tw);
}

// ---- 2: the race ----------------------------------------------------------------

/// **Two buyers race the last tonnes; the second reads `STALE_QUOTE` before any
/// contract** — the first accept's commit decremented the counter to zero, which
/// deleted the listing (a revision bump at any other quantity).
#[test]
fn ic7_two_buyers_racing_the_last_tonnes_the_second_is_refused_before_any_contract() {
    let mut r = rig_with(Opts {
        draw: Some(10),
        ..Opts::default()
    });
    let listing = r.listing.expect("listed");
    let before = market_of(&r.tw).listing(listing).expect("listed").revision;
    let qp = quote(&mut r.tw, r.pat, listing, 10);
    let qq = quote(&mut r.tw, r.quinn, listing, 10);
    ok(
        &mut r.tw,
        r.pat,
        &format!("+market/accept {}={}", qp.0, r.pat_hold.vault().0),
    );
    let _pat_leg = take(&mut r.tw);
    assert!(
        market_of(&r.tw)
            .listing(listing)
            .is_none_or(|l| l.quantity == 0 && l.revision > before),
        "the first accept moved the listing's counter (and so its revision)"
    );
    let refused = err(
        &mut r.tw,
        r.quinn,
        &format!("+market/accept {}={}", qq.0, r.quinn_hold.vault().0),
    );
    assert!(refused.contains(STALE_QUOTE), "{refused}");
    assert!(
        market_of(&r.tw)
            .quote(qq)
            .is_some_and(|q| q.contract.is_none())
    );
    assert!(r.tw.engine_mut().take_authoritative_submit().is_none());
    assert_eq!(posted(&r.tw, r.quinn_account), FUNDS);
    assert_eq!(held(&r.tw, r.quinn_account), 0);
    no_settlement_pages(&r.tw);
}

// ---- 3: the gate refuses before money ------------------------------------------

/// **A gate refusal escrows nothing**: the quote is refused in the gate's own words with
/// no quote written, and an accept by a buyer who walked away is refused with no
/// contract, no submission and nothing moved.
#[test]
fn ic7_a_gate_refusal_escrows_nothing() {
    let mut r = rig();
    let listing = r.listing.expect("listed");
    r.tw.world_mut()
        .move_to(r.pat, r.corridor)
        .expect("walk out");
    let quotes_before = market_of(&r.tw).quotes().count();
    let refused = err(&mut r.tw, r.pat, &format!("+market/quote {listing}=10"));
    assert!(refused.contains(AWAY), "{refused}");
    assert_eq!(
        market_of(&r.tw).quotes().count(),
        quotes_before,
        "no quote written"
    );

    r.tw.world_mut().move_to(r.pat, LIMBO).expect("walk back");
    let q = quote(&mut r.tw, r.pat, listing, 10);
    r.tw.world_mut()
        .move_to(r.pat, r.corridor)
        .expect("walk out");
    let refused = err(&mut r.tw, r.pat, &format!("+market/accept {}", q.0));
    assert!(refused.contains(AWAY), "{refused}");
    assert!(
        market_of(&r.tw)
            .quote(q)
            .is_some_and(|q| q.contract.is_none())
    );
    assert!(r.tw.engine_mut().take_authoritative_submit().is_none());
    assert_eq!(posted(&r.tw, r.pat_account), FUNDS);
    assert_eq!(held(&r.tw, r.pat_account), 0);
    assert_eq!(listing_qty(&r.tw, listing), Some(100));
    assert_eq!(pool(&mut r.tw, &r.families, r.shop), (100, 100));
    assert_eq!(pool(&mut r.tw, &r.families, r.pat_hold), (0, 0));
    no_settlement_pages(&r.tw);
}

// ---- 4: the gate's defaults -------------------------------------------------------

/// **With no argument, the board and the destination come from the gate**: bare
/// `+market/list` at Luna's counter shows only Luna's listings, `+market/accept <q>`
/// delivers into the gate's hold, and `+market/accept <q> to=<hold>` is the same verb.
#[test]
fn ic7_no_argument_list_and_accept_default_from_the_gate() {
    let mut r = rig();
    let listing = r.listing.expect("listed");
    // A second vendor, trading in another venue, with a Draw listing of its own.
    ok(&mut r.tw, GOD, "@org/create Belt");
    let belt =
        r.tw.world()
            .module_state::<mush_org::OrgStore>()
            .and_then(|s| s.state().orgs().map(|o| o.id).max())
            .expect("the org exists");
    ok(
        &mut r.tw,
        GOD,
        &format!("@account/open org:#{}={CREDIT}", belt.0),
    );
    let backing =
        r.tw.world_mut()
            .create(ObjKind::Thing, "BeltStore", GOD, None)
            .expect("a container");
    ok(
        &mut r.tw,
        GOD,
        &format!(
            "@vault/create BeltStore=container:#{} owner=org:#{}",
            backing.0, belt.0
        ),
    );
    let belt_store = EndpointRef(
        economy_of(&r.tw)
            .snapshot()
            .vaults()
            .find(|v| v.owner == PrincipalId::Organization(belt))
            .expect("the vault")
            .id,
    );
    ok(
        &mut r.tw,
        GOD,
        &format!(
            "@market/vendor create #{}=#{} name=Belt market=ceres buy=300",
            belt.0,
            belt_store.vault().0
        ),
    );
    let belt_vendor = market_of(&r.tw).vendor_of_org(belt).expect("registered").id;
    seed_rooted_pool(&mut r.tw, &r.families, belt_store, 40, LotId(0xb3));
    ok(
        &mut r.tw,
        GOD,
        &format!(
            "@market/vendor stock #{}={GOOD} price=650 qty=40",
            belt_vendor.0
        ),
    );
    let ceres = ok(&mut r.tw, r.pat, "+market/list market=ceres");
    assert!(ceres.contains("[ceres] at Belt"), "{ceres}");

    let board = ok(&mut r.tw, r.pat, "+market/list");
    assert!(board.contains(&format!("#{listing} ")), "{board}");
    assert!(board.contains("[luna] at Ares"), "{board}");
    assert!(
        !board.contains("[ceres]"),
        "the bare board is Luna's: {board}"
    );
    // Away from the counter, no venue: the whole page, as before IC7.
    r.tw.world_mut()
        .move_to(r.pat, r.corridor)
        .expect("walk out");
    let page = ok(&mut r.tw, r.pat, "+market/list");
    assert!(
        page.contains("[ceres]") && page.contains("[luna]"),
        "{page}"
    );
    r.tw.world_mut().move_to(r.pat, LIMBO).expect("walk back");

    let q = quote(&mut r.tw, r.pat, listing, 5);
    ok(&mut r.tw, r.pat, &format!("+market/accept {}", q.0));
    land(&mut r.tw, 0x41).expect("the first draw lands");
    assert_eq!(pool(&mut r.tw, &r.families, r.pat_hold), (5, 5));

    let q = quote(&mut r.tw, r.pat, listing, 7);
    ok(
        &mut r.tw,
        r.pat,
        &format!("+market/accept {} to={}", q.0, r.pat_hold.vault().0),
    );
    land(&mut r.tw, 0x42).expect("the second draw lands");
    assert_eq!(pool(&mut r.tw, &r.families, r.pat_hold), (12, 12));
    // A second word that is not `to` is no command at all.
    let refused = err(&mut r.tw, r.pat, &format!("+market/accept {} into=5", q.0));
    assert!(refused.contains("No such quote."), "{refused}");
    no_settlement_pages(&r.tw);
}

// ---- 5: capacity refuses before money -----------------------------------------------

/// **A provider refusing on volume escrows nothing**: its line verbatim, no contract,
/// no submission, nothing charged, no pool moved.
#[test]
fn ic7_a_provider_refusing_on_volume_escrows_nothing() {
    let mut r = rig_with(Opts {
        capacity: Some(5),
        ..Opts::default()
    });
    let listing = r.listing.expect("listed");
    let q = quote(&mut r.tw, r.pat, listing, 10);
    let refused = err(&mut r.tw, r.pat, &format!("+market/accept {}", q.0));
    assert!(refused.contains(&packed_line(0, 5, 10)), "{refused}");
    assert!(
        market_of(&r.tw)
            .quote(q)
            .is_some_and(|q| q.contract.is_none())
    );
    assert!(r.tw.engine_mut().take_authoritative_submit().is_none());
    assert_eq!(posted(&r.tw, r.pat_account), FUNDS);
    assert_eq!(held(&r.tw, r.pat_account), 0);
    assert_eq!(listing_qty(&r.tw, listing), Some(100));
    assert_eq!(pool(&mut r.tw, &r.families, r.shop), (100, 100));
    assert_eq!(pool(&mut r.tw, &r.families, r.pat_hold), (0, 0));
    no_settlement_pages(&r.tw);
}

// ---- 6: a Draw refused at drain -------------------------------------------------------

/// **A Draw over the limit at drain is refused in its prepare, its contract never
/// settles, and it expires with the escrow returned and the listing restored.**
///
/// Two accepts into one hold of 100 units: the first's submission is held back while
/// the second is quoted (after the first accept, against the decremented counter),
/// accepted (the pre-check reads an empty hold and passes) and landed; then the first
/// is driven and its prepare's capacity backstop refuses it.
#[test]
fn ic7_a_draw_over_the_limit_at_drain_is_refused_and_its_escrow_returns() {
    let mut r = rig_with(Opts {
        capacity: Some(100),
        pool: 130,
        draw: Some(130),
        ..Opts::default()
    });
    let listing = r.listing.expect("listed");
    let q1 = quote(&mut r.tw, r.pat, listing, 60);
    ok(&mut r.tw, r.pat, &format!("+market/accept {}", q1.0));
    let c1 = contract_of(&r.tw, q1);
    let s1 = take(&mut r.tw);
    let q2 = quote(&mut r.tw, r.pat, listing, 60);
    ok(&mut r.tw, r.pat, &format!("+market/accept {}", q2.0));
    let c2 = contract_of(&r.tw, q2);
    assert_eq!(listing_qty(&r.tw, listing), Some(10));
    land(&mut r.tw, 0x61).expect("the second draw lands");
    let refusal = land_submit(&mut r.tw, &s1, 0x62)
        .expect_err("the first draw would overfill the hold and is refused at its prepare");
    assert!(refusal.starts_with("prepare:"), "{refusal}");
    // It was refused FOR VOLUME (reviewer T3): the prepare read the hold's existing pool
    // (60, through `families.pool` inside the txn) and the plan said 40 more fit — not an
    // unreadable read, which would refuse with the same one line.
    assert_eq!(
        r.cfg.lock().expect("cfg").last_check,
        Some(Err(CapacityRefusal::Refused {
            fits: 40,
            line: packed_line(60, 100, 60),
        })),
        "the prepare's capacity backstop refused on volume"
    );
    assert_eq!(pool(&mut r.tw, &r.families, r.pat_hold), (60, 60));
    assert_eq!(pool(&mut r.tw, &r.families, r.shop), (70, 70));

    settle(&mut r.tw, c2);
    assert!(matches!(state_of(&r.tw, c1), Some(ContractState::Active)));
    assert!(receipt(&r.tw, c1).is_none());
    assert!(
        market_of(&r.tw)
            .quote(q1)
            .is_some_and(|q| q.drawn_at.is_none())
    );

    let accepted_at = market_of(&r.tw)
        .quote(q1)
        .and_then(|q| q.lot.as_ref().map(|l| l.accepted_at))
        .expect("the terms");
    at(&mut r.tw, accepted_at + DRAW_WINDOW_SECS + 1);
    at(&mut r.tw, accepted_at + DRAW_WINDOW_SECS + 62);
    assert!(matches!(state_of(&r.tw, c1), Some(ContractState::Expired)));
    assert_eq!(posted(&r.tw, r.pat_account), FUNDS - 60 * 700);
    assert_eq!(held(&r.tw, r.pat_account), 0, "the escrow came back");
    assert_eq!(
        listing_qty(&r.tw, listing),
        Some(70),
        "the listing is restored by 60"
    );
    let pages: Vec<_> = economy_of(&r.tw).escalations().collect();
    assert!(pages.is_empty(), "no page of any kind: {pages:?}");
}

// ---- 6b: a refused Draw, re-staged once there is room ---------------------------------

/// **A Draw refused at its prepare is prepared again when it is re-staged** (reviewer T2,
/// the stage doc's "a re-stage after a refused prepare is prepared again"): test 6's
/// refusal, then the hold is emptied, `+market/accept <q>` re-stages the identical
/// request, it lands through the capacity backstop reading an existing pool, and the deal
/// settles once — nothing moves twice and the listing is not restored.
#[test]
fn ic7_a_draw_refused_at_its_prepare_is_restaged_and_lands() {
    let mut r = rig_with(Opts {
        capacity: Some(100),
        pool: 130,
        draw: Some(130),
        ..Opts::default()
    });
    let listing = r.listing.expect("listed");
    let q1 = quote(&mut r.tw, r.pat, listing, 60);
    ok(&mut r.tw, r.pat, &format!("+market/accept {}", q1.0));
    let c1 = contract_of(&r.tw, q1);
    let s1 = take(&mut r.tw);
    let q2 = quote(&mut r.tw, r.pat, listing, 60);
    ok(&mut r.tw, r.pat, &format!("+market/accept {}", q2.0));
    let c2 = contract_of(&r.tw, q2);
    land(&mut r.tw, 0x6a).expect("the second draw lands");
    land_submit(&mut r.tw, &s1, 0x6b).expect_err("the first draw is refused for volume");
    assert!(matches!(
        r.cfg.lock().expect("cfg").last_check,
        Some(Err(CapacityRefusal::Refused { fits: 40, .. }))
    ));

    // The hold is unloaded (by hand, standing in for whatever empties it in play).
    let families = r.families.clone();
    let (hold, depot) = (r.pat_hold, r.depot);
    apply(&mut r.tw, &families, |txn| {
        families
            .transfer(txn, hold, depot, &commodity("iron"), 60)
            .map(|_| ())
    })
    .expect("the hold is emptied");
    assert_eq!(pool(&mut r.tw, &r.families, r.pat_hold), (0, 0));

    let again = ok(&mut r.tw, r.pat, &format!("+market/accept {}", q1.0));
    assert!(
        again.contains(&format!(
            "Asking again for the hand-over on contract #{c1}."
        )),
        "{again}"
    );
    let s1b = take(&mut r.tw);
    assert_eq!(s1b, s1, "the re-stage is the refused request itself");
    land_submit(&mut r.tw, &s1b, 0x6c).expect("prepared again, it lands");
    assert_eq!(
        r.cfg.lock().expect("cfg").last_check,
        Some(Ok(())),
        "the backstop read the (now empty) hold pool and admitted it"
    );
    assert!(receipt(&r.tw, c1).is_some());
    assert_eq!(pool(&mut r.tw, &r.families, r.pat_hold), (60, 60));
    assert_eq!(pool(&mut r.tw, &r.families, r.shop), (10, 10));

    settle(&mut r.tw, c1);
    settle(&mut r.tw, c2);
    assert_eq!(posted(&r.tw, r.pat_account), FUNDS - 120 * 700);
    assert_eq!(held(&r.tw, r.pat_account), 0);
    assert_eq!(listing_qty(&r.tw, listing), Some(10), "nothing is restored");
    let pages: Vec<_> = economy_of(&r.tw).escalations().collect();
    assert!(pages.is_empty(), "no page of any kind: {pages:?}");
}

// ---- 7 and 8: presence and the typed-read delegation -----------------------------------

/// **`Presence::Ship` reaches the gate**: a buyer away from the counter is refused by
/// the typed verb, and admitted by `accept_at(.., Presence::Ship(hull))`.
#[test]
fn ic7_presence_ship_reaches_the_gate() {
    let mut r = rig_with(Opts {
        probe_delegate: true,
        ..Opts::default()
    });
    let listing = r.listing.expect("listed");
    let q = quote(&mut r.tw, r.pat, listing, 10);
    r.tw.world_mut()
        .move_to(r.pat, r.corridor)
        .expect("walk out");
    let refused = err(&mut r.tw, r.pat, &format!("+market/accept {}", q.0));
    assert!(refused.contains(AWAY), "{refused}");
    let hull = r.hull;
    let taken = with_probe(&mut r.tw, r.pat, move |ctx| {
        mush_market::accept_at(ctx, q, None, Presence::Ship(hull)).map_err(|e| format!("{e:?}"))
    });
    let contract = taken.expect("a queued order is admitted on its hull");
    assert_eq!(r.cfg.lock().expect("cfg").last, Some(Presence::Ship(hull)));
    land(&mut r.tw, 0x71).expect("the draw lands");
    settle(&mut r.tw, contract);
    assert_eq!(pool(&mut r.tw, &r.families, r.pat_hold), (10, 10));
    no_settlement_pages(&r.tw);
}

/// **An unreadable pool refuses before money moves**: the probe is not a quantity
/// delegate, so its typed reads fail, and the capacity pre-check refuses rather than
/// reading the hold as empty (the IC11-without-IC5-delegation case).
#[test]
fn ic7_an_unreadable_pool_refuses_before_money_moves() {
    let mut r = rig_with(Opts {
        capacity: Some(100),
        ..Opts::default()
    });
    let listing = r.listing.expect("listed");
    let q = quote(&mut r.tw, r.pat, listing, 10);
    let hull = r.hull;
    let taken = with_probe(&mut r.tw, r.pat, move |ctx| {
        mush_market::accept_at(ctx, q, None, Presence::Ship(hull)).map_err(|e| format!("{e:?}"))
    });
    let refusal = taken.expect_err("an unreadable hold refuses");
    assert!(refusal.contains(CAPACITY_UNREADABLE), "{refusal}");
    assert!(
        market_of(&r.tw)
            .quote(q)
            .is_some_and(|q| q.contract.is_none())
    );
    assert!(r.tw.engine_mut().take_authoritative_submit().is_none());
    assert_eq!(posted(&r.tw, r.pat_account), FUNDS);
    assert_eq!(held(&r.tw, r.pat_account), 0);
    no_settlement_pages(&r.tw);
}

// ---- 9: the contract key ------------------------------------------------------------

/// **A re-staged lot leg is the same request and moves nothing twice.** The re-stage
/// carries the identical key, parameters, actor, action, subject and targets — every
/// field the driver digests into the request uid, so the driver resolves it as the
/// request it already finalized (the HIT, by proxy: L13). Driven anyway through the
/// rig, which bypasses the driver's preflight, the receipt guard refuses it.
#[test]
fn ic7_a_restaged_lot_leg_is_the_same_request_and_moves_nothing_twice() {
    let mut r = rig();
    let listing = r.listing.expect("listed");
    let q = quote(&mut r.tw, r.pat, listing, 10);
    ok(&mut r.tw, r.pat, &format!("+market/accept {}", q.0));
    let contract = contract_of(&r.tw, q);
    let s1 = take(&mut r.tw);
    assert_eq!(s1.key.as_str(), format!("market.lot.draw.{:x}", contract.0));
    land_submit(&mut r.tw, &s1, 0x91).expect("the draw lands");

    // Time passes before the re-stage (reviewer T2): five minutes on the real clock, and
    // so on the IC clock at ratio 1, still well before `draw_by` and with no sweep run
    // (the mirror must not see the receipt yet). A paid leg's key and parameters carry
    // nothing time-varying, so the re-stage is still the same request.
    let ic_before = r.tw.world().world_ic_now().at;
    let staged_at = r.tw.world().now();
    r.tw.set_now(staged_at + 300);
    assert!(
        r.tw.world().world_ic_now().at > ic_before,
        "the IC clock moved too"
    );
    let again = ok(&mut r.tw, r.pat, &format!("+market/accept {}", q.0));
    assert!(
        again.contains(&format!(
            "Asking again for the hand-over on contract #{contract}. It settles once the goods \
             have moved."
        )),
        "{again}"
    );
    let s2 = take(&mut r.tw);
    assert_eq!(s2.key, s1.key);
    assert_eq!(s2.parameters, s1.parameters);
    assert_eq!(s2.actor, s1.actor);
    assert_eq!(s2.action, s1.action);
    assert_eq!(s2.subject, s1.subject);
    assert_eq!(s2.targets, s1.targets);
    assert_eq!(s2, s1, "the whole submission is the same request");
    let refusal = land_submit(&mut r.tw, &s2, 0x92)
        .expect_err("the receipt guard refuses a second draw for one contract");
    assert!(refusal.starts_with("prepare:"), "{refusal}");
    assert_eq!(pool(&mut r.tw, &r.families, r.shop), (90, 90));
    assert_eq!(pool(&mut r.tw, &r.families, r.pat_hold), (10, 10));

    settle(&mut r.tw, contract);
    assert!(
        market_of(&r.tw)
            .quote(q)
            .is_some_and(|q| q.drawn_at.is_some())
    );
    let done = err(&mut r.tw, r.pat, &format!("+market/accept {}", q.0));
    assert!(done.contains("already been taken up"), "{done}");
    assert_eq!(posted(&r.tw, r.pat_account), FUNDS - 7_000);
    no_settlement_pages(&r.tw);
}

// ---- 10: the governed rule ------------------------------------------------------------

/// **A non-port listing never delivers into a governed endpoint**: a hold-backed
/// `Supply::Lot` listing, and a Draw whose seller the gate does not call a port seller.
#[test]
fn ic7_a_player_listing_never_delivers_into_a_governed_endpoint() {
    let mut r = rig();
    let listing = r.listing.expect("listed");
    ok(
        &mut r.tw,
        GOD,
        &format!(
            "@market/vendor stock #{}={GOOD} price=700 qty=10 endpoint={}",
            r.vendor.0,
            r.depot.vault().0
        ),
    );
    land(&mut r.tw, 0xa1).expect("the hold-backed listing's reserve lands");
    let held_listing = market_of(&r.tw)
        .lot_listings()
        .map(|l| l.id)
        .next()
        .expect("the hold-backed listing");
    let q = quote(&mut r.tw, r.pat, held_listing, 10);
    let refused = err(
        &mut r.tw,
        r.pat,
        &format!("+market/accept {}={}", q.0, r.pat_hold.vault().0),
    );
    assert!(refused.contains(PORT_ONLY_ENDPOINT), "{refused}");
    assert!(
        market_of(&r.tw)
            .quote(q)
            .is_some_and(|q| q.contract.is_none())
    );

    r.cfg.lock().expect("cfg").port_seller = None;
    let q = quote(&mut r.tw, r.pat, listing, 10);
    let refused = err(&mut r.tw, r.pat, &format!("+market/accept {}", q.0));
    assert!(refused.contains(PORT_ONLY_ENDPOINT), "{refused}");
    assert!(r.tw.engine_mut().take_authoritative_submit().is_none());
    assert_eq!(posted(&r.tw, r.pat_account), FUNDS);
    assert_eq!(held(&r.tw, r.pat_account), 0);
    no_settlement_pages(&r.tw);
}

// ---- 11: the party-cap source -------------------------------------------------------------

/// **The party-cap source admits after the table, and confers no money authority.**
#[test]
fn ic7_the_party_cap_source_admits_after_the_table_and_confers_no_money_authority() {
    // Source off, table empty: the vendor cannot accept, and the buyer's escrow is
    // released by the cancel.
    let mut r = rig();
    let listing = r.listing.expect("listed");
    r.source_on.store(false, Ordering::SeqCst);
    let q = quote(&mut r.tw, r.pat, listing, 10);
    let refused = err(&mut r.tw, r.pat, &format!("+market/accept {}", q.0));
    assert!(
        refused.contains("That vendor cannot complete the sale right now. Nothing was agreed."),
        "{refused}"
    );
    assert_eq!(posted(&r.tw, r.pat_account), FUNDS);
    assert_eq!(held(&r.tw, r.pat_account), 0, "the escrow was released");
    assert!(r.tw.engine_mut().take_authoritative_submit().is_none());
    assert_eq!(listing_qty(&r.tw, listing), Some(100));

    // Source on: the sale goes through.
    r.source_on.store(true, Ordering::SeqCst);
    let q = quote(&mut r.tw, r.pat, listing, 10);
    ok(&mut r.tw, r.pat, &format!("+market/accept {}", q.0));
    let contract = contract_of(&r.tw, q);
    land(&mut r.tw, 0xb1).expect("the draw lands");
    settle(&mut r.tw, contract);
    assert_eq!(posted(&r.tw, r.pat_account), FUNDS - 7_000);
    no_settlement_pages(&r.tw);

    // The vendor in the frozen table and the source off: it still sells, and the source
    // is never consulted — the consideration site asks `may_view_account` first, so the
    // buyer's own account never reaches the predicate, and the table answers the accept.
    let mut r = rig_with(Opts {
        table_vendor: true,
        ..Opts::default()
    });
    let listing = r.listing.expect("listed");
    r.source_on.store(false, Ordering::SeqCst);
    let q = quote(&mut r.tw, r.pat, listing, 10);
    ok(&mut r.tw, r.pat, &format!("+market/accept {}", q.0));
    let contract = contract_of(&r.tw, q);
    land(&mut r.tw, 0xb2).expect("the draw lands");
    settle(&mut r.tw, contract);
    assert_eq!(posted(&r.tw, r.pat_account), FUNDS - 7_000);
    assert_eq!(
        r.source_calls.load(Ordering::SeqCst),
        0,
        "the source was not consulted"
    );
    no_settlement_pages(&r.tw);
}

// ---- 11b: the other two party-cap sites ---------------------------------------------------

/// `+market/sell`'s shape, built by hand: an offer whose one party is the vendor and whose
/// consideration is the VENDOR's account, paying `to`.
fn vendor_paid_spec(
    vendor_account: AccountId,
    to: AccountId,
    now: i64,
) -> mush_economy::ContractSpec {
    use mush_economy::{
        Amount, ConsiderationSpec, ContractRoles, ContractSpec, EconomicClock, EconomicDeadline,
        EconomicVisibility, EvidenceKind, Expiry, OutcomeTerms, PayoutShare, Share, TypedCondition,
    };
    let deadline = EconomicDeadline {
        clock: EconomicClock::RealUtc,
        at: now + 3_600,
    };
    ContractSpec {
        name: "ic7 vendor-paid offer".into(),
        scope: None,
        parties: vec![PrincipalId::Organization(VENDOR_ORG)],
        considerations: vec![ConsiderationSpec {
            account: vendor_account,
            amount: Amount::from_minor(100),
        }],
        outcomes: vec![OutcomeTerms {
            name: "paid".into(),
            when: vec![0],
            payouts: vec![PayoutShare {
                to,
                share: Share::Exact(Amount::from_minor(100)),
            }],
            expense_treatment: mush_economy::ExpenseTreatment::None,
            asset_consequences: Vec::new(),
        }],
        remainder_to: vendor_account,
        conditions: vec![TypedCondition::Before { deadline }],
        adjudicator: None,
        evidence: vec![EvidenceKind::Before],
        deadline: Some(deadline),
        on_expiry: Expiry::Refund,
        visibility: EconomicVisibility::Parties,
        roles: ContractRoles {
            author: None,
            payer: Some(PrincipalId::Organization(VENDOR_ORG)),
            representative: None,
            facilitator: None,
        },
        deterministic: true,
        deliverables: Vec::new(),
    }
}

/// **The party-cap source reaches the other two sites** (reviewer T4): the
/// consideration-naming rule (a player may name a source-named vendor's own account as the
/// consideration of an offer the vendor is a party to — `+market/sell`'s shape) and
/// `deliver_settled` (a source-named vendor delivers against a deal it made). With the
/// frozen table empty, the source off refuses both and the source on admits both, and
/// the source is asked each time.
#[test]
fn ic7_the_party_cap_source_reaches_the_consideration_rule_and_the_delivery_seam() {
    let mut r = rig();
    let listing = r.listing.expect("listed");
    let q = quote(&mut r.tw, r.pat, listing, 10);
    ok(&mut r.tw, r.pat, &format!("+market/accept {}", q.0));
    let contract = contract_of(&r.tw, q);
    land(&mut r.tw, 0xd1).expect("the draw lands");
    settle(&mut r.tw, contract);
    let (pat_account, vendor_account) = (r.pat_account, r.vendor_account);
    let pat_before = posted(&r.tw, pat_account);
    let vendor_before = posted(&r.tw, vendor_account);

    // (a) The consideration rule. Pat cannot view the vendor's account, so only a
    // party-capable vendor's account may be named back to it.
    r.source_on.store(false, Ordering::SeqCst);
    let calls = r.source_calls.load(Ordering::SeqCst);
    let now = r.tw.world().now();
    let refused = with_probe(&mut r.tw, r.pat, move |ctx| {
        mush_economy::offer_contract(ctx, vendor_paid_spec(vendor_account, pat_account, now))
    });
    assert_eq!(
        refused,
        Err(mush_economy::ContractError::NoSuchAccount(vendor_account)),
        "with the source off the vendor's account is not nameable"
    );
    assert!(
        r.source_calls.load(Ordering::SeqCst) > calls,
        "the source was asked"
    );
    r.source_on.store(true, Ordering::SeqCst);
    let calls = r.source_calls.load(Ordering::SeqCst);
    let offered = with_probe(&mut r.tw, r.pat, move |ctx| {
        mush_economy::offer_contract(ctx, vendor_paid_spec(vendor_account, pat_account, now))
    })
    .expect("a source-named vendor's account may be named back to it");
    assert!(matches!(
        state_of(&r.tw, offered),
        Some(ContractState::Offered)
    ));
    assert!(
        r.source_calls.load(Ordering::SeqCst) > calls,
        "the source was asked"
    );

    // (b) `deliver_settled` against the settled purchase, from the vendor's account.
    r.source_on.store(false, Ordering::SeqCst);
    let calls = r.source_calls.load(Ordering::SeqCst);
    let refused = with_probe(&mut r.tw, r.pat, move |ctx| {
        mush_economy::deliver_settled(
            ctx,
            contract,
            vendor_account,
            pat_account,
            mush_economy::Amount::from_minor(100),
        )
        .map(|_| ())
    });
    assert_eq!(refused, Err(mush_economy::ContractError::NoStanding));
    assert!(
        r.source_calls.load(Ordering::SeqCst) > calls,
        "the source was asked"
    );
    assert_eq!(posted(&r.tw, vendor_account), vendor_before);
    assert_eq!(posted(&r.tw, pat_account), pat_before);
    r.source_on.store(true, Ordering::SeqCst);
    with_probe(&mut r.tw, r.pat, move |ctx| {
        mush_economy::deliver_settled(
            ctx,
            contract,
            vendor_account,
            pat_account,
            mush_economy::Amount::from_minor(100),
        )
        .map(|_| ())
    })
    .expect("a source-named vendor delivers against its own settled deal");
    assert_eq!(posted(&r.tw, vendor_account), vendor_before - 100);
    assert_eq!(posted(&r.tw, pat_account), pat_before + 100);
    no_settlement_pages(&r.tw);
}

// ---- 12: the paid projection ------------------------------------------------------------

/// **The paid legs are projected only over the actor's own paid contracts** — and that
/// closes IC0's direct-consume path (L12).
#[test]
fn ic7_the_paid_legs_project_only_paid_contracts() {
    let mut r = rig();
    let listing = r.listing.expect("listed");
    let (sam, _sam_account, sam_hold) = buyer(&mut r.tw, "Sam", r.faucet);

    // (a) An endpoint and no paid quote: reserve and release only, no `contract`.
    let offer = lot_offer(&r.tw, sam).expect("an endpoint holder is offered the lot action");
    assert_eq!(
        choices(&offer, "op"),
        Some(vec!["reserve".to_owned(), "release".to_owned()])
    );
    assert!(choices(&offer, "contract").is_none());

    // (b) The consume hole, closed: a live hold-backed listing, and a stranger naming it
    // directly cannot even resolve the op.
    ok(
        &mut r.tw,
        GOD,
        &format!(
            "@market/vendor stock #{}={GOOD} price=700 qty=10 endpoint={}",
            r.vendor.0,
            r.depot.vault().0
        ),
    );
    land(&mut r.tw, 0xc1).expect("the reserve lands");
    let held_listing = market_of(&r.tw)
        .lot_listings()
        .map(|l| l.id)
        .next()
        .expect("the hold-backed listing");
    let theft = forged(
        &r.tw,
        sam,
        json!({
            "op": "consume",
            "listing": held_listing.0,
            "endpoint": sam_hold.vault().0.to_string(),
            "source": r.depot.vault().0.to_string(),
            "commodity": "iron",
            "quantity": 10,
            "grade": 0,
        }),
    );
    let refusal = land_submit(&mut r.tw, &theft, 0xc2).expect_err("no such op is offered");
    assert!(refusal.starts_with("resolve:"), "{refusal}");
    assert_eq!(
        pool(&mut r.tw, &r.families, r.depot),
        (50, 40),
        "the hold stands"
    );
    assert_eq!(live_lots(&mut r.tw, &r.families, sam_hold), 0);

    // (c) A forged draw naming another buyer's contract fails resolution, even for an
    // actor whose own offer carries a `contract` choice.
    let qp = quote(&mut r.tw, r.pat, listing, 10);
    ok(&mut r.tw, r.pat, &format!("+market/accept {}", qp.0));
    let cp = contract_of(&r.tw, qp);
    let pat_leg = take(&mut r.tw);
    let qq = quote(&mut r.tw, r.quinn, listing, 10);
    ok(
        &mut r.tw,
        r.quinn,
        &format!("+market/accept {}={}", qq.0, r.quinn_hold.vault().0),
    );
    let cq = contract_of(&r.tw, qq);
    let quinn_leg = take(&mut r.tw);
    let quinn_offer = lot_offer(&r.tw, r.quinn).expect("offered");
    assert_eq!(
        choices(&quinn_offer, "contract"),
        Some(vec![cq.0.to_string()])
    );
    let mut stolen = pat_leg.parameters.clone();
    stolen["endpoint"] = json!(r.quinn_hold.vault().0.to_string());
    let forgery = forged(&r.tw, r.quinn, stolen);
    let refusal = land_submit(&mut r.tw, &forgery, 0xc3).expect_err("not her contract");
    assert!(refusal.starts_with("resolve:"), "{refusal}");
    assert_eq!(
        cp.0.to_string(),
        pat_leg.parameters["contract"].as_str().unwrap_or_default()
    );

    // (d) Her own contract with a larger quantity resolves and is refused in the prepare.
    let mut greedy = pat_leg.parameters.clone();
    greedy["quantity"] = json!(11);
    let greedy = forged(&r.tw, r.pat, greedy);
    let refusal = land_submit(&mut r.tw, &greedy, 0xc4).expect_err("the terms are fixed");
    assert!(refusal.starts_with("prepare:"), "{refusal}");
    assert_eq!(pool(&mut r.tw, &r.families, r.shop), (100, 100));

    // The genuine legs still land.
    land_submit(&mut r.tw, &pat_leg, 0xc5).expect("Pat's draw lands");
    land_submit(&mut r.tw, &quinn_leg, 0xc6).expect("Quinn's draw lands");
    assert_eq!(pool(&mut r.tw, &r.families, r.shop), (80, 80));

    // (e) A hold-backed purchase by a buyer with no draw entry resolves and lands: IC0's
    // path, now carrying its contract.
    let qs = quote(&mut r.tw, sam, held_listing, 10);
    ok(
        &mut r.tw,
        sam,
        &format!("+market/accept {}={}", qs.0, sam_hold.vault().0),
    );
    let cs = contract_of(&r.tw, qs);
    let leg = take(&mut r.tw);
    assert_eq!(leg.parameters["op"], json!("consume"));
    assert_eq!(leg.parameters["contract"], json!(cs.0.to_string()));
    assert_eq!(leg.key.as_str(), format!("market.lot.consume.{:x}", cs.0));
    let sam_offer = lot_offer(&r.tw, sam).expect("offered");
    assert_eq!(
        choices(&sam_offer, "op"),
        Some(vec![
            "reserve".to_owned(),
            "consume".to_owned(),
            "release".to_owned()
        ])
    );
    // (f) The consume cross-check (L12, reviewer T1): Sam's OWN paid consume contract,
    // carrying a SECOND hold-backed listing's terms, resolves (consume is offered, the
    // contract is his) and is refused in the prepare by the field-for-field match. Without
    // that match the prepare would derive the second listing's reservation from the
    // submitted fields and settle its lot into Sam's hold with no contract for it.
    let yard = org_vault(&mut r.tw, r.root_scope, "AresYard");
    seed_rooted_pool(&mut r.tw, &r.families, yard, 50, LotId(0xb3));
    ok(
        &mut r.tw,
        GOD,
        &format!(
            "@market/vendor stock #{}={GOOD} price=700 qty=10 endpoint={}",
            r.vendor.0,
            yard.vault().0
        ),
    );
    land(&mut r.tw, 0xc8).expect("the second reserve lands");
    let yard_listing = market_of(&r.tw)
        .lot_listings()
        .map(|l| l.id)
        .max()
        .expect("the second hold-backed listing");
    assert_ne!(yard_listing, held_listing);
    assert_eq!(pool(&mut r.tw, &r.families, yard), (50, 40));
    let mut swapped = leg.parameters.clone();
    swapped["listing"] = json!(yard_listing.0);
    swapped["source"] = json!(yard.vault().0.to_string());
    let swapped = forged(&r.tw, sam, swapped);
    let refusal =
        land_submit(&mut r.tw, &swapped, 0xc9).expect_err("another listing's terms are refused");
    assert!(refusal.starts_with("prepare:"), "{refusal}");
    assert_eq!(
        pool(&mut r.tw, &r.families, yard),
        (50, 40),
        "the second listing's hold still stands"
    );
    assert!(market_of(&r.tw).listing(yard_listing).is_some());
    assert_eq!(live_lots(&mut r.tw, &r.families, sam_hold), 0);

    // (g) The op cross-check: with a paid Draw of his own beside the consume, Sam's offer
    // names both contracts; a `consume` naming the DRAW contract, with the Draw entry's
    // own terms, resolves and is refused in the prepare (the entry's op is `draw`).
    let qd = quote(&mut r.tw, sam, listing, 10);
    ok(
        &mut r.tw,
        sam,
        &format!("+market/accept {}={}", qd.0, sam_hold.vault().0),
    );
    let cd = contract_of(&r.tw, qd);
    let sam_draw = take(&mut r.tw);
    assert_eq!(sam_draw.parameters["op"], json!("draw"));
    let sam_offer = lot_offer(&r.tw, sam).expect("offered");
    let mut contracts = choices(&sam_offer, "contract").expect("a contract choice");
    contracts.sort();
    let mut expected = vec![cs.0.to_string(), cd.0.to_string()];
    expected.sort();
    assert_eq!(contracts, expected);
    let mut wrong_op = sam_draw.parameters.clone();
    wrong_op["op"] = json!("consume");
    let wrong_op = forged(&r.tw, sam, wrong_op);
    let refusal =
        land_submit(&mut r.tw, &wrong_op, 0xca).expect_err("a draw contract is not consumed");
    assert!(refusal.starts_with("prepare:"), "{refusal}");
    assert_eq!(pool(&mut r.tw, &r.families, r.shop), (80, 80));
    assert_eq!(pool(&mut r.tw, &r.families, sam_hold), (0, 0));
    assert_eq!(live_lots(&mut r.tw, &r.families, sam_hold), 0);

    // The genuine legs land, each once.
    land_submit(&mut r.tw, &leg, 0xc7).expect("the consume lands");
    assert_eq!(live_lots(&mut r.tw, &r.families, sam_hold), 1);
    land_submit(&mut r.tw, &sam_draw, 0xcb).expect("Sam's draw lands");
    assert_eq!(pool(&mut r.tw, &r.families, r.shop), (70, 70));
    assert_eq!(
        pool(&mut r.tw, &r.families, yard),
        (50, 40),
        "nothing ever touched the second listing's hold"
    );
    let _ = (cp, cq);
    no_settlement_pages(&r.tw);
}

// ---- 13: publishing a Draw listing ------------------------------------------------------------

/// **A Draw listing is checked against its storefront** before anything is written.
#[test]
fn ic7_a_draw_listing_is_checked_against_its_storefront() {
    let mut r = rig();
    let listing = r.listing.expect("listed");
    let v = r.vendor.0;
    // More than the pool holds.
    ok(
        &mut r.tw,
        GOD,
        &format!("@market/vendor withdraw {listing}"),
    );
    let refused = err(
        &mut r.tw,
        GOD,
        &format!("@market/vendor stock #{v}={GOOD} price=700 qty=101"),
    );
    assert!(
        refused.contains("That storefront holds less than that, so nothing was listed."),
        "{refused}"
    );
    // A second listing on the pool, of either kind.
    ok(
        &mut r.tw,
        GOD,
        &format!("@market/vendor stock #{v}={GOOD} price=700 qty=100"),
    );
    let refused = err(
        &mut r.tw,
        GOD,
        &format!("@market/vendor stock #{v}={GOOD} price=700 qty=5"),
    );
    assert!(
        refused.contains("already carries a live listing"),
        "{refused}"
    );
    let refused = err(
        &mut r.tw,
        GOD,
        &format!(
            "@market/vendor stock #{v}={GOOD} price=700 qty=5 endpoint={}",
            r.shop.vault().0
        ),
    );
    assert!(
        refused.contains("already carries a live listing"),
        "{refused}"
    );
    assert!(r.tw.engine_mut().take_authoritative_submit().is_none());
    // A drawn listing out of any vault but the storefront.
    let vendor = r.vendor;
    let depot = r.depot.vault();
    let refused = with_probe(&mut r.tw, GOD, move |ctx| {
        mush_market::list_goods(
            ctx,
            mush_market::ListingSpec {
                vendor,
                catalog: mush_economy::CatalogRef {
                    ruleset: "ic7".into(),
                    code: "iron".into(),
                },
                price: mush_economy::Amount::from_minor(700),
                quantity: 5,
                source: mush_market::Supply::Stock { vault: depot },
                object: None,
                visibility: mush_market::MarketVisibility::Public,
                ttl_secs: 86_400,
            },
        )
        .map_err(|e| format!("{e:?}"))
    });
    let refused = refused.expect_err("the depot is not the storefront");
    assert!(
        refused.contains("A drawn listing sells out of the vendor's own storefront vault."),
        "{refused}"
    );
    // A stranger may not; an officer of the vendor org may.
    let stranger = r.tw.spawn_player("Stranger");
    let refused = err(
        &mut r.tw,
        stranger,
        &format!("@market/vendor stock #{v}={GOOD} price=700 qty=5"),
    );
    assert!(refused.contains("No such vendor."), "{refused}");
    let live = market_of(&r.tw)
        .live_listings()
        .map(|l| l.id)
        .max()
        .expect("live");
    ok(&mut r.tw, GOD, &format!("@market/vendor withdraw {live}"));
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
    let said = ok(
        &mut r.tw,
        officer,
        &format!("@market/vendor stock #{v}={GOOD} price=700 qty=5"),
    );
    assert!(said.contains("published"), "{said}");
    no_settlement_pages(&r.tw);
}

// ---- 14: the fact namespace ------------------------------------------------------------

/// **Without the `market` fact namespace a Draw sale is refused before money moves**: a
/// Draw contract nobody can prove would only ever refund.
#[test]
fn ic7_without_the_market_fact_namespace_a_draw_sale_is_refused_before_money_moves() {
    let mut r = rig_with(Opts {
        facts: false,
        ..Opts::default()
    });
    let listing = r.listing.expect("listed");
    let q = quote(&mut r.tw, r.pat, listing, 10);
    let refused = err(&mut r.tw, r.pat, &format!("+market/accept {}", q.0));
    assert!(
        refused.contains(
            "That cannot be delivered to you right now, so the sale was not made. Nothing was \
             charged."
        ),
        "{refused}"
    );
    assert!(
        market_of(&r.tw)
            .quote(q)
            .is_some_and(|q| q.contract.is_none())
    );
    assert!(r.tw.engine_mut().take_authoritative_submit().is_none());
    assert_eq!(posted(&r.tw, r.pat_account), FUNDS);
    assert_eq!(held(&r.tw, r.pat_account), 0);
    no_settlement_pages(&r.tw);
}

// ---- 15: the cutoff ------------------------------------------------------------

/// **No Draw is offered or re-staged after its cutoff**, and the deal then lapses with
/// its refund.
#[test]
fn ic7_no_draw_is_offered_or_restaged_after_its_cutoff() {
    let mut r = rig();
    let listing = r.listing.expect("listed");
    let q = quote(&mut r.tw, r.pat, listing, 10);
    ok(&mut r.tw, r.pat, &format!("+market/accept {}", q.0));
    let contract = contract_of(&r.tw, q);
    let _never_landed = take(&mut r.tw);
    let offer = lot_offer(&r.tw, r.pat).expect("offered");
    assert_eq!(
        choices(&offer, "contract"),
        Some(vec![contract.0.to_string()])
    );
    assert!(choices(&offer, "op").is_some_and(|ops| ops.contains(&"draw".to_owned())));

    let accepted_at = market_of(&r.tw)
        .quote(q)
        .and_then(|q| q.lot.as_ref().map(|l| l.accepted_at))
        .expect("the terms");
    r.tw.set_now(accepted_at + DRAW_WINDOW_SECS - DRAW_SETTLE_MARGIN_SECS + 1);
    let offer = lot_offer(&r.tw, r.pat).expect("still offered reserve/release");
    assert!(
        choices(&offer, "contract").is_none(),
        "no draw entry past the cutoff"
    );
    assert!(choices(&offer, "op").is_some_and(|ops| !ops.contains(&"draw".to_owned())));
    let late = err(&mut r.tw, r.pat, &format!("+market/accept {}", q.0));
    assert!(late.contains(DRAW_TOO_LATE), "{late}");
    assert!(r.tw.engine_mut().take_authoritative_submit().is_none());

    at(&mut r.tw, accepted_at + DRAW_WINDOW_SECS + 1);
    at(&mut r.tw, accepted_at + DRAW_WINDOW_SECS + 62);
    assert!(matches!(
        state_of(&r.tw, contract),
        Some(ContractState::Expired)
    ));
    assert_eq!(posted(&r.tw, r.pat_account), FUNDS);
    assert_eq!(held(&r.tw, r.pat_account), 0);
    assert_eq!(listing_qty(&r.tw, listing), Some(100));
    no_settlement_pages(&r.tw);
}

// ---- 16: the no-relist guard ------------------------------------------------------------

/// **Goods drawn but lapsed are never relisted.** The mirror is starved (no sweep runs
/// between the landing and the deadline), so the contract expires with the goods moved:
/// whichever sweep arm runs first, the listing is not restored, and exactly one
/// `BootReconciliation` page names the contract.
#[test]
fn ic7_goods_drawn_but_lapsed_are_never_relisted() {
    let mut r = rig();
    let listing = r.listing.expect("listed");
    let q = quote(&mut r.tw, r.pat, listing, 10);
    ok(&mut r.tw, r.pat, &format!("+market/accept {}", q.0));
    let contract = contract_of(&r.tw, q);
    land(&mut r.tw, 0xf1).expect("the draw lands");
    assert!(receipt(&r.tw, contract).is_some());
    assert_eq!(listing_qty(&r.tw, listing), Some(90));

    let accepted_at = market_of(&r.tw)
        .quote(q)
        .and_then(|q| q.lot.as_ref().map(|l| l.accepted_at))
        .expect("the terms");
    at(&mut r.tw, accepted_at + DRAW_WINDOW_SECS + 1);
    at(&mut r.tw, accepted_at + DRAW_WINDOW_SECS + 62);
    assert!(matches!(
        state_of(&r.tw, contract),
        Some(ContractState::Expired)
    ));
    assert_eq!(
        listing_qty(&r.tw, listing),
        Some(90),
        "the listing is not restored"
    );
    assert_eq!(posted(&r.tw, r.pat_account), FUNDS, "the payment came back");
    assert_eq!(
        pool(&mut r.tw, &r.families, r.pat_hold),
        (10, 10),
        "the buyer holds the goods"
    );
    let pages: Vec<_> = economy_of(&r.tw)
        .escalations()
        .filter(|e| matches!(e.kind, EscalationKind::BootReconciliation))
        .filter(|e| matches!(e.subject, EscalationSubject::Contract(c) if c == contract))
        .collect();
    assert_eq!(pages.len(), 1, "one page names the contract: {pages:?}");
    no_settlement_pages(&r.tw);
}
