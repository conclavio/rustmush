//! **Doc 18 cut IC11 acceptance — captain's orders, the first cut with the full
//! daily loop** (doc 18 §4.4 "The queued-orders model", "Offline execution",
//! "Failure handling", §2 steps 10–17; D-18-31, D-18-32, D-18-41; lead rulings
//! L1–L29 of the IC11 brief).
//!
//! A chartered pilot files the day's sailing plan with `+ship/order …`, reads it
//! with `+ship/orders`, dry-runs it with `+ship/plan`, and logs off; the
//! `tacsim.port` tick's order arm executes it **as the pilot**, one closed typed
//! call per step (`mush_market::quote` + `accept_at`, `sell_into_bid`, the leg
//! re-stages, or the `tacsim_port_burn` submission), and verifies every step by
//! pull. Every money and tonnage figure is computed here from the posted prices,
//! the goods table, the hull and the planner (the `ic_slice_loop.rs` discipline),
//! except where the §5 row gives the literal and the tree reproduces it (36 t, 37 t,
//! 20 crew-days, 73 t, 20 t "funds" at lane v4; 19 crew-days at v3).
//!
//! Composed on the composition root (`server_modules_with_tacsim` and the wired
//! test-world layer with game content: `sale.rs`'s `Roster`, with the sky rate a
//! parameter for the not-before lens), so enrolment needs no rate 24.

mod support;

#[path = "support/vertex.rs"]
mod vertex;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use mush_core::{
    AccountId, AccountIdentity, ActionStateBootRecord, Audience, AuthoritativeSubmit, CanonicalId,
    CommitToken, GOD, IdempotencyKey, IdempotencyScope, ModuleWrite, ObjId, OfferExecutionContext,
    OfferSubmission, RequestUid, ScopedIdempotencyKey,
};
use mush_economy::{ContractId, ContractState, EconomyConfig, EconomyStore, VaultId};
use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
use mush_market::{ListingId, MarketStore, PaidKind, VenueGate};
use mush_org::PrincipalId;
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops_into};
use smol_str::SmolStr;
use tacsim::config::PackSource;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK, LANE_VERSION};
use tacsim::pack::{PackEntry, load_pack};
use tacsim::port::apply::room_by_key;
use tacsim::port::executor::{
    BOUGHT, BOUGHT_FILL_LOCKER, BOUGHT_FILL_TANK, BOUGHT_NONE, BOUGHT_SHORT, DEPART_SET,
    HELD_ASHORE, HELD_LINE, HELD_STOOD_DOWN, HELD_UNCONFIRMED, HELD_WINDOW, HOUSE_CANNOT_PAY_SKIP,
    LEFT_BEHIND_PASSENGER, LEFT_BEHIND_PILOT, NOT_ADMITTED, ORDER_RESTAGE_SECS, SKIP_LINE, STOPPED,
    WAITING_LINE,
};
use tacsim::port::orders::{
    AHEAD_OF_INFLIGHT, CLEARED, DELETED, FILL_ONLY_CONSUMABLE, MOVED, NO_PLAN, NOT_HELD,
    NOT_ORDERABLE, ORDER_AFTER_DEPART, ORDER_FINISHED, ORDER_FIRES_NEXT, ORDER_INFLIGHT,
    ORDERS_DARK, ORDERS_FULL, ORDERS_WHERE, PAUSED, RESUMED, SKIPPED, UNKNOWN_GOOD, UNKNOWN_PORT,
};
use tacsim::port::voyage::{
    CLEARANCE_GRANTED, CLEARANCE_LAPSED, CLEARANCE_STOOD_DOWN, NO_ROUTE, NOTHING_WRITTEN,
    TANK_SHORT, VIA_UNKNOWN, Voyage, VoyageState,
};
use tacsim::port::when::BAD_TIME;
use tacsim::port::{
    HullRow, OrderState, Orders, OrdersState, PlanState, PortDef, PortHandles, PortMap,
    PortMapState, PortVenueGate, Ticket, TicketId, orders_key, room_key,
};
use tacsim::port_tick::{NOTICE_HEADING, pending_notices};
use tacsim::{TacsimConfig, TacsimStore};

use support::ground::{UNSIGNED, sign};

// ---------------------------------------------------------------------------
// The rig (sale.rs's roster and flight, voyage.rs's reboot and disconnect)
// ---------------------------------------------------------------------------

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

fn lane_id() -> CanonicalId {
    cid(LANE_PACK)
}

/// `sale.rs`'s roster — the shipped lane or an operator seed in its place — with
/// `[tacsim.time.ephemeris]`'s rate a parameter (the not-before lens).
#[derive(Clone, Default)]
struct Roster {
    seed_lane: Option<String>,
    sky_rate: u32,
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
        cfg.time.ephemeris.rate = self.sky_rate;
        let game_content = self.seed_lane.is_none();
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
                let mut packs = comp::boot_pack_list(true, true).expect("the boot list");
                packs.retain(|p| p.manifest().id().as_str() != LANE_PACK);
                packs.push(mush_content::load_pack(lane).expect("the seed lane verifies"));
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
    /// Every idempotency key landed so far: a re-staged leg under a key that already
    /// landed is the engine's replay HIT, which this rig's request-keyed landing
    /// cannot model, so [`land_all`] passes over its refusal (and only its).
    keys: std::collections::BTreeSet<String>,
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
            mush_core::OfferSecret::new(1, [0x6b; 32]).expect("an offer secret"),
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
        keys: std::collections::BTreeSet::new(),
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
    reboot_editing(rig, |_| {})
}

/// [`reboot`], the folded store edited first (`voyage.rs`'s corrupt-document reboot).
fn reboot_editing(
    rig: Rig,
    edit: impl FnOnce(&mut HashMap<SmolStr, mush_core::ModuleBootData>),
) -> Rig {
    let Rig {
        mut tw,
        oracle,
        roster,
        mut store,
        typed,
        keys,
    } = rig;
    fold_module_ops_into(&mut store, tw.take_module_ops());
    edit(&mut store);
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
        keys,
    }
}

/// Edit the folded `tacsim:orders:{hull}` document's JSON in place.
fn edit_orders(
    folded: &mut HashMap<SmolStr, mush_core::ModuleBootData>,
    hull: ObjId,
    edit: impl FnOnce(&mut serde_json::Value),
) {
    let key = orders_key(hull);
    let doc = folded
        .get_mut(tacsim::NS)
        .and_then(|data| data.kv.iter_mut().find(|(k, _)| *k == key))
        .expect("the sailing plan document");
    edit(&mut doc.1);
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

fn p01(tw: &TestWorld) -> ObjId {
    station_room(tw, "stickney_port", "room.p01_berth_gallery")
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
    let mut bytes = [0x6b_u8; 16];
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
    rig.keys.insert(submit.key.as_str().to_owned());
    Ok(())
}

/// Every queued submission of the port's lanes, dropping other modules'; asserts
/// none kept of tacsim's is GOD's.
fn drain(tw: &mut TestWorld) -> Vec<AuthoritativeSubmit> {
    let mut kept = Vec::new();
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        let owner = submit.action.key().owner.as_str();
        assert_ne!(submit.actor, GOD, "GOD never submits from the port tick");
        if owner == tacsim::NS || owner == "inventory" || owner == "market" {
            kept.push(*submit);
        }
    }
    kept
}

fn land_all(rig: &mut Rig, queued: &[AuthoritativeSubmit]) {
    for submit in queued {
        let replay = rig.keys.contains(submit.key.as_str());
        land(rig, submit).unwrap_or_else(|error| {
            if replay {
                return;
            }
            panic!(
                "{} by #{} ({}) did not land: {error}",
                submit.action.key().id,
                submit.actor.0,
                submit.key.as_str()
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

/// `n` fires, every queued tacsim, market and inventory submission landed (and
/// returned) — the sweeps run with them, since each fire moves the clock 6 s.
fn drive(rig: &mut Rig, n: usize) -> Vec<AuthoritativeSubmit> {
    let mut landed = Vec::new();
    for _ in 0..n {
        let (_, queued) = fire(rig);
        land_all(rig, &queued);
        landed.extend(queued);
    }
    landed
}

/// Fire and land until `done`, at most `max` fires; the landed submissions.
fn drive_until(
    rig: &mut Rig,
    max: usize,
    mut done: impl FnMut(&Rig) -> bool,
) -> Vec<AuthoritativeSubmit> {
    let mut landed = Vec::new();
    for _ in 0..max {
        if done(rig) {
            return landed;
        }
        let (_, queued) = fire(rig);
        land_all(rig, &queued);
        landed.extend(queued);
    }
    assert!(done(rig), "the condition did not hold within {max} fires");
    landed
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
    let out = status(&mut rig.tw);
    panic!("the ports were not stocked within 40 fires:\n{out}");
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

/// **Provision the hold for `dest` from the planner's own figures** (`sale.rs`'s).
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
/// every queued submission; the landed ones.
fn to_ic(rig: &mut Rig, target: i64) -> Vec<AuthoritativeSubmit> {
    let rate = i64::from(world_rate(&rig.tw).max(1));
    let ahead = target.saturating_sub(ic_now(&rig.tw)).max(0);
    let real = (ahead + rate - 1) / rate;
    let t = rig.tw.world().now().saturating_add(real);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    land_all(rig, &queued);
    queued
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

/// Board from a gallery (a `+ship/board` from the gallery cycles the lock into the
/// Airlock).
fn board_at(rig: &mut Rig, pilot: ObjId, name: &str, gallery: ObjId) {
    teleport(&mut rig.tw, pilot, gallery);
    let out = run(&mut rig.tw, pilot, "+ship/board");
    assert!(
        out.contains(&format!("You cycle the lock into the {name}.")),
        "{out}"
    );
}

fn board(rig: &mut Rig, pilot: ObjId, name: &str) {
    let gallery = c01(&rig.tw);
    board_at(rig, pilot, name, gallery);
}

/// **Fly `(pilot, row, name)` to Stickney at rate 3600 by hand** and put the pilot
/// ashore in P07 (`sale.rs`'s flight). The world rate stays at 3600.
fn fly_to_stickney(rig: &mut Rig, pilot: ObjId, row: &HullRow, name: &str) {
    set_rate(&mut rig.tw, 3_600);
    board(rig, pilot, name);
    let out = run(&mut rig.tw, pilot, "+ship/undock stickney");
    assert!(out.contains("Clearance filed for Stickney Yards"), "{out}");
    let queued = drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "one burn: {out}");
    land_all(rig, &queued);
    let _ = drive(rig, 1);
    let t = ticket(&mut rig.tw, ObjId(row.hull), 1).expect("a ticket");
    let arrival = t.plan.depart_ic + t.plan.total_s;
    let _ = to_ic(rig, arrival - 3_600);
    let _ = to_ic(rig, arrival);
    let _ = to_ic(rig, arrival + 3_600);
    match tacsim::port::ship::berth_of(rig.tw.world(), ObjId(row.hull)) {
        tacsim::port::ship::Berth::Docked { def, .. } => {
            assert_eq!(def.venue.as_str(), "stickney");
        }
        other => panic!("the hull did not dock at Stickney: {other:?}"),
    }
    assert_eq!(location(&rig.tw, pilot), Some(room_of(row, "airlock")));
    let out = run(&mut rig.tw, pilot, "+ship/leave");
    assert!(out.contains("You cycle the lock out into"), "{out}");
    let receiving = p07(&rig.tw);
    teleport(&mut rig.tw, pilot, receiving);
}

/// **A chartered pilot standing in P07 with the hull docked at Stickney**, carrying
/// `cargo` (each good seeded with its own rooted lot), provisioned for the passage
/// from the planner's figures (so the tank and the locker are spent by the flight).
fn docked_at_stickney(rig: &mut Rig, cargo: &[(&str, u64)]) -> (ObjId, HullRow) {
    let (pilot, row) = chartered(rig, "Pilot", 1, "Salt Moth");
    let vault = hold_vault(&rig.tw, &row);
    for (n, (good, qty)) in cargo.iter().enumerate() {
        seed_pool(rig, vault, good, *qty, LotId(901 + n as u64));
    }
    provision(rig, pilot, &row, "stickney", 910);
    fly_to_stickney(rig, pilot, &row, "Salt Moth");
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

/// A vendor's JSON in a lane document.
fn vendor<'a>(doc: &'a mut serde_json::Value, entry: &str, org: &str) -> &'a mut serde_json::Value {
    doc["entries"][entry]["def"]["vendors"]
        .as_array_mut()
        .expect("vendors")
        .iter_mut()
        .find(|v| v["org"] == org)
        .expect("the vendor")
}

fn disconnect(tw: &mut TestWorld, who: ObjId) {
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    let conns: Vec<u64> = (1..=64)
        .filter(|conn| tw.engine().actor_of(*conn) == Some(who))
        .collect();
    assert!(!conns.is_empty(), "{who} holds a connection");
    for conn in conns {
        tw.engine_mut().step(
            mush_core::Inbound::ConnDown {
                conn,
                actor: who,
                reason: mush_core::DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
    }
    tw.engine_mut().step(
        mush_core::Inbound::SessionGone { actor: who },
        &mut out,
        &mut fx,
    );
}

// ---- the books -----------------------------------------------------------------

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

/// An account's available balance (posted − held), minor units.
fn available(tw: &TestWorld, account: mush_economy::AccountId) -> i64 {
    let a = economy(tw)
        .snapshot()
        .account(account)
        .expect("the account")
        .clone();
    a.posted_balance.minor() - a.held_amount.minor()
}

fn pilot_available(tw: &TestWorld, pilot: ObjId) -> i64 {
    available(tw, account_of(tw, &PrincipalId::Object(pilot)))
}

fn state_of(tw: &TestWorld, c: ContractId) -> Option<ContractState> {
    economy(tw).snapshot().contract(c).map(|c| c.state)
}

fn kasei_bid(tw: &TestWorld, good: &str) -> ListingId {
    ListingId(port_map(tw, "stickney_port").vendors[&cid("kasei")].bids[&cid(good)].listing)
}

fn bid_qty(tw: &TestWorld, id: ListingId) -> Option<u64> {
    market(tw).listing(id).map(|l| l.quantity)
}

fn listing_of(tw: &TestWorld, entry: &str, org: &str, good: &str) -> ListingId {
    let vault = VaultId(port_map(tw, entry).vendors[&cid(org)].vault);
    market(tw)
        .lot_listing_of(EndpointRef(vault), &cid(good))
        .unwrap_or_else(|| panic!("{entry}/{org} lists {good}"))
}

fn price_of(tw: &TestWorld, listing: ListingId) -> i64 {
    market(tw)
        .listing(listing)
        .map(|l| l.price.minor())
        .expect("the listing")
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
    let price = price_of(&rig.tw, listing);
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

/// [`buy`] from `entry`'s `org`'s ask for `good`.
fn buy_from(rig: &mut Rig, pilot: ObjId, entry: &str, org: &str, good: &str, qty: u64) -> i64 {
    let listing = listing_of(&rig.tw, entry, org, good);
    buy(rig, pilot, listing, qty)
}

/// Sweep until every Draw the pilot bought reads `drawn_at` (the market mirror).
fn swept_drawn(rig: &mut Rig, pilot: ObjId) {
    for _ in 0..10 {
        let all = market(&rig.tw)
            .quotes_of_buyer(&PrincipalId::Object(pilot))
            .all(|q| q.drawn_at.is_some() || q.settled.is_some());
        if all {
            return;
        }
        sweep(&mut rig.tw, 61);
    }
    panic!("the hand-bought Draws were not drawn");
}

/// The money render (`5600.00cr`).
fn cr(minor: i64) -> String {
    format!("{}.{:02}cr", minor / 100, minor % 100)
}

/// A unix second's UTC wall time, `HH:MM`.
fn hhmm(unix: i64) -> String {
    let of_day = unix.rem_euclid(86_400);
    format!("{:02}:{:02}", of_day / 3_600, (of_day % 3_600) / 60)
}

/// The hull's readable sailing plan.
fn orders(tw: &TestWorld, hull: ObjId) -> Orders {
    match store(tw).orders_state(hull) {
        OrdersState::Present(orders) => orders.clone(),
        OrdersState::Absent => panic!("no sailing plan"),
        OrdersState::Unreadable => panic!("the sailing plan is unreadable"),
    }
}

/// The plan's log texts, oldest first.
fn log_texts(tw: &TestWorld, hull: ObjId) -> Vec<String> {
    orders(tw, hull)
        .log
        .iter()
        .map(|l| l.text.clone())
        .collect()
}

/// One order's state.
fn order_state(tw: &TestWorld, hull: ObjId, id: u32) -> Option<OrderState> {
    match store(tw).orders_state(hull) {
        OrdersState::Present(orders) => orders.order(id).map(|o| o.state),
        OrdersState::Absent | OrdersState::Unreadable => None,
    }
}

/// `+ship/order <line>` as `who`.
fn order(rig: &mut Rig, who: ObjId, line: &str) -> String {
    run(&mut rig.tw, who, &format!("+ship/order {line}"))
}

/// Whether `hull` is on a journey now.
fn under_way(tw: &TestWorld, hull: ObjId) -> bool {
    matches!(
        tacsim::port::ship::berth_of(tw.world(), hull),
        tacsim::port::ship::Berth::UnderWay
    )
}

/// The journey `hull` is on (or last flew: an arrived journey stays in the book),
/// as `(route, departed IC)`.
fn journey_of(tw: &TestWorld, hull: ObjId) -> Option<(CanonicalId, i64)> {
    tw.world()
        .module_state::<mush_sim::JourneyBook>()
        .and_then(|book| book.get(hull))
        .map(|j| (j.route().id().clone(), j.departed().at))
}

/// The vertex load of metals and ice (`support/vertex.rs`; never the literals).
fn vertex_load() -> (u64, u64) {
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
    vertex::both_limits_vertex(&goods, &compartment, &cid("metals"), &cid("volatiles"))
        .expect("an integral vertex")
}

/// Every burn key landed among `landed`.
fn burn_keys(landed: &[AuthoritativeSubmit]) -> Vec<String> {
    landed
        .iter()
        .filter(|s| s.action.key().id.as_str() == tacsim::port::BURN_ACTION)
        .map(|s| s.key.as_str().to_owned())
        .collect()
}

// ---------------------------------------------------------------------------
// 1. The verbs (item 3)
// ---------------------------------------------------------------------------

/// **Every sub-verb edits the plan** (doc 18 §4.4 "Verbs"; the IC11 brief's item 3):
/// the replies and the document's effect, `rev` bumped by each edit and never by
/// `+ship/orders`, the render's lines, the refusals, the 16-order rule.
#[test]
#[allow(clippy::too_many_lines)]
fn ic11_the_verbs_edit_the_plan() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    let out = run(&mut rig.tw, pilot, "+ship/orders");
    assert!(
        out.contains(&NO_PLAN.replace("{name}", "Salt Moth")),
        "{out}"
    );

    // Where (Q-18-28): the Slack Line is no port room by the gate's own rule, so a
    // filing there is refused and writes nothing; the dockmaster's office is one.
    let slack = station_room(&rig.tw, "ceres_port", "room.c10_slack_line");
    assert_eq!(PortVenueGate.venue_of_room(rig.tw.world(), slack), None);
    teleport(&mut rig.tw, pilot, slack);
    let out = order(&mut rig, pilot, "stop");
    assert!(
        out.contains(&ORDERS_WHERE.replace("{name}", "Salt Moth")),
        "{out}"
    );
    assert!(matches!(
        store(&rig.tw).orders_state(hull),
        OrdersState::Absent
    ));
    let dockmaster = station_room(&rig.tw, "ceres_port", "room.c14_dockmaster");
    assert!(
        PortVenueGate
            .venue_of_room(rig.tw.world(), dockmaster)
            .is_some()
    );
    teleport(&mut rig.tw, pilot, dockmaster);
    let out = order(&mut rig, pilot, "sell metals all min=250");
    assert!(
        out.contains(
            "Order 1 filed: sell metal concentrate, all, not under 250.00cr/t. The sailing plan is at revision 1."
        ),
        "{out}"
    );
    assert!(out.contains(&ORDER_FIRES_NEXT.replace("{n}", "1")), "{out}");
    assert!(!out.contains(ORDER_AFTER_DEPART), "{out}");
    let exchange = c07(&rig.tw);
    teleport(&mut rig.tw, pilot, exchange);
    let at = rig.tw.world().now() + 7_200;
    let out = order(&mut rig, pilot, "depart stickney at=+2h until=+1h");
    assert!(
        out.contains(&format!(
            "Order 2 filed: depart for Stickney Yards at {} UTC ",
            hhmm(at)
        )) && out.contains(", until 1 h 0 min after. The sailing plan is at revision 2."),
        "{out}"
    );
    assert!(!out.contains("fires on the next tick"), "{out}");
    let out = order(
        &mut rig,
        pilot,
        "buy structural_alloys 55 max=400 short=skip",
    );
    assert!(
        out.contains(
            "Order 3 filed: buy structural alloy stock, 55 t, not over 400.00cr/t, if short skip. The sailing plan is at revision 3."
        ),
        "{out}"
    );
    assert!(out.contains(ORDER_AFTER_DEPART), "{out}");
    let out = order(&mut rig, pilot, "buy propellant fill");
    assert!(
        out.contains("Order 4 filed: buy reaction mass, fill. The sailing plan is at revision 4."),
        "{out}"
    );
    // The render, and `+ship/orders` writes nothing.
    let mark = row.mark.clone();
    let out = run(&mut rig.tw, pilot, "+ship/orders");
    for want in [
        format!("The Salt Moth's sailing plan ({mark}), revision 4: running."),
        "  1. sell metal concentrate, all, not under 250.00cr/t — next".to_owned(),
        "  3. buy structural alloy stock, 55 t, not over 400.00cr/t, if short skip — pending, at Stickney Yards"
            .to_owned(),
        "  4. buy reaction mass, fill — pending, at Stickney Yards".to_owned(),
        "Next due: order 1 on the next tick.".to_owned(),
        "Log: nothing yet.".to_owned(),
    ] {
        assert!(out.lines().any(|l| l == want), "missing {want:?}:\n{out}");
    }
    assert!(
        out.lines()
            .any(|l| l.starts_with("  2. depart for Stickney Yards at ")
                && l.ends_with(" — pending")),
        "{out}"
    );
    assert_eq!(orders(&rig.tw, hull).rev, 4);

    // Refusals write nothing.
    for (line, want) in [
        (
            "depart stickney via=nowhere",
            VIA_UNKNOWN
                .replace("{via}", "nowhere")
                .replace("{list}", "Ahuna Flank, Aitken Highport, Ceres Freeport, Freeport Commons, Juling Shadow, Kerwan Floor, Kirkwood Depot, Ligeia Terminal, Occator Rim, Rheasilvia Commons, Stickney Yards, Voltaire Anchorage"),
        ),
        (
            "depart stickney throttle=100",
            tacsim::port::THROTTLE_USAGE
                .replace("{min}", "250")
                .replace("{max}", "1000"),
        ),
        ("depart stickney at=+169h", BAD_TIME.to_owned()),
        ("buy metals fill", FILL_ONLY_CONSUMABLE.to_owned()),
        ("board", NOT_ORDERABLE.to_owned()),
        (
            "buy unobtainium 5",
            UNKNOWN_GOOD.replace(
                "{list}",
                // Doc 20 cut SM.3a: goods v3's four ores close the list.
                "metals, volatiles, structural_alloys, industrial_parts, propellant, provisions, \
                 instrument_boards, carbon_feedstock, metal_ore, ice_ore, carbon_ore, \
                 strategic_ore",
            ),
        ),
        (
            "depart mars",
            UNKNOWN_PORT.replace(
                "{list}",
                "Ahuna Flank, Ceres Freeport, Freeport Commons, Voltaire Anchorage, Juling Shadow, Kerwan Floor, Aitken Highport, Occator Rim, Rheasilvia Commons, Stickney Yards, Ligeia Terminal",
            ),
        ),
    ] {
        let out = order(&mut rig, pilot, line);
        assert!(out.contains(&want), "`{line}`: {out}");
    }
    assert_eq!(orders(&rig.tw, hull).rev, 4, "the refusals wrote nothing");

    // move, del, skip, pause, resume.
    let out = order(&mut rig, pilot, "move 4=3");
    assert!(
        out.contains(&MOVED.replace("{n}", "4").replace("{pos}", "3")),
        "{out}"
    );
    let ids: Vec<u32> = orders(&rig.tw, hull).orders.iter().map(|o| o.id).collect();
    assert_eq!(ids, vec![1, 2, 4, 3]);
    let out = order(&mut rig, pilot, "del 3");
    assert!(out.contains(&DELETED.replace("{n}", "3")), "{out}");
    let out = order(&mut rig, pilot, "skip 1");
    assert!(out.contains(&SKIPPED.replace("{n}", "1")), "{out}");
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.order(1).map(|o| o.state), Some(OrderState::Skipped));
    assert_eq!(doc.cursor, Some(2));
    assert_eq!(doc.rev, 7);
    let out = order(&mut rig, pilot, "pause");
    assert!(out.contains(PAUSED), "{out}");
    let out = run(&mut rig.tw, pilot, "+ship/orders");
    assert!(
        out.contains("Next due: nothing while the plan is paused (order 2 is next)."),
        "{out}"
    );
    let out = order(&mut rig, pilot, "resume");
    assert!(out.contains(&RESUMED.replace("{n}", "2")), "{out}");
    let out = order(&mut rig, pilot, "resume");
    assert!(out.contains(NOT_HELD), "{out}");
    let out = run(&mut rig.tw, pilot, "+ship/orders");
    assert!(
        out.contains("Next due: order 2, departure for Stickney Yards, "),
        "{out}"
    );
    assert!(out.contains("; in 2 h 0 min)"), "{out}");

    // Sixteen orders: a 17th drops the skipped one; then sixteen open refuse.
    for _ in 0..13 {
        let out = order(&mut rig, pilot, "stop");
        assert!(out.contains("filed: stop."), "{out}");
    }
    assert_eq!(orders(&rig.tw, hull).orders.len(), 16);
    let out = order(&mut rig, pilot, "stop");
    assert!(out.contains("filed: stop."), "{out}");
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.orders.len(), 16);
    assert!(doc.order(1).is_none(), "the skipped order made room");
    let out = order(&mut rig, pilot, "stop");
    assert!(out.contains(ORDERS_FULL), "{out}");
    let out = order(&mut rig, pilot, "clear");
    assert!(out.contains(CLEARED), "{out}");
    assert!(orders(&rig.tw, hull).orders.is_empty());
    let out = order(&mut rig, pilot, "move 9=1");
    assert!(out.contains("The sailing plan has no order 9."), "{out}");

    // A held order: del and skip clear its note, and the plan runs again.
    for line in ["buy volatiles 5", "buy volatiles 6"] {
        let _ = order(&mut rig, pilot, line);
    }
    let held = |rig: Rig, id: u32| {
        reboot_editing(rig, |folded| {
            edit_orders(folded, hull, |doc| {
                doc["state"] = serde_json::json!("held");
                doc["hold"] = serde_json::json!({
                    "order": id, "reason": "no house here buys water ice.", "since": 1, "retry": false
                });
                for order in doc["orders"].as_array_mut().expect("orders") {
                    if order["id"] == id {
                        order["state"] = serde_json::json!("held");
                    }
                }
            });
        })
    };
    let first = orders(&rig.tw, hull).cursor.expect("a cursor");
    let mut rig = held(rig, first);
    assert_eq!(orders(&rig.tw, hull).state, PlanState::Held);
    let _ = rig.tw.connect(pilot);
    // The render of a stopped hold: the state word, the next-due form, the note.
    let out = run(&mut rig.tw, pilot, "+ship/orders");
    for want in [
        format!("  {first}. buy water ice, 5 t — held"),
        format!("Next due: nothing while order {first} is held."),
        "Held: no house here buys water ice.".to_owned(),
    ] {
        assert!(out.lines().any(|l| l == want), "missing {want:?}:\n{out}");
    }
    let out = order(&mut rig, pilot, &format!("del {first}"));
    assert!(
        out.contains(&DELETED.replace("{n}", &first.to_string())),
        "{out}"
    );
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.state, PlanState::Running);
    assert!(doc.hold.is_none());
    doc.check().expect("the plan still checks");
    let second = doc.cursor.expect("a cursor");
    let mut rig = held(rig, second);
    let out = order(&mut rig, pilot, &format!("skip {second}"));
    assert!(
        out.contains(&SKIPPED.replace("{n}", &second.to_string())),
        "{out}"
    );
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.state, PlanState::Running);
    assert!(doc.hold.is_none());
    doc.check().expect("the plan still checks");
    // A skipped order and no cursor: the plan is done.
    let out = run(&mut rig.tw, pilot, "+ship/orders");
    for want in [
        format!("  {second}. buy water ice, 6 t — skipped"),
        "Next due: nothing; the plan is done.".to_owned(),
    ] {
        assert!(out.lines().any(|l| l == want), "missing {want:?}:\n{out}");
    }
    // A skip of an order already passed over is refused, writing nothing.
    let rev = orders(&rig.tw, hull).rev;
    let out = order(&mut rig, pilot, &format!("skip {second}"));
    assert!(
        out.contains(&ORDER_FINISHED.replace("{n}", &second.to_string())),
        "{out}"
    );
    assert_eq!(orders(&rig.tw, hull).rev, rev, "nothing written");

    // A done order and a waiting note: the state words, the waiting line; `del` of
    // the waiting order drops its note.
    let _ = order(&mut rig, pilot, "buy volatiles 8");
    let third = orders(&rig.tw, hull).cursor.expect("a cursor");
    let mut rig = reboot_editing(rig, |folded| {
        edit_orders(folded, hull, |doc| {
            doc["hold"] = serde_json::json!({
                "order": third, "reason": "your charter of the Salt Moth is no longer in force.",
                "since": 1, "retry": true
            });
            for order in doc["orders"].as_array_mut().expect("orders") {
                if order["id"] == second {
                    order["state"] = serde_json::json!("done");
                }
            }
        });
    });
    let _ = rig.tw.connect(pilot);
    let out = run(&mut rig.tw, pilot, "+ship/orders");
    for want in [
        format!("  {second}. buy water ice, 6 t — done"),
        format!("  {third}. buy water ice, 8 t — next"),
        format!("Next due: order {third} on the next tick."),
        "Waiting: your charter of the Salt Moth is no longer in force.".to_owned(),
    ] {
        assert!(out.lines().any(|l| l == want), "missing {want:?}:\n{out}");
    }
    let out = order(&mut rig, pilot, &format!("del {third}"));
    assert!(
        out.contains(&DELETED.replace("{n}", &third.to_string())),
        "{out}"
    );
    let doc = orders(&rig.tw, hull);
    assert!(doc.hold.is_none(), "the waiting note went with its order");
    doc.check().expect("the plan still checks");

    // The inflight order is untouchable by del, move, skip and clear.
    let _ = order(&mut rig, pilot, "buy volatiles 7");
    let _ = order(&mut rig, pilot, "stop");
    let doc = orders(&rig.tw, hull);
    let inflight = doc.cursor.expect("a cursor");
    let mut rig = reboot_editing(rig, |folded| {
        edit_orders(folded, hull, |doc| {
            for order in doc["orders"].as_array_mut().expect("orders") {
                if order["id"] == inflight {
                    order["state"] = serde_json::json!("inflight");
                }
            }
            doc["inflight"] = serde_json::json!({
                "order": inflight, "attempt": 1, "key": "market.lot.draw.1",
                "refs": {"quote": null, "contract": null, "ticket": null},
                "staged_utc": 1, "issued_ic": 1, "restaged": 0, "want": 7, "qty": 7,
                "bounds": [], "line": null
            });
        });
    });
    let _ = rig.tw.connect(pilot);
    let n = inflight.to_string();
    let out = run(&mut rig.tw, pilot, "+ship/orders");
    for want in [
        format!("  {n}. buy water ice, 7 t — at the counter"),
        format!("Next due: order {n} is at the counter."),
    ] {
        assert!(out.lines().any(|l| l == want), "missing {want:?}:\n{out}");
    }
    for line in [
        format!("del {n}"),
        format!("skip {n}"),
        format!("move {n}=2"),
    ] {
        let out = order(&mut rig, pilot, &line);
        assert!(
            out.contains(&ORDER_INFLIGHT.replace("{n}", &n)),
            "`{line}`: {out}"
        );
    }
    let stop = orders(&rig.tw, hull)
        .orders
        .last()
        .map(|o| o.id)
        .expect("the stop");
    let out = order(&mut rig, pilot, &format!("move {stop}=1"));
    assert!(
        out.contains(
            &AHEAD_OF_INFLIGHT
                .replace("{n}", &stop.to_string())
                .replace("{m}", &n)
        ),
        "{out}"
    );
    let out = order(&mut rig, pilot, "clear");
    assert!(
        out.contains(&format!("{CLEARED} Order {n}, being carried out, stays.")),
        "{out}"
    );
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.orders.len(), 1);
    assert_eq!(
        doc.order(inflight).map(|o| o.state),
        Some(OrderState::Inflight)
    );
}

/// **A new tenant files a fresh plan** (lead ruling L14): another author's document
/// reads as no plan, and the first edit starts at revision 1 with ids past the old.
#[test]
fn ic11_a_new_tenant_files_a_fresh_plan() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    for line in ["buy volatiles 5", "stop", "stop"] {
        let _ = order(&mut rig, pilot, line);
    }
    let other = rig.tw.spawn_player("Previous");
    // Another author's plan whose step is still in flight: every edit waits for it
    // (gate 3), writing nothing — the replace would orphan a step that may land.
    let mut rig = reboot_editing(rig, |folded| {
        edit_orders(folded, hull, |doc| {
            doc["author"] = serde_json::json!(other.0);
            doc["orders"][0]["state"] = serde_json::json!("inflight");
            doc["inflight"] = serde_json::json!({
                "order": 1, "attempt": 1, "key": "market.lot.draw.1",
                "refs": {"quote": null, "contract": null, "ticket": null},
                "staged_utc": 1, "issued_ic": 1, "restaged": 0, "want": 5, "qty": 5,
                "bounds": [], "line": null
            });
        });
    });
    let _ = rig.tw.connect(pilot);
    let _ = rig.tw.take_module_ops();
    let out = order(&mut rig, pilot, "buy volatiles 9");
    assert!(out.contains(&ORDER_INFLIGHT.replace("{n}", "1")), "{out}");
    let ops = rig.tw.take_module_ops();
    assert!(
        !format!("{ops:?}").contains("tacsim:orders:"),
        "the refusal wrote no sailing plan"
    );
    assert_eq!(orders(&rig.tw, hull).author, other, "the old plan stands");
    // Its step resolved (the record gone), the new tenant's first edit replaces it.
    let mut rig = reboot_editing(rig, |folded| {
        edit_orders(folded, hull, |doc| {
            doc["orders"][0]["state"] = serde_json::json!("pending");
            doc["inflight"] = serde_json::Value::Null;
        });
    });
    let _ = rig.tw.connect(pilot);
    let out = run(&mut rig.tw, pilot, "+ship/orders");
    assert!(
        out.contains(&NO_PLAN.replace("{name}", "Salt Moth")),
        "{out}"
    );
    let out = order(&mut rig, pilot, "buy volatiles 9");
    assert!(
        out.contains("Order 4 filed: buy water ice, 9 t. The sailing plan is at revision 1."),
        "{out}"
    );
    let doc = orders(&rig.tw, hull);
    assert_eq!((doc.author, doc.rev, doc.orders.len()), (pilot, 1, 1));
}

/// **An unreadable plan fails closed** (the store's table row): corrupt and misfiled
/// documents answer `ORDERS_DARK` and write nothing; the arm skips that hull while
/// another hull's plan runs.
#[test]
fn ic11_an_unreadable_orders_document_fails_closed() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let (other, other_row) = chartered(&mut rig, "Other", 2, "Grey Heron");
    let hull = ObjId(row.hull);
    let other_hull = ObjId(other_row.hull);
    let _ = order(&mut rig, pilot, "buy volatiles 5");
    let _ = order(&mut rig, other, "stop");
    let mut rig = reboot_editing(rig, |folded| {
        edit_orders(folded, hull, |doc| {
            doc["extra"] = serde_json::json!(1);
        });
    });
    let _ = rig.tw.connect(pilot);
    let _ = rig.tw.connect(other);
    assert!(matches!(
        store(&rig.tw).orders_state(hull),
        OrdersState::Unreadable
    ));
    let dark = ORDERS_DARK.replace("{name}", "Salt Moth");
    let _ = rig.tw.take_module_ops();
    for line in ["+ship/orders", "+ship/order stop", "+ship/order clear"] {
        let out = run(&mut rig.tw, pilot, line);
        assert!(out.contains(&dark), "`{line}`: {out}");
    }
    let ops = rig.tw.take_module_ops();
    assert!(
        !format!("{ops:?}").contains("tacsim:orders:"),
        "the refusals wrote no sailing plan"
    );
    // The arm skips the dark hull (paging it once) and runs the other's stop.
    let _ = drive(&mut rig, 3);
    assert!(matches!(
        store(&rig.tw).orders_state(hull),
        OrdersState::Unreadable
    ));
    assert_eq!(orders(&rig.tw, other_hull).state, PlanState::Paused);
    assert_eq!(
        order_state(&rig.tw, other_hull, 1),
        Some(OrderState::Done),
        "the readable plan ran"
    );
    // A misfiled document reads as unreadable too.
    let mut rig = reboot_editing(rig, |folded| {
        edit_orders(folded, other_hull, |doc| {
            doc["hull"] = serde_json::json!(other_hull.0 + 1_000);
        });
    });
    let _ = rig.tw.connect(other);
    let out = run(&mut rig.tw, other, "+ship/orders");
    assert!(
        out.contains(&ORDERS_DARK.replace("{name}", "Grey Heron")),
        "{out}"
    );
    let out = run(&mut rig.tw, other, "+ship/order stop");
    assert!(
        out.contains(&ORDERS_DARK.replace("{name}", "Grey Heron")),
        "{out}"
    );
}

// ---------------------------------------------------------------------------
// 2. The unattended day (lens 1)
// ---------------------------------------------------------------------------

/// A sale's line as the policy writes it, from the quote's own figures.
fn sale_head(tw: &TestWorld, quote: &mush_market::Quote, label: &str) -> String {
    let gross = quote.price.minor() * i64::try_from(quote.quantity).expect("qty");
    let share: i64 = quote.cuts.iter().map(|c| c.amount.minor()).sum();
    let _ = tw;
    format!(
        "Sold {} t of {label} to Kasei Metals at {}/t: {}; Co-op share {}; you receive {} when the goods are in.",
        quote.quantity,
        cr(quote.price.minor()),
        cr(gross),
        cr(share),
        cr(gross - share)
    )
}

/// **The unattended day** (doc 18 §2 steps 10–17; §1 lens 1): a pilot enrolled at
/// Ceres buys the both-limits outbound cargo and §2 step 7's reaction mass and stores
/// by hand, boards, files §2 step 10's eight orders and disconnects; with no further
/// typed line, the port flies her out, sells at Stickney, fills the tank and the
/// locker for the return the queue projects, buys the return cargo (the alloys cut
/// short by funds), announces the hour and burns home at it.
#[test]
#[allow(clippy::too_many_lines)]
fn ic11_the_queue_runs_the_day_unattended() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    let hold = hold_vault(&rig.tw, &row);
    set_rate(&mut rig.tw, 3_600);
    let start = pilot_available(&rig.tw, pilot);
    let (metals, ice) = vertex_load();
    let mut ceres = 0;
    ceres += buy_from(
        &mut rig,
        pilot,
        "ceres_port",
        "freeport_exchange",
        "metals",
        metals,
    );
    ceres += buy_from(&mut rig, pilot, "ceres_port", "occator", "volatiles", ice);
    // Doc 18 §2 step 7's 42 t and 21 crew-days — 53 t and 19 crew-days under the
    // flip law since doc 19 cut A.2 (the burn is 52 t at this load; the 2° hop to
    // Kirkwood is 3.7 days and the chord 14.5).
    ceres += buy_from(&mut rig, pilot, "ceres_port", "occator", "propellant", 53);
    ceres += buy_from(&mut rig, pilot, "ceres_port", "coop", "provisions", 19);
    swept_drawn(&mut rig, pilot);
    let hand_quotes = market(&rig.tw)
        .quotes_of_buyer(&PrincipalId::Object(pilot))
        .count();
    board(&mut rig, pilot, "Salt Moth");
    let at = (rig.tw.world().now() + 7_200 + 59) / 60 * 60;
    let home = hhmm(at);
    let filed = [
        "depart stickney at=now".to_owned(),
        "sell metals all min=250".to_owned(),
        "sell volatiles all min=150".to_owned(),
        "buy propellant fill max=60".to_owned(),
        "buy provisions fill".to_owned(),
        "buy industrial_parts 49 max=400".to_owned(),
        "buy structural_alloys 55 max=400".to_owned(),
        format!("depart ceres at=daily {home}"),
    ];
    for (n, line) in filed.iter().enumerate() {
        let out = order(&mut rig, pilot, line);
        assert!(out.contains(&format!("Order {} filed: ", n + 1)), "{out}");
        if n == 0 {
            assert!(
                out.contains("Order 1 filed: depart for Stickney Yards now."),
                "{out}"
            );
            assert!(out.contains(&ORDER_FIRES_NEXT.replace("{n}", "1")), "{out}");
        } else {
            assert!(out.contains(ORDER_AFTER_DEPART), "{out}");
        }
    }
    disconnect(&mut rig.tw, pilot);

    // ---- no typed line by the pilot from here until the reconnect ----
    let mut landed: Vec<AuthoritativeSubmit> = Vec::new();
    landed.extend(drive_until(&mut rig, 12, |rig| {
        order_state(&rig.tw, hull, 1) == Some(OrderState::Done)
    }));
    let out_ticket = ticket(&mut rig.tw, hull, 1).expect("the outbound ticket");
    assert_eq!((out_ticket.order, out_ticket.attempt), (Some(1), 1));
    // §2 step 7: the tank and the locker cover the outbound with a tonne to spare.
    assert_eq!(53 - out_ticket.plan.burn_t, 1, "tank 1 after the outbound");
    assert_eq!(19 - u64::from(out_ticket.plan.crew_days), 0);
    let arrival = out_ticket.plan.depart_ic + out_ticket.plan.total_s;
    landed.extend(to_ic(&mut rig, arrival - 3_600));
    landed.extend(to_ic(&mut rig, arrival + 60));
    match tacsim::port::ship::berth_of(rig.tw.world(), hull) {
        tacsim::port::ship::Berth::Docked { def, .. } => {
            assert_eq!(def.venue.as_str(), "stickney");
        }
        other => panic!("the hull did not dock at Stickney: {other:?}"),
    }
    let sold_metals_bid = bid_qty(&rig.tw, kasei_bid(&rig.tw, "metals")).expect("the bid");
    landed.extend(drive_until(&mut rig, 400, |rig| {
        orders(&rig.tw, hull).order(8).is_some_and(|o| o.announced)
    }));
    // The closed calls' effects, from the tick: two sales, settled.
    let sales: Vec<mush_market::Quote> = market(&rig.tw)
        .quotes_of_seller(&PrincipalId::Object(pilot))
        .cloned()
        .collect();
    assert_eq!(sales.len(), 2, "two sale contracts");
    for sale in &sales {
        assert_eq!(sale.lot.as_ref().map(|l| l.kind), Some(PaidKind::Deliver));
        assert!(sale.settled.is_some(), "each sale settled");
    }
    assert_eq!(
        bid_qty(&rig.tw, kasei_bid(&rig.tw, "metals")),
        Some(sold_metals_bid - metals)
    );
    let draws: Vec<mush_market::Quote> = market(&rig.tw)
        .quotes_of_buyer(&PrincipalId::Object(pilot))
        .skip(hand_quotes)
        .cloned()
        .collect();
    assert_eq!(draws.len(), 4, "four Draw contracts from the tick");
    for draw in &draws {
        assert!(
            draw.contract.is_some() && draw.drawn_at.is_some(),
            "{draw:?}"
        );
    }
    // The hold after the Stickney buys.
    assert_eq!(pool(&mut rig.tw, hold, "metals"), 0);
    assert_eq!(pool(&mut rig.tw, hold, "volatiles"), 0);
    assert_eq!(pool(&mut rig.tw, hold, "propellant"), 50);
    assert_eq!(pool(&mut rig.tw, hold, "provisions"), 19);
    assert_eq!(pool(&mut rig.tw, hold, "industrial_parts"), 49);
    // 17 t since doc 19 cut A.2: the costlier bunkering leaves less for the alloys.
    assert_eq!(pool(&mut rig.tw, hold, "structural_alloys"), 17);
    // The funds path, from journals: the stake less Ceres, plus the two nets, less
    // the four Stickney costs.
    let net = |q: &mush_market::Quote| {
        q.price.minor() * i64::try_from(q.quantity).expect("qty")
            - q.cuts.iter().map(|c| c.amount.minor()).sum::<i64>()
    };
    let nets: i64 = sales.iter().map(net).sum();
    let costs: i64 = draws
        .iter()
        .map(|q| q.price.minor() * i64::try_from(q.quantity).expect("qty"))
        .sum();
    assert_eq!(
        pilot_available(&rig.tw, pilot),
        start - ceres + nets - costs,
        "the funds path"
    );
    let fuel = price_of(
        &rig.tw,
        listing_of(&rig.tw, "stickney_port", "kasei", "propellant"),
    );
    let food = price_of(
        &rig.tw,
        listing_of(&rig.tw, "stickney_port", "yard_commissary", "provisions"),
    );
    let parts = price_of(
        &rig.tw,
        listing_of(
            &rig.tw,
            "stickney_port",
            "stickney_yards",
            "industrial_parts",
        ),
    );
    let alloys = price_of(
        &rig.tw,
        listing_of(&rig.tw, "stickney_port", "kasei", "structural_alloys"),
    );
    // 49 t of fuel and 17 t of alloys since doc 19 cut A.2 (the return's fill is 50 t
    // under the flip law, one aboard; the slice's were 36 and 20).
    let before_alloys = start - ceres + nets - 49 * fuel - 19 * food - 49 * parts;
    assert_eq!(
        before_alloys / alloys,
        17,
        "the alloys are cut to 17 t by funds"
    );

    // Hold the second departure's hour: walk the clock to it.
    let now = rig.tw.world().now();
    assert!(now < at, "the hour is still ahead");
    sweep(&mut rig.tw, at - now - 1);
    let _ = drain(&mut rig.tw);
    assert!(
        voyage(&rig.tw, hull).and_then(|v| v.pending).is_none(),
        "not before the hour"
    );
    landed.extend(drive_until(&mut rig, 12, |rig| {
        order_state(&rig.tw, hull, 8) == Some(OrderState::Done)
    }));
    let keys = burn_keys(&landed);
    assert_eq!(
        keys,
        vec![
            format!("tacsim.port.burn/{}/1/1", hull.0),
            format!("tacsim.port.burn/{}/8/1", hull.0),
        ],
        "exactly two burns"
    );
    let home_ticket = ticket(&mut rig.tw, hull, 2).expect("the return ticket");
    assert_eq!((home_ticket.order, home_ticket.attempt), (Some(8), 1));
    assert!(matches!(
        tacsim::port::ship::berth_of(rig.tw.world(), hull),
        tacsim::port::ship::Berth::UnderWay
    ));

    // The log, in order, and no waiting line among it.
    let granted = |t: &Ticket, port: &str, log: &str| {
        let head = CLEARANCE_GRANTED
            .replace("{burn}", &t.plan.burn_t.to_string())
            .replace("{days}", &t.plan.crew_days.to_string())
            .replace("{name}", "Salt Moth")
            .replace("{port}", port);
        let cut = head.find("{icdays}").unwrap_or(head.len());
        log.starts_with(&head[..cut])
    };
    let log = log_texts(&rig.tw, hull);
    assert_eq!(log.len(), 9, "{log:#?}");
    assert!(granted(&out_ticket, "Stickney Yards", &log[0]), "{log:#?}");
    let metals_sale = sales
        .iter()
        .find(|q| {
            q.lot
                .as_ref()
                .is_some_and(|l| l.commodity.0.as_str() == "metals")
        })
        .expect("the metals sale");
    let ice_sale = sales
        .iter()
        .find(|q| {
            q.lot
                .as_ref()
                .is_some_and(|l| l.commodity.0.as_str() == "volatiles")
        })
        .expect("the ice sale");
    assert!(
        log[1].starts_with(&sale_head(&rig.tw, metals_sale, "metal concentrate")),
        "{log:#?}"
    );
    assert!(
        log[2].starts_with(&sale_head(&rig.tw, ice_sale, "water ice")),
        "{log:#?}"
    );
    assert_eq!(
        log[3],
        BOUGHT_FILL_TANK
            .replace("{qty}", "49")
            .replace("{label}", "reaction mass")
            .replace("{price}", &cr(fuel))
            .replace("{total}", &cr(49 * fuel))
            .replace("{after}", "50")
            .replace("{limit}", "80")
            .replace("{cargo}", "73")
    );
    assert_eq!(
        log[4],
        BOUGHT_FILL_LOCKER
            .replace("{qty}", "19")
            .replace("{total}", &cr(19 * food))
    );
    assert_eq!(
        log[5],
        BOUGHT
            .replace("{qty}", "49")
            .replace("{unit}", "t")
            .replace("{label}", "yard fittings, crated")
            .replace("{price}", &cr(parts))
            .replace("{total}", &cr(49 * parts))
    );
    assert_eq!(
        log[6],
        BOUGHT_SHORT
            .replace("{qty}", "17")
            .replace("{unit}", "t")
            .replace("{label}", "structural alloy stock")
            .replace("{price}", &cr(alloys))
            .replace("{total}", &cr(17 * alloys))
            .replace("{want}", "55")
            .replace("{bounds}", "funds")
    );
    assert_eq!(
        log[7],
        DEPART_SET
            .replace("{port}", "Ceres Freeport")
            .replace("{hhmm}", &home)
    );
    assert!(granted(&home_ticket, "Ceres Freeport", &log[8]), "{log:#?}");
    assert!(
        !log.iter().any(|l| l.contains(" waits: ")),
        "no waiting line: {log:#?}"
    );

    // The reconnect: the heading once, then each queued line once, in order — the
    // outbound's IC9 notices first, and every log line among them.
    let queued = pending_notices(rig.tw.world(), pilot);
    assert!(
        granted(&out_ticket, "Stickney Yards", &queued[0]),
        "{queued:#?}"
    );
    let mut at_line = 0;
    for line in log.iter().skip(1).take(6) {
        let found = queued
            .iter()
            .skip(at_line)
            .position(|q| q == line)
            .unwrap_or_else(|| panic!("{line:?} was not queued in order: {queued:#?}"));
        at_line += found + 1;
    }
    assert!(queued.iter().any(|q| *q == log[7]), "{queued:#?}");
    let events = rig.tw.connect(pilot);
    let seen = rig.tw.seen_by(&events, pilot);
    assert_eq!(
        seen.iter().filter(|l| l.contains(NOTICE_HEADING)).count(),
        1,
        "{seen:#?}"
    );
    let first = seen
        .iter()
        .position(|l| l.contains(NOTICE_HEADING))
        .expect("the heading");
    let rest: Vec<&String> = seen.iter().skip(first + 1).collect();
    for line in &queued {
        assert_eq!(
            rest.iter().filter(|l| **l == line).count(),
            1,
            "{line:?} once: {seen:#?}"
        );
    }
    // ... and in the order they were queued, element by element.
    let shown: Vec<&String> = rest
        .iter()
        .copied()
        .filter(|l| queued.contains(l))
        .collect();
    assert_eq!(
        shown,
        queued.iter().collect::<Vec<_>>(),
        "the reconnect replays the queue in order: {seen:#?}"
    );
    assert!(pending_notices(rig.tw.world(), pilot).is_empty());
    // The render signs every log line with the pilot's name (Q-18-24).
    let out = run(&mut rig.tw, pilot, "+ship/orders");
    let signed: Vec<&str> = out
        .lines()
        .filter(|l| l.contains(", signed Pilot: "))
        .collect();
    assert_eq!(signed.len(), 8, "the last eight lines, each signed: {out}");
    assert!(out.contains("Log, each line signed Pilot:"), "{out}");
}

// ---------------------------------------------------------------------------
// 3. Reboots, stand-downs, the lease (lens 2)
// ---------------------------------------------------------------------------

/// A chartered pilot aboard the *Salt Moth*, docked at Stickney with `cargo` aboard
/// and the tank and the locker topped up for the return.
fn aboard_at_stickney(rig: &mut Rig, cargo: &[(&str, u64)]) -> (ObjId, HullRow) {
    let (pilot, row) = docked_at_stickney(rig, cargo);
    let gallery = p01(&rig.tw);
    board_at(rig, pilot, "Salt Moth", gallery);
    provision(rig, pilot, &row, "ceres", 950);
    (pilot, row)
}

/// **A reboot mid-step re-runs nothing** (doc 18 §4.4 "a reboot verifying and never
/// re-running", "not admitted retried by pull"; §1 lens 2; lead rulings L6, L7, L12):
/// (a) a sale staged and the server rebooted before its leg lands is re-staged once
/// under its contract's key and settles once; (b) a queued depart's burn lost to a
/// reboot lapses silently, reads Dropped, and is filed again under attempt 2 and the
/// next sequence — one ticket, one debit, one journey, no notice for the retry; (c) a
/// landing before the verify is verified done, nothing re-staged; (c') a landing the
/// market mirror had not read before the reboot is re-staged at most once and replays.
#[test]
#[allow(clippy::too_many_lines)]
fn ic11_a_reboot_mid_step_reruns_nothing() {
    // (a) The sale, its leg lost to a reboot.
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let hull = ObjId(row.hull);
    let bid = kasei_bid(&rig.tw, "metals");
    let wanted = bid_qty(&rig.tw, bid).expect("the bid");
    let out = order(&mut rig, pilot, "sell metals all");
    assert!(out.contains("Order 1 filed"), "{out}");
    let (_, queued) = fire(&mut rig);
    assert_eq!(queued.len(), 1, "the deliver leg");
    let staged = orders(&rig.tw, hull)
        .inflight
        .clone()
        .expect("the sale in flight");
    let contract = staged.refs.contract.expect("the contract");
    assert_eq!(
        staged.key,
        format!("market.lot.deliver.{contract:x}"),
        "the market leg's key, for the audit"
    );
    let mut rig = reboot(rig);
    let mut landed = drive_until(&mut rig, 40, |rig| {
        order_state(&rig.tw, hull, 1) == Some(OrderState::Done)
    });
    landed.retain(|s| s.action.key().owner.as_str() == "market");
    assert_eq!(landed.len(), 1, "one re-staged leg landed");
    assert_eq!(
        landed[0].key.as_str(),
        format!("market.lot.deliver.{contract:x}")
    );
    let sales: Vec<mush_market::Quote> = market(&rig.tw)
        .quotes_of_seller(&PrincipalId::Object(pilot))
        .cloned()
        .collect();
    assert_eq!(sales.len(), 1, "one sale quote");
    assert_eq!(sales[0].contract, Some(ContractId(contract)));
    assert!(matches!(
        state_of(&rig.tw, ContractId(contract)),
        Some(ContractState::Settled { .. })
    ));
    assert_eq!(
        bid_qty(&rig.tw, bid),
        Some(wanted - 20),
        "one bid decrement"
    );
    let doc = orders(&rig.tw, hull);
    assert!(doc.hold.is_none(), "no hold");
    assert_eq!(doc.state, PlanState::Running);
    let hold = hold_vault(&rig.tw, &row);
    assert_eq!(pool(&mut rig.tw, hold, "metals"), 0);

    // (b) A queued depart's burn lost to a reboot.
    let mut rig = stocked();
    let (pilot, row) = aboard_at_stickney(&mut rig, &[]);
    let hull = ObjId(row.hull);
    let hold = hold_vault(&rig.tw, &row);
    let tank = pool(&mut rig.tw, hold, "propellant");
    let out = order(&mut rig, pilot, "depart ceres");
    assert!(out.contains("Order 1 filed"), "{out}");
    disconnect(&mut rig.tw, pilot);
    let (_, queued) = fire(&mut rig);
    assert_eq!(
        burn_keys(&queued),
        vec![format!("tacsim.port.burn/{}/1/1", hull.0)]
    );
    let seq = voyage(&rig.tw, hull)
        .and_then(|v| v.pending)
        .map(|p| p.seq)
        .expect("the filed passage");
    let mut rig = reboot(rig);
    let landed = drive_until(&mut rig, 80, |rig| {
        order_state(&rig.tw, hull, 1) == Some(OrderState::Done)
    });
    assert_eq!(
        burn_keys(&landed),
        vec![format!("tacsim.port.burn/{}/1/2", hull.0)],
        "filed again under attempt 2"
    );
    assert!(
        ticket(&mut rig.tw, hull, seq).is_none(),
        "no ticket at the lost seq"
    );
    let t = ticket(&mut rig.tw, hull, seq + 1).expect("one ticket at seq + 1");
    assert_eq!((t.order, t.attempt), (Some(1), 2));
    assert_eq!(
        pool(&mut rig.tw, hold, "propellant"),
        tank - t.plan.burn_t,
        "one debit of the tank"
    );
    assert!(under_way(&rig.tw, hull), "one journey");
    assert_eq!(
        journey_of(&rig.tw, hull),
        Some((t.plan.route.clone(), t.plan.depart_ic)),
        "the journey is the second filing's"
    );
    let log = log_texts(&rig.tw, hull);
    assert_eq!(
        log.iter()
            .filter(|l| **l == NOT_ADMITTED.replace("{n}", "1"))
            .count(),
        1,
        "{log:#?}"
    );
    let told = pending_notices(rig.tw.world(), pilot);
    assert!(
        !told.iter().any(|l| l.contains("lapsed before the burn")
            || l.contains("not taken up")
            || l.contains("tried again")),
        "the retry is never notified: {told:#?}"
    );
    let _ = CLEARANCE_LAPSED;

    // (c) A landing before the verify: verified done, nothing re-staged. A buy's
    // landing is its quote's `drawn_at` (L5), which only the market sweep's mirror
    // writes; a reboot re-arms that sweep one whole period on, which for a leg staged
    // at the boot instant is the first re-stage deadline (ORDER_RESTAGE_SECS) and the
    // port tick, due first, re-stages a landed leg whose mirror had not yet run (it
    // replays as a HIT: phase (c') below). So the landing here is mirrored before the
    // reboot — the clock stepped a second at a time, so the sweep's fire is not the
    // port tick's and the order is still in flight — and the reboot falls between the
    // mirror and the verify that reads it.
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[]);
    let hull = ObjId(row.hull);
    let hold = hold_vault(&rig.tw, &row);
    let out = order(&mut rig, pilot, "buy structural_alloys 5");
    assert!(out.contains("Order 1 filed"), "{out}");
    let (_, queued) = fire(&mut rig);
    assert_eq!(queued.len(), 1, "the draw leg");
    land_all(&mut rig, &queued);
    let drawn = |rig: &Rig| {
        market(&rig.tw)
            .quotes_of_buyer(&PrincipalId::Object(pilot))
            .all(|q| q.drawn_at.is_some())
    };
    for _ in 0..ORDER_RESTAGE_SECS {
        if drawn(&rig) {
            break;
        }
        sweep(&mut rig.tw, 1);
        assert!(
            drain(&mut rig.tw).is_empty(),
            "nothing staged before the mirror"
        );
    }
    assert!(drawn(&rig), "the market mirror read the landing");
    assert_eq!(
        order_state(&rig.tw, hull, 1),
        Some(OrderState::Inflight),
        "the reboot comes before the verify"
    );
    assert_eq!(
        orders(&rig.tw, hull).inflight.map(|i| i.restaged),
        Some(0),
        "nothing re-staged before the reboot"
    );
    let mut rig = reboot(rig);
    // Every market submission the arm queues after the boot is counted, landed or not
    // (a re-staged leg of a landed Draw would replay, so counting landings could never
    // see it).
    let mut restaged: Vec<String> = Vec::new();
    for _ in 0..40 {
        if order_state(&rig.tw, hull, 1) == Some(OrderState::Done) {
            break;
        }
        let (_, queued) = fire(&mut rig);
        restaged.extend(
            queued
                .iter()
                .filter(|s| s.action.key().owner.as_str() == "market")
                .map(|s| s.key.as_str().to_owned()),
        );
        land_quietly(&mut rig, &queued);
    }
    assert_eq!(order_state(&rig.tw, hull, 1), Some(OrderState::Done));
    assert!(restaged.is_empty(), "nothing re-staged: {restaged:?}");
    assert_eq!(pool(&mut rig.tw, hold, "structural_alloys"), 5);
    assert_eq!(
        market(&rig.tw)
            .quotes_of_buyer(&PrincipalId::Object(pilot))
            .count(),
        1,
        "one quote"
    );

    // (c') The same landing rebooted before the market mirror read it: the first
    // re-stage deadline comes before the evidence, so the leg is re-staged once under
    // its contract's key — and replays: the goods are aboard once, one quote, no hold.
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[]);
    let hull = ObjId(row.hull);
    let hold = hold_vault(&rig.tw, &row);
    let out = order(&mut rig, pilot, "buy structural_alloys 5");
    assert!(out.contains("Order 1 filed"), "{out}");
    let (_, queued) = fire(&mut rig);
    assert_eq!(queued.len(), 1, "the draw leg");
    let key = queued[0].key.as_str().to_owned();
    land_all(&mut rig, &queued);
    let mut rig = reboot(rig);
    let mut restaged: Vec<String> = Vec::new();
    for _ in 0..40 {
        if order_state(&rig.tw, hull, 1) == Some(OrderState::Done) {
            break;
        }
        let (_, queued) = fire(&mut rig);
        restaged.extend(
            queued
                .iter()
                .filter(|s| s.action.key().owner.as_str() == "market")
                .map(|s| s.key.as_str().to_owned()),
        );
        land_quietly(&mut rig, &queued);
    }
    assert_eq!(order_state(&rig.tw, hull, 1), Some(OrderState::Done));
    assert!(
        restaged.len() <= 1 && restaged.iter().all(|k| *k == key),
        "at most the one re-stage, under the contract's key: {restaged:?}"
    );
    assert_eq!(
        pool(&mut rig.tw, hold, "structural_alloys"),
        5,
        "aboard once"
    );
    assert_eq!(
        market(&rig.tw)
            .quotes_of_buyer(&PrincipalId::Object(pilot))
            .count(),
        1,
        "one quote"
    );
    let doc = orders(&rig.tw, hull);
    assert!(doc.hold.is_none(), "no hold");
    assert_eq!(doc.state, PlanState::Running);
}

/// **A Draw lost to a reboot is re-staged from the tick** (the buy analogue of lens
/// 2(a); reviewer F14's rule that every closed call is shown succeeding from the tick
/// with its effect asserted): a buy staged and the server rebooted before its draw leg
/// lands — at `staged_utc + ORDER_RESTAGE_SECS` `mush_market::restage_draw`, run as the
/// pilot inside the `tacsim.port` tick, re-stages the leg under the contract's key; it
/// lands, the goods are aboard once, one quote, the funds debited once.
#[test]
fn ic11_a_draw_lost_to_a_reboot_is_restaged_from_the_tick() {
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[]);
    let hull = ObjId(row.hull);
    let hold = hold_vault(&rig.tw, &row);
    let price = price_of(
        &rig.tw,
        listing_of(&rig.tw, "stickney_port", "kasei", "structural_alloys"),
    );
    let out = order(&mut rig, pilot, "buy structural_alloys 5");
    assert!(out.contains("Order 1 filed"), "{out}");
    disconnect(&mut rig.tw, pilot);
    let a0 = pilot_available(&rig.tw, pilot);
    let (_, queued) = fire(&mut rig);
    assert_eq!(queued.len(), 1, "the draw leg");
    let staged = orders(&rig.tw, hull)
        .inflight
        .clone()
        .expect("the buy in flight");
    let contract = staged.refs.contract.expect("the contract");
    let quote = staged.refs.quote.expect("the quote");
    assert_eq!(staged.key, format!("market.lot.draw.{contract:x}"));
    // The queued leg is lost with the engine's RAM submit queue.
    let mut rig = reboot(rig);
    let mut legs: Vec<AuthoritativeSubmit> = Vec::new();
    for _ in 0..40 {
        if order_state(&rig.tw, hull, 1) == Some(OrderState::Done) {
            break;
        }
        let (_, queued) = fire(&mut rig);
        legs.extend(
            queued
                .iter()
                .filter(|s| s.action.key().owner.as_str() == "market")
                .cloned(),
        );
        land_all(&mut rig, &queued);
    }
    assert_eq!(order_state(&rig.tw, hull, 1), Some(OrderState::Done));
    assert_eq!(legs.len(), 1, "one re-staged draw");
    assert_eq!(
        legs[0].key.as_str(),
        format!("market.lot.draw.{contract:x}")
    );
    assert_eq!(legs[0].actor, pilot, "staged as the pilot");
    let quotes: Vec<mush_market::Quote> = market(&rig.tw)
        .quotes_of_buyer(&PrincipalId::Object(pilot))
        .cloned()
        .collect();
    assert_eq!(quotes.len(), 1, "one quote");
    assert_eq!(quotes[0].id.0, quote);
    assert_eq!(quotes[0].contract, Some(ContractId(contract)));
    assert!(quotes[0].drawn_at.is_some(), "the draw landed");
    assert_eq!(pool(&mut rig.tw, hold, "structural_alloys"), 5);
    assert_eq!(
        pilot_available(&rig.tw, pilot),
        a0 - 5 * price,
        "the funds debited once"
    );
    let doc = orders(&rig.tw, hull);
    assert!(doc.hold.is_none(), "no hold");
    assert_eq!(
        log_texts(&rig.tw, hull),
        vec![
            BOUGHT
                .replace("{qty}", "5")
                .replace("{unit}", "t")
                .replace("{label}", "structural alloy stock")
                .replace("{price}", &cr(price))
                .replace("{total}", &cr(5 * price))
        ]
    );
}

/// **A staff stand-down holds the order and files nothing** (reviewer F4, lead ruling
/// L28): the order holds with the attempt consumed, the pilot hears only the
/// stand-down, a late burn is refused for want of its pending, and `resume` files it
/// again under attempt 2.
#[test]
fn ic11_a_staff_stand_down_holds_the_order_and_files_nothing() {
    let mut rig = stocked();
    let (pilot, row) = aboard_at_stickney(&mut rig, &[]);
    let hull = ObjId(row.hull);
    let _ = order(&mut rig, pilot, "depart ceres");
    disconnect(&mut rig.tw, pilot);
    let (_, queued) = fire(&mut rig);
    let late = queued.clone();
    assert_eq!(burn_keys(&queued).len(), 1);
    let out = run(&mut rig.tw, GOD, &format!("@tacsim/port clear #{}", hull.0));
    assert!(out.contains("The sailing plan holds at order 1."), "{out}");
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.state, PlanState::Held);
    assert!(doc.inflight.is_none());
    let order1 = doc.order(1).cloned().expect("order 1");
    assert_eq!((order1.state, order1.attempt), (OrderState::Held, 2));
    assert_eq!(
        doc.hold.as_ref().map(|h| h.reason.clone()),
        Some(HELD_STOOD_DOWN.replace("{port}", "Ceres Freeport"))
    );
    let told = pending_notices(rig.tw.world(), pilot);
    assert_eq!(
        told,
        vec![
            CLEARANCE_STOOD_DOWN
                .replace("{name}", "Salt Moth")
                .replace("{port}", "Ceres Freeport")
        ],
        "the stand-down notice only"
    );
    // The late burn is refused (its pending is gone), and nothing is re-filed.
    for submit in &late {
        assert!(land(&mut rig, submit).is_err(), "the late burn is refused");
    }
    let landed = drive(&mut rig, 15);
    assert!(burn_keys(&landed).is_empty(), "no second filing");
    assert!(voyage(&rig.tw, hull).and_then(|v| v.pending).is_none());
    assert!(
        ticket(&mut rig.tw, hull, 2).is_none(),
        "no ticket for the stood-down filing"
    );
    assert!(matches!(
        tacsim::port::ship::berth_of(rig.tw.world(), hull),
        tacsim::port::ship::Berth::Docked { .. }
    ));
    let _ = rig.tw.connect(pilot);
    let out = order(&mut rig, pilot, "resume");
    assert!(out.contains(&RESUMED.replace("{n}", "1")), "{out}");
    let landed = drive_until(&mut rig, 12, |rig| {
        order_state(&rig.tw, hull, 1) == Some(OrderState::Done)
    });
    assert_eq!(
        burn_keys(&landed),
        vec![format!("tacsim.port.burn/{}/1/2", hull.0)]
    );
    assert!(under_way(&rig.tw, hull));
    assert!(
        ticket(&mut rig.tw, hull, 2).is_none(),
        "the stood-down seq stays burned"
    );
    assert!(
        ticket(&mut rig.tw, hull, 3).is_some(),
        "the resumed filing's ticket"
    );
}

/// **A lapsed charter holds and stages nothing** (reviewer F5/F16; L10, L25): with a
/// sale in flight and its leg lost, the charter ended — past the re-stage deadline
/// nothing is re-staged, and when the contract refunds the order reads Dropped with
/// nothing filed again.
///
/// Doc 18 cut IC13 (lead 2i, lead ruling L4): the staff-ended charter's hull is not
/// back in the pool (she lies at Stickney with cargo), so the port tick's Return arm
/// holds the lapsed author's plan — a *stopped* hold over the inflight step
/// ([`HELD_RECALLED`]), logged and told once — where IC11's order arm wrote a waiting
/// note; the drop then leaves the order Held under it.
#[test]
fn ic11_a_lapsed_charter_holds_and_stages_nothing() {
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let hull = ObjId(row.hull);
    let _ = order(&mut rig, pilot, "sell metals all");
    let (_, queued) = fire(&mut rig);
    assert_eq!(queued.len(), 1);
    let contract = orders(&rig.tw, hull)
        .inflight
        .and_then(|i| i.refs.contract)
        .map(ContractId)
        .expect("the contract");
    let mut rig = reboot(rig);
    let lease = tacsim::port::live_charter(rig.tw.world(), pilot)
        .expect("the charter")
        .contract;
    let _ = run(&mut rig.tw, GOD, &format!("@contract/cancel #{}", lease.0));
    assert_eq!(tacsim::port::live_charter(rig.tw.world(), pilot), None);
    let mut market_legs = 0;
    for _ in 0..20 {
        let (_, queued) = fire(&mut rig);
        market_legs += queued
            .iter()
            .filter(|s| s.action.key().owner.as_str() == "market")
            .count();
        land_quietly(&mut rig, &queued);
    }
    assert_eq!(
        market_legs, 0,
        "nothing re-staged while the charter is ended"
    );
    let waits = HELD_LINE.replace("{n}", "1").replace(
        "{reason}",
        &tacsim::port::recall::HELD_RECALLED.replace("{name}", "Salt Moth"),
    );
    let log = log_texts(&rig.tw, hull);
    assert_eq!(log.iter().filter(|l| **l == waits).count(), 1, "{log:#?}");
    let told = pending_notices(rig.tw.world(), pilot);
    assert_eq!(told.iter().filter(|l| **l == waits).count(), 1, "{told:#?}");
    // The shape is "stopped over inflight": the step and its record stand, the
    // contract is still open (so "nothing re-staged" is the Return arm's stopped hold
    // doing it — the order arm never re-stages a Held plan, and L12 takes the hull off
    // its agents — not a terminal contract's), and the hold stands.
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.order(1).map(|o| o.state), Some(OrderState::Inflight));
    assert!(doc.inflight.is_some(), "the record stands");
    assert!(
        doc.hold.as_ref().is_some_and(|h| !h.retry && h.order == 1),
        "{:?}",
        doc.hold
    );
    assert_eq!(doc.state, PlanState::Held);
    assert!(
        state_of(&rig.tw, contract).is_some_and(|s| !s.is_terminal()),
        "{:?}",
        state_of(&rig.tw, contract)
    );
    // The contract refunds at its cutoff: Dropped, nothing filed again — nor during
    // the long sweeps.
    for _ in 0..20 {
        if state_of(&rig.tw, contract).is_some_and(|s| s.is_terminal()) {
            break;
        }
        sweep(&mut rig.tw, 900);
        let queued = drain(&mut rig.tw);
        assert!(
            queued
                .iter()
                .all(|s| s.action.key().owner.as_str() != "market"),
            "nothing staged during the sweeps"
        );
    }
    assert!(
        state_of(&rig.tw, contract).is_some_and(|s| s.is_terminal()),
        "{:?}",
        state_of(&rig.tw, contract)
    );
    let landed = drive(&mut rig, 30);
    assert!(
        landed
            .iter()
            .all(|s| s.action.key().owner.as_str() != "market"),
        "nothing filed again"
    );
    let doc = orders(&rig.tw, hull);
    assert!(doc.inflight.is_none(), "the drop is read");
    let order1 = doc.order(1).cloned().expect("order 1");
    assert_eq!((order1.state, order1.attempt), (OrderState::Held, 2));
    assert!(
        market(&rig.tw)
            .quotes_of_seller(&PrincipalId::Object(pilot))
            .all(|q| q.contract == Some(contract)),
        "no second sale"
    );
    // The hold stands over the dropped order, and was neither logged nor told a
    // second time.
    assert!(
        doc.hold.as_ref().is_some_and(|h| !h.retry && h.order == 1),
        "{:?}",
        doc.hold
    );
    let log = log_texts(&rig.tw, hull);
    assert_eq!(log.iter().filter(|l| **l == waits).count(), 1, "{log:#?}");
    let told = pending_notices(rig.tw.world(), pilot);
    assert_eq!(told.iter().filter(|l| **l == waits).count(), 1, "{told:#?}");
}

/// **A false second miss heals itself** (reviewer F3, L6, L24): a sale whose leg is
/// held back past both re-stage deadlines holds over the step, told once; when the
/// leg then lands and settles, the order is done, the plan runs and the hold is gone,
/// with no second sale.
#[test]
fn ic11_a_false_second_miss_heals_itself() {
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let hull = ObjId(row.hull);
    let _ = order(&mut rig, pilot, "sell metals all");
    disconnect(&mut rig.tw, pilot);
    let mut legs: Vec<AuthoritativeSubmit> = Vec::new();
    let mut staged_utc: Option<i64> = None;
    for _ in 0..45 {
        let (_, queued) = fire(&mut rig);
        // Hold every market leg back; land everything else.
        let (market_legs, rest): (Vec<_>, Vec<_>) = queued
            .into_iter()
            .partition(|s| s.action.key().owner.as_str() == "market");
        legs.extend(market_legs);
        land_quietly(&mut rig, &rest);
        if staged_utc.is_none() {
            staged_utc = orders(&rig.tw, hull).inflight.map(|i| i.staged_utc);
        }
        if orders(&rig.tw, hull).state == PlanState::Held {
            break;
        }
    }
    let staged_utc = staged_utc.expect("the sale was staged");
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.state, PlanState::Held, "the second miss holds");
    assert_eq!(
        doc.hold.as_ref().map(|h| h.reason.as_str()),
        Some(HELD_UNCONFIRMED)
    );
    assert_eq!(doc.order(1).map(|o| o.state), Some(OrderState::Inflight));
    assert_eq!(doc.inflight.as_ref().map(|i| i.restaged), Some(2));
    // L6's cadence, exactly: the stage, the re-stage at the first deadline and the one
    // at the second — never a re-stage per fire.
    assert_eq!(
        legs.len(),
        3,
        "staged, then re-staged at each of the two deadlines"
    );
    assert!(
        doc.hold.as_ref().is_some_and(|h| h.since
            >= staged_utc
                + tacsim::port::ORDER_RESTAGE_SECS
                + tacsim::port::ORDER_SETTLE_GRACE_SECS),
        "the hold waits the whole grace past the first re-stage: {:?} vs staged {staged_utc}",
        doc.hold
    );
    let held = HELD_LINE
        .replace("{n}", "1")
        .replace("{reason}", HELD_UNCONFIRMED);
    let told = pending_notices(rig.tw.world(), pilot);
    assert_eq!(told.iter().filter(|l| **l == held).count(), 1, "{told:#?}");
    // The leg lands at last (once: the rest replay under the contract's key).
    let last = legs.last().cloned().expect("a leg");
    land_all(&mut rig, &[last]);
    let _ = drive_until(&mut rig, 40, |rig| {
        order_state(&rig.tw, hull, 1) == Some(OrderState::Done)
    });
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.state, PlanState::Running);
    assert!(doc.hold.is_none());
    let sales: Vec<mush_market::Quote> = market(&rig.tw)
        .quotes_of_seller(&PrincipalId::Object(pilot))
        .cloned()
        .collect();
    assert_eq!(sales.len(), 1, "no second sale");
    // After the heal, read afresh: the hold was told once and only once, and the
    // sale's own done line once.
    let told = pending_notices(rig.tw.world(), pilot);
    assert_eq!(told.iter().filter(|l| **l == held).count(), 1, "{told:#?}");
    let done = sale_head(&rig.tw, &sales[0], "metal concentrate");
    assert_eq!(
        told.iter().filter(|l| l.starts_with(&done)).count(),
        1,
        "{told:#?}"
    );
}

/// **A late first re-stage gives the second deadline its whole grace** (reviewer F3,
/// L6): a sale staged, its leg lost to a reboot that lasts longer than both deadlines
/// (the clock moved on past `staged_utc + ORDER_RESTAGE_SECS +
/// ORDER_SETTLE_GRACE_SECS`). The first fire after the boot re-stages the leg, which
/// lands; the second deadline then counts from that re-stage, so the sweeps settle it
/// before any second miss — no hold, no page, one sale.
#[test]
fn ic11_a_long_downtime_raises_no_false_second_miss() {
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let hull = ObjId(row.hull);
    let bid = kasei_bid(&rig.tw, "metals");
    let wanted = bid_qty(&rig.tw, bid).expect("the bid");
    let _ = order(&mut rig, pilot, "sell metals all");
    disconnect(&mut rig.tw, pilot);
    let (_, queued) = fire(&mut rig);
    assert_eq!(queued.len(), 1, "the deliver leg");
    let staged = orders(&rig.tw, hull)
        .inflight
        .clone()
        .expect("the sale in flight");
    let contract = staged.refs.contract.expect("the contract");
    let mut rig = reboot(rig);
    // The downtime: the clock jumps past both deadlines before the first fire.
    let down = tacsim::port::ORDER_RESTAGE_SECS + tacsim::port::ORDER_SETTLE_GRACE_SECS + 120;
    let t = rig.tw.world().now() + down;
    rig.tw.set_now(t);
    let mut market_legs = 0;
    for _ in 0..60 {
        if order_state(&rig.tw, hull, 1) == Some(OrderState::Done) {
            break;
        }
        let (_, queued) = fire(&mut rig);
        market_legs += queued
            .iter()
            .filter(|s| s.action.key().owner.as_str() == "market")
            .count();
        land_all(&mut rig, &queued);
        assert!(
            orders(&rig.tw, hull).hold.is_none(),
            "no hold: {:?}",
            log_texts(&rig.tw, hull)
        );
    }
    assert_eq!(order_state(&rig.tw, hull, 1), Some(OrderState::Done));
    assert_eq!(market_legs, 1, "one re-stage, at the first deadline only");
    assert!(matches!(
        state_of(&rig.tw, ContractId(contract)),
        Some(ContractState::Settled { .. })
    ));
    assert_eq!(bid_qty(&rig.tw, bid), Some(wanted - 20), "one sale");
    assert!(
        !log_texts(&rig.tw, hull)
            .iter()
            .any(|l| l.contains(HELD_UNCONFIRMED)),
        "{:?}",
        log_texts(&rig.tw, hull)
    );
    assert!(
        !pending_notices(rig.tw.world(), pilot)
            .iter()
            .any(|l| l.contains(HELD_UNCONFIRMED))
    );
}

// ---------------------------------------------------------------------------
// 4. Failures (lens 3)
// ---------------------------------------------------------------------------

/// **A sold-out buy goes partial** (§1 lens 3): a 5 t Exchange stock, `buy metals 10`
/// buys the five and is done, the stock named.
#[test]
fn ic11_a_sold_out_buy_goes_partial() {
    let lane = lane_variant(LANE_VERSION, |doc| {
        vendor(doc, "ceres_port", "freeport_exchange")["stock"][0]["qty"] = serde_json::json!(5);
    });
    let mut rig = stocked_with(Roster::seeded(lane));
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    let hold = hold_vault(&rig.tw, &row);
    let price = price_of(
        &rig.tw,
        listing_of(&rig.tw, "ceres_port", "freeport_exchange", "metals"),
    );
    let _ = order(&mut rig, pilot, "buy metals 10");
    let _ = drive_until(&mut rig, 30, |rig| {
        order_state(&rig.tw, hull, 1) == Some(OrderState::Done)
    });
    assert_eq!(pool(&mut rig.tw, hold, "metals"), 5);
    assert_eq!(
        log_texts(&rig.tw, hull),
        vec![
            BOUGHT_SHORT
                .replace("{qty}", "5")
                .replace("{unit}", "t")
                .replace("{label}", "metal concentrate")
                .replace("{price}", &cr(price))
                .replace("{total}", &cr(5 * price))
                .replace("{want}", "10")
                .replace("{bounds}", "stock")
        ]
    );
    // Sold out now: a second buy takes nothing, and only the stock bit — no ask
    // means no price, and no price never reads as "funds".
    let _ = order(&mut rig, pilot, "buy metals 10");
    let _ = drive_until(&mut rig, 30, |rig| {
        order_state(&rig.tw, hull, 2) == Some(OrderState::Done)
    });
    assert_eq!(pool(&mut rig.tw, hold, "metals"), 5);
    assert_eq!(
        log_texts(&rig.tw, hull).last().cloned(),
        Some(
            BOUGHT_NONE
                .replace("{unit}", "t")
                .replace("{label}", "metal concentrate")
                .replace("{want}", "10")
                .replace("{bounds}", "stock")
        )
    );
}

/// **An unfunded bid skips** (§1 lens 3; doc 18 §4.4 "an unfunded or exhausted bid is
/// skipped and logged"): Kasei's available funds below one tonne's price — the order is
/// passed over, no quote and no contract, the pilot told.
///
/// Doc 18 cut IC12: the stocking helper waits for every readiness line but the sky to
/// read ready, and a bid whose house cannot fund a tonne reads pending ("not buying
/// until its capital settles"), so a world authored with Kasei unfunded can never be
/// stocked by it. The house is drained after stocking instead
/// ([`leave_kasei_under_a_tonne`]) — staff move all but 100.00cr of its available
/// balance to the treasury — which leaves it exactly as the lane variant did: funded,
/// but below one tonne's price (the house's units are floored, so 100.00cr against the
/// bid is 0 units, never 1). Since cut P-B (Q-18-60, R10) that pending line no longer
/// closes `+coop/enrol`; `pb_the_tick_pages_once_a_boot_while_a_house_cannot_fund_a_tonne`
/// pins the open desk and the once-a-boot page, this test the skip.
#[test]
fn ic11_an_unfunded_bid_skips() {
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let hull = ObjId(row.hull);
    let _ = leave_kasei_under_a_tonne(&mut rig);
    let contracts = economy(&rig.tw).snapshot().contracts().count();
    let _ = order(&mut rig, pilot, "sell metals all");
    disconnect(&mut rig.tw, pilot);
    let _ = drive(&mut rig, 3);
    assert_eq!(order_state(&rig.tw, hull, 1), Some(OrderState::Skipped));
    assert_eq!(
        market(&rig.tw)
            .quotes_of_seller(&PrincipalId::Object(pilot))
            .count(),
        0,
        "no quote"
    );
    assert_eq!(
        economy(&rig.tw).snapshot().contracts().count(),
        contracts,
        "no contract"
    );
    let skipped = SKIP_LINE.replace("{n}", "1").replace(
        "{reason}",
        &HOUSE_CANNOT_PAY_SKIP
            .replace("{house}", "Kasei Metals")
            .replace("{label}", "metal concentrate"),
    );
    assert_eq!(log_texts(&rig.tw, hull), vec![skipped.clone()]);
    assert_eq!(pending_notices(rig.tw.world(), pilot), vec![skipped]);
}

/// **Drain Kasei to 100.00cr** — under one tonne's price at its metals bid, but not
/// nothing — by a staff payment to the treasury; the minor units left.
fn leave_kasei_under_a_tonne(rig: &mut Rig) -> i64 {
    let kasei = port_map(&rig.tw, "stickney_port").orgs[&cid("kasei")]
        .account
        .expect("Kasei's account");
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let funds = economy(&rig.tw)
        .snapshot()
        .account(mush_economy::AccountId(kasei))
        .and_then(mush_economy::Account::available)
        .map(mush_economy::Amount::minor)
        .expect("Kasei's available balance");
    // 100.00cr stays: under one tonne's price at the bid, but not nothing.
    let left = 10_000;
    let bid = price_of(&rig.tw, kasei_bid(&rig.tw, "metals"));
    assert!(
        funds > left && left < bid,
        "{funds} available; the bid {bid}"
    );
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/pay #{kasei}=#{treasury} {}", funds - left),
    );
    assert!(out.contains("Paid: transaction #"), "{out}");
    assert_eq!(
        economy(&rig.tw)
            .snapshot()
            .account(mush_economy::AccountId(kasei))
            .and_then(mush_economy::Account::available)
            .map(mush_economy::Amount::minor),
        Some(left),
        "the house is left under one tonne"
    );
    left
}

/// **The tick pages once a boot while a house cannot fund a tonne, and the desk stays
/// open** (doc 18 Q-18-60, the owner's R10 of 2026-10-18; cut P-B): Kasei drained to
/// 100.00cr, under one tonne at either of its bids — both Bids lines read `[pending]`
/// "not buying until its capital settles" and the report reads "Not ready", yet
/// `+coop/enrol` still charters a new pilot; the first fire after the drain pages once
/// and the fires after it do not. The skip itself (`HOUSE_CANNOT_PAY_SKIP`) is
/// `ic11_an_unfunded_bid_skips`'s; the funded world never pages.
#[test]
fn pb_the_tick_pages_once_a_boot_while_a_house_cannot_fund_a_tonne() {
    let mut rig = stocked();
    assert_eq!(
        tacsim::port_tick::funds_pages(rig.tw.world()),
        0,
        "a funded house never pages"
    );
    // The report's lines that are not `[ready]`. Doc 20 SM.1: a keyless world's geology line is
    // always flagged, so "Not ready" alone no longer proves the two bid lines are counted; the
    // count before and after the drain does.
    let not_ready = |out: &str| {
        out.lines()
            .filter(|line| line.ends_with(']') && !line.ends_with("[ready]"))
            .count()
    };
    let before = not_ready(&status(&mut rig.tw));
    let _ = leave_kasei_under_a_tonne(&mut rig);
    let out = status(&mut rig.tw);
    let bids: Vec<&str> = out
        .lines()
        .filter(|line| line.contains(" bid kasei "))
        .collect();
    assert_eq!(bids.len(), 2, "{out}");
    assert!(
        bids.iter()
            .all(|line| line.ends_with("; not buying until its capital settles. [pending]")),
        "{out}"
    );
    assert_eq!(
        not_ready(&out),
        before + 2,
        "the two bid lines join:\n{out}"
    );
    assert!(
        out.contains(&format!("Not ready: {} of ", before + 2)),
        "the report still counts them:\n{out}"
    );
    // The desk stays open: a pilot charters while Kasei cannot pay.
    let (second, _) = chartered(&mut rig, "Second", 2, "Second Wind");
    assert!(tacsim::port::live_charter(rig.tw.world(), second).is_some());
    // The page: raised on the first fire after the drain, once for the boot.
    assert_eq!(
        tacsim::port_tick::funds_pages(rig.tw.world()),
        0,
        "no fire since the drain pages"
    );
    let _ = drive(&mut rig, 1);
    assert_eq!(
        tacsim::port_tick::funds_pages(rig.tw.world()),
        1,
        "the first fire pages"
    );
    let _ = drive(&mut rig, 3);
    assert_eq!(
        tacsim::port_tick::funds_pages(rig.tw.world()),
        1,
        "and pages once a boot, however many fires"
    );
}

/// **A house that cannot pay is skipped, never retried** (reviewer F13): Kasei's
/// account reads funded but its acceptance needs an approval its rule has not given
/// (the market's `HOUSE_CANNOT_PAY`); the order is passed over with no refusal
/// counted.
#[test]
fn ic11_a_house_that_cannot_pay_is_skipped() {
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let hull = ObjId(row.hull);
    let kasei = org_account(&rig.tw, "stickney_port", "kasei");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@account/approval {kasei}=100 threshold:1"),
    );
    let _ = out;
    let _ = order(&mut rig, pilot, "sell metals all");
    disconnect(&mut rig.tw, pilot);
    let _ = drive(&mut rig, 3);
    let doc = orders(&rig.tw, hull);
    let order1 = doc.order(1).cloned().expect("order 1");
    assert_eq!(order1.state, OrderState::Skipped, "{:?}", doc.log);
    assert_eq!(order1.refusals, 0, "no refusal counted");
    assert_eq!(
        log_texts(&rig.tw, hull),
        vec![
            SKIP_LINE.replace("{n}", "1").replace(
                "{reason}",
                &HOUSE_CANNOT_PAY_SKIP
                    .replace("{house}", "Kasei Metals")
                    .replace("{label}", "metal concentrate")
            )
        ]
    );
}

/// **A depart short of fuel holds and burns nothing** (§1 lens 3; doc 18 §4.4 "holds
/// with nothing burned, re-checked silently each fire, logged once"): the waiting
/// note with `TANK_SHORT`'s sentence, logged and told once across ten fires, no
/// clearance, no submission, the tank unchanged — then the tank topped up by hand and
/// she files and burns by herself.
#[test]
fn ic11_a_depart_short_of_fuel_holds_and_burns_nothing() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    let hold = hold_vault(&rig.tw, &row);
    board(&mut rig, pilot, "Salt Moth");
    // The tank one tonne under the empty-tank burn (more mass only burns more), the
    // locker full, from the planner's own figures.
    let plan = run(&mut rig.tw, pilot, "+ship/plan stickney");
    let (empty_burn, empty_tank) = plan_numbers(&plan, "Burn ", "(tank ").expect(&plan);
    assert_eq!(empty_tank, 0);
    seed_pool(&mut rig, hold, "propellant", empty_burn - 1, LotId(970));
    seed_pool(&mut rig, hold, "provisions", 100, LotId(971));
    let plan = run(&mut rig.tw, pilot, "+ship/plan stickney");
    let (burn, tank) = plan_numbers(&plan, "Burn ", "(tank ").expect(&plan);
    assert!(tank < burn, "short: {tank} < {burn}");
    let _ = order(&mut rig, pilot, "depart stickney");
    disconnect(&mut rig.tw, pilot);
    let mut submissions = 0;
    for _ in 0..10 {
        let (_, queued) = fire(&mut rig);
        submissions += queued
            .iter()
            .filter(|s| s.action.key().owner.as_str() == tacsim::NS)
            .count();
        land_all(&mut rig, &queued);
    }
    assert_eq!(submissions, 0, "nothing submitted");
    assert!(voyage(&rig.tw, hull).and_then(|v| v.pending).is_none());
    assert_eq!(
        pool(&mut rig.tw, hold, "propellant"),
        tank,
        "the tank unchanged"
    );
    let waits = WAITING_LINE.replace("{n}", "1").replace(
        "{reason}",
        &TANK_SHORT
            .replace("{port}", "Stickney Yards")
            .replace("{burn}", &burn.to_string())
            .replace("{tank}", &tank.to_string()),
    );
    let log = log_texts(&rig.tw, hull);
    assert_eq!(log, vec![waits.clone()]);
    assert_eq!(
        pending_notices(rig.tw.world(), pilot)
            .iter()
            .filter(|l| **l == waits)
            .count(),
        1
    );
    assert_eq!(orders(&rig.tw, hull).state, PlanState::Running);
    // Topped up by hand: she files and burns by herself.
    seed_pool(&mut rig, hold, "propellant", burn - tank + 2, LotId(980));
    let landed = drive_until(&mut rig, 12, |rig| {
        order_state(&rig.tw, hull, 1) == Some(OrderState::Done)
    });
    assert_eq!(
        burn_keys(&landed),
        vec![format!("tacsim.port.burn/{}/1/1", hull.0)]
    );
    assert!(
        orders(&rig.tw, hull).hold.is_none(),
        "the note cleared itself"
    );
}

/// **A depart with no route holds until edited** (L10's `NO_ROUTE` as a stopped hold):
/// a depart to the port she is docked at holds, told once; striking it lets the plan
/// run on.
#[test]
fn ic11_a_depart_with_no_route_holds_until_edited() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    board(&mut rig, pilot, "Salt Moth");
    provision(&mut rig, pilot, &row, "stickney", 960);
    let _ = order(&mut rig, pilot, "depart ceres");
    let _ = order(&mut rig, pilot, "depart stickney");
    disconnect(&mut rig.tw, pilot);
    let _ = drive(&mut rig, 6);
    let held = HELD_LINE.replace("{n}", "1").replace(
        "{reason}",
        &NO_ROUTE
            .replace("{here}", "Ceres Freeport")
            .replace("{list}", "Ahuna Flank, Freeport Commons, Juling Shadow, Kerwan Floor, Aitken Highport, Occator Rim, Rheasilvia Commons, Stickney Yards"),
    );
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.state, PlanState::Held);
    assert_eq!(doc.order(1).map(|o| o.state), Some(OrderState::Held));
    assert_eq!(log_texts(&rig.tw, hull), vec![held.clone()]);
    assert_eq!(
        pending_notices(rig.tw.world(), pilot)
            .iter()
            .filter(|l| **l == held)
            .count(),
        1
    );
    let _ = rig.tw.connect(pilot);
    let out = order(&mut rig, pilot, "del 1");
    assert!(out.contains(&DELETED.replace("{n}", "1")), "{out}");
    let landed = drive_until(&mut rig, 12, |rig| {
        order_state(&rig.tw, hull, 2) == Some(OrderState::Done)
    });
    assert_eq!(
        burn_keys(&landed),
        vec![format!("tacsim.port.burn/{}/2/1", hull.0)]
    );
}

// ---------------------------------------------------------------------------
// 5. The clamp, the reserve and fill (lenses 4–7)
// ---------------------------------------------------------------------------

/// The Dray's compartments, read from the hold's pools as the planning core reads
/// them.
fn readings(rig: &mut Rig, row: &HullRow) -> Vec<tacsim::port::gauges::Reading> {
    let goods = tacsim::definitions::goods::goods_table().expect("the goods table");
    let dray = load_pack(tacsim::definitions::content::DRAY_JSON).expect("the Dray pack");
    let Some(PackEntry::Unit(spec)) = dray.entry(&cid(tacsim::definitions::content::DRAY)) else {
        panic!("the Dray unit");
    };
    let hold = hold_vault(&rig.tw, row);
    let mut out = Vec::new();
    for (id, def) in &spec.logistics.as_ref().expect("logistics").stowage {
        let comp = tacsim::port::capacity::Compartment::from_stowage(id, def);
        let mut contents = BTreeMap::new();
        for good in goods
            .goods
            .iter()
            .filter(|g| comp.accepts.contains(&g.stowage))
        {
            let qty = pool(&mut rig.tw, hold, good.id.as_str());
            if qty > 0 {
                contents.insert(good.id.clone(), qty);
            }
        }
        out.push((id.clone(), comp, contents));
    }
    out
}

/// The planner over the shipped lane at `sky_rate`, for the Dray, from `from` to
/// `to`: `(payload, occupants, depart_ic) -> plan`.
fn lane_planner(
    sky_rate: u32,
    from: &str,
    to: &str,
) -> impl Fn(u64, u32, i64) -> Result<tacsim::port::PassagePlan, tacsim::port::PlanError> {
    let pack = load_pack(LANE_JSON).expect("the lane");
    let cfg = tacsim::EphemerisConfig {
        rate: sky_rate,
        ..TacsimConfig::default().time.ephemeris
    };
    let sky = tacsim::Sky::from_pack(&pack, &cfg);
    let catalog =
        tacsim::definitions::Catalog::build_fielded(&[], tacsim::NS).expect("the catalog");
    let spec = catalog
        .specs
        .get(&cid(tacsim::definitions::content::DRAY))
        .expect("the Dray");
    let logistics = spec.logistics.clone().expect("logistics");
    let drive = tacsim::port::HullDrive {
        class: logistics
            .drive
            .as_ref()
            .expect("the Dray's drive")
            .class
            .clone(),
        law: *catalog
            .drive_for_class(&logistics.drive.as_ref().expect("the Dray's drive").class)
            .expect("a law"),
        payload_ceiling_kg: logistics.payload_ceiling_kg,
        throttle_permille: tacsim::port::DEFAULT_THROTTLE_PERMILLE,
    };
    let dry = u64::from(logistics.dry_mass_t) * 1_000;
    let routes = port_def(from).routes[&cid(to)].clone();
    move |payload, occupants, depart_ic| {
        tacsim::port::choose_route(
            &sky,
            &pack,
            &routes,
            &drive,
            dry,
            payload,
            occupants,
            depart_ic,
            tacsim::port::RoutePolicy::default(),
            None,
        )
    }
}

/// The fill targets the planning core sizes for `cargo` with what `readings` hold,
/// at `depart_ic`.
fn targets_for(
    readings: &[tacsim::port::gauges::Reading],
    cargo: &BTreeMap<CanonicalId, u64>,
    extra: u64,
    planner: &dyn Fn(u64, u32, i64) -> Result<tacsim::port::PassagePlan, tacsim::port::PlanError>,
    depart_ic: i64,
) -> tacsim::port::FillTargets {
    let goods = tacsim::definitions::goods::goods_table().expect("the goods table");
    let aboard = |good: &str| {
        readings
            .iter()
            .filter_map(|(_, _, c)| c.get(&cid(good)))
            .sum::<u64>()
    };
    tacsim::port::fill_targets(
        &goods,
        readings,
        cargo,
        extra,
        aboard("propellant"),
        aboard("provisions"),
        1,
        100,
        |payload| planner(payload, 1, depart_ic),
    )
    .expect("the targets")
}

/// **The cargo clamp reserves the next departure's fuel and stores** (§1 lens 4; L17,
/// reviewer F10(2)): at Stickney with cash-bound funds, the cargo buys queued before
/// the fills leave exactly the reserve the next departure's targets price, both fills
/// complete and the depart files.
#[test]
#[allow(clippy::too_many_lines)]
fn ic11_the_cargo_clamp_reserves_the_next_departures_fuel_and_stores() {
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[]);
    let hull = ObjId(row.hull);
    let hold = hold_vault(&rig.tw, &row);
    let gallery = p01(&rig.tw);
    board_at(&mut rig, pilot, "Salt Moth", gallery);
    for line in [
        "buy industrial_parts 49",
        "buy structural_alloys 55",
        "buy propellant fill",
        "buy provisions fill",
        "depart ceres",
    ] {
        let out = order(&mut rig, pilot, line);
        assert!(out.contains("filed"), "{out}");
    }
    let parts = price_of(
        &rig.tw,
        listing_of(
            &rig.tw,
            "stickney_port",
            "stickney_yards",
            "industrial_parts",
        ),
    );
    let alloys = price_of(
        &rig.tw,
        listing_of(&rig.tw, "stickney_port", "kasei", "structural_alloys"),
    );
    let fuel = price_of(
        &rig.tw,
        listing_of(&rig.tw, "stickney_port", "kasei", "propellant"),
    );
    let food = price_of(
        &rig.tw,
        listing_of(&rig.tw, "stickney_port", "yard_commissary", "provisions"),
    );
    // The reserve: the depart's targets over the projection 49 + the alloys that fit.
    let live = readings(&mut rig, &row);
    let goods = tacsim::definitions::goods::goods_table().expect("goods");
    let span = [
        tacsim::port::What::Buy {
            good: cid("industrial_parts"),
            qty: tacsim::port::Qty::Units(49),
            max_major: None,
            on_short: tacsim::port::OnShort::Partial,
        },
        tacsim::port::What::Buy {
            good: cid("structural_alloys"),
            qty: tacsim::port::Qty::Units(55),
            max_major: None,
            on_short: tacsim::port::OnShort::Partial,
        },
    ];
    let refs: Vec<&tacsim::port::What> = span.iter().collect();
    let (cargo, extra) = tacsim::port::project_cargo(&goods, &live, &refs).expect("projects");
    let planner = lane_planner(0, "stickney_port", "ceres");
    let targets = targets_for(&live, &cargo, extra, &planner, ic_now(&rig.tw));
    let tank0 = pool(&mut rig.tw, hold, "propellant");
    let locker0 = pool(&mut rig.tw, hold, "provisions");
    let reserve = tacsim::port::reserve_for(
        &targets,
        tank0,
        locker0,
        Some(mush_economy::Amount::from_minor(fuel)),
        Some(mush_economy::Amount::from_minor(food)),
        None,
    )
    .minor();
    assert!(reserve > 0, "the departure needs fuel and stores");
    let a0 = pilot_available(&rig.tw, pilot);
    let want_parts = 49.min(u64::try_from((a0 - reserve) / parts).expect("units"));
    let a1 = a0 - parts * i64::try_from(want_parts).expect("qty");
    let room = 24_u64;
    let want_alloys = room.min(u64::try_from((a1 - reserve) / alloys).expect("units"));
    assert!(
        want_alloys < room,
        "the alloys are cash-bound: {want_alloys}"
    );
    // The reserve bit: with no reserve the same funds would buy more alloys.
    let without_reserve = room.min(u64::try_from(a1 / alloys).expect("units"));
    assert!(
        without_reserve > want_alloys,
        "the reserve bit: {without_reserve} without it, {want_alloys} with it"
    );
    disconnect(&mut rig.tw, pilot);
    // The pilot's own balance once the alloys are bought (the escrow is posted at
    // accept) and before the fills run: what is left is the reserve, to within one
    // tonne of alloys.
    let _ = drive_until(&mut rig, 120, |rig| {
        order_state(&rig.tw, hull, 2) == Some(OrderState::Inflight)
    });
    let left = pilot_available(&rig.tw, pilot);
    assert!(
        left >= reserve && left - reserve < alloys,
        "the cargo buys leave the reserve: {left} left, {reserve} reserved, {alloys} a tonne"
    );
    let _ = drive_until(&mut rig, 120, |rig| {
        order_state(&rig.tw, hull, 2) == Some(OrderState::Done)
    });
    assert_eq!(pool(&mut rig.tw, hold, "industrial_parts"), want_parts);
    assert_eq!(pool(&mut rig.tw, hold, "structural_alloys"), want_alloys);
    let alloys_quote = market(&rig.tw)
        .quotes_of_buyer(&PrincipalId::Object(pilot))
        .find(|q| {
            q.lot
                .as_ref()
                .is_some_and(|l| l.commodity.0.as_str() == "structural_alloys")
        })
        .cloned()
        .expect("the alloys quote");
    assert_eq!(alloys_quote.quantity, want_alloys);
    assert!(
        log_texts(&rig.tw, hull)
            .iter()
            .any(|l| l.ends_with("short of 55 t: funds.")),
        "{:?}",
        log_texts(&rig.tw, hull)
    );
    let landed = drive_until(&mut rig, 120, |rig| {
        order_state(&rig.tw, hull, 5) == Some(OrderState::Done)
    });
    assert_eq!(order_state(&rig.tw, hull, 3), Some(OrderState::Done));
    assert_eq!(order_state(&rig.tw, hull, 4), Some(OrderState::Done));
    assert_eq!(
        burn_keys(&landed),
        vec![format!("tacsim.port.burn/{}/5/1", hull.0)]
    );
}

/// **Every bound that bit is logged** (§1 lens 5): the re-used 49/55 with full funds
/// logs `hold volume`; `buy volatiles 100` into an empty hold logs "short of 100 t:
/// hold volume"; a full hold logs "hold mass and hold volume".
#[test]
fn ic11_the_clamp_logs_every_bound_that_bit() {
    // 49 t of fittings aboard at Stickney: the alloys fit 24 of 55 by volume.
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[("industrial_parts", 49)]);
    let hull = ObjId(row.hull);
    let alloys = price_of(
        &rig.tw,
        listing_of(&rig.tw, "stickney_port", "kasei", "structural_alloys"),
    );
    let _ = order(&mut rig, pilot, "buy structural_alloys 55");
    let _ = drive_until(&mut rig, 30, |rig| {
        order_state(&rig.tw, hull, 1) == Some(OrderState::Done)
    });
    assert_eq!(
        log_texts(&rig.tw, hull),
        vec![
            BOUGHT_SHORT
                .replace("{qty}", "24")
                .replace("{unit}", "t")
                .replace("{label}", "structural alloy stock")
                .replace("{price}", &cr(alloys))
                .replace("{total}", &cr(24 * alloys))
                .replace("{want}", "55")
                .replace("{bounds}", "hold volume")
        ]
    );

    // An empty hold at Ceres: the ice clamps to 85 t by volume.
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    let ice = price_of(
        &rig.tw,
        listing_of(&rig.tw, "ceres_port", "occator", "volatiles"),
    );
    let _ = order(&mut rig, pilot, "buy volatiles 100");
    let _ = drive_until(&mut rig, 30, |rig| {
        order_state(&rig.tw, hull, 1) == Some(OrderState::Done)
    });
    assert_eq!(
        log_texts(&rig.tw, hull),
        vec![
            BOUGHT_SHORT
                .replace("{qty}", "85")
                .replace("{unit}", "t")
                .replace("{label}", "water ice")
                .replace("{price}", &cr(ice))
                .replace("{total}", &cr(85 * ice))
                .replace("{want}", "100")
                .replace("{bounds}", "hold volume")
        ]
    );

    // A hold full at both limits (the vertex): nothing fits, both named.
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    let hold = hold_vault(&rig.tw, &row);
    let (metals, ice) = vertex_load();
    seed_pool(&mut rig, hold, "metals", metals, LotId(990));
    seed_pool(&mut rig, hold, "volatiles", ice, LotId(991));
    let _ = order(&mut rig, pilot, "buy metals 10");
    let _ = drive(&mut rig, 2);
    assert_eq!(order_state(&rig.tw, hull, 1), Some(OrderState::Done));
    assert_eq!(
        log_texts(&rig.tw, hull),
        vec![
            BOUGHT_NONE
                .replace("{unit}", "t")
                .replace("{label}", "metal concentrate")
                .replace("{want}", "10")
                .replace("{bounds}", "hold mass and hold volume")
        ]
    );
    assert_eq!(pool(&mut rig.tw, hold, "metals"), metals);
}

/// **`fill` and the cargo clamp plan at the depart instant** (§1 lens 7; D-18-41, lead
/// note 2b): under a moving sky (`[tacsim.time.ephemeris].rate = 1000`) the crew-days
/// of the passage at the depart's `not_before` differ from those now; the fill buys
/// the `not_before` figure, and a cash-bound cargo buy queued before it reserves the
/// `not_before` stores.
#[test]
#[allow(clippy::too_many_lines)]
fn fill_and_clamp_plan_at_the_depart_instant() {
    let lane = lane_variant(LANE_VERSION, |doc| {
        let exchange = vendor(doc, "ceres_port", "freeport_exchange");
        exchange["asks"][0]["price_major"] = serde_json::json!(1_000);
    });
    let mut rig = stocked_with(Roster {
        seed_lane: Some(lane),
        sky_rate: 1_000,
    });
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    let hold = hold_vault(&rig.tw, &row);
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot, "Salt Moth");
    seed_pool(&mut rig, hold, "propellant", 80, LotId(992));
    let metals_price = price_of(
        &rig.tw,
        listing_of(&rig.tw, "ceres_port", "freeport_exchange", "metals"),
    );
    let food = price_of(
        &rig.tw,
        listing_of(&rig.tw, "ceres_port", "coop", "provisions"),
    );
    let planner = lane_planner(1_000, "ceres_port", "stickney");
    let goods = tacsim::definitions::goods::goods_table().expect("goods");
    let live = readings(&mut rig, &row);
    let buy_metals = tacsim::port::What::Buy {
        good: cid("metals"),
        qty: tacsim::port::Qty::Units(100),
        max_major: None,
        on_short: tacsim::port::OnShort::Partial,
    };
    let (projected, extra) =
        tacsim::port::project_cargo(&goods, &live, &[&buy_metals]).expect("projects");
    let a0 = pilot_available(&rig.tw, pilot);
    let now = rig.tw.world().now();
    let ic_of = |rig: &Rig, utc: i64| rig.tw.world().world_timeline().current().ic_at(utc);
    // The cargo step runs on the first fire after the filing (each fire is 6 s on) and
    // the fill once the metals read drawn (within a market sweep or two): "now" for a
    // planner that wrongly used it is any of these instants, so the not-before figure
    // must differ from each of them.
    let cargo_window: Vec<i64> = (1..=5).map(|k| now + 6 * k).collect();
    let fill_window: Vec<i64> = (1..=30).map(|k| now + 6 * k).collect();
    // The tank is full (its term is 0), so the reserve is the locker's target priced
    // at the Co-op's provisions.
    let reserve_at = |days: u64| i64::try_from(days).expect("days") * food;
    let tonnes = |funds: i64, reserve: i64| {
        100.min(u64::try_from((funds - reserve).max(0) / metals_price).expect("units"))
    };
    // The first +Nh whose not-before stores differ from every "now" figure, for both
    // the reserve's projection and the fill's own load — with the pilot's funds set so
    // the cargo clamp resolves the difference to a whole tonne: F = the larger reserve
    // + k tonnes − 1 minor, so the larger reserve affords k − 1 tonnes and the smaller
    // affords k.
    let mut chosen = None;
    for n in 1..=48_i64 {
        let due = now + n * 3_600;
        let at_due = targets_for(&live, &projected, extra, &planner, ic_of(&rig, due));
        let r_due = reserve_at(at_due.locker);
        let r_nows: Vec<i64> = cargo_window
            .iter()
            .map(|t| {
                reserve_at(targets_for(&live, &projected, extra, &planner, ic_of(&rig, *t)).locker)
            })
            .collect();
        if r_nows.contains(&r_due) {
            continue;
        }
        let r_max = r_nows.iter().copied().fold(r_due, i64::max);
        let k = ((a0 - r_max) / metals_price).min(10);
        assert!(k >= 2, "the stake affords a few tonnes of metals");
        let funds = r_max + k * metals_price - 1;
        let q = tonnes(funds, r_due);
        if r_nows.iter().any(|r| tonnes(funds, *r) == q) {
            continue;
        }
        let loaded = BTreeMap::from([(cid("metals"), q)]);
        let mut after = live.clone();
        for (id, _, contents) in &mut after {
            if id.as_str() == "cargo" {
                contents.insert(cid("metals"), q);
            }
        }
        let fill_due = targets_for(&after, &loaded, 0, &planner, ic_of(&rig, due));
        let fill_differs = fill_window.iter().all(|t| {
            targets_for(&after, &loaded, 0, &planner, ic_of(&rig, *t)).locker != fill_due.locker
        });
        if fill_differs {
            chosen = Some((n, funds, r_due, q, fill_due.locker));
            break;
        }
    }
    let (n, funds, reserve, q, locker_due) =
        chosen.expect("a departure hour whose stores differ from every now's");
    assert!(q < 100, "the metals buy is cash-bound");
    // Spend the pilot's funds down to F (staff move the difference to a house).
    let mine = account_of(&rig.tw, &PrincipalId::Object(pilot));
    let theirs = org_account(&rig.tw, "stickney_port", "kasei");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/pay {mine}={theirs} {} drawn down", a0 - funds),
    );
    assert_eq!(pilot_available(&rig.tw, pilot), funds, "{out}");
    for line in [
        "buy metals 100".to_owned(),
        "buy provisions fill".to_owned(),
        format!("depart stickney at=+{n}h"),
    ] {
        let out = order(&mut rig, pilot, &line);
        assert!(out.contains("filed"), "{out}");
    }
    let _ = drive_until(&mut rig, 60, |rig| {
        order_state(&rig.tw, hull, 2) == Some(OrderState::Done)
    });
    assert_eq!(
        pool(&mut rig.tw, hold, "metals"),
        q,
        "the cargo buy reserved the not-before stores ({reserve})"
    );
    assert_eq!(
        pool(&mut rig.tw, hold, "provisions"),
        locker_due,
        "the fill bought the not-before crew-days"
    );
}

// ---------------------------------------------------------------------------
// 6. Two pilots, the pilot ashore, passengers, rates, stops and holds
// ---------------------------------------------------------------------------

/// **Two pilots with the same due time both complete** (§1 lens 8): both burn and
/// both launch; no fan-out void, no hold.
#[test]
fn ic11_two_pilots_with_the_same_due_time_both_complete() {
    let mut rig = stocked();
    let (moth, moth_row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let (heron, heron_row) = chartered(&mut rig, "Other", 2, "Grey Heron");
    provision(&mut rig, moth, &moth_row, "stickney", 960);
    provision(&mut rig, heron, &heron_row, "stickney", 970);
    board(&mut rig, moth, "Salt Moth");
    board(&mut rig, heron, "Grey Heron");
    let at = (rig.tw.world().now() + 300 + 59) / 60 * 60;
    let line = format!("depart stickney at={}", hhmm(at));
    for who in [moth, heron] {
        let out = order(&mut rig, who, &line);
        assert!(out.contains("Order 1 filed"), "{out}");
    }
    let (a, b) = (ObjId(moth_row.hull), ObjId(heron_row.hull));
    let now = rig.tw.world().now();
    sweep(&mut rig.tw, at - now - 1);
    let _ = drain(&mut rig.tw);
    let landed = drive_until(&mut rig, 20, |rig| {
        order_state(&rig.tw, a, 1) == Some(OrderState::Done)
            && order_state(&rig.tw, b, 1) == Some(OrderState::Done)
    });
    let mut keys = burn_keys(&landed);
    keys.sort();
    let mut want = vec![
        format!("tacsim.port.burn/{}/1/1", a.0),
        format!("tacsim.port.burn/{}/1/1", b.0),
    ];
    want.sort();
    assert_eq!(keys, want, "both burn");
    assert!(
        under_way(&rig.tw, a) && under_way(&rig.tw, b),
        "both launch"
    );
    for hull in [a, b] {
        let doc = orders(&rig.tw, hull);
        assert!(doc.hold.is_none());
        assert!(
            !doc.log
                .iter()
                .any(|l| l.text.contains(" waits: ") || l.text.contains(" holds: ")),
            "{:?}",
            doc.log
        );
    }
}

/// **A pilot ashore holds the departure** (lead note 2d, Q-18-17): the waiting note
/// `HELD_ASHORE` once, nothing burned; boarding again lets her go by herself.
#[test]
fn ic11_a_pilot_ashore_holds_the_departure() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    provision(&mut rig, pilot, &row, "stickney", 960);
    let _ = order(&mut rig, pilot, "depart stickney");
    let mut burns = 0;
    for _ in 0..5 {
        let (_, queued) = fire(&mut rig);
        burns += burn_keys(&queued).len();
        land_all(&mut rig, &queued);
    }
    assert_eq!(burns, 0, "nothing burned");
    let waits = WAITING_LINE.replace("{n}", "1").replace(
        "{reason}",
        &HELD_ASHORE
            .replace("{name}", "Salt Moth")
            .replace("{port}", "Ceres Freeport"),
    );
    assert_eq!(log_texts(&rig.tw, hull), vec![waits]);
    board(&mut rig, pilot, "Salt Moth");
    let landed = drive_until(&mut rig, 12, |rig| {
        order_state(&rig.tw, hull, 1) == Some(OrderState::Done)
    });
    assert_eq!(burn_keys(&landed).len(), 1);
    assert!(orders(&rig.tw, hull).hold.is_none());
}

/// **A listed passenger ashore is left behind** (Q-18-17, lead ruling L11): aboard
/// when the depart is filed, ashore at its hour — the hull sails, and both are told.
#[test]
fn ic11_a_listed_passenger_ashore_is_left_behind() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    board(&mut rig, pilot, "Salt Moth");
    let mate = rig.tw.spawn_player("Mate");
    let airlock = room_of(&row, "airlock");
    teleport(&mut rig.tw, mate, airlock);
    support::with_ctx(&mut rig.tw, |ctx| {
        mush_sim::place_interior(ctx, mate, airlock, None)
    })
    .expect("the mate is placed aboard");
    provision(&mut rig, pilot, &row, "stickney", 960);
    let out = order(&mut rig, pilot, "depart stickney at=+5m");
    assert!(out.contains("Order 1 filed"), "{out}");
    match &orders(&rig.tw, hull).order(1).expect("order 1").what {
        tacsim::port::What::Depart { crew, .. } => assert_eq!(crew, &vec![mate]),
        other => panic!("{other:?}"),
    }
    let out = run(&mut rig.tw, mate, "+ship/leave");
    assert!(out.contains("You cycle the lock out into"), "{out}");
    disconnect(&mut rig.tw, pilot);
    disconnect(&mut rig.tw, mate);
    sweep(&mut rig.tw, 300);
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    for _ in 0..20 {
        if order_state(&rig.tw, hull, 1) == Some(OrderState::Done) {
            break;
        }
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
    }
    assert_eq!(
        order_state(&rig.tw, hull, 1),
        Some(OrderState::Done),
        "{:?} {:?} {:?}",
        log_texts(&rig.tw, hull),
        pending_notices(rig.tw.world(), pilot),
        voyage(&rig.tw, hull)
    );
    assert!(under_way(&rig.tw, hull), "she sails");
    assert!(
        pending_notices(rig.tw.world(), pilot).contains(
            &LEFT_BEHIND_PILOT
                .replace("{who}", "Mate")
                .replace("{port}", "Ceres Freeport")
        ),
        "{:?}",
        pending_notices(rig.tw.world(), pilot)
    );
    assert!(
        pending_notices(rig.tw.world(), mate).contains(
            &LEFT_BEHIND_PASSENGER
                .replace("{name}", "Salt Moth")
                .replace("{dest}", "Stickney Yards")
                .replace("{port}", "Ceres Freeport")
        ),
        "{:?}",
        pending_notices(rig.tw.world(), mate)
    );
}

/// **A rate change relabels nothing** (doc 18 §4.4 "IC-stamped conditions relabel on
/// a rate generation like journeys"): a depart filed `at=+1h` at rate 3600, the rate
/// changed to 24 and back before the hour — it fires at its UTC hour, not before, and
/// the order is unchanged in shape.
#[test]
fn ic11_a_rate_change_relabels_nothing() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot, "Salt Moth");
    provision(&mut rig, pilot, &row, "stickney", 960);
    let due = rig.tw.world().now() + 3_600;
    let _ = order(&mut rig, pilot, "depart stickney at=+1h");
    let what = orders(&rig.tw, hull)
        .order(1)
        .expect("order 1")
        .what
        .clone();
    set_rate(&mut rig.tw, 24);
    let landed = drive(&mut rig, 3);
    assert!(burn_keys(&landed).is_empty());
    set_rate(&mut rig.tw, 3_600);
    assert_eq!(
        orders(&rig.tw, hull).order(1).expect("order 1").what,
        what,
        "the order is unchanged"
    );
    let now = rig.tw.world().now();
    sweep(&mut rig.tw, due - now - 1);
    let _ = drain(&mut rig.tw);
    assert!(
        voyage(&rig.tw, hull).and_then(|v| v.pending).is_none(),
        "not before its hour"
    );
    let landed = drive_until(&mut rig, 6, |rig| {
        voyage(&rig.tw, hull).and_then(|v| v.pending).is_some()
            || order_state(&rig.tw, hull, 1) == Some(OrderState::Done)
    });
    assert!(rig.tw.world().now() >= due, "at its UTC hour");
    let _ = landed;
    assert!(
        log_texts(&rig.tw, hull).first().is_some_and(|l| l
            == &DEPART_SET
                .replace("{port}", "Stickney Yards")
                .replace("{hhmm}", &hhmm(due))),
        "{:?}",
        log_texts(&rig.tw, hull)
    );
}

/// **A stop pauses the plan and resume runs on** (owner item O5).
#[test]
fn ic11_a_stop_pauses_and_resume_runs_on() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    let hold = hold_vault(&rig.tw, &row);
    let _ = order(&mut rig, pilot, "stop");
    let _ = order(&mut rig, pilot, "buy volatiles 5");
    let _ = drive(&mut rig, 6);
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.state, PlanState::Paused);
    assert_eq!(doc.order(1).map(|o| o.state), Some(OrderState::Done));
    assert_eq!(log_texts(&rig.tw, hull), vec![STOPPED.replace("{n}", "1")]);
    assert_eq!(pool(&mut rig.tw, hold, "volatiles"), 0, "nothing more runs");
    let out = order(&mut rig, pilot, "resume");
    assert!(out.contains(&RESUMED.replace("{n}", "2")), "{out}");
    let _ = drive_until(&mut rig, 30, |rig| {
        order_state(&rig.tw, hull, 2) == Some(OrderState::Done)
    });
    assert_eq!(pool(&mut rig.tw, hold, "volatiles"), 5);
}

/// **Held steps wait for resume** (owner item O4): a `short=hold` buy holds and buys
/// nothing, and `resume` retries it; a past-window depart holds, and `resume` sails
/// now.
#[test]
fn ic11_held_steps_wait_for_resume() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    let hold = hold_vault(&rig.tw, &row);
    let _ = order(&mut rig, pilot, "buy volatiles 100 short=hold");
    let _ = drive(&mut rig, 4);
    let held = HELD_LINE
        .replace("{n}", "1")
        .replace("{reason}", "short of 100 t: hold volume.");
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.state, PlanState::Held);
    assert_eq!(log_texts(&rig.tw, hull), vec![held.clone()]);
    assert_eq!(
        pool(&mut rig.tw, hold, "volatiles"),
        0,
        "holds consume nothing"
    );
    let out = order(&mut rig, pilot, "resume");
    assert!(out.contains(&RESUMED.replace("{n}", "1")), "{out}");
    let _ = drive(&mut rig, 3);
    assert_eq!(
        log_texts(&rig.tw, hull),
        vec![held.clone(), held],
        "resume retried it"
    );
    let _ = order(&mut rig, pilot, "clear");

    // A fixed hour reached past its window holds; resume sails now.
    board(&mut rig, pilot, "Salt Moth");
    provision(&mut rig, pilot, &row, "stickney", 960);
    let due = rig.tw.world().now() + 60;
    let out = order(&mut rig, pilot, "depart stickney at=+1m until=+1m");
    assert!(out.contains("filed"), "{out}");
    let id = orders(&rig.tw, hull).cursor.expect("the depart");
    sweep(&mut rig.tw, 200);
    let _ = drain(&mut rig.tw);
    let _ = drive(&mut rig, 2);
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.state, PlanState::Held);
    assert_eq!(
        doc.hold.as_ref().map(|h| h.reason.clone()),
        Some(
            HELD_WINDOW
                .replace("{port}", "Stickney Yards")
                .replace("{hhmm}", &hhmm(due))
        )
    );
    let out = order(&mut rig, pilot, "resume");
    assert!(out.contains("runs again"), "{out}");
    let landed = drive_until(&mut rig, 12, |rig| {
        order_state(&rig.tw, hull, id) == Some(OrderState::Done)
    });
    assert_eq!(burn_keys(&landed).len(), 1, "resume sails now");
    // Under way, an order filed aboard runs at the next port: the render names it.
    assert!(under_way(&rig.tw, hull));
    let out = order(&mut rig, pilot, "buy metals 5");
    assert!(out.contains("filed"), "{out}");
    let next = orders(&rig.tw, hull).cursor.expect("the buy");
    let out = run(&mut rig.tw, pilot, "+ship/orders");
    for want in [
        format!("  {next}. buy metal concentrate, 5 t — next, at Stickney Yards"),
        format!("Next due: order {next} when the Salt Moth docks at Stickney Yards."),
    ] {
        assert!(out.lines().any(|l| l == want), "missing {want:?}:\n{out}");
    }
}

// ---------------------------------------------------------------------------
// 7. +ship/plan over the queue, the status line (item 5)
// ---------------------------------------------------------------------------

/// The lowest live ask price for `good` at `entry` (minor), over its vendors.
fn lowest_ask(rig: &Rig, entry: &str, good: &CanonicalId) -> Option<i64> {
    let def = port_def(entry);
    let map = port_map(&rig.tw, entry);
    def.vendors
        .iter()
        .filter(|v| v.asks.iter().any(|a| &a.good == good))
        .filter_map(|v| {
            let vault = VaultId(map.vendors.get(&v.org)?.vault);
            let listing = market(&rig.tw).lot_listing_of(EndpointRef(vault), good)?;
            Some(price_of(&rig.tw, listing))
        })
        .min()
}

/// `+ship/plan`'s affordable line at `entry` (the §5 row's "affordable tonnage at
/// this board"), computed here: every ask on the board in the board's order, at its
/// lowest live price, `floor(max(0, available − reserve) / price)`.
fn affordable_line(rig: &Rig, entry: &str, available: i64, reserve: i64) -> String {
    let goods = tacsim::definitions::goods::goods_table().expect("goods");
    let def = port_def(entry);
    let mut asked: Vec<CanonicalId> = Vec::new();
    for ask in def.vendors.iter().flat_map(|v| v.asks.iter()) {
        if !asked.contains(&ask.good) {
            asked.push(ask.good.clone());
        }
    }
    let parts: Vec<String> = asked
        .iter()
        .filter_map(|good| {
            let price = lowest_ask(rig, entry, good)?;
            let row = goods.good(good).expect("a good");
            let unit = match row.ledger_unit.as_str() {
                "crew_day" => "crew-days".to_owned(),
                other => other.to_owned(),
            };
            Some(format!(
                "{} {} {unit}",
                row.label,
                (available - reserve).max(0) / price
            ))
        })
        .collect();
    format!(
        "Affordable at this board, after the next departure's fuel and stores: {}.",
        parts.join(", ")
    )
}

/// **`+ship/plan` dry-runs the queue** (doc 18 §4.4; the §5 row "each buy's clamp and
/// bound, the affordable tonnage at this board"): at Stickney with the cash-bound
/// return queue — the fills at the depart's `not_before`, each buy's clamp and bound
/// (the alloys short by funds), the depart, the affordable line, `NOTHING_WRITTEN`
/// last; no store op, no submission.
#[test]
#[allow(clippy::too_many_lines)]
fn ic11_the_plan_dry_runs_the_queue() {
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[]);
    let hull = ObjId(row.hull);
    let hold = hold_vault(&rig.tw, &row);
    for line in [
        "buy propellant fill",
        "buy provisions fill",
        "buy industrial_parts 49",
        "buy structural_alloys 55",
        "depart ceres",
    ] {
        let _ = order(&mut rig, pilot, line);
    }
    let tank = pool(&mut rig.tw, hold, "propellant");
    let locker = pool(&mut rig.tw, hold, "provisions");
    let fuel = price_of(
        &rig.tw,
        listing_of(&rig.tw, "stickney_port", "kasei", "propellant"),
    );
    let food = price_of(
        &rig.tw,
        listing_of(&rig.tw, "stickney_port", "yard_commissary", "provisions"),
    );
    let parts = price_of(
        &rig.tw,
        listing_of(
            &rig.tw,
            "stickney_port",
            "stickney_yards",
            "industrial_parts",
        ),
    );
    let alloys = price_of(
        &rig.tw,
        listing_of(&rig.tw, "stickney_port", "kasei", "structural_alloys"),
    );
    let a0 = pilot_available(&rig.tw, pilot);
    let fill_t = 50 - tank;
    let fill_d = 19 - locker;
    let k = (a0
        - i64::try_from(fill_t).expect("t") * fuel
        - i64::try_from(fill_d).expect("d") * food
        - 49 * parts)
        / alloys;
    let _ = rig.tw.take_module_ops();
    let out = run(&mut rig.tw, pilot, "+ship/plan");
    let lines: Vec<&str> = out.lines().collect();
    for want in [
        "The sailing plan from order 1, as it stands now:".to_owned(),
        format!(
            "  Order 1, buy reaction mass, fill: {fill_t} t, tank 50/80, for the 73 t your orders can load."
        ),
        format!(
            "  Order 2, buy voyage provisions, fill: {fill_d} crew-days, locker 19/120, for the 73 t your orders can load."
        ),
        format!(
            "  Order 3, buy yard fittings, crated: 49 t at {}/t.",
            cr(parts)
        ),
        format!(
            "  Order 4, buy structural alloy stock: {k} t at {}/t; short of 55 t: funds.",
            cr(alloys)
        ),
    ] {
        assert!(lines.iter().any(|l| *l == want), "missing {want:?}:\n{out}");
    }
    // The depart, planned on the projected load (the clamped alloys, not the 73 t
    // the fills were sized for) at its not-before (now: `depart ceres`), with the one
    // allowance of an empty hull (the pilot is ashore; the planner counts at least one).
    let goods = tacsim::definitions::goods::goods_table().expect("goods");
    let mass = |good: &str| {
        goods
            .good(&cid(good))
            .and_then(tacsim::definitions::GoodDef::mass_kg_per_unit)
            .expect("a mass")
    };
    let allowance = {
        let dray = load_pack(tacsim::definitions::content::DRAY_JSON).expect("the Dray pack");
        let Some(PackEntry::Unit(spec)) = dray.entry(&cid(tacsim::definitions::content::DRAY))
        else {
            panic!("the Dray unit");
        };
        u64::from(
            spec.logistics
                .as_ref()
                .expect("logistics")
                .occupant_allowance_kg,
        )
    };
    let k_t = u64::try_from(k).expect("tonnes");
    let payload = 49 * mass("industrial_parts")
        + k_t * mass("structural_alloys")
        + 50 * mass("propellant")
        + 19 * mass("provisions")
        + allowance;
    let planner = lane_planner(0, "stickney_port", "ceres");
    let plan = planner(payload, 1, ic_now(&rig.tw)).expect("the return plans");
    assert!(
        plan.burn_t <= 50 && u64::from(plan.crew_days) <= 19,
        "{plan:?}"
    );
    let tenths = (payload + 50) / 100;
    let depart = format!(
        "  Order 5, depart for Ceres Freeport: payload {}.{} t, burn {} t, {} crew-days.",
        tenths / 10,
        tenths % 10,
        plan.burn_t,
        plan.crew_days
    );
    assert!(
        lines.iter().any(|l| *l == depart),
        "missing {depart:?}:\n{out}"
    );
    // The affordable tonnage at this board: the pilot's funds now, less the next
    // departure's unmet fuel and stores (the fills' targets, 37 t and 20 crew-days),
    // over every ask on the board at its lowest live price, in the board's order.
    let ask = |good: &str| lowest_ask(&rig, "stickney_port", &cid(good)).expect("an ask");
    let reserve = i64::try_from(fill_t).expect("t") * ask("propellant")
        + i64::try_from(fill_d).expect("d") * ask("provisions");
    let affordable = affordable_line(&rig, "stickney_port", a0, reserve);
    assert!(
        lines.iter().any(|l| *l == affordable),
        "missing {affordable:?}:\n{out}"
    );
    assert_eq!(lines.last().copied(), Some(NOTHING_WRITTEN), "{out}");
    let ops = rig.tw.take_module_ops();
    assert!(
        ops.iter().all(|op| op.ns.as_str() != tacsim::NS),
        "the dry run writes no tacsim document: {ops:?}"
    );
    assert!(drain(&mut rig.tw).is_empty(), "the dry run submits nothing");
    assert_eq!(orders(&rig.tw, hull).rev, 5);
}

/// **The status line names the departure hour** (doc 18 §2 step 15): `look` aboard
/// and `+ship` show the `departs` clause while a depart is the next order; with no
/// plan both read as before.
#[test]
fn ic11_the_status_line_names_the_departure_hour() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let mark = row.mark.clone();
    board(&mut rig, pilot, "Salt Moth");
    let before = format!("Aboard the Salt Moth ({mark}), docked at Ceres Freeport.");
    let out = run(&mut rig.tw, pilot, "look");
    assert!(out.contains(&before), "{out}");
    let at = rig.tw.world().now() + 7_200;
    let _ = order(&mut rig, pilot, "depart stickney at=+2h");
    let out = run(&mut rig.tw, pilot, "look");
    assert!(
        out.contains(&format!(
            "Aboard the Salt Moth ({mark}), docked at Ceres Freeport; departs {} UTC (in 2 h 0 min).",
            hhmm(at)
        )),
        "{out}"
    );
    // Doc 18 cut IC14 row 42: the same status line from the Bridge (the hull's, from
    // any of her rooms), walked through her own exits and back.
    let _ = run(&mut rig.tw, pilot, "in");
    let _ = run(&mut rig.tw, pilot, "bridge");
    assert_eq!(location(&rig.tw, pilot), Some(room_of(&row, "bridge")));
    let out = run(&mut rig.tw, pilot, "look");
    assert!(
        out.contains(&format!(
            "Aboard the Salt Moth ({mark}), docked at Ceres Freeport; departs {} UTC (in 2 h 0 min).",
            hhmm(at)
        )),
        "the Bridge: {out}"
    );
    let _ = run(&mut rig.tw, pilot, "down");
    let _ = run(&mut rig.tw, pilot, "airlock");
    assert_eq!(location(&rig.tw, pilot), Some(room_of(&row, "airlock")));
    let out = run(&mut rig.tw, pilot, "+ship");
    assert!(
        out.contains(&format!("; departs {} UTC (in 2 h 0 min).", hhmm(at))),
        "{out}"
    );
    // With the depart at the cursor, `+ship/plan`'s affordable line reserves THAT
    // departure's fuel and stores (the next departure is the cursor itself).
    let hold = hold_vault(&rig.tw, &row);
    let tank0 = pool(&mut rig.tw, hold, "propellant");
    let locker0 = pool(&mut rig.tw, hold, "provisions");
    let live = readings(&mut rig, &row);
    let planner = lane_planner(0, "ceres_port", "stickney");
    let due_ic = rig.tw.world().world_timeline().current().ic_at(at);
    let targets = targets_for(&live, &BTreeMap::new(), 0, &planner, due_ic);
    let ask = |good: &str| lowest_ask(&rig, "ceres_port", &cid(good)).expect("an ask");
    let reserve = i64::try_from(targets.tank - tank0).expect("t") * ask("propellant")
        + i64::try_from(targets.locker - locker0).expect("d") * ask("provisions");
    assert!(reserve > 0, "an empty tank and locker need filling");
    let a0 = pilot_available(&rig.tw, pilot);
    let out = run(&mut rig.tw, pilot, "+ship/plan");
    let affordable = affordable_line(&rig, "ceres_port", a0, reserve);
    assert!(
        out.lines().any(|l| l == affordable),
        "missing {affordable:?}:\n{out}"
    );
    let _ = order(&mut rig, pilot, "clear");
    let out = run(&mut rig.tw, pilot, "look");
    assert!(out.contains(&before), "{out}");

    // A lapsed tenant's plan names no hour (L14): the arm never runs it, so the
    // status renders do not either. The charter ended, the status line on her own
    // `look` (the hull's, whoever looks) drops the clause.
    let _ = order(&mut rig, pilot, "depart stickney at=+2h");
    let out = run(&mut rig.tw, pilot, "look");
    assert!(out.contains("; departs "), "{out}");
    let lease = tacsim::port::live_charter(rig.tw.world(), pilot)
        .expect("the charter")
        .contract;
    let _ = run(&mut rig.tw, GOD, &format!("@contract/cancel #{}", lease.0));
    assert_eq!(tacsim::port::live_charter(rig.tw.world(), pilot), None);
    let out = run(&mut rig.tw, pilot, "look");
    assert!(out.contains(", docked at Ceres Freeport."), "{out}");
    assert!(!out.contains("; departs "), "{out}");

    // Nor to the hull's next tenant. At HEAD a revoked Dray is not chartered again
    // while its hold vault stands (`coop::hull_is_free`, until IC13's recall), so the
    // next tenant is built the way `ic11_a_new_tenant_files_a_fresh_plan` builds one:
    // a second pilot's hull, whose plan is re-signed to the lapsed tenant through
    // `reboot_editing` — another author's plan on the tenant's hull.
    let (other, other_row) = chartered(&mut rig, "Other", 2, "Grey Heron");
    let other_hull = ObjId(other_row.hull);
    let other_mark = other_row.mark.clone();
    board(&mut rig, other, "Grey Heron");
    let _ = order(&mut rig, other, "depart stickney at=+2h");
    let out = run(&mut rig.tw, other, "+ship");
    assert!(out.contains("; departs "), "{out}");
    let mut rig = reboot_editing(rig, |folded| {
        edit_orders(folded, other_hull, |doc| {
            doc["author"] = serde_json::json!(pilot.0);
        });
    });
    let _ = rig.tw.connect(other);
    assert_eq!(orders(&rig.tw, other_hull).author, pilot, "the lapsed plan");
    let out = run(&mut rig.tw, other, "look");
    assert!(
        out.contains(&format!(
            "Aboard the Grey Heron ({other_mark}), docked at Ceres Freeport."
        )),
        "{out}"
    );
    assert!(!out.contains("; departs "), "{out}");
    let out = run(&mut rig.tw, other, "+ship");
    assert!(!out.contains("; departs "), "{out}");
}

// ---------------------------------------------------------------------------
// Doc 18 cut IC12: `+ship/order resume` clears a refused call's count
// ---------------------------------------------------------------------------

/// **`+ship/order resume` clears the refusal count** (doc 18 cut IC12 item 1.8,
/// F14): a plan stopped at its third refused closed call — order 1 `Held` with
/// `refusals` 3 and a `retry_at`, the plan `Held` on a stopped note naming it —
/// runs again from order 1 after `resume`, the order pending with `refusals` 0 and
/// no `retry_at`, and the document still checks. The stopped shape is written
/// through the folded store (`reboot_editing`, this file's corrupt-document idiom),
/// because no step on the shipped composition reaches `refused_call` reproducibly
/// (the unit `ic12_a_refused_call_logs_once_and_holds_at_three` covers the count).
#[test]
fn ic12_resume_clears_the_refusal_count() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    let out = order(&mut rig, pilot, "buy volatiles 5");
    assert!(out.contains("Order 1 filed"), "{out}");
    let reason = "That listing is not taking quotes.";
    let mut rig = reboot_editing(rig, |folded| {
        edit_orders(folded, hull, |doc| {
            doc["state"] = serde_json::json!("held");
            doc["hold"] = serde_json::json!({
                "order": 1, "reason": reason, "since": 1, "retry": false
            });
            for order in doc["orders"].as_array_mut().expect("orders") {
                if order["id"] == 1 {
                    order["state"] = serde_json::json!("held");
                    order["refusals"] = serde_json::json!(3);
                    order["retry_at"] = serde_json::json!(1_000);
                }
            }
        });
    });
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.state, PlanState::Held);
    assert_eq!(doc.order(1).map(|o| o.refusals), Some(3));
    let _ = rig.tw.connect(pilot);
    let out = order(&mut rig, pilot, "resume");
    assert!(out.contains(&RESUMED.replace("{n}", "1")), "{out}");
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.state, PlanState::Running);
    assert!(doc.hold.is_none());
    let resumed = doc.order(1).expect("order 1");
    assert_eq!(resumed.state, OrderState::Pending);
    assert_eq!(resumed.refusals, 0, "the count is cleared");
    assert_eq!(resumed.retry_at, None);
    doc.check().expect("the plan still checks");
}

// ---------------------------------------------------------------------------
// Doc 18 cut IC14 row 2: a full engine queue is back-pressure, not a refusal
// ---------------------------------------------------------------------------

/// **A full engine queue defers the sell step and counts nothing** (doc 18 cut IC14
/// row 2; reviewer F3). With 64 other actors' submissions queued, a queued `sell` meets
/// the market's `SELL_QUEUE_FULL` before any contract: the step waits (`Deferred`), the
/// order's refusal count stays 0, no `REFUSED_RETRY` line reaches the plan's log and
/// the stored plan is unchanged across three busy fires. Once the queue drains the
/// sale proceeds.
#[test]
fn ic14_a_full_queue_defers_the_sell_step_and_counts_nothing() {
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let hull = ObjId(row.hull);
    let _ = order(&mut rig, pilot, "sell metals all");
    disconnect(&mut rig.tw, pilot);
    let action = mush_market::lot_action_ref(&rig.tw.engine().registry).expect("the lot action");
    for i in 0..64_u32 {
        rig.tw
            .engine_mut()
            .queue_authoritative_submit(Box::new(AuthoritativeSubmit {
                actor: ObjId(900_000 + i),
                action: action.clone(),
                subject: mush_core::PerceivedRef::SelfActor,
                targets: Vec::new(),
                parameters: serde_json::json!({}),
                key: mush_core::IdempotencyKeyText::parse_canonical("ic14.filler").expect("a key"),
            }))
            .expect("the queue has room");
    }
    let before = orders(&rig.tw, hull);
    let contracts = economy(&rig.tw).snapshot().contracts().count();
    for _ in 0..3 {
        let t = rig.tw.world().now().saturating_add(6);
        rig.tw.set_now(t);
        let _ = rig.tw.fire_due(t.saturating_mul(1_000));
        let stats = tacsim::port_tick::last_fire(rig.tw.world()).expect("a fire");
        assert!(stats.deferred >= 1, "the sell step waited: {stats:?}");
        assert_eq!(stats.refused, 0, "{stats:?}");
    }
    let after = orders(&rig.tw, hull);
    assert_eq!(after, before, "the plan was not written");
    assert_eq!(
        after.order(1).map(|o| o.refusals),
        Some(0),
        "nothing counted"
    );
    let retry = tacsim::port::executor::REFUSED_RETRY.replace("{n}", "1");
    assert!(
        !log_texts(&rig.tw, hull)
            .iter()
            .any(|line| line.starts_with(retry.split("{line}").next().unwrap_or_default())),
        "{:?}",
        log_texts(&rig.tw, hull)
    );
    assert_eq!(
        economy(&rig.tw).snapshot().contracts().count(),
        contracts,
        "no contract while the queue is full"
    );
    // The queue drains (the driver takes the fillers); the sale proceeds.
    while rig.tw.engine_mut().take_authoritative_submit().is_some() {}
    let _ = drive_until(&mut rig, 10, |rig| {
        market(&rig.tw)
            .quotes_of_seller(&PrincipalId::Object(pilot))
            .count()
            > 0
    });
    assert_eq!(orders(&rig.tw, hull).order(1).map(|o| o.refusals), Some(0));
}

// ---------------------------------------------------------------------------
// Doc 18 cut IC14 (rows 25–27): the executor's and the plan's residuals
// ---------------------------------------------------------------------------

/// Set every `"to"` string in `value` equal to `from` to `to` (a depart's stored venue
/// word, wherever the nested wire keeps it).
fn retarget(value: &mut serde_json::Value, from: &str, to: &str) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, inner) in map.iter_mut() {
                if key == "to" && inner.as_str() == Some(from) {
                    *inner = serde_json::json!(to);
                } else {
                    retarget(inner, from, to);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for inner in items {
                retarget(inner, from, to);
            }
        }
        _ => {}
    }
}

/// **Doc 18 cut IC14 row 25: a queued depart never resolves by prefix.** A stored
/// venue word that is only a prefix of a port's name (`stick`, of Stickney Yards) — the
/// live verb's fallback would fly it there — holds `NO_ROUTE`; the exact key departs.
#[test]
fn ic14_a_queued_depart_never_resolves_by_prefix() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    board(&mut rig, pilot, "Salt Moth");
    provision(&mut rig, pilot, &row, "stickney", 960);
    let _ = order(&mut rig, pilot, "depart stickney");
    let mut rig = reboot_editing(rig, |folded| {
        edit_orders(folded, hull, |doc| retarget(doc, "stickney", "stick"));
    });
    // The reboot left the pilot offline, which the order arm needs.
    let _ = drive(&mut rig, 6);
    let held = HELD_LINE.replace("{n}", "1").replace(
        "{reason}",
        &NO_ROUTE
            .replace("{here}", "Ceres Freeport")
            .replace("{list}", "Ahuna Flank, Freeport Commons, Juling Shadow, Kerwan Floor, Aitken Highport, Occator Rim, Rheasilvia Commons, Stickney Yards"),
    );
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.state, PlanState::Held, "{:#?}", doc.log);
    assert_eq!(log_texts(&rig.tw, hull), vec![held]);
    assert!(
        voyage(&rig.tw, hull).and_then(|v| v.pending).is_none(),
        "nothing filed"
    );
    // The exact key departs.
    let _ = rig.tw.connect(pilot);
    let _ = order(&mut rig, pilot, "del 1");
    let _ = order(&mut rig, pilot, "depart stickney");
    let landed = drive_until(&mut rig, 12, |rig| {
        order_state(&rig.tw, hull, 2) == Some(OrderState::Done)
    });
    assert_eq!(
        burn_keys(&landed),
        vec![format!("tacsim.port.burn/{}/2/1", hull.0)]
    );
}

/// The empty-tank burn to Stickney, from the planner's own figures, with the locker
/// stocked (100 crew-days) so only the tank binds.
fn empty_tank_burn(rig: &mut Rig, pilot: ObjId, row: &HullRow) -> u64 {
    let hold = hold_vault(&rig.tw, row);
    seed_pool(rig, hold, "provisions", 100, LotId(971));
    let plan = run(&mut rig.tw, pilot, "+ship/plan stickney");
    let (burn, tank) = plan_numbers(&plan, "Burn ", "(tank ").expect(&plan);
    assert_eq!(tank, 0, "an empty tank");
    burn
}

/// The plan's log carries no `HELD_SHORT_FOR_DEPART` hold.
fn never_short(rig: &Rig, hull: ObjId) {
    let log = log_texts(&rig.tw, hull);
    assert!(
        !log.iter()
            .any(|line| line.contains("would leave the passage to")),
        "{log:#?}"
    );
}

/// **Doc 18 cut IC14 row 26 (lead ruling L22; Q-18-51's default changed to §4.4's
/// wording): a short buy with a later `fill` does not hold.** One tonne of reaction mass,
/// then a `fill` of it, then the depart: the first buy is short of the depart's need on
/// its own, but the later fill covers it, so nothing holds and she departs.
#[test]
fn ic14_a_short_buy_with_a_later_fill_does_not_hold() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    board(&mut rig, pilot, "Salt Moth");
    let _ = empty_tank_burn(&mut rig, pilot, &row);
    let _ = order(&mut rig, pilot, "buy propellant 1");
    let _ = order(&mut rig, pilot, "buy propellant fill");
    let _ = order(&mut rig, pilot, "depart stickney");
    disconnect(&mut rig.tw, pilot);
    let _ = drive_until(&mut rig, 60, |rig| {
        order_state(&rig.tw, hull, 3) == Some(OrderState::Done)
    });
    assert_eq!(order_state(&rig.tw, hull, 1), Some(OrderState::Done));
    assert_eq!(order_state(&rig.tw, hull, 2), Some(OrderState::Done));
    never_short(&rig, hull);
}

/// **Doc 18 cut IC14 row 26: a short buy with later units that cover does not hold.**
/// One tonne, then the empty-tank burn plus ten more tonnes, then the depart: the later
/// units cover the need, so the first buy goes through and she departs. (Ten, since doc
/// 19 cut A.2: the flip law charges about fifteen percent of the fuel's own mass, so an
/// empty-tank burn of 40 t needs 48 t once 46 are aboard; the coast law's 105 kg per
/// tonne made five enough.)
#[test]
fn ic14_a_short_buy_with_later_units_that_cover_does_not_hold() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    board(&mut rig, pilot, "Salt Moth");
    let burn = empty_tank_burn(&mut rig, pilot, &row);
    let _ = order(&mut rig, pilot, "buy propellant 1");
    let _ = order(&mut rig, pilot, &format!("buy propellant {}", burn + 10));
    let _ = order(&mut rig, pilot, "depart stickney");
    disconnect(&mut rig.tw, pilot);
    let _ = drive_until(&mut rig, 60, |rig| {
        order_state(&rig.tw, hull, 3) == Some(OrderState::Done)
    });
    assert_eq!(order_state(&rig.tw, hull, 1), Some(OrderState::Done));
    never_short(&rig, hull);
}

/// **Doc 18 cut IC14 row 27: pause then resume over an in-flight hold re-arms the
/// re-stage.** A sale whose leg never lands holds `HELD_UNCONFIRMED` at its second
/// deadline (`restaged` 2); `+ship/order pause` takes the hold away and leaves the step
/// in flight; `+ship/order resume` re-arms its re-stage clock — `restaged` 0 and
/// `staged_utc` now — so the step is re-staged again rather than never.
#[test]
fn ic14_pause_then_resume_over_an_inflight_hold_rearms_the_restage() {
    let mut rig = stocked();
    let (pilot, row) = docked_at_stickney(&mut rig, &[("metals", 20)]);
    let hull = ObjId(row.hull);
    let _ = order(&mut rig, pilot, "sell metals all");
    disconnect(&mut rig.tw, pilot);
    for _ in 0..60 {
        let (_, queued) = fire(&mut rig);
        let (_market, rest): (Vec<_>, Vec<_>) = queued
            .into_iter()
            .partition(|s| s.action.key().owner.as_str() == "market");
        land_quietly(&mut rig, &rest);
        if orders(&rig.tw, hull).state == PlanState::Held {
            break;
        }
    }
    let doc = orders(&rig.tw, hull);
    assert_eq!(
        doc.hold.as_ref().map(|h| h.reason.as_str()),
        Some(HELD_UNCONFIRMED)
    );
    assert_eq!(doc.inflight.as_ref().map(|i| i.restaged), Some(2));
    let _ = order(&mut rig, pilot, "pause");
    let paused = orders(&rig.tw, hull);
    assert_eq!(paused.state, PlanState::Paused);
    assert!(paused.hold.is_none(), "the pause took the hold away");
    assert_eq!(paused.inflight.as_ref().map(|i| i.restaged), Some(2));
    let _ = order(&mut rig, pilot, "resume");
    let resumed = orders(&rig.tw, hull);
    assert_eq!(resumed.state, PlanState::Running);
    let inflight = resumed.inflight.expect("still in flight");
    assert_eq!(inflight.restaged, 0, "the re-stage clock is re-armed");
    assert_eq!(inflight.staged_utc, rig.tw.world().now());
}
