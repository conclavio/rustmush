//! **Doc 20 cut SM.4b acceptance — driving and the map** (doc 20 §1 beats 4, 7 and 9, §4.1,
//! §4.3, §5, §8, §10, §11; D-20-6, D-20-11, D-20-13; the SM.4b brief's acceptance V1–V5 and its
//! tests D1–D15 in 3.15).
//!
//! A First Captain whose Dray-P is settled on a site's pad drives a rig: `+tv/1 drive C117`
//! previews the route — its length, its IC time, the charge it costs and what is left after the
//! trip home — and `go` commits ONE `tacsim_surface_act` (`act: drive`) whose prepare writes the
//! frozen leg the command planned with ONE A*; the `tacsim.surface` tick projects the leg for
//! interpolation, places the rig at its end at the due instant and finishes it through
//! `tacsim_surface_done`, and the operator hears `ARRIVED`. A recover from anywhere on the ground
//! drives home and climbs aboard as one act. `+tv/map` and `+tv/map site` print the `site.map`
//! frame, every ore glyph the viewer's own reading.
//!
//! The first test is **the freeze census** (the brief's 3.1a, a hard gate run before anything
//! after the planner was built): from the Dray-P's ramp cell to all 64 field middles of every lane
//! mining site, over each site's keyless terrain (`tacsim::surface::ground::site_terrain`), the
//! drive planner's one A* and its freeze; no site may refuse more than 3 of its 64. Its accepted
//! set — every middle of every site, all at a 1-cell tolerance when the census was first run —
//! is where the world tests' targets come from.
//!
//! Composed on the composition root (`knowing.rs`'s rig, copied: the per-file precedent),
//! **booted keyed** with the geology key handed in through `tacsim::surface::install_geology`
//! on every boot (save where a test boots dark on purpose). Every act goes through the real verb,
//! the real land path and the real tick; rigs are put in place by the tick-batch placement seam a
//! test may call directly (`mush_sim::place`, `staff_place`), never a fake drive verb.

mod support;

#[path = "support/geology_keyed.rs"]
mod geology_keyed;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use mush_core::{
    AccountId, AccountIdentity, ActionStateBootRecord, Audience, AuthoritativeSubmit, CanonicalId,
    CommitToken, GOD, IdempotencyKey, IdempotencyScope, ModuleWrite, ObjId, OfferExecutionContext,
    OfferSubmission, PerceivedRef, RequestUid, ScopedIdempotencyKey, World,
};
use mush_economy::{EconomyConfig, VaultId};
use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
use mush_proto::OutputEvent;
use mush_sim::data::{SpatialParent, SpatialState};
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde_json::{Value, json};
use smol_str::SmolStr;
use tacsim::definitions::content::{DRAY_P, LANE_JSON, LANE_PACK, TV_PROSPECTOR};
use tacsim::definitions::tools::{BatteryDef, ToolDef, tools_table};
use tacsim::pack::{PackEntry, RulesPack, load_pack};
use tacsim::port::tv::{
    ARRIVED, DRIVE_HERE, DRIVE_OFF_SITE, DRIVE_USAGE, GROUND_COLD, IMPLIED_LAUNCH,
    INSTRUMENTS_DARK, LAUNCH_OPEN, MAP_FAMILY_UNKNOWN, MAP_NO_RIGS, RECOVER_SHORT, RECOVERED,
    RESERVE_LOW, TV_DARK, TWO_LEGS,
};
use tacsim::port::{HullRow, PortHandles, PortMap, PortMapState, Ticket, TicketId, room_key};
use tacsim::surface::address::{pocket_address, pocket_of_block_index};
use tacsim::surface::cursor::{CursorRow, POCKET_CURSOR_SCHEMA, PocketCursor};
use tacsim::surface::drive::{
    DRIVE_LOCOMOTION, DRIVE_PATH_BUDGET_CELLS, DriveRates, HomeField, LEG_TOLERANCE_CELLS, Leg,
    LegRefused, cu_of_cost, drive_leg, home_cu_to, recover_leg, secs_of_cost,
};
use tacsim::surface::geology::{BodyGeology, FieldGround, field_ground};
use tacsim::surface::ground::{SiteGeologySpec, SiteGround, site_ground_sync};
use tacsim::surface::reading::read_r2;
use tacsim::surface::{
    ActKind, ActParams, ActResult, Charge, ChartKey, ChartRecord, GeologyBoot, GeologyState,
    RigKey, RigRecord, SURFACE_ACT_ACTION, SURFACE_DONE_ACTION, TvRec, TvRecState, charge_at,
    pocket_row_text,
};
use tacsim::{TacsimConfig, TacsimStore};

use geology_keyed::{KEY_1, mac, secret};

/// The Dray-P's stern ramp on a site's grid.
const RAMP: (i64, i64, u32) = (128_000, 124_500, 180_000);
/// The ramp's cell.
const RAMP_CELL: (i64, i64) = (128, 124);
/// Ahuna Flank's port entry.
const AHUNA: &str = "ahuna_port";

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

// ---------------------------------------------------------------------------
// The rig (excursions.rs's, booted keyed)
// ---------------------------------------------------------------------------

/// The composition: `commerce.rs`'s bare roster (no sheet, the test world's layer, the
/// shipped game content).
#[derive(Clone, Copy, Default)]
struct Roster;

impl Roster {
    fn modules(self) -> Vec<Box<dyn mush_core::GameModule>> {
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
        mods.push(comp::tacsim_test_world_module_wired(cfg, true, inventory));
        mods.push(comp::content_module_with(true, true).expect("the content module composes"));
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
            mush_core::OfferSecret::new(1, [0x52; 32]).expect("an offer secret"),
            [],
        )
        .expect("an offer key ring"),
    );
}

/// **Compose over `world`, hand the key in (or not), then run the boot pass over `store`**
/// (`build_engine`'s order).
fn boot_over(
    world: World,
    store: HashMap<SmolStr, mush_core::ModuleBootData>,
    keyed: bool,
) -> TestWorld {
    let (_, registry) = mush_testkit::boot_world();
    let mut engine = mush_core::Engine::with_modules(world, registry, Roster.modules())
        .expect("the roster composes");
    if keyed {
        assert!(tacsim::surface::install_geology(
            &mut engine.world,
            GeologyBoot::Key(secret(KEY_1))
        ));
    }
    let (mut out, mut fx) = (mush_core::Outbox::new(), mush_core::Effects::new());
    engine
        .modules_loaded(store, &mut out, &mut fx)
        .expect("the world boots");
    let tw = TestWorld::from_engine(engine);
    no_refused_writes(&tw);
    tw
}

/// Boot (keyed or not), settle and stock every port, at world rate 1.
fn stocked_with(keyed: bool) -> Rig {
    let (world, _) = mush_testkit::boot_world();
    let mut tw = boot_over(world, HashMap::new(), keyed);
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
    set_rate(&mut rig.tw, 1);
    rig
}

/// The keyed rig.
fn stocked() -> Rig {
    stocked_with(true)
}

/// Fold every write since the first boot and reboot keyed into the same roster; every
/// connection drops (a real restart has no sessions) and `reconnect` come back.
fn reboot(rig: Rig, reconnect: &[ObjId]) -> Rig {
    reboot_with(rig, reconnect, true)
}

/// [`reboot`], keyed or not (a restart without the geology key darkens the layer).
fn reboot_with(rig: Rig, reconnect: &[ObjId], keyed: bool) -> Rig {
    let Rig {
        mut tw,
        oracle,
        mut store,
        typed,
    } = rig;
    fold_module_ops_into(&mut store, tw.take_module_ops());
    let mut world = tw.into_engine().world;
    let connected: Vec<ObjId> = world
        .iter()
        .map(|o| o.id)
        .filter(|id| world.is_connected(*id))
        .collect();
    for id in connected {
        world.set_connected(id, false);
    }
    let mut tw = boot_over(world, store.clone(), keyed);
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
    for who in reconnect {
        let _ = tw.connect(*who);
    }
    Rig {
        tw,
        oracle,
        store,
        typed,
    }
}

/// Fold every write and reboot with the stored `tacsim:` document `key` edited.
fn reboot_editing(
    mut rig: Rig,
    key: &str,
    reconnect: &[ObjId],
    edit: impl FnOnce(&mut serde_json::Value),
) -> Rig {
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    let doc = rig
        .store
        .values_mut()
        .flat_map(|data| data.kv.iter_mut())
        .find(|(k, _)| k.as_str() == key)
        .map(|(_, value)| value)
        .expect("the document is in the store");
    edit(doc);
    reboot(rig, reconnect)
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

fn port_map(tw: &TestWorld, entry: &str) -> PortMap {
    match store(tw).port_map(&cid(LANE_PACK), &cid(entry)) {
        PortMapState::Present(map) => map.clone(),
        other => panic!("{entry}'s map: {other:?}"),
    }
}

fn lane() -> RulesPack {
    load_pack(LANE_JSON).expect("the lane verifies")
}

fn station_room(tw: &TestWorld, entry: &str, local: &str) -> ObjId {
    let def = match lane().entry(&cid(entry)) {
        Some(PackEntry::Port(def)) => def.clone(),
        other => panic!("{entry} is a port entry: {other:?}"),
    };
    let key = room_key(&def.area, &format!("{}.{local}", def.area)).expect("a room key");
    tacsim::port::apply::room_by_key(tw.world(), &key).expect("the room is on this world")
}

fn teleport(tw: &mut TestWorld, who: ObjId, room: ObjId) {
    let said = run(tw, GOD, &format!("@teleport #{}=#{}", who.0, room.0));
    assert_eq!(
        tw.world().get(who).and_then(|o| o.location),
        Some(room),
        "the teleport landed: {said}"
    );
}

/// A fresh request/commit token per landing, across the process.
fn next_token() -> [u8; 16] {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut bytes = [0x6c_u8; 16];
    bytes[..4].copy_from_slice(&n.to_be_bytes());
    bytes
}

/// Keep a prepared bundle's typed post-images for a reboot.
fn keep(rig: &mut Rig, ops: Vec<mush_core::ModuleOp>) {
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

/// Drive one queued submission the way the world task does, keeping its typed post-images
/// for a reboot; the error carries the prepare's own message.
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
    keep(rig, ops);
    Ok(())
}

/// Every queued submission of the port's lanes, dropping other modules'; asserts none kept of
/// tacsim's is GOD's.
fn drain(tw: &mut TestWorld) -> Vec<AuthoritativeSubmit> {
    let mut kept = Vec::new();
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        let owner = submit.action.key().owner.as_str();
        if owner == tacsim::NS {
            assert_ne!(submit.actor, GOD, "GOD never submits from a tick");
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

/// One fire (6 s on), its submissions drained (not landed).
fn fire(rig: &mut Rig) -> (Vec<(Audience, OutputEvent)>, Vec<AuthoritativeSubmit>) {
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    (events, queued)
}

/// The surface finishes a fire queued.
fn finishes(queued: &[AuthoritativeSubmit]) -> Vec<&AuthoritativeSubmit> {
    queued
        .iter()
        .filter(|submit| submit.action.key().id.as_str() == SURFACE_DONE_ACTION)
        .collect()
}

/// Fire and land the port tick until every port is stocked and listed.
fn stock(rig: &mut Rig) {
    for _ in 0..40 {
        let out = run(&mut rig.tw, GOD, "@tacsim/port status");
        let gated: Vec<&str> = out
            .lines()
            .filter(|line| {
                // Doc 20 cut SM.6 (D2): a keyed site laid up and still opening is the site
                // clerk's work, not the stock clerks'; a rig that hops to a site opens it.
                line.ends_with(']')
                    && !line.starts_with("sky: ")
                    && !line.starts_with("geology: ")
                    && !(line.contains(" site: ")
                        && (line.contains("; opening, ")
                            || line.contains("; opening: its ground is still being mapped")))
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

/// A pilot on a gateway account of their own, standing at the Registry desk.
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

/// A chartered, licensed pilot whose hull the yard has made a Dray-P.
fn prospector(rig: &mut Rig, who: &str, seed: i64, name: &str) -> (ObjId, HullRow) {
    let pilot = pilot(rig, who, seed);
    let out = run(&mut rig.tw, pilot, &format!("+coop/enrol {name}"));
    assert!(out.contains(&format!("now the {name}")), "{out}");
    let charter = tacsim::port::live_charter(rig.tw.world(), pilot).expect("a charter");
    let row = port_map(&rig.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|row| row.hull == charter.hull.0)
        .expect("the hull's row");
    let out = run(&mut rig.tw, pilot, "+coop/refit prospector");
    assert!(out.contains("as a Dray-P"), "{out}");
    support::licence::buy_licence(&mut rig.tw, pilot);
    (pilot, row)
}

/// Seed `qty` of `good` into `vault`'s pool through `lot` (the inventory's own path).
fn seed_pool(rig: &mut Rig, vault: VaultId, good: &str, qty: u64, lot: LotId) {
    let endpoint = EndpointRef(vault);
    let good = cid(good);
    let families = rig
        .tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
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
        keep(rig, ops);
    }
}

/// `tank` t and `locker` crew-days into the hold.
fn load(rig: &mut Rig, row: &HullRow, tank: u64, locker: u64, lot: u64) {
    let stowage = ObjId(row.rooms[&cid("stowage")]);
    let vault = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy")
        .snapshot()
        .vault_by_backing(&mush_economy::CustodyRef::SpatialInterior(stowage))
        .expect("the hold vault")
        .id;
    seed_pool(rig, vault, "propellant", tank, LotId(lot));
    seed_pool(rig, vault, "provisions", locker, LotId(lot + 1));
}

/// Board from C01.
fn board(rig: &mut Rig, pilot: ObjId) {
    let gallery = station_room(&rig.tw, "ceres_port", "room.c01_berth_gallery");
    teleport(&mut rig.tw, pilot, gallery);
    let out = run(&mut rig.tw, pilot, "+ship/board");
    assert!(out.contains("You cycle the lock into the "), "{out}");
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

/// Sweep to the first real second whose IC instant is at or past `target`; the events and
/// the submissions queued (not landed).
fn advance_to(
    rig: &mut Rig,
    target: i64,
) -> (Vec<(Audience, OutputEvent)>, Vec<AuthoritativeSubmit>) {
    let rate = i64::from(world_rate(&rig.tw).max(1));
    let ahead = target.saturating_sub(ic_now(&rig.tw)).max(0);
    let real = ((ahead + rate - 1) / rate).max(1);
    let t = rig.tw.world().now().saturating_add(real);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    (events, queued)
}

/// [`advance_to`], landing what it queued quietly.
fn to_ic(rig: &mut Rig, target: i64) -> Vec<(Audience, OutputEvent)> {
    let (events, queued) = advance_to(rig, target);
    land_quietly(rig, &queued);
    events
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

/// A command and the submissions it queued.
fn command(rig: &mut Rig, who: ObjId, line: &str) -> (String, Vec<AuthoritativeSubmit>) {
    let out = run(&mut rig.tw, who, line);
    let queued = drain(&mut rig.tw);
    (out, queued)
}

/// File a passage with `line`, land its burn, launch it on the next fire; the ticket.
fn file_and_launch(rig: &mut Rig, pilot: ObjId, hull: ObjId, line: &str) -> Ticket {
    let (out, queued) = command(rig, pilot, line);
    assert!(out.contains("Clearance filed for "), "{line}: {out}");
    assert_eq!(queued.len(), 1, "one burn: {out}");
    let seq = match store(&rig.tw).voyage_state(hull) {
        tacsim::port::voyage::VoyageState::Present(voyage) => voyage
            .pending
            .as_ref()
            .map(|p| p.seq)
            .expect("a filed passage"),
        other => panic!("the sailing plan: {other:?}"),
    };
    land_all(rig, &queued);
    let t = ticket(&mut rig.tw, hull, seq).expect("the ticket");
    let (_, queued) = fire(rig);
    land_quietly(rig, &queued);
    t
}

/// Lease one rig with `+tv/lease go` at Occator's counter.
fn lease_one(rig: &mut Rig, pilot: ObjId) {
    let counter = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, pilot, counter);
    let out = run(&mut rig.tw, pilot, "+tv/lease go");
    assert!(
        out.contains("Occator Water & Propellant leases you "),
        "{out}"
    );
}

/// The live lease records of `pilot`, by slot: their rigs.
fn rigs_of(tw: &TestWorld, pilot: ObjId) -> Vec<ObjId> {
    let mut records: Vec<(u8, ObjId)> = store(tw)
        .tv_leases_of(pilot)
        .filter(|record| record.live())
        .map(|record| (record.slot, record.tv))
        .collect();
    records.sort_unstable();
    records.into_iter().map(|(_, tv)| tv).collect()
}

/// **A Dray-P on Ahuna Flank's pad**, its pilot aboard, `n_rigs` rigs leased; the site's ground
/// warmed (the arrival asked for it; the surface tick builds it).
fn on_pad_as(
    rig: &mut Rig,
    who: &str,
    seed: i64,
    name: &str,
    n_rigs: usize,
) -> (ObjId, HullRow, Vec<ObjId>) {
    let (pilot, row) = prospector(rig, who, seed, name);
    for _ in 0..n_rigs {
        lease_one(rig, pilot);
    }
    load(
        rig,
        &row,
        20,
        20,
        9_100 + u64::try_from(seed).unwrap_or(0) * 10,
    );
    board(rig, pilot);
    let hull = ObjId(row.hull);
    // Doc 20 cut SM.6 (D2): a keyed site's pad is closed until its clerk has opened every
    // deposit, so the site is opened first.
    open_ahuna(rig);
    let t = file_and_launch(rig, pilot, hull, "+ship/undock ahuna");
    let _ = to_ic(rig, t.plan.depart_ic + t.plan.total_s);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == AHUNA
        ),
        "the hull is settled on Ahuna Flank's pad"
    );
    warm(rig);
    let tvs = rigs_of(&rig.tw, pilot);
    assert_eq!(tvs.len(), n_rigs);
    (pilot, row, tvs)
}

/// **Open Ahuna Flank** (doc 20 cut SM.6, D2; `support::sites::open_site`), keeping the landed
/// bundles' typed post-images for a reboot; the port fires it took. A no-op once it is open.
fn open_ahuna(rig: &mut Rig) -> u32 {
    let mut kept = Vec::new();
    let fires = support::sites::open_site_keeping(&mut rig.tw, AHUNA, &mut |ops| kept.push(ops));
    for ops in kept {
        keep(rig, ops);
    }
    fires
}

/// [`on_pad_as`] for the usual pilot.
fn on_pad(rig: &mut Rig, n_rigs: usize) -> (ObjId, HullRow, Vec<ObjId>) {
    on_pad_as(rig, "Pilot", 1, "Salt Moth", n_rigs)
}

/// Fire until Ahuna Flank's ground is cached — and, since doc 20 cut SM.4b, its home field built
/// (a scan or a sounding from the ground reserves the real trip home; the surface tick builds the
/// home the fire after the site).
fn warm(rig: &mut Rig) {
    for _ in 0..8 {
        let homed = rig
            .tw
            .world()
            .module_state::<GeologyState>()
            .is_some_and(|state| !state.homes().is_empty());
        if cached(&rig.tw) && homed {
            return;
        }
        let (_, queued) = fire(rig);
        land_quietly(rig, &queued);
    }
    panic!("Ahuna Flank's ground was not warmed");
}

/// Whether any site's ground is cached.
fn cached(tw: &TestWorld) -> bool {
    tw.world()
        .module_state::<GeologyState>()
        .is_some_and(|state| !state.sites().is_empty())
}

fn parent(tw: &TestWorld, obj: ObjId) -> SpatialParent {
    tw.world()
        .module_state::<SpatialState>()
        .expect("the simulation")
        .parent_of(obj)
}

/// Ahuna Flank's passive domain.
fn ahuna_domain(tw: &TestWorld) -> mush_sim::DomainId {
    mush_sim::DomainId(port_map(tw, AHUNA).domain.expect("a domain"))
}

/// Whether `tv` stands `Direct` on Ahuna's domain at `(x, y, heading)`.
fn on_ground_at(tw: &TestWorld, tv: ObjId, at: (i64, i64, u32)) -> bool {
    matches!(parent(tw, tv), SpatialParent::Direct { domain, pose }
        if domain == ahuna_domain(tw)
            && (pose.x_cm, pose.y_cm, pose.heading_mdeg) == at)
}

/// A rig's typed record.
fn record_of(tw: &mut TestWorld, tv: ObjId) -> Option<RigRecord> {
    let h = handles(tw);
    typed(tw, &h.families.excursions, &RigKey(tv))
}

/// A principal's chart row for Ahuna Flank.
fn chart_of(tw: &mut TestWorld, who: ObjId) -> Option<ChartRecord> {
    let h = handles(tw);
    typed(
        tw,
        &h.families.charts,
        &ChartKey {
            principal: who,
            site: cid(AHUNA),
        },
    )
}

/// A rig's projection.
fn projection(tw: &TestWorld, tv: ObjId) -> Option<TvRec> {
    match store(tw).tv_rec(tv) {
        Some(TvRecState::Present(rec)) => Some(rec.clone()),
        Some(TvRecState::Unreadable) => panic!("the projection is unreadable"),
        None => None,
    }
}

/// The act in flight's due instant.
fn due_of(tw: &mut TestWorld, tv: ObjId) -> i64 {
    record_of(tw, tv)
        .and_then(|record| record.act)
        .map(|act| act.due_ic)
        .expect("an act in flight")
}

/// **`line` as `who`, and exactly one start queued**, landed; the reply.
fn act_go(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let (out, queued) = command(rig, who, line);
    assert_eq!(queued.len(), 1, "{line}: one start: {out}");
    assert_eq!(queued[0].actor, who, "the start is the operator's");
    assert_eq!(queued[0].action.key().id.as_str(), SURFACE_ACT_ACTION);
    land_all(rig, &queued);
    out
}

/// **Finish `tv`'s act**: to its due instant (the fire there queues the finish, landed), then
/// one more fire for the tell; what each of `who` heard over both, in order.
fn finish_heard(rig: &mut Rig, tv: ObjId, who: &[ObjId]) -> Vec<String> {
    let due = due_of(&mut rig.tw, tv);
    let (events, queued) = advance_to(rig, due);
    let mut heard: Vec<String> = who.iter().map(|w| said(&rig.tw, &events, *w)).collect();
    assert_eq!(
        finishes(&queued).len(),
        1,
        "one finish at the due instant: {queued:?}"
    );
    land_all(rig, &queued);
    let (events, queued) = fire(rig);
    land_all(rig, &queued);
    for (line, w) in heard.iter_mut().zip(who) {
        line.push('\n');
        line.push_str(&said(&rig.tw, &events, *w));
    }
    heard
}

/// [`finish_heard`] for one listener.
fn finish(rig: &mut Rig, tv: ObjId, who: ObjId) -> String {
    finish_heard(rig, tv, &[who]).remove(0)
}

/// How many times `line` appears in `text`.
fn count(text: &str, line: &str) -> usize {
    text.matches(line).count()
}

/// Staff place `tv` `Direct` on Ahuna's domain at `(x, y)`, facing south.
fn staff_place(rig: &mut Rig, tv: ObjId, x: i64, y: i64) {
    let domain = ahuna_domain(&rig.tw);
    let placed = support::with_ctx(&mut rig.tw, |ctx| {
        let posture = mush_sim::data::passive_rules_value(serde_json::json!({}))
            .map_err(|e| e.to_string())?;
        mush_sim::place(
            ctx,
            tv,
            domain,
            mush_sim::data::Pose {
                x_cm: x,
                y_cm: y,
                z_cm: 0,
                heading_mdeg: 180_000,
                posture,
            },
        )
    });
    placed.expect("staff place the rig");
}

/// Staff move `tv` to the centre of site cell `(x, y)` (the placement seam).
fn to_cell(rig: &mut Rig, tv: ObjId, (x, y): (i64, i64)) {
    staff_place(rig, tv, x * 1_000 + 500, y * 1_000 + 500);
}

/// Write `record` into the family through a typed transaction (staff's hand).
fn put_record(rig: &mut Rig, record: RigRecord) {
    let family = handles(&rig.tw).families.excursions;
    let engine = rig.tw.engine_mut();
    let prepared = {
        let mut txn = engine
            .action_transaction(&family)
            .expect("an excursion transaction");
        let key = record.key();
        let exists = txn.get(&family, &key).expect("reads").is_some();
        if exists {
            txn.replace(&family, &key, record).expect("replaced");
        } else {
            txn.insert(&family, &key, record).expect("inserted");
        }
        txn.prepare().expect("prepares")
    };
    let (ops, install) = prepared.into_parts();
    engine.install_prepared_action(install).expect("installs");
    keep(rig, ops);
}

/// Write `cursor` into the pocket cursor family through a typed transaction (staff's hand; SM.7
/// is the real writer).
fn put_cursor(rig: &mut Rig, cursor: PocketCursor) {
    let family = handles(&rig.tw).families.pockets;
    let engine = rig.tw.engine_mut();
    let prepared = {
        let mut txn = engine
            .action_transaction(&family)
            .expect("a cursor transaction");
        let key = cursor.deposit;
        let exists = txn.get(&family, &key).expect("reads").is_some();
        if exists {
            txn.replace(&family, &key, cursor).expect("replaced");
        } else {
            txn.insert(&family, &key, cursor).expect("inserted");
        }
        txn.prepare().expect("prepares")
    };
    let (ops, install) = prepared.into_parts();
    engine.install_prepared_action(install).expect("installs");
    keep(rig, ops);
}

/// A built start of `act` (the start's wire word) for `tv` at `ordinal`.
fn start(rig: &Rig, pilot: ObjId, tv: ObjId, act: &str, ordinal: u32) -> AuthoritativeSubmit {
    AuthoritativeSubmit {
        actor: pilot,
        action: handles(&rig.tw).surface_act.expect("registered"),
        subject: PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters: serde_json::json!({ "tv": tv.0.to_string(), "act": act, "ordinal": ordinal }),
        key: tacsim::surface::act_key(tv, ordinal).expect("a key"),
    }
}

/// Launch rig 1 and finish it (it stands at the ramp).
fn launched(rig: &mut Rig, pilot: ObjId, tv: ObjId) {
    let _ = act_go(rig, pilot, "+tv/1 launch go");
    let _ = finish(rig, tv, pilot);
    assert!(on_ground_at(&rig.tw, tv, RAMP));
}

// ---------------------------------------------------------------------------
// The ground, read the way a test may: the lane, the keyed geology, the cache
// ---------------------------------------------------------------------------

fn battery() -> BatteryDef {
    *tools_table()
        .expect("the tools table")
        .values()
        .find_map(ToolDef::battery)
        .expect("a battery row")
}

/// A lane site's frame (`sites.rs`'s `site_frame`).
fn site_frame(location: &str) -> mush_atlas::window::WindowFrame {
    let region = format!("region.{location}");
    mush_atlas::window::WindowFrame {
        region: mush_atlas::RecordKey::new(cid(&region), cid(&region)),
        surfaces: vec![mush_atlas::SurfaceRef::Ground],
        bbox: mush_atlas::RectCm {
            min: mush_atlas::PointCm { x: 0, y: 0 },
            max: mush_atlas::PointCm {
                x: 256_000,
                y: 256_000,
            },
        },
        cell_cm: 1_000,
    }
}

/// A lane site's spec from its own lane entry and the world's installed body table.
fn spec_of(tw: &TestWorld, entry: &str) -> (SiteGeologySpec, Arc<BodyGeology>, String) {
    let pack = lane();
    let def = match pack.entry(&cid(entry)) {
        Some(PackEntry::Port(def)) => def.clone(),
        other => panic!("{entry} is a port entry: {other:?}"),
    };
    let location = match pack.entry(&def.location) {
        Some(PackEntry::Location(location)) => location.clone(),
        other => panic!("{} is a location: {other:?}", def.location),
    };
    let body = tacsim::surface::body_geology(tw.world(), location.body.as_ref().expect("a body"))
        .expect("the body's table is installed");
    let spec =
        tacsim::port::site::geology_spec(def.site.as_ref().expect("a site"), &location, &body)
            .expect("a spec");
    (spec, body, def.location.to_string())
}

/// Ahuna Flank's keyed ground, as a test reads it.
struct Ground {
    spec: SiteGeologySpec,
    body: Arc<BodyGeology>,
    ground: Arc<SiteGround>,
}

impl Ground {
    /// The ground surface of the composite.
    fn surface(&self) -> &mush_atlas::window::SurfaceComposite {
        self.ground
            .composite
            .surface(&mush_atlas::SurfaceRef::Ground)
            .expect("a ground surface")
    }

    /// **The truth of a site field**, computed independently of the render: `field_ground` at the
    /// body lattice field under the field's block's shift — and held to the site table the
    /// cache built.
    fn field(&self, (fx, fy): (i64, i64)) -> FieldGround {
        let shift = self.spec.shifts[usize::try_from((fy / 2) * 4 + fx / 2).expect("a block")];
        let truth = field_ground(
            &mac(KEY_1),
            &self.body,
            (self.spec.origin.0 / 8 + fx, self.spec.origin.1 / 8 + fy),
            shift,
        )
        .expect("the field's ground");
        let (words, counts) = self
            .ground
            .geology
            .field(fx, fy)
            .expect("the site table's field");
        assert_eq!((truth.words, truth.counts), (words, counts), "({fx}, {fy})");
        truth
    }

    /// The truth of a site pocket, held to the site table.
    fn pocket(&self, (px, py): (i64, i64)) -> [tacsim::surface::Word; 4] {
        let row = self
            .field((px / 8, py / 8))
            .pocket(px % 8, py % 8)
            .expect("a pocket");
        let table = self
            .ground
            .geology
            .pocket(px, py)
            .expect("the site table's pocket");
        assert_eq!(row.classes, table.classes, "({px}, {py})");
        read_r2(&row)
    }
}

/// Ahuna Flank's ground (a cache hit once warm).
fn ahuna(tw: &mut TestWorld) -> Ground {
    let (spec, body, location) = spec_of(tw, AHUNA);
    let ground =
        site_ground_sync(tw.world_mut(), &site_frame(&location), &spec).expect("the site builds");
    Ground { spec, body, ground }
}

// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// The charge, by staff's hand (knowing.rs's)
// ---------------------------------------------------------------------------

/// Put a rig's charge at `value_cu` in (or out of) the bay at the world's IC now.
fn put_charge(rig: &mut Rig, tv: ObjId, value_cu: u32, in_bay: bool) -> Charge {
    let charge = Charge {
        value_cu,
        at_ic: ic_now(&rig.tw),
        in_bay,
    };
    let mut record = record_of(&mut rig.tw, tv).unwrap_or_else(|| RigRecord::fresh(tv, &battery()));
    record.charge = charge;
    put_record(rig, record);
    charge
}

/// Whole percent of the Mk1 battery.
fn pct(cu: u32) -> u32 {
    cu * 100 / battery().capacity_cu
}
/// Staff take every battery out of the one tools table the layer reads (`PortHandles`
/// `.surface_tools`): the rig's battery row no longer resolves (`excursions.rs`'s).
fn unbattery(rig: &mut Rig) {
    let handles = rig
        .tw
        .world_mut()
        .module_state_mut::<PortHandles>()
        .expect("port handles");
    let tools = handles
        .surface_tools
        .as_ref()
        .map(|tools| {
            tools
                .iter()
                .filter(|(_, tool)| tool.battery().is_none())
                .map(|(id, tool)| (id.clone(), tool.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .expect("the tools table");
    handles.surface_tools = Some(Arc::new(tools));
}

// ---------------------------------------------------------------------------
// Doc 20 cut SM.4b: the drive's figures, as a test reads them
// ---------------------------------------------------------------------------

/// The TV's driving figures, from content: the unit's tracked ceiling and the battery row.
fn rates(tw: &TestWorld) -> DriveRates {
    let speed = store(tw)
        .definitions()
        .specs
        .get(&cid(TV_PROSPECTOR))
        .and_then(|spec| spec.mobility.tracked.as_ref())
        .map(|tracked| tracked.max_speed_cms)
        .expect("the TV drives on tracks");
    DriveRates {
        max_speed_cms: speed,
        cu_per_nominal_cell: battery().drive_cu_per_nominal_cell,
    }
}

/// The atlas's locomotion table.
fn table() -> &'static mush_atlas::physics::LocomotionTable {
    mush_atlas::physics::LocomotionTable::shipped()
}

/// The reserve floor: `capacity × reserve_permille / 1000` (5,000 cu on the Mk1).
fn floor() -> u32 {
    let b = battery();
    b.capacity_cu * b.reserve_permille / 1_000
}

/// Ahuna Flank's home field for the Dray-P's ramp cell, as the world built it.
fn home_field(tw: &TestWorld) -> Arc<HomeField> {
    home_field_at(tw, AHUNA)
}

/// A site's home field for the Dray-P's ramp cell.
fn home_field_at(tw: &TestWorld, entry: &str) -> Arc<HomeField> {
    tw.world()
        .module_state::<GeologyState>()
        .and_then(|state| state.home_field(&cid(entry), RAMP_CELL))
        .expect("the home field is built")
}

/// The leg the planner gives from `from_cm` to `to` over Ahuna Flank's cached composite.
fn expect_leg(rig: &mut Rig, from_cm: (i64, i64), to: (i64, i64)) -> Leg {
    let rates = rates(&rig.tw);
    let ground = ahuna(&mut rig.tw);
    drive_leg(ground.surface(), table(), from_cm, to, &rates).expect("a leg")
}

/// A cell's centre, centimetres.
fn centre((x, y): (i64, i64)) -> (i64, i64) {
    (x * 1_000 + 500, y * 1_000 + 500)
}

/// The act in flight.
fn act_of(tw: &mut TestWorld, tv: ObjId) -> tacsim::surface::Act {
    record_of(tw, tv)
        .and_then(|record| record.act)
        .expect("an act in flight")
}

/// The rig's charge value.
fn value_of(tw: &mut TestWorld, tv: ObjId) -> u32 {
    record_of(tw, tv)
        .map(|record| record.charge.value_cu)
        .expect("a record")
}

/// **The `site.map` frame `who` gets** through the feed's own query (the engine's query seam,
/// the typed snapshot): `args` the feed's params.
fn map_frame(rig: &mut Rig, who: ObjId, args: Value) -> Value {
    let (mut out, mut fx) = (mush_core::Outbox::new(), mush_core::Effects::new());
    rig.tw.engine_mut().step(
        mush_core::Inbound::Query {
            conn: 7,
            actor: who,
            id: 1,
            name: SmolStr::new("site.map"),
            args,
        },
        &mut out,
        &mut fx,
    );
    out.events()
        .iter()
        .find_map(|(_, event)| match event {
            OutputEvent::Reply { event, .. } => match event.as_ref() {
                OutputEvent::Panel { data, .. } => Some(data.clone()),
                _ => None,
            },
            _ => None,
        })
        .expect("a site.map frame")
}

/// A frame's rows as glyph strings, row 0 the south edge, through the TUI's own decoder.
fn frame_rows(frame: &Value) -> Vec<String> {
    let width = frame["width"].as_u64().expect("a width") as usize;
    let height = frame["height"].as_u64().expect("a height") as usize;
    let rows = frame["rows"].as_array().expect("rows");
    mush_ansi::panel::window_rows(rows, width, height, 0, 0, width, height)
        .into_iter()
        .map(|row| row.into_iter().collect())
        .collect()
}

/// The glyph at a frame cell.
fn glyph_at(rows: &[String], (x, y): (usize, usize)) -> Option<char> {
    rows.get(y).and_then(|row| row.chars().nth(x))
}

/// **A Dray-P on a site's pad** (`on_pad_as` for any site): the site opened, the hop, the warm.
fn on_pad_at(
    rig: &mut Rig,
    who: &str,
    seed: i64,
    name: &str,
    venue: &str,
    entry: &str,
) -> (ObjId, HullRow, Vec<ObjId>) {
    let (pilot, row) = prospector(rig, who, seed, name);
    lease_one(rig, pilot);
    load(
        rig,
        &row,
        20,
        20,
        9_500 + u64::try_from(seed).unwrap_or(0) * 10,
    );
    board(rig, pilot);
    let hull = ObjId(row.hull);
    let mut kept = Vec::new();
    let _ = support::sites::open_site_keeping(&mut rig.tw, entry, &mut |ops| kept.push(ops));
    for ops in kept {
        keep(rig, ops);
    }
    let t = file_and_launch(rig, pilot, hull, &format!("+ship/undock {venue}"));
    let _ = to_ic(rig, t.plan.depart_ic + t.plan.total_s);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == entry
        ),
        "the hull is settled on {entry}'s pad"
    );
    warm(rig);
    let tvs = rigs_of(&rig.tw, pilot);
    (pilot, row, tvs)
}

// ---------------------------------------------------------------------------
// 3.1a. The freeze census (the hard gate), and D15
// ---------------------------------------------------------------------------

/// A world booted without the geology key (the census reads no key), its ticks swept once.
fn booted() -> TestWorld {
    let (world, _) = mush_testkit::boot_world();
    let mut tw = boot_over(world, HashMap::new(), false);
    sweep(&mut tw, 120);
    tw
}

/// The lane's mining sites: every port entry with a `site` block, `(entry, name)`, entry order.
fn lane_sites() -> Vec<(CanonicalId, String)> {
    lane()
        .entries()
        .iter()
        .filter_map(|(id, entry)| match entry {
            PackEntry::Port(def) => def
                .site
                .as_ref()
                .map(|site| (id.clone(), site.name.clone())),
            _ => None,
        })
        .collect()
}

/// min / median / max of `values`.
fn spread(mut values: Vec<usize>) -> (usize, usize, usize) {
    values.sort_unstable();
    let min = values.first().copied().unwrap_or(0);
    let max = values.last().copied().unwrap_or(0);
    let median = values.get(values.len() / 2).copied().unwrap_or(0);
    (min, median, max)
}

/// **The freeze census** (the brief's 3.1a and reviewer resolution R1; a hard gate): for every
/// lane mining site — at least the lane's six — from the ramp cell `(128, 124)` (pose `(128_000,
/// 124_500)`) to all 64 field middles `(32fx + 16, 32fy + 16)` over the site's keyless terrain,
/// one `drive_leg` each. Prints per site the accepted and refused counts, the tolerance each
/// accepted leg used, the waypoint counts and the exact paths' cell counts (`--nocapture`).
/// **No site refuses more than 3 of its 64.** The regression guard for the lane's terrain.
#[test]
fn sm4b_the_freeze_census() {
    let tw = booted();
    let sites = lane_sites();
    let names: Vec<&str> = sites.iter().map(|(_, name)| name.as_str()).collect();
    for want in [
        "Freeport Commons",
        "Kerwan Floor",
        "Ahuna Flank",
        "Juling Shadow",
        "Occator Rim",
        "Rheasilvia Commons",
    ] {
        assert!(names.contains(&want), "{want} is a lane site: {names:?}");
    }
    let rates = rates(&tw);
    let mut worst = 0_usize;
    for (entry, name) in &sites {
        let terrain = tacsim::surface::ground::site_terrain(tw.world(), entry)
            .unwrap_or_else(|error| panic!("{name}'s terrain: {error}"));
        let surface = terrain
            .surface(&mush_atlas::SurfaceRef::Ground)
            .expect("a ground surface");
        let mut refused = Vec::new();
        let mut tolerances = [0_usize; 4];
        let mut waypoints = Vec::new();
        let mut cells = Vec::new();
        for fy in 0..8_i64 {
            for fx in 0..8_i64 {
                let to = (fx * 32 + 16, fy * 32 + 16);
                match drive_leg(surface, table(), (RAMP.0, RAMP.1), to, &rates) {
                    Ok(leg) => {
                        if let Some(slot) = LEG_TOLERANCE_CELLS
                            .iter()
                            .position(|t| *t == leg.tolerance_cells)
                            .and_then(|i| tolerances.get_mut(i))
                        {
                            *slot += 1;
                        }
                        waypoints.push(leg.waypoints.len());
                        cells.push(leg.cells.len());
                    }
                    Err(why) => refused.push((to, why)),
                }
            }
        }
        let (wmin, wmed, wmax) = spread(waypoints);
        let (cmin, cmed, cmax) = spread(cells);
        println!(
            "{name}: accepted {}, refused {} {:?}; tolerance 1/2/4/8 cells: {}/{}/{}/{}; \
             waypoints min/median/max {wmin}/{wmed}/{wmax}; path cells {cmin}/{cmed}/{cmax}",
            64 - refused.len(),
            refused.len(),
            refused,
            tolerances[0],
            tolerances[1],
            tolerances[2],
            tolerances[3],
        );
        assert!(
            refused
                .iter()
                .all(|(_, why)| matches!(why, LegRefused::Winding { .. } | LegRefused::NoPath)),
            "{name}: {refused:?}"
        );
        worst = worst.max(refused.len());
        assert!(
            refused.len() <= 3,
            "{name} refuses {} of its 64 field middles: {refused:?}",
            refused.len()
        );
    }
    println!("the worst site refuses {worst} of 64");
}

/// **The least cost from `from` to every cell of `surface`** (cut AP's independent oracle): a
/// plain single-source Dijkstra with no heuristic, over `mush_atlas::physics::move_cost` (tracked,
/// no tags) in the atlas's own neighbour order — the string-keyed step rule, never the dense row
/// cut AP's search reads. Row-major from the south-west; `None` unreachable.
fn least_from_cell(
    surface: &mush_atlas::window::SurfaceComposite,
    from: (u16, u16),
) -> Vec<Option<u64>> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let w = usize::from(surface.grid.width);
    let h = usize::from(surface.grid.height);
    let cell_cm = u32::from(surface.grid.resolution_cm);
    let mut best: Vec<Option<u64>> = vec![None; w * h];
    let mut done = vec![false; w * h];
    let mut open = BinaryHeap::new();
    best[usize::from(from.1) * w + usize::from(from.0)] = Some(0);
    open.push(Reverse((0_u64, from.1, from.0)));
    while let Some(Reverse((g, y, x))) = open.pop() {
        let i = usize::from(y) * w + usize::from(x);
        if done[i] {
            continue;
        }
        done[i] = true;
        let here = surface.cells.get(x, y).expect("on the grid");
        for (dx, dy) in [
            (1, 0),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (1, -1),
            (-1, 1),
            (-1, -1),
        ] {
            let (nx, ny) = (i32::from(x) + dx, i32::from(y) + dy);
            let (Ok(nx), Ok(ny)) = (u16::try_from(nx), u16::try_from(ny)) else {
                continue;
            };
            let Some(next) = surface.cells.get(nx, ny) else {
                continue;
            };
            let kind = if dx != 0 && dy != 0 {
                mush_atlas::StepKind::Diagonal
            } else {
                mush_atlas::StepKind::Orthogonal
            };
            let Ok(step) =
                mush_atlas::move_cost(table(), here, next, kind, cell_cm, DRIVE_LOCOMOTION, &[])
            else {
                continue;
            };
            let ni = usize::from(ny) * w + usize::from(nx);
            let ng = g + u64::from(step);
            if !done[ni] && best[ni].is_none_or(|b| ng < b) {
                best[ni] = Some(ng);
                open.push(Reverse((ng, ny, nx)));
            }
        }
    }
    best
}

/// **Every lane site's 64 ramp legs' Σ cost, recorded at `a0030f71` + SM.7 before cut AP changed
/// `mush_atlas::physics::path`** (the brief's Step 0), keyed by port entry in entry order, over
/// each site's keyless terrain (`tacsim::surface::ground::site_terrain`, the census's composite)
/// from [`RAMP`]. Re-bless only with the lane's terrain (a lane version or a terrain generation
/// change), never for AP, which proves every cost equal.
const AP_SITE_LEG_COST_TOTALS: [(&str, u64); 6] = [
    ("ahuna_port", 1_094_385),
    ("commons_port", 1_039_283),
    ("juling_port", 1_072_064),
    ("kerwan_port", 1_086_989),
    ("occator_port", 1_063_028),
    ("rheasilvia_port", 1_056_596),
];

/// **B1 — every lane leg costs the least, and what it did** (cut AP; the lead's acceptance: "every
/// path cost equal before and after", and the freeze census's 64 of 64 on every lane site). For
/// every lane site, over its keyless terrain from the ramp: a plain forward Dijkstra over
/// `move_cost` ([`least_from_cell`], independent of the atlas's dense `LocomotionCosts`), then
/// all 64 field middles' `drive_leg` — **every one accepted**, each leg's cost the flood's least
/// cost at its target, and the site's Σ cost the total recorded before AP changed `path`
/// ([`AP_SITE_LEG_COST_TOTALS`]). Prints per site the Σ, the tolerance histogram and the waypoint
/// and path-cell spreads, as the census does (`--nocapture`); those are printed, not asserted —
/// a tie may break the other way on an equal cost (the brief's Step 4 freeze rule governs them).
#[test]
fn ap_every_lane_leg_costs_the_least_and_what_it_did() {
    let tw = booted();
    let sites = lane_sites();
    assert_eq!(sites.len(), AP_SITE_LEG_COST_TOTALS.len(), "{sites:?}");
    let rates = rates(&tw);
    let ramp = (
        u16::try_from(RAMP_CELL.0).expect("x"),
        u16::try_from(RAMP_CELL.1).expect("y"),
    );
    for (n, (entry, name)) in sites.iter().enumerate() {
        let terrain = tacsim::surface::ground::site_terrain(tw.world(), entry)
            .unwrap_or_else(|error| panic!("{name}'s terrain: {error}"));
        let surface = terrain
            .surface(&mush_atlas::SurfaceRef::Ground)
            .expect("a ground surface");
        let least = least_from_cell(surface, ramp);
        let width = usize::from(surface.grid.width);
        let mut total = 0_u64;
        let mut tolerances = [0_usize; 4];
        let mut waypoints = Vec::new();
        let mut cells = Vec::new();
        for fy in 0..8_i64 {
            for fx in 0..8_i64 {
                let to = (fx * 32 + 16, fy * 32 + 16);
                let leg = drive_leg(surface, table(), (RAMP.0, RAMP.1), to, &rates)
                    .unwrap_or_else(|why| panic!("{name}: the leg to {to:?} is refused: {why:?}"));
                let at =
                    usize::try_from(to.1).expect("y") * width + usize::try_from(to.0).expect("x");
                assert_eq!(Some(leg.cost), least[at], "{name}: {to:?} costs the least");
                total += leg.cost;
                if let Some(slot) = LEG_TOLERANCE_CELLS
                    .iter()
                    .position(|t| *t == leg.tolerance_cells)
                    .and_then(|i| tolerances.get_mut(i))
                {
                    *slot += 1;
                }
                waypoints.push(leg.waypoints.len());
                cells.push(leg.cells.len());
            }
        }
        let (wmin, wmed, wmax) = spread(waypoints);
        let (cmin, cmed, cmax) = spread(cells);
        println!(
            "{name} ({entry}): accepted 64; total cost {total}; tolerance 1/2/4/8 cells: \
             {}/{}/{}/{}; waypoints min/median/max {wmin}/{wmed}/{wmax}; path cells \
             {cmin}/{cmed}/{cmax}",
            tolerances[0], tolerances[1], tolerances[2], tolerances[3],
        );
        let (want_entry, want_total) = AP_SITE_LEG_COST_TOTALS[n];
        assert_eq!(
            entry.as_str(),
            want_entry,
            "the lane's sites in entry order"
        );
        assert_eq!(
            total, want_total,
            "{name}: the 64 legs' total cost moved from what the old search found"
        );
    }
}

/// **D15 — the lane sites hold only footings and passable craters** (the brief's decisions P5
/// and P9, D17): every lane site's composite is regolith, rock or ice in every cell, and every
/// feature it carries blocks no movement — the drive's A* reads cells alone (the atlas's "a path
/// is geometry"), so the day a site authors a movement-blocking feature this fails.
#[test]
fn sm4b_the_lane_sites_hold_only_footings_and_passable_craters() {
    let tw = booted();
    for (entry, name) in lane_sites() {
        let terrain = tacsim::surface::ground::site_terrain(tw.world(), &entry)
            .unwrap_or_else(|error| panic!("{name}'s terrain: {error}"));
        let surface = terrain
            .surface(&mush_atlas::SurfaceRef::Ground)
            .expect("a ground surface");
        for (x, y, cell) in surface.cells.iter() {
            assert!(
                matches!(
                    cell.class,
                    mush_atlas::TerrainClass::Regolith
                        | mush_atlas::TerrainClass::Rock
                        | mush_atlas::TerrainClass::Ice
                ),
                "{name} ({x}, {y}) is {:?}",
                cell.class
            );
        }
        for feature in &surface.features {
            assert!(
                !feature.blocks.movement,
                "{name}'s {:?} blocks movement",
                feature.key
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 3.2. The home field's cache and its warm
// ---------------------------------------------------------------------------

/// Whether Ahuna Flank's home field for the ramp cell is built (and keyless).
fn home_built(tw: &TestWorld) -> Option<bool> {
    tw.world().module_state::<GeologyState>().and_then(|state| {
        state
            .homes()
            .into_iter()
            .find(|(entry, ramp, _)| entry.as_str() == AHUNA && *ramp == RAMP_CELL)
            .map(|(_, _, keyless)| keyless)
    })
}

/// **A home is warmed after its site and read by pointer** (the brief's 3.2; decision P1): after
/// a restart nothing is cached and the home is wanted; the first fire builds the site's ground,
/// the next the home over it. A drive from the ground answers `GROUND_COLD` until both are warm.
/// Then the body's table is re-installed — the site cache drops the site — and the home built
/// over the old ground reads cold: the drive refuses `GROUND_COLD` again, one fire rebuilds the
/// site, one more the home, and the drive previews.
#[test]
fn sm4b_a_home_is_warmed_after_its_site_and_read_by_pointer() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let mut rig = reboot(rig, &[pilot]);
    assert!(!cached(&rig.tw));
    assert_eq!(home_built(&rig.tw), None);
    assert!(
        rig.tw
            .world()
            .module_state::<GeologyState>()
            .is_some_and(|state| state.homes_wanted().contains(&(cid(AHUNA), RAMP_CELL))),
        "the boot pass wants the docked hull's home"
    );
    let cold = GROUND_COLD.replace("{site}", "Ahuna Flank");
    let (out, queued) = command(&mut rig, pilot, "+tv/1 drive 140,96");
    assert_eq!(out.trim(), cold);
    assert!(queued.is_empty());
    let (_, queued) = fire(&mut rig);
    land_quietly(&mut rig, &queued);
    assert!(cached(&rig.tw), "the site first");
    assert_eq!(home_built(&rig.tw), None, "the home the fire after");
    let (out, _) = command(&mut rig, pilot, "+tv/1 drive 140,96");
    assert_eq!(
        out.trim(),
        cold,
        "a warm site with a cold home is still cold"
    );
    let (_, queued) = fire(&mut rig);
    land_quietly(&mut rig, &queued);
    assert_eq!(
        home_built(&rig.tw),
        Some(false),
        "built over the warm ground"
    );
    let (out, _) = command(&mut rig, pilot, "+tv/1 drive 140,96");
    assert!(out.contains(" would drive "), "{out}");
    // A rebuilt ground: the old field reads cold until it is built again over the new one.
    let ceres = tacsim::surface::body_geology(rig.tw.world(), &cid("ceres")).expect("ceres");
    let table: BodyGeology = (*ceres).clone();
    tacsim::surface::install_body_geology(rig.tw.world_mut(), table).expect("re-installed");
    assert!(!cached(&rig.tw), "the re-install dropped the site");
    let (out, _) = command(&mut rig, pilot, "+tv/1 drive 140,96");
    assert_eq!(out.trim(), cold);
    let (_, queued) = fire(&mut rig);
    land_quietly(&mut rig, &queued);
    assert!(cached(&rig.tw));
    let (out, _) = command(&mut rig, pilot, "+tv/1 drive 140,96");
    assert_eq!(
        out.trim(),
        cold,
        "the home built over the old ground reads cold"
    );
    let (_, queued) = fire(&mut rig);
    land_quietly(&mut rig, &queued);
    let (out, _) = command(&mut rig, pilot, "+tv/1 drive 140,96");
    assert!(out.contains(" would drive "), "{out}");
    // The review's correctness/F3: a drive's arrival wants its site's home again, and that costs
    // no build while the field over the same ground stands — the same field, by pointer.
    let field = home_field(&rig.tw);
    let _ = act_go(&mut rig, pilot, "+tv/1 drive 140,96 go");
    let _ = finish(&mut rig, tv, pilot);
    for _ in 0..2 {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
    }
    assert!(
        Arc::ptr_eq(&field, &home_field(&rig.tw)),
        "the arrival's want did not rebuild the home"
    );
    assert!(
        rig.tw
            .world()
            .module_state::<GeologyState>()
            .is_some_and(|state| state.homes_wanted().is_empty()),
        "nothing left wanted"
    );
    no_refused_writes(&rig.tw);
}

/// **A dark site still warms its way home** (the brief's 3.2; R3): the keyed world's home field
/// is read; then the world restarts without the key — the geology layer dark — and the boot pass
/// still wants the docked hull's home; one fire builds it from the keyless terrain, and on 64
/// sampled cells its cost home equals the keyed world's, cell for cell.
#[test]
fn sm4b_a_dark_site_still_warms_its_way_home() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    launched(&mut rig, pilot, tvs[0]);
    let keyed = home_field(&rig.tw);
    let mut rig = reboot_with(rig, &[pilot], false);
    assert!(!cached(&rig.tw));
    assert_eq!(home_built(&rig.tw), None);
    let (_, queued) = fire(&mut rig);
    land_quietly(&mut rig, &queued);
    assert!(!cached(&rig.tw), "a dark layer builds no site ground");
    assert_eq!(home_built(&rig.tw), Some(true), "the home, keyless");
    let dark = home_field(&rig.tw);
    for n in 0..64_i64 {
        let cell = ((n * 37 + 11) % 256, (n * 91 + 5) % 256);
        assert_eq!(dark.cost_at(cell), keyed.cost_at(cell), "{cell:?}");
    }
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// D1–D3. The drive
// ---------------------------------------------------------------------------

/// A pocket two fields north of the ramp's (`(36, 44)`), its address and its address cell.
fn north_pocket() -> (String, (i64, i64), (i64, i64)) {
    let pocket = (36, 44);
    let addr = pocket_address(pocket.0, pocket.1).expect("an address");
    (addr, pocket, (pocket.0 * 4 + 2, pocket.1 * 4 + 2))
}

/// **D1 — a drive is one start, a projected leg and an arrival** (§8; lead rulings 1–3): from
/// the ramp to a pocket two fields north, the preview names the metres, the IC seconds and the
/// charge words — after the drive and after the trip home — and commits nothing; `go` queues
/// exactly one start with `act: drive`; landed, the act carries the leg (the planner's, from the
/// ramp pose, ≤ 32 waypoints) and is due `leg.secs` on; the next fire writes the projection's
/// leg; at the due instant the rig stands at the end cell's centre, the finish lands `Done` with
/// `arrived`, the charge fell by exactly the leg's cu and the operator hears exactly `ARRIVED`.
/// After a sounding of that pocket, a drive back to it names the reading in brackets.
#[test]
#[allow(clippy::too_many_lines)]
fn sm4b_a_drive_is_one_start_a_projected_leg_and_an_arrival() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let (addr, pocket, to) = north_pocket();
    let leg = expect_leg(&mut rig, (RAMP.0, RAMP.1), to);
    let rates = rates(&rig.tw);
    let home = home_cu_to(&home_field(&rig.tw), to, &rates).expect("a way home");
    let before = value_of(&mut rig.tw, tv);
    // The preview.
    let (out, queued) = command(&mut rig, pilot, &format!("+tv/1 drive {addr}"));
    assert!(queued.is_empty(), "a preview queues nothing");
    let head = format!(
        "Rig 1 (OWP-TV-01) would drive {} m to {addr}: {} IC s",
        leg.metres, leg.secs
    );
    let tail = format!(
        "; charge {} → {} %, {} % after the trip home. +tv/1 drive {addr} go sends it.",
        pct(before),
        pct(before - leg.cu),
        pct(before - leg.cu - home)
    );
    assert!(
        out.trim().starts_with(&head) && out.trim().ends_with(&tail),
        "{out}\n{head}\n{tail}"
    );
    assert!(leg.waypoints.len() <= 32);
    // The go.
    let out = act_go(&mut rig, pilot, &format!("+tv/1 drive {addr} go"));
    assert!(
        out.contains(&format!("Rig 1 drives for {addr}: there ")),
        "{out}"
    );
    let act = act_of(&mut rig.tw, tv);
    assert_eq!(act.kind, ActKind::Drive);
    assert_eq!(act.due_ic - act.started_ic, i64::from(leg.secs));
    assert_eq!(act.reserved_cu, leg.cu);
    match &act.params {
        ActParams::Drive {
            from_bay,
            target,
            to: end,
            waypoints,
            secs,
            cost,
            ..
        } => {
            assert!(!from_bay);
            assert_eq!(target, &addr);
            assert_eq!(*end, [to.0, to.1]);
            assert_eq!(waypoints, &leg.waypoints);
            assert_eq!(*secs, leg.secs);
            assert_eq!(*cost, leg.cost);
        }
        other => panic!("{other:?}"),
    }
    // The next fire projects the leg for interpolation.
    let (_, queued) = fire(&mut rig);
    land_quietly(&mut rig, &queued);
    let projected = projection(&rig.tw, tv)
        .and_then(|rec| rec.leg)
        .expect("the projected leg");
    assert_eq!(projected.started_ic, act.started_ic);
    assert_eq!(projected.waypoints, leg.waypoints);
    let first = projected.waypoints.first().expect("a first waypoint");
    assert_eq!((first.x_cm, first.y_cm, first.at_s), (RAMP.0, RAMP.1, 0));
    let last = projected.waypoints.last().expect("a last waypoint");
    assert_eq!(
        (last.x_cm, last.y_cm, last.at_s),
        (to.0 * 1_000 + 500, to.1 * 1_000 + 500, leg.secs)
    );
    // Mid-leg (the review's tests/T5; decision P8, lead ruling 6): the map draws the rig at its
    // leg's integer interpolation at the frame's instant — never at its start — and the frame
    // carries the act and the leg; the print's rig line names the drive.
    let (_, queued) = advance_to(&mut rig, act.started_ic + i64::from(leg.secs) / 2);
    land_quietly(&mut rig, &queued);
    let now = ic_now(&rig.tw);
    assert!(now < act.due_ic, "still under way");
    let (x, y) = tacsim::port::site_map::leg_position(&leg.waypoints, now - act.started_ic)
        .expect("a position along the leg");
    let moving = (x.div_euclid(1_000), y.div_euclid(1_000));
    assert_ne!(moving, RAMP_CELL, "the rig has left its start");
    let frame = map_frame(&mut rig, pilot, json!({ "view": "drive", "rig": 1 }));
    let x0 = frame["origin_cell"][0].as_i64().expect("x0");
    let y0 = frame["origin_cell"][1].as_i64().expect("y0");
    assert_eq!(frame["you"], json!([moving.0 - x0, moving.1 - y0]));
    let rows = frame_rows(&frame);
    assert_eq!(
        glyph_at(
            &rows,
            (
                usize::try_from(moving.0 - x0).expect("x"),
                usize::try_from(moving.1 - y0).expect("y")
            )
        ),
        Some('@')
    );
    let shown = &frame["site"]["rigs"][0];
    assert_eq!(
        shown["act"],
        json!({ "kind": "drive", "due_ic": act.due_ic })
    );
    assert_eq!(shown["leg"]["started_ic"], json!(act.started_ic));
    assert_eq!(
        shown["leg"]["waypoints"],
        serde_json::to_value(&leg.waypoints).expect("encodes")
    );
    let out = run(&mut rig.tw, pilot, "+tv/1/map");
    assert!(
        out.contains(&format!("Rig 1: ({}, {}), charge ", moving.0, moving.1))
            && out.contains(&format!("; driving to {addr}, there ")),
        "{out}"
    );
    // The arrival.
    let heard = finish(&mut rig, tv, pilot);
    assert!(on_ground_at(
        &rig.tw,
        tv,
        (to.0 * 1_000 + 500, to.1 * 1_000 + 500, leg.heading_mdeg)
    ));
    let record = record_of(&mut rig.tw, tv).expect("the record");
    let last = record.last.expect("the outcome");
    assert_eq!((last.kind, last.result), (ActKind::Drive, ActResult::Done));
    assert_eq!(last.arrived, Some([to.0, to.1]));
    assert_eq!(record.charge.value_cu, before - leg.cu);
    assert!(!record.charge.in_bay);
    assert!(projection(&rig.tw, tv).is_some_and(|rec| rec.leg.is_none()));
    let arrived = ARRIVED
        .replace("{n}", "1")
        .replace("{addr}", &addr)
        .replace("{reading}", "");
    assert_eq!(count(&heard, &arrived), 1, "{heard}");
    // A sounding of that pocket; away one pocket and back: the reading in brackets.
    let _ = act_go(&mut rig, pilot, "+tv/1 sound go");
    let _ = finish(&mut rig, tv, pilot);
    let _ = act_go(
        &mut rig,
        pilot,
        &format!("+tv/1 drive {},{} go", to.0 + 4, to.1),
    );
    let _ = finish(&mut rig, tv, pilot);
    let _ = act_go(&mut rig, pilot, &format!("+tv/1 drive {addr} go"));
    let heard = finish(&mut rig, tv, pilot);
    let ground = ahuna(&mut rig.tw);
    let text = pocket_row_text(&addr, &ground.pocket(pocket));
    let words = text
        .strip_prefix(&format!("{addr} "))
        .expect("the pocket's words");
    let arrived = ARRIVED
        .replace("{n}", "1")
        .replace("{addr}", &addr)
        .replace("{reading}", &format!(" ({words})"));
    assert_eq!(count(&heard, &arrived), 1, "{heard}");
    no_refused_writes(&rig.tw);
}

/// **D2 — every target kind lands where it says** (§10): from the ramp, `C117`, `c-nw`, `C`
/// and `140,96` each preview exactly the planner's leg to the pocket's address cell, the field's
/// middle, the block's middle and the cell; from a staff-placed stand `ne 300` the leg 21 cells
/// up each axis. A malformed target answers `DRIVE_USAGE`, a cell off the site
/// `DRIVE_OFF_SITE`, the rig's own cell `DRIVE_HERE`; none queues anything.
#[test]
fn sm4b_every_target_kind_lands_where_it_says() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let c117 = pocket_of_block_index('C', 116).expect("C117");
    for (typed, words, to) in [
        ("C117", "C117", (c117.0 * 4 + 2, c117.1 * 4 + 2)),
        ("c-nw", "field C-NW", (144, 240)),
        ("C", "block C", (160, 224)),
        ("140,96", "cell 140,96", (140, 96)),
    ] {
        let leg = expect_leg(&mut rig, (RAMP.0, RAMP.1), to);
        let (out, queued) = command(&mut rig, pilot, &format!("+tv/1 drive {typed}"));
        assert!(queued.is_empty());
        assert!(
            out.contains(&format!(
                " would drive {} m to {words}: {} IC s",
                leg.metres, leg.secs
            )) && out.contains(&format!("+tv/1 drive {typed} go sends it.")),
            "{typed}: {out}"
        );
    }
    to_cell(&mut rig, tv, (100, 100));
    let leg = expect_leg(&mut rig, centre((100, 100)), (121, 121));
    let (out, queued) = command(&mut rig, pilot, "+tv/1 drive ne 300");
    assert!(queued.is_empty());
    assert!(
        out.contains(&format!(
            " would drive {} m to 300 m NE: {} IC s",
            leg.metres, leg.secs
        )),
        "{out}"
    );
    for (typed, line) in [
        ("xyz", DRIVE_USAGE.replace("{n}", "1")),
        ("", DRIVE_USAGE.replace("{n}", "1")),
        ("300,10", DRIVE_OFF_SITE.replace("{site}", "Ahuna Flank")),
        ("s 3000", DRIVE_OFF_SITE.replace("{site}", "Ahuna Flank")),
        ("100,100", DRIVE_HERE.replace("{n}", "1")),
    ] {
        for go in ["", " go"] {
            let (out, queued) = command(&mut rig, pilot, &format!("+tv/1 drive {typed}{go}"));
            assert_eq!(out.trim(), line, "{typed}{go}");
            assert!(queued.is_empty(), "{typed}{go}");
        }
    }
    no_refused_writes(&rig.tw);
}

/// **D3 — a drive costs its cells in the world** (V2's world half; lead ruling 3): the drive's
/// `due − started` is `secs_of_cost` of the least path cost the test recomputes with the atlas's
/// own `path` over the cached composite, and the settled charge falls by exactly `cu_of_cost` of
/// it.
#[test]
fn sm4b_a_kilometre_of_regolith_costs_its_cells_in_the_world() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let rates = rates(&rig.tw);
    let cost = {
        let ground = ahuna(&mut rig.tw);
        match mush_atlas::physics::path(
            ground.surface(),
            table(),
            (128, 124),
            (140, 96),
            DRIVE_LOCOMOTION,
            &[],
            DRIVE_PATH_BUDGET_CELLS,
        ) {
            mush_atlas::physics::PathOutcome::Found(path) => u64::from(path.cost),
            other => panic!("{other:?}"),
        }
    };
    let before = value_of(&mut rig.tw, tv);
    let _ = act_go(&mut rig, pilot, "+tv/1 drive 140,96 go");
    let act = act_of(&mut rig.tw, tv);
    assert_eq!(
        Some(u32::try_from(act.due_ic - act.started_ic).expect("seconds")),
        secs_of_cost(cost, &rates)
    );
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(
        Some(value_of(&mut rig.tw, tv)),
        cu_of_cost(cost, &rates).map(|cu| before - cu)
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// D4. The reserve with the real trip home (V3)
// ---------------------------------------------------------------------------

/// **D4 — an act that would strand the rig is refused** (V3; lead ruling 4): a drive from the
/// ramp to the far corner field's middle whose own cost fits but whose cost and trip home leave
/// one cu under the floor refuses `RESERVE_LOW` — figures pinned — and queues nothing, preview or
/// `go`; one more cu and it starts. From that far stand a scan at the same margin (its 1,000 cu
/// and the trip home from where it stands) refuses likewise, and at exactly the floor it runs.
/// (`NO_WAY_HOME` has no world case on today's lane — every slope is under the tracked limit — so
/// it is pinned by `drive.rs`'s `sm4b_no_way_home_is_none_and_the_bound_never_understates` and
/// the refusal line's unit test.)
#[test]
fn sm4b_an_act_that_would_strand_the_rig_is_refused() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let to = (16, 16);
    let leg = expect_leg(&mut rig, (RAMP.0, RAMP.1), to);
    let rates = rates(&rig.tw);
    let home = home_cu_to(&home_field(&rig.tw), to, &rates).expect("a way home");
    assert!(home > 0);
    let short = floor() - 1 + leg.cu + home;
    put_charge(&mut rig, tv, short, false);
    let refusal = RESERVE_LOW
        .replace("{n}", "1")
        .replace("{now}", &pct(short).to_string())
        .replace("{after}", &pct(floor() - 1).to_string())
        .replace("{floor}", "5");
    for go in ["", " go"] {
        let (out, queued) = command(&mut rig, pilot, &format!("+tv/1 drive 16,16{go}"));
        assert_eq!(out.trim(), refusal, "{go}");
        assert!(queued.is_empty(), "{go}");
    }
    put_charge(&mut rig, tv, short + 1, false);
    let _ = act_go(&mut rig, pilot, "+tv/1 drive 16,16 go");
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(value_of(&mut rig.tw, tv), floor() + home);
    // The scan from the far stand carries the real trip home too.
    let scan_cu = tools_table()
        .expect("the tools table")
        .values()
        .find_map(ToolDef::scanner)
        .map(|scanner| scanner.scan_cu)
        .expect("a scanner row");
    put_charge(&mut rig, tv, floor() - 1 + scan_cu + home, false);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 scan");
    assert!(
        out.trim().ends_with("under the 5 % reserve."),
        "the scan is held to the trip home: {out}"
    );
    assert!(queued.is_empty());
    put_charge(&mut rig, tv, floor() + scan_cu + home, false);
    let _ = act_go(&mut rig, pilot, "+tv/1 scan go");
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(value_of(&mut rig.tw, tv), floor() + home);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// D5. The recover drives home
// ---------------------------------------------------------------------------

/// The recover's preview head for a leg home.
fn recover_head(leg: &Leg) -> String {
    format!(
        "Rig 1 (OWP-TV-01) would drive {} m to the stern ramp and climb into the Salt Moth's \
         bay: {} IC s, then 300 IC s",
        leg.metres, leg.secs
    )
}

/// **D5 — a recover from afar drives home and climbs aboard** (lead ruling 5; decision P6; R3):
/// from a far stand the preview is `RECOVER_DRIVE_PREVIEW`; `go` queues one start whose act is
/// a `Recover` carrying the leg home, due `leg.secs + 300` on; at the due instant the rig is in
/// the bay, `Done`, `RECOVERED` told, the excursion closed and the charge the value less the
/// leg's cu, in the bay. Short of the leg's charge it refuses `RECOVER_SHORT`; between the leg
/// and the leg plus the floor it recovers (the floor is waived for the way home). With the layer
/// dark after a keyless restart a drive and a scan answer `INSTRUMENTS_DARK`, but the recover —
/// cold until one fire builds the keyless home — previews, starts and lands with `RECOVERED`, its
/// leg costing what the keyed world's would. With the battery rows taken out it still previews
/// (`…_UNREAD`), starts and lands.
#[test]
#[allow(clippy::too_many_lines)]
fn sm4b_recover_from_afar_drives_home_and_climbs_aboard() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let rates = rates(&rig.tw);
    let far = (60, 200);
    // (a) The recover with its leg home.
    launched(&mut rig, pilot, tv);
    to_cell(&mut rig, tv, far);
    let leg = recover_leg(&home_field(&rig.tw), centre(far), &rates).expect("a way home");
    let before = value_of(&mut rig.tw, tv);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 recover");
    assert!(queued.is_empty());
    assert!(
        out.trim().starts_with(&recover_head(&leg))
            && out.trim().ends_with(&format!(
                "; charge {} → {} %; hopper empty. +tv/1 recover go brings it in.",
                pct(before),
                pct(before - leg.cu)
            )),
        "{out}"
    );
    let out = act_go(&mut rig, pilot, "+tv/1 recover go");
    assert!(
        out.contains("Rig 1 drives to the stern ramp: in the bay "),
        "{out}"
    );
    let act = act_of(&mut rig.tw, tv);
    assert_eq!(act.kind, ActKind::Recover);
    assert_eq!(act.due_ic - act.started_ic, i64::from(leg.secs) + 300);
    assert!(matches!(
        &act.params,
        ActParams::Drive { from_bay: false, target, .. } if target == "the stern ramp"
    ));
    let heard = finish(&mut rig, tv, pilot);
    assert!(matches!(
        parent(&rig.tw, tv),
        SpatialParent::Interior { .. }
    ));
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(
        record.last.as_ref().map(|l| (l.kind, l.result)),
        Some((ActKind::Recover, ActResult::Done))
    );
    assert!(record.excursion.is_none());
    assert_eq!(
        record.charge,
        Charge {
            value_cu: before - leg.cu,
            at_ic: act.due_ic,
            in_bay: true
        }
    );
    let recovered = RECOVERED.replace("{n}", "1").replace("{name}", "Salt Moth");
    assert_eq!(count(&heard, &recovered), 1, "{heard}");
    // (b) Short of the leg: refused, whole percent rounded up for the need.
    launched(&mut rig, pilot, tv);
    to_cell(&mut rig, tv, far);
    put_charge(&mut rig, tv, leg.cu - 1, false);
    let need = (u64::from(leg.cu) * 100).div_ceil(u64::from(battery().capacity_cu));
    let (out, queued) = command(&mut rig, pilot, "+tv/1 recover");
    assert_eq!(
        out.trim(),
        RECOVER_SHORT
            .replace("{n}", "1")
            .replace("{now}", &pct(leg.cu - 1).to_string())
            .replace("{need}", &need.to_string())
            .replace("{ramp}", "stern ramp")
    );
    assert!(queued.is_empty());
    // (c) Between the leg and the leg plus the floor: it comes home.
    put_charge(&mut rig, tv, leg.cu + floor() - 1, false);
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(value_of(&mut rig.tw, tv), floor() - 1);
    // (e) Dark: the keyless way home.
    put_charge(&mut rig, tv, 100_000, true);
    launched(&mut rig, pilot, tv);
    let keyed_cu = leg.cu;
    let mut rig = reboot_with(rig, &[pilot], false);
    // A staff move is placement only (never folded into the store): the rig is put far out
    // after the restart, as the tick-batch seam a test may call.
    to_cell(&mut rig, tv, far);
    let dark = INSTRUMENTS_DARK.to_owned();
    for line in ["+tv/1 drive 140,96", "+tv/1 scan"] {
        let (out, queued) = command(&mut rig, pilot, line);
        assert_eq!(out.trim(), dark, "{line}");
        assert!(queued.is_empty());
    }
    let (out, _) = command(&mut rig, pilot, "+tv/1 recover");
    assert_eq!(out.trim(), GROUND_COLD.replace("{site}", "Ahuna Flank"));
    let (_, queued) = fire(&mut rig);
    land_quietly(&mut rig, &queued);
    let (out, _) = command(&mut rig, pilot, "+tv/1 recover");
    assert!(out.trim().starts_with(&recover_head(&leg)), "{out}");
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    assert_eq!(act_of(&mut rig.tw, tv).reserved_cu, keyed_cu);
    let heard = finish(&mut rig, tv, pilot);
    assert_eq!(count(&heard, &recovered), 1, "{heard}");
    assert!(matches!(
        parent(&rig.tw, tv),
        SpatialParent::Interior { .. }
    ));
    // (d) The battery rows gone: unread, and still home.
    launched(&mut rig, pilot, tv);
    to_cell(&mut rig, tv, far);
    let before = value_of(&mut rig.tw, tv);
    unbattery(&mut rig);
    let (out, _) = command(&mut rig, pilot, "+tv/1 recover");
    assert!(
        out.trim().starts_with(&recover_head(&leg)) && out.contains("its charge cannot be read"),
        "{out}"
    );
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let heard = finish(&mut rig, tv, pilot);
    assert_eq!(count(&heard, &recovered), 1, "{heard}");
    assert_eq!(
        value_of(&mut rig.tw, tv),
        before,
        "no battery row, no figure to spend"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// D6, D7. The implied launch and the restart
// ---------------------------------------------------------------------------

/// The charge a from-bay drive leaves, or a launch and then the same drive from the ramp.
fn implied_or_twin(implied: bool) -> (u32, Charge) {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let c0 = put_charge(&mut rig, tv, 20_000, true);
    let (addr, _, to) = north_pocket();
    let leg = expect_leg(&mut rig, (RAMP.0, RAMP.1), to);
    if implied {
        let (out, queued) = command(&mut rig, pilot, &format!("+tv/1 drive {addr}"));
        assert!(queued.is_empty());
        let launch = IMPLIED_LAUNCH
            .replace("{ramp}", "stern ramp")
            .replace("{site}", "Ahuna Flank");
        let legs = TWO_LEGS
            .replace("{down}", "300")
            .replace("{secs}", &leg.secs.to_string());
        assert!(
            out.contains(&format!(
                " would {launch}drive {} m to {addr}: {legs}",
                leg.metres
            )),
            "{out}"
        );
        let out = act_go(&mut rig, pilot, &format!("+tv/1 drive {addr} go"));
        assert!(
            out.contains(&format!(
                "Rig 1 starts down the stern ramp and drives for {addr}: there "
            )),
            "{out}"
        );
        let record = record_of(&mut rig.tw, tv).expect("the record");
        let act = record.act.clone().expect("the act");
        assert_eq!(
            record.excursion.map(|e| e.id.launch_ic),
            Some(act.started_ic),
            "the excursion opens at the start"
        );
        assert_eq!(act.due_ic, act.started_ic + 300 + i64::from(leg.secs));
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
        assert_eq!(
            projection(&rig.tw, tv)
                .and_then(|r| r.leg)
                .map(|l| l.started_ic),
            Some(act.started_ic + 300),
            "the leg starts when the launch half ends"
        );
        let _ = finish(&mut rig, tv, pilot);
        let charge = record_of(&mut rig.tw, tv).expect("the record").charge;
        assert_eq!(
            charge.value_cu,
            charge_at(&c0, act.started_ic + 300, &battery()) - leg.cu
        );
        assert!(!charge.in_bay);
        assert!(on_ground_at(
            &rig.tw,
            tv,
            (to.0 * 1_000 + 500, to.1 * 1_000 + 500, leg.heading_mdeg)
        ));
        (leg.cu, charge)
    } else {
        launched(&mut rig, pilot, tv);
        let _ = act_go(&mut rig, pilot, &format!("+tv/1 drive {addr} go"));
        let _ = finish(&mut rig, tv, pilot);
        (
            leg.cu,
            record_of(&mut rig.tw, tv).expect("the record").charge,
        )
    }
}

/// **D6 — a drive from the bay implies the launch** (lead ruling 1; D22's rule): one start, the
/// excursion opened at the start, due `300 + leg.secs` on, the leg from the ramp pose and
/// projected from the end of the launch half; the charge `charge_at(c0, start + 300) − cu` out
/// of the bay (the bay credits the launch half and no further) — equal to a twin world's
/// `+tv/launch` then the same drive from the ramp.
#[test]
fn sm4b_a_drive_from_the_bay_implies_the_launch() {
    let (cu, implied) = implied_or_twin(true);
    let (twin_cu, twin) = implied_or_twin(false);
    assert_eq!(cu, twin_cu);
    assert_eq!(
        implied.value_cu, twin.value_cu,
        "the bay credits the launch half alike"
    );
    assert_eq!(implied.in_bay, twin.in_bay);
}

/// One world through a drive and a far recover, restarted with each in flight or not: the rig
/// record, the projection and the placement at the end.
fn drive_and_recover(restart: bool) -> (RigRecord, TvRec, SpatialParent) {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let (addr, _, _) = north_pocket();
    for line in [
        format!("+tv/1 drive {addr} go"),
        "+tv/1 recover go".to_owned(),
    ] {
        let _ = act_go(&mut rig, pilot, &line);
        let due = due_of(&mut rig.tw, tv);
        if restart {
            rig = reboot(rig, &[pilot]);
            assert_eq!(due_of(&mut rig.tw, tv), due, "the act survives the restart");
        }
        let (_, queued) = advance_to(&mut rig, due + 40);
        assert_eq!(finishes(&queued).len(), 1);
        land_all(&mut rig, &queued);
        for _ in 0..2 {
            let (_, queued) = fire(&mut rig);
            land_all(&mut rig, &queued);
        }
    }
    (
        record_of(&mut rig.tw, tv).expect("the record"),
        projection(&rig.tw, tv).expect("the projection"),
        parent(&rig.tw, tv),
    )
}

/// **D7 — a restart mid-drive equals an uninterrupted run** (§8's restart acceptance; lead ruling
/// 11): two worlds drive to a pocket and recover from it; one restarts with each act in flight.
/// The drive lands at its own due instant and the far recover with its leg likewise: the record,
/// the projection and the placement are equal.
#[test]
fn sm4b_a_restart_mid_drive_equals_an_uninterrupted_run() {
    let straight = drive_and_recover(false);
    let restarted = drive_and_recover(true);
    assert_eq!(restarted.0, straight.0, "the record");
    assert_eq!(restarted.1, straight.1, "the projection");
    assert_eq!(restarted.2, straight.2, "the placement");
    assert!(matches!(straight.2, SpatialParent::Interior { .. }));
}

// ---------------------------------------------------------------------------
// D8. Rights: no licence, a held block (V1)
// ---------------------------------------------------------------------------

/// **D8 — a rig without a licence drives and recovers on a held block** (V1; lead ruling 9): on
/// Occator Rim, whose inner blocks F, G, J and K are held by Occator Water & Propellant, a pilot
/// whose licence has lapsed (`licence_gate` errs, asserted) launches, drives to a pocket in block
/// F and recovers from there; both are `Done`. No licence and no tenure is read by a movement act.
#[test]
fn sm4b_no_licence_drives_and_recovers_on_a_held_block() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad_at(&mut rig, "Pilot", 1, "Salt Moth", "occator", "occator_port");
    let tv = tvs[0];
    let key = {
        let account = rig
            .oracle
            .0
            .lock()
            .ok()
            .and_then(|map| map.get(&pilot).copied())
            .expect("an account");
        tacsim::port::licence::licence_key(account)
    };
    let mut rig = reboot_editing(rig, key.as_str(), &[pilot], |doc| {
        let first = doc["first_bought_ic"].as_i64().expect("a first instant");
        doc["expires_ic"] = json!(first + 1);
        doc["grace_secs"] = json!(0);
    });
    warm(&mut rig);
    let now = ic_now(&rig.tw);
    assert!(tacsim::port::licence::licence_gate(rig.tw.world(), pilot, now).is_err());
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    let f120 = pocket_of_block_index('F', 119).expect("F120");
    let addr = pocket_address(f120.0, f120.1).expect("an address");
    assert!(addr.starts_with('F'), "{addr}: a held block");
    let _ = act_go(&mut rig, pilot, &format!("+tv/1 drive {addr} go"));
    let _ = finish(&mut rig, tv, pilot);
    let last = record_of(&mut rig.tw, tv).and_then(|r| r.last);
    assert_eq!(
        last.map(|l| (l.kind, l.result)),
        Some((ActKind::Drive, ActResult::Done))
    );
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let _ = finish(&mut rig, tv, pilot);
    let last = record_of(&mut rig.tw, tv).and_then(|r| r.last);
    assert_eq!(
        last.map(|l| (l.kind, l.result)),
        Some((ActKind::Recover, ActResult::Done))
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// D9, D10. A forged or stale start; cold ground
// ---------------------------------------------------------------------------

/// **D9 — a forged or stale drive start is refused** (the brief's decision P2): a raw start with
/// `act: drive` and no order refuses "that act was not planned; preview it again" (doc 20 cut
/// SM.7's rewording of the shared word); an order
/// planned for one ordinal, overtaken by another act, cannot start at the next ("that rig's
/// orders moved on"); a rig staff-moved after its drive's `go` and before the start lands refuses
/// "that rig has moved since the drive was planned". None writes the record.
#[test]
fn sm4b_a_forged_or_stale_drive_start_is_refused() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let n = record_of(&mut rig.tw, tv).expect("the record").ordinal;
    let forged = start(&rig, pilot, tv, "drive", n + 1);
    let refused = land(&mut rig, &forged).expect_err("no order");
    assert!(
        refused.contains("that act was not planned; preview it again"),
        "{refused}"
    );
    // An order for ordinal n + 1, overtaken by a scan at n + 1.
    let (_, queued) = command(&mut rig, pilot, "+tv/1 drive 140,96 go");
    assert_eq!(queued.len(), 1);
    let scan = start(&rig, pilot, tv, "scan", n + 1);
    land(&mut rig, &scan).expect("the scan lands");
    let _ = finish(&mut rig, tv, pilot);
    let before = record_of(&mut rig.tw, tv);
    let refused = land(&mut rig, &queued[0]).expect_err("overtaken");
    assert!(refused.contains("that rig's orders moved on"), "{refused}");
    let stale = start(&rig, pilot, tv, "drive", n + 2);
    let refused = land(&mut rig, &stale).expect_err("the order's ordinal");
    assert!(refused.contains("that rig's orders moved on"), "{refused}");
    assert_eq!(record_of(&mut rig.tw, tv), before);
    // A rig moved after the go.
    let (_, queued) = command(&mut rig, pilot, "+tv/1 drive 140,96 go");
    assert_eq!(queued.len(), 1);
    to_cell(&mut rig, tv, (130, 130));
    let refused = land(&mut rig, &queued[0]).expect_err("moved");
    assert!(
        refused.contains("that rig has moved since the drive was planned"),
        "{refused}"
    );
    assert_eq!(record_of(&mut rig.tw, tv), before);
    // The review's correctness/F5: two `go`s before either lands. The second order replaces the
    // first, so the first's start — which names the first's end cell — refuses and changes
    // nothing; the second's runs the second's leg. The world never runs a drive it did not tell.
    let (_, first) = command(&mut rig, pilot, "+tv/1 drive 144,112 go");
    let (_, second) = command(&mut rig, pilot, "+tv/1 drive 112,112 go");
    assert_eq!((first.len(), second.len()), (1, 1));
    assert_eq!(first[0].parameters["to"], json!(112 * 256 + 144));
    assert_eq!(second[0].parameters["to"], json!(112 * 256 + 112));
    let refused = land(&mut rig, &first[0]).expect_err("replaced");
    assert!(refused.contains("that rig's orders moved on"), "{refused}");
    assert_eq!(record_of(&mut rig.tw, tv), before);
    land(&mut rig, &second[0]).expect("the second order starts");
    assert!(matches!(
        act_of(&mut rig.tw, tv).params,
        ActParams::Drive { to: [112, 112], .. }
    ));
    no_refused_writes(&rig.tw);
}

/// **D10 — a drive refuses cold ground and warms it** (lead ruling 6's rule; R4): after a
/// restart the site is cold — the drive answers `GROUND_COLD` and queues nothing; one fire warms
/// the site, and the home field is still cold — `GROUND_COLD` again; the next fire warms the
/// home, and the drive previews.
#[test]
fn sm4b_a_drive_refuses_cold_ground_and_warms_it() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    launched(&mut rig, pilot, tvs[0]);
    let mut rig = reboot(rig, &[pilot]);
    let cold = GROUND_COLD.replace("{site}", "Ahuna Flank");
    for fires in 0..2 {
        let (out, queued) = command(&mut rig, pilot, "+tv/1 drive 140,96");
        assert_eq!(out.trim(), cold, "after {fires} fires");
        assert!(queued.is_empty());
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
    }
    let (out, queued) = command(&mut rig, pilot, "+tv/1 drive 140,96");
    assert!(out.contains(" would drive "), "{out}");
    assert!(queued.is_empty());
    no_refused_writes(&rig.tw);
}

/// Staff place `tv` back `Interior` in `row`'s bay compartment (`excursions.rs`'s).
fn staff_bay(rig: &mut Rig, tv: ObjId, row: &HullRow) {
    let bay = ObjId(row.rooms[&cid("bay")]);
    let slot = mush_sim::SlotId::new("rigs").expect("a slot");
    let placed = support::with_ctx(&mut rig.tw, |ctx| {
        mush_sim::place_interior(ctx, tv, bay, Some(slot))
    });
    placed.expect("staff place the rig in the bay");
}

/// **The cheap gates answer before the search** (R4; the review's tests/T4): on a cold site —
/// right after a restart, nothing warmed, where a search would first answer `GROUND_COLD` — a rig
/// back in the bay with its excursion still open answers `LAUNCH_OPEN`, the battery rows gone
/// answer `TV_DARK`, and a rig mid-drive answers `RIG_BUSY` — never `GROUND_COLD`, so none of
/// them reached the peeks or paid the search; none queues anything.
#[test]
fn sm4b_the_cheap_gates_answer_before_the_search() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let mut rig = reboot(rig, &[pilot]);
    assert!(!cached(&rig.tw), "the site is cold");
    // Back in the bay with the excursion open: the implied launch's half refuses.
    staff_bay(&mut rig, tv, &row);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 drive 144,112");
    assert_eq!(out.trim(), LAUNCH_OPEN.replace("{n}", "1"));
    assert!(queued.is_empty());
    // The battery rows gone: dark, before the record and the search.
    unbattery(&mut rig);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 drive 144,112");
    assert_eq!(out.trim(), TV_DARK.replace("{name}", "Salt Moth"));
    assert!(queued.is_empty());
    // Busy: a drive in flight, then a restart that leaves the site cold.
    let mut rig = reboot(rig, &[pilot]);
    staff_place(&mut rig, tv, RAMP.0, RAMP.1);
    warm(&mut rig);
    let _ = act_go(&mut rig, pilot, "+tv/1 drive 144,112 go");
    let mut rig = reboot(rig, &[pilot]);
    assert!(!cached(&rig.tw), "the site is cold again");
    let (out, queued) = command(&mut rig, pilot, "+tv/1 drive 112,112");
    assert!(
        out.trim()
            .starts_with("Rig 1 is driving to cell 144,112: done "),
        "{out}"
    );
    assert!(queued.is_empty());
    no_refused_writes(&rig.tw);
}

/// **The map shows only its own site's rigs** (the review's correctness/F1): B docked on Ahuna
/// Flank, C on Occator Rim. C's rig driving on Occator — its leg in Occator's cells — is never
/// drawn on B's Ahuna map: B's frames mid-drive are byte-identical to before it. Then C's rig,
/// fresh from a loud sounding on Occator, is staff-placed onto Ahuna's ground: B sees it as an
/// `o` with no sounding mark (the sounding was another site's), and C — none of whose rigs is on
/// Occator's ground or in the bay now — is refused with `MAP_NO_RIGS`, naming no slot.
#[test]
#[allow(clippy::too_many_lines)]
fn sm4b_the_map_shows_only_its_own_sites_rigs() {
    let mut rig = stocked();
    let (c, _, c_tvs) = on_pad_at(&mut rig, "Pilot", 1, "Salt Moth", "occator", "occator_port");
    let c_tv = c_tvs[0];
    let (b, _, b_tvs) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    // B's one rig in the bay at full charge, so B's own `rigs` block cannot drift (D12's rule).
    put_charge(&mut rig, b_tvs[0], 100_000, true);
    // Both sites warm, their homes built (`warm` stops at the first site it finds).
    for entry in ["occator_port", AHUNA] {
        for _ in 0..8 {
            if rig
                .tw
                .world()
                .module_state::<GeologyState>()
                .is_some_and(|state| {
                    state
                        .homes()
                        .iter()
                        .any(|(site, _, keyless)| site.as_str() == entry && !keyless)
                })
            {
                break;
            }
            let (_, queued) = fire(&mut rig);
            land_quietly(&mut rig, &queued);
        }
    }
    let _ = act_go(&mut rig, c, "+tv/1 launch go");
    let _ = finish(&mut rig, c_tv, c);
    let views = [json!({ "view": "drive" }), json!({ "view": "site" })];
    let frames = |rig: &mut Rig| -> Vec<Value> {
        views
            .iter()
            .map(|args| map_frame(rig, b, args.clone()))
            .collect()
    };
    let before = frames(&mut rig);
    for frame in &before {
        assert!(
            frame["units"]
                .as_array()
                .is_some_and(|units| units.iter().all(|u| u["glyph"] != "o"))
        );
        assert_eq!(frame["site"]["others"], json!([]));
    }
    // C drives on Occator; mid-leg, B's Ahuna map is unchanged.
    let _ = act_go(&mut rig, c, "+tv/1 drive 144,112 go");
    let act = act_of(&mut rig.tw, c_tv);
    assert_eq!(act.site.as_str(), "occator_port");
    let (_, queued) = advance_to(&mut rig, act.started_ic + (act.due_ic - act.started_ic) / 2);
    land_quietly(&mut rig, &queued);
    assert!(ic_now(&rig.tw) < act.due_ic, "still under way");
    assert_eq!(
        frames(&mut rig),
        before,
        "another site's drive is not drawn"
    );
    let _ = finish(&mut rig, c_tv, c);
    // A loud sounding on Occator, then staff put C's rig on Ahuna's ground inside the hour.
    let _ = act_go(&mut rig, c, "+tv/1 sound go");
    let _ = finish(&mut rig, c_tv, c);
    let last = record_of(&mut rig.tw, c_tv)
        .and_then(|record| record.last)
        .expect("the sounding's outcome");
    assert_eq!(
        (last.kind, last.result, last.site.as_str()),
        (ActKind::Sound, ActResult::Done, "occator_port")
    );
    to_cell(&mut rig, c_tv, (150, 150));
    let site = map_frame(&mut rig, b, json!({ "view": "site" }));
    let others = site["site"]["others"].as_array().expect("others");
    assert_eq!(others.len(), 1, "{others:?}");
    assert_eq!(others[0]["cell"], json!([150, 150]));
    assert_eq!(
        others[0]["sounding"],
        json!(false),
        "a sounding on another site is no mark here"
    );
    let out = run(&mut rig.tw, b, "+tv/map site");
    assert!(!out.contains(" (a sounding)"), "{out}");
    // C, with no rig on Occator's ground or in the bay: no slot named.
    let (out, queued) = command(&mut rig, c, "+tv/map");
    assert_eq!(out.trim(), MAP_NO_RIGS.replace("{site}", "Occator Rim"));
    assert!(queued.is_empty());
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// D11–D13. The map
// ---------------------------------------------------------------------------

/// The keyed truth's glyph at a pocket for a family (the frame's digit rule).
fn truth_glyph(ground: &Ground, pocket: (i64, i64), family: usize) -> char {
    let classes = ground.pocket(pocket);
    match classes[family] {
        tacsim::surface::Word::Rich => '3',
        tacsim::surface::Word::Fair => '2',
        tacsim::surface::Word::Lean => '1',
        tacsim::surface::Word::Barren
            if classes.iter().any(|c| *c != tacsim::surface::Word::Barren) =>
        {
            '+'
        }
        tacsim::surface::Word::Barren => '.',
    }
}

/// The Dray-P's ore compartments' mass limit, kilograms, from content.
fn ore_mass_limit(tw: &TestWorld) -> u64 {
    let spec = store(tw)
        .definitions()
        .specs
        .get(&cid(DRAY_P))
        .cloned()
        .expect("the Dray-P");
    let logistics = spec.logistics.expect("its logistics");
    logistics
        .stowage
        .values()
        .filter(|compartment| compartment.accepts.contains(&cid("stow.ore")))
        .map(|compartment| {
            compartment
                .limits
                .get(&cid("si.mass_kg"))
                .copied()
                .unwrap_or(0)
        })
        .sum()
}

/// **D11 — the map shows your readings and nothing else** (lead rulings 6 and 7): two rigs on
/// the ground, the first having scanned and sounded a field's middle. `+tv/1 map` prints the
/// title, doc 20's legend verbatim (over two lines within 118 columns), 28 grid rows of 56
/// columns, the footer and the vector lines; every sounded pocket's address cell in the window
/// holds the keyed truth's digit in the family shown — the family with most readings — and `@`
/// stands on each rig with its slot east of it. `+tv/map site` prints 32 rows of 64 columns with
/// every block's letter at its north-west corner and `H` on the pad; `+tv/map volatiles` switches
/// the family; `+tv/map quartz` answers `MAP_FAMILY_UNKNOWN`. The frame's `units` reproduce its
/// `rows` through `mush_ansi::expand_rows` (R5), and with one rig's hopper holding 3 t of metal
/// ore and the hold 10 t, the hold's unreserved room is `⌊(limit − 13,000) / 1000⌋` t (R6).
#[test]
#[allow(clippy::too_many_lines)]
fn sm4b_the_map_shows_your_readings_and_nothing_else() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 2);
    launched(&mut rig, pilot, tvs[0]);
    let _ = act_go(&mut rig, pilot, "+tv/2 launch go");
    let _ = finish(&mut rig, tvs[1], pilot);
    let stand = (144, 176);
    to_cell(&mut rig, tvs[0], stand);
    to_cell(&mut rig, tvs[1], (148, 170));
    let _ = act_go(&mut rig, pilot, "+tv/1 scan go");
    let _ = finish(&mut rig, tvs[0], pilot);
    let _ = act_go(&mut rig, pilot, "+tv/1 sound go");
    let _ = finish(&mut rig, tvs[0], pilot);
    let chart = chart_of(&mut rig.tw, pilot).expect("the row");
    let ground = ahuna(&mut rig.tw);
    // The family with most readings among the sounded pockets in the drive view.
    let frame = map_frame(&mut rig, pilot, json!({ "view": "drive", "rig": 1 }));
    let x0 = frame["origin_cell"][0].as_i64().expect("x0");
    let y0 = frame["origin_cell"][1].as_i64().expect("y0");
    let family_label = frame["site"]["family"]
        .as_str()
        .expect("a family")
        .to_owned();
    let families = [
        "metals",
        "volatiles",
        "carbon feedstock",
        "strategic elements",
    ];
    let family = families
        .iter()
        .position(|f| *f == family_label)
        .expect("a family");
    let in_view = |p: (i64, i64)| {
        let (cx, cy) = (p.0 * 4 + 2, p.1 * 4 + 2);
        (x0..x0 + 28).contains(&cx) && (y0..y0 + 28).contains(&cy)
    };
    let sounded: Vec<(i64, i64)> = (0..64_i64)
        .flat_map(|py| (0..64_i64).map(move |px| (px, py)))
        .filter(|p| chart.pocket_sounded(*p))
        .collect();
    assert!(!sounded.is_empty());
    // The default family: the most non-barren readings in view — sounded pockets at their
    // address cells and scanned fields at their middles, ties in family order.
    let field_in_view = |f: (i64, i64)| {
        let (cx, cy) = (f.0 * 32 + 16, f.1 * 32 + 16);
        (x0..x0 + 28).contains(&cx) && (y0..y0 + 28).contains(&cy)
    };
    let counts: Vec<usize> = (0..4)
        .map(|f| {
            let pockets = sounded
                .iter()
                .filter(|p| in_view(**p))
                .filter(|p| ground.pocket(**p)[f] != tacsim::surface::Word::Barren)
                .count();
            let fields = chart
                .fields
                .iter()
                .filter_map(|(index, row)| {
                    let field = (i64::from(*index) % 8, i64::from(*index) / 8);
                    (field_in_view(field) && row.footing != tacsim::surface::Footing::Ice).then(
                        || {
                            tacsim::surface::reading::read_r1(
                                &geology_keyed::mac(KEY_1),
                                &ground.body,
                                &ground.field(field),
                                row.footing,
                            )[f]
                        },
                    )
                })
                .filter(|reading| {
                    *reading != tacsim::surface::FieldReading::Exact(tacsim::surface::Word::Barren)
                })
                .count();
            pockets + fields
        })
        .collect();
    let best = counts.iter().copied().max().unwrap_or(0);
    let first_best = counts.iter().position(|c| *c == best).unwrap_or(0);
    assert_eq!(family, first_best, "{counts:?} vs {family_label}");
    let rows = frame_rows(&frame);
    assert_eq!(rows.len(), 28);
    let at = |cell: (i64, i64)| {
        (
            usize::try_from(cell.0 - x0).expect("x"),
            usize::try_from(cell.1 - y0).expect("y"),
        )
    };
    // The rigs and their slots stand over whatever is beneath them; everywhere else a sounded
    // pocket's address cell holds the keyed truth's glyph in the family shown.
    let rigs_cells = [stand, (stand.0 + 1, stand.1), (148, 170), (149, 170)];
    let mut checked = 0;
    for p in sounded.iter().filter(|p| in_view(**p)) {
        let cell = (p.0 * 4 + 2, p.1 * 4 + 2);
        if rigs_cells.contains(&cell) {
            continue;
        }
        let glyph = glyph_at(&rows, at(cell)).expect("a glyph");
        assert_eq!(glyph, truth_glyph(&ground, *p, family), "{p:?}");
        checked += 1;
    }
    assert!(checked > 0);
    assert_eq!(glyph_at(&rows, at(stand)), Some('@'));
    assert_eq!(glyph_at(&rows, at((stand.0 + 1, stand.1))), Some('1'));
    assert_eq!(glyph_at(&rows, at((148, 170))), Some('@'));
    assert_eq!(glyph_at(&rows, at((149, 170))), Some('2'));
    // The units overlay reproduces the rows (R5).
    let expanded: Vec<String> = mush_ansi::panel::expand_rows(&frame)
        .into_iter()
        .map(|r| r.into_iter().collect())
        .collect();
    assert_eq!(expanded, rows);
    // The review's correctness/F6: every reading the map lists is one the render door
    // (`chart::pocket_reading`, `chart::field_reading`) gives the same viewer — the map is never
    // a second way to read a reading.
    let listed = frame["site"]["readings"].as_array().expect("readings");
    assert!(!listed.is_empty());
    for reading in listed {
        let text = reading["text"].as_str().expect("a text");
        let cell = (
            reading["cell"][0].as_i64().expect("x"),
            reading["cell"][1].as_i64().expect("y"),
        );
        let (name, words) = text.split_once(' ').expect("a name and its words");
        let (door, separator) = if name.contains('-') {
            (
                tacsim::surface::chart::field_reading(
                    rig.tw.world(),
                    &cid(AHUNA),
                    Some(&chart),
                    (cell.0 / 32, cell.1 / 32),
                ),
                "; ",
            )
        } else {
            (
                tacsim::surface::chart::pocket_reading(
                    rig.tw.world(),
                    &cid(AHUNA),
                    Some(&chart),
                    (cell.0 / 4, cell.1 / 4),
                ),
                ", ",
            )
        };
        let parts = door
            .strip_prefix(&format!("{name} "))
            .unwrap_or_else(|| panic!("the door names {name}: {door}"));
        assert!(
            parts.split(separator).any(|part| part == words),
            "{text} is not the door's {door}"
        );
    }
    // The line print.
    let out = run(&mut rig.tw, pilot, "+tv/1/map");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines.first().copied(),
        Some(format!("Rig 1 on Ahuna Flank: {family_label} shown").as_str())
    );
    assert_eq!(
        format!("{}{}", lines[1], lines[2]),
        tacsim::port::site_map::SITE_LEGEND
    );
    for (i, row) in rows.iter().rev().enumerate() {
        let doubled: String = row.chars().flat_map(|c| [c, c]).collect();
        assert_eq!(lines[3 + i], doubled, "row {i}");
        assert_eq!(lines[3 + i].chars().count(), 56);
    }
    assert!(
        lines[31].starts_with(&format!(
            "x {x0}..{}  y {y0}..{}  of 256x256  rig 1 at (144, 176)",
            x0 + 27,
            y0 + 27
        )),
        "{}",
        lines[31]
    );
    assert!(lines.iter().all(|l| l.chars().count() <= 118), "{out}");
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with(&format!("Readings, {family_label}: "))),
        "{out}"
    );
    // The site view.
    let site = map_frame(&mut rig, pilot, json!({ "view": "site" }));
    let site_rows = frame_rows(&site);
    assert_eq!(site_rows.len(), 32);
    // Each block's letter on its north-west corner cell, under the layers above it (a reading,
    // a pit, a rig or the hull may stand there).
    let mut letters = 0;
    for by in 0..4_usize {
        for bx in 0..4_usize {
            let letter = char::from(b"ABCDEFGHIJKLMNOP"[(3 - by) * 4 + bx]);
            let glyph = glyph_at(&site_rows, (bx * 8, by * 8 + 7)).expect("a corner");
            if glyph == letter {
                letters += 1;
            } else {
                assert!(
                    "123+.vV#@oH".contains(glyph),
                    "block {letter}'s corner holds {glyph:?}"
                );
            }
        }
    }
    assert!(letters >= 12, "{letters} letters show");
    assert_eq!(glyph_at(&site_rows, (16, 16)), Some('H'));
    let expanded: Vec<String> = mush_ansi::panel::expand_rows(&site)
        .into_iter()
        .map(|r| r.into_iter().collect())
        .collect();
    assert_eq!(expanded, site_rows);
    let out = run(&mut rig.tw, pilot, "+tv/map site");
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines.len() > 3 + 32, "{out}");
    for (i, row) in site_rows.iter().rev().enumerate() {
        assert_eq!(lines[3 + i].chars().count(), 64, "{}", lines[3 + i]);
        let doubled: String = row.chars().flat_map(|c| [c, c]).collect();
        assert_eq!(lines[3 + i], doubled);
    }
    assert_eq!(lines[35], "the whole site, 80 m a cell, north up");
    // The family switch and an unknown one.
    let out = run(&mut rig.tw, pilot, "+tv/1/map volatiles");
    assert!(
        out.starts_with("Rig 1 on Ahuna Flank: volatiles shown"),
        "{out}"
    );
    let out = run(&mut rig.tw, pilot, "+tv/map quartz");
    assert_eq!(out.trim(), MAP_FAMILY_UNKNOWN.replace("{word}", "quartz"));
    // The hold's unreserved room (R6): 3 t in rig 1's hopper, 10 t in the hold.
    let hopper = tacsim::port::vehicle::hopper_of(rig.tw.world(), tvs[0]).expect("a hopper");
    seed_pool(&mut rig, hopper, "metal_ore", 3, LotId(9_901));
    let stowage = ObjId(row.rooms[&cid("stowage")]);
    let hold = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy")
        .snapshot()
        .vault_by_backing(&mush_economy::CustodyRef::SpatialInterior(stowage))
        .expect("the hold vault")
        .id;
    seed_pool(&mut rig, hold, "metal_ore", 10, LotId(9_903));
    let frame = map_frame(&mut rig, pilot, json!({ "view": "site" }));
    let limit = ore_mass_limit(&rig.tw);
    assert_eq!(
        frame["site"]["hold"]["room_t"].as_u64(),
        Some((limit - 13_000) / 1_000),
        "the limit less the hold's 10 t less the hopper's 3 t"
    );
    let out = run(&mut rig.tw, pilot, "+tv/map site");
    assert!(
        out.contains(&format!(
            "Hold: {} t of room unreserved.",
            (limit - 13_000) / 1_000
        )),
        "{out}"
    );
    no_refused_writes(&rig.tw);
}

/// The ore glyphs no second principal's grid may hold.
const ORE_GLYPHS: [char; 5] = ['1', '2', '3', '+', '.'];

/// A frame with its `others` rows' `sounding` set to false and the mark's words struck from its
/// text (`site.lines`, `site.print`, `site.other_rigs`): what the frame would be without the
/// transient mark.
fn without_mark(mut frame: Value) -> Value {
    if let Some(others) = frame["site"]["others"].as_array_mut() {
        for other in others {
            other["sounding"] = json!(false);
        }
    }
    for key in ["lines", "print"] {
        if let Some(lines) = frame["site"][key].as_array_mut() {
            for line in lines {
                let text = line
                    .as_str()
                    .unwrap_or_default()
                    .replace(" (a sounding)", "");
                *line = json!(text);
            }
        }
    }
    // Doc 20 cut SM.9's pre-commit round: the other-rigs sentence rides whole as
    // `site.other_rigs` too, the mark in it.
    if let Some(text) = frame["site"]["other_rigs"].as_str() {
        frame["site"]["other_rigs"] = json!(text.replace(" (a sounding)", ""));
    }
    frame
}

/// Two parties on Ahuna Flank's pad: A's rig launched and placed at `stand`; B's rig in the bay
/// at full charge (so B's `rigs` block cannot drift between frames, F12; and no slot digit can
/// stand in B's grid, F9).
fn two_parties(rig: &mut Rig, stand: (i64, i64)) -> (ObjId, ObjId, ObjId, ObjId) {
    let (a, _, a_tvs) = on_pad(rig, 1);
    let (b, _, b_tvs) = on_pad_as(rig, "Other", 2, "Tally Moth", 1);
    launched(rig, a, a_tvs[0]);
    to_cell(rig, a_tvs[0], stand);
    put_charge(rig, b_tvs[0], 100_000, true);
    (a, a_tvs[0], b, b_tvs[0])
}

/// **D12 — a second principal sees no ore glyph** (V5's world half; §5's observer rule): B on
/// the same pad, B's one rig in the bay at full charge. B's `site.map` frames (both views) and
/// B's `+tv/map site` print before and after A's scan and quiet sounding are byte-identical — A's
/// rig unmoved — none of B's grid rows holds `1`, `2`, `3`, `+` or `.`, B's readings are empty,
/// and A's rig is an `o` whose only id is `other-1`. A's loud sounding then changes exactly one
/// thing for B — the `others` row's `sounding` and the print's ` (a sounding)` — inside the IC
/// hour, and nothing after it. No field, tier or address of A's reaches B.
#[test]
#[allow(clippy::too_many_lines)]
fn sm4b_a_second_principal_sees_no_ore_glyph() {
    let mut rig = stocked();
    let stand = (144, 176);
    let (a, a_tv, b, _) = two_parties(&mut rig, stand);
    let views = [json!({ "view": "drive" }), json!({ "view": "site" })];
    let frames = |rig: &mut Rig| -> Vec<Value> {
        views
            .iter()
            .map(|args| map_frame(rig, b, args.clone()))
            .collect()
    };
    let before = frames(&mut rig);
    let print_before = run(&mut rig.tw, b, "+tv/map site");
    let _ = act_go(&mut rig, a, "+tv/1 scan go");
    let _ = finish(&mut rig, a_tv, a);
    let _ = act_go(&mut rig, a, "+tv/1 sound quiet go");
    let _ = finish(&mut rig, a_tv, a);
    assert!(on_ground_at(
        &rig.tw,
        a_tv,
        (stand.0 * 1_000 + 500, stand.1 * 1_000 + 500, 180_000)
    ));
    let after = frames(&mut rig);
    assert_eq!(
        serde_json::to_string(&after).expect("encodes"),
        serde_json::to_string(&before).expect("encodes"),
        "byte-identical after A's scan and quiet sounding"
    );
    let print_after = run(&mut rig.tw, b, "+tv/map site");
    assert_eq!(print_after, print_before);
    let a_chart = chart_of(&mut rig.tw, a).expect("A's row");
    let a_addresses: Vec<String> = (0..64_i64)
        .flat_map(|py| (0..64_i64).map(move |px| (px, py)))
        .filter(|p| a_chart.pocket_sounded(*p))
        .filter_map(|(px, py)| pocket_address(px, py))
        .collect();
    assert!(!a_addresses.is_empty());
    for frame in &after {
        for row in frame_rows(frame) {
            assert!(
                !row.chars().any(|c| ORE_GLYPHS.contains(&c)),
                "an ore glyph in B's grid: {row}"
            );
        }
        assert_eq!(frame["site"]["readings"], json!([]));
        let units = frame["units"].as_array().expect("units");
        let others: Vec<&Value> = units.iter().filter(|u| u["glyph"] == "o").collect();
        assert!(
            others
                .iter()
                .all(|u| u["id"] == "other-1" && u["name"] == "rig")
        );
    }
    assert!(
        after[1]["units"]
            .as_array()
            .is_some_and(|units| units.iter().any(|u| u["id"] == "other-1"))
    );
    // Past the title, the legend and the grid (the footer on): no class word, no address of A's.
    let vector: Vec<&str> = print_after.lines().skip(3 + 32).collect();
    assert!(!vector.is_empty());
    for line in &vector {
        for word in ["lean", "fair", "rich", "barren"] {
            assert!(!line.contains(word), "{word} in B's print: {line}");
        }
        for addr in &a_addresses {
            assert!(
                !line.contains(&format!("{addr} ")),
                "{addr} reached B: {line}"
            );
        }
    }
    // A's loud sounding marks B's row of A and nothing else, inside the hour.
    let _ = act_go(&mut rig, a, "+tv/1 sound go");
    let sounded_at = due_of(&mut rig.tw, a_tv);
    let _ = finish(&mut rig, a_tv, a);
    let loud = frames(&mut rig);
    assert_eq!(loud[1]["site"]["others"][0]["sounding"], json!(true));
    let unmarked: Vec<Value> = loud.iter().cloned().map(without_mark).collect();
    assert_eq!(unmarked, before, "only the mark changed");
    let print_loud = run(&mut rig.tw, b, "+tv/map site");
    assert!(print_loud.contains(" (a sounding)"), "{print_loud}");
    assert_eq!(print_loud.replace(" (a sounding)", ""), print_before);
    // After the IC hour, nothing.
    let _ = to_ic(&mut rig, sounded_at + 3_601);
    assert_eq!(frames(&mut rig), before);
    // The public pits (the review's tests/T3; the lead's note 0b, doc 20 §3.4): two cursor rows
    // of block P's metals deposit — one pocket worked (1 t drawn of more), one exhausted (its
    // keyed tonnes drawn) — show `v` and `V` on both principals' site views, at the 80 m cell
    // holding each pocket; nobody has a reading there, so no class digit stands beside them, and
    // B's grid still holds no ore glyph.
    let ground = ahuna(&mut rig.tw);
    let ore: Vec<(usize, (i64, i64), u32)> = (0..256_usize)
        .filter_map(|index| {
            let pocket = pocket_of_block_index('P', index)?;
            let tonnes = u32::from(ground.ground.geology.pocket(pocket.0, pocket.1)?.tonnes[0]);
            // Clear of block P's letter (its north-west 80 m cell) and of each other's cell.
            (pocket.1 < 14 && tonnes >= 1).then_some((index, pocket, tonnes))
        })
        .collect();
    let worked = *ore
        .iter()
        .find(|(_, _, tonnes)| *tonnes >= 2)
        .expect("a pocket of block P with 2 t of metals or more");
    let exhausted = *ore
        .iter()
        .find(|(_, pocket, _)| (pocket.0 / 2, pocket.1 / 2) != (worked.1.0 / 2, worked.1.1 / 2))
        .expect("another metal pocket of block P");
    let deposit = mush_industry::derive_system_deposit_id(&tacsim::surface::opening::receipt(
        &cid(AHUNA),
        'P',
        tacsim::surface::geology::OreFamily::Metals,
    ))
    .expect("a deposit id");
    let mut rows = vec![
        CursorRow {
            pocket: u16::try_from(worked.0 + 1).expect("a pocket number"),
            drawn_t: 1,
            lock: None,
        },
        CursorRow {
            pocket: u16::try_from(exhausted.0 + 1).expect("a pocket number"),
            drawn_t: exhausted.2,
            lock: None,
        },
    ];
    rows.sort_by_key(|row| row.pocket);
    put_cursor(
        &mut rig,
        PocketCursor {
            schema: POCKET_CURSOR_SCHEMA,
            deposit,
            rows,
        },
    );
    let a_site = map_frame(&mut rig, a, json!({ "view": "site" }));
    let b_site = map_frame(&mut rig, b, json!({ "view": "site" }));
    for frame in [&a_site, &b_site] {
        let rows = frame_rows(frame);
        let at = |pocket: (i64, i64)| {
            (
                usize::try_from(pocket.0 / 2).expect("x"),
                usize::try_from(pocket.1 / 2).expect("y"),
            )
        };
        assert_eq!(glyph_at(&rows, at(worked.1)), Some('v'), "{worked:?}");
        assert_eq!(glyph_at(&rows, at(exhausted.1)), Some('V'), "{exhausted:?}");
    }
    for row in frame_rows(&b_site) {
        assert!(
            !row.chars().any(|c| ORE_GLYPHS.contains(&c)),
            "an ore glyph in B's grid: {row}"
        );
    }
    no_refused_writes(&rig.tw);
}

/// Where a blessed site-map fixture lives (`tests/chart.rs`'s precedent).
fn fixture_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../web/src/lib/fixtures/site")
        .join(name)
}

/// `TACSIM_BLESS=1` writes `text` to the fixture; every other run compares byte for byte.
fn bless(name: &str, text: &str) {
    let path = fixture_path(name);
    if std::env::var_os("TACSIM_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the fixture dir");
        std::fs::write(&path, text).expect("the fixture is written");
    } else {
        let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            on_disk, text,
            "web/src/lib/fixtures/site/{name} is out of date; re-bless with TACSIM_BLESS=1 \
             cargo test -p tacsim --test driving sm4b_the_site_map_fixtures_are_blessed"
        );
    }
}

/// **D13 — the site-map fixtures are blessed** (lead ruling 7; the brief's decision P10): after
/// D12's setup A drives to a pocket (`ARRIVED`), scans and sounds there; at that fixed instant A's
/// site-view frame is `web/src/lib/fixtures/site/site_map.json`, B's is `site_map_other.json`,
/// and the events — A's real `ARRIVED` event and B's real `+tv/map site` events — are
/// `site_events.json`. Each frame's `site.print` is `site_map_lines` of the frame (through the
/// TUI's decoder, glyphs doubled).
#[test]
#[allow(clippy::too_many_lines)]
fn sm4b_the_site_map_fixtures_are_blessed() {
    let mut rig = stocked();
    let (a, a_tv, b, _) = two_parties(&mut rig, (128, 124));
    let (addr, _, _) = north_pocket();
    let _ = act_go(&mut rig, a, &format!("+tv/1 drive {addr} go"));
    let due = due_of(&mut rig.tw, a_tv);
    let (events, queued) = advance_to(&mut rig, due);
    land_all(&mut rig, &queued);
    let (more, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    // A's own arrival (the review's tests/T6): the event A sees, and exactly `ARRIVED` for the
    // north pocket with no reading (A has sounded nothing there).
    let arrived = ARRIVED
        .replace("{n}", "1")
        .replace("{addr}", &addr)
        .replace("{reading}", "");
    let arrival = events
        .iter()
        .chain(more.iter())
        .find(|(audience, event)| {
            said(&rig.tw, &[(audience.clone(), event.clone())], a).trim() == arrived
        })
        .map(|(_, event)| event.clone())
        .expect("A's arrival event, exactly ARRIVED");
    let _ = act_go(&mut rig, a, "+tv/1 scan go");
    let _ = finish(&mut rig, a_tv, a);
    let _ = act_go(&mut rig, a, "+tv/1 sound quiet go");
    let _ = finish(&mut rig, a_tv, a);
    let mine = map_frame(&mut rig, a, json!({ "view": "site" }));
    let other = map_frame(&mut rig, b, json!({ "view": "site" }));
    for frame in [&mine, &other] {
        let print: Vec<String> = frame["site"]["print"]
            .as_array()
            .expect("a print")
            .iter()
            .map(|l| l.as_str().expect("a line").to_owned())
            .collect();
        let rows = frame_rows(frame);
        for (i, row) in rows.iter().rev().enumerate() {
            let doubled: String = row.chars().flat_map(|c| [c, c]).collect();
            assert_eq!(print[3 + i], doubled);
        }
    }
    assert!(
        frame_rows(&mine)
            .iter()
            .any(|row| row.chars().any(|c| ORE_GLYPHS.contains(&c)))
    );
    assert!(
        frame_rows(&other)
            .iter()
            .all(|row| !row.chars().any(|c| ORE_GLYPHS.contains(&c)))
    );
    // The command's print is the feed frame's print, line for line, for both principals (the
    // review's tests/T6: the `Ctx` reads and the `HookReadContext` reads agree — the SSH fixture
    // below and the TUI/web fixtures above are one print).
    for (who, frame) in [(a, &mine), (b, &other)] {
        let ran = rig.tw.run(who, "+tv/map site");
        let printed: Vec<String> = rig
            .tw
            .seen_by(&ran, who)
            .iter()
            .flat_map(|text| text.lines().map(str::to_owned).collect::<Vec<_>>())
            .collect();
        let print: Vec<String> = frame["site"]["print"]
            .as_array()
            .expect("a print")
            .iter()
            .map(|l| l.as_str().expect("a line").to_owned())
            .collect();
        assert_eq!(printed, print);
    }
    let other_map: Vec<OutputEvent> = rig
        .tw
        .run(b, "+tv/map site")
        .into_iter()
        .map(|(_, event)| event)
        .collect();
    assert!(!other_map.is_empty());
    let pretty = |value: &Value| serde_json::to_string_pretty(value).expect("encodes") + "\n";
    bless("site_map.json", &pretty(&mine));
    bless("site_map_other.json", &pretty(&other));
    bless(
        "site_events.json",
        &pretty(&json!({ "arrival": arrival, "other_map": other_map })),
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// D14. The planner, measured (lead ruling 13)
// ---------------------------------------------------------------------------

/// The median of five timings, milliseconds (fractional).
fn median_ms(samples: &[f64]) -> f64 {
    let mut samples = samples.to_vec();
    samples.sort_by(f64::total_cmp);
    samples.get(samples.len() / 2).copied().unwrap_or(0.0)
}

/// The least of five timings, milliseconds (fractional): cut AP's gate statistic — a slow
/// runner's load is not the code's cost.
fn min_ms(samples: &[f64]) -> f64 {
    samples.iter().copied().fold(f64::INFINITY, f64::min)
}

/// **Ahuna Flank's corner-to-corner leg's cost**, `drive_leg` from pose `(500, 500)` to cell
/// `(255, 255)` over the keyed cached composite (`site_ground_cached`'s warm ground surface) —
/// recorded at `a0030f71` + SM.7 before cut AP changed `mush_atlas::physics::path` (294 cells,
/// 18 waypoints, the old search visiting 58,159 cells). Re-bless only with the lane's terrain (a
/// lane version or a terrain generation change), never for AP, which proves every cost equal.
const AP_AHUNA_CORNER_COST: u64 = 60_720;

/// **D14 — the drive planner is measured, and gated** (lead ruling 13; R4, F8; cut AP turned it
/// into a release gate): on Ahuna Flank's cached composite, (a) the worst case corner to corner,
/// `(0, 0)` → `(255, 255)`, from the peek onward (the site's non-building peek, the target, the
/// A*, the freeze, the charge home); (b) the budget exhausted — an unreachable target on an open
/// 256 × 256 synthetic composite, walled by an impassable ring; (c) the home field's build over
/// the cached composite, and the keyless terrain capture and build. Five samples each; the min
/// and the median printed (`--nocapture`). **The gate is the minimum of five**, because a slow
/// runner's load is not the code's cost: in a release build, always, planning ≤ 10 ms, the
/// cached build ≤ 20 ms and the exhausted flood (the same flood) ≤ 20 ms — the gate runner runs
/// this test with `--release`; in a debug build under `TACSIM_ASSERT=1`, the lead's debug
/// figures, 100 ms / 250 ms / 250 ms. CI runs only debug, where the tight figures assert only
/// under `TACSIM_ASSERT=1`, because a shared runner's speed is not a threshold (D-H-8, D-16-19):
/// without it a debug build keeps SM.4b's generous ceilings on the median — ten times the old
/// STOP, 250 ms planning and 2,000 ms a build — so a slow runner never flakes. The keyless
/// capture and build keep that old ceiling in every build (AP does not own the capture). Outside
/// the timed loop, the corner leg's cost is the one recorded before AP
/// ([`AP_AHUNA_CORNER_COST`]), in every build.
#[test]
fn sm4b_the_drive_planner_is_measured() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    launched(&mut rig, pilot, tvs[0]);
    let rates = rates(&rig.tw);
    let (spec, _, location) = spec_of(&rig.tw, AHUNA);
    let frame = site_frame(&location);
    let field = home_field(&rig.tw);
    // (a) Planning, from the peek onward.
    let planning: Vec<f64> = (0..5)
        .map(|_| {
            let started = std::time::Instant::now();
            let ground = tacsim::surface::site_ground_cached(rig.tw.world(), &frame, &spec)
                .expect("peeks")
                .expect("warm");
            let surface = ground
                .composite
                .surface(&mush_atlas::SurfaceRef::Ground)
                .expect("a surface");
            let to = tacsim::surface::drive::target_cell(
                &tacsim::surface::drive::Target::Cell { x: 255, y: 255 },
                (0, 0),
            )
            .expect("on the site");
            let leg = drive_leg(surface, table(), (500, 500), to, &rates).expect("a leg");
            let _ = home_cu_to(&field, to, &rates);
            assert!(leg.waypoints.len() <= 32);
            started.elapsed().as_secs_f64() * 1_000.0
        })
        .collect();
    // (b) The budget exhausted.
    let open = {
        let ground = ahuna(&mut rig.tw);
        let mut surface = ground.surface().clone();
        for y in 0..256_u16 {
            for x in 0..256_u16 {
                surface.cells.set(
                    x,
                    y,
                    mush_atlas::Cell {
                        class: mush_atlas::TerrainClass::Regolith,
                        elev_cm: 0,
                        cover: 0,
                        room: 0,
                    },
                );
            }
        }
        for (x, y) in [
            (199, 199),
            (200, 199),
            (201, 199),
            (199, 200),
            (201, 200),
            (199, 201),
            (200, 201),
            (201, 201),
        ] {
            surface.cells.set(
                x,
                y,
                mush_atlas::Cell {
                    class: mush_atlas::TerrainClass::Wall,
                    elev_cm: 0,
                    cover: 0,
                    room: 0,
                },
            );
        }
        surface
    };
    let exhausted: Vec<f64> = (0..5)
        .map(|_| {
            let started = std::time::Instant::now();
            let refused = drive_leg(&open, table(), (500, 500), (200, 200), &rates);
            assert_eq!(refused, Err(LegRefused::NoPath));
            started.elapsed().as_secs_f64() * 1_000.0
        })
        .collect();
    // (c) The home field's builds.
    let keyed_build: Vec<f64> = (0..5)
        .map(|_| {
            let ground = ahuna(&mut rig.tw);
            let started = std::time::Instant::now();
            let built = HomeField::build(ground.surface(), table(), RAMP_CELL).expect("a field");
            assert_eq!(built.ramp(), RAMP_CELL);
            started.elapsed().as_secs_f64() * 1_000.0
        })
        .collect();
    let keyless_build: Vec<f64> = (0..5)
        .map(|_| {
            let started = std::time::Instant::now();
            let terrain = tacsim::surface::ground::site_terrain(rig.tw.world(), &cid(AHUNA))
                .expect("the terrain");
            let surface = terrain
                .surface(&mush_atlas::SurfaceRef::Ground)
                .expect("a surface");
            let _ = HomeField::build(surface, table(), RAMP_CELL).expect("a field");
            started.elapsed().as_secs_f64() * 1_000.0
        })
        .collect();
    // The corner leg's cost, outside the timed loop (cut AP): the old search's, on the same
    // keyed cached composite from the same pose.
    {
        let ground = tacsim::surface::site_ground_cached(rig.tw.world(), &frame, &spec)
            .expect("peeks")
            .expect("warm");
        let surface = ground
            .composite
            .surface(&mush_atlas::SurfaceRef::Ground)
            .expect("a surface");
        let leg = drive_leg(surface, table(), (500, 500), (255, 255), &rates).expect("a leg");
        let visited = match mush_atlas::path(
            surface,
            table(),
            (0, 0),
            (255, 255),
            DRIVE_LOCOMOTION,
            &[],
            DRIVE_PATH_BUDGET_CELLS,
        ) {
            mush_atlas::PathOutcome::Found(found) => found.visited,
            other => panic!("{other:?}"),
        };
        println!(
            "the corner-to-corner leg: cost {}, {} cells, {} waypoints; the search visited \
             {visited} cells",
            leg.cost,
            leg.cells.len(),
            leg.waypoints.len()
        );
        assert_eq!(
            leg.cost, AP_AHUNA_CORNER_COST,
            "the corner-to-corner leg's cost moved from what the old search found"
        );
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let (planning_min, exhausted_min, keyed_min, keyless_min) = (
        min_ms(&planning),
        min_ms(&exhausted),
        min_ms(&keyed_build),
        min_ms(&keyless_build),
    );
    let (planning, exhausted, keyed_build, keyless_build) = (
        median_ms(&planning),
        median_ms(&exhausted),
        median_ms(&keyed_build),
        median_ms(&keyless_build),
    );
    println!(
        "{profile}: planning corner to corner min {planning_min:.2} / median {planning:.2} ms; \
         the budget exhausted min {exhausted_min:.2} / median {exhausted:.2} ms; the home field \
         over the cached composite min {keyed_min:.2} / median {keyed_build:.2} ms; the keyless \
         terrain and its home field min {keyless_min:.2} / median {keyless_build:.2} ms"
    );
    let asserted = std::env::var("TACSIM_ASSERT").is_ok_and(|v| v.trim() == "1");
    // SM.4b's STOP, whose ten-times ceiling the debug default and the keyless build keep.
    let (plan_stop, home_stop) = if cfg!(debug_assertions) {
        (250.0, 2_000.0)
    } else {
        (20.0, 50.0)
    };
    if !cfg!(debug_assertions) {
        // Cut AP's release gate (the lead's 10 ms / 20 ms), on the minimum of five.
        assert!(
            planning_min <= 10.0,
            "release planning {planning_min} ms > 10 ms"
        );
        assert!(
            keyed_min <= 20.0,
            "release home build {keyed_min} ms > 20 ms"
        );
        assert!(
            exhausted_min <= 20.0,
            "release exhausted {exhausted_min} ms > 20 ms"
        );
    } else if asserted {
        // The lead's debug figures, under `TACSIM_ASSERT=1` only (the brief's R8).
        assert!(
            planning_min <= 100.0,
            "debug planning {planning_min} ms > 100 ms"
        );
        assert!(
            keyed_min <= 250.0,
            "debug home build {keyed_min} ms > 250 ms"
        );
        assert!(
            exhausted_min <= 250.0,
            "debug exhausted {exhausted_min} ms > 250 ms"
        );
    } else {
        assert!(planning < plan_stop * 10.0, "{planning} ms");
        assert!(exhausted < plan_stop * 10.0, "{exhausted} ms");
        assert!(keyed_build < home_stop * 10.0, "{keyed_build} ms");
    }
    assert!(keyless_build < home_stop * 10.0, "{keyless_build} ms");
}
