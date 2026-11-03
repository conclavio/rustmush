//! The helpers of `driving` (crates/tacsim/tests/shard_N/driving.rs), hoisted
//! here on 2026-11-03 so that the modules which import them can live in any
//! shard binary: a test module can only import from a module of its own
//! binary, and with the helpers inside the test file every importer was
//! pinned to the file's shard (the field cluster was 62 summed test-minutes
//! in one shard). Every item and import is `pub(crate)`; the test half keeps
//! its name and takes them back with `use crate::driving_rig::*`. Each shard
//! root declares this module by path, like support/vertex.rs.
#![allow(dead_code, unused_imports)]

pub(crate) use crate::support;

pub(crate) use crate::geology_keyed;

pub(crate) use std::collections::{BTreeMap, HashMap};
pub(crate) use std::sync::{Arc, Mutex};

pub(crate) use mush_core::{
    AccountId, AccountIdentity, ActionStateBootRecord, Audience, AuthoritativeSubmit, CanonicalId,
    CommitToken, GOD, IdempotencyKey, IdempotencyScope, ModuleWrite, ObjId, OfferExecutionContext,
    OfferSubmission, PerceivedRef, RequestUid, ScopedIdempotencyKey, World,
};
pub(crate) use mush_economy::{EconomyConfig, VaultId};
pub(crate) use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
pub(crate) use mush_proto::OutputEvent;
pub(crate) use mush_sim::data::{SpatialParent, SpatialState};
pub(crate) use mush_testkit::{TestWorld, fold_module_ops_into};
pub(crate) use serde_json::{Value, json};
pub(crate) use smol_str::SmolStr;
pub(crate) use tacsim::definitions::content::{DRAY_P, LANE_JSON, LANE_PACK, TV_PROSPECTOR};
pub(crate) use tacsim::definitions::tools::{BatteryDef, ToolDef, tools_table};
pub(crate) use tacsim::pack::{PackEntry, RulesPack, load_pack};
pub(crate) use tacsim::port::tv::{
    ARRIVED, DRIVE_HERE, DRIVE_OFF_SITE, DRIVE_USAGE, GROUND_COLD, IMPLIED_LAUNCH,
    INSTRUMENTS_DARK, LAUNCH_OPEN, MAP_FAMILY_UNKNOWN, MAP_NO_RIGS, RECOVER_SHORT, RECOVERED,
    RESERVE_LOW, TV_DARK, TWO_LEGS,
};
pub(crate) use tacsim::port::{
    HullRow, PortHandles, PortMap, PortMapState, Ticket, TicketId, room_key,
};
pub(crate) use tacsim::surface::address::{pocket_address, pocket_of_block_index};
pub(crate) use tacsim::surface::cursor::{CursorRow, POCKET_CURSOR_SCHEMA, PocketCursor};
pub(crate) use tacsim::surface::drive::{
    DRIVE_LOCOMOTION, DRIVE_PATH_BUDGET_CELLS, DriveRates, HomeField, LEG_TOLERANCE_CELLS, Leg,
    LegRefused, cu_of_cost, drive_leg, home_cu_to, recover_leg, secs_of_cost,
};
pub(crate) use tacsim::surface::geology::{BodyGeology, FieldGround, field_ground};
pub(crate) use tacsim::surface::ground::{SiteGeologySpec, SiteGround, site_ground_sync};
pub(crate) use tacsim::surface::reading::read_r2;
pub(crate) use tacsim::surface::{
    ActKind, ActParams, ActResult, Charge, ChartKey, ChartRecord, GeologyBoot, GeologyState,
    RigKey, RigRecord, SURFACE_ACT_ACTION, SURFACE_DONE_ACTION, TvRec, TvRecState, charge_at,
    pocket_row_text,
};
pub(crate) use tacsim::{TacsimConfig, TacsimStore};

pub(crate) use geology_keyed::{KEY_1, mac, secret};

/// The Dray-P's stern ramp on a site's grid.
pub(crate) const RAMP: (i64, i64, u32) = (128_000, 124_500, 180_000);
/// The ramp's cell.
pub(crate) const RAMP_CELL: (i64, i64) = (128, 124);
/// Ahuna Flank's port entry.
pub(crate) const AHUNA: &str = "ahuna_port";

pub(crate) fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

// ---------------------------------------------------------------------------
// The rig (excursions.rs's, booted keyed)
// ---------------------------------------------------------------------------

/// The composition: `commerce.rs`'s bare roster (no sheet, the test world's layer, the
/// shipped game content).
#[derive(Clone, Default)]
pub(crate) struct Roster(pub(crate) TacsimConfig);

/// The `[tacsim]` every keyed rig boots under: the layer on, everything else the default.
pub(crate) fn rig_config() -> TacsimConfig {
    TacsimConfig {
        enabled: true,
        ..TacsimConfig::default()
    }
}

impl Roster {
    pub(crate) fn modules(self) -> Vec<Box<dyn mush_core::GameModule>> {
        use tacsim::composition as comp;

        let game = tacsim::GameEconomyConfig::default();
        let cfg = self.0;
        let (mut mods, inventory) = comp::domain_modules_with_economy(
            EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &game,
        );
        mods.push(comp::npc_module(comp::npc_agent_adapter()));
        mods.push(comp::tacsim_test_world_module_wired(cfg, true, inventory));
        mods.push(comp::content_module_with(true, true).expect("the content module composes"));
        mods
    }
}

/// A fixed gateway-account oracle (`match_lifecycle.rs`'s precedent).
#[derive(Default)]
pub(crate) struct MapOracle(pub(crate) Mutex<BTreeMap<ObjId, AccountId>>);

impl AccountIdentity for MapOracle {
    fn account_of(&self, character: ObjId) -> Option<AccountId> {
        self.0.lock().ok()?.get(&character).copied()
    }
}

/// A booted world, its oracle and the cumulative store.
pub(crate) struct Rig {
    pub(crate) tw: TestWorld,
    /// The `[tacsim]` it boots and reboots under (doc 32 cut CF.2: a field test tunes it).
    pub(crate) cfg: TacsimConfig,
    pub(crate) oracle: Arc<MapOracle>,
    pub(crate) store: HashMap<SmolStr, mush_core::ModuleBootData>,
    pub(crate) typed: BTreeMap<(SmolStr, SmolStr), serde_json::Value>,
}

pub(crate) fn no_refused_writes(tw: &TestWorld) {
    let refused = tw.world().refused_namespace_writes();
    assert!(refused.is_empty(), "refused namespace writes: {refused:?}");
}

pub(crate) fn install_oracle(tw: &mut TestWorld, oracle: &Arc<MapOracle>) {
    *tw.world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .expect("the simulation is composed") = mush_sim::AccountOracle::new(oracle.clone());
}

pub(crate) fn with_offer_keys(tw: &mut TestWorld) {
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
pub(crate) fn boot_over(
    world: World,
    store: HashMap<SmolStr, mush_core::ModuleBootData>,
    keyed: bool,
    cfg: &TacsimConfig,
) -> TestWorld {
    boot_over_typed(world, store, None, keyed, cfg)
}

/// [`boot_over`], with `typed_first` hydrated before any module's load hook runs when given:
/// production's order (`mush_server::modules::boot_modules`), so a boot hook that reads a typed
/// record (the field host's resume reads the site record) sees what the store holds.
fn boot_over_typed(
    world: World,
    store: HashMap<SmolStr, mush_core::ModuleBootData>,
    typed_first: Option<&BTreeMap<(SmolStr, SmolStr), serde_json::Value>>,
    keyed: bool,
    cfg: &TacsimConfig,
) -> TestWorld {
    let (_, registry) = mush_testkit::boot_world();
    let mut engine =
        mush_core::Engine::with_modules(world, registry, Roster(cfg.clone()).modules())
            .expect("the roster composes");
    if let Some(typed) = typed_first {
        hydrate(&mut engine, typed);
    }
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
pub(crate) fn stocked_with(keyed: bool) -> Rig {
    stocked_under(keyed, rig_config())
}

/// **The keyed rig under a tuned `[tacsim]`** (doc 32 cut CF.2): `tune` edits the layer's
/// config before the first boot, and every reboot keeps it.
pub(crate) fn stocked_tuned(tune: impl FnOnce(&mut TacsimConfig)) -> Rig {
    let mut cfg = rig_config();
    tune(&mut cfg);
    stocked_under(true, cfg)
}

pub(crate) fn stocked_under(keyed: bool, cfg: TacsimConfig) -> Rig {
    let (world, _) = mush_testkit::boot_world();
    let mut tw = boot_over(world, HashMap::new(), keyed, &cfg);
    with_offer_keys(&mut tw);
    let oracle = Arc::new(MapOracle::default());
    install_oracle(&mut tw, &oracle);
    let mut rig = Rig {
        tw,
        cfg,
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
pub(crate) fn stocked() -> Rig {
    stocked_with(true)
}

/// **Change the `[tacsim]` the next [`reboot`] boots under** (the 2026-10-31 review's CF-3a gate
/// lowers the field cap between two fields).
pub(crate) fn retune(rig: &mut Rig, tune: impl FnOnce(&mut TacsimConfig)) {
    tune(&mut rig.cfg);
}

/// Fold every write since the first boot and reboot keyed into the same roster; every
/// connection drops (a real restart has no sessions) and `reconnect` come back. Production's
/// boot order (`mush_server::modules::boot_modules`, doc 36a lane 5): the typed records hydrate
/// before any module's load hook runs, so the field host's boot finds the site record and resumes
/// a running field.
pub(crate) fn reboot(rig: Rig, reconnect: &[ObjId]) -> Rig {
    reboot_with(rig, reconnect, true)
}

/// **The restart**: every arena task stopped and its egress drained, then [`reboot`].
pub(crate) async fn crash(
    rig: Rig,
    rx: &mut tokio::sync::mpsc::Receiver<tacsim::arena_task::ArenaOut>,
    reconnect: &[ObjId],
) -> Rig {
    let tasks = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        .expect("the registry");
    tasks.shutdown(tacsim::arena_bridge::JOIN_TIMEOUT).await;
    while rx.try_recv().is_ok() {}
    reboot(rig, reconnect)
}

/// The typed records into `engine`'s action state.
fn hydrate(
    engine: &mut mush_core::Engine,
    typed: &BTreeMap<(SmolStr, SmolStr), serde_json::Value>,
) {
    engine
        .hydrate_action_state(
            typed
                .iter()
                .map(|((ns, key), value)| {
                    ActionStateBootRecord::new(ns.clone(), key.clone(), value.clone())
                })
                .collect::<Vec<_>>(),
        )
        .expect("the typed state hydrates");
}

/// [`reboot`], keyed or not (a restart without the geology key darkens the layer).
pub(crate) fn reboot_with(rig: Rig, reconnect: &[ObjId], keyed: bool) -> Rig {
    let Rig {
        mut tw,
        cfg,
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
    let mut tw = boot_over_typed(world, store.clone(), Some(&typed), keyed, &cfg);
    with_offer_keys(&mut tw);
    install_oracle(&mut tw, &oracle);
    for who in reconnect {
        let _ = tw.connect(*who);
    }
    Rig {
        tw,
        cfg,
        oracle,
        store,
        typed,
    }
}

/// **Write one typed record by hand** (doc 36 cut GC.6: a landholder's block tenure, which only
/// industry's composed claim act writes in play): through a bootstrapping transaction, kept for
/// the next reboot.
pub(crate) fn seed_typed<K, V>(
    rig: &mut Rig,
    handle: &mush_core::StateFamilyHandle<K, V>,
    key: &K,
    value: V,
) where
    K: Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    let prepared = {
        let engine = rig.tw.engine_mut();
        let mut txn = engine.action_transaction(handle).expect("a transaction");
        txn.insert(handle, key, value).expect("the record writes");
        txn.prepare().expect("prepares")
    };
    let (ops, install) = prepared.into_parts();
    rig.tw
        .engine_mut()
        .install_prepared_action(install)
        .expect("installs");
    keep(rig, ops);
}

/// [`seed_typed`] over a record that may already stand: rewritten when it does, written when
/// not (doc 36a lane 2: a lane site's record carries the seeded custody rows).
pub(crate) fn reseed_typed<K, V>(
    rig: &mut Rig,
    handle: &mush_core::StateFamilyHandle<K, V>,
    key: &K,
    value: V,
) where
    K: Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    let prepared = {
        let engine = rig.tw.engine_mut();
        let mut txn = engine.action_transaction(handle).expect("a transaction");
        let stands = txn.get(handle, key).expect("the record reads").is_some();
        if stands {
            txn.replace(handle, key, value)
                .expect("the record rewrites");
        } else {
            txn.insert(handle, key, value).expect("the record writes");
        }
        txn.prepare().expect("prepares")
    };
    let (ops, install) = prepared.into_parts();
    rig.tw
        .engine_mut()
        .install_prepared_action(install)
        .expect("installs");
    keep(rig, ops);
}

/// Fold every write and reboot with the stored `tacsim:` document `key` edited.
pub(crate) fn reboot_editing(
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
pub(crate) fn sweep(tw: &mut TestWorld, secs: i64) {
    let t = tw.world().now().saturating_add(secs);
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1000));
}

pub(crate) fn said(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    tw.seen_by(events, who).join("\n")
}

pub(crate) fn run(tw: &mut TestWorld, who: ObjId, line: &str) -> String {
    let events = tw.run(who, line);
    said(tw, &events, who)
}

pub(crate) fn store(tw: &TestWorld) -> &TacsimStore {
    tw.world()
        .module_state::<TacsimStore>()
        .expect("tacsim state")
}

pub(crate) fn port_map(tw: &TestWorld, entry: &str) -> PortMap {
    match store(tw).port_map(&cid(LANE_PACK), &cid(entry)) {
        PortMapState::Present(map) => map.clone(),
        other => panic!("{entry}'s map: {other:?}"),
    }
}

pub(crate) fn lane() -> RulesPack {
    load_pack(LANE_JSON).expect("the lane verifies")
}

pub(crate) fn station_room(tw: &TestWorld, entry: &str, local: &str) -> ObjId {
    let def = match lane().entry(&cid(entry)) {
        Some(PackEntry::Port(def)) => def.clone(),
        other => panic!("{entry} is a port entry: {other:?}"),
    };
    let key = room_key(&def.area, &format!("{}.{local}", def.area)).expect("a room key");
    tacsim::port::apply::room_by_key(tw.world(), &key).expect("the room is on this world")
}

pub(crate) fn teleport(tw: &mut TestWorld, who: ObjId, room: ObjId) {
    let said = run(tw, GOD, &format!("@teleport #{}=#{}", who.0, room.0));
    assert_eq!(
        tw.world().get(who).and_then(|o| o.location),
        Some(room),
        "the teleport landed: {said}"
    );
}

/// A fresh request/commit token per landing, across the process.
pub(crate) fn next_token() -> [u8; 16] {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut bytes = [0x6c_u8; 16];
    bytes[..4].copy_from_slice(&n.to_be_bytes());
    bytes
}

/// Keep a prepared bundle's typed post-images for a reboot.
pub(crate) fn keep(rig: &mut Rig, ops: Vec<mush_core::ModuleOp>) {
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
pub(crate) fn land(rig: &mut Rig, submit: &AuthoritativeSubmit) -> Result<(), String> {
    land_with(rig, submit, false)
}

/// **Land one queued submission on the world task's composed path** (doc 36 cut GC.0; Q-36-49):
/// the frozen registry's owner reads, participants and account hooks reach the prepare, as
/// production's action coordinator passes them. [`land`]'s plain prepare hands the provider none,
/// so the Org unit grant (`unitgrant_issue`, a DT activation's and an operational loan's) refuses
/// there on its owner preimage read; this lands it. Object writes the install applied to the
/// World stay there; the module writes are kept for a reboot as [`land`] keeps them.
pub(crate) fn land_composed(rig: &mut Rig, submit: &AuthoritativeSubmit) -> Result<(), String> {
    land_with(rig, submit, true)
}

pub(crate) fn land_with(
    rig: &mut Rig,
    submit: &AuthoritativeSubmit,
    composed: bool,
) -> Result<(), String> {
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
    if composed {
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
        let earlier = rig.tw.take_module_ops();
        fold_module_ops_into(&mut rig.store, earlier);
        let writes: Vec<_> = writes
            .into_iter()
            .filter(|write| {
                !matches!(
                    write,
                    mush_core::authoritative::ReplayWrite::ObjectSet { .. }
                        | mush_core::authoritative::ReplayWrite::ObjectDelete { .. }
                        | mush_core::authoritative::ReplayWrite::ObjectAllocatorSet { .. }
                )
            })
            .collect();
        let (ops, ledger) = support::composed::split(&writes);
        keep(rig, ops);
        fold_module_ops_into(&mut rig.store, ledger);
        return Ok(());
    }
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
pub(crate) fn drain(tw: &mut TestWorld) -> Vec<AuthoritativeSubmit> {
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

pub(crate) fn land_all(rig: &mut Rig, queued: &[AuthoritativeSubmit]) {
    land_all_with(rig, queued, false);
}

/// [`land_all`], on the composed path when `composed` ([`land_composed`]).
pub(crate) fn land_all_with(rig: &mut Rig, queued: &[AuthoritativeSubmit], composed: bool) {
    for submit in queued {
        let landed = if composed {
            land_composed(rig, submit)
        } else {
            land(rig, submit)
        };
        landed.unwrap_or_else(|error| {
            panic!(
                "{} by #{} did not land: {error}",
                submit.action.key().id,
                submit.actor.0
            )
        });
    }
}

pub(crate) fn land_quietly(rig: &mut Rig, queued: &[AuthoritativeSubmit]) {
    for submit in queued {
        let _ = land(rig, submit);
    }
}

/// One fire (6 s on), its submissions drained (not landed).
pub(crate) fn fire(rig: &mut Rig) -> (Vec<(Audience, OutputEvent)>, Vec<AuthoritativeSubmit>) {
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    (events, queued)
}

/// The surface finishes a fire queued.
pub(crate) fn finishes(queued: &[AuthoritativeSubmit]) -> Vec<&AuthoritativeSubmit> {
    queued
        .iter()
        .filter(|submit| submit.action.key().id.as_str() == SURFACE_DONE_ACTION)
        .collect()
}

/// Fire and land the port tick until every port is stocked and listed.
pub(crate) fn stock(rig: &mut Rig) {
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

/// A pilot on a gateway account of their own, standing at the Registry desk.
pub(crate) fn pilot(rig: &mut Rig, who: &str, seed: i64) -> ObjId {
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
pub(crate) fn prospector(rig: &mut Rig, who: &str, seed: i64, name: &str) -> (ObjId, HullRow) {
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

/// The hold vault of a hull row.
pub(crate) fn hold_vault_of(tw: &TestWorld, row: &HullRow) -> VaultId {
    let stowage = ObjId(row.rooms[&cid("stowage")]);
    tw.world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy")
        .snapshot()
        .vault_by_backing(&mush_economy::CustodyRef::SpatialInterior(stowage))
        .expect("the hold vault")
        .id
}

/// Seed `qty` of `good` into `row`'s hold through `lot` (staff's path).
pub(crate) fn seed_hold(rig: &mut Rig, row: &HullRow, good: &str, qty: u64, lot: u64) {
    let vault = hold_vault_of(&rig.tw, row);
    seed_pool(rig, vault, good, qty, LotId(lot));
}

/// One pool's balance in `vault`.
pub(crate) fn hold_pool(tw: &mut TestWorld, vault: VaultId, good: &str) -> u64 {
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

/// Seed `qty` of `good` into `vault`'s pool through `lot` (the inventory's own path).
pub(crate) fn seed_pool(rig: &mut Rig, vault: VaultId, good: &str, qty: u64, lot: LotId) {
    seed_steps(rig, vault, good, qty, lot, 2);
}

/// Seed `qty` of `good` as the undissolved lot `lot` at `vault` (doc 36a P2-4: a lot for
/// `@inv/dissolve` to be refused on).
pub(crate) fn seed_lot(rig: &mut Rig, vault: VaultId, good: &str, qty: u64, lot: LotId) {
    seed_steps(rig, vault, good, qty, lot, 1);
}

/// [`seed_pool`]'s two steps — the origin mint, then its dissolve — the first `steps` of them.
pub(crate) fn seed_steps(
    rig: &mut Rig,
    vault: VaultId,
    good: &str,
    qty: u64,
    lot: LotId,
    steps: usize,
) {
    let endpoint = EndpointRef(vault);
    let good = cid(good);
    let families = rig
        .tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    for step in 0..steps {
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
pub(crate) fn load(rig: &mut Rig, row: &HullRow, tank: u64, locker: u64, lot: u64) {
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
pub(crate) fn board(rig: &mut Rig, pilot: ObjId) {
    let gallery = station_room(&rig.tw, "ceres_port", "room.c01_berth_gallery");
    teleport(&mut rig.tw, pilot, gallery);
    let out = run(&mut rig.tw, pilot, "+ship/board");
    assert!(out.contains("You cycle the lock into the "), "{out}");
}

pub(crate) fn world_rate(tw: &TestWorld) -> u32 {
    tw.world().world_timeline().current().rate
}

pub(crate) fn set_rate(tw: &mut TestWorld, rate: u32) {
    if world_rate(tw) != rate {
        let out = run(tw, GOD, &format!("@tacsim/time rate={rate}"));
        assert_eq!(world_rate(tw), rate, "{out}");
    }
}

pub(crate) fn ic_now(tw: &TestWorld) -> i64 {
    tw.world().world_ic_now().at
}

/// Sweep to the first real second whose IC instant is at or past `target`; the events and
/// the submissions queued (not landed).
pub(crate) fn advance_to(
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
pub(crate) fn to_ic(rig: &mut Rig, target: i64) -> Vec<(Audience, OutputEvent)> {
    let (events, queued) = advance_to(rig, target);
    land_quietly(rig, &queued);
    events
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

/// A command and the submissions it queued.
pub(crate) fn command(rig: &mut Rig, who: ObjId, line: &str) -> (String, Vec<AuthoritativeSubmit>) {
    let out = run(&mut rig.tw, who, line);
    let queued = drain(&mut rig.tw);
    (out, queued)
}

/// File a passage with `line`, land its burn, launch it on the next fire; the ticket.
pub(crate) fn file_and_launch(rig: &mut Rig, pilot: ObjId, hull: ObjId, line: &str) -> Ticket {
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
pub(crate) fn lease_one(rig: &mut Rig, pilot: ObjId) {
    let counter = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, pilot, counter);
    let out = run(&mut rig.tw, pilot, "+tv/lease go");
    assert!(
        out.contains("Occator Water & Propellant leases you "),
        "{out}"
    );
}

/// The live lease records of `pilot`, by slot: their rigs.
pub(crate) fn rigs_of(tw: &TestWorld, pilot: ObjId) -> Vec<ObjId> {
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
pub(crate) fn on_pad_as(
    rig: &mut Rig,
    who: &str,
    seed: i64,
    name: &str,
    n_rigs: usize,
) -> (ObjId, HullRow, Vec<ObjId>) {
    on_pad_as_at(rig, (who, seed, name), ("ahuna", AHUNA), n_rigs)
}

/// **[`on_pad_as`] on any site** (doc 32 cut S.2): `(venue, entry)` the `+ship/undock` word and
/// the site's port entry.
pub(crate) fn on_pad_as_at(
    rig: &mut Rig,
    (who, seed, name): (&str, i64, &str),
    site: (&str, &str),
    n_rigs: usize,
) -> (ObjId, HullRow, Vec<ObjId>) {
    let (pilot, row, tvs) = on_pad_with_at(rig, (who, seed, name), site, |rig, pilot| {
        for _ in 0..n_rigs {
            lease_one(rig, pilot);
        }
    });
    assert_eq!(tvs.len(), n_rigs);
    (pilot, row, tvs)
}

/// **Stand at Occator's counter** in the Exchange Hall (`+tv/lease`'s room).
pub(crate) fn at_counter(rig: &mut Rig, pilot: ObjId) {
    let counter = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, pilot, counter);
}

/// **[`on_pad_as`] with the leases its pilot signs at Ceres given by `lease`** (doc 32 cut
/// CF.6c): `lease` runs after the charter, the refit and the licence, before the hull is loaded
/// and sails; the rigs are whatever it leased, by slot.
pub(crate) fn on_pad_with(
    rig: &mut Rig,
    who: &str,
    seed: i64,
    name: &str,
    lease: impl FnOnce(&mut Rig, ObjId),
) -> (ObjId, HullRow, Vec<ObjId>) {
    on_pad_with_at(rig, (who, seed, name), ("ahuna", AHUNA), lease)
}

/// **[`on_pad_with`] on any site** (doc 32 cut S.2): `(venue, entry)` the `+ship/undock` word
/// and the site's port entry.
pub(crate) fn on_pad_with_at(
    rig: &mut Rig,
    (who, seed, name): (&str, i64, &str),
    (venue, entry): (&str, &str),
    lease: impl FnOnce(&mut Rig, ObjId),
) -> (ObjId, HullRow, Vec<ObjId>) {
    let (pilot, row) = prospector(rig, who, seed, name);
    lease(rig, pilot);
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
    open_site(rig, entry);
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

/// **A Dray-P under way to Ahuna Flank's pad** (doc 36 cut GC.1): [`on_pad_as`] stopped at the
/// launch, so a test can open a field before it settles; the pilot, the hull's row, its one
/// leased rig and the ticket, whose plan's `depart_ic + total_s` is the landing.
pub(crate) fn en_route(
    rig: &mut Rig,
    who: &str,
    seed: i64,
    name: &str,
) -> (ObjId, HullRow, ObjId, Ticket) {
    let (pilot, row) = prospector(rig, who, seed, name);
    lease_one(rig, pilot);
    load(
        rig,
        &row,
        20,
        20,
        9_100 + u64::try_from(seed).unwrap_or(0) * 10,
    );
    board(rig, pilot);
    let hull = ObjId(row.hull);
    open_site(rig, AHUNA);
    let t = file_and_launch(rig, pilot, hull, "+ship/undock ahuna");
    let tvs = rigs_of(&rig.tw, pilot);
    assert_eq!(tvs.len(), 1);
    (pilot, row, tvs[0], t)
}

/// **Open a site by its port entry** (doc 20 cut SM.6, D2; `support::sites::open_site`), keeping
/// the landed bundles' typed post-images for a reboot; the port fires it took. A no-op once it is
/// open.
pub(crate) fn open_site(rig: &mut Rig, entry: &str) -> u32 {
    let mut kept = Vec::new();
    let fires = support::sites::open_site_keeping(&mut rig.tw, entry, &mut |ops| kept.push(ops));
    for ops in kept {
        keep(rig, ops);
    }
    fires
}

/// [`on_pad_as`] for the usual pilot.
pub(crate) fn on_pad(rig: &mut Rig, n_rigs: usize) -> (ObjId, HullRow, Vec<ObjId>) {
    on_pad_as(rig, "Pilot", 1, "Salt Moth", n_rigs)
}

/// Fire until Ahuna Flank's ground is cached — and, since doc 20 cut SM.4b, its home field built
/// (a scan or a sounding from the ground reserves the real trip home; the surface tick builds the
/// home the fire after the site).
pub(crate) fn warm(rig: &mut Rig) {
    // Doc 32 cut S.1: lane v17's seventh site is wanted too, and the tick warms one wanted
    // site a fire before it builds a home field, so the home can land a fire later.
    for _ in 0..16 {
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
pub(crate) fn cached(tw: &TestWorld) -> bool {
    tw.world()
        .module_state::<GeologyState>()
        .is_some_and(|state| !state.sites().is_empty())
}

pub(crate) fn parent(tw: &TestWorld, obj: ObjId) -> SpatialParent {
    tw.world()
        .module_state::<SpatialState>()
        .expect("the simulation")
        .parent_of(obj)
}

/// Ahuna Flank's passive domain.
pub(crate) fn ahuna_domain(tw: &TestWorld) -> mush_sim::DomainId {
    site_domain(tw, AHUNA)
}

/// A site's passive domain, by its port entry (doc 32 cut S.2).
pub(crate) fn site_domain(tw: &TestWorld, entry: &str) -> mush_sim::DomainId {
    mush_sim::DomainId(port_map(tw, entry).domain.expect("a domain"))
}

/// **A site's Dray-P stern ramp pose** (doc 32 cut S.2): [`RAMP`] off the site's own pad, which
/// its pad room puts at `(128, 128)` on every 256 site and `(256, 256)` on Urvara Reach.
pub(crate) fn ramp_at(tw: &TestWorld, entry: &str) -> (i64, i64, u32) {
    let (x, y) = tacsim::surface::excursion::site_pad_cm(tw.world(), &cid(entry));
    let pad = tacsim::surface::excursion::PAD_CENTRE_CM;
    (x + RAMP.0 - pad, y + RAMP.1 - pad, RAMP.2)
}

/// Whether `tv` stands `Direct` on Ahuna's domain at `(x, y, heading)`.
pub(crate) fn on_ground_at(tw: &TestWorld, tv: ObjId, at: (i64, i64, u32)) -> bool {
    matches!(parent(tw, tv), SpatialParent::Direct { domain, pose }
        if domain == ahuna_domain(tw)
            && (pose.x_cm, pose.y_cm, pose.heading_mdeg) == at)
}

/// A rig's typed record.
pub(crate) fn record_of(tw: &mut TestWorld, tv: ObjId) -> Option<RigRecord> {
    let h = handles(tw);
    typed(tw, &h.families.excursions, &RigKey(tv))
}

/// A principal's chart row for Ahuna Flank.
pub(crate) fn chart_of(tw: &mut TestWorld, who: ObjId) -> Option<ChartRecord> {
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
pub(crate) fn projection(tw: &TestWorld, tv: ObjId) -> Option<TvRec> {
    match store(tw).tv_rec(tv) {
        Some(TvRecState::Present(rec)) => Some(rec.clone()),
        Some(TvRecState::Unreadable) => panic!("the projection is unreadable"),
        None => None,
    }
}

/// The act in flight's due instant.
pub(crate) fn due_of(tw: &mut TestWorld, tv: ObjId) -> i64 {
    record_of(tw, tv)
        .and_then(|record| record.act)
        .map(|act| act.due_ic)
        .expect("an act in flight")
}

/// **`line` as `who`, and exactly one start queued**, landed; the reply.
pub(crate) fn act_go(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let (out, queued) = command(rig, who, line);
    assert_eq!(queued.len(), 1, "{line}: one start: {out}");
    assert_eq!(queued[0].actor, who, "the start is the operator's");
    assert_eq!(queued[0].action.key().id.as_str(), SURFACE_ACT_ACTION);
    land_all(rig, &queued);
    out
}

/// **Finish `tv`'s act**: to its due instant (the fire there queues the finish, landed), then
/// one more fire for the tell; what each of `who` heard over both, in order.
pub(crate) fn finish_heard(rig: &mut Rig, tv: ObjId, who: &[ObjId]) -> Vec<String> {
    finish_heard_with(rig, tv, who, false)
}

/// [`finish_heard`], the finish landed on the composed path when `composed` (doc 36a lane 4):
/// the production path, under which the finish's black box line is written.
pub(crate) fn finish_heard_with(
    rig: &mut Rig,
    tv: ObjId,
    who: &[ObjId],
    composed: bool,
) -> Vec<String> {
    let due = due_of(&mut rig.tw, tv);
    let (events, queued) = advance_to(rig, due);
    let mut heard: Vec<String> = who.iter().map(|w| said(&rig.tw, &events, *w)).collect();
    assert_eq!(
        finishes(&queued).len(),
        1,
        "one finish at the due instant: {queued:?}"
    );
    land_all_with(rig, &queued, composed);
    let (events, queued) = fire(rig);
    land_all_with(rig, &queued, composed);
    for (line, w) in heard.iter_mut().zip(who) {
        line.push('\n');
        line.push_str(&said(&rig.tw, &events, *w));
    }
    heard
}

/// [`finish_heard`] for one listener.
pub(crate) fn finish(rig: &mut Rig, tv: ObjId, who: ObjId) -> String {
    finish_heard(rig, tv, &[who]).remove(0)
}

/// [`finish`] with the finish landed on the composed path ([`land_composed`]): the act's black
/// box line is written, as production writes it.
pub(crate) fn finish_composed(rig: &mut Rig, tv: ObjId, who: ObjId) -> String {
    finish_heard_with(rig, tv, &[who], true).remove(0)
}

/// How many times `line` appears in `text`.
pub(crate) fn count(text: &str, line: &str) -> usize {
    text.matches(line).count()
}

/// Staff place `tv` `Direct` on Ahuna's domain at `(x, y)`, facing south.
pub(crate) fn staff_place(rig: &mut Rig, tv: ObjId, x: i64, y: i64) {
    let domain = ahuna_domain(&rig.tw);
    let staff = support::place_cm_keeping(&mut rig.tw, domain, tv, (x, y));
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    fold_module_ops_into(&mut rig.store, staff);
}

/// **Staff move `tv` to the centre of site cell `(x, y)` on `entry`'s ground** (doc 32 cut S.1:
/// [`to_cell`] for any site).
pub(crate) fn to_cell_on(rig: &mut Rig, entry: &str, tv: ObjId, (x, y): (i64, i64)) {
    let domain = mush_sim::DomainId(port_map(&rig.tw, entry).domain.expect("a domain"));
    let staff =
        support::place_cm_keeping(&mut rig.tw, domain, tv, (x * 1_000 + 500, y * 1_000 + 500));
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    fold_module_ops_into(&mut rig.store, staff);
}

/// **Staff's hand on the world** (doc 32 cut CF.2's fixtures): `f` runs with a `Ctx` as GOD and
/// every durable write it defers is folded into the store, so a reboot finds what it made.
pub(crate) fn staff_hand<T>(rig: &mut Rig, f: impl FnOnce(&mut mush_core::Ctx<'_>) -> T) -> T {
    let mut out = mush_core::Outbox::new();
    let answer = {
        let engine = rig.tw.engine_mut();
        let mut ctx = mush_core::Ctx {
            w: &mut engine.world,
            actor: GOD,
            switches: mush_core::SwitchSet::default(),
            args: mush_core::ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(SmolStr::new(tacsim::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        f(&mut ctx)
    };
    let ops: Vec<mush_core::ModuleOp> = out
        .take_deferred()
        .into_iter()
        .filter_map(|deferred| match deferred {
            mush_core::Deferred::Persist(op) => Some(op),
            _ => None,
        })
        .collect();
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    fold_module_ops_into(&mut rig.store, ops);
    answer
}

/// Staff move `tv` to the centre of site cell `(x, y)` (the placement seam).
pub(crate) fn to_cell(rig: &mut Rig, tv: ObjId, (x, y): (i64, i64)) {
    staff_place(rig, tv, x * 1_000 + 500, y * 1_000 + 500);
}

/// Write `record` into the family through a typed transaction (staff's hand).
pub(crate) fn put_record(rig: &mut Rig, record: RigRecord) {
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
pub(crate) fn put_cursor(rig: &mut Rig, cursor: PocketCursor) {
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
pub(crate) fn start(
    rig: &Rig,
    pilot: ObjId,
    tv: ObjId,
    act: &str,
    ordinal: u32,
) -> AuthoritativeSubmit {
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
pub(crate) fn launched(rig: &mut Rig, pilot: ObjId, tv: ObjId) {
    launched_at(rig, AHUNA, pilot, tv);
}

/// **[`launched`] with the launch's finish landed on the composed path** (doc 36a lane 4;
/// [`land_composed`], as production's world task lands it): the finish opens the rig's epoch and
/// writes its black box open line, so the epoch is recorded (`unrecorded` false) and a later
/// close line can be asserted. [`launched`]'s plain prepare hands the finish no owner
/// participants, so it writes no line and the epoch says `unrecorded`.
pub(crate) fn launched_composed(rig: &mut Rig, pilot: ObjId, tv: ObjId) {
    launched_at_with(rig, AHUNA, pilot, tv, true);
}

/// [`launched`] on any site (doc 32 cut S.2): rig 1 stands at the ramp off `entry`'s pad.
pub(crate) fn launched_at(rig: &mut Rig, entry: &str, pilot: ObjId, tv: ObjId) {
    launched_at_with(rig, entry, pilot, tv, false);
}

pub(crate) fn launched_at_with(
    rig: &mut Rig,
    entry: &str,
    pilot: ObjId,
    tv: ObjId,
    composed: bool,
) {
    let _ = act_go(rig, pilot, "+tv/1 launch go");
    let _ = if composed {
        finish_composed(rig, tv, pilot)
    } else {
        finish(rig, tv, pilot)
    };
    let (x, y, heading) = ramp_at(&rig.tw, entry);
    assert!(
        matches!(parent(&rig.tw, tv), SpatialParent::Direct { domain, pose }
            if domain == site_domain(&rig.tw, entry)
                && (pose.x_cm, pose.y_cm, pose.heading_mdeg) == (x, y, heading)),
        "#{} stands at {entry}'s ramp",
        tv.0
    );
}

// ---------------------------------------------------------------------------
// The ground, read the way a test may: the lane, the keyed geology, the cache
// ---------------------------------------------------------------------------

pub(crate) fn battery() -> BatteryDef {
    *tools_table()
        .expect("the tools table")
        .values()
        .find_map(ToolDef::battery)
        .expect("a battery row")
}

/// A lane site's frame (`sites.rs`'s `site_frame`).
pub(crate) fn site_frame(location: &str) -> mush_atlas::window::WindowFrame {
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
pub(crate) fn spec_of(tw: &TestWorld, entry: &str) -> (SiteGeologySpec, Arc<BodyGeology>, String) {
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
pub(crate) struct Ground {
    pub(crate) spec: SiteGeologySpec,
    pub(crate) body: Arc<BodyGeology>,
    pub(crate) ground: Arc<SiteGround>,
}

impl Ground {
    /// The ground surface of the composite.
    pub(crate) fn surface(&self) -> &mush_atlas::window::SurfaceComposite {
        self.ground
            .composite
            .surface(&mush_atlas::SurfaceRef::Ground)
            .expect("a ground surface")
    }

    /// **The truth of a site field**, computed independently of the render: `field_ground` at the
    /// body lattice field under the field's block's shift — and held to the site table the
    /// cache built.
    pub(crate) fn field(&self, (fx, fy): (i64, i64)) -> FieldGround {
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
    pub(crate) fn pocket(&self, (px, py): (i64, i64)) -> [tacsim::surface::Word; 4] {
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
pub(crate) fn ahuna(tw: &mut TestWorld) -> Ground {
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
pub(crate) fn put_charge(rig: &mut Rig, tv: ObjId, value_cu: u32, in_bay: bool) -> Charge {
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
pub(crate) fn pct(cu: u32) -> u32 {
    cu * 100 / battery().capacity_cu
}
/// Staff take every battery out of the one tools table the layer reads (`PortHandles`
/// `.surface_tools`): the rig's battery row no longer resolves (`excursions.rs`'s).
pub(crate) fn unbattery(rig: &mut Rig) {
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
pub(crate) fn rates(tw: &TestWorld) -> DriveRates {
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
pub(crate) fn table() -> &'static mush_atlas::physics::LocomotionTable {
    mush_atlas::physics::LocomotionTable::shipped()
}

/// The reserve floor: `capacity × reserve_permille / 1000` (5,000 cu on the Mk1).
pub(crate) fn floor() -> u32 {
    let b = battery();
    b.capacity_cu * b.reserve_permille / 1_000
}

/// Ahuna Flank's home field for the Dray-P's ramp cell, as the world built it.
pub(crate) fn home_field(tw: &TestWorld) -> Arc<HomeField> {
    home_field_at(tw, AHUNA)
}

/// A site's home field for the Dray-P's ramp cell.
pub(crate) fn home_field_at(tw: &TestWorld, entry: &str) -> Arc<HomeField> {
    tw.world()
        .module_state::<GeologyState>()
        .and_then(|state| state.home_field(&cid(entry), RAMP_CELL))
        .expect("the home field is built")
}

/// The leg the planner gives from `from_cm` to `to` over Ahuna Flank's cached composite.
pub(crate) fn expect_leg(rig: &mut Rig, from_cm: (i64, i64), to: (i64, i64)) -> Leg {
    let rates = rates(&rig.tw);
    let ground = ahuna(&mut rig.tw);
    drive_leg(ground.surface(), table(), from_cm, to, &rates).expect("a leg")
}

/// A cell's centre, centimetres.
pub(crate) fn centre((x, y): (i64, i64)) -> (i64, i64) {
    (x * 1_000 + 500, y * 1_000 + 500)
}

/// The act in flight.
pub(crate) fn act_of(tw: &mut TestWorld, tv: ObjId) -> tacsim::surface::Act {
    record_of(tw, tv)
        .and_then(|record| record.act)
        .expect("an act in flight")
}

/// The rig's charge value.
pub(crate) fn value_of(tw: &mut TestWorld, tv: ObjId) -> u32 {
    record_of(tw, tv)
        .map(|record| record.charge.value_cu)
        .expect("a record")
}

/// **The `site.map` frame `who` gets** through the feed's own query (the engine's query seam,
/// the typed snapshot): `args` the feed's params.
pub(crate) fn map_frame(rig: &mut Rig, who: ObjId, args: Value) -> Value {
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
pub(crate) fn frame_rows(frame: &Value) -> Vec<String> {
    let width = frame["width"].as_u64().expect("a width") as usize;
    let height = frame["height"].as_u64().expect("a height") as usize;
    let rows = frame["rows"].as_array().expect("rows");
    mush_ansi::panel::window_rows(rows, width, height, 0, 0, width, height)
        .into_iter()
        .map(|row| row.into_iter().collect())
        .collect()
}

/// The glyph at a frame cell.
pub(crate) fn glyph_at(rows: &[String], (x, y): (usize, usize)) -> Option<char> {
    rows.get(y).and_then(|row| row.chars().nth(x))
}

/// **A Dray-P on a site's pad** (`on_pad_as` for any site): the site opened, the hop, the warm.
pub(crate) fn on_pad_at(
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
pub(crate) fn booted() -> TestWorld {
    let (world, _) = mush_testkit::boot_world();
    let mut tw = boot_over(world, HashMap::new(), false, &rig_config());
    sweep(&mut tw, 120);
    tw
}

/// The lane's mining sites: every port entry with a `site` block, `(entry, name)`, entry order.
pub(crate) fn lane_sites() -> Vec<(CanonicalId, String)> {
    lane()
        .entries()
        .iter()
        // Doc 32 cut S.1: the 256 sites these censuses pin from the 256 ramp; the 512 site's own
        // census is doc 32 cut S.2's measurement.
        .filter(|(id, _)| id.as_str() != "urvara_port")
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
pub(crate) fn spread(mut values: Vec<usize>) -> (usize, usize, usize) {
    values.sort_unstable();
    let min = values.first().copied().unwrap_or(0);
    let max = values.last().copied().unwrap_or(0);
    let median = values.get(values.len() / 2).copied().unwrap_or(0);
    (min, median, max)
}
