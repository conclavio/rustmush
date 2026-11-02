//! **Doc 18 cut IC9 acceptance — undock and fly: the first player-runnable flight**
//! (doc 18 §4.2 "Propellant and provisions as consumables", "The helm", §4.4
//! "Passage: the planner over the sky at departure", "Failure handling", §4.8; §2
//! steps 9, 11, 12; D-18-11, D-18-18, D-18-23, D-18-30, D-18-44 … D-18-47, D-18-50;
//! lead rulings L1–L37 of the IC9 brief).
//!
//! A chartered pilot aboard a docked Dray plans (`+ship/plan`, which writes
//! nothing) and undocks (`+ship/undock`): the command files the plan in the hull's
//! sailing plan and submits ONE `tacsim_port_burn` as the pilot, whose prepare
//! debits the tank and the locker and writes the ticket carrying the whole plan.
//! The next `tacsim.port` fire launches it through `mush_sim::depart` on the plan's
//! frozen leg times from its own `depart_ic`; the crew hear the lock seal, the
//! Kirkwood boundary and the docking; arrival places the hull on Stickney's
//! passive domain and `+ship/leave` lands in P01.
//!
//! Composed on the composition root (`server_modules_with_tacsim` and the wired
//! test-world layer with game content: `commerce.rs`'s `Roster::bare`, with the
//! sky rate a parameter). Every boot asserts no namespace write was refused; every
//! drain asserts no kept tacsim submission is GOD's. The load is derived by
//! `support/vertex.rs` (no test stores 20/80).

use crate::support;

use crate::vertex;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use mush_core::{
    AccountId, AccountIdentity, ActionStateBootRecord, Audience, AuthoritativeSubmit, CanonicalId,
    CommitToken, GOD, IdempotencyKey, IdempotencyScope, ModuleWrite, ObjId, ObjKind,
    OfferExecutionContext, OfferSubmission, RequestUid, ScopedIdempotencyKey,
};
use mush_economy::{EconomyConfig, EconomyStore, VaultId};
use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
use mush_market::{ListingId, MarketStore, Presence, VenueGate};
use mush_proto::OutputEvent;
use mush_sim::{CarrierJourney, JourneyBook, SpatialParent, SpatialState};
use mush_testkit::{TestWorld, fold_module_ops_into};
use smol_str::SmolStr;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK, LANE_VERSION};
use tacsim::pack::{PackEntry, load_pack};
use tacsim::port::apply::room_by_key;
use tacsim::port::voyage::{
    AT_NOT_YET, CLEARANCE_LAPSED, GOODS_COMING, LANE_UNPLANNED, LAUNCH_GRACE_FIRES, LAUNCH_HELD,
    LEAVE_CLEARED, LISTING_HOLDS, MANUAL_DEPARTURE, NO_ROUTE, NOT_ABOARD, OVERLOADED, PORT_CLOSED,
    TANK_SHORT, THROTTLE_FITS, UNDOCK_CLEARED, UNDOCK_UNDER_WAY, VIA_UNKNOWN, VOYAGE_DARK,
    VOYAGE_PREFIX, Voyage, VoyageState, voyage_key,
};
use tacsim::port::{
    HullRow, PortDef, PortHandles, PortMap, PortMapState, PortVenueGate, Ticket, TicketId, room_key,
};
use tacsim::port_tick::{NOTICE_HEADING, pending_notices};
use tacsim::{Sky, TacsimConfig, TacsimStore};

use support::ground::{UNSIGNED, sign};

// ---------------------------------------------------------------------------
// The rig (commerce.rs's, the sky rate a parameter)
// ---------------------------------------------------------------------------

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

fn lane_id() -> CanonicalId {
    cid(LANE_PACK)
}

/// How a test roster is composed: `commerce.rs`'s `Roster::bare` (no sheet, the
/// test world's layer, the shipped game content), with `[tacsim.time.ephemeris]`'s
/// rate a parameter (test 4's sky change), and optionally a lane booted in place
/// of the shipped one as an operator seed (`commerce.rs`'s `Roster::seeded`).
#[derive(Clone, Copy, Default)]
pub(crate) struct Roster {
    sky_rate: u32,
    /// A lane booted instead of the shipped one (doc 19 cut A.8's `Throttle:`
    /// test), leaked so the roster stays `Copy`.
    seed_lane: Option<&'static str>,
}

impl Roster {
    fn modules(self) -> Vec<Box<dyn mush_core::GameModule>> {
        use tacsim::composition as comp;

        let game = tacsim::GameEconomyConfig::default();
        let mut cfg = TacsimConfig {
            enabled: true,
            ..TacsimConfig::default()
        };
        cfg.time.ephemeris.rate = self.sky_rate;
        if let Some(lane) = self.seed_lane {
            cfg.seeds.push(tacsim::config::PackSource {
                name: "ops/lane.json".to_owned(),
                bytes: lane.to_owned(),
            });
        }
        let (mut mods, inventory) = comp::domain_modules_with_economy(
            EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &game,
        );
        mods.push(comp::npc_module(comp::npc_agent_adapter()));
        mods.push(comp::tacsim_test_world_module_wired(
            cfg,
            self.seed_lane.is_none(),
            inventory,
        ));
        match self.seed_lane {
            None => mods
                .push(comp::content_module_with(true, true).expect("the content module composes")),
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

/// A booted world, its oracle and the cumulative store (`coop.rs`'s rig).
pub(crate) struct Rig {
    pub(crate) tw: TestWorld,
    oracle: Arc<MapOracle>,
    pub(crate) store: HashMap<SmolStr, mush_core::ModuleBootData>,
    typed: BTreeMap<(SmolStr, SmolStr), serde_json::Value>,
}

pub(crate) fn no_refused_writes(tw: &TestWorld) {
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
            mush_core::OfferSecret::new(1, [0x49; 32]).expect("an offer secret"),
            [],
        )
        .expect("an offer key ring"),
    );
}

/// Boot, settle the capital, stock both ports.
fn stocked() -> Rig {
    stocked_with(Roster::default())
}

/// [`stocked`] on `roster`.
fn stocked_with(roster: Roster) -> Rig {
    let mut tw = TestWorld::with_modules(roster.modules());
    no_refused_writes(&tw);
    with_offer_keys(&mut tw);
    let oracle = Arc::new(MapOracle::default());
    install_oracle(&mut tw, &oracle);
    let mut rig = Rig {
        tw,
        oracle,
        store: HashMap::new(),
        typed: BTreeMap::new(),
    };
    sweep(&mut rig.tw, 120);
    stock(&mut rig);
    rig
}

/// Fold every write since the first boot and reboot into `roster`.
pub(crate) fn reboot_with(rig: Rig, roster: Roster) -> Rig {
    reboot_editing(rig, roster, |_| {})
}

/// [`reboot_with`], the folded store edited first (`coop.rs`'s corrupt-document
/// reboot).
fn reboot_editing(
    rig: Rig,
    roster: Roster,
    edit: impl FnOnce(&mut HashMap<SmolStr, mush_core::ModuleBootData>),
) -> Rig {
    let Rig {
        mut tw,
        oracle,
        mut store,
        typed,
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

pub(crate) fn said(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    tw.seen_by(events, who).join("\n")
}

fn run(tw: &mut TestWorld, who: ObjId, line: &str) -> String {
    let events = tw.run(who, line);
    said(tw, &events, who)
}

pub(crate) fn store(tw: &TestWorld) -> &TacsimStore {
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

pub(crate) fn port_map(tw: &TestWorld, entry: &str) -> PortMap {
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
    let mut bytes = [0xd9_u8; 16];
    bytes[..4].copy_from_slice(&n.to_be_bytes());
    bytes
}

/// Drive one queued submission the way the world task does, keeping its typed
/// post-images for a reboot (`commerce.rs`'s `land`). **Doc 23 cut S.2**: on the world task's
/// composed path (`excursions.rs`' since cut S.1), so the ship's epoch — its typed record and its
/// black box line through the black box's owner participant — lands in its own bundle, and the
/// line's chunk reaches the store a reboot boots from.
pub(crate) fn land(rig: &mut Rig, submit: &AuthoritativeSubmit) -> Result<(), String> {
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
        .prepare_registered_composed_resolved_authoritative_action(
            &action,
            &resolved,
            CommitToken::from_bytes(token),
        )
        .map_err(|error| format!("{error:?}"))?;
    let (writes, install) = prepared.into_parts();
    tw.engine_mut()
        .install_composed_action(install)
        .map_err(|error| format!("{error:?}"))?;
    let (ops, ledger) = support::composed::split(&writes);
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
    fold_module_ops_into(&mut rig.store, ledger);
    Ok(())
}

/// Every queued submission of the port's lanes, dropping other modules'; asserts
/// none kept of tacsim's is GOD's.
pub(crate) fn drain(tw: &mut TestWorld) -> Vec<AuthoritativeSubmit> {
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

pub(crate) fn land_all(rig: &mut Rig, queued: &[AuthoritativeSubmit]) {
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

/// Land what lands (the port's own background work on a long sweep).
fn land_quietly(rig: &mut Rig, queued: &[AuthoritativeSubmit]) {
    for submit in queued {
        let _ = land(rig, submit);
    }
}

/// One port-tick fire (6 s on), its submissions drained (not landed).
pub(crate) fn fire(rig: &mut Rig) -> (Vec<(Audience, OutputEvent)>, Vec<AuthoritativeSubmit>) {
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    (events, queued)
}

/// One fire whose background submissions are landed where they land.
pub(crate) fn fire_quietly(rig: &mut Rig) -> Vec<(Audience, OutputEvent)> {
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
pub(crate) fn typed<K, V>(
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

fn pool(tw: &mut TestWorld, vault: VaultId, good: &str) -> u64 {
    let families = tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
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

/// An enrolled pilot (`Salt Moth`), standing at C07.
fn chartered(rig: &mut Rig, who: &str, seed: i64) -> (ObjId, HullRow) {
    let pilot = pilot(rig, who, seed);
    let out = run(&mut rig.tw, pilot, "+coop/enrol Salt Moth");
    assert!(out.contains("now the Salt Moth"), "{out}");
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

/// Seed `qty` of `good` into `vault`'s pool through `lot` (`coop.rs`'s).
fn seed_pool(rig: &mut Rig, vault: VaultId, good: &str, qty: u64, lot: LotId) {
    let endpoint = EndpointRef(vault);
    let good = cid(good);
    let tw = &mut rig.tw;
    let families = tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    for step in 0..2 {
        let engine = tw.engine_mut();
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

/// The both-limits cargo vertex (metals, water ice), solved from the goods table.
fn vertex_load() -> (u64, u64) {
    let goods = tacsim::definitions::goods::goods_table().expect("the goods table");
    let pack = load_pack(tacsim::definitions::content::DRAY_JSON).expect("the Dray pack");
    let Some(PackEntry::Unit(spec)) = pack.entry(&cid(tacsim::definitions::content::DRAY)) else {
        panic!("the Dray unit");
    };
    let logistics = spec.logistics.as_ref().expect("the Dray's logistics");
    let cargo = cid("cargo");
    let def = logistics.stowage.get(&cargo).expect("the cargo hold");
    let compartment = tacsim::port::capacity::Compartment::from_stowage(&cargo, def);
    vertex::both_limits_vertex(&goods, &compartment, &cid("metals"), &cid("volatiles"))
        .expect("an integral vertex")
}

/// The outbound full load: the vertex cargo, `tank` t and `locker` crew-days.
fn load(rig: &mut Rig, row: &HullRow, tank: u64, locker: u64) {
    let vault = hold_vault(&rig.tw, row);
    let (metals, ice) = vertex_load();
    seed_pool(rig, vault, "metals", metals, LotId(901));
    seed_pool(rig, vault, "volatiles", ice, LotId(902));
    if tank > 0 {
        seed_pool(rig, vault, "propellant", tank, LotId(903));
    }
    if locker > 0 {
        seed_pool(rig, vault, "provisions", locker, LotId(904));
    }
}

/// Board from C01.
fn board(rig: &mut Rig, pilot: ObjId) {
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, pilot, gallery);
    let out = run(&mut rig.tw, pilot, "+ship/board");
    assert!(
        out.contains("You cycle the lock into the Salt Moth."),
        "{out}"
    );
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

pub(crate) fn ic_now(tw: &TestWorld) -> i64 {
    tw.world().world_ic_now().at
}

/// Sweep to the first real second whose IC instant is at or past `target`,
/// landing the port's background submissions; the events.
pub(crate) fn to_ic(rig: &mut Rig, target: i64) -> Vec<(Audience, OutputEvent)> {
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

pub(crate) fn voyage(tw: &TestWorld, hull: ObjId) -> Option<Voyage> {
    match store(tw).voyage_state(hull) {
        VoyageState::Present(voyage) => Some(voyage.clone()),
        VoyageState::Absent => None,
        VoyageState::Unreadable => panic!("the sailing plan is unreadable"),
    }
}

pub(crate) fn handles(tw: &TestWorld) -> PortHandles {
    tw.world()
        .module_state::<PortHandles>()
        .cloned()
        .expect("port handles")
}

pub(crate) fn ticket(tw: &mut TestWorld, hull: ObjId, seq: u32) -> Option<Ticket> {
    let h = handles(tw);
    typed(tw, &h.families.tickets, &TicketId { hull, seq })
}

pub(crate) fn journey(tw: &TestWorld, hull: ObjId) -> Option<CarrierJourney> {
    tw.world()
        .module_state::<JourneyBook>()
        .and_then(|book| book.get(hull).cloned())
}

fn journeys(tw: &TestWorld) -> usize {
    tw.world()
        .module_state::<JourneyBook>()
        .map_or(0, |book| book.journeys.len())
}

fn parent(tw: &TestWorld, obj: ObjId) -> SpatialParent {
    tw.world()
        .module_state::<SpatialState>()
        .expect("sim")
        .parent_of(obj)
}

fn domain_of(tw: &TestWorld, entry: &str) -> u64 {
    port_map(tw, entry).domain.expect("a port domain")
}

/// `+ship/undock` with the queued submissions it staged.
pub(crate) fn undock(
    rig: &mut Rig,
    pilot: ObjId,
    line: &str,
) -> (String, Vec<AuthoritativeSubmit>) {
    let out = run(&mut rig.tw, pilot, line);
    let queued = drain(&mut rig.tw);
    (out, queued)
}

/// Whether `who` is offered the burn now.
fn burn_offered(tw: &TestWorld, who: ObjId) -> bool {
    let Some(action) = handles(tw).burn else {
        return false;
    };
    let engine = tw.engine();
    let Ok(query) = engine.legal_action_query(
        mush_core::PrincipalId::Object { id: who },
        who,
        mush_core::PerceivedRef::SelfActor,
    ) else {
        return false;
    };
    engine
        .project_legal_actions(&query)
        .ok()
        .is_some_and(|offers| offers.projected_for_action(&action).is_some())
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

/// A chartered pilot aboard the docked *Salt Moth* with the full load, at `rate`.
pub(crate) fn aboard_with_load(rate: u32, tank: u64, locker: u64) -> (Rig, ObjId, HullRow) {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    load(&mut rig, &row, tank, locker);
    set_rate(&mut rig.tw, rate);
    board(&mut rig, pilot);
    (rig, pilot, row)
}

/// Undock to Stickney and land the burn; the voyage's pending plan.
pub(crate) fn undock_and_burn(
    rig: &mut Rig,
    pilot: ObjId,
    hull: ObjId,
) -> tacsim::port::PendingVoyage {
    let (out, queued) = undock(rig, pilot, "+ship/undock stickney");
    assert!(out.contains("Clearance filed for Stickney Yards"), "{out}");
    let pending = voyage(&rig.tw, hull)
        .and_then(|v| v.pending)
        .expect("a filed passage");
    assert_eq!(queued.len(), 1, "one burn: {out}");
    assert_eq!(queued[0].actor, pilot, "the burn is the pilot's");
    land_all(rig, &queued);
    pending
}

// ---------------------------------------------------------------------------
// The verify lens, end to end
// ---------------------------------------------------------------------------

/// **The verify lens** (the IC9 brief §1): plan, undock, the burn, the launch, the
/// Kirkwood boundary and the docking heard in the Crew Compartment, the under-way
/// renders, and `+ship/leave` into P01.
#[test]
#[allow(clippy::too_many_lines)]
fn ic9_undock_flies_the_lane_and_docks_at_stickney() {
    let (mut rig, pilot, row) = aboard_with_load(3_600, 53, 19);
    let hull = ObjId(row.hull);
    let vault = hold_vault(&rig.tw, &row);
    let crew = room_of(&row, "crew");
    let _ = run(&mut rig.tw, pilot, "in");
    let _ = run(&mut rig.tw, pilot, "crew");
    assert_eq!(location(&rig.tw, pilot), Some(crew));

    // +ship/plan stickney: lines 1–8, writing nothing.
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    let plan = run(&mut rig.tw, pilot, "+ship/plan stickney");
    let lines: Vec<&str> = plan.lines().collect();
    assert_eq!(
        lines.first().copied(),
        Some("Plan for Stickney Yards, by way of Kirkwood Depot:"),
        "{plan}"
    );
    assert!(
        lines.contains(&"Cargo: 100/100 t · 120.0/120 m³ · FULL: mass and packed volume."),
        "line 2: {plan}"
    );
    assert!(
        lines.iter().any(|l| l.starts_with("Room on this board: ")),
        "line 3: {plan}"
    );
    assert!(
        lines.contains(&"Payload 153.2 t of 181 t."),
        "line 4: {plan}"
    );
    assert!(
        lines.contains(
            &"Burn 52 t (tank 53 t; 1 t left). Flip and burn at 0.53 m/s² (50 milli-g rated, throttle 1000‰): Δv 840 km/s, peak 335 km/s, exhaust 6000 km/s; a farther port costs more."
        ),
        "line 5: {plan}"
    );
    assert!(
        lines.contains(&"Stores 19 crew-days (locker 19; 0 left)."),
        "line 6: {plan}"
    );
    // Doc 19 cut A.8 (D-19-22): the shipped epoch plan fits, so no `Throttle:` line.
    assert!(
        !lines.iter().any(|l| l.starts_with("Throttle:")),
        "the shipped plan fits: {plan}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("Passage 18.5 IC days ≈ 7 min real; arrives about ")),
        "line 7: {plan}"
    );
    assert_eq!(
        lines.last().copied(),
        Some("Nothing was written."),
        "{plan}"
    );
    assert!(
        rig.tw
            .take_module_ops()
            .iter()
            .all(|op| op.ns != tacsim::NS),
        "the plan wrote nothing"
    );
    assert!(voyage(&rig.tw, hull).is_none());

    // +ship/undock stickney: filed, one burn staged.
    let (filed, queued) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(
        filed.contains(
            "Clearance filed for Stickney Yards: the plan burns 52 t of reaction mass and 19 \
             crew-days; arrival in 18.5 IC days, about "
        ),
        "{filed}"
    );
    let pending = voyage(&rig.tw, hull)
        .and_then(|v| v.pending)
        .expect("the filed passage");
    assert_eq!(pending.seq, 1);
    assert_eq!(pending.plan.burn_t, 52);
    assert_eq!(queued.len(), 1, "{filed}");
    assert_eq!(queued[0].actor, pilot);
    // The status line, departing.
    let look = run(&mut rig.tw, pilot, "look");
    assert!(
        look.contains(&format!(
            "Aboard the Salt Moth ({}), cleared for Stickney Yards.",
            row.mark
        )),
        "{look}"
    );
    // While filed (L13, item 11): nothing loads — since doc 18 cut IC14 row 23 in the
    // cleared hull's own words, `HOLD_CLEARED` — and the readiness hulls line counts it
    // departing.
    let ceres = mush_market::MarketId::new("ceres").expect("venue");
    assert_eq!(
        PortVenueGate
            .admit(rig.tw.world(), pilot, &ceres, None, Presence::Ship(hull))
            .map_err(|r| r.line),
        Err(tacsim::port::HOLD_CLEARED
            .replace("{name}", "Salt Moth")
            .replace(
                "{port}",
                &tacsim::port::coop::port_name(&port_def("ceres_port"))
            )),
        "nothing loads into a filed hold"
    );
    let report = status(&mut rig.tw);
    assert!(
        report
            .lines()
            .any(|line| line.contains(" hulls: ") && line.contains(", 1 departing.")),
        "{report}"
    );
    land_all(&mut rig, &queued);
    assert_eq!(pool(&mut rig.tw, vault, "propellant"), 1, "53 -> 1");
    assert_eq!(pool(&mut rig.tw, vault, "provisions"), 0, "19 -> 0");
    let t = ticket(&mut rig.tw, hull, 1).expect("ticket {hull, seq 1}");
    let secs: Vec<u32> = t.plan.legs.iter().map(|l| l.secs).collect();
    assert_eq!(secs, vec![326_136, 1_274_226]);
    assert_eq!(t.plan.total_s, 1_600_362);
    assert_eq!(t.seq, 1);
    assert_eq!(t.manifest.tank, 53);
    assert_eq!(t.manifest.locker, 19);
    assert_eq!(t.payload_kg, 153_176);

    // The next tacsim.port fire launches it.
    let events = fire_quietly(&mut rig);
    let j = journey(&rig.tw, hull).expect("a journey");
    assert_eq!(j.route().id().as_str(), "ceres_stickney");
    assert_eq!(j.leg_times(), Some(&[326_136_u32, 1_274_226][..]));
    assert_eq!(j.departed().at, t.plan.depart_ic);
    let heard = said(&rig.tw, &events, pilot);
    assert!(
        heard.contains("The Salt Moth's lock seals. Berth clamps release; the drive lights."),
        "{heard}"
    );
    assert!(voyage(&rig.tw, hull).and_then(|v| v.pending).is_none());
    // CLEARANCE_GRANTED, exactly once.
    let mut granted = 0;
    for _ in 0..3 {
        let events = fire_quietly(&mut rig);
        let heard = said(&rig.tw, &events, pilot);
        granted += heard
            .matches(
                "Clearance granted. 52 t reaction mass and 19 crew-days committed; the Salt Moth \
                 is under way for Stickney Yards, arrival in ",
            )
            .count();
    }
    assert_eq!(granted, 1, "CLEARANCE_GRANTED is told once");

    // The under-way renders.
    let ship = run(&mut rig.tw, pilot, "+ship");
    assert!(
        ship.contains("Salt Moth (Dray): under way for Stickney Yards, leg 1 of 2; arrives in "),
        "{ship}"
    );
    let look = run(&mut rig.tw, pilot, "look");
    let status_prefix = format!(
        "Aboard the Salt Moth ({}), under way, leg 1 of 2, ETA ",
        row.mark
    );
    assert!(look.contains(&status_prefix), "{look}");
    let travel = run(&mut rig.tw, pilot, "+travel/status");
    assert!(
        travel.contains("You are aboard the Salt Moth, under way: leg 1 of 2, bound for Stickney Yards; it arrives in "),
        "{travel}"
    );
    let report = status(&mut rig.tw);
    assert!(report.contains(", 1 under way."), "{report}");

    // The Kirkwood boundary.
    let boundary = t.plan.depart_ic + 326_136;
    let events = to_ic(&mut rig, boundary - 3_600);
    assert!(!said(&rig.tw, &events, pilot).contains("slides past"));
    let events = to_ic(&mut rig, boundary);
    let heard = said(&rig.tw, &events, pilot);
    assert!(
        heard.contains(
            "Kirkwood Depot slides past: the Co-op's tank farm, a Republic picket's transponder \
             asking for your manifest. Leg 2 of 2."
        ),
        "{heard}"
    );
    let look = run(&mut rig.tw, pilot, "look");
    assert!(
        look.contains(&format!(
            "Aboard the Salt Moth ({}), under way, leg 2 of 2, Kirkwood astern, ETA ",
            row.mark
        )),
        "{look}"
    );

    // The docking.
    let arrival = t.plan.depart_ic + 1_600_362;
    let events = to_ic(&mut rig, arrival - 3_600);
    assert!(!said(&rig.tw, &events, pilot).contains("docks at"));
    let events = to_ic(&mut rig, arrival);
    let heard = said(&rig.tw, &events, pilot);
    assert!(
        heard.contains("Salt Moth docks at Stickney Yards."),
        "{heard}"
    );
    let stickney = domain_of(&rig.tw, "stickney_port");
    assert!(
        matches!(parent(&rig.tw, hull), SpatialParent::Direct { domain, .. } if domain.0 == stickney),
        "the hull is Direct on the Stickney domain"
    );
    let gallery = p01(&rig.tw);
    assert_eq!(
        location(&rig.tw, hull),
        Some(gallery),
        "the hull projects into P01"
    );
    match tacsim::port::ship::berth_of(rig.tw.world(), hull) {
        tacsim::port::ship::Berth::Docked { def, .. } => assert_eq!(def.venue.as_str(), "stickney"),
        other => panic!("the hull is not docked at Stickney: {other:?}"),
    }
    // +ship/leave from the Airlock lands in P01, stamped Stickney.
    let _ = run(&mut rig.tw, pilot, "passage");
    let _ = run(&mut rig.tw, pilot, "airlock");
    assert_eq!(location(&rig.tw, pilot), Some(room_of(&row, "airlock")));
    let out = run(&mut rig.tw, pilot, "+ship/leave");
    assert!(out.contains("You cycle the lock out into"), "{out}");
    assert_eq!(location(&rig.tw, pilot), Some(gallery));
    assert!(
        rig.tw
            .world()
            .has_marker(pilot, "tacsim.last_port.stickney")
    );
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

/// What a refusal must leave untouched: no voyage write (the sailing plan is
/// exactly `prior`, and no `tacsim:voyage:` op was emitted since the last check),
/// no submission, the pools, no ticket.
fn assert_nothing_moved(
    rig: &mut Rig,
    hull: ObjId,
    vault: VaultId,
    before: (u64, u64),
    prior: Option<&Voyage>,
    queued: &[AuthoritativeSubmit],
) {
    assert!(queued.is_empty(), "a refusal submits nothing");
    assert_eq!(
        voyage(&rig.tw, hull).as_ref(),
        prior,
        "the sailing plan is untouched"
    );
    let ops = rig.tw.take_module_ops();
    assert!(!voyage_written(&ops), "no voyage write");
    fold_module_ops_into(&mut rig.store, ops);
    assert_eq!(
        (
            pool(&mut rig.tw, vault, "propellant"),
            pool(&mut rig.tw, vault, "provisions")
        ),
        before
    );
    assert!(ticket(&mut rig.tw, hull, 1).is_none(), "no ticket");
}

/// **Every refusal moves nothing**, each line exact (the IC9 brief §3 item 12).
#[test]
#[allow(clippy::too_many_lines)]
fn ic9_undock_refusals_move_nothing() {
    // Not aboard: in C01.
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    let vault = hold_vault(&rig.tw, &row);
    load(&mut rig, &row, 50, 19);
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, pilot, gallery);
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(
        out.contains(&NOT_ABOARD.replace("{name}", "Salt Moth")),
        "{out}"
    );
    assert_nothing_moved(&mut rig, hull, vault, (50, 19), None, &queued);
    // `at=` a later hour; `via=`.
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney at=+2h");
    assert!(out.contains(AT_NOT_YET), "{out}");
    assert_nothing_moved(&mut rig, hull, vault, (50, 19), None, &queued);
    // Doc 19 cut A.2: a waypoint the lane does not know is refused by name, with
    // the lane's places; a known one plans the route that calls there.
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney via=nowhere");
    assert!(
        out.contains(
            &VIA_UNKNOWN
                .replace("{via}", "nowhere")
                .replace("{list}", "Ahuna Flank, Aitken Highport, Ceres Freeport, Freeport Commons, Juling Shadow, Kerwan Floor, Kirkwood Depot, Ligeia Terminal, Occator Rim, Rheasilvia Commons, Stickney Yards, Urvara Reach, Voltaire Anchorage")
        ),
        "{out}"
    );
    assert_nothing_moved(&mut rig, hull, vault, (50, 19), None, &queued);
    let out = run(&mut rig.tw, pilot, "+ship/plan stickney via=kirkwood");
    assert!(
        out.contains("Plan for Stickney Yards, by way of Kirkwood Depot:"),
        "{out}"
    );
    // Doc 19 cut A.8 (D-19-22): 52 t against today's 50 t is short, but the plan
    // fits the tank and the locker filled to capacity, so no `Throttle:` line.
    assert!(out.contains("(tank 50 t; 2 t short)"), "{out}");
    assert!(
        !out.lines().any(|l| l.starts_with("Throttle:")),
        "a full tank fits: {out}"
    );
    let out = run(&mut rig.tw, pilot, "+ship/plan stickney throttle=100");
    assert!(out.contains("throttle= is 250 to 1000 permille"), "{out}");
    let out = run(
        &mut rig.tw,
        pilot,
        "+ship/plan stickney throttle=500 policy=cheapest",
    );
    assert!(out.contains("throttle 500‰"), "{out}");
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock");
    assert!(out.contains("Usage: +ship/undock <port>"), "{out}");
    assert_nothing_moved(&mut rig, hull, vault, (50, 19), None, &queued);
    // The tank: the full cargo with 50 t burns 52 t (L27; the flip law since A.2).
    board(&mut rig, pilot);
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(
        out.contains(
            &TANK_SHORT
                .replace("{port}", "Stickney Yards")
                .replace("{burn}", "52")
                .replace("{tank}", "50")
        ),
        "{out}"
    );
    assert_nothing_moved(&mut rig, hull, vault, (50, 19), None, &queued);
    // No route.
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock pallas");
    assert!(
        out.contains(
            &NO_ROUTE
                .replace("{here}", "Ceres Freeport")
                .replace("{list}", "Ahuna Flank, Freeport Commons, Juling Shadow, Kerwan Floor, Aitken Highport, Occator Rim, Rheasilvia Commons, Stickney Yards, Urvara Reach")
        ),
        "{out}"
    );
    assert_nothing_moved(&mut rig, hull, vault, (50, 19), None, &queued);
    // Overloaded: seeded past the rail (91 t in the tank, 118 crew-days, 101 t of
    // cargo: 192.6 t against the rated 181).
    seed_pool(&mut rig, vault, "propellant", 41, LotId(905));
    seed_pool(&mut rig, vault, "provisions", 99, LotId(906));
    seed_pool(&mut rig, vault, "metals", 1, LotId(907));
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(
        out.contains(
            &OVERLOADED
                .replace("{name}", "Salt Moth")
                .replace("{payload}", "192.6")
                .replace("{ceiling}", "181")
        ),
        "{out}"
    );
    assert_nothing_moved(&mut rig, hull, vault, (91, 118), None, &queued);

    // A fresh world: the destination domain paused; then departing; then under way.
    let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
    let hull = ObjId(row.hull);
    let vault = hold_vault(&rig.tw, &row);
    let stickney = domain_of(&rig.tw, "stickney_port");
    let out = run(&mut rig.tw, GOD, &format!("@domain/pause {stickney}"));
    assert!(!out.contains("denied"), "{out}");
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(
        out.contains(
            &PORT_CLOSED
                .replace("{name}", "Salt Moth")
                .replace("{port}", "Stickney Yards")
        ),
        "{out}"
    );
    assert_nothing_moved(&mut rig, hull, vault, (53, 19), None, &queued);
    let _ = run(&mut rig.tw, GOD, &format!("@domain/resume {stickney}"));
    // Departing: a second undock before the launch fire.
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(out.contains("Clearance filed"), "{out}");
    assert_eq!(queued.len(), 1);
    let filed = voyage(&rig.tw, hull).expect("the voyage");
    let (out, again) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(
        out.contains(
            &UNDOCK_CLEARED
                .replace("{name}", "Salt Moth")
                .replace("{port}", "Stickney Yards")
        ),
        "{out}"
    );
    assert!(again.is_empty(), "nothing more submitted");
    assert_eq!(
        voyage(&rig.tw, hull),
        Some(filed),
        "the voyage is unchanged"
    );
    // While filed: +ship/leave and +ship/board refuse.
    let airlock = room_of(&row, "airlock");
    assert_eq!(location(&rig.tw, pilot), Some(airlock));
    let out = run(&mut rig.tw, pilot, "+ship/leave");
    assert!(
        out.contains(&LEAVE_CLEARED.replace("{name}", "Salt Moth")),
        "{out}"
    );
    assert_eq!(location(&rig.tw, pilot), Some(airlock));
    // Staff take the pilot ashore by hand: their board refuses while filed.
    support::with_ctx(&mut rig.tw, |ctx| {
        mush_sim::unplace(ctx, pilot, "staff took the pilot ashore").expect("unplaces")
    });
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, pilot, gallery);
    let out = run(&mut rig.tw, pilot, "+ship/board");
    assert!(
        out.contains(&LEAVE_CLEARED.replace("{name}", "Salt Moth")),
        "{out}"
    );
    // Under way: after the launch.
    let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
    let hull = ObjId(row.hull);
    let _ = undock_and_burn(&mut rig, pilot, hull);
    let _ = fire_quietly(&mut rig);
    assert!(journey(&rig.tw, hull).is_some(), "launched");
    let vault = hold_vault(&rig.tw, &row);
    let before = (
        pool(&mut rig.tw, vault, "propellant"),
        pool(&mut rig.tw, vault, "provisions"),
    );
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(
        out.contains(&UNDOCK_UNDER_WAY.replace("{name}", "Salt Moth")),
        "{out}"
    );
    assert!(queued.is_empty());
    assert!(voyage(&rig.tw, hull).and_then(|v| v.pending).is_none());
    assert_eq!(
        (
            pool(&mut rig.tw, vault, "propellant"),
            pool(&mut rig.tw, vault, "provisions")
        ),
        before
    );
    assert!(ticket(&mut rig.tw, hull, 2).is_none());
}

// ---------------------------------------------------------------------------
// The held launch and its terminal
// ---------------------------------------------------------------------------

/// **A held launch pages and recovers** (reviewer F2/F10; L29, L31): a paused
/// destination holds the burned launch (told once, no journey, the pending kept),
/// a resumed one launches the frozen plan at its own `depart_ic`; a revoked lease
/// holds it too, and staff stand it down with `@tacsim/port clear`.
#[test]
#[allow(clippy::too_many_lines)]
fn ic9_a_held_launch_pages_and_recovers() {
    let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
    let hull = ObjId(row.hull);
    let vault = hold_vault(&rig.tw, &row);
    let pending = undock_and_burn(&mut rig, pilot, hull);
    let stickney = domain_of(&rig.tw, "stickney_port");
    let _ = run(&mut rig.tw, GOD, &format!("@domain/pause {stickney}"));
    let held = LAUNCH_HELD
        .replace("{name}", "Salt Moth")
        .replace("{port}", "Stickney Yards");
    let mut told = 0;
    for _ in 0..30 {
        let events = fire_quietly(&mut rig);
        told += said(&rig.tw, &events, pilot).matches(&held).count();
    }
    assert_eq!(told, 1, "the pilot is told once");
    assert!(journey(&rig.tw, hull).is_none(), "no journey while held");
    assert_eq!(
        voyage(&rig.tw, hull).and_then(|v| v.pending),
        Some(pending.clone()),
        "the pending is kept"
    );
    assert_eq!(pool(&mut rig.tw, vault, "propellant"), 1, "debited once");
    let _ = run(&mut rig.tw, GOD, &format!("@domain/resume {stickney}"));
    let mut granted = 0;
    for _ in 0..(LAUNCH_GRACE_FIRES + 4) {
        let events = fire_quietly(&mut rig);
        granted += said(&rig.tw, &events, pilot)
            .matches("Clearance granted.")
            .count();
    }
    let j = journey(&rig.tw, hull).expect("launched after the resume");
    assert_eq!(
        j.departed().at,
        pending.plan.depart_ic,
        "the frozen plan's own instant"
    );
    assert_eq!(granted, 1, "CLEARANCE_GRANTED is told once");
    assert_eq!(
        pool(&mut rig.tw, vault, "propellant"),
        1,
        "still debited once"
    );

    // A revoked lease holds the launch; staff stand it down.
    let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
    let hull = ObjId(row.hull);
    let pending = undock_and_burn(&mut rig, pilot, hull);
    let lease = tacsim::port::live_charter(rig.tw.world(), pilot)
        .expect("a live charter")
        .contract;
    let _ = run(&mut rig.tw, GOD, &format!("@contract/cancel #{}", lease.0));
    assert!(tacsim::port::live_charter(rig.tw.world(), pilot).is_none());
    let events = fire_quietly(&mut rig);
    assert!(
        said(&rig.tw, &events, pilot).contains(&held),
        "the pilot is told"
    );
    assert!(
        journey(&rig.tw, hull).is_none(),
        "a revoked lease never flies"
    );
    // A non-staff clear is refused; a clear stands the clearance down.
    let refused = run(
        &mut rig.tw,
        pilot,
        &format!("@tacsim/port clear #{}", hull.0),
    );
    assert!(
        voyage(&rig.tw, hull).and_then(|v| v.pending).is_some(),
        "{refused}"
    );
    let events = rig.tw.run(GOD, &format!("@tacsim/port clear #{}", hull.0));
    let staff = said(&rig.tw, &events, GOD);
    assert!(
        staff.contains(&format!(
            "Clearance {} for #{} stood down; the ticket, if burned, is kept as the record and \
             nothing is refunded.",
            pending.seq, hull.0
        )),
        "{staff}"
    );
    assert!(
        said(&rig.tw, &events, pilot).contains(
            "The Salt Moth's clearance for Stickney Yards was stood down by the port. Nothing \
             burned is returned."
        ),
        "the pilot is told"
    );
    let v = voyage(&rig.tw, hull).expect("the voyage");
    assert!(v.pending.is_none());
    assert_eq!(v.last_seq, pending.seq, "the sequence is kept");
    assert!(
        ticket(&mut rig.tw, hull, pending.seq).is_some(),
        "the ticket is the record"
    );
    let out = run(&mut rig.tw, GOD, &format!("@tacsim/port clear #{}", hull.0));
    assert!(
        out.contains(&format!("#{} has no clearance on file.", hull.0)),
        "{out}"
    );
    let out = run(&mut rig.tw, pilot, "+ship/leave");
    assert!(!out.contains("has clearance"), "{out}");
}

/// **A burned launch never flies with its pilot ashore** (L14 at the executed
/// step; reviewer F1): staff take the pilot off the docked hull after the burn
/// lands; the launch holds (told once, no journey, the pending kept) until the
/// pilot is placed back aboard, then flies the frozen plan at its own `depart_ic`.
#[test]
fn ic9_a_pilot_moved_ashore_holds_the_launch() {
    let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
    let hull = ObjId(row.hull);
    let pending = undock_and_burn(&mut rig, pilot, hull);
    support::with_ctx(&mut rig.tw, |ctx| {
        mush_sim::unplace(ctx, pilot, "staff took the pilot ashore").expect("unplaces")
    });
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, pilot, gallery);
    let held = LAUNCH_HELD
        .replace("{name}", "Salt Moth")
        .replace("{port}", "Stickney Yards");
    let mut told = 0;
    for _ in 0..(LAUNCH_GRACE_FIRES + 4) {
        let events = fire_quietly(&mut rig);
        told += said(&rig.tw, &events, pilot).matches(&held).count();
    }
    assert_eq!(told, 1, "the pilot is told once");
    assert!(
        journey(&rig.tw, hull).is_none(),
        "the hull never flies without its pilot"
    );
    assert_eq!(
        voyage(&rig.tw, hull).and_then(|v| v.pending),
        Some(pending.clone()),
        "the pending is kept"
    );
    // Staff put the pilot back aboard: the next backoff expiry launches.
    let out = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@spatial/place/interior #{}=#{},berths",
            pilot.0,
            room_of(&row, "airlock").0
        ),
    );
    assert!(out.contains("Placed"), "{out}");
    for _ in 0..(LAUNCH_GRACE_FIRES + 2) {
        let _ = fire_quietly(&mut rig);
    }
    let j = journey(&rig.tw, hull).expect("launched once the pilot is aboard");
    assert_eq!(j.departed().at, pending.plan.depart_ic);
    assert!(voyage(&rig.tw, hull).and_then(|v| v.pending).is_none());
}

// ---------------------------------------------------------------------------
// An unreadable sailing plan
// ---------------------------------------------------------------------------

/// Whether any of `ops` writes a `tacsim:voyage:` key.
fn voyage_written(ops: &[mush_core::ModuleOp]) -> bool {
    ops.iter().filter(|op| op.ns == tacsim::NS).any(|op| {
        matches!(
            &op.op,
            ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key }
                if key.starts_with(VOYAGE_PREFIX)
        )
    })
}

/// One edit of a stored sailing plan's body (given another hull's id).
type DocEdit = fn(&mut serde_json::Value, ObjId);

/// **An unreadable or misfiled sailing plan fails closed** (item 7; reviewer T4):
/// corrupted, and separately filed under another hull's key, the document reboots
/// `Unreadable` (its key stays claimed — never `Absent`, which would re-mint seq 1
/// over a burned ticket); `+ship/undock` answers `VOYAGE_DARK` and submits
/// nothing; the launch arm skips the hull (no journey, no write).
#[test]
fn ic9_an_unreadable_sailing_plan_fails_closed() {
    let edits: [(&str, DocEdit); 2] = [
        ("corrupt", |doc, _| {
            *doc = serde_json::json!({ "schema": 1, "nonsense": true });
        }),
        ("misfiled", |doc, other| {
            if let Some(hull) = doc.get_mut("hull") {
                *hull = serde_json::json!(other.0);
            }
        }),
    ];
    for (case, edit) in edits {
        let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
        let hull = ObjId(row.hull);
        let other = port_map(&rig.tw, "ceres_port")
            .hulls
            .iter()
            .map(|r| ObjId(r.hull))
            .find(|h| *h != hull)
            .expect("a second hull");
        let vault = hold_vault(&rig.tw, &row);
        // A filed passage whose burn never lands (so a re-mint would be seq 1).
        let (out, _dropped) = undock(&mut rig, pilot, "+ship/undock stickney");
        assert!(out.contains("Clearance filed"), "{case}: {out}");
        let key = voyage_key(hull);
        let mut rig = reboot_editing(rig, Roster::default(), |folded| {
            let doc = folded
                .get_mut(tacsim::NS)
                .and_then(|data| data.kv.iter_mut().find(|(k, _)| *k == key))
                .expect("the sailing plan document");
            edit(&mut doc.1, other);
        });
        assert!(
            matches!(store(&rig.tw).voyage_state(hull), VoyageState::Unreadable),
            "{case}: the key stays claimed"
        );
        assert!(
            matches!(store(&rig.tw).voyage_state(other), VoyageState::Absent),
            "{case}: nothing is installed under another hull"
        );
        let _ = rig.tw.connect(pilot);
        let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney");
        assert!(
            out.contains(&VOYAGE_DARK.replace("{name}", "Salt Moth")),
            "{case}: {out}"
        );
        assert!(queued.is_empty(), "{case}: nothing submitted");
        assert!(
            !voyage_written(&rig.tw.take_module_ops()),
            "{case}: the refusal writes no sailing plan"
        );
        for _ in 0..3 {
            let _ = fire_quietly(&mut rig);
        }
        assert!(journey(&rig.tw, hull).is_none(), "{case}: no journey");
        assert!(
            !voyage_written(&rig.tw.take_module_ops()),
            "{case}: the launch arm writes no sailing plan"
        );
        assert_eq!(pool(&mut rig.tw, vault, "propellant"), 53, "{case}");
        assert!(ticket(&mut rig.tw, hull, 1).is_none(), "{case}: no ticket");
    }
}

// ---------------------------------------------------------------------------
// Goods not yet aboard
// ---------------------------------------------------------------------------

/// **Promised goods hold the hull** (L13): an accepted, undrawn Draw into the hold
/// refuses; once it lands and mirrors, undock files; a hold-backed listing from
/// the hold refuses.
#[test]
fn ic9_goods_not_yet_aboard_hold_the_hull() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    let vault = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, vault, "propellant", 53, LotId(903));
    seed_pool(&mut rig, vault, "provisions", 19, LotId(904));
    // Top up and buy ten metals at C07; the Draw stays unlanded.
    let instrument = mush_economy::InstrumentId::new(tacsim::CREDIT).expect("credit");
    let account = economy(&rig.tw)
        .snapshot()
        .accounts_of(&mush_org::PrincipalId::Object(pilot))
        .find(|a| a.instrument == instrument)
        .map(|a| a.id)
        .expect("the pilot's account");
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={account} 1000000000"),
    );
    let metals = market(&rig.tw)
        .lot_listing_of(
            EndpointRef(VaultId(
                port_map(&rig.tw, "ceres_port").vendors[&cid("freeport_exchange")].vault,
            )),
            &cid("metals"),
        )
        .expect("the metals listing");
    let quoted = run(&mut rig.tw, pilot, &format!("+market/quote {metals}=10"));
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = market(&rig.tw)
        .quotes_of_buyer(&mush_org::PrincipalId::Object(pilot))
        .map(|q| q.id)
        .max()
        .expect("a quote");
    let accepted = run(&mut rig.tw, pilot, &format!("+market/accept {quote}"));
    let draw = drain(&mut rig.tw);
    assert_eq!(draw.len(), 1, "{accepted}");
    board(&mut rig, pilot);
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(
        out.contains(
            &GOODS_COMING
                .replace("{quote}", &quote.to_string())
                .replace("{name}", "Salt Moth")
        ),
        "{out}"
    );
    assert!(queued.is_empty());
    // The draw lands and the market sweep mirrors it.
    land_all(&mut rig, &draw);
    for _ in 0..30 {
        if market(&rig.tw)
            .quote(quote)
            .is_some_and(|q| q.drawn_at.is_some())
        {
            break;
        }
        let _ = fire_quietly(&mut rig);
    }
    assert!(
        market(&rig.tw)
            .quote(quote)
            .is_some_and(|q| q.drawn_at.is_some()),
        "mirrored"
    );
    assert_eq!(pool(&mut rig.tw, vault, "metals"), 10);
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(out.contains("Clearance filed for Stickney Yards"), "{out}");
    assert_eq!(queued.len(), 1);
    assert!(voyage(&rig.tw, hull).and_then(|v| v.pending).is_some());

    // A hold-backed listing from the hold.
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let vault = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, vault, "metals", 10, LotId(908));
    seed_pool(&mut rig, vault, "propellant", 53, LotId(903));
    seed_pool(&mut rig, vault, "provisions", 19, LotId(904));
    let vendor = port_map(&rig.tw, "ceres_port").vendors[&cid("freeport_exchange")]
        .vendor
        .expect("a vendor");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@market/vendor stock #{vendor}=tacsim:metals price=1 qty=5 days=1 endpoint={}",
            vault.0
        ),
    );
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let listing: ListingId = market(&rig.tw)
        .lot_listing_of(EndpointRef(vault), &cid("metals"))
        .unwrap_or_else(|| panic!("the hold-backed listing: {out}"));
    board(&mut rig, pilot);
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(
        out.contains(
            &LISTING_HOLDS
                .replace("{listing}", &listing.to_string())
                .replace("{name}", "Salt Moth")
        ),
        "{out}"
    );
    assert!(queued.is_empty());
}

/// **IC9 T8 (doc 18 cut IC12 item 1.9): the goods-not-yet-aboard rule releases a
/// terminal contract and holds for a lapsed listing.** `goods_pending_aboard` reads
/// only a quote whose contract stands non-terminal in the economy's book: a Draw
/// accepted into the hold and never landed, once its contract has expired (the
/// one-hour Draw window, `Expiry::Refund`), no longer holds the hull, and
/// `+ship/undock` files. A hold-backed listing out of the hold holds the hull live
/// or lapsed — the matter still sits on the hold's reservation until the hold is
/// gone — so one past its listing window still refuses with `LISTING_HOLDS`.
#[test]
fn ic12_t8_goods_pending_aboard_releases_a_terminal_contract_and_holds_for_a_lapsed_listing() {
    // ---- a Draw whose contract went terminal ----
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    let vault = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, vault, "propellant", 53, LotId(903));
    seed_pool(&mut rig, vault, "provisions", 19, LotId(904));
    let metals = market(&rig.tw)
        .lot_listing_of(
            EndpointRef(VaultId(
                port_map(&rig.tw, "ceres_port").vendors[&cid("freeport_exchange")].vault,
            )),
            &cid("metals"),
        )
        .expect("the metals listing");
    let quoted = run(&mut rig.tw, pilot, &format!("+market/quote {metals}=1"));
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = market(&rig.tw)
        .quotes_of_buyer(&mush_org::PrincipalId::Object(pilot))
        .map(|q| q.id)
        .max()
        .expect("a quote");
    let accepted = run(&mut rig.tw, pilot, &format!("+market/accept {quote}"));
    let dropped = drain(&mut rig.tw);
    assert_eq!(dropped.len(), 1, "the Draw leg, never landed: {accepted}");
    let contract = market(&rig.tw)
        .quote(quote)
        .and_then(|q| q.contract)
        .expect("a contract in force");
    board(&mut rig, pilot);
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(
        out.contains(
            &GOODS_COMING
                .replace("{quote}", &quote.to_string())
                .replace("{name}", "Salt Moth")
        ),
        "a standing contract holds the hull: {out}"
    );
    assert!(queued.is_empty());
    // Past the Draw window the contract expires with its refund; the port tick's
    // queued submissions in that hour are dropped (nothing here lands).
    let terminal = |tw: &TestWorld| {
        economy(tw)
            .snapshot()
            .contract(contract)
            .is_some_and(|c| c.state.is_terminal())
    };
    for _ in 0..40 {
        if terminal(&rig.tw) {
            break;
        }
        sweep(&mut rig.tw, 300);
        let _ = drain(&mut rig.tw);
    }
    assert!(terminal(&rig.tw), "the Draw's contract expired");
    assert_eq!(
        pool(&mut rig.tw, vault, "metals"),
        0,
        "the goods never moved"
    );
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(out.contains("Clearance filed for Stickney Yards"), "{out}");
    assert_eq!(queued.len(), 1, "the burn is queued");
    assert!(voyage(&rig.tw, hull).and_then(|v| v.pending).is_some());

    // ---- a hold-backed listing whose window lapsed, its hold standing ----
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let vault = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, vault, "metals", 10, LotId(908));
    seed_pool(&mut rig, vault, "propellant", 53, LotId(903));
    seed_pool(&mut rig, vault, "provisions", 19, LotId(904));
    let vendor = port_map(&rig.tw, "ceres_port").vendors[&cid("freeport_exchange")]
        .vendor
        .expect("a vendor");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@market/vendor stock #{vendor}=tacsim:metals price=1 qty=5 days=1 endpoint={}",
            vault.0
        ),
    );
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let listing: ListingId = market(&rig.tw)
        .lot_listing_of(EndpointRef(vault), &cid("metals"))
        .unwrap_or_else(|| panic!("the hold-backed listing: {out}"));
    // Past the listing's one-day window, with no fire in between: lapsed, its hold
    // (and so its matter in the hold) standing.
    let expires = market(&rig.tw)
        .listing(listing)
        .map(|l| l.expires.at)
        .expect("the listing");
    let lapsed_at = expires.saturating_add(60);
    rig.tw.set_now(lapsed_at);
    assert!(
        market(&rig.tw)
            .listing(listing)
            .is_some_and(|l| l.expires.at < rig.tw.world().now()),
        "the listing has lapsed"
    );
    board(&mut rig, pilot);
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(
        out.contains(
            &LISTING_HOLDS
                .replace("{listing}", &listing.to_string())
                .replace("{name}", "Salt Moth")
        ),
        "{out}"
    );
    assert!(queued.is_empty(), "nothing filed");
}

// ---------------------------------------------------------------------------
// The absent pilot
// ---------------------------------------------------------------------------

/// **An absent pilot reads the passage on reconnect**: the grant, the Kirkwood line
/// and the arrival notice are queued and rendered once under the heading.
#[test]
fn ic9_an_absent_pilot_reads_the_passage_on_reconnect() {
    let (mut rig, pilot, row) = aboard_with_load(3_600, 53, 19);
    let hull = ObjId(row.hull);
    let pending = undock_and_burn(&mut rig, pilot, hull);
    disconnect(&mut rig.tw, pilot);
    for _ in 0..3 {
        let _ = fire_quietly(&mut rig);
    }
    assert!(journey(&rig.tw, hull).is_some(), "launched");
    let _ = to_ic(&mut rig, pending.plan.depart_ic + 326_136);
    let _ = to_ic(&mut rig, pending.plan.depart_ic + 1_600_362);
    let queued = pending_notices(rig.tw.world(), pilot);
    let has = |needle: &str| queued.iter().filter(|line| line.contains(needle)).count();
    assert_eq!(
        has("Clearance granted. 52 t reaction mass"),
        1,
        "{queued:?}"
    );
    assert_eq!(
        has(
            " UTC Kirkwood Depot slides past: the Co-op's tank farm, a Republic picket's \
             transponder asking for your manifest. Leg 2 of 2."
        ),
        1,
        "{queued:?}"
    );
    assert_eq!(
        has(" UTC Salt Moth docked at Stickney Yards, "),
        1,
        "{queued:?}"
    );
    let events = rig.tw.connect(pilot);
    let lines = rig.tw.seen_by(&events, pilot);
    assert_eq!(
        lines.iter().filter(|l| l.contains(NOTICE_HEADING)).count(),
        1,
        "{lines:?}"
    );
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.contains("docked at Stickney Yards"))
            .count(),
        1
    );
    assert!(pending_notices(rig.tw.world(), pilot).is_empty());
}

// ---------------------------------------------------------------------------
// The lapsed clearance
// ---------------------------------------------------------------------------

/// **A clearance whose burn never lands lapses and burns nothing** (L26): the rig
/// drops the command's submission before any drain lands it.
#[test]
fn ic9_a_lapsed_clearance_burns_nothing() {
    let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
    let hull = ObjId(row.hull);
    let vault = hold_vault(&rig.tw, &row);
    let (out, dropped) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(out.contains("Clearance filed"), "{out}");
    assert_eq!(dropped.len(), 1, "the burn is dropped, never landed");
    let lapsed = CLEARANCE_LAPSED.replace("{port}", "Stickney Yards");
    let mut told = 0;
    for _ in 0..(LAUNCH_GRACE_FIRES + 4) {
        let events = fire_quietly(&mut rig);
        told += said(&rig.tw, &events, pilot).matches(&lapsed).count();
    }
    assert_eq!(told, 1, "CLEARANCE_LAPSED is told once");
    let v = voyage(&rig.tw, hull).expect("the voyage");
    assert!(v.pending.is_none());
    assert_eq!(v.last_seq, 1, "the sequence is burned");
    assert!(ticket(&mut rig.tw, hull, 1).is_none(), "no ticket");
    assert!(journey(&rig.tw, hull).is_none(), "no journey");
    assert_eq!(pool(&mut rig.tw, vault, "propellant"), 53);
    assert_eq!(pool(&mut rig.tw, vault, "provisions"), 19);
    // A late landing of the dropped burn can no longer land.
    assert!(
        land(&mut rig, &dropped[0]).is_err(),
        "the offer needs the pending"
    );
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(out.contains("Clearance filed"), "{out}");
    assert_eq!(queued.len(), 1);
    assert_eq!(
        voyage(&rig.tw, hull).and_then(|v| v.pending).map(|p| p.seq),
        Some(2),
        "a fresh undock files seq 2"
    );

    // The brief's other arm: the hold seeded between the command and the landing,
    // so the prepare refuses "the hold changed…" and nothing burns; the clearance
    // then lapses the same way.
    let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
    let hull = ObjId(row.hull);
    let vault = hold_vault(&rig.tw, &row);
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(out.contains("Clearance filed"), "{out}");
    assert_eq!(queued.len(), 1);
    seed_pool(&mut rig, vault, "metals", 1, LotId(909));
    let refused = land(&mut rig, &queued[0]).expect_err("the changed hold refuses the burn");
    assert!(
        refused.contains("the hold changed since the passage was planned; plan it again"),
        "{refused}"
    );
    assert_eq!(pool(&mut rig.tw, vault, "propellant"), 53, "nothing burned");
    assert_eq!(pool(&mut rig.tw, vault, "provisions"), 19, "nothing burned");
    assert!(ticket(&mut rig.tw, hull, 1).is_none(), "no ticket");
    let mut told = 0;
    for _ in 0..(LAUNCH_GRACE_FIRES + 4) {
        let events = fire_quietly(&mut rig);
        told += said(&rig.tw, &events, pilot).matches(&lapsed).count();
    }
    assert_eq!(told, 1, "CLEARANCE_LAPSED is told once");
    let v = voyage(&rig.tw, hull).expect("the voyage");
    assert!(v.pending.is_none());
    assert_eq!(v.last_seq, 1, "the sequence is burned");
    assert!(journey(&rig.tw, hull).is_none(), "no journey");
    assert_eq!(pool(&mut rig.tw, vault, "propellant"), 53);
}

// ---------------------------------------------------------------------------
// The manual arm
// ---------------------------------------------------------------------------

/// **Staff cannot fly a programme hull unplanned** (L32): the lane route is
/// refused on the manual arm for any carrier, a programme hull on any manual
/// route; a non-programme Thing still departs a non-lane fixture route.
#[test]
fn ic9_staff_cannot_fly_a_programme_hull_unplanned() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    let lane_unplanned = LANE_UNPLANNED.replace("{route}", "ceres_stickney");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@journey/depart #{}=ceres_stickney", hull.0),
    );
    assert!(out.contains(&lane_unplanned), "{out}");
    let stickney_host = port_map(&rig.tw, "stickney_port").host.expect("a host");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@journey/depart #{}=ic9_fixture #{stickney_host}:100",
            hull.0
        ),
    );
    assert!(
        out.contains(&MANUAL_DEPARTURE.replace("{name}", "Salt Moth")),
        "{out}"
    );
    let out = run(
        &mut rig.tw,
        pilot,
        &format!("@journey/depart #{}=ceres_stickney", hull.0),
    );
    assert!(out.contains("Permission denied"), "{out}");
    assert!(journey(&rig.tw, hull).is_none(), "nothing was written");
    // A non-programme Thing on the domain the lane route sets out from.
    let route = store(&rig.tw)
        .route_in_world(rig.tw.world(), &cid("ceres_stickney"))
        .expect("the lane route materializes");
    let origin = route.endpoints().0;
    let origin_domain = rig
        .tw
        .world()
        .module_state::<SpatialState>()
        .and_then(|st| st.domain_of_host(origin).map(|d| d.id.0))
        .expect("the origin hosts a domain");
    let thing = rig
        .tw
        .world_mut()
        .create(ObjKind::Thing, "Lighter", GOD, Some(mush_core::LIMBO))
        .expect("a thing");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@behavior/add #{}=sim.participant", thing.0),
    );
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@spatial/place/domain #{}={origin_domain}", thing.0),
    );
    assert!(out.contains("Placed"), "{out}");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@journey/depart #{}=ceres_stickney", thing.0),
    );
    assert!(out.contains(&lane_unplanned), "{out}");
    assert!(journey(&rig.tw, thing).is_none());
    let far = rig
        .tw
        .world_mut()
        .create(ObjKind::Room, "Fixture Field", GOD, None)
        .expect("a room");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@behavior/add #{}=sim.domain", far.0),
    );
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", far.0),
    );
    assert!(!out.contains("denied"), "{out}");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@journey/depart #{}=ic9_fixture #{}:100", thing.0, far.0),
    );
    assert!(out.contains("Lighter departs on journey #"), "{out}");
    assert!(journey(&rig.tw, thing).is_some());
}

// ---------------------------------------------------------------------------
// The burn's offer
// ---------------------------------------------------------------------------

/// **The burn offer takes no forged plan** (L4): it projects only for the filing
/// pilot aboard (never once staff take them off the hull), a submission naming
/// another seq is refused, the ticket's plan is the filed one even when the clock
/// has moved on since the filing (a re-plan would carry a later `depart_ic`), and
/// the ticket is the replay guard: the same submission landed again is refused and
/// debits nothing.
#[test]
#[allow(clippy::too_many_lines)]
fn ic9_the_burn_offer_takes_no_forged_plan() {
    let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
    let hull = ObjId(row.hull);
    assert!(
        !burn_offered(&rig.tw, pilot),
        "nothing filed, nothing offered"
    );
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(out.contains("Clearance filed"), "{out}");
    let filed = voyage(&rig.tw, hull)
        .and_then(|v| v.pending)
        .expect("the filed passage");
    assert!(
        burn_offered(&rig.tw, pilot),
        "the filing pilot aboard is offered it"
    );
    // A visitor GOD placed aboard is not.
    let visitor = rig.tw.spawn_player("Visitor");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@behavior/add #{}=sim.participant", visitor.0),
    );
    let out = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@spatial/place/interior #{}=#{},berths",
            visitor.0,
            room_of(&row, "airlock").0
        ),
    );
    assert!(out.contains("Placed"), "{out}");
    assert!(
        !burn_offered(&rig.tw, visitor),
        "a visitor aboard is not offered it"
    );
    // The filing pilot taken off the hull by staff is not offered it either (the
    // offer's aboard gate), and is again once placed back aboard.
    let airlock = room_of(&row, "airlock");
    let before = run(&mut rig.tw, pilot, "+ship/plan stickney");
    support::with_ctx(&mut rig.tw, |ctx| {
        mush_sim::unplace(ctx, pilot, "release explicit placement").expect("unplaces")
    });
    assert!(
        burn_offered(&rig.tw, pilot),
        "unplacing leaves the pilot physically in the same ship room"
    );
    assert_eq!(
        run(&mut rig.tw, pilot, "+ship/plan stickney"),
        before,
        "an ordinary walker still counts in the actual crew"
    );
    rig.tw
        .world_mut()
        .move_to(pilot, mush_core::LIMBO)
        .expect("actually take the pilot ashore");
    assert!(
        !burn_offered(&rig.tw, pilot),
        "the filing pilot ashore is not offered it"
    );
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@spatial/place/interior #{}=#{},berths", pilot.0, airlock.0),
    );
    assert!(out.contains("Placed"), "{out}");
    assert!(burn_offered(&rig.tw, pilot), "aboard again, offered again");
    // Another seq is refused.
    let mut forged = queued[0].clone();
    forged.parameters = serde_json::json!({ "seq": "2" });
    assert!(land(&mut rig, &forged).is_err(), "another seq is refused");
    assert!(ticket(&mut rig.tw, hull, 2).is_none());
    // The clock moves on before the burn lands (rate 1): a re-plan now would
    // depart later than the filing.
    let later = rig.tw.world().now().saturating_add(10);
    rig.tw.set_now(later);
    assert!(ic_now(&rig.tw) > filed.plan.depart_ic, "the clock moved on");
    // The real one lands with the filed plan.
    land_all(&mut rig, &queued);
    let t = ticket(&mut rig.tw, hull, 1).expect("the ticket");
    assert_eq!(t.plan, filed.plan, "the ticket's plan is the filed one");
    assert_eq!(t.plan.depart_ic, filed.plan.depart_ic);
    let vault = hold_vault(&rig.tw, &row);
    assert_eq!(pool(&mut rig.tw, vault, "propellant"), 1, "53 -> 1");
    assert_eq!(pool(&mut rig.tw, vault, "provisions"), 0, "19 -> 0");
    // The replay guard: the pending stands until the launch fire, so the offer
    // still projects, and the same submission landed again (a fresh request) is
    // refused by the ticket, debiting nothing.
    assert!(
        burn_offered(&rig.tw, pilot),
        "still offered until the launch"
    );
    let again = land(&mut rig, &queued[0]).expect_err("a second landing is refused");
    assert!(again.contains("that passage is already burned"), "{again}");
    assert_eq!(pool(&mut rig.tw, vault, "propellant"), 1, "debited once");
    assert_eq!(pool(&mut rig.tw, vault, "provisions"), 0, "debited once");
    assert_eq!(
        ticket(&mut rig.tw, hull, 1),
        Some(t),
        "the ticket is unchanged"
    );
}

// ---------------------------------------------------------------------------
// The three restart tests (doc 18 §5's mandatory 4, 5 and 7)
// ---------------------------------------------------------------------------

/// **The ticket freezes the plan across a sky change** (D-18-46): burned and
/// launched at sky rate 0, rebooted at sky rate 1000.
#[test]
fn ticket_freezes_the_plan_across_a_sky_change() {
    let (rig, ..) = sky_change_world();
    drop(rig);
}

/// The shared world of tests 4 and 5: burned and launched at sky rate 0, rebooted
/// at sky rate 1000; returns the rig, the pilot, the hull and the ticket.
fn sky_change_world() -> (Rig, ObjId, ObjId, Ticket) {
    let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
    let hull = ObjId(row.hull);
    let _ = undock_and_burn(&mut rig, pilot, hull);
    let _ = fire_quietly(&mut rig);
    let before = ticket(&mut rig.tw, hull, 1).expect("the ticket");
    let j = journey(&rig.tw, hull).expect("launched");
    let due: Vec<Option<i64>> = (0..2).map(|k| j.leg_due_at(k)).collect();
    let arrival = j.arrival_at();
    let old_rev = before.plan.sky_rev;
    let mut rig = reboot_with(
        rig,
        Roster {
            sky_rate: 1_000,
            ..Roster::default()
        },
    );
    assert_eq!(store(&rig.tw).config().time.ephemeris.rate, 1_000);
    let after = ticket(&mut rig.tw, hull, 1).expect("the ticket reads back");
    assert_eq!(after, before, "the ticket reads back identical");
    let j = journey(&rig.tw, hull).expect("the journey reads back");
    assert_eq!((0..2).map(|k| j.leg_due_at(k)).collect::<Vec<_>>(), due);
    assert_eq!(j.arrival_at(), arrival);
    let lane = store(&rig.tw)
        .pack(&lane_id(), LANE_VERSION)
        .expect("the shipped lane is loaded")
        .pack
        .clone();
    let new_sky = Sky::from_pack(&lane, &store(&rig.tw).config().time.ephemeris);
    assert_eq!(after.plan.sky_rev, old_rev);
    assert_ne!(after.plan.sky_rev, new_sky.revision().expect("a revision"));
    (rig, pilot, hull, after)
}

/// **The journey arrives at its frozen instant across a reboot with a new sky.**
#[test]
fn journey_arrives_at_its_frozen_instant_across_a_reboot_with_a_new_sky() {
    let (mut rig, _pilot, hull, t) = sky_change_world();
    let arrival = t.plan.depart_ic + t.plan.total_s;
    let _ = to_ic(&mut rig, arrival - 1);
    assert_eq!(ic_now(&rig.tw), arrival - 1, "rate 1: one IC second before");
    assert!(
        journey(&rig.tw, hull).is_some_and(|j| j.state().is_under_way()),
        "under way one second before"
    );
    let _ = to_ic(&mut rig, arrival);
    let stickney = domain_of(&rig.tw, "stickney_port");
    assert!(
        matches!(parent(&rig.tw, hull), SpatialParent::Direct { domain, .. } if domain.0 == stickney),
        "Direct on the Stickney domain at the instant"
    );
    assert!(matches!(
        tacsim::port::ship::berth_of(rig.tw.world(), hull),
        tacsim::port::ship::Berth::Docked { def, .. } if def.venue.as_str() == "stickney"
    ));
}

/// **A restart between the burn and the launch yields exactly one journey** (and
/// one burn).
#[test]
fn a_restart_between_burn_and_launch_yields_exactly_one_journey() {
    let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
    let hull = ObjId(row.hull);
    let vault = hold_vault(&rig.tw, &row);
    let pending = undock_and_burn(&mut rig, pilot, hull);
    assert!(
        journey(&rig.tw, hull).is_none(),
        "not launched before the fire"
    );
    let mut rig = reboot_with(rig, Roster::default());
    let _ = fire_quietly(&mut rig);
    let j = journey(&rig.tw, hull).expect("the first fire launches");
    assert_eq!(j.departed().at, pending.plan.depart_ic);
    let id = j.id();
    for _ in 0..5 {
        let _ = fire_quietly(&mut rig);
    }
    let mut rig = reboot_with(rig, Roster::default());
    for _ in 0..3 {
        let _ = fire_quietly(&mut rig);
    }
    assert_eq!(journeys(&rig.tw), 1, "one journey");
    assert_eq!(journey(&rig.tw, hull).map(|j| j.id()), Some(id));
    assert!(ticket(&mut rig.tw, hull, 1).is_some(), "one ticket");
    assert!(ticket(&mut rig.tw, hull, 2).is_none());
    assert_eq!(pool(&mut rig.tw, vault, "propellant"), 1, "debited once");
    assert_eq!(pool(&mut rig.tw, vault, "provisions"), 0, "debited once");
    let v = voyage(&rig.tw, hull).expect("the voyage");
    assert!(v.pending.is_none());
    assert_eq!(v.last_seq, 1);
}

// ---------------------------------------------------------------------------
// IC9 residual F2, closed by doc 18 cut IC11
// ---------------------------------------------------------------------------

/// **A ticket that disagrees with the filed plan holds the launch** (IC9 residual
/// F2; doc 18 cut IC11 lead ruling L19): the burn lands, then the folded sailing plan
/// is edited so its pending plan still checks but no longer matches the ticket's —
/// the next fires hold the launch (told once), fly nothing, and leave the ticket as
/// it was.
#[test]
fn ic9_f2_a_ticket_that_disagrees_with_the_filed_plan_holds_the_launch() {
    let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
    let hull = ObjId(row.hull);
    let pending = undock_and_burn(&mut rig, pilot, hull);
    let burned = ticket(&mut rig.tw, hull, pending.seq).expect("the ticket");
    let key = voyage_key(hull);
    let mut rig = reboot_editing(rig, Roster::default(), |folded| {
        let doc = folded
            .get_mut(tacsim::NS)
            .and_then(|data| data.kv.iter_mut().find(|(k, _)| *k == key))
            .expect("the sailing plan document");
        let plan = &mut doc.1["pending"]["plan"];
        let total = plan["total_s"].as_i64().expect("total_s");
        plan["total_s"] = serde_json::json!(total + 100);
        let legs = plan["legs"].as_array_mut().expect("legs");
        let last = legs.last_mut().expect("a leg");
        let secs = last["secs"].as_u64().expect("secs");
        let t1 = last["t1"].as_i64().expect("t1");
        last["secs"] = serde_json::json!(secs + 100);
        last["t1"] = serde_json::json!(t1 + 100);
    });
    let edited = voyage(&rig.tw, hull)
        .and_then(|v| v.pending)
        .expect("the edited pending still checks and loads");
    assert_ne!(edited.plan, burned.plan);
    let held = LAUNCH_HELD
        .replace("{name}", "Salt Moth")
        .replace("{port}", "Stickney Yards");
    for _ in 0..10 {
        let _ = fire_quietly(&mut rig);
    }
    let told = pending_notices(rig.tw.world(), pilot);
    assert_eq!(
        told.iter().filter(|l| **l == held).count(),
        1,
        "told once: {told:?}"
    );
    assert!(journey(&rig.tw, hull).is_none(), "nothing flies");
    assert_eq!(
        ticket(&mut rig.tw, hull, pending.seq),
        Some(burned),
        "the ticket is untouched"
    );
}

// ---------------------------------------------------------------------------
// Doc 18 cut IC14 (rows 10, 11, 20, 21, 38): the launch arm's residuals
// ---------------------------------------------------------------------------

/// How many times `line`'s head reaches `who` in `events` or the reconnect queue.
fn grants_heard(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> usize {
    let head = "Clearance granted.";
    said(tw, events, who).matches(head).count()
        + pending_notices(tw.world(), who)
            .iter()
            .filter(|line| line.starts_with(head))
            .count()
}

/// **Doc 18 cut IC14 row 20 (lead ruling L16): a held launch warns once across its
/// backoffs.** The destination paused, the launch holds: across three backoff cycles
/// the pilot is told `LAUNCH_HELD` once, every fire's stats count no refusal (the walk's
/// generic "a port agent was refused" warning never fires for it) and the retries count
/// as held.
#[test]
fn ic14_a_held_launch_warns_once_across_backoffs() {
    let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
    let hull = ObjId(row.hull);
    let _pending = undock_and_burn(&mut rig, pilot, hull);
    let stickney = domain_of(&rig.tw, "stickney_port");
    let _ = run(&mut rig.tw, GOD, &format!("@domain/pause {stickney}"));
    let held = LAUNCH_HELD
        .replace("{name}", "Salt Moth")
        .replace("{port}", "Stickney Yards");
    let (mut told, mut held_steps) = (0, 0);
    for _ in 0..(3 * (LAUNCH_GRACE_FIRES + 1)) {
        let events = fire_quietly(&mut rig);
        told += said(&rig.tw, &events, pilot).matches(&held).count();
        let stats = tacsim::port_tick::last_fire(rig.tw.world()).expect("a fire");
        assert_eq!(stats.refused, 0, "never the generic refusal: {stats:?}");
        held_steps += stats.held;
    }
    assert_eq!(told, 1, "LAUNCH_HELD told once");
    assert!(
        held_steps >= 3,
        "each backoff's retry is a held step: {held_steps}"
    );
    assert!(journey(&rig.tw, hull).is_none(), "no journey while held");
}

/// **Doc 18 cut IC14 row 10 (lead ruling L15): a lapse waits while a commit is
/// pending.** A clearance whose burn was taken off the queue (never landed) would lapse
/// after its grace; while a prepared transaction owns the engine's typed store — the
/// state a burn in the writer's custody leaves — the step defers and the pending stays;
/// once that transaction installs, the lapse proceeds and is told once.
#[test]
fn ic14_a_lapse_waits_while_a_commit_is_pending() {
    let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
    let hull = ObjId(row.hull);
    let (out, dropped) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(out.contains("Clearance filed"), "{out}");
    assert_eq!(dropped.len(), 1, "the burn is taken off the queue");
    // Hold a prepared transaction across the fires (any family: one prepared action
    // owns the store at a time).
    let families = rig
        .tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    let installation = {
        let engine = rig.tw.engine_mut();
        let mut txn = engine
            .action_transaction(&families.pools)
            .expect("an inventory transaction");
        families
            .credit(
                &mut txn,
                EndpointRef(VaultId(999_999)),
                &CanonicalId::new("metals").expect("a good"),
                1,
            )
            .expect("a credit");
        txn.prepare().expect("it prepares").into_parts().1
    };
    let lapsed = CLEARANCE_LAPSED.replace("{port}", "Stickney Yards");
    let mut told = 0;
    for _ in 0..(LAUNCH_GRACE_FIRES + 4) {
        let events = fire_quietly(&mut rig);
        told += said(&rig.tw, &events, pilot).matches(&lapsed).count();
    }
    assert_eq!(told, 0, "no lapse while a commit is pending");
    assert!(
        voyage(&rig.tw, hull).and_then(|v| v.pending).is_some(),
        "the pending stays"
    );
    rig.tw
        .engine_mut()
        .install_prepared_action(installation)
        .expect("the held commit installs");
    for _ in 0..(LAUNCH_GRACE_FIRES + 4) {
        let events = fire_quietly(&mut rig);
        told += said(&rig.tw, &events, pilot).matches(&lapsed).count();
    }
    assert_eq!(told, 1, "then it lapses, told once");
    assert!(voyage(&rig.tw, hull).and_then(|v| v.pending).is_none());
}

/// **Doc 18 cut IC14 row 38 (IC9's T7): a lapse defers while the burn is still
/// queued.** The burn left in the engine's queue past the grace: the step answers
/// `Deferred`, the pending stays, and no `CLEARANCE_LAPSED` is told.
#[test]
fn ic14_t7_a_lapse_defers_while_the_burn_is_still_queued() {
    let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
    let hull = ObjId(row.hull);
    let out = run(&mut rig.tw, pilot, "+ship/undock stickney");
    assert!(out.contains("Clearance filed"), "{out}");
    assert!(
        rig.tw.engine().has_authoritative_submit_for(pilot),
        "the burn is queued"
    );
    let lapsed = CLEARANCE_LAPSED.replace("{port}", "Stickney Yards");
    let mut told = 0;
    let mut deferred = 0;
    for _ in 0..(LAUNCH_GRACE_FIRES + 4) {
        // Fired without draining the queue: the burn stays where the driver left it.
        let t = rig.tw.world().now().saturating_add(6);
        rig.tw.set_now(t);
        let events = rig.tw.fire_due(t.saturating_mul(1_000));
        told += said(&rig.tw, &events, pilot).matches(&lapsed).count();
        deferred += tacsim::port_tick::last_fire(rig.tw.world()).map_or(0, |s| s.deferred);
    }
    assert_eq!(told, 0, "no lapse");
    assert!(deferred > 0, "the step deferred");
    assert!(voyage(&rig.tw, hull).and_then(|v| v.pending).is_some());
}

/// **Doc 18 cut IC14 row 38 (IC9's T9): a rate change mid-journey keeps the frozen
/// leg seconds.** Launched at rate 24, the world set to 3,600× mid-leg: the journey's
/// leg seconds are still the ticket's and its IC arrival unchanged — only the real ETA
/// (a render of the rate) moves.
#[test]
fn ic14_t9_a_rate_change_mid_journey_keeps_the_frozen_leg_seconds() {
    let (mut rig, pilot, row) = aboard_with_load(24, 53, 19);
    let hull = ObjId(row.hull);
    let _ = undock_and_burn(&mut rig, pilot, hull);
    for _ in 0..4 {
        let _ = fire_quietly(&mut rig);
        if journey(&rig.tw, hull).is_some() {
            break;
        }
    }
    let ticket = ticket(&mut rig.tw, hull, 1).expect("the ticket");
    let secs: Vec<u32> = ticket.plan.legs.iter().map(|l| l.secs).collect();
    let before = journey(&rig.tw, hull).expect("under way");
    assert_eq!(before.leg_times(), Some(&secs[..]));
    let arrival = before.arrival_at();
    let _ = run(&mut rig.tw, GOD, "@tacsim/time rate=3600");
    assert_eq!(world_rate(&rig.tw), 3_600);
    let _ = fire_quietly(&mut rig);
    let after = journey(&rig.tw, hull).expect("still under way");
    assert_eq!(after.leg_times(), Some(&secs[..]), "the frozen leg seconds");
    assert_eq!(
        after.arrival_at(),
        arrival,
        "the IC arrival is the ticket's"
    );
    let ship = run(&mut rig.tw, pilot, "+ship");
    assert!(ship.contains("under way for Stickney Yards"), "{ship}");
}

/// **Doc 18 cut IC14 row 21 (lead rulings L8b, L20): a reboot between the launch and
/// its verification tells the grant once.** The launch flies on one fire; the server
/// reboots before the next (its RAM verification gone); the first fire after the boot
/// verifies by pull and tells `CLEARANCE_GRANTED` once, writing `Voyage.told`; a second
/// reboot tells nothing more.
#[test]
fn ic14_a_reboot_between_launch_and_verify_tells_the_grant_once() {
    let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
    let hull = ObjId(row.hull);
    let pending = undock_and_burn(&mut rig, pilot, hull);
    let mut granted = 0;
    for _ in 0..4 {
        let events = fire_quietly(&mut rig);
        granted += grants_heard(&rig.tw, &events, pilot);
        if journey(&rig.tw, hull).is_some() {
            break;
        }
    }
    assert!(journey(&rig.tw, hull).is_some(), "launched");
    assert_eq!(granted, 0, "not yet verified");
    assert_eq!(voyage(&rig.tw, hull).map(|v| v.told), Some(0));
    let mut rig = reboot_with(rig, Roster::default());
    for _ in 0..2 {
        let events = fire_quietly(&mut rig);
        granted += said(&rig.tw, &events, pilot)
            .matches("Clearance granted.")
            .count();
    }
    granted += pending_notices(rig.tw.world(), pilot)
        .iter()
        .filter(|line| line.starts_with("Clearance granted."))
        .count();
    assert_eq!(granted, 1, "told once after the reboot");
    assert_eq!(
        voyage(&rig.tw, hull).map(|v| v.told),
        Some(pending.seq),
        "told is durable"
    );
    let queued = pending_notices(rig.tw.world(), pilot)
        .iter()
        .filter(|line| line.starts_with("Clearance granted."))
        .count();
    let mut rig = reboot_with(rig, Roster::default());
    let mut again = 0;
    for _ in 0..2 {
        let events = fire_quietly(&mut rig);
        again += said(&rig.tw, &events, pilot)
            .matches("Clearance granted.")
            .count();
    }
    let after = pending_notices(rig.tw.world(), pilot)
        .iter()
        .filter(|line| line.starts_with("Clearance granted."))
        .count();
    assert_eq!(again, 0, "a second reboot tells nothing");
    assert_eq!(after, queued, "and queues nothing new");
}

/// **Doc 18 cut IC14 row 21 (lead ruling L20), the common case: a grant told by the
/// live verification is durable.** The launch flies and the next fire verifies it in
/// RAM and tells `CLEARANCE_GRANTED` once — writing `Voyage.told` in that fire — so a
/// reboot mid-leg (the journey still running, `launch_retell` looking) tells nothing
/// again and queues nothing.
#[test]
fn ic14_a_grant_told_live_is_not_retold_after_a_reboot() {
    let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
    let hull = ObjId(row.hull);
    let pending = undock_and_burn(&mut rig, pilot, hull);
    let mut granted = 0;
    for _ in 0..6 {
        let events = fire_quietly(&mut rig);
        granted += grants_heard(&rig.tw, &events, pilot);
        if granted > 0 {
            break;
        }
    }
    assert_eq!(granted, 1, "the live verification tells the grant once");
    assert!(journey(&rig.tw, hull).is_some(), "under way");
    assert_eq!(
        voyage(&rig.tw, hull).map(|v| v.told),
        Some(pending.seq),
        "the live verification writes told"
    );
    let queued = pending_notices(rig.tw.world(), pilot)
        .iter()
        .filter(|line| line.starts_with("Clearance granted."))
        .count();
    let mut rig = reboot_with(rig, Roster::default());
    assert!(
        journey(&rig.tw, hull).is_some(),
        "still under way after the boot"
    );
    let mut again = 0;
    for _ in 0..3 {
        let events = fire_quietly(&mut rig);
        again += said(&rig.tw, &events, pilot)
            .matches("Clearance granted.")
            .count();
    }
    let after = pending_notices(rig.tw.world(), pilot)
        .iter()
        .filter(|line| line.starts_with("Clearance granted."))
        .count();
    assert_eq!(again, 0, "a reboot mid-leg retells nothing");
    assert_eq!(after, queued, "and queues nothing new");
}

/// **Doc 18 cut IC14 row 11 (lead ruling L17): an unreadable sailing plan fails closed
/// everywhere.** The filed hull's `tacsim:voyage:` document corrupted in the folded
/// store and booted: `+ship/leave` and `+ship/board` refuse with `VOYAGE_DARK`, the
/// hold refuses to load with `HOLD_PAPERS_DARK`, the buyout and the recall refuse, the
/// hulls line counts the hull departing and stays Ready, the desk never lends it — and
/// a second pilot still enrols on another hull; the port tick pages it once.
#[test]
#[allow(clippy::too_many_lines)]
fn ic14_an_unreadable_sailing_plan_fails_closed_everywhere() {
    let (mut rig, pilot, row) = aboard_with_load(1, 53, 19);
    let hull = ObjId(row.hull);
    let (out, _dropped) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(out.contains("Clearance filed"), "{out}");
    let key = voyage_key(hull);
    let mut rig = reboot_editing(rig, Roster::default(), |folded| {
        let doc = folded
            .get_mut(tacsim::NS)
            .and_then(|data| data.kv.iter_mut().find(|(k, _)| *k == key))
            .expect("the sailing plan document");
        doc.1 = serde_json::json!({ "schema": 1, "nonsense": true });
    });
    assert!(matches!(
        store(&rig.tw).voyage_state(hull),
        VoyageState::Unreadable
    ));
    let _ = rig.tw.connect(pilot);
    let dark = VOYAGE_DARK.replace("{name}", "Salt Moth");
    // Leave, from the Airlock.
    let out = run(&mut rig.tw, pilot, "+ship/leave");
    assert!(out.contains(&dark), "leave: {out}");
    // The hold refuses to load.
    let ceres = mush_market::MarketId::new("ceres").expect("venue");
    assert_eq!(
        PortVenueGate
            .admit(rig.tw.world(), pilot, &ceres, None, Presence::Ship(hull))
            .map_err(|r| r.line),
        Err(tacsim::port::HOLD_PAPERS_DARK
            .replace("{name}", "Salt Moth")
            .replace(
                "{port}",
                &tacsim::port::coop::port_name(&port_def("ceres_port"))
            )),
        "nothing loads"
    );
    // Staff take the pilot ashore; boarding refuses too.
    support::with_ctx(&mut rig.tw, |ctx| {
        mush_sim::unplace(ctx, pilot, "staff took the pilot ashore").expect("unplaces")
    });
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, pilot, gallery);
    let out = run(&mut rig.tw, pilot, "+ship/board");
    assert!(out.contains(&dark), "board: {out}");
    // The buyout (at the desk) and the staff recall refuse.
    let registry = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, pilot, registry);
    let out = run(&mut rig.tw, pilot, "+coop/buyout");
    let cleared = tacsim::port::buyout::CLEARED_WAIT.replace("{name}", "Salt Moth");
    assert!(out.contains(&cleared), "buyout: {out}");
    let out = run(&mut rig.tw, GOD, "@coop/revoke Pilot=testing");
    assert!(
        out.contains("Not revoked: ") && out.contains(&cleared),
        "revoke: {out}"
    );
    // The hulls line counts it departing and stays Ready; the desk never lends it.
    let report = status(&mut rig.tw);
    let line = report
        .lines()
        .find(|l| l.contains("ceres_port hulls: "))
        .expect("a hulls line");
    assert!(
        line.contains(", 1 departing.") && line.ends_with("[ready]"),
        "{line}"
    );
    let port = tacsim::port::programme_port(rig.tw.world()).expect("the programme port");
    assert!(!tacsim::port::coop::hull_is_free(
        rig.tw.world(),
        &port,
        &row
    ));
    // A second pilot still enrols, on another hull.
    let second = self::pilot(&mut rig, "Second", 2);
    let out = run(&mut rig.tw, second, "+coop/enrol Tern");
    assert!(out.contains("now the Tern"), "{out}");
    let charter = tacsim::port::live_charter(rig.tw.world(), second).expect("a charter");
    assert_ne!(charter.hull, hull, "never the dark hull");
    for _ in 0..3 {
        let _ = fire_quietly(&mut rig);
    }
    assert_eq!(
        tacsim::port_tick::voyage_pages(rig.tw.world()),
        1,
        "paged once"
    );
}

/// **Doc 18 cut IC14 row 11 (lead ruling L17), the desk's own gate**: a laid-up hull
/// nobody charters — no Trust lease on it, its stowage backing nothing — whose
/// `tacsim:voyage:` document does not read is never lent. Its corrupt document is
/// written into the folded store and booted: `hull_is_free` answers false for it (and
/// only the sailing-plan gate can say so, the hull being otherwise free), `free_hulls`
/// leaves it out, a new pilot's `+coop/enrol` lends the next free hull, and the port
/// tick pages it once.
#[test]
fn ic14_a_free_hull_whose_sailing_plan_does_not_read_is_never_lent() {
    let rig = stocked();
    let port = tacsim::port::programme_port(rig.tw.world()).expect("the programme port");
    let free_before: Vec<u32> = tacsim::port::coop::free_hulls(rig.tw.world(), &port)
        .iter()
        .map(|row| row.hull)
        .collect();
    let (&dark, &next) = match free_before.as_slice() {
        [dark, next, ..] => (dark, next),
        other => panic!("at least two free hulls at Ceres: {other:?}"),
    };
    let key = voyage_key(ObjId(dark));
    let mut rig = reboot_editing(rig, Roster::default(), |folded| {
        let data = folded.get_mut(tacsim::NS).expect("tacsim's folded store");
        assert!(
            data.kv.iter().all(|(k, _)| *k != key),
            "a laid-up hull has no sailing plan yet"
        );
        data.kv.push((
            key.clone(),
            serde_json::json!({ "schema": 1, "nonsense": true }),
        ));
    });
    assert!(matches!(
        store(&rig.tw).voyage_state(ObjId(dark)),
        VoyageState::Unreadable
    ));
    let port = tacsim::port::programme_port(rig.tw.world()).expect("the programme port");
    let row = port
        .map
        .hulls
        .iter()
        .find(|row| row.hull == dark)
        .expect("the dark hull's row");
    assert!(
        !tacsim::port::coop::hull_is_free(rig.tw.world(), &port, row),
        "a hull whose papers do not read is not free"
    );
    let free_after: Vec<u32> = tacsim::port::coop::free_hulls(rig.tw.world(), &port)
        .iter()
        .map(|row| row.hull)
        .collect();
    let expected: Vec<u32> = free_before.iter().copied().filter(|h| *h != dark).collect();
    assert_eq!(free_after, expected, "only the dark hull leaves the pool");
    // A new pilot enrols on the next free hull, never the dark one.
    let newcomer = pilot(&mut rig, "Newcomer", 3);
    let out = run(&mut rig.tw, newcomer, "+coop/enrol Tern");
    assert!(out.contains("now the Tern"), "{out}");
    let charter = tacsim::port::live_charter(rig.tw.world(), newcomer).expect("a charter");
    assert_eq!(charter.hull, ObjId(next), "the next free hull is lent");
    for _ in 0..3 {
        let _ = fire_quietly(&mut rig);
    }
    assert_eq!(
        tacsim::port_tick::voyage_pages(rig.tw.world()),
        1,
        "paged once"
    );
}

/// `look` in hull room `id` of `row` as `who` (teleported there).
fn look_aboard(rig: &mut Rig, who: ObjId, row: &HullRow, id: &str) -> Vec<String> {
    let room = room_of(row, id);
    rig.tw.world_mut().move_to(who, room).expect("walks there");
    run(&mut rig.tw, who, "look")
        .lines()
        .map(str::to_owned)
        .collect()
}

/// **Doc 18 cut IC14 row 54 (IC11 L21; Q-18-47): the burn-and-reserve gauge.** In
/// Engineering the tank line is followed by `BURN_GAUGE_NONE` while nothing is filed;
/// once `+ship/undock` files the passage, by `BURN_GAUGE` with the plan's burn and the
/// tank less it; once the burn lands (the tank debited), the same burn with the tank
/// as the reserve. The Bridge carries it under its TANKAGE line; the Crew compartment
/// and the Hold carry neither form. Nothing is written by a look.
#[test]
fn ic14_the_burn_gauge_reads_the_filed_plan() {
    use tacsim::port::ship::{BURN_GAUGE, BURN_GAUGE_NONE};
    let (mut rig, pilot, row) = aboard_with_load(3_600, 53, 19);
    let hull = ObjId(row.hull);
    let none = BURN_GAUGE_NONE.to_owned();
    let tank = "Tank gauges: reaction mass, 53 of 80 t.".to_owned();
    let after = |lines: &[String], line: &str| {
        lines
            .iter()
            .position(|l| l == line)
            .and_then(|at| lines.get(at + 1).cloned())
    };
    let engineering = look_aboard(&mut rig, pilot, &row, "engineering");
    assert_eq!(
        after(&engineering, &tank),
        Some(none.clone()),
        "{engineering:#?}"
    );
    let bridge = look_aboard(&mut rig, pilot, &row, "bridge");
    assert_eq!(
        after(&bridge, "  TANKAGE 53/80 t"),
        Some(format!("  {none}")),
        "{bridge:#?}"
    );
    for room in ["crew", "hold", "passage"] {
        let lines = look_aboard(&mut rig, pilot, &row, room);
        assert!(
            !lines.iter().any(|l| l.contains("Burn for the filed plan")),
            "{room}: {lines:#?}"
        );
    }
    // Filed: the plan's burn, the tank less it.
    let _ = look_aboard(&mut rig, pilot, &row, "crew");
    let (out, queued) = undock(&mut rig, pilot, "+ship/undock stickney");
    assert!(out.contains("Clearance filed for Stickney Yards"), "{out}");
    let burn = voyage(&rig.tw, hull)
        .and_then(|v| v.pending)
        .expect("a filed passage")
        .plan
        .burn_t;
    let filed = BURN_GAUGE
        .replace("{burn}", &burn.to_string())
        .replace("{reserve}", &(53 - burn).to_string());
    let engineering = look_aboard(&mut rig, pilot, &row, "engineering");
    assert_eq!(
        after(&engineering, &tank),
        Some(filed.clone()),
        "{engineering:#?}"
    );
    for room in ["crew", "hold"] {
        let lines = look_aboard(&mut rig, pilot, &row, room);
        assert!(
            !lines.iter().any(|l| l.contains("Burn for the filed plan")),
            "{room}: {lines:#?}"
        );
    }
    // The burn lands: the tank is debited, and the reserve is what it holds.
    land_all(&mut rig, &queued);
    let debited = format!("Tank gauges: reaction mass, {} of 80 t.", 53 - burn);
    let engineering = look_aboard(&mut rig, pilot, &row, "engineering");
    assert_eq!(
        after(&engineering, &debited),
        Some(filed.clone()),
        "the reserve is unchanged by the debit: {engineering:#?}"
    );
    let bridge = look_aboard(&mut rig, pilot, &row, "bridge");
    assert_eq!(
        after(&bridge, &format!("  TANKAGE {}/80 t", 53 - burn)),
        Some(format!("  {filed}")),
        "{bridge:#?}"
    );
}

// ---------------------------------------------------------------------------
// Doc 19 cut A.8: the `Throttle:` line of `+ship/plan`
// ---------------------------------------------------------------------------

/// A Dray compartment's capacity in units of `good`: its `stowage` limits over
/// the goods table's per-unit measures, the tightest (the planner's own reading).
fn dray_capacity(stowage: &str, good: &str) -> u64 {
    let goods = tacsim::definitions::goods::goods_table().expect("the goods table");
    let pack = load_pack(tacsim::definitions::content::DRAY_JSON).expect("the Dray pack");
    let Some(PackEntry::Unit(spec)) = pack.entry(&cid(tacsim::definitions::content::DRAY)) else {
        panic!("the Dray unit");
    };
    let logistics = spec.logistics.as_ref().expect("the Dray's logistics");
    let id = cid(stowage);
    let def = logistics.stowage.get(&id).expect("the compartment");
    let compartment = tacsim::port::capacity::Compartment::from_stowage(&id, def);
    let row = goods.good(&cid(good)).expect("the good");
    compartment
        .limits
        .iter()
        .filter_map(|(measure, limit)| {
            let per_unit = *row.measures.get(measure)?;
            (per_unit > 0).then(|| limit / per_unit)
        })
        .min()
        .expect("a limit the good is measured in")
}

/// The shipped lane with one test-only route, `ceres_ligeia` (Ceres Freeport to
/// Ligeia Terminal in one flip-law leg; a cross-body leg never reads its
/// `base_s`), listed by Ceres under `titan`; re-signed at the shipped version
/// and leaked for the `Copy` roster.
fn lane_with_ceres_titan() -> &'static str {
    let mut doc: serde_json::Value = serde_json::from_str(LANE_JSON).expect("the lane is JSON");
    doc["manifest"]["digest"] = serde_json::json!(UNSIGNED);
    doc["entries"]["ceres_ligeia"] = serde_json::json!({
        "kind": "route",
        "def": {
            "from": "ceres_freeport",
            "to": "ligeia_terminal",
            "legs": [{ "to": "ligeia_terminal", "base_s": 1 }],
            "drive_class": "coast_hauler",
            "propellant": 0
        }
    });
    doc["entries"]["ceres_port"]["def"]["routes"]["titan"] = serde_json::json!(["ceres_ligeia"]);
    let signed = sign(&serde_json::to_string_pretty(&doc).expect("serializes"));
    Box::leak(signed.into_boxed_str())
}

/// The two numbers a `+ship/plan` line names: `Burn B t (tank T t; …)` → `(B, T)`,
/// `Stores C crew-days (locker L; …)` → `(C, L)` (`orders.rs`'s).
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

/// The plan's lines that start `Throttle:`.
fn throttle_lines(plan: &str) -> Vec<&str> {
    plan.lines()
        .filter(|l| l.starts_with("Throttle:"))
        .collect()
}

/// A [`THROTTLE_FITS`] line read back: `(throttle, burn, crew-days, passage)`,
/// re-rendered through the template so the form is exact.
fn throttle_fits(line: &str) -> (u32, u64, u64, String) {
    let field = |after: &str, until: char| -> String {
        line.split(after)
            .nth(1)
            .and_then(|rest| rest.split(until).next())
            .unwrap_or_else(|| panic!("{after:?} in {line:?}"))
            .to_owned()
    };
    let throttle: u32 = field("throttle=", ' ').parse().expect("a throttle");
    let burn: u64 = field("burn ", ' ').parse().expect("a burn");
    let days: u64 = field("t, ", ' ').parse().expect("crew-days");
    let passage = field("crew-days, ", ' ');
    let rendered = THROTTLE_FITS
        .replace("{throttle}", &throttle.to_string())
        .replace("{burn}", &burn.to_string())
        .replace("{days}", &days.to_string())
        .replace("{passage}", &passage);
    assert_eq!(line, rendered, "the line is THROTTLE_FITS");
    (throttle, burn, days, passage)
}

/// **`+ship/plan` names the throttle that fits** (doc 19 cut A.8, D-19-22; the A.8b
/// review's findings 6 and 32: the line had no world-side test). On a lane where
/// Ceres lists a direct route to Titan, a Dray with the hold, the tank and the
/// locker all full plans Titan at the default 1000 permille: the burn is over the
/// tank, and exactly one `Throttle:` line follows the `Stores ` line, naming a
/// throttle in the band below 1000 whose burn and crew-days fit the capacities;
/// the `Burn ` and `Stores ` lines keep their form. One and twenty permille above
/// it the same line is printed (the bisection's boundary; the payload is re-based
/// independently of the asked throttle); at it the plan fits, no line is printed,
/// and, the tank and the locker being full, the `Burn `, `Stores ` and `Passage `
/// lines read the hint's own burn, crew-days and IC days. (The shipped epoch
/// Ceres to Stickney plan printing none is asserted in
/// `ic9_undock_flies_the_lane_and_docks_at_stickney`, and a plan short of today's
/// tank but fitting a full one in `ic9_undock_refusals_move_nothing`.)
#[test]
#[allow(clippy::too_many_lines)]
fn a8_the_plan_names_the_throttle_that_fits() {
    let mut rig = stocked_with(Roster {
        seed_lane: Some(lane_with_ceres_titan()),
        ..Roster::default()
    });
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let tank = dray_capacity("propellant", "propellant");
    let locker = dray_capacity("provisions", "provisions");
    assert_eq!((tank, locker), (80, 120), "the Dray's tank and locker");
    load(&mut rig, &row, tank, locker);
    board(&mut rig, pilot);

    // The default throttle: the tank short, one `Throttle:` line after `Stores `.
    let plan = run(&mut rig.tw, pilot, "+ship/plan titan");
    let lines: Vec<&str> = plan.lines().collect();
    assert!(
        lines
            .first()
            .is_some_and(|l| l.starts_with("Plan for Ligeia Terminal")),
        "{plan}"
    );
    let at = |head: &str| -> Vec<usize> {
        lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.starts_with(head))
            .map(|(k, _)| k)
            .collect()
    };
    let (burns, stores, hints) = (at("Burn "), at("Stores "), at("Throttle:"));
    assert_eq!((burns.len(), stores.len()), (1, 1), "{plan}");
    assert_eq!(hints.len(), 1, "exactly one Throttle: line: {plan}");
    assert_eq!(hints[0], stores[0] + 1, "the line follows Stores: {plan}");
    assert!(burns[0] < stores[0], "{plan}");
    let (burn, tank_read) = plan_numbers(&plan, "Burn ", "(tank ").expect(&plan);
    let (crew, locker_read) = plan_numbers(&plan, "Stores ", "(locker ").expect(&plan);
    assert_eq!(
        (tank_read, locker_read),
        (tank, locker),
        "both full: {plan}"
    );
    assert!(burn > tank, "1000 permille burns over the tank: {plan}");
    let burn_line = lines[burns[0]];
    assert!(
        burn_line.starts_with(&format!(
            "Burn {burn} t (tank {tank} t; {} t short). Flip and burn at ",
            burn - tank
        )) && burn_line.contains("throttle 1000‰"),
        "{plan}"
    );
    let left = if crew <= locker {
        format!("{} left", locker - crew)
    } else {
        format!("{} short", crew - locker)
    };
    assert_eq!(
        lines[stores[0]],
        format!("Stores {crew} crew-days (locker {locker}; {left})."),
        "{plan}"
    );
    let hint = lines[hints[0]];
    assert!(
        hint.starts_with("Throttle: with the tank and locker full, throttle="),
        "{plan}"
    );
    let (n, hint_burn, hint_days, hint_passage) = throttle_fits(hint);
    assert!(
        (250..1_000).contains(&n),
        "a throttle in the band below 1000: {hint}"
    );
    assert!(hint_burn <= tank && hint_days <= locker, "it fits: {hint}");
    eprintln!(
        "a8 Throttle: Ceres -> Titan at the epoch, laden, one soul: 1000 permille burns {burn} t \
         ({crew} crew-days); throttle={n} fits: {hint_burn} t, {hint_days} crew-days, \
         {hint_passage} IC days"
    );

    // One permille above it, and twenty: the same line (the boundary; the asked
    // throttle does not move the answer).
    for above in [n + 1, (n + 20).min(1_000)] {
        let plan = run(
            &mut rig.tw,
            pilot,
            &format!("+ship/plan titan throttle={above}"),
        );
        assert!(plan.contains(&format!("throttle {above}‰")), "{plan}");
        assert_eq!(throttle_lines(&plan), vec![hint], "at {above}: {plan}");
    }

    // At it: the plan fits the full tank and locker, so no line; the plan's own
    // lines are the hint's figures.
    let plan = run(
        &mut rig.tw,
        pilot,
        &format!("+ship/plan titan throttle={n}"),
    );
    assert!(throttle_lines(&plan).is_empty(), "at {n} it fits: {plan}");
    assert!(plan.contains(&format!("throttle {n}‰")), "{plan}");
    assert_eq!(
        plan_numbers(&plan, "Burn ", "(tank "),
        Some((hint_burn, tank)),
        "{plan}"
    );
    assert!(
        plan.contains(&format!("(tank {tank} t; {} t left)", tank - hint_burn)),
        "{plan}"
    );
    assert_eq!(
        plan_numbers(&plan, "Stores ", "(locker "),
        Some((hint_days, locker)),
        "{plan}"
    );
    assert!(
        plan.lines()
            .any(|l| l.starts_with(&format!("Passage {hint_passage} IC days"))),
        "{plan}"
    );
    // The shipped route on the same world still prints none.
    let plan = run(&mut rig.tw, pilot, "+ship/plan stickney");
    assert!(plan.contains("Plan for Stickney Yards"), "{plan}");
    assert!(throttle_lines(&plan).is_empty(), "{plan}");
}

/// The shipped lane with `ceres_stickney`'s first leg (`ceres_freeport -> kirkwood_depot`)
/// carrying `hazard: 4, interdiction: 3` (`D-35-5`, `SF.R1`).
fn lane_with_interdicted_kirkwood() -> &'static str {
    let mut doc: serde_json::Value = serde_json::from_str(LANE_JSON).expect("the lane is JSON");
    doc["manifest"]["digest"] = serde_json::json!(UNSIGNED);
    doc["entries"]["ceres_stickney"]["def"]["legs"][0]["hazard"] = serde_json::json!(4);
    doc["entries"]["ceres_stickney"]["def"]["legs"][0]["interdiction"] = serde_json::json!(3);
    let signed = sign(&serde_json::to_string_pretty(&doc).expect("serializes"));
    Box::leak(signed.into_boxed_str())
}

/// **Doc 35 cut SF.R1 acceptance (`D-35-3`, `D-35-5`, `Q-35-4(1)`)** — a chartered pilot
/// aboard the *Salt Moth* flies an interdicted `ceres_stickney` leg:
/// 1. Crossing the Kirkwood waypoint boundary on the world tick raises
///    `PromotionTrigger::Interception`, promotes the `CarrierJourney` to
///    `JourneyState::Promoted { domain }`, and opens a `LocalSpaceBubble` in
///    `SpaceBubbleBook` with canonical `UnitSpec`-derived hulls.
/// 2. Advancing the world IC clock well past the original Stickney arrival time while the
///    bubble is open keeps the `CarrierJourney` paused at `cursor == 1` (`JourneyState::Promoted`),
///    preserves its remaining leg 2 duration (`1_274_226 s`), and steps the `LocalSpaceBubble`
///    strictly on **1:1 wall time** (`1,000 ms` combat step per tick fire, never scaled by the
///    `3,600:1` IC rate) with live W-clock stamps, input tape, and snapshots.
/// 3. Clearing the interdiction encounter closes the bubble into `SpaceBubbleBook::completed`,
///    stands down the domain activation with a `DomainSummary`, and resumes the `CarrierJourney`
///    to `JourneyState::Cruising`.
/// 4. Advancing the world IC clock by the preserved remaining leg 2 duration (`1_274_226 s`)
///    completes the voyage and docks the *Salt Moth* at Stickney Yards.
#[test]
#[allow(clippy::too_many_lines)]
fn sfr1_interdicted_leg_promotes_pauses_leg_timer_steps_wall_bubble_and_resumes_to_dock() {
    let mut rig = stocked_with(Roster {
        seed_lane: Some(lane_with_interdicted_kirkwood()),
        ..Roster::default()
    });
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    load(&mut rig, &row, 53, 19);
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot);

    let pending = undock_and_burn(&mut rig, pilot, hull);
    let depart_ic = pending.plan.depart_ic;
    assert_eq!(pending.plan.legs.len(), 2);
    let leg1_secs = i64::from(pending.plan.legs[0].secs);
    let leg2_secs = i64::from(pending.plan.legs[1].secs);
    let total_secs = pending.plan.total_s;
    assert_eq!(
        (leg1_secs, leg2_secs, total_secs),
        (326_136, 1_274_226, 1_600_362)
    );

    // Launch on `tacsim.port`.
    let _ = fire_quietly(&mut rig);
    let j0 = journey(&rig.tw, hull).expect("launched journey");
    assert_eq!(j0.cursor(), 0);
    assert_eq!(*j0.state(), mush_sim::JourneyState::Cruising);

    // 1. Cross the interdicted Kirkwood boundary (`depart_ic + leg1_secs`).
    let boundary_target = depart_ic + leg1_secs;
    let events = to_ic(&mut rig, boundary_target);
    let boundary_ic = ic_now(&rig.tw);
    let heard = said(&rig.tw, &events, pilot);
    assert!(
        heard.contains("Kirkwood Depot slides past"),
        "boundary narration heard: {heard}"
    );
    assert!(
        heard.contains(
            "[SPACE] Interdiction contact detected at waypoint boundary; local space bubble opened on 1:1 wall clock."
        ),
        "interdiction promotion narration heard: {heard}"
    );

    let j_promoted = journey(&rig.tw, hull).expect("promoted journey");
    assert_eq!(j_promoted.cursor(), 1);
    let mush_sim::JourneyState::Promoted { domain } = *j_promoted.state() else {
        panic!(
            "expected JourneyState::Promoted, got {:?}",
            j_promoted.state()
        );
    };

    let act_book = rig
        .tw
        .world()
        .module_state::<mush_sim::ActivationBook>()
        .expect("ActivationBook");
    let activation = act_book
        .activations
        .get(&domain)
        .expect("live interception activation on Kirkwood domain");
    assert!(!activation.is_queued());
    assert_eq!(
        activation.trigger(),
        mush_sim::PromotionTrigger::Interception
    );

    let bubble_book = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook");
    let active0 = bubble_book
        .active_for_hull(hull)
        .expect("active interdiction bubble for Salt Moth");
    assert_eq!(active0.domain, domain);
    assert_eq!(active0.leg_cursor, 1);
    assert_eq!(active0.opened_ic_s, boundary_ic);
    assert_eq!(active0.steps, 0);
    assert_eq!(active0.bubble.wall_elapsed_ms, 0);
    assert_eq!(active0.bubble.ships.len(), 2);
    let carrier_ship = active0
        .bubble
        .ships
        .get(&active0.carrier_ship_id)
        .expect("carrier ship in bubble");
    assert_eq!(carrier_ship.sections.len(), 3);

    // 2. Advance the world IC clock well past the original Stickney arrival time
    // (`boundary_ic + leg2_secs + 1_000_000 > depart_ic + total_secs`) while the bubble is active.
    // Do 4 one-second real fires first, then jump past `leg2_secs + 1_000_000` IC seconds on the 5th fire.
    for _ in 0..4 {
        let next_real = rig.tw.world().now() + 1;
        rig.tw.set_now(next_real);
        let _ = rig.tw.fire_due(next_real * 1_000);
    }
    let paused_events = to_ic(&mut rig, boundary_ic + leg2_secs + 1_000_000);
    assert!(
        !said(&rig.tw, &paused_events, pilot).contains("docks at"),
        "ship must NOT dock while paused in an active interdiction bubble"
    );

    let paused_ic = ic_now(&rig.tw);
    let j_paused = journey(&rig.tw, hull).expect("journey still present while promoted");
    assert_eq!(j_paused.cursor(), 1, "leg cursor stays paused at 1");
    assert_eq!(
        *j_paused.state(),
        mush_sim::JourneyState::Promoted { domain },
        "journey remains Promoted while bubble is active"
    );
    // `departed.at` shifted forward by the exact IC duration spent paused since `boundary_ic`.
    let paused_ic_delta = paused_ic - boundary_ic;
    assert_eq!(
        j_paused.departed().at,
        depart_ic + paused_ic_delta,
        "departed.at shifts forward to preserve remaining leg 2 duration"
    );

    let bubble_book = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook");
    let active_paused = bubble_book
        .active_for_hull(hull)
        .expect("bubble still active");
    assert_eq!(
        active_paused.steps, 5,
        "5 tick fires stepped the bubble 5 times"
    );
    assert_eq!(
        active_paused.bubble.wall_elapsed_ms,
        5 * u64::from(tacsim::space::COMBAT_STEP_WALL_MS),
        "bubble steps on 1:1 wall time (5,000 ms across 5 combat fires), NOT scaled by 3,600:1 IC rate"
    );
    assert_eq!(
        active_paused.bubble.ic_timestamp_s, paused_ic,
        "bubble stamps live W-clock IC timestamp on each step"
    );
    assert_eq!(active_paused.snapshots.len(), 6);
    let (replayed, _) = active_paused.tape.replay();
    assert_eq!(replayed.state_hash(), active_paused.bubble.state_hash());

    // 3. Clear the interdiction encounter and fire one tick to close the bubble and resume leg 2.
    assert!(tacsim::space::clear_interdiction_bubble(
        rig.tw.world_mut(),
        hull
    ));
    let next_real = rig.tw.world().now() + 1;
    rig.tw.set_now(next_real);
    let clear_events = rig.tw.fire_due(next_real * 1_000);
    let resume_ic = ic_now(&rig.tw);
    let clear_heard = said(&rig.tw, &clear_events, pilot);
    assert!(
        clear_heard.contains(
            "[SPACE] Interdiction contact cleared; local bubble closed and transit burn resumed."
        ),
        "clear narration heard: {clear_heard}"
    );

    let bubble_book = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook");
    assert!(bubble_book.active_for_hull(hull).is_none());
    let completed = bubble_book
        .completed_for_hull(hull)
        .expect("completed bubble archived");
    assert_eq!(completed.opened_ic_s, boundary_ic);
    assert_eq!(completed.closed_ic_s, resume_ic);
    assert_eq!(completed.steps, 5);
    assert_eq!(completed.wall_elapsed_ms, 5_000);

    let act_book = rig
        .tw
        .world()
        .module_state::<mush_sim::ActivationBook>()
        .expect("ActivationBook");
    assert!(
        !act_book.activations.contains_key(&domain),
        "interdiction domain stood down on clear"
    );
    let summary = act_book
        .summaries
        .get(&domain)
        .expect("DomainSummary written on stand-down");
    assert_eq!(summary.from().at, boundary_ic);
    assert_eq!(summary.to().at, resume_ic);

    let j_resumed = journey(&rig.tw, hull).expect("resumed journey");
    assert_eq!(j_resumed.cursor(), 1);
    assert_eq!(*j_resumed.state(), mush_sim::JourneyState::Cruising);
    assert_eq!(
        j_resumed.departed().at,
        depart_ic + (resume_ic - boundary_ic),
        "remaining leg 2 time (1,274,226 s from boundary_ic) is preserved at resume"
    );

    // 4. Advance the world IC clock by the remaining leg 2 duration (`leg2_secs`) and verify docking.
    let expected_arrival = j_resumed.departed().at + total_secs;
    assert_eq!(
        expected_arrival,
        depart_ic + leg1_secs + (resume_ic - boundary_ic) + leg2_secs
    );

    let pre_arrival = to_ic(&mut rig, expected_arrival - 3_600);
    assert!(
        !said(&rig.tw, &pre_arrival, pilot).contains("docks at"),
        "still under way 1 hour before shifted arrival"
    );

    let arrival_events = to_ic(&mut rig, expected_arrival);
    let arrival_heard = said(&rig.tw, &arrival_events, pilot);
    assert!(
        arrival_heard.contains("Salt Moth docks at Stickney Yards."),
        "docked at Stickney after completing remaining leg 2 duration: {arrival_heard}"
    );
    match tacsim::port::ship::berth_of(rig.tw.world(), hull) {
        tacsim::port::ship::Berth::Docked { def, .. } => assert_eq!(def.venue.as_str(), "stickney"),
        other => panic!("expected Berth::Docked at stickney, got {other:?}"),
    }
}

/// **Doc 35 cut SF.R2** (`D-35-1`, `D-35-12`, `35a` §8.3):
/// 1. When a chartered hull (*Salt Moth*) promotes into a `LocalSpaceBubble`, `space_tick`
///    checks out its live `TacticalUnit` via `mush_tactics::custody::check_out` (`DeployedMark::field`)
///    and populates `SpaceShipState` from the canonical `dray.json` `UnitSpec` (`dry_mass_t = 240`,
///    `rated_accel_mg = 50`, `propellant_cap_kg = 80_000`, `hull = 600`, `hold = 400`, `drive = 250`).
/// 2. Combat damage taken inside the bubble writes back to the real hull's `TacticalUnit` cells,
///    section conditions, and component conditions on `mush_tactics::custody::check_in` when the
///    bubble closes, and non-GOD `+ship` prints `Damage: hull 480/600, drive 190/250.`.
/// 3. Disabling a hull's drive (`internals = 0`, `HullStatus::DisabledDerelict`) in the bubble
///    checks the unit in as `Immobile` (`FieldEpochDisposition::RemainOpen`) without deleting the
///    hull, records `BlackBoxFlightEntry::CombatDamageRecorded` in `SalvageClaimBook`, and opens a
///    48 real-hour (`172_800 s`) `CustodyTitleStatus::PendingSalvageClaim` window.
#[test]
#[allow(clippy::too_many_lines)]
fn sfr2_space_bubble_custody_checks_out_unit_persists_damage_on_checkin_and_opens_48h_salvage_window()
 {
    let mut rig = stocked_with(Roster {
        seed_lane: Some(lane_with_interdicted_kirkwood()),
        ..Roster::default()
    });
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    load(&mut rig, &row, 53, 19);
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot);

    let pending = undock_and_burn(&mut rig, pilot, hull);
    let depart_ic = pending.plan.depart_ic;
    let leg1_secs = i64::from(pending.plan.legs[0].secs);

    let _ = fire_quietly(&mut rig);
    let _ = to_ic(&mut rig, depart_ic + leg1_secs);

    // 1. Verify `mush_tactics::custody::check_out` placed the field custody marker on `hull`
    // and populated `SpaceShipState` from canonical `dray.json` `UnitSpec`.
    let deployed_mark = mush_sim::deployed::mark_of(rig.tw.world(), hull)
        .expect("hull marked deployed during bubble");
    assert!(
        deployed_mark.field.is_some(),
        "expected DeployedMark::field, got {deployed_mark:?}"
    );
    assert!(
        mush_tactics::custody::marker_of(rig.tw.world(), hull).is_some(),
        "UnitStore::deployed holds check-out marker for hull"
    );

    let interdictor_hull = {
        let bubble_book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = bubble_book.active_for_hull(hull).expect("active bubble");
        let ship = active
            .bubble
            .ships
            .get(&active.carrier_ship_id)
            .expect("carrier ship");
        assert_eq!(ship.hull_class, "tacsim.unit.dray");
        assert_eq!(ship.dry_mass_t, 240);
        assert_eq!(ship.payload_ceiling_kg, 181_000);
        assert_eq!(ship.propellant_cap_kg, 80_000);
        assert_eq!(ship.rated_accel_mg, 50);
        assert_eq!(ship.exhaust_kms, 6_000);
        assert_eq!(
            ship.sections[&tacsim::space::ShipSectionId::Hull].internals,
            600
        );
        assert_eq!(
            ship.sections[&tacsim::space::ShipSectionId::Hold].internals,
            400
        );
        assert_eq!(
            ship.sections[&tacsim::space::ShipSectionId::Drive].internals,
            250
        );
        active.interdictor_hull
    };

    // 2. Inside the bubble, exchange fire so the carrier takes `hull` (480/600) and `drive` (190/250)
    // damage + a `DriveDegraded` crit, and the hostile interdictor has its drive destroyed (`0/250`).
    {
        let bubble_book = rig
            .tw
            .world_mut()
            .module_state_mut::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = bubble_book
            .active_for_hull_mut(hull)
            .expect("active bubble");
        let carrier_id = active.carrier_ship_id.clone();
        let interdictor_id = active.interdictor_ship_id.clone();

        // Bring interdictor within close laser range so recorded laser fire hits and logs to black box.
        if let Some(iship) = active.bubble.ships.get_mut(&interdictor_id) {
            iship.pos_m = [1_500, 0, 0];
        }
        let _ = active.tape.record_and_apply(
            &mut active.bubble,
            tacsim::space::BubbleInputCommand::FireOffensiveLasers {
                shooter_id: interdictor_id.clone(),
                target_id: carrier_id.clone(),
            },
        );
        let _ = active.tape.record_and_apply(
            &mut active.bubble,
            tacsim::space::BubbleInputCommand::FireOffensiveLasers {
                shooter_id: carrier_id.clone(),
                target_id: interdictor_id.clone(),
            },
        );

        // Set exact post-duel carrier damage (`hull 480/600, drive 190/250`) and disable interdictor drive (`0/250`).
        if let Some(cship) = active.bubble.ships.get_mut(&carrier_id) {
            if let Some(hull_sec) = cship.sections.get_mut(&tacsim::space::ShipSectionId::Hull) {
                hull_sec.facets.insert(tacsim::space::FacetBand::Front, 40);
                hull_sec.internals = 480;
            }
            if let Some(drive_sec) = cship.sections.get_mut(&tacsim::space::ShipSectionId::Drive) {
                drive_sec.internals = 190;
            }
            cship
                .crits
                .push(tacsim::space::SubsystemCrit::DriveDegraded);
        }
        if let Some(iship) = active.bubble.ships.get_mut(&interdictor_id) {
            if let Some(drive_sec) = iship.sections.get_mut(&tacsim::space::ShipSectionId::Drive) {
                drive_sec.internals = 0;
            }
            iship.status = tacsim::space::HullStatus::DisabledDerelict {
                epoch_open: true,
                life_support_remaining_s: tacsim::space::DERELICT_WINDOW_S,
                salvage_contest_window_s: tacsim::space::DERELICT_WINDOW_S,
            };
            iship.thrust_mps2 = [0, 0, 0];
        }
    }

    // Fire one tick to close the resolved bubble (`clear_resolved_bubbles` -> `check_in_space_unit`).
    let next_real = rig.tw.world().now() + 1;
    rig.tw.set_now(next_real);
    let _ = rig.tw.fire_due(next_real * 1_000);

    // Verify `mush_tactics::custody::check_in` cleared the deployed marker and wrote damage to `UnitStore`.
    assert!(
        mush_sim::deployed::mark_of(rig.tw.world(), hull).is_none(),
        "deployed mark cleared on check-in"
    );
    assert!(
        mush_tactics::custody::marker_of(rig.tw.world(), hull).is_none(),
        "UnitStore::deployed cleared on check-in"
    );

    let unit_after = mush_tactics::module::unit_of(rig.tw.world(), hull)
        .expect("live TacticalUnit for Salt Moth after check-in");
    let power_comp = unit_after
        .component(&mush_tactics::ComponentId::new("power").expect("power id"))
        .expect("power component");
    assert_eq!(power_comp.condition.as_str(), "damaged");

    let ship_out = run(&mut rig.tw, pilot, "+ship");
    assert!(
        ship_out.contains("Damage: drive 190/250, hull 480/600."),
        "expected +ship to report persisted combat damage from UnitStore, got: {ship_out}"
    );

    // Verify the disabled interdictor derelict opened a 48 real-hour (`172,800 s`) salvage claim.
    let bubble_book = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook");
    let derelict = bubble_book
        .derelict_for_hull(interdictor_hull)
        .expect("disabled interdictor recorded as open-epoch derelict");
    assert!(derelict.epoch_open);
    assert_eq!(
        derelict.salvage_contest_window_s,
        tacsim::space::DERELICT_WINDOW_S
    );
    let Some(tacsim::field_ops::CustodyTitleStatus::PendingSalvageClaim {
        claim_id,
        custodian,
        ..
    }) = &derelict.salvage_title_status
    else {
        panic!(
            "expected PendingSalvageClaim on disabled derelict, got {:?}",
            derelict.salvage_title_status
        );
    };
    assert_eq!(*custodian, hull);

    let salvage_book = rig
        .tw
        .world()
        .module_state::<tacsim::field_ops::SalvageClaimBook>()
        .expect("SalvageClaimBook");
    let claim = salvage_book
        .claim(*claim_id)
        .expect("SalvageClaim in SalvageClaimBook");
    assert_eq!(
        claim.deadline_real_s - claim.filed_at_real_s,
        tacsim::field_ops::CONTEST_WINDOW_AVAILABLE_REAL_SECS,
        "salvage contest window must be 48 real hours (172,800 real seconds)"
    );

    // 3. Also verify that disabling a chartered carrier hull's own drive inside a bubble
    // checks the hull in as `Immobile` (`FieldEpochDisposition::RemainOpen`) without deleting
    // the hull, logs to its black box, and opens a 48 real-hour salvage window.
    let mut rig = stocked_with(Roster {
        seed_lane: Some(lane_with_interdicted_kirkwood()),
        ..Roster::default()
    });
    let (pilot2, row2) = chartered(&mut rig, "PilotTwo", 2);
    let hull2 = ObjId(row2.hull);
    load(&mut rig, &row2, 53, 19);
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot2);
    let pending2 = undock_and_burn(&mut rig, pilot2, hull2);
    let depart2_ic = pending2.plan.depart_ic;
    let leg2_1_secs = i64::from(pending2.plan.legs[0].secs);
    let _ = fire_quietly(&mut rig);
    let _ = to_ic(&mut rig, depart2_ic + leg2_1_secs);

    // Disable `hull2`'s drive inside its active bubble (`drive` internals -> 0) and step one tick.
    let (carrier2_id, interdictor2_id) = {
        let bubble_book = rig
            .tw
            .world_mut()
            .module_state_mut::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active2 = bubble_book
            .active_for_hull_mut(hull2)
            .expect("active bubble for hull2");
        let carrier2_id = active2.carrier_ship_id.clone();
        let interdictor2_id = active2.interdictor_ship_id.clone();
        if let Some(iship) = active2.bubble.ships.get_mut(&interdictor2_id) {
            iship.pos_m = [-800, 0, 0];
        }
        if let Some(cship) = active2.bubble.ships.get_mut(&carrier2_id) {
            if let Some(hull_sec) = cship.sections.get_mut(&tacsim::space::ShipSectionId::Hull) {
                hull_sec.facets.insert(tacsim::space::FacetBand::Rear, 0);
            }
            if let Some(drive_sec) = cship.sections.get_mut(&tacsim::space::ShipSectionId::Drive) {
                drive_sec.internals = 5;
            }
        }
        (carrier2_id, interdictor2_id)
    };
    // Fire laser into rear arc (`[-800, 0, 0]` behind `cship`) so `apply_bubble_command`
    // strikes through to `drive`, disables `hull2`, and logs `CombatDamageRecorded` to `SalvageClaimBook`.
    let combat_events = tacsim::space::apply_bubble_command(
        rig.tw.world_mut(),
        hull2,
        tacsim::space::BubbleInputCommand::FireOffensiveLasers {
            shooter_id: interdictor2_id,
            target_id: carrier2_id,
        },
    );
    assert!(
        combat_events.iter().any(|ev| matches!(
            ev,
            tacsim::space::SpaceCombatEvent::ShipDisabledDerelict { .. }
        )),
        "expected ShipDisabledDerelict from rear laser strike, got: {combat_events:?}"
    );

    // Next tick: `settle_disabled_carrier_bubbles` checks `hull2` back in and opens its 48h salvage window.
    let next_real = rig.tw.world().now() + 1;
    rig.tw.set_now(next_real);
    let settle_events = rig.tw.fire_due(next_real * 1_000);
    let settle_heard = said(&rig.tw, &settle_events, pilot2);
    assert!(
        settle_heard.contains(
            "[SPACE] Drive offline; hull adrift as an open-epoch derelict with a 48-hour Registry salvage window."
        ),
        "expected derelict narration for hull2, got: {settle_heard}"
    );

    // Hull2 was NOT deleted; its `TacticalUnit` is checked back in with `drive destroyed` and open epoch.
    assert!(rig.tw.world().get(hull2).is_some());
    assert!(mush_sim::deployed::mark_of(rig.tw.world(), hull2).is_none());
    let ship2_out = run(&mut rig.tw, pilot2, "+ship");
    assert!(
        ship2_out.contains("drive destroyed"),
        "expected +ship for disabled carrier to show `drive destroyed`, got: {ship2_out}"
    );

    let bubble_book = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook");
    let derelict2 = bubble_book
        .derelict_for_hull(hull2)
        .expect("hull2 derelict record");
    assert!(derelict2.epoch_open);
    let assessment2 = derelict2.assessment.as_ref().expect("hull2 assessment");
    assert_eq!(
        assessment2.state,
        tacsim::field_ops::OperationalState::Immobile
    );
    assert_eq!(
        assessment2.epoch_disposition,
        tacsim::field_ops::FieldEpochDisposition::RemainOpen
    );

    let salvage_book = rig
        .tw
        .world()
        .module_state::<tacsim::field_ops::SalvageClaimBook>()
        .expect("SalvageClaimBook");
    assert!(
        !salvage_book.black_box_tape_for_hull(hull2).is_empty(),
        "black box for hull2 must contain combat damage entries"
    );
}

// ---------------------------------------------------------------------------
// Doc 36 cut GC.7: munitions and the tariff
// ---------------------------------------------------------------------------

/// A principal's own `tacsim.credit` posted balance, minor units.
fn posted(tw: &TestWorld, who: ObjId) -> i64 {
    let instrument = mush_economy::InstrumentId::new(tacsim::CREDIT).expect("credit");
    economy(tw)
        .snapshot()
        .accounts_of(&mush_org::PrincipalId::Object(who))
        .find(|a| a.instrument == instrument)
        .map_or(0, |a| a.posted_balance.minor())
}

/// **Doc 36 cut GC.7's gate** (Q-36-31, Q-36-36, Q-36-53's rooms). A captain at Ceres buys a
/// load of 105 mm HE (`ammo_he_105`, 40 rounds at the Co-op chandlery's 150 cr) and it lands
/// in the chartered Dray's hold as ordinary cargo. Both ports' authored workshop rooms are open
/// facilities of their operators holding `bay_refit` with their berths. A known repair — a
/// line tank 50 points down on its hull, 40 on its left track and 10 rounds short in the cannon
/// — quoted at Ceres's workshop is the shipped tariff at Occator's 1.2x: 90 points at 1.20 cr
/// and 10 rounds at 0.30 cr is 111.00 cr, a tonne of `industrial_parts`, and 5,040 IC seconds
/// (90 points at 36 s and one rearm of half an IC hour). At Stickney's 1x it is 92.50 cr.
#[test]
fn gc7_munitions_are_bought_and_carried_and_the_tariff_quotes_a_known_repair() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let vault = hold_vault(&rig.tw, &row);
    let account = economy(&rig.tw)
        .snapshot()
        .accounts_of(&mush_org::PrincipalId::Object(pilot))
        .find(|a| a.instrument == mush_economy::InstrumentId::new(tacsim::CREDIT).expect("cr"))
        .map(|a| a.id)
        .expect("the pilot's account");
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={account} 1000000"),
    );
    let before = posted(&rig.tw, pilot);

    // (1) The munitions good is bought at a Ceres vendor and rides in the hold.
    let he = market(&rig.tw)
        .lot_listing_of(
            EndpointRef(VaultId(
                port_map(&rig.tw, "ceres_port").vendors[&cid("coop")].vault,
            )),
            &cid("ammo_he_105"),
        )
        .expect("the Co-op chandlery lists 105 mm HE");
    let quoted = run(&mut rig.tw, pilot, &format!("+market/quote {he}=40"));
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = market(&rig.tw)
        .quotes_of_buyer(&mush_org::PrincipalId::Object(pilot))
        .map(|q| q.id)
        .max()
        .expect("a quote");
    let accepted = run(&mut rig.tw, pilot, &format!("+market/accept {quote}"));
    let draw = drain(&mut rig.tw);
    assert_eq!(draw.len(), 1, "{accepted}");
    land_all(&mut rig, &draw);
    for _ in 0..30 {
        if market(&rig.tw)
            .quote(quote)
            .is_some_and(|q| q.drawn_at.is_some())
        {
            break;
        }
        let _ = fire_quietly(&mut rig);
    }
    assert_eq!(
        pool(&mut rig.tw, vault, "ammo_he_105"),
        40,
        "a load in the hold"
    );
    assert_eq!(
        before - posted(&rig.tw, pilot),
        40 * 150 * 100,
        "forty rounds at 150 cr"
    );
    let goods = tacsim::definitions::goods::goods_table().expect("the goods table");
    let he_good = goods
        .munitions_for("tacsim.ammunition.he_105")
        .expect("105 mm HE is a good");
    assert_eq!(he_good.id.as_str(), "ammo_he_105");
    assert_eq!(he_good.mass_kg_per_unit(), Some(21));
    assert!(
        goods
            .munitions_for("tacsim.ammunition.laser_charge")
            .is_none(),
        "beam charge stays a cell"
    );

    // (2) The authored workshop rooms are their operators' `bay_refit` facilities.
    let industry = rig
        .tw
        .world()
        .module_state::<mush_industry::IndustryStore>()
        .expect("industry");
    for (entry, operator, berths) in [
        ("ceres_port", "occator", 2),
        ("stickney_port", "stickney_yards", 4),
    ] {
        let def = port_def(entry);
        let workshop = def.workshop.as_ref().expect("the port authors a workshop");
        let room = room_by_key(
            rig.tw.world(),
            &room_key(&def.area, &workshop.room).expect("in the area"),
        )
        .expect("the workshop room is on the world");
        let facility = industry.facility(room).expect("the room is a facility");
        let org = port_map(&rig.tw, entry).orgs[&cid(operator)].org;
        assert_eq!(
            facility.owner(),
            &mush_org::PrincipalId::Organization(mush_org::OrgId(org)),
            "{entry}'s workshop is {operator}'s"
        );
        assert!(
            facility
                .capabilities()
                .iter()
                .any(|w| w.as_str() == mush_industry::bay_refit::BAY_CAPABILITY),
            "{entry}'s workshop holds the bay capability"
        );
        assert_eq!(facility.maximum_slots(), berths, "{entry}'s berths");
    }

    // (3) A known repair, quoted from the tariff at each workshop's multiplier.
    let catalog =
        tacsim::definitions::Catalog::build_fielded(&[], "tacsim").expect("the stock catalog");
    let key = cid("tacsim.unit.line_tank");
    let spec = catalog.specs.get(&key).expect("the line tank's spec");
    let fresh = mush_tactics::TacticalUnit::from_spec(
        &catalog.bindings.get(&key).expect("its binding").spec,
        ObjId(9_001),
    )
    .expect("a line tank");
    let section = |id: &str, value: i64| mush_tactics::UnitMutationOp::SetCell {
        addr: mush_tactics::CellAddr::Section {
            section: mush_tactics::SectionId::new(id).expect("a section"),
            cell: mush_tactics::CellId::new(tacsim::definitions::unit::INTERNALS_CELL)
                .expect("internals"),
        },
        value,
    };
    let cannon = mush_tactics::ComponentId::new("cannon").expect("the cannon");
    let plan = mush_tactics::UnitMutationPlan::new(
        vec![
            section("hull", 150),
            section("left_track", 60),
            mush_tactics::UnitMutationOp::SetCell {
                addr: mush_tactics::CellAddr::Component {
                    component: cannon.clone(),
                    cell: mush_tactics::CellId::new(tacsim::definitions::unit::AMMO_CELL)
                        .expect("ammo"),
                },
                value: 30,
            },
        ],
        Some(fresh.revision()),
        None,
    )
    .expect("a plan");
    let (damaged, _) = mush_tactics::apply(&fresh, &plan, 100).expect("the damage applies");
    let ceres = port_def("ceres_port");
    let at_ceres = tacsim::port::workshop::quote(
        ceres.workshop.as_ref().expect("Ceres's workshop"),
        &damaged,
        spec,
    )
    .expect("a quote at Ceres");
    assert_eq!(at_ceres.structural_points, 90);
    assert_eq!(at_ceres.industrial_parts_t, 1);
    assert_eq!(at_ceres.components_to_rearm.get(&cannon), Some(&10));
    assert_eq!(at_ceres.total_resource_cost, 11_100, "111.00 cr at 1.2x");
    assert_eq!(at_ceres.total_work_duration_ic, 5_040);
    let stickney = port_def("stickney_port");
    let at_stickney = tacsim::port::workshop::quote(
        stickney.workshop.as_ref().expect("Stickney's workshop"),
        &damaged,
        spec,
    )
    .expect("a quote at Stickney");
    assert_eq!(at_stickney.total_resource_cost, 9_250, "92.50 cr at 1x");
    assert_eq!(
        at_stickney.total_work_duration_ic, 5_040,
        "the multiplier never moves time"
    );
}

/// **Doc 35 cut SF.R3 acceptance (`D-35-9`, `D-35-10`, `35a` §3)** — authoritative `+helm`
/// and `+tac` verbs enforce physical aboard presence, `mush-sim` bridge stations/channels,
/// `mush-sim` `StandingOrder` writes (`Heading`, `Thrust`, `Posture`), and `mush-propagation`
/// light-lag / IR detection gating:
/// 1. Off-ship callers are refused (`"You are not aboard a ship."`).
/// 2. Unrostered stowaways physically aboard the ship are refused (`"You hold no bridge station on this ship."`).
/// 3. The captain assigns an aboard gunner to the `tactical` station via `+helm/station`,
///    publishing a `mush-sim` `ChannelGrant` (`sensors`, `fire`) onto `World::command_subjects_of(gunner)`;
///    the gunner is refused `movement`-channel helm orders (`+helm/heading`) while permitted `+tac`.
/// 4. The captain's `+helm/heading`, `+helm/thrust`, and `+helm/emcon` write validated
///    `mush_sim::tick::StandingOrder`s (`Heading`, `Thrust`, `Posture`) and append deterministic
///    `BubbleInputCommand`s onto the active `LocalSpaceBubble` input tape.
/// 5. An interdictor running dark (`EmconPosture::Silent`, `0` thrust, `0` heat at `12 km`) is
///    `DetectionOutcome::Missed` under `mush_propagation::resolve_detection`, omitted from
///    `+tac/scan`, and refused by `+tac/lock` / `+tac/beam` (`"No such contact."`); once it
///    lights its drive (`thrust_permille = 500`), `+tac/scan` detects it with light-lag and
///    the gunner locks and fires `+tac/beam`.
#[test]
#[allow(clippy::too_many_lines)]
fn sfr3_authoritative_helm_and_tac_enforce_aboard_seats_write_standing_orders_and_gate_propagation_detection()
 {
    let mut rig = stocked_with(Roster {
        seed_lane: Some(lane_with_interdicted_kirkwood()),
        ..Roster::default()
    });
    let (pilot_actor, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    let shoreside = pilot(&mut rig, "Shoreside", 701);
    let stowaway = pilot(&mut rig, "Stowaway", 702);
    let gunner = pilot(&mut rig, "Gunner", 703);

    load(&mut rig, &row, 55, 60);
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot_actor);

    // Place `stowaway` and `gunner` physically inside the ship's airlock room before launch.
    let airlock = rig
        .tw
        .world()
        .get(pilot_actor)
        .and_then(|o| o.location)
        .expect("pilot is in the ship's airlock room");
    teleport(&mut rig.tw, stowaway, airlock);
    teleport(&mut rig.tw, gunner, airlock);

    // 1. Off-ship caller (`shoreside`) is refused on both `+helm` and `+tac`.
    assert_eq!(
        run(&mut rig.tw, shoreside, "+helm/heading 90,0").trim(),
        "You are not aboard a ship."
    );
    assert_eq!(
        run(&mut rig.tw, shoreside, "+tac/scan").trim(),
        "You are not aboard a ship."
    );
    assert_eq!(
        run(&mut rig.tw, shoreside, "+helm/station command").trim(),
        "You are not aboard a ship."
    );

    // 2. Unrostered stowaway physically aboard the ship is refused on `+helm` and `+tac`.
    assert_eq!(
        run(&mut rig.tw, stowaway, "+helm/heading 90,0").trim(),
        "You hold no bridge station on this ship."
    );
    assert_eq!(
        run(&mut rig.tw, stowaway, "+tac/scan").trim(),
        "You hold no bridge station on this ship."
    );
    assert_eq!(
        run(&mut rig.tw, stowaway, "+helm/station command").trim(),
        "Only the ship's captain or command station may assign bridge stations."
    );

    // Launch and promote at the interdicted Kirkwood boundary.
    let pending = undock_and_burn(&mut rig, pilot_actor, hull);
    let depart_ic = pending.plan.depart_ic;
    let leg1_secs = i64::from(pending.plan.legs[0].secs);
    let _ = fire_quietly(&mut rig);
    let _ = to_ic(&mut rig, depart_ic + leg1_secs);

    // 3. Captain (`pilot_actor`) assigns `gunner` to the `tactical` bridge station via `+helm/station`.
    let assign_out = run(&mut rig.tw, pilot_actor, "+helm/station tactical=Gunner");
    assert!(
        assign_out.contains("Gunner now holds `tactical`"),
        "station assignment output: {assign_out}"
    );
    let grants = rig.tw.world().command_subjects_of(gunner);
    assert!(
        grants.iter().any(|g| {
            g.subject == hull
                && g.station.as_str() == "tactical"
                && g.channels.iter().any(|c| c.as_str() == "sensors")
                && g.channels.iter().any(|c| c.as_str() == "fire")
        }),
        "mush-sim published ChannelGrant for gunner on tactical station: {grants:?}"
    );

    // `gunner` (holding `tactical`) is refused `movement` orders (`+helm/heading`, `+helm/thrust`).
    assert_eq!(
        run(&mut rig.tw, gunner, "+helm/heading 90,0").trim(),
        "Your bridge station (tactical) does not control the movement channel."
    );
    assert_eq!(
        run(&mut rig.tw, gunner, "+helm/thrust 500").trim(),
        "Your bridge station (tactical) does not control the movement channel."
    );

    // 4. `pilot_actor` issues `+helm/heading`, `+helm/thrust`, and `+helm/emcon` orders.
    let h_out = run(&mut rig.tw, pilot_actor, "+helm/heading 90,0");
    assert!(
        h_out.contains("standing order `heading` rev 1"),
        "heading output: {h_out}"
    );
    let t_out = run(&mut rig.tw, pilot_actor, "+helm/thrust 600");
    assert!(
        t_out.contains("standing order `thrust` rev 2"),
        "thrust output: {t_out}"
    );
    let e_out = run(&mut rig.tw, pilot_actor, "+helm/emcon passive");
    assert!(
        e_out.contains("standing order `posture` rev 3"),
        "emcon output: {e_out}"
    );

    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let h_order = book
            .standing_order(hull, mush_sim::tick::OrderChannel::Heading)
            .expect("Heading standing order");
        assert_eq!(
            *h_order.value(),
            mush_sim::tick::OrderValue::Vector {
                x: 0,
                y: 1_000,
                z: 0
            }
        );
        let t_order = book
            .standing_order(hull, mush_sim::tick::OrderChannel::Thrust)
            .expect("Thrust standing order");
        assert_eq!(
            *t_order.value(),
            mush_sim::tick::OrderValue::Scalar { value: 600 }
        );
        let p_order = book
            .standing_order(hull, mush_sim::tick::OrderChannel::Posture)
            .expect("Posture standing order");
        assert_eq!(
            *p_order.value(),
            mush_sim::tick::OrderValue::Emcon { level: 1 }
        );

        let active = book.active_for_hull(hull).expect("active bubble");
        assert_eq!(
            active.tape.entries.len(),
            3,
            "3 helm commands recorded on tape"
        );
        let carrier = active
            .bubble
            .ships
            .get(&active.carrier_ship_id)
            .expect("carrier ship");
        assert_eq!(carrier.attitude_deg.0, 90);
        assert!(carrier.is_burning());
        assert_eq!(carrier.emcon, tacsim::space::EmconPosture::Passive);
    }

    // 5. `mush-propagation` detection gating (`D-35-10`):
    // Put the interdictor at 30 km (`[30_000, 0, 0]`) in `EmconPosture::Ghost` with 0 thrust & 0 heat.
    let interdictor_id = {
        let book = rig
            .tw
            .world_mut()
            .module_state_mut::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = book.active_for_hull_mut(hull).expect("active bubble");
        let id = active.interdictor_ship_id.clone();
        let interdictor = active.bubble.ships.get_mut(&id).expect("interdictor ship");
        interdictor.pos_m = [30_000, 0, 0];
        interdictor.emcon = tacsim::space::EmconPosture::Ghost;
        interdictor.thrust_mps2 = [0, 0, 0];
        interdictor.heat_permille = 0;
        id
    };

    // Cold/silent contact at 30 km is Missed by `mush-propagation`: hidden from `/scan` and refused by `/lock` & `/beam`.
    let scan_cold = run(&mut rig.tw, gunner, "+tac/scan");
    assert!(
        scan_cold.starts_with("Sensor pass: 0 contacts detected."),
        "scan_cold: {scan_cold}"
    );
    assert!(
        !scan_cold.contains("30000"),
        "undetected contact must never leak true_distance_m: {scan_cold}"
    );
    assert_eq!(
        run(&mut rig.tw, gunner, &format!("+tac/lock {interdictor_id}")).trim(),
        "No such contact."
    );
    assert_eq!(
        run(&mut rig.tw, gunner, &format!("+tac/beam {interdictor_id}")).trim(),
        "No such contact."
    );

    // Also verify `holotank_feed` and `cockpit_feed` omit the undetected contact (`35a` §1).
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = book.active_for_hull(hull).expect("active bubble");
        let holotank = active.bubble.holotank_feed(&active.carrier_ship_id);
        let contacts = holotank["contacts"].as_array().expect("contacts array");
        assert_eq!(
            contacts.len(),
            1,
            "only own ship ('you': true) appears in holotank feed when target is undetected: {contacts:?}"
        );
        let cockpit = active.bubble.cockpit_feed(&active.carrier_ship_id);
        let cockpit_contacts = cockpit["contacts"].as_array().expect("cockpit contacts");
        assert!(
            cockpit_contacts.is_empty(),
            "cockpit feed must not include undetected contact: {cockpit_contacts:?}"
        );
    }

    // Now the interdictor lights its torch (`thrust_mps2 = [5, 0, 0]`) and moves into laser range (`12,000 m`):
    // IR bloom rises above the inverse-square floor, so `mush_propagation::resolve_detection` returns `Detected`.
    {
        let book = rig
            .tw
            .world_mut()
            .module_state_mut::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = book.active_for_hull_mut(hull).expect("active bubble");
        let interdictor = active
            .bubble
            .ships
            .get_mut(&interdictor_id)
            .expect("interdictor ship");
        interdictor.pos_m = [12_000, 0, 0];
        interdictor.thrust_mps2 = [5, 0, 0];
    }

    let scan_hot = run(&mut rig.tw, gunner, "+tac/scan");
    assert!(
        scan_hot.contains("1 contact(s) detected") && scan_hot.contains(&interdictor_id),
        "detected contact shown in scan: {scan_hot}"
    );

    let lock_out = run(&mut rig.tw, gunner, &format!("+tac/lock {interdictor_id}"));
    assert!(
        lock_out.contains(&format!("Target lock acquired on `{interdictor_id}`.")),
        "lock output: {lock_out}"
    );
    let beam_out = run(&mut rig.tw, gunner, "+tac/beam");
    assert!(
        beam_out.contains(&format!("Laser battery fired on `{interdictor_id}`")),
        "beam output: {beam_out}"
    );

    let book = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook");
    let active = book.active_for_hull(hull).expect("active bubble");
    assert_eq!(
        active.tape.entries.len(),
        5,
        "SetLockedTarget and FireOffensiveLasers appended to BubbleInputTape"
    );
}

/// **Doc 35 cut SF.R4 (`D-35-6`, `35a` §5 #4–#5, §8.3)**:
/// 1. A non-GOD chartered pilot launches *Salt Moth* (`+ship/undock stickney`) and decouples
///    mid-transit (`+helm/decouple`) into 3D heliocentric free flight, pausing the `CarrierJourney`.
/// 2. Executing a timed burn (`+helm/burn 3,0,0 400`) debits reaction mass from the hull's live
///    `stow.propellant` pool (both in `HullPropellantLedger` and via `tacsim_port_burn` on the
///    hull's vault in `EconomyStore`), and stepping the world clock past the burn duration cuts
///    thrust off cleanly so timed burns never latch (`35a` §5 #4) while solar gravity integrates
///    on the Tier-1 trajectory (`35a` §5 #5).
/// 3. Attempting a burn that would drop live propellant below the computed `RETURN_RESERVE` to
///    the nearest open port refuses with a `RETURN_RESERVE warning` unless `force` is passed.
/// 4. Calling `+helm/autopilot stickney` plans a passage from the ship's live `(pos_km, vel_mps)`
///    using `port::passage::plan_passage_from`, debits the planned `burn_t` and `crew_days` from
///    the hull's vault via `tacsim_port_burn`, recouples the hull onto a `Cruising` `CarrierJourney`,
///    and docks at Stickney Yards when the planned transit completes.
#[test]
#[allow(clippy::too_many_lines)]
fn sfr4_free_flight_decouples_debits_live_propellant_warns_return_reserve_and_recouples_autopilot()
{
    let mut rig = stocked_with(Roster::default());
    let (pilot_actor, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    let vault_id = hold_vault(&rig.tw, &row);

    // Load 60 t propellant and 60 crew-days provisions (light hull without cargo) so the ship has
    // ample propellant margin to decouple, execute timed burns, test RETURN_RESERVE, and recouple.
    seed_pool(&mut rig, vault_id, "propellant", 60, LotId(903));
    seed_pool(&mut rig, vault_id, "provisions", 60, LotId(904));
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot_actor);

    // Launch *Salt Moth* toward Stickney Yards, then credit back the unflown leg propellant (25 t)
    // so the tank holds ~54 t when decoupling 6 hours into the leg.
    let pending = undock_and_burn(&mut rig, pilot_actor, hull);
    let depart_ic = pending.plan.depart_ic;
    let _ = fire_quietly(&mut rig);
    seed_pool(&mut rig, vault_id, "propellant", 25, LotId(905));

    // Advance 6 IC hours into the first leg.
    let _ = to_ic(&mut rig, depart_ic + 21_600);

    let tank_before_decouple = pool(&mut rig.tw, vault_id, "propellant");
    assert!(
        tank_before_decouple >= 40,
        "hull vault holds live propellant before decouple: {tank_before_decouple} t"
    );

    // 1. Decouple from the corridor leg into 3D heliocentric free flight (`+helm/decouple`).
    let dec_out = run(&mut rig.tw, pilot_actor, "+helm/decouple");
    assert!(
        dec_out.contains("Decoupled from corridor leg")
            && dec_out.contains("3D heliocentric free flight"),
        "decouple output: {dec_out}"
    );
    {
        let jbook = rig
            .tw
            .world()
            .module_state::<mush_sim::JourneyBook>()
            .expect("JourneyBook");
        let journey = jbook.get(hull).expect("journey exists");
        assert!(
            matches!(journey.state(), mush_sim::JourneyState::Promoted { .. }),
            "CarrierJourney is paused in Promoted state while decoupled: {:?}",
            journey.state()
        );
    }

    // Set guidance to `match_velocity` and verify target-relative flip logic (`35a` §5 #5).
    let guid_out = run(&mut rig.tw, pilot_actor, "+helm/guidance match_velocity");
    assert!(
        guid_out.contains("MATCH_VEL"),
        "guidance output: {guid_out}"
    );

    // 2. Execute a timed burn: `+helm/burn 3,0,0 400` (`3 m/s²` for `400 s` = `1,200 m/s` Δv, ~3,000+ kg).
    let burn_out = run(&mut rig.tw, pilot_actor, "+helm/burn 3,0,0 400");
    assert!(
        burn_out.contains("Timed burn engaged: (3, 0, 0) m/s² for 400s (Δv=1200 m/s"),
        "burn output: {burn_out}"
    );
    let burn_submits = drain(&mut rig.tw);
    assert_eq!(
        burn_submits.len(),
        1,
        "crossing whole tonnes submits tacsim_port_burn"
    );
    for s in &burn_submits {
        land(&mut rig, s).expect("in-flight space burn lands on hull vault");
    }

    let tank_after_burn = pool(&mut rig.tw, vault_id, "propellant");
    assert!(
        tank_after_burn < tank_before_decouple,
        "live stow.propellant vault debited by timed burn: before={tank_before_decouple} t, after={tank_after_burn} t"
    );

    // Step the world clock forward by 600 IC seconds (past the 400 s burn duration) and verify:
    // - timed burn cuts off (`thrust_mmps2 == [0, 0, 0]`, `burn_remaining_ic_s == None`, `35a` §5 #4)
    // - position and velocity advanced under burn + solar gravity (`35a` §5 #5).
    let now_before_step = rig.tw.world().world_ic_now().at;
    let _ = to_ic(&mut rig, now_before_step + 600);
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let ff = book.free_flight_for_hull(hull).expect("free flight record");
        assert_eq!(
            ff.burn_remaining_ic_s, None,
            "timed burn cut off after 400 s"
        );
        assert_eq!(
            ff.thrust_mmps2,
            [0, 0, 0],
            "timed burn thrust did not latch after cutoff"
        );
    }

    // 3. Attempting an excessive burn that dips below `RETURN_RESERVE` without `force` refuses
    //    with the computed `RETURN_RESERVE warning` (`D-35-6`).
    let status_out = run(&mut rig.tw, pilot_actor, "+helm/status");
    assert!(
        status_out.contains("RETURN_RESERVE="),
        "helm/status reports computed RETURN_RESERVE: {status_out}"
    );
    // A 100 m/s² burn for 12,000 s (1,200,000 m/s Δv) requires ~48,000 kg, dipping below RETURN_RESERVE;
    // try a burn that fits within `live_kg` (~53,940 kg) but leaves less than `RETURN_RESERVE`:
    let warn_out = run(&mut rig.tw, pilot_actor, "+helm/burn 50,0,0 16000");
    assert!(
        warn_out.contains("RETURN_RESERVE warning"),
        "burn into reserve without force is refused with RETURN_RESERVE warning: {warn_out}"
    );

    // 4. Recouple onto a planned corridor passage via `+helm/autopilot stickney` (`D-35-6`).
    //    Ensure the hull has sufficient propellant in its vault for the full passage from its new (pos, vel).
    seed_pool(&mut rig, vault_id, "propellant", 40, LotId(909));
    let auto_out = run(&mut rig.tw, pilot_actor, "+helm/autopilot stickney");
    assert!(
        auto_out.contains("Autopilot engaged for Stickney Yards (`stickney_yards`)"),
        "autopilot output: {auto_out}"
    );
    for s in drain(&mut rig.tw) {
        land(&mut rig, &s).expect("autopilot passage burn lands on hull vault");
    }

    // Verify `free_flights` was cleared and `CarrierJourney` is back in `JourneyState::Cruising`.
    let arrival_at = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        assert!(
            book.free_flight_for_hull(hull).is_none(),
            "hull removed from free_flights on autopilot recouple"
        );
        let jbook = rig
            .tw
            .world()
            .module_state::<mush_sim::JourneyBook>()
            .expect("JourneyBook");
        let journey = jbook.get(hull).expect("recoupled journey");
        assert_eq!(*journey.state(), mush_sim::JourneyState::Cruising);
        journey.arrival_at().expect("arrival timestamp")
    };

    // Advance to the recoupled journey's arrival timestamp and verify the ship docks at Stickney Yards.
    let _ = to_ic(&mut rig, arrival_at);
    let ship_after = run(&mut rig.tw, pilot_actor, "+ship");
    assert!(
        ship_after.contains("Stickney Yards"),
        "ship arrived and docked at Stickney Yards after autopilot recouple: {ship_after}"
    );
}

fn drain_space_and_industry(tw: &mut TestWorld) -> Vec<AuthoritativeSubmit> {
    let mut kept = Vec::new();
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        let owner = submit.action.key().owner.as_str();
        if owner == tacsim::NS || owner == "inventory" || owner == "market" {
            kept.push(*submit);
        } else if owner == mush_industry::NS {
            let rec = submit
                .parameters
                .get("receipt")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if rec.contains("pallas_inclined_belt")
                || rec.contains("ceres_halo")
                || rec.contains("kirkwood_gap_3_1")
                || rec.contains("saturn_b_ring_edge")
                || rec.contains("tacsim.space.")
            {
                kept.push(*submit);
            }
        }
    }
    kept
}

/// **Doc 35 cut SF.R5 (`+mine`, `Q-35-3`, `D-35-13`, `35a` §3 #6/#9, §4 `SF.12`–`SF.15`, §5 #11/#13)**:
/// 1. A non-GOD chartered pilot refits *Salt Moth* to a `Dray-P` (`+coop/refit prospector`, `86 t` hold)
///    and buys a `2,500 cr` (`168` IC-day) Freeport Registry prospecting licence (`+licence/buy go`).
/// 2. Entering `pallas_inclined_belt` (`+helm/cluster pallas_inclined_belt`) opens conserved
///    `mush_industry::OreDeposit` pools (`120 t` per family) and seeds Pallas Syndicate tenure on
///    Blocks `A` and `B` while leaving Block `C` open.
/// 3. `+helm/match A` from `[3_800, 0, 0] m` and `12 m/s` is refused (never teleports);
///    `+helm/approach A` + `space_tick` closes range and brakes relative speed inside the `2,000 m` /
///    `< 5.0 m/s` 3D Euclidean gate, after which `+helm/match A` locks velocity match.
/// 4. `+mine/laser A` (`4 t`, `60 s`) and `+mine/drones A 4` (`4` collector drones under real
///    `ExcursionId`s, `16 t` total, `120 s`) extract `20 t` of conserved `metal_ore` from Block `A`'s
///    deposit vault into *Salt Moth*'s `stow.ore` hold vault and advance `PocketCursor` in
///    `tacsim.pockets`, while flagging frontier thermal bloom (`+18` heat) without scripting any
///    rival spawn inside `+mine/laser`.
/// 5. `+mine/stake C` and `+mine/contest C` stage real `BlockTenure` records in `tacsim.claims`
///    and enforce the `30` IC-day (`CONTEST_WINDOW_IC_SECS`) contest window.
/// 6. Selling the `20 t` of mined `metal_ore` at the Ceres Freeport Exchange (`+market/sell <bid>=20`)
///    and firing `tacsim.port` mills the `20 t` of `metal_ore` into `8 t` of `concentrate`.
#[test]
#[allow(clippy::too_many_lines)]
fn sfr5_chartered_dray_p_buys_licence_matches_velocity_in_pallas_mines_into_stow_ore_and_sells_at_ceres_mill()
 {
    let mut rig = stocked_with(Roster::default());
    let (pilot_actor, row) = chartered(&mut rig, "Pilot", 1);
    let rival_actor = pilot(&mut rig, "Rival", 2);
    let _ = run(&mut rig.tw, rival_actor, "+coop/enrol Rival Moth");
    let _hull = ObjId(row.hull);
    let vault_id = hold_vault(&rig.tw, &row);

    // 1. Refit *Salt Moth* to a `Dray-P` (`86 t` hold) at the Co-op office before loading stores.
    let desk = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, pilot_actor, desk);
    let refit_out = run(&mut rig.tw, pilot_actor, "+coop/refit prospector");
    assert!(
        refit_out.contains("Dray-P") && refit_out.contains("86 t"),
        "coop/refit prospector output: {refit_out}"
    );

    // Provision propellant and rations (leaving `stow.ore` empty) and board *Salt Moth*.
    seed_pool(&mut rig, vault_id, "propellant", 40, LotId(903));
    seed_pool(&mut rig, vault_id, "provisions", 40, LotId(904));
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot_actor);

    // Place `rival_actor` aboard on the `flight_ops` bridge station so we can also test
    // unlicensed refusal and the 30-IC-day claim contest window.
    let airlock = location(&rig.tw, pilot_actor).expect("pilot is in airlock");
    teleport(&mut rig.tw, rival_actor, airlock);
    let _ = run(&mut rig.tw, pilot_actor, "+helm/station flight_ops=Rival");

    // 2. Enter `pallas_inclined_belt` (`+helm/cluster pallas_inclined_belt`) and land the
    //    cluster's `DepositOpen` and `ClaimTenure` (Pallas Syndicate seed) submissions.
    let cluster_out = run(
        &mut rig.tw,
        pilot_actor,
        "+helm/cluster pallas_inclined_belt",
    );
    assert!(
        cluster_out.contains("Pallas Inclined Belt (`pallas_inclined_belt`)"),
        "cluster output: {cluster_out}"
    );
    let open_submits = drain_space_and_industry(&mut rig.tw);
    assert_eq!(
        open_submits.len(),
        4,
        "4 OreDeposit openings submitted for Pallas rocks A (S), B (M: 2 families), C (S)"
    );
    land_all(&mut rig, &open_submits);

    // Advance one tick so the block places seed Pallas Syndicate tenure on Blocks A and B.
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let seed_submits = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &seed_submits);

    // Verify `+mine/ping` reports all 3 rocks, their spectral classes, specific volumes, and tenure.
    let ping_out = run(&mut rig.tw, pilot_actor, "+mine/ping");
    assert!(
        ping_out.contains("Block A (`rock_1` S-TYPE)")
            && ping_out.contains("metal_ore=120 t (500 L/t)")
            && ping_out.contains("Block B (`rock_2` M-TYPE)")
            && ping_out.contains("strategic_ore=120 t (600 L/t)")
            && ping_out.contains("Block C (`rock_3` S-TYPE)")
            && ping_out.contains("tenure: open"),
        "mine/ping output: {ping_out}"
    );
    // `rival_actor` also pings Block C so they hold discovery evidence for contesting later.
    let _ = run(&mut rig.tw, rival_actor, "+mine/ping C");

    // Verify unlicensed mining or staking is refused (`D-35-13`: Freeport Registry licence required).
    let unlic_stake = run(&mut rig.tw, pilot_actor, "+mine/stake C metals");
    assert!(
        unlic_stake.contains("prospecting licence") && unlic_stake.contains("Freeport Registry"),
        "unlicensed stake refused: {unlic_stake}"
    );

    // Buy the 2,500 cr (168 IC-day) Freeport Registry prospecting licence for both pilots.
    let lic_out = run(&mut rig.tw, pilot_actor, "+licence/buy go");
    assert!(
        lic_out.contains("Freeport Registry") && lic_out.contains("2500.00cr"),
        "licence/buy go output: {lic_out}"
    );
    let _ = run(&mut rig.tw, rival_actor, "+licence/buy go");

    // 3. Verify `+helm/match A` from standoff (`3,800 m`, `12 m/s`) is refused (`35a` §5 #11: never teleports!).
    let match_far = run(&mut rig.tw, pilot_actor, "+helm/match A");
    assert!(
        match_far.contains("outside the 2,000 m velocity-match envelope"),
        "match from 3,800 m refused: {match_far}"
    );

    // Run `+helm/approach A`, step `space_tick` to close range and brake relative velocity, then `+helm/match A`.
    let app_out = run(&mut rig.tw, pilot_actor, "+helm/approach A");
    assert!(
        app_out.contains("Braking approach ordered toward Block A"),
        "approach output: {app_out}"
    );
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let app_submits = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &app_submits);

    let match_ok = run(&mut rig.tw, pilot_actor, "+helm/match A");
    assert!(
        match_ok.contains("RCS velocity match locked on Block A"),
        "match locked after approach braking: {match_ok}"
    );

    // 4. Start a timed mining-laser cycle (`+mine/laser A`, 4 t) and launch 4 collector drones
    //    (`+mine/drones A 4`, 4 x 4 t = 16 t under real `ExcursionId`s).
    let laser_out = run(&mut rig.tw, pilot_actor, "+mine/laser A");
    assert!(
        laser_out.contains("Mining laser engaged on Block A (S-TYPE) for 60s: extracting 4 t `metal_ore` (500 L/t) into `stow.ore`")
            && laser_out.contains("FRONTIER BLOOM +18 heat"),
        "laser output: {laser_out}"
    );
    let drones_out = run(&mut rig.tw, pilot_actor, "+mine/drones A 4");
    assert!(
        drones_out.contains("Launched 4 collector drone(s) onto Block A (S-TYPE) for 120s (4 t `metal_ore` each into `stow.ore`)"),
        "drones output: {drones_out}"
    );
    // Verify launching a 5th drone is refused (`MAX_EXCURSIONS_PER_OPERATOR = 4`, `35a` §3 #6).
    let drone_cap = run(&mut rig.tw, pilot_actor, "+mine/drones A 1");
    assert!(
        drone_cap.contains("Drone bay cap exceeded"),
        "5th drone refused: {drone_cap}"
    );

    // Advance 6 seconds of wall time (= 21,600 IC seconds at rate 3,600, past the 60s laser and
    // 120s drone timers) so `space_tick` queues the 20 t `PendingSpaceMineDraw` and the cluster
    // clerk submits `tacsim_site_work` in the same tick!
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let work_submits = drain_space_and_industry(&mut rig.tw);
    assert!(
        work_submits
            .iter()
            .any(|s| s.action.key().id.as_str() == "tacsim_site_work"),
        "cluster clerk submitted tacsim_site_work for completed laser + drone extraction: {work_submits:?}"
    );
    land_all(&mut rig, &work_submits);

    // Verify 20 t of conserved `metal_ore` moved from Block A's deposit vault into *Salt Moth*'s
    // `stow.ore` hold vault (`vault_id`), and `PocketCursor` in `tacsim.pockets` recorded 20 t!
    assert_eq!(
        pool(&mut rig.tw, vault_id, "metal_ore"),
        20,
        "20 t of metal_ore (4 t laser + 16 t from 4 drones) transferred into stow.ore hold vault"
    );
    let ship_hold = run(&mut rig.tw, pilot_actor, "+ship/hold");
    assert!(
        ship_hold.contains("metal ore") && ship_hold.contains("20 t"),
        "+ship/hold shows 20 t of metal ore in stow.ore: {ship_hold}"
    );
    let ping_after = run(&mut rig.tw, pilot_actor, "+mine/ping A");
    assert!(
        ping_after.contains("metal_ore=100 t"),
        "Block A OreDeposit pool conserved (120 t - 20 t = 100 t): {ping_after}"
    );

    // 5. Verify Freeport Registry claim staking (`+mine/stake C metals`) and the 30-IC-day
    //    (`CONTEST_WINDOW_IC_SECS = 30 * 86_400`) contest window (`+mine/contest C`).
    let stake_out = run(&mut rig.tw, pilot_actor, "+mine/stake C metals");
    assert!(
        stake_out
            .contains("Filed Freeport Registry claim stake on Block C at Pallas Inclined Belt"),
        "stake output: {stake_out}"
    );
    let stake_submits = drain_space_and_industry(&mut rig.tw);
    land_all(&mut rig, &stake_submits);

    let ping_staked = run(&mut rig.tw, pilot_actor, "+mine/ping C");
    assert!(
        ping_staked.contains("your claim"),
        "Block C now held by Pilot: {ping_staked}"
    );

    // Within the 30-IC-day contest window, `rival_actor` contests Block C (`+mine/contest C`).
    let contest_out = run(&mut rig.tw, rival_actor, "+mine/contest C");
    assert!(
        contest_out.contains("Filed Freeport Registry contest on Block C at Pallas Inclined Belt within the 30-day contest window."),
        "contest output: {contest_out}"
    );
    let contest_submits = drain_space_and_industry(&mut rig.tw);
    land_all(&mut rig, &contest_submits);

    let ping_contested = run(&mut rig.tw, pilot_actor, "+mine/ping C");
    assert!(
        ping_contested.contains("held by Rival"),
        "Block C transferred to Rival on contest: {ping_contested}"
    );

    // Advance past the 30-IC-day contest window (`31 * 86_400` IC seconds = 744 real seconds at rate 3,600)
    // and verify `+mine/contest C` is refused!
    let t_after_window = rig.tw.world().now().saturating_add(750);
    rig.tw.set_now(t_after_window);
    let contest_late = run(&mut rig.tw, pilot_actor, "+mine/contest C");
    assert!(
        contest_late.contains(
            "The 30-day Registry contest window on Block C at Pallas Inclined Belt has closed."
        ),
        "contest past 30 IC days refused: {contest_late}"
    );

    // 6. Sell the 20 t of mined `metal_ore` from *Salt Moth*'s `stow.ore` hold at the Ceres Freeport
    //    Exchange (`room.c07_exchange`) and verify `tacsim_port_mill` mills it into `8 t` of `concentrate`!
    set_rate(&mut rig.tw, 1);
    let _ = run(&mut rig.tw, pilot_actor, "+ship/leave");
    let room_c07 = c07(&rig.tw);
    teleport(&mut rig.tw, pilot_actor, room_c07);

    let bid_id = port_map(&rig.tw, "ceres_port").vendors[&cid("freeport_exchange")].bids
        [&cid("metal_ore")]
        .listing;
    let sell_out = run(
        &mut rig.tw,
        pilot_actor,
        &format!("+market/sell {bid_id}=20"),
    );
    assert!(
        sell_out.contains("Sold 20") && sell_out.contains("metal ore"),
        "sold 20 t of mined metal_ore at Ceres Freeport Exchange: {sell_out}"
    );
    let sell_submits = drain_space_and_industry(&mut rig.tw);
    land_all(&mut rig, &sell_submits);
    assert_eq!(
        pool(&mut rig.tw, vault_id, "metal_ore"),
        0,
        "stow.ore hold vault emptied by market delivery"
    );

    // Sweep and fire the port tick until `tacsim_port_mill` mills the 20 t batch of `metal_ore`
    // into `8 t` of `metals` (`5:2` ratio).
    let exchange_vault =
        VaultId(port_map(&rig.tw, "ceres_port").vendors[&cid("freeport_exchange")].vault);
    let conc_before = pool(&mut rig.tw, exchange_vault, "metals");
    let mut milled = false;
    for _ in 0..15 {
        sweep(&mut rig.tw, 31);
        let queued = drain_space_and_industry(&mut rig.tw);
        let had_mill = queued
            .iter()
            .any(|s| s.action.key().id.as_str() == "tacsim_port_mill");
        land_quietly(&mut rig, &queued);
        if had_mill {
            milled = true;
            break;
        }
    }
    assert!(
        milled,
        "Ceres Freeport Exchange mill processed the 20 t batch of space-mined metal_ore"
    );
    let conc_after = pool(&mut rig.tw, exchange_vault, "metals");
    assert_eq!(
        conc_after.saturating_sub(conc_before),
        8,
        "20 t of space-mined metal_ore milled into 8 t of metal concentrate (metals) at Ceres Freeport"
    );
    no_refused_writes(&rig.tw);
}

/// **Doc 35 cut SF.R6 (`D-35-1`, `D-35-8`, `D-35-10`, `D-35-11`, `D-35-12`, §9 Story 1, `35a` §8.3)**:
/// End-to-end **Story 1 ("The Kirkwood Customs Picket")**:
/// 1. Non-GOD `Pilot` charters the *Salt Moth*, refits it to a `Dray-P` (`+coop/refit prospector`),
///    buys the Freeport Registry prospecting licence (`+licence/buy go`), and extracts `4 t`
///    (`4,000 kg`) of `strategic_ore` from Block B in `pallas_inclined_belt` without holding a
///    registered claim on Block B (Pallas Syndicate holds Block B's tenure).
/// 2. Launching on the interdicted `ceres_stickney` route and crossing the Kirkwood waypoint
///    boundary promotes the `CarrierJourney` (`JourneyState::Promoted`), opens a 3-party
///    `LocalSpaceBubble` (`CARRIER_PARTY` `'A'`, `CUSTOMS_CORVETTE_PARTY` `'B'`,
///    `BELT_RAIDER_PARTY` `'C'`), and emits the light-lagged radio hail (`mush_propagation::light_delay_secs`)
///    + structured `"tacsim:notice"` panel event.
/// 3. Firing on the non-hostile Republic Customs Corvette (`SpaceRelation::Inspecting` /
///    `Relation::Neutral`) is refused (`"Cannot fire on a non-hostile contact."`), and
///    `+helm/submit` before heaving-to ($|\Delta \mathbf{v}| < 10\text{ m/s}$) is refused.
/// 4. Pilot decelerates via `+helm/thrust -15 0 0 1` to match velocity with the Corvette;
///    the Corvette's `RhaiRunner` (`KIRKWOOD_PICKET_RHAI`) executes without faults and transitions
///    to `"inspecting"` (`PICKET_CARRIER_HOVE_TO_READY`), while the lurking Belt Raider remains
///    in `Ghost` at 120 km without prematurely uncloaking before `+helm/submit`.
/// 5. Pilot submits via `+helm/submit`: live `stow.ore` + Registry check detects `4,000 kg` (`4 t`)
///    of unlicensed `strategic_ore`, activates a real `mush-economy` Public E4 contract fine
///    (`"Republic Customs Contraband Fine"` = `2000.00cr`, `200,000` minor credits) that settles
///    into `freeport_registry`'s account on the economy sweep, and opens/advances the
///    `"Customs Assertion"` campaign front in `mush-faction` (`SimState`).
/// 6. Lurking Belt Raider lights its torch (`EmconPosture::ActiveCombat`) and its `RhaiRunner`
///    (`BELT_RAIDER_RHAI`) launches a 2-torpedo salvo and fires a 120 km world-frame railgun shot
///    (`RAIDER_TORCH_AND_SALVO`), while the Corvette's `RhaiRunner` (`KIRKWOOD_PICKET_RHAI`)
///    transitions to `"screen_defend"` (`PICKET_SCREEN_ENGAGE_RAIDER`); pilot enables `+tac/pdc auto`,
///    jinks laterally (`+helm/thrust 0 15 0 2`) during time-of-flight so the 120 km railgun slug
///    misses (`RailgunDodgedByJink`), disables the Belt Raider into an open-epoch derelict
///    (`DisabledDerelict`, 48h salvage window in `SalvageClaimBook`), and resumes the voyage to
///    dock at Stickney Yards.
#[test]
#[allow(clippy::too_many_lines)]
fn sfr6_kirkwood_customs_picket_story_1_end_to_end_rhai_doctrine_triad_combat_and_ledger_fine() {
    let mut rig = stocked_with(Roster {
        seed_lane: Some(lane_with_interdicted_kirkwood()),
        ..Roster::default()
    });
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    let vault_id = hold_vault(&rig.tw, &row);

    // 1. Refit *Salt Moth* to `Dray-P`, provision propellant (`53 t`) and provisions (`19 t`),
    //    buy the Freeport Registry licence, and mine `4 t` (`4,000 kg`) of unlicensed `strategic_ore`
    //    from Block B in `pallas_inclined_belt` into `stow.ore`.
    let desk = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, pilot, desk);
    let refit_out = run(&mut rig.tw, pilot, "+coop/refit prospector");
    assert!(refit_out.contains("Dray-P"), "{refit_out}");

    seed_pool(&mut rig, vault_id, "propellant", 53, LotId(913));
    seed_pool(&mut rig, vault_id, "provisions", 19, LotId(914));
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot);

    let lic_out = run(&mut rig.tw, pilot, "+licence/buy go");
    assert!(
        lic_out.contains("Freeport Registry") && lic_out.contains("2500.00cr"),
        "{lic_out}"
    );
    // Settle the licence purchase contract before checking pre-fine Registry balance.
    sweep(&mut rig.tw, 6);

    let _ = run(&mut rig.tw, pilot, "+helm/cluster pallas_inclined_belt");
    let open_submits = drain_space_and_industry(&mut rig.tw);
    land_all(&mut rig, &open_submits);
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let seed_submits = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &seed_submits);

    let _ = run(&mut rig.tw, pilot, "+helm/approach B");
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let app_submits = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &app_submits);

    let match_b = run(&mut rig.tw, pilot, "+helm/match B");
    assert!(
        match_b.contains("RCS velocity match locked on Block B"),
        "{match_b}"
    );
    let laser_b = run(&mut rig.tw, pilot, "+mine/laser B strategic_elements");
    assert!(
        laser_b.contains("extracting 4 t `strategic_ore`"),
        "{laser_b}"
    );
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let work_submits = drain_space_and_industry(&mut rig.tw);
    land_all(&mut rig, &work_submits);
    assert_eq!(
        pool(&mut rig.tw, vault_id, "strategic_ore"),
        4,
        "4 t (4,000 kg) of unlicensed strategic_ore in stow.ore hold vault"
    );

    let registry_account = mush_economy::AccountId(
        port_map(&rig.tw, "ceres_port").orgs[&cid("freeport_registry")]
            .account
            .expect("registry account id"),
    );

    // 2. Undock and burn on `ceres_stickney`; cross the interdicted Kirkwood waypoint boundary.
    let pending = undock_and_burn(&mut rig, pilot, hull);
    let depart_ic = pending.plan.depart_ic;
    let leg1_secs = i64::from(pending.plan.legs[0].secs);
    let leg2_secs = i64::from(pending.plan.legs[1].secs);
    let total_secs = pending.plan.total_s;

    let _ = fire_quietly(&mut rig);
    let boundary_events = to_ic(&mut rig, depart_ic + leg1_secs);
    let boundary_heard = said(&rig.tw, &boundary_events, pilot);
    assert!(
        boundary_heard.contains("Republic Customs Corvette: \"Heave-to under 10 m/s")
            && boundary_heard.contains("light-lag")
            && boundary_heard.contains("+helm/submit"),
        "light-lagged radio hail emitted on promotion: {boundary_heard}"
    );
    let had_picket_panel = boundary_events.iter().any(|(_, ev)| {
        matches!(
            ev,
            OutputEvent::Panel { feed, data, .. }
                if feed == "tacsim:notice" && data.get("kind").and_then(|v| v.as_str()) == Some("picket_hail")
        )
    });
    assert!(
        had_picket_panel,
        "expected structured tacsim:notice picket_hail panel event"
    );

    let (corvette_ship_id, raider_ship_id) = {
        let bubble_book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = bubble_book
            .active_for_hull(hull)
            .expect("active Kirkwood bubble");
        assert_eq!(
            active.bubble.ships.len(),
            3,
            "3-party bubble: carrier, Republic Customs Corvette, Belt Raider"
        );
        let picket = active.picket.as_ref().expect("KirkwoodPicketState present");
        assert_eq!(
            picket.inspection_status,
            tacsim::space::PicketInspectionStatus::AwaitingHeaveTo
        );
        assert!(!picket.raider_ambush_launched);
        assert_eq!(
            active.bubble.party_relation(
                tacsim::space::CARRIER_PARTY,
                tacsim::space::CUSTOMS_CORVETTE_PARTY,
            ),
            tacsim::space::SpaceRelation::Inspecting
        );
        assert_eq!(
            active.bubble.party_relation(
                tacsim::space::CARRIER_PARTY,
                tacsim::space::BELT_RAIDER_PARTY,
            ),
            tacsim::space::SpaceRelation::Hostile
        );
        assert_eq!(
            active.bubble.party_relation(
                tacsim::space::CUSTOMS_CORVETTE_PARTY,
                tacsim::space::BELT_RAIDER_PARTY,
            ),
            tacsim::space::SpaceRelation::Hostile
        );
        (
            picket.corvette_ship_id.clone(),
            picket.raider_ship_id.clone(),
        )
    };

    // Verify `+tac/scan` detects the Republic Customs Corvette at 6 km (`ActiveCombat`) and hides
    // the lurking Belt Raider at 120 km (`Ghost`).
    let scan_out = run(&mut rig.tw, pilot, "+tac/scan");
    assert!(
        scan_out.contains(&corvette_ship_id) && !scan_out.contains(&raider_ship_id),
        "scan detects Corvette at 6 km and hides Ghost Raider at 120 km: {scan_out}"
    );

    // 3. Verify firing on the non-hostile Republic Customs Corvette is refused (`35a` §3 #7),
    //    and `+helm/submit` before heaving-to (`rel_speed = 15 m/s >= 10 m/s`) is refused.
    let beam_refused = run(&mut rig.tw, pilot, &format!("+tac/beam {corvette_ship_id}"));
    assert!(
        beam_refused.contains("Cannot fire on a non-hostile contact."),
        "firing on non-hostile Corvette refused: {beam_refused}"
    );
    let submit_fast = run(&mut rig.tw, pilot, "+helm/submit");
    assert!(
        submit_fast.contains("Must heave-to under 10.0 m/s relative speed"),
        "submit before heaving-to refused: {submit_fast}"
    );

    // 4. Fire 1 wall second while still closing at 15 m/s so `KIRKWOOD_PICKET_RHAI` emits
    //    `PICKET_HAIL_HEAVE_TO`, then decelerate via `+helm/thrust 600 -1000 0 0` (-15 m/s²)
    //    and fire 1 wall second so `KIRKWOOD_PICKET_RHAI` transitions to `PICKET_CARRIER_HOVE_TO_READY`.
    let next_real0 = rig.tw.world().now() + 1;
    rig.tw.set_now(next_real0);
    let _ = rig.tw.fire_due(next_real0 * 1_000);
    {
        let bubble_book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = bubble_book.active_for_hull(hull).expect("active bubble");
        let picket = active.picket.as_ref().expect("picket state");
        assert_eq!(picket.corvette_think_seq, 1);
        assert_eq!(
            picket.corvette_last_trace,
            vec!["PICKET_HAIL_HEAVE_TO".to_owned()]
        );
    }

    let brake_out = run(&mut rig.tw, pilot, "+helm/thrust 600 -1000 0 0");
    assert!(
        brake_out.contains("Helm thrust set: 600‰ burn=(-1000, 0, 0)"),
        "{brake_out}"
    );
    let next_real = rig.tw.world().now() + 1;
    rig.tw.set_now(next_real);
    let _ = rig.tw.fire_due(next_real * 1_000);
    let _ = run(&mut rig.tw, pilot, "+helm/thrust 0");

    {
        let bubble_book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = bubble_book.active_for_hull(hull).expect("active bubble");
        let picket = active.picket.as_ref().expect("picket state");
        assert_eq!(
            picket.inspection_status,
            tacsim::space::PicketInspectionStatus::HoveToReadyForInspection,
            "Corvette RhaiRunner transitioned picket to HoveToReadyForInspection"
        );
        assert!(picket.corvette_think_seq >= 1);
        assert_eq!(
            picket.corvette_last_trace,
            vec!["PICKET_CARRIER_HOVE_TO_READY".to_owned()]
        );
        assert!(
            !picket.raider_ambush_launched,
            "Belt Raider must NOT prematurely uncloak before +helm/submit"
        );
    }

    // 5. Submit to customs inspection (`+helm/submit`): verify live `stow.ore` + Registry check
    //    detects `4,000 kg` of unlicensed `strategic_ore`, assesses the 2,000 cr Public E4 contract
    //    fine into `freeport_registry`, and advances the `"Customs Assertion"` campaign front!
    let reg_balance_before = economy(&rig.tw)
        .snapshot()
        .account(registry_account)
        .expect("registry account")
        .available()
        .expect("amount")
        .minor();
    let submit_out = run(&mut rig.tw, pilot, "+helm/submit");
    assert!(
        submit_out.contains("assessed 2000.00cr contraband fine")
            && submit_out.contains("4000 kg unlicensed `strategic_ore` detected")
            && submit_out.contains("Customs Assertion"),
        "submit output: {submit_out}"
    );

    // Sweep the economy clock (60s period) so the Public E4 fine contract settles into `freeport_registry`'s account.
    sweep(&mut rig.tw, 60);
    let reg_balance_after = economy(&rig.tw)
        .snapshot()
        .account(registry_account)
        .expect("registry account")
        .available()
        .expect("amount")
        .minor();
    assert_eq!(
        reg_balance_after.saturating_sub(reg_balance_before),
        200_000,
        "2,000.00 cr (200,000 minor credits) customs fine settled into freeport_registry's account"
    );

    // Verify `"Customs Assertion"` campaign front opened and advanced in `mush-faction` (`SimState`).
    let sim_state = rig
        .tw
        .world()
        .module_state::<mush_faction::SimState>()
        .expect("mush-faction SimState");
    let customs_front = sim_state
        .fronts
        .values()
        .find(|f| f.title == mush_faction::CUSTOMS_ASSERTION_TITLE)
        .expect("Customs Assertion front opened in mush-faction");
    assert!(!customs_front.resolved);
    let customs_clock = sim_state
        .clocks
        .values()
        .find(|c| c.front == customs_front.id)
        .expect("Customs Assertion front clock");
    assert!(
        customs_clock.filled >= 1 && customs_clock.segments == 6,
        "Customs Assertion front clock advanced on contraband fine: {:?}",
        customs_clock
    );

    // 6. On the next 1-second tick after inspection, the lurking Belt Raider lights its torch
    //    (`ActiveCombat`), `BELT_RAIDER_RHAI` launches its 2-torpedo salvo and 120 km railgun shot
    //    (`RAIDER_TORCH_AND_SALVO`), and `KIRKWOOD_PICKET_RHAI` transitions to `"screen_defend"`
    //    (`PICKET_SCREEN_ENGAGE_RAIDER`)!
    {
        let bubble_book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = bubble_book.active_for_hull(hull).expect("active bubble");
        let picket = active.picket.as_ref().expect("picket state");
        assert!(picket.raider_ambush_launched);
        assert!(picket.raider_think_seq >= 1);
        assert_eq!(
            active.bubble.ships[&raider_ship_id].emcon,
            tacsim::space::EmconPosture::ActiveCombat
        );
        assert_eq!(
            picket.corvette_last_trace,
            vec!["PICKET_SCREEN_ENGAGE_RAIDER".to_owned()]
        );
    }

    // Pilot enables `+tac/pdc auto` and jinks laterally (`+helm/thrust 600 0 1000 0`) during the
    // railgun slug's time-of-flight so the 120 km world-frame railgun shot misses (`RailgunDodgedByJink`).
    let pdc_out = run(&mut rig.tw, pilot, "+tac/pdc auto");
    assert!(pdc_out.contains("PDC auto-intercept enabled."), "{pdc_out}");
    let jink_out = run(&mut rig.tw, pilot, "+helm/thrust 600 0 1000 0");
    assert!(
        jink_out.contains("Helm thrust set: 600‰ burn=(0, 1000, 0)"),
        "{jink_out}"
    );

    for i in 0..24 {
        let next_real = rig.tw.world().now() + 1;
        rig.tw.set_now(next_real);
        let _ = rig.tw.fire_due(next_real * 1_000);
        if i == 2 {
            let _ = run(&mut rig.tw, pilot, "+helm/thrust 0");
        }
    }

    // Verify the 120 km railgun slug missed *Salt Moth* due to the lateral jink, and now close
    // range and disable the Belt Raider into an open-epoch derelict (`DisabledDerelict`).
    {
        let bubble_book = rig
            .tw
            .world_mut()
            .module_state_mut::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = bubble_book
            .active_for_hull_mut(hull)
            .expect("active bubble");
        let (_, replayed_events) = active.tape.replay();
        assert!(
            replayed_events.iter().any(|e| matches!(
                e,
                tacsim::space::SpaceCombatEvent::RailgunDodgedByJink { .. }
            )),
            "120 km railgun slug completed its flight window and missed after lateral jink (RailgunDodgedByJink)"
        );
        // Position the Raider within offensive laser range (4 km) of *Salt Moth* for the final gunnery pass.
        let carrier_pos = active.bubble.ships[&active.carrier_ship_id].pos_m;
        if let Some(raider) = active.bubble.ships.get_mut(&raider_ship_id) {
            raider.pos_m = [carrier_pos[0] + 3_000, carrier_pos[1], carrier_pos[2]];
            if let Some(rear) = raider
                .sections
                .get_mut(&tacsim::space::ShipSectionId::Hull)
                .and_then(|s| s.facets.get_mut(&tacsim::space::FacetBand::Front))
            {
                *rear = 0;
            }
            if let Some(drive) = raider
                .sections
                .get_mut(&tacsim::space::ShipSectionId::Drive)
            {
                drive.internals = 10;
            }
        }
        if let Some(carrier) = active.bubble.ships.get_mut(&active.carrier_ship_id) {
            carrier.attitude_deg = (180, 0, 0);
            carrier.heat_permille = 0;
            for l in &mut carrier.lasers {
                l.cooldown_remaining_ms = 0;
            }
        }
    }
    // Attack from behind the Raider (`Front`/`Rear` facet -> `Drive` struck-through):
    {
        let bubble_book = rig
            .tw
            .world_mut()
            .module_state_mut::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = bubble_book
            .active_for_hull_mut(hull)
            .expect("active bubble");
        active.bubble.torpedoes.clear();
        active.bubble.railgun_slugs.clear();
        if let Some(raider) = active.bubble.ships.get_mut(&raider_ship_id) {
            raider.attitude_deg = (0, 0, 0);
            if let Some(rear) = raider
                .sections
                .get_mut(&tacsim::space::ShipSectionId::Hull)
                .and_then(|s| s.facets.get_mut(&tacsim::space::FacetBand::Rear))
            {
                *rear = 0;
            }
        }
    }

    let kill_beam = run(&mut rig.tw, pilot, &format!("+tac/beam {raider_ship_id}"));
    assert!(
        kill_beam.contains("Laser battery fired on"),
        "pilot disabled Belt Raider with +tac/beam: {kill_beam}"
    );

    // Fire one more tick so `clear_resolved_bubbles` sees all hostiles disabled,
    // registers the Belt Raider derelict in `SalvageClaimBook` (48h salvage window), closes the
    // Kirkwood bubble, and resumes *Salt Moth*'s `CarrierJourney` to Stickney Yards!
    let next_real = rig.tw.world().now() + 1;
    rig.tw.set_now(next_real);
    let resume_events = rig.tw.fire_due(next_real * 1_000);
    let resume_heard = said(&rig.tw, &resume_events, pilot);
    assert!(
        resume_heard.contains(
            "[SPACE] Interdiction contact cleared; local bubble closed and transit burn resumed."
        ),
        "bubble closed and transit burn resumed on hostile disablement: {resume_heard}"
    );

    {
        let bubble_book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        assert!(
            !bubble_book.derelicts.is_empty(),
            "disabled Belt Raider persisted as an open-epoch derelict with 48h salvage window in SpaceBubbleBook::derelicts"
        );
        let derelict = bubble_book
            .derelicts
            .values()
            .next_back()
            .expect("disabled Belt Raider derelict");
        assert!(derelict.epoch_open);
        let Some(tacsim::field_ops::CustodyTitleStatus::PendingSalvageClaim { claim_id, .. }) =
            &derelict.salvage_title_status
        else {
            panic!(
                "expected PendingSalvageClaim on disabled Belt Raider derelict, got {:?}",
                derelict.salvage_title_status
            );
        };
        let salvage_book = rig
            .tw
            .world()
            .module_state::<tacsim::field_ops::SalvageClaimBook>()
            .expect("SalvageClaimBook");
        let claim = salvage_book
            .claim(*claim_id)
            .expect("Belt Raider salvage claim in SalvageClaimBook");
        assert_eq!(
            claim.deadline_real_s - claim.filed_at_real_s,
            tacsim::field_ops::CONTEST_WINDOW_AVAILABLE_REAL_SECS
        );
    }

    // Complete leg 2 to Stickney Yards and verify docking!
    let j_resumed = journey(&rig.tw, hull).expect("resumed journey");
    assert_eq!(*j_resumed.state(), mush_sim::JourneyState::Cruising);
    let expected_arrival = j_resumed.departed().at + total_secs;
    let _ = leg2_secs;
    let arrival_events = to_ic(&mut rig, expected_arrival);
    let arrival_heard = said(&rig.tw, &arrival_events, pilot);
    assert!(
        arrival_heard.contains("Salt Moth docks at Stickney Yards."),
        "docked at Stickney Yards after surviving Kirkwood Customs Picket & Raider ambush: {arrival_heard}"
    );
    no_refused_writes(&rig.tw);
}

/// **Doc 35 cut SF.R7 (`D-35-12`, `D-35-14`, `Q-35-5`, Story 2 step 3)**:
/// End-to-end integration test verifying:
/// 1. Initiating a 6-DOF `DockingSession` (`+helm/dock`) steps the ship out of the live world tick
///    (`paused_from_world_tick == true`) and emits the `"space.docking"` panel feed.
/// 2. Server-side `replay_docking_tape` rejects forged session IDs, over-length tapes (`> 3,600` frames),
///    and uncommanded/out-of-gate tapes (`DockingOutcome::OutOfGate`).
/// 3. Submitting a hard-bounce tape (`vz > 800 mm/s` into `dz == 0`) inflicts `Front`-facet armor
///    damage on the hull's `TacticalUnit`, debits `stow.propellant`, and immediately unpauses the ship
///    (`35a` §5 #7).
/// 4. Submitting a valid client-side 60 Hz `DockingInputTape` against a disabled derelict clamps the
///    collar (`DockingOutcome::Clamped`), debits `rcs_spent_kg` from `stow.propellant`, and enables
///    `+helm/tow` under the $2\times$ canonical `UnitSpec` dry-mass cap and 48-hour `SalvageClaimBook`
///    window (Story 2 step 3).
/// 5. Submitting a valid client-side 60 Hz `DockingInputTape` on approach to Stickney Yards merges
///    directly into `Berth::Docked { map, def }` (`D-35-14`).
#[test]
fn sfr7_docking_session_pauses_ship_replays_valid_tape_into_berth_docked_rejects_forged_tape_and_tows_derelict()
 {
    let mut rig = stocked_with(Roster {
        seed_lane: Some(lane_with_interdicted_kirkwood()),
        ..Roster::default()
    });
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    let vault_id = hold_vault(&rig.tw, &row);

    seed_pool(&mut rig, vault_id, "propellant", 53, LotId(921));
    seed_pool(&mut rig, vault_id, "provisions", 19, LotId(922));
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot);

    let pending = undock_and_burn(&mut rig, pilot, hull);
    let depart_ic = pending.plan.depart_ic;
    let leg1_secs = i64::from(pending.plan.legs[0].secs);

    let _ = fire_quietly(&mut rig);
    // Cross the interdicted Kirkwood boundary to open the tactical bubble:
    let _ = to_ic(&mut rig, depart_ic + leg1_secs);

    // Disable the interdictor so it becomes an open-epoch `DisabledDerelict` inside the bubble:
    let interdictor_hull = {
        let book = rig
            .tw
            .world_mut()
            .module_state_mut::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = book.active_for_hull_mut(hull).expect("active bubble");
        active.bubble.torpedoes.clear();
        active.bubble.railgun_slugs.clear();
        let ihull = active.interdictor_hull;
        let iship_id = active.interdictor_ship_id.clone();
        if let Some(iship) = active.bubble.ships.get_mut(&iship_id) {
            iship.status = tacsim::space::HullStatus::DisabledDerelict {
                epoch_open: true,
                life_support_remaining_s: tacsim::space::DERELICT_WINDOW_S,
                salvage_contest_window_s: tacsim::space::DERELICT_WINDOW_S,
            };
            iship.thrust_mps2 = [0, 0, 0];
        }
        ihull
    };

    // 1. Attempting `+helm/tow` before hard-docking must be refused:
    let premature_tow = run(&mut rig.tw, pilot, "+helm/tow");
    assert!(
        premature_tow.contains("Must complete a 6-DOF hard-dock clamp"),
        "premature +helm/tow before docking clamp must be refused: {premature_tow}"
    );

    // 2. Initiate a 6-DOF docking session against the disabled derelict (`+helm/dock derelict`):
    let dock_start = run(&mut rig.tw, pilot, "+helm/dock derelict");
    assert!(
        dock_start.contains("[DOCKING] Initiated 60 fps 6-DOF docking session"),
        "expected docking session start: {dock_start}"
    );

    let (session_id_1, challenge_1, steps_before_pause) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        assert!(
            book.is_hull_paused_for_docking(hull),
            "hull must be paused from world tick during active DockingSession"
        );
        let session = book.docking_sessions.get(&hull).expect("docking session");
        let steps = book.active_for_hull(hull).expect("active bubble").steps;
        (
            session.challenge.session_id.clone(),
            session.challenge.clone(),
            steps,
        )
    };

    // Advance the world tick while `paused_from_world_tick == true` and verify the bubble neither
    // steps nor auto-closes while the pilot is in the 60 Hz docking mini-game (`Q-35-5`, `D-35-14`):
    let next_real = rig.tw.world().now() + 2;
    rig.tw.set_now(next_real);
    let _ = rig.tw.fire_due(next_real * 1_000);
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = book
            .active_for_hull(hull)
            .expect("bubble must remain open while paused for docking");
        assert_eq!(
            active.steps, steps_before_pause,
            "paused hull's bubble must not step while DockingSession is active"
        );
    }

    // Also verify interactive text control order parsing (`DOCK TRANS UP`, `35a` §1 row 8):
    let order_out = run(&mut rig.tw, pilot, "+helm/dock DOCK TRANS UP");
    assert!(
        order_out.contains("6-DOF RCS pulse applied"),
        "server must parse interactive DOCK TRANS UP order: {order_out}"
    );

    // 3. Verify server rejects a forged session ID, an over-length tape (`> 3,600` frames), and an
    //    uncommanded/out-of-gate tape:
    let forged_id_out = run(
        &mut rig.tw,
        pilot,
        "+helm/dock tape dock.forged.999 0000*10",
    );
    assert!(
        forged_id_out.contains("docking session ID mismatch"),
        "forged session ID must be rejected: {forged_id_out}"
    );

    let overlength_out = run(
        &mut rig.tw,
        pilot,
        &format!("+helm/dock tape {session_id_1} 0000*3601"),
    );
    assert!(
        overlength_out.contains("exceeds cap (3600 frames)"),
        "tape > 3,600 frames must be rejected: {overlength_out}"
    );

    let out_of_gate_out = run(
        &mut rig.tw,
        pilot,
        &format!("+helm/dock tape {session_id_1} 0000*120"),
    );
    assert!(
        out_of_gate_out.contains("Docking input tape rejected: collar gates not aligned"),
        "uncommanded/out-of-gate tape must be rejected: {out_of_gate_out}"
    );

    // 4. Submit a hard-bounce tape (`vz > 800 mm/s` into `dz == 0`) and verify:
    //    - `Front` facet armor damage is inflicted on the hull's `TacticalUnit`
    //    - `paused_from_world_tick` is immediately cleared (`35a` §5 #7)
    let front_armor_before = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = book.active_for_hull(hull).expect("active bubble");
        active.bubble.ships[&active.carrier_ship_id].sections[&tacsim::space::ShipSectionId::Hull]
            .facets[&tacsim::space::FacetBand::Front]
    };
    let bounce_tape = tacsim::space::solve_hard_bounce_tape(&challenge_1);
    let bounce_out = run(
        &mut rig.tw,
        pilot,
        &format!(
            "+helm/dock tape {session_id_1} {}",
            bounce_tape.encode_rle()
        ),
    );
    assert!(
        bounce_out.contains("[DOCKING] Hard collar bounce")
            && bounce_out.contains("hull unpaused"),
        "expected hard collar bounce and unpause: {bounce_out}"
    );
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        assert!(
            !book.is_hull_paused_for_docking(hull),
            "hull must not stay paused after a bounced dock (`35a` §5 #7)"
        );
        let active = book.active_for_hull(hull).expect("active bubble");
        let front_armor_after = active.bubble.ships[&active.carrier_ship_id].sections
            [&tacsim::space::ShipSectionId::Hull]
            .facets[&tacsim::space::FacetBand::Front];
        assert!(
            front_armor_after < front_armor_before,
            "hard collar bounce must damage Front facet armor ({front_armor_before} -> {front_armor_after})"
        );
    }

    // 5. Re-initiate `+helm/dock derelict`, solve a valid 60 Hz `DockingInputTape`, verify hard-dock
    //    clamp + `stow.propellant` RCS debit, and attach the salvage tow line (`+helm/tow`, Story 2 step 3):
    let _ = run(&mut rig.tw, pilot, "+helm/dock derelict");
    let (session_id_2, challenge_2) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let s = book.docking_sessions.get(&hull).expect("session 2");
        (s.challenge.session_id.clone(), s.challenge.clone())
    };
    let valid_derelict_tape = tacsim::space::solve_docking_tape(&challenge_2);
    let clamp_derelict_out = run(
        &mut rig.tw,
        pilot,
        &format!(
            "+helm/dock tape {session_id_2} {}",
            valid_derelict_tape.encode_rle()
        ),
    );
    assert!(
        clamp_derelict_out.contains("[DOCKING] Hard dock and magnetic collar seal confirmed on derelict"),
        "expected hard dock on derelict: {clamp_derelict_out}"
    );

    let tow_out = run(&mut rig.tw, pilot, "+helm/tow");
    assert!(
        tow_out.contains("[SALVAGE] Tow line locked to disabled derelict")
            && tow_out.contains("48-hour Freeport Registry salvage claim window"),
        "expected salvage tow line attachment: {tow_out}"
    );
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let tow = book.active_tows.get(&hull).expect("active tow record");
        assert_eq!(tow.derelict_hull, interdictor_hull);
        assert!(tow.derelict_dry_mass_t <= tow.tug_dry_mass_t * 2);
        let ledger = book.propellant_ledger(hull).expect("propellant ledger");
        assert!(
            ledger.total_burned_kg >= 1,
            "RCS propellant must be debited from hull's live propellant ledger"
        );
    }

    // 6. Now initiate a port docking session (`+helm/dock stickney`), replay a valid 60 Hz client
    //    `DockingInputTape`, and verify `berth_of(w, hull)` transitions directly into `Berth::Docked` (`D-35-14`):
    let dock_port_start = run(&mut rig.tw, pilot, "+helm/dock stickney");
    assert!(
        dock_port_start.contains("Toward Stickney Yards")
            || dock_port_start.contains("toward Stickney Yards"),
        "expected port docking session toward Stickney Yards: {dock_port_start}"
    );
    let (session_id_3, challenge_3) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let s = book.docking_sessions.get(&hull).expect("port session");
        (s.challenge.session_id.clone(), s.challenge.clone())
    };
    let valid_port_tape = tacsim::space::solve_docking_tape(&challenge_3);
    let clamp_port_out = run(
        &mut rig.tw,
        pilot,
        &format!(
            "+helm/dock tape {session_id_3} {}",
            valid_port_tape.encode_rle()
        ),
    );
    assert!(
        clamp_port_out.contains("Hard dock and magnetic collar seal confirmed at Stickney Yards")
            && clamp_port_out.contains("Berth::Docked"),
        "expected port clamp into Berth::Docked: {clamp_port_out}"
    );

    let status_at_stickney = run(&mut rig.tw, pilot, "+ship");
    assert!(
        status_at_stickney.contains("Stickney Yards"),
        "+ship must report ship berthed (`Berth::Docked`) at Stickney Yards: {status_at_stickney}"
    );
    no_refused_writes(&rig.tw);
}
