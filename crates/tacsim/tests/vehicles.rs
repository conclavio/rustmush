//! **Doc 20 cut SM.3a acceptance — the vehicle and its carrier** (doc 20 §1 beat 1, §4.1,
//! §4.2, §4.3, §11; D-20-6, D-20-7; Q-20-6, Q-20-8; the SM.3a brief's acceptance A1–A8 and
//! decisions D1–D20).
//!
//! Lane v10 lays up Occator Water & Propellant's eight terrain vehicles in a staff-only shed
//! (each a unit, a cab and a hopper riding it, a hopper vault the system principal
//! `tacsim.vehicles` owns, a placement in the shed and a title to Occator) and a vehicle bay on
//! every programme hull; `+coop/refit` re-binds a chartered hull between the Dray and the
//! Dray-P in place; `+tv/lease` leases a rig from Occator's counter into the bay (an economy
//! lease, a public fee post, an engine-owned deposit) and `+tv/<n> lease return` hands it
//! back; a Dray-P with rigs aboard plans as a Dray at equal payload; a charter that ends with
//! rigs aboard repossesses them.
//!
//! Composed on the composition root (`sites.rs`'s rig: `commerce.rs`'s `Roster`, the test
//! world's layer with the game content, `voyage.rs`'s helpers). Every boot asserts no
//! namespace write was refused; every drain asserts no kept tacsim submission is GOD's. The
//! world tests run at world rate 1, so every IC instant a test names is the instant the world
//! reads.

mod support;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use mush_content::parse_pack;
use mush_core::{
    AccountId, AccountIdentity, ActionStateBootRecord, Audience, AuthoritativeSubmit, CanonicalId,
    CommitToken, GOD, IdempotencyKey, IdempotencyScope, ModuleWrite, ObjId, OfferExecutionContext,
    OfferSubmission, RequestUid, ScopedIdempotencyKey,
};
use mush_economy::{
    ContractId, EconomyConfig, EconomyStore, HoldId, HoldPurpose, HoldStatus, VaultId,
};
use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
use mush_org::{OrgId, PrincipalId, SystemPrincipalId};
use mush_proto::OutputEvent;
use mush_sim::data::{SpaceAnchor, SpatialParent, SpatialState};
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde_json::Value;
use smol_str::SmolStr;
use tacsim::config::PackSource;
use tacsim::definitions::content::{
    DRAY, DRAY_P, LANE_JSON, LANE_PACK, LANE_VERSION, LINE_TANK, TV_PROSPECTOR,
};
use tacsim::pack::{PackEntry, RulesPack, load_pack};
use tacsim::port::apply::room_by_key;
use tacsim::port::refit::{
    REFIT_ALREADY, REFIT_BAY, REFIT_FAILED, REFIT_HOLD, REFIT_NOT_HERE, REFIT_NOT_OFFERED,
    REFIT_UNDER_WAY, REFIT_USAGE, REFIT_WHERE, REFITTED_PROSPECTOR, REFITTED_STANDARD,
};
use tacsim::port::tv::{
    BAY_FULL, HOPPER_FULL, LEASE_FAILED, LEASE_FUNDS, LEASE_NOT_HERE, LEASE_PREVIEW,
    LEASE_UNDER_WAY, LEASE_USAGE, LEASE_WHERE, LEASED, NO_BAY, NO_RIG, NO_RIG_FREE, REPOSSESSED,
    REPOSSESSED_FORFEIT, RETURN_PREVIEW, RETURN_PREVIEW_FORFEIT, RETURNED, RETURNED_FORFEIT,
    RIG_EMPTY, RIG_FORFEIT, RIG_LEASE_ENDED, RIG_LINE, RIG_OUT, RIG_PAST_TERM, SLOT_TAKEN,
    TERM_FORFEIT, TERM_RENEWED, TV_DARK, TvLease, TvLeaseState, TvLeaseStatus, WHICH_RIG,
};
use tacsim::port::voyage::{Voyage, VoyageState};
use tacsim::port::{
    Binding, HullRow, LineState, PortDef, PortHandles, PortMap, PortMapState, RigRow, Ticket,
    TicketId, room_key,
};
use tacsim::{TacsimConfig, TacsimStore};

use support::ground::{UNSIGNED, sign};
use support::vehicles::strip_vehicles;

/// Lane v9's published digest (doc 20 cut SM.2), which `strip_vehicles` rebuilds.
const LANE_V9_DIGEST: &str = "cce73a4c60ed6213caaeed321d61926be526cb36c6794ee09e2b60891531402c";

/// IC seconds in an IC day.
const DAY: i64 = 86_400;

/// The term fee and the deposit as the economy renders them.
const FEE: &str = "1400.00cr";
const DEPOSIT: &str = "5000.00cr";
/// The term fee and the deposit, minor units.
const FEE_MINOR: i64 = 140_000;
const DEPOSIT_MINOR: i64 = 500_000;

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

fn lane_id() -> CanonicalId {
    cid(LANE_PACK)
}

fn lane() -> RulesPack {
    load_pack(LANE_JSON).expect("the lane verifies")
}

/// The lane's bytes as JSON, edited by `edit`, at `version`, re-signed.
fn lane_variant(version: u32, edit: impl FnOnce(&mut Value)) -> String {
    let mut doc: Value = serde_json::from_str(LANE_JSON).expect("the lane is JSON");
    doc["manifest"]["version"] = serde_json::json!(version);
    doc["manifest"]["digest"] = serde_json::json!(UNSIGNED);
    edit(&mut doc);
    sign(&serde_json::to_string_pretty(&doc).expect("serializes"))
}

fn port_def(entry: &str) -> PortDef {
    match lane().entry(&cid(entry)) {
        Some(PackEntry::Port(def)) => def.clone(),
        other => panic!("{entry} is a port entry: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// The rig (sites.rs's: commerce.rs's Roster, voyage.rs's helpers)
// ---------------------------------------------------------------------------

/// How a test roster is composed: `commerce.rs`'s (no sheet, the test world's layer, the
/// shipped game content), an operator lane in place of the shipped one, a shelf, or an extra
/// lane version beside the shipped one.
#[derive(Clone, Default)]
struct Roster {
    seed_lane: Option<String>,
    shelf: Option<Vec<mush_content::Pack>>,
    extra_lane: Option<String>,
    /// `[tacsim.time.ephemeris].rate`: 0 (the default) holds the sky constant.
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
            let mut full =
                tacsim::definitions::content::station_packs().expect("the station packs");
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
            None => mods
                .push(comp::content_module_with(true, true).expect("the content module composes")),
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

/// A booted world, its roster, its oracle and the cumulative store.
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
            mush_core::OfferSecret::new(1, [0x52; 32]).expect("an offer secret"),
            [],
        )
        .expect("an offer key ring"),
    );
}

/// Boot `roster` without settling the capital or firing the port tick.
fn boot_unsettled(roster: Roster) -> Rig {
    let mut tw = TestWorld::with_modules(roster.modules());
    no_refused_writes(&tw);
    with_offer_keys(&mut tw);
    let oracle = Arc::new(MapOracle::default());
    install_oracle(&mut tw, &oracle);
    Rig {
        tw,
        oracle,
        roster,
        store: HashMap::new(),
        typed: BTreeMap::new(),
    }
}

/// Boot and sweep once, so the capital settles.
fn boot(roster: Roster) -> Rig {
    let mut rig = boot_unsettled(roster);
    sweep(&mut rig.tw, 120);
    rig
}

/// Boot, settle and stock every port, at world rate 1 (every IC instant exact).
fn stocked() -> Rig {
    stocked_on(Roster::bare())
}

/// [`stocked`] on `roster`.
fn stocked_on(roster: Roster) -> Rig {
    let mut rig = boot(roster);
    stock(&mut rig);
    set_rate(&mut rig.tw, 1);
    rig
}

/// Fold every write since the first boot and reboot into the rig's roster.
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

fn port_map(tw: &TestWorld, entry: &str) -> PortMap {
    match store(tw).port_map(&lane_id(), &cid(entry)) {
        PortMapState::Present(map) => map.clone(),
        other => panic!("{entry}'s map: {other:?}"),
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

/// Drive one queued submission the way the world task does, keeping its typed post-images
/// for a reboot.
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

/// Every queued submission of the port's lanes, dropping other modules'; asserts none kept of
/// tacsim's is GOD's.
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

/// Fire and land the port tick until every port is stocked and listed.
fn stock(rig: &mut Rig) {
    for _ in 0..40 {
        let out = status(&mut rig.tw);
        let gated: Vec<&str> = out
            .lines()
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

/// An enrolled pilot whose ship is named `name`.
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
    (pilot, row)
}

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

/// Seed `qty` of `good` into `vault`'s pool through `lot` (additive).
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

/// `cargo_t` tonnes of metal concentrate, `tank` t and `locker` crew-days into the hold.
fn load(rig: &mut Rig, row: &HullRow, cargo_t: u64, tank: u64, locker: u64, lot: u64) {
    let vault = hold_vault(&rig.tw, row);
    if cargo_t > 0 {
        seed_pool(rig, vault, "metals", cargo_t, LotId(lot));
    }
    if tank > 0 {
        seed_pool(rig, vault, "propellant", tank, LotId(lot + 1));
    }
    if locker > 0 {
        seed_pool(rig, vault, "provisions", locker, LotId(lot + 2));
    }
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

/// Sweep to the first real second whose IC instant is at or past `target`, landing the port's
/// background submissions; the events.
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

/// A command and the submissions it queued.
fn command(rig: &mut Rig, who: ObjId, line: &str) -> (String, Vec<AuthoritativeSubmit>) {
    let out = run(&mut rig.tw, who, line);
    let queued = drain(&mut rig.tw);
    (out, queued)
}

/// File a passage with `line`, land its burn, launch it on the next fire: what the pilot
/// heard at the launch, and the ticket.
fn file_and_launch(rig: &mut Rig, pilot: ObjId, hull: ObjId, line: &str) -> (String, Ticket) {
    let (out, queued) = command(rig, pilot, line);
    assert!(out.contains("Clearance filed for "), "{line}: {out}");
    assert_eq!(queued.len(), 1, "one burn: {out}");
    assert_eq!(queued[0].actor, pilot, "the burn is the pilot's");
    let seq = voyage(&rig.tw, hull)
        .and_then(|v| v.pending)
        .map(|p| p.seq)
        .expect("a filed passage");
    land_all(rig, &queued);
    let t = ticket(&mut rig.tw, hull, seq).expect("the ticket");
    let events = fire_quietly(rig);
    let heard = said(&rig.tw, &events, pilot);
    (heard, t)
}

/// The arrival instant of a ticket's plan.
fn arrival_of(t: &Ticket) -> i64 {
    t.plan.depart_ic + t.plan.total_s
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

// ---------------------------------------------------------------------------
// The rigs, the refit and the lease (this cut's helpers)
// ---------------------------------------------------------------------------

/// The Registry & Co-op desk (C05).
fn desk(tw: &TestWorld) -> ObjId {
    station_room(tw, "ceres_port", "room.c05_registry")
}

/// Occator's counter: the Exchange Hall (C07; decision D11).
fn counter(tw: &TestWorld) -> ObjId {
    station_room(tw, "ceres_port", "room.c07_exchange")
}

fn name_of(tw: &TestWorld, obj: ObjId) -> String {
    tw.world()
        .get(obj)
        .map(|o| o.name.clone())
        .unwrap_or_default()
}

/// `{counter}` and `{port}` filled, as the texts read them.
fn at_counter(tw: &TestWorld, template: &str) -> String {
    template
        .replace("{counter}", &name_of(tw, counter(tw)))
        .replace("{port}", "Ceres Freeport")
}

/// `+coop/refit <word>` at the desk.
fn refit(rig: &mut Rig, pilot: ObjId, word: &str) -> String {
    let desk = desk(&rig.tw);
    teleport(&mut rig.tw, pilot, desk);
    run(&mut rig.tw, pilot, &format!("+coop/refit {word}"))
}

/// A chartered pilot whose hull the yard has made a Dray-P.
fn prospector(rig: &mut Rig, who: &str, seed: i64, name: &str) -> (ObjId, HullRow) {
    let (pilot, row) = chartered(rig, who, seed, name);
    let out = refit(rig, pilot, "prospector");
    assert!(out.contains("as a Dray-P"), "{out}");
    (pilot, row)
}

/// `line` at Occator's counter.
fn at_occator(rig: &mut Rig, pilot: ObjId, line: &str) -> String {
    let counter = counter(&rig.tw);
    teleport(&mut rig.tw, pilot, counter);
    run(&mut rig.tw, pilot, line)
}

/// Lease one rig with `+tv/lease go` at the counter, asserting the lease.
fn lease_one(rig: &mut Rig, pilot: ObjId) -> String {
    let out = at_occator(rig, pilot, "+tv/lease go");
    assert!(
        out.contains("Occator Water & Propellant leases you "),
        "{out}"
    );
    out
}

/// Occator's rig rows.
fn rigs(tw: &TestWorld) -> Vec<RigRow> {
    port_map(tw, "ceres_port")
        .vehicles
        .expect("the rig pool's row")
        .rigs
}

/// Occator's shed.
fn shed(tw: &TestWorld) -> ObjId {
    ObjId(
        port_map(tw, "ceres_port")
            .vehicles
            .and_then(|row| row.shed)
            .expect("the shed"),
    )
}

/// The rig whose mark is `mark`.
fn rig_named(tw: &TestWorld, mark: &str) -> RigRow {
    rigs(tw)
        .into_iter()
        .find(|rig| rig.mark == mark)
        .expect("a rig of the pool")
}

/// The unit definition `obj` is bound to.
fn unit_id(tw: &TestWorld, obj: ObjId) -> String {
    mush_tactics::module::unit_of(tw.world(), obj)
        .map(|unit| unit.schema().key().id.as_str().to_owned())
        .expect("a live unit")
}

fn parent(tw: &TestWorld, obj: ObjId) -> SpatialParent {
    tw.world()
        .module_state::<SpatialState>()
        .expect("the simulation")
        .parent_of(obj)
}

/// Whether `tv` stands in `bay`'s `rigs` compartment.
fn in_bay(tw: &TestWorld, tv: ObjId, bay: ObjId) -> bool {
    matches!(parent(tw, tv), SpatialParent::Interior { space, slot: Some(slot) }
        if space == bay && slot.as_str() == "rigs")
}

/// Whether `tv` stands in Occator's shed.
fn in_shed(tw: &TestWorld, tv: ObjId) -> bool {
    matches!(parent(tw, tv), SpatialParent::Interior { space, slot: None } if space == shed(tw))
}

/// Every readable lease record of `pilot`, any status.
fn records(tw: &TestWorld, pilot: ObjId) -> Vec<TvLease> {
    store(tw).tv_leases_of(pilot).cloned().collect()
}

/// The newest record of `pilot`'s lease on `tv`, any status.
fn record_on(tw: &TestWorld, pilot: ObjId, tv: ObjId) -> TvLease {
    records(tw, pilot)
        .into_iter()
        .filter(|record| record.tv == tv)
        .max_by_key(|record| record.lease)
        .expect("a lease record")
}

/// The economy state of a contract.
fn contract_terminal(tw: &TestWorld, id: u64) -> bool {
    economy(tw)
        .snapshot()
        .contract(ContractId(id))
        .is_some_and(|c| c.state.is_terminal())
}

/// The pilot's credit account.
fn account_of(tw: &TestWorld, who: ObjId) -> mush_economy::AccountId {
    tacsim::port::credit::own_account(tw.world(), who)
        .expect("the pilot's account")
        .id
}

/// `(posted, held)` of an account, minor units.
fn balances(tw: &TestWorld, account: mush_economy::AccountId) -> (i64, i64) {
    economy(tw)
        .snapshot()
        .account(account)
        .map(|a| (a.posted_balance.minor(), a.held_amount.minor()))
        .expect("the account")
}

/// Occator's account.
fn occator_account(tw: &TestWorld) -> mush_economy::AccountId {
    mush_economy::AccountId(
        port_map(tw, "ceres_port").orgs[&cid("occator")]
            .account
            .expect("Occator's account"),
    )
}

/// Occator as a principal.
fn occator(tw: &TestWorld) -> PrincipalId {
    PrincipalId::Organization(OrgId(port_map(tw, "ceres_port").orgs[&cid("occator")].org))
}

/// The status of hold `id`.
fn hold_status(tw: &TestWorld, id: u64) -> HoldStatus {
    economy(tw)
        .snapshot()
        .hold(HoldId(id))
        .map(|hold| hold.status.clone())
        .expect("the hold")
}

/// Hold every available minor unit of `who`'s account, as `who` (`@hold/place`); its id.
fn drain_funds(rig: &mut Rig, who: ObjId) -> u64 {
    let account = account_of(&rig.tw, who);
    let (posted, held) = balances(&rig.tw, account);
    let available = posted - held;
    let out = run(
        &mut rig.tw,
        who,
        &format!("@hold/place #{}={available} test drain", account.0),
    );
    let (_, after) = balances(&rig.tw, account);
    assert_eq!(after, posted, "{out}");
    economy(&rig.tw)
        .snapshot()
        .holds_of(account)
        .filter(|hold| matches!(hold.purpose, HoldPurpose::Manual { .. }))
        .map(|hold| hold.id.0)
        .max()
        .expect("the drain hold")
}

/// Release a drain hold, as `who`.
fn undrain(rig: &mut Rig, who: ObjId, hold: u64) {
    let out = run(&mut rig.tw, who, &format!("@hold/release {hold}"));
    assert_eq!(hold_status(&rig.tw, hold), HoldStatus::Released, "{out}");
}

/// Move the world `days` IC days on without firing a tick (at rate 1, real days too: no
/// recall runs unless a test fires one).
fn skip_days(rig: &mut Rig, days: i64) {
    let t = rig.tw.world().now().saturating_add(days * DAY);
    rig.tw.set_now(t);
}

/// `tw.run`, with the events (so another listener's lines can be read).
fn heard_by(rig: &mut Rig, who: ObjId, line: &str, listener: ObjId) -> (String, String) {
    let events = rig.tw.run(who, line);
    (
        said(&rig.tw, &events, who),
        said(&rig.tw, &events, listener),
    )
}

/// The expected take line.
fn leased_line(template: &str, mark: &str, name: &str, n: u8) -> String {
    template
        .replace("{mark}", mark)
        .replace("{name}", name)
        .replace("{n}", &n.to_string())
        .replace("{fee}", FEE)
        .replace("{days}", "168")
        .replace("{deposit}", DEPOSIT)
}

/// A rig's hopper vault.
fn hopper(tw: &TestWorld, tv: ObjId) -> VaultId {
    tacsim::port::vehicle::hopper_of(tw.world(), tv).expect("a hopper vault")
}

/// Staff put `t` tonnes of metal ore in `tv`'s hopper through the inventory's own path.
fn fill_hopper(rig: &mut Rig, tv: ObjId, t: u64, lot: u64) {
    let vault = hopper(&rig.tw, tv);
    seed_pool(rig, vault, "metal_ore", t, LotId(lot));
}

/// The bay room of a hull row.
fn bay_of(row: &HullRow) -> ObjId {
    room_of(row, "bay")
}

/// The pilot's live charter's Trust lease.
fn trust_lease(tw: &TestWorld, pilot: ObjId) -> u64 {
    store(tw).enrolment_of(pilot).expect("an enrolment").lease
}

/// A `MakeWriter` that collects formatted log lines (`boot_parity.rs`'s).
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("the capture buffer")).into_owned()
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

/// A dispatcher writing ERROR and up into `capture` (`boot_parity.rs`'s registration anchor).
fn capture_dispatch(capture: Capture) -> tracing::Dispatch {
    static ANCHOR: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
    ANCHOR.get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
    tracing::Dispatch::new(
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::ERROR)
            .with_writer(capture)
            .finish(),
    )
}

fn treasury_account(tw: &TestWorld) -> u64 {
    port_map(tw, "ceres_port").treasury.expect("a treasury")
}

/// Mint `minor` into `who`'s own credit account from the treasury (`commerce.rs`'s).
fn top_up(tw: &mut TestWorld, who: ObjId, minor: i64) {
    let account = account_of(tw, who);
    let treasury = treasury_account(tw);
    let out = run(
        tw,
        GOD,
        &format!("@econ/mint {treasury}={} {minor}", account.0),
    );
    assert!(!out.contains("Usage"), "{out}");
}

// ---------------------------------------------------------------------------
// 1. The applier
// ---------------------------------------------------------------------------

/// **The apply lays up Occator's rigs and every hull's bay** (part h; part f step 6b):
/// eight rigs `OWP-TV-01..08`, each a live Thing in the shed with a unit bound to the
/// terrain vehicle, a cab and a hopper space riding it, a hopper vault owned by
/// `System(tacsim.vehicles)` with no scope, placed `Interior` in the shed and titled Active
/// to Occator; every programme hull's bay recorded, a space riding the hull with `rigs` at
/// two; the vehicles line Ready with its text and never gated; an update of the entry makes
/// nothing; the hulls line still Ready.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3a_the_apply_lays_up_occators_rigs_and_every_hulls_bay() {
    let next = lane_variant(LANE_VERSION + 1, |doc| {
        doc["entries"]["ceres_port"]["def"]["programme"]["dormancy_real_days"] =
            serde_json::json!(8);
    });
    let roster = Roster {
        shelf: Some(vec![
            mush_content::load_pack(&next).expect("the variant verifies"),
        ]),
        extra_lane: Some(next),
        ..Roster::default()
    };
    let mut rig = stocked_on(roster);
    let tw = &rig.tw;
    let shed = shed(tw);
    assert_eq!(name_of(tw, shed), "Occator Vehicle Shed");
    let shed_record = &tw
        .world()
        .module_state::<SpatialState>()
        .expect("the simulation")
        .spaces[&shed];
    assert_eq!(shed_record.anchor, SpaceAnchor::Unanchored);
    let occator_org = occator(tw);
    let owner = PrincipalId::System(SystemPrincipalId::new("tacsim.vehicles").expect("an id"));
    let rows = rigs(tw);
    assert_eq!(rows.len(), 8);
    for (i, row) in rows.iter().enumerate() {
        let n = u32::try_from(i).expect("small");
        assert_eq!(row.n, n);
        assert_eq!(row.mark, format!("OWP-TV-{:02}", n + 1));
        let tv = ObjId(row.tv);
        let obj = tw.world().get(tv).expect("the rig");
        assert_eq!(obj.kind, mush_core::ObjKind::Thing);
        assert_eq!(obj.name, row.mark);
        assert_eq!(obj.owner, GOD, "the applying actor, never a pilot");
        assert_eq!(unit_id(tw, tv), TV_PROSPECTOR);
        let st = tw.world().module_state::<SpatialState>().expect("sim");
        for room in ["cab", "hopper"] {
            let space = room_of_rig(row, room);
            assert_eq!(
                st.spaces[&space].anchor,
                SpaceAnchor::MovingObject { host: tv },
                "{} {room}",
                row.mark
            );
        }
        let berths = mush_sim::SlotId::new("berths").expect("a slot");
        assert_eq!(
            st.spaces[&room_of_rig(row, "cab")].compartments[&berths].maximum_items,
            Some(1)
        );
        let vault = economy(tw)
            .snapshot()
            .vault(VaultId(row.vault.expect("a hopper vault")))
            .cloned()
            .expect("the vault");
        assert_eq!(vault.owner, owner);
        assert_eq!(vault.scope, None);
        assert_eq!(vault.name.as_str(), format!("{} hopper", row.mark));
        assert_eq!(
            vault.backing,
            mush_economy::CustodyRef::SpatialInterior(room_of_rig(row, "hopper"))
        );
        assert!(in_shed(tw, tv), "{} stands in the shed", row.mark);
        let asset = economy(tw).snapshot().asset(tv).cloned().expect("titled");
        assert_eq!(asset.title_owner, occator_org);
        assert_eq!(asset.lifecycle, mush_economy::AssetLifecycle::Active);
    }
    let rigs_slot = mush_sim::SlotId::new("rigs").expect("a slot");
    for row in &port_map(tw, "ceres_port").hulls {
        let bay = bay_of(row);
        let st = tw.world().module_state::<SpatialState>().expect("sim");
        assert_eq!(
            st.spaces[&bay].anchor,
            SpaceAnchor::MovingObject {
                host: ObjId(row.hull)
            }
        );
        assert_eq!(
            st.spaces[&bay].compartments[&rigs_slot].maximum_items,
            Some(2)
        );
    }
    // The readiness lines: the vehicles line Ready, reported and never gated.
    let report = tacsim::port::readiness(tw.world());
    let line = report
        .lines
        .iter()
        .find(|line| line.binding == Binding::Vehicles)
        .expect("the vehicles line");
    assert_eq!(line.state, LineState::Ready);
    assert_eq!(
        line.text,
        "ceres_port vehicles: 8 of 8 made and titled to Occator Water & Propellant; 8 in the \
         shed, 0 leased."
    );
    assert!(!tacsim::port::gated(Binding::Vehicles));
    assert!(tacsim::port::enrol_gate(&report).is_ok());
    let hulls = report
        .lines
        .iter()
        .find(|line| line.binding == Binding::Hulls)
        .expect("the hulls line");
    assert_eq!(hulls.state, LineState::Ready);

    // An update of the entry makes nothing more: objects, vaults and titles unchanged.
    let objects = rig.tw.world().iter().count();
    let vaults = economy(&rig.tw).snapshot().vaults().count();
    let assets = economy(&rig.tw).snapshot().assets().count();
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@{}", LANE_VERSION + 1)),
        "{plan}\n{apply}"
    );
    assert_eq!(rig.tw.world().iter().count(), objects, "no new object");
    assert_eq!(economy(&rig.tw).snapshot().vaults().count(), vaults);
    assert_eq!(economy(&rig.tw).snapshot().assets().count(), assets);
    assert_eq!(rigs(&rig.tw), rows, "the rig rows stand");
    no_refused_writes(&rig.tw);
}

/// A rig's room `id`.
fn room_of_rig(row: &RigRow, id: &str) -> ObjId {
    ObjId(row.rooms[&cid(id)])
}

// ---------------------------------------------------------------------------
// 2. No player reaches a hopper (A4)
// ---------------------------------------------------------------------------

/// **No player may take from a hopper** (A4; §4.2): `live_endpoints` never lists a hopper
/// for the lessee (with a rig leased) or a stranger; `may_take` is false for both and true
/// for staff; the lessee's `@hold/release` of the deposit hold is refused and the hold stays
/// Held.
#[test]
fn sm3a_no_player_may_take_from_a_hopper() {
    let mut rig = stocked();
    let (pilot, _) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    lease_one(&mut rig, pilot);
    let stranger = rig.tw.spawn_player("Stranger");
    let hoppers: Vec<VaultId> = rigs(&rig.tw)
        .iter()
        .map(|row| VaultId(row.vault.expect("a hopper vault")))
        .collect();
    for who in [pilot, stranger] {
        let endpoints = mush_inventory::provider::live_endpoints(rig.tw.world(), who);
        for vault in &hoppers {
            assert!(!endpoints.contains(vault), "#{} reaches a hopper", who.0);
            assert!(!mush_economy::may_take_from_vault(
                rig.tw.world(),
                who,
                *vault
            ));
        }
    }
    for vault in &hoppers {
        assert!(mush_economy::may_take_from_vault(
            rig.tw.world(),
            GOD,
            *vault
        ));
    }
    let record = records(&rig.tw, pilot).pop().expect("the lease record");
    let out = run(
        &mut rig.tw,
        pilot,
        &format!("@hold/release {}", record.hold),
    );
    assert_eq!(hold_status(&rig.tw, record.hold), HoldStatus::Held, "{out}");
}

// ---------------------------------------------------------------------------
// 3. The refit (A5)
// ---------------------------------------------------------------------------

/// **The refit, there and back** (D1): with only the tank aboard the yard re-binds the hull
/// to the Dray-P with its exact line (86 t, 103.2 m³), `+ship` reads `(Dray-P)` and the hold
/// line's new limits; `standard` brings it back (100 t, 120 m³); `REFIT_ALREADY` both ways;
/// the enrolment document is unchanged.
#[test]
fn sm3a_refit_prospector_and_back() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    load(&mut rig, &row, 0, 10, 0, 9_100);
    let before = store(&rig.tw).enrolment_of(pilot).cloned();
    let out = refit(&mut rig, pilot, "prospector");
    let want = REFITTED_PROSPECTOR
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark)
        .replace("{tonnes}", "86")
        .replace("{m3}", "103.2")
        .replace("{countersign}", tacsim::port::coop::COUNTERSIGN);
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    assert_eq!(unit_id(&rig.tw, hull), DRAY_P);
    let ship = run(&mut rig.tw, pilot, "+ship");
    assert!(ship.contains("Salt Moth (Dray-P): docked "), "{ship}");
    // The gauge prints a limit's cubic metres whole (103.2 m³ reads 103).
    assert!(ship.contains("cargo 0/86 t · 0.0/103 m³"), "{ship}");
    let again = refit(&mut rig, pilot, "prospector");
    assert!(
        again.contains("The Salt Moth is already a Dray-P."),
        "{again}"
    );
    let out = refit(&mut rig, pilot, "standard");
    let want = REFITTED_STANDARD
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark)
        .replace("{tonnes}", "100")
        .replace("{m3}", "120.0")
        .replace("{countersign}", tacsim::port::coop::COUNTERSIGN);
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    assert_eq!(unit_id(&rig.tw, hull), DRAY);
    let ship = run(&mut rig.tw, pilot, "+ship");
    assert!(ship.contains("Salt Moth (Dray): docked "), "{ship}");
    assert!(ship.contains("cargo 0/100 t · 0.0/120 m³"), "{ship}");
    let again = refit(&mut rig, pilot, "standard");
    assert!(
        again.contains(
            &REFIT_ALREADY
                .replace("{name}", "Salt Moth")
                .replace("{class}", "standard Dray")
        ),
        "{again}"
    );
    assert_eq!(
        store(&rig.tw).enrolment_of(pilot).cloned(),
        before,
        "the enrolment document is unchanged"
    );
    no_refused_writes(&rig.tw);
}

/// **The refit refuses** (A5): away from the desk, with cargo, with provisions, with a rig
/// in the bay and under way — each with its exact line, and nothing written (the unit
/// unchanged).
#[test]
#[allow(clippy::too_many_lines)]
fn sm3a_refit_refuses_under_way_with_cargo_with_provisions_with_a_rig_and_away_from_the_desk() {
    let mut rig = stocked();
    // Away from the desk.
    let (a, row_a) = chartered(&mut rig, "Ann", 1, "Salt Moth");
    let out = at_occator(&mut rig, a, "+coop/refit prospector");
    let want = REFIT_WHERE
        .replace("{office}", &name_of(&rig.tw, desk(&rig.tw)))
        .replace("{port}", "Ceres Freeport");
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    assert_eq!(unit_id(&rig.tw, ObjId(row_a.hull)), DRAY);
    // Cargo.
    let (b, row_b) = chartered(&mut rig, "Bea", 2, "Tally Moth");
    load(&mut rig, &row_b, 1, 0, 0, 9_200);
    let out = refit(&mut rig, b, "prospector");
    assert!(
        out.contains(&REFIT_HOLD.replace("{name}", "Tally Moth")),
        "{out}"
    );
    assert_eq!(unit_id(&rig.tw, ObjId(row_b.hull)), DRAY);
    // Provisions.
    let (c, row_c) = chartered(&mut rig, "Cal", 3, "Cold Moth");
    load(&mut rig, &row_c, 0, 0, 4, 9_300);
    let out = refit(&mut rig, c, "prospector");
    assert!(
        out.contains(&REFIT_HOLD.replace("{name}", "Cold Moth")),
        "{out}"
    );
    assert_eq!(unit_id(&rig.tw, ObjId(row_c.hull)), DRAY);
    // A rig in the bay.
    let (d, row_d) = prospector(&mut rig, "Dee", 4, "Dry Moth");
    lease_one(&mut rig, d);
    let out = refit(&mut rig, d, "standard");
    assert!(
        out.contains(&REFIT_BAY.replace("{n}", "1").replace("{name}", "Dry Moth")),
        "{out}"
    );
    assert_eq!(unit_id(&rig.tw, ObjId(row_d.hull)), DRAY_P);
    // Under way, bound for Ahuna Flank.
    let (e, row_e) = chartered(&mut rig, "Eve", 5, "Even Moth");
    load(&mut rig, &row_e, 0, 20, 20, 9_400);
    board(&mut rig, e);
    let _ = file_and_launch(&mut rig, e, ObjId(row_e.hull), "+ship/undock ahuna");
    let out = run(&mut rig.tw, e, "+coop/refit prospector");
    let want = REFIT_UNDER_WAY
        .replace("{name}", "Even Moth")
        .replace("{port}", "Ceres Freeport");
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    assert_eq!(unit_id(&rig.tw, ObjId(row_e.hull)), DRAY);
    // And settled elsewhere.
    let t = ticket(&mut rig.tw, ObjId(row_e.hull), 1).expect("the ticket");
    let _ = to_ic(&mut rig, arrival_of(&t));
    let out = run(&mut rig.tw, e, "+coop/refit prospector");
    let want = REFIT_NOT_HERE
        .replace("{name}", "Even Moth")
        .replace("{port}", "Ceres Freeport");
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    assert_eq!(unit_id(&rig.tw, ObjId(row_e.hull)), DRAY);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 4. The lease (A2)
// ---------------------------------------------------------------------------

/// **The lease takes two rigs into the bay** (§4.3; D6, D7, D9): a bare `+tv/lease` previews
/// and writes nothing; `go` posts the fee and holds the deposit, leases and places the rig
/// in slot 1 with an Active record; `+tv` reads rig 1 and an empty slot 2; a second `go`
/// takes rig 2, a third is `BAY_FULL`, and `+tv/2/lease go` is `SLOT_TAKEN`.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3a_lease_takes_two_rigs_into_the_bay() {
    let mut rig = stocked();
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    let bay = bay_of(&row);
    let account = account_of(&rig.tw, pilot);
    let (posted, held) = balances(&rig.tw, account);
    let occ = occator_account(&rig.tw);
    let (occ_posted, _) = balances(&rig.tw, occ);
    let first = rig_named(&rig.tw, "OWP-TV-01");
    let tv1 = ObjId(first.tv);

    let out = at_occator(&mut rig, pilot, "+tv/lease");
    let want = leased_line(LEASE_PREVIEW, "OWP-TV-01", "Salt Moth", 1);
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    assert!(
        records(&rig.tw, pilot).is_empty(),
        "the preview signs nothing"
    );
    assert_eq!(balances(&rig.tw, account), (posted, held));
    assert!(
        tacsim::port::vehicle::occator_lease_on(rig.tw.world(), &occator(&rig.tw), tv1).is_none()
    );
    assert!(in_shed(&rig.tw, tv1));

    let out = at_occator(&mut rig, pilot, "+tv/lease go");
    let want = leased_line(LEASED, "OWP-TV-01", "Salt Moth", 1);
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    assert_eq!(
        balances(&rig.tw, account),
        (posted - FEE_MINOR, held + DEPOSIT_MINOR)
    );
    assert_eq!(
        balances(&rig.tw, occ).0,
        occ_posted + FEE_MINOR,
        "the fee is Occator's"
    );
    let lease = tacsim::port::vehicle::occator_lease_on(rig.tw.world(), &occator(&rig.tw), tv1)
        .map(|c| c.id.0)
        .expect("a live lease from Occator pledges the rig");
    assert!(in_bay(&rig.tw, tv1, bay), "rig 1 stands in the bay");
    let record = record_on(&rig.tw, pilot, tv1);
    assert_eq!(record.lease, lease);
    assert_eq!((record.slot, record.status), (1, TvLeaseStatus::Active));
    assert_eq!(record.terms_paid, 1);
    assert_eq!(record.term_ends_ic - record.leased_at_ic, 168 * DAY);
    assert_eq!(hold_status(&rig.tw, record.hold), HoldStatus::Held);

    let status = run(&mut rig.tw, pilot, "+tv");
    let line1 = RIG_LINE
        .replace("{n}", "1")
        .replace("{mark}", "OWP-TV-01")
        .replace("{span}", "168 d 0 h")
        .replace("{hopper}", "empty");
    assert!(status.contains(&line1), "{status}\nwanted: {line1}");
    let line2 = at_counter(&rig.tw, RIG_EMPTY).replace("{n}", "2");
    assert!(status.contains(&line2), "{status}\nwanted: {line2}");

    let out = at_occator(&mut rig, pilot, "+tv/lease go");
    let want = leased_line(LEASED, "OWP-TV-02", "Salt Moth", 2);
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    let tv2 = ObjId(rig_named(&rig.tw, "OWP-TV-02").tv);
    assert!(in_bay(&rig.tw, tv2, bay));
    let out = at_occator(&mut rig, pilot, "+tv/lease go");
    assert!(
        out.contains(&BAY_FULL.replace("{name}", "Salt Moth")),
        "{out}"
    );
    let out = at_occator(&mut rig, pilot, "+tv/2/lease go");
    assert!(
        out.contains(
            &SLOT_TAKEN
                .replace("{n}", "2")
                .replace("{name}", "Salt Moth")
        ),
        "{out}"
    );
    assert_eq!(records(&rig.tw, pilot).len(), 2);
    no_refused_writes(&rig.tw);
}

/// **The lease refuses** (A2): a standard Dray (`NO_BAY`), away from the counter, short of
/// funds, under way and away from Ceres — each with its exact line, and nothing written. It
/// is not refused for want of a licence (the seam is SM.5's).
#[test]
#[allow(clippy::too_many_lines)]
fn sm3a_lease_is_refused_without_a_bay_away_from_ceres_away_from_the_counter_and_short_of_funds() {
    let mut rig = stocked();
    // A standard Dray.
    let (a, _) = chartered(&mut rig, "Ann", 1, "Salt Moth");
    let out = at_occator(&mut rig, a, "+tv/lease go");
    let want = NO_BAY
        .replace("{name}", "Salt Moth")
        .replace("{office}", &name_of(&rig.tw, desk(&rig.tw)))
        .replace("{port}", "Ceres Freeport");
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    // Away from the counter.
    let (b, _) = prospector(&mut rig, "Bea", 2, "Tally Moth");
    let out = run(&mut rig.tw, b, "+tv/lease go");
    assert!(out.contains(&at_counter(&rig.tw, LEASE_WHERE)), "{out}");
    // Short of funds.
    let drained = drain_funds(&mut rig, b);
    let out = at_occator(&mut rig, b, "+tv/lease go");
    let want = LEASE_FUNDS
        .replace("{total}", "6400.00cr")
        .replace("{fee}", FEE)
        .replace("{deposit}", DEPOSIT)
        .replace("{available}", "0.00cr");
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    undrain(&mut rig, b, drained);
    assert!(records(&rig.tw, b).is_empty());
    // Under way, then settled on Ahuna Flank's pad.
    let (c, row_c) = prospector(&mut rig, "Cal", 3, "Cold Moth");
    load(&mut rig, &row_c, 0, 20, 20, 9_300);
    board(&mut rig, c);
    let (_, t) = file_and_launch(&mut rig, c, ObjId(row_c.hull), "+ship/undock ahuna");
    let out = run(&mut rig.tw, c, "+tv/lease go");
    let want = LEASE_UNDER_WAY
        .replace("{name}", "Cold Moth")
        .replace("{port}", "Ceres Freeport");
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    let _ = to_ic(&mut rig, arrival_of(&t));
    let out = run(&mut rig.tw, c, "+tv/lease go");
    assert!(
        out.contains(&LEASE_NOT_HERE.replace("{port}", "Ceres Freeport")),
        "{out}"
    );
    for who in [a, b, c] {
        assert!(records(&rig.tw, who).is_empty(), "nothing was written");
    }
    assert!(
        rigs(&rig.tw)
            .iter()
            .all(|row| in_shed(&rig.tw, ObjId(row.tv))),
        "every rig is in the shed"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 5. The return (A3)
// ---------------------------------------------------------------------------

/// **The return refunds the deposit and is refused elsewhere** (A3; reviewer F7): away from
/// the counter, a hopper not empty, no slot named with two rigs, and a rig out of the bay
/// (`RIG_OUT`) each refuse; at the counter with an empty hopper the lease ends, the deposit
/// is released, the rig goes back to the shed and the record is Returned; a fresh lease
/// takes it again.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3a_return_refunds_the_deposit_and_is_refused_elsewhere() {
    let mut rig = stocked();
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    let bay = bay_of(&row);
    lease_one(&mut rig, pilot);
    lease_one(&mut rig, pilot);
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    let tv2 = ObjId(rig_named(&rig.tw, "OWP-TV-02").tv);
    let account = account_of(&rig.tw, pilot);
    // Away from the counter.
    let desk = desk(&rig.tw);
    teleport(&mut rig.tw, pilot, desk);
    let out = run(&mut rig.tw, pilot, "+tv/1 lease return go");
    assert!(out.contains(&at_counter(&rig.tw, LEASE_WHERE)), "{out}");
    // A hopper not empty (staff put 2 t of metal ore in rig 2's).
    fill_hopper(&mut rig, tv2, 2, 9_500);
    let out = at_occator(&mut rig, pilot, "+tv/2 lease return go");
    assert!(out.contains(&HOPPER_FULL.replace("{n}", "2")), "{out}");
    let status = run(&mut rig.tw, pilot, "+tv/2");
    assert!(status.contains("hopper 2 of 12 t"), "{status}");
    // Two rigs and no slot named.
    let out = at_occator(&mut rig, pilot, "+tv/lease return go");
    assert!(out.contains(WHICH_RIG), "{out}");
    // RIG_OUT: staff move rig 1 out of the bay, then back.
    let shed = shed(&rig.tw);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@spatial/place/interior #{}=#{}", tv1.0, shed.0),
    );
    assert!(in_shed(&rig.tw, tv1), "staff moved rig 1");
    let out = at_occator(&mut rig, pilot, "+tv/1 lease return go");
    let want = RIG_OUT.replace("{n}", "1").replace("{name}", "Salt Moth");
    assert_eq!(out.trim(), want, "{out}");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@spatial/place/interior #{}=#{},rigs", tv1.0, bay.0),
    );
    assert!(in_bay(&rig.tw, tv1, bay), "staff put it back");
    // The return.
    let (posted, held) = balances(&rig.tw, account);
    let record = record_on(&rig.tw, pilot, tv1);
    let out = at_occator(&mut rig, pilot, "+tv/1 lease return go");
    let want = RETURNED
        .replace("{mark}", "OWP-TV-01")
        .replace("{deposit}", DEPOSIT);
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    assert!(
        contract_terminal(&rig.tw, record.lease),
        "the lease is terminal"
    );
    assert_eq!(hold_status(&rig.tw, record.hold), HoldStatus::Released);
    assert_eq!(balances(&rig.tw, account), (posted, held - DEPOSIT_MINOR));
    assert!(in_shed(&rig.tw, tv1));
    let after = record_on(&rig.tw, pilot, tv1);
    assert_eq!(after.status, TvLeaseStatus::Returned);
    assert!(after.ended_at_ic.is_some() && !after.repossessed);
    // A fresh lease takes it again, into slot 1.
    let out = at_occator(&mut rig, pilot, "+tv/lease go");
    let want = leased_line(LEASED, "OWP-TV-01", "Salt Moth", 1);
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    no_refused_writes(&rig.tw);
}

/// **A revoked pilot can still return a rig** (reviewer F1(b)): staff cancel the Trust's
/// lease without a tick; `+tv` still reads the rig (the hull from the record) and the return
/// at the counter succeeds; the return arm's next fire then takes the hull back with no bay
/// block.
#[test]
fn sm3a_a_revoked_pilot_can_still_return_a_rig() {
    let mut rig = stocked();
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    lease_one(&mut rig, pilot);
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    let lease = trust_lease(&rig.tw, pilot);
    let _ = run(&mut rig.tw, GOD, &format!("@contract/cancel #{lease}"));
    assert!(tacsim::port::live_charter(rig.tw.world(), pilot).is_none());
    let status = run(&mut rig.tw, pilot, "+tv");
    assert!(
        status.contains("Rig 1: OWP-TV-01, in the bay; "),
        "{status}"
    );
    let record = record_on(&rig.tw, pilot, tv1);
    let out = at_occator(&mut rig, pilot, "+tv/1 lease return go");
    assert!(
        out.contains(
            &RETURNED
                .replace("{mark}", "OWP-TV-01")
                .replace("{deposit}", DEPOSIT)
        ),
        "{out}"
    );
    assert_eq!(hold_status(&rig.tw, record.hold), HoldStatus::Released);
    assert!(in_shed(&rig.tw, tv1));
    let notice = tacsim::port::recall::RETURNED_NOTICE
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark);
    let mut heard = String::new();
    for _ in 0..3 {
        let events = fire_quietly(&mut rig);
        heard.push_str(&said(&rig.tw, &events, pilot));
        if heard.contains(&notice) {
            break;
        }
    }
    assert!(heard.contains(&notice), "{heard}");
    no_refused_writes(&rig.tw);
}

/// **A staff-ended rig lease reads ended and still returns** (reviewer F15): staff cancel
/// Occator's lease; `+tv` reads `RIG_LEASE_ENDED`; past the term no `+tv/lease` act
/// re-debits it; the return succeeds and releases the deposit.
#[test]
fn sm3a_a_staff_ended_rig_lease_reads_ended_and_still_returns() {
    let mut rig = stocked();
    let (pilot, _) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    lease_one(&mut rig, pilot);
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    let record = record_on(&rig.tw, pilot, tv1);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@contract/cancel #{}", record.lease),
    );
    assert!(contract_terminal(&rig.tw, record.lease));
    let status = run(&mut rig.tw, pilot, "+tv");
    let want = at_counter(&rig.tw, RIG_LEASE_ENDED)
        .replace("{n}", "1")
        .replace("{mark}", "OWP-TV-01")
        .replace("{hopper}", "empty");
    assert!(status.contains(&want), "{status}\nwanted: {want}");
    skip_days(&mut rig, 170);
    let account = account_of(&rig.tw, pilot);
    let (posted, held) = balances(&rig.tw, account);
    let preview = at_occator(&mut rig, pilot, "+tv/1 lease return");
    assert!(!preview.contains("re-debits"), "{preview}");
    let out = at_occator(&mut rig, pilot, "+tv/1 lease return go");
    assert!(
        out.contains(
            &RETURNED
                .replace("{mark}", "OWP-TV-01")
                .replace("{deposit}", DEPOSIT)
        ),
        "{out}"
    );
    assert!(!out.contains("re-debits"), "{out}");
    assert_eq!(
        balances(&rig.tw, account),
        (posted, held - DEPOSIT_MINOR),
        "no fee; the deposit released"
    );
    assert_eq!(hold_status(&rig.tw, record.hold), HoldStatus::Released);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 6. The slots after a forfeit (reviewer F2)
// ---------------------------------------------------------------------------

/// **A forfeit rig keeps its slot** (reviewer F2): two rigs, the pilot drained, the clock
/// past the term; the return of rig 1 forfeits both deposits first and hands rig 1 back;
/// `+tv` reads slot 1 empty and slot 2 forfeit (never unlisted); `+tv/2/lease go` is
/// `SLOT_TAKEN`; with funds a bare `go` takes slot 1, then the bay is full.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3a_a_forfeit_rig_keeps_its_slot() {
    let mut rig = stocked();
    let (pilot, _) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    lease_one(&mut rig, pilot);
    lease_one(&mut rig, pilot);
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    let tv2 = ObjId(rig_named(&rig.tw, "OWP-TV-02").tv);
    let drained = drain_funds(&mut rig, pilot);
    skip_days(&mut rig, 170);
    let occ = occator_account(&rig.tw);
    let (occ_posted, _) = balances(&rig.tw, occ);
    let out = at_occator(&mut rig, pilot, "+tv/1 lease return go");
    for mark in ["OWP-TV-01", "OWP-TV-02"] {
        let want = TERM_FORFEIT
            .replace("{mark}", mark)
            .replace("{owed}", FEE)
            .replace("{deposit}", DEPOSIT);
        assert!(out.contains(&want), "{out}\nwanted: {want}");
    }
    let want = RETURNED_FORFEIT.replace("{mark}", "OWP-TV-01");
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    assert_eq!(
        balances(&rig.tw, occ).0,
        occ_posted + 2 * DEPOSIT_MINOR,
        "both deposits captured to Occator"
    );
    assert_eq!(
        record_on(&rig.tw, pilot, tv1).status,
        TvLeaseStatus::Returned
    );
    assert_eq!(
        record_on(&rig.tw, pilot, tv2).status,
        TvLeaseStatus::Forfeit
    );
    let status = run(&mut rig.tw, pilot, "+tv");
    let empty = at_counter(&rig.tw, RIG_EMPTY).replace("{n}", "1");
    assert!(status.contains(&empty), "{status}\nwanted: {empty}");
    let forfeit = RIG_FORFEIT
        .replace("{n}", "2")
        .replace("{mark}", "OWP-TV-02")
        .replace("{hopper}", "empty");
    assert!(status.contains(&forfeit), "{status}\nwanted: {forfeit}");
    assert!(!status.contains("Rig ?:"), "{status}");
    let out = at_occator(&mut rig, pilot, "+tv/2/lease go");
    assert!(
        out.contains(
            &SLOT_TAKEN
                .replace("{n}", "2")
                .replace("{name}", "Salt Moth")
        ),
        "{out}"
    );
    undrain(&mut rig, pilot, drained);
    let out = at_occator(&mut rig, pilot, "+tv/lease go");
    let want = leased_line(LEASED, "OWP-TV-01", "Salt Moth", 1);
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    let out = at_occator(&mut rig, pilot, "+tv/lease go");
    assert!(
        out.contains(&BAY_FULL.replace("{name}", "Salt Moth")),
        "{out}"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 7. The dispatcher (reviewer F10)
// ---------------------------------------------------------------------------

/// **`+tv/3` is an unknown switch** (reviewer F10): the dispatcher refuses the undeclared
/// switch before `+tv`'s handler runs.
#[test]
fn sm3a_tv_three_is_an_unknown_switch() {
    let mut rig = boot(Roster::bare());
    let pilot = rig.tw.spawn_player("Pilot");
    let out = run(&mut rig.tw, pilot, "+tv/3");
    assert_eq!(out.trim(), "Unknown switch \"/3\" for +tv.", "{out}");
}

// ---------------------------------------------------------------------------
// 8. A charter that ends with rigs aboard (A8; reviewer F1)
// ---------------------------------------------------------------------------

/// **The buyout is refused with a rig aboard** (A8; reviewer F1(a)): funds for the buyout, a
/// rig in the bay, `+coop/buyout` refuses `BUYOUT_BAY` and writes nothing; with the rig
/// returned the buyout files.
#[test]
fn sm3a_buyout_is_refused_with_a_rig_aboard() {
    let mut rig = stocked();
    let (pilot, _) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    top_up(&mut rig.tw, pilot, 10_000_000);
    lease_one(&mut rig, pilot);
    let desk = desk(&rig.tw);
    teleport(&mut rig.tw, pilot, desk);
    let out = run(&mut rig.tw, pilot, "+coop/buyout");
    let want = tacsim::port::buyout::BUYOUT_BAY
        .replace("{n}", "1")
        .replace("{name}", "Salt Moth");
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    let e = store(&rig.tw)
        .enrolment_of(pilot)
        .cloned()
        .expect("an enrolment");
    assert_eq!(e.status, tacsim::port::EnrolmentStatus::Active);
    assert_eq!(e.buyout, None, "no buyout contract");
    let out = at_occator(&mut rig, pilot, "+tv/1 lease return go");
    assert!(out.contains("back across Occator's counter"), "{out}");
    teleport(&mut rig.tw, pilot, desk);
    let out = run(&mut rig.tw, pilot, "+coop/buyout");
    assert!(
        out.contains("The Registry writes the title transfer for the Salt Moth"),
        "{out}"
    );
    no_refused_writes(&rig.tw);
}

/// **A revoke repossesses the rigs** (A8; D14, D16): two rigs within their term;
/// `@coop/revoke` succeeds, both of Occator's leases end, both rigs go back to the shed,
/// both records read Returned and repossessed, both deposits are released and the pilot is
/// told twice; the hull is back in the pool. In a second world, a hopper not empty holds
/// the revoke with `RECALL_BAY`, and nothing is written.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3a_revoke_repossesses_the_rigs() {
    let mut rig = stocked();
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    lease_one(&mut rig, pilot);
    lease_one(&mut rig, pilot);
    let account = account_of(&rig.tw, pilot);
    let tvs: Vec<ObjId> = ["OWP-TV-01", "OWP-TV-02"]
        .iter()
        .map(|mark| ObjId(rig_named(&rig.tw, mark).tv))
        .collect();
    let before: Vec<TvLease> = tvs
        .iter()
        .map(|tv| record_on(&rig.tw, pilot, *tv))
        .collect();
    let (out, told) = heard_by(&mut rig, GOD, "@coop/revoke Pilot=test", pilot);
    let reply = tacsim::port::recall::REVOKED_REPLY
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark)
        .replace("{pilot}", "Pilot")
        .replace("{why}", "test");
    assert!(out.contains(&reply), "{out}\nwanted: {reply}");
    for (tv, record) in tvs.iter().zip(&before) {
        assert!(contract_terminal(&rig.tw, record.lease));
        assert!(in_shed(&rig.tw, *tv));
        let after = record_on(&rig.tw, pilot, *tv);
        assert_eq!(after.status, TvLeaseStatus::Returned);
        assert!(after.repossessed);
        assert_eq!(hold_status(&rig.tw, record.hold), HoldStatus::Released);
        let line = REPOSSESSED
            .replace("{mark}", &record.mark)
            .replace("{name}", "Salt Moth")
            .replace("{deposit}", DEPOSIT);
        assert!(told.contains(&line), "{told}\nwanted: {line}");
    }
    assert_eq!(balances(&rig.tw, account).1, 0, "every deposit released");
    assert!(tacsim::port::live_charter(rig.tw.world(), pilot).is_none());
    no_refused_writes(&rig.tw);

    // A second world: rig 1's hopper staff-filled holds the revoke.
    let mut rig = stocked();
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    lease_one(&mut rig, pilot);
    lease_one(&mut rig, pilot);
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    let tv2 = ObjId(rig_named(&rig.tw, "OWP-TV-02").tv);
    fill_hopper(&mut rig, tv1, 2, 9_600);
    let out = run(&mut rig.tw, GOD, "@coop/revoke Pilot=test");
    assert!(out.contains(tacsim::port::recall::NOT_REVOKED), "{out}");
    let bay_line = tacsim::port::recall::RECALL_BAY.replace("{name}", "Salt Moth");
    assert!(out.contains(&bay_line), "{out}");
    for tv in [tv1, tv2] {
        let record = record_on(&rig.tw, pilot, tv);
        assert_eq!(record.status, TvLeaseStatus::Active, "nothing written");
        assert!(!contract_terminal(&rig.tw, record.lease));
        assert_eq!(hold_status(&rig.tw, record.hold), HoldStatus::Held);
        assert!(in_bay(&rig.tw, tv, bay_of(&row)));
    }
    assert_eq!(
        store(&rig.tw).enrolment_of(pilot).map(|e| e.status),
        Some(tacsim::port::EnrolmentStatus::Active)
    );
    no_refused_writes(&rig.tw);
}

/// **The return arm repossesses after a staff cancel** (A8; D16): a rig leased, the clock
/// past its term, the Trust's lease cancelled by staff; the next fire repossesses the rig
/// past its term — the whole deposit captured to Occator, the pilot told
/// `REPOSSESSED_FORFEIT` — and returns the hull. The engine debits no fee: the pilot's
/// available funds are unchanged by the fire (the captured deposit was already held).
#[test]
fn sm3a_the_return_arm_repossesses_after_a_staff_cancel() {
    let mut rig = stocked();
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    lease_one(&mut rig, pilot);
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    let record = record_on(&rig.tw, pilot, tv1);
    skip_days(&mut rig, 170);
    let lease = trust_lease(&rig.tw, pilot);
    let _ = run(&mut rig.tw, GOD, &format!("@contract/cancel #{lease}"));
    let account = account_of(&rig.tw, pilot);
    let (posted, held) = balances(&rig.tw, account);
    let occ = occator_account(&rig.tw);
    let (occ_posted, _) = balances(&rig.tw, occ);
    let forfeit = REPOSSESSED_FORFEIT
        .replace("{mark}", "OWP-TV-01")
        .replace("{name}", "Salt Moth");
    let notice = tacsim::port::recall::RETURNED_NOTICE
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark);
    let mut heard = String::new();
    for _ in 0..3 {
        let events = fire_quietly(&mut rig);
        heard.push_str(&said(&rig.tw, &events, pilot));
        if heard.contains(&notice) {
            break;
        }
    }
    assert!(heard.contains(&forfeit), "{heard}\nwanted: {forfeit}");
    assert!(heard.contains(&notice), "{heard}\nwanted: {notice}");
    assert_eq!(balances(&rig.tw, occ).0, occ_posted + DEPOSIT_MINOR);
    assert_eq!(
        balances(&rig.tw, account),
        (posted - DEPOSIT_MINOR, held - DEPOSIT_MINOR),
        "the deposit captured, no fee debited: available unchanged"
    );
    assert!(matches!(
        hold_status(&rig.tw, record.hold),
        HoldStatus::Consumed { .. }
    ));
    let after = record_on(&rig.tw, pilot, tv1);
    assert_eq!(after.status, TvLeaseStatus::Returned);
    assert!(after.repossessed);
    assert!(in_shed(&rig.tw, tv1));
    no_refused_writes(&rig.tw);
}

/// **A dormant hull repossesses its rigs** (A8; D14, D16): a rig leased and no departure for
/// longer than the dormancy window; the recall arm's fire repossesses the rig (within its
/// term: the deposit released) and recalls the hull (`DORMANT_NOTICE`). In a second world,
/// with the hopper staff-filled and a sailing plan waiting, the hull is held (`HELD_DORMANT`
/// on the plan) and the log carries the bay page verbatim.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3a_a_dormant_hull_repossesses_its_rigs() {
    let mut rig = stocked();
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    lease_one(&mut rig, pilot);
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    let record = record_on(&rig.tw, pilot, tv1);
    skip_days(&mut rig, 8);
    let dormant = tacsim::port::recall::DORMANT_NOTICE
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark);
    let told = REPOSSESSED
        .replace("{mark}", "OWP-TV-01")
        .replace("{name}", "Salt Moth")
        .replace("{deposit}", DEPOSIT);
    let mut heard = String::new();
    for _ in 0..3 {
        let events = fire_quietly(&mut rig);
        heard.push_str(&said(&rig.tw, &events, pilot));
        if heard.contains(&dormant) {
            break;
        }
    }
    assert!(heard.contains(&told), "{heard}\nwanted: {told}");
    assert!(heard.contains(&dormant), "{heard}\nwanted: {dormant}");
    assert_eq!(hold_status(&rig.tw, record.hold), HoldStatus::Released);
    assert!(in_shed(&rig.tw, tv1));
    assert!(tacsim::port::live_charter(rig.tw.world(), pilot).is_none());
    no_refused_writes(&rig.tw);

    // A second world: the hopper filled and a sailing plan waiting.
    let mut rig = stocked();
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    lease_one(&mut rig, pilot);
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    fill_hopper(&mut rig, tv1, 2, 9_700);
    let filed = run(&mut rig.tw, pilot, "+ship/order depart stickney at=now");
    let _ = fire_quietly(&mut rig);
    let hull = ObjId(row.hull);
    let waiting = match tacsim::port::orders_state(rig.tw.world(), hull) {
        tacsim::port::OrdersState::Present(doc) => doc.clone(),
        other => panic!("a sailing plan: {filed}\n{other:?}"),
    };
    assert_eq!(
        waiting.state,
        tacsim::port::PlanState::Running,
        "{waiting:?}"
    );
    skip_days(&mut rig, 8);
    let capture = Capture::default();
    let dispatch = capture_dispatch(capture.clone());
    tracing::dispatcher::with_default(&dispatch, || {
        let _ = fire_quietly(&mut rig);
    });
    let doc = match tacsim::port::orders_state(rig.tw.world(), hull) {
        tacsim::port::OrdersState::Present(doc) => doc.clone(),
        other => panic!("a sailing plan: {other:?}"),
    };
    let reason = tacsim::port::recall::HELD_DORMANT
        .replace("{name}", "Salt Moth")
        .replace("{port}", "Ceres Freeport");
    assert_eq!(doc.state, tacsim::port::PlanState::Held, "{doc:?}");
    assert!(
        doc.hold.as_ref().is_some_and(|h| h.reason == reason),
        "{:?}",
        doc.hold
    );
    assert!(
        capture.text().contains(tacsim::port::recall::BAY_PAGE),
        "{}",
        capture.text()
    );
    assert!(
        tacsim::port::live_charter(rig.tw.world(), pilot).is_some(),
        "the charter stands"
    );
    assert_eq!(
        record_on(&rig.tw, pilot, tv1).status,
        TvLeaseStatus::Active,
        "the rig stays leased"
    );
    no_refused_writes(&rig.tw);
}

/// **`+coop/release` holds with a rig aboard** (A8): the pilot's own release refuses
/// `RECALL_BAY`.
#[test]
fn sm3a_release_holds_with_a_rig_aboard() {
    let mut rig = stocked();
    let (pilot, _) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    lease_one(&mut rig, pilot);
    let desk = desk(&rig.tw);
    teleport(&mut rig.tw, pilot, desk);
    let out = run(&mut rig.tw, pilot, "+coop/release");
    let want = tacsim::port::recall::RECALL_BAY.replace("{name}", "Salt Moth");
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    assert!(tacsim::port::live_charter(rig.tw.world(), pilot).is_some());
}

// ---------------------------------------------------------------------------
// 9. The upgrade (reviewer F3)
// ---------------------------------------------------------------------------

/// **A world on lane v9 takes v10 by the shelf** (reviewer F3; `sites.rs`'s model), in two
/// parts.
///
/// 1. The brief's path exactly: booted on the v9 shape (`strip_vehicles`, the published v9
///    digest), rebooted into the shipped roster, the plan and apply are one update
///    (`ceres_port`); every hull gains its bay, a live space with `rigs` at two, and no other
///    room or exit; eight rigs are made, titled to Occator, their hoppers the system
///    principal's; a second apply changes nothing.
/// 2. **With an enrolled hull.** A v9-latched world cannot enrol: its Lane line reads
///    Flagged ("this server ships @10"), and the Lane binding gates `+coop/enrol`. So the
///    charter is signed on the same content numbered as the shipped version (the v10 bytes
///    with the SM.3a blocks stripped, which the gate reads current), and the shipped content
///    one version on is applied from the shelf: the same `ceres_port` update, through the
///    same applier, over an enrolled hull. The enrolled hull gains its bay like the rest;
///    the enrolment document, the Trust's lease and the hold vault are untouched; the hulls
///    line reads as before and the vehicles line Ready.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3a_a_world_on_lane_v9_takes_v10_by_the_shelf() {
    // ---- 1. v9 -> v10 ----
    let v9 = lane_variant(9, strip_vehicles);
    assert_eq!(
        parse_pack(&v9).expect("v9").manifest().digest().to_hex(),
        LANE_V9_DIGEST,
        "strip_vehicles rebuilds the published v9"
    );
    let rig = boot(Roster::seeded(v9));
    let before_rows = port_map(&rig.tw, "ceres_port").hulls;
    assert!(port_map(&rig.tw, "ceres_port").vehicles.is_none());
    assert!(
        before_rows
            .iter()
            .all(|row| !row.rooms.contains_key(&cid("bay"))),
        "a v9 world lays no bay"
    );
    let Rig {
        mut tw,
        oracle,
        typed,
        ..
    } = rig;
    let folded = mush_testkit::fold_module_ops(tw.take_module_ops());
    let tw = tw
        .reboot_into(Roster::bare().modules(), folded.clone())
        .expect("the reboot boots");
    let mut rig = Rig {
        tw,
        oracle,
        roster: Roster::bare(),
        store: folded,
        typed,
    };
    no_refused_writes(&rig.tw);
    with_offer_keys(&mut rig.tw);
    let entries = parse_pack(LANE_JSON).expect("parses").entries().len();
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        plan.contains(&format!(
            "Plan for {LANE_PACK}@{LANE_VERSION}: 0 create, 1 update, 0 retire, {} unchanged",
            entries - 1
        )),
        "{plan}"
    );
    assert!(
        plan.lines()
            .any(|l| l.contains("update") && l.contains("ceres_port (port)")),
        "{plan}"
    );
    assert!(
        apply.contains(&format!(
            "Applied {LANE_PACK}@{LANE_VERSION}: 0 created, 1 updated, 0 retired, {} unchanged.",
            entries - 1
        )),
        "{plan}\n{apply}"
    );
    assert_bays_laid(&rig.tw, &before_rows);
    let made = assert_rigs_made(&rig.tw);
    let objects = rig.tw.world().iter().count();
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert_eq!(rig.tw.world().iter().count(), objects, "{plan}\n{apply}");
    assert_eq!(rigs(&rig.tw), made, "no drift on a second apply");
    no_refused_writes(&rig.tw);

    // ---- 2. an enrolled hull through the same update ----
    let seed = lane_variant(LANE_VERSION, strip_vehicles);
    let next = lane_variant(LANE_VERSION + 1, |_| {});
    let roster = Roster {
        seed_lane: Some(seed),
        shelf: Some(vec![
            mush_content::load_pack(&next).expect("the next lane verifies"),
        ]),
        extra_lane: Some(next),
        ..Roster::default()
    };
    let mut rig = boot(roster);
    stock(&mut rig);
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let enrolment = store(&rig.tw).enrolment_of(pilot).cloned();
    let hold = hold_vault(&rig.tw, &row);
    let hold_record = economy(&rig.tw).snapshot().vault(hold).cloned();
    let before_rows = port_map(&rig.tw, "ceres_port").hulls;
    let hulls_line = |tw: &TestWorld| {
        tacsim::port::readiness(tw.world())
            .lines
            .into_iter()
            .find(|line| line.binding == Binding::Hulls)
            .map(|line| (line.state, line.text))
            .expect("the hulls line")
    };
    let hulls_before = hulls_line(&rig.tw);
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains(&format!(
            "Applied {LANE_PACK}@{}: 0 created, 1 updated, 0 retired, {} unchanged.",
            LANE_VERSION + 1,
            entries - 1
        )),
        "{plan}\n{apply}"
    );
    assert_bays_laid(&rig.tw, &before_rows);
    let _ = assert_rigs_made(&rig.tw);
    assert_eq!(store(&rig.tw).enrolment_of(pilot).cloned(), enrolment);
    assert!(tacsim::port::live_charter(rig.tw.world(), pilot).is_some());
    assert_eq!(
        economy(&rig.tw).snapshot().vault(hold).cloned(),
        hold_record
    );
    assert_eq!(
        hulls_line(&rig.tw),
        hulls_before,
        "the hulls line as before"
    );
    let vehicles = tacsim::port::readiness(rig.tw.world())
        .lines
        .into_iter()
        .find(|line| line.binding == Binding::Vehicles)
        .expect("the vehicles line");
    assert_eq!(vehicles.state, LineState::Ready, "{}", vehicles.text);
    no_refused_writes(&rig.tw);
}

/// Every hull of `before` has gained its bay — a live space riding it with `rigs` at two —
/// and no other room or exit moved.
fn assert_bays_laid(tw: &TestWorld, before: &[HullRow]) {
    let after = port_map(tw, "ceres_port").hulls;
    assert_eq!(after.len(), before.len());
    let rigs_slot = mush_sim::SlotId::new("rigs").expect("a slot");
    for (was, now) in before.iter().zip(&after) {
        let mut rooms = now.rooms.clone();
        let bay = ObjId(rooms.remove(&cid("bay")).expect("the bay room"));
        assert_eq!(rooms, was.rooms, "{}: no other room moved", was.mark);
        assert_eq!(now.exits, was.exits, "{}: no exit moved", was.mark);
        let st = tw.world().module_state::<SpatialState>().expect("sim");
        assert_eq!(
            st.spaces[&bay].anchor,
            SpaceAnchor::MovingObject {
                host: ObjId(now.hull)
            }
        );
        assert_eq!(
            st.spaces[&bay].compartments[&rigs_slot].maximum_items,
            Some(2)
        );
    }
}

/// Eight rigs made, each titled to Occator with its hopper the system principal's.
fn assert_rigs_made(tw: &TestWorld) -> Vec<RigRow> {
    let owner = PrincipalId::System(SystemPrincipalId::new("tacsim.vehicles").expect("an id"));
    let lessor = occator(tw);
    let made = rigs(tw);
    assert_eq!(made.len(), 8);
    for row in &made {
        let tv = ObjId(row.tv);
        assert_eq!(
            economy(tw)
                .snapshot()
                .asset(tv)
                .map(|a| a.title_owner.clone()),
            Some(lessor.clone())
        );
        assert_eq!(
            economy(tw)
                .snapshot()
                .vault(VaultId(row.vault.expect("a hopper")))
                .map(|v| v.owner.clone()),
            Some(owner.clone())
        );
    }
    made
}

// ---------------------------------------------------------------------------
// 10. The term (lead ruling 9; D8)
// ---------------------------------------------------------------------------

/// **A term past due is re-debited, or the deposit forfeit** (lead ruling 9; D8): 170 IC days
/// on, `+tv` reads the rig past its term; a return with funds re-debits one term first and
/// returns the rig; a second pilot drained of funds forfeits the deposit to Occator at the
/// return, which then releases nothing.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3a_a_term_past_due_is_re_debited_or_the_deposit_forfeit() {
    let mut rig = stocked();
    let (a, _) = prospector(&mut rig, "Ann", 1, "Salt Moth");
    lease_one(&mut rig, a);
    let (b, _) = prospector(&mut rig, "Bea", 2, "Tally Moth");
    lease_one(&mut rig, b);
    let tv_a = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    let tv_b = ObjId(rig_named(&rig.tw, "OWP-TV-02").tv);
    let drained = drain_funds(&mut rig, b);
    skip_days(&mut rig, 170);
    let status = run(&mut rig.tw, a, "+tv");
    let past = RIG_PAST_TERM
        .replace("{n}", "1")
        .replace("{mark}", "OWP-TV-01")
        .replace("{fee}", FEE)
        .replace("{hopper}", "empty");
    assert!(status.contains(&past), "{status}\nwanted: {past}");
    // With funds: one term re-debited (166 IC days left on the new term), then the return.
    let account_a = account_of(&rig.tw, a);
    let (posted, held) = balances(&rig.tw, account_a);
    let occ = occator_account(&rig.tw);
    let renewed = TERM_RENEWED
        .replace("{fee}", FEE)
        .replace("{mark}", "OWP-TV-01")
        .replace("{terms}", "1")
        .replace("{days}", "166");
    // The bare return previews the settle and the return and signs nothing (reviewer
    // finding tests/T3): the settle's line, then `RETURN_PREVIEW`, and no money, hold or
    // record moves.
    let record_was = record_on(&rig.tw, a, tv_a);
    let occ_was = balances(&rig.tw, occ);
    let preview = at_occator(&mut rig, a, "+tv/lease return");
    let return_preview = RETURN_PREVIEW
        .replace("{mark}", "OWP-TV-01")
        .replace("{deposit}", DEPOSIT)
        .replace("{n}", "1");
    assert_eq!(
        preview.lines().collect::<Vec<_>>(),
        vec![renewed.as_str(), return_preview.as_str()],
        "{preview}"
    );
    assert_eq!(balances(&rig.tw, account_a), (posted, held));
    assert_eq!(balances(&rig.tw, occ), occ_was);
    assert_eq!(record_on(&rig.tw, a, tv_a), record_was);
    assert_eq!(hold_status(&rig.tw, record_was.hold), HoldStatus::Held);
    let out = at_occator(&mut rig, a, "+tv/lease return go");
    assert!(out.contains(&renewed), "{out}\nwanted: {renewed}");
    assert!(
        out.contains(
            &RETURNED
                .replace("{mark}", "OWP-TV-01")
                .replace("{deposit}", DEPOSIT)
        ),
        "{out}"
    );
    assert_eq!(
        balances(&rig.tw, account_a),
        (posted - FEE_MINOR, held - DEPOSIT_MINOR)
    );
    let record_a = record_on(&rig.tw, a, tv_a);
    assert_eq!(
        (record_a.terms_paid, record_a.status),
        (2, TvLeaseStatus::Returned)
    );
    // Drained: the deposit is forfeit to Occator at the next +tv/lease act.
    let (occ_posted, _) = balances(&rig.tw, occ);
    let account_b = account_of(&rig.tw, b);
    let (_, held_b) = balances(&rig.tw, account_b);
    let forfeit = TERM_FORFEIT
        .replace("{mark}", "OWP-TV-02")
        .replace("{owed}", FEE)
        .replace("{deposit}", DEPOSIT);
    // The bare return previews the forfeit and `RETURN_PREVIEW_FORFEIT`, and captures
    // nothing (reviewer finding tests/T3).
    let record_was = record_on(&rig.tw, b, tv_b);
    let b_was = balances(&rig.tw, account_b);
    let preview = at_occator(&mut rig, b, "+tv/lease return");
    let forfeit_preview = RETURN_PREVIEW_FORFEIT
        .replace("{mark}", "OWP-TV-02")
        .replace("{n}", "1");
    assert_eq!(
        preview.lines().collect::<Vec<_>>(),
        vec![forfeit.as_str(), forfeit_preview.as_str()],
        "{preview}"
    );
    assert_eq!(balances(&rig.tw, account_b), b_was);
    assert_eq!(balances(&rig.tw, occ).0, occ_posted);
    assert_eq!(record_on(&rig.tw, b, tv_b), record_was);
    assert_eq!(hold_status(&rig.tw, record_was.hold), HoldStatus::Held);
    let out = at_occator(&mut rig, b, "+tv/lease return go");
    assert!(out.contains(&forfeit), "{out}\nwanted: {forfeit}");
    assert!(
        out.contains(&RETURNED_FORFEIT.replace("{mark}", "OWP-TV-02")),
        "{out}"
    );
    assert_eq!(balances(&rig.tw, occ).0, occ_posted + DEPOSIT_MINOR);
    assert_eq!(
        balances(&rig.tw, account_b).1,
        held_b - DEPOSIT_MINOR,
        "the deposit captured; nothing more released"
    );
    assert_eq!(record_on(&rig.tw, b, tv_b).status, TvLeaseStatus::Returned);
    undrain(&mut rig, b, drained);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 11. Equal payload, equal plan (A1)
// ---------------------------------------------------------------------------

/// The passage lines of a `+ship/plan` block: everything but the hold's slack and room
/// lines, which name the two hulls' different holds.
fn passage_lines(plan: &str) -> Vec<String> {
    plan.lines()
        .filter(|line| {
            line.starts_with("Payload ")
                || line.starts_with("Burn ")
                || line.starts_with("Stores ")
                || line.starts_with("Throttle")
                || line.starts_with("Plan for ")
                || line.starts_with("Passage ")
                || line.contains(" IC days")
        })
        .map(str::to_owned)
        .collect()
}

/// **A Dray-P with two rigs plans as a Dray at equal payload** (A1; D-20-7; reviewer F8):
/// pilot A's Dray-P with two empty rigs and 40 t of cargo, pilot B's Dray with 52 t, equal
/// tank and locker: `+ship/plan stickney` prints identical passage, burn, stores and throttle
/// lines, and after `+ship/undock` the two filed pendings carry equal payloads and plans,
/// pilot A's with two carried rows of 6,000 kg. At the stowage maxima with two rigs the rail
/// is never refused.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3a_a_dray_p_with_two_rigs_plans_as_a_dray_at_equal_payload() {
    let mut rig = stocked();
    let (a, row_a) = prospector(&mut rig, "Ann", 1, "Salt Moth");
    lease_one(&mut rig, a);
    lease_one(&mut rig, a);
    let (b, row_b) = chartered(&mut rig, "Bea", 2, "Tally Moth");
    load(&mut rig, &row_a, 40, 60, 60, 9_100);
    load(&mut rig, &row_b, 52, 60, 60, 9_200);
    board(&mut rig, a);
    board(&mut rig, b);
    let plan_a = run(&mut rig.tw, a, "+ship/plan stickney");
    let plan_b = run(&mut rig.tw, b, "+ship/plan stickney");
    let (lines_a, lines_b) = (passage_lines(&plan_a), passage_lines(&plan_b));
    assert!(lines_a.iter().any(|l| l.starts_with("Burn ")), "{plan_a}");
    assert_eq!(lines_a, lines_b, "{plan_a}\n---\n{plan_b}");
    // The payload, observably: the two filed pendings (reviewer F8).
    let (out_a, queued_a) = command(&mut rig, a, "+ship/undock stickney");
    assert!(out_a.contains("Clearance filed for "), "{out_a}");
    let (out_b, queued_b) = command(&mut rig, b, "+ship/undock stickney");
    assert!(out_b.contains("Clearance filed for "), "{out_b}");
    let pending = |tw: &TestWorld, hull: ObjId| {
        tacsim::port::voyage::pending_of(tw.world(), hull)
            .cloned()
            .expect("a filed passage")
    };
    let (pa, pb) = (
        pending(&rig.tw, ObjId(row_a.hull)),
        pending(&rig.tw, ObjId(row_b.hull)),
    );
    assert_eq!(pa.payload_kg, pb.payload_kg);
    assert_eq!(pa.plan, pb.plan);
    assert_eq!(pa.carried.len(), 2);
    assert!(
        pa.carried.iter().all(|r| r.dry_kg == 6_000),
        "{:?}",
        pa.carried
    );
    assert!(pb.carried.is_empty());
    land_all(&mut rig, &queued_a);
    land_all(&mut rig, &queued_b);
    // At the maxima with two rigs: 86 t, 80 t, 120 crew-days, one soul — under the rail.
    let (c, row_c) = prospector(&mut rig, "Cal", 3, "Cold Moth");
    lease_one(&mut rig, c);
    lease_one(&mut rig, c);
    load(&mut rig, &row_c, 86, 80, 120, 9_300);
    board(&mut rig, c);
    let plan_c = run(&mut rig.tw, c, "+ship/plan stickney");
    assert!(plan_c.contains("Payload 178.6 t of 181 t."), "{plan_c}");
    assert!(!plan_c.contains("is over the rated"), "{plan_c}");
    no_refused_writes(&rig.tw);
}

/// **A filed passage burns with its rigs** (A1; D13; reviewer finding tests/T1): two Dray-Ps
/// with two rigs each and equal holds, one rig's hopper staff-filled with 2 t of metal ore;
/// `+ship/undock` files two carried rows on each, every row naming its rig's hopper vault and
/// the four ores it accepts, and the loaded hull's payload is the other's plus exactly
/// 2,000 kg; each burn's prepare re-reads the hoppers and passes, its ticket's payload equal
/// to the filing's; a return while the clearance is filed refuses with the cleared line.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3a_a_filed_passage_burns_with_its_rigs() {
    let mut rig = stocked();
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    lease_one(&mut rig, pilot);
    lease_one(&mut rig, pilot);
    let (other, row_o) = prospector(&mut rig, "Other", 2, "Tally Moth");
    lease_one(&mut rig, other);
    lease_one(&mut rig, other);
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    fill_hopper(&mut rig, tv1, 2, 9_050);
    load(&mut rig, &row, 10, 60, 60, 9_100);
    load(&mut rig, &row_o, 10, 60, 60, 9_200);
    board(&mut rig, pilot);
    board(&mut rig, other);
    let hull = ObjId(row.hull);
    let hull_o = ObjId(row_o.hull);
    let (out, queued) = command(&mut rig, pilot, "+ship/undock stickney");
    assert!(out.contains("Clearance filed for "), "{out}");
    let (out_o, queued_o) = command(&mut rig, other, "+ship/undock stickney");
    assert!(out_o.contains("Clearance filed for "), "{out_o}");
    let filed = tacsim::port::voyage::pending_of(rig.tw.world(), hull)
        .cloned()
        .expect("a filed passage");
    let filed_o = tacsim::port::voyage::pending_of(rig.tw.world(), hull_o)
        .cloned()
        .expect("a filed passage");
    assert_eq!(filed.carried.len(), 2);
    assert_eq!(filed_o.carried.len(), 2);
    assert_eq!(
        filed.payload_kg,
        filed_o.payload_kg + 2_000,
        "the loaded hopper weighs its 2 t of ore"
    );
    let ores = ["metal_ore", "ice_ore", "carbon_ore", "strategic_ore"].map(cid);
    for carried in filed.carried.iter().chain(&filed_o.carried) {
        let tv = ObjId(carried.tv);
        assert_eq!(carried.dry_kg, 6_000);
        assert_eq!(carried.hopper, Some(hopper(&rig.tw, tv).0), "{carried:?}");
        assert!(
            ores.iter().all(|ore| carried.goods.contains(ore)),
            "{carried:?}"
        );
    }
    assert!(
        filed.carried.iter().any(|carried| carried.tv == tv1.0),
        "the loaded rig is carried"
    );
    land_all(&mut rig, &queued);
    land_all(&mut rig, &queued_o);
    let t = ticket(&mut rig.tw, hull, filed.seq).expect("the burn landed");
    assert_eq!(
        t.payload_kg, filed.payload_kg,
        "the prepare recomputed the same payload, the hopper's ore included"
    );
    let t_o = ticket(&mut rig.tw, hull_o, filed_o.seq).expect("the burn landed");
    assert_eq!(t_o.payload_kg, filed_o.payload_kg);
    let out = run(&mut rig.tw, pilot, "+tv/2 lease return go");
    let cleared = tacsim::port::buyout::CLEARED_WAIT.replace("{name}", "Salt Moth");
    assert!(out.contains(&cleared), "{out}");
    no_refused_writes(&rig.tw);
}

/// **The queue plans a Dray-P with its rigs** (reviewer finding tests/T4: the queue's
/// `Planner::carrying` and the dry run's projected payload): pilot A's Dray-P with two
/// empty rigs and 40 t of cargo, pilot B's Dray with 52 t, equal tank and locker, each with
/// `+ship/order depart stickney at=now`: the two bare `+ship/plan` dry-run depart lines are
/// identical and A's names the payload its own `+ship/plan` prints; the queue then files
/// each departure by itself, at equal payloads and plans, A's with two carried rows, and
/// each burn lands at its filing's payload.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3a_the_queue_plans_a_dray_p_with_its_rigs() {
    let mut rig = stocked();
    let (a, row_a) = prospector(&mut rig, "Ann", 1, "Salt Moth");
    lease_one(&mut rig, a);
    lease_one(&mut rig, a);
    let (b, row_b) = chartered(&mut rig, "Bea", 2, "Tally Moth");
    load(&mut rig, &row_a, 40, 60, 60, 9_100);
    load(&mut rig, &row_b, 52, 60, 60, 9_200);
    board(&mut rig, a);
    board(&mut rig, b);
    let plan_a = run(&mut rig.tw, a, "+ship/plan stickney");
    let payload = plan_a
        .lines()
        .find_map(|line| line.strip_prefix("Payload "))
        .and_then(|rest| rest.split(" t").next())
        .unwrap_or_else(|| panic!("{plan_a}"))
        .to_owned();
    let depart_line = |rig: &mut Rig, who: ObjId| {
        let out = run(&mut rig.tw, who, "+ship/order depart stickney at=now");
        let orders = run(&mut rig.tw, who, "+ship/plan");
        orders
            .lines()
            .find(|line| line.starts_with("  Order 1, depart for "))
            .map(str::to_owned)
            .unwrap_or_else(|| panic!("{out}\n{orders}"))
    };
    let line_a = depart_line(&mut rig, a);
    let line_b = depart_line(&mut rig, b);
    assert_eq!(line_a, line_b, "the dry run counts A's rigs");
    assert!(
        line_a.contains(&format!(": payload {payload} t, ")),
        "{line_a}\n{plan_a}"
    );
    // The queue files both departures by itself.
    let (hull_a, hull_b) = (ObjId(row_a.hull), ObjId(row_b.hull));
    let mut queued = Vec::new();
    for _ in 0..6 {
        let (_, more) = fire(&mut rig);
        queued.extend(more);
        let filed =
            |tw: &TestWorld, hull| tacsim::port::voyage::pending_of(tw.world(), hull).is_some();
        if filed(&rig.tw, hull_a) && filed(&rig.tw, hull_b) {
            break;
        }
    }
    let pending = |tw: &TestWorld, hull: ObjId| {
        tacsim::port::voyage::pending_of(tw.world(), hull)
            .cloned()
            .expect("the queue filed the departure")
    };
    let (pa, pb) = (pending(&rig.tw, hull_a), pending(&rig.tw, hull_b));
    assert_eq!(pa.payload_kg, pb.payload_kg, "{pa:?}\n{pb:?}");
    assert_eq!(pa.plan, pb.plan);
    assert_eq!(pa.carried.len(), 2);
    assert!(pb.carried.is_empty());
    land_all(&mut rig, &queued);
    let ta = ticket(&mut rig.tw, hull_a, pa.seq).expect("A's burn landed");
    let tb = ticket(&mut rig.tw, hull_b, pb.seq).expect("B's burn landed");
    assert_eq!(ta.payload_kg, pa.payload_kg);
    assert_eq!(tb.payload_kg, pb.payload_kg);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 12. Restart (A7) and the pad
// ---------------------------------------------------------------------------

/// **Leases and rigs survive a restart** (A7; reviewer findings conformance/F3 and tests/T2,
/// `hulls.rs`'s second-boot precedent): with a rig leased, a reboot on the same store reads
/// the record Present, the rig in the bay and the seven others in the shed, the hull still a
/// Dray-P and `+tv` the same lines; the boot writes no port map, no placement and no lease
/// record, makes no object and posts nothing (objects, journal, titles and vaults unchanged).
/// Then **the next apply writes nothing** with the rig leased: an update of the entry from the
/// shelf makes no object, vault or title, re-places no rig and leaves the record Active.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3a_leases_and_rigs_survive_a_restart() {
    let next = lane_variant(LANE_VERSION + 1, |doc| {
        doc["entries"]["ceres_port"]["def"]["programme"]["dormancy_real_days"] =
            serde_json::json!(8);
    });
    let roster = Roster {
        shelf: Some(vec![
            mush_content::load_pack(&next).expect("the variant verifies"),
        ]),
        extra_lane: Some(next),
        ..Roster::default()
    };
    let mut rig = stocked_on(roster);
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    lease_one(&mut rig, pilot);
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    let bay = bay_of(&row);
    let record = record_on(&rig.tw, pilot, tv1);
    let before = run(&mut rig.tw, pilot, "+tv");
    let counts = |tw: &TestWorld| {
        (
            tw.world().iter().count(),
            economy(tw).journal_len(),
            economy(tw).snapshot().assets().count(),
            economy(tw).snapshot().vaults().count(),
        )
    };
    let was = counts(&rig.tw);
    let mut rig = reboot(rig);
    let ops = rig.tw.take_module_ops();
    let written = |prefix: &str| {
        ops.iter().any(|op| {
            matches!(&op.op, ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key }
                if key.starts_with(prefix))
        })
    };
    assert!(
        !written(tacsim::port::PORT_MAP_PREFIX),
        "the boot applies nothing twice"
    );
    assert!(
        !written(tacsim::port::tv::TV_LEASE_PREFIX),
        "the boot writes no lease record"
    );
    assert!(
        !ops.iter().any(|op| op.ns == mush_sim::data::NS
            && matches!(&op.op, ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key }
                if key.starts_with("placement:"))),
        "the boot writes no placement"
    );
    assert_eq!(
        counts(&rig.tw),
        was,
        "no object made, nothing posted, no title or vault"
    );
    assert!(matches!(
        store(&rig.tw).tv_lease(record.lease),
        Some(TvLeaseState::Present(_))
    ));
    assert!(in_bay(&rig.tw, tv1, bay));
    let others = rigs(&rig.tw)
        .iter()
        .filter(|row| ObjId(row.tv) != tv1 && in_shed(&rig.tw, ObjId(row.tv)))
        .count();
    assert_eq!(others, 7);
    assert_eq!(unit_id(&rig.tw, ObjId(row.hull)), DRAY_P);
    let after = run(&mut rig.tw, pilot, "+tv");
    assert_eq!(after, before);

    // The next apply, with the rig leased into the bay, writes nothing of the rigs.
    let was = counts(&rig.tw);
    let rows = rigs(&rig.tw);
    let _ = rig.tw.take_module_ops();
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@{}", LANE_VERSION + 1)),
        "{plan}\n{apply}"
    );
    let ops = rig.tw.take_module_ops();
    assert!(
        !ops.iter().any(|op| op.ns == mush_sim::data::NS
            && matches!(&op.op, ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key }
                if key.starts_with("placement:"))),
        "the apply re-places no rig: {ops:?}"
    );
    assert!(
        !ops.iter()
            .any(|op| matches!(&op.op, ModuleWrite::JsonSet { key, .. }
            if key.starts_with(tacsim::port::tv::TV_LEASE_PREFIX))),
        "the apply writes no lease record"
    );
    let (objects, _, assets, vaults) = counts(&rig.tw);
    assert_eq!((objects, assets, vaults), (was.0, was.2, was.3));
    assert_eq!(rigs(&rig.tw), rows, "the rig rows stand");
    assert!(in_bay(&rig.tw, tv1, bay), "the leased rig stays in the bay");
    assert_eq!(record_on(&rig.tw, pilot, tv1).status, TvLeaseStatus::Active);
    assert_eq!(hold_status(&rig.tw, record.hold), HoldStatus::Held);
    assert_eq!(run(&mut rig.tw, pilot, "+tv"), before);
    no_refused_writes(&rig.tw);
}

/// **`+tv` prints the stores line on a pad** (SM.2's deferral): a Dray-P settled at Ahuna
/// Flank prints SM.2's stores line under its rig lines.
#[test]
fn sm3a_tv_prints_the_stores_line_on_a_pad() {
    let mut rig = stocked();
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    lease_one(&mut rig, pilot);
    load(&mut rig, &row, 0, 20, 20, 9_100);
    board(&mut rig, pilot);
    let hull = ObjId(row.hull);
    let (_, t) = file_and_launch(&mut rig, pilot, hull, "+ship/undock ahuna");
    let out = run(&mut rig.tw, pilot, "+tv/1 lease return go");
    assert!(
        out.contains(
            &LEASE_UNDER_WAY
                .replace("{name}", "Salt Moth")
                .replace("{port}", "Ceres Freeport")
        ),
        "{out}"
    );
    let _ = to_ic(&mut rig, arrival_of(&t));
    let status = run(&mut rig.tw, pilot, "+tv");
    let lines: Vec<&str> = status.lines().collect();
    let rig_at = lines
        .iter()
        .position(|l| l.starts_with("Rig 1: OWP-TV-01, in the bay; "))
        .unwrap_or_else(|| panic!("{status}"));
    let stores_at = lines
        .iter()
        .position(|l| l.starts_with("Stores ") && l.contains("crew-days"))
        .unwrap_or_else(|| panic!("{status}"));
    assert!(stores_at > rig_at, "{status}");
    let out = run(&mut rig.tw, pilot, "+tv/1 lease return go");
    assert!(
        out.contains(&LEASE_NOT_HERE.replace("{port}", "Ceres Freeport")),
        "{out}"
    );
}

// ---------------------------------------------------------------------------
// 13. The fixer's findings (deposits, live records, the dark name, a lost bay)
// ---------------------------------------------------------------------------

/// [`reboot`], with `edit` applied first to lease `lease`'s stored record (a test rig:
/// staff's hand on the document, or a store fault).
fn reboot_editing_lease(mut rig: Rig, lease: u64, edit: impl FnOnce(&mut Value)) -> Rig {
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    let key = tacsim::port::tv::tv_lease_key(lease);
    let doc = rig
        .store
        .values_mut()
        .flat_map(|data| data.kv.iter_mut())
        .find(|(k, _)| *k == key)
        .map(|(_, value)| value)
        .expect("the lease record is in the store");
    edit(doc);
    reboot(rig)
}

/// **A deposit that does not resolve never strands its record** (reviewer finding
/// correctness/F1): a lease record naming a hold that is not its deposit refuses the
/// return (`LEASE_FAILED`) and holds the revoke (`RECALL_BAY`) before anything is written —
/// the record Active, the economy lease live, the true deposit `Held`, the rig in the bay,
/// the charter standing — and once staff repair the record's hold the return succeeds and
/// releases the deposit. Never a `Returned` record over a `Held` engine-owned deposit.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3a_a_deposit_that_does_not_resolve_never_strands_its_record() {
    let mut rig = stocked();
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    lease_one(&mut rig, pilot);
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    let bay = bay_of(&row);
    let record = record_on(&rig.tw, pilot, tv1);
    let rig = reboot_editing_lease(rig, record.lease, |doc| {
        doc["hold"] = serde_json::json!(record.hold + 9_999);
    });
    let mut rig = rig;
    let unchanged = |tw: &TestWorld| {
        let now = record_on(tw, pilot, tv1);
        assert_eq!(now.status, TvLeaseStatus::Active, "the record stays live");
        assert!(!contract_terminal(tw, record.lease), "the lease stands");
        assert_eq!(hold_status(tw, record.hold), HoldStatus::Held);
        assert!(in_bay(tw, tv1, bay), "the rig stays aboard");
    };
    let out = at_occator(&mut rig, pilot, "+tv/1 lease return go");
    assert_eq!(out.trim(), LEASE_FAILED, "{out}");
    unchanged(&rig.tw);
    let out = run(&mut rig.tw, GOD, "@coop/revoke Pilot=test");
    assert!(
        out.contains(tacsim::port::recall::NOT_REVOKED)
            && out.contains(&tacsim::port::recall::RECALL_BAY.replace("{name}", "Salt Moth")),
        "{out}"
    );
    unchanged(&rig.tw);
    assert!(tacsim::port::live_charter(rig.tw.world(), pilot).is_some());
    // Staff repair the record's hold: the return goes through.
    let rig = reboot_editing_lease(rig, record.lease, |doc| {
        doc["hold"] = serde_json::json!(record.hold);
    });
    let mut rig = rig;
    let out = at_occator(&mut rig, pilot, "+tv/1 lease return go");
    let want = RETURNED
        .replace("{mark}", "OWP-TV-01")
        .replace("{deposit}", DEPOSIT);
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    assert_eq!(hold_status(&rig.tw, record.hold), HoldStatus::Released);
    assert_eq!(
        record_on(&rig.tw, pilot, tv1).status,
        TvLeaseStatus::Returned
    );
    no_refused_writes(&rig.tw);
}

/// **A revoked pilot's unreadable record reads dark by the ship's name** (reviewer findings
/// correctness/F3 and conformance/F4): the Trust's lease cancelled by staff (Revoked, no
/// tick) and the pilot's only lease record unreadable; `+tv` and the return both answer
/// `TV_DARK` with the hull the record still names, never an empty name and never the
/// charter's "has ended" line.
#[test]
fn sm3a_a_revoked_pilots_unreadable_record_reads_dark_by_name() {
    let mut rig = stocked();
    let (pilot, _) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    lease_one(&mut rig, pilot);
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    let record = record_on(&rig.tw, pilot, tv1);
    let lease = trust_lease(&rig.tw, pilot);
    let _ = run(&mut rig.tw, GOD, &format!("@contract/cancel #{lease}"));
    assert!(tacsim::port::live_charter(rig.tw.world(), pilot).is_none());
    let mut rig = reboot_editing_lease(rig, record.lease, |doc| {
        doc["slot"] = serde_json::json!(0);
    });
    assert!(matches!(
        store(&rig.tw).tv_lease(record.lease),
        Some(TvLeaseState::Unreadable { .. })
    ));
    let dark = TV_DARK.replace("{name}", "Salt Moth");
    let status = run(&mut rig.tw, pilot, "+tv");
    assert_eq!(status.trim(), dark, "{status}");
    let out = at_occator(&mut rig, pilot, "+tv/1 lease return go");
    assert_eq!(out.trim(), dark, "{out}");
}

/// **A live record out of the bay still holds the hull** (reviewer finding correctness/F2):
/// staff move a leased rig out of the bay into the shed; its live record still refuses
/// `+coop/refit standard` (`REFIT_BAY`), `+coop/release` (`RECALL_BAY`) and `+coop/buyout`
/// (`BUYOUT_BAY`), so no charter ends with the lease live; `@coop/revoke` repossesses it
/// from where it stands — the lease ended, the deposit released, the record Returned and
/// repossessed — and the hull goes back to the pool with no live record on it.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3a_a_live_record_out_of_the_bay_still_holds_the_hull() {
    let mut rig = stocked();
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    top_up(&mut rig.tw, pilot, 10_000_000);
    lease_one(&mut rig, pilot);
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    let record = record_on(&rig.tw, pilot, tv1);
    let shed = shed(&rig.tw);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@spatial/place/interior #{}=#{}", tv1.0, shed.0),
    );
    assert!(in_shed(&rig.tw, tv1), "staff moved the rig out of the bay");
    let out = refit(&mut rig, pilot, "standard");
    let want = REFIT_BAY.replace("{n}", "1").replace("{name}", "Salt Moth");
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    assert_eq!(unit_id(&rig.tw, ObjId(row.hull)), DRAY_P, "still a Dray-P");
    let out = run(&mut rig.tw, pilot, "+coop/release");
    let want = tacsim::port::recall::RECALL_BAY.replace("{name}", "Salt Moth");
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    let out = run(&mut rig.tw, pilot, "+coop/buyout");
    let want = tacsim::port::buyout::BUYOUT_BAY
        .replace("{n}", "1")
        .replace("{name}", "Salt Moth");
    assert!(out.contains(&want), "{out}\nwanted: {want}");
    assert!(tacsim::port::live_charter(rig.tw.world(), pilot).is_some());
    // The revoke repossesses it from the shed.
    let (out, told) = heard_by(&mut rig, GOD, "@coop/revoke Pilot=test", pilot);
    assert!(out.contains("revoked from Pilot"), "{out}");
    assert!(contract_terminal(&rig.tw, record.lease));
    assert_eq!(hold_status(&rig.tw, record.hold), HoldStatus::Released);
    let after = record_on(&rig.tw, pilot, tv1);
    assert_eq!(after.status, TvLeaseStatus::Returned);
    assert!(after.repossessed);
    assert!(in_shed(&rig.tw, tv1));
    let line = REPOSSESSED
        .replace("{mark}", "OWP-TV-01")
        .replace("{name}", "Salt Moth")
        .replace("{deposit}", DEPOSIT);
    assert!(told.contains(&line), "{told}\nwanted: {line}");
    assert!(
        store(&rig.tw)
            .tv_leases_of(pilot)
            .all(|record| !record.live()),
        "no live record outlives the charter"
    );
    no_refused_writes(&rig.tw);
}

/// **`+tv/<n>` answers the unlisted rig in its slot** (reviewer finding conformance/F2):
/// staff put a rig with no lease record in a Dray-P's bay; `+tv/1` prints its
/// `RIG_UNLISTED` line (never nothing), `+tv/2` the empty slot, and the bare `+tv` the
/// empty slot then the unlisted rig.
#[test]
fn sm3a_tv_n_answers_the_unlisted_rig_in_its_slot() {
    let mut rig = stocked();
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    let bay = bay_of(&row);
    let tv3 = ObjId(rig_named(&rig.tw, "OWP-TV-03").tv);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@spatial/place/interior #{}=#{},rigs", tv3.0, bay.0),
    );
    assert!(in_bay(&rig.tw, tv3, bay), "staff placed the rig");
    let unlisted = tacsim::port::tv::RIG_UNLISTED.replace("{mark}", "OWP-TV-03");
    let empty = at_counter(&rig.tw, RIG_EMPTY).replace("{n}", "2");
    let one = run(&mut rig.tw, pilot, "+tv/1");
    assert_eq!(one.trim(), unlisted, "{one}");
    let two = run(&mut rig.tw, pilot, "+tv/2");
    assert_eq!(two.trim(), empty, "{two}");
    let all = run(&mut rig.tw, pilot, "+tv");
    assert_eq!(
        all.lines().collect::<Vec<_>>(),
        vec![empty.as_str(), unlisted.as_str()],
        "{all}"
    );
}

/// **A lost bay room never closes the desk** (D20; reviewer F13 and finding
/// conformance/F1): with one free hull's bay room destroyed, the gated hulls line still
/// reads Ready, `enrol_gate` is Ok and a pilot enrols; the ungated vehicles line names the
/// hull whose bay is not laid up.
#[test]
fn sm3a_a_lost_bay_room_never_closes_the_desk() {
    let mut rig = stocked();
    let row = port_map(&rig.tw, "ceres_port").hulls[7].clone();
    let bay = bay_of(&row);
    rig.tw.world_mut().destroy(bay).expect("destroy");
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    rig.tw.engine_mut().emit_world_event(
        &mush_core::WorldEvent::ObjDestroyed { obj: bay },
        &mut out,
        &mut fx,
    );
    let report = tacsim::port::readiness(rig.tw.world());
    let line = |binding: Binding| {
        report
            .lines
            .iter()
            .find(|line| line.binding == binding)
            .cloned()
            .expect("the line")
    };
    let hulls = line(Binding::Hulls);
    assert_eq!(hulls.state, LineState::Ready, "{}", hulls.text);
    let vehicles = line(Binding::Vehicles);
    assert_eq!(vehicles.state, LineState::Missing);
    assert_eq!(
        vehicles.text,
        format!(
            "ceres_port vehicles: hull {}'s bay is not laid up.",
            row.mark
        )
    );
    assert!(tacsim::port::enrol_gate(&report).is_ok());
    let (pilot, _) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    assert!(tacsim::port::live_charter(rig.tw.world(), pilot).is_some());
}

// ---------------------------------------------------------------------------
// 11. The lead's pre-commit round
// ---------------------------------------------------------------------------

/// **The lease's help prints money as every receipt does** (the pre-commit round, item 3):
/// the house renders major units with no thousands separator and the symbol flush, which is
/// how every preview and receipt prints the fee and the deposit (`FEE`, `DEPOSIT`, checked
/// against the live lines above); `+tv`'s help and the player's help file name them the
/// same way.
#[test]
fn sm3a_the_lease_help_names_money_as_the_receipts_do() {
    let texts = [
        ("TV_HELP", tacsim::port::tv::TV_HELP),
        (
            "docs/help/ships.md",
            include_str!("../../../docs/help/ships.md"),
        ),
    ];
    for (name, text) in texts {
        assert!(
            text.contains(&format!("{FEE} a term")) || text.contains(&format!("{FEE} for a term")),
            "{name} names the fee as {FEE}"
        );
        assert!(
            text.contains(&format!("{DEPOSIT} deposit")),
            "{name} names the deposit as {DEPOSIT}"
        );
        for spelled in ["1,400.00cr", "5,000.00cr"] {
            assert!(!text.contains(spelled), "{name} spells {spelled}");
        }
    }
}

/// **The `+tv` grammar** (the pre-commit round, item 6): two slot switches, a stray word on
/// the bare status, an unknown act and a word after `go` each answer `LEASE_USAGE`, the
/// whole reply, before anything is read.
#[test]
fn sm3a_the_tv_grammar_answers_lease_usage() {
    let mut rig = boot(Roster::bare());
    let pilot = rig.tw.spawn_player("Pilot");
    for line in [
        "+tv/1/2",
        "+tv/1/2/lease",
        "+tv sideways",
        "+tv/lease sideways",
        "+tv/lease go now",
        "+tv lease return go twice",
    ] {
        let out = run(&mut rig.tw, pilot, line);
        assert_eq!(out.trim(), LEASE_USAGE, "{line}");
    }
}

/// **A return names no rig that is not there** (the pre-commit round, item 6): a Dray-P's
/// pilot with no rig hears `NO_RIG` for rig 1 on a bare return; with rig 1 aboard,
/// `+tv/2 lease return go` hears it for rig 2 and rig 1 stays leased.
#[test]
fn sm3a_a_return_names_no_rig_that_is_not_there() {
    let mut rig = stocked();
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    let no_rig = |n: &str| NO_RIG.replace("{n}", n).replace("{name}", "Salt Moth");
    let out = at_occator(&mut rig, pilot, "+tv/lease return");
    assert_eq!(out.trim(), no_rig("1"));
    lease_one(&mut rig, pilot);
    let out = at_occator(&mut rig, pilot, "+tv/2 lease return go");
    assert_eq!(out.trim(), no_rig("2"));
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    assert_eq!(record_on(&rig.tw, pilot, tv1).status, TvLeaseStatus::Active);
    assert!(in_bay(&rig.tw, tv1, bay_of(&row)));
    no_refused_writes(&rig.tw);
}

/// **An empty shed answers `NO_RIG_FREE`** (the pre-commit round, item 6): four Dray-Ps take
/// Occator's eight rigs two apiece; a fifth pilot at the counter with an empty bay hears
/// `NO_RIG_FREE`, the whole reply, on the preview and on `go`, and nothing is written for
/// them.
#[test]
fn sm3a_an_empty_shed_answers_no_rig_free() {
    let mut rig = stocked();
    for (who, seed, name) in [
        ("Ann", 1, "Salt Moth"),
        ("Bea", 2, "Tally Moth"),
        ("Cal", 3, "Cold Moth"),
        ("Dee", 4, "Dry Moth"),
    ] {
        let (pilot, _) = prospector(&mut rig, who, seed, name);
        lease_one(&mut rig, pilot);
        lease_one(&mut rig, pilot);
    }
    assert!(
        rigs(&rig.tw)
            .iter()
            .all(|row| !in_shed(&rig.tw, ObjId(row.tv))),
        "the shed is empty"
    );
    let (eve, _) = prospector(&mut rig, "Eve", 5, "Even Moth");
    let account = account_of(&rig.tw, eve);
    let before = balances(&rig.tw, account);
    for line in ["+tv/lease", "+tv/lease go"] {
        let out = at_occator(&mut rig, eve, line);
        assert_eq!(out.trim(), NO_RIG_FREE, "{line}");
    }
    assert!(records(&rig.tw, eve).is_empty());
    assert_eq!(balances(&rig.tw, account), before);
    no_refused_writes(&rig.tw);
}

/// **The refit's grammar** (the pre-commit round, item 6): no word, two words and a word the
/// programme does not author each answer `REFIT_USAGE`, the whole reply, before the charter
/// is read.
#[test]
fn sm3a_the_refit_grammar_answers_refit_usage() {
    let mut rig = boot(Roster::bare());
    let pilot = rig.tw.spawn_player("Pilot");
    for line in [
        "+coop/refit",
        "+coop/refit prospector now",
        "+coop/refit tanker",
    ] {
        let out = run(&mut rig.tw, pilot, line);
        assert_eq!(out.trim(), REFIT_USAGE, "{line}");
    }
}

/// **A programme with no refit row offers none** (the pre-commit round, item 6): on the
/// shipped lane with SM.3a's blocks stripped (numbered as the shipped version, so the desk
/// is open), a chartered pilot at the desk hears `REFIT_NOT_OFFERED`, the whole reply, for
/// either word, and the hull stays a Dray.
#[test]
fn sm3a_a_programme_without_refits_answers_refit_not_offered() {
    let mut rig = stocked_on(Roster::seeded(lane_variant(LANE_VERSION, strip_vehicles)));
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    for word in ["prospector", "standard"] {
        let out = refit(&mut rig, pilot, word);
        assert_eq!(out.trim(), REFIT_NOT_OFFERED, "{word}");
    }
    assert_eq!(unit_id(&rig.tw, ObjId(row.hull)), DRAY);
    no_refused_writes(&rig.tw);
}

/// A chartered Dray-P with rig 1 in her bay whose unit staff then re-create as a line tank
/// (`@unit/remove`, `@unit/create`): the hull's live unit names a definition with no
/// interior, so `hull_view` reads `None` (correctness finding F5's case).
fn dark_view(rig: &mut Rig) -> (ObjId, HullRow, ObjId) {
    let (pilot, row) = prospector(rig, "Pilot", 1, "Salt Moth");
    lease_one(rig, pilot);
    let hull = ObjId(row.hull);
    let removed = run(&mut rig.tw, GOD, &format!("@unit/remove #{}", hull.0));
    let made = run(
        &mut rig.tw,
        GOD,
        &format!("@unit/create #{}={LINE_TANK}", hull.0),
    );
    assert_eq!(unit_id(&rig.tw, hull), LINE_TANK, "{removed}\n{made}");
    assert!(
        tacsim::port::ship::hull_view(rig.tw.world(), hull).is_none(),
        "the hull's view is dark"
    );
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    assert!(in_bay(&rig.tw, tv1, bay_of(&row)));
    (pilot, row, tv1)
}

/// Rig 1 still leased and aboard, and the charter standing.
fn still_aboard(rig: &Rig, pilot: ObjId, row: &HullRow, tv1: ObjId) {
    let record = record_on(&rig.tw, pilot, tv1);
    assert_eq!(record.status, TvLeaseStatus::Active, "nothing written");
    assert!(!contract_terminal(&rig.tw, record.lease));
    assert_eq!(hold_status(&rig.tw, record.hold), HoldStatus::Held);
    assert!(in_bay(&rig.tw, tv1, bay_of(row)));
    assert!(tacsim::port::live_charter(rig.tw.world(), pilot).is_some());
    assert_eq!(
        store(&rig.tw).enrolment_of(pilot).map(|e| e.status),
        Some(tacsim::port::EnrolmentStatus::Active)
    );
}

/// **`+coop/release` refuses when the hull's view is dark** (the pre-commit round, item 4;
/// correctness F5): the bay cannot be seen, so the release answers the rigs' dark line and
/// the charter and the rig stand.
#[test]
fn sm3a_a_dark_hull_view_refuses_the_release() {
    let mut rig = stocked();
    let (pilot, row, tv1) = dark_view(&mut rig);
    let desk = desk(&rig.tw);
    teleport(&mut rig.tw, pilot, desk);
    let out = run(&mut rig.tw, pilot, "+coop/release");
    assert_eq!(out.trim(), TV_DARK.replace("{name}", "Salt Moth"));
    still_aboard(&rig, pilot, &row, tv1);
    no_refused_writes(&rig.tw);
}

/// **`+coop/buyout` refuses when the hull's view is dark** (the pre-commit round, item 4;
/// correctness F5): funds for the buyout, the bay unseen; the buyout answers the rigs' dark
/// line and files nothing.
#[test]
fn sm3a_a_dark_hull_view_refuses_the_buyout() {
    let mut rig = stocked();
    let (pilot, row, tv1) = dark_view(&mut rig);
    top_up(&mut rig.tw, pilot, 10_000_000);
    let desk = desk(&rig.tw);
    teleport(&mut rig.tw, pilot, desk);
    let out = run(&mut rig.tw, pilot, "+coop/buyout");
    assert_eq!(out.trim(), TV_DARK.replace("{name}", "Salt Moth"));
    assert_eq!(
        store(&rig.tw).enrolment_of(pilot).and_then(|e| e.buyout),
        None,
        "no buyout contract"
    );
    still_aboard(&rig, pilot, &row, tv1);
    no_refused_writes(&rig.tw);
}

/// **`@coop/revoke` holds when the hull's view is dark** (the pre-commit round, item 4;
/// correctness F5): the repossession cannot see the bay either, so the revoke answers
/// `NOT_REVOKED` with the rigs' dark line and nothing is written (the hold a hopper that is
/// not empty makes).
#[test]
fn sm3a_a_dark_hull_view_holds_the_revoke() {
    let mut rig = stocked();
    let (pilot, row, tv1) = dark_view(&mut rig);
    let out = run(&mut rig.tw, GOD, "@coop/revoke Pilot=test");
    let want = format!(
        "{}{}",
        tacsim::port::recall::NOT_REVOKED,
        TV_DARK.replace("{name}", "Salt Moth")
    );
    assert_eq!(out.trim(), want);
    still_aboard(&rig, pilot, &row, tv1);
    no_refused_writes(&rig.tw);
}

/// **The dormancy recall holds and pages when the hull's view is dark** (the pre-commit
/// round, item 4; correctness F5): a week without a departure, the bay unseen; the fire
/// holds the hull, the log carries the bay page verbatim, and the charter and the rig
/// stand.
#[test]
fn sm3a_a_dark_hull_view_holds_the_dormancy_recall_and_pages() {
    let mut rig = stocked();
    let (pilot, row, tv1) = dark_view(&mut rig);
    skip_days(&mut rig, 8);
    let capture = Capture::default();
    let dispatch = capture_dispatch(capture.clone());
    tracing::dispatcher::with_default(&dispatch, || {
        let _ = fire_quietly(&mut rig);
    });
    assert!(
        capture.text().contains(tacsim::port::recall::BAY_PAGE),
        "{}",
        capture.text()
    );
    still_aboard(&rig, pilot, &row, tv1);
    no_refused_writes(&rig.tw);
}

/// [`reboot`], with `edit` applied first to the stored document under `key` (a test rig:
/// staff's hand on the document, or a store fault).
fn reboot_editing(mut rig: Rig, key: &str, edit: impl FnOnce(&mut Value)) -> Rig {
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    let doc = rig
        .store
        .values_mut()
        .flat_map(|data| data.kv.iter_mut())
        .find(|(k, _)| k.as_str() == key)
        .map(|(_, value)| value)
        .unwrap_or_else(|| panic!("{key} is in the store"));
    edit(doc);
    reboot(rig)
}

/// **A hull whose unit record boot held reads dark** (the pre-commit round, item 6;
/// reviewer F6): a Dray-P with rig 1 aboard, its `unit:<hull>` document given a record
/// version no decoder reads and the world rebooted, so the record is quarantined and the
/// hull's unit is Held: `+tv` answers `TV_DARK`, `+ship/plan` the planner's dark line (the
/// basis is `PlanError::Unreadable`) and `+coop/refit standard` the bay's refusal (the bay
/// is checked first), each the whole reply, and nothing moves; with the bay empty the same
/// held record answers the refit `REFIT_FAILED`.
#[test]
fn sm3a_a_held_unit_record_reads_dark() {
    let mut rig = stocked();
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    lease_one(&mut rig, pilot);
    let hull = ObjId(row.hull);
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    let mut rig = reboot_editing(rig, &format!("unit:{}", hull.0), |doc| {
        doc["record_version"] = serde_json::json!(99);
    });
    assert!(
        mush_tactics::module::unit_of(rig.tw.world(), hull).is_none(),
        "boot held the record"
    );
    assert!(
        tacsim::port::ship::hull_view(rig.tw.world(), hull).is_some_and(|view| view.unit_held),
        "the view reads the unit as held"
    );
    let out = run(&mut rig.tw, pilot, "+tv");
    assert_eq!(out.trim(), TV_DARK.replace("{name}", "Salt Moth"));
    let out = run(&mut rig.tw, pilot, "+ship/plan stickney");
    let dark = tacsim::port::voyage::PLAN_DARK
        .replace("{name}", "Salt Moth")
        .replace(tacsim::port::voyage::NOTHING_BURNED, "");
    assert_eq!(out.trim(), dark);
    // The refit's bay check (step 7) comes before its held check: a held record never
    // hides a rig from the yard.
    let out = refit(&mut rig, pilot, "standard");
    assert_eq!(
        out.trim(),
        REFIT_BAY.replace("{n}", "1").replace("{name}", "Salt Moth")
    );
    still_aboard(&rig, pilot, &row, tv1);
    no_refused_writes(&rig.tw);

    // A second world: the same held record with the bay empty reaches REFIT_FAILED.
    let mut rig = stocked();
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    let mut rig = reboot_editing(rig, &format!("unit:{}", hull.0), |doc| {
        doc["record_version"] = serde_json::json!(99);
    });
    let out = refit(&mut rig, pilot, "standard");
    assert_eq!(out.trim(), REFIT_FAILED.replace("{name}", "Salt Moth"));
    assert!(
        mush_tactics::module::unit_of(rig.tw.world(), hull).is_none(),
        "nothing re-bound the held hull"
    );
    no_refused_writes(&rig.tw);
}

/// A world on the shipped lane with the shipped lane one version on, edited by `edit`, on
/// the shelf: booted, settled and stocked, ready for `plan_and_apply`.
fn with_next(edit: impl FnOnce(&mut Value)) -> Rig {
    let next = lane_variant(LANE_VERSION + 1, edit);
    let roster = Roster {
        shelf: Some(vec![
            mush_content::load_pack(&next).expect("the next lane verifies"),
        ]),
        extra_lane: Some(next),
        ..Roster::default()
    };
    stocked_on(roster)
}

/// `ceres_port`'s vehicles block in a lane document.
fn vehicles_block(doc: &mut Value) -> &mut Value {
    &mut doc["entries"]["ceres_port"]["def"]["vehicles"]
}

/// Destroy `obj` as staff would, with the world event the engine emits.
fn destroy(rig: &mut Rig, obj: ObjId) {
    rig.tw.world_mut().destroy(obj).expect("destroy");
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    rig.tw.engine_mut().emit_world_event(
        &mush_core::WorldEvent::ObjDestroyed { obj },
        &mut out,
        &mut fx,
    );
}

/// The apply of the next lane is refused with every one of `words`, and nothing of the rig
/// pool moved.
fn assert_apply_refused(rig: &mut Rig, case: &str, words: &[&str]) {
    let before = rigs(&rig.tw);
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    for word in words {
        assert!(apply.contains(word), "{case}: {word}\n{plan}\n{apply}");
    }
    assert!(!apply.contains("Applied "), "{case}:\n{apply}");
    assert_eq!(rigs(&rig.tw), before, "{case}: the rig pool did not move");
}

/// **The rig pool's preflight refuses** (the pre-commit round, item 6): an update of
/// `ceres_port` that would retire, renumber or re-mark a recorded rig, name an unregistered
/// title issuer, or lay up a pool over a unit with no interior, and an otherwise harmless
/// update (the fee) over a recorded rig or shed staff destroyed, or over a rig row that (a
/// store fault: the port map edited before a reboot) records a room its template does not
/// author or names a hopper vault `tacsim.vehicles` does not own: each refused before any
/// write with its own sentence, every rig where it was.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3a_the_rig_pools_preflight_refuses_each_recorded_breach() {
    let key = tacsim::port::port_map_key(&lane_id(), &cid("ceres_port"));
    let mut rig = with_next(|doc| {
        if let Some(def) = doc["entries"]["ceres_port"]["def"].as_object_mut() {
            def.remove("vehicles");
        }
    });
    assert_apply_refused(
        &mut rig,
        "the block dropped",
        &[&format!(
            "ceres_port: the port map {key} records 8 rigs but the entry has no vehicles \
             block; rigs are never retired by a port update"
        )],
    );
    let mut rig = with_next(|doc| vehicles_block(doc)["pool"] = serde_json::json!(4));
    assert_apply_refused(
        &mut rig,
        "the pool shrunk",
        &[&format!(
            "ceres_port: rig OWP-TV-05 is in the port map {key} but the pool is 4; rigs are \
             never retired by a port update"
        )],
    );
    let mut rig = with_next(|doc| {
        vehicles_block(doc)["registry_prefix"] = serde_json::json!("OWP-RV-");
    });
    assert_apply_refused(
        &mut rig,
        "the marks changed",
        &[&format!(
            "ceres_port: rig OWP-TV-01 in the port map {key} is not OWP-RV-01; registry marks \
             are never re-issued"
        )],
    );
    let mut rig = with_next(|doc| {
        vehicles_block(doc)["issuer"] = serde_json::json!("tacsim.nobody");
    });
    assert_apply_refused(
        &mut rig,
        "the issuer unregistered",
        &[
            "ceres_port: vehicles occator.rig_lease: the title issuer tacsim.nobody is not \
           registered on this world",
        ],
    );
    let mut rig = with_next(|doc| {
        vehicles_block(doc)["unit"] = serde_json::json!(LINE_TANK);
    });
    assert_apply_refused(
        &mut rig,
        "the unit has no interior",
        &[&format!(
            "ceres_port: vehicles occator.rig_lease: unit {LINE_TANK} carries no interior \
             template or no logistics"
        )],
    );
    // The fee moved, so the update reaches the preflight over what staff destroyed.
    let fee = |doc: &mut Value| vehicles_block(doc)["fee_major"] = serde_json::json!(1_500);
    let mut rig = with_next(fee);
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    destroy(&mut rig, tv1);
    assert_apply_refused(
        &mut rig,
        "a rig destroyed",
        &[&format!(
            "ceres_port: rig OWP-TV-01 in the port map {key} names #{}, which does not exist; \
             it is never re-created — repair the map by hand",
            tv1.0
        )],
    );
    let mut rig = with_next(fee);
    let shed = shed(&rig.tw);
    destroy(&mut rig, shed);
    assert_apply_refused(
        &mut rig,
        "the shed destroyed",
        &[&format!(
            "ceres_port: the port map {key} names the shed #{}, which does not exist; it is \
             never re-created — repair the map by hand",
            shed.0
        )],
    );
    // A store fault (reviewer R2): the port map edited before a reboot so rig 1's row
    // records a room the rig's template does not author.
    let mut rig = reboot_editing_map(with_next(fee), |doc| {
        let rooms = doc["vehicles"]["rigs"][0]["rooms"]
            .as_object_mut()
            .expect("the rig's rooms");
        let hopper = rooms.remove("hopper").expect("the hopper room");
        rooms.insert("galley".to_owned(), hopper);
    });
    assert_apply_refused(
        &mut rig,
        "a recorded room the template does not author",
        &[&format!(
            "ceres_port: rig OWP-TV-01 in the port map {key} records room galley, which the \
             template does not author"
        )],
    );
    // A store fault (reviewer R2; doc 20 §4.2, no player owns a hopper): rig 1's row
    // names a live vault `tacsim.vehicles` does not own, a chartered hull's hold.
    let mut rig = with_next(fee);
    let (_, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hold = hold_vault(&rig.tw, &row);
    let vehicles = PrincipalId::System(SystemPrincipalId::new("tacsim.vehicles").expect("an id"));
    assert!(
        economy(&rig.tw)
            .snapshot()
            .vault(hold)
            .is_some_and(|v| v.owner != vehicles),
        "the hold vault is live and not the vehicles principal's"
    );
    let mut rig = reboot_editing_map(rig, |doc| {
        doc["vehicles"]["rigs"][0]["vault"] = serde_json::json!(hold.0);
    });
    assert_apply_refused(
        &mut rig,
        "a hopper vault tacsim.vehicles does not own",
        &[&format!(
            "ceres_port: rig OWP-TV-01 in the port map {key} names hopper vault #{}, which is \
             gone or not owned by tacsim.vehicles; it is never re-created — repair the map by \
             hand",
            hold.0
        )],
    );
}

/// The readiness report's vehicles line: its state and its text.
fn vehicles_line(tw: &TestWorld) -> (LineState, String) {
    tacsim::port::readiness(tw.world())
        .lines
        .into_iter()
        .find(|line| line.binding == Binding::Vehicles)
        .map(|line| (line.state, line.text))
        .expect("the vehicles line")
}

/// [`reboot_editing`] on `ceres_port`'s stored port map.
fn reboot_editing_map(rig: Rig, edit: impl FnOnce(&mut Value)) -> Rig {
    let key = tacsim::port::port_map_key(&lane_id(), &cid("ceres_port"));
    reboot_editing(rig, &key, edit)
}

/// **The vehicles line names what is missing** (the pre-commit round, item 6): a rig staff
/// destroyed, a rig titled away from Occator, and (a store fault: the port map edited
/// before a reboot) two rig rows gone, a rig row without its hopper room and a rig row
/// without its hopper vault: each reads `Missing` with its own text, and the gated hulls
/// line never moves.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3a_the_vehicles_line_names_what_is_missing() {
    let missing = |text: &str| (LineState::Missing, format!("ceres_port vehicles: {text}"));
    // A rig destroyed.
    let mut rig = stocked();
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    destroy(&mut rig, tv1);
    assert_eq!(
        vehicles_line(&rig.tw),
        missing(&format!("OWP-TV-01 (#{}) does not exist.", tv1.0))
    );
    // A rig titled away from Occator.
    let mut rig = stocked();
    let tv1 = ObjId(rig_named(&rig.tw, "OWP-TV-01").tv);
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/transfer #{}=#{}", tv1.0, GOD.0),
    );
    assert!(out.contains("Title transferred"), "{out}");
    assert_eq!(
        vehicles_line(&rig.tw),
        missing("OWP-TV-01 is not titled to Occator Water & Propellant.")
    );
    // Two rig rows gone from the map.
    let rig = reboot_editing_map(stocked(), |doc| {
        let rows = doc["vehicles"]["rigs"]
            .as_array_mut()
            .expect("the rig rows");
        rows.truncate(6);
    });
    assert_eq!(vehicles_line(&rig.tw), missing("6 of 8 are made."));
    // A rig row without its hopper room.
    let rig = reboot_editing_map(stocked(), |doc| {
        doc["vehicles"]["rigs"][0]["rooms"]
            .as_object_mut()
            .expect("the rig's rooms")
            .remove("hopper");
    });
    assert_eq!(
        vehicles_line(&rig.tw),
        missing("OWP-TV-01's interior is incomplete.")
    );
    // A rig row without its hopper vault.
    let rig = reboot_editing_map(stocked(), |doc| {
        doc["vehicles"]["rigs"][0]["vault"] = Value::Null;
    });
    assert_eq!(
        vehicles_line(&rig.tw),
        missing("OWP-TV-01 has no hopper vault.")
    );
    let hulls = tacsim::port::readiness(rig.tw.world())
        .lines
        .into_iter()
        .find(|line| line.binding == Binding::Hulls)
        .map(|line| line.state)
        .expect("the hulls line");
    assert_eq!(
        hulls,
        LineState::Ready,
        "mining content never closes the desk"
    );
}
