//! **Doc 18 cut IC10 acceptance — selling and the Co-op share: the loop closes
//! interactively** (doc 18 §4.1 "The share", §4.3 "Bids that take real lots", §6;
//! D-18-24, D-18-26, D-18-27; lead rulings and decisions 1–24 of the IC10 brief).
//!
//! A chartered pilot whose Dray is docked at Stickney stands at Kasei's counter (P07)
//! and types `+market/sell <bid listing>=<qty>`. The market's lot sell arm offers the
//! sale contract with the pilot as issuer, accepts it in the verb as the pilot, and
//! Kasei accepts through the port's party caps, escrowing its payment; the one
//! `deliver` leg moves the tonnes from the hold into Kasei's storefront through
//! inventory's own `transfer_marked`, and the contract settles only once the
//! `inventory.transferred` fact is true — money never moves before the matter. The
//! proceeds policy (`CoopProceeds`) pays the Co-op First Captain Trust 250‰ of the
//! margin over the Freeport Exchange's live metals ask, prints the sale line naming
//! Kasei's signatory, and folds the sale into the enrolment once it settles.
//!
//! Composed on the composition root (`server_modules_with_tacsim` and the wired
//! test-world layer with game content: `commerce.rs`'s `Roster`, with an operator
//! seed for the lane variants this cut needs — no pack version moves). The flight
//! is `voyage.rs`'s (rate 3600, the burn and the crew-days read off the planner's
//! own `+ship/plan`, never literals). After `+ship/leave` the pilot walks P01 to P07
//! through the shipped Stickney exits (doc 18 cut IC14 row 65); this file is not the
//! story test (`ic_slice_loop.rs` is).

mod support;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use mush_core::{
    AccountId, AccountIdentity, ActionStateBootRecord, Audience, AuthoritativeSubmit, CanonicalId,
    CommitToken, GOD, IdempotencyKey, IdempotencyScope, LIMBO, ModuleWrite, ObjId,
    OfferExecutionContext, OfferSubmission, RequestUid, ScopedIdempotencyKey,
};
use mush_economy::{
    ContractId, ContractState, EconomyConfig, EconomyStore, EscalationKind, VaultId,
};
use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
use mush_market::{
    ListingId, MarketStore, Presence, ProceedsPolicy, QuoteId, SaleTerms, VenueGate,
};
use mush_org::PrincipalId;
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops_into};
use smol_str::SmolStr;
use tacsim::config::PackSource;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK, LANE_VERSION};
use tacsim::pack::{PackEntry, load_pack};
use tacsim::port::apply::room_by_key;
use tacsim::port::voyage::{GOODS_LEAVING, Voyage, VoyageState};
use tacsim::port::{
    CoopProceeds, HOLD_ELSEWHERE, HullRow, NOT_IN_PORT, PortDef, PortHandles, PortMap,
    PortMapState, PortVenueGate, SALE_NOT_PILOT, Ticket, TicketId, room_key,
};
use tacsim::{TacsimConfig, TacsimStore};

use support::ground::{UNSIGNED, sign};

// ---------------------------------------------------------------------------
// The rig (commerce.rs's roster, voyage.rs's flight)
// ---------------------------------------------------------------------------

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

fn lane_id() -> CanonicalId {
    cid(LANE_PACK)
}

/// `commerce.rs`'s roster: the shipped lane, or an operator seed in its place, an
/// optional shelf for `@atlas/pack`, and an optional extra lane version the layer
/// loads beside it.
#[derive(Clone, Default)]
struct Roster {
    seed_lane: Option<String>,
    shelf: Option<Vec<mush_content::Pack>>,
    extra_lane: Option<String>,
}

impl Roster {
    fn bare() -> Roster {
        Roster::default()
    }

    fn seeded(lane: String) -> Roster {
        Roster {
            seed_lane: Some(lane),
            ..Roster::default()
        }
    }

    fn modules(&self) -> Vec<Box<dyn mush_core::GameModule>> {
        use tacsim::composition as comp;

        let llm = Arc::new(
            mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
                .expect("a disabled LLM service composes"),
        );
        let onboarding = mush_core::OnboardingConfig::default();
        let game = tacsim::GameEconomyConfig::default();
        let mut cfg = TacsimConfig {
            enabled: true,
            ..TacsimConfig::default()
        };
        let game_content = self.seed_lane.is_none();
        if let Some(lane) = &self.extra_lane {
            cfg.seeds.push(PackSource {
                name: "ops/lane_next.json".to_owned(),
                bytes: lane.clone(),
            });
        }
        if let Some(lane) = &self.seed_lane {
            cfg.seeds.push(PackSource {
                name: "ops/lane.json".to_owned(),
                bytes: lane.clone(),
            });
        }
        let (mut mods, inventory) = comp::server_modules_with_tacsim(
            mush_weather::WeatherConfig::default(),
            EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &onboarding,
            &game,
            comp::recovery_policy(&cfg.recovery),
        );
        if let Some(shelf) = &self.shelf {
            let at = mods
                .iter()
                .position(|m| m.name() == mush_atlas::NS)
                .expect("the atlas composes");
            let mut full = tacsim::definitions::content::station_packs().expect("station packs");
            full.extend(shelf.iter().cloned());
            mods[at] = Box::new(
                mush_atlas::AtlasModule::with_config(mush_atlas::AtlasConfig::default())
                    .with_shelf(full),
            );
        }
        mods.push(comp::worldgen_module_with(
            Arc::clone(&llm),
            mush_worldgen::WorldgenConfig::default(),
        ));
        mods.push(comp::npc_module_with(
            comp::npc_agent_adapter(),
            &mush_config::config::NpcCfg::default(),
        ));
        mods.push(comp::tacsim_test_world_module_wired(
            cfg,
            game_content,
            inventory,
        ));
        match &self.seed_lane {
            None => {
                mods.push(
                    comp::content_module_with(true, true).expect("the content module composes"),
                );
            }
            Some(lane) => {
                let packs = support::boot_packs_with_lane(
                    mush_content::load_pack(lane).expect("the seed lane verifies"),
                );
                let mut appliers = mush_content::Appliers::new();
                mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
                mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
                tacsim::register_appliers(&mut appliers).expect("tacsim appliers");
                mods.push(Box::new(
                    mush_content::ContentModule::new()
                        .with_appliers(appliers)
                        .with_boot_packs(packs),
                ));
            }
        }
        mods
    }
}

/// A fixed gateway-account oracle (`match_lifecycle.rs`'s precedent).
#[derive(Default)]
struct MapOracle(Mutex<BTreeMap<ObjId, AccountId>>);

impl AccountIdentity for MapOracle {
    fn account_of(&self, character: ObjId) -> Option<AccountId> {
        self.0.lock().ok()?.get(&character).copied()
    }
}

/// A booted world, its oracle and the cumulative store.
struct Rig {
    tw: TestWorld,
    oracle: Arc<MapOracle>,
    roster: Roster,
    store: HashMap<SmolStr, mush_core::ModuleBootData>,
    typed: BTreeMap<(SmolStr, SmolStr), serde_json::Value>,
}

fn no_refused_writes(tw: &TestWorld) {
    let refused = tw.world().refused_namespace_writes();
    assert!(refused.is_empty(), "refused namespace writes: {refused:?}");
}

fn install_oracle(tw: &mut TestWorld, oracle: &Arc<MapOracle>) {
    *tw.world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .expect("the simulation is composed") = mush_sim::AccountOracle::new(oracle.clone());
}

fn with_offer_keys(tw: &mut TestWorld) {
    tw.engine_mut().set_offer_keys(
        mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(1, [0x5a; 32]).expect("an offer secret"),
            [],
        )
        .expect("an offer key ring"),
    );
}

/// Boot `roster`, settle the capital and stock both ports.
fn stocked_with(roster: Roster) -> Rig {
    let mut tw = TestWorld::with_modules(roster.modules());
    no_refused_writes(&tw);
    with_offer_keys(&mut tw);
    let oracle = Arc::new(MapOracle::default());
    install_oracle(&mut tw, &oracle);
    let mut rig = Rig {
        tw,
        oracle,
        roster,
        store: HashMap::new(),
        typed: BTreeMap::new(),
    };
    sweep(&mut rig.tw, 120);
    stock(&mut rig);
    rig
}

fn stocked() -> Rig {
    stocked_with(Roster::bare())
}

/// Fold every write since the first boot and reboot onto the same roster.
fn reboot(rig: Rig) -> Rig {
    let Rig {
        mut tw,
        oracle,
        roster,
        mut store,
        typed,
    } = rig;
    fold_module_ops_into(&mut store, tw.take_module_ops());
    let mut tw = tw
        .reboot_into(roster.modules(), store.clone())
        .expect("the reboot boots");
    no_refused_writes(&tw);
    tw.engine_mut()
        .hydrate_action_state(
            typed
                .iter()
                .map(|((ns, key), value)| {
                    ActionStateBootRecord::new(ns.clone(), key.clone(), value.clone())
                })
                .collect::<Vec<_>>(),
        )
        .expect("the typed state hydrates");
    with_offer_keys(&mut tw);
    install_oracle(&mut tw, &oracle);
    Rig {
        tw,
        oracle,
        roster,
        store,
        typed,
    }
}

/// Everything due `secs` on.
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
    match store(tw).port_map(&lane_id(), &cid(entry)) {
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

fn c01(tw: &TestWorld) -> ObjId {
    station_room(tw, "ceres_port", "room.c01_berth_gallery")
}

fn c07(tw: &TestWorld) -> ObjId {
    station_room(tw, "ceres_port", "room.c07_exchange")
}

fn p07(tw: &TestWorld) -> ObjId {
    station_room(tw, "stickney_port", "room.p07_receiving")
}

fn location(tw: &TestWorld, obj: ObjId) -> Option<ObjId> {
    tw.world().get(obj).and_then(|o| o.location)
}

fn teleport(tw: &mut TestWorld, who: ObjId, room: ObjId) {
    let _ = run(tw, GOD, &format!("@teleport #{}=#{}", who.0, room.0));
    assert_eq!(location(tw, who), Some(room), "the teleport landed");
}

/// A fresh request/commit token per landing, across the process.
fn next_token() -> [u8; 16] {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut bytes = [0x5a_u8; 16];
    bytes[..4].copy_from_slice(&n.to_be_bytes());
    bytes
}

/// Drive one queued submission the way the world task does, keeping its typed
/// post-images for a reboot.
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
            .map_err(|error| format!("resolve: {error:?}"))?;
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
        .map_err(|error| format!("prepare: {error:?}"))?;
    let (ops, install) = prepared.into_parts();
    tw.engine_mut()
        .install_prepared_action(install)
        .map_err(|error| format!("{error:?}"))?;
    for op in ops {
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                rig.typed.insert((op.ns, key), value);
            }
            ModuleWrite::Del { key } => {
                rig.typed.remove(&(op.ns, key));
            }
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    Ok(())
}

/// Every queued submission of the port's lanes, dropping other modules'; asserts
/// none kept of tacsim's is GOD's.
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

/// One port-tick fire (6 s on), its submissions drained (not landed).
fn fire(rig: &mut Rig) -> (Vec<(Audience, OutputEvent)>, Vec<AuthoritativeSubmit>) {
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    (events, queued)
}

fn fire_quietly(rig: &mut Rig) -> Vec<(Audience, OutputEvent)> {
    let (events, queued) = fire(rig);
    land_quietly(rig, &queued);
    events
}

fn status(tw: &mut TestWorld) -> String {
    run(tw, GOD, "@tacsim/port status")
}

/// **Fire and land the port tick until both ports are stocked and listed.**
fn stock(rig: &mut Rig) {
    for _ in 0..40 {
        let out = status(&mut rig.tw);
        let gated: Vec<&str> = out
            .lines()
            // Doc 18 cut IC12: the gate is every report line but the sky (D-18-50).
            // doc 20 SM.1: the geology line is reported and never gated
            .filter(|line| {
                line.ends_with(']') && !line.starts_with("sky: ") && !line.starts_with("geology: ")
            })
            .collect();
        if gated.len() > 1 && gated.iter().all(|line| line.ends_with("[ready]")) {
            return;
        }
        let (_, queued) = fire(rig);
        land_all(rig, &queued);
    }
    panic!("the ports were not stocked within 40 fires");
}

/// Read one typed value through an owner-stamped transaction.
fn typed<K, V>(
    tw: &mut TestWorld,
    handle: &mush_core::StateFamilyHandle<K, V>,
    key: &K,
) -> Option<V>
where
    K: Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    let mut txn = tw
        .engine_mut()
        .action_transaction(handle)
        .expect("a transaction");
    txn.get(handle, key).expect("the family reads").cloned()
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
    typed(tw, &families.pools, &key).map_or(0, |scalar| {
        u64::try_from(scalar.current()).expect("a non-negative pool")
    })
}

/// A pilot on a gateway account of their own, standing at the desk.
fn pilot(rig: &mut Rig, who: &str, seed: i64) -> ObjId {
    let pilot = rig.tw.spawn_player(who);
    if let Ok(mut map) = rig.oracle.0.lock() {
        map.insert(
            pilot,
            AccountId::mint(&format!("{who}Acct"), seed).expect("an account id"),
        );
    }
    let desk = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, pilot, desk);
    pilot
}

/// An enrolled pilot whose hull is named `name`, standing at C07.
fn chartered(rig: &mut Rig, who: &str, seed: i64, name: &str) -> (ObjId, HullRow) {
    let pilot = pilot(rig, who, seed);
    let out = run(&mut rig.tw, pilot, &format!("+coop/enrol {name}"));
    assert!(out.contains(&format!("now the {name}")), "{out}");
    let charter = tacsim::port::live_charter(rig.tw.world(), pilot).expect("a charter");
    let row = port_map(&rig.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|row| row.hull == charter.hull.0)
        .expect("the hull's row");
    let exchange = c07(&rig.tw);
    teleport(&mut rig.tw, pilot, exchange);
    (pilot, row)
}

/// The hold vault of a chartered hull.
fn hold_vault(tw: &TestWorld, row: &HullRow) -> VaultId {
    let stowage = ObjId(row.rooms[&cid("stowage")]);
    economy(tw)
        .snapshot()
        .vault_by_backing(&mush_economy::CustodyRef::SpatialInterior(stowage))
        .expect("the hold vault")
        .id
}

fn room_of(row: &HullRow, id: &str) -> ObjId {
    ObjId(row.rooms[&cid(id)])
}

/// Seed `qty` of `good` into `vault`'s pool through `lot`: one Origin lot,
/// dissolved, so the pool carries a lineage root.
fn seed_pool(rig: &mut Rig, vault: VaultId, good: &str, qty: u64, lot: LotId) {
    let endpoint = EndpointRef(vault);
    let good = cid(good);
    let families = families(&rig.tw);
    for step in 0..2 {
        let engine = rig.tw.engine_mut();
        let prepared = {
            let mut txn = engine
                .action_transaction(&families.pools)
                .expect("inventory transaction");
            if step == 0 {
                families
                    .mint_origin(
                        &mut txn,
                        endpoint,
                        &good,
                        qty,
                        Grade::new(2).expect("grade"),
                        mush_economy::ExternalRef {
                            provider: "test".into(),
                            kind: "seed".into(),
                            id: lot.0.to_string().into(),
                            revision: None,
                        },
                        lot,
                    )
                    .map(|_| ())
                    .expect("seed mints one origin lot");
            } else {
                families
                    .dissolve(&mut txn, endpoint, lot)
                    .map(|_| ())
                    .expect("the seed lot dissolves");
            }
            txn.prepare().expect("prepares")
        };
        let (ops, install) = prepared.into_parts();
        engine.install_prepared_action(install).expect("installs");
        for op in ops {
            match op.op {
                ModuleWrite::JsonSet { key, value } => {
                    rig.typed.insert((op.ns, key), value);
                }
                ModuleWrite::Del { key } => {
                    rig.typed.remove(&(op.ns, key));
                }
                ModuleWrite::StreamAppend { .. } => {}
            }
        }
    }
}

/// The two numbers a `+ship/plan` line names: `Burn B t (tank T t; …)` → `(B, T)`,
/// `Stores C crew-days (locker L; …)` → `(C, L)`.
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

/// **Provision the hold for `dest` from the planner's own figures** (never
/// literals): read `+ship/plan <dest>`, seed whatever reaction mass and crew-days it
/// says are short, and read it again until the tank and the locker cover the burn
/// and the stores (the burn grows with the mass aboard, so it converges in a round
/// or two). Lot ids start at `lot`.
fn provision(rig: &mut Rig, pilot: ObjId, row: &HullRow, dest: &str, mut lot: u64) {
    let vault = hold_vault(&rig.tw, row);
    for _ in 0..6 {
        let plan = run(&mut rig.tw, pilot, &format!("+ship/plan {dest}"));
        let (burn, tank) = plan_numbers(&plan, "Burn ", "(tank ").expect(&plan);
        let (crew, locker) = plan_numbers(&plan, "Stores ", "(locker ").expect(&plan);
        if tank >= burn && locker >= crew {
            return;
        }
        if tank < burn {
            seed_pool(rig, vault, "propellant", burn - tank, LotId(lot));
            lot += 1;
        }
        if locker < crew {
            seed_pool(rig, vault, "provisions", crew - locker, LotId(lot));
            lot += 1;
        }
    }
    panic!("the planner's burn and stores did not converge");
}

fn world_rate(tw: &TestWorld) -> u32 {
    tw.world().world_timeline().current().rate
}

fn set_rate(tw: &mut TestWorld, rate: u32) {
    if world_rate(tw) != rate {
        let out = run(tw, GOD, &format!("@tacsim/time rate={rate}"));
        assert_eq!(world_rate(tw), rate, "{out}");
    }
}

fn ic_now(tw: &TestWorld) -> i64 {
    tw.world().world_ic_now().at
}

/// Sweep to the first real second whose IC instant is at or past `target`, landing
/// the port's background submissions; the events.
fn to_ic(rig: &mut Rig, target: i64) -> Vec<(Audience, OutputEvent)> {
    let rate = i64::from(world_rate(&rig.tw).max(1));
    let ahead = target.saturating_sub(ic_now(&rig.tw)).max(0);
    let real = (ahead + rate - 1) / rate;
    let t = rig.tw.world().now().saturating_add(real);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    land_quietly(rig, &queued);
    events
}

fn voyage(tw: &TestWorld, hull: ObjId) -> Option<Voyage> {
    match store(tw).voyage_state(hull) {
        VoyageState::Present(voyage) => Some(voyage.clone()),
        VoyageState::Absent => None,
        VoyageState::Unreadable => panic!("the sailing plan is unreadable"),
    }
}

fn handles(tw: &TestWorld) -> PortHandles {
    tw.world()
        .module_state::<PortHandles>()
        .cloned()
        .expect("port handles")
}

fn ticket(tw: &mut TestWorld, hull: ObjId, seq: u32) -> Option<Ticket> {
    let h = handles(tw);
    typed(tw, &h.families.tickets, &TicketId { hull, seq })
}

/// Board from C01 (a `+ship/board` from the gallery cycles the lock into the
/// Airlock).
fn board(rig: &mut Rig, pilot: ObjId, name: &str) {
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, pilot, gallery);
    let out = run(&mut rig.tw, pilot, "+ship/board");
    assert!(
        out.contains(&format!("You cycle the lock into the {name}.")),
        "{out}"
    );
}

/// **Fly every `(pilot, row, name)` to Stickney at rate 3600** and put each pilot
/// ashore in P07 (by `teleport`, after `+ship/leave` from the Airlock). The world
/// rate returns to 24 once docked.
fn fly_to_stickney(rig: &mut Rig, crews: &[(ObjId, HullRow, &str)]) {
    set_rate(&mut rig.tw, 3_600);
    let mut arrival = 0_i64;
    for (pilot, row, name) in crews {
        board(rig, *pilot, name);
        let out = run(&mut rig.tw, *pilot, "+ship/undock stickney");
        assert!(out.contains("Clearance filed for Stickney Yards"), "{out}");
        let queued = drain(&mut rig.tw);
        assert_eq!(queued.len(), 1, "one burn: {out}");
        land_all(rig, &queued);
        let _ = row;
    }
    let _ = fire_quietly(rig);
    for (_, row, _) in crews {
        let t = ticket(&mut rig.tw, ObjId(row.hull), 1).expect("a ticket");
        arrival = arrival.max(t.plan.depart_ic + t.plan.total_s);
    }
    let _ = to_ic(rig, arrival - 3_600);
    let _ = to_ic(rig, arrival);
    let _ = to_ic(rig, arrival + 3_600);
    set_rate(&mut rig.tw, 24);
    let receiving = p07(&rig.tw);
    for (pilot, row, _) in crews {
        match tacsim::port::ship::berth_of(rig.tw.world(), ObjId(row.hull)) {
            tacsim::port::ship::Berth::Docked { def, .. } => {
                assert_eq!(def.venue.as_str(), "stickney");
            }
            other => panic!("the hull did not dock at Stickney: {other:?}"),
        }
        assert_eq!(location(&rig.tw, *pilot), Some(room_of(row, "airlock")));
        let out = run(&mut rig.tw, *pilot, "+ship/leave");
        assert!(out.contains("You cycle the lock out into"), "{out}");
        // Doc 18 cut IC14 row 65: walked through the shipped Stickney exits by doc 18
        // §6's words, P01 to P07, never teleported; each step's room asserted.
        for (word, local) in [
            ("lobby", "room.p02_pressure"),
            ("customs", "room.p03_customs"),
            ("concourse", "room.p04_concourse"),
            ("exchange", "room.p08_exchange"),
            ("down", "room.p07_receiving"),
        ] {
            let out = run(&mut rig.tw, *pilot, word);
            assert_eq!(
                location(&rig.tw, *pilot),
                Some(station_room(&rig.tw, "stickney_port", local)),
                "`{word}`: {out}"
            );
        }
        assert_eq!(location(&rig.tw, *pilot), Some(receiving));
    }
}

/// **A chartered pilot standing in P07 with the hull docked at Stickney**, carrying
/// `cargo` (each good seeded with its own rooted lot) and provisioned for the passage
/// from the planner's figures.
fn docked_at_stickney(rig: &mut Rig, cargo: &[(&str, u64)]) -> (ObjId, HullRow) {
    let (pilot, row) = chartered(rig, "Pilot", 1, "Salt Moth");
    let vault = hold_vault(&rig.tw, &row);
    for (n, (good, qty)) in cargo.iter().enumerate() {
        seed_pool(rig, vault, good, *qty, LotId(901 + n as u64));
    }
    provision(rig, pilot, &row, "stickney", 910);
    fly_to_stickney(rig, &[(pilot, row.clone(), "Salt Moth")]);
    (pilot, row)
}

/// The lane's bytes as JSON, edited by `edit`, at `version`, re-signed.
fn lane_variant(version: u32, edit: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut doc: serde_json::Value = serde_json::from_str(LANE_JSON).expect("the lane is JSON");
    doc["manifest"]["version"] = serde_json::json!(version);
    doc["manifest"]["digest"] = serde_json::json!(UNSIGNED);
    edit(&mut doc);
    sign(&serde_json::to_string_pretty(&doc).expect("serializes"))
}

/// The Stickney Kasei vendor's JSON in a lane document.
fn kasei(doc: &mut serde_json::Value) -> &mut serde_json::Value {
    let vendors = doc["entries"]["stickney_port"]["def"]["vendors"]
        .as_array_mut()
        .expect("vendors");
    vendors
        .iter_mut()
        .find(|v| v["org"] == "kasei")
        .expect("kasei")
}

/// Kasei's metals bid's JSON in a lane document.
fn kasei_metals_bid(doc: &mut serde_json::Value) -> &mut serde_json::Value {
    kasei(doc)["bids"]
        .as_array_mut()
        .expect("bids")
        .iter_mut()
        .find(|b| b["good"] == "metals")
        .expect("the metals bid")
}

/// `@atlas/pack plan` then `@atlas/pack apply <fingerprint>` on the lane.
fn plan_and_apply(tw: &mut TestWorld) -> (String, String) {
    let plan = run(tw, GOD, &format!("@atlas/pack plan {LANE_PACK}"));
    let fingerprint = plan
        .split("@atlas/pack apply ")
        .nth(1)
        .map(|rest| rest.split_whitespace().next().unwrap_or("").to_owned())
        .unwrap_or_default();
    let apply = run(tw, GOD, &format!("@atlas/pack apply {fingerprint}"));
    (plan, apply)
}

// ---- the books -------------------------------------------------------------------

fn credit() -> mush_economy::InstrumentId {
    mush_economy::InstrumentId::new(tacsim::CREDIT).expect("credit")
}

/// A principal's own credit account.
fn account_of(tw: &TestWorld, owner: &PrincipalId) -> mush_economy::AccountId {
    economy(tw)
        .snapshot()
        .accounts_of(owner)
        .find(|a| a.instrument == credit())
        .map(|a| a.id)
        .expect("a credit account")
}

fn org_account(tw: &TestWorld, entry: &str, org: &str) -> mush_economy::AccountId {
    mush_economy::AccountId(
        port_map(tw, entry).orgs[&cid(org)]
            .account
            .expect("the org's account"),
    )
}

fn posted(tw: &TestWorld, account: mush_economy::AccountId) -> i64 {
    economy(tw)
        .snapshot()
        .account(account)
        .expect("the account")
        .posted_balance
        .minor()
}

fn held(tw: &TestWorld, account: mush_economy::AccountId) -> i64 {
    economy(tw)
        .snapshot()
        .account(account)
        .expect("the account")
        .held_amount
        .minor()
}

fn kasei_account(tw: &TestWorld) -> mush_economy::AccountId {
    org_account(tw, "stickney_port", "kasei")
}

fn trust_account(tw: &TestWorld) -> mush_economy::AccountId {
    org_account(tw, "ceres_port", "coop_trust")
}

fn kasei_bid(tw: &TestWorld, good: &str) -> ListingId {
    ListingId(port_map(tw, "stickney_port").vendors[&cid("kasei")].bids[&cid(good)].listing)
}

fn kasei_store(tw: &TestWorld) -> VaultId {
    VaultId(port_map(tw, "stickney_port").vendors[&cid("kasei")].vault)
}

fn bid_qty(tw: &TestWorld, id: ListingId) -> Option<u64> {
    market(tw).listing(id).map(|l| l.quantity)
}

fn state_of(tw: &TestWorld, c: ContractId) -> Option<ContractState> {
    economy(tw).snapshot().contract(c).map(|c| c.state)
}

/// The pilot's newest sale quote.
fn last_sale(tw: &TestWorld, pilot: ObjId) -> mush_market::Quote {
    market(tw)
        .quotes_of_seller(&PrincipalId::Object(pilot))
        .last()
        .cloned()
        .expect("a sale quote")
}

/// The settlement transaction's `(account, delta)` lines, sorted.
fn settlement_lines(tw: &TestWorld, contract: ContractId) -> Vec<(mush_economy::AccountId, i64)> {
    let Some(ContractState::Settled { transaction }) = state_of(tw, contract) else {
        panic!(
            "contract #{contract} is not settled: {:?}",
            state_of(tw, contract)
        );
    };
    let mut lines: Vec<_> = economy(tw)
        .transaction(transaction)
        .expect("the settlement transaction")
        .lines
        .iter()
        .map(|l| (l.account, l.delta.minor()))
        .collect();
    lines.sort();
    lines
}

fn refused_settlements(tw: &TestWorld) -> usize {
    economy(tw)
        .escalations()
        .filter(|e| matches!(e.kind, EscalationKind::SettlementRefused))
        .count()
}

fn enrolment_of(tw: &TestWorld, pilot: ObjId) -> tacsim::port::Enrolment {
    store(tw)
        .enrolment_of(pilot)
        .cloned()
        .expect("the pilot's enrolment")
}

/// A contract's name.
fn contract_name(tw: &TestWorld, contract: ContractId) -> String {
    economy(tw)
        .snapshot()
        .contract(contract)
        .map(|c| c.name.to_string())
        .expect("the contract")
}

/// A landed sale's leg whose marker inventory's mirror has not swept yet: the
/// `inventory.transferred` fact answers nothing and the contract is still `Active`.
fn unswept(tw: &TestWorld, contract: ContractId, marker: &str) {
    use mush_economy::FactProvider;
    assert!(
        mush_inventory::InventoryFactProvider
            .resolve(tw.world(), mush_inventory::FACT_TRANSFERRED, marker, None)
            .is_none(),
        "the mirror has not swept the marker yet"
    );
    assert!(
        matches!(state_of(tw, contract), Some(ContractState::Active)),
        "landed but unswept, the sale still stands: {:?}",
        state_of(tw, contract)
    );
}

/// Sweep (inventory and market 60 s, economy 30 s) until `contract` settles.
fn settle(rig: &mut Rig, contract: ContractId) {
    for _ in 0..12 {
        if matches!(
            state_of(&rig.tw, contract),
            Some(ContractState::Settled { .. })
        ) {
            return;
        }
        sweep(&mut rig.tw, 31);
    }
    assert!(
        matches!(
            state_of(&rig.tw, contract),
            Some(ContractState::Settled { .. })
        ),
        "contract #{contract} did not settle: {:?}",
        state_of(&rig.tw, contract)
    );
}

/// What "nothing happened" is measured against.
#[derive(Debug, PartialEq, Eq)]
struct Books {
    contracts: usize,
    kasei_available: i64,
    quotes: usize,
    bids: Vec<Option<u64>>,
}

fn books(tw: &TestWorld) -> Books {
    let kasei = kasei_account(tw);
    Books {
        contracts: economy(tw).snapshot().contracts().count(),
        kasei_available: posted(tw, kasei) - held(tw, kasei),
        quotes: market(tw).quotes().count(),
        bids: ["metals", "volatiles"]
            .iter()
            .map(|good| bid_qty(tw, kasei_bid(tw, good)))
            .collect(),
    }
}

/// A refusal before any contract: the line, and nothing written anywhere.
fn refused(rig: &mut Rig, who: ObjId, line: &str, want: &str) {
    let before = books(&rig.tw);
    let out = run(&mut rig.tw, who, line);
    assert!(out.contains(want), "`{line}`: {out}");
    assert_eq!(books(&rig.tw), before, "`{line}` wrote something");
    assert!(
        drain(&mut rig.tw).is_empty(),
        "`{line}` queued a submission"
    );
}

fn port_label(entry: &str) -> String {
    tacsim::port::coop::port_name(&port_def(entry))
}

const SALE_20_METALS: &str = "Sold 20 t of metal concentrate to Kasei Metals at 280.00cr/t: 5600.00cr; Co-op share 400.00cr; you receive 5200.00cr when the goods are in. Jonas Achterberg-Nkemelu countersigns.";

// ---------------------------------------------------------------------------
// 1. The verify lens
// ---------------------------------------------------------------------------

/// **The verify lens** (the IC10 brief §1): 20 t of metals sold into Kasei's 280.00cr
/// bid at Stickney. The figures are lane v3's (a 280 bid); doc 18 §5's IC10 row's "70 t
/// … 21,000 / 19,250 / 1,750" are pre-R7 figures at a 300 bid and are superseded.
#[test]
#[allow(clippy::too_many_lines)]
fn ic10_selling_metals_at_stickney_pays_the_pilot_and_the_trust_from_journals() {
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[("metals", 20), ("volatiles", 20)]);
    let hold = hold_vault(&rig.tw, &row);
    let kasei = kasei_account(&rig.tw);
    let trust = trust_account(&rig.tw);
    let me = account_of(&rig.tw, &PrincipalId::Object(pilot));
    let (kasei_posted, trust_posted, my_posted) = (
        posted(&rig.tw, kasei),
        posted(&rig.tw, trust),
        posted(&rig.tw, me),
    );
    let metals_bid = kasei_bid(&rig.tw, "metals");
    assert_eq!(bid_qty(&rig.tw, metals_bid), Some(4_000));

    let out = run(&mut rig.tw, pilot, &format!("+market/sell {metals_bid}=20"));
    assert!(out.lines().any(|l| l == SALE_20_METALS), "{out}");
    let q = last_sale(&rig.tw, pilot);
    let contract = q.contract.expect("the sale contract");
    let lot = q.lot.clone().expect("the frozen terms");
    let marker = lot.marker.clone().expect("the marker");
    // Priced off the Exchange's live ask, not the goods table's baseline (review C3).
    assert_eq!(
        contract_name(&rig.tw, contract),
        format!("market sale #{} to bid #{metals_bid}", q.id)
    );
    let queued = drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "{out}");
    assert_eq!(queued[0].actor, pilot, "the pilot's own leg");
    assert_eq!(queued[0].parameters["op"], serde_json::json!("deliver"));
    assert_eq!(
        queued[0].key.as_str(),
        format!("market.lot.deliver.{:x}", contract.0)
    );
    // Escrowed, not paid: Kasei's available is down 5,600.00, its posted unchanged.
    assert_eq!(posted(&rig.tw, kasei), kasei_posted);
    assert_eq!(held(&rig.tw, kasei), 560_000);
    assert_eq!(bid_qty(&rig.tw, metals_bid), Some(3_980));

    // Money only after the marker: sweeps leave it Active and nobody paid.
    for _ in 0..4 {
        sweep(&mut rig.tw, 31);
    }
    assert!(matches!(
        state_of(&rig.tw, contract),
        Some(ContractState::Active)
    ));
    assert_eq!(posted(&rig.tw, me), my_posted);
    assert_eq!(posted(&rig.tw, trust), trust_posted);
    assert_eq!(held(&rig.tw, kasei), 560_000);

    land_all(&mut rig, &queued);
    assert_eq!(pool(&mut rig.tw, hold, "metals"), 0);
    let store_vault = kasei_store(&rig.tw);
    assert_eq!(pool(&mut rig.tw, store_vault, "metals"), 20);
    // Landed, but inventory's mirror has not swept: the fact is not yet true and the
    // contract still stands (IC10 fixer, review T4).
    unswept(&rig.tw, contract, &marker);
    settle(&mut rig, contract);

    // The journal: Kasei −5,600.00 = pilot +5,200.00 + Trust +400.00.
    let mut want = vec![(kasei, -560_000), (me, 520_000), (trust, 40_000)];
    want.sort();
    assert_eq!(settlement_lines(&rig.tw, contract), want);
    assert_eq!(want.iter().map(|(_, d)| d).sum::<i64>(), 0);
    assert_eq!(posted(&rig.tw, kasei), kasei_posted - 560_000);
    assert_eq!(posted(&rig.tw, me), my_posted + 520_000);
    assert_eq!(posted(&rig.tw, trust), trust_posted + 40_000);

    // Lineage: Kasei's storefront metals root inherits the hold's, carrying 20; the
    // marker names the move.
    let families = families(&rig.tw);
    let (hold_root, store_root, record) = {
        let mut txn = rig
            .tw
            .engine_mut()
            .action_transaction(&families.pools)
            .expect("a transaction");
        (
            families
                .tranche_for(&mut txn, EndpointRef(hold), &cid("metals"))
                .expect("reads")
                .expect("the hold's root"),
            families
                .tranche_for(&mut txn, EndpointRef(store_vault), &cid("metals"))
                .expect("reads")
                .expect("the storefront's root"),
            txn.get(&families.applied, &marker)
                .expect("reads")
                .cloned()
                .expect("the marker"),
        )
    };
    assert!(store_root.1.inherited().contains(&hold_root.0));
    assert_eq!(store_root.1.quantity(), 20);
    assert_eq!(record.kind(), mush_inventory::FACT_TRANSFERRED);
    assert_eq!(
        (
            record.from(),
            record.to(),
            record.quantity(),
            record.actor()
        ),
        (
            Some(EndpointRef(hold)),
            Some(EndpointRef(store_vault)),
            20,
            pilot
        )
    );

    // The readiness Bids line and the board.
    let report = status(&mut rig.tw);
    assert!(
        report.contains(&format!(
            "stickney_port bid kasei metals: listing #{metals_bid} at 280.00cr/t, 3980 of 4000 t \
             wanted. [ready]"
        )),
        "{report}"
    );
    let look = run(&mut rig.tw, pilot, "look");
    assert!(
        look.contains(
            "  wanted, metal concentrate: 280.00cr/t, 3,980 t more (Kasei Metals; signed Jonas \
             Achterberg-Nkemelu)."
        ),
        "{look}"
    );

    // The books: the enrolment folds once; the statement and +ship read them.
    let e = enrolment_of(&rig.tw, pilot);
    assert_eq!(
        (e.sales, e.margin_minor, e.shares_minor),
        (1, 160_000, 40_000)
    );
    let statement = run(&mut rig.tw, pilot, "+coop/statement");
    assert!(
        statement.contains("  Sales: 1; margin 1600.00cr; Co-op share paid 400.00cr."),
        "{statement}"
    );
    let ship = run(&mut rig.tw, pilot, "+ship");
    assert!(ship.contains("Programme: 1 sale;"), "{ship}");

    // The settled sale is a trade-tape row (residual F11a, pinned).
    let catalog = mush_economy::CatalogRef {
        ruleset: SmolStr::new_static(tacsim::NS),
        code: SmolStr::new_static("metals"),
    };
    let venue = mush_market::MarketId::new("stickney").expect("venue");
    assert!(
        market(&rig.tw)
            .trades_for(&catalog, &venue)
            .any(|t| t.id == q.id),
        "the settled sale is on Stickney's metals tape"
    );
    assert_eq!(refused_settlements(&rig.tw), 0);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 2. A sale at reference
// ---------------------------------------------------------------------------

/// **A sale at reference pays no share**: Kasei's metals bid priced at the Exchange's
/// ask (200) — one payout row, the line's share 0.00cr, the counters +1 / +0 / +0.
#[test]
fn ic10_a_sale_at_reference_pays_no_share() {
    let lane = lane_variant(LANE_VERSION, |doc| {
        kasei_metals_bid(doc)["price_major"] = serde_json::json!(200);
    });
    let mut rig = stocked_with(Roster::seeded(lane));
    let (pilot, _) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let bid = kasei_bid(&rig.tw, "metals");
    let out = run(&mut rig.tw, pilot, &format!("+market/sell {bid}=20"));
    assert!(
        out.contains("at 200.00cr/t: 4000.00cr; Co-op share 0.00cr; you receive 4000.00cr"),
        "{out}"
    );
    let contract = last_sale(&rig.tw, pilot).contract.expect("contract");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    settle(&mut rig, contract);
    let me = account_of(&rig.tw, &PrincipalId::Object(pilot));
    let mut want = vec![(kasei_account(&rig.tw), -400_000), (me, 400_000)];
    want.sort();
    assert_eq!(settlement_lines(&rig.tw, contract), want, "one payout row");
    let e = enrolment_of(&rig.tw, pilot);
    assert_eq!((e.sales, e.margin_minor, e.shares_minor), (1, 0, 0));
    assert_eq!(refused_settlements(&rig.tw), 0);
}

// ---------------------------------------------------------------------------
// 2b. The live reference off the baseline (IC10 fixer, review C3)
// ---------------------------------------------------------------------------

/// **The reference is the origin ask's live price** (decision 8): the Exchange's metals
/// ask priced at 220.00cr/t, off the goods table's 200 baseline — the share is 250‰ of
/// (280 − 220) × 20 = 300.00cr, never the baseline's 400.00, and the contract carries no
/// `(baseline ref)` memo.
#[test]
fn ic10_the_reference_is_the_origin_asks_live_price() {
    let lane = lane_variant(LANE_VERSION, |doc| {
        let vendors = doc["entries"]["ceres_port"]["def"]["vendors"]
            .as_array_mut()
            .expect("vendors");
        let exchange = vendors
            .iter_mut()
            .find(|v| v["org"] == "freeport_exchange")
            .expect("the Exchange");
        let ask = exchange["asks"]
            .as_array_mut()
            .expect("asks")
            .iter_mut()
            .find(|a| a["good"] == "metals")
            .expect("the metals ask");
        ask["price_major"] = serde_json::json!(220);
    });
    let mut rig = stocked_with(Roster::seeded(lane));
    let exchange_store =
        VaultId(port_map(&rig.tw, "ceres_port").vendors[&cid("freeport_exchange")].vault);
    let ask = market(&rig.tw)
        .lot_listing_of(EndpointRef(exchange_store), &cid("metals"))
        .expect("the Exchange's metals ask");
    assert_eq!(
        market(&rig.tw).listing(ask).map(|l| l.price.minor()),
        Some(22_000)
    );
    let (pilot, _) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let bid = kasei_bid(&rig.tw, "metals");
    let out = run(&mut rig.tw, pilot, &format!("+market/sell {bid}=20"));
    assert!(
        out.contains("at 280.00cr/t: 5600.00cr; Co-op share 300.00cr; you receive 5300.00cr"),
        "{out}"
    );
    let q = last_sale(&rig.tw, pilot);
    let contract = q.contract.expect("contract");
    assert!(
        !contract_name(&rig.tw, contract).contains("(baseline ref)"),
        "{}",
        contract_name(&rig.tw, contract)
    );
    assert_eq!(q.cuts.first().map(|c| c.amount.minor()), Some(30_000));
    assert_eq!(q.basis.minor(), 120_000);
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    settle(&mut rig, contract);
    let me = account_of(&rig.tw, &PrincipalId::Object(pilot));
    let mut want = vec![
        (kasei_account(&rig.tw), -560_000),
        (me, 530_000),
        (trust_account(&rig.tw), 30_000),
    ];
    want.sort();
    assert_eq!(settlement_lines(&rig.tw, contract), want);
    let e = enrolment_of(&rig.tw, pilot);
    assert_eq!(
        (e.sales, e.margin_minor, e.shares_minor),
        (1, 120_000, 30_000)
    );
    assert_eq!(refused_settlements(&rig.tw), 0);
}

// ---------------------------------------------------------------------------
// 3. The baseline reference
// ---------------------------------------------------------------------------

/// **The reference falls back to the goods table's baseline when the origin ask is
/// gone**: the Exchange's metals Draw is withdrawn and no port tick fires before the
/// sale (the stock arm would relist it); the share is still 400.00 against
/// `reference_major` 200 and the contract's name carries `(baseline ref)`.
#[test]
fn ic10_the_reference_falls_back_to_the_baseline_when_the_origin_ask_is_gone() {
    let mut rig = stocked();
    let (pilot, _) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let exchange_store =
        VaultId(port_map(&rig.tw, "ceres_port").vendors[&cid("freeport_exchange")].vault);
    let ask = market(&rig.tw)
        .lot_listing_of(EndpointRef(exchange_store), &cid("metals"))
        .expect("the Exchange's metals ask");
    let out = run(&mut rig.tw, GOD, &format!("@market/vendor withdraw {ask}"));
    assert!(market(&rig.tw).listing(ask).is_none(), "{out}");
    let bid = kasei_bid(&rig.tw, "metals");
    let out = run(&mut rig.tw, pilot, &format!("+market/sell {bid}=20"));
    assert!(out.contains("Co-op share 400.00cr"), "{out}");
    let q = last_sale(&rig.tw, pilot);
    let name = economy(&rig.tw)
        .snapshot()
        .contract(q.contract.expect("contract"))
        .map(|c| c.name.to_string())
        .expect("named");
    assert!(name.ends_with("(baseline ref)"), "{name}");
    assert_eq!(q.cuts.first().map(|c| c.amount.minor()), Some(40_000));
}

// ---------------------------------------------------------------------------
// 3b. The programme's moon reference rows (doc 19 cut A.8)
// ---------------------------------------------------------------------------

/// The shipped lane at its own version, re-signed, with Kasei bidding instrument
/// boards at 1,100.00cr/t (100 t) so an enrolled pilot can sell boards at Stickney;
/// with `reference` false Ceres's programme also loses its `instrument_boards`
/// reference row (the A.7 shape).
fn kasei_boards_lane(reference: bool) -> String {
    lane_variant(LANE_VERSION, |doc| {
        kasei(doc)["bids"]
            .as_array_mut()
            .expect("bids")
            .push(serde_json::json!({
                "good": "instrument_boards",
                "price_major": 1100,
                "intake": 100,
                "boards": ["receiving", "exchange"]
            }));
        if !reference {
            doc["entries"]["ceres_port"]["def"]["programme"]["reference"]
                .as_object_mut()
                .expect("the programme's reference")
                .remove("instrument_boards")
                .expect("the boards row");
        }
    })
}

/// Sell 20 t of instrument boards into Kasei's boards bid on `lane`: the posted bid
/// price (major), the quote and the settled sale's enrolment.
fn sell_boards(lane: String) -> (u64, mush_market::Quote, tacsim::port::Enrolment, String) {
    let mut rig = stocked_with(Roster::seeded(lane));
    let (pilot, _) = docked_at_stickney(&mut rig, &[("instrument_boards", 20)]);
    let bid = kasei_bid(&rig.tw, "instrument_boards");
    let price = market(&rig.tw)
        .listing(bid)
        .map(|l| l.price.minor())
        .expect("Kasei's boards bid is posted");
    assert_eq!(price % 100, 0, "a whole-credit bid");
    let out = run(&mut rig.tw, pilot, &format!("+market/sell {bid}=20"));
    assert!(out.contains("Sold 20 t of "), "{out}");
    let q = last_sale(&rig.tw, pilot);
    let contract = q.contract.expect("the sale contract");
    let name = contract_name(&rig.tw, contract);
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    settle(&mut rig, contract);
    assert_eq!(refused_settlements(&rig.tw), 0);
    no_refused_writes(&rig.tw);
    let e = enrolment_of(&rig.tw, pilot);
    (
        u64::try_from(price / 100).expect("a positive price"),
        q,
        e,
        name,
    )
}

/// **The programme's boards reference names Crisium's live ask** (doc 19 §3.8, Q8,
/// D-19-19): an enrolled pilot sells 20 t of instrument boards into a Kasei boards bid
/// at Stickney; Ceres's `reference.instrument_boards` is
/// `luna_port/crisium_instruments`, a live 900.00cr/t ask, so the margin is
/// 20 × (bid − 900), the share 250‰ of it, and the contract carries no
/// `(baseline ref)` memo.
#[test]
fn a8_the_programme_reference_names_the_moon_asks() {
    let (price, q, e, name) = sell_boards(kasei_boards_lane(true));
    assert_eq!(
        price, 1_100,
        "no reach premium: no boards supplier routes to Stickney"
    );
    let margin = 20 * (i64::try_from(price).expect("fits") - 900) * 100;
    assert_eq!(q.basis.minor(), margin);
    assert_eq!(q.cuts.first().map(|c| c.amount.minor()), Some(margin / 4));
    assert!(
        !name.contains(&format!("({})", tacsim::port::BASELINE_MEMO)),
        "{name}"
    );
    assert_eq!(
        (e.sales, e.margin_minor, e.shares_minor),
        (1, margin, margin / 4)
    );
}

/// **The control: without the reference row the same sale stamps the baseline
/// memo** — the share falls back to the goods table's `reference_major` (also 900
/// for boards, so the figures agree) and the contract's name ends `(baseline ref)`.
/// The memo, not the money, is what tells the live reference from the baseline.
#[test]
fn a8_without_the_boards_reference_row_the_sale_stamps_the_baseline_memo() {
    let (price, q, e, name) = sell_boards(kasei_boards_lane(false));
    let margin = 20 * (i64::try_from(price).expect("fits") - 900) * 100;
    assert!(
        name.ends_with(&format!("({})", tacsim::port::BASELINE_MEMO)),
        "{name}"
    );
    assert_eq!(q.basis.minor(), margin);
    assert_eq!(
        (e.sales, e.margin_minor, e.shares_minor),
        (1, margin, margin / 4)
    );
}

// ---------------------------------------------------------------------------
// 4. Refusals before any contract
// ---------------------------------------------------------------------------

/// **Refusals before any contract**: over-selling (and a second sale with the first
/// leg taken but unlanded — `SELL_SHORT`, not `SELL_BUSY`), an unfunded bid, the
/// cross-port sales (owner item O2: in the gate's load-worded lines), a good the bid
/// does not name, and Limbo.
#[test]
#[allow(clippy::too_many_lines)]
fn ic10_refusals_before_any_contract() {
    // Cross-port, no flight: the hull is still at Ceres.
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let vault = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, vault, "metals", 20, LotId(901));
    let bid = kasei_bid(&rig.tw, "metals");
    let line = format!("+market/sell {bid}=20");
    // The pilot at Ceres with the hull at Ceres.
    refused(
        &mut rig,
        pilot,
        &line,
        &NOT_IN_PORT.replace("{port}", &port_label("stickney_port")),
    );
    // The pilot at Stickney with the hull still at Ceres: the cross-port sale.
    let receiving = p07(&rig.tw);
    teleport(&mut rig.tw, pilot, receiving);
    refused(
        &mut rig,
        pilot,
        &line,
        &HOLD_ELSEWHERE
            .replace("{name}", "Salt Moth")
            .replace("{other}", &port_label("ceres_port"))
            .replace("{port}", &port_label("stickney_port")),
    );
    // From Limbo.
    teleport(&mut rig.tw, pilot, LIMBO);
    refused(
        &mut rig,
        pilot,
        &line,
        &NOT_IN_PORT.replace("{port}", &port_label("stickney_port")),
    );

    // Docked at Stickney with only metals aboard.
    let mut rig = stocked();
    let (pilot, _) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let metals = kasei_bid(&rig.tw, "metals");
    let ice = kasei_bid(&rig.tw, "volatiles");
    refused(
        &mut rig,
        pilot,
        &format!("+market/sell {metals}=21"),
        "You have 20 of that there to sell, not 21, so nothing was sold.",
    );
    refused(
        &mut rig,
        pilot,
        &format!("+market/sell {ice}=5"),
        "You have 0 of that there to sell, not 5, so nothing was sold.",
    );
    let out = run(&mut rig.tw, pilot, &format!("+market/sell {metals}=20"));
    assert!(out.contains("Sold 20 t"), "{out}");
    let taken = drain(&mut rig.tw);
    assert_eq!(taken.len(), 1);
    refused(
        &mut rig,
        pilot,
        &format!("+market/sell {metals}=1"),
        "You have 0 of that there to sell, not 1, so nothing was sold.",
    );

    // An unfunded house.
    let lane = lane_variant(LANE_VERSION, |doc| {
        let orgs = doc["entries"]["stickney_port"]["def"]["orgs"]
            .as_array_mut()
            .expect("orgs");
        let kasei = orgs.iter_mut().find(|o| o["id"] == "kasei").expect("kasei");
        kasei["capital_major"] = serde_json::json!(1_000);
    });
    let mut rig = stocked_with(Roster::seeded(lane));
    let (pilot, _) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let metals = kasei_bid(&rig.tw, "metals");
    refused(
        &mut rig,
        pilot,
        &format!("+market/sell {metals}=20"),
        mush_market::UNFUNDED,
    );
    assert_eq!(refused_settlements(&rig.tw), 0);
}

// ---------------------------------------------------------------------------
// 5. A filed clearance
// ---------------------------------------------------------------------------

/// **A filed clearance refuses the sale** (`HOLD_CLEARED` since doc 18 cut IC14 row
/// 23; nothing moved), and the
/// gate admits `Presence::Ship(hull)` on the hold while docked with the pilot aboard,
/// out of the port's rooms (lead 2g).
#[test]
fn ic10_a_filed_clearance_refuses_the_sale() {
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let hull = ObjId(row.hull);
    let hold = hold_vault(&rig.tw, &row);
    provision(&mut rig, pilot, &row, "ceres", 940);
    let gallery = station_room(&rig.tw, "stickney_port", "room.p01_berth_gallery");
    teleport(&mut rig.tw, pilot, gallery);
    let out = run(&mut rig.tw, pilot, "+ship/board");
    assert!(
        out.contains("You cycle the lock into the Salt Moth."),
        "{out}"
    );
    let venue = mush_market::MarketId::new("stickney").expect("venue");
    assert_eq!(
        PortVenueGate
            .admit(
                rig.tw.world(),
                pilot,
                &venue,
                Some(EndpointRef(hold)),
                Presence::Ship(hull)
            )
            .map_err(|r| r.line),
        Ok(()),
        "a queued order is admitted aboard the docked hull"
    );
    let out = run(&mut rig.tw, pilot, "+ship/undock ceres");
    assert!(out.contains("Clearance filed for Ceres"), "{out}");
    let _burn = drain(&mut rig.tw);
    assert!(voyage(&rig.tw, hull).and_then(|v| v.pending).is_some());
    // Staff take the pilot ashore (a filed hull keeps its crew; `voyage.rs`'s
    // `ic9_a_pilot_moved_ashore_holds_the_launch` precedent) and put them in P07.
    support::with_ctx(&mut rig.tw, |ctx| {
        mush_sim::unplace(ctx, pilot, "staff took the pilot ashore").expect("unplaces")
    });
    let receiving = p07(&rig.tw);
    teleport(&mut rig.tw, pilot, receiving);
    let bid = kasei_bid(&rig.tw, "metals");
    refused(
        &mut rig,
        pilot,
        &format!("+market/sell {bid}=20"),
        &tacsim::port::HOLD_CLEARED
            .replace("{name}", "Salt Moth")
            .replace("{port}", &port_label("stickney_port")),
    );
    assert_eq!(pool(&mut rig.tw, hold, "metals"), 20);
}

// ---------------------------------------------------------------------------
// 6. Undock waits for the hand-over
// ---------------------------------------------------------------------------

/// **Undock waits for the hand-over** (`GOODS_LEAVING`), the remedy re-stages it, and
/// once it lands and the inventory mirror sweeps the undock proceeds; a dropped leg
/// left past its cutoff no longer holds the hull while the contract still stands.
#[test]
fn ic10_undock_waits_for_the_hand_over() {
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    provision(&mut rig, pilot, &row, "ceres", 950);
    let bid = kasei_bid(&rig.tw, "metals");
    let out = run(&mut rig.tw, pilot, &format!("+market/sell {bid}=20"));
    assert!(out.contains("Sold 20 t"), "{out}");
    let _dropped = drain(&mut rig.tw);
    let q = last_sale(&rig.tw, pilot);
    let gallery = station_room(&rig.tw, "stickney_port", "room.p01_berth_gallery");
    teleport(&mut rig.tw, pilot, gallery);
    let _ = run(&mut rig.tw, pilot, "+ship/board");
    let out = run(&mut rig.tw, pilot, "+ship/undock ceres");
    assert!(
        out.contains(
            &GOODS_LEAVING
                .replace("{quote}", &q.id.to_string())
                .replace("{name}", "Salt Moth")
        ),
        "{out}"
    );
    assert!(drain(&mut rig.tw).is_empty());
    let again = run(&mut rig.tw, pilot, &format!("+market/accept {}", q.id));
    assert!(
        again.contains("Asking again for the hand-over on contract #"),
        "{again}"
    );
    let restaged = drain(&mut rig.tw);
    assert_eq!(restaged.len(), 1);
    land_all(&mut rig, &restaged);
    sweep(&mut rig.tw, 61);
    let out = run(&mut rig.tw, pilot, "+ship/undock ceres");
    assert!(!out.contains("goods have not left the hold"), "{out}");
    assert!(out.contains("Clearance filed for Ceres"), "{out}");
    let _ = drain(&mut rig.tw);

    // A dropped leg left past its cutoff no longer holds the hull.
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    provision(&mut rig, pilot, &row, "ceres", 960);
    let bid = kasei_bid(&rig.tw, "metals");
    let _ = run(&mut rig.tw, pilot, &format!("+market/sell {bid}=20"));
    let _dropped = drain(&mut rig.tw);
    let q = last_sale(&rig.tw, pilot);
    let contract = q.contract.expect("contract");
    let cutoff = q.lot.as_ref().and_then(|l| l.draw_by).expect("the cutoff");
    rig.tw.set_now(cutoff + 1);
    let gallery = station_room(&rig.tw, "stickney_port", "room.p01_berth_gallery");
    teleport(&mut rig.tw, pilot, gallery);
    let boarded = run(&mut rig.tw, pilot, "+ship/board");
    assert!(
        boarded.contains("You cycle the lock into the Salt Moth."),
        "{boarded}"
    );
    let out = run(&mut rig.tw, pilot, "+ship/undock ceres");
    assert!(!out.contains("goods have not left the hold"), "{out}");
    assert!(out.contains("Clearance filed for Ceres"), "{out}");
    assert!(matches!(
        state_of(&rig.tw, contract),
        Some(ContractState::Active)
    ));
    let _ = drain(&mut rig.tw);
}

// ---------------------------------------------------------------------------
// 7. An unlanded leg
// ---------------------------------------------------------------------------

/// **An unlanded leg refunds Kasei and restores the bid**: past 3,600 real seconds the
/// contract expires, Kasei's hold is released, the pilot and the Trust are unpaid,
/// the hold still holds 20, the bid is back at 4,000 and the counters are unchanged.
#[test]
fn ic10_an_unlanded_leg_refunds_kasei_and_restores_the_bid() {
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let hold = hold_vault(&rig.tw, &row);
    let kasei = kasei_account(&rig.tw);
    let (kasei_posted, trust_posted) = (
        posted(&rig.tw, kasei),
        posted(&rig.tw, trust_account(&rig.tw)),
    );
    let me = account_of(&rig.tw, &PrincipalId::Object(pilot));
    let my_posted = posted(&rig.tw, me);
    let bid = kasei_bid(&rig.tw, "metals");
    let _ = run(&mut rig.tw, pilot, &format!("+market/sell {bid}=20"));
    let _dropped = drain(&mut rig.tw);
    let contract = last_sale(&rig.tw, pilot).contract.expect("contract");
    sweep(&mut rig.tw, 3_601);
    sweep(&mut rig.tw, 62);
    sweep(&mut rig.tw, 62);
    assert!(matches!(
        state_of(&rig.tw, contract),
        Some(ContractState::Expired)
    ));
    assert_eq!(held(&rig.tw, kasei), 0);
    assert_eq!(posted(&rig.tw, kasei), kasei_posted);
    assert_eq!(posted(&rig.tw, me), my_posted);
    assert_eq!(posted(&rig.tw, trust_account(&rig.tw)), trust_posted);
    assert_eq!(pool(&mut rig.tw, hold, "metals"), 20);
    assert_eq!(bid_qty(&rig.tw, bid), Some(4_000));
    let e = enrolment_of(&rig.tw, pilot);
    assert_eq!((e.sales, e.margin_minor, e.shares_minor), (0, 0, 0));
}

// ---------------------------------------------------------------------------
// 8. The exhausted bid
// ---------------------------------------------------------------------------

/// **An exhausted bid reads taken and is never re-posted**: Kasei's metals intake 20,
/// sold whole; the listing stands at 0, the readiness line reads taken and ready, the
/// board "not buying"; past its deadline the zero bid stands and the clerk posts
/// nothing.
#[test]
fn ic10_an_exhausted_bid_reads_taken_and_is_never_reposted() {
    let lane = lane_variant(LANE_VERSION, |doc| {
        kasei_metals_bid(doc)["intake"] = serde_json::json!(20);
    });
    let mut rig = stocked_with(Roster::seeded(lane));
    let (pilot, _) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let bid = kasei_bid(&rig.tw, "metals");
    let _ = run(&mut rig.tw, pilot, &format!("+market/sell {bid}=20"));
    let contract = last_sale(&rig.tw, pilot).contract.expect("contract");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    // Settled before the jump past the bid's deadline (IC10 fixer, review T2): left
    // unswept, the landed sale would lapse unpaid at its own deadline instead.
    settle(&mut rig, contract);
    assert_eq!(bid_qty(&rig.tw, bid), Some(0));
    let report = status(&mut rig.tw);
    assert!(
        report.contains(&format!(
            "stickney_port bid kasei metals: listing #{bid} not buying; its intake is taken. \
             [ready]"
        )),
        "{report}"
    );
    let look = run(&mut rig.tw, pilot, "look");
    assert!(
        look.contains("  wanted, metal concentrate: not buying (Kasei Metals)."),
        "{look}"
    );
    let deadline = market(&rig.tw)
        .listing(bid)
        .map(|l| l.expires.at)
        .expect("the bid");
    let now = rig.tw.world().now();
    sweep(&mut rig.tw, deadline - now + 1);
    for _ in 0..6 {
        let _ = fire_quietly(&mut rig);
    }
    sweep(&mut rig.tw, 61);
    assert_eq!(bid_qty(&rig.tw, bid), Some(0), "the zero bid stands");
    assert_eq!(
        kasei_bid(&rig.tw, "metals"),
        bid,
        "the clerk posted nothing"
    );
    // Doc 19 cut A.8: Luna's Cabeus Works bids metals too, at its own port; at
    // Stickney, Kasei's zero bid stands alone.
    let cabeus = ListingId(
        port_map(&rig.tw, "luna_port").vendors[&cid("cabeus_works")].bids[&cid("metals")].listing,
    );
    let wanted_metals = market(&rig.tw)
        .live_listings()
        .filter(|l| l.id != cabeus)
        .filter(|l| {
            matches!(&l.source, mush_market::Supply::Wanted { commodity, .. } if commodity.0.as_str() == "metals")
        })
        .count();
    assert_eq!(wanted_metals, 1, "only Kasei's zero bid stands for metals");
    let pages: Vec<String> = economy(&rig.tw)
        .escalations()
        .map(|e| e.context.clone())
        .collect();
    assert!(pages.is_empty(), "no escalation of any kind: {pages:?}");
}

// ---------------------------------------------------------------------------
// 9. The lapsed bid
// ---------------------------------------------------------------------------

/// **A lapsed bid is reposted by the clerk**: past its deadline the market sweep
/// deletes it, readiness reads it lapsed and pending, the Stickney clerk posts it again
/// (a new listing, the same price, the full authored intake, at most one bid per port
/// per fire) and readiness is ready; a bid withdrawn by hand before its deadline reads
/// `no bid is posted` and is not re-posted.
#[test]
fn ic10_a_lapsed_bid_is_reposted_by_the_clerk() {
    let mut rig = stocked();
    let old = kasei_bid(&rig.tw, "metals");
    let expires =
        port_map(&rig.tw, "stickney_port").vendors[&cid("kasei")].bids[&cid("metals")].expires;
    let old_ice = kasei_bid(&rig.tw, "volatiles");
    let now = rig.tw.world().now();
    sweep(&mut rig.tw, expires - now + 1);
    // Whichever of Kasei's two bids the clerk has not re-posted yet reads lapsed and
    // pending (the arm re-lists the port's lapsed asks first, one step per fire).
    let mut seen_pending = false;
    for _ in 0..3 {
        let report = status(&mut rig.tw);
        for (good, recorded) in [("metals", old), ("volatiles", old_ice)] {
            if kasei_bid(&rig.tw, good) == recorded && market(&rig.tw).listing(recorded).is_none() {
                assert!(
                    report.contains(&format!(
                        "stickney_port bid kasei {good}: listing #{recorded} lapsed; the clerk \
                         posts it again. [pending]"
                    )),
                    "{report}"
                );
                seen_pending = true;
            }
        }
        if seen_pending {
            break;
        }
        sweep(&mut rig.tw, 61);
    }
    assert!(seen_pending, "a lapsed bid read pending");
    // Fire until the clerk has posted it again; never more than one bid per port per
    // fire.
    let stickney_bids = |tw: &TestWorld| -> usize {
        let map = port_map(tw, "stickney_port");
        map.vendors
            .values()
            .flat_map(|row| row.bids.values())
            .filter(|b| market(tw).listing(ListingId(b.listing)).is_some())
            .count()
    };
    let mut reposted = false;
    for _ in 0..20 {
        let before = stickney_bids(&rig.tw);
        let _ = fire_quietly(&mut rig);
        let after = stickney_bids(&rig.tw);
        assert!(after <= before + 1, "one bid per port per fire");
        if kasei_bid(&rig.tw, "metals") != old {
            reposted = true;
            break;
        }
    }
    assert!(reposted, "the clerk posted the bid again");
    let new = kasei_bid(&rig.tw, "metals");
    let listing = market(&rig.tw).listing(new).expect("the new bid").clone();
    assert_eq!(listing.price.minor(), 28_000);
    assert_eq!(listing.quantity, 4_000);
    let row =
        port_map(&rig.tw, "stickney_port").vendors[&cid("kasei")].bids[&cid("metals")].clone();
    assert_eq!(row.expires, listing.expires.at);
    assert_eq!(row.intake, 4_000);
    let report = status(&mut rig.tw);
    assert!(
        report.contains(&format!(
            "stickney_port bid kasei metals: listing #{new} at 280.00cr/t, 4000 of 4000 t wanted. \
             [ready]"
        )),
        "{report}"
    );

    // Withdrawn by hand before its deadline: missing, and not re-posted.
    let _ = run(&mut rig.tw, GOD, &format!("@market/vendor withdraw {new}"));
    assert!(market(&rig.tw).listing(new).is_none());
    let report = status(&mut rig.tw);
    assert!(
        report.contains("stickney_port bid kasei metals: no bid is posted. [missing]"),
        "{report}"
    );
    for _ in 0..6 {
        let _ = fire_quietly(&mut rig);
    }
    assert_eq!(
        kasei_bid(&rig.tw, "metals"),
        new,
        "not re-posted before its deadline"
    );
    assert!(market(&rig.tw).listing(new).is_none());
}

// ---------------------------------------------------------------------------
// 9b. A gone bid whose authored intake changed (IC10 fixer, review F2)
// ---------------------------------------------------------------------------

/// **A gone bid whose authored intake changed is still posted again**: Kasei's metals
/// bid is withdrawn by hand, an update authoring intake 3,000 applies before its
/// deadline (the bid is not re-posted then — it was withdrawn by hand), the `BidRow`
/// records the new intake, and once the deadline has passed readiness reads it lapsed
/// and pending — never "waits for the next apply" with no apply left to post it — and
/// the clerk posts it at 3,000.
#[test]
fn ic10_a_gone_bid_with_a_changed_intake_is_posted_at_the_new_intake() {
    let base = lane_variant(LANE_VERSION, |_| {});
    let mut next_doc: serde_json::Value = serde_json::from_str(&base).expect("JSON");
    next_doc["manifest"]["version"] = serde_json::json!(LANE_VERSION + 1);
    next_doc["manifest"]["digest"] = serde_json::json!(UNSIGNED);
    kasei_metals_bid(&mut next_doc)["intake"] = serde_json::json!(3_000);
    let next_json = sign(&serde_json::to_string_pretty(&next_doc).expect("serializes"));
    let next = mush_content::load_pack(&next_json).expect("the variant verifies");
    let mut roster = Roster::seeded(base);
    roster.shelf = Some(vec![next]);
    roster.extra_lane = Some(next_json);
    let mut rig = stocked_with(roster);
    let old = kasei_bid(&rig.tw, "metals");
    let expires =
        port_map(&rig.tw, "stickney_port").vendors[&cid("kasei")].bids[&cid("metals")].expires;
    let _ = run(&mut rig.tw, GOD, &format!("@market/vendor withdraw {old}"));
    assert!(market(&rig.tw).listing(old).is_none());
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@{}", LANE_VERSION + 1)),
        "{plan}\n{apply}"
    );
    let row =
        port_map(&rig.tw, "stickney_port").vendors[&cid("kasei")].bids[&cid("metals")].clone();
    assert_eq!(
        (row.listing, row.intake, row.expires),
        (old.0, 3_000, expires),
        "the authored intake is recorded; the listing and deadline are kept"
    );
    assert_eq!(
        kasei_bid(&rig.tw, "metals"),
        old,
        "not re-posted by the apply"
    );
    let report = status(&mut rig.tw);
    assert!(
        report.contains("stickney_port bid kasei metals: no bid is posted. [missing]"),
        "{report}"
    );

    // Past the deadline: lapsed and pending, then posted by the clerk at 3,000.
    let now = rig.tw.world().now();
    rig.tw.set_now(expires.max(now) + 1);
    let report = status(&mut rig.tw);
    assert!(
        report.contains(&format!(
            "stickney_port bid kasei metals: listing #{old} lapsed; the clerk posts it again. \
             [pending]"
        )),
        "{report}"
    );
    assert!(!report.contains("waits for the next apply"), "{report}");
    let mut reposted = false;
    for _ in 0..20 {
        let _ = fire_quietly(&mut rig);
        if kasei_bid(&rig.tw, "metals") != old {
            reposted = true;
            break;
        }
    }
    assert!(reposted, "the clerk posted the bid again");
    let new = kasei_bid(&rig.tw, "metals");
    let listing = market(&rig.tw).listing(new).expect("the new bid").clone();
    assert_eq!(listing.quantity, 3_000);
    assert_eq!(listing.price.minor(), 28_000);
    let report = status(&mut rig.tw);
    assert!(
        report.contains(&format!(
            "stickney_port bid kasei metals: listing #{new} at 280.00cr/t, 3000 of 3000 t wanted. \
             [ready]"
        )),
        "{report}"
    );
}

// ---------------------------------------------------------------------------
// 10. A reboot between the leg and the settlement
// ---------------------------------------------------------------------------

/// **A reboot between the leg and the settlement settles once**: the counters fold
/// once and a re-landed leg is refused.
#[test]
fn ic10_a_reboot_between_the_leg_and_the_settlement_settles_once() {
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let bid = kasei_bid(&rig.tw, "metals");
    let _ = run(&mut rig.tw, pilot, &format!("+market/sell {bid}=20"));
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let sale = last_sale(&rig.tw, pilot);
    let contract = sale.contract.expect("contract");
    let marker = sale.lot.and_then(|l| l.marker).expect("the marker");
    // The reboot falls between the leg and the settlement (review T4).
    unswept(&rig.tw, contract, &marker);
    let mut rig = reboot(rig);
    settle(&mut rig, contract);
    for _ in 0..4 {
        sweep(&mut rig.tw, 31);
    }
    let e = enrolment_of(&rig.tw, pilot);
    assert_eq!(
        (e.sales, e.margin_minor, e.shares_minor),
        (1, 160_000, 40_000)
    );
    let refusal = land(&mut rig, &queued[0]);
    assert!(refusal.is_err(), "a re-landed leg is refused: {refusal:?}");
    let hold = hold_vault(&rig.tw, &row);
    assert_eq!(pool(&mut rig.tw, hold, "metals"), 0);
    assert_eq!(refused_settlements(&rig.tw), 0);
}

// ---------------------------------------------------------------------------
// 11. Retire
// ---------------------------------------------------------------------------

/// **Retire refuses with a sale in progress**, naming the vendor; once the sale settles
/// with the metals bid exhausted (the intake-20 variant), the retire withdraws the zero
/// bid and drops its `BidRow` (owner item O4(c)).
#[test]
fn ic10_retire_refuses_with_a_sale_in_progress() {
    let base = lane_variant(LANE_VERSION, |doc| {
        kasei_metals_bid(doc)["intake"] = serde_json::json!(20);
    });
    let mut base_doc: serde_json::Value = serde_json::from_str(&base).expect("JSON");
    base_doc["manifest"]["version"] = serde_json::json!(LANE_VERSION + 1);
    base_doc["manifest"]["digest"] = serde_json::json!(UNSIGNED);
    base_doc["entries"]
        .as_object_mut()
        .expect("entries")
        .remove("stickney_port");
    let retired_json = sign(&serde_json::to_string_pretty(&base_doc).expect("serializes"));
    let retired = mush_content::load_pack(&retired_json).expect("the retiring variant verifies");
    // On the atlas shelf only (`commerce.rs`'s retire precedent): the layer cannot load
    // a lane whose remaining port routes to a venue it no longer authors.
    let mut roster = Roster::seeded(base);
    roster.shelf = Some(vec![retired]);
    let mut rig = stocked_with(roster);
    let (pilot, _) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let bid = kasei_bid(&rig.tw, "metals");
    let _ = run(&mut rig.tw, pilot, &format!("+market/sell {bid}=20"));
    let queued = drain(&mut rig.tw);
    let contract = last_sale(&rig.tw, pilot).contract.expect("contract");
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains("stickney_port: vendor kasei has a deal in progress"),
        "{plan}\n{apply}"
    );
    land_all(&mut rig, &queued);
    settle(&mut rig, contract);
    assert_eq!(bid_qty(&rig.tw, bid), Some(0));
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@{}", LANE_VERSION + 1)),
        "{plan}\n{apply}"
    );
    assert!(
        market(&rig.tw).listing(bid).is_none(),
        "the zero bid is withdrawn"
    );
    assert!(
        !port_map(&rig.tw, "stickney_port").vendors[&cid("kasei")]
            .bids
            .contains_key(&cid("metals")),
        "its BidRow leaves the map"
    );
}

// ---------------------------------------------------------------------------
// 12. The books fold by pilot and hold
// ---------------------------------------------------------------------------

/// **The books fold by pilot and hold** (reviewer F3): two enrolled pilots on two Drays
/// each sell once and each enrolment folds exactly its own sale; B naming A's hold is
/// refused before any contract; and the policy called directly with B as the seller of
/// A's hold refuses `SALE_NOT_PILOT` naming A's hull. At HEAD a revoked Dray is not
/// re-chartered while its hold vault stands (`hull_is_free`), so the re-charter case
/// the key guards becomes reachable only with IC13's recall; the pilot-and-hold key is
/// taken now so IC13 inherits it.
#[test]
#[allow(clippy::too_many_lines)]
fn ic10_the_books_fold_by_pilot_and_hold() {
    let mut rig = stocked();
    let (a, row_a) = chartered(&mut rig, "Alpha", 1, "Salt Moth");
    let (b, row_b) = chartered(&mut rig, "Bravo", 2, "Tern");
    let (hold_a, hold_b) = (hold_vault(&rig.tw, &row_a), hold_vault(&rig.tw, &row_b));
    seed_pool(&mut rig, hold_a, "metals", 20, LotId(801));
    seed_pool(&mut rig, hold_b, "metals", 10, LotId(802));
    provision(&mut rig, a, &row_a, "stickney", 810);
    provision(&mut rig, b, &row_b, "stickney", 820);
    fly_to_stickney(
        &mut rig,
        &[(a, row_a.clone(), "Salt Moth"), (b, row_b.clone(), "Tern")],
    );
    let bid = kasei_bid(&rig.tw, "metals");
    let out = run(&mut rig.tw, a, &format!("+market/sell {bid}=20"));
    assert!(out.contains("Sold 20 t"), "{out}");
    let ca = last_sale(&rig.tw, a).contract.expect("A's contract");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let out = run(&mut rig.tw, b, &format!("+market/sell {bid}=10"));
    assert!(out.contains("Sold 10 t"), "{out}");
    let cb = last_sale(&rig.tw, b).contract.expect("B's contract");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    settle(&mut rig, ca);
    settle(&mut rig, cb);
    let ea = enrolment_of(&rig.tw, a);
    let eb = enrolment_of(&rig.tw, b);
    assert_eq!(
        (ea.sales, ea.margin_minor, ea.shares_minor),
        (1, 160_000, 40_000)
    );
    assert_eq!(
        (eb.sales, eb.margin_minor, eb.shares_minor),
        (1, 80_000, 20_000)
    );
    let sa = run(&mut rig.tw, a, "+coop/statement");
    assert!(
        sa.contains("  Sales: 1; margin 1600.00cr; Co-op share paid 400.00cr."),
        "{sa}"
    );
    let sb = run(&mut rig.tw, b, "+coop/statement");
    assert!(
        sb.contains("  Sales: 1; margin 800.00cr; Co-op share paid 200.00cr."),
        "{sb}"
    );

    // B names A's hold: refused before any contract, by custody (`may_take`, check 7)
    // before the gate is asked — the one existence-hiding answer, never the gate's
    // `NOT_YOUR_CHARTER` (review C4: the arm that fires is pinned).
    seed_pool(&mut rig, hold_a, "metals", 5, LotId(803));
    let before = books(&rig.tw);
    let out = run(
        &mut rig.tw,
        b,
        &format!("+market/sell {bid}=5 from={}", hold_a.0),
    );
    assert!(out.contains(mush_market::NO_SUCH_LISTING), "{out}");
    assert!(!out.contains("charter"), "{out}");
    assert_eq!(books(&rig.tw), before);
    assert!(drain(&mut rig.tw).is_empty());

    // The policy's own belt.
    let house = PrincipalId::Organization(mush_org::OrgId(
        port_map(&rig.tw, "stickney_port").orgs[&cid("kasei")].org,
    ));
    let venue = mush_market::MarketId::new("stickney").expect("venue");
    let metals = cid("metals");
    let terms = SaleTerms {
        seller: b,
        source: EndpointRef(hold_a),
        house: &house,
        market: &venue,
        listing: bid,
        commodity: &metals,
        quantity: 5,
        price: mush_economy::Amount::from_minor(28_000),
        gross: mush_economy::Amount::from_minor(140_000),
    };
    let refusal = CoopProceeds
        .proceeds(rig.tw.world(), &terms)
        .expect_err("not the hold's pilot");
    assert_eq!(refusal.line, SALE_NOT_PILOT.replace("{name}", "Salt Moth"));
    let _ = (QuoteId(0), hold_b);
}

// ---------------------------------------------------------------------------
// 13. An intake cut over an exhausted bid
// ---------------------------------------------------------------------------

/// **An intake cut over an exhausted bid leaves it at zero** (decision 22): the
/// intake-20 variant sold whole, then an update to intake 10 applies (no "out of
/// range"), the listing stands at 0 and the `BidRow` records intake 10.
#[test]
fn ic10_an_intake_cut_over_an_exhausted_bid_leaves_it_at_zero() {
    let base = lane_variant(LANE_VERSION, |doc| {
        kasei_metals_bid(doc)["intake"] = serde_json::json!(20);
    });
    let mut next_doc: serde_json::Value = serde_json::from_str(&base).expect("JSON");
    next_doc["manifest"]["version"] = serde_json::json!(LANE_VERSION + 1);
    next_doc["manifest"]["digest"] = serde_json::json!(UNSIGNED);
    kasei_metals_bid(&mut next_doc)["intake"] = serde_json::json!(10);
    let next_json = sign(&serde_json::to_string_pretty(&next_doc).expect("serializes"));
    let next = mush_content::load_pack(&next_json).expect("the variant verifies");
    let mut roster = Roster::seeded(base);
    roster.shelf = Some(vec![next]);
    roster.extra_lane = Some(next_json);
    let mut rig = stocked_with(roster);
    let (pilot, _) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let bid = kasei_bid(&rig.tw, "metals");
    let _ = run(&mut rig.tw, pilot, &format!("+market/sell {bid}=20"));
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    assert_eq!(bid_qty(&rig.tw, bid), Some(0));
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@{}", LANE_VERSION + 1)),
        "{plan}\n{apply}"
    );
    assert!(!apply.contains("out of range"), "{apply}");
    assert_eq!(bid_qty(&rig.tw, bid), Some(0), "it stands exhausted");
    let row =
        port_map(&rig.tw, "stickney_port").vendors[&cid("kasei")].bids[&cid("metals")].clone();
    assert_eq!(row.intake, 10);
    assert_eq!(row.listing, bid.0);
}

// ---------------------------------------------------------------------------
// Doc 18 cut IC14 row 9: a boot-healed settlement folds once
// ---------------------------------------------------------------------------

/// **A settlement the economy healed at boot folds into the enrolment once** (doc 18
/// cut IC14 row 9; lead ruling L4). The real state: the economy settled and the market
/// never heard, so **neither** the quote's `settled` stamp **nor** the enrolment fold
/// ran. The rig reaches it by route (b): settle normally, then rewrite the folded store
/// — the quote's `settled` to `null`, the enrolment's `sales`, `margin_minor` and
/// `shares_minor` back to the pre-sale baseline — and reboot. The market's boot
/// reconcile stamps the quote; its first sweep tells the proceeds policy, once: the
/// counters read the baseline plus exactly this sale's figures (read from the
/// contract's own journal, never typed), and a third sweep and a second reboot fold
/// nothing more.
#[test]
fn ic14_a_settlement_healed_at_boot_folds_once() {
    let mut rig = stocked();
    let (pilot, _row) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let baseline = enrolment_of(&rig.tw, pilot);
    let base = (baseline.sales, baseline.margin_minor, baseline.shares_minor);
    let bid = kasei_bid(&rig.tw, "metals");
    let _ = run(&mut rig.tw, pilot, &format!("+market/sell {bid}=20"));
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let sale = last_sale(&rig.tw, pilot);
    let contract = sale.contract.expect("contract");
    settle(&mut rig, contract);
    // The sale's own figures, from the settled contract and the frozen quote.
    let share: i64 = settlement_lines(&rig.tw, contract)
        .iter()
        .filter(|(account, _)| *account == trust_account(&rig.tw))
        .map(|(_, delta)| *delta)
        .sum();
    let margin = sale.basis.minor();
    let folded = enrolment_of(&rig.tw, pilot);
    assert_eq!(
        (folded.sales, folded.margin_minor, folded.shares_minor),
        (base.0 + 1, base.1 + margin, base.2 + share),
        "the live settlement folded"
    );

    // Route (b): the economy's settlement stands; the market's stamp and the fold go.
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    let quote_key = format!("market:quote:{}", sale.id.0);
    let enrolment_key = tacsim::port::coop::enrolment_key(folded.account);
    {
        let market_docs = rig.store.get_mut(mush_market::NS).expect("market docs");
        let mut found = false;
        for (key, value) in &mut market_docs.kv {
            if key.as_str() == quote_key {
                value["settled"] = serde_json::Value::Null;
                found = true;
            }
        }
        assert!(found, "the quote document");
        let tacsim_docs = rig.store.get_mut(tacsim::NS).expect("tacsim docs");
        let mut found = false;
        for (key, value) in &mut tacsim_docs.kv {
            if *key == enrolment_key {
                value["sales"] = serde_json::json!(base.0);
                value["margin_minor"] = serde_json::json!(base.1);
                value["shares_minor"] = serde_json::json!(base.2);
                found = true;
            }
        }
        assert!(found, "the enrolment document");
    }
    let mut rig = reboot(rig);
    assert!(
        market(&rig.tw)
            .quote(sale.id)
            .is_some_and(|q| q.settled.is_some()),
        "the boot reconcile stamped the healed settlement"
    );
    let at_boot = enrolment_of(&rig.tw, pilot);
    assert_eq!(
        (at_boot.sales, at_boot.margin_minor, at_boot.shares_minor),
        base,
        "nothing folds inside the market's boot (the game's store is not hydrated yet)"
    );
    sweep(&mut rig.tw, 61);
    sweep(&mut rig.tw, 61);
    let healed = enrolment_of(&rig.tw, pilot);
    assert_eq!(
        (healed.sales, healed.margin_minor, healed.shares_minor),
        (base.0 + 1, base.1 + margin, base.2 + share),
        "folded once on the first sweep"
    );
    sweep(&mut rig.tw, 61);
    let mut rig = reboot(rig);
    sweep(&mut rig.tw, 61);
    let again = enrolment_of(&rig.tw, pilot);
    assert_eq!(
        (again.sales, again.margin_minor, again.shares_minor),
        (healed.sales, healed.margin_minor, healed.shares_minor),
        "a third sweep and a second reboot fold nothing more"
    );
    assert_eq!(refused_settlements(&rig.tw), 0);
}
