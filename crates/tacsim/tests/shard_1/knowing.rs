//! **Doc 20 cut SM.4a acceptance — knowing** (doc 20 §1 beats 4–6, §5, §8, §10, §11; D-20-8,
//! D-20-11; Q-20-8, Q-20-9; the SM.4a brief's acceptance K1–K14 and its tests in 3.13).
//!
//! A First Captain whose Dray-P is settled on Ahuna Flank's pad works the knowing half of the
//! quarry loop from aboard: `+tv/1 scan` previews and `+tv/1 scan go` reads the nine fields
//! around the rig's own that the mast sees, sharpened by its footing; `+tv/1 sound [quiet] go`
//! reads every pocket in reach, a loud sounding heard across the site as a bearing and a band
//! once an IC hour; from the bay either act implies the launch. Every act **starts** as one
//! `tacsim_surface_act` submission and **finishes** on the `tacsim.surface` tick through
//! `tacsim_surface_done`, which writes the operator's `tacsim.chart` row — coverage and
//! provenance, never a reading; every reading is re-derived from the keyed geology through
//! reading rule v1 (`tacsim::surface::field_reading` / `pocket_reading`, the one render door).
//! `+tv/forget` strikes a site's or a body's readings by its own action; `+tv/quiet` keeps a
//! listener from hearing soundings.
//!
//! Composed on the composition root (`excursions.rs`'s rig, copied: the per-file precedent),
//! **booted keyed**: the geology key is handed in through `tacsim::surface::install_geology`
//! between composition and the module boot pass, on the first boot and on every reboot
//! (`sites.rs`'s `sm2_a_site_re_apply_keeps_the_site_cache` shape). Every boot asserts no
//! refused namespace write; every drain asserts no kept tacsim submission is GOD's. The world
//! runs at rate 1, so every IC instant a test names is the instant the world reads. Rigs are
//! put in a field's middle by the tick-batch placement seam a test may call directly
//! (`mush_sim::place`, `staff_place`), never a fake drive verb.

use crate::support;

use crate::geology_keyed;

// Passenger journeys share this production roster, typed commit/restart path,
// and stocked ship fixture; no separate large integration binary is needed.
#[path = "passengers.rs"]
mod passengers;

#[path = "quests.rs"]
mod quests;

#[path = "onboarding_quests.rs"]
mod onboarding_quests;

#[path = "onboarding_trade.rs"]
mod onboarding_trade;

#[path = "quest_followups.rs"]
mod quest_followups;

#[path = "quest_contacts.rs"]
mod quest_contacts;

#[path = "quest_passages.rs"]
mod quest_passages;

#[path = "quest_commissions.rs"]
mod quest_commissions;

use std::collections::{BTreeMap, BTreeSet, HashMap};
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
use smol_str::SmolStr;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK};
use tacsim::definitions::tools::{BatteryDef, ScannerDef, ToolDef, tools_table};
use tacsim::pack::{PackEntry, RulesPack, load_pack};
use tacsim::port::tv::{
    ACT_FAILED, BAND_FAR, BAND_MID, BAND_NEAR, CHART_FULL, CHART_UNREAD, FOOTING_ICE,
    FOOTING_ICE_NONE, FOOTING_REGOLITH, FOOTING_REGOLITH_NONE, FOOTING_ROCK, FORGET_NOTHING,
    FORGET_PREVIEW, FORGET_UNKNOWN, FORGOTTEN, GROUND_COLD, GROUND_COLD_LINE, GROUND_DARK_LINE,
    GROUND_LINE, HEARD, HEARD_HERE, HIDDEN_FIELDS, IMPLIED_LAUNCH, INSTRUMENTS_DARK,
    LAUNCH_FORFEIT, LEARN_NOTHING, NO_FOOTING, NO_READING, NO_READINGS_HERE, QUIET_OFF, QUIET_ON,
    READINGS_DARK, READINGS_GONE, SCAN_FILED, SCAN_FILED_LAUNCH, SCAN_PREVIEW, SCANNED,
    SCANNED_ICE, TWO_LEGS, WHY_MOVED, WHY_NEVER_LEFT,
};
use tacsim::port::voyage::TV_OUT;
use tacsim::port::{HullRow, PortHandles, PortMap, PortMapState, Ticket, TicketId, room_key};
use tacsim::surface::address::{
    field_index, field_name, field_of_cell, field_of_index, pocket_address, pocket_of_cell,
    pockets_in_reach,
};
use tacsim::surface::geology::{BodyGeology, FieldGround, field_ground};
use tacsim::surface::ground::{
    SiteGeologySpec, SiteGround, build_site_ground, capture_site, site_ground_sync,
};
use tacsim::surface::reading::{FieldReading, read_r1, read_r2};
use tacsim::surface::{
    ActKind, ActParams, ActResult, Charge, ChartKey, ChartProvenance, ChartRecord, Footing,
    GeologyBoot, GeologyState, Instrument, Octant, ReadSpan, RefusedCode, RigKey, RigRecord,
    SURFACE_ACT_ACTION, SURFACE_DONE_ACTION, TvRec, TvRecState, Via, better_footing, charge_at,
    field_reading, field_row_text, footing_at, pocket_reading, pocket_row_text, seen_fields,
    site_ground_cached, sound_adds,
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
    fn modules(self, passenger_v1: bool) -> Vec<Box<dyn mush_core::GameModule>> {
        use tacsim::composition as comp;

        let game = tacsim::GameEconomyConfig::default();
        let cfg = TacsimConfig {
            enabled: true,
            ..TacsimConfig::default()
        };
        let (mut mods, inventory) = comp::domain_modules_with_economy(
            EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &game,
        );
        mods.push(comp::npc_module(comp::npc_agent_adapter()));
        mods.push(comp::tacsim_test_world_module_wired(cfg, true, inventory));
        if passenger_v1 {
            let mut packs = comp::boot_pack_list(true, true).expect("the boot packs verify");
            // This old-binary fixture predates both passenger v2 and its later
            // follow-ups; unlike production it has no dependency-upgrade wait.
            packs.retain(|pack| pack.manifest().id().as_str() != tacsim::quest::packs::LEILA_PACK);
            let old =
                mush_content::load_pack(include_str!("../../content/passengers/stickney_v1.json"))
                    .expect("the immutable passenger v1 pack verifies");
            assert_eq!(old.manifest().version(), 1);
            let current = packs
                .iter_mut()
                .find(|pack| pack.manifest().id() == old.manifest().id())
                .expect("the passenger pack is composed");
            *current = old;
            let mut appliers = mush_content::Appliers::new();
            mush_atlas::register_appliers(&mut appliers).unwrap();
            mush_npc::content::register_appliers(&mut appliers).unwrap();
            tacsim::register_appliers(&mut appliers).unwrap();
            mods.push(Box::new(
                mush_content::ContentModule::new()
                    .with_appliers(appliers)
                    .with_boot_packs(packs),
            ));
        } else {
            mods.push(comp::content_module_with(true, true).expect("the content module composes"));
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
    boot_over_with_passenger(world, store, keyed, false)
}

fn boot_over_with_passenger(
    world: World,
    store: HashMap<SmolStr, mush_core::ModuleBootData>,
    keyed: bool,
    passenger_v1: bool,
) -> TestWorld {
    let (_, registry) = mush_testkit::boot_world();
    let mut engine = mush_core::Engine::with_modules(world, registry, Roster.modules(passenger_v1))
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
    stocked_with_passenger(keyed, false)
}

fn stocked_with_passenger(keyed: bool, passenger_v1: bool) -> Rig {
    let (world, _) = mush_testkit::boot_world();
    let mut tw = boot_over_with_passenger(world, HashMap::new(), keyed, passenger_v1);
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

/// Keep the guards when a regression deliberately holds resolution across a
/// world change, as the production driver's admission checks do.
struct ResolvedWithGuards {
    action: mush_core::DefinitionRef,
    resolved: mush_core::ResolvedActionInvocation,
    guards: mush_core::GuardSet,
    token: [u8; 16],
}

fn resolve_with_guards(
    rig: &Rig,
    submit: &AuthoritativeSubmit,
) -> Result<ResolvedWithGuards, String> {
    let token = next_token();
    let tw = &rig.tw;
    let (action, resolved, guards) = {
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
        resolved.into_authoritative_parts()
    };
    Ok(ResolvedWithGuards {
        action,
        resolved,
        guards,
        token,
    })
}

/// Drive one queued submission, keeping its typed post-images for a reboot.
fn land(rig: &mut Rig, submit: &AuthoritativeSubmit) -> Result<(), String> {
    let ResolvedWithGuards {
        action,
        resolved,
        guards: _,
        token,
    } = resolve_with_guards(rig, submit)?;
    let tw = &mut rig.tw;
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

/// The Mk1 scanner and battery rows of the one tools table.
fn scanner() -> ScannerDef {
    *tools_table()
        .expect("the tools table")
        .values()
        .find_map(ToolDef::scanner)
        .expect("a scanner row")
}

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

    /// The expected text of a covered field at `footing`.
    fn field_text(&self, field: (i64, i64), footing: Footing) -> String {
        let name = field_name(field.0, field.1).expect("a name");
        let truth = self.field(field);
        field_row_text(
            &name,
            &read_r1(&mac(KEY_1), &self.body, &truth, footing),
            footing,
        )
    }
}

/// Ahuna Flank's ground (a cache hit once warm).
fn ahuna(tw: &mut TestWorld) -> Ground {
    let (spec, body, location) = spec_of(tw, AHUNA);
    let ground =
        site_ground_sync(tw.world_mut(), &site_frame(&location), &spec).expect("the site builds");
    Ground { spec, body, ground }
}

/// The first field middle (rows from the north) whose footing is `want`, away from the site's
/// edge and outside `avoid` fields' reach.
fn middle_with(g: &Ground, want: Footing, avoid: &[(i64, i64)]) -> (i64, i64) {
    for fy in (1..7).rev() {
        for fx in 1..7 {
            let far = avoid
                .iter()
                .all(|(ax, ay)| (fx - ax).abs() >= 3 || (fy - ay).abs() >= 3);
            let cell = (fx * 32 + 16, fy * 32 + 16);
            if far && footing_at(g.surface(), cell) == Ok(want) {
                return cell;
            }
        }
    }
    panic!("no field middle of Ahuna Flank stands on {want:?}");
}

/// The preview's footing words (§3.8), from the composite.
fn footing_words(g: &Ground, cell: (i64, i64), footing: Footing) -> String {
    let radius = i64::from(scanner().scan_radius_cells);
    let better = better_footing(g.surface(), cell, footing, radius);
    let r = (radius * 10).to_string();
    match (footing, better) {
        (Footing::Rock, _) => FOOTING_ROCK.to_owned(),
        (Footing::Regolith, Some((_, m, dir))) => FOOTING_REGOLITH
            .replace("{m}", &m.to_string())
            .replace("{dir}", dir.abbrev()),
        (Footing::Regolith, None) => FOOTING_REGOLITH_NONE.replace("{r}", &r),
        (Footing::Ice, Some((sharper, m, dir))) => FOOTING_ICE
            .replace("{better}", sharper.name())
            .replace("{m}", &m.to_string())
            .replace("{dir}", dir.abbrev()),
        (Footing::Ice, None) => FOOTING_ICE_NONE.replace("{r}", &r),
    }
}

/// The status's ground line for a covered or uncovered stand, built from the parts.
fn ground_line(block: &str, field: &str, footing: &str, reading: &str) -> String {
    GROUND_LINE
        .replace("{block}", block)
        .replace("{tenure}", "open")
        .replace("{field}", field)
        .replace("{footing}", footing)
        .replace("{reading}", reading)
}

/// The block letter, pocket address and field name of a site cell.
fn names(cell: (i64, i64)) -> (String, String, String) {
    let (px, py) = pocket_of_cell(cell).expect("on the site");
    let (fx, fy) = field_of_cell(cell).expect("on the site");
    let addr = pocket_address(px, py).expect("an address");
    (
        addr.chars().next().expect("a letter").to_string(),
        addr,
        field_name(fx, fy).expect("a name"),
    )
}

// ---------------------------------------------------------------------------
// K13. The implied launch
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

/// **K13 — the implied launch** (lead ruling 2; the brief's reviewer finding F1, D22): rig 1 in
/// the bay with its charge put at 20,000 cu, so the bay's recharge shows. The bare scan
/// previews exactly `SCAN_PREVIEW` with `IMPLIED_LAUNCH` and `TWO_LEGS`, the ramp cell's field
/// and footing and the charge after the launch half; `go` queues one start; landed, the act is
/// `Scan { from_bay, ramp cell, .. }` due 900 IC s on with 1,000 cu reserved and the excursion
/// open from the start; at the due instant one fire places the rig at the ramp pose and
/// submits the finish; landed `Done`, the charge is `charge_at(c0, start + 300) − 1,000` out
/// of the bay — strictly less than `charge_at(c0, start + 900) − 1,000` — the chart covers
/// exactly the seen fields at the ramp's footing, the pilot hears the scan's rows and `+tv`
/// shows the ground line with the field's row. A twin world launching and then scanning from
/// the ramp leaves the same charge. A sounding from the bay settles `charge_at(c0, start + 300)
/// − 2,000` likewise; a forfeit lease refuses `LAUNCH_FORFEIT` and writes nothing.
#[test]
#[allow(clippy::too_many_lines)]
fn sm4a_an_implied_launch_scans_from_the_ramp() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let g = ahuna(&mut rig.tw);
    let c0 = put_charge(&mut rig, tv, 20_000, true);
    let start_ic = ic_now(&rig.tw);
    let footing = footing_at(g.surface(), RAMP_CELL).expect("the ramp cell gives a footing");
    let (seen, hidden) = seen_fields(g.surface(), RAMP_CELL, &scanner());
    let launch_end = charge_at(&c0, start_ic + 300, &battery());
    let hidden_words = if hidden.is_empty() {
        String::new()
    } else {
        HIDDEN_FIELDS.replace("{k}", &hidden.len().to_string())
    };
    let preview = SCAN_PREVIEW
        .replace("{n}", "1")
        .replace("{mark}", "OWP-TV-01")
        .replace(
            "{launch}",
            &IMPLIED_LAUNCH
                .replace("{ramp}", "stern ramp")
                .replace("{site}", "Ahuna Flank"),
        )
        .replace("{count}", &seen.len().to_string())
        .replace("{field}", "K-NW")
        .replace("{footing}", &footing_words(&g, RAMP_CELL, footing))
        .replace("{hidden}", &hidden_words)
        .replace(
            "{time}",
            &TWO_LEGS.replace("{down}", "300").replace("{secs}", "600"),
        )
        .replace("{real}", ", about 15 min")
        .replace("{now}", &pct(20_000).to_string())
        .replace("{after}", &pct(launch_end - 1_000).to_string());
    let (out, queued) = command(&mut rig, pilot, "+tv/1 scan");
    assert_eq!(out.trim(), preview);
    assert!(queued.is_empty());
    let out = act_go(&mut rig, pilot, "+tv/1 scan go");
    let filed = SCAN_FILED_LAUNCH
        .replace("{n}", "1")
        .replace("{ramp}", "stern ramp");
    assert!(
        out.starts_with(filed.split("{when}").next().unwrap_or_default()),
        "{out}"
    );
    let record = record_of(&mut rig.tw, tv).expect("the record");
    let act = record.act.clone().expect("the act");
    assert_eq!(act.kind, ActKind::Scan);
    assert_eq!(
        act.params,
        ActParams::Scan {
            from_bay: true,
            body: cid("ceres"),
            cell: [RAMP_CELL.0, RAMP_CELL.1],
            footing,
            fields: seen.clone(),
            hidden: hidden.clone(),
        }
    );
    assert_eq!(act.started_ic, start_ic);
    assert_eq!(act.due_ic, start_ic + 900);
    assert_eq!(act.reserved_cu, 1_000);
    assert_eq!(
        record.excursion.as_ref().map(|e| e.id.launch_ic),
        Some(start_ic)
    );
    // At the due instant: the placement at the ramp, then one finish.
    let (_, queued) = advance_to(&mut rig, act.due_ic);
    assert!(on_ground_at(&rig.tw, tv, RAMP), "{:?}", parent(&rig.tw, tv));
    assert_eq!(finishes(&queued).len(), 1);
    land_all(&mut rig, &queued);
    let (events, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    let heard = said(&rig.tw, &events, pilot);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(
        record.last.as_ref().map(|l| (l.kind, l.result)),
        Some((ActKind::Scan, ActResult::Done))
    );
    assert_eq!(
        record.charge,
        Charge {
            value_cu: launch_end - 1_000,
            at_ic: act.due_ic,
            in_bay: false
        },
        "the bay credits only the launch half"
    );
    assert!(record.charge.value_cu < charge_at(&c0, start_ic + 900, &battery()) - 1_000);
    let row = chart_of(&mut rig.tw, pilot).expect("the chart row");
    assert_eq!(
        row.fields.keys().copied().collect::<BTreeSet<u8>>(),
        seen.iter().copied().collect::<BTreeSet<u8>>()
    );
    assert!(row.fields.values().all(|f| f.footing == footing));
    assert_eq!(row.scan_stand, Some([RAMP_CELL.0, RAMP_CELL.1]));
    if footing == Footing::Ice {
        assert!(
            heard.contains(
                &SCANNED_ICE
                    .replace("{n}", "1")
                    .replace("{count}", &seen.len().to_string())
            )
        );
    } else {
        let head = SCANNED
            .replace("{n}", "1")
            .replace("{count}", &seen.len().to_string())
            .replace("{footing}", footing.name())
            .replace("{hidden}", &hidden_words);
        assert!(heard.contains(&head), "{heard}");
        for index in &seen {
            let field = field_of_index(*index).expect("a field");
            assert!(
                heard.contains(&format!("  {}", g.field_text(field, footing))),
                "{heard}"
            );
        }
    }
    let status = run(&mut rig.tw, pilot, "+tv");
    let reading = field_reading(rig.tw.world(), &cid(AHUNA), Some(&row), (4, 3));
    assert_eq!(reading, g.field_text((4, 3), footing));
    assert!(
        status.contains(&ground_line("K", "K-NW", footing.name(), &reading)),
        "{status}"
    );
    // The twin: `+tv/1 launch go`, its finish, then a scan from the ramp cell.
    let mut twin = stocked();
    let (twin_pilot, _, twin_tvs) = on_pad(&mut twin, 1);
    let twin_tv = twin_tvs[0];
    assert_eq!(ic_now(&twin.tw), start_ic, "the twin is built alike");
    let _ = put_charge(&mut twin, twin_tv, 20_000, true);
    launched(&mut twin, twin_pilot, twin_tv);
    let _ = act_go(&mut twin, twin_pilot, "+tv/1 scan go");
    let _ = finish(&mut twin, twin_tv, twin_pilot);
    assert_eq!(
        record_of(&mut twin.tw, twin_tv).map(|r| r.charge.value_cu),
        Some(record.charge.value_cu),
        "an implied launch leaves exactly what a launch then a scan leaves"
    );
    // A sounding from the bay settles likewise: back up the ramp first.
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let _ = finish(&mut rig, tv, pilot);
    let c1 = put_charge(&mut rig, tv, 20_000, true);
    let sound_ic = ic_now(&rig.tw);
    let _ = act_go(&mut rig, pilot, "+tv/1 sound go");
    assert_eq!(due_of(&mut rig.tw, tv), sound_ic + 300 + 1_200);
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(
        record_of(&mut rig.tw, tv).map(|r| r.charge),
        Some(Charge {
            value_cu: charge_at(&c1, sound_ic + 300, &battery()) - 2_000,
            at_ic: sound_ic + 1_500,
            in_bay: false
        })
    );
    // A forfeit lease refuses the implied launch's gate and writes nothing.
    let mut forfeit = stocked();
    let (f_pilot, _, f_tvs) = on_pad(&mut forfeit, 1);
    let lease = store(&forfeit.tw)
        .tv_leases_of(f_pilot)
        .next()
        .map(|record| record.lease)
        .expect("a lease");
    let key = tacsim::port::tv::tv_lease_key(lease);
    let mut forfeit = reboot_editing(forfeit, key.as_str(), &[f_pilot], |doc| {
        doc["status"] = serde_json::json!("forfeit");
    });
    let before = record_of(&mut forfeit.tw, f_tvs[0]);
    for line in ["+tv/1 scan", "+tv/1 scan go"] {
        let (out, queued) = command(&mut forfeit, f_pilot, line);
        let head = LAUNCH_FORFEIT.replace("{n}", "1");
        assert!(
            out.trim()
                .starts_with(head.split("{counter}").next().unwrap_or_default()),
            "{line}: {out}"
        );
        assert!(queued.is_empty(), "{line}");
    }
    assert_eq!(record_of(&mut forfeit.tw, f_tvs[0]), before);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// K1, K2, K9. The readings are the quantiser's, the viewer's own, and honest
// ---------------------------------------------------------------------------

/// **K1 — readings equal the quantiser for covered cells only** (lead rulings 3 and 4): after
/// a regolith scan, a rock scan (nine other fields) and a sounding, every one of Ahuna Flank's
/// 64 fields renders exactly `read_r1`'s row at the row's footing where the chart covers it and
/// `NO_READING` where it does not, and every one of its 4,096 pockets exactly `read_r2`'s text
/// where sounded and `NO_READING` where not — the truth read independently through
/// `field_ground` and held to the site table the cache built.
#[test]
fn sm4a_readings_equal_the_quantiser_for_covered_cells_only() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let g = ahuna(&mut rig.tw);
    let regolith = middle_with(&g, Footing::Regolith, &[]);
    let rock = middle_with(
        &g,
        Footing::Rock,
        &[field_of_cell(regolith).expect("a field")],
    );
    for (stand, act) in [
        (regolith, "+tv/1 scan go"),
        (rock, "+tv/1 scan go"),
        (rock, "+tv/1 sound go"),
    ] {
        to_cell(&mut rig, tv, stand);
        let _ = act_go(&mut rig, pilot, act);
        let _ = finish(&mut rig, tv, pilot);
    }
    let row = chart_of(&mut rig.tw, pilot).expect("the chart row");
    let footings: BTreeSet<&str> = row.fields.values().map(|f| f.footing.name()).collect();
    assert!(
        footings.contains("regolith") && footings.contains("rock"),
        "{footings:?}"
    );
    let site = cid(AHUNA);
    let w: &World = rig.tw.world();
    let mut covered = 0;
    for index in 0..64_u8 {
        let field = field_of_index(index).expect("a field");
        let text = field_reading(w, &site, Some(&row), field);
        match row.fields.get(&index) {
            Some(known) => {
                covered += 1;
                assert_eq!(text, g.field_text(field, known.footing), "{field:?}");
            }
            None => assert_eq!(text, NO_READING, "{field:?}"),
        }
    }
    assert_eq!(covered, row.fields.len());
    assert!(covered >= 2, "both scans covered fields");
    let mut sounded = 0;
    for py in 0..64 {
        for px in 0..64 {
            let text = pocket_reading(w, &site, Some(&row), (px, py));
            if row.pocket_sounded((px, py)) {
                sounded += 1;
                let addr = pocket_address(px, py).expect("an address");
                assert_eq!(text, pocket_row_text(&addr, &g.pocket((px, py))));
            } else {
                assert_eq!(text, NO_READING, "({px}, {py})");
            }
        }
    }
    assert_eq!(sounded, 64);
    no_refused_writes(&rig.tw);
}

/// **K9 — every band a regolith scan prints holds the truth** (§5): from a regolith stand,
/// every field the scan covered reads a two-word band per family whose low or high word is
/// the field's true word, and the printed row is exactly those bands.
#[test]
fn sm4a_a_regolith_scan_prints_bands_that_hold_the_truth() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let g = ahuna(&mut rig.tw);
    let stand = middle_with(&g, Footing::Regolith, &[]);
    to_cell(&mut rig, tv, stand);
    let _ = act_go(&mut rig, pilot, "+tv/1 scan go");
    let heard = finish(&mut rig, tv, pilot);
    let row = chart_of(&mut rig.tw, pilot).expect("the chart row");
    assert!(!row.fields.is_empty());
    for (index, known) in &row.fields {
        assert_eq!(known.footing, Footing::Regolith);
        let field = field_of_index(*index).expect("a field");
        let truth = g.field(field);
        let readings = read_r1(&mac(KEY_1), &g.body, &truth, Footing::Regolith);
        for (reading, word) in readings.iter().zip(truth.words) {
            match reading {
                FieldReading::Band(low, high) => {
                    assert!(
                        *low == word || *high == word,
                        "{field:?}: {reading:?} {word:?}"
                    );
                }
                other => panic!("a regolith reading is a band: {other:?}"),
            }
        }
        let text = field_row_text(
            &field_name(field.0, field.1).expect("a name"),
            &readings,
            Footing::Regolith,
        );
        assert!(heard.contains(&format!("  {text}")), "{heard}");
    }
    no_refused_writes(&rig.tw);
}

/// A `+tv` reply with its two time-varying spans — the rig line's lease span and the stores
/// line's span to short rations — replaced by `{span}`, so two readings taken at different
/// instants compare byte for byte on everything else (the SM.4a fixer round's findings F3 and
/// conformance F2: the whole reply, never one line of it).
fn timeless(status: &str) -> String {
    status
        .lines()
        .map(|line| {
            let mut line = line.to_owned();
            for (open, close) in [
                ("leased from Occator for ", " more;"),
                ("crew-days: ", " here before"),
            ] {
                if let Some(at) = line.find(open) {
                    let from = at + open.len();
                    if let Some(len) = line[from..].find(close) {
                        line.replace_range(from..from + len, "{span}");
                    }
                }
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// **K2 — a second principal reads nothing of the first's** (§5 "Fog, by construction"): two
/// Dray-Ps on Ahuna's pad, both rigs on the ground, the second's standing at (130, 118) —
/// inside the field the first scans and the pocket square the first sounds from the ramp.
/// After the first pilot's scan and sounding, the second has no chart row and their whole
/// `+tv` is byte-identical before and after (the time-varying spans aside: `timeless`), the
/// hint still on their ground line, while the first standing on that same cell reads the
/// pocket there; every reading rendered for the second — an unknown site, every field the
/// first covered, and a barren pocket the first sounded — is `NO_READING`, byte for byte, while
/// the first reads that barren pocket as barren; and the first's own row, offered for any other
/// lane site, renders `NO_READING` there (the row-to-site filter, shown not to be a dark site by
/// the same row relabelled for a Ceres site reading there).
#[test]
#[allow(clippy::too_many_lines)]
fn sm4a_a_second_principal_reads_nothing_of_the_first() {
    let mut rig = stocked();
    let (first, _, tvs) = on_pad(&mut rig, 1);
    let (second, _, others) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    launched(&mut rig, first, tvs[0]);
    launched(&mut rig, second, others[0]);
    let inside = (130, 118);
    to_cell(&mut rig, others[0], inside);
    let before = timeless(&run(&mut rig.tw, second, "+tv"));
    assert!(before.contains(&format!("{NO_READINGS_HERE}.")), "{before}");
    let _ = act_go(&mut rig, first, "+tv/1 scan go");
    let _ = finish(&mut rig, tvs[0], first);
    let _ = act_go(&mut rig, first, "+tv/1 sound go");
    let _ = finish(&mut rig, tvs[0], first);
    assert_eq!(timeless(&run(&mut rig.tw, second, "+tv")), before);
    assert_eq!(chart_of(&mut rig.tw, second), None);
    let row = chart_of(&mut rig.tw, first).expect("the first's row");
    // The second stands inside the first's coverage: a leak would show on their ground line.
    let (fx, fy) = field_of_cell(inside).expect("a field");
    let pocket = pocket_of_cell(inside).expect("a pocket");
    assert!(
        row.fields
            .contains_key(&field_index(fx, fy).expect("an index"))
    );
    assert!(row.pocket_sounded(pocket));
    let g = ahuna(&mut rig.tw);
    let (_, addr_inside, _) = names(inside);
    let text_inside = pocket_row_text(&addr_inside, &g.pocket(pocket));
    assert!(!before.contains(&text_inside), "{before}");
    to_cell(&mut rig, tvs[0], inside);
    let first_status = run(&mut rig.tw, first, "+tv");
    assert!(
        first_status.contains(&format!("{text_inside}.")),
        "the first reads the pocket there: {first_status}"
    );
    assert_eq!(timeless(&run(&mut rig.tw, second, "+tv")), before);
    let w: &World = rig.tw.world();
    let site = cid(AHUNA);
    assert_eq!(
        field_reading(w, &cid("nowhere_port"), None, (0, 0)),
        NO_READING
    );
    for index in row.fields.keys() {
        let field = field_of_index(*index).expect("a field");
        assert_eq!(field_reading(w, &site, None, field), NO_READING);
        assert_ne!(field_reading(w, &site, Some(&row), field), NO_READING);
    }
    let barren = pockets_in_reach(RAMP_CELL, 16)
        .into_iter()
        .find(|pocket| g.pocket(*pocket) == [tacsim::surface::Word::Barren; 4])
        .expect("a barren pocket the first sounded");
    assert_eq!(pocket_reading(w, &site, None, barren), NO_READING);
    let addr = pocket_address(barren.0, barren.1).expect("an address");
    assert_eq!(
        pocket_reading(w, &site, Some(&row), barren),
        format!("{addr} barren")
    );
    // The first's Ahuna row offered for every other lane site reads nothing there.
    let pack = lane();
    let mut resolving = 0;
    for (entry, def) in pack.entries() {
        let PackEntry::Port(port) = def else {
            continue;
        };
        if port.site.is_none() || entry.as_str() == AHUNA {
            continue;
        }
        let relabelled = ChartRecord {
            site: entry.clone(),
            ..row.clone()
        };
        for index in row.fields.keys() {
            let field = field_of_index(*index).expect("a field");
            assert_eq!(
                field_reading(w, entry, Some(&row), field),
                NO_READING,
                "{entry}"
            );
            if field_reading(w, entry, Some(&relabelled), field) != NO_READING {
                resolving += 1;
            }
        }
        assert_eq!(
            pocket_reading(w, entry, Some(&row), barren),
            NO_READING,
            "{entry}"
        );
    }
    assert!(
        resolving > 0,
        "some other lane site resolves, so the filter (not a dark site) answered"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// K3. A repeat from the same cell
// ---------------------------------------------------------------------------

/// **K3 — a repeat is refused at no cost** (§5, literally; D4; the brief's reviewer finding
/// F2): a scan from a field's local (16, 16), then `+tv/1 scan` bare and with `go` answer
/// `LEARN_NOTHING`, queue nothing and change neither the record nor the chart; a forged start
/// is refused "you learn nothing new here"; a quiet sounding there, then a loud one (it adds
/// pockets) runs, then a quiet one is refused; and from local (17, 16) — whose sounding reach is
/// the same 64 pockets — a loud sounding still runs although it adds nothing: the charge falls
/// by 2,000, the block's provenance gains `Local { Sound, at: due, Hand }`, its `last_at`
/// moves, and `sound_stand` is the new cell.
#[test]
#[allow(clippy::too_many_lines)]
fn sm4a_a_repeat_is_refused_at_no_cost() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let here = (128 + 16, 96 + 16);
    to_cell(&mut rig, tv, here);
    let _ = act_go(&mut rig, pilot, "+tv/1 scan go");
    let _ = finish(&mut rig, tv, pilot);
    let unchanged = |rig: &mut Rig| (record_of(&mut rig.tw, tv), chart_of(&mut rig.tw, pilot));
    let before = unchanged(&mut rig);
    for line in ["+tv/1 scan", "+tv/1 scan go"] {
        let (out, queued) = command(&mut rig, pilot, line);
        assert_eq!(out.trim(), LEARN_NOTHING, "{line}");
        assert!(queued.is_empty(), "{line}");
    }
    let n = before.0.as_ref().map(|r| r.ordinal).expect("an ordinal");
    let forged = start(&rig, pilot, tv, "scan", n + 1);
    let refused = land(&mut rig, &forged).expect_err("the forged start refuses");
    assert!(refused.contains("you learn nothing new here"), "{refused}");
    assert_eq!(unchanged(&mut rig), before, "no cost, no write");
    // Quiet, then loud (it adds), then quiet again (it does not).
    let _ = act_go(&mut rig, pilot, "+tv/1 sound quiet go");
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(
        chart_of(&mut rig.tw, pilot).map(|r| r.sounded_pockets()),
        Some(16)
    );
    let _ = act_go(&mut rig, pilot, "+tv/1 sound go");
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(
        chart_of(&mut rig.tw, pilot).map(|r| r.sounded_pockets()),
        Some(64)
    );
    let (out, queued) = command(&mut rig, pilot, "+tv/1 sound quiet go");
    assert_eq!(out.trim(), LEARN_NOTHING);
    assert!(queued.is_empty());
    // The neighbouring cell: the same 64 pockets, yet it runs.
    let next = (here.0 + 1, here.1);
    let row = chart_of(&mut rig.tw, pilot).expect("the row");
    assert!(!sound_adds(Some(&row), &pockets_in_reach(next, 16)));
    to_cell(&mut rig, tv, next);
    let charge = record_of(&mut rig.tw, tv)
        .map(|r| r.charge.value_cu)
        .expect("a charge");
    let _ = act_go(&mut rig, pilot, "+tv/1 sound go");
    let due = due_of(&mut rig.tw, tv);
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(
        record_of(&mut rig.tw, tv).map(|r| r.charge.value_cu),
        Some(charge - 2_000)
    );
    let after = chart_of(&mut rig.tw, pilot).expect("the row");
    assert_eq!(after.sound_stand, Some([next.0, next.1]));
    assert_eq!(after.sounded_pockets(), 64, "it added nothing");
    let (block, _, _) = names(next);
    let kept = after.blocks.get(&block).expect("the block");
    assert_eq!(kept.last_at, due);
    assert_eq!(
        kept.provenance.last(),
        Some(&ChartProvenance::Local {
            instrument: Instrument::Sound,
            at: due,
            via: Via::Hand
        })
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// K4, K5, K6. The heard sounding
// ---------------------------------------------------------------------------

/// Two Dray-Ps on Ahuna's pad, both rigs launched: the sounder's and the listener's.
fn two_on_pad(rig: &mut Rig) -> (ObjId, ObjId, ObjId, ObjId) {
    let (sounder, _, tvs) = on_pad(rig, 1);
    let (listener, _, others) = on_pad_as(rig, "Other", 2, "Tally Moth", 1);
    launched(rig, sounder, tvs[0]);
    launched(rig, listener, others[0]);
    (sounder, tvs[0], listener, others[0])
}

/// The heard line a listener at `at` hears of a sounding from `from` (D19).
fn heard_line(from: (i64, i64), at: (i64, i64)) -> String {
    let Some(octant) = Octant::of(from.0 - at.0, from.1 - at.1) else {
        return HEARD_HERE.to_owned();
    };
    let cells = (from.0 - at.0).abs().max((from.1 - at.1).abs());
    let band = if cells < 50 {
        BAND_NEAR
    } else if cells <= 100 {
        BAND_MID
    } else {
        BAND_FAR
    };
    HEARD
        .replace("{bearing}", octant.word())
        .replace("{band}", band)
}

/// How many heard lines `text` holds, of any bearing.
fn heard_count(text: &str) -> usize {
    text.lines()
        .filter(|line| line.starts_with("A sounding thumps through the ground"))
        .count()
}

/// **K4 — a sounding reaches another party as a bearing and a band and nothing more** (§5,
/// Q-20-9; D8, D19): the listener's rig stands at (60, 200), the sounder sounds from (170, 90);
/// the listener hears exactly `HEARD` with the octant and band the two positions give, once,
/// and nothing of the readings; their chart row stays absent, their projection unchanged and
/// their whole `+tv` byte-identical (the time-varying spans aside: `timeless`); the sounder's
/// projection gains exactly the act's own log line (the sounding's head) and `told`, nothing
/// heard.
#[test]
#[allow(clippy::too_many_lines)]
fn sm4a_a_sounding_is_heard_as_a_bearing_and_a_band() {
    let mut rig = stocked();
    let (sounder, tv, listener, other) = two_on_pad(&mut rig);
    to_cell(&mut rig, other, (60, 200));
    to_cell(&mut rig, tv, (170, 90));
    let projection_before = projection(&rig.tw, other);
    let status_before = timeless(&run(&mut rig.tw, listener, "+tv"));
    let sounder_before = projection(&rig.tw, tv).expect("the sounder's projection");
    let _ = act_go(&mut rig, sounder, "+tv/1 sound go");
    let ordinal = record_of(&mut rig.tw, tv)
        .and_then(|r| r.act)
        .map(|a| a.ordinal)
        .expect("the act");
    let heard = finish_heard(&mut rig, tv, &[listener, sounder]);
    let expected = heard_line((170, 90), (60, 200));
    assert_eq!(
        expected,
        HEARD
            .replace("{bearing}", "southeast")
            .replace("{band}", BAND_FAR)
    );
    assert_eq!(count(&heard[0], &expected), 1, "{}", heard[0]);
    assert_eq!(heard_count(&heard[0]), 1);
    assert!(
        !heard[0].contains("sounded") && !heard[0].contains("metals"),
        "{}",
        heard[0]
    );
    assert_eq!(heard_count(&heard[1]), 0, "the sounder hears no heard line");
    assert_eq!(chart_of(&mut rig.tw, listener), None);
    assert_eq!(projection(&rig.tw, other), projection_before);
    assert_eq!(timeless(&run(&mut rig.tw, listener, "+tv")), status_before);
    let sounder_after = projection(&rig.tw, tv).expect("the sounder's projection");
    assert_eq!(sounder_after.told, ordinal);
    assert_eq!(sounder_after.act, None);
    assert_eq!(
        sounder_after.log.len(),
        sounder_before.log.len() + 1,
        "{:?}",
        sounder_after.log
    );
    assert_eq!(
        sounder_after.log[..sounder_before.log.len()],
        sounder_before.log[..]
    );
    let logged = &sounder_after.log[sounder_before.log.len()].text;
    assert!(logged.starts_with("Rig 1 sounded "), "{logged}");
    assert_eq!(heard_count(logged), 0);
    let record = record_of(&mut rig.tw, tv).expect("the sounder's record");
    assert!(matches!(
        record.last.and_then(|last| last.span),
        Some(ReadSpan::Sound {
            heard: true,
            quiet: false,
            ..
        })
    ));
    no_refused_writes(&rig.tw);
}

/// **K5 — a quiet sounding reaches nobody**: the same two parties; `+tv/1 sound quiet go`
/// reads 16 pockets and the listener hears no line; the sounder's budget is untouched.
#[test]
fn sm4a_a_quiet_sounding_reaches_nobody() {
    let mut rig = stocked();
    let (sounder, tv, listener, _) = two_on_pad(&mut rig);
    to_cell(&mut rig, tv, (170, 90));
    let _ = act_go(&mut rig, sounder, "+tv/1 sound quiet go");
    let heard = finish(&mut rig, tv, listener);
    assert_eq!(heard_count(&heard), 0, "{heard}");
    let row = chart_of(&mut rig.tw, sounder).expect("the row");
    assert_eq!(row.sounded_pockets(), 16);
    assert_eq!(row.heard_ic, None);
    no_refused_writes(&rig.tw);
}

/// Fire until every one of `tvs` has no act in flight and its last act is told; what
/// `listener` heard.
fn settle(rig: &mut Rig, tvs: &[ObjId], listener: ObjId) -> String {
    let due = tvs
        .iter()
        .filter_map(|tv| {
            record_of(&mut rig.tw, *tv)
                .and_then(|r| r.act)
                .map(|a| a.due_ic)
        })
        .max()
        .expect("an act in flight");
    let (events, queued) = advance_to(rig, due);
    let mut heard = said(&rig.tw, &events, listener);
    land_all(rig, &queued);
    for _ in 0..12 {
        let done = tvs.iter().all(|tv| {
            let record = record_of(&mut rig.tw, *tv);
            let told = projection(&rig.tw, *tv).map_or(0, |rec| rec.told);
            record.is_some_and(|r| r.act.is_none() && r.last.is_some_and(|l| l.ordinal <= told))
        });
        if done {
            return heard;
        }
        let (events, queued) = fire(rig);
        heard.push_str(&said(&rig.tw, &events, listener));
        land_all(rig, &queued);
    }
    panic!("the acts did not settle");
}

/// **K6 — one heard line a sounder an IC hour** (§11 Caps; D7): the sounder runs two rigs and
/// sounds with both, from a new cell each time, three rounds — every sounding two rigs fit in
/// the hour after the first is heard: exactly one heard line reaches the listener; the first
/// sounding due an hour or more after the heard one is heard again.
#[test]
fn sm4a_one_heard_line_a_sounder_an_ic_hour() {
    let mut rig = stocked();
    let (sounder, _, tvs) = on_pad(&mut rig, 2);
    let (listener, _, _) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 0);
    for (slot, tv) in tvs.iter().enumerate() {
        let n = slot + 1;
        let _ = act_go(&mut rig, sounder, &format!("+tv/{n} launch go"));
        let _ = finish(&mut rig, *tv, sounder);
    }
    let mut heard = String::new();
    // Every sounding is shown to have run (the SM.4a fixer round's finding T2): each rig's last
    // act is a loud `Sound` `Done`, and exactly one of the six spans says it was heard.
    let mut soundings = 0;
    let mut heard_spans = 0;
    for round in 0..3_i64 {
        for (slot, tv) in tvs.iter().enumerate() {
            let n = slot + 1;
            to_cell(
                &mut rig,
                *tv,
                (60 + round * 20 + i64::try_from(n).unwrap_or(0) * 7, 60),
            );
            let _ = act_go(&mut rig, sounder, &format!("+tv/{n} sound go"));
        }
        heard.push_str(&settle(&mut rig, &tvs, listener));
        for tv in &tvs {
            let last = record_of(&mut rig.tw, *tv)
                .and_then(|r| r.last)
                .expect("a finished sounding");
            assert_eq!(
                (last.kind, last.result),
                (ActKind::Sound, ActResult::Done),
                "round {round}"
            );
            match last.span {
                Some(ReadSpan::Sound {
                    quiet: false,
                    heard,
                    ..
                }) => {
                    soundings += 1;
                    if heard {
                        heard_spans += 1;
                    }
                }
                other => panic!("a loud sounding's span: {other:?}"),
            }
        }
    }
    // One heard line in the hour, so the budget still holds the first one's stamp.
    let first_heard = chart_of(&mut rig.tw, sounder)
        .and_then(|row| row.heard_ic)
        .expect("the first sounding is heard");
    let last_due = record_of(&mut rig.tw, tvs[1])
        .and_then(|r| r.last)
        .map(|l| l.at_ic)
        .expect("the last sounding");
    assert!(
        last_due < first_heard + 3_600,
        "every sounding fit in the hour"
    );
    assert_eq!(soundings, 6, "every sounding two rigs fit in the hour ran");
    assert_eq!(heard_spans, 1, "exactly one of them was heard");
    assert_eq!(heard_count(&heard), 1, "{heard}");
    // Past the hour: heard again.
    let _ = to_ic(&mut rig, first_heard + 3_600);
    to_cell(&mut rig, tvs[0], (200, 200));
    let _ = act_go(&mut rig, sounder, "+tv/1 sound go");
    let text = finish(&mut rig, tvs[0], listener);
    assert_eq!(heard_count(&text), 1, "{text}");
    no_refused_writes(&rig.tw);
}

/// **`+tv/quiet` drops heard lines and survives a restart** (D10): the listener's `+tv/quiet`
/// answers `QUIET_ON` and a loud sounding reaches them with no line; after a restart the
/// preference still stands (an hour later, a loud sounding again prints none); `+tv/quiet off`
/// answers `QUIET_OFF` and the next hour's sounding is heard.
#[test]
fn sm4a_quiet_drops_heard_lines_and_survives_a_restart() {
    let mut rig = stocked();
    let (sounder, tv, listener, _) = two_on_pad(&mut rig);
    let out = run(&mut rig.tw, listener, "+tv/quiet");
    assert_eq!(out.trim(), QUIET_ON);
    assert!(store(&rig.tw).tv_quiet(listener));
    to_cell(&mut rig, tv, (170, 90));
    let _ = act_go(&mut rig, sounder, "+tv/1 sound go");
    let due = due_of(&mut rig.tw, tv);
    let heard = finish(&mut rig, tv, listener);
    assert_eq!(heard_count(&heard), 0, "{heard}");
    let mut rig = reboot(rig, &[sounder, listener]);
    assert!(store(&rig.tw).tv_quiet(listener), "the preference survives");
    // Doc 20 cut SM.4b: a sounding from the ground reserves the real trip home, so the restart's
    // cold site and home field are warmed (the surface tick, one a fire) before it.
    warm(&mut rig);
    let _ = to_ic(&mut rig, due + 3_600);
    to_cell(&mut rig, tv, (171, 90));
    let _ = act_go(&mut rig, sounder, "+tv/1 sound go");
    let due = due_of(&mut rig.tw, tv);
    let heard = finish(&mut rig, tv, listener);
    assert_eq!(heard_count(&heard), 0, "{heard}");
    let out = run(&mut rig.tw, listener, "+tv/quiet off");
    assert_eq!(out.trim(), QUIET_OFF);
    let _ = to_ic(&mut rig, due + 3_600);
    to_cell(&mut rig, tv, (172, 90));
    let _ = act_go(&mut rig, sounder, "+tv/1 sound go");
    let heard = finish(&mut rig, tv, listener);
    assert_eq!(heard_count(&heard), 1, "{heard}");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// K7, K8. A field's middle
// ---------------------------------------------------------------------------

/// **K7 / K8 — a field's middle reads its field and its neighbours** (§5, §11 Instruments;
/// D5, D6): with the pure `seen_fields` over the cached composite, a field of Ahuna Flank whose
/// middle sees all eight neighbours is found; from it a scan covers exactly those nine fields,
/// a quiet sounding exactly 16 pockets and a loud one exactly that field's 64. A field middle
/// with fields the mast does not see exists on Ahuna Flank (pinned), and a scan from it leaves
/// those fields uncovered.
#[test]
fn sm4a_a_field_middle_reads_its_field_and_its_neighbours() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let g = ahuna(&mut rig.tw);
    let mut open: Option<(i64, i64)> = None;
    let mut blind: Option<((i64, i64), Vec<u8>)> = None;
    for fy in 1..7 {
        for fx in 1..7 {
            let cell = (fx * 32 + 16, fy * 32 + 16);
            if footing_at(g.surface(), cell).is_err() {
                continue;
            }
            let (seen, hidden) = seen_fields(g.surface(), cell, &scanner());
            if seen.len() == 9 && open.is_none() {
                open = Some(cell);
            }
            if !hidden.is_empty() && blind.is_none() {
                blind = Some((cell, hidden));
            }
        }
    }
    // The hidden half first, on a fresh chart. Ahuna Flank's relief hides fields from some
    // field middle (at the shipped lane and `KEY_1`, the first found is (48, 48), hiding eight
    // of its neighbours): pinned, so the half always runs (the SM.4a fixer round's finding T3;
    // the sight rule itself is unit-tested over a hand-built composite in `surface::chart`).
    let (cell, hidden) = blind.expect("a field middle of Ahuna Flank hides some field");
    let (seen, _) = seen_fields(g.surface(), cell, &scanner());
    to_cell(&mut rig, tv, cell);
    let _ = act_go(&mut rig, pilot, "+tv/1 scan go");
    let _ = finish(&mut rig, tv, pilot);
    let row = chart_of(&mut rig.tw, pilot).expect("the row");
    for index in hidden {
        assert!(
            !row.fields.contains_key(&index),
            "a hidden field is not covered"
        );
    }
    for index in seen {
        assert!(row.fields.contains_key(&index), "a seen field is covered");
    }
    let middle = open.expect("a field middle that sees all eight neighbours");
    let own = field_of_cell(middle).expect("a field");
    to_cell(&mut rig, tv, middle);
    let _ = act_go(&mut rig, pilot, "+tv/1 scan go");
    let _ = finish(&mut rig, tv, pilot);
    let row = chart_of(&mut rig.tw, pilot).expect("the row");
    for dy in -1..=1 {
        for dx in -1..=1 {
            let index = field_index(own.0 + dx, own.1 + dy).expect("a field");
            assert!(row.fields.contains_key(&index), "({dx}, {dy})");
        }
    }
    let _ = act_go(&mut rig, pilot, "+tv/1 sound quiet go");
    let _ = finish(&mut rig, tv, pilot);
    let row = chart_of(&mut rig.tw, pilot).expect("the row");
    assert_eq!(row.sounded_pockets(), 16);
    let _ = act_go(&mut rig, pilot, "+tv/1 sound go");
    let _ = finish(&mut rig, tv, pilot);
    let row = chart_of(&mut rig.tw, pilot).expect("the row");
    assert_eq!(row.sounded_pockets(), 64);
    for py in own.1 * 8..own.1 * 8 + 8 {
        for px in own.0 * 8..own.0 * 8 + 8 {
            assert!(row.pocket_sounded((px, py)), "({px}, {py})");
        }
    }
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// K10. No gate on knowing
// ---------------------------------------------------------------------------

/// **K10 — a lapsed licence still scans and sounds** (§8 "rights are per act", §19 item 3;
/// lead ruling 9): the pilot's stored licence is edited to have lapsed (staff's hand, through
/// a restart); `licence_gate` errs; a scan from the bay (the implied launch reads no licence)
/// and a sounding both finish `Done`.
#[test]
fn sm4a_a_lapsed_licence_still_scans_and_sounds() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
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
        doc["expires_ic"] = serde_json::json!(first + 1);
        doc["grace_secs"] = serde_json::json!(0);
    });
    // The restart forgot the site cache; the boot asked for the docked site, and the surface
    // tick warms it.
    warm(&mut rig);
    let now = ic_now(&rig.tw);
    assert!(tacsim::port::licence::licence_gate(rig.tw.world(), pilot, now).is_err());
    let _ = act_go(&mut rig, pilot, "+tv/1 scan go");
    let _ = finish(&mut rig, tv, pilot);
    let _ = act_go(&mut rig, pilot, "+tv/1 sound go");
    let _ = finish(&mut rig, tv, pilot);
    let last = record_of(&mut rig.tw, tv).and_then(|r| r.last);
    assert_eq!(
        last.map(|l| (l.kind, l.result)),
        Some((ActKind::Sound, ActResult::Done))
    );
    let row = chart_of(&mut rig.tw, pilot).expect("the row");
    assert!(!row.fields.is_empty() && row.sounded_pockets() == 64);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// K11. A restart mid-act
// ---------------------------------------------------------------------------

/// One world through a scan and a sounding, restarted mid-act or not; the rig record, the
/// chart row and the projection at the end.
fn scan_and_sound(restart: bool) -> (RigRecord, ChartRecord, TvRec) {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    for act in ["+tv/1 scan go", "+tv/1 sound go"] {
        let _ = act_go(&mut rig, pilot, act);
        let due = due_of(&mut rig.tw, tv);
        if restart {
            rig = reboot(rig, &[pilot]);
            assert!(!cached(&rig.tw), "the restart forgets the site cache");
            assert_eq!(due_of(&mut rig.tw, tv), due, "the act survives the restart");
            // The due fire runs as the uninterrupted world's does (the warm, the due step, the
            // finish queued); then the world restarts again before the finish lands, so it lands
            // with the site cache cold and builds nothing (lead ruling 6, D11; the SM.4a fixer
            // round's finding T1: the surface tick warms the cache at the top of every fire, so
            // without the second restart the finish would always meet a warm cache).
            let (_, queued) = advance_to(&mut rig, due + 40);
            assert_eq!(finishes(&queued).len(), 1);
            let ordinal = record_of(&mut rig.tw, tv)
                .and_then(|r| r.act)
                .map(|a| a.ordinal)
                .expect("the act in flight");
            rig = reboot(rig, &[pilot]);
            assert_eq!(ic_now(&rig.tw), due + 40);
            assert!(!cached(&rig.tw), "the finish lands on a cold cache");
            land_all(&mut rig, &queued);
            assert!(!cached(&rig.tw), "the finish built no site ground");
            assert_eq!(
                record_of(&mut rig.tw, tv)
                    .and_then(|r| r.last)
                    .map(|l| (l.ordinal, l.result)),
                Some((ordinal, ActResult::Done))
            );
        } else {
            let (_, queued) = advance_to(&mut rig, due + 40);
            assert_eq!(finishes(&queued).len(), 1);
            land_all(&mut rig, &queued);
        }
        for _ in 0..2 {
            let (_, queued) = fire(&mut rig);
            land_all(&mut rig, &queued);
        }
    }
    (
        record_of(&mut rig.tw, tv).expect("the record"),
        chart_of(&mut rig.tw, pilot).expect("the row"),
        projection(&rig.tw, tv).expect("the projection"),
    )
}

/// **K11 — the post-image after a restart equals an uninterrupted run** (§8; D11): two worlds
/// built alike run a scan and a sounding; one restarts with each in flight, and again between
/// the due fire queuing each finish and its landing, so each finish lands with the site cache
/// cold (the finish needs no composite, and builds none). The rig records, the chart rows and the projections are
/// equal.
#[test]
fn sm4a_a_restart_mid_scan_and_mid_sounding_equals_an_uninterrupted_run() {
    let a = scan_and_sound(false);
    let b = scan_and_sound(true);
    assert_eq!(a.0, b.0, "the rig records");
    assert_eq!(a.1, b.1, "the chart rows");
    assert_eq!(a.2, b.2, "the projections");
    assert!(a.1.has_coverage());
}

// ---------------------------------------------------------------------------
// K12. Forget
// ---------------------------------------------------------------------------

/// **K12 — `+tv/forget` strikes a site or a body** (D7, D9, D27): the listener scans; the
/// sounder scans and sounds loud (heard). `+tv/forget ahuna FLANK` previews, echoing the name as
/// typed, and writes nothing; `+tv/forget Ahuna Flank go` strikes the row to a coverage-free
/// budget-keeper (its `heard_ic` kept) and the listener's row is untouched; a second forget
/// answers `FORGET_NOTHING`; an unknown name `FORGET_UNKNOWN`; a loud sounding inside the same
/// IC hour then reaches the listener with no heard line; past the hour a forget deletes the
/// row outright; the listener's `+tv/forget Ceres go` strikes theirs by body.
#[test]
#[allow(clippy::too_many_lines)]
fn sm4a_forget_strikes_a_site_or_a_body() {
    let mut rig = stocked();
    let (sounder, tv, listener, other) = two_on_pad(&mut rig);
    let _ = act_go(&mut rig, listener, "+tv/1 scan go");
    let _ = finish(&mut rig, other, listener);
    to_cell(&mut rig, tv, (170, 90));
    let _ = act_go(&mut rig, sounder, "+tv/1 scan go");
    let _ = finish(&mut rig, tv, sounder);
    let _ = act_go(&mut rig, sounder, "+tv/1 sound go");
    let heard = finish(&mut rig, tv, listener);
    assert_eq!(heard_count(&heard), 1);
    let row = chart_of(&mut rig.tw, sounder).expect("the row");
    let heard_ic = row.heard_ic.expect("the budget");
    let preview = FORGET_PREVIEW
        .replace("{what}", "Ahuna Flank")
        .replace("{fields}", &row.fields.len().to_string())
        .replace("{pockets}", "64")
        .replace("{arg}", "ahuna FLANK");
    let (out, queued) = command(&mut rig, sounder, "+tv/forget ahuna FLANK");
    assert_eq!(out.trim(), preview);
    assert!(queued.is_empty());
    assert_eq!(
        chart_of(&mut rig.tw, sounder),
        Some(row.clone()),
        "a preview writes nothing"
    );
    let (out, queued) = command(&mut rig, sounder, "+tv/forget Ahuna Flank go");
    let forgotten = FORGOTTEN.replace("{what}", "Ahuna Flank");
    assert_eq!(out.lines().next(), Some(forgotten.as_str()), "{out}");
    assert_eq!(queued.len(), 1);
    land_all(&mut rig, &queued);
    let kept = chart_of(&mut rig.tw, sounder).expect("the budget-keeper");
    assert!(!kept.has_coverage());
    assert_eq!(kept.heard_ic, Some(heard_ic));
    assert!(chart_of(&mut rig.tw, listener).is_some_and(|r| r.has_coverage()));
    let (out, queued) = command(&mut rig, sounder, "+tv/forget Ahuna Flank");
    assert_eq!(out.trim(), FORGET_NOTHING.replace("{what}", "Ahuna Flank"));
    assert!(queued.is_empty());
    let (out, _) = command(&mut rig, sounder, "+tv/forget Nowhere");
    assert_eq!(out.trim(), FORGET_UNKNOWN.replace("{arg}", "Nowhere"));
    // Inside the hour: no heard line.
    to_cell(&mut rig, tv, (175, 90));
    let _ = act_go(&mut rig, sounder, "+tv/1 sound go");
    let heard = finish(&mut rig, tv, listener);
    assert_eq!(
        heard_count(&heard),
        0,
        "forget keeps the heard budget: {heard}"
    );
    // The sounding ran (the SM.4a fixer round's finding T2): `Done`, loud, and not heard.
    let last = record_of(&mut rig.tw, tv)
        .and_then(|r| r.last)
        .expect("the sounding");
    assert_eq!((last.kind, last.result), (ActKind::Sound, ActResult::Done));
    assert!(
        matches!(
            last.span,
            Some(ReadSpan::Sound {
                quiet: false,
                heard: false,
                ..
            })
        ),
        "{:?}",
        last.span
    );
    // Past the hour the forget deletes the row.
    let _ = to_ic(&mut rig, heard_ic + 3_600);
    let (_, queued) = command(&mut rig, sounder, "+tv/forget ahuna_flank go");
    land_all(&mut rig, &queued);
    assert_eq!(chart_of(&mut rig.tw, sounder), None);
    // By body.
    let (out, queued) = command(&mut rig, listener, "+tv/forget Ceres go");
    let forgotten = FORGOTTEN.replace("{what}", "Ceres");
    assert_eq!(out.lines().next(), Some(forgotten.as_str()), "{out}");
    land_all(&mut rig, &queued);
    assert_eq!(chart_of(&mut rig.tw, listener), None);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// K14 and the dark layer. The site ground, warmed off the act's path
// ---------------------------------------------------------------------------

/// The median of `samples`.
fn median(mut samples: Vec<u128>) -> u128 {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

/// **K14 — a cold site refuses, then warms off the act's path** (lead ruling 6; D11, D25; the
/// brief's reviewer finding F5): a restart with the hull on the pad leaves the site cache
/// empty; `+tv/1 scan` answers `GROUND_COLD` and queues nothing and `+tv` prints
/// `GROUND_COLD_LINE`; one surface fire later the scan previews. The cold build (capture,
/// revision and both builds) and the warm peek (`site_ground_cached`, the median of 21) are
/// timed and printed against the brief's STOP thresholds (a crossed threshold is the lead's
/// decision, not a failure: debug 2,000 ms / 5 ms, release 50 ms / 2 ms). Since the cut's
/// pre-commit round (item 1) the warm peek finds its entry by the captured input — no
/// serialise, no hash — so it is the capture and one comparison (about 12 ms in a debug build
/// before, well under a tenth of a millisecond after).
#[test]
fn sm4a_a_cold_site_refuses_then_warms_off_the_act_path() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let mut rig = reboot(rig, &[pilot]);
    assert!(!cached(&rig.tw));
    let (out, queued) = command(&mut rig, pilot, "+tv/1 scan");
    assert_eq!(out.trim(), GROUND_COLD.replace("{site}", "Ahuna Flank"));
    assert!(queued.is_empty());
    let status = run(&mut rig.tw, pilot, "+tv");
    let cold = GROUND_COLD_LINE
        .replace("{block}", "K")
        .replace("{tenure}", "open")
        .replace("{field}", "K-NW");
    assert!(status.contains(&cold), "{status}");
    // The cold build, timed without touching the cache.
    let (spec, body, location) = spec_of(&rig.tw, AHUNA);
    let frame = site_frame(&location);
    let started = std::time::Instant::now();
    let input = capture_site(rig.tw.world(), &frame).expect("captures");
    let _ = tacsim::surface::ground::site_revision(&input, &spec).expect("a revision");
    let built = build_site_ground(&input, &spec, &body, &mac(KEY_1), 1).expect("builds");
    let cold_ms = started.elapsed().as_millis();
    eprintln!("cold site ground build: {cold_ms} ms");
    assert!(built.geology.pocket(0, 0).is_some());
    assert!(!cached(&rig.tw), "the measurement cached nothing");
    // One surface fire warms it; the scan then previews.
    let (_, queued) = fire(&mut rig);
    land_quietly(&mut rig, &queued);
    assert!(cached(&rig.tw), "the surface tick warmed the site");
    let (out, queued) = command(&mut rig, pilot, "+tv/1 scan");
    assert!(out.contains(" would scan the "), "{out}");
    assert!(queued.is_empty());
    // The warm peek: the median of 21.
    let samples: Vec<u128> = (0..21)
        .map(|_| {
            let started = std::time::Instant::now();
            let hit = site_ground_cached(rig.tw.world(), &frame, &spec).expect("peeks");
            let us = started.elapsed().as_micros();
            assert!(hit.is_some());
            us
        })
        .collect();
    let peek_us = median(samples);
    eprintln!("warm site ground peek: {peek_us} us");
    no_refused_writes(&rig.tw);
}

/// **A dark layer never wants a site** (D16, D25; the brief's reviewer finding F4): in an
/// unkeyed world, landing and `+tv` leave nothing wanted or failed, a fire warms nothing, `+tv`
/// prints `GROUND_DARK_LINE`, and `+tv/1 scan` and `+tv/1 sound` answer `INSTRUMENTS_DARK` and
/// queue nothing.
#[test]
fn sm4a_a_dark_layer_never_wants_a_site() {
    let mut rig = stocked_with(false);
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    lease_one(&mut rig, pilot);
    load(&mut rig, &row, 20, 20, 9_110);
    board(&mut rig, pilot);
    let t = file_and_launch(&mut rig, pilot, ObjId(row.hull), "+ship/undock ahuna");
    let _ = to_ic(&mut rig, t.plan.depart_ic + t.plan.total_s);
    let tv = rigs_of(&rig.tw, pilot)[0];
    launched(&mut rig, pilot, tv);
    let status = run(&mut rig.tw, pilot, "+tv");
    let dark = GROUND_DARK_LINE
        .replace("{block}", "K")
        .replace("{tenure}", "open")
        .replace("{field}", "K-NW");
    assert!(status.contains(&dark), "{status}");
    let _ = fire(&mut rig);
    let state = rig
        .tw
        .world()
        .module_state::<GeologyState>()
        .expect("the geology slot");
    assert!(state.wanted().is_empty() && state.failed().is_empty());
    assert!(state.sites().is_empty());
    for line in ["+tv/1 scan", "+tv/1 sound go"] {
        let (out, queued) = command(&mut rig, pilot, line);
        assert_eq!(out.trim(), INSTRUMENTS_DARK, "{line}");
        assert!(queued.is_empty(), "{line}");
    }
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// The heard line waits for the projection; a moved rig; the status line; the ramps
// ---------------------------------------------------------------------------

/// **A heard line goes once the fitted projection lands** (D24; the brief's reviewer finding F8)
/// — **as doc 20 cut SM.4b leaves it** (its decision P7; the review's tests/T1 and
/// conformance/F3): a loud sounding lands its finish; before the tell, staff pad the rig's stored
/// projection (through a restart) — its log only, no leg, with lines of characters JSON escapes —
/// to just under its 4 KiB bound, so the tell's write, one more log line, would pass it
/// unfitted (asserted on the image). `TvRec::fit` drops the oldest log lines, never a leg, so the
/// write lands on the first tell: the operator is told, the listener hears exactly one heard line,
/// the projection stays under its bound with the oldest lines gone, the rest in order and the
/// sounding's line newest, and the next fires never repeat it. (That a refused write holds the
/// heard line back is pinned by `surface::tick`'s `sm4b_a_heard_line_waits_for_the_written_projection`:
/// no padding can refuse a fitted write.)
#[test]
#[allow(clippy::too_many_lines)]
fn sm4b_a_heard_line_goes_once_the_fitted_projection_lands() {
    use tacsim::surface::{MAX_TV_LOG_LINES, MAX_TV_REC_BYTES, TvLogLine};
    let mut rig = stocked();
    let (sounder, tv, listener, _) = two_on_pad(&mut rig);
    to_cell(&mut rig, tv, (170, 90));
    let _ = act_go(&mut rig, sounder, "+tv/1 sound go");
    let due = due_of(&mut rig.tw, tv);
    let (_, queued) = advance_to(&mut rig, due);
    assert_eq!(finishes(&queued).len(), 1);
    land_all(&mut rig, &queued);
    // The finish has landed; the tell has not run. Pad the stored projection's log — no leg, no
    // act shown — to just under its bound with lines of `"` (two JSON bytes a character): the
    // tell's write (one more log line) then cannot pass it unfitted.
    let mut padded = projection(&rig.tw, tv).expect("the projection beside the placement");
    padded.leg = None;
    padded.act = None;
    let size = |rec: &TvRec| serde_json::to_vec(rec).map(|b| b.len()).unwrap_or(0);
    let mut n = 0_u32;
    while size(&padded) < MAX_TV_REC_BYTES - 12 {
        match padded.log.last_mut() {
            Some(line) if line.text.starts_with('#') && line.text.len() < 160 => {
                line.text.push('"');
            }
            _ => {
                n += 1;
                padded.log.push(TvLogLine {
                    at_ic: 0,
                    text: format!("#{n:02} "),
                });
            }
        }
    }
    assert!(
        padded.log.len() < MAX_TV_LOG_LINES,
        "room for one more line by count: only the bytes overflow"
    );
    assert!(size(&padded) <= MAX_TV_REC_BYTES);
    assert_eq!(padded.check(), Ok(()));
    let key = tacsim::surface::tv_rec_key(tv);
    let wire = serde_json::to_value(&padded).expect("serializes");
    let mut rig = reboot_editing(rig, key.as_str(), &[sounder, listener], |doc| {
        *doc = wire;
    });
    let mut operator = String::new();
    let mut heard = String::new();
    for _ in 0..2 {
        let (events, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
        operator.push_str(&said(&rig.tw, &events, sounder));
        heard.push_str(&said(&rig.tw, &events, listener));
    }
    assert!(
        operator.contains("Rig 1 sounded "),
        "the operator is told: {operator}"
    );
    assert_eq!(heard_count(&heard), 1, "the fitted write lands: {heard}");
    let stored = projection(&rig.tw, tv).expect("the projection");
    assert!(stored.told > padded.told, "the tell's write landed");
    assert!(size(&stored) <= MAX_TV_REC_BYTES);
    let newest = stored.log.last().cloned().expect("a newest line");
    assert!(
        newest.text.starts_with("Rig 1 sounded "),
        "the newest line is the sounding's: {newest:?}"
    );
    // The fit, not luck: the same record with the padded log whole and the new line is past the
    // bound, and what was kept is the padded log less its oldest lines, in order.
    let mut unfitted = stored.clone();
    unfitted.log = padded.log.clone();
    unfitted.log.push(newest);
    assert!(
        size(&unfitted) > MAX_TV_REC_BYTES,
        "unfitted, the tell's write would be refused"
    );
    let kept = &stored.log[..stored.log.len() - 1];
    assert!(kept.len() < padded.log.len(), "the oldest lines went");
    assert_eq!(kept, &padded.log[padded.log.len() - kept.len()..]);
    // Never twice.
    let mut again = String::new();
    for _ in 0..3 {
        let (events, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
        again.push_str(&said(&rig.tw, &events, listener));
    }
    assert_eq!(heard_count(&again), 0, "{again}");
    no_refused_writes(&rig.tw);
}

/// **A rig moved mid-act is refused** (D14): a scan started at one cell, the rig moved by staff
/// before it is due: the finish records `Refused { Moved }`, the pilot hears `ACT_FAILED` with
/// `WHY_MOVED`, the charge is untouched and no chart row is written.
#[test]
fn sm4a_a_rig_moved_mid_act_is_refused() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let charge = record_of(&mut rig.tw, tv)
        .map(|r| r.charge)
        .expect("a charge");
    let out = act_go(&mut rig, pilot, "+tv/1 scan go");
    assert!(
        out.starts_with(
            SCAN_FILED
                .split("{when}")
                .next()
                .unwrap_or_default()
                .replace("{n}", "1")
                .as_str()
        ),
        "{out}"
    );
    to_cell(&mut rig, tv, (100, 100));
    let heard = finish(&mut rig, tv, pilot);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(
        record.last.map(|l| (l.kind, l.result)),
        Some((
            ActKind::Scan,
            ActResult::Refused {
                code: RefusedCode::Moved
            }
        ))
    );
    assert_eq!(record.charge, charge, "the charge is untouched");
    let failed = ACT_FAILED
        .replace("{n}", "1")
        .replace("{act}", "scan")
        .replace("{why}", WHY_MOVED);
    assert_eq!(count(&heard, &failed), 1, "{heard}");
    assert_eq!(chart_of(&mut rig.tw, pilot), None);
    no_refused_writes(&rig.tw);
}

/// **An implied launch moved off its stand leaves the bay at the launch half** (D14, D22; §4.1;
/// the SM.4a fixer round's finding F1): rig 1 in the bay with its charge put at 20,000 cu; a
/// sounding from the bay starts (the launch implied, the excursion opened), and staff move the
/// rig onto the ground away from the ramp before it is due. The finish records
/// `Refused { Moved }`; the excursion stays open (the rig is out) and the charge leaves the bay
/// credited to the end of the launch half and no further, nothing spent:
/// `charge_at(c0, start + 300)` out of the bay at the due instant. No chart row is written, and
/// an IC hour on, the status's charge has not moved (no bay credit accrues outside the bay).
#[test]
fn sm4a_a_moved_implied_launch_leaves_the_bay_at_the_launch_half() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let c0 = put_charge(&mut rig, tv, 20_000, true);
    let start_ic = ic_now(&rig.tw);
    let _ = act_go(&mut rig, pilot, "+tv/1 sound go");
    let due = due_of(&mut rig.tw, tv);
    assert_eq!(due, start_ic + 300 + 1_200);
    to_cell(&mut rig, tv, (100, 100));
    let heard = finish(&mut rig, tv, pilot);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(
        record.last.as_ref().map(|l| (l.kind, l.result)),
        Some((
            ActKind::Sound,
            ActResult::Refused {
                code: RefusedCode::Moved
            }
        ))
    );
    let left = Charge {
        value_cu: charge_at(&c0, start_ic + 300, &battery()),
        at_ic: due,
        in_bay: false,
    };
    assert_eq!(record.charge, left, "out of the bay at the launch half");
    assert!(left.value_cu > c0.value_cu && left.value_cu < charge_at(&c0, due, &battery()));
    assert!(
        record.excursion.is_some(),
        "the rig is out: its excursion stays open"
    );
    let failed = ACT_FAILED
        .replace("{n}", "1")
        .replace("{act}", "sound")
        .replace("{why}", WHY_MOVED);
    assert_eq!(count(&heard, &failed), 1, "{heard}");
    assert_eq!(chart_of(&mut rig.tw, pilot), None);
    let _ = to_ic(&mut rig, due + 3_600);
    let status = run(&mut rig.tw, pilot, "+tv");
    assert!(
        status.contains(&format!("charge {} %", pct(left.value_cu))),
        "{status}"
    );
    assert_eq!(record_of(&mut rig.tw, tv).map(|r| r.charge), Some(left));
    no_refused_writes(&rig.tw);
}

/// **A reading told in the dark waits on the chart** (D26; the SM.4a fixer round's finding F2):
/// a scan's finish lands and writes the chart row; the world restarts without the geology key
/// before the tell. The tell says `READINGS_DARK` — the readings are on the chart — never
/// `READINGS_GONE`; the projection logs that line; the row stands untouched.
#[test]
fn sm4a_a_reading_told_in_the_dark_waits_on_the_chart() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let _ = act_go(&mut rig, pilot, "+tv/1 scan go");
    let due = due_of(&mut rig.tw, tv);
    let ordinal = record_of(&mut rig.tw, tv)
        .and_then(|r| r.act)
        .map(|a| a.ordinal)
        .expect("the act");
    let (_, queued) = advance_to(&mut rig, due);
    assert_eq!(finishes(&queued).len(), 1);
    land_all(&mut rig, &queued);
    let row = chart_of(&mut rig.tw, pilot).expect("the finish wrote the row");
    assert!(row.has_coverage());
    let mut rig = reboot_with(rig, &[pilot], false);
    let mut told = String::new();
    for _ in 0..2 {
        let (events, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
        told.push_str(&said(&rig.tw, &events, pilot));
    }
    let dark = READINGS_DARK.replace("{n}", "1").replace("{act}", "scan");
    assert_eq!(count(&told, &dark), 1, "{told}");
    let gone = READINGS_GONE.replace("{n}", "1").replace("{act}", "scan");
    assert_eq!(count(&told, &gone), 0, "{told}");
    let rec = projection(&rig.tw, tv).expect("the projection");
    assert_eq!(rec.told, ordinal);
    assert_eq!(
        rec.log.last().map(|line| line.text.as_str()),
        Some(dark.as_str())
    );
    assert_eq!(chart_of(&mut rig.tw, pilot), Some(row));
    no_refused_writes(&rig.tw);
}

/// **The status names block, tenure, field and footing** (§5, §10; lead ruling 8, D18): a rig
/// staff put at (100, 150) reads its ground line with the hint before any reading, the field's
/// row after a scan, and the pocket's text after a sounding.
#[test]
fn sm4a_the_status_names_block_tenure_field_and_footing() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let g = ahuna(&mut rig.tw);
    let stand = middle_with(&g, Footing::Rock, &[]);
    to_cell(&mut rig, tv, stand);
    let (block, addr, field) = names(stand);
    let footing = footing_at(g.surface(), stand).expect("a footing").name();
    let status = run(&mut rig.tw, pilot, "+tv");
    assert!(
        status.contains(&ground_line(&block, &field, footing, NO_READINGS_HERE)),
        "{status}"
    );
    let _ = act_go(&mut rig, pilot, "+tv/1 scan go");
    let _ = finish(&mut rig, tv, pilot);
    let status = run(&mut rig.tw, pilot, "+tv");
    let fxy = field_of_cell(stand).expect("a field");
    assert!(
        status.contains(&ground_line(
            &block,
            &field,
            footing,
            &g.field_text(fxy, Footing::Rock)
        )),
        "{status}"
    );
    let _ = act_go(&mut rig, pilot, "+tv/1 sound go");
    let _ = finish(&mut rig, tv, pilot);
    let status = run(&mut rig.tw, pilot, "+tv");
    let pocket = pocket_of_cell(stand).expect("a pocket");
    let text = pocket_row_text(&addr, &g.pocket(pocket));
    assert!(
        status.contains(&ground_line(&block, &field, footing, &text)),
        "{status}"
    );
    no_refused_writes(&rig.tw);
}

/// **Every site's ramp cell gives a footing**: for each lane site, its keyed ground's cell under
/// the Dray-P's stern ramp is rock, regolith or ice — so the implied launch never refuses
/// `NO_FOOTING` at the foot of a ramp.
#[test]
fn sm4a_every_sites_ramp_cell_gives_a_footing() {
    let (world, _) = mush_testkit::boot_world();
    let mut tw = boot_over(world, HashMap::new(), true);
    let pack = lane();
    let mut sites = 0;
    for (entry, def) in pack.entries() {
        let PackEntry::Port(port) = def else {
            continue;
        };
        if port.site.is_none() {
            continue;
        }
        let (spec, _, location) = spec_of(&tw, entry.as_str());
        let ground = site_ground_sync(tw.world_mut(), &site_frame(&location), &spec)
            .unwrap_or_else(|e| panic!("{entry}: {e}"));
        let surface = ground
            .composite
            .surface(&mush_atlas::SurfaceRef::Ground)
            .expect("a ground surface");
        assert!(
            footing_at(surface, RAMP_CELL).is_ok(),
            "{entry}'s ramp cell: {:?}",
            footing_at(surface, RAMP_CELL)
        );
        sites += 1;
    }
    assert_eq!(sites, 6);
}

// ---------------------------------------------------------------------------
// The pre-commit round: the battery-free finish, the fallbacks, the forget's prune, the dark
// site warned once
// ---------------------------------------------------------------------------

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

/// Staff place `tv` back `Interior` in `row`'s bay compartment (`excursions.rs`'s).
fn staff_bay(rig: &mut Rig, tv: ObjId, row: &HullRow) {
    let bay = ObjId(row.rooms[&cid("bay")]);
    let slot = mush_sim::SlotId::new("rigs").expect("a slot");
    let placed = support::with_ctx(&mut rig.tw, |ctx| {
        mush_sim::place_interior(ctx, tv, bay, Some(slot))
    });
    placed.expect("staff place the rig in the bay");
}

/// Write `row` into the chart family under `key` through a typed transaction (staff's hand;
/// the key need not be the row's own).
fn put_chart_under(rig: &mut Rig, key: ChartKey, row: ChartRecord) {
    let family = handles(&rig.tw).families.charts;
    let engine = rig.tw.engine_mut();
    let prepared = {
        let mut txn = engine
            .action_transaction(&family)
            .expect("a chart transaction");
        let exists = txn.get(&family, &key).expect("reads").is_some();
        if exists {
            txn.replace(&family, &key, row).expect("replaced");
        } else {
            txn.insert(&family, &key, row).expect("inserted");
        }
        txn.prepare().expect("prepares")
    };
    let (ops, install) = prepared.into_parts();
    engine.install_prepared_action(install).expect("installs");
    keep(rig, ops);
}

/// Build a start of `act` for `tv` at `ordinal` past the command ([`start`]) and land it.
fn try_start(
    rig: &mut Rig,
    pilot: ObjId,
    tv: ObjId,
    act: &str,
    ordinal: u32,
) -> Result<(), String> {
    let submit = start(rig, pilot, tv, act, ordinal);
    land(rig, &submit)
}

/// A rig's act ordinal so far.
fn ordinal_of(tw: &mut TestWorld, tv: ObjId) -> u32 {
    record_of(tw, tv).map_or(0, |record| record.ordinal)
}

/// WARN-and-up log lines, captured (`plumbing.rs`'s).
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("the capture buffer")).into_owned()
    }

    /// How many captured lines contain `needle`.
    fn count(&self, needle: &str) -> usize {
        self.text()
            .lines()
            .filter(|line| line.contains(needle))
            .count()
    }
}

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("the capture buffer")
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Capture {
        self.clone()
    }
}

/// A dispatcher writing WARN and up into `capture` (`boot_parity.rs`'s registration anchor).
fn capture_dispatch(capture: Capture) -> tracing::Dispatch {
    static ANCHOR: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
    ANCHOR.get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
    tracing::Dispatch::new(
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .with_writer(capture)
            .finish(),
    )
}

/// Staff strike Ahuna Flank's region record from the atlas's RAM records: every capture of the
/// site's window refuses ("no region …") from here on.
fn drop_ahuna_region(rig: &mut Rig) {
    let (_, _, location) = spec_of(&rig.tw, AHUNA);
    let frame = site_frame(&location);
    let removed = rig
        .tw
        .world_mut()
        .module_state_mut::<mush_atlas::AtlasState>()
        .expect("the atlas is composed")
        .records
        .regions
        .remove(&frame.region);
    assert!(removed.is_some(), "Ahuna Flank's region was in the records");
}

/// **A ground reading never wedges without its battery row** (§4.1 "a TV cannot be stranded by
/// its battery"; the pre-commit round's item 2): rig 1, launched and standing at the ramp,
/// starts a scan from the ground; then every battery leaves the one tools table. At the due
/// instant its finish lands `Done`: the act is cleared, the charge falls by exactly the scan's
/// 1,000 cu, stamped at the due instant and still out of the bay (what the battery's own settle
/// gives out of the bay), the chart row is written from the ramp's cell and the pilot hears the
/// scan; `+ship/lift ceres` answers `TV_OUT` (the rig out, never `TV_BUSY`), and the rig comes
/// home without its battery (`+tv/1 recover go`, `Done`), after which nothing of the rig holds
/// the hull in `+ship/plan ceres`.
#[test]
#[allow(clippy::too_many_lines)]
fn sm4a_a_ground_reading_never_wedges_without_its_battery() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let before = record_of(&mut rig.tw, tv).expect("the record").charge;
    assert!(!before.in_bay);
    let _ = act_go(&mut rig, pilot, "+tv/1 scan go");
    let due = due_of(&mut rig.tw, tv);
    unbattery(&mut rig);
    let heard = finish(&mut rig, tv, pilot);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(
        record.last.as_ref().map(|l| (l.kind, l.result)),
        Some((ActKind::Scan, ActResult::Done))
    );
    assert_eq!(record.act, None, "the rig is free");
    assert_eq!(
        record.charge,
        Charge {
            value_cu: before.value_cu - 1_000,
            at_ic: due,
            in_bay: false,
        },
        "the reservation off the value, out of the bay"
    );
    let chart = chart_of(&mut rig.tw, pilot).expect("the chart row is written");
    assert!(chart.has_coverage());
    assert_eq!(chart.scan_stand, Some([RAMP_CELL.0, RAMP_CELL.1]));
    let head = SCANNED.replace("{n}", "1");
    assert!(
        heard.contains(head.split("{count}").next().unwrap_or_default()),
        "{heard}"
    );
    // The hull is held by a rig out, never by an act in flight.
    let (out, queued) = command(&mut rig, pilot, "+ship/lift ceres");
    assert_eq!(
        out.trim(),
        format!(
            "{}{}",
            TV_OUT
                .replace("{n}", "1")
                .replace("{site}", "Ahuna Flank")
                .replace("{name}", "Salt Moth"),
            tacsim::port::voyage::LEAVE_HINT.replace("{list}", "1")
        )
    );
    assert!(queued.is_empty());
    // And the rig comes home without its battery.
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(
        record_of(&mut rig.tw, tv)
            .and_then(|r| r.last)
            .map(|l| (l.kind, l.result)),
        Some((ActKind::Recover, ActResult::Done))
    );
    let bay = ObjId(row.rooms[&cid("bay")]);
    assert!(matches!(parent(&rig.tw, tv), SpatialParent::Interior { space, .. } if space == bay));
    let plan = run(&mut rig.tw, pilot, "+ship/plan ceres");
    assert!(!plan.contains("Rig 1 "), "{plan}");
    no_refused_writes(&rig.tw);
}

/// **An implied launch without its battery still fails closed** (§4.1; the pre-commit round's
/// item 2, the in-bay arm kept as SM.3b's launch): rig 1 in the bay starts a sounding (the launch
/// implied); every battery leaves the tools table; at the due instant the tick places the rig at
/// the ramp and queues the finish, which refuses "that rig cannot be read" — the bay's credit
/// cannot be settled without the battery — and writes nothing: the act stays in flight and no
/// chart row is written.
#[test]
fn sm4a_an_implied_launch_without_its_battery_fails_closed() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let _ = act_go(&mut rig, pilot, "+tv/1 sound go");
    let due = due_of(&mut rig.tw, tv);
    unbattery(&mut rig);
    let (_, queued) = advance_to(&mut rig, due);
    let done = finishes(&queued);
    assert_eq!(done.len(), 1);
    let before = record_of(&mut rig.tw, tv);
    let refused = land(&mut rig, done[0]).expect_err("the implied launch fails closed");
    assert!(refused.contains("that rig cannot be read"), "{refused}");
    assert_eq!(record_of(&mut rig.tw, tv), before, "nothing written");
    assert!(
        before.and_then(|r| r.act).is_some(),
        "the act stays in flight"
    );
    assert_eq!(chart_of(&mut rig.tw, pilot), None);
    no_refused_writes(&rig.tw);
}

/// **A reading forgotten before its tell is gone** (D26; the pre-commit round's item 7 a): a
/// scan's finish lands and writes the chart row; before the next fire's tell the pilot strikes
/// Ahuna Flank from their chart (`+tv/forget Ahuna Flank go`, the row deleted: it holds no heard
/// budget). The tell says `READINGS_GONE` once — never the rows — the projection logs it and
/// advances `told`.
#[test]
fn sm4a_a_reading_forgotten_before_its_tell_is_gone() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let _ = act_go(&mut rig, pilot, "+tv/1 scan go");
    let due = due_of(&mut rig.tw, tv);
    let ordinal = ordinal_of(&mut rig.tw, tv);
    let (_, queued) = advance_to(&mut rig, due);
    assert_eq!(finishes(&queued).len(), 1);
    land_all(&mut rig, &queued);
    assert!(chart_of(&mut rig.tw, pilot).is_some_and(|row| row.has_coverage()));
    let (out, queued) = command(&mut rig, pilot, "+tv/forget Ahuna Flank go");
    let forgotten = FORGOTTEN.replace("{what}", "Ahuna Flank");
    assert_eq!(out.lines().next(), Some(forgotten.as_str()), "{out}");
    assert_eq!(queued.len(), 1);
    land_all(&mut rig, &queued);
    assert_eq!(chart_of(&mut rig.tw, pilot), None);
    let mut told = String::new();
    for _ in 0..2 {
        let (events, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
        told.push_str(&said(&rig.tw, &events, pilot));
    }
    let gone = READINGS_GONE.replace("{n}", "1").replace("{act}", "scan");
    assert_eq!(count(&told, &gone), 1, "{told}");
    assert!(!told.contains("Rig 1 scanned "), "{told}");
    let rec = projection(&rig.tw, tv).expect("the projection");
    assert_eq!(rec.told, ordinal);
    assert_eq!(
        rec.log.last().map(|line| line.text.as_str()),
        Some(gone.as_str())
    );
    no_refused_writes(&rig.tw);
}

/// **An implied launch held in the bay never left** (D22; the pre-commit round's item 7 b): rig
/// 1 in the bay, its charge put at 20,000 cu, starts a scan (the launch implied, the excursion
/// opened); at the due instant the tick places it at the ramp and queues the finish, and before
/// the finish lands staff put the rig back in the bay. The finish records `Refused { NeverLeft }`:
/// the excursion is closed, the act cleared, the charge exactly as it was (still on the
/// umbilical, nothing spent), no chart row written, and the pilot hears `ACT_FAILED` with
/// `WHY_NEVER_LEFT` once.
#[test]
fn sm4a_an_implied_launch_held_in_the_bay_never_left() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let c0 = put_charge(&mut rig, tv, 20_000, true);
    let _ = act_go(&mut rig, pilot, "+tv/1 scan go");
    let due = due_of(&mut rig.tw, tv);
    assert!(record_of(&mut rig.tw, tv).is_some_and(|r| r.excursion.is_some()));
    let (_, queued) = advance_to(&mut rig, due);
    assert!(on_ground_at(&rig.tw, tv, RAMP), "the tick placed it");
    assert_eq!(finishes(&queued).len(), 1);
    staff_bay(&mut rig, tv, &row);
    land_all(&mut rig, &queued);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(
        record.last.as_ref().map(|l| (l.kind, l.result)),
        Some((
            ActKind::Scan,
            ActResult::Refused {
                code: RefusedCode::NeverLeft
            }
        ))
    );
    assert_eq!(record.excursion, None, "the excursion closes");
    assert_eq!(record.act, None);
    assert_eq!(record.charge, c0, "no charge spent, still on the umbilical");
    assert_eq!(chart_of(&mut rig.tw, pilot), None);
    let (events, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    let heard = said(&rig.tw, &events, pilot);
    let failed = ACT_FAILED
        .replace("{n}", "1")
        .replace("{act}", "scan")
        .replace("{why}", WHY_NEVER_LEFT);
    assert_eq!(count(&heard, &failed), 1, "{heard}");
    no_refused_writes(&rig.tw);
}

/// Staff repaint the middle cell of every tile Ahuna Flank's capture reads as `class` (in the
/// atlas's RAM records, `definitions.rs`'s precedent); returns how many tiles they touched.
fn repaint_ahuna_middle(rig: &mut Rig, class: mush_atlas::TerrainClass) -> usize {
    let (_, _, location) = spec_of(&rig.tw, AHUNA);
    let frame = site_frame(&location);
    let keys: Vec<mush_atlas::RecordKey> = capture_site(rig.tw.world(), &frame)
        .expect("the site captures")
        .tiles
        .iter()
        .map(|tile| tile.key.clone())
        .collect();
    let atlas = rig
        .tw
        .world_mut()
        .module_state_mut::<mush_atlas::AtlasState>()
        .expect("the atlas is composed");
    let mut touched = 0;
    for key in keys {
        let tile = atlas.records.tiles.get_mut(&key).expect("the tile");
        let (x, y) = (tile.cells.width() / 2, tile.cells.height() / 2);
        if tile.cells.update(x, y, |cell| cell.class = class) {
            touched += 1;
        }
    }
    touched
}

/// **A rig with no footing under it is refused** (§5; the pre-commit round's items 7 c and 1):
/// no lane site has a cell without a footing, so staff repaint the middle cell of each of Ahuna
/// Flank's tiles as paved. The changed atlas is a changed capture: the warm entry is not served
/// (`+tv/1 scan` answers `GROUND_COLD`, the revision cache found no entry built over this input)
/// and the next fire builds the new ground, in which some cell gives no footing. Staff put the
/// launched rig on the first such cell (rows from the south); `+tv/1 scan` bare and with `go`
/// answer `NO_FOOTING` naming the class, queue nothing and write nothing, and a start built past
/// the command is refused by its prepare "the rig stands where the mast reads nothing".
#[test]
#[allow(clippy::too_many_lines)]
fn sm4a_a_rig_with_no_footing_is_refused() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let paved = mush_atlas::TerrainClass::Paved;
    assert_eq!(Footing::of(paved), None);
    assert!(repaint_ahuna_middle(&mut rig, paved) > 0);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 scan");
    assert_eq!(
        out.trim(),
        GROUND_COLD.replace("{site}", "Ahuna Flank"),
        "the changed capture is not the cached ground"
    );
    assert!(queued.is_empty());
    let (_, queued) = fire(&mut rig);
    land_quietly(&mut rig, &queued);
    let g = ahuna(&mut rig.tw);
    let (bare, terrain) = (0..256_i64)
        .flat_map(|y| (0..256_i64).map(move |x| (x, y)))
        .find_map(|cell| {
            footing_at(g.surface(), cell)
                .err()
                .map(|class| (cell, class))
        })
        .expect("the repainted ground has a cell that gives no footing");
    to_cell(&mut rig, tv, bare);
    let before = (record_of(&mut rig.tw, tv), chart_of(&mut rig.tw, pilot));
    let refusal = NO_FOOTING
        .replace("{n}", "1")
        .replace("{terrain}", &terrain.label().replace('_', " "));
    for line in ["+tv/1 scan", "+tv/1 scan go"] {
        let (out, queued) = command(&mut rig, pilot, line);
        assert_eq!(out.trim(), refusal, "{line}");
        assert!(queued.is_empty(), "{line}");
    }
    let n = ordinal_of(&mut rig.tw, tv);
    let refused = try_start(&mut rig, pilot, tv, "scan", n + 1).expect_err("the prepare refuses");
    assert!(
        refused.contains("the rig stands where the mast reads nothing"),
        "{refused}"
    );
    assert_eq!(
        (record_of(&mut rig.tw, tv), chart_of(&mut rig.tw, pilot)),
        before
    );
    no_refused_writes(&rig.tw);
}

/// **A full chart refuses until a forget frees room** (§5's caps, D1, D3; the pre-commit round's
/// item 7 d): staff write the pilot's rows for the four other Ceres sites, each with all sixteen
/// blocks sounded — the body's 64-block cap. (The 1,024-field cap cannot be reached on the
/// shipped lane: Ceres has five sites of 64 fields; the block cap is the one a chart fills.)
/// `+tv/1 sound` bare and with `go` then answer `CHART_FULL` for Ceres and queue nothing, a
/// start built past the command is refused "your chart of that body is full", and nothing is
/// written; `+tv/forget <site> go` strikes one of those sites and the sounding previews again.
#[test]
#[allow(clippy::too_many_lines)]
fn sm4a_a_full_chart_refuses_until_a_forget_frees_room() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let pack = lane();
    let others: Vec<CanonicalId> = pack
        .entries()
        .iter()
        .filter_map(|(entry, def)| match def {
            PackEntry::Port(port) if port.site.is_some() && entry.as_str() != AHUNA => {
                let (spec, _, _) = spec_of(&rig.tw, entry.as_str());
                (spec.body.as_str() == "ceres").then(|| entry.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(others.len(), 4, "Ceres's other four sites: {others:?}");
    let every: Vec<(i64, i64)> = (0..64)
        .flat_map(|py| (0..64).map(move |px| (px, py)))
        .collect();
    let now = ic_now(&rig.tw);
    for site in &others {
        let mut row = ChartRecord::new(pilot, site.clone(), cid("ceres"));
        tacsim::surface::write_sound(
            &mut row,
            [0, 0],
            &every,
            true,
            now,
            tacsim::surface::Via::Hand,
        );
        assert_eq!(row.blocks.len(), 16);
        assert_eq!(row.check(), Ok(()));
        put_chart_under(&mut rig, row.key(), row);
    }
    let full = CHART_FULL.replace("{body}", "Ceres");
    let before = (record_of(&mut rig.tw, tv), chart_of(&mut rig.tw, pilot));
    for line in ["+tv/1 sound", "+tv/1 sound go"] {
        let (out, queued) = command(&mut rig, pilot, line);
        assert_eq!(out.trim(), full, "{line}");
        assert!(queued.is_empty(), "{line}");
    }
    let n = ordinal_of(&mut rig.tw, tv);
    let refused = try_start(&mut rig, pilot, tv, "sound", n + 1).expect_err("the prepare refuses");
    assert!(
        refused.contains("your chart of that body is full"),
        "{refused}"
    );
    assert_eq!(
        (record_of(&mut rig.tw, tv), chart_of(&mut rig.tw, pilot)),
        before
    );
    // A forget of one of those sites frees sixteen blocks.
    let (_, queued) = command(&mut rig, pilot, &format!("+tv/forget {} go", others[0]));
    assert_eq!(queued.len(), 1);
    land_all(&mut rig, &queued);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 sound");
    assert!(out.contains(" would sound the "), "{out}");
    assert!(queued.is_empty());
    no_refused_writes(&rig.tw);
}

/// **A start on a cold site is refused by its prepare** (lead ruling 6, D11; the pre-commit
/// round's item 7 e): after a restart the site cache is cold; a start built past the command
/// (which would answer `GROUND_COLD`) carries the offer's fixed `ground_cold` plan and is
/// refused by the prepare "the site's ground is still being mapped" — nothing written, nothing
/// built; one surface fire warms the site and the very same start lands, the scan in flight.
#[test]
fn sm4a_a_start_on_a_cold_site_is_refused_by_its_prepare() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let mut rig = reboot(rig, &[pilot]);
    assert!(!cached(&rig.tw));
    let before = (record_of(&mut rig.tw, tv), chart_of(&mut rig.tw, pilot));
    let n = ordinal_of(&mut rig.tw, tv);
    let refused = try_start(&mut rig, pilot, tv, "scan", n + 1).expect_err("a cold site refuses");
    assert!(
        refused.contains("the site's ground is still being mapped"),
        "{refused}"
    );
    assert_eq!(
        (record_of(&mut rig.tw, tv), chart_of(&mut rig.tw, pilot)),
        before
    );
    assert!(!cached(&rig.tw), "the prepare built nothing");
    let (_, queued) = fire(&mut rig);
    land_quietly(&mut rig, &queued);
    assert!(cached(&rig.tw), "the surface tick warmed the site");
    try_start(&mut rig, pilot, tv, "scan", n + 1).expect("the same start lands warm");
    assert_eq!(
        record_of(&mut rig.tw, tv)
            .and_then(|r| r.act)
            .map(|a| a.kind),
        Some(ActKind::Scan)
    );
    no_refused_writes(&rig.tw);
}

/// The status's dark ground line for a rig at the ramp.
fn dark_at_ramp() -> String {
    GROUND_DARK_LINE
        .replace("{block}", "K")
        .replace("{tenure}", "open")
        .replace("{field}", "K-NW")
}

/// **A site whose build fails reads dark until the next boot** (D25; the pre-commit round's
/// item 7 f): a restart leaves Ahuna Flank wanted and cold; staff strike its region record, so
/// the surface tick's warm fails — warned once, the site joins the boot's dark set and leaves
/// the wanted set. `+tv` then prints `GROUND_DARK_LINE` (twice, never "still being mapped") and
/// `+tv/1 scan` answers `INSTRUMENTS_DARK`, queuing nothing; a second fire warns nothing more.
#[test]
fn sm4a_a_site_that_does_not_build_reads_dark_until_the_boot() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    launched(&mut rig, pilot, tvs[0]);
    let mut rig = reboot(rig, &[pilot]);
    assert!(!cached(&rig.tw));
    drop_ahuna_region(&mut rig);
    let capture = Capture::default();
    tracing::dispatcher::with_default(&capture_dispatch(capture.clone()), || {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
        for _ in 0..2 {
            let status = run(&mut rig.tw, pilot, "+tv");
            assert!(status.contains(&dark_at_ramp()), "{status}");
        }
        let (out, queued) = command(&mut rig, pilot, "+tv/1 scan");
        assert_eq!(out.trim(), INSTRUMENTS_DARK);
        assert!(queued.is_empty());
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
    });
    assert_eq!(
        capture.count("tacsim: a site's ground"),
        1,
        "{}",
        capture.text()
    );
    assert_eq!(capture.count("does not build"), 1, "{}", capture.text());
    let state = rig
        .tw
        .world()
        .module_state::<GeologyState>()
        .expect("the geology slot");
    assert_eq!(state.failed(), vec![cid(AHUNA)]);
    // Doc 20 cut SM.6 (lead ruling 1): after a reboot every keyed site's clerk wants its own
    // cold site, so the other sites may be wanted here; the failed one never is.
    assert!(!state.wanted().contains(&cid(AHUNA)));
    no_refused_writes(&rig.tw);
}

/// **A capture failure warns once a boot** (D25; the pre-commit round's item 5): Ahuna Flank
/// warm, the rig at the ramp; staff strike the site's region record, so every capture of its
/// window refuses. Two `+tv` statuses and a `+tv/1 scan` read it dark — `GROUND_DARK_LINE`,
/// `INSTRUMENTS_DARK` — and only the first peek warns; the site joins the boot's dark set.
#[test]
fn sm4a_a_capture_failure_warns_once_a_boot() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    launched(&mut rig, pilot, tvs[0]);
    assert!(cached(&rig.tw));
    drop_ahuna_region(&mut rig);
    let capture = Capture::default();
    tracing::dispatcher::with_default(&capture_dispatch(capture.clone()), || {
        for _ in 0..2 {
            let status = run(&mut rig.tw, pilot, "+tv");
            assert!(status.contains(&dark_at_ramp()), "{status}");
        }
        let (out, queued) = command(&mut rig, pilot, "+tv/1 scan");
        assert_eq!(out.trim(), INSTRUMENTS_DARK);
        assert!(queued.is_empty());
    });
    assert_eq!(
        capture.count("tacsim: a site's ground cannot be captured"),
        1,
        "two peeks, one warning: {}",
        capture.text()
    );
    let state = rig
        .tw
        .world()
        .module_state::<GeologyState>()
        .expect("the geology slot");
    assert_eq!(state.failed(), vec![cid(AHUNA)]);
    no_refused_writes(&rig.tw);
}

/// **A budget-keeper is pruned once its hour is over** (D7; the pre-commit round's item 6): a
/// loud sounding sets the heard budget; `+tv/forget Ahuna Flank go` inside the IC hour keeps the
/// coverage-free budget-keeper, and a second forget inside the hour answers `FORGET_NOTHING`
/// and queues nothing (the row unchanged); past the hour the bare forget still answers
/// `FORGET_NOTHING` and writes nothing (a keeper is never previewed as "0 fields and 0
/// pockets", the lead-round review's S1), and `go` answers the same line while its one
/// submission deletes the row; with nothing left a forget answers `FORGET_NOTHING` and queues
/// nothing; and a scan after that writes a fresh row with no heard budget.
#[test]
#[allow(clippy::too_many_lines)]
fn sm4a_forget_prunes_a_budget_keeper_after_its_hour() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let _ = act_go(&mut rig, pilot, "+tv/1 sound go");
    let _ = finish(&mut rig, tv, pilot);
    let heard_ic = chart_of(&mut rig.tw, pilot)
        .and_then(|row| row.heard_ic)
        .expect("the heard budget");
    let (_, queued) = command(&mut rig, pilot, "+tv/forget Ahuna Flank go");
    land_all(&mut rig, &queued);
    let keeper = chart_of(&mut rig.tw, pilot).expect("the budget-keeper");
    assert!(!keeper.has_coverage());
    assert_eq!(keeper.heard_ic, Some(heard_ic));
    let nothing = FORGET_NOTHING.replace("{what}", "Ahuna Flank");
    for line in ["+tv/forget Ahuna Flank", "+tv/forget Ahuna Flank go"] {
        let (out, queued) = command(&mut rig, pilot, line);
        assert_eq!(out.trim(), nothing, "{line}");
        assert!(queued.is_empty(), "{line}");
    }
    assert_eq!(
        chart_of(&mut rig.tw, pilot),
        Some(keeper),
        "inside the hour"
    );
    // Past the hour: the keeper is still nothing the player can see (the lead-round review's
    // S1: never a "0 fields and 0 pockets" preview that flips on a hidden timer) — the bare
    // forget answers `FORGET_NOTHING` and writes nothing; `go` answers the same line and
    // submits the one forget that prunes it.
    let _ = to_ic(&mut rig, heard_ic + 3_600);
    let (out, queued) = command(&mut rig, pilot, "+tv/forget Ahuna Flank");
    assert_eq!(out.trim(), nothing, "bare, past the hour");
    assert!(queued.is_empty());
    let keeper = chart_of(&mut rig.tw, pilot).expect("the keeper, untouched by a bare forget");
    assert!(!keeper.has_coverage());
    let (out, queued) = command(&mut rig, pilot, "+tv/forget Ahuna Flank go");
    // The first line is the command's answer; the engine adds its receipt ("Queued.") after
    // any authoritative submission, as it does after `FORGOTTEN`.
    assert_eq!(
        out.lines().next(),
        Some(nothing.as_str()),
        "go, past the hour: {out}"
    );
    assert!(
        !out.contains(&FORGOTTEN.replace("{what}", "Ahuna Flank")),
        "{out}"
    );
    assert_eq!(queued.len(), 1);
    land_all(&mut rig, &queued);
    assert_eq!(chart_of(&mut rig.tw, pilot), None, "the keeper is pruned");
    let (out, queued) = command(&mut rig, pilot, "+tv/forget Ahuna Flank go");
    assert_eq!(out.trim(), nothing);
    assert!(queued.is_empty());
    // The next finish writes a fresh row.
    to_cell(&mut rig, tv, (RAMP_CELL.0 + 3, RAMP_CELL.1));
    let _ = act_go(&mut rig, pilot, "+tv/1 scan go");
    let _ = finish(&mut rig, tv, pilot);
    let fresh = chart_of(&mut rig.tw, pilot).expect("a fresh row");
    assert!(fresh.has_coverage());
    assert_eq!(fresh.heard_ic, None);
    assert!(fresh.blocks.is_empty());
    no_refused_writes(&rig.tw);
}

/// **A chart row that will not read is never forgotten blind** (the families rule; the
/// pre-commit round's item 8): staff write, under the pilot's Ahuna Flank key, a row that is
/// another character's; `+tv/forget Ahuna Flank` bare and with `go` answer `CHART_UNREAD` —
/// never `INSTRUMENTS_DARK`, which speaks of the ground under a rig — queue nothing and change
/// nothing.
#[test]
fn sm4a_an_unreadable_chart_row_is_never_forgotten_blind() {
    let mut rig = stocked();
    let (pilot, _, _) = on_pad(&mut rig, 1);
    let mut foreign = ChartRecord::new(ObjId(pilot.0 + 1_000_000), cid(AHUNA), cid("ceres"));
    tacsim::surface::write_scan(
        &mut foreign,
        [128, 124],
        &[27],
        Footing::Rock,
        1,
        tacsim::surface::Via::Hand,
    );
    let key = ChartKey {
        principal: pilot,
        site: cid(AHUNA),
    };
    put_chart_under(&mut rig, key, foreign.clone());
    let unread = CHART_UNREAD.replace("{what}", "Ahuna Flank");
    assert_eq!(
        unread,
        "Your chart of Ahuna Flank cannot be read right now; nothing changes."
    );
    for line in ["+tv/forget Ahuna Flank", "+tv/forget Ahuna Flank go"] {
        let (out, queued) = command(&mut rig, pilot, line);
        assert_eq!(out.trim(), unread, "{line}");
        assert!(queued.is_empty(), "{line}");
    }
    assert_eq!(chart_of(&mut rig.tw, pilot), Some(foreign));
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// Survey commissions: fresh soundings, durable proof and escrow settlement
// ---------------------------------------------------------------------------

fn commission(tw: &mut TestWorld, offer: &str) -> Option<tacsim::surface::survey::SurveyRecord> {
    let family = handles(tw).families.surveys;
    let key = tacsim::surface::survey::SurveyKey::parse(offer).expect("a survey offer");
    typed(tw, &family, &key)
}

fn put_commission(rig: &mut Rig, row: tacsim::surface::survey::SurveyRecord) {
    let family = handles(&rig.tw).families.surveys;
    let engine = rig.tw.engine_mut();
    let prepared = {
        let mut txn = engine
            .action_transaction(&family)
            .expect("a survey transaction");
        txn.replace(&family, &row.offer.clone(), row)
            .expect("replace a retained receipt");
        txn.prepare().expect("a prepared receipt")
    };
    let (ops, install) = prepared.into_parts();
    engine
        .install_prepared_action(install)
        .expect("install the test receipt");
    keep(rig, ops);
}

fn occator_standing(tw: &TestWorld, pilot: ObjId) -> i16 {
    let organization = mush_org::OrgId(port_map(tw, "ceres_port").orgs[&cid("occator")].org);
    let faction = tw
        .world()
        .module_state::<mush_faction::FactionState>()
        .unwrap()
        .factions
        .values()
        .find(|faction| faction.org == Some(organization))
        .expect("Occator has its actual public faction")
        .id;
    mush_faction::public_standing(tw.world(), faction, pilot)
        .expect("public standing")
        .score
}

fn survey_relationship(
    tw: &TestWorld,
    row: &tacsim::surface::survey::SurveyRecord,
) -> (i16, i16, usize) {
    let npc = row
        .standing
        .as_ref()
        .and_then(|terms| terms.representative)
        .expect("acceptance freezes the publicly assigned survey representative");
    tw.world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap()
        .relationships
        .record_for(npc, row.pilot)
        .map_or((0, 0, 0), |relation| {
            (relation.trust, relation.disposition, relation.effects.len())
        })
}

fn survey_money(tw: &TestWorld, account: u64) -> (i64, i64) {
    let economy = tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("the economy");
    let account = economy
        .snapshot()
        .account(mush_economy::AccountId(account))
        .expect("an account");
    (account.posted_balance.minor(), account.held_amount.minor())
}

fn survey_contract(tw: &TestWorld, contract: u64) -> mush_economy::ContractState {
    tw.world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("the economy")
        .snapshot()
        .contract(mush_economy::ContractId(contract))
        .expect("the commission's contract")
        .state
}

/// Advance through a full economy sweep as well as the port and surface ticks.
fn survey_sweep(rig: &mut Rig) {
    let next = ic_now(&rig.tw) + 61;
    let (_, queued) = advance_to(rig, next);
    land_all(rig, &queued);
}

/// Accept through the command, its authoritative activation, and Occator's escrow tick.
fn accept_commission(
    rig: &mut Rig,
    pilot: ObjId,
    offer: &str,
) -> tacsim::surface::survey::SurveyRecord {
    let capture = Capture::default();
    let dispatch = capture_dispatch(capture.clone());
    let (out, queued) = tracing::dispatcher::with_default(&dispatch, || {
        command(rig, pilot, &format!("+survey/accept {offer} go"))
    });
    assert!(
        out.contains("Standing after completed work is paid:"),
        "{out}"
    );
    assert!(
        out.contains("Your original public survey representative"),
        "{out}"
    );
    assert_eq!(
        queued.len(),
        1,
        "one commission activation: {out}\n{}",
        capture.text()
    );
    assert_eq!(
        queued[0].action.key().id.as_str(),
        tacsim::surface::survey::SURVEY_ACTION,
        "{out}"
    );
    assert_eq!(queued[0].actor, pilot);
    land_all(rig, &queued);
    let accepted = commission(&mut rig.tw, offer).expect("a durable commission");
    let standing = accepted
        .standing
        .as_ref()
        .expect("newly accepted public standing terms");
    assert_eq!(standing.delta, 10);
    assert!(
        standing.secondary.is_empty(),
        "current surveys harm no rival"
    );
    assert!(standing.representative.is_some());
    assert_eq!(accepted.pilot, pilot);
    assert_eq!(accepted.covered(), 0);
    assert_eq!(accepted.reward_minor, 50_000);
    assert_eq!(accepted.deadline_ic - accepted.accepted_ic, 7 * 86_400);
    assert_eq!(
        survey_contract(&rig.tw, accepted.contract),
        mush_economy::ContractState::Offered,
        "the authoritative activation has not yet spent Occator's money"
    );
    let before = survey_money(&rig.tw, accepted.sponsor_account);
    for _ in 0..8 {
        let (_, queued) = fire(rig);
        land_all(rig, &queued);
        if survey_contract(&rig.tw, accepted.contract) == mush_economy::ContractState::Active {
            break;
        }
    }
    assert_eq!(
        survey_contract(&rig.tw, accepted.contract),
        mush_economy::ContractState::Active,
        "Occator accepted and funded the commission"
    );
    assert_eq!(
        survey_money(&rig.tw, accepted.sponsor_account),
        (before.0, before.1 + accepted.reward_minor),
        "the reward is held before any work earns it"
    );
    accepted
}

/// Retain the spatial recovery ledger too, so a midway reboot keeps this real placement.
fn survey_to_cell(rig: &mut Rig, tv: ObjId, cell: (i64, i64)) {
    let domain = ahuna_domain(&rig.tw);
    let staff = support::place_keeping(&mut rig.tw, domain, tv, cell);
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    fold_module_ops_into(&mut rig.store, staff);
}

fn survey_sound(rig: &mut Rig, pilot: ObjId, tv: ObjId, cell: (i64, i64), quiet: bool) {
    survey_to_cell(rig, tv, cell);
    let line = if quiet {
        "+tv/1 sound quiet go"
    } else {
        "+tv/1 sound go"
    };
    let _ = act_go(rig, pilot, line);
    let _ = finish(rig, tv, pilot);
    assert_eq!(
        record_of(&mut rig.tw, tv)
            .and_then(|row| row.last)
            .map(|last| last.result),
        Some(ActResult::Done),
        "a successful, committed sounding supplies the proof"
    );
}

#[test]
fn qureshi_shares_her_private_story_after_two_real_paid_surveys() {
    const STORY: &str = "My first Occator stamp came back backwards.";

    // Interplanetary travel also advances house rigs. Finish this captain's exact act
    // once while landing the background work that became due during the journey.
    fn finish_with_background(rig: &mut Rig, tv: ObjId, pilot: ObjId) {
        let act = record_of(&mut rig.tw, tv).unwrap().act.unwrap();
        let (_, queued) = advance_to(rig, act.due_ic);
        let tv_text = tv.0.to_string();
        assert_eq!(
            finishes(&queued)
                .into_iter()
                .filter(|submit| {
                    submit.actor == pilot
                        && submit
                            .parameters
                            .get("tv")
                            .and_then(serde_json::Value::as_str)
                            == Some(tv_text.as_str())
                        && submit
                            .parameters
                            .get("ordinal")
                            .and_then(serde_json::Value::as_u64)
                            == Some(u64::from(act.ordinal))
                })
                .count(),
            1,
            "one finish for this rig and ordinal: {queued:?}"
        );
        land_all(rig, &queued);
        let after = record_of(&mut rig.tw, tv).unwrap();
        assert!(after.act.is_none());
        let last = after.last.unwrap();
        assert_eq!(last.ordinal, act.ordinal);
        assert_eq!(last.result, ActResult::Done);
        let (_, queued) = fire(rig);
        land_all(rig, &queued);
    }

    let mut rig = stocked();
    let (pilot, hull, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hull_id = ObjId(hull.hull);
    launched(&mut rig, pilot, tv);
    let first = accept_commission(&mut rig, pilot, "ahuna-a");
    let factor = first.standing.as_ref().unwrap().representative.unwrap();
    for cell in [(16, 208), (48, 208), (16, 240), (48, 240)] {
        survey_sound(&mut rig, pilot, tv, cell, false);
    }
    for _ in 0..8 {
        survey_sweep(&mut rig);
        if survey_relationship(&rig.tw, &first).0 == 10 {
            break;
        }
    }
    assert_eq!(survey_relationship(&rig.tw, &first), (10, 5, 1));

    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    finish_with_background(&mut rig, tv, pilot);
    let passage = file_and_launch(&mut rig, pilot, hull_id, "+ship/undock ceres");
    let _ = to_ic(&mut rig, passage.plan.depart_ic + passage.plan.total_s);
    let left = run(&mut rig.tw, pilot, "+ship/leave");
    let exchange = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, pilot, exchange);
    let hint = run(
        &mut rig.tw,
        pilot,
        &format!("+npc/lore {factor}=Qureshi's stories"),
    );
    assert!(
        hint.contains("The first stamp"),
        "public exact-topic hint: {hint}; {left}"
    );
    let query = "+npc/lore Rosalind=The first stamp";
    let refused = run(&mut rig.tw, pilot, query);
    assert!(
        refused.contains("No lore is available"),
        "one completed survey is insufficient: {refused}"
    );
    assert!(!refused.contains(STORY));

    let mut kept = Vec::new();
    support::sites::open_site_keeping(&mut rig.tw, "kerwan_port", &mut |ops| kept.push(ops));
    for ops in kept {
        keep(&mut rig, ops);
    }
    board(&mut rig, pilot);
    let passage = file_and_launch(&mut rig, pilot, hull_id, "+ship/undock kerwan");
    let _ = to_ic(&mut rig, passage.plan.depart_ic + passage.plan.total_s);
    assert!(
        matches!(tacsim::port::ship::berth_of(rig.tw.world(), hull_id),
        tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == "kerwan_port")
    );
    for _ in 0..8 {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
    }
    let second = accept_commission(&mut rig, pilot, "kerwan-f");
    assert_eq!(
        second.standing.as_ref().unwrap().representative,
        Some(factor)
    );
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    finish_with_background(&mut rig, tv, pilot);
    let domain = mush_sim::DomainId(port_map(&rig.tw, "kerwan_port").domain.unwrap());
    for cell in [(80, 144), (112, 144), (80, 176), (112, 176)] {
        let staff = support::place_keeping(&mut rig.tw, domain, tv, cell);
        fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
        fold_module_ops_into(&mut rig.store, staff);
        let _ = act_go(&mut rig, pilot, "+tv/1 sound go");
        finish_with_background(&mut rig, tv, pilot);
    }
    assert_eq!(commission(&mut rig.tw, "kerwan-f").unwrap().covered(), 256);
    for _ in 0..8 {
        survey_sweep(&mut rig);
        if survey_relationship(&rig.tw, &second).0 == 20 {
            break;
        }
    }
    assert_eq!(survey_relationship(&rig.tw, &second), (20, 10, 2));
    assert_eq!(occator_standing(&rig.tw, pilot), 20);
    assert!(matches!(
        survey_contract(&rig.tw, second.contract),
        mush_economy::ContractState::Settled { .. }
    ));

    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    finish_with_background(&mut rig, tv, pilot);
    let passage = file_and_launch(&mut rig, pilot, hull_id, "+ship/undock ceres");
    let _ = to_ic(&mut rig, passage.plan.depart_ic + passage.plan.total_s);
    let _ = run(&mut rig.tw, pilot, "+ship/leave");
    teleport(&mut rig.tw, pilot, exchange);
    rig = reboot(rig, &[pilot]);
    let bystander = self::pilot(&mut rig, "Listener", 2);
    teleport(&mut rig.tw, bystander, exchange);
    let events = rig.tw.run(pilot, query);
    assert!(
        said(&rig.tw, &events, pilot).contains(STORY),
        "earned story persists across reboot"
    );
    assert!(
        !said(&rig.tw, &events, bystander).contains(STORY),
        "the reply is private"
    );
    let stranger = run(&mut rig.tw, bystander, query);
    assert!(
        !stranger.contains(STORY),
        "another character cannot borrow earned trust"
    );
    let relationship = run(&mut rig.tw, pilot, "+npc/relationship Rosalind");
    assert!(
        relationship.contains("Trust: trusting.")
            && relationship.contains("Disposition: warm.")
            && relationship.contains("Public standing with Occator Water & Propellant: friendly."),
        "current personal relationship: {relationship}"
    );
    let host = rig
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap();
    assert!(
        !host
            .runtime(factor)
            .map(|runtime| runtime.memory.transcript(16_384))
            .unwrap_or_default()
            .contains(STORY)
    );
    assert!(
        !host
            .personal_memory
            .record_for(factor)
            .map(|record| serde_json::to_string(record).unwrap())
            .unwrap_or_default()
            .contains(STORY)
    );
    assert_eq!(survey_relationship(&rig.tw, &second), (20, 10, 2));
    no_refused_writes(&rig.tw);
}

#[test]
fn survey_commission_pays_fresh_complete_soundings_once_across_restart_and_forgetting() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    let (board, queued) = command(&mut rig, pilot, "+survey board");
    assert!(queued.is_empty(), "the board is a read: {board}");
    for offer in ["ahuna-a", "kerwan-f", "juling-k", "occator-p"] {
        assert!(board.contains(offer), "{offer}: {board}");
    }
    let (preview, queued) = command(&mut rig, pilot, "+survey/accept ahuna-a");
    assert!(queued.is_empty(), "a preview does not accept: {preview}");
    assert!(
        preview.contains("Occator Water & Propellant +10"),
        "{preview}"
    );
    assert!(
        preview.contains("+10 trust and +5 disposition"),
        "{preview}"
    );
    assert!(commission(&mut rig.tw, "ahuna-a").is_none());

    // This sounding completes after acceptance, but began before it: old work earns nothing.
    let middles = [(16, 208), (48, 208), (16, 240), (48, 240)];
    survey_to_cell(&mut rig, tv, middles[0]);
    let _ = act_go(&mut rig, pilot, "+tv/1 sound go");
    let started_ic = ic_now(&rig.tw);
    // Restore the content's unbound public-role state immediately before go.
    // This exercises command-time initialization as an ordinary pilot, with no
    // intervening coarse tick that could hide an attribute-owner regression.
    let factor = rig
        .tw
        .world()
        .module_state::<mush_content::ContentState>()
        .unwrap()
        .latches
        .applied(&cid(tacsim::definitions::content::CAST_CERES_PACK))
        .unwrap()
        .entries()[&cid("npc.qureshi")]
        .object()
        .unwrap();
    let profile = rig
        .tw
        .world()
        .attr(factor, mush_npc::PROFILE_ATTR)
        .unwrap()
        .clone();
    let content_owner = profile.owner;
    assert_ne!(content_owner, pilot);
    let mut unbound = profile.as_json().unwrap().clone();
    unbound["public_faction"] = serde_json::Value::Null;
    rig.tw
        .world_mut()
        .set_attr(
            factor,
            mush_npc::PROFILE_ATTR,
            mush_core::Attr::json(unbound, content_owner),
        )
        .unwrap();
    rig.tw
        .world_mut()
        .remove_attr(factor, "tacsim.public_desk_binding")
        .unwrap();
    let unbound_profile = rig
        .tw
        .world()
        .attr(factor, mush_npc::PROFILE_ATTR)
        .unwrap()
        .clone();
    let (preview, queued) = command(&mut rig, pilot, "+survey/accept ahuna-a");
    assert!(queued.is_empty(), "a preview does not accept: {preview}");
    assert!(
        preview.contains(
            "An assigned public survey representative gains +10 trust and +5 disposition"
        ),
        "{preview}"
    );
    assert_eq!(
        rig.tw.world().attr(factor, mush_npc::PROFILE_ATTR),
        Some(&unbound_profile)
    );
    assert!(
        rig.tw
            .world()
            .attr(factor, "tacsim.public_desk_binding")
            .is_none()
    );
    let accepted = accept_commission(&mut rig, pilot, "ahuna-a");
    assert_eq!(
        rig.tw
            .world()
            .attr(factor, mush_npc::PROFILE_ATTR)
            .unwrap()
            .owner,
        content_owner
    );
    assert_eq!(
        rig.tw
            .world()
            .attr(factor, "tacsim.public_desk_binding")
            .unwrap()
            .owner,
        GOD
    );
    assert_eq!(
        accepted.standing.as_ref().unwrap().representative,
        Some(factor)
    );
    assert_eq!(occator_standing(&rig.tw, pilot), 0);
    assert_eq!(survey_relationship(&rig.tw, &accepted), (0, 0, 0));
    assert_eq!(
        accepted.accepted_ic, started_ic,
        "earlier work in the same IC second is stale too"
    );
    let sponsor_before = survey_money(&rig.tw, accepted.sponsor_account);
    let payee_before = survey_money(&rig.tw, accepted.payee);
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(commission(&mut rig.tw, "ahuna-a").unwrap().covered(), 0);
    assert_eq!(chart_of(&mut rig.tw, pilot).unwrap().sounded_pockets(), 64);
    let (_, queued) = command(&mut rig, pilot, "+tv/forget Ahuna Flank go");
    land_all(&mut rig, &queued);

    survey_sound(&mut rig, pilot, tv, middles[0], true);
    assert_eq!(commission(&mut rig.tw, "ahuna-a").unwrap().covered(), 16);
    survey_sound(&mut rig, pilot, tv, middles[0], false);
    assert_eq!(commission(&mut rig.tw, "ahuna-a").unwrap().covered(), 64);
    survey_sound(&mut rig, pilot, tv, middles[1], false);
    let half = commission(&mut rig.tw, "ahuna-a").unwrap();
    assert_eq!(half.covered(), 128);
    assert!(half.completed.is_none());
    survey_sweep(&mut rig);
    assert_eq!(survey_money(&rig.tw, accepted.payee), payee_before);
    assert_eq!(
        occator_standing(&rig.tw, pilot),
        0,
        "partial work earns no standing"
    );
    assert_eq!(
        survey_money(&rig.tw, accepted.sponsor_account),
        sponsor_before
    );

    survey_to_cell(&mut rig, tv, middles[2]);
    let _ = act_go(&mut rig, pilot, "+tv/1 sound go");
    let due = due_of(&mut rig.tw, tv);
    rig = reboot(rig, &[pilot]);
    assert_eq!(commission(&mut rig.tw, "ahuna-a"), Some(half));
    assert_eq!(due_of(&mut rig.tw, tv), due);
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(commission(&mut rig.tw, "ahuna-a").unwrap().covered(), 192);
    warm(&mut rig);

    survey_to_cell(&mut rig, tv, middles[3]);
    let _ = act_go(&mut rig, pilot, "+tv/1 sound go");
    let due = due_of(&mut rig.tw, tv);
    let (_, queued) = advance_to(&mut rig, due);
    let last_finish = finishes(&queued)[0].clone();
    land_all(&mut rig, &queued);
    let completed = commission(&mut rig.tw, "ahuna-a").unwrap();
    assert_eq!(completed.covered(), 256);
    assert_eq!(completed.completed.as_ref().unwrap().tv, tv);
    assert_eq!(survey_money(&rig.tw, accepted.payee), payee_before);
    assert_eq!(
        occator_standing(&rig.tw, pilot),
        0,
        "complete but unpaid work has no grant"
    );
    // A retained row cannot enlarge its public effects after acceptance. Even a
    // well-shaped edit breaks the fact reference frozen in both parties' consent.
    let mut forged = completed.clone();
    forged.standing.as_mut().unwrap().delta = 20;
    put_commission(&mut rig, forged);
    survey_sweep(&mut rig);
    assert!(!matches!(
        survey_contract(&rig.tw, accepted.contract),
        mush_economy::ContractState::Settled { .. }
    ));
    assert_eq!(survey_money(&rig.tw, accepted.payee), payee_before);
    assert_eq!(occator_standing(&rig.tw, pilot), 0);
    assert_eq!(survey_relationship(&rig.tw, &accepted), (0, 0, 0));
    put_commission(&mut rig, completed.clone());
    rig = reboot(rig, &[pilot]);
    for _ in 0..8 {
        survey_sweep(&mut rig);
        if matches!(
            survey_contract(&rig.tw, accepted.contract),
            mush_economy::ContractState::Settled { .. }
        ) {
            break;
        }
    }
    let settled = survey_contract(&rig.tw, accepted.contract);
    assert!(matches!(
        settled,
        mush_economy::ContractState::Settled { .. }
    ));
    // The standing reconciler may run before settlement in this same scheduler
    // fire; the next pass repairs it without depending on an observer event.
    survey_sweep(&mut rig);
    assert_eq!(occator_standing(&rig.tw, pilot), 10);
    assert_eq!(survey_relationship(&rig.tw, &accepted), (10, 5, 1));
    let sponsor_paid = survey_money(&rig.tw, accepted.sponsor_account);
    let payee_paid = survey_money(&rig.tw, accepted.payee);
    assert_eq!(
        sponsor_paid,
        (sponsor_before.0 - 50_000, sponsor_before.1 - 50_000)
    );
    assert_eq!(payee_paid, (payee_before.0 + 50_000, payee_before.1));
    assert_eq!(
        sponsor_paid.0 + payee_paid.0,
        sponsor_before.0 + payee_before.0
    );
    assert!(
        land(&mut rig, &last_finish).is_err(),
        "a finish cannot replay"
    );

    let (_, queued) = command(&mut rig, pilot, "+tv/forget Ahuna Flank go");
    land_all(&mut rig, &queued);
    assert_eq!(commission(&mut rig.tw, "ahuna-a"), Some(completed.clone()));
    rig = reboot(rig, &[pilot]);
    survey_sweep(&mut rig);
    let (out, queued) = command(&mut rig, pilot, "+survey/accept ahuna-a go");
    assert!(
        queued.is_empty(),
        "a completed global offer cannot reopen: {out}"
    );
    assert_eq!(commission(&mut rig.tw, "ahuna-a"), Some(completed));
    assert_eq!(survey_contract(&rig.tw, accepted.contract), settled);
    assert_eq!(
        survey_money(&rig.tw, accepted.sponsor_account),
        sponsor_paid
    );
    assert_eq!(survey_money(&rig.tw, accepted.payee), payee_paid);
    assert_eq!(
        occator_standing(&rig.tw, pilot),
        10,
        "restart and replay grant only once"
    );
    assert_eq!(survey_relationship(&rig.tw, &accepted), (10, 5, 1));
    no_refused_writes(&rig.tw);
}

#[test]
fn survey_commission_rejects_another_pilots_work_wrong_block_and_wrong_site() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let (other, _, other_tvs) = on_pad_as(&mut rig, "Other", 2, "Copper Moth", 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);
    launched(&mut rig, other, other_tvs[0]);
    let accepted = accept_commission(&mut rig, pilot, "ahuna-a");
    let sponsor = survey_money(&rig.tw, accepted.sponsor_account);
    let payee = survey_money(&rig.tw, accepted.payee);

    let sibling = self::pilot(&mut rig, "Sibling", 3);
    rig.oracle
        .0
        .lock()
        .expect("the account oracle")
        .insert(sibling, accepted.account);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}=tacsim.credit", sibling.0),
    );
    let (out, queued) = command(&mut rig, sibling, "+survey/accept kerwan-f go");
    assert!(
        out.contains("account already has"),
        "the gateway limit covers another character: {out}"
    );
    assert!(queued.is_empty());

    for (who, offer) in [(other, "ahuna-a"), (pilot, "kerwan-f")] {
        let (out, queued) = command(&mut rig, who, &format!("+survey/accept {offer} go"));
        assert!(
            queued.is_empty(),
            "global offer and gateway exclusivity: {out}"
        );
    }
    survey_sound(&mut rig, other, other_tvs[0], (16, 208), false);
    assert_eq!(commission(&mut rig.tw, "ahuna-a").unwrap().covered(), 0);
    survey_sound(&mut rig, pilot, tv, (80, 208), false);
    assert_eq!(commission(&mut rig.tw, "ahuna-a").unwrap().covered(), 0);
    survey_to_cell(&mut rig, tv, (16, 208));
    let _ = act_go(&mut rig, pilot, "+tv/1 sound go");
    survey_to_cell(&mut rig, tv, (48, 208));
    let _ = finish(&mut rig, tv, pilot);
    assert_ne!(
        record_of(&mut rig.tw, tv)
            .and_then(|row| row.last)
            .map(|last| last.result),
        Some(ActResult::Done),
        "a rig moved before its sounding finishes has no successful proof"
    );
    assert_eq!(commission(&mut rig.tw, "ahuna-a").unwrap().covered(), 0);
    survey_sweep(&mut rig);
    assert_eq!(survey_money(&rig.tw, accepted.sponsor_account), sponsor);
    assert_eq!(survey_money(&rig.tw, accepted.payee), payee);

    let (out, queued) = command(&mut rig, pilot, "+survey/cancel ahuna-a go");
    assert_eq!(queued.len(), 1, "one authoritative cancellation: {out}");
    let cancellation = queued[0].clone();
    land_all(&mut rig, &queued);
    let cancelled = commission(&mut rig.tw, "ahuna-a").unwrap();
    assert!(cancelled.cancelled.is_some());
    assert!(cancelled.completed.is_none());
    for _ in 0..8 {
        survey_sweep(&mut rig);
        if matches!(
            survey_contract(&rig.tw, accepted.contract),
            mush_economy::ContractState::Settled { .. }
        ) {
            break;
        }
    }
    let refunded = survey_contract(&rig.tw, accepted.contract);
    assert!(
        matches!(refunded, mush_economy::ContractState::Settled { .. }),
        "{out}"
    );
    assert_eq!(survey_money(&rig.tw, accepted.payee), payee);
    assert_eq!(
        occator_standing(&rig.tw, pilot),
        0,
        "a settled cancellation is not paid work"
    );
    assert_eq!(survey_relationship(&rig.tw, &accepted), (0, 0, 0));
    assert_eq!(
        survey_money(&rig.tw, accepted.sponsor_account),
        (sponsor.0, sponsor.1 - 50_000)
    );
    assert!(
        land(&mut rig, &cancellation).is_err(),
        "a refunded cancellation cannot replay"
    );
    rig = reboot(rig, &[pilot, other, sibling]);
    survey_sweep(&mut rig);
    assert!(
        land(&mut rig, &cancellation).is_err(),
        "a restart cannot revive the cancellation"
    );
    assert_eq!(commission(&mut rig.tw, "ahuna-a"), Some(cancelled));
    assert_eq!(survey_contract(&rig.tw, accepted.contract), refunded);
    assert_eq!(survey_money(&rig.tw, accepted.payee), payee);
    assert_eq!(
        survey_money(&rig.tw, accepted.sponsor_account),
        (sponsor.0, sponsor.1 - 50_000)
    );
    let kerwan = accept_commission(&mut rig, pilot, "kerwan-f");
    assert_ne!(kerwan.contract, accepted.contract);
    // This is block F, but it is still Ahuna rather than the commissioned Kerwan site.
    warm(&mut rig);
    survey_sound(&mut rig, pilot, tv, (80, 144), false);
    let untouched = commission(&mut rig.tw, "kerwan-f").unwrap();
    assert_eq!(untouched.covered(), 0);
    assert!(untouched.completed.is_none());
    survey_sweep(&mut rig);
    assert_eq!(survey_money(&rig.tw, kerwan.payee), payee);
    for character in [pilot, other, sibling] {
        assert_eq!(occator_standing(&rig.tw, character), 0);
    }
    assert_eq!(
        survey_contract(&rig.tw, kerwan.contract),
        mush_economy::ContractState::Active
    );
    no_refused_writes(&rig.tw);
}

#[test]
fn survey_commission_expiry_refunds_escrow_and_reopens_with_a_new_contract() {
    let mut rig = stocked();
    let pilot = pilot(&mut rig, "Pilot", 1);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}=tacsim.credit", pilot.0),
    );
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .expect("the economy")
            .peek_next_contract(),
        0,
        "the first commission exercises the economy's valid zero contract id"
    );
    let accepted = accept_commission(&mut rig, pilot, "juling-k");
    assert_eq!(accepted.contract, 0);
    let sponsor = survey_money(&rig.tw, accepted.sponsor_account);
    let payee = survey_money(&rig.tw, accepted.payee);
    rig = reboot(rig, &[pilot]);
    let (_, queued) = advance_to(&mut rig, accepted.deadline_ic + 86_401);
    land_quietly(&mut rig, &queued);
    for _ in 0..8 {
        survey_sweep(&mut rig);
        if survey_contract(&rig.tw, accepted.contract) == mush_economy::ContractState::Expired {
            break;
        }
    }
    assert_eq!(
        survey_contract(&rig.tw, accepted.contract),
        mush_economy::ContractState::Expired
    );
    assert_eq!(survey_money(&rig.tw, accepted.payee), payee);
    assert_eq!(
        survey_money(&rig.tw, accepted.sponsor_account),
        (sponsor.0, sponsor.1 - 50_000)
    );
    let renewed = accept_commission(&mut rig, pilot, "juling-k");
    assert_eq!(
        occator_standing(&rig.tw, pilot),
        0,
        "expiry refunds earn no standing"
    );
    assert_eq!(survey_relationship(&rig.tw, &accepted), (0, 0, 0));
    assert_ne!(renewed.contract, accepted.contract);
    assert!(renewed.accepted_ic > accepted.deadline_ic);
    assert_eq!(renewed.covered(), 0);
    assert!(renewed.completed.is_none());
    assert_eq!(
        survey_contract(&rig.tw, accepted.contract),
        mush_economy::ContractState::Expired
    );
    no_refused_writes(&rig.tw);
}

#[test]
fn survey_commission_does_not_offer_an_unfunded_reward() {
    let mut rig = stocked();
    let pilot = pilot(&mut rig, "Pilot", 1);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}=tacsim.credit", pilot.0),
    );
    let sponsor = port_map(&rig.tw, "ceres_port").orgs[&cid("occator")]
        .account
        .expect("Occator's credit account");
    let before = survey_money(&rig.tw, sponsor);
    let reserve = before.0 - before.1 - 49_999;
    assert!(reserve > 0);
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@hold/place {sponsor}={reserve} survey funding fixture"),
    );
    let held = survey_money(&rig.tw, sponsor);
    assert_eq!(held, (before.0, before.1 + reserve), "{out}");
    let contracts = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .unwrap()
        .snapshot()
        .contracts()
        .count();
    for line in ["+survey/accept occator-p", "+survey/accept occator-p go"] {
        let (out, queued) = command(&mut rig, pilot, line);
        assert!(out.contains("cannot fund"), "{out}");
        assert!(
            queued.is_empty(),
            "an unfunded reward cannot be filed: {out}"
        );
        assert!(commission(&mut rig.tw, "occator-p").is_none());
        assert_eq!(survey_money(&rig.tw, sponsor), held);
    }
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .unwrap()
            .snapshot()
            .contracts()
            .count(),
        contracts,
        "insufficient funds do not leave an offered contract behind"
    );
    assert_eq!(occator_standing(&rig.tw, pilot), 0);
    no_refused_writes(&rig.tw);
}

#[test]
fn survey_commission_uncommitted_filing_stays_unfunded_and_expires_after_restart() {
    let mut rig = stocked();
    let pilot = pilot(&mut rig, "Pilot", 1);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}=tacsim.credit", pilot.0),
    );
    let sponsor = port_map(&rig.tw, "ceres_port").orgs[&cid("occator")]
        .account
        .expect("Occator's credit account");
    let before = survey_money(&rig.tw, sponsor);
    let (out, queued) = command(&mut rig, pilot, "+survey/accept occator-p go");
    assert_eq!(queued.len(), 1, "{out}");
    let (contract, deadline) = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .unwrap()
        .snapshot()
        .contracts()
        .filter(|contract| contract.name.as_str() == "Occator survey: occator-p")
        .map(|contract| {
            (
                contract.id.0,
                contract.consent.deadline.as_ref().expect("a deadline").at,
            )
        })
        .next()
        .expect("the command offered a contract");
    assert_eq!(
        survey_contract(&rig.tw, contract),
        mush_economy::ContractState::Offered
    );
    assert!(commission(&mut rig.tw, "occator-p").is_none());
    assert_eq!(survey_money(&rig.tw, sponsor), before);
    rig = reboot(rig, &[pilot]);
    survey_sweep(&mut rig);
    assert_eq!(
        survey_contract(&rig.tw, contract),
        mush_economy::ContractState::Offered,
        "a filing with no durable acceptance never reaches sponsor funding"
    );
    assert_eq!(survey_money(&rig.tw, sponsor), before);
    assert!(
        land(&mut rig, &queued[0]).is_err(),
        "the lost activation cannot revive the orphan"
    );
    let (_, queued) = advance_to(&mut rig, deadline + 1);
    land_quietly(&mut rig, &queued);
    for _ in 0..8 {
        survey_sweep(&mut rig);
        if survey_contract(&rig.tw, contract) == mush_economy::ContractState::Expired {
            break;
        }
    }
    assert_eq!(
        survey_contract(&rig.tw, contract),
        mush_economy::ContractState::Expired
    );
    assert!(commission(&mut rig.tw, "occator-p").is_none());
    assert_eq!(survey_money(&rig.tw, sponsor), before);
    let retry = accept_commission(&mut rig, pilot, "occator-p");
    assert_eq!(
        occator_standing(&rig.tw, pilot),
        0,
        "an orphaned filing proves no work"
    );
    assert_ne!(retry.contract, contract);
    no_refused_writes(&rig.tw);
}

#[test]
fn survey_commission_refuses_generic_amendment_but_reopens_after_unfunded_cancellation() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, tv);

    // The pilot's filing is durable, but Occator has not yet accepted its frozen terms.
    let (out, queued) = command(&mut rig, pilot, "+survey/accept ahuna-a go");
    assert_eq!(queued.len(), 1, "{out}");
    land_all(&mut rig, &queued);
    let original = commission(&mut rig.tw, "ahuna-a").expect("a durable filing");
    assert_eq!(
        survey_contract(&rig.tw, original.contract),
        mush_economy::ContractState::Offered
    );
    let sponsor = survey_money(&rig.tw, original.sponsor_account);
    let payee = survey_money(&rig.tw, original.payee);
    let frozen_contract = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("the economy")
        .snapshot()
        .contract(mush_economy::ContractId(original.contract))
        .expect("the frozen contract")
        .clone();
    let out = run(
        &mut rig.tw,
        pilot,
        &format!("@contract/amend #{}", original.contract),
    );
    assert!(out.contains("No such contract"), "{out}");
    assert_eq!(
        commission(&mut rig.tw, "ahuna-a"),
        Some(original.clone()),
        "the refused amendment does not change the commissioned work"
    );
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .expect("the economy")
            .snapshot()
            .contract(mush_economy::ContractId(original.contract)),
        Some(&frozen_contract),
        "the contract, its consent and its acceptances remain unchanged"
    );
    assert_eq!(survey_money(&rig.tw, original.sponsor_account), sponsor);
    assert_eq!(survey_money(&rig.tw, original.payee), payee);

    let out = run(
        &mut rig.tw,
        pilot,
        &format!("@contract/cancel #{}", original.contract),
    );
    survey_sweep(&mut rig);
    assert_eq!(
        survey_contract(&rig.tw, original.contract),
        mush_economy::ContractState::Cancelled,
        "{out}"
    );
    assert_eq!(survey_money(&rig.tw, original.sponsor_account), sponsor);
    assert_eq!(survey_money(&rig.tw, original.payee), payee);
    let economy = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("the economy");
    let contract = economy
        .snapshot()
        .contract(mush_economy::ContractId(original.contract))
        .unwrap();
    assert!(contract.pending.is_none());
    assert!(contract.escrow.iter().all(|id| {
        economy
            .snapshot()
            .hold(*id)
            .is_some_and(|hold| !hold.status.is_held())
    }));

    let renewed = accept_commission(&mut rig, pilot, "ahuna-a");
    assert_ne!(renewed.contract, original.contract);
    assert_eq!(renewed.covered(), 0);
    assert!(renewed.completed.is_none());
    assert_eq!(
        survey_contract(&rig.tw, original.contract),
        mush_economy::ContractState::Cancelled
    );
    survey_sound(&mut rig, pilot, tv, (48, 208), false);
    assert_eq!(commission(&mut rig.tw, "ahuna-a").unwrap().covered(), 64);
    assert_eq!(survey_money(&rig.tw, renewed.payee), payee);
    assert_eq!(occator_standing(&rig.tw, pilot), 0);
    no_refused_writes(&rig.tw);
}
