//! **Doc 18 §6 "The story test", part 1 (from IC10): one leg of the trade run, played
//! by hand, line by line** (doc 18 §2, §4.1–§4.4, §6; cut IC10 decision 15).
//!
//! A fresh character on a gateway account of their own walks in from Limbo through
//! the IC door, enrols in the Co-op's First Captain programme, buys the both-limits
//! cargo at the Ceres Exchange (derived from the goods table by
//! `support/vertex.rs::both_limits_vertex`, never named — D-18-39), buys reaction mass
//! and provisions at the chandlery at exactly the planner's burn and crew-days (read
//! off the pilot's own `+ship/plan`), boards, undocks for Stickney, flies the lane at
//! rate 3600 hearing the Kirkwood boundary and the docking in the Crew Compartment,
//! walks ashore to Kasei's Receiving Bay by the Stickney pack's own exit words, and
//! sells both goods into Kasei's standing bids. The one staff act during play is
//! `@tacsim/time rate=3600`, made once the hold is loaded and before the walk to the
//! gallery (§6's order: enrol → load at Ceres → fly at rate 3600). Each sale's deliver
//! leg is **landed by the rig** (standing in for the driver; the next sale would
//! otherwise refuse `SELL_BUSY` — reviewer F10).
//!
//! The assertions are §6's part-1 half: every expected reply is computed from the
//! posted prices and the vertex (stake − Σ costs + Σ nets, never a literal); on each
//! sale the pilot's net plus the Trust's share equals Kasei's payment, read from the
//! settlement journal; the money over the pilot, the Trust and every port org is
//! unchanged across the leg; every tonne in the hold traces to a seeded Origin
//! receipt; seeded − sold = remaining on every touched listing and pool; Kasei's pools
//! are the delivered intake; and no escalation of any kind is open.
//!
//! **Composed on the production layer** (`server_modules_with_tacsim` and
//! `tacsim_module_wired` with game content — no test-world flag, no probe action; the
//! IC10 brief's roster, review F3/C2). The production readiness report flags any world
//! rate but 24 and its Rate line gates `+coop/enrol`, while a test world boots at rate
//! 1: so the world is first put at the play rate by the deploy step dev1 takes (doc 18
//! §4.7, `@tacsim/time rate=24`, typed at boot before the clerks stock), and the one
//! staff act during play, rate 3600, comes after the enrolment and the purchases. (Doc
//! 18 §6's "rate 3600 the only staff act, none after boot" cannot hold on the
//! production layer as written — enrolment at 3600 is refused by the Rate line — an
//! IC12 matter with the `build_engine` promotion.) **Part 2** (the queue, cut IC11) and
//! the promotion of this file to `crates/mush-server/tests/` on `build_engine` (doc 18
//! §6) are later cuts'.

mod support;

#[path = "support/vertex.rs"]
mod vertex;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use mush_core::{
    AccountId, AccountIdentity, Audience, AuthoritativeSubmit, CanonicalId, CommitToken, GOD,
    IdempotencyKey, IdempotencyScope, LIMBO, ModuleWrite, ObjId, OfferExecutionContext,
    OfferSubmission, RequestUid, ScopedIdempotencyKey,
};
use mush_economy::{ContractId, ContractState, EconomyConfig, EconomyStore, VaultId};
use mush_inventory::{EndpointRef, InventoryHandles, LotId};
use mush_market::{ListingId, MarketStore};
use mush_org::PrincipalId;
use mush_proto::OutputEvent;
use mush_testkit::TestWorld;
use smol_str::SmolStr;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK};
use tacsim::pack::{PackEntry, load_pack};
use tacsim::port::apply::room_by_key;
use tacsim::port::{HullRow, PortDef, PortHandles, PortMap, PortMapState, StockIssueId, room_key};
use tacsim::{TacsimConfig, TacsimStore};

// ---------------------------------------------------------------------------
// The rig (voyage.rs's)
// ---------------------------------------------------------------------------

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

fn modules() -> Vec<Box<dyn mush_core::GameModule>> {
    use tacsim::composition as comp;

    let llm = Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let onboarding = mush_core::OnboardingConfig::default();
    let game = tacsim::GameEconomyConfig::default();
    let cfg = TacsimConfig {
        enabled: true,
        ..TacsimConfig::default()
    };
    let (mut mods, inventory) = comp::server_modules_with_tacsim(
        mush_weather::WeatherConfig::default(),
        EconomyConfig::default(),
        mush_atlas::AtlasConfig::default(),
        &onboarding,
        &game,
        comp::recovery_policy(&cfg.recovery),
    );
    mods.push(comp::worldgen_module_with(
        Arc::clone(&llm),
        mush_worldgen::WorldgenConfig::default(),
    ));
    mods.push(comp::npc_module_with(
        comp::npc_agent_adapter(),
        &mush_config::config::NpcCfg::default(),
    ));
    mods.push(comp::tacsim_module_wired(cfg, true, inventory));
    mods.push(comp::content_module_with(true, true).expect("the content module composes"));
    mods
}

#[derive(Default)]
struct MapOracle(Mutex<BTreeMap<ObjId, AccountId>>);

impl AccountIdentity for MapOracle {
    fn account_of(&self, character: ObjId) -> Option<AccountId> {
        self.0.lock().ok()?.get(&character).copied()
    }
}

struct Rig {
    tw: TestWorld,
    oracle: Arc<MapOracle>,
    typed: BTreeMap<(SmolStr, SmolStr), serde_json::Value>,
}

fn sweep(tw: &mut TestWorld, secs: i64) {
    let t = tw.world().now().saturating_add(secs);
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1000));
}

fn said(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    tw.seen_by(events, who).join("\n")
}

fn run(tw: &mut TestWorld, who: ObjId, line: &str) -> String {
    let events = tw.run(who, line);
    said(tw, &events, who)
}

fn store(tw: &TestWorld) -> &TacsimStore {
    tw.world()
        .module_state::<TacsimStore>()
        .expect("tacsim state")
}

fn economy(tw: &TestWorld) -> &EconomyStore {
    tw.world().module_state::<EconomyStore>().expect("economy")
}

fn market(tw: &TestWorld) -> &MarketStore {
    tw.world().module_state::<MarketStore>().expect("market")
}

fn port_map(tw: &TestWorld, entry: &str) -> PortMap {
    match store(tw).port_map(&cid(LANE_PACK), &cid(entry)) {
        PortMapState::Present(map) => map.clone(),
        other => panic!("{entry}'s map: {other:?}"),
    }
}

fn port_def(entry: &str) -> PortDef {
    let pack = load_pack(LANE_JSON).expect("the lane verifies");
    match pack.entry(&cid(entry)) {
        Some(PackEntry::Port(def)) => def.clone(),
        _ => panic!("{entry} is a port entry"),
    }
}

fn station_room(tw: &TestWorld, entry: &str, local: &str) -> ObjId {
    let def = port_def(entry);
    let key = room_key(&def.area, &format!("{}.{local}", def.area)).expect("a room key");
    room_by_key(tw.world(), &key).expect("the room is on this world")
}

fn location(tw: &TestWorld, obj: ObjId) -> Option<ObjId> {
    tw.world().get(obj).and_then(|o| o.location)
}

fn next_token() -> [u8; 16] {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut bytes = [0x6c_u8; 16];
    bytes[..4].copy_from_slice(&n.to_be_bytes());
    bytes
}

/// Drive one queued submission the way the world task does.
fn land(rig: &mut Rig, submit: &AuthoritativeSubmit) -> Result<(), String> {
    let token = next_token();
    let tw = &mut rig.tw;
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
            request: RequestUid::from_bytes(token),
            principal: mush_core::PrincipalId::Object { id: submit.actor },
            actor: submit.actor,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: mush_core::PrincipalId::Object { id: submit.actor },
                    owner: submit.action.key().owner.clone(),
                    namespace: CanonicalId::new(submit.action.key().id.as_str())
                        .map_err(|_| "namespace".to_owned())?,
                },
                1,
                IdempotencyKey::Request(RequestUid::from_bytes(token)),
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
            CommitToken::from_bytes(token),
        )
        .map_err(|error| format!("{error:?}"))?;
    let (ops, install) = prepared.into_parts();
    tw.engine_mut()
        .install_prepared_action(install)
        .map_err(|error| format!("{error:?}"))?;
    for op in ops {
        if let ModuleWrite::JsonSet { key, value } = op.op {
            rig.typed.insert((op.ns, key), value);
        }
    }
    Ok(())
}

fn drain(tw: &mut TestWorld) -> Vec<AuthoritativeSubmit> {
    let mut kept = Vec::new();
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        let owner = submit.action.key().owner.as_str();
        if owner == tacsim::NS {
            assert_ne!(submit.actor, GOD, "GOD never submits from the port tick");
        }
        if owner == tacsim::NS || owner == "inventory" || owner == "market" {
            kept.push(*submit);
        }
    }
    kept
}

fn land_all(rig: &mut Rig, queued: &[AuthoritativeSubmit]) {
    for submit in queued {
        land(rig, submit).unwrap_or_else(|error| {
            panic!(
                "{} by #{} did not land: {error}",
                submit.action.key().id,
                submit.actor.0
            )
        });
    }
}

fn land_quietly(rig: &mut Rig, queued: &[AuthoritativeSubmit]) {
    for submit in queued {
        let _ = land(rig, submit);
    }
}

fn fire_quietly(rig: &mut Rig) -> Vec<(Audience, OutputEvent)> {
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    land_quietly(rig, &queued);
    events
}

/// The port tick, until both ports are stocked and listed (background work, not a
/// staff act).
fn stock(rig: &mut Rig) {
    for _ in 0..40 {
        let out = run(&mut rig.tw, GOD, "@tacsim/port status");
        let gated: Vec<&str> = out
            .lines()
            .filter(|line| {
                line.contains(" vendor ")
                    || line.contains(" ask ")
                    || line.starts_with("market facts")
                    || line.starts_with("inventory facts")
            })
            .collect();
        if gated.len() > 1 && gated.iter().all(|line| line.ends_with("[ready]")) {
            return;
        }
        let t = rig.tw.world().now().saturating_add(6);
        rig.tw.set_now(t);
        let _ = rig.tw.fire_due(t.saturating_mul(1_000));
        let queued = drain(&mut rig.tw);
        land_all(rig, &queued);
    }
    panic!("the ports were not stocked within 40 fires");
}

fn families(tw: &TestWorld) -> mush_inventory::InventoryFamilies {
    tw.world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone()
}

fn pool(tw: &mut TestWorld, vault: VaultId, good: &str) -> u64 {
    let families = families(tw);
    let key = mush_inventory::pool_key(EndpointRef(vault), &cid(good)).expect("a pool key");
    let mut txn = tw
        .engine_mut()
        .action_transaction(&families.pools)
        .expect("a transaction");
    txn.get(&families.pools, &key)
        .expect("reads")
        .map_or(0, |scalar| {
            u64::try_from(scalar.current()).expect("non-negative")
        })
}

fn ic_now(tw: &TestWorld) -> i64 {
    tw.world().world_ic_now().at
}

fn to_ic(rig: &mut Rig, target: i64) -> Vec<(Audience, OutputEvent)> {
    let rate = i64::from(rig.tw.world().world_timeline().current().rate.max(1));
    let ahead = target.saturating_sub(ic_now(&rig.tw)).max(0);
    let real = (ahead + rate - 1) / rate;
    let t = rig.tw.world().now().saturating_add(real);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    land_quietly(rig, &queued);
    events
}

/// One typed line as the pilot, asserting where it leaves them.
fn walk(rig: &mut Rig, pilot: ObjId, word: &str, to: ObjId) {
    let out = run(&mut rig.tw, pilot, word);
    assert_eq!(location(&rig.tw, pilot), Some(to), "`{word}`: {out}");
}

fn credit_account(tw: &TestWorld, owner: &PrincipalId) -> mush_economy::AccountId {
    let credit = mush_economy::InstrumentId::new(tacsim::CREDIT).expect("credit");
    economy(tw)
        .snapshot()
        .accounts_of(owner)
        .find(|a| a.instrument == credit)
        .map(|a| a.id)
        .expect("a credit account")
}

fn posted(tw: &TestWorld, account: mush_economy::AccountId) -> i64 {
    economy(tw)
        .snapshot()
        .account(account)
        .expect("the account")
        .posted_balance
        .minor()
}

/// The money render (`5600.00cr`).
fn cr(minor: i64) -> String {
    format!("{}.{:02}cr", minor / 100, minor % 100)
}

fn listing_of(tw: &TestWorld, entry: &str, org: &str, good: &str) -> ListingId {
    let vault = VaultId(port_map(tw, entry).vendors[&cid(org)].vault);
    market(tw)
        .lot_listing_of(EndpointRef(vault), &cid(good))
        .unwrap_or_else(|| panic!("{entry}/{org} lists {good}"))
}

fn newest_quote(tw: &TestWorld, buyer: ObjId) -> mush_market::QuoteId {
    market(tw)
        .quotes_of_buyer(&PrincipalId::Object(buyer))
        .map(|q| q.id)
        .max()
        .expect("a quote")
}

/// `+market/quote <listing>=<qty>`, `+market/accept <quote>`, the Draw landed; the
/// price paid (minor).
fn buy(rig: &mut Rig, pilot: ObjId, listing: ListingId, qty: u64) -> i64 {
    let price = market(&rig.tw)
        .listing(listing)
        .map(|l| l.price.minor())
        .expect("the listing");
    let quoted = run(
        &mut rig.tw,
        pilot,
        &format!("+market/quote {listing}={qty}"),
    );
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = newest_quote(&rig.tw, pilot);
    let accepted = run(&mut rig.tw, pilot, &format!("+market/accept {quote}"));
    assert!(accepted.contains("Agreed."), "{accepted}");
    let queued = drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "{accepted}");
    land_all(rig, &queued);
    price * i64::try_from(qty).expect("qty")
}

/// `Burn B t (tank T t; …)` → `(B, T)`; `Stores C crew-days (locker L; …)` → `(C, L)`.
fn plan_numbers(plan: &str, head: &str, open: &str) -> Option<(u64, u64)> {
    let line = plan.lines().find(|l| l.starts_with(head))?;
    let first = line
        .strip_prefix(head)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let second = line
        .split(open)
        .nth(1)?
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    Some((first, second))
}

/// The settlement transaction's `(account, delta)` lines.
fn settlement(tw: &TestWorld, contract: ContractId) -> BTreeMap<mush_economy::AccountId, i64> {
    let Some(ContractState::Settled { transaction }) =
        economy(tw).snapshot().contract(contract).map(|c| c.state)
    else {
        panic!("contract #{contract} did not settle");
    };
    economy(tw)
        .transaction(transaction)
        .expect("the settlement")
        .lines
        .iter()
        .map(|l| (l.account, l.delta.minor()))
        .collect()
}

/// The sum of posted credit over the pilot, the Trust and every port org.
fn closed_sum(tw: &TestWorld, pilot: ObjId) -> i64 {
    let mut sum = posted(tw, credit_account(tw, &PrincipalId::Object(pilot)));
    for entry in ["ceres_port", "stickney_port"] {
        for row in port_map(tw, entry).orgs.values() {
            if let Some(account) = row.account {
                sum += posted(tw, mush_economy::AccountId(account));
            }
        }
    }
    sum
}

// ---------------------------------------------------------------------------
// Part 1
// ---------------------------------------------------------------------------

#[test]
#[allow(clippy::too_many_lines)]
fn ic10_part_one_a_leg_by_hand() {
    // Boot; the capital settles and the clerks stock both ports.
    let mut tw = TestWorld::with_modules(modules());
    assert!(tw.world().refused_namespace_writes().is_empty());
    tw.engine_mut().set_offer_keys(
        mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(1, [0x6c; 32]).expect("an offer secret"),
            [],
        )
        .expect("an offer key ring"),
    );
    let oracle = Arc::new(MapOracle::default());
    *tw.world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .expect("the simulation is composed") = mush_sim::AccountOracle::new(oracle.clone());
    let mut rig = Rig {
        tw,
        oracle,
        typed: BTreeMap::new(),
    };
    // Standing the world up as dev1 is stood up (doc 18 §4.7 "Config flips on dev1"):
    // the deploy step that opens the programme puts the world at the play rate, 24 —
    // the production readiness report flags any other rate, and its Rate line gates
    // `+coop/enrol`. A test world boots at rate 1, so the step is typed here, before
    // play, as the operator types it on the box.
    let out = run(&mut rig.tw, GOD, "@tacsim/time rate=24");
    assert_eq!(rig.tw.world().world_timeline().current().rate, 24, "{out}");
    sweep(&mut rig.tw, 120);
    stock(&mut rig);
    let _ = &rig.typed;

    // The character, on a gateway account of their own, in Limbo.
    let pilot = rig.tw.spawn_player("Pilot");
    if let Ok(mut map) = rig.oracle.0.lock() {
        map.insert(pilot, AccountId::mint("PilotAcct", 1).expect("an account"));
    }
    assert_eq!(location(&rig.tw, pilot), Some(LIMBO));
    let c01 = station_room(&rig.tw, "ceres_port", "room.c01_berth_gallery");
    let c02 = station_room(&rig.tw, "ceres_port", "room.c02_pressure");
    let c03 = station_room(&rig.tw, "ceres_port", "room.c03_arrivals");
    let c04 = station_room(&rig.tw, "ceres_port", "room.c04_concourse");
    let c05 = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    let c07 = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    let c08 = station_room(&rig.tw, "ceres_port", "room.c08_chandlery");

    // The IC door, the walk to the desk, the terms, the enrolment.
    walk(&mut rig, pilot, "c", c03);
    walk(&mut rig, pilot, "registry", c05);
    let drays = port_map(&rig.tw, "ceres_port").hulls.len();
    let terms = run(&mut rig.tw, pilot, "+coop");
    assert!(
        terms.contains(&format!("Drays free: {drays} of {drays}.")),
        "{terms}"
    );
    let enrolled = run(&mut rig.tw, pilot, "+coop/enrol Salt Moth");
    assert!(enrolled.contains("now the Salt Moth"), "{enrolled}");
    let ship = run(&mut rig.tw, pilot, "+ship");
    assert!(ship.contains("Salt Moth (Dray): docked "), "{ship}");
    assert!(ship.contains("Programme: 0 sales;"), "{ship}");
    let charter = tacsim::port::live_charter(rig.tw.world(), pilot).expect("a charter");
    let row: HullRow = port_map(&rig.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|row| row.hull == charter.hull.0)
        .expect("the hull's row");
    let stowage = ObjId(row.rooms[&cid("stowage")]);
    let hold = economy(&rig.tw)
        .snapshot()
        .vault_by_backing(&mush_economy::CustodyRef::SpatialInterior(stowage))
        .expect("the hold vault")
        .id;
    let me = credit_account(&rig.tw, &PrincipalId::Object(pilot));
    let stake = posted(&rig.tw, me);
    assert!(
        terms.contains(&format!("Terms: a {} stake; ", cr(stake))),
        "{terms}"
    );
    let books_before = closed_sum(&rig.tw, pilot);

    // The Exchange: the both-limits cargo.
    walk(&mut rig, pilot, "out", c03);
    walk(&mut rig, pilot, "concourse", c04);
    walk(&mut rig, pilot, "exchange", c07);
    let listed = run(&mut rig.tw, pilot, "+market/list");
    let metals_ask = listing_of(&rig.tw, "ceres_port", "freeport_exchange", "metals");
    let ice_ask = listing_of(&rig.tw, "ceres_port", "occator", "volatiles");
    assert!(listed.contains(&format!("#{metals_ask} ")), "{listed}");
    assert!(listed.contains(&format!("#{ice_ask} ")), "{listed}");
    let goods = tacsim::definitions::goods::goods_table().expect("the goods table");
    let dray = load_pack(tacsim::definitions::content::DRAY_JSON).expect("the Dray pack");
    let Some(PackEntry::Unit(spec)) = dray.entry(&cid(tacsim::definitions::content::DRAY)) else {
        panic!("the Dray unit");
    };
    let cargo = cid("cargo");
    let def = spec
        .logistics
        .as_ref()
        .and_then(|l| l.stowage.get(&cargo))
        .expect("the cargo hold");
    let compartment = tacsim::port::capacity::Compartment::from_stowage(&cargo, def);
    let (metals, ice) =
        vertex::both_limits_vertex(&goods, &compartment, &cid("metals"), &cid("volatiles"))
            .expect("an integral vertex");
    let (metals_seeded, ice_seeded) = (
        market(&rig.tw)
            .listing(metals_ask)
            .map(|l| l.quantity)
            .expect("listed"),
        market(&rig.tw)
            .listing(ice_ask)
            .map(|l| l.quantity)
            .expect("listed"),
    );
    let mut costs = buy(&mut rig, pilot, metals_ask, metals);
    costs += buy(&mut rig, pilot, ice_ask, ice);

    // The chandlery: reaction mass and provisions at exactly the planner's figures.
    walk(&mut rig, pilot, "chandlery", c08);
    let fuel_ask = listing_of(&rig.tw, "ceres_port", "occator", "propellant");
    let food_ask = listing_of(&rig.tw, "ceres_port", "coop", "provisions");
    let (fuel_seeded, food_seeded) = (
        market(&rig.tw)
            .listing(fuel_ask)
            .map(|l| l.quantity)
            .expect("listed"),
        market(&rig.tw)
            .listing(food_ask)
            .map(|l| l.quantity)
            .expect("listed"),
    );
    let (mut fuel, mut food) = (0_u64, 0_u64);
    for _ in 0..6 {
        let plan = run(&mut rig.tw, pilot, "+ship/plan stickney");
        let (burn, tank) = plan_numbers(&plan, "Burn ", "(tank ").expect(&plan);
        let (days, locker) = plan_numbers(&plan, "Stores ", "(locker ").expect(&plan);
        if tank >= burn && locker >= days {
            break;
        }
        if tank < burn {
            costs += buy(&mut rig, pilot, fuel_ask, burn - tank);
            fuel += burn - tank;
        }
        if locker < days {
            costs += buy(&mut rig, pilot, food_ask, days - locker);
            food += days - locker;
        }
    }
    let plan = run(&mut rig.tw, pilot, "+ship/plan stickney");
    let (burn, tank) = plan_numbers(&plan, "Burn ", "(tank ").expect(&plan);
    let (days, locker) = plan_numbers(&plan, "Stores ", "(locker ").expect(&plan);
    assert_eq!((tank, locker), (fuel, food), "{plan}");
    assert!(tank >= burn && locker >= days, "{plan}");
    // `+ship/hold`: the hold at both limits, the tank and the locker at what was bought.
    let held = run(&mut rig.tw, pilot, "+ship/hold");
    let logistics = spec.logistics.as_ref().expect("the Dray's logistics");
    let tank_cap = single_cap(&goods, &logistics.stowage[&cid("propellant")], "propellant");
    let locker_cap = single_cap(&goods, &logistics.stowage[&cid("provisions")], "provisions");
    let (mass, volume) = (
        def.limits[&cid("si.mass_kg")],
        def.limits[&cid("si.volume_l")],
    );
    let cell = |good: &str, qty: u64| {
        let row = goods.good(&cid(good)).expect("the good");
        format!(
            "    {}: {qty} t · {} m³",
            row.label,
            tacsim::port::capacity::format_m3(qty * row.measures[&cid("si.volume_l")])
        )
    };
    let label = |good: &str| goods.good(&cid(good)).expect("the good").label.clone();
    let want = [
        format!("Hold of the Salt Moth ({}):", row.mark),
        "  HOLD".to_owned(),
        cell("metals", metals),
        cell("volatiles", ice),
        cell("structural_alloys", 0),
        cell("industrial_parts", 0),
        format!(
            "    TOTAL: {} t · {} m³",
            tacsim::port::capacity::format_tonnes(mass),
            tacsim::port::capacity::format_m3(volume)
        ),
        format!("    LIMIT: {} t · {} m³", mass / 1_000, volume / 1_000),
        "    FULL: mass and packed volume".to_owned(),
        format!("  TANKAGE {}, {fuel} of {tank_cap} t", label("propellant")),
        format!(
            "  STORES {}, {food} of {locker_cap} crew-days",
            label("provisions")
        ),
    ];
    assert!(
        held.contains(&want.join("\n")),
        "{held}\nwanted:\n{}",
        want.join("\n")
    );
    let credit = run(&mut rig.tw, pilot, "+credit");
    let after_buying = stake - costs;
    assert!(
        credit.contains(&format!("{} available", cr(after_buying))),
        "{credit}"
    );

    // Every tonne aboard traces to a seeded Origin receipt.
    let receipts: Vec<(StockIssueId, tacsim::port::StockReceipt)> = {
        let handles = rig
            .tw
            .world()
            .module_state::<PortHandles>()
            .cloned()
            .expect("port handles");
        let mut out = Vec::new();
        for map in [port_map(&rig.tw, "ceres_port")] {
            for row in map.vendors.values() {
                for stock in &row.stock {
                    let id = StockIssueId::parse(&stock.id).expect("an issue id");
                    let mut txn = rig
                        .tw
                        .engine_mut()
                        .action_transaction(&handles.families.port_stock)
                        .expect("a transaction");
                    if let Some(receipt) = txn
                        .get(&handles.families.port_stock, &id)
                        .expect("reads")
                        .cloned()
                    {
                        out.push((id, receipt));
                    }
                }
            }
        }
        out
    };
    let families = families(&rig.tw);
    for good in ["metals", "volatiles", "propellant", "provisions"] {
        let mut txn = rig
            .tw
            .engine_mut()
            .action_transaction(&families.pools)
            .expect("a transaction");
        let (_, hold_root) = families
            .tranche_for(&mut txn, EndpointRef(hold), &cid(good))
            .expect("reads")
            .unwrap_or_else(|| panic!("the hold's {good} root"));
        let traced = hold_root.inherited().iter().any(|parent| {
            let Ok(Some(source)) = txn.get(&families.tranches, parent) else {
                return false;
            };
            receipts
                .iter()
                .any(|(_, r)| source.absorbed().contains(&LotId(r.lot)))
        });
        assert!(
            traced,
            "the hold's {good} traces to a seeded Origin receipt"
        );
    }

    // The gallery, aboard, the plan and the undock. The walk takes a couple of real
    // minutes, in which the market's sweep sees the Draws' receipts (a hull waits for
    // promised goods until then: `GOODS_COMING`).
    sweep(&mut rig.tw, 61);
    sweep(&mut rig.tw, 61);
    // The one staff act during play: the lane flown at 3600 (after the enrolment,
    // whose readiness gate reads the production Rate line).
    let out = run(&mut rig.tw, GOD, "@tacsim/time rate=3600");
    assert_eq!(
        rig.tw.world().world_timeline().current().rate,
        3_600,
        "{out}"
    );
    walk(&mut rig, pilot, "exchange", c07);
    walk(&mut rig, pilot, "concourse", c04);
    walk(&mut rig, pilot, "arrivals", c03);
    walk(&mut rig, pilot, "lobby", c02);
    walk(&mut rig, pilot, "gallery", c01);
    let boarded = run(&mut rig.tw, pilot, "+ship/board");
    assert!(
        boarded.contains("You cycle the lock into the Salt Moth."),
        "{boarded}"
    );
    let plan = run(&mut rig.tw, pilot, "+ship/plan stickney");
    assert!(plan.starts_with("Plan for Stickney Yards"), "{plan}");
    let filed = run(&mut rig.tw, pilot, "+ship/undock stickney");
    assert!(
        filed.contains("Clearance filed for Stickney Yards"),
        "{filed}"
    );
    let burn_leg = drain(&mut rig.tw);
    assert_eq!(burn_leg.len(), 1, "{filed}");
    land_all(&mut rig, &burn_leg);

    // The flight, heard in the Crew Compartment.
    let crew = ObjId(row.rooms[&cid("crew")]);
    let airlock = ObjId(row.rooms[&cid("airlock")]);
    let _ = run(&mut rig.tw, pilot, "in");
    walk(&mut rig, pilot, "crew", crew);
    let _ = fire_quietly(&mut rig);
    let hull = ObjId(row.hull);
    let handles = rig
        .tw
        .world()
        .module_state::<PortHandles>()
        .cloned()
        .expect("port handles");
    let ticket = {
        let mut txn = rig
            .tw
            .engine_mut()
            .action_transaction(&handles.families.tickets)
            .expect("a transaction");
        txn.get(
            &handles.families.tickets,
            &tacsim::port::TicketId { hull, seq: 1 },
        )
        .expect("reads")
        .cloned()
        .expect("the ticket")
    };
    // The burn and the crew-days left the hold at departure: bought − burned.
    assert_eq!(
        pool(&mut rig.tw, hold, "propellant"),
        fuel - ticket.plan.burn_t,
        "the tank after the burn"
    );
    assert_eq!(
        pool(&mut rig.tw, hold, "provisions"),
        food - u64::from(ticket.plan.crew_days),
        "the locker after the passage's crew-days"
    );
    let mut heard = String::new();
    let boundary = ticket.plan.depart_ic + i64::from(ticket.plan.legs[0].secs);
    let arrival = ticket.plan.depart_ic + ticket.plan.total_s;
    for target in [boundary, arrival, arrival + 3_600] {
        let events = to_ic(&mut rig, target);
        heard.push_str(&said(&rig.tw, &events, pilot));
        heard.push('\n');
    }
    assert!(heard.contains("Kirkwood Depot slides past"), "{heard}");
    assert!(
        heard.contains("Salt Moth docks at Stickney Yards."),
        "{heard}"
    );

    // Ashore, and the walk to Kasei's Receiving Bay by the pack's own exit words.
    let p01 = station_room(&rig.tw, "stickney_port", "room.p01_berth_gallery");
    let p02 = station_room(&rig.tw, "stickney_port", "room.p02_pressure");
    let p03 = station_room(&rig.tw, "stickney_port", "room.p03_customs");
    let p04 = station_room(&rig.tw, "stickney_port", "room.p04_concourse");
    let p07 = station_room(&rig.tw, "stickney_port", "room.p07_receiving");
    let p08 = station_room(&rig.tw, "stickney_port", "room.p08_exchange");
    let passage = ObjId(row.rooms[&cid("passage")]);
    walk(&mut rig, pilot, "passage", passage);
    walk(&mut rig, pilot, "airlock", airlock);
    let ashore = run(&mut rig.tw, pilot, "+ship/leave");
    assert_eq!(location(&rig.tw, pilot), Some(p01), "{ashore}");
    walk(&mut rig, pilot, "lobby", p02);
    walk(&mut rig, pilot, "customs", p03);
    walk(&mut rig, pilot, "concourse", p04);
    walk(&mut rig, pilot, "exchange", p08);
    walk(&mut rig, pilot, "down", p07);

    // Kasei's two bids, and both goods sold into them.
    let kasei = port_map(&rig.tw, "stickney_port").vendors[&cid("kasei")].clone();
    let metals_bid = ListingId(kasei.bids[&cid("metals")].listing);
    let ice_bid = ListingId(kasei.bids[&cid("volatiles")].listing);
    let listed = run(&mut rig.tw, pilot, "+market/list");
    assert!(listed.contains(&format!("#{metals_bid} ")), "{listed}");
    assert!(listed.contains(&format!("#{ice_bid} ")), "{listed}");
    let (metals_intake, ice_intake) = (
        market(&rig.tw)
            .listing(metals_bid)
            .map(|l| l.quantity)
            .expect("bid"),
        market(&rig.tw)
            .listing(ice_bid)
            .map(|l| l.quantity)
            .expect("bid"),
    );
    let permille = store(&rig.tw)
        .enrolment_of(pilot)
        .map(|e| e.share_permille)
        .expect("the enrolment");
    let mut contracts: Vec<(ContractId, i64)> = Vec::new();
    let (mut margins, mut shares_billed) = (0_i64, 0_i64);
    for (bid, ask, good, qty) in [
        (metals_bid, metals_ask, "metals", metals),
        (ice_bid, ice_ask, "volatiles", ice),
    ] {
        let price = market(&rig.tw)
            .listing(bid)
            .map(|l| l.price.minor())
            .expect("bid");
        // The reference is the origin ask's live price (decision 8).
        let reference = market(&rig.tw)
            .listing(ask)
            .map(|l| l.price.minor())
            .expect("the origin ask stands");
        let gross = price * i64::try_from(qty).expect("qty");
        let (margin, share) = margin_and_share(price, reference, qty, permille);
        margins += margin;
        shares_billed += share;
        let line = format!(
            "Sold {} t of {} to Kasei Metals at {}/t: {}; Co-op share {}; you receive {} when \
             the goods are in. Jonas Achterberg-Nkemelu countersigns.",
            grouped(qty),
            label(good),
            cr(price),
            cr(gross),
            cr(share),
            cr(gross - share)
        );
        let sold = run(&mut rig.tw, pilot, &format!("+market/sell {bid}={qty}"));
        assert!(sold.lines().any(|l| l == line), "{sold}\nwanted: {line}");
        let q = market(&rig.tw)
            .quotes_of_seller(&PrincipalId::Object(pilot))
            .last()
            .cloned()
            .expect("the sale");
        let leg = drain(&mut rig.tw);
        assert_eq!(leg.len(), 1, "{sold}");
        land_all(&mut rig, &leg);
        contracts.push((
            q.contract.expect("contract"),
            price * i64::try_from(qty).expect("qty"),
        ));
    }
    for _ in 0..12 {
        sweep(&mut rig.tw, 31);
    }

    // Journals: pilot + Trust = Kasei's payment on each sale.
    let kasei_account = mush_economy::AccountId(
        port_map(&rig.tw, "stickney_port").orgs[&cid("kasei")]
            .account
            .expect("Kasei's account"),
    );
    let trust = mush_economy::AccountId(
        port_map(&rig.tw, "ceres_port").orgs[&cid("coop_trust")]
            .account
            .expect("the Trust's account"),
    );
    let mut nets = 0_i64;
    let mut shares = 0_i64;
    for (contract, gross) in &contracts {
        let lines = settlement(&rig.tw, *contract);
        let paid = -lines.get(&kasei_account).copied().unwrap_or(0);
        let net = lines.get(&me).copied().unwrap_or(0);
        let share = lines.get(&trust).copied().unwrap_or(0);
        assert_eq!(paid, *gross);
        assert_eq!(net + share, paid, "pilot + Trust = Kasei's payment");
        assert_eq!(lines.values().sum::<i64>(), 0);
        nets += net;
        shares += share;
    }
    let credit = run(&mut rig.tw, pilot, "+credit");
    assert!(
        credit.contains(&format!("{} available", cr(stake - costs + nets))),
        "{credit}"
    );
    assert_eq!(
        shares, shares_billed,
        "the Trust was paid what each line billed"
    );
    let statement = run(&mut rig.tw, pilot, "+coop/statement");
    assert!(
        statement.contains(&format!(
            "  Sales: 2; margin {}; Co-op share paid {}.",
            cr(margins),
            cr(shares)
        )),
        "{statement}"
    );
    assert_eq!(
        closed_sum(&rig.tw, pilot),
        books_before,
        "money is conserved"
    );

    // Seeded − sold = remaining on every touched listing and pool; Kasei's pools are
    // the delivered intake.
    let exchange_store =
        VaultId(port_map(&rig.tw, "ceres_port").vendors[&cid("freeport_exchange")].vault);
    let occator_store = VaultId(port_map(&rig.tw, "ceres_port").vendors[&cid("occator")].vault);
    let coop_store = VaultId(port_map(&rig.tw, "ceres_port").vendors[&cid("coop")].vault);
    for (listing, seeded, bought) in [
        (metals_ask, metals_seeded, metals),
        (ice_ask, ice_seeded, ice),
        (fuel_ask, fuel_seeded, fuel),
        (food_ask, food_seeded, food),
    ] {
        assert_eq!(
            market(&rig.tw).listing(listing).map(|l| l.quantity),
            Some(seeded - bought)
        );
    }
    assert_eq!(
        pool(&mut rig.tw, exchange_store, "metals"),
        metals_seeded - metals
    );
    assert_eq!(
        pool(&mut rig.tw, occator_store, "volatiles"),
        ice_seeded - ice
    );
    assert_eq!(
        pool(&mut rig.tw, occator_store, "propellant"),
        fuel_seeded - fuel
    );
    assert_eq!(
        pool(&mut rig.tw, coop_store, "provisions"),
        food_seeded - food
    );
    assert_eq!(bid_left(&rig.tw, metals_bid), Some(metals_intake - metals));
    assert_eq!(bid_left(&rig.tw, ice_bid), Some(ice_intake - ice));
    let kasei_store = VaultId(kasei.vault);
    assert_eq!(pool(&mut rig.tw, kasei_store, "metals"), metals);
    assert_eq!(pool(&mut rig.tw, kasei_store, "volatiles"), ice);
    assert_eq!(pool(&mut rig.tw, hold, "metals"), 0);
    assert_eq!(pool(&mut rig.tw, hold, "volatiles"), 0);
    let pages: Vec<_> = economy(&rig.tw).escalations().collect();
    assert!(pages.is_empty(), "no escalation of any kind: {pages:?}");
    assert!(rig.tw.world().refused_namespace_writes().is_empty());
}

/// `{n}` with thousands grouped (`1,234`), the board's and the sale line's render.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The Co-op's `(basis, share)` of one sale, computed here from the posted prices:
/// the margin over the reference ask, and its permille floored (D-18-24, decision 9).
fn margin_and_share(price: i64, reference: i64, qty: u64, permille: u32) -> (i64, i64) {
    let basis = (price - reference).max(0) * i64::try_from(qty).expect("qty");
    (basis, basis * i64::from(permille) / 1_000)
}

/// What a single-good compartment can hold of `good`: the least of its limits over
/// the good's per-unit measures.
fn single_cap(
    goods: &tacsim::definitions::goods::GoodsTable,
    stowage: &tacsim::definitions::logistics::StowageDef,
    good: &str,
) -> u64 {
    let row = goods.good(&cid(good)).expect("the good");
    stowage
        .limits
        .iter()
        .filter_map(|(measure, limit)| row.measures.get(measure).map(|per| limit / per))
        .min()
        .expect("a limited measure")
}

fn bid_left(tw: &TestWorld, id: ListingId) -> Option<u64> {
    market(tw).listing(id).map(|l| l.quantity)
}
