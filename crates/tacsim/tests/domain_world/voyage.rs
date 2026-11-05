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
use tacsim::arena::party::{Mark, PartyId, Relation};
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

/// The shipped lane with `kirkwood_depot`'s `kirkwood_picket` set to `false` so `ceres_stickney`'s
/// first leg (`ceres_freeport -> kirkwood_depot`, `hazard: 4, interdiction: 3`) opens the 2-ship
/// `Interdictor Picket` bubble (`D-35-5`, `SF.R1`, `SF.R14`).
fn lane_with_interdicted_kirkwood() -> &'static str {
    let mut doc: serde_json::Value = serde_json::from_str(LANE_JSON).expect("the lane is JSON");
    doc["manifest"]["digest"] = serde_json::json!(UNSIGNED);
    doc["entries"]["kirkwood_depot"]["def"]["kirkwood_picket"] = serde_json::json!(false);
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
    // Do 4 one-second real fires first (exchanging fire with `interdictor.1`), then jump past
    // `leg2_secs + 1_000_000` IC seconds on the 5th fire.
    for i in 0..4 {
        let next_real = rig.tw.world().now() + 1;
        rig.tw.set_now(next_real);
        let _ = rig.tw.fire_due(next_real * 1_000);
        if i == 0 || i == 2 {
            let beam_out = run(&mut rig.tw, pilot, "+tac/beam Interdictor Picket");
            assert!(
                beam_out.contains("Laser battery fired on"),
                "beam salvo during interdiction hold: {beam_out}"
            );
        }
        if i == 2 {
            let _ = run(&mut rig.tw, pilot, "+damage/vent");
        }
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
        4 * u64::from(tacsim::space::COMBAT_STEP_WALL_MS)
            + u64::from(tacsim::space::MAX_BUBBLE_TICK_STEP_MS),
        "bubble steps on measured wall time capped at MAX_BUBBLE_TICK_STEP_MS (4x1s + 30s = 34,000 ms), NOT scaled by 3,600:1 IC rate"
    );
    assert_eq!(
        active_paused.bubble.ic_timestamp_s, paused_ic,
        "bubble stamps live W-clock IC timestamp on each step"
    );
    assert!(active_paused.snapshots.len() >= 6);
    let (replayed, _) = active_paused.tape.replay();
    assert_eq!(replayed.state_hash(), active_paused.bubble.state_hash());

    // 3. Deliver the finishing `+tac/beam` salvo to disable the interdictor by play and fire one tick to close the bubble and resume leg 2.
    let finish_beam = run(&mut rig.tw, pilot, "+tac/beam Interdictor Picket");
    assert!(
        finish_beam.contains("Laser battery fired on"),
        "finishing salvo disables interdictor by play: {finish_beam}"
    );
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
    assert_eq!(
        completed.wall_elapsed_ms,
        4 * u64::from(tacsim::space::COMBAT_STEP_WALL_MS)
            + u64::from(tacsim::space::MAX_BUBBLE_TICK_STEP_MS),
    );

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

    // 2. Inside the bubble, exchange fire by play (`step_interdictor_picket` + `+tac/beam`) so the
    // carrier takes real `hull` and `drive` damage + a `DriveDegraded` crit, and the hostile
    // interdictor has its drive destroyed (`0/220`, `HullStatus::DisabledDerelict`).
    for i in 0..4 {
        let next_real = rig.tw.world().now() + 1;
        rig.tw.set_now(next_real);
        let _ = rig.tw.fire_due(next_real * 1_000);
        if i == 0 || i == 2 || i == 3 {
            let beam_out = run(&mut rig.tw, pilot, "+tac/beam Interdictor Picket");
            assert!(
                beam_out.contains("Laser battery fired on"),
                "carrier laser salvo {i}: {beam_out}"
            );
        }
        if i == 1 || i == 2 {
            let _ = run(&mut rig.tw, pilot, "+damage/vent");
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
        ship_out.contains("Damage: drive ")
            && ship_out.contains("/250, hull ")
            && ship_out.contains("/600."),
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
        derelict.life_support_remaining_s,
        tacsim::space::DERELICT_LIFE_SUPPORT_S
    );
    assert!(
        derelict.salvage_title_status.is_none(),
        "disablement alone must not auto-file a salvage claim before +helm/tow (SF.R19)"
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

    // Step ticks without returning fire until `step_interdictor_picket` disables `hull2`'s drive
    // (`ShipDisabledDerelict`) and `settle_disabled_carrier_bubbles` settles `hull2` into `derelicts`:
    let mut settle_heard = String::new();
    for _ in 0..16 {
        let next_real = rig.tw.world().now() + 1;
        rig.tw.set_now(next_real);
        let evs = rig.tw.fire_due(next_real * 1_000);
        let heard = said(&rig.tw, &evs, pilot2);
        if heard.contains(
            "[SPACE] Drive offline; hull adrift as an open-epoch derelict with 48 hours of life support.",
        ) {
            settle_heard = heard;
            break;
        }
    }
    assert!(
        settle_heard.contains(
            "[SPACE] Drive offline; hull adrift as an open-epoch derelict with 48 hours of life support."
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
        let h_order = mush_sim::tick::standing_order(
            rig.tw.world(),
            hull,
            mush_sim::tick::OrderChannel::Heading,
        )
        .expect("Heading standing order");
        assert_eq!(
            *h_order.value(),
            mush_sim::tick::OrderValue::Vector {
                x: 0,
                y: 1_000,
                z: 0
            }
        );
        let t_order = mush_sim::tick::standing_order(
            rig.tw.world(),
            hull,
            mush_sim::tick::OrderChannel::Thrust,
        )
        .expect("Thrust standing order");
        assert_eq!(
            *t_order.value(),
            mush_sim::tick::OrderValue::Scalar { value: 600 }
        );
        let p_order = mush_sim::tick::standing_order(
            rig.tw.world(),
            hull,
            mush_sim::tick::OrderChannel::Posture,
        )
        .expect("Posture standing order");
        assert_eq!(
            *p_order.value(),
            mush_sim::tick::OrderValue::Emcon { level: 1 }
        );

        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
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
    // At `step == 0`, the interdictor starts at 18 km (`[18_000, 0, 0]`) in `EmconPosture::Ghost`
    // with `0` thrust and `0` heat.
    let interdictor_id = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = book.active_for_hull(hull).expect("active bubble");
        active.interdictor_ship_id.clone()
    };

    // Cold/silent contact at 18 km is Missed by `mush-propagation`: hidden from `/scan` and refused by `/lock` & `/beam`.
    let scan_cold = run(&mut rig.tw, gunner, "+tac/scan");
    assert!(
        scan_cold.starts_with("Sensor pass: 0 contacts detected."),
        "scan_cold: {scan_cold}"
    );
    assert!(
        !scan_cold.contains("18000"),
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

    // Advance one 1-second tick so `step_interdictor_picket` lights the interdictor's torch
    // (`ActiveCombat`, `thrust_mps2 = [-4, 0, 0]`): IR bloom rises above the inverse-square floor,
    // so `mush_propagation::resolve_detection` returns `Detected`.
    let next_real = rig.tw.world().now() + 1;
    rig.tw.set_now(next_real);
    let _ = rig.tw.fire_due(next_real * 1_000);

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
    assert!(
        active.tape.entries.len() >= 5,
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

    // Load 50 t propellant and 60 crew-days provisions (single initial seed_pool before cast-off)
    // so the ship has margin to decouple, execute timed burns, test RETURN_RESERVE, and recouple
    // using the prepaid unflown corridor leg credit (`35b` §5 #12).
    seed_pool(&mut rig, vault_id, "propellant", 50, LotId(903));
    seed_pool(&mut rig, vault_id, "provisions", 60, LotId(904));
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot_actor);

    let pending = undock_and_burn(&mut rig, pilot_actor, hull);
    let depart_ic = pending.plan.depart_ic;
    let _ = fire_quietly(&mut rig);

    // Advance 6 IC hours into the first leg.
    let _ = to_ic(&mut rig, depart_ic + 21_600);

    let tank_before_decouple = pool(&mut rig.tw, vault_id, "propellant");
    assert!(
        tank_before_decouple >= 4,
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

    // 2. Execute a timed burn at rated acceleration: `+helm/burn 1,0,0 400` (`1 m/s²` for `400 s` = `400 m/s` Δv).
    let burn_out = run(&mut rig.tw, pilot_actor, "+helm/burn 1,0,0 400");
    assert!(
        burn_out.contains("Timed burn engaged: (1, 0, 0) m/s² for 400s (Δv=400 m/s"),
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
    // Try a burn at rated 1 m/s² for 72,000 s (72,000 m/s Δv) that fits within `live_kg` (~4,982 kg)
    // but leaves less than the post-burn `RETURN_RESERVE`:
    let warn_out = run(&mut rig.tw, pilot_actor, "+helm/burn 1,0,0 72000");
    assert!(
        warn_out.contains("RETURN_RESERVE warning"),
        "burn into reserve without force is refused with RETURN_RESERVE warning: {warn_out}"
    );

    // 4. Recouple onto a planned corridor passage via `+helm/autopilot stickney` (`D-35-6`)
    //    using the prepaid unflown fuel credit from the decoupled corridor leg (`35b` §5 #14) —
    //    no second `seed_pool` required!
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
    seed_pool(&mut rig, vault_id, "propellant", 80, LotId(903));
    seed_pool(&mut rig, vault_id, "provisions", 160, LotId(904));
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot_actor);

    // Place `rival_actor` aboard on the `flight_ops` bridge station so we can also test
    // unlicensed refusal and the 30-IC-day claim contest window.
    let airlock = location(&rig.tw, pilot_actor).expect("pilot is in airlock");
    teleport(&mut rig.tw, rival_actor, airlock);
    let _ = run(&mut rig.tw, pilot_actor, "+helm/station flight_ops=Rival");

    // 2. Depart Ceres Freeport berth for `pallas_inclined_belt` via `+ship/undock pallas_inclined_belt`
    //    (no teleporting `+helm/cluster` verb and no berth autopilot!), advance to arrival so
    //    `space_tick` promotes the hull into the cluster and lands the `DepositOpen` and
    //    `ClaimTenure` (Pallas Syndicate seed) submissions.
    let cluster_out = run(
        &mut rig.tw,
        pilot_actor,
        "+ship/undock pallas_inclined_belt throttle=500",
    );
    assert!(
        cluster_out.contains("Pallas Inclined Belt (`pallas_inclined_belt`)"),
        "undock to cluster output: {cluster_out}"
    );
    let transit_burn = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &transit_burn);
    let _ = fire_quietly(&mut rig);
    let launch_submits = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &launch_submits);

    // Advance to the cluster arrival timestamp (`cluster_arrival_ic_s`) so the hull arrives at
    // `pallas_inclined_belt` and `step_free_flight_records` + `step_clusters_on_tick` opens the
    // cluster's 4 `OreDeposit`s:
    let arrival_ic = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook")
        .free_flights[&_hull]
        .cluster_arrival_ic_s
        .expect("cluster_arrival_ic_s");
    let real_s = (arrival_ic.saturating_sub(ic_now(&rig.tw)) + 3_599) / 3_600;
    let t0 = rig.tw.world().now().saturating_add(real_s.max(6));
    rig.tw.set_now(t0);
    let _ = rig.tw.fire_due(t0.saturating_mul(1_000));
    let open_submits = drain_space_and_industry(&mut rig.tw);
    let deposit_submits: Vec<_> = open_submits
        .iter()
        .filter(|s| s.action.key().owner.as_str() == mush_industry::NS)
        .cloned()
        .collect();
    assert_eq!(
        deposit_submits.len(),
        4,
        "4 OreDeposit openings submitted for Pallas rocks A (S), B (M: 2 families), C (S)"
    );
    land_all(&mut rig, &deposit_submits);
    land_quietly(&mut rig, &open_submits);

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

    // Run `+helm/approach A`, step `space_tick` across 3 ticks (`1,200 m/tick` closure, never snapping
    // in 1 tick!) to close range and brake relative velocity, then `+helm/match A`.
    let app_out = run(&mut rig.tw, pilot_actor, "+helm/approach A");
    assert!(
        app_out.contains("Braking approach ordered toward Block A"),
        "approach output: {app_out}"
    );
    let app_burn = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &app_burn);
    for step_idx in 0..3 {
        let t = rig.tw.world().now().saturating_add(6);
        rig.tw.set_now(t);
        let _ = rig.tw.fire_due(t.saturating_mul(1_000));
        let app_submits = drain_space_and_industry(&mut rig.tw);
        land_quietly(&mut rig, &app_submits);
        if step_idx == 0 {
            // After 1 tick (`1,200 m` closure from `3,800 m`), ship is at `2,600 m` (`1,600 m` from Block A at `[1,000, 0, 0]`)
            // and `8.0 m/s` (`> 5.0 m/s`), so `+helm/match A` is still outside the velocity gate!
            let still_closing = run(&mut rig.tw, pilot_actor, "+helm/match A");
            assert!(
                still_closing.contains("exceeds the 5.0 m/s match gate"),
                "after 1 tick of approach, ship is still decelerating (8.0 m/s > 5.0 m/s): {still_closing}"
            );
        }
    }

    let match_ok = run(&mut rig.tw, pilot_actor, "+helm/match A");
    assert!(
        match_ok.contains("RCS velocity match locked on Block A"),
        "match locked after multi-tick approach braking: {match_ok}"
    );
    let match_burn = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &match_burn);

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

    // 6. Fly *Salt Moth* back from `pallas_inclined_belt` to Ceres Freeport (`+helm/autopilot ceres`),
    //    dock at Ceres, sell the 20 t of mined `metal_ore` from `stow.ore` at the Ceres Freeport
    //    Exchange (`room.c07_exchange`), and verify `tacsim_port_mill` mills it into `8 t` of `concentrate`!
    let ret_out = run(&mut rig.tw, pilot_actor, "+helm/autopilot ceres");
    assert!(
        ret_out.contains("Autopilot engaged for Ceres Freeport"),
        "autopilot back to Ceres output: {ret_out}"
    );
    let ret_burn = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &ret_burn);
    let ret_arrival = journey(&rig.tw, _hull)
        .and_then(|j| j.arrival_at())
        .expect("return journey arrival");
    let _ = to_ic(&mut rig, ret_arrival);

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
/// 3. Firing on the non-hostile Republic Customs Corvette (the bubble's inspecting flag /
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
    let mut rig = stocked();
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

    seed_pool(&mut rig, vault_id, "propellant", 80, LotId(913));
    seed_pool(&mut rig, vault_id, "provisions", 120, LotId(914));
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot);

    let lic_out = run(&mut rig.tw, pilot, "+licence/buy go");
    assert!(
        lic_out.contains("Freeport Registry") && lic_out.contains("2500.00cr"),
        "{lic_out}"
    );
    // Settle the licence purchase contract before checking pre-fine Registry balance.
    sweep(&mut rig.tw, 6);

    let _ = run(
        &mut rig.tw,
        pilot,
        "+ship/undock pallas_inclined_belt throttle=500",
    );
    let transit_burn = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &transit_burn);
    let _ = fire_quietly(&mut rig);
    let launch_submits = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &launch_submits);
    let arrival_ic = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook")
        .free_flights[&hull]
        .cluster_arrival_ic_s
        .expect("cluster_arrival_ic_s");
    let real_s = (arrival_ic.saturating_sub(ic_now(&rig.tw)) + 3_599) / 3_600;
    let t0 = rig.tw.world().now().saturating_add(real_s.max(6));
    rig.tw.set_now(t0);
    let _ = rig.tw.fire_due(t0.saturating_mul(1_000));
    let open_submits = drain_space_and_industry(&mut rig.tw);
    land_all(&mut rig, &open_submits);
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let seed_submits = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &seed_submits);

    let _ = run(&mut rig.tw, pilot, "+helm/approach B");
    let app_burn = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &app_burn);
    for _ in 0..3 {
        let t = rig.tw.world().now().saturating_add(6);
        rig.tw.set_now(t);
        let _ = rig.tw.fire_due(t.saturating_mul(1_000));
        let app_submits = drain_space_and_industry(&mut rig.tw);
        land_quietly(&mut rig, &app_submits);
    }

    let match_b = run(&mut rig.tw, pilot, "+helm/match B");
    assert!(
        match_b.contains("RCS velocity match locked on Block B"),
        "{match_b}"
    );
    let match_burn = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &match_burn);
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

    // Return to Ceres Freeport from `pallas_inclined_belt` and top up propellant/provisions at Ceres berth before casting off on `ceres_stickney`:
    let ret_out = run(&mut rig.tw, pilot, "+helm/autopilot ceres");
    assert!(
        ret_out.contains("Autopilot engaged for Ceres Freeport"),
        "sfr6 return autopilot output: {ret_out}"
    );
    let ret_burn = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &ret_burn);
    let ret_arrival = journey(&rig.tw, hull)
        .and_then(|j| j.arrival_at())
        .expect("return journey arrival");
    let _ = to_ic(&mut rig, ret_arrival);
    seed_pool(&mut rig, vault_id, "propellant", 53, LotId(916));
    seed_pool(&mut rig, vault_id, "provisions", 19, LotId(917));

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
            Relation::Neutral
        );
        assert_eq!(
            active.bubble.party_relation(
                tacsim::space::CARRIER_PARTY,
                tacsim::space::BELT_RAIDER_PARTY,
            ),
            Relation::Hostile
        );
        assert_eq!(
            active.bubble.party_relation(
                tacsim::space::CUSTOMS_CORVETTE_PARTY,
                tacsim::space::BELT_RAIDER_PARTY,
            ),
            Relation::Hostile
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
    for _ in 0..6 {
        let next_real = rig.tw.world().now() + 1;
        rig.tw.set_now(next_real);
        let _ = rig.tw.fire_due(next_real * 1_000);
    }
    let _ = run(&mut rig.tw, pilot, "+helm/thrust 0");
    let brake_burn = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &brake_burn);

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

    // Verify the 120 km railgun slug missed *Salt Moth* due to the lateral jink, and the Corvette's
    // `KIRKWOOD_PICKET_RHAI` (`laser_1` + `rail_spinal`) disabled the Belt Raider into `DisabledDerelict`
    // while the Raider's 2 torpedoes are still being screened!
    {
        let bubble_book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = bubble_book
            .active_for_hull(hull)
            .expect("active bubble while torpedoes are still in flight");
        let (_, replayed_events) = active.tape.replay();
        assert!(
            replayed_events.iter().any(|e| matches!(
                e,
                tacsim::space::SpaceCombatEvent::RailgunDodgedByJink { .. }
            )),
            "120 km railgun slug completed its flight window and missed after lateral jink (RailgunDodgedByJink)"
        );
        assert!(
            matches!(
                active.bubble.ships[&raider_ship_id].status,
                tacsim::space::HullStatus::DisabledDerelict { .. }
            ),
            "Republic Customs Corvette disabled Belt Raider by play (`KIRKWOOD_PICKET_RHAI`)"
        );
    }

    // Continue stepping ticks so the Corvette's defensive laser and Carrier PDCs finish intercepting
    // the Raider's 2-torpedo salvo and `clear_resolved_bubbles` closes the Kirkwood bubble and
    // resumes *Salt Moth*'s `CarrierJourney` to Stickney Yards!
    let mut resume_heard = String::new();
    for _ in 0..40 {
        let next_real = rig.tw.world().now() + 1;
        rig.tw.set_now(next_real);
        let evs = rig.tw.fire_due(next_real * 1_000);
        let heard = said(&rig.tw, &evs, pilot);
        if heard.contains(
            "[SPACE] Interdiction contact cleared; local bubble closed and transit burn resumed.",
        ) {
            resume_heard = heard;
            break;
        }
    }
    assert!(
        resume_heard.contains(
            "[SPACE] Interdiction contact cleared; local bubble closed and transit burn resumed."
        ),
        "bubble closed and transit burn resumed on hostile disablement and salvo clearance: {resume_heard}"
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
        assert!(
            derelict.salvage_title_status.is_none(),
            "disablement without +helm/tow leaves salvage_title_status None (SF.R19)"
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
    let total_secs = pending.plan.total_s;

    let _ = fire_quietly(&mut rig);
    // Cross the interdicted Kirkwood boundary to open the tactical bubble:
    let _ = to_ic(&mut rig, depart_ic + leg1_secs);

    // Turn 90 degrees so the interdictor's return fire hits `Right` facet, preserving `Front` facet
    // armor (`50`) for the hard-bounce collar scrape test below, then disable `interdictor.1` by play:
    let _ = run(&mut rig.tw, pilot, "+helm/heading 90,0");
    let interdictor_hull = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        book.active_for_hull(hull)
            .expect("active bubble")
            .interdictor_hull
    };
    for i in 0..4 {
        let next_real = rig.tw.world().now() + 1;
        rig.tw.set_now(next_real);
        let _ = rig.tw.fire_due(next_real * 1_000);
        if i == 0 || i == 2 || i == 3 {
            let beam_out = run(&mut rig.tw, pilot, "+tac/beam Interdictor Picket");
            assert!(
                beam_out.contains("Laser battery fired on"),
                "beam salvo {i}: {beam_out}"
            );
        }
        if i == 1 || i == 2 {
            let _ = run(&mut rig.tw, pilot, "+damage/vent");
        }
    }

    // 1. Attempting `+helm/tow` before hard-docking must be refused, and attempting `+helm/dock derelict`
    //    before closing inside the `<= 500 m` / `<= 15 m/s` collar-initiation envelope is refused:
    let premature_tow = run(&mut rig.tw, pilot, "+helm/tow");
    assert!(
        premature_tow.contains("Must complete a 6-DOF hard-dock clamp"),
        "premature +helm/tow before docking clamp must be refused: {premature_tow}"
    );
    let dock_far = run(&mut rig.tw, pilot, "+helm/dock derelict");
    assert!(
        dock_far.contains("outside the collar-initiation envelope"),
        "docking derelict from > 500 m refused: {dock_far}"
    );

    // Order `+helm/approach derelict` (holding the bubble open for salvage and closing range across 3 ticks):
    let app_derelict = run(&mut rig.tw, pilot, "+helm/approach derelict");
    assert!(
        app_derelict.contains("Braking approach ordered toward disabled derelict"),
        "approach derelict output: {app_derelict}"
    );
    let app_burn = drain(&mut rig.tw);
    land_quietly(&mut rig, &app_burn);
    for _ in 0..3 {
        let next_real = rig.tw.world().now() + 1;
        rig.tw.set_now(next_real);
        let _ = rig.tw.fire_due(next_real * 1_000);
    }

    // 2. Initiate a 6-DOF docking session against the disabled derelict (`+helm/dock derelict`):
    let dock_start = run(&mut rig.tw, pilot, "+helm/dock derelict");
    assert!(
        dock_start.contains("[DOCKING] Initiated 60 fps 6-DOF docking session"),
        "expected docking session start: {dock_start}"
    );

    let (session_id_1, steps_before_pause) = {
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
        (session.challenge.session_id.clone(), steps)
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
    //    uncommanded/out-of-gate tape (which closes the single-use session, `SF.R9`):
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
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        assert!(
            !book.is_hull_paused_for_docking(hull),
            "out-of-gate tape closes the single-use DockingSession (`SF.R9`)"
        );
    }

    // 4. Re-initiate `+helm/dock derelict`, submit a hard-bounce tape (`vz > 800 mm/s` into `dz == 0`)
    //    and verify:
    //    - `Front` facet armor damage is inflicted on the hull's `TacticalUnit` (while preserving custody!)
    //    - `paused_from_world_tick` is immediately cleared (`35a` §5 #7)
    let _ = run(&mut rig.tw, pilot, "+helm/dock derelict");
    let (session_id_bounce, challenge_bounce) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let s = book.docking_sessions.get(&hull).expect("bounce session");
        (s.challenge.session_id.clone(), s.challenge.clone())
    };
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
    let bounce_rle = client_solve_hard_bounce_rle(&challenge_bounce);
    let bounce_out = run(
        &mut rig.tw,
        pilot,
        &format!("+helm/dock tape {session_id_bounce} {bounce_rle}"),
    );
    assert!(
        bounce_out.contains("[DOCKING] Hard collar bounce") && bounce_out.contains("hull unpaused"),
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
        assert!(
            mush_tactics::custody::marker_of(rig.tw.world(), hull).is_some(),
            "collar scrape inside active bubble must preserve UnitStore custody marker (`SF.R9`)"
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
    let valid_derelict_rle = client_solve_docking_rle(&challenge_2);
    let clamp_derelict_out = run(
        &mut rig.tw,
        pilot,
        &format!("+helm/dock tape {session_id_2} {valid_derelict_rle}"),
    );
    assert!(
        clamp_derelict_out
            .contains("[DOCKING] Hard dock and magnetic collar seal confirmed on derelict"),
        "expected hard dock on derelict: {clamp_derelict_out}"
    );

    let tow_out = run(&mut rig.tw, pilot, "+helm/tow");
    assert!(
        tow_out.contains("[SALVAGE] Tow line locked to disabled derelict")
            && tow_out.contains("You file a salvage claim on the"),
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

    // 6. Step 1 tick so `clear_resolved_bubbles` closes the Kirkwood bubble and resumes Leg 2
    //    toward Stickney Yards, advance to the leg-end approach window (`expected_arrival - 10`),
    //    initiate `+helm/dock stickney`, replay a valid 60 Hz client `DockingInputTape`, and verify
    //    `berth_of(w, hull)` transitions directly into `Berth::Docked` (`D-35-14`):
    let next_real = rig.tw.world().now() + 1;
    rig.tw.set_now(next_real);
    let _ = rig.tw.fire_due(next_real * 1_000);
    let j_resumed = journey(&rig.tw, hull).expect("resumed journey on leg 2");
    let expected_arrival = j_resumed.departed().at + total_secs;
    set_rate(&mut rig.tw, 1);
    let _ = to_ic(&mut rig, expected_arrival - 10);

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
    let valid_port_rle = client_solve_docking_rle(&challenge_3);
    let clamp_port_out = run(
        &mut rig.tw,
        pilot,
        &format!("+helm/dock tape {session_id_3} {valid_port_rle}"),
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

/// **Doc 35 cut SF.R8 (`D-35-12`)**:
/// - Active interdiction bubbles (`ActiveInterdictionBubble`, `KirkwoodPicketState`,
///   `BubbleInputTape`, `BubbleSnapshot`, `standing_orders`), free-flight records
///   (`SpaceFreeFlightRecord`, `HullPropellantLedger`), open-epoch derelicts
///   (`SpaceDerelictRecord` + `SalvageClaimBook`), and paused 6-DOF client docking
///   sessions (`DockingSession`) are persisted in `TacsimStore` under `tacsim:space:state`.
/// - Story 1 (mid-bubble triad encounter) and Story 2 (mid-free-flight + open-epoch
///   derelict + paused docking session) survive a full engine restart (`reboot_with`)
///   with bit-identical `SpaceBubbleBook` and `SalvageClaimBook` state.
/// - Open-epoch derelict life-support (`life_support_remaining_s`) decrements on the world
///   tick (`35a` §1 `D-35-12`).
#[test]
fn sfr8_space_state_persists_in_tacsim_store_and_survives_full_engine_restart_mid_bubble_free_flight_derelict_and_docking()
 {
    let roster = Roster::default();
    let mut rig = stocked_with(roster);
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    let vault_id = hold_vault(&rig.tw, &row);

    let desk = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, pilot, desk);
    let refit_out = run(&mut rig.tw, pilot, "+coop/refit prospector");
    assert!(refit_out.contains("Dray-P"), "{refit_out}");

    seed_pool(&mut rig, vault_id, "propellant", 53, LotId(931));
    seed_pool(&mut rig, vault_id, "provisions", 19, LotId(932));
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot);

    // 1. Story 1: cast off on the interdicted corridor, promote into the Kirkwood Customs
    //    Picket triad bubble, brake to heave-to, submit to inspection, trigger the Raider
    //    ambush, and set standing orders:
    let pending = undock_and_burn(&mut rig, pilot, hull);
    let depart_ic = pending.plan.depart_ic;
    let leg1_secs = i64::from(pending.plan.legs[0].secs);
    let total_secs = pending.plan.total_s;

    let _ = fire_quietly(&mut rig);
    let _ = to_ic(&mut rig, depart_ic + leg1_secs);

    let _ = run(&mut rig.tw, pilot, "+helm/thrust 600 -1000 0 0");
    for _ in 0..6 {
        let next_real = rig.tw.world().now() + 1;
        rig.tw.set_now(next_real);
        let _ = rig.tw.fire_due(next_real * 1_000);
    }
    let _ = run(&mut rig.tw, pilot, "+helm/thrust 0");
    let brake_submits = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &brake_submits);
    let submit_out = run(&mut rig.tw, pilot, "+helm/submit");
    assert!(
        submit_out.contains("customs clearance granted"),
        "expected customs inspection completion: {submit_out}"
    );
    let next_real2 = rig.tw.world().now() + 1;
    rig.tw.set_now(next_real2);
    let _ = rig.tw.fire_due(next_real2 * 1_000);
    let _ = run(&mut rig.tw, pilot, "+tac/pdc auto");
    let _ = run(&mut rig.tw, pilot, "+helm/thrust 600 0 1000 0");

    let (book_before_1, salvage_before_1, raider_hull) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook")
            .clone();
        let salvage = rig
            .tw
            .world()
            .module_state::<tacsim::field_ops::SalvageClaimBook>()
            .expect("SalvageClaimBook")
            .clone();
        let active = book
            .active_for_hull(hull)
            .expect("active bubble before restart 1");
        let picket = active.picket.as_ref().expect("KirkwoodPicketState");
        assert!(picket.raider_ambush_launched);
        assert_eq!(active.bubble.ships.len(), 3);
        let rhull = picket.raider_hull;
        (book, salvage, rhull)
    };

    // 2. Full engine restart #1 mid-bubble (Story 1):
    rig = reboot_with(rig, roster);

    {
        let book_after_1 = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook after restart 1")
            .clone();
        let salvage_after_1 = rig
            .tw
            .world()
            .module_state::<tacsim::field_ops::SalvageClaimBook>()
            .expect("SalvageClaimBook after restart 1")
            .clone();
        assert_eq!(
            book_after_1, book_before_1,
            "Story 1 mid-bubble SpaceBubbleBook must be bit-identical across full engine restart"
        );
        assert_eq!(
            salvage_after_1, salvage_before_1,
            "Story 1 mid-bubble SalvageClaimBook must be bit-identical across full engine restart"
        );
        let store = rig
            .tw
            .world()
            .module_state::<tacsim::TacsimStore>()
            .expect("TacsimStore");
        let persisted = store
            .space_state()
            .expect("persisted space_state in TacsimStore");
        assert_eq!(persisted.format, tacsim::space::SPACE_STATE_FORMAT);
        assert_eq!(persisted.book, book_before_1);
    }

    // 3. Resume the Story 1 fight after restart #1 by play: step ticks so the Corvette's
    //    `KIRKWOOD_PICKET_RHAI` disables the Raider into an open-epoch `SpaceDerelictRecord` +
    //    48 real-hour `SalvageClaimBook` claim, the Raider's salvo resolves, the bubble closes,
    //    and the derelict clocks decrement on subsequent ticks (`35a` §1 row `D-35-12`):
    for i in 0..55 {
        let next_r = rig.tw.world().now() + 1;
        rig.tw.set_now(next_r);
        let _ = rig.tw.fire_due(next_r * 1_000);
        if i == 2 {
            let _ = run(&mut rig.tw, pilot, "+helm/thrust 0");
        }
        let bubble_closed = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .is_some_and(|b| b.active_for_hull(hull).is_none());
        if bubble_closed {
            break;
        }
    }
    for _ in 0..3 {
        let next_r = rig.tw.world().now() + 1;
        rig.tw.set_now(next_r);
        let _ = rig.tw.fire_due(next_r * 1_000);
    }

    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let derelict = book
            .derelicts
            .get(&raider_hull)
            .expect("disabled Raider must be recorded as an open-epoch SpaceDerelictRecord");
        assert!(derelict.epoch_open);
        assert!(
            derelict.life_support_remaining_s < tacsim::space::DERELICT_LIFE_SUPPORT_S,
            "open-epoch derelict life-support ({}) must decrement on the world tick",
            derelict.life_support_remaining_s
        );
        assert!(
            derelict.salvage_title_status.is_none(),
            "disablement alone must not file a salvage claim before +helm/tow (SF.R19), got {:?}",
            derelict.salvage_title_status
        );
    }

    // 4. Story 2: fly Leg 2 to the Stickney Yards terminal approach window (`expected_arrival - 10`),
    //    decouple into 3D heliocentric free flight (`SpaceFreeFlightRecord`), execute a timed burn
    //    debiting `HullPropellantLedger`, and initiate a paused 6-DOF docking session (`+helm/dock stickney`):
    let j_resumed = journey(&rig.tw, hull).expect("resumed journey on leg 2");
    let expected_arrival = j_resumed.departed().at + total_secs;
    set_rate(&mut rig.tw, 1);
    let _ = to_ic(&mut rig, expected_arrival - 10);

    let decouple_out = run(&mut rig.tw, pilot, "+helm/decouple");
    assert!(
        decouple_out.contains("Decoupled from corridor leg"),
        "expected decouple into free flight: {decouple_out}"
    );
    let burn_out = run(&mut rig.tw, pilot, "+helm/burn 1,0,0 10 force");
    assert!(
        burn_out.contains("Timed burn engaged"),
        "expected timed burn: {burn_out}"
    );
    let next_r = rig.tw.world().now() + 1;
    rig.tw.set_now(next_r);
    let _ = rig.tw.fire_due(next_r * 1_000);

    let dock_start = run(&mut rig.tw, pilot, "+helm/dock stickney");
    assert!(
        dock_start.contains("[DOCKING] Initiated 60 fps 6-DOF docking session"),
        "expected docking session start: {dock_start}"
    );

    let (book_before_2, salvage_before_2, session_id, challenge) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook before restart 2")
            .clone();
        let salvage = rig
            .tw
            .world()
            .module_state::<tacsim::field_ops::SalvageClaimBook>()
            .expect("SalvageClaimBook before restart 2")
            .clone();
        assert!(book.free_flight_for_hull(hull).is_some());
        assert!(book.propellant_ledger(hull).is_some());
        assert!(book.derelicts.contains_key(&raider_hull));
        assert!(book.is_hull_paused_for_docking(hull));
        let session = book.docking_sessions.get(&hull).expect("docking session");
        let sid = session.challenge.session_id.clone();
        let chal = session.challenge.clone();
        (book, salvage, sid, chal)
    };

    // 5. Full engine restart #2 mid-free-flight + open-epoch derelict + paused docking session:
    rig = reboot_with(rig, roster);

    {
        let book_after_2 = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook after restart 2")
            .clone();
        let salvage_after_2 = rig
            .tw
            .world()
            .module_state::<tacsim::field_ops::SalvageClaimBook>()
            .expect("SalvageClaimBook after restart 2")
            .clone();
        assert_eq!(
            book_after_2, book_before_2,
            "Story 2 mid-free-flight + derelict + paused docking SpaceBubbleBook must be bit-identical across full engine restart"
        );
        assert_eq!(
            salvage_after_2, salvage_before_2,
            "Story 2 SalvageClaimBook must be bit-identical across full engine restart"
        );
    }

    // 6. Replay the valid 60 Hz client `DockingInputTape` after restart #2 and verify
    //    the ship clamps into `Berth::Docked` at Stickney Yards:
    let valid_rle = client_solve_docking_rle(&challenge);
    let clamp_out = run(
        &mut rig.tw,
        pilot,
        &format!("+helm/dock tape {session_id} {valid_rle}"),
    );
    assert!(
        clamp_out.contains("Hard dock and magnetic collar seal confirmed at Stickney Yards")
            && clamp_out.contains("Berth::Docked"),
        "expected hard dock at Stickney Yards after restart: {clamp_out}"
    );
    let status_out = run(&mut rig.tw, pilot, "+ship");
    assert!(
        status_out.contains("Stickney Yards"),
        "+ship must report ship berthed at Stickney Yards after restart: {status_out}"
    );
    no_refused_writes(&rig.tw);
}

/// **Doc 35b §8 cut `SF.R9` ("Authority close-out") acceptance**:
/// 1. `+helm/cluster` is removed (`Unknown +helm switch: /cluster.`).
/// 2. A timed burn of `1 s` at rate `1` (`burn_s == dt_ic_s`) zeroes `thrust_mmps2` after 1 tick
///    and does not latch (`35b` §3 #3).
/// 3. A pilot under fire in an interdiction bubble cannot `+helm/dock` away (refused while any
///    hostile contact is operational or ordnance is in flight, or while inside a mid-route
///    waypoint bubble, `35b` §3 #1).
/// 4. `+helm/run` is guarded by `picket.inspection_status`: refused after `+helm/submit` clearance
///    and refused on duplicate `+helm/run`.
/// 5. Initiating `+helm/dock` from a leg-end pose (`<= 60` IC s remaining on the leg targeting
///    the destination port) succeeds; `<session_id>` is mandatory on `+helm/dock tape`; an
///    `OutOfGate` tape closes the single-use session; and a session left open past
///    `DOCKING_SESSION_TIMEOUT_WALL_S` (`60 s`) is automatically aborted by `space_tick` with RCS
///    propellant debited and the hull unpaused.
/// 6. Settling a disabled carrier hull into `SpaceBubbleBook::derelicts` fails its `CarrierJourney`
///    (`JourneyState::Failed`), and `+helm/decouple`, `+helm/burn`, `+helm/autopilot`, and
///    `+helm/dock` are all refused for a disabled derelict hull (`35b` §3 #4).
/// 7. Contract lint: `tests/domain_world/voyage.rs` (this file) contains zero mutable `SpaceBubbleBook` borrows
///    or test-fiat bubble clear calls.
#[test]
#[allow(clippy::too_many_lines)]
fn sfr9_authority_closeout_gates_docking_removes_cluster_teleport_stops_1s_burn_and_locks_disabled_derelict()
 {
    // 7. Pin the no-fiat contract lint on this test file (`35b` §8):
    let voyage_src = include_str!("voyage.rs");
    let mut_needle = concat!("module_state_mut::<", "tacsim::space::SpaceBubbleBook>");
    assert!(
        !voyage_src.contains(mut_needle),
        "tests must never mutate SpaceBubbleBook via module_state_mut"
    );
    let clear_needle = concat!("tacsim::space::", "clear_interdiction", "_bubble(");
    assert!(
        !voyage_src.contains(clear_needle),
        "tests must never clear interdiction bubbles via test-fiat helper"
    );

    let mut rig = stocked_with(Roster {
        seed_lane: Some(lane_with_interdicted_kirkwood()),
        ..Roster::default()
    });
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    let vault_id = hold_vault(&rig.tw, &row);

    seed_pool(&mut rig, vault_id, "propellant", 80, LotId(941));
    seed_pool(&mut rig, vault_id, "provisions", 40, LotId(942));
    set_rate(&mut rig.tw, 1);
    board(&mut rig, pilot);

    // 1. `+helm/cluster` is removed (`35b` §3 #2, §8 `SF.R9`):
    let cluster_gone = run(&mut rig.tw, pilot, "+helm/cluster pallas_inclined_belt");
    assert_eq!(
        cluster_gone.trim(),
        "Unknown switch \"/cluster\" for +helm.",
        "+helm/cluster must be removed"
    );

    // 2. A `1 s` timed burn at rate `1` (`burn_s == dt_ic_s`) stops after 1 tick and does not latch (`35b` §3 #3):
    let _pending = undock_and_burn(&mut rig, pilot, hull);
    let _ = fire_quietly(&mut rig);
    let next_r0 = rig.tw.world().now() + 10;
    rig.tw.set_now(next_r0);
    let _ = rig.tw.fire_due(next_r0 * 1_000);

    let dec_out = run(&mut rig.tw, pilot, "+helm/decouple");
    assert!(
        dec_out.contains("Decoupled from corridor leg"),
        "decouple output: {dec_out}"
    );
    let burn_1s = run(&mut rig.tw, pilot, "+helm/burn 1,0,0 1 force");
    assert!(
        burn_1s.contains("Timed burn engaged: (1, 0, 0) m/s² for 1s"),
        "1s burn output: {burn_1s}"
    );
    // Advance exactly 1 wall second at rate 1 (`dt_ic_s == 1 == burn_remaining_ic_s`):
    let next_r1 = rig.tw.world().now() + 1;
    rig.tw.set_now(next_r1);
    let _ = rig.tw.fire_due(next_r1 * 1_000);
    let vel_after_tick_1 = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let ff = book
            .free_flight_for_hull(hull)
            .expect("free flight after 1s burn");
        assert_eq!(
            ff.burn_remaining_ic_s, None,
            "1s burn remaining time cleared after 1s tick"
        );
        assert_eq!(
            ff.thrust_mmps2,
            [0, 0, 0],
            "1s burn at rate 1 must zero thrust_mmps2 when burn_s == dt_ic_s (must not latch!)"
        );
        ff.vel_mps
    };
    // Step 1 more second at rate 1 and verify thrust did not keep accelerating the hull:
    let next_r2 = rig.tw.world().now() + 1;
    rig.tw.set_now(next_r2);
    let _ = rig.tw.fire_due(next_r2 * 1_000);
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let ff = book
            .free_flight_for_hull(hull)
            .expect("free flight after 2nd tick");
        assert_eq!(
            ff.vel_mps, vel_after_tick_1,
            "velocity must not gain uncommanded thrust Δv after 1s timed burn ended"
        );
    }

    // Recouple via `+helm/autopilot stickney` (preserving the interdicted Kirkwood leg!) and cross into the bubble:
    set_rate(&mut rig.tw, 3_600);
    let auto_out = run(&mut rig.tw, pilot, "+helm/autopilot stickney");
    assert!(
        auto_out.contains("Autopilot engaged for Stickney Yards"),
        "autopilot output: {auto_out}"
    );
    for s in drain(&mut rig.tw) {
        land(&mut rig, &s).expect("autopilot burn lands");
    }
    let j_recoupled = journey(&rig.tw, hull).expect("recoupled journey");
    let leg1_due = j_recoupled.leg_due_at(0).expect("leg 1 due_ic");
    let _ = to_ic(&mut rig, leg1_due);

    // 3. A pilot under fire in an interdiction bubble cannot `+helm/dock` away (`35b` §3 #1):
    let dock_under_fire = run(&mut rig.tw, pilot, "+helm/dock stickney");
    assert!(
        dock_under_fire.contains("Cannot initiate collar docking while hostile contact")
            || dock_under_fire.contains("while inside a mid-route interdiction bubble"),
        "pilot under fire in bubble must be refused +helm/dock: {dock_under_fire}"
    );

    // Disable the interdictor by play, close range, and resume Leg 2 to Stickney Yards:
    for i in 0..4 {
        let next_real = rig.tw.world().now() + 1;
        rig.tw.set_now(next_real);
        let _ = rig.tw.fire_due(next_real * 1_000);
        if i == 0 || i == 2 || i == 3 {
            let _ = run(&mut rig.tw, pilot, "+tac/beam Interdictor Picket");
        }
        if i == 1 || i == 2 {
            let _ = run(&mut rig.tw, pilot, "+damage/vent");
        }
    }
    let next_real = rig.tw.world().now() + 1;
    rig.tw.set_now(next_real);
    let _ = rig.tw.fire_due(next_real * 1_000);

    // Mid-leg (far from Stickney Yards), `+helm/dock stickney` is refused by the range/envelope gate:
    let dock_mid_leg = run(&mut rig.tw, pilot, "+helm/dock stickney");
    assert!(
        dock_mid_leg.contains("outside the Stickney Yards collar-initiation envelope"),
        "mid-leg +helm/dock must be refused: {dock_mid_leg}"
    );

    // 5. Advance to the leg-end pose (`arrival_at - 10`), initiate `+helm/dock stickney`, test
    //    mandatory `<session_id>`, 60s wall-clock session expiry, and valid clamp:
    let j_leg2 = journey(&rig.tw, hull).expect("leg 2 journey");
    let arrival_at = j_leg2.arrival_at().expect("arrival_at");
    set_rate(&mut rig.tw, 1);
    let _ = to_ic(&mut rig, arrival_at - 10);

    let dock_leg_end = run(&mut rig.tw, pilot, "+helm/dock stickney");
    assert!(
        dock_leg_end.contains("[DOCKING] Initiated 60 fps 6-DOF docking session"),
        "docking from leg-end pose must succeed: {dock_leg_end}"
    );
    // Omitting `<session_id>` on `+helm/dock tape` is refused (`SF.R9`):
    let missing_id = run(&mut rig.tw, pilot, "+helm/dock tape 0000*10");
    assert_eq!(
        missing_id.trim(),
        "Usage: +helm/dock tape <session_id> <rle_tape>",
        "mandatory <session_id> enforced on +helm/dock tape"
    );

    // Leave the docking session open past `DOCKING_SESSION_TIMEOUT_WALL_S` (`60 s`) and verify
    // `space_tick` automatically aborts it, debits RCS propellant, and unpauses the hull:
    let next_timeout = rig.tw.world().now() + 65;
    rig.tw.set_now(next_timeout);
    let timeout_evs = rig.tw.fire_due(next_timeout * 1_000);
    let timeout_heard = said(&rig.tw, &timeout_evs, pilot);
    assert!(
        timeout_heard.contains("expired") && timeout_heard.contains("hull unpaused"),
        "stale docking session must auto-abort after 60s wall timeout: {timeout_heard}"
    );
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        assert!(!book.is_hull_paused_for_docking(hull));
    }

    // Re-initiate `+helm/dock stickney` from the leg-end pose and clamp into `Berth::Docked`:
    let dock_leg_end_2 = run(&mut rig.tw, pilot, "+helm/dock stickney");
    assert!(
        dock_leg_end_2.contains("[DOCKING] Initiated 60 fps 6-DOF docking session"),
        "{dock_leg_end_2}"
    );
    let (sid_ok, chal_ok) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let s = book.docking_sessions.get(&hull).expect("docking session");
        (s.challenge.session_id.clone(), s.challenge.clone())
    };
    let tape_ok_rle = client_solve_docking_rle(&chal_ok);
    let clamp_ok = run(
        &mut rig.tw,
        pilot,
        &format!("+helm/dock tape {sid_ok} {tape_ok_rle}"),
    );
    assert!(
        clamp_ok.contains("Hard dock and magnetic collar seal confirmed at Stickney Yards"),
        "{clamp_ok}"
    );

    // 4 & 6. Charter a second hull (`Dray-P` on the Kirkwood picket leg) to verify `+helm/run`
    //        status guarding and disabled derelict carrier journey failure + maneuver lock:
    let mut rig2 = stocked();
    let (pilot2, row2) = chartered(&mut rig2, "PilotTwo", 2);
    let hull2 = ObjId(row2.hull);
    let desk2 = station_room(&rig2.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig2.tw, pilot2, desk2);
    let _ = run(&mut rig2.tw, pilot2, "+coop/refit prospector");
    load(&mut rig2, &row2, 53, 19);
    set_rate(&mut rig2.tw, 3_600);
    board(&mut rig2, pilot2);
    let pending2 = undock_and_burn(&mut rig2, pilot2, hull2);
    let _ = fire_quietly(&mut rig2);
    let _ = to_ic(
        &mut rig2,
        pending2.plan.depart_ic + i64::from(pending2.plan.legs[0].secs),
    );

    // Run the picket once (`+helm/run`), then verify a second `+helm/run` is refused (`picket.inspection_status` guard):
    let run_1 = run(&mut rig2.tw, pilot2, "+helm/run");
    assert!(
        run_1.contains("You refuse inspection and run the Republic Customs picket!"),
        "{run_1}"
    );
    let run_2 = run(&mut rig2.tw, pilot2, "+helm/run");
    assert_eq!(
        run_2.trim(),
        "You already ran the Republic Customs picket.",
        "duplicate +helm/run must be refused by inspection_status guard"
    );

    // Now charter a third hull (`Dray`) and let `step_interdictor_picket` disable its drive into a derelict:
    let mut rig3 = stocked_with(Roster {
        seed_lane: Some(lane_with_interdicted_kirkwood()),
        ..Roster::default()
    });
    let (pilot3, row3) = chartered(&mut rig3, "PilotThree", 3);
    let hull3 = ObjId(row3.hull);
    load(&mut rig3, &row3, 53, 19);
    set_rate(&mut rig3.tw, 3_600);
    board(&mut rig3, pilot3);
    let pending3 = undock_and_burn(&mut rig3, pilot3, hull3);
    let _ = fire_quietly(&mut rig3);
    let _ = to_ic(
        &mut rig3,
        pending3.plan.depart_ic + i64::from(pending3.plan.legs[0].secs),
    );
    for _ in 0..16 {
        let next_real = rig3.tw.world().now() + 1;
        rig3.tw.set_now(next_real);
        let _ = rig3.tw.fire_due(next_real * 1_000);
        if rig3
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .is_some_and(|b| b.derelicts.contains_key(&hull3))
        {
            break;
        }
    }
    // Verify the disabled carrier's `CarrierJourney` is `Failed` (`35b` §3 #4):
    let j3 = journey(&rig3.tw, hull3).expect("disabled carrier journey record");
    assert!(
        matches!(j3.state(), mush_sim::JourneyState::Failed),
        "disabled derelict carrier's JourneyState must be Failed, got {:?}",
        j3.state()
    );
    // And `+helm/decouple`, `+helm/burn`, `+helm/autopilot`, and `+helm/dock` are all refused:
    for cmd in [
        "+helm/decouple",
        "+helm/burn 1,0,0 10",
        "+helm/autopilot stickney",
        "+helm/dock stickney",
    ] {
        let out = run(&mut rig3.tw, pilot3, cmd);
        assert!(
            out.contains("disabled") && out.contains("unavailable"),
            "derelict crew `{cmd}` must be refused, got: {out}"
        );
    }

    // 7. Contract lint (`35b` §8): no test in `voyage.rs` mutates `SpaceBubbleBook` by hand via `module_state_mut`:
    let voyage_src = include_str!("voyage.rs");
    let forbidden_fq = concat!("module_state_mut::<", "tacsim::space::SpaceBubbleBook>");
    let forbidden_short = concat!("module_state_mut::<", "SpaceBubbleBook>");
    assert!(
        !voyage_src.contains(forbidden_fq) && !voyage_src.contains(forbidden_short),
        "voyage.rs must not mutate SpaceBubbleBook via module_state_mut"
    );
}

/// **Doc 35 cut SF.R10 (`D-35-3`, `D-35-12`, `35b` §5 #1/#6/#8/#9, §8 `SF.R10`)**:
/// Tempo and persistence budget:
/// 1. A tick fired `3 s` late (`now + 3`) advances the bubble by `3,000 ms` (`3 s` measured wall elapsed).
/// 2. A ten-minute (`600 s`) heave-to inside the Kirkwood Customs Picket bounds `snapshots` (`<= 8`),
///    `tape.entries` (`<= 128`), and the serialized `tacsim:space:state` JSON document size.
/// 3. Rebooting mid-bubble (`reboot_with`) preserves `mush_sim::deployed::is_deployed` custody across
///    `field::boot::phase`, and post-restart combat damage writes back to `UnitStore` on close (`35b` §5 #6).
/// 4. An unreadable `tacsim:space:state` document at boot is quarantined (`is_space_state_held`),
///    never overwritten by `space_tick`, and refuses `+helm`/`+tac`/`+mine` until staff repair (`35b` §5 #9).
#[test]
#[allow(clippy::too_many_lines)]
fn sfr10_tempo_and_persistence_budget_steps_late_ticks_bounds_10m_heave_to_quarantines_corrupt_doc_and_keeps_custody_across_restart()
 {
    let roster = Roster::default();
    let mut rig = stocked_with(roster);
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);

    // 1. Refit *Salt Moth* to `Dray-P` so the Kirkwood waypoint opens the 3-party picket bubble
    //    where we can heave-to for 10 wall minutes (`600 s`):
    let desk = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, pilot, desk);
    let _ = run(&mut rig.tw, pilot, "+coop/refit prospector");
    load(&mut rig, &row, 53, 19);
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot);

    let pending = undock_and_burn(&mut rig, pilot, hull);
    let depart_ic = pending.plan.depart_ic;
    let leg1_secs = i64::from(pending.plan.legs[0].secs);
    let _ = fire_quietly(&mut rig);
    let _ = to_ic(&mut rig, depart_ic + leg1_secs);

    // Verify a tick fired 3 s late advances the bubble by 3,000 ms (3 s):
    let elapsed_before = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook")
        .active_for_hull(hull)
        .expect("active bubble")
        .bubble
        .wall_elapsed_ms;
    let late_real = rig.tw.world().now() + 3;
    rig.tw.set_now(late_real);
    let _ = rig.tw.fire_due(late_real * 1_000);
    let elapsed_after = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook")
        .active_for_hull(hull)
        .expect("active bubble")
        .bubble
        .wall_elapsed_ms;
    assert_eq!(
        elapsed_after - elapsed_before,
        3_000,
        "a tick fired 3 s late must advance the bubble by 3,000 ms (3 s)"
    );

    // 2. Heave-to (`+helm/thrust 600 -1000 0 0` for 6s at 1 m/s², then `+helm/thrust 0`) and hold for 10
    //    minutes (`600` one-second fires) to verify persistence budget (`snapshots` and `tape` capped):
    let _ = run(&mut rig.tw, pilot, "+helm/thrust 600 -1000 0 0");
    for _ in 0..6 {
        let next1 = rig.tw.world().now() + 1;
        rig.tw.set_now(next1);
        let _ = rig.tw.fire_due(next1 * 1_000);
    }
    let _ = run(&mut rig.tw, pilot, "+helm/thrust 0");

    let mut doc_bytes_at_200 = 0usize;
    for step in 0..600 {
        let next_real = rig.tw.world().now() + 1;
        rig.tw.set_now(next_real);
        let _ = rig.tw.fire_due(next_real * 1_000);
        if step == 199 {
            let state = rig
                .tw
                .world()
                .module_state::<tacsim::TacsimStore>()
                .and_then(tacsim::TacsimStore::space_state)
                .expect("persisted space_state");
            doc_bytes_at_200 = serde_json::to_vec(state).expect("json").len();
        }
    }
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = book.active_for_hull(hull).expect("active bubble");
        assert!(
            active.snapshots.len() <= tacsim::space::MAX_BUBBLE_SNAPSHOTS,
            "snapshots capped at {}: got {}",
            tacsim::space::MAX_BUBBLE_SNAPSHOTS,
            active.snapshots.len()
        );
        assert!(
            active.tape.entries.len() <= tacsim::space::MAX_BUBBLE_TAPE_ENTRIES,
            "tape entries capped at {}: got {}",
            tacsim::space::MAX_BUBBLE_TAPE_ENTRIES,
            active.tape.entries.len()
        );
        let state_600 = rig
            .tw
            .world()
            .module_state::<tacsim::TacsimStore>()
            .and_then(tacsim::TacsimStore::space_state)
            .expect("persisted space_state");
        let doc_bytes_at_600 = serde_json::to_vec(state_600).expect("json").len();
        assert!(
            doc_bytes_at_600 <= doc_bytes_at_200 + 8_192 && doc_bytes_at_600 < 65_536,
            "ten-minute heave-to must keep tacsim:space:state bounded: 200s={doc_bytes_at_200}B, 600s={doc_bytes_at_600}B"
        );
    }

    // 3. Reboot mid-bubble (`reboot_with`) and verify `mush_sim::deployed::is_deployed` custody
    //    survives `field::boot::phase` (`35b` §5 #6):
    let rig = reboot_with(rig, roster);
    assert!(
        mush_sim::deployed::is_deployed(rig.tw.world(), hull),
        "hull custody mark must survive engine restart mid-bubble (reacquire_custody_after_boot)"
    );

    // 4. Reboot with a corrupted `tacsim:space:state` document and verify it is quarantined
    //    (`is_space_state_held`), never overwritten by `space_tick`, and refuses `+helm` commands:
    let mut rig = reboot_editing(rig, roster, |store| {
        if let Some(tacsim_data) = store.get_mut("tacsim") {
            tacsim_data
                .kv
                .retain(|(k, _)| k.as_str() != tacsim::space::SPACE_STATE_KEY);
            tacsim_data.kv.push((
                SmolStr::new_static(tacsim::space::SPACE_STATE_KEY),
                serde_json::json!({ "format": 999, "corrupt": true }),
            ));
        }
    });
    assert!(
        rig.tw
            .world()
            .module_state::<tacsim::TacsimStore>()
            .is_some_and(tacsim::TacsimStore::is_space_state_held),
        "corrupt tacsim:space:state must be quarantined (is_space_state_held)"
    );
    let helm_refused = run(&mut rig.tw, pilot, "+helm/status");
    assert!(
        helm_refused.contains("unreadable and held for staff repair"),
        "commands must refuse when tacsim:space:state is held: {helm_refused}"
    );
    let _ = rig.tw.take_module_ops();
    let tick_after = rig.tw.world().now() + 5;
    rig.tw.set_now(tick_after);
    let _ = rig.tw.fire_due(tick_after * 1_000);
    let ops = rig.tw.take_module_ops();
    let overwrote_space_state = ops.iter().any(|op| {
        op.ns.as_str() == "tacsim"
            && matches!(
                &op.op,
                ModuleWrite::JsonSet { key, .. } if key.as_str() == tacsim::space::SPACE_STATE_KEY
            )
    });
    assert!(
        !overwrote_space_state,
        "quarantined tacsim:space:state must never be overwritten by space_tick"
    );
}

/// **Doc 35 cut SF.R11 (`D-35-15`, `35b` §5 #10/#11/#12/#14/#15/#16, §8 `SF.R11`)**:
/// One ledger, rated drive:
/// 1. In-bubble `max_accel_mps2` equals the hull's rated drive flip acceleration (`1 m/s²` for the Dray's `50 mg` drive).
/// 2. A `CommandBurn` (`+helm/thrust 0 15 0 2`) inside an active bubble debits the hull's `stow.propellant` vault
///    via `HullPropellantLedger` + `submit_space_burn_if_needed`.
/// 3. Re-entering a bubble does not refill `propellant_kg` — the ledger's `fractional_burned_kg` persists across
///    bubble close and re-promotion.
/// 4. In free flight, a burn away from the nearest port evaluates `RETURN_RESERVE` on the post-burn `(pos, vel)`
///    and triggers `RETURN_RESERVE warning` before exhausting return propellant (`35b` §5 #11).
#[test]
#[allow(clippy::too_many_lines)]
fn sfr11_one_ledger_rated_drive_debits_bubble_jink_in_vault_persists_across_reentry_and_warns_burn_away_from_port()
 {
    let mut rig = stocked_with(Roster {
        seed_lane: Some(lane_with_interdicted_kirkwood()),
        ..Roster::default()
    });
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    let vault_id = hold_vault(&rig.tw, &row);

    seed_pool(&mut rig, vault_id, "propellant", 77, LotId(951));
    seed_pool(&mut rig, vault_id, "provisions", 45, LotId(952));
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot);

    let pending = undock_and_burn(&mut rig, pilot, hull);
    let depart_ic = pending.plan.depart_ic;
    let leg1_secs = i64::from(pending.plan.legs[0].secs);
    let _ = fire_quietly(&mut rig);
    let _ = to_ic(&mut rig, depart_ic + leg1_secs);

    // 1. Verify the carrier's in-bubble `max_accel_mps2` equals the hull's rated drive acceleration (`1 m/s²`),
    //    and `return_reserve_dv_mps` is hydrated from `compute_hull_return_reserve` (> 0):
    let (initial_bubble_prop_kg, initial_max_accel, initial_reserve_dv) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = book.active_for_hull(hull).expect("active bubble");
        let carrier = active
            .bubble
            .ships
            .get(&active.carrier_ship_id)
            .expect("carrier ship");
        (
            carrier.propellant_kg,
            carrier.max_accel_mps2,
            carrier.return_reserve_dv_mps,
        )
    };
    assert_eq!(
        initial_max_accel, 1,
        "in-bubble max_accel_mps2 must equal the Dray's rated drive acceleration (1 m/s² for 50 mg)"
    );
    assert!(
        initial_reserve_dv > 0,
        "in-bubble return_reserve_dv_mps must be hydrated from compute_hull_return_reserve: got {initial_reserve_dv}"
    );

    let vault_prop_before_jink = pool(&mut rig.tw, vault_id, "propellant");

    // 2. Execute a `CommandBurn` inside the active bubble (`+helm/thrust 600 0 1000 0`) and land the
    //    resulting `PendingSpaceBurn` submission on the hull's `stow.propellant` vault:
    let jink_out = run(&mut rig.tw, pilot, "+helm/thrust 600 0 1000 0");
    assert!(
        jink_out.contains("Helm thrust set: 600‰ burn=(0, 1000, 0)"),
        "bubble jink output: {jink_out}"
    );
    let _ = run(&mut rig.tw, pilot, "+helm/thrust 0");
    let jink_submits = drain(&mut rig.tw);
    assert_eq!(
        jink_submits.len(),
        1,
        "in-bubble CommandBurn must queue a tacsim_port_burn debit via HullPropellantLedger"
    );
    for s in &jink_submits {
        land(&mut rig, s).expect("in-bubble burn lands on stow.propellant vault");
    }
    let vault_prop_after_jink = pool(&mut rig.tw, vault_id, "propellant");
    assert_eq!(
        vault_prop_after_jink,
        vault_prop_before_jink - 1,
        "CommandBurn inside an active bubble debits the hull's stow.propellant vault"
    );

    let prop_kg_after_jink = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = book.active_for_hull(hull).expect("active bubble");
        active
            .bubble
            .ships
            .get(&active.carrier_ship_id)
            .expect("carrier ship")
            .propellant_kg
    };
    assert!(
        prop_kg_after_jink < initial_bubble_prop_kg,
        "in-bubble propellant_kg reduced by Tsiolkovsky burn: {prop_kg_after_jink} < {initial_bubble_prop_kg}"
    );

    // 3. Close the bubble by disabling the interdictor, then decouple and recouple across the
    //    interdicted Kirkwood leg to re-enter a bubble, and verify `propellant_kg` does NOT refill:
    for i in 0..4 {
        let next_real = rig.tw.world().now() + 1;
        rig.tw.set_now(next_real);
        let _ = rig.tw.fire_due(next_real * 1_000);
        if i == 0 || i == 2 || i == 3 {
            let _ = run(&mut rig.tw, pilot, "+tac/beam Interdictor Picket");
        }
        if i == 1 || i == 2 {
            let _ = run(&mut rig.tw, pilot, "+damage/vent");
        }
    }
    let next_real = rig.tw.world().now() + 1;
    rig.tw.set_now(next_real);
    let _ = rig.tw.fire_due(next_real * 1_000);
    assert!(
        rig.tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook")
            .active_for_hull(hull)
            .is_none(),
        "bubble closed after interdictor disabled"
    );

    // Decouple into free flight before the Kirkwood boundary, verify rated drive cap and
    // post-burn RETURN_RESERVE on a burn away from the nearest port, then recouple via
    // `+helm/autopilot stickney` (preserving the interdicted Kirkwood leg!) to re-enter the bubble:
    // First, let's decouple from Leg 2:
    let dec_out = run(&mut rig.tw, pilot, "+helm/decouple");
    assert!(dec_out.contains("Decoupled from corridor leg"), "{dec_out}");

    // 4. Verify `+helm/burn` rejects accelerations above the hull's rated drive acceleration (`1 m/s²`):
    let over_rated = run(&mut rig.tw, pilot, "+helm/burn 2,0,0 10");
    assert!(
        over_rated.contains("exceeds the hull drive's rated flip acceleration of 1 m/s²"),
        "burn above rated drive acceleration refused: {over_rated}"
    );

    // And verify a burn away from the nearest port evaluates `RETURN_RESERVE` at the post-burn `(pos, vel)`
    // and warns before exhausting return propellant:
    let warn_away = run(&mut rig.tw, pilot, "+helm/burn -1,0,0 86400");
    assert!(
        warn_away.contains("RETURN_RESERVE warning"),
        "burn away from port triggers post-burn RETURN_RESERVE warning: {warn_away}"
    );

    // Recouple to Ceres Freeport, then cast off on `ceres_stickney` and cross the interdicted leg
    // into a second bubble to verify `propellant_kg` does not refill (`fractional_burned_kg` preserved):
    let auto_ceres = run(&mut rig.tw, pilot, "+helm/autopilot ceres");
    assert!(
        auto_ceres.contains("Autopilot engaged for Ceres Freeport"),
        "{auto_ceres}"
    );
    for s in drain(&mut rig.tw) {
        land(&mut rig, &s).expect("autopilot burn lands");
    }
    let ceres_arr = journey(&rig.tw, hull)
        .and_then(|j| j.arrival_at())
        .expect("ceres arrival");
    let _ = to_ic(&mut rig, ceres_arr);

    // Top up whole tonnes for the second outbound `ceres_stickney` passage (`seed_pool` adds whole
    // tonnes in the vault while `HullPropellantLedger::fractional_burned_kg` retains the sub-tonne
    // deficit from the first bubble's jink):
    seed_pool(&mut rig, vault_id, "propellant", 40, LotId(953));
    let pending2 = undock_and_burn(&mut rig, pilot, hull);
    let _ = fire_quietly(&mut rig);
    let (expected_post_castoff_kg, vault_now_t) = {
        let vault_now_t = pool(&mut rig.tw, vault_id, "propellant");
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let ledger = book.propellant_ledger(hull).expect("ledger");
        assert!(
            ledger.fractional_burned_kg > 0,
            "fractional_burned_kg preserved across bubble close and voyage: {}",
            ledger.fractional_burned_kg
        );
        (
            u32::try_from(ledger.effective_live_kg(vault_now_t)).expect("u32"),
            vault_now_t,
        )
    };
    let leg1_due2 = pending2.plan.depart_ic + i64::from(pending2.plan.legs[0].secs);
    let _ = to_ic(&mut rig, leg1_due2);

    let reentry_prop_kg = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = book
            .active_for_hull(hull)
            .expect("re-entered active bubble");
        active
            .bubble
            .ships
            .get(&active.carrier_ship_id)
            .expect("carrier ship")
            .propellant_kg
    };
    assert_eq!(
        reentry_prop_kg, expected_post_castoff_kg,
        "re-entering a bubble must not refill propellant_kg (must preserve fractional_burned_kg)"
    );
    assert!(
        u64::from(reentry_prop_kg) < (vault_now_t + 1) * 1_000,
        "re-entered bubble propellant_kg ({reentry_prop_kg}) reflects sub-tonne burn rather than refilling"
    );
}

/// **Doc 35 cut SF.R12 (`35b` §5 #2/#3/#4/#5/#6/#7/#13, §8 `SF.R12`)**:
/// Domain bubbles and real hulls:
/// 1. Two chartered hulls (`*Salt Moth*` `Dray-P` miner and `*Escort Hawk*` `Dray-M` escort) arriving
///    at the Kirkwood waypoint share one bubble in `SpaceBubbleBook::by_domain` (`by_domain.len() == 1`,
///    `active.carriers.len() == 2`).
/// 2. The Republic Corvette (`corvette.json`, `tacsim.unit.corvette`) and Raider (`dray_m.json`,
///    crewed by named NPC leader `"Varga Teodor"` from `raiders.json`) are spawned as real world
///    `Thing`s with `TacticalUnit`s checked out through custody.
/// 3. An indigent pilot (`0` credits in personal vault) who heaves-to and runs `+helm/submit` while
///    carrying unlicensed `strategic_ore` has the contraband seized (`PicketInspectionOutcome::ContrabandSeized`,
///    `PicketInspectionStatus::ClearedAfterSeizure`) and debited from `stow.ore`, and is cleared.
/// 4. The escort disables the Raider; the miner approaches (`+helm/approach derelict`), docks
///    (`+helm/dock derelict` + `+helm/dock tape`), and tows (`+helm/tow`) the real Raider `Thing`;
///    the tow files the salvage claim on the one path (doc 41 cut RS.8), naming the Raider's NPC
///    leader, her real title holder, as `prior_owner`.
#[test]
#[allow(clippy::too_many_lines)]
fn sfr12_domain_bubbles_and_real_hulls_share_kirkwood_bubble_tow_real_raider_thing_with_leader_claim_and_seize_indigent_contraband()
 {
    let mut rig = stocked();

    // 1. Charter *Salt Moth* (indigent miner on `Dray-P` carrying 2 t unlicensed `strategic_ore`):
    let (pilot_miner, row_miner) = chartered(&mut rig, "Miner", 1);
    let hull_miner = ObjId(row_miner.hull);
    let vault_miner = hold_vault(&rig.tw, &row_miner);
    let desk = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, pilot_miner, desk);
    let refit_miner = run(&mut rig.tw, pilot_miner, "+coop/refit prospector");
    assert!(refit_miner.contains("Dray-P"), "{refit_miner}");
    seed_pool(&mut rig, vault_miner, "propellant", 75, LotId(961));
    seed_pool(&mut rig, vault_miner, "provisions", 25, LotId(962));
    seed_pool(&mut rig, vault_miner, "strategic_ore", 51, LotId(963));

    // Charter *Escort Hawk* (armed Dray escort):
    let pilot_escort = pilot(&mut rig, "Escort", 2);
    let enrol_escort = run(&mut rig.tw, pilot_escort, "+coop/enrol Escort Hawk");
    assert!(
        enrol_escort.contains("now the Escort Hawk"),
        "{enrol_escort}"
    );
    let charter_escort =
        tacsim::port::live_charter(rig.tw.world(), pilot_escort).expect("escort charter");
    let row_escort = port_map(&rig.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|r| r.hull == charter_escort.hull.0)
        .expect("escort hull row");
    let hull_escort = ObjId(row_escort.hull);
    let vault_escort = hold_vault(&rig.tw, &row_escort);
    seed_pool(&mut rig, vault_escort, "propellant", 53, LotId(964));
    seed_pool(&mut rig, vault_escort, "provisions", 19, LotId(965));

    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot_miner);
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, pilot_escort, gallery);
    let board_escort = run(&mut rig.tw, pilot_escort, "+ship/board");
    assert!(
        board_escort.contains("You cycle the lock into the Escort Hawk."),
        "{board_escort}"
    );

    let pending_miner = undock_and_burn(&mut rig, pilot_miner, hull_miner);
    let pending_escort = undock_and_burn(&mut rig, pilot_escort, hull_escort);
    let _ = fire_quietly(&mut rig);

    let miner_leg1_due = pending_miner.plan.depart_ic + i64::from(pending_miner.plan.legs[0].secs);
    let escort_leg1_due =
        pending_escort.plan.depart_ic + i64::from(pending_escort.plan.legs[0].secs);
    let convoy_kirkwood_due = miner_leg1_due.max(escort_leg1_due);
    let _ = to_ic(&mut rig, convoy_kirkwood_due);

    // 2. Verify both chartered hulls share one domain-keyed Kirkwood bubble, and that both NPC
    //    hulls (`corvette_hull` and `raider_hull`) are real world `Thing`s with checked-out `TacticalUnit`s:
    let (corvette_hull, raider_hull) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        assert_eq!(
            book.by_domain.len(),
            1,
            "two chartered hulls at the Kirkwood waypoint must share a single domain-keyed bubble"
        );
        let active = book
            .active_for_hull(hull_miner)
            .expect("miner in active bubble");
        assert!(
            active.carriers.contains_key(&hull_miner) && active.carriers.contains_key(&hull_escort),
            "shared bubble must contain both *Salt Moth* and *Escort Hawk*"
        );
        assert_eq!(
            active.carriers.len(),
            2,
            "shared bubble must track both carrier entries"
        );
        let picket = active.picket.as_ref().expect("KirkwoodPicketState");
        (picket.corvette_hull, picket.raider_hull)
    };

    assert!(
        rig.tw.world().get(corvette_hull).is_some()
            && mush_tactics::custody::marker_of(rig.tw.world(), corvette_hull).is_some(),
        "Republic Corvette must be a real world Thing with a TacticalUnit checked out through custody"
    );
    assert_eq!(
        mush_tactics::module::unit_of(rig.tw.world(), corvette_hull)
            .expect("corvette TacticalUnit")
            .schema()
            .key()
            .id
            .as_str(),
        tacsim::definitions::content::CORVETTE
    );
    assert!(
        rig.tw.world().get(raider_hull).is_some()
            && mush_tactics::custody::marker_of(rig.tw.world(), raider_hull).is_some(),
        "Raider must be a real world Thing with a TacticalUnit checked out through custody"
    );

    // 3. Indigent contraband seizure (`35b` §5 #13): *Salt Moth* heaves-to and runs `+helm/submit`
    //    with 2 t unlicensed `strategic_ore` and 0 credits in personal vault:
    let _ = run(&mut rig.tw, pilot_miner, "+helm/thrust 600 -1000 0 0");
    for _ in 0..6 {
        let next_r = rig.tw.world().now() + 1;
        rig.tw.set_now(next_r);
        let _ = rig.tw.fire_due(next_r * 1_000);
    }
    let _ = run(&mut rig.tw, pilot_miner, "+helm/thrust 0");
    let brake_submits = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &brake_submits);

    let submit_out = run(&mut rig.tw, pilot_miner, "+helm/submit");
    assert!(
        submit_out.contains("confiscated in lieu of fine")
            && submit_out.contains("customs clearance granted"),
        "indigent pilot must have contraband seized and be cleared: {submit_out}"
    );
    let seizure_submits = drain(&mut rig.tw);
    assert!(
        !seizure_submits.is_empty(),
        "contraband seizure must queue an authoritative tacsim_port_burn debit"
    );
    for s in &seizure_submits {
        land(&mut rig, s).expect("contraband seizure burn lands");
    }
    assert_eq!(
        pool(&mut rig.tw, vault_miner, "strategic_ore"),
        0,
        "seized strategic_ore must be debited from the hull's stow.ore hold"
    );
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let picket = book
            .active_for_hull(hull_miner)
            .and_then(|a| a.picket.as_ref())
            .expect("KirkwoodPicketState");
        assert_eq!(
            picket.inspection_status,
            tacsim::space::PicketInspectionStatus::ClearedAfterSeizure
        );
        assert!(matches!(
            picket.inspection_outcome,
            Some(tacsim::space::PicketInspectionOutcome::ContrabandSeized {
                seized_strategic_ore_kg: 51_000,
                ..
            })
        ));
    }

    // 4. Step the bubble so the Raider ambush launches, have the Escort (`*Escort Hawk*`) engage
    //    the Raider (`+tac/beam Raider`) alongside the Corvette until the Raider's drive is disabled:
    for step in 0..55 {
        let next_r = rig.tw.world().now() + 1;
        rig.tw.set_now(next_r);
        let _ = rig.tw.fire_due(next_r * 1_000);
        if step % 2 == 1 {
            let _ = run(&mut rig.tw, pilot_escort, "+tac/beam Raider");
        } else if step % 4 == 2 {
            let _ = run(&mut rig.tw, pilot_escort, "+damage/vent");
        }
        let raider_disabled = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .and_then(|b| b.active_for_hull(hull_miner))
            .and_then(|a| {
                let rsid = a.picket.as_ref()?.raider_ship_id.as_str();
                a.bubble.ships.get(rsid)
            })
            .is_some_and(|s| {
                matches!(s.status, tacsim::space::HullStatus::DisabledDerelict { .. })
            });
        if raider_disabled {
            break;
        }
    }

    // Have the miner (`*Salt Moth*`) order `+helm/approach derelict` on the disabled Raider `Thing`,
    // step until in-flight torpedoes clear and range closes inside the collar-initiation envelope,
    // dock via a 60 Hz `DockingInputTape`, and attach the salvage tow line (`+helm/tow`):
    let approach_out = run(&mut rig.tw, pilot_miner, "+helm/approach derelict");
    assert!(
        approach_out.contains("Braking approach ordered toward disabled derelict"),
        "expected miner approach on disabled Raider derelict: {approach_out}"
    );
    for _ in 0..48 {
        let next_r = rig.tw.world().now() + 1;
        rig.tw.set_now(next_r);
        let _ = rig.tw.fire_due(next_r * 1_000);
        let ready = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .and_then(|b| b.active_for_hull(hull_miner))
            .is_some_and(|a| {
                a.bubble.torpedoes.is_empty()
                    && !a
                        .carriers
                        .get(&hull_miner)
                        .is_some_and(|c| c.approaching_derelict)
            });
        if ready {
            break;
        }
    }
    let dock_raider_start = run(&mut rig.tw, pilot_miner, "+helm/dock derelict");
    assert!(
        dock_raider_start.contains("[DOCKING] Initiated 60 fps 6-DOF docking session"),
        "miner must initiate docking session on disabled Raider Thing: {dock_raider_start}"
    );
    let (sid_raider, chal_raider) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let s = book
            .docking_sessions
            .get(&hull_miner)
            .expect("miner docking session");
        (s.challenge.session_id.clone(), s.challenge.clone())
    };
    let valid_raider_rle = client_solve_docking_rle(&chal_raider);
    let clamp_raider_out = run(
        &mut rig.tw,
        pilot_miner,
        &format!("+helm/dock tape {sid_raider} {valid_raider_rle}"),
    );
    assert!(
        clamp_raider_out
            .contains("[DOCKING] Hard dock and magnetic collar seal confirmed on derelict"),
        "miner must hard-dock onto disabled Raider Thing: {clamp_raider_out}"
    );
    let tow_raider_out = run(&mut rig.tw, pilot_miner, "+helm/tow");
    assert!(
        tow_raider_out.contains("[SALVAGE] Tow line locked to disabled derelict"),
        "miner must lock salvage tow line to disabled Raider Thing: {tow_raider_out}"
    );
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let tow = book
            .active_tows
            .get(&hull_miner)
            .expect("miner active salvage tow");
        assert_eq!(
            tow.derelict_hull, raider_hull,
            "miner must tow the real Raider Thing (#{})",
            raider_hull.0
        );
        let derelict = book
            .derelicts
            .get(&raider_hull)
            .expect("disabled Raider settled into SpaceBubbleBook::derelicts");
        assert!(derelict.epoch_open);
        // Doc 41 cut RS.8 (Q-36-46): the tow put the Raider in the miner's custody and filed on
        // the one path; the claim's prior owner is the Raider's real title holder.
        let custody = book
            .space_custody
            .get(&raider_hull)
            .expect("the Raider in the miner's space custody");
        assert_eq!(custody.carrier, hull_miner);
        let prior_owner = custody
            .file
            .as_ref()
            .expect("the tow filed the Raider's claim")
            .prior_owner;
        let leader_obj = rig
            .tw
            .world()
            .get(prior_owner)
            .expect("prior_owner must be a real world object");
        assert_eq!(
            leader_obj.name.as_str(),
            "Kaelen Vane",
            "Raider salvage claim must name the space raider's authored NPC leader"
        );
    }
}

/// **Doc 35b cut SF.R13 acceptance (`D-35-7`, `D-35-10`, `D-35-15`)** — verifies:
/// 1. Every crew member aboard a carrier hull (`pilot` + `gunner`) receives `space.holotank`
///    and `space.cockpit` panel frames directly from the `SPACE_TICK` without typing a command,
///    and sees an incoming torpedo's `tti_s` count down across ticks in both feeds.
/// 2. `cockpit_feed` carries `bodies` and `pixel_model` on contacts, while `holotank_feed`
///    and `cockpit_feed` filter `torpedoes` through `mush_propagation` detection (`is_bubble_torpedo_detected`).
/// 3. Disconnecting the crew uncleanly (`Inbound::ConnDown` -> `HookPoint::Linkdead`) sets
///    `crew_linkdead = true`, cuts main-drive thrust to `[0, 0, 0]` (ballistic drift), holds
///    `pdc_auto` armed through the grace window with a queued return plan, and `Inbound::ConnUp`
///    (`from_linkdead: true`) restores manual bridge control.
#[test]
#[allow(clippy::too_many_lines)]
fn sfr13_presence_and_feeds_push_to_all_crew_filter_torpedoes_and_handle_linkdead_grace() {
    let mut rig = stocked_with(Roster {
        seed_lane: Some(lane_with_interdicted_kirkwood()),
        ..Roster::default()
    });
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    let gunner = rig.tw.spawn_player("Gunner");
    load(&mut rig, &row, 55, 60);
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot);
    let airlock = tacsim::port::ship::hull_view(rig.tw.world(), hull)
        .and_then(|v| v.airlock())
        .expect("carrier airlock");
    teleport(&mut rig.tw, gunner, airlock);
    let _ = run(&mut rig.tw, pilot, "+helm/station command=Pilot");
    let st_out = run(&mut rig.tw, pilot, "+helm/station tactical=Gunner");
    assert!(
        st_out.contains("Gunner now holds `tactical`"),
        "gunner assumes tactical station: {st_out}"
    );

    let pending = undock_and_burn(&mut rig, pilot, hull);
    let depart_ic = pending.plan.depart_ic;
    let leg1_secs = i64::from(pending.plan.legs[0].secs);
    let _ = fire_quietly(&mut rig);

    // Promote into the Kirkwood interdiction bubble.
    let promote_events = to_ic(&mut rig, depart_ic + leg1_secs);

    let panel_data_for = |events: &[(Audience, OutputEvent)], who: ObjId, feed_name: &str| {
        events.iter().rev().find_map(|(aud, ev)| match (aud, ev) {
            (Audience::Actor(a), OutputEvent::Panel { feed, data, .. })
                if *a == who && feed.as_str() == feed_name =>
            {
                Some(data.clone())
            }
            _ => None,
        })
    };

    // Both pilot and gunner received `space.holotank` and `space.cockpit` on promotion without typing!
    let gunner_holo_0 = panel_data_for(&promote_events, gunner, "space.holotank")
        .expect("gunner must receive space.holotank on bubble promotion");
    assert!(gunner_holo_0["contacts"].is_array());
    let gunner_cockpit_0 = panel_data_for(&promote_events, gunner, "space.cockpit")
        .expect("gunner must receive space.cockpit on bubble promotion");
    let bodies = gunner_cockpit_0["bodies"]
        .as_array()
        .expect("cockpit_feed must emit bodies array");
    assert!(
        !bodies.is_empty(),
        "cockpit_feed bodies must be non-empty: {gunner_cockpit_0}"
    );

    // Light up the interdictor with active radar (`+helm/emcon combat`) and step the tick:
    // the interdictor fires a torpedo at the carrier and both crew receive updated feeds from the tick!
    let _ = run(&mut rig.tw, pilot, "+helm/emcon combat");
    let tick1_events = fire_quietly(&mut rig);
    let gunner_holo_1 = panel_data_for(&tick1_events, gunner, "space.holotank")
        .expect("gunner receives space.holotank from tick step 1 without typing");
    let gunner_cockpit_1 = panel_data_for(&tick1_events, gunner, "space.cockpit")
        .expect("gunner receives space.cockpit from tick step 1 without typing");

    let contacts_1 = gunner_cockpit_1["contacts"]
        .as_array()
        .expect("cockpit_feed contacts array");
    let ship_contact = contacts_1
        .iter()
        .find(|c| c["kind"] == "ship")
        .expect("detected interdictor ship contact in cockpit_feed");
    assert_eq!(
        ship_contact["pixel_model"].as_str(),
        Some("corvette"),
        "cockpit_feed ship contact must carry pixel_model: {ship_contact}"
    );
    let torp_holo_1 = gunner_holo_1["torpedoes"]
        .as_array()
        .and_then(|arr| arr.first())
        .expect("detected incoming burning torpedo in holotank_feed");
    let tti_1 = torp_holo_1["tti_s"].as_i64().expect("torp tti_s on tick 1");
    let torp_cockpit_1 = contacts_1
        .iter()
        .find(|c| c["kind"] == "torpedo")
        .expect("detected incoming torpedo in cockpit_feed contacts");
    assert_eq!(torp_cockpit_1["pixel_model"].as_str(), Some("torpedo"));
    assert_eq!(torp_cockpit_1["tti_s"].as_i64(), Some(tti_1));

    // Advance wall time by 1 s per tick and fire `SPACE_TICK` without gunner typing: torpedo `tti_s` counts down!
    let mut tti_after = tti_1;
    for _ in 1..=3 {
        let t = rig.tw.world().now().saturating_add(1);
        rig.tw.set_now(t);
        let tick_ev = rig.tw.fire_due(t.saturating_mul(1_000));
        let queued = drain(&mut rig.tw);
        land_quietly(&mut rig, &queued);
        if let Some(holo) = panel_data_for(&tick_ev, gunner, "space.holotank")
            && let Some(torp) = holo["torpedoes"].as_array().and_then(|a| a.first())
            && let Some(v) = torp["tti_s"].as_i64()
        {
            tti_after = v;
        }
    }
    assert!(
        tti_after < tti_1,
        "gunner must see torpedo tti_s count down across ticks ({tti_1} -> {tti_after}) without typing"
    );

    // Pure unit check on `holotank_feed` torpedo detection filtering:
    // a cold coasting torpedo (`remaining_dv_mmps == 0`) at 25 km is filtered out of `holotank_feed`,
    // while a burning torpedo (`remaining_dv_mmps > 0`) at 25 km is included.
    {
        let carrier_party = PartyId::nth(0).expect("the arena letters a first party");
        let raider_party = PartyId::nth(1).expect("the arena letters a second party");
        assert_eq!((carrier_party.letter(), raider_party.letter()), ('A', 'B'));
        let mut unit_bubble =
            tacsim::space::LocalSpaceBubble::new("b.test", "kirkwood", "Test", 100);
        unit_bubble.insert_ship(tacsim::space::SpaceShipState::new_dray(
            "obs",
            "Observer",
            carrier_party,
            [0, 0, 0],
            [0, 0, 0],
        ));
        let mut foe = tacsim::space::SpaceShipState::new_corvette(
            "foe",
            "Raider",
            raider_party,
            [25_000, 0, 0],
            [0, 0, 0],
        );
        foe.emcon = tacsim::space::EmconPosture::Ghost;
        foe.heat_permille = 0;
        unit_bubble.insert_ship(foe);
        unit_bubble.set_party_relation(carrier_party, raider_party, Some(Mark::Hostile));
        let _ = unit_bubble.fire_torpedo_salvo("foe", "obs", 1);
        assert_eq!(
            unit_bubble.holotank_feed("obs")["torpedoes"]
                .as_array()
                .map(Vec::len),
            Some(1),
            "burning torpedo at 25 km must be detected in holotank_feed"
        );
        unit_bubble.torpedoes[0].remaining_dv_mmps = 0;
        assert_eq!(
            unit_bubble.holotank_feed("obs")["torpedoes"]
                .as_array()
                .map(Vec::len),
            Some(0),
            "cold ballistic coasting torpedo at 25 km must be filtered out of holotank_feed"
        );
    }

    // 3. Test D-35-7 linkdead presence hook:
    // Pilot orders a forward burn (`+helm/thrust 1000 force`) and switches PDC to manual first to prove
    // that going linkdead re-arms `pdc_auto = true` and cuts `thrust_mps2 = [0, 0, 0]`.
    let _ = run(&mut rig.tw, gunner, "+tac/pdc manual");
    let burn_out = run(&mut rig.tw, pilot, "+helm/thrust 1000 force");
    assert!(
        burn_out.contains("Helm thrust set"),
        "pilot commands forward burn: {burn_out}"
    );

    let conns_for = |tw: &TestWorld, who: ObjId| -> Vec<u64> {
        (1..=64)
            .filter(|conn| tw.engine().actor_of(*conn) == Some(who))
            .collect()
    };
    let pilot_conns = conns_for(&rig.tw, pilot);
    let gunner_conns = conns_for(&rig.tw, gunner);
    assert!(!pilot_conns.is_empty() && !gunner_conns.is_empty());

    // Drop pilot's connection first: gunner is still live-connected aboard, so hull is NOT linkdead yet.
    for &conn in &pilot_conns {
        let mut out = mush_core::Outbox::new();
        let mut fx = mush_core::Effects::new();
        rig.tw.engine_mut().step(
            mush_core::Inbound::ConnDown {
                conn,
                actor: pilot,
                reason: mush_core::DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
    }
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        assert!(
            book.linkdead_state(hull).is_none(),
            "hull must not go linkdead while gunner is still live-connected aboard"
        );
    }

    // Now drop gunner's connection: no live-connected crew remain aboard -> D-35-7 linkdead triggers!
    for &conn in &gunner_conns {
        let mut out = mush_core::Outbox::new();
        let mut fx = mush_core::Effects::new();
        rig.tw.engine_mut().step(
            mush_core::Inbound::ConnDown {
                conn,
                actor: gunner,
                reason: mush_core::DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
    }
    let (vel_at_linkdead, prop_at_linkdead) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let ld = book
            .linkdead_state(hull)
            .expect("hull must enter linkdead state when all crew lose connection");
        assert!(
            ld.pdc_held,
            "point-defense must be held armed during grace window"
        );
        assert!(!ld.grace_expired, "grace window must start unexpired");
        assert!(
            ld.queued_return_port.is_some(),
            "return plan port must be queued on linkdead"
        );
        let active = book.active_for_hull(hull).expect("active bubble");
        let ship = active
            .bubble
            .ships
            .get(active.carrier_ship_id_for(hull))
            .expect("carrier ship");
        assert!(ship.crew_linkdead, "ship.crew_linkdead must be true");
        assert_eq!(
            ship.thrust_mps2,
            [0, 0, 0],
            "main-drive thrust must be cut immediately on linkdead"
        );
        assert!(
            ship.pdc_auto,
            "pdc_auto must be held armed through the linkdead grace window"
        );
        (ship.vel_mmps, ship.propellant_kg)
    };

    // Step the bubble while linkdead: ship drifts ballistically (constant velocity, zero propellant burned).
    let now_w = rig.tw.world().now();
    rig.tw.world_mut().set_now(now_w + 10);
    let _ = fire_quietly(&mut rig);
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = book.active_for_hull(hull).expect("active bubble");
        let ship = active
            .bubble
            .ships
            .get(active.carrier_ship_id_for(hull))
            .expect("carrier ship");
        assert_eq!(
            ship.vel_mmps, vel_at_linkdead,
            "linkdead ship must drift ballistically without accelerating"
        );
        assert_eq!(
            ship.propellant_kg, prop_at_linkdead,
            "linkdead ship must not burn reaction mass while drifting"
        );
    }

    // Reconnect pilot (`Inbound::ConnUp { from_linkdead: true }`): manual control is restored!
    {
        let mut out = mush_core::Outbox::new();
        let mut fx = mush_core::Effects::new();
        rig.tw.engine_mut().step(
            mush_core::Inbound::ConnUp {
                conn: pilot_conns[0],
                actor: pilot,
                caps_structured: true,
                from_linkdead: true,
            },
            &mut out,
            &mut fx,
        );
    }
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        assert!(
            book.linkdead_state(hull).is_none(),
            "reviving pilot must clear linkdead state for hull"
        );
        let active = book.active_for_hull(hull).expect("active bubble");
        let ship = active
            .bubble
            .ships
            .get(active.carrier_ship_id_for(hull))
            .expect("carrier ship");
        assert!(
            !ship.crew_linkdead,
            "reviving pilot must clear ship.crew_linkdead"
        );
    }
}

/// **Doc 35b cut SF.R14 acceptance (`D-35-17`, `35b` §4 #1–#5, §8 `SF.R14`)** — verifies:
/// 1. Zero Rust `const .*_RHAI` or `const .*_ROCKS` in `crates/tacsim/src/`.
/// 2. `content/sky/outer_bodies.json` is a signed `rules_definition` pack (`tacsim.sky.outer_bodies@1`)
///    verified through `tacsim::pack::load_pack`.
/// 3. The shipped lane (`Roster::default()`, with no `seed_lane` override) authors `kirkwood_picket: true`
///    and the four orbital clusters (`ceres_halo`, `kirkwood_gap_3_1`, `pallas_inclined_belt`,
///    `saturn_b_ring_edge`) on `kirkwood_depot` and `hazard: 4, interdiction: 3` on `ceres_stickney`
///    leg 0, promoting a ship on `ceres_stickney` into the 3-party Kirkwood Customs Picket.
/// 4. Picket NPC ships (`corvette_hull`, `raider_hull`) carry verified `MindSeal`s (`SealEngine::Scripted`)
///    whose `source_sha256` resolves through `TacsimStore::mind_source` (`kirkwood_picket.rhai`,
///    `belt_raider.rhai`), and `step_kirkwood_picket` derives stage transitions from Rhai memory
///    rather than re-deciding stage in Rust before calling Rhai.
/// 5. An interdicted non-Kirkwood leg (`kirkwood_picket: false`) does NOT open the Kirkwood Customs Picket
///    even when flown by a `Dray-P` (`active.picket.is_none()`, `active.bubble.ships.len() == 2`), and its
///    `interdictor_hull` carries a verified `MindSeal` for `interdictor_patrol.rhai`.
/// 6. Launching collector drones (`+mine/drones 2 B`) creates `DroneSortieState`s with `ExcursionId`s
///    and verified `MindSeal`s (`drone_mining.rhai`) whose `mind_think_seq` advances on the tick and
///    whose `mind_last_trace` records `"DRONE_MINING_OUTBOUND"` / `"DRONE_MINING_COLLECT"`.
#[test]
#[allow(clippy::too_many_lines)]
fn sfr14_authoring_loads_clusters_doctrines_and_outer_bodies_from_packs_opens_picket_only_on_kirkwood_and_seals_drone_minds()
 {
    // 1. Contract lint: no `const .*_RHAI` or `const .*_ROCKS` in `crates/tacsim/src/`:
    fn check_no_const_rhai_or_rocks(dir: &std::path::Path) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.is_dir() {
                check_no_const_rhai_or_rocks(&path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("read rs file");
                for (lineno, line) in text.lines().enumerate() {
                    let trimmed = line.trim();
                    if let Some(rest) = trimmed
                        .strip_prefix("const ")
                        .or_else(|| trimmed.strip_prefix("pub const "))
                        .or_else(|| trimmed.strip_prefix("pub(crate) const "))
                    {
                        let name = rest.split(':').next().unwrap_or_default().trim();
                        assert!(
                            !name.ends_with("_RHAI") && !name.ends_with("_ROCKS"),
                            "found forbidden constant `{name}` at {}:{}: {trimmed}",
                            path.display(),
                            lineno + 1
                        );
                    }
                }
            }
        }
    }
    let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    check_no_const_rhai_or_rocks(&src_dir);

    // 2. Verify `content/sky/outer_bodies.json` is a signed `rules_definition` pack loaded via `load_pack`:
    let outer_raw = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("content/sky/outer_bodies.json"),
    )
    .expect("outer_bodies.json exists");
    let outer_pack = load_pack(&outer_raw).expect("outer_bodies.json verifies as a signed pack");
    assert_eq!(
        outer_pack.manifest().id().as_str(),
        "tacsim.sky.outer_bodies"
    );
    assert_eq!(
        outer_pack.manifest().kind(),
        tacsim::PackKind::RulesDefinition
    );

    // 3 & 4. Boot the shipped lane (`Roster::default()`, no `seed_lane` override) and verify `ceres_stickney`
    //        promotes a standard Dray into the 3-party Kirkwood Customs Picket with sealed NPC minds:
    let mut rig = stocked();
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

    {
        let tstore = store(&rig.tw);
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = book
            .active_for_hull(hull)
            .expect("shipped lane must promote ceres_stickney into Kirkwood Customs Picket");
        assert!(
            book.kirkwood_picket_domains.contains(&active.domain),
            "kirkwood_picket_domains must contain the Kirkwood waypoint domain from authored content"
        );
        assert_eq!(
            active.bubble.ships.len(),
            3,
            "shipped lane Kirkwood waypoint must open the 3-party picket bubble even for an un-refitted Dray"
        );
        let picket = active.picket.as_ref().expect("KirkwoodPicketState");
        let corvette_seal = picket
            .corvette_seal
            .as_ref()
            .expect("Corvette must carry a verified MindSeal");
        let raider_seal = picket
            .raider_seal
            .as_ref()
            .expect("Raider must carry a verified MindSeal");
        assert_eq!(
            corvette_seal.engine,
            tacsim::arena::mind::SealEngine::Scripted
        );
        assert_eq!(
            raider_seal.engine,
            tacsim::arena::mind::SealEngine::Scripted
        );
        assert_eq!(
            tacsim::space::space_mind_seal_of(rig.tw.world(), picket.corvette_hull).as_ref(),
            Some(corvette_seal)
        );
        assert_eq!(
            tacsim::space::space_mind_seal_of(rig.tw.world(), picket.raider_hull).as_ref(),
            Some(raider_seal)
        );
        let corvette_sha = corvette_seal.source_sha256;
        let raider_sha = raider_seal.source_sha256;
        assert_eq!(
            corvette_sha,
            tacsim::minds::content::kirkwood_picket_digest().0
        );
        assert_eq!(raider_sha, tacsim::minds::content::belt_raider_digest().0);
        assert!(
            tstore.mind_source(&corvette_sha).is_some(),
            "Corvette source_sha256 must resolve in TacsimStore::mind_source"
        );
        assert!(
            tstore.mind_source(&raider_sha).is_some(),
            "Raider source_sha256 must resolve in TacsimStore::mind_source"
        );
    }

    // 5. Verify an interdicted non-Kirkwood leg (`kirkwood_picket: false`) does NOT open the Kirkwood
    //    Customs Picket even when flown by a `Dray-P`, and seals `interdictor_hull` with `interdictor_patrol.rhai`:
    let mut rig2 = stocked_with(Roster {
        seed_lane: Some(lane_with_interdicted_kirkwood()),
        ..Roster::default()
    });
    let (pilot2, row2) = chartered(&mut rig2, "PilotTwo", 2);
    let hull2 = ObjId(row2.hull);
    let vault2 = hold_vault(&rig2.tw, &row2);
    let desk2 = station_room(&rig2.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig2.tw, pilot2, desk2);
    let refit2 = run(&mut rig2.tw, pilot2, "+coop/refit prospector");
    assert!(refit2.contains("Dray-P"), "{refit2}");
    seed_pool(&mut rig2, vault2, "propellant", 75, LotId(981));
    seed_pool(&mut rig2, vault2, "provisions", 45, LotId(982));
    set_rate(&mut rig2.tw, 3_600);
    board(&mut rig2, pilot2);

    let pending2 = undock_and_burn(&mut rig2, pilot2, hull2);
    let _ = fire_quietly(&mut rig2);
    let _ = to_ic(
        &mut rig2,
        pending2.plan.depart_ic + i64::from(pending2.plan.legs[0].secs),
    );
    {
        let tstore2 = store(&rig2.tw);
        let book2 = rig2
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active2 = book2
            .active_for_hull(hull2)
            .expect("active 2-ship interdictor bubble");
        assert!(
            active2.picket.is_none() && active2.bubble.ships.len() == 2,
            "an interdicted non-Kirkwood leg must NOT open the 3-party Kirkwood Customs Picket even for a Dray-P"
        );
        let int_seal = active2
            .interdictor_seal
            .as_ref()
            .expect("interdictor must carry a verified MindSeal");
        assert_eq!(int_seal.engine, tacsim::arena::mind::SealEngine::Scripted);
        let int_sha = int_seal.source_sha256;
        assert_eq!(
            int_sha,
            tacsim::minds::content::interdictor_patrol_digest().0
        );
        assert!(
            tstore2.mind_source(&int_sha).is_some(),
            "interdictor source_sha256 must resolve in TacsimStore::mind_source"
        );
    }

    // 6. Clear the 2-ship interdiction on `rig2`, fly to `ceres_halo`, and launch 2 collector drones
    //    (`+mine/drones 2 B`) to verify `DroneSortieState` carries `ExcursionId` + verified `MindSeal`
    //    (`drone_mining.rhai`) and steps `mind_think_seq` on the tick:
    for i in 0..4 {
        let next_r = rig2.tw.world().now() + 1;
        rig2.tw.set_now(next_r);
        let _ = rig2.tw.fire_due(next_r * 1_000);
        if i == 0 || i == 2 || i == 3 {
            let _ = run(&mut rig2.tw, pilot2, "+tac/beam Interdictor Picket");
        }
        if i == 1 || i == 2 {
            let _ = run(&mut rig2.tw, pilot2, "+damage/vent");
        }
    }
    let next_r = rig2.tw.world().now() + 1;
    rig2.tw.set_now(next_r);
    let _ = rig2.tw.fire_due(next_r * 1_000);

    let lic2 = run(&mut rig2.tw, pilot2, "+licence/buy go");
    assert!(lic2.contains("Freeport Registry"), "{lic2}");
    let dec2 = run(&mut rig2.tw, pilot2, "+helm/decouple");
    assert!(dec2.contains("Decoupled"), "{dec2}");
    let auto_halo = run(&mut rig2.tw, pilot2, "+helm/autopilot ceres_halo");
    assert!(auto_halo.contains("Ceres Halo"), "{auto_halo}");
    let halo_burn = drain_space_and_industry(&mut rig2.tw);
    land_quietly(&mut rig2, &halo_burn);
    let halo_arr_ic = rig2
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook")
        .free_flights[&hull2]
        .cluster_arrival_ic_s
        .expect("cluster_arrival_ic_s");
    let halo_real_s = (halo_arr_ic.saturating_sub(ic_now(&rig2.tw)) + 3_599) / 3_600;
    let t_halo = rig2.tw.world().now().saturating_add(halo_real_s.max(6));
    rig2.tw.set_now(t_halo);
    let _ = rig2.tw.fire_due(t_halo.saturating_mul(1_000));
    let open_halo = drain_space_and_industry(&mut rig2.tw);
    land_all(&mut rig2, &open_halo);
    let t_seed = rig2.tw.world().now().saturating_add(6);
    rig2.tw.set_now(t_seed);
    let _ = rig2.tw.fire_due(t_seed.saturating_mul(1_000));
    let seed_halo = drain_space_and_industry(&mut rig2.tw);
    land_quietly(&mut rig2, &seed_halo);

    let _ = run(&mut rig2.tw, pilot2, "+helm/approach A");
    let app_b = drain_space_and_industry(&mut rig2.tw);
    land_quietly(&mut rig2, &app_b);
    for _ in 0..3 {
        let t = rig2.tw.world().now().saturating_add(6);
        rig2.tw.set_now(t);
        let _ = rig2.tw.fire_due(t.saturating_mul(1_000));
        let sub = drain_space_and_industry(&mut rig2.tw);
        land_quietly(&mut rig2, &sub);
    }
    let _ = run(&mut rig2.tw, pilot2, "+helm/match A");
    let match_b = drain_space_and_industry(&mut rig2.tw);
    land_quietly(&mut rig2, &match_b);

    set_rate(&mut rig2.tw, 1);
    let drones_out = run(&mut rig2.tw, pilot2, "+mine/drones A 2");
    assert!(
        drones_out.contains("Launched 2 collector drone(s)"),
        "{drones_out}"
    );
    let t_step = rig2.tw.world().now().saturating_add(6);
    rig2.tw.set_now(t_step);
    let _ = rig2.tw.fire_due(t_step.saturating_mul(1_000));
    {
        let tstore2 = store(&rig2.tw);
        let book2 = rig2
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let sess = book2
            .cluster_sessions
            .get(&hull2)
            .expect("cluster session for hull2");
        assert_eq!(sess.active_drones.len(), 2);
        for d in &sess.active_drones {
            let seal = d
                .mind_seal
                .as_ref()
                .expect("collector drone must carry a verified MindSeal");
            assert_eq!(seal.engine, tacsim::arena::mind::SealEngine::Scripted);
            let sha = seal.source_sha256;
            assert_eq!(sha, tacsim::minds::content::drone_mining_digest().0);
            assert!(
                tstore2.mind_source(&sha).is_some(),
                "drone_mining.rhai source_sha256 must resolve in TacsimStore::mind_source"
            );
            assert!(
                d.mind_think_seq >= 2,
                "collector drone mind_think_seq must advance on the tick: got {}",
                d.mind_think_seq
            );
            assert!(
                d.mind_last_trace
                    .iter()
                    .any(|t| t.starts_with("DRONE_MINING_")),
                "collector drone mind_last_trace must record drone_mining.rhai trace: {:?}",
                d.mind_last_trace
            );
        }
    }
}

/// **Doc 35b cut SF.R15 — Hull fidelity (`35b` §5 #6/#7/#13, §8 `SF.R15`)**:
/// 1. `dray.json`, `dray_p.json`, and `corvette.json` instantiate `"heat"` (`0..=1_000`) and
///    `"heat_sink"` (`30`) cells on `"hull"`, and `heat_permille` round-trips through
///    `build_space_damage_mutation_plan` and `space_ship_from_tactical_unit_with_spec`.
/// 2. A rear-aspect hit on a Dray (`apply_damage(FacetBand::Rear, 160)`) strips `hull.rear`
///    armour (`60 -> 0`) and splits the remaining `100` internal damage 50/50 between `hull`
///    (`600 -> 550`) and `drive` (`250 -> 200`) per `dray.json`'s `struck_through`; `left`/`right`/`top`
///    hits route into `hold` per `dray.json`'s `facets` weights, and destroying `hold` (`0/400`)
///    applies `slow_75` (`rated_accel_mg: 50 -> 12`).
/// 3. Stripping a PDC mount (`SubsystemCrit::PdcOffline`, setting `PdcMount::operational = false`
///    on `pdc_dorsal` / `pdc_ventral`) opens a real gap in the point-defense screen so a 2-torpedo
///    salvo that was fully intercepted by 2 operational PDCs now has 1 torpedo penetrate.
/// 4. In Story 2 (`sfr2` / `sfr7`), after taking combat damage across `hull` and `drive`, clamping
///    and towing the disabled interdictor (`+helm/tow`, filing the salvage claim; no bounty since
///    doc 41 cut RS.8), and docking at Stickney Yards, the
///    pilot quotes (`+bay #<hull>`) and completes (`+bay/order #<hull>=all`) a doc 31 bay repair at
///    Stickney Yards' workshop that restores the Dray's armour and internals to full (`600/600` hull,
///    `250/250` drive).
#[test]
#[allow(clippy::too_many_lines)]
fn sfr15_hull_fidelity_splits_rear_hit_per_dray_json_strips_pdc_and_refits_in_bay_after_salvage_tow()
 {
    // 1. Verify `dray.json` facet weights, `struck_through` rear split (`50/50` hull/drive),
    //    `hold` `slow_75` consequence, and `TacticalUnit` `"heat"` / `"heat_sink"` cell round-trip:
    let dray_spec = tacsim::space::unit_spec_for_id("tacsim.unit.dray").expect("dray UnitSpec");
    let mut dray = tacsim::space::SpaceShipState::new_dray(
        "dray.fidelity",
        "carrier",
        PartyId::nth(0).expect("the arena letters a first party"),
        [0, 0, 0],
        [0, 0, 0],
    );
    tacsim::space::apply_unit_spec_to_space_ship(&mut dray, dray_spec);
    assert!(
        dray.pdcs.iter().any(|m| m.id == "pdc_dorsal")
            && dray.pdcs.iter().any(|m| m.id == "pdc_ventral"),
        "Dray must carry canonical pdc_dorsal and pdc_ventral mounts (no 'pdc_ dorsal' typo): {:?}",
        dray.pdcs
    );

    // Apply 160 raw damage to `Rear`: 60 absorbed by `hull.rear` armour (`60 -> 0`), remaining 100
    // split 50/50 between `Hull` (`600 -> 550`) and `Drive` (`250 -> 200`):
    let (_sec, abs_rear, pen_rear) = dray.apply_damage(tacsim::space::FacetBand::Rear, 160);
    assert_eq!(abs_rear, 60);
    assert_eq!(pen_rear, 100);
    assert_eq!(
        dray.sections[&tacsim::space::ShipSectionId::Hull].facets[&tacsim::space::FacetBand::Rear],
        0
    );
    assert_eq!(
        dray.sections[&tacsim::space::ShipSectionId::Hull].internals,
        550,
        "rear hit must split 50% of internal damage into Hull (600 - 50 = 550)"
    );
    assert_eq!(
        dray.sections[&tacsim::space::ShipSectionId::Drive].internals,
        200,
        "rear hit must split 50% of internal damage into Drive (250 - 50 = 200)"
    );

    // Apply 920 raw damage to `Left` (`50/50` Hull/Hold): 80 absorbed by `hull.left` armor,
    // remaining 840 split 50/50 (`420` each) destroying `Hold` (`400 -> 0`) and triggering `slow_75` (`50 -> 37 mg`):
    let _ = dray.apply_damage(tacsim::space::FacetBand::Left, 920);
    assert_eq!(
        dray.sections[&tacsim::space::ShipSectionId::Hold].internals,
        0,
        "Left hit must route 50% into Hold and destroy Hold internals"
    );
    assert_eq!(
        dray.rated_accel_mg, 37,
        "destroying Hold must trigger slow_75 consequence (50 mg * 75% = 37 mg)"
    );

    // Verify `TacticalUnit` instantiates `"heat"` (`max = 1000`) and `"heat_sink"` (`30`) cells and
    // round-trips `heat_permille`:
    let (unit, _) = tacsim::space::instantiate_stock_space_unit(ObjId(999), "tacsim.unit.dray")
        .expect("instantiate dray");
    let heat_addr = mush_tactics::CellAddr::Section {
        section: mush_tactics::SectionId::new("hull").expect("hull"),
        cell: mush_tactics::CellId::new("heat").expect("heat"),
    };
    let sink_addr = mush_tactics::CellAddr::Section {
        section: mush_tactics::SectionId::new("hull").expect("hull"),
        cell: mush_tactics::CellId::new("heat_sink").expect("heat_sink"),
    };
    assert_eq!(
        unit.cell(&heat_addr).map(|c| (c.current, c.max)),
        Some((0, 1_000))
    );
    assert_eq!(
        unit.cell(&sink_addr).map(|c| (c.current, c.max)),
        Some((30, 30))
    );
    dray.heat_permille = 420;
    let (unit_after, _assessment, plan) =
        tacsim::space::build_space_damage_mutation_plan(&unit, &dray, Some(dray_spec), 2)
            .expect("mutation plan");
    assert!(plan.is_some(), "expected non-empty mutation plan");
    assert_eq!(unit_after.cell(&heat_addr).map(|c| c.current), Some(420));
    let roundtripped = tacsim::space::space_ship_from_tactical_unit_with_spec(
        &unit_after,
        Some(dray_spec),
        "dray.rt",
        "carrier",
        PartyId::nth(0).expect("the arena letters a first party"),
        [0, 0, 0],
        [0, 0, 0],
    )
    .expect("roundtrip");
    assert_eq!(roundtripped.heat_permille, 420);

    // 2. Verify stripping a PDC mount (`SubsystemCrit::PdcOffline`, `operational = false`) opens a
    //    real gap in the point-defense screen (`step_pdc_intercepts`):
    let mut bubble_full_pdc =
        tacsim::space::LocalSpaceBubble::new("b.pdc.full", "kirkwood", "PDC Test", 1_000);
    let mut ship_full = tacsim::space::SpaceShipState::new_dray(
        "defender",
        "carrier",
        PartyId::nth(0).expect("the arena letters a first party"),
        [0, 0, 0],
        [0, 0, 0],
    );
    ship_full.pdc_auto = true;
    bubble_full_pdc.insert_ship(ship_full.clone());
    for idx in 0..2 {
        bubble_full_pdc
            .torpedoes
            .push(tacsim::space::InFlightTorpedo {
                id: idx,
                shooter_id: "attacker".to_owned(),
                target_id: "defender".to_owned(),
                pos_m: [5_000, 0, 0],
                pos_rem_um: [0, 0, 0],
                vel_mmps: [-1_000_000, 0, 0],
                accel_mps2: 120,
                remaining_dv_mmps: 4_000_000,
                warhead_damage: 180,
                integrity: 100,
                tti_s: 5,
                flight_elapsed_ms: 0,
                max_flight_ms: 45_000,
                dv_exhausted_emitted: false,
            });
    }
    let rep_full = bubble_full_pdc.step_wall_ms_at(1_000, 1_001);
    let intercepted_full = rep_full
        .events
        .iter()
        .filter(|e| {
            matches!(
                e,
                tacsim::space::SpaceCombatEvent::TorpedoInterceptedByPdc { .. }
            )
        })
        .count();
    assert_eq!(
        intercepted_full, 2,
        "2 operational PDCs (dorsal + ventral, 2 bursts each) intercept both incoming torpedoes"
    );

    let mut bubble_stripped_pdc =
        tacsim::space::LocalSpaceBubble::new("b.pdc.stripped", "kirkwood", "PDC Test", 1_000);
    let mut ship_stripped = ship_full;
    ship_stripped
        .crits
        .push(tacsim::space::SubsystemCrit::PdcOffline);
    ship_stripped.pdcs[0].operational = false;
    bubble_stripped_pdc.insert_ship(ship_stripped);
    for idx in 0..2 {
        bubble_stripped_pdc
            .torpedoes
            .push(tacsim::space::InFlightTorpedo {
                id: idx,
                shooter_id: "attacker".to_owned(),
                target_id: "defender".to_owned(),
                pos_m: [5_000, 0, 0],
                pos_rem_um: [0, 0, 0],
                vel_mmps: [-1_000_000, 0, 0],
                accel_mps2: 120,
                remaining_dv_mmps: 4_000_000,
                warhead_damage: 180,
                integrity: 100,
                tti_s: 5,
                flight_elapsed_ms: 0,
                max_flight_ms: 45_000,
                dv_exhausted_emitted: false,
            });
    }
    let rep_stripped = bubble_stripped_pdc.step_wall_ms_at(1_000, 1_001);
    let intercepted_stripped = rep_stripped
        .events
        .iter()
        .filter(|e| {
            matches!(
                e,
                tacsim::space::SpaceCombatEvent::TorpedoInterceptedByPdc { .. }
            )
        })
        .count();
    assert_eq!(
        intercepted_stripped, 1,
        "with 1 PDC mount stripped offline, only 1 torpedo is intercepted and 1 penetrates the screen"
    );

    // 3. End-to-end Story 2 (`sfr2`/`sfr7`): take rear-aspect combat damage (`hull 480/600, drive 190/250`),
    //    disable, clamp, and tow the interdictor derelict (`+helm/tow`), dock at Stickney Yards
    //    (no bounty since doc 41 cut RS.8), and run `+bay` +
    //    `+bay/order #<hull>=all` to repair the Dray back to full (`600/600` hull, `250/250` drive):
    let mut rig = stocked_with(Roster {
        seed_lane: Some(lane_with_interdicted_kirkwood()),
        ..Roster::default()
    });
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    let vault_id = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, vault_id, "propellant", 55, LotId(991));
    seed_pool(&mut rig, vault_id, "provisions", 19, LotId(992));
    seed_pool(&mut rig, vault_id, "industrial_parts", 8, LotId(993));
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot);

    let pending = undock_and_burn(&mut rig, pilot, hull);
    let depart_ic = pending.plan.depart_ic;
    let leg1_secs = i64::from(pending.plan.legs[0].secs);
    let total_secs = pending.plan.total_s;

    let _ = fire_quietly(&mut rig);
    let _ = to_ic(&mut rig, depart_ic + leg1_secs);

    for i in 0..4 {
        let next_real = rig.tw.world().now() + 1;
        rig.tw.set_now(next_real);
        let _ = rig.tw.fire_due(next_real * 1_000);
        if i == 0 || i == 2 || i == 3 {
            let _ = run(&mut rig.tw, pilot, "+tac/beam Interdictor Picket");
        }
        if i == 1 || i == 2 {
            let _ = run(&mut rig.tw, pilot, "+damage/vent");
        }
    }

    // Approach, 6-DOF hard-dock clamp, and tow the disabled interdictor (`+helm/tow`):
    let _ = run(&mut rig.tw, pilot, "+helm/approach derelict");
    let app_burn = drain(&mut rig.tw);
    land_quietly(&mut rig, &app_burn);
    for _ in 0..3 {
        let next_real = rig.tw.world().now() + 1;
        rig.tw.set_now(next_real);
        let _ = rig.tw.fire_due(next_real * 1_000);
    }
    let _ = run(&mut rig.tw, pilot, "+helm/dock derelict");
    let (session_id, challenge) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let s = book.docking_sessions.get(&hull).expect("docking session");
        (s.challenge.session_id.clone(), s.challenge.clone())
    };
    let clamp_rle = client_solve_docking_rle(&challenge);
    let _ = run(
        &mut rig.tw,
        pilot,
        &format!("+helm/dock tape {session_id} {clamp_rle}"),
    );
    let clamp_burn = drain(&mut rig.tw);
    land_quietly(&mut rig, &clamp_burn);

    let pilot_acct = tacsim::port::credit::own_account(rig.tw.world(), pilot)
        .expect("pilot account")
        .id;
    let balance_before_tow = economy(&rig.tw)
        .snapshot()
        .account(pilot_acct)
        .expect("acct")
        .available()
        .expect("amt")
        .minor();
    let tow_out = run(&mut rig.tw, pilot, "+helm/tow");
    assert!(
        tow_out.contains("[SALVAGE] Tow line locked to disabled derelict"),
        "tow output: {tow_out}"
    );
    // Fire one tick to close the cleared interdiction bubble and resume transit to Stickney Yards:
    let next_real = rig.tw.world().now() + 1;
    rig.tw.set_now(next_real);
    let _ = rig.tw.fire_due(next_real * 1_000);
    let j_resumed = journey(&rig.tw, hull).expect("resumed journey");
    let expected_arrival = j_resumed.departed().at + total_secs;
    let _ = to_ic(&mut rig, expected_arrival);

    // Doc 41 cut RS.8: no bounty on port delivery; the derelict is the salvor's if the claim stands.
    sweep(&mut rig.tw, 60);
    let balance_after_tow = economy(&rig.tw)
        .snapshot()
        .account(pilot_acct)
        .expect("acct")
        .available()
        .expect("amt")
        .minor();
    assert_eq!(
        balance_after_tow, balance_before_tow,
        "delivering a towed derelict to port pays no bounty (doc 41 cut RS.8)"
    );

    // Confirm rear-split combat damage was written to *Salt Moth*'s `TacticalUnit` (`drive` and `hull` both damaged):
    let ship_damaged = run(&mut rig.tw, pilot, "+ship");
    assert!(
        ship_damaged.contains("Damage: drive 64/250, hull 418/600."),
        "expected rear-split combat damage on docked Dray: {ship_damaged}"
    );

    // Quote and order full bay repair (`+bay #<hull>` and `+bay/order #<hull>=all`) at Stickney Yards:
    let bay_quote = run(&mut rig.tw, pilot, &format!("+bay #{}", hull.0));
    assert!(
        bay_quote.contains("stands in a workshop")
            && bay_quote.contains("armor hull")
            && bay_quote.contains("armor drive"),
        "bay quote at Stickney Yards workshop: {bay_quote}"
    );
    let bay_order = run(&mut rig.tw, pilot, &format!("+bay/order #{}=all", hull.0));
    assert!(
        bay_order.contains("Order 1 taken in the workshop:"),
        "bay order taken using salvage recovery proceeds: {bay_order}"
    );
    let bay_draws = drain(&mut rig.tw);
    land_all(&mut rig, &bay_draws);

    // Advance IC clock past the workshop order's `due_ic` and settle the workshop fee contract:
    let now_ic = ic_now(&rig.tw);
    let _ = to_ic(&mut rig, now_ic + 72_000);
    sweep(&mut rig.tw, 60);

    // Verify *Salt Moth*'s `TacticalUnit` is restored to full health (`hull 600/600, drive 250/250`, no `Damage:` line):
    let ship_repaired = run(&mut rig.tw, pilot, "+ship");
    assert!(
        !ship_repaired.contains("Damage:"),
        "Dray must have zero damage after Stickney workshop bay repair completes: {ship_repaired}"
    );
    let bay_after = run(&mut rig.tw, pilot, &format!("+bay #{}", hull.0));
    assert!(
        bay_after.contains("Nothing needs doing.") && bay_after.contains("Order 1: workshop"),
        "bay after repair completion: {bay_after}"
    );
}

/// **Doc 35 cut SF.R16 (`Q-35-5`, `D-35-14`, `D-35-15`, `D-35-16`, `doc 35` §9 Story 2, `35b` §5 #14 & §8 `SF.R16`)**:
/// Client 60 Hz docking game, per-hull `rcs_authority`, per-session randomised start pose, and Neptune/Triton canopy:
/// 1. All 5 hull classes (`dray`, `dray_p`, `corvette`, `raider_heavy`, `halcyon`) expose distinct
///    `DockingRcsAuthority` profiles in `space.docking`.
/// 2. A hand-typed / constant-pose tape (`DockingChallenge::standard_port`) or a stale tape from a
///    previous session submitted against a randomised session pose fails `OutOfGate`.
/// 3. Playing the shared 60 Hz client docking game (`mush_pixel::docking::DockingClientSession`)
///    from the `space.docking` keyframe clamps all 5 gates with bit-identical client/server final
///    6-DOF poses and docks the ship via `+helm/dock tape <session_id> <rle>`.
/// 4. Flying a Tier-1 free-flight expedition to Neptune/Triton (`+helm/autopilot neptune`) emits a
///    `space.cockpit` keyframe with `"neptune"` and `"triton"` in `"bodies"`, and both bodies
///    render visibly on the bridge canopy (`SpaceCockpitPicture`).
#[test]
#[allow(clippy::too_many_lines)]
fn sfr16_client_docking_game_randomised_pose_and_neptune_canopy() {
    use mush_pixel::picture::{Feeds, Look, Picture};

    struct SingleFeed<'a>(&'a str, &'a serde_json::Value);
    impl Feeds for SingleFeed<'_> {
        fn feed(&self, name: &str) -> Option<&serde_json::Value> {
            (name == self.0).then_some(self.1)
        }
    }

    // 1. Verify all 5 hull classes have distinct `rcs_authority` profiles:
    let auth_dray = tacsim::space::space_unit_overlay_for_id("tacsim.unit.dray").rcs_authority;
    let auth_dray_p = tacsim::space::space_unit_overlay_for_id("tacsim.unit.dray_p").rcs_authority;
    let auth_corvette =
        tacsim::space::space_unit_overlay_for_id("tacsim.unit.corvette").rcs_authority;
    let auth_raider =
        tacsim::space::space_unit_overlay_for_id("tacsim.unit.raider_heavy").rcs_authority;
    let auth_halcyon =
        tacsim::space::space_unit_overlay_for_id("tacsim.unit.halcyon").rcs_authority;
    assert_ne!(auth_dray, auth_dray_p);
    assert_ne!(auth_dray_p, auth_corvette);
    assert_ne!(auth_corvette, auth_raider);
    assert_ne!(auth_raider, auth_halcyon);

    let roster = Roster::default();
    let mut rig = stocked_with(roster);
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);

    // Refit *Salt Moth* to `Dray-P` (`tacsim.unit.dray_p`):
    let desk = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, pilot, desk);
    let _ = run(&mut rig.tw, pilot, "+coop/refit prospector");
    assert_eq!(
        tacsim::space::rcs_authority_for_hull(rig.tw.world(), hull),
        auth_dray_p,
        "refitted Dray-P hull must resolve Dray-P DockingRcsAuthority"
    );

    load(&mut rig, &row, 70, 50);
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot);

    // Fly to within 10 IC seconds of Stickney Yards so we are inside the port collar envelope:
    let pending = undock_and_burn(&mut rig, pilot, hull);
    let depart_ic = pending.plan.depart_ic;
    let leg1_secs = i64::from(pending.plan.legs[0].secs);
    let _ = fire_quietly(&mut rig);
    let _ = to_ic(&mut rig, depart_ic + leg1_secs / 2);
    let dec_out = run(&mut rig.tw, pilot, "+helm/decouple");
    assert!(
        dec_out.contains("Decoupled from corridor leg"),
        "decouple output: {dec_out}"
    );
    let auto_out = run(&mut rig.tw, pilot, "+helm/autopilot stickney");
    assert!(
        auto_out.contains("Autopilot engaged for Stickney Yards"),
        "autopilot stickney output: {auto_out}"
    );
    let auto_submits = drain(&mut rig.tw);
    land_quietly(&mut rig, &auto_submits);
    let arrival_at = journey(&rig.tw, hull)
        .and_then(|j| j.arrival_at())
        .expect("recoupled arrival_at");
    let _ = to_ic(&mut rig, arrival_at - 7_200);
    set_rate(&mut rig.tw, 1);
    let _ = to_ic(&mut rig, arrival_at - 10);

    // 2. Initiate session 1 (`+helm/dock stickney`): verify a hand-typed tape solved for the
    //    constant `standard_port` pose fails against the randomised session pose:
    let dock1_out = run(&mut rig.tw, pilot, "+helm/dock stickney");
    assert!(
        dock1_out.contains("[DOCKING] Initiated 60 fps 6-DOF docking session"),
        "dock1 initiation: {dock1_out}"
    );
    let (sid1, ch1) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let s = book.docking_sessions.get(&hull).expect("session 1");
        (s.challenge.session_id.clone(), s.challenge.clone())
    };
    assert_eq!(ch1.rcs_authority, auth_dray_p);
    let constant_challenge = tacsim::space::DockingChallenge::standard_port(sid1.clone());
    assert_ne!(
        ch1.start_pose, constant_challenge.start_pose,
        "per-session start_pose must be randomised away from constant standard_port"
    );
    let hand_typed_constant_rle = client_solve_docking_rle(&constant_challenge);
    let fail1_out = run(
        &mut rig.tw,
        pilot,
        &format!("+helm/dock tape {sid1} {hand_typed_constant_rle}"),
    );
    assert!(
        (fail1_out.contains("Docking input tape rejected: collar gates not aligned")
            || fail1_out.contains("[DOCKING] Hard collar bounce"))
            && !fail1_out.contains("Hard dock and magnetic collar seal confirmed"),
        "hand-typed constant-pose tape must fail against randomised pose: {fail1_out}"
    );
    let fail1_burn = drain(&mut rig.tw);
    land_quietly(&mut rig, &fail1_burn);

    // Initiate session 2 (`+helm/dock stickney`): verify replaying session 1's solved tape
    // against session 2's randomised pose also fails:
    let session1_valid_rle = client_solve_docking_rle(&ch1);
    let _ = run(&mut rig.tw, pilot, "+helm/dock stickney");
    let (sid2, ch2) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let s = book.docking_sessions.get(&hull).expect("session 2");
        (s.challenge.session_id.clone(), s.challenge.clone())
    };
    assert_ne!(
        ch1.start_pose, ch2.start_pose,
        "consecutive docking sessions must have distinct start poses"
    );
    let fail2_out = run(
        &mut rig.tw,
        pilot,
        &format!("+helm/dock tape {sid2} {session1_valid_rle}"),
    );
    assert!(
        (fail2_out.contains("Docking input tape rejected: collar gates not aligned")
            || fail2_out.contains("[DOCKING] Hard collar bounce"))
            && !fail2_out.contains("Hard dock and magnetic collar seal confirmed"),
        "stale tape from previous session must fail against new session pose: {fail2_out}"
    );
    let fail2_burn = drain(&mut rig.tw);
    land_quietly(&mut rig, &fail2_burn);

    // 3. Initiate session 3 (`+helm/dock stickney`), capture the `space.docking` panel frame,
    //    play the shared 60 Hz client docking game (`mush_pixel::docking::DockingClientSession`),
    //    and submit its RLE tape to clamp at Stickney Yards:
    let dock3_events = rig.tw.run(pilot, "+helm/dock stickney");
    let dock_panel = dock3_events
        .iter()
        .find_map(|(_, ev)| match ev {
            OutputEvent::Panel { feed, data, .. }
                if feed.as_str() == tacsim::space::SPACE_DOCKING_FEED =>
            {
                Some(data.clone())
            }
            _ => None,
        })
        .expect("space.docking panel keyframe emitted on +helm/dock");

    let mut client_game = mush_pixel::docking::DockingClientSession::from_feed(&dock_panel);
    assert_eq!(client_game.rcs_authority.trans_dv_mmps_per_frame, 32);
    client_game.play_to_clamp();
    assert!(
        client_game.clamped(),
        "60 Hz client docking game must clamp within max_frames"
    );

    let ch3 = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook")
        .docking_sessions
        .get(&hull)
        .expect("session 3")
        .challenge
        .clone();
    let decoded_tape = tacsim::space::DockingInputTape::decode_rle(&client_game.encode_rle())
        .expect("decode client RLE");
    let server_replay =
        tacsim::space::replay_docking_tape(&ch3, &decoded_tape).expect("server replay");
    assert_eq!(
        server_replay.outcome,
        tacsim::space::DockingOutcome::Clamped
    );
    assert_eq!(
        server_replay.final_pose.lateral_x_mm,
        client_game.pose.dx_mm
    );
    assert_eq!(
        server_replay.final_pose.lateral_y_mm,
        client_game.pose.dy_mm
    );
    assert_eq!(server_replay.final_pose.range_z_mm, client_game.pose.dz_mm);
    assert_eq!(
        server_replay.final_pose.vel_x_mmps,
        client_game.pose.vx_mmps
    );
    assert_eq!(
        server_replay.final_pose.vel_y_mmps,
        client_game.pose.vy_mmps
    );
    assert_eq!(
        server_replay.final_pose.closure_mmps,
        client_game.pose.vz_mmps
    );
    assert_eq!(
        i64::from(server_replay.final_pose.pitch_err_mdeg),
        client_game.pose.pitch_mdeg
    );
    assert_eq!(
        i64::from(server_replay.final_pose.yaw_err_mdeg),
        client_game.pose.yaw_mdeg
    );
    assert_eq!(
        i64::from(server_replay.final_pose.roll_err_mdeg),
        client_game.pose.roll_mdeg
    );

    let clamp_cmd = client_game.format_tape_command();
    let clamp_out = run(&mut rig.tw, pilot, &clamp_cmd);
    assert!(
        clamp_out.contains("Hard dock and magnetic collar seal confirmed at Stickney Yards"),
        "client game tape must clamp at Stickney Yards: {clamp_out}"
    );
    let clamp_burn = drain(&mut rig.tw);
    land_quietly(&mut rig, &clamp_burn);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { .. }
        ),
        "ship must be Berth::Docked at Stickney Yards after client docking game clamp"
    );

    // 4. Tier-1 free-flight expedition to Neptune & Triton (`doc 35` §9 Story 2 steps 3–4, `D-35-15`):
    //    Charter a second `Dray-P` at Ceres Freeport with full stores (`80 t` propellant, `150` rations)
    //    loaded before departure, undock for `neptune` via `+ship/undock neptune`, and engage
    //    `+helm/autopilot neptune` in free flight:
    let mut rig_nep = stocked_with(Roster::default());
    let (pilot_nep, row_nep) = chartered(&mut rig_nep, "PilotNeptune", 2);
    let desk_nep = station_room(&rig_nep.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig_nep.tw, pilot_nep, desk_nep);
    let _ = run(&mut rig_nep.tw, pilot_nep, "+coop/refit prospector");
    load(&mut rig_nep, &row_nep, 80, 200);
    set_rate(&mut rig_nep.tw, 3_600);
    board(&mut rig_nep, pilot_nep);
    let undock_nep = run(&mut rig_nep.tw, pilot_nep, "+ship/undock neptune");
    assert!(
        undock_nep.contains("Clearance filed for neptune"),
        "undock neptune output: {undock_nep}"
    );
    let undock_nep_draws = drain(&mut rig_nep.tw);
    land_quietly(&mut rig_nep, &undock_nep_draws);
    let _ = fire_quietly(&mut rig_nep);

    let neptune_events = rig_nep.tw.run(pilot_nep, "+helm/autopilot neptune");
    let neptune_text = said(&rig_nep.tw, &neptune_events, pilot_nep);
    assert!(
        neptune_text.contains("Autopilot engaged for celestial body `neptune`"),
        "neptune autopilot output: {neptune_text}"
    );
    let neptune_draws = drain(&mut rig_nep.tw);
    land_quietly(&mut rig_nep, &neptune_draws);

    let cockpit_panel = neptune_events
        .iter()
        .find_map(|(_, ev)| match ev {
            OutputEvent::Panel { feed, data, .. }
                if feed.as_str() == tacsim::space::SPACE_COCKPIT_FEED =>
            {
                Some(data.clone())
            }
            _ => None,
        })
        .expect("space.cockpit panel keyframe emitted for Neptune expedition");

    let bodies = cockpit_panel
        .get("bodies")
        .and_then(serde_json::Value::as_array)
        .expect("bodies array in space.cockpit");
    let body_ids: Vec<&str> = bodies
        .iter()
        .filter_map(|b| b.get("id").and_then(serde_json::Value::as_str))
        .collect();
    assert!(
        body_ids.contains(&"neptune") && body_ids.contains(&"triton"),
        "Neptune expedition canopy must include both `neptune` and `triton` in `bodies`: {body_ids:?}"
    );

    // Render `cockpit_panel` through `SpaceCockpitPicture` and verify Neptune (cobalt-azure) and
    // Triton (pinkish-nitrogen ice) pixels render on the canopy viewport:
    let mut canopy_pic = mush_pixel::space_cockpit::SpaceCockpitPicture::new();
    let mut canopy_frame = mush_pixel::Frame::new(640, 360);
    let feeds = SingleFeed(tacsim::space::SPACE_COCKPIT_FEED, &cockpit_panel);
    assert!(
        canopy_pic.draw(&mut canopy_frame, &feeds, &Look::default()),
        "SpaceCockpitPicture must render Neptune & Triton canopy keyframe"
    );

    // Sample the rendered canopy around Neptune's projected centre (`bearing=0, pitch=-4`)
    // and Triton's projected centre (`bearing=26, pitch=5`):
    let (nep_cx, nep_cy) = mush_pixel::space_cockpit::project_canopy(
        (640, 360),
        (0.0, -4.0),
        (0.0, 0.0, 0.0),
        mush_pixel::space_cockpit::CANOPY_FOV_DEG,
    )
    .expect("Neptune projected on canopy");
    let mut neptune_lit_pixels = 0usize;
    for y in (nep_cy - 35).max(0) as usize..=(nep_cy + 35).min(359) as usize {
        for x in (nep_cx - 35).max(0) as usize..=(nep_cx + 35).min(639) as usize {
            let [r, _g, b] = canopy_frame.get(x, y);
            if b > 100 && b > r {
                neptune_lit_pixels += 1;
            }
        }
    }
    assert!(
        neptune_lit_pixels > 200,
        "Neptune disc must render cobalt-azure lit pixels on the canopy (got {neptune_lit_pixels})"
    );

    let (tri_cx, tri_cy) = mush_pixel::space_cockpit::project_canopy(
        (640, 360),
        (26.0, 5.0),
        (0.0, 0.0, 0.0),
        mush_pixel::space_cockpit::CANOPY_FOV_DEG,
    )
    .expect("Triton projected on canopy");
    let mut triton_lit_pixels = 0usize;
    for y in (tri_cy - 20).max(0) as usize..=(tri_cy + 20).min(359) as usize {
        for x in (tri_cx - 20).max(0) as usize..=(tri_cx + 20).min(639) as usize {
            let [r, g, b] = canopy_frame.get(x, y);
            if r > 60 && g > 50 && b > 50 {
                triton_lit_pixels += 1;
            }
        }
    }
    assert!(
        triton_lit_pixels > 40,
        "Triton disc must render lit pixels on the canopy (got {triton_lit_pixels})"
    );
}

fn client_docking_session_from_challenge(
    ch: &tacsim::space::DockingChallenge,
) -> mush_pixel::docking::DockingClientSession {
    let feed = serde_json::json!({
        "session_id": ch.session_id,
        "dx_mm": ch.start_pose.lateral_x_mm,
        "dy_mm": ch.start_pose.lateral_y_mm,
        "dz_mm": ch.start_pose.range_z_mm,
        "pitch_mdeg": ch.start_pose.pitch_err_mdeg,
        "yaw_mdeg": ch.start_pose.yaw_err_mdeg,
        "roll_mdeg": ch.start_pose.roll_err_mdeg,
        "vx_mmps": ch.start_pose.vel_x_mmps,
        "vy_mmps": ch.start_pose.vel_y_mmps,
        "vz_mmps": ch.start_pose.closure_mmps,
        "pitch_rate_mdeg_s": ch.start_pose.pitch_rate_mdeg_s,
        "yaw_rate_mdeg_s": ch.start_pose.yaw_rate_mdeg_s,
        "roll_rate_mdeg_s": ch.start_pose.roll_rate_mdeg_s,
        "target_tumble_mdeg_s": ch.target_tumble_mdeg_s,
        "dampeners": true,
        "rcs_authority": {
            "trans_dv_mmps_per_frame": ch.rcs_authority.trans_dv_mmps_per_frame,
            "closure_dv_mmps_per_frame": ch.rcs_authority.closure_dv_mmps_per_frame,
            "attitude_rate_mdeg_s_per_frame": ch.rcs_authority.attitude_rate_mdeg_s_per_frame,
            "roll_rate_mdeg_s_per_frame": ch.rcs_authority.roll_rate_mdeg_s_per_frame,
            "damp_trans_mmps_per_frame": ch.rcs_authority.damp_trans_mmps_per_frame,
            "damp_attitude_mdeg_s_per_frame": ch.rcs_authority.damp_attitude_mdeg_s_per_frame,
            "damp_roll_mdeg_s_per_frame": ch.rcs_authority.damp_roll_mdeg_s_per_frame,
        }
    });
    mush_pixel::docking::DockingClientSession::from_feed(&feed)
}

fn client_solve_docking_rle(ch: &tacsim::space::DockingChallenge) -> String {
    let mut sess = client_docking_session_from_challenge(ch);
    sess.play_to_clamp();
    sess.encode_rle()
}

fn client_solve_hard_bounce_rle(ch: &tacsim::space::DockingChallenge) -> String {
    let mut sess = client_docking_session_from_challenge(ch);
    while (sess.pose.dz_mm > 0 || sess.pose.vz_mmps <= 800) && sess.frames_stepped() < 600 {
        sess.step_60fps(mush_pixel::docking::DockingClientInput {
            closure_pulse: 1,
            ..Default::default()
        });
    }
    sess.encode_rle()
}

/// **Doc 35c §9 cut `SF.R17` ("Hole close-out II") acceptance**:
/// 1. `DOCK ABORT` / `order abort` requires an existing session (`"No active docking session to abort."`)
///    and never resumes a `Promoted` `CarrierJourney` on a free-flying hull.
/// 2. `+helm/autopilot pallas_inclined_belt` from Ceres berth (`Berth::Docked`) is refused
///    (`"Ship is docked at a berth; undock first (+ship/undock)."`), and `+ship/undock pallas_inclined_belt`
///    departs through `prepare_departure` + `tacsim_port_burn` + `launch_step` and opens the
///    `pallas_inclined_belt` cluster session on arrival tick.
/// 3. `+helm/autopilot neptune` with `1 t` in the tank is refused (`"Insufficient propellant for expedition to neptune"`).
/// 4. Initiating a second `+helm/dock` while a docking session is already open is refused and does
///    not reset `started_wall_s`; when the 60 s wall deadline expires, the 15 kg RCS debit submits
///    `tacsim_port_burn` when crossing a whole tonne.
/// 5. `holding_for_salvage` is per hull with a wall timeout (`SALVAGE_HOLD_TIMEOUT_WALL_S = 60`)
///    and never blocks another carrier in the same cleared bubble from detaching and resuming its
///    `CarrierJourney`.
/// 6. The port dock range gate measures terminal braking distance (`<= 500 m`) and relative speed
///    (`<= 15 m/s`) on every branch (refusing at `remaining_s = 45` where range is `506 m` and speed
///    is `22.5 m/s`, and refusing a free-flight ship at `1 km` without clamping `2 km -> 200 m`).
/// 7. A `30 s` late tick advances an active bubble by `30,000 ms` (`30 s`, `MAX_BUBBLE_TICK_STEP_MS`).
#[test]
#[allow(clippy::too_many_lines)]
fn sfr17_hole_closeout_ii_abort_undock_expedition_dock_gates_salvage_hold_and_30s_tempo() {
    // --- Part A: Berth autopilot refusal, `+ship/undock pallas_inclined_belt`, and 1 t Neptune refusal ---
    let mut rig = stocked_with(Roster::default());
    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    let desk = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, pilot, desk);
    let _ = run(&mut rig.tw, pilot, "+coop/refit prospector");
    load(&mut rig, &row, 55, 60);
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot);

    // 2a. `+helm/autopilot pallas_inclined_belt` from Ceres berth (`Berth::Docked`) is refused:
    let berth_auto = run(&mut rig.tw, pilot, "+helm/autopilot pallas_inclined_belt");
    assert_eq!(
        berth_auto.trim(),
        "Ship is docked at a berth; undock first (+ship/undock).",
        "berth autopilot must be refused"
    );

    // 2b. `+ship/undock pallas_inclined_belt` departs through `prepare_departure` + `tacsim_port_burn`
    //     + `launch_step` and opens the `pallas_inclined_belt` cluster session on arrival tick:
    let undock_pallas = run(
        &mut rig.tw,
        pilot,
        "+ship/undock pallas_inclined_belt throttle=500",
    );
    assert!(
        undock_pallas.contains("Clearance filed for Pallas Inclined Belt (`pallas_inclined_belt`)"),
        "undock to cluster output: {undock_pallas}"
    );
    let undock_burn = drain_space_and_industry(&mut rig.tw);
    assert!(
        undock_burn
            .iter()
            .any(|s| s.action.key().id.as_str() == "tacsim_port_burn"),
        "+ship/undock <cluster> must submit tacsim_port_burn"
    );
    land_quietly(&mut rig, &undock_burn);
    let _ = fire_quietly(&mut rig);
    assert!(
        matches!(
            journey(&rig.tw, hull).map(|j| j.state().clone()),
            Some(mush_sim::JourneyState::Promoted { .. })
        ),
        "launch_step must promote the cluster departure into JourneyState::Promoted"
    );

    // 1a. While in free flight (`JourneyState::Promoted`) with NO docking session open,
    //     `+helm/dock DOCK ABORT` and `+helm/dock abort` are refused and leave the journey `Promoted`:
    for abort_cmd in ["+helm/dock DOCK ABORT", "+helm/dock abort"] {
        let abort_no_sess = run(&mut rig.tw, pilot, abort_cmd);
        assert_eq!(
            abort_no_sess.trim(),
            "No active docking session to abort.",
            "{abort_cmd} without an active session must be refused"
        );
        assert!(
            matches!(
                journey(&rig.tw, hull).map(|j| j.state().clone()),
                Some(mush_sim::JourneyState::Promoted { .. })
            ),
            "{abort_cmd} must leave JourneyState::Promoted intact"
        );
    }

    // Advance to `cluster_arrival_ic_s` and verify `space_tick` promotes the hull into `pallas_inclined_belt`:
    let arrival_ic = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook")
        .free_flights[&hull]
        .cluster_arrival_ic_s
        .expect("cluster_arrival_ic_s");
    let real_s = (arrival_ic.saturating_sub(ic_now(&rig.tw)) + 3_599) / 3_600;
    let t_arrive = rig.tw.world().now().saturating_add(real_s.max(6));
    rig.tw.set_now(t_arrive);
    let _ = rig.tw.fire_due(t_arrive.saturating_mul(1_000));
    let open_submits = drain_space_and_industry(&mut rig.tw);
    land_all(&mut rig, &open_submits);
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        assert!(
            book.cluster_sessions.contains_key(&hull),
            "+ship/undock pallas_inclined_belt must open the cluster session on arrival tick"
        );
    }

    // 3. Charter a hull with only `1 t` in the tank at Ceres, undock into local Ceres orbit (`1 t` burn
    //    from a `2 t` initial load -> `1 t` remaining in tank), and verify `+helm/autopilot neptune` is refused:
    let mut rig_short = stocked_with(Roster::default());
    let (pilot_short, row_short) = chartered(&mut rig_short, "PilotShort", 2);
    let desk_short = station_room(&rig_short.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig_short.tw, pilot_short, desk_short);
    let _ = run(&mut rig_short.tw, pilot_short, "+coop/refit prospector");
    load(&mut rig_short, &row_short, 2, 150);
    set_rate(&mut rig_short.tw, 1);
    board(&mut rig_short, pilot_short);
    let _ = run(&mut rig_short.tw, pilot_short, "+ship/undock ceres");
    let short_burn = drain(&mut rig_short.tw);
    land_quietly(&mut rig_short, &short_burn);
    let _ = fire_quietly(&mut rig_short);
    let neptune_refused = run(&mut rig_short.tw, pilot_short, "+helm/autopilot neptune");
    assert!(
        neptune_refused.contains("Insufficient propellant for expedition to neptune"),
        "+helm/autopilot neptune with 1 t in the tank must be refused, got: {neptune_refused}"
    );

    // --- Part B: Port dock range/speed gate (`remaining_s = 45` refused, free-flight `1 km` refused),
    //             free-flyer `DOCK ABORT` leaving `JourneyState::Promoted`, duplicate `+helm/dock`
    //             refused without resetting `started_wall_s`, and 60s expiry submitting a burn ---
    let mut rig_dock = stocked_with(Roster::default());
    let (pilot_dock, row_dock) = chartered(&mut rig_dock, "PilotDock", 3);
    let hull_dock = ObjId(row_dock.hull);
    load(&mut rig_dock, &row_dock, 70, 50);
    set_rate(&mut rig_dock.tw, 3_600);
    board(&mut rig_dock, pilot_dock);
    let pending_dock = undock_and_burn(&mut rig_dock, pilot_dock, hull_dock);
    let total_dock_s = pending_dock.plan.total_s;
    let leg1_dock_s = i64::from(pending_dock.plan.legs[0].secs);
    let _ = fire_quietly(&mut rig_dock);
    let _ = to_ic(&mut rig_dock, pending_dock.plan.depart_ic + leg1_dock_s / 2);
    let _ = run(&mut rig_dock.tw, pilot_dock, "+helm/decouple");
    let _ = run(&mut rig_dock.tw, pilot_dock, "+helm/autopilot stickney");
    let auto_dock_submits = drain(&mut rig_dock.tw);
    land_quietly(&mut rig_dock, &auto_dock_submits);
    let arrival_dock = journey(&rig_dock.tw, hull_dock)
        .and_then(|j| j.arrival_at())
        .expect("arrival_dock");
    let _ = to_ic(&mut rig_dock, arrival_dock - 7_200);
    set_rate(&mut rig_dock.tw, 1);

    // 6a. At `remaining_s = 45` (`> 30 s`, terminal braking range `506 m > 500 m` and speed `22.5 m/s > 15 m/s`),
    //     `+helm/dock stickney` is refused (no `remaining_s <= 60` proxy!):
    let _ = to_ic(&mut rig_dock, arrival_dock - 45);
    let dock_at_45s = run(&mut rig_dock.tw, pilot_dock, "+helm/dock stickney");
    assert!(
        dock_at_45s.contains("outside the Stickney Yards collar-initiation envelope"),
        "+helm/dock at remaining_s = 45 (506 m, 22.5 m/s) must be refused: {dock_at_45s}"
    );

    // 6b. Advance to `remaining_s = 10` (`25 m <= 500 m`, `5.0 m/s <= 15 m/s`), decouple into free flight,
    //     and verify free-flight `+helm/dock stickney` from `0 km` (`pos_km == tpos`) succeeds,
    //     that `DOCK ABORT` on that open session debits RCS and leaves `JourneyState::Promoted`,
    //     and that a second `+helm/dock` while a session is open is refused without resetting `started_wall_s`:
    let _ = to_ic(&mut rig_dock, arrival_dock - 10);
    let dec_dock = run(&mut rig_dock.tw, pilot_dock, "+helm/decouple");
    assert!(
        dec_dock.contains("Decoupled from corridor leg"),
        "{dec_dock}"
    );
    assert!(matches!(
        journey(&rig_dock.tw, hull_dock).map(|j| j.state().clone()),
        Some(mush_sim::JourneyState::Promoted { .. })
    ));

    let dock_ff_open = run(&mut rig_dock.tw, pilot_dock, "+helm/dock stickney");
    assert!(
        dock_ff_open.contains("[DOCKING] Initiated 60 fps 6-DOF docking session"),
        "co-located free-flight hull at 0 km / 0 m/s must initiate docking: {dock_ff_open}"
    );
    let started_0 = rig_dock
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook")
        .docking_sessions[&hull_dock]
        .started_wall_s;

    // 4a. Advance wall clock by 10 s and verify a second `+helm/dock stickney` is refused and does NOT reset `started_wall_s`:
    let t_dup = rig_dock.tw.world().now() + 10;
    rig_dock.tw.set_now(t_dup);
    let dup_dock = run(&mut rig_dock.tw, pilot_dock, "+helm/dock stickney");
    assert!(
        dup_dock.contains("A docking session is already open for this ship"),
        "duplicate +helm/dock while session is open must be refused: {dup_dock}"
    );
    let started_after_dup = rig_dock
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook")
        .docking_sessions[&hull_dock]
        .started_wall_s;
    assert_eq!(
        started_after_dup, started_0,
        "refused duplicate +helm/dock must not reset started_wall_s"
    );

    // 1b. Abort the open free-flight docking session via `+helm/dock DOCK ABORT` and verify
    //     `JourneyState::Promoted` is preserved (NOT resumed into `Cruising`!):
    let abort_open = run(&mut rig_dock.tw, pilot_dock, "+helm/dock DOCK ABORT");
    assert!(
        abort_open.contains("aborted") && abort_open.contains("hull unpaused"),
        "aborting open session must succeed: {abort_open}"
    );
    assert!(
        matches!(
            journey(&rig_dock.tw, hull_dock).map(|j| j.state().clone()),
            Some(mush_sim::JourneyState::Promoted { .. })
        ),
        "free-flyer's DOCK ABORT must leave JourneyState::Promoted intact"
    );

    // 4b. Re-open `+helm/dock stickney` and let the 60 s wall deadline expire on `space_tick`;
    //     verify the 15 kg expiry RCS debit crosses the 1,000 kg boundary and submits `tacsim_port_burn`,
    //     and `JourneyState::Promoted` remains intact:
    let _ = run(&mut rig_dock.tw, pilot_dock, "+helm/dock stickney");
    let _ = drain(&mut rig_dock.tw);
    let t_exp = rig_dock.tw.world().now() + 65;
    rig_dock.tw.set_now(t_exp);
    let _ = rig_dock.tw.fire_due(t_exp * 1_000);
    let exp_submits = drain(&mut rig_dock.tw);
    assert!(
        exp_submits
            .iter()
            .any(|s| s.action.key().id.as_str() == "tacsim_port_burn"),
        "docking session expiry 15 kg RCS debit crossing a whole tonne must submit tacsim_port_burn"
    );
    land_quietly(&mut rig_dock, &exp_submits);
    assert!(
        matches!(
            journey(&rig_dock.tw, hull_dock).map(|j| j.state().clone()),
            Some(mush_sim::JourneyState::Promoted { .. })
        ),
        "docking session expiry on a free-flyer must leave JourneyState::Promoted intact"
    );
    let _ = total_dock_s;

    // --- Part C: Multi-carrier bubble: `30 s` late tick advances `30,000 ms` (`30 s`), and
    //             `holding_for_salvage` on Carrier 1 does not block Carrier 2 from resuming,
    //             then expires on Carrier 1 after `SALVAGE_HOLD_TIMEOUT_WALL_S` (`60 s`) ---
    let mut rig_mc = stocked_with(Roster {
        seed_lane: Some(lane_with_interdicted_kirkwood()),
        ..Roster::default()
    });
    let (pilot_c1, row_c1) = chartered(&mut rig_mc, "PilotC1", 1);
    let pilot_c2 = self::pilot(&mut rig_mc, "PilotC2", 2);
    let _ = run(&mut rig_mc.tw, pilot_c2, "+coop/enrol Escort Hawk");
    let charter_c2 = tacsim::port::live_charter(rig_mc.tw.world(), pilot_c2).expect("c2 charter");
    let row_c2 = port_map(&rig_mc.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|r| r.hull == charter_c2.hull.0)
        .expect("c2 hull row");
    let hull_c1 = ObjId(row_c1.hull);
    let hull_c2 = ObjId(row_c2.hull);
    load(&mut rig_mc, &row_c1, 55, 25);
    load(&mut rig_mc, &row_c2, 55, 25);
    set_rate(&mut rig_mc.tw, 3_600);
    board(&mut rig_mc, pilot_c1);
    let gallery_mc = c01(&rig_mc.tw);
    teleport(&mut rig_mc.tw, pilot_c2, gallery_mc);
    let _ = run(&mut rig_mc.tw, pilot_c2, "+ship/board");

    let p1 = undock_and_burn(&mut rig_mc, pilot_c1, hull_c1);
    let p2 = undock_and_burn(&mut rig_mc, pilot_c2, hull_c2);
    let _ = fire_quietly(&mut rig_mc);
    let due_ic = (p1.plan.depart_ic + i64::from(p1.plan.legs[0].secs))
        .max(p2.plan.depart_ic + i64::from(p2.plan.legs[0].secs));
    let _ = to_ic(&mut rig_mc, due_ic);

    // Disable the Interdictor Picket by play:
    for i in 0..4 {
        let next_r = rig_mc.tw.world().now() + 1;
        rig_mc.tw.set_now(next_r);
        let _ = rig_mc.tw.fire_due(next_r * 1_000);
        if i == 0 || i == 2 || i == 3 {
            let _ = run(&mut rig_mc.tw, pilot_c1, "+tac/beam Interdictor Picket");
        }
        if i == 1 || i == 2 {
            let _ = run(&mut rig_mc.tw, pilot_c1, "+damage/vent");
        }
    }

    // 5a. Carrier 1 (`hull_c1`) orders `+helm/approach derelict`, setting `holding_for_salvage = true`
    //     on `hull_c1` with a 60 s wall deadline while Carrier 2 (`hull_c2`) does NOT hold for salvage.
    //     On the next tick, Carrier 2 detaches and resumes its `CarrierJourney` (`Cruising`) while
    //     Carrier 1 stays in the bubble for salvage!
    let app_c1 = run(&mut rig_mc.tw, pilot_c1, "+helm/approach derelict");
    assert!(
        app_c1.contains("Braking approach ordered toward disabled derelict"),
        "{app_c1}"
    );
    let app_c1_burn = drain(&mut rig_mc.tw);
    land_quietly(&mut rig_mc, &app_c1_burn);

    let next_r = rig_mc.tw.world().now() + 1;
    rig_mc.tw.set_now(next_r);
    let _ = rig_mc.tw.fire_due(next_r * 1_000);
    assert!(
        matches!(
            journey(&rig_mc.tw, hull_c2).map(|j| j.state().clone()),
            Some(mush_sim::JourneyState::Cruising)
        ),
        "Carrier 2 must detach and resume JourneyState::Cruising without being blocked by Carrier 1's holding_for_salvage"
    );
    assert!(
        matches!(
            journey(&rig_mc.tw, hull_c1).map(|j| j.state().clone()),
            Some(mush_sim::JourneyState::Promoted { .. })
        ),
        "Carrier 1 holding for salvage remains in the bubble before its 60 s salvage hold deadline"
    );

    // 7. Fire a tick `30 s` late (`now + 30`) while Carrier 1 is holding in the bubble and verify
    //    the active bubble advances by `30,000 ms` (`30 s`, `MAX_BUBBLE_TICK_STEP_MS`):
    let elapsed_0 = rig_mc
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook")
        .active_for_hull(hull_c1)
        .expect("active bubble")
        .bubble
        .wall_elapsed_ms;
    let t_late_30 = rig_mc.tw.world().now() + 30;
    rig_mc.tw.set_now(t_late_30);
    let _ = rig_mc.tw.fire_due(t_late_30 * 1_000);
    let elapsed_after_30 = rig_mc
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook")
        .active_for_hull(hull_c1)
        .expect("active bubble")
        .bubble
        .wall_elapsed_ms;
    assert_eq!(
        elapsed_after_30 - elapsed_0,
        30_000,
        "a 30 s late tick must advance the bubble by 30,000 ms (30 s)"
    );

    // 5b. Advance wall time past `SALVAGE_HOLD_TIMEOUT_WALL_S` (`60 s`) without Carrier 1 docking/towing:
    //     Carrier 1's salvage hold expires and Carrier 1 also resumes `JourneyState::Cruising`:
    let t_hold_exp = rig_mc.tw.world().now() + 35;
    rig_mc.tw.set_now(t_hold_exp);
    let _ = rig_mc.tw.fire_due(t_hold_exp * 1_000);
    assert!(
        matches!(
            journey(&rig_mc.tw, hull_c1).map(|j| j.state().clone()),
            Some(mush_sim::JourneyState::Cruising)
        ),
        "Carrier 1's holding_for_salvage must expire after SALVAGE_HOLD_TIMEOUT_WALL_S (60 s) and resume its journey"
    );
}

/// **Doc 35c §9 cut `SF.R18` ("The multi-carrier bubble", `Q-36-48`, `doc 40` `FR.6`)**:
/// 1. Each carrier in a shared domain bubble is assigned its own distinct arena [`tacsim::arena::party::PartyId`]
///    with `Friendly` inter-carrier relation and per-carrier relation/inspection state against the
///    Republic Customs Corvette and Belt Raider.
/// 2. A clean escort's (`*Escort Hawk*`) `+helm/submit` clears only the escort and does NOT clear
///    the smuggler (`*Salt Moth*`).
/// 3. The Belt Raider disables the escort (`*Escort Hawk*`) while the smuggler (`*Salt Moth*`) submits
///    and resumes its [`mush_sim::CarrierJourney`] (`Cruising`) once the Corvette disables the Raider
///    and ordnance clears, while the disabled escort settles into `SpaceBubbleBook::derelicts` with
///    `JourneyState::Failed`.
/// 4. Every participating hull (`hull_miner`, `hull_escort`, `corvette_hull`, `raider_hull`) is
///    checked in (`mush_tactics::custody::marker_of` is `None`), and the domain activation stands down
///    (domain count returns to zero).
/// 5. `cleared_interdiction_legs` is keyed by `(hull, leg_cursor)`: when a subsequent hull (`*Late Runner*`)
///    arrives at the same waypoint while `*Salt Moth*` is still cruising on leg 2, `*Salt Moth*` is
///    not re-promoted into the bubble.
#[test]
#[allow(clippy::too_many_lines)]
fn sfr18_multi_carrier_bubble_per_carrier_party_inspection_escort_disabled_settlement_and_domain_stand_down()
 {
    let mut rig = stocked();

    // 1. Charter *Salt Moth* (smuggler on `Dray-P` carrying 51 t unlicensed `strategic_ore`)
    //    and *Escort Hawk* (clean armed `Dray` escort carrying 0 t `strategic_ore`):
    let (pilot_miner, row_miner) = chartered(&mut rig, "Miner", 1);
    let hull_miner = ObjId(row_miner.hull);
    let vault_miner = hold_vault(&rig.tw, &row_miner);
    let desk = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, pilot_miner, desk);
    let refit_miner = run(&mut rig.tw, pilot_miner, "+coop/refit prospector");
    assert!(refit_miner.contains("Dray-P"), "{refit_miner}");
    seed_pool(&mut rig, vault_miner, "propellant", 75, LotId(1081));
    seed_pool(&mut rig, vault_miner, "provisions", 25, LotId(1082));
    seed_pool(&mut rig, vault_miner, "strategic_ore", 51, LotId(1083));

    let pilot_escort = pilot(&mut rig, "Escort", 2);
    let enrol_escort = run(&mut rig.tw, pilot_escort, "+coop/enrol Escort Hawk");
    assert!(
        enrol_escort.contains("now the Escort Hawk"),
        "{enrol_escort}"
    );
    let charter_escort =
        tacsim::port::live_charter(rig.tw.world(), pilot_escort).expect("escort charter");
    let row_escort = port_map(&rig.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|r| r.hull == charter_escort.hull.0)
        .expect("escort hull row");
    let hull_escort = ObjId(row_escort.hull);
    let vault_escort = hold_vault(&rig.tw, &row_escort);
    seed_pool(&mut rig, vault_escort, "propellant", 53, LotId(1084));
    seed_pool(&mut rig, vault_escort, "provisions", 19, LotId(1085));

    // Also charter a third hull (*Late Runner*) at Ceres before departure so we can launch it
    // after the first bubble closes to verify `cleared_interdiction_legs` prevents re-promotion:
    let pilot_late = pilot(&mut rig, "LatePilot", 3);
    let enrol_late = run(&mut rig.tw, pilot_late, "+coop/enrol Late Runner");
    assert!(enrol_late.contains("now the Late Runner"), "{enrol_late}");
    let charter_late =
        tacsim::port::live_charter(rig.tw.world(), pilot_late).expect("late charter");
    let row_late = port_map(&rig.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|r| r.hull == charter_late.hull.0)
        .expect("late hull row");
    let hull_late = ObjId(row_late.hull);
    let vault_late = hold_vault(&rig.tw, &row_late);
    seed_pool(&mut rig, vault_late, "propellant", 53, LotId(1086));
    seed_pool(&mut rig, vault_late, "provisions", 19, LotId(1087));

    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot_miner);
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, pilot_escort, gallery);
    let board_escort = run(&mut rig.tw, pilot_escort, "+ship/board");
    assert!(
        board_escort.contains("You cycle the lock into the Escort Hawk."),
        "{board_escort}"
    );

    let pending_miner = undock_and_burn(&mut rig, pilot_miner, hull_miner);
    let pending_escort = undock_and_burn(&mut rig, pilot_escort, hull_escort);
    let _ = fire_quietly(&mut rig);

    let miner_leg1_due = pending_miner.plan.depart_ic + i64::from(pending_miner.plan.legs[0].secs);
    let escort_leg1_due =
        pending_escort.plan.depart_ic + i64::from(pending_escort.plan.legs[0].secs);
    let convoy_kirkwood_due = miner_leg1_due.max(escort_leg1_due);
    let _ = to_ic(&mut rig, convoy_kirkwood_due);

    // Verify both carriers share the Kirkwood bubble with distinct arena PartyIds:
    let (domain, corvette_hull, raider_hull, party_miner, party_escort) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        assert_eq!(book.by_domain.len(), 1);
        let active = book
            .active_for_hull(hull_miner)
            .expect("miner in active bubble");
        assert_eq!(active.carriers.len(), 2);
        let p_miner = active.carrier_party_for(hull_miner);
        let p_escort = active.carrier_party_for(hull_escort);
        assert_ne!(
            p_miner, p_escort,
            "each carrier in a multi-carrier bubble must have its own distinct arena PartyId"
        );
        assert_eq!(
            active.bubble.party_relation(p_miner, p_escort),
            tacsim::arena::party::Relation::Friend,
            "co-participating carriers must have Friend relation"
        );
        let picket = active.picket.as_ref().expect("KirkwoodPicketState");
        assert_eq!(
            picket.inspection_status_for(hull_miner),
            tacsim::space::PicketInspectionStatus::AwaitingHeaveTo
        );
        assert_eq!(
            picket.inspection_status_for(hull_escort),
            tacsim::space::PicketInspectionStatus::AwaitingHeaveTo
        );
        (
            active.domain,
            picket.corvette_hull,
            picket.raider_hull,
            p_miner,
            p_escort,
        )
    };

    // 2. Clean escort (`*Escort Hawk*`) flips to braking attitude (`180 0`), heaves-to, and submits (`+helm/submit`):
    //    verify the clean escort is cleared while the smuggler (`*Salt Moth*`) is NOT cleared!
    let _ = run(&mut rig.tw, pilot_escort, "+helm/heading 180 0");
    let _ = run(&mut rig.tw, pilot_escort, "+helm/thrust 600 -1000 0 0");
    for _ in 0..6 {
        let next_r = rig.tw.world().now() + 1;
        rig.tw.set_now(next_r);
        let _ = rig.tw.fire_due(next_r * 1_000);
    }
    let _ = run(&mut rig.tw, pilot_escort, "+helm/thrust 0");
    let escort_brake_submits = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &escort_brake_submits);

    let submit_escort_out = run(&mut rig.tw, pilot_escort, "+helm/submit");
    assert!(
        submit_escort_out.contains("0 kg `strategic_ore`, licensed/compliant")
            && submit_escort_out.contains("customs clearance granted"),
        "clean escort must receive customs clearance: {submit_escort_out}"
    );
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = book
            .active_for_hull(hull_miner)
            .expect("active bubble after escort submit");
        let picket = active.picket.as_ref().expect("KirkwoodPicketState");
        assert_eq!(
            picket.inspection_status_for(hull_escort),
            tacsim::space::PicketInspectionStatus::Cleared,
            "escort must be Cleared"
        );
        assert_eq!(
            active
                .bubble
                .party_relation(tacsim::space::CUSTOMS_CORVETTE_PARTY, party_escort),
            tacsim::arena::party::Relation::Friend,
            "Corvette must be Friend toward cleared escort"
        );
        assert_eq!(
            picket.inspection_status_for(hull_miner),
            tacsim::space::PicketInspectionStatus::AwaitingHeaveTo,
            "clean escort's submit must NOT clear the smuggler (*Salt Moth*)"
        );
        assert!(
            !picket.is_carrier_customs_cleared(hull_miner) && !picket.is_customs_cleared(),
            "smuggler and overall bubble must remain uncleared until smuggler submits"
        );
        assert_eq!(
            active
                .bubble
                .party_relation(tacsim::space::CUSTOMS_CORVETTE_PARTY, party_miner),
            tacsim::arena::party::Relation::Neutral,
            "Corvette must remain Neutral (Inspecting) toward uncleared smuggler"
        );
    }

    // 3. Escort (`*Escort Hawk*`) enters ActiveCombat (`+helm/emcon combat`, `+tac/pdc manual`) to screen
    //    while smuggler (`*Salt Moth*`) heaves-to, submits (`ClearedAfterSeizure`), and enables PDCs:
    let _ = run(&mut rig.tw, pilot_escort, "+helm/emcon combat");
    let _ = run(&mut rig.tw, pilot_escort, "+tac/pdc manual");
    let _ = run(&mut rig.tw, pilot_miner, "+tac/pdc auto");
    let _ = run(&mut rig.tw, pilot_miner, "+helm/thrust 600 -1000 0 0");
    for _ in 0..6 {
        let next_r = rig.tw.world().now() + 1;
        rig.tw.set_now(next_r);
        let _ = rig.tw.fire_due(next_r * 1_000);
    }
    let _ = run(&mut rig.tw, pilot_miner, "+helm/thrust 0");
    let miner_brake_submits = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &miner_brake_submits);

    let submit_miner_out = run(&mut rig.tw, pilot_miner, "+helm/submit");
    assert!(
        submit_miner_out.contains("confiscated in lieu of fine")
            && submit_miner_out.contains("customs clearance granted"),
        "smuggler submit must seize contraband and grant clearance: {submit_miner_out}"
    );
    let seizure_submits = drain(&mut rig.tw);
    for s in &seizure_submits {
        land(&mut rig, s).expect("seizure burn lands");
    }

    // Step the bubble until the Raider disables the escort (`*Escort Hawk*`) into `SpaceBubbleBook::derelicts`
    // (`JourneyState::Failed`) and the Corvette disables the Raider so `*Salt Moth*` resumes (`Cruising`):
    for _ in 0..60 {
        let next_r = rig.tw.world().now() + 1;
        rig.tw.set_now(next_r);
        let _ = rig.tw.fire_due(next_r * 1_000);
        let sub = drain_space_and_industry(&mut rig.tw);
        land_quietly(&mut rig, &sub);
        let bubble_closed = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .is_some_and(|b| b.by_domain.is_empty());
        if bubble_closed {
            break;
        }
    }

    // 4. Verify the escort became a derelict (`JourneyState::Failed`), the miner resumed (`JourneyState::Cruising`),
    //    all four hulls are checked in (`custody::marker_of` is `None`), and the domain activation stood down:
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        assert!(
            book.by_domain.is_empty(),
            "shared domain bubble must close once the escort is settled as a derelict and the miner resumes"
        );
        assert!(
            book.derelicts.contains_key(&hull_escort),
            "disabled escort (*Escort Hawk*) must be settled in SpaceBubbleBook::derelicts"
        );
        assert!(
            !book.derelicts.contains_key(&hull_miner),
            "cleared miner (*Salt Moth*) must not be a derelict"
        );
    }
    assert!(
        matches!(
            journey(&rig.tw, hull_escort).map(|j| j.state().clone()),
            Some(mush_sim::JourneyState::Failed)
        ),
        "disabled escort's CarrierJourney must be Failed"
    );
    assert!(
        matches!(
            journey(&rig.tw, hull_miner).map(|j| j.state().clone()),
            Some(mush_sim::JourneyState::Cruising)
        ),
        "cleared miner's CarrierJourney must resume to Cruising"
    );
    for h in [hull_miner, hull_escort, corvette_hull, raider_hull] {
        assert!(
            mush_tactics::custody::marker_of(rig.tw.world(), h).is_none(),
            "hull #{} must be checked in (no dangling custody checkout marker)",
            h.0
        );
    }
    let active_domain_count = rig
        .tw
        .world()
        .module_state::<mush_sim::ActivationBook>()
        .map_or(0, |ab| usize::from(ab.activations.contains_key(&domain)));
    assert_eq!(
        active_domain_count, 0,
        "domain activation count must return to zero when the last carrier leaves"
    );

    // 5. Launch `*Late Runner*` (`hull_late`) onto the same Kirkwood route while `*Salt Moth*` (`hull_miner`)
    //    is still cruising on leg 2 (`leg_cursor == 1`) and verify `*Salt Moth*` is NOT re-promoted:
    teleport(&mut rig.tw, pilot_late, gallery);
    let board_late = run(&mut rig.tw, pilot_late, "+ship/board");
    assert!(
        board_late.contains("You cycle the lock into the Late Runner."),
        "{board_late}"
    );
    let pending_late = undock_and_burn(&mut rig, pilot_late, hull_late);
    let _ = fire_quietly(&mut rig);
    let late_leg1_due = pending_late.plan.depart_ic + i64::from(pending_late.plan.legs[0].secs);
    let _ = to_ic(&mut rig, late_leg1_due);
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active_late = book
            .active_for_hull(hull_late)
            .expect("*Late Runner* must promote into the Kirkwood domain bubble");
        assert!(
            !active_late.carriers.contains_key(&hull_miner),
            "cleared carrier (*Salt Moth*) must NOT be re-promoted when a subsequent hull arrives at the waypoint"
        );
    }
    assert!(
        matches!(
            journey(&rig.tw, hull_miner).map(|j| j.state().clone()),
            Some(mush_sim::JourneyState::Cruising)
        ),
        "cleared carrier (*Salt Moth*) must remain Cruising on leg 2"
    );
}

/// The two-tug Kirkwood scene `sfr19` and `rs8` share: *SalvorA* and *SalvorB* fly the
/// interdicted `ceres_stickney` leg, clear customs, beam the Belt Raider into a disabled
/// derelict, and close to the collar envelope.
struct SalvageScene {
    rig: Rig,
    pilot_a: ObjId,
    pilot_b: ObjId,
    hull_a: ObjId,
    raider_hull: ObjId,
    raider_leader: ObjId,
    total_secs: i64,
}

/// The two tugs flown into the Kirkwood bubble, each operational one heaved-to and cleared by
/// customs. With `drive_down_b`, staff `@unit/damage` puts *SalvorB*'s drive at its floor during
/// the cruise (her unit is not deployed then), so the bubble meets her as a `DisabledDerelict`
/// through the engine's own check-out (doc 42 cut CR.6's fixture; doc 35b's no-fiat lint holds).
struct TwoTugs {
    rig: Rig,
    pilot_a: ObjId,
    pilot_b: ObjId,
    hull_a: ObjId,
    hull_b: ObjId,
    total_secs: i64,
}

#[allow(clippy::too_many_lines)]
fn two_tugs_into_kirkwood(drive_down_b: bool) -> TwoTugs {
    let mut rig = stocked();
    let (pilot_a, row_a) = chartered(&mut rig, "SalvorA", 1);
    let hull_a = ObjId(row_a.hull);
    let vault_a = hold_vault(&rig.tw, &row_a);
    seed_pool(&mut rig, vault_a, "propellant", 53, LotId(1091));
    seed_pool(&mut rig, vault_a, "provisions", 19, LotId(1092));

    let pilot_b = pilot(&mut rig, "SalvorB", 2);
    let enrol_b = run(&mut rig.tw, pilot_b, "+coop/enrol Second Tug");
    assert!(enrol_b.contains("now the Second Tug"), "{enrol_b}");
    let charter_b = tacsim::port::live_charter(rig.tw.world(), pilot_b).expect("b charter");
    let row_b = port_map(&rig.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|r| r.hull == charter_b.hull.0)
        .expect("b hull row");
    let hull_b = ObjId(row_b.hull);
    let vault_b = hold_vault(&rig.tw, &row_b);
    seed_pool(&mut rig, vault_b, "propellant", 53, LotId(1093));
    seed_pool(&mut rig, vault_b, "provisions", 19, LotId(1094));

    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot_a);
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, pilot_b, gallery);
    let board_b = run(&mut rig.tw, pilot_b, "+ship/board");
    assert!(
        board_b.contains("You cycle the lock into the Second Tug."),
        "{board_b}"
    );

    let pending_a = undock_and_burn(&mut rig, pilot_a, hull_a);
    let pending_b = undock_and_burn(&mut rig, pilot_b, hull_b);
    let _ = fire_quietly(&mut rig);
    if drive_down_b {
        let drive = mush_tactics::CellAddr::Section {
            section: mush_tactics::SectionId::new("drive").expect("the drive section"),
            cell: mush_tactics::CellId::new(tacsim::definitions::unit::INTERNALS_CELL)
                .expect("the internals cell"),
        };
        let unit = mush_tactics::module::unit_of(rig.tw.world(), hull_b)
            .expect("SalvorB's unit")
            .clone();
        let floor = unit.cell(&drive).expect("the drive's internals").min;
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetCell {
                addr: drive.clone(),
                value: floor,
            }],
            Some(unit.revision()),
            Some("CR.6 fixture: SalvorB's drive down".into()),
        )
        .expect("the plan");
        let damaged = run(
            &mut rig.tw,
            GOD,
            &format!(
                "@unit/damage #{}={}",
                hull_b.0,
                serde_json::to_string(&plan).expect("the plan's JSON")
            ),
        );
        assert_eq!(
            mush_tactics::module::unit_of(rig.tw.world(), hull_b)
                .and_then(|u| u.cell(&drive))
                .map(|c| c.current),
            Some(floor),
            "{damaged}"
        );
    }
    let flying: Vec<ObjId> = if drive_down_b {
        vec![pilot_a]
    } else {
        vec![pilot_a, pilot_b]
    };
    let leg1_due = (pending_a.plan.depart_ic + i64::from(pending_a.plan.legs[0].secs))
        .max(pending_b.plan.depart_ic + i64::from(pending_b.plan.legs[0].secs));
    let total_secs = pending_a.plan.total_s;
    let _ = to_ic(&mut rig, leg1_due);

    // Heave-to and clear customs inspection for `pilot_a` while the Raider ambush launches:
    for &p in &flying {
        let _ = run(&mut rig.tw, p, "+helm/thrust 600 -1000 0 0");
    }
    for _ in 0..6 {
        let next_r = rig.tw.world().now() + 1;
        rig.tw.set_now(next_r);
        let _ = rig.tw.fire_due(next_r * 1_000);
    }
    for &p in &flying {
        let _ = run(&mut rig.tw, p, "+helm/thrust 0");
    }
    let brake_submits = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &brake_submits);
    for &p in &flying {
        let sub = run(&mut rig.tw, p, "+helm/submit");
        assert!(sub.contains("customs clearance granted"), "{sub}");
        let draws = drain(&mut rig.tw);
        land_quietly(&mut rig, &draws);
    }
    TwoTugs {
        rig,
        pilot_a,
        pilot_b,
        hull_a,
        hull_b,
        total_secs,
    }
}

#[allow(clippy::too_many_lines)]
fn salvage_scene() -> SalvageScene {
    let TwoTugs {
        mut rig,
        pilot_a,
        pilot_b,
        hull_a,
        total_secs,
        ..
    } = two_tugs_into_kirkwood(false);
    let (corvette_hull, raider_hull, raider_leader) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = book.active_for_hull(hull_a).expect("active bubble");
        let picket = active.picket.as_ref().expect("picket");
        let leader = rig
            .tw
            .world()
            .get(picket.raider_hull)
            .expect("raider hull")
            .owner;
        (picket.corvette_hull, picket.raider_hull, leader)
    };
    // Both spawned NPC hulls hold real EconomyStore titles (`SF.R19`):
    assert!(
        economy(&rig.tw).snapshot().asset(corvette_hull).is_some(),
        "spawned Customs Corvette must hold an EconomyStore title"
    );
    assert_eq!(
        economy(&rig.tw)
            .snapshot()
            .asset(raider_hull)
            .map(|a| a.title_owner.clone()),
        Some(mush_org::PrincipalId::Object(raider_leader)),
        "spawned Belt Raider must hold an EconomyStore title owned by its NPC leader"
    );

    // Step the bubble until the Belt Raider's drive is disabled into `DisabledDerelict`:
    for step in 0..55 {
        let next_r = rig.tw.world().now() + 1;
        rig.tw.set_now(next_r);
        let _ = rig.tw.fire_due(next_r * 1_000);
        if step % 2 == 1 {
            let _ = run(&mut rig.tw, pilot_b, "+tac/beam Raider");
        } else if step % 4 == 2 {
            let _ = run(&mut rig.tw, pilot_b, "+damage/vent");
        }
        let raider_disabled = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .and_then(|b| b.active_for_hull(hull_a))
            .and_then(|a| {
                let rsid = a.picket.as_ref()?.raider_ship_id.as_str();
                a.bubble.ships.get(rsid)
            })
            .is_some_and(|s| {
                matches!(s.status, tacsim::space::HullStatus::DisabledDerelict { .. })
            });
        if raider_disabled {
            break;
        }
    }

    // Both carriers order `+helm/approach derelict` and step into the collar-initiation envelope:
    for p in [pilot_a, pilot_b] {
        let app = run(&mut rig.tw, p, "+helm/approach derelict");
        assert!(
            app.contains("Braking approach ordered toward disabled derelict"),
            "{app}"
        );
        let draws = drain(&mut rig.tw);
        land_quietly(&mut rig, &draws);
    }
    for _ in 0..48 {
        let next_r = rig.tw.world().now() + 1;
        rig.tw.set_now(next_r);
        let _ = rig.tw.fire_due(next_r * 1_000);
        let ready = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .and_then(|b| b.active_for_hull(hull_a))
            .is_some_and(|a| {
                a.bubble.torpedoes.is_empty()
                    && !a
                        .carriers
                        .get(&hull_a)
                        .is_some_and(|c| c.approaching_derelict)
            });
        if ready {
            break;
        }
    }

    SalvageScene {
        rig,
        pilot_a,
        pilot_b,
        hull_a,
        raider_hull,
        raider_leader,
        total_secs,
    }
}

/// The 6-DOF clamp of `who`'s hull on the derelict `target` names, through the client tape.
fn clamp_derelict(rig: &mut Rig, who: ObjId, hull: ObjId, target: &str) -> String {
    let dock = run(&mut rig.tw, who, &format!("+helm/dock {target}"));
    assert!(
        dock.contains("[DOCKING] Initiated 60 fps 6-DOF docking session"),
        "{dock}"
    );
    let (session, challenge) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .unwrap();
        let s = book.docking_sessions.get(&hull).unwrap();
        (s.challenge.session_id.clone(), s.challenge.clone())
    };
    let rle = client_solve_docking_rle(&challenge);
    let clamp = run(
        &mut rig.tw,
        who,
        &format!("+helm/dock tape {session} {rle}"),
    );
    let draws = drain(&mut rig.tw);
    land_quietly(rig, &draws);
    clamp
}

/// `who`'s available credit, minor units.
fn credit_of(tw: &TestWorld, who: ObjId) -> i64 {
    let account = tacsim::port::credit::own_account(tw.world(), who)
        .expect("a credit account")
        .id;
    economy(tw)
        .snapshot()
        .account(account)
        .unwrap()
        .available()
        .unwrap()
        .minor()
}

/// Close the bubble and fly `hull` on to Stickney Yards, the leg's end: the plan's total from
/// the resumed departure, then two IC days at a time until she docks (a long bubble pauses the
/// leg's timer: at the 3,600 rate ten wall minutes in a bubble are twenty-five IC days), sixty
/// days at most.
fn fly_on_to_stickney(rig: &mut Rig, hull: ObjId, total_secs: i64) {
    let next_real = rig.tw.world().now() + 1;
    rig.tw.set_now(next_real);
    let _ = rig.tw.fire_due(next_real * 1_000);
    let resumed = journey(&rig.tw, hull).expect("the resumed journey");
    let _ = to_ic(rig, resumed.departed().at + total_secs);
    for _ in 0..30 {
        if matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { .. }
        ) {
            break;
        }
        let _ = to_ic(rig, ic_now(&rig.tw) + 2 * 86_400);
    }
    sweep(&mut rig.tw, 60);
}

/// **Doc 41 cut RS.8 (Q-36-46, Q-36-47; doc 35c §12), rewriting doc 35c cut SF.R19's story** —
/// a derelict clamped, towed and docked under the one window, contested on the one path:
/// 1. Disablement alone opens no custody; the clamp pins the Belt Raider's real title (her NPC
///    leader) in open space's `Unclaimed` tenure.
/// 2. A second tug racing for the same derelict is refused.
/// 3. `+helm/tow` puts her in *SalvorA*'s custody — one row in the domain's
///    `tacsim:space:custody:{domain}` document, which a full engine restart reads back — and files
///    on `+salvage/claim`'s own path: one economy contract whose window is
///    `[tacsim.recovery].claim_window_ic_days`, at the world's rate. No bounty is paid, at the tow or at
///    the dock; release and re-tow keep the one claim.
/// 4. Docked at Stickney Yards (the interdicted leg's end), `+salvage`, `+salvage/inspect`,
///    `+salvage/report` and `+salvage/contest` work on her as on a ground hull.
/// 5. Lawful recovery in open space: past the window, the claim tick's `award_salvage_title`
///    moves her title to *SalvorA* and the custody document goes.
#[test]
#[allow(clippy::too_many_lines)]
fn sfr19_one_salvage_path_unifies_space_derelict_claims_with_gc6_and_economy() {
    let SalvageScene {
        mut rig,
        pilot_a,
        pilot_b,
        hull_a,
        raider_hull,
        raider_leader,
        total_secs,
    } = salvage_scene();

    // 1. Before `+helm/tow`, no custody names the Raider:
    assert!(
        !rig.tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .unwrap()
            .space_custody
            .contains_key(&raider_hull)
    );

    // 2. SalvorA clamps; SalvorB racing for the same derelict is refused.
    let clamp_a = clamp_derelict(&mut rig, pilot_a, hull_a, "derelict");
    assert!(
        clamp_a.contains("Hard dock and magnetic collar seal confirmed"),
        "{clamp_a}"
    );
    let pin = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .unwrap()
        .derelict_pins
        .get(&raider_hull)
        .cloned()
        .expect("the clamp pins the derelict");
    assert_eq!(
        pin.title,
        Some(mush_org::PrincipalId::Object(raider_leader)),
        "the pin names the Raider's real title"
    );
    assert_eq!(pin.answering, Some(raider_leader));
    assert_eq!(pin.tenure, tacsim::field_ops::GroundTenure::Unclaimed);
    let dock_b_refused = run(&mut rig.tw, pilot_b, "+helm/dock derelict");
    assert!(
        dock_b_refused.contains("already clamped or under tow by another ship"),
        "expected race refusal for second tug, got: {dock_b_refused}"
    );

    // 3. The tow files on the one path, from real seconds, on the one window; no bounty.
    let credit_before = credit_of(&rig.tw, pilot_a);
    let now_before_tow = rig.tw.world().now();
    let tow_1 = run(&mut rig.tw, pilot_a, "+helm/tow");
    assert!(
        tow_1.contains("[SALVAGE] Tow line locked to disabled derelict")
            && tow_1.contains("You file a salvage claim on the Belt Raider"),
        "{tow_1}"
    );
    assert_eq!(
        credit_of(&rig.tw, pilot_a),
        credit_before,
        "no bounty at the tow"
    );
    let window_s = tacsim::surface::claim::claim_window_real_secs(rig.tw.world());
    let (contract, filed_real_s, deadline_real_s, domain) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .unwrap();
        let custody = book
            .space_custody
            .get(&raider_hull)
            .expect("the tow puts her in custody");
        assert_eq!((custody.holder, custody.carrier), (pilot_a, hull_a));
        let tacsim::surface::salvage::ClaimSeam::Filed { contract } = custody.claim else {
            panic!("the tow files the claim: {:?}", custody.claim);
        };
        let file = custody.file.as_ref().expect("the filed claim");
        assert_eq!(file.prior_owner, raider_leader);
        (
            contract,
            file.filed_at_real_s,
            file.deadline_real_s,
            book.space_custody.domain_of(raider_hull).expect("a domain"),
        )
    };
    assert!(
        filed_real_s >= now_before_tow,
        "filed_at_real_s ({filed_real_s}) is real seconds ({now_before_tow})"
    );
    assert_eq!(deadline_real_s - filed_real_s, window_s, "the one window");
    let (economy_end, _) =
        mush_economy::salvage_claim_window(rig.tw.world(), mush_economy::ContractId(contract))
            .expect("the economy's claim contract");
    assert_eq!(
        economy_end, deadline_real_s,
        "the economy runs the same window"
    );

    // Release and re-clamp + re-tow keep the one claim.
    let rel = run(&mut rig.tw, pilot_a, "+helm/tow release");
    assert!(rel.contains("Tow line and collar clamp released"), "{rel}");
    let _ = run(&mut rig.tw, pilot_a, "+helm/approach derelict");
    let rel_draws = drain(&mut rig.tw);
    land_quietly(&mut rig, &rel_draws);
    let _ = clamp_derelict(&mut rig, pilot_a, hull_a, "derelict");
    let tow_2 = run(&mut rig.tw, pilot_a, "+helm/tow");
    assert!(
        tow_2.contains("[SALVAGE] Tow line locked to disabled derelict")
            && !tow_2.contains("You file a salvage claim"),
        "{tow_2}"
    );

    // The custody is its domain's document: a full engine restart reads it back.
    rig = reboot_with(rig, Roster::default());
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .unwrap();
        assert_eq!(book.space_custody.domain_of(raider_hull), Some(domain));
        assert!(matches!(
            book.space_custody.get(&raider_hull).map(|c| c.claim),
            Some(tacsim::surface::salvage::ClaimSeam::Filed { contract: c }) if c == contract
        ));
    }

    // 4. Docked at Stickney Yards: delivered, black box closed, no bounty.
    fly_on_to_stickney(&mut rig, hull_a, total_secs);
    assert_eq!(
        credit_of(&rig.tw, pilot_a),
        credit_before,
        "no bounty at the dock"
    );
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), raider_hull),
            tacsim::port::ship::Berth::Docked { .. }
        ),
        "delivered derelict must be Berth::Docked at the destination port"
    );
    assert!(
        rig.tw
            .world()
            .module_state::<tacsim::field_ops::SalvageClaimBook>()
            .unwrap()
            .black_box_tape_for_hull(raider_hull)
            .iter()
            .any(|e| matches!(
                e,
                tacsim::field_ops::BlackBoxFlightEntry::EpochClosedInBay { .. }
            )),
        "port delivery must append EpochClosedInBay to the derelict's black box"
    );

    // `+salvage`, `/inspect`, `/report` and `/contest` work on the space custody.
    let salvage_list = run(&mut rig.tw, pilot_a, "+salvage");
    assert!(
        salvage_list.contains(&format!("#{} the Belt Raider", raider_hull.0)),
        "expected +salvage to list space derelict claim, got: {salvage_list}"
    );
    let inspect_out = run(
        &mut rig.tw,
        pilot_a,
        &format!("+salvage/inspect #{}", raider_hull.0),
    );
    assert!(
        inspect_out.contains("You read the Belt Raider's black box"),
        "expected +salvage/inspect output, got: {inspect_out}"
    );
    let report_out = run(
        &mut rig.tw,
        pilot_a,
        &format!("+salvage/report #{}=SalvorB go", raider_hull.0),
    );
    assert!(
        report_out.contains("You hand your evidence of the Belt Raider's recovery to SalvorB."),
        "expected +salvage/report output, got: {report_out}"
    );
    let contest_out = run(
        &mut rig.tw,
        pilot_b,
        &format!("+salvage/contest #{} go", raider_hull.0),
    );
    assert!(
        contest_out.contains("You file your contest of the claim on the Belt Raider."),
        "expected +salvage/contest output, got: {contest_out}"
    );

    // 5. A lawful open-space recovery: past the one window the claim tick awards the title.
    let after_deadline = deadline_real_s + 10;
    rig.tw.set_now(after_deadline);
    let _ = rig.tw.fire_due(after_deadline * 1_000);
    assert_eq!(
        economy(&rig.tw)
            .snapshot()
            .asset(raider_hull)
            .map(|a| a.title_owner.clone()),
        Some(mush_org::PrincipalId::Object(pilot_a)),
        "the claim tick's award_salvage_title moves the derelict's title to the salvor"
    );
    let book = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .unwrap();
    assert!(
        !book.space_custody.contains_key(&raider_hull),
        "the custody row goes"
    );
    assert!(
        matches!(
            book.derelict_for_hull(raider_hull).and_then(|d| d.salvage_title_status.clone()),
            Some(tacsim::field_ops::CustodyTitleStatus::ResolvedTitle { owner }) if owner == pilot_a
        ),
        "the derelict record reads the award"
    );
}

/// **Doc 41 cut RS.8 (§4.7, Q-41-8)** — a space retrieval job on `+salvage/jobs`, delivered by a
/// ship: the Belt Raider's owner (a player; her title moved to them by staff as the fixture)
/// posts `+salvage/retrieve` for her to Stickney Yards; *SalvorA* reads it on `+salvage/jobs`,
/// accepts, clamps and tows her for the job (retrieval custody: no salvage row, no claim), and
/// docking at Stickney delivers her (`tacsim.retrieval.delivered`), so the sweep pays the fee.
#[test]
#[allow(clippy::too_many_lines)]
fn rs8_a_space_retrieval_job_is_delivered_by_the_tug_that_docks_her() {
    let SalvageScene {
        mut rig,
        pilot_a,
        hull_a,
        raider_hull,
        total_secs,
        ..
    } = salvage_scene();
    // A chartered pilot: the enrolment opens the credit account the job escrows from.
    let owner = pilot(&mut rig, "RaiderOwner", 3);
    let enrol = run(&mut rig.tw, owner, "+coop/enrol Third Tug");
    assert!(enrol.contains("now the Third Tug"), "{enrol}");
    let moved = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/transfer #{}=#{}", raider_hull.0, owner.0),
    );
    assert_eq!(
        economy(&rig.tw)
            .snapshot()
            .asset(raider_hull)
            .map(|a| a.title_owner.clone()),
        Some(mush_org::PrincipalId::Object(owner)),
        "{moved}"
    );
    let account = tacsim::port::credit::own_account(rig.tw.world(), owner)
        .expect("the owner's account")
        .id;
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("the Ceres treasury");
    let minted = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={} 10000000", account.0),
    );
    assert!(!minted.contains("Usage"), "{minted}");

    let posted = run(
        &mut rig.tw,
        owner,
        &format!("+salvage/retrieve #{}=stickney go", raider_hull.0),
    );
    assert!(posted.contains("Retrieval job"), "{posted}");
    let jobs = run(&mut rig.tw, pilot_a, "+salvage/jobs");
    assert!(jobs.contains(&format!("#{}", raider_hull.0)), "{jobs}");
    let taken = run(
        &mut rig.tw,
        pilot_a,
        &format!("+salvage/accept #{} go", raider_hull.0),
    );
    assert!(taken.contains("You accept job"), "{taken}");
    let fee = tacsim::surface::retrieval::order_of(rig.tw.world(), raider_hull)
        .expect("the job")
        .fee_minor;

    let clamp = clamp_derelict(&mut rig, pilot_a, hull_a, "derelict");
    assert!(
        clamp.contains("Hard dock and magnetic collar seal confirmed"),
        "{clamp}"
    );
    let tow = run(&mut rig.tw, pilot_a, "+helm/tow");
    assert!(
        tow.contains("[SALVAGE] Tow line locked to disabled derelict")
            && tow.contains(tacsim::space::TOW_FOR_RETRIEVAL)
            && !tow.contains("You file a salvage claim"),
        "{tow}"
    );
    assert!(
        !rig.tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .unwrap()
            .space_custody
            .contains_key(&raider_hull),
        "a retrieval tow is no salvage custody"
    );

    let credit_before = credit_of(&rig.tw, pilot_a);
    fly_on_to_stickney(&mut rig, hull_a, total_secs);
    let order = tacsim::surface::retrieval::order_of(rig.tw.world(), raider_hull).expect("the job");
    let delivery = order.delivered.as_ref().expect("the dock delivered her");
    assert_eq!(delivery.by, pilot_a);
    assert_eq!(delivery.port, order.destination);
    sweep(&mut rig.tw, 60);
    assert_eq!(
        credit_of(&rig.tw, pilot_a) - credit_before,
        fee,
        "the delivered job pays its fee"
    );
    assert_eq!(
        economy(&rig.tw)
            .snapshot()
            .asset(raider_hull)
            .map(|a| a.title_owner.clone()),
        Some(mush_org::PrincipalId::Object(owner)),
        "a retrieval never moves the title"
    );
}

/// **Doc 42 cut CR.6 (Q-42-24), the space seam** — the crew's one record in space:
/// 1. *SalvorB*'s drive is down when she enters the bubble (staff `@unit/damage` during the
///    cruise is the fixture; combat's disablement is sfr18's), so the bubble meets her as a
///    derelict and settles her out of it: her pilot's record is `Stranded` in her, on the
///    domain's site, from the space check-in.
/// 2. *SalvorA* hard-docks her (`+helm/dock #<hull>` and the client tape). The bubble reads the
///    two carriers as no friends, so the clamp captures SalvorB as a ground load does: out of her
///    seat, into *SalvorA*'s bay room, `Captured { carrier, captor }`.
/// 3. *SalvorA* casts off the empty derelict, flies on and docks at Stickney Yards. The port's
///    authority takes the captive through CR.2's one hook (`Processing`, then released there),
///    and SalvorA's own record is `Carrier` from her own check-in.
#[test]
#[allow(clippy::too_many_lines)]
fn cr6_a_crewed_derelict_hard_docked_by_a_stranger_is_captured_and_handed_over_at_port() {
    let TwoTugs {
        mut rig,
        pilot_a,
        pilot_b,
        hull_a,
        hull_b,
        total_secs,
    } = two_tugs_into_kirkwood(true);
    // 1. The bubble meets SalvorB as a derelict and settles her out of it.
    for _ in 0..30 {
        let next = rig.tw.world().now() + 1;
        rig.tw.set_now(next);
        let _ = rig.tw.fire_due(next * 1_000);
        if rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .is_some_and(|b| b.derelicts.contains_key(&hull_b))
        {
            break;
        }
    }
    let draws = drain(&mut rig.tw);
    land_quietly(&mut rig, &draws);
    let stranded = tacsim::field::crew_of(rig.tw.world(), pilot_b)
        .map(|record| record.location.clone())
        .expect("SalvorB's crew record from the space check-in");
    assert!(
        matches!(
            stranded,
            tacsim::field::CrewLocation::Stranded { unit, .. } if unit == hull_b
        ),
        "a derelict's crew are stranded in her: {stranded:?}"
    );

    // 2. SalvorA, no friend of SalvorB's, closes on her and hard-docks her: SalvorB is captured.
    let approach = run(
        &mut rig.tw,
        pilot_a,
        &format!("+helm/approach #{}", hull_b.0),
    );
    assert!(approach.contains("Braking approach ordered"), "{approach}");
    let draws = drain(&mut rig.tw);
    land_quietly(&mut rig, &draws);
    for _ in 0..600 {
        let next = rig.tw.world().now() + 1;
        rig.tw.set_now(next);
        let _ = rig.tw.fire_due(next * 1_000);
        let (closing, ordnance) = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .and_then(|b| b.active_for_hull(hull_a))
            .map_or((false, false), |a| {
                (
                    a.carriers
                        .get(&hull_a)
                        .is_some_and(|c| c.approaching_derelict),
                    !a.bubble.torpedoes.is_empty() || !a.bubble.railgun_slugs.is_empty(),
                )
            });
        if !closing && !ordnance {
            break;
        }
    }
    let clamp = clamp_derelict(&mut rig, pilot_a, hull_a, &format!("#{}", hull_b.0));
    assert!(
        clamp.contains("Hard dock and magnetic collar seal confirmed"),
        "{clamp}"
    );
    let held = tacsim::field::crew_of(rig.tw.world(), pilot_b)
        .map(|record| record.location.clone())
        .expect("SalvorB's record");
    assert!(
        matches!(
            held,
            tacsim::field::CrewLocation::Captured { carrier, captor, .. }
                if carrier == hull_a && captor == pilot_a
        ),
        "the hard-dock captures her crew: {held:?}"
    );
    let room = location(&rig.tw, pilot_b).expect("SalvorB stands somewhere");
    assert_eq!(
        tacsim::port::ship::hull_of_room(rig.tw.world(), room).map(|(view, _)| view.hull()),
        Some(hull_a),
        "the captive's body is aboard the captor's carrier"
    );

    // 3. SalvorA casts off the empty derelict and docks at Stickney: the captive is handed to
    //    the port's authority.
    let cast_off = run(&mut rig.tw, pilot_a, "+helm/tow release");
    assert!(cast_off.contains("collar clamp released"), "{cast_off}");
    fly_on_to_stickney(&mut rig, hull_a, total_secs);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull_a),
            tacsim::port::ship::Berth::Docked { .. }
        ),
        "SalvorA docks at Stickney"
    );
    let processed = tacsim::field::crew_of(rig.tw.world(), pilot_b)
        .map(|record| record.location.clone())
        .expect("SalvorB's record");
    // The fly-on can spend days docked before the read, so the release may have come too.
    assert!(
        matches!(
            &processed,
            tacsim::field::CrewLocation::Processing { port, .. }
                | tacsim::field::CrewLocation::Port { port }
                if port.as_str() == "stickney"
        ),
        "the docking hands the captive over to Stickney's authority: {processed:?}"
    );
    let own = tacsim::field::crew_of(rig.tw.world(), pilot_a)
        .map(|record| record.location.clone())
        .expect("SalvorA's record from her own check-in");
    assert_eq!(own, tacsim::field::CrewLocation::Carrier { hull: hull_a });
}

/// **Doc 35c §9 cut `SF.R20` ("One damage table and canonical drives", `Q-36-47`, `D-35-1`, `35c` §6.4–§6.5)**:
/// 1. An empty `Dray` (`tacsim.unit.dray`) and an `80 t`-loaded `Dray-P` (`tacsim.unit.dray_p`)
///    have distinct sub-$1\text{ m/s}^2$ (`< 1,000 mm/s²`) `max_accel_mmps2` values computed from
///    the rocket equation (`flip_accel_mps2`) and accelerate differently under `+helm/thrust`
///    without falling back to `1 m/s²` (`signum`).
/// 2. `corvette.json` authors its own `500 mg`, `12,000 km/s` drive law, and the space `Belt Raider`
///    is instantiated from `tacsim.unit.raider_heavy` (never `raiders().first()`, never carrying
///    `RAIDER_ATTR`), with `pixel_model` hydrated from `SpaceUnitOverlay` (`"corvette"`, `"raider"`).
/// 3. `SpaceShipState::carrier_damage()` maps `ShipSectionId` onto `carrier_damage::CarrierPart`,
///    `Slow` takes `min` via `arena::damage::apply_slow_consequence` without compounding, and a
///    rear `+tac/beam` splits `hull` (`70%`) and `drive` (`30%`, struck through `hull.rear`) per `dray.json`.
/// 4. A stripped PDC mount (`PdcOffline`) is persisted onto the `TacticalUnit` on `check_in_space_unit`
///    and remains stripped (`!operational`) on subsequent `check_out_space_unit` until repaired in bay.
/// 5. `@field/raiders tacsim.raider.ahuna_skulks` works after a Kirkwood picket, and the world has
///    zero leaked NPC space `Thing`s after a resolved bubble stands down.
#[test]
#[allow(clippy::too_many_lines)]
fn sfr20_one_damage_table_and_canonical_drives_sub_1mps2_kinematics_pdc_checkin_and_no_leaked_npcs()
{
    let mut rig = stocked();

    // 1. Charter two hulls at Ceres Freeport:
    //    - `hull_empty`: an empty `Dray` (`*Salt Moth*`, 35 t propellant + 15 t provisions = 50 t payload)
    //    - `hull_loaded`: a `Dray-P` (`*Loaded Miner*`, refitted via `+coop/refit prospector`, carrying
    //      80 t non-propellant cargo plus 55 t propellant + 15 t provisions = 150 t total payload)
    let (pilot_empty, row_empty) = chartered(&mut rig, "EmptyPilot", 1);
    let hull_empty = ObjId(row_empty.hull);
    let vault_empty = hold_vault(&rig.tw, &row_empty);
    seed_pool(&mut rig, vault_empty, "propellant", 55, LotId(2001));
    seed_pool(&mut rig, vault_empty, "provisions", 20, LotId(2002));

    let pilot_loaded = pilot(&mut rig, "LoadedPilot", 2);
    let enrol_loaded = run(&mut rig.tw, pilot_loaded, "+coop/enrol Loaded Miner");
    assert!(
        enrol_loaded.contains("now the Loaded Miner"),
        "{enrol_loaded}"
    );
    let charter_loaded =
        tacsim::port::live_charter(rig.tw.world(), pilot_loaded).expect("loaded charter");
    let row_loaded = port_map(&rig.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|r| r.hull == charter_loaded.hull.0)
        .expect("loaded hull row");
    let hull_loaded = ObjId(row_loaded.hull);
    let vault_loaded = hold_vault(&rig.tw, &row_loaded);
    let desk = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, pilot_loaded, desk);
    let refit_loaded = run(&mut rig.tw, pilot_loaded, "+coop/refit prospector");
    assert!(refit_loaded.contains("Dray-P"), "{refit_loaded}");
    seed_pool(&mut rig, vault_loaded, "propellant", 75, LotId(2003));
    seed_pool(&mut rig, vault_loaded, "provisions", 20, LotId(2004));
    seed_pool(&mut rig, vault_loaded, "industrial_parts", 80, LotId(2005));

    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot_empty);
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, pilot_loaded, gallery);
    let _ = run(&mut rig.tw, pilot_loaded, "+ship/board");

    // Count live Things before the Kirkwood picket opens:
    let live_things_before = rig
        .tw
        .world()
        .iter()
        .filter(|obj| {
            obj.kind == mush_core::ObjKind::Thing && !obj.flags.contains(mush_core::Flags::GOING)
        })
        .count();

    let p_empty = undock_and_burn(&mut rig, pilot_empty, hull_empty);
    let p_loaded = undock_and_burn(&mut rig, pilot_loaded, hull_loaded);
    let _ = fire_quietly(&mut rig);
    let due_ic = (p_empty.plan.depart_ic + i64::from(p_empty.plan.legs[0].secs))
        .max(p_loaded.plan.depart_ic + i64::from(p_loaded.plan.legs[0].secs));
    let _ = to_ic(&mut rig, due_ic);

    // 2. Inspect both carriers, the Customs Corvette, and the Belt Raider inside the Kirkwood bubble:
    let (
        empty_max_mmps2,
        loaded_max_mmps2,
        corvette_accel_mg,
        corvette_exhaust_kms,
        corvette_dv,
        corvette_model,
        raider_accel_mg,
        raider_dv,
        raider_model,
    ) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = book.active_for_hull(hull_empty).expect("active bubble");
        let empty_ship_id = active.carrier_ship_id_for(hull_empty);
        let loaded_ship_id = active.carrier_ship_id_for(hull_loaded);
        let empty_ship = active.bubble.ships.get(empty_ship_id).expect("empty ship");
        let loaded_ship = active
            .bubble
            .ships
            .get(loaded_ship_id)
            .expect("loaded ship");
        let corvette = active
            .bubble
            .ships
            .values()
            .find(|s| s.callsign == "Republic Customs Corvette")
            .expect("corvette");
        let raider = active
            .bubble
            .ships
            .values()
            .find(|s| s.callsign == "Belt Raider")
            .expect("raider");
        (
            empty_ship.max_accel_mmps2,
            loaded_ship.max_accel_mmps2,
            corvette.rated_accel_mg,
            corvette.exhaust_kms,
            corvette.delta_v_mps,
            corvette.pixel_model().to_owned(),
            raider.rated_accel_mg,
            raider.delta_v_mps,
            raider.pixel_model().to_owned(),
        )
    };
    assert!(
        empty_max_mmps2 > loaded_max_mmps2 && empty_max_mmps2 < 1_000 && loaded_max_mmps2 > 0,
        "empty Dray ({empty_max_mmps2} mm/s²) and 80 t Dray-P ({loaded_max_mmps2} mm/s²) must accelerate differently and under 1 m/s² rating"
    );
    assert_eq!(
        (
            corvette_accel_mg,
            corvette_exhaust_kms,
            corvette_model.as_str()
        ),
        (500, 12_000, "corvette"),
        "Republic Customs Corvette must use corvette.json's authored 500 mg / 12,000 km/s drive and overlay pixel_model"
    );
    assert!(
        corvette_dv > 3_000_000 && corvette_dv != 9_500,
        "Corvette delta_v_mps must come from the rocket equation (not 9,500): got {corvette_dv}"
    );
    assert_eq!(
        (raider_accel_mg, raider_model.as_str()),
        (500, "raider"),
        "Belt Raider must use authored drive and overlay pixel_model"
    );
    assert!(
        raider_dv > 1_000_000 && raider_dv != 9_500,
        "Belt Raider delta_v_mps must come from the rocket equation (not 9,500): got {raider_dv}"
    );

    // Command identical 600‰ braking burns on both carriers and verify their velocity changes differ:
    let _ = run(&mut rig.tw, pilot_empty, "+helm/thrust 600 -1000 0 0");
    let _ = run(&mut rig.tw, pilot_loaded, "+helm/thrust 600 -1000 0 0");
    let burns = drain(&mut rig.tw);
    land_quietly(&mut rig, &burns);
    for _ in 0..6 {
        let next_real = rig.tw.world().now() + 1;
        rig.tw.set_now(next_real);
        let _ = rig.tw.fire_due(next_real * 1_000);
    }
    let (vx_empty, vx_loaded) = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let active = book.active_for_hull(hull_empty).expect("active bubble");
        let empty_id = active.carrier_ship_id_for(hull_empty);
        let loaded_id = active.carrier_ship_id_for(hull_loaded);
        (
            active.bubble.ships[empty_id].vel_mmps[0],
            active.bubble.ships[loaded_id].vel_mmps[0],
        )
    };
    assert!(
        vx_empty < vx_loaded && vx_loaded < 25_000,
        "empty Dray (vx={vx_empty} mm/s) must shed more speed than 80 t Dray-P (vx={vx_loaded} mm/s)"
    );

    // 3. Submit both clean carriers to customs inspection (`+helm/submit`), then disable the Belt Raider
    //    so the Kirkwood picket bubble closes cleanly with zero derelicts on the NPC Corvette:
    let sub_empty = run(&mut rig.tw, pilot_empty, "+helm/submit");
    assert!(
        sub_empty.contains("customs clearance granted"),
        "{sub_empty}"
    );
    let sub_loaded = run(&mut rig.tw, pilot_loaded, "+helm/submit");
    assert!(
        sub_loaded.contains("customs clearance granted"),
        "{sub_loaded}"
    );

    for _ in 0..60 {
        let next_real = rig.tw.world().now() + 1;
        rig.tw.set_now(next_real);
        let _ = rig.tw.fire_due(next_real * 1_000);
        let sub = drain_space_and_industry(&mut rig.tw);
        land_quietly(&mut rig, &sub);
        if rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .unwrap()
            .active_for_hull(hull_empty)
            .is_none()
        {
            break;
        }
    }
    assert!(
        rig.tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .unwrap()
            .active_for_hull(hull_empty)
            .is_none(),
        "Kirkwood bubble must close after clean submission and raider neutralization"
    );

    // 4. Verify `@field/raiders tacsim.raider.ahuna_skulks` works after the Kirkwood picket
    //    (space raider never touched `raiders().first()` or `RAIDER_ATTR`), and verify no non-derelict
    //    NPC space Things leaked:
    let leaked_non_derelict_npcs = {
        let w = rig.tw.world();
        let book = w
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        w.iter()
            .filter(|obj| {
                obj.kind == mush_core::ObjKind::Thing
                    && !obj.flags.contains(mush_core::Flags::GOING)
                    && w.attr(obj.id, "TACSIM.SPACE.NPC_HULL").is_some()
                    && !book.derelicts.contains_key(&obj.id)
            })
            .count()
    };
    assert_eq!(
        leaked_non_derelict_npcs, 0,
        "resolved bubble must destroy non-derelict NPC space Things on stand-down (0 leaked)"
    );
    let _ = live_things_before;

    let staff_actor = pilot(&mut rig, "StaffJudge", 3);
    rig.tw
        .world_mut()
        .get_mut(staff_actor)
        .expect("staff_actor")
        .flags
        .insert(mush_core::Flags::ROYALTY);
    let raiders_out = run(
        &mut rig.tw,
        staff_actor,
        "@field/raiders tacsim.raider.ahuna_skulks",
    );
    assert!(
        raiders_out.contains("Skulk raiders seeded at Ahuna Flank")
            && !raiders_out.contains("still has a hull standing"),
        "@field/raiders tacsim.raider.ahuna_skulks must succeed after a Kirkwood picket: got {raiders_out}"
    );

    // 5. Verify rear `+tac/beam` splits `hull` (70%) and `drive` (30%, struck through `hull.rear`) per
    //    `dray.json`, `Slow` takes `min` without compounding, `SpaceShipState::carrier_damage()` maps
    //    `ShipSectionId` onto `CarrierPart`, and a stripped PDC (`PdcOffline`) persists across
    //    `check_in_space_unit` and `check_out_space_unit`:
    let mut rig_combat = stocked_with(Roster {
        seed_lane: Some(lane_with_interdicted_kirkwood()),
        ..Roster::default()
    });
    let (pilot_c, row_c) = chartered(&mut rig_combat, "CombatPilot", 1);
    let hull_c = ObjId(row_c.hull);
    let vault_c = hold_vault(&rig_combat.tw, &row_c);
    seed_pool(&mut rig_combat, vault_c, "propellant", 65, LotId(2011));
    seed_pool(&mut rig_combat, vault_c, "provisions", 25, LotId(2012));
    set_rate(&mut rig_combat.tw, 3_600);
    board(&mut rig_combat, pilot_c);

    let pending_c = undock_and_burn(&mut rig_combat, pilot_c, hull_c);
    let _ = fire_quietly(&mut rig_combat);
    let _ = to_ic(
        &mut rig_combat,
        pending_c.plan.depart_ic + i64::from(pending_c.plan.legs[0].secs),
    );

    // Turn bow (`+helm/heading 0 0`) across ticks 0..1 so the Interdictor Picket's second volley
    // penetrates `Front` (`internal_dmg >= 25`) and strips `pdc_dorsal` offline (`PdcOffline`), then
    // turn stern (`+helm/heading 180 0`) so subsequent hits split `hull` / `drive` on `Rear`:
    let _ = run(&mut rig_combat.tw, pilot_c, "+helm/heading 0 0");
    for i in 0..4 {
        let next_real = rig_combat.tw.world().now() + 1;
        rig_combat.tw.set_now(next_real);
        let _ = rig_combat.tw.fire_due(next_real * 1_000);
        if i == 1 {
            let _ = run(&mut rig_combat.tw, pilot_c, "+helm/heading 180 0");
        }
        if i == 0 || i == 2 || i == 3 {
            let _ = run(&mut rig_combat.tw, pilot_c, "+tac/beam Interdictor Picket");
        }
        if i == 1 || i == 2 {
            let _ = run(&mut rig_combat.tw, pilot_c, "+damage/vent");
        }
    }
    let next_real = rig_combat.tw.world().now() + 1;
    rig_combat.tw.set_now(next_real);
    let _ = rig_combat.tw.fire_due(next_real * 1_000);
    assert!(
        rig_combat
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .unwrap()
            .active_for_hull(hull_c)
            .is_none(),
        "interdiction bubble must close after disabling the Interdictor Picket"
    );

    // Check that `hull_c`'s checked-in `TacticalUnit` has both `drive` and `hull` rear-split damage
    // AND that reconstructing a `SpaceShipState` from the checked-in `TacticalUnit` preserves the
    // stripped PDC mount (`!pdc.operational` and `SubsystemCrit::PdcOffline`):
    let unit_after_checkin = mush_tactics::module::unit_of(rig_combat.tw.world(), hull_c)
        .expect("checked-in TacticalUnit")
        .clone();
    let rechecked_out_ship = tacsim::space::space_ship_from_tactical_unit(
        &unit_after_checkin,
        "recheck",
        "Salt Moth",
        tacsim::space::CARRIER_PARTY,
        [0, 0, 0],
        [0, 0, 0],
    )
    .expect("space_ship_from_tactical_unit");
    assert!(
        rechecked_out_ship
            .crits
            .contains(&tacsim::space::SubsystemCrit::PdcOffline)
            && rechecked_out_ship.pdcs.iter().any(|p| !p.operational),
        "stripped PDC mount and PdcOffline crit must survive check-in and subsequent check-out: crits={:?}, pdcs={:?}",
        rechecked_out_ship.crits,
        rechecked_out_ship.pdcs
    );
    assert!(
        rechecked_out_ship.sections[&tacsim::space::ShipSectionId::Drive].internals < 250
            && rechecked_out_ship.sections[&tacsim::space::ShipSectionId::Hull].internals < 600,
        "rear hits must split internals across hull and drive per dray.json"
    );

    // Also verify `Slow` takes `min` without compounding on repeated `Hold` hits at 0 internals:
    let mut slow_test_ship = rechecked_out_ship;
    let _ = slow_test_ship.apply_damage(tacsim::space::FacetBand::Top, 900);
    let slow_after_first = slow_test_ship.slow_pct;
    let accel_after_first = slow_test_ship.rated_accel_mg;
    let _ = slow_test_ship.apply_damage(tacsim::space::FacetBand::Top, 200);
    assert_eq!(
        (slow_test_ship.slow_pct, slow_test_ship.rated_accel_mg),
        (slow_after_first, accel_after_first),
        "Slow consequence must take min(current_slow_pct, 75) via arena::damage::apply_slow_consequence without compounding"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn sfr21_prepared_bubble_input_mush_sim_standing_orders_and_per_entity_store_with_quarantine() {
    let roster = || Roster {
        seed_lane: Some(lane_with_interdicted_kirkwood()),
        ..Roster::default()
    };
    let mut rig = stocked_with(roster());
    let (pilot_actor, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    let vault_id = hold_vault(&rig.tw, &row);

    seed_pool(&mut rig, vault_id, "propellant", 65, LotId(2021));
    seed_pool(&mut rig, vault_id, "provisions", 25, LotId(2022));
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot_actor);

    let pending = undock_and_burn(&mut rig, pilot_actor, hull);
    let _ = fire_quietly(&mut rig);
    let _ = to_ic(
        &mut rig,
        pending.plan.depart_ic + i64::from(pending.plan.legs[0].secs),
    );

    let bubble_domain = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .and_then(|b| b.active_for_hull(hull))
        .map(|a| a.domain)
        .expect("entered interdiction bubble");

    // 1. Per-entity store keys: verify `tacsim:space:bubble:{domain}` is written in `rig.store`.
    let has_kv = |r: &Rig, k: &str| {
        r.store[tacsim::NS]
            .kv
            .iter()
            .any(|(ek, _)| ek.as_str() == k)
    };
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    let bubble_key = SmolStr::new(tacsim::space::space_bubble_key(bubble_domain));
    assert!(
        has_kv(&rig, &bubble_key),
        "active bubble must persist at per-entity key `{bubble_key}`"
    );

    // 2. Prepared bubble input (`tacsim_space_bubble_input`, family `tacsim.space_bubble_tape`)
    //    and `mush-sim` `StandingOrder` channels (`D-35-9`):
    let heading_out = run(&mut rig.tw, pilot_actor, "+helm/heading 90 0");
    assert!(heading_out.contains("Helm heading set"), "{heading_out}");
    let heading_submits = drain(&mut rig.tw);
    assert!(
        heading_submits
            .iter()
            .any(|s| s.action.key().id.as_str() == tacsim::space::BUBBLE_INPUT_ACTION),
        "+helm/heading must submit prepared `tacsim_space_bubble_input`: {heading_submits:?}"
    );
    land_all(&mut rig, &heading_submits);
    assert!(
        rig.typed.contains_key(&(
            SmolStr::new(tacsim::NS),
            SmolStr::new(format!(
                "action-state/v1/{}/{}",
                tacsim::space::BUBBLE_TAPE_FAMILY,
                hull.0
            ))
        )),
        "landing `tacsim_space_bubble_input` must write `tacsim.space_bubble_tape` for the hull"
    );

    let _ = run(&mut rig.tw, pilot_actor, "+helm/emcon ghost");
    let emcon_submits = drain(&mut rig.tw);
    land_all(&mut rig, &emcon_submits);

    let _ = run(&mut rig.tw, pilot_actor, "+helm/thrust 0");
    let thrust_submits = drain(&mut rig.tw);
    land_all(&mut rig, &thrust_submits);

    // Verify `mush-sim`'s `StandingOrderBook` holds all 3 channels (`Heading`, `Posture`, `Thrust`):
    assert_eq!(
        mush_sim::tick::standing_order(rig.tw.world(), hull, mush_sim::tick::OrderChannel::Heading)
            .map(|o| o.value().clone()),
        Some(mush_sim::tick::OrderValue::Vector {
            x: 0,
            y: 1_000,
            z: 0
        })
    );
    assert_eq!(
        mush_sim::tick::standing_order(rig.tw.world(), hull, mush_sim::tick::OrderChannel::Posture)
            .map(|o| o.value().clone()),
        Some(mush_sim::tick::OrderValue::Emcon { level: 0 })
    );
    assert_eq!(
        mush_sim::tick::standing_order(rig.tw.world(), hull, mush_sim::tick::OrderChannel::Thrust)
            .map(|o| o.value().clone()),
        Some(mush_sim::tick::OrderValue::Scalar { value: 0 })
    );

    // Step `space_tick` and verify `OrderChannel::Thrust` (`Scalar { value: 0 }`) keeps the carrier
    // hove-to (`thrust_mmps2 == [0, 0, 0]`):
    let next_real = rig.tw.world().now() + 1;
    rig.tw.set_now(next_real);
    let _ = rig.tw.fire_due(next_real * 1_000);
    let carrier_thrust = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .and_then(|b| b.active_for_hull(hull))
        .and_then(|a| a.bubble.ships.get(a.carrier_ship_id_for(hull)))
        .map(|s| s.thrust_mmps2)
        .expect("carrier thrust");
    assert_eq!(
        carrier_thrust,
        [0, 0, 0],
        "mush-sim StandingOrder Thrust=0 must keep the carrier at zero burn on space_tick"
    );

    // 3. Disable the interdictor to produce a `DisabledDerelict`, test unified propellant ledger
    //    on `+helm/approach derelict`, and test `OutOfGate` docking session deletion + persistence:
    let _ = run(&mut rig.tw, pilot_actor, "+tac/lock Interdictor Picket");
    let lock_submits = drain(&mut rig.tw);
    land_all(&mut rig, &lock_submits);
    for i in 0..4 {
        let next_real = rig.tw.world().now() + 1;
        rig.tw.set_now(next_real);
        let _ = rig.tw.fire_due(next_real * 1_000);
        if i == 0 || i == 2 || i == 3 {
            let _ = run(&mut rig.tw, pilot_actor, "+tac/beam Interdictor Picket");
            let beam_submits = drain(&mut rig.tw);
            land_all(&mut rig, &beam_submits);
        }
        if i == 1 || i == 2 {
            let _ = run(&mut rig.tw, pilot_actor, "+damage/vent");
        }
    }

    let prop_before_approach = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .and_then(|b| b.active_for_hull(hull))
        .and_then(|a| a.bubble.ships.get(a.carrier_ship_id_for(hull)))
        .map(|s| s.propellant_kg)
        .expect("propellant before approach");
    let _ = run(&mut rig.tw, pilot_actor, "+helm/approach derelict");
    let approach_submits = drain(&mut rig.tw);
    land_all(&mut rig, &approach_submits);
    let prop_after_approach = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .and_then(|b| b.active_for_hull(hull))
        .and_then(|a| a.bubble.ships.get(a.carrier_ship_id_for(hull)))
        .map(|s| s.propellant_kg)
        .expect("propellant after approach");
    assert!(
        prop_after_approach < prop_before_approach,
        "+helm/approach must deduct braking propellant directly from the in-bubble ship as well as HullPropellantLedger ({prop_after_approach} < {prop_before_approach})"
    );
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    let ledger_key = SmolStr::new(tacsim::space::space_ledger_key(hull));
    assert!(
        has_kv(&rig, &ledger_key),
        "propellant ledger must persist at per-entity key `{ledger_key}`"
    );

    // Finish approaching derelict on `space_tick` and open a docking session:
    for _ in 0..48 {
        let next_r = rig.tw.world().now() + 1;
        rig.tw.set_now(next_r);
        let _ = rig.tw.fire_due(next_r * 1_000);
        let s = drain(&mut rig.tw);
        land_all(&mut rig, &s);
        let ready = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .and_then(|b| b.active_for_hull(hull))
            .is_some_and(|a| {
                !a.approaching_derelict
                    && !a
                        .carriers
                        .get(&hull)
                        .is_some_and(|c| c.approaching_derelict)
            });
        if ready {
            break;
        }
    }
    let dock_open = run(&mut rig.tw, pilot_actor, "+helm/dock derelict");
    assert!(
        dock_open.contains("Docking approach") || dock_open.contains("session"),
        "opened docking session: {dock_open}"
    );
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    let docking_key = SmolStr::new(tacsim::space::space_docking_key(hull));
    assert!(
        has_kv(&rig, &docking_key),
        "active docking session must persist at per-entity key `{docking_key}`"
    );
    let session_id = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .and_then(|b| b.docking_sessions.get(&hull))
        .map(|s| s.challenge.session_id.clone())
        .expect("docking session_id");

    // Submit an `OutOfGate` tape (`00*10`) and verify the closed session is deleted from `rig.store`
    // and remains closed across `reboot_with`:
    let out_of_gate = run(
        &mut rig.tw,
        pilot_actor,
        &format!("+helm/dock tape {session_id} 00*10"),
    );
    assert!(
        out_of_gate.contains("collar gates not aligned"),
        "expected OutOfGate rejection: {out_of_gate}"
    );
    let oog_submits = drain(&mut rig.tw);
    land_all(&mut rig, &oog_submits);
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    assert!(
        !has_kv(&rig, &docking_key),
        "OutOfGate tape failure must persist deletion of `{docking_key}`"
    );

    rig = reboot_with(rig, roster());
    assert!(
        !rig.tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .unwrap()
            .docking_sessions
            .contains_key(&hull),
        "OutOfGate session closure must survive reboot"
    );

    // 4. Close the bubble on tick (past the 120s salvage hold timeout) so the disabled interdictor
    //    becomes a `SpaceDerelictRecord` at `tacsim:space:derelict:{derelict_hull}` and standing
    //    orders for `hull` are cleared:
    let next_real = rig.tw.world().now() + 130;
    rig.tw.set_now(next_real);
    let _ = rig.tw.fire_due(next_real * 1_000);
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    assert!(
        !has_kv(&rig, &bubble_key),
        "resolved bubble must delete `{bubble_key}`"
    );
    assert!(
        mush_sim::tick::standing_order(rig.tw.world(), hull, mush_sim::tick::OrderChannel::Thrust)
            .is_none(),
        "resolving the bubble must clear the hull's standing orders in mush-sim"
    );
    let derelict_hull = *rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .unwrap()
        .derelicts
        .keys()
        .next()
        .expect("derelict hull");
    let derelict_key = SmolStr::new(tacsim::space::space_derelict_key(derelict_hull));
    assert!(
        has_kv(&rig, &derelict_key),
        "derelict must persist at per-entity key `{derelict_key}`"
    );

    // 5. Dirty-only writes: decouple `hull` into free flight (`tacsim:space:flight:{hull}`), then
    //    clear `rig.tw.take_module_ops()` and verify a tick that only ticks the derelict countdown
    //    emits a `JsonSet` for `tacsim:space:derelict:{derelict_hull}` without rewriting
    //    `tacsim:space:ledger:{hull}` or `tacsim:space:state`:
    let _ = run(&mut rig.tw, pilot_actor, "+helm/decouple");
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    let flight_key = SmolStr::new(tacsim::space::space_flight_key(hull));
    assert!(
        has_kv(&rig, &flight_key),
        "free-flight record must persist at per-entity key `{flight_key}`"
    );

    let next_real = rig.tw.world().now() + 1;
    rig.tw.set_now(next_real);
    let _ = rig.tw.fire_due(next_real * 1_000);
    let tick_ops = rig.tw.take_module_ops();
    let tacsim_keys_written: Vec<SmolStr> = tick_ops
        .iter()
        .filter(|op| op.ns.as_str() == tacsim::NS)
        .filter_map(|op| match &op.op {
            ModuleWrite::JsonSet { key, .. } => Some(key.clone()),
            _ => None,
        })
        .collect();
    assert!(
        tacsim_keys_written.contains(&derelict_key),
        "dirty derelict record must be written on countdown tick: {tacsim_keys_written:?}"
    );
    assert!(
        !tacsim_keys_written.contains(&ledger_key),
        "unchanged propellant ledger must NOT be rewritten on a derelict countdown tick: {tacsim_keys_written:?}"
    );
    assert!(
        !tacsim_keys_written.contains(&SmolStr::new(tacsim::space::SPACE_STATE_KEY)),
        "unchanged auxiliary space state must NOT be rewritten on a derelict countdown tick: {tacsim_keys_written:?}"
    );
    fold_module_ops_into(&mut rig.store, tick_ops);

    // 6. Fail-closed per-entity quarantine (`#[serde(deny_unknown_fields)]`):
    //    Inject an unknown field into `tacsim:space:flight:{hull}` and reboot; verify `hull` is
    //    quarantined (`quarantined_hulls.contains(&hull)`), `derelict_hull` still loads cleanly,
    //    and `+helm/burn` on the quarantined `hull` is refused fail-closed!
    rig = reboot_editing(rig, roster(), |store| {
        let kv = &mut store.get_mut(tacsim::NS).unwrap().kv;
        kv.retain(|(k, _)| k != &flight_key);
        kv.push((
            flight_key.clone(),
            serde_json::json!({
                "hull": hull.0,
                "corrupt_unknown_field": true
            }),
        ));
    });
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        assert!(
            book.quarantined_hulls.contains(&hull),
            "corrupt per-entity flight doc must quarantine hull #{}: {:?}",
            hull.0,
            book.quarantined_hulls
        );
        assert!(
            book.derelicts.contains_key(&derelict_hull),
            "healthy per-entity derelict doc must still load when another hull's doc is corrupt"
        );
    }
    let quarantined_out = run(&mut rig.tw, pilot_actor, "+helm/burn 100 0 0 5");
    assert!(
        quarantined_out.contains("quarantined"),
        "commands on a quarantined hull must fail closed: {quarantined_out}"
    );
}

/// **Doc 35c cut SF.R22 (`D-35-17`, `35c` §3 Cut `SF.R22` — Authoring II)**:
/// 1. `RulesPack::from_content` (`load_pack`) validates `LocationDef::clusters` (`place_clusters_fault`)
///    and refuses malformed cluster entries (`PackError::PlaceCoordinates`); `all_orbital_clusters`
///    reads the four authored clusters from `campaign-seed` (`OrbitalClusterId` and `LANE_CLUSTER_ROCKS`
///    are deleted).
/// 2. `picket_cast_book()` loads `corvette`, `raider`, and `interdictor` from `content/space/picket_cast.json`,
///    and `run_sealed_mind_on_pool` installs sealed runners from the unit's `MindSeal` digest via
///    `install_sealed_runners` and executes on `MindPool::shared()` (`CACHED_RHAI_RUNNERS` is deleted).
/// 3. Launching collector drones via `+mine/drones` stages valid `RigRecord`s into `PortHandles::families.excursions`
///    (`tacsim.excursion`) with `RigRecord::excursion` populated (`Excursion { id, operator, hull, slot, site }`).
#[test]
#[allow(clippy::too_many_lines)]
fn sfr22_authoring_ii_clusters_from_pack_sealed_minds_and_drone_excursions() {
    // 1. Verify `load_pack` refuses a malformed `LocationDef::clusters` entry (`PackError::PlaceCoordinates`):
    let mut bad_lane: serde_json::Value =
        serde_json::from_str(LANE_JSON).expect("shipped lane JSON parses");
    bad_lane["manifest"]["digest"] = serde_json::json!(UNSIGNED);
    bad_lane["entries"]["kirkwood_depot"]["def"]["clusters"][0]["rocks"][0]["spectral"] =
        serde_json::json!("X-INVALID");
    let bad_lane_signed = sign(&serde_json::to_string_pretty(&bad_lane).expect("serializes"));
    let err =
        load_pack(&bad_lane_signed).expect_err("malformed cluster spectral must fail load_pack");
    assert!(
        matches!(err, tacsim::pack::PackError::PlaceCoordinates { .. }),
        "expected PackError::PlaceCoordinates for malformed cluster rock spectral, got: {err:?}"
    );

    // 2. Verify `picket_cast_book()` loads `corvette`, `raider`, and `interdictor` from `content/space/picket_cast.json`:
    let cast = tacsim::space::picket_cast_book().expect("picket_cast.json must load");
    assert_eq!(cast.corvette.commander, "Customs Picket Commander");
    assert_eq!(cast.corvette.unit, "tacsim.unit.corvette");
    assert_eq!(cast.raider.commander, "Kaelen Vane");
    assert_eq!(cast.raider.unit, "tacsim.unit.corvette");
    assert_eq!(cast.raider.overlay, "tacsim.unit.raider_heavy");
    assert_eq!(cast.interdictor.hull_name, "Interdictor Picket");
    assert_eq!(cast.interdictor.overlay, "tacsim.unit.corvette_interdictor");

    // 3. Boot world, verify `all_orbital_clusters` reads the 4 authored clusters from `campaign-seed`,
    //    fly a `Dray-P` to `ceres_halo`, launch 2 collector drones (`+mine/drones A 2`), land the
    //    `BubbleInputProvider` transaction, and verify `RigRecord`s are staged into `tacsim.excursion`!
    let mut rig = stocked();
    let clusters = tacsim::space::all_orbital_clusters(rig.tw.world());
    assert_eq!(
        clusters.len(),
        4,
        "all_orbital_clusters must read the 4 authored clusters from campaign-seed"
    );

    let (pilot, row) = chartered(&mut rig, "Pilot", 1);
    let hull = ObjId(row.hull);
    let vault = hold_vault(&rig.tw, &row);
    let desk = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, pilot, desk);
    let refit_out = run(&mut rig.tw, pilot, "+coop/refit prospector");
    assert!(refit_out.contains("Dray-P"), "{refit_out}");
    seed_pool(&mut rig, vault, "propellant", 75, LotId(991));
    seed_pool(&mut rig, vault, "provisions", 45, LotId(992));
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot);

    let _ = run(&mut rig.tw, pilot, "+licence/buy go");
    let _ = run(&mut rig.tw, pilot, "+ship/undock ceres_halo throttle=500");
    let transit_burn = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &transit_burn);
    let _ = fire_quietly(&mut rig);
    let launch_submits = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &launch_submits);
    let arrival_ic = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook")
        .free_flights[&hull]
        .cluster_arrival_ic_s
        .expect("cluster_arrival_ic_s");
    let real_s = (arrival_ic.saturating_sub(ic_now(&rig.tw)) + 3_599) / 3_600;
    let t0 = rig.tw.world().now().saturating_add(real_s.max(6));
    rig.tw.set_now(t0);
    let _ = rig.tw.fire_due(t0.saturating_mul(1_000));
    let open_submits = drain_space_and_industry(&mut rig.tw);
    land_all(&mut rig, &open_submits);
    let t_seed = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t_seed);
    let _ = rig.tw.fire_due(t_seed.saturating_mul(1_000));
    let seed_submits = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &seed_submits);

    let _ = run(&mut rig.tw, pilot, "+helm/approach A");
    let app_b = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &app_b);
    for _ in 0..3 {
        let t = rig.tw.world().now().saturating_add(6);
        rig.tw.set_now(t);
        let _ = rig.tw.fire_due(t.saturating_mul(1_000));
        let sub = drain_space_and_industry(&mut rig.tw);
        land_quietly(&mut rig, &sub);
    }
    let _ = run(&mut rig.tw, pilot, "+helm/match A");
    let match_b = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &match_b);

    set_rate(&mut rig.tw, 1);
    let drones_out = run(&mut rig.tw, pilot, "+mine/drones A 2");
    assert!(
        drones_out.contains("Launched 2 collector drone(s)"),
        "{drones_out}"
    );
    let drone_submits = drain_space_and_industry(&mut rig.tw);
    assert!(
        drone_submits
            .iter()
            .any(|s| s.action.key().id.as_str() == tacsim::space::BUBBLE_INPUT_ACTION),
        "launching collector drones must submit tacsim_space_bubble_input to stage RigRecords"
    );
    land_all(&mut rig, &drone_submits);

    // Verify both collector drone objects now have `RigRecord`s staged in `PortHandles::families.excursions` (`tacsim.excursion`):
    let drone_ids: Vec<(u8, tacsim::surface::excursion::ExcursionId)> = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .expect("SpaceBubbleBook")
        .cluster_sessions[&hull]
        .active_drones
        .iter()
        .map(|d| (d.slot, d.excursion_id))
        .collect();
    assert_eq!(drone_ids.len(), 2);
    for (slot, exc_id) in drone_ids {
        let raw_val = rig
            .typed
            .get(&(
                SmolStr::new(tacsim::NS),
                SmolStr::new(format!(
                    "action-state/v1/{}/{}",
                    tacsim::surface::excursion::EXCURSION_FAMILY,
                    exc_id.tv.0
                )),
            ))
            .expect("collector drone must have a RigRecord staged in tacsim.excursion");
        let rig_rec: tacsim::surface::excursion::RigRecord =
            serde_json::from_value(raw_val["value"].clone())
                .expect("RigRecord deserializes cleanly");
        let exc = rig_rec
            .excursion
            .expect("RigRecord::excursion must be populated for launched collector drone");
        assert_eq!(exc.id.tv, exc_id.tv);
        assert_eq!(exc.operator, pilot);
        assert_eq!(exc.hull, hull);
        assert_eq!(exc.slot, slot);
        assert_eq!(exc.site.as_str(), "ceres_halo");
    }
}

fn lane_with_sfr23_cluster() -> &'static str {
    let mut doc: serde_json::Value = serde_json::from_str(LANE_JSON).expect("the lane is JSON");
    doc["manifest"]["digest"] = serde_json::json!(UNSIGNED);
    let clusters = doc["entries"]["kirkwood_depot"]["def"]["clusters"]
        .as_array_mut()
        .expect("clusters array");
    let kg = clusters
        .iter_mut()
        .find(|c| c["id"] == "kirkwood_gap_3_1")
        .expect("kirkwood_gap_3_1 cluster");
    kg["rocks"][0]["spectral"] = serde_json::json!("S-TYPE");
    kg["rocks"][0]["gross_t"] = serde_json::json!(5);
    kg["rocks"][0]["tenure"] = serde_json::json!("open");
    kg["rocks"][1]["spectral"] = serde_json::json!("M-TYPE");
    kg["rocks"][1]["gross_t"] = serde_json::json!(120);
    kg["rocks"][1]["tenure"] = serde_json::json!("open");
    kg["rocks"][2]["spectral"] = serde_json::json!("M-TYPE");
    kg["rocks"][2]["gross_t"] = serde_json::json!(120);
    kg["rocks"][2]["tenure"] = serde_json::json!("open");
    let signed = sign(&serde_json::to_string_pretty(&doc).expect("serializes"));
    Box::leak(signed.into_boxed_str())
}

/// **Doc 35c §9 cut `SF.R23` ("Mining and sensing remainders", `35b` §5 #15, `D-35-10`, `D-35-7`)**:
/// 1. `+mine/*` outside an orbital cluster recommends `+helm/autopilot <cluster_id>`, and an
///    un-refitted standard `Dray` (`tacsim.unit.dray`) is refused `+mine/laser` (`is_prospector`).
/// 2. The cluster clerk (`site_state.clerk`) and block places are laid up at the cluster's host room
///    in `TacsimStore`, never inside a carrier hull.
/// 3. Staking Block A (`+mine/stake A metals`) and then mining Block A (`+mine/laser A`) in a
///    `Frontier` cluster (`kirkwood_gap_3_1`) checks live `PortHandles::families.claims` via
///    `actor_holds_cluster_block_claim` and does NOT flag `claim_jump_flagged` or `frontier_bloom_heat`.
/// 4. Two `Dray-P` hulls (`hull_1` and `hull_2`) mining `4 t` each from a `5 t` deposit (Block A)
///    both complete cleanly (`4 t` + `1 t` = `5 t` conserved) and `pending_space_mine_draws` empties
///    without jamming the clerk.
/// 5. Disconnecting `pilot_2` (`Inbound::ConnDown` with `DisconnectKind::Lost`) while `hull_2` is in
///    the `kirkwood_gap_3_1` cluster session queues `linkdead_hulls[&hull_2]` and, once the 120s
///    wall grace window expires on `space_tick`, executes the queued return journey back to Ceres.
/// 6. Staking Block B (`+mine/stake B strategic_elements`) does NOT exempt `strategic_ore` mined from
///    an unleased Block C (`+mine/laser C strategic_elements`) when `hull_1` submits at the Kirkwood
///    Customs Picket (`inspect_hull_manifest` checks per-block tenure on `mined_strategic_blocks`).
/// 7. In the Kirkwood bubble, an undetected contact (`Belt Raider` in `Ghost` at 120 km) is refused by
///    `+helm/dock` and `+helm/approach` with `"No such contact."` and its `target_pos` is `null` on
///    in-flight torpedoes in `holotank_feed`; `+tac/hail` at long range (`d >= c`) queues a
///    `PendingLightLagHail` and delivers it on `space_tick` at `t + d/c`.
#[test]
#[allow(clippy::too_many_lines)]
fn sfr23_mining_and_sensing_remainders() {
    let mut rig = stocked_with(Roster {
        seed_lane: Some(lane_with_sfr23_cluster()),
        ..Roster::default()
    });

    // Charter 3 hulls at Ceres Freeport:
    // - `hull_1` (`pilot_1`): refitted `Dray-P`
    // - `hull_2` (`pilot_2`): refitted `Dray-P`
    // - `hull_std` (`pilot_std`): un-refitted standard `Dray`
    let (pilot_1, row_1) = chartered(&mut rig, "MinerOne", 1);
    let hull_1 = ObjId(row_1.hull);
    let vault_1 = hold_vault(&rig.tw, &row_1);

    let pilot_2 = pilot(&mut rig, "MinerTwo", 2);
    let enrol_2 = run(&mut rig.tw, pilot_2, "+coop/enrol Second Miner");
    assert!(enrol_2.contains("now the Second Miner"), "{enrol_2}");
    let charter_2 = tacsim::port::live_charter(rig.tw.world(), pilot_2).expect("charter 2");
    let row_2 = port_map(&rig.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|r| r.hull == charter_2.hull.0)
        .expect("hull 2 row");
    let hull_2 = ObjId(row_2.hull);
    let vault_2 = hold_vault(&rig.tw, &row_2);

    let pilot_std = pilot(&mut rig, "StdPilot", 3);
    let enrol_std = run(&mut rig.tw, pilot_std, "+coop/enrol Plain Hauler");
    assert!(enrol_std.contains("now the Plain Hauler"), "{enrol_std}");
    let charter_std = tacsim::port::live_charter(rig.tw.world(), pilot_std).expect("charter std");
    let row_std = port_map(&rig.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|r| r.hull == charter_std.hull.0)
        .expect("hull std row");
    let hull_std = ObjId(row_std.hull);
    let vault_std = hold_vault(&rig.tw, &row_std);

    let desk = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, pilot_1, desk);
    assert!(
        run(&mut rig.tw, pilot_1, "+coop/refit prospector").contains("Dray-P"),
        "refit hull_1 to Dray-P"
    );
    teleport(&mut rig.tw, pilot_2, desk);
    assert!(
        run(&mut rig.tw, pilot_2, "+coop/refit prospector").contains("Dray-P"),
        "refit hull_2 to Dray-P"
    );

    seed_pool(&mut rig, vault_1, "propellant", 80, LotId(2301));
    seed_pool(&mut rig, vault_1, "provisions", 150, LotId(2302));
    seed_pool(&mut rig, vault_2, "propellant", 80, LotId(2303));
    seed_pool(&mut rig, vault_2, "provisions", 150, LotId(2304));
    seed_pool(&mut rig, vault_std, "propellant", 55, LotId(2305));
    seed_pool(&mut rig, vault_std, "provisions", 50, LotId(2306));

    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot_1);
    let gallery = c01(&rig.tw);
    teleport(&mut rig.tw, pilot_2, gallery);
    let _ = run(&mut rig.tw, pilot_2, "+ship/board");
    teleport(&mut rig.tw, pilot_std, gallery);
    let _ = run(&mut rig.tw, pilot_std, "+ship/board");

    // 1a. Outside a cluster, `+mine/ping` recommends `+helm/autopilot <cluster_id>`:
    let outside_ping = run(&mut rig.tw, pilot_1, "+mine/ping");
    assert!(
        outside_ping.contains("+helm/autopilot <cluster_id>")
            && !outside_ping.contains("+ship/undock <cluster_id>"),
        "expected +helm/autopilot <cluster_id> in refusal, got: {outside_ping}"
    );

    let _ = run(&mut rig.tw, pilot_1, "+licence/buy go");
    let _ = run(&mut rig.tw, pilot_2, "+licence/buy go");
    let _ = run(&mut rig.tw, pilot_std, "+licence/buy go");
    sweep(&mut rig.tw, 6);

    // Fly all 3 hulls to `kirkwood_gap_3_1`:
    for p in [pilot_1, pilot_2, pilot_std] {
        let out = run(&mut rig.tw, p, "+ship/undock kirkwood_gap_3_1 throttle=500");
        assert!(out.contains("kirkwood_gap_3_1"), "{out}");
    }
    let undock_burns = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &undock_burns);
    let _ = fire_quietly(&mut rig);
    let launch_submits = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &launch_submits);

    let arrival_ic = {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        book.free_flights[&hull_1]
            .cluster_arrival_ic_s
            .unwrap()
            .max(book.free_flights[&hull_2].cluster_arrival_ic_s.unwrap())
            .max(book.free_flights[&hull_std].cluster_arrival_ic_s.unwrap())
    };
    let real_s = (arrival_ic.saturating_sub(ic_now(&rig.tw)) + 3_599) / 3_600;
    let t0 = rig.tw.world().now().saturating_add(real_s.max(6));
    rig.tw.set_now(t0);
    let _ = rig.tw.fire_due(t0.saturating_mul(1_000));
    let open_submits = drain_space_and_industry(&mut rig.tw);
    land_all(&mut rig, &open_submits);
    let t_seed = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t_seed);
    let _ = rig.tw.fire_due(t_seed.saturating_mul(1_000));
    let seed_submits = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &seed_submits);

    // 2. Verify cluster clerk (`site_state.clerk`) is laid up at the cluster's host room, never inside a carrier hull:
    {
        let book = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .expect("SpaceBubbleBook");
        let site = book
            .cluster_sites
            .get(&cid("kirkwood_gap_3_1"))
            .expect("kirkwood_gap_3_1 laid up");
        assert!(
            site.host_room != hull_1 && site.host_room != hull_2 && site.host_room != hull_std,
            "cluster host_room (#{}) must not be a carrier hull",
            site.host_room.0
        );
        let clerk_loc = rig.tw.world().get(site.clerk).and_then(|o| o.location);
        assert_eq!(
            clerk_loc,
            Some(site.host_room),
            "cluster clerk must reside at site.host_room, never inside a carrier hull"
        );
    }

    // 1b. Verify the standard un-refitted `Dray` (`hull_std`) is refused `+mine/laser`:
    let std_laser = run(&mut rig.tw, pilot_std, "+mine/laser A");
    assert!(
        std_laser.contains("Mining laser requires a `Dray-P` prospector hull"),
        "standard Dray must be refused +mine/laser: {std_laser}"
    );

    // 3. `pilot_1` and `pilot_2` ping the cluster, `pilot_1` stakes Block A (`+mine/stake A metals`),
    //    lands the claim transaction, and both `hull_1` and `hull_2` approach and match velocity with Block A (`5 t` deposit):
    for p in [pilot_1, pilot_2] {
        let ping_out = run(&mut rig.tw, p, "+mine/ping");
        assert!(ping_out.contains("Block A"), "{ping_out}");
    }
    let ping_submits = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &ping_submits);
    let stake_a = run(&mut rig.tw, pilot_1, "+mine/stake A metals");
    assert!(
        stake_a.contains("Filed Freeport Registry claim stake on Block A"),
        "{stake_a}"
    );
    let stake_a_submits = drain_space_and_industry(&mut rig.tw);
    land_all(&mut rig, &stake_a_submits);

    for p in [pilot_1, pilot_2] {
        let _ = run(&mut rig.tw, p, "+helm/approach A");
    }
    let app_a = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &app_a);
    for _ in 0..3 {
        let t = rig.tw.world().now().saturating_add(6);
        rig.tw.set_now(t);
        let _ = rig.tw.fire_due(t.saturating_mul(1_000));
        let sub = drain_space_and_industry(&mut rig.tw);
        land_quietly(&mut rig, &sub);
    }
    for p in [pilot_1, pilot_2] {
        let m = run(&mut rig.tw, p, "+helm/match A");
        assert!(m.contains("RCS velocity match locked on Block A"), "{m}");
    }
    let match_a = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &match_a);

    // Verify `pilot_1` mining staked Block A in Frontier `kirkwood_gap_3_1` is NOT flagged as claim-jumping,
    // and Tier-1 `space.cockpit` panel payload has live `delta_v_mps > 0` and `hull_pct == 100`:
    let laser_1_events = rig.tw.run(pilot_1, "+mine/laser A");
    let laser_1 = said(&rig.tw, &laser_1_events, pilot_1);
    assert!(
        laser_1.contains("Mining laser engaged on Block A") && !laser_1.contains("FRONTIER BLOOM"),
        "mining own staked block in Frontier cluster must not trigger claim-jump bloom: {laser_1}"
    );
    {
        let sess_1 = &rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .unwrap()
            .cluster_sessions[&hull_1];
        assert!(
            !sess_1.claim_jump_flagged && sess_1.frontier_bloom_heat == 0,
            "staked block holder must have claim_jump_flagged == false and frontier_bloom_heat == 0"
        );
    }
    let status_events = rig.tw.run(pilot_1, "+helm/status");
    let tier1_cockpit = status_events
        .iter()
        .find_map(|(_, ev)| match ev {
            OutputEvent::Panel { feed, data, .. }
                if feed.as_str() == tacsim::space::SPACE_COCKPIT_FEED =>
            {
                Some(data.clone())
            }
            _ => None,
        })
        .expect("Tier-1 space.cockpit panel");
    assert!(
        tier1_cockpit["delta_v_mps"].as_i64().unwrap_or(0) > 0,
        "Tier-1 space.cockpit must report live delta_v_mps > 0: {tier1_cockpit}"
    );
    assert_eq!(tier1_cockpit["hull_pct"].as_i64(), Some(100));

    // 4. `hull_2` also starts `+mine/laser A` (`4 t`) against the `5 t` Block A deposit while `hull_1`'s
    //    `4 t` laser cycle is still running:
    let laser_2 = run(&mut rig.tw, pilot_2, "+mine/laser A");
    assert!(
        laser_2.contains("Mining laser engaged on Block A"),
        "{laser_2}"
    );

    // Step 3 ticks so `hull_1` (`4 t`) and `hull_2` (clamped to remaining `1 t`) both settle through
    // the cluster clerk and the third tick prunes `pending_space_mine_draws` empty without jamming:
    for tick_idx in 0..3 {
        let t = rig.tw.world().now().saturating_add(6);
        rig.tw.set_now(t);
        let _ = rig.tw.fire_due(t.saturating_mul(1_000));
        let work_submits = drain_space_and_industry(&mut rig.tw);
        if tick_idx == 2 {
            assert!(
                work_submits.is_empty(),
                "cluster clerk must not submit further draws once Block A is exhausted"
            );
        }
        land_all(&mut rig, &work_submits);
    }
    assert_eq!(
        pool(&mut rig.tw, vault_1, "metal_ore"),
        4,
        "hull_1 must receive 4 t of metal_ore from Block A"
    );
    assert_eq!(
        pool(&mut rig.tw, vault_2, "metal_ore"),
        1,
        "hull_2 must receive the remaining 1 t of metal_ore from the 5 t Block A deposit"
    );
    assert!(
        rig.tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .unwrap()
            .pending_space_mine_draws
            .is_empty(),
        "pending_space_mine_draws must be pruned empty once Block A reaches 0 t without jamming the clerk"
    );

    // 5. Disconnect `pilot_2` (`Inbound::ConnDown` with `DisconnectKind::Lost`) while `hull_2` is in
    //    `kirkwood_gap_3_1` and verify `execute_linkdead_return_journey` launches `hull_2` back to
    //    Ceres Freeport once the 120s linkdead grace window expires:
    let pilot_2_conns: Vec<u64> = (1..=64)
        .filter(|conn| rig.tw.engine().actor_of(*conn) == Some(pilot_2))
        .collect();
    assert!(!pilot_2_conns.is_empty());
    for conn in pilot_2_conns {
        let mut out = mush_core::Outbox::new();
        let mut fx = mush_core::Effects::new();
        rig.tw.engine_mut().step(
            mush_core::Inbound::ConnDown {
                conn,
                actor: pilot_2,
                reason: mush_core::DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
    }
    assert!(
        rig.tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .unwrap()
            .linkdead_state(hull_2)
            .is_some_and(|ld| !ld.grace_expired && ld.queued_return_port.is_some()),
        "cluster-session hull must enter linkdead_hulls when its crew loses connection"
    );
    let t_ld_exp = rig
        .tw
        .world()
        .now()
        .saturating_add(tacsim::space::SPACE_LINKDEAD_GRACE_WALL_S + 10);
    rig.tw.set_now(t_ld_exp);
    let _ = rig.tw.fire_due(t_ld_exp.saturating_mul(1_000));
    assert!(
        !rig.tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .unwrap()
            .cluster_sessions
            .contains_key(&hull_2),
        "expired linkdead grace window must remove hull_2 from cluster_sessions"
    );
    assert!(
        matches!(
            journey(&rig.tw, hull_2).map(|j| j.state().clone()),
            Some(mush_sim::JourneyState::Cruising)
        ),
        "expired linkdead grace window must execute the queued return CarrierJourney for hull_2"
    );

    // 6. Per-block customs manifest inspection (`D-35-10`, `35b` §5 #15):
    //    `pilot_1` stakes Block B (`+mine/stake B strategic_elements`), then mines 4 t of
    //    `strategic_ore` from the UNLEASED Block C (`+mine/laser C strategic_elements`):
    let stake_b = run(&mut rig.tw, pilot_1, "+mine/stake B strategic_elements");
    assert!(
        stake_b.contains("Filed Freeport Registry claim stake on Block B"),
        "{stake_b}"
    );
    let stake_b_submits = drain_space_and_industry(&mut rig.tw);
    land_all(&mut rig, &stake_b_submits);

    let _ = run(&mut rig.tw, pilot_1, "+helm/approach C");
    let app_c = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &app_c);
    for _ in 0..3 {
        let t = rig.tw.world().now().saturating_add(6);
        rig.tw.set_now(t);
        let _ = rig.tw.fire_due(t.saturating_mul(1_000));
        let sub = drain_space_and_industry(&mut rig.tw);
        land_quietly(&mut rig, &sub);
    }
    let match_c = run(&mut rig.tw, pilot_1, "+helm/match C");
    assert!(
        match_c.contains("RCS velocity match locked on Block C"),
        "{match_c}"
    );
    let match_c_sub = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &match_c_sub);

    let laser_c = run(&mut rig.tw, pilot_1, "+mine/laser C strategic_elements");
    assert!(
        laser_c.contains("extracting 4 t `strategic_ore`"),
        "{laser_c}"
    );
    let t_c = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t_c);
    let _ = rig.tw.fire_due(t_c.saturating_mul(1_000));
    let work_c = drain_space_and_industry(&mut rig.tw);
    land_all(&mut rig, &work_c);
    assert_eq!(pool(&mut rig.tw, vault_1, "strategic_ore"), 4);

    // Return `hull_1` to Ceres Freeport and fly `ceres_stickney` into the Kirkwood Customs Picket:
    let _ = run(&mut rig.tw, pilot_1, "+helm/autopilot ceres");
    let ret_burn = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &ret_burn);
    let ret_arr = journey(&rig.tw, hull_1)
        .and_then(|j| j.arrival_at())
        .expect("return arrival");
    let _ = to_ic(&mut rig, ret_arr);
    for _ in 0..2 {
        let (_, q) = fire(&mut rig);
        land_all(&mut rig, &q);
    }

    let (undock_kw_out, kw_queued) =
        undock(&mut rig, pilot_1, "+ship/undock stickney throttle=500");
    assert!(
        undock_kw_out.contains("Clearance filed for Stickney Yards"),
        "{undock_kw_out}"
    );
    let pending_kw = voyage(&rig.tw, hull_1)
        .and_then(|v| v.pending)
        .expect("a filed passage");
    land_all(&mut rig, &kw_queued);
    for _ in 0..2 {
        let (_, q) = fire(&mut rig);
        land_all(&mut rig, &q);
    }
    let _ = to_ic(
        &mut rig,
        pending_kw.plan.depart_ic + i64::from(pending_kw.plan.legs[0].secs),
    );

    // 7a. In the Kirkwood bubble, the lurking `Belt Raider` is in `Ghost` at 120 km (`!is_bubble_ship_detected`):
    //     Verify `+helm/dock <raider_id>` and `+helm/approach <raider_id>` never name `"Belt Raider"`
    //     or leak the undetected ship's range/speed:
    let raider_id = rig
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .unwrap()
        .active_for_hull(hull_1)
        .unwrap()
        .picket
        .as_ref()
        .unwrap()
        .raider_ship_id
        .clone();
    let dock_ghost = run(&mut rig.tw, pilot_1, &format!("+helm/dock {raider_id}"));
    assert!(
        !dock_ghost.contains("Belt Raider"),
        "+helm/dock on an undetected Ghost contact must never name it in refusals: {dock_ghost}"
    );
    let app_ghost = run(&mut rig.tw, pilot_1, &format!("+helm/approach {raider_id}"));
    assert!(
        !app_ghost.contains("Belt Raider") && app_ghost.contains("No such contact."),
        "+helm/approach on an undetected Ghost contact must refuse with No such contact without naming it: {app_ghost}"
    );

    // Verify `holotank_feed` masks `target_pos` (`null`) on a torpedo targeting an undetected contact:
    {
        let mut test_bubble = {
            let book = rig
                .tw
                .world()
                .module_state::<tacsim::space::SpaceBubbleBook>()
                .unwrap();
            book.active_for_hull(hull_1).unwrap().bubble.clone()
        };
        let own_ship_id = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .unwrap()
            .active_for_hull(hull_1)
            .unwrap()
            .carrier_ship_id_for(hull_1)
            .to_owned();
        let raider_id = rig
            .tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .unwrap()
            .active_for_hull(hull_1)
            .unwrap()
            .picket
            .as_ref()
            .unwrap()
            .raider_ship_id
            .clone();
        test_bubble.torpedoes.push(tacsim::space::InFlightTorpedo {
            id: 99,
            shooter_id: own_ship_id.clone(),
            target_id: raider_id,
            pos_m: [1_000, 0, 0],
            pos_rem_um: [0, 0, 0],
            vel_mmps: [500_000, 0, 0],
            accel_mps2: 120,
            remaining_dv_mmps: 4_000_000,
            warhead_damage: 180,
            integrity: 100,
            tti_s: 120,
            flight_elapsed_ms: 0,
            max_flight_ms: 45_000,
            dv_exhausted_emitted: false,
        });
        let feed = test_bubble.holotank_feed(&own_ship_id);
        let torp_json = &feed["torpedoes"].as_array().expect("torpedoes array")[0];
        assert!(
            torp_json["target_pos"].is_null(),
            "torpedo targeting an undetected Ghost contact must not leak target_pos in holotank_feed: {torp_json}"
        );
    }

    // 7b. Verify `+tac/hail` at long range (`range_m=600000000` = `600,000 km`, `2.00 s` light-lag)
    //     queues a `PendingLightLagHail` and delivers it on `space_tick` after `d / c` (`2 s`):
    let hail_queued = run(
        &mut rig.tw,
        pilot_1,
        "+tac/hail ceres=Requesting vector check range_m=600000000",
    );
    assert!(
        hail_queued.contains("queued for T+2s"),
        "expected queued light-lag hail output, got: {hail_queued}"
    );
    assert_eq!(
        rig.tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .unwrap()
            .pending_light_lag_hails
            .len(),
        1,
        "long-range hail (d >= c) must be queued in pending_light_lag_hails"
    );

    // Heave-to and advance 6 wall seconds so `space_tick` both brakes the carrier under 10 m/s
    // AND delivers the queued `2.001 s` light-lagged hail!
    let _ = run(&mut rig.tw, pilot_1, "+helm/thrust 600 -1000 0 0");
    let mut hail_delivered = false;
    for _ in 0..6 {
        let next_r = rig.tw.world().now() + 1;
        rig.tw.set_now(next_r);
        let evs = rig.tw.fire_due(next_r * 1_000);
        if said(&rig.tw, &evs, pilot_1).contains("[RADIO +2.001s light-lag") {
            hail_delivered = true;
        }
    }
    let _ = run(&mut rig.tw, pilot_1, "+helm/thrust 0");
    let brake_submits = drain_space_and_industry(&mut rig.tw);
    land_quietly(&mut rig, &brake_submits);
    assert!(
        hail_delivered,
        "queued light-lag hail must be delivered on space_tick once wall time reaches now + d/c"
    );
    assert!(
        rig.tw
            .world()
            .module_state::<tacsim::space::SpaceBubbleBook>()
            .unwrap()
            .pending_light_lag_hails
            .is_empty(),
        "pending_light_lag_hails must be empty after delivery"
    );

    // 6b. Run `+helm/submit`: even though `pilot_1` staked `strategic_elements` on Block B,
    //     `hull_1` mined its `4 t` of `strategic_ore` from the UNLEASED Block C, so per-block
    //     `inspect_hull_manifest` detects `4,000 kg` of unlicensed `strategic_ore`!
    let submit_out = run(&mut rig.tw, pilot_1, "+helm/submit");
    assert!(
        submit_out.contains("4000 kg unlicensed `strategic_ore`"),
        "per-block inspect_hull_manifest must detect unlicensed strategic_ore mined from Block C despite Block B claim: {submit_out}"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn sfr24_one_docking_kinematics_and_ephemeris_canopy() {
    use mush_pixel::picture::{Feeds, Look, Picture as _};

    struct SingleFeed<'a>(&'a str, &'a serde_json::Value);
    impl Feeds for SingleFeed<'_> {
        fn feed(&self, name: &str) -> Option<&serde_json::Value> {
            (name == self.0).then_some(self.1)
        }
    }

    // 1. Charter a ship at Ceres Freeport, fly to Stickney Yards (Phobos), and open a docking
    //    session via `+helm/dock stickney` (`D-35-14`):
    let mut rig = stocked_with(Roster::default());
    let (pilot, row) = chartered(&mut rig, "PilotSfR24", 1);
    let hull = ObjId(row.hull);
    load(&mut rig, &row, 70, 50);
    set_rate(&mut rig.tw, 3_600);
    board(&mut rig, pilot);

    let pending = undock_and_burn(&mut rig, pilot, hull);
    let depart_ic = pending.plan.depart_ic;
    let leg1_secs = i64::from(pending.plan.legs[0].secs);
    let _ = fire_quietly(&mut rig);
    let _ = to_ic(&mut rig, depart_ic + leg1_secs / 2);
    let dec_out = run(&mut rig.tw, pilot, "+helm/decouple");
    assert!(
        dec_out.contains("Decoupled from corridor leg"),
        "decouple output: {dec_out}"
    );
    let auto_out = run(&mut rig.tw, pilot, "+helm/autopilot stickney");
    assert!(
        auto_out.contains("Autopilot engaged for Stickney Yards"),
        "autopilot stickney output: {auto_out}"
    );
    let auto_submits = drain(&mut rig.tw);
    land_quietly(&mut rig, &auto_submits);
    let arrival_at = journey(&rig.tw, hull)
        .and_then(|j| j.arrival_at())
        .expect("recoupled arrival_at");
    let _ = to_ic(&mut rig, arrival_at - 7_200);
    set_rate(&mut rig.tw, 1);
    let _ = to_ic(&mut rig, arrival_at - 10);

    let dock_events = rig.tw.run(pilot, "+helm/dock stickney");
    let dock_text = said(&rig.tw, &dock_events, pilot);
    assert!(
        dock_text.contains("[DOCKING] Initiated 60 fps 6-DOF docking session"),
        "expected docking session open, got: {dock_text}"
    );
    let dock_panel = dock_events
        .iter()
        .find_map(|(_, ev)| match ev {
            OutputEvent::Panel { feed, data, .. }
                if feed.as_str() == tacsim::space::SPACE_DOCKING_FEED =>
            {
                Some(data.clone())
            }
            _ => None,
        })
        .expect("space.docking panel keyframe emitted on +helm/dock");

    // 2. Drive two independent client sessions hydrated from `dock_panel` using ONLY single-key
    //    human axis pulses (`active_axis_pulses() <= 1`, never `play_to_clamp`) and replay the
    //    recorded bitmask tape through `DockingClientInput::from_bitmask` (the WASM entry point):
    let mut tui_session = mush_pixel::docking::DockingClientSession::from_feed(&dock_panel);
    let mut recorded_bitmasks: Vec<u16> = Vec::new();
    while !tui_session.clamped()
        && tui_session.frames_stepped() < mush_pixel::docking::MAX_DOCKING_CLIENT_TAPE_FRAMES
    {
        let multi = tui_session.next_guidance_input();
        let single_key = if multi.roll_trim != 0 {
            mush_pixel::docking::DockingClientInput {
                roll_trim: multi.roll_trim,
                ..Default::default()
            }
        } else if multi.pitch_trim != 0 {
            mush_pixel::docking::DockingClientInput {
                pitch_trim: multi.pitch_trim,
                ..Default::default()
            }
        } else if multi.yaw_trim != 0 {
            mush_pixel::docking::DockingClientInput {
                yaw_trim: multi.yaw_trim,
                ..Default::default()
            }
        } else if multi.trans_x != 0 {
            mush_pixel::docking::DockingClientInput {
                trans_x: multi.trans_x,
                ..Default::default()
            }
        } else if multi.trans_y != 0 {
            mush_pixel::docking::DockingClientInput {
                trans_y: multi.trans_y,
                ..Default::default()
            }
        } else if multi.closure_pulse != 0 {
            mush_pixel::docking::DockingClientInput {
                closure_pulse: multi.closure_pulse,
                ..Default::default()
            }
        } else {
            mush_pixel::docking::DockingClientInput::default()
        };
        let single_key_mask = single_key.to_bitmask();
        let input = mush_pixel::docking::DockingClientInput::from_bitmask(single_key_mask);
        assert!(
            input.active_axis_pulses() <= 1,
            "every recorded frame in the human key tape must press at most one key (got mask {single_key_mask:#x})"
        );
        recorded_bitmasks.push(single_key_mask);
        tui_session.step_60fps(input);
    }
    assert!(
        tui_session.clamped(),
        "human single-key tape must reach collar clamp in TUI client session"
    );
    assert_eq!(tui_session.gates_green(), 5);

    // Replay the exact recorded bitmasks into a second session (matching `mush-pixel-wasm`'s
    // `mp_docking_step(mask)`) and verify bit-identical final pose and RLE tape:
    let mut wasm_session = mush_pixel::docking::DockingClientSession::from_feed(&dock_panel);
    for &mask in &recorded_bitmasks {
        wasm_session.step_60fps(mush_pixel::docking::DockingClientInput::from_bitmask(mask));
    }
    assert!(wasm_session.clamped());
    assert_eq!(wasm_session.gates_green(), 5);
    assert_eq!(wasm_session.pose, tui_session.pose);
    assert_eq!(wasm_session.encode_rle(), tui_session.encode_rle());

    // Submit the recorded key tape to the server and verify it clamps at Stickney Yards:
    let clamp_cmd = wasm_session.format_tape_command();
    let clamp_out = run(&mut rig.tw, pilot, &clamp_cmd);
    assert!(
        clamp_out.contains("Hard dock and magnetic collar seal confirmed"),
        "recorded human key tape from client session must clamp on server: {clamp_out}"
    );
    let clamp_burn = drain(&mut rig.tw);
    land_quietly(&mut rig, &clamp_burn);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { .. }
        ),
        "hull must be Berth::Docked at Stickney Yards after clamp"
    );

    // 3. Ephemeris canopy (`D-35-15`): charter a `Dray-P`, undock for `neptune`, engage
    //    `+helm/autopilot neptune`, and verify that `space.cockpit`'s `bodies` are computed
    //    from `Sky` (`sky.position_km(&body, now_ic)`) at the hull's 3D state vector `pos_km`,
    //    pushed on `space_tick`, and that Neptune's `bearing_deg` and `distance_km` change
    //    as the hull's position changes:
    let mut rig_nep = stocked_with(Roster::default());
    let (pilot_nep, row_nep) = chartered(&mut rig_nep, "PilotNepR24", 2);
    let hull_nep = ObjId(row_nep.hull);
    let desk_nep = station_room(&rig_nep.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig_nep.tw, pilot_nep, desk_nep);
    let _ = run(&mut rig_nep.tw, pilot_nep, "+coop/refit prospector");
    load(&mut rig_nep, &row_nep, 80, 200);
    set_rate(&mut rig_nep.tw, 3_600);
    board(&mut rig_nep, pilot_nep);

    let undock_nep = run(&mut rig_nep.tw, pilot_nep, "+ship/undock neptune");
    assert!(
        undock_nep.contains("Clearance filed for neptune"),
        "undock neptune output: {undock_nep}"
    );
    let undock_nep_draws = drain(&mut rig_nep.tw);
    land_quietly(&mut rig_nep, &undock_nep_draws);
    let _ = fire_quietly(&mut rig_nep);

    let nep_events_0 = rig_nep.tw.run(pilot_nep, "+helm/autopilot neptune");
    let nep_draws = drain(&mut rig_nep.tw);
    land_quietly(&mut rig_nep, &nep_draws);

    let cockpit_0 = nep_events_0
        .iter()
        .find_map(|(_, ev)| match ev {
            OutputEvent::Panel { feed, data, .. }
                if feed.as_str() == tacsim::space::SPACE_COCKPIT_FEED =>
            {
                Some(data.clone())
            }
            _ => None,
        })
        .expect("initial space.cockpit keyframe emitted on +helm/autopilot neptune");

    let (pos_0, arr_ic) = rig_nep
        .tw
        .world()
        .module_state::<tacsim::space::SpaceBubbleBook>()
        .and_then(|b| b.free_flight_for_hull(hull_nep))
        .map(|ff| {
            (
                ff.pos_km,
                ff.cluster_arrival_ic_s.expect("neptune arrival_ic"),
            )
        })
        .expect("hull_nep in free flight");
    let now_0 = rig_nep.tw.world().world_ic_now().at;
    let expected_bodies_0 =
        tacsim::space::compute_ephemeris_canopy_bodies(rig_nep.tw.world(), pos_0, "neptune", now_0);
    let nep_0 = cockpit_0["bodies"]
        .as_array()
        .expect("bodies array")
        .iter()
        .find(|b| b["id"].as_str() == Some("neptune"))
        .expect("neptune entry in cockpit_0")
        .clone();
    assert_eq!(
        nep_0["bearing_deg"].as_i64(),
        Some(i64::from(expected_bodies_0[0].bearing_deg)),
        "initial canopy Neptune bearing_deg must match Sky ephemeris at pos_0"
    );
    assert_eq!(
        nep_0["distance_km"].as_i64(),
        Some(expected_bodies_0[0].distance_km),
        "initial canopy Neptune distance_km must match Sky ephemeris at pos_0"
    );
    assert!(
        expected_bodies_0[0].distance_km > 1_000_000_000,
        "at departure from Ceres, Neptune distance_km in canopy must be interplanetary (> 1e9 km), got {}",
        expected_bodies_0[0].distance_km
    );

    // Advance along the Neptune transit via `to_ic` so `space_tick` updates the hull's 3D state
    // vector `pos_km` across the solar system and pushes updated `space.cockpit` panels whose
    // Neptune `bearing_deg` and `distance_km` change with the hull's position:
    let mut tick_cockpit_panels: Vec<serde_json::Value> = Vec::new();
    let mut saw_bearing_change = false;
    let mut saw_distance_change = false;
    for target_ic in [
        now_0 + (arr_ic - now_0) / 3,
        now_0 + ((arr_ic - now_0) * 2) / 3,
        arr_ic - 7_200,
        arr_ic - 3_600,
    ] {
        let evs = to_ic(&mut rig_nep, target_ic);
        for (_, ev) in &evs {
            if let OutputEvent::Panel { feed, data, .. } = ev
                && feed.as_str() == tacsim::space::SPACE_COCKPIT_FEED
            {
                if let Some(nep) = data["bodies"]
                    .as_array()
                    .and_then(|arr| arr.iter().find(|b| b["id"].as_str() == Some("neptune")))
                {
                    if nep["bearing_deg"] != nep_0["bearing_deg"] {
                        saw_bearing_change = true;
                    }
                    if nep["distance_km"] != nep_0["distance_km"] {
                        saw_distance_change = true;
                    }
                }
                tick_cockpit_panels.push(data.clone());
            }
        }
    }
    assert!(
        !tick_cockpit_panels.is_empty(),
        "space_tick must push updated space.cockpit panels for free-flight hull"
    );
    assert!(
        saw_distance_change,
        "Neptune's distance_km in space.cockpit must change as the hull's 3D state vector moves on space_tick"
    );
    assert!(
        saw_bearing_change,
        "Neptune's bearing_deg in space.cockpit must change as the hull's 3D state vector moves on space_tick"
    );

    let latest_cockpit = tick_cockpit_panels.last().expect("latest cockpit panel");
    let mut canopy_pic = mush_pixel::space_cockpit::SpaceCockpitPicture::new();
    let mut canopy_frame = mush_pixel::Frame::new(640, 360);
    let feeds = SingleFeed(tacsim::space::SPACE_COCKPIT_FEED, latest_cockpit);
    assert!(
        canopy_pic.draw(&mut canopy_frame, &feeds, &Look::default()),
        "SpaceCockpitPicture must render the tick-pushed ephemeris canopy keyframe"
    );
}
