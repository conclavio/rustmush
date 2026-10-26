//! **Doc 20 cut SM.11 acceptance — plans and offline** (doc 20 §1 beat 11, §5 `via: Plan`, §8
//! "Leaving it running", §10, §11's Tick, Caps and Income rows; D-20-6, D-20-9, D-20-11; Q-20-3;
//! the SM.11 brief's acceptance A1–A6 and its tests P1–P13 in 3.12).
//!
//! A pilot with readings files a **fill plan** for a rig (`+tv/1 plan <pockets and blocks> go`);
//! from then on the `tacsim.surface` tick works it as the pilot, one existing act at a time —
//! drives, soundings, bare mining runs, recovers (SM.8's queued unload follows by itself) — and
//! parks it when the hold has no room, the list is worked out, its 288 IC hours run or a right
//! lapses. Every reading it makes is `via: Plan`. The captain's queue waits for the rigs with
//! `await tv`; a rig left idle a day comes home; offline, the pilot is told one line a trip and
//! one summary a plan.
//!
//! Composed on the composition root (`unloading.rs`'s rig, copied: the per-file precedent),
//! **booted keyed** with the tacsim suites' fixed geology key; Ahuna Flank opened once a world.
//! Every act goes through the real verb, the real land path and the real tick. Pockets are chosen
//! from the keyed truth (`picks`) and printed (`--nocapture`) for the report.

use crate::support;

use crate::geology_keyed;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use mush_core::{
    AccountId, AccountIdentity, ActionStateBootRecord, Audience, AuthoritativeSubmit, CanonicalId,
    CommitToken, GOD, IdempotencyKey, IdempotencyKeyText, IdempotencyScope, ModuleWrite, ObjId,
    OfferExecutionContext, OfferSubmission, PerceivedRef, RequestUid, ScopedIdempotencyKey, World,
};
use mush_economy::{EconomyConfig, VaultId};
use mush_industry::OreDepositId;
use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde_json::{Value, json};
use smol_str::SmolStr;
use tacsim::definitions::content::{DRAY_P, LANE_JSON, LANE_PACK};
use tacsim::definitions::tools::{BatteryDef, ToolDef, tools_table};
use tacsim::pack::{PackEntry, RulesPack, load_pack};
use tacsim::port::licence::{LICENCE_LAPSED, ic_stamp, licence_key};
use tacsim::port::orders::{OrderState, Orders, OrdersState};
use tacsim::port::tv::{
    ACT_FAILED, CAP_OPERATOR, END_BOUND, MINE_CLAIMED, MINE_NO_LICENCE, NO_READING, PLAN_ALREADY,
    PLAN_CLEARED, PLAN_DROPPED, PLAN_ENDED, PLAN_HOLD_FULL, PLAN_LIVE, PLAN_NONE, PLAN_PARKING,
    PLAN_PREVIEW_HEAD, PLAN_PREVIEW_STEP, PLAN_PREVIEW_TAIL, PLAN_REMEDY, PLAN_SHARED,
    PLAN_STEP_HOURS, PLAN_TOO_MANY, PLAN_USAGE, PLAN_WAITS, RECOVERED_UNLOADING, UNLOADED,
    WAIT_ASHORE, WAIT_BUSY, WAIT_CAP, WAIT_HELM, WAIT_MARKERS, WHY_MARKERS_QUEUED, ore_words,
};
use tacsim::port::{HullRow, PortHandles, PortMap, PortMapState, Ticket, TicketId, room_key};
use tacsim::surface::cursor::PocketCursor;
use tacsim::surface::geology::{OreFamily, Word};
use tacsim::surface::ground::{SiteGeologySpec, SiteGround, site_ground_sync};
use tacsim::surface::mined::{MinedFact, MinedState};
use tacsim::surface::plan::{ActBy, FillPlan, FillState, PlanEnd, PlanNoteKind, PlanStep};
use tacsim::surface::reading::Footing;
use tacsim::surface::{
    ActKind, Charge, GeologyBoot, GeologyState, RigKey, RigRecord, Run, SURFACE_ACT_ACTION,
    SURFACE_DONE_ACTION,
};
use tacsim::surface::{ChartKey, ChartProvenance, ChartRecord, TvRec, TvRecState, TvState, Via};
use tacsim::{TacsimConfig, TacsimStore};

use geology_keyed::{KEY_1, secret};

/// The ramp's cell.
const RAMP_CELL: (i64, i64) = (128, 124);
/// Ahuna Flank's port entry.
const AHUNA: &str = "ahuna_port";
/// The usual hull's name.
const HULL: &str = "Salt Moth";

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

// ---------------------------------------------------------------------------
// The rig (mining.rs's, booted keyed)
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
    /// **The harness's idempotency ledger** (`mining.rs`'s): the identity of every submission
    /// that landed, over exactly the fields `mush_server`'s `ActionCoordinator::command_ingress`
    /// digests into the request's idempotency key. A second landing of an identity already here
    /// is the world task's HIT ([`land`] returns [`HIT`]). It survives a restart.
    ledger: std::collections::BTreeSet<String>,
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
            mush_core::OfferSecret::new(1, [0x53; 32]).expect("an offer secret"),
            [],
        )
        .expect("an offer key ring"),
    );
}

/// **Compose over `world`, hand the key in, then run the boot pass over `store`**
/// (`build_engine`'s order).
fn boot_over(world: World, store: HashMap<SmolStr, mush_core::ModuleBootData>) -> TestWorld {
    let (_, registry) = mush_testkit::boot_world();
    let mut engine = mush_core::Engine::with_modules(world, registry, Roster.modules())
        .expect("the roster composes");
    assert!(tacsim::surface::install_geology(
        &mut engine.world,
        GeologyBoot::Key(secret(KEY_1))
    ));
    let (mut out, mut fx) = (mush_core::Outbox::new(), mush_core::Effects::new());
    engine
        .modules_loaded(store, &mut out, &mut fx)
        .expect("the world boots");
    let tw = TestWorld::from_engine(engine);
    no_refused_writes(&tw);
    tw
}

/// Boot keyed, settle and stock every port, at world rate 1.
fn stocked() -> Rig {
    let (world, _) = mush_testkit::boot_world();
    let mut tw = boot_over(world, HashMap::new());
    with_offer_keys(&mut tw);
    let oracle = Arc::new(MapOracle::default());
    install_oracle(&mut tw, &oracle);
    let mut rig = Rig {
        tw,
        oracle,
        store: HashMap::new(),
        typed: BTreeMap::new(),
        ledger: std::collections::BTreeSet::new(),
    };
    sweep(&mut rig.tw, 120);
    stock(&mut rig);
    set_rate(&mut rig.tw, 1);
    rig
}

/// Fold every write since the first boot and reboot keyed into the same roster; every connection
/// drops (a real restart has no sessions) and `reconnect` come back.
fn reboot(rig: Rig, reconnect: &[ObjId]) -> Rig {
    let Rig {
        mut tw,
        oracle,
        mut store,
        typed,
        ledger,
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
    let mut tw = boot_over(world, store.clone());
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
        ledger,
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
    let mut bytes = [0x75_u8; 16];
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

/// What [`land`] answers for a submission already in the harness's ledger (the HIT).
const HIT: &str = "HIT: answered from the idempotency record";

/// **A submission's ledger identity** — the fields `ActionCoordinator::command_ingress` digests
/// into its request uid.
fn ledger_identity(submit: &AuthoritativeSubmit) -> String {
    json!({
        "actor": submit.actor,
        "action": submit.action,
        "subject": submit.subject,
        "targets": submit.targets,
        "parameters": submit.parameters,
        "key": submit.key,
    })
    .to_string()
}

/// A submission resolved against the world as it stands — the offer fixed — and not yet
/// prepared (U9's race holds two across each other's landing).
struct Resolved {
    action: mush_core::DefinitionRef,
    resolved: mush_core::ResolvedActionInvocation,
    token: [u8; 16],
}

/// **The landing's first half**: the offer projected and resolved under a fresh request token.
fn resolve(rig: &Rig, submit: &AuthoritativeSubmit) -> Result<Resolved, String> {
    let token = next_token();
    let tw = &rig.tw;
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
    Ok(Resolved {
        action,
        resolved,
        token,
    })
}

/// **The landing's second half**: the provider's prepare over the world as it stands now, the
/// bundle installed and its typed post-images kept; the action outcome's JSON. The error carries
/// the prepare's own message.
fn prepare(rig: &mut Rig, resolved: &Resolved) -> Result<Value, String> {
    let (prepared, output) = rig
        .tw
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(
            &resolved.action,
            &resolved.resolved,
            CommitToken::from_bytes(resolved.token),
        )
        .map_err(|error| format!("{error:?}"))?;
    let outcome = output.outcome().data().clone();
    let (ops, install) = prepared.into_parts();
    rig.tw
        .engine_mut()
        .install_prepared_action(install)
        .map_err(|error| format!("{error:?}"))?;
    keep(rig, ops);
    Ok(outcome)
}

/// **Drive one queued submission the way the world task does** — the ledger first (an identity
/// that already landed is the HIT), then [`resolve`] and [`prepare`] — recording the identity once
/// it lands.
fn land(rig: &mut Rig, submit: &AuthoritativeSubmit) -> Result<(), String> {
    let identity = ledger_identity(submit);
    if rig.ledger.contains(&identity) {
        return Err(HIT.to_owned());
    }
    let resolved = resolve(rig, submit)?;
    prepare(rig, &resolved)?;
    rig.ledger.insert(identity);
    Ok(())
}

/// Every queued submission of the port's lanes and industry's, dropping other modules'; asserts
/// none kept of tacsim's is GOD's.
fn drain(tw: &mut TestWorld) -> Vec<AuthoritativeSubmit> {
    let mut kept = Vec::new();
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        let owner = submit.action.key().owner.as_str();
        if owner == tacsim::NS {
            assert_ne!(submit.actor, GOD, "GOD never submits from a tick");
        }
        if owner == tacsim::NS || owner == "inventory" || owner == "market" || owner == "industry" {
            kept.push(*submit);
        }
    }
    kept
}

/// Land every submission; each must land or be the ledger's HIT.
fn land_all(rig: &mut Rig, queued: &[AuthoritativeSubmit]) {
    for submit in queued {
        land(rig, submit)
            .or_else(|error| if error == HIT { Ok(()) } else { Err(error) })
            .unwrap_or_else(|error| {
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

/// The surface finishes a fire queued — the players' (the house's dropped: doc 20 cut SM.12b).
fn finishes<'q>(tw: &TestWorld, queued: &'q [AuthoritativeSubmit]) -> Vec<&'q AuthoritativeSubmit> {
    queued
        .iter()
        .filter(|submit| submit.action.key().id.as_str() == SURFACE_DONE_ACTION)
        .filter(|submit| !support::drones::is_house(tw, submit))
        .collect()
}

/// Fire and land the port tick until every port is stocked and listed.
fn stock(rig: &mut Rig) {
    for _ in 0..40 {
        let out = run(&mut rig.tw, GOD, "@tacsim/port status");
        let gated: Vec<&str> = out
            .lines()
            .filter(|line| {
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

/// The hull's hold vault.
fn hold_vault(tw: &TestWorld, row: &HullRow) -> VaultId {
    let stowage = ObjId(row.rooms[&cid("stowage")]);
    tw.world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy")
        .snapshot()
        .vault_by_backing(&mush_economy::CustodyRef::SpatialInterior(stowage))
        .expect("the hold vault")
        .id
}

/// `tank` t and `locker` crew-days into the hold.
fn load(rig: &mut Rig, row: &HullRow, tank: u64, locker: u64, lot: u64) {
    let vault = hold_vault(&rig.tw, row);
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

/// Sweep to the first real second whose IC instant is at or past `target`; the events and the
/// submissions queued (not landed).
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
fn to_ic(rig: &mut Rig, target: i64) {
    let (_, queued) = advance_to(rig, target);
    land_quietly(rig, &queued);
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

/// **A Dray-P on Ahuna Flank's pad**, its pilot aboard, `n_rigs` rigs leased; the site opened
/// first (its pad is closed until then) and its ground and home warmed.
fn on_pad_as(
    rig: &mut Rig,
    (who, seed, name): (&str, i64, &str),
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
    let mut kept = Vec::new();
    let _ = support::sites::open_site_keeping(&mut rig.tw, AHUNA, &mut |ops| kept.push(ops));
    for ops in kept {
        keep(rig, ops);
    }
    let t = file_and_launch(rig, pilot, hull, "+ship/undock ahuna");
    to_ic(rig, t.plan.depart_ic + t.plan.total_s);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == AHUNA
        ),
        "the hull is settled on Ahuna's pad"
    );
    warm(rig);
    let tvs = rigs_of(&rig.tw, pilot);
    assert_eq!(tvs.len(), n_rigs);
    (pilot, row, tvs)
}

/// [`on_pad_as`] for the usual pilot.
fn on_pad(rig: &mut Rig, n_rigs: usize) -> (ObjId, HullRow, Vec<ObjId>) {
    on_pad_as(rig, ("Pilot", 1, HULL), n_rigs)
}

/// Fire until Ahuna's ground is cached and its home field built.
fn warm(rig: &mut Rig) {
    let (spec, location) = spec_of(&rig.tw);
    for _ in 0..8 {
        let cached =
            tacsim::surface::site_ground_cached(rig.tw.world(), &site_frame(&location), &spec)
                .ok()
                .flatten()
                .is_some();
        let homed = rig
            .tw
            .world()
            .module_state::<GeologyState>()
            .is_some_and(|state| {
                state
                    .homes()
                    .iter()
                    .any(|(site, ramp, _)| site.as_str() == AHUNA && *ramp == RAMP_CELL)
            });
        if cached && homed {
            return;
        }
        let (_, queued) = fire(rig);
        land_quietly(rig, &queued);
    }
    panic!("Ahuna's ground was not warmed");
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

/// Ahuna's spec from its own lane entry and the world's installed body table.
fn spec_of(tw: &TestWorld) -> (SiteGeologySpec, String) {
    let pack = lane();
    let def = match pack.entry(&cid(AHUNA)) {
        Some(PackEntry::Port(def)) => def.clone(),
        other => panic!("Ahuna is a port entry: {other:?}"),
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
    (spec, def.location.to_string())
}

/// Ahuna's keyed ground, as a test reads it (a cache hit once warm).
fn ground_of(tw: &mut TestWorld) -> Arc<SiteGround> {
    let (spec, location) = spec_of(tw);
    site_ground_sync(tw.world_mut(), &site_frame(&location), &spec).expect("the site builds")
}

/// **One pocket of Ahuna, from the keyed truth**: its address, the stand cell at its middle, the
/// classes and tonnes per family, the footing at the stand.
#[derive(Clone, Debug)]
struct Pick {
    addr: String,
    pocket: (i64, i64),
    stand: (i64, i64),
    classes: [Word; 4],
    tonnes: [u8; 4],
    footing: Footing,
}

impl Pick {
    /// The families with ore.
    fn families(&self) -> usize {
        self.classes.iter().filter(|c| **c != Word::Barren).count()
    }

    /// The block letter.
    fn letter(&self) -> char {
        self.addr.chars().next().expect("an address")
    }

    /// The deposit of `family` at this pocket's block.
    fn deposit(&self, family: OreFamily) -> OreDepositId {
        mush_industry::derive_system_deposit_id(&tacsim::surface::opening::receipt(
            &cid(AHUNA),
            self.letter(),
            family,
        ))
        .expect("a deposit id")
    }
}

/// **Every pocket of Ahuna with a footing at its middle**, read from the site's keyed table and
/// its composite — the truth a test may read, never a player.
fn picks(tw: &mut TestWorld) -> Vec<Pick> {
    let ground = ground_of(tw);
    let surface = ground
        .composite
        .surface(&mush_atlas::SurfaceRef::Ground)
        .expect("a ground surface");
    let mut out = Vec::new();
    for py in 0..64_i64 {
        for px in 0..64_i64 {
            let Some(row) = ground.geology.pocket(px, py) else {
                continue;
            };
            let stand = (px * 4 + 2, py * 4 + 2);
            let Ok(footing) = tacsim::surface::chart::footing_at(surface, stand) else {
                continue;
            };
            out.push(Pick {
                addr: tacsim::surface::address::pocket_address(px, py).expect("an address"),
                pocket: (px, py),
                stand,
                classes: row.classes,
                tonnes: row.tonnes,
                footing,
            });
        }
    }
    out
}

/// The first pick that `wanted` takes, named for the report (`--nocapture`).
fn pick(all: &[Pick], name: &str, wanted: impl Fn(&Pick) -> bool) -> Pick {
    let found = all
        .iter()
        .find(|p| wanted(p))
        .cloned()
        .unwrap_or_else(|| panic!("no pocket for {name}"));
    println!(
        "{name}: {} pocket {:?} stand {:?} classes {:?} tonnes {:?} footing {:?}",
        found.addr, found.pocket, found.stand, found.classes, found.tonnes, found.footing
    );
    found
}

/// **`FAIR7`**: fair metals, 7 t, the only family, on regolith or ice (`mining.rs`'s M1 pocket).
fn fair7(all: &[Pick]) -> Pick {
    pick(all, "FAIR7", |p| {
        p.footing != Footing::Rock
            && p.families() == 1
            && p.classes[0] == Word::Fair
            && p.tonnes[0] == 7
    })
}

/// A fair single-family metal pocket on soft ground other than `not`.
fn fair_other(all: &[Pick], name: &str, not: &[&Pick]) -> Pick {
    pick(all, name, |p| {
        p.footing != Footing::Rock
            && p.families() == 1
            && p.classes[0] == Word::Fair
            && not.iter().all(|n| n.pocket != p.pocket)
    })
}

/// A rig's typed record.
fn record_of(tw: &mut TestWorld, tv: ObjId) -> Option<RigRecord> {
    let h = handles(tw);
    typed(tw, &h.families.excursions, &RigKey(tv))
}

/// A rig's run.
fn run_of(tw: &mut TestWorld, tv: ObjId) -> Option<Run> {
    record_of(tw, tv).and_then(|record| record.run)
}

/// A pool's quantity.
fn pool(tw: &mut TestWorld, vault: VaultId, good: &str) -> u64 {
    let families = tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    let mut txn = tw
        .engine_mut()
        .action_transaction(&families.pools)
        .expect("inventory transaction");
    families
        .pool(&mut txn, EndpointRef(vault), &cid(good))
        .expect("the pool reads")
        .map_or(0, |pool| pool.quantity)
}

/// A rig's hopper vault.
fn hopper(tw: &TestWorld, tv: ObjId) -> VaultId {
    tacsim::port::vehicle::hopper_of(tw.world(), tv).expect("a hopper vault")
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

/// **Finish `tv`'s act**: to its due instant (the fire there queues the finish, landed), then one
/// more fire for the tell (its submissions — a recover's queued unload among them — landed); what
/// `who` heard over both.
fn finish(rig: &mut Rig, tv: ObjId, who: ObjId) -> String {
    let due = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .map(|act| act.due_ic)
        .expect("an act in flight");
    let (events, queued) = advance_to(rig, due);
    let mut heard = said(&rig.tw, &events, who);
    assert_eq!(
        finishes(&rig.tw, &queued).len(),
        1,
        "one finish: {queued:?}"
    );
    land_all(rig, &queued);
    let (events, queued) = fire(rig);
    land_all(rig, &queued);
    heard.push('\n');
    heard.push_str(&said(&rig.tw, &events, who));
    heard
}

/// **Land `tv`'s cycle in flight**: to its due, the finish landed, one more fire for the tell.
fn cycle(rig: &mut Rig, tv: ObjId, who: ObjId) -> String {
    let run = run_of(&mut rig.tw, tv).expect("a run in flight");
    let (events, queued) = advance_to(rig, run.due_ic);
    let mut heard = said(&rig.tw, &events, who);
    assert_eq!(
        finishes(&rig.tw, &queued).len(),
        1,
        "one finish at the cycle's due"
    );
    land_all(rig, &queued);
    let (events, queued) = fire(rig);
    land_all(rig, &queued);
    heard.push('\n');
    heard.push_str(&said(&rig.tw, &events, who));
    heard
}

/// How many times `line` appears in `text`.
fn count(text: &str, line: &str) -> usize {
    text.matches(line).count()
}

/// Staff place `tv` `Direct` on Ahuna's domain at site cell `(x, y)`'s centre, facing south — and
/// keep the placement's writes for a reboot (the lead's pre-commit round, item 5, the judge's
/// T15: `support::with_ctx` dropped its outbox, so the placement's recovery-ledger append never
/// reached the store, and P1's and P3's reboots held only because the plan's own drives placed
/// the rig again before each reboot point). `support::place_keeping`, `claims.rs`'s helper.
fn to_cell(rig: &mut Rig, tv: ObjId, cell: (i64, i64)) {
    let domain = mush_sim::DomainId(port_map(&rig.tw, AHUNA).domain.expect("a domain"));
    let staff = support::place_keeping(&mut rig.tw, domain, tv, cell);
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    fold_module_ops_into(&mut rig.store, staff);
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

/// Launch rig `slot` of `pilot` and finish it (it stands at the ramp).
fn launched(rig: &mut Rig, pilot: ObjId, slot: u8, tv: ObjId) {
    let _ = act_go(rig, pilot, &format!("+tv/{slot} launch go"));
    let _ = finish(rig, tv, pilot);
}

/// Put `tv` on `at`'s stand and sound there (its operator reads the pocket).
fn sounded_at(rig: &mut Rig, pilot: ObjId, slot: u8, tv: ObjId, at: &Pick) {
    to_cell(rig, tv, at.stand);
    let _ = act_go(rig, pilot, &format!("+tv/{slot} sound quiet go"));
    let _ = finish(rig, tv, pilot);
}

/// The Dray-P's ore compartments' mass limit, whole tonnes, from content.
fn ore_limit_t(tw: &TestWorld) -> u64 {
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
        .sum::<u64>()
        / 1_000
}

// ---------------------------------------------------------------------------
// Unloading's own helpers
// ---------------------------------------------------------------------------

/// **A real run on `at`**: sounded from its stand, `+tv/<slot> mine go`, every cycle landed until
/// the run ends; the hopper's metal ore after.
fn mined(rig: &mut Rig, pilot: ObjId, slot: u8, tv: ObjId, at: &Pick) -> u64 {
    sounded_at(rig, pilot, slot, tv, at);
    let _ = act_go(rig, pilot, &format!("+tv/{slot} mine go"));
    while run_of(&mut rig.tw, tv).is_some() {
        let _ = cycle(rig, tv, pilot);
    }
    let vault = hopper(&rig.tw, tv);
    pool(&mut rig.tw, vault, "metal_ore")
}

/// The rig's record's ordinal (0 with none).
fn ordinal_of(tw: &mut TestWorld, tv: ObjId) -> u32 {
    record_of(tw, tv).map_or(0, |record| record.ordinal)
}

// ---------------------------------------------------------------------------
// The plans' own helpers
// ---------------------------------------------------------------------------

/// Drop every connection of `who` and end their session (`orders.rs`'s helper).
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

/// The lines queued for `who`'s reconnect.
fn pending(tw: &TestWorld, who: ObjId) -> Vec<String> {
    tacsim::port_tick::pending_notices(tw.world(), who)
}

/// A rig's plan.
fn plan_of(tw: &mut TestWorld, tv: ObjId) -> Option<FillPlan> {
    record_of(tw, tv).and_then(|record| record.plan)
}

/// A key's text.
fn key_text(key: Option<IdempotencyKeyText>) -> String {
    key.map(|key| key.as_str().to_owned())
        .expect("the key forms")
}

/// A start's `act` word.
fn act_word(submit: &AuthoritativeSubmit) -> String {
    submit.parameters["act"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// **File a plan**: `line` with ` go` as `who`, exactly one `plan_file` start queued keyed
/// `…/plan/{no}/file`, landed; the reply.
fn file_plan(rig: &mut Rig, who: ObjId, tv: ObjId, line: &str) -> String {
    let no = record_of(&mut rig.tw, tv).map_or(0, |record| record.plans) + 1;
    let (out, queued) = command(rig, who, &format!("{line} go"));
    assert_eq!(queued.len(), 1, "{line} go: one filing: {out}");
    assert_eq!(act_word(&queued[0]), "plan_file", "{out}");
    assert_eq!(queued[0].actor, who);
    assert_eq!(
        queued[0].key.as_str(),
        key_text(tacsim::surface::act::plan_file_key(tv, no))
    );
    land_all(rig, &queued);
    assert_eq!(
        plan_of(&mut rig.tw, tv).map(|plan| plan.no),
        Some(no),
        "the filing landed: {out}"
    );
    out
}

/// What one step of [`work`] saw land: the rig and the act kind its new ordinal started, with the
/// `by` the start carried.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Landed {
    tv: ObjId,
    kind: ActKind,
    by: Option<ActBy>,
}

/// Whether `tv` still has work: a live plan, an act, a run or a queued unload.
fn busy(tw: &mut TestWorld, tv: ObjId) -> bool {
    record_of(tw, tv).is_some_and(|record| {
        record.act.is_some()
            || record.run.is_some()
            || record.queued.is_some()
            || record.plan.as_ref().is_some_and(FillPlan::live)
    })
}

/// The next instant `tvs` need the clock at: the earliest act or cycle due, a cached plan wait's
/// end, else one IC minute on.
fn next_instant(rig: &mut Rig, tvs: &[ObjId]) -> i64 {
    let now = ic_now(&rig.tw);
    let mut next: Option<i64> = None;
    for tv in tvs {
        let record = record_of(&mut rig.tw, *tv);
        let due =
            record
                .as_ref()
                .and_then(|record| match (record.act.as_ref(), record.run.as_ref()) {
                    (Some(act), Some(run)) => Some(act.due_ic.min(run.due_ic)),
                    (Some(act), None) => Some(act.due_ic),
                    (None, Some(run)) => Some(run.due_ic),
                    (None, None) => None,
                });
        let wait = tacsim::surface::tick::plan_wait_for_tests(rig.tw.world(), *tv)
            .map(|(_, until)| until)
            .filter(|until| *until > now);
        for at in [due, wait].into_iter().flatten() {
            next = Some(next.map_or(at, |n: i64| n.min(at)));
        }
    }
    next.filter(|at| *at > now).unwrap_or(now + 60)
}

/// **Run the clock until `tvs` have no work** (at most `max` steps): each step advances to the next
/// instant a rig needs and lands everything queued; every new ordinal's act kind is recorded, with
/// its `by`, in order. `check` runs after every step.
fn work(rig: &mut Rig, tvs: &[ObjId], max: usize, check: &mut dyn FnMut(&mut Rig)) -> Vec<Landed> {
    work_heard(rig, tvs, max, None, check).0
}

/// [`work`], and what `who` heard over every step (the lead's pre-commit round, item 6: P13's
/// online arm reads the recover's and the unload's lines).
fn work_heard(
    rig: &mut Rig,
    tvs: &[ObjId],
    max: usize,
    who: Option<ObjId>,
    check: &mut dyn FnMut(&mut Rig),
) -> (Vec<Landed>, String) {
    let mut seen: BTreeMap<ObjId, u32> = tvs
        .iter()
        .map(|tv| (*tv, ordinal_of(&mut rig.tw, *tv)))
        .collect();
    let mut landed = Vec::new();
    let mut heard = String::new();
    let hear = |heard: &mut String, rig: &Rig, events: &[(Audience, OutputEvent)]| {
        if let Some(who) = who {
            heard.push_str(&said(&rig.tw, events, who));
            heard.push('\n');
        }
    };
    for _ in 0..max {
        if !tvs.iter().any(|tv| busy(&mut rig.tw, *tv)) {
            // Two more fires: the last outcome's tell and the plan's end are told on the fires
            // after the landing.
            for _ in 0..2 {
                let (events, queued) = fire(rig);
                hear(&mut heard, rig, &events);
                land_all(rig, &queued);
            }
            return (landed, heard);
        }
        let target = next_instant(rig, tvs);
        let (events, queued) = advance_to(rig, target);
        hear(&mut heard, rig, &events);
        land_all(rig, &queued);
        for tv in tvs {
            let Some(record) = record_of(&mut rig.tw, *tv) else {
                continue;
            };
            if seen.get(tv).copied() != Some(record.ordinal) {
                seen.insert(*tv, record.ordinal);
                let (kind, by) = match (&record.act, &record.run, &record.last) {
                    (Some(act), _, _) if act.ordinal == record.ordinal => (act.kind, act.by),
                    (_, Some(run), _) if run.ordinal == record.ordinal => (ActKind::Mine, run.by),
                    (_, _, Some(last)) => (last.kind, last.by),
                    _ => continue,
                };
                landed.push(Landed { tv: *tv, kind, by });
            }
        }
        check(rig);
    }
    panic!("the rigs still had work after {max} steps: {landed:?}");
}

/// The usual set-up: one rig on Ahuna's pad, launched, its pilot's own soundings at two fair
/// single-family metal pockets (`FAIR7` and another); the rig stands on the second's stand.
fn read_two(rig: &mut Rig) -> (ObjId, HullRow, ObjId, Pick, Pick) {
    let (pilot, row, tvs) = on_pad(rig, 1);
    let tv = tvs[0];
    let all = picks(&mut rig.tw);
    let a = fair7(&all);
    let b = fair_other(&all, "FAIR_B", &[&a]);
    launched(rig, pilot, 1, tv);
    sounded_at(rig, pilot, 1, tv, &a);
    sounded_at(rig, pilot, 1, tv, &b);
    (pilot, row, tv, a, b)
}

// ---------------------------------------------------------------------------
// P5 — the preview and the filing
// ---------------------------------------------------------------------------

/// **P5 — the preview commits nothing and `go` files exactly it**: the bare preview names the
/// rig, the site and the ore, one step a line (`mine <addr>`, with its hours) and the tail that
/// names the typed list, and queues and writes nothing; `go` queues ONE `plan_file` start keyed
/// `…/plan/1/file`; landed, the record's steps are the preview's; `plan show` reads them back; a
/// second filing is `PLAN_ALREADY`; a hand-built `plan_file` start after the filing landed is
/// refused "that act was not planned" without a compile, and the next fire's prune drops the order.
#[test]
#[allow(clippy::too_many_lines)]
fn sm11_a_plan_previews_and_go_files_exactly_it() {
    let mut rig = stocked();
    let (pilot, _, tv, a, b) = read_two(&mut rig);
    let list = format!("{},{}", a.addr, b.addr);
    let before = record_of(&mut rig.tw, tv);
    let (out, queued) = command(
        &mut rig,
        pilot,
        &format!("+tv/1 plan {list} family=metal_ore"),
    );
    println!("P5 preview:\n{out}");
    assert!(queued.is_empty(), "a preview queues nothing");
    assert_eq!(
        record_of(&mut rig.tw, tv),
        before,
        "a preview writes nothing"
    );
    // The preview whole, from its constants (the reviewer's T9): the head, each step with its
    // estimate from the tools row, the tail with the hold's room and the lower bound.
    let steps = vec![
        PlanStep::Mine {
            pocket: [a.pocket.0, a.pocket.1],
        },
        PlanStep::Mine {
            pocket: [b.pocket.0, b.pocket.1],
        },
    ];
    assert_preview(&out, &preview_lines(&rig, &steps, &list, 0));
    let compiles = tacsim::surface::act::plan_counts_for_tests(rig.tw.world()).0;
    let filed = file_plan(
        &mut rig,
        pilot,
        tv,
        &format!("+tv/1 plan {list} family=metal_ore"),
    );
    assert!(
        filed.starts_with("Rig 1's plan 1 is filed; it works until "),
        "{filed}"
    );
    let plan = plan_of(&mut rig.tw, tv).expect("the plan");
    assert_eq!(
        plan.steps,
        vec![
            PlanStep::Mine {
                pocket: [a.pocket.0, a.pocket.1]
            },
            PlanStep::Mine {
                pocket: [b.pocket.0, b.pocket.1]
            },
        ]
    );
    assert_eq!(plan.list, vec![a.addr.clone(), b.addr.clone()]);
    assert_eq!(plan.family, OreFamily::Metals);
    assert_eq!(plan.author, pilot);
    assert_eq!(plan.state, FillState::Live);
    assert_eq!(record_of(&mut rig.tw, tv).map(|r| r.plans), Some(1));
    // The read back.
    let shown = run(&mut rig.tw, pilot, "+tv/1 plan show");
    assert!(
        shown.starts_with("Rig 1's plan 1 on Ahuna Flank, mining "),
        "{shown}"
    );
    assert!(
        shown.contains(&format!("  1. mine {}: ", a.addr)),
        "{shown}"
    );
    assert!(
        shown.contains(&format!("  2. mine {}: ", b.addr)),
        "{shown}"
    );
    assert_eq!(
        shown,
        run(&mut rig.tw, pilot, "+tv/1/plan/show"),
        "the switch form"
    );
    // A second filing.
    let (out, queued) = command(&mut rig, pilot, &format!("+tv/1 plan {list} go"));
    assert_eq!(
        out.trim(),
        PLAN_ALREADY.replace("{n}", "1").replace("{no}", "1")
    );
    assert!(queued.is_empty());
    // A hand-built filing once the filing landed: unplanned, and no compile for it.
    let compiled_once = tacsim::surface::act::plan_counts_for_tests(rig.tw.world()).0;
    assert!(compiled_once > compiles, "the filing compiled at its offer");
    let fake = AuthoritativeSubmit {
        actor: pilot,
        action: handles(&rig.tw).surface_act.expect("the start"),
        subject: PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters: json!({
            "tv": tv.0.to_string(),
            "act": "plan_file",
            "ordinal": ordinal_of(&mut rig.tw, tv).max(1),
        }),
        key: tacsim::surface::act::plan_file_key(tv, 2).expect("a key"),
    };
    let refused = land(&mut rig, &fake).expect_err("a stale filing is refused");
    assert!(refused.contains(UNPLANNED), "{refused}");
    assert_eq!(
        tacsim::surface::act::plan_counts_for_tests(rig.tw.world()).0,
        compiled_once,
        "a stale filing compiles nothing"
    );
    let (_, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    assert_eq!(
        tacsim::surface::act::plan_order_for_tests(rig.tw.world(), tv),
        None,
        "the prune drops the landed filing"
    );
    no_refused_writes(&rig.tw);

    // The chart moves between the preview and the landing (the reviewer's T1; D10, "go files
    // exactly the preview or nothing"): the pilot's second rig sounds the pocket the preview read
    // only at R1, so the offer's compile drops the sounding step — the filing refuses
    // `plan_changed` and writes nothing. Rig 1's own orders never move, so it is not `unplanned`.
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 2);
    let all = picks(&mut rig.tw);
    let c = fair7(&all);
    launched(&mut rig, pilot, 1, tvs[0]);
    launched(&mut rig, pilot, 2, tvs[1]);
    to_cell(&mut rig, tvs[0], c.stand);
    scanned_here(&mut rig, pilot, 1, tvs[0]);
    let line = format!("+tv/1 plan {} family=metal_ore", c.addr);
    let (preview, queued) = command(&mut rig, pilot, &line);
    assert!(queued.is_empty());
    let (fx, fy) = tacsim::surface::address::field_of_pocket(c.pocket).expect("a field");
    let field = tacsim::surface::address::field_name(fx, fy).expect("a field name");
    let lines: Vec<&str> = preview.lines().collect();
    assert!(
        lines
            .get(1)
            .is_some_and(|line| line.starts_with(&format!("  1. sound {field}")))
            && lines
                .get(2)
                .is_some_and(|line| line.starts_with(&format!("  2. mine {}", c.addr))),
        "read at R1 only, the pocket is sounded first: {preview}"
    );
    let (out, held) = command(&mut rig, pilot, &format!("{line} go"));
    assert_eq!(held.len(), 1, "{out}");
    assert_eq!(act_word(&held[0]), "plan_file");
    sounded_at(&mut rig, pilot, 2, tvs[1], &c);
    let before = record_of(&mut rig.tw, tvs[0]);
    assert_eq!(before.as_ref().map(|record| record.plans), Some(0));
    let refused = land(&mut rig, &held[0]).expect_err("a filing whose compile moved");
    assert!(refused.contains(PLAN_CHANGED_REFUSAL), "{refused}");
    assert_eq!(record_of(&mut rig.tw, tvs[0]), before, "nothing written");
    no_refused_writes(&rig.tw);
}

/// The start's message for a filing whose compile moved since its preview.
const PLAN_CHANGED_REFUSAL: &str = "that plan changed since its preview; preview it again";

/// The shipped drill and scanner rows.
fn drill_and_scanner() -> (
    tacsim::definitions::tools::DrillDef,
    tacsim::definitions::tools::ScannerDef,
) {
    let table = tools_table().expect("the tools table");
    (
        *table
            .values()
            .find_map(ToolDef::drill)
            .expect("a drill row"),
        *table
            .values()
            .find_map(ToolDef::scanner)
            .expect("a scanner row"),
    )
}

/// **The preview's lines, built from the constants** (the reviewer's T9; the brief's 3.3 step 10
/// and 3.5): the head (`{real}` left for [`assert_preview`]: the real-time helper is the crate's),
/// one `PLAN_PREVIEW_STEP` a step of `steps` — a mine step's estimate a fair pocket's least tonnes
/// over the drill's fair yield, rounded up, times the cycle on regolith (every pocket the tests
/// name is fair, single-family metal and on soft ground); a sounding's the scanner's `sound_s`; a
/// block step's none, and none of its pockets far — and the tail: the hold's room (the Dray-P's ore
/// limit less `seeded`), the lower bound (every step, then one recover and unload a hopper of
/// 12 t of the mine steps' least tonnes), the list as typed with `family=metal_ore`.
fn preview_lines(rig: &Rig, steps: &[PlanStep], list: &str, seeded: u64) -> Vec<String> {
    use tacsim::surface::address::{field_name, field_of_index, pocket_address};
    let (drill, scanner) = drill_and_scanner();
    let fair_t = u64::from(Word::Fair.tonnes_range().0);
    let fair_secs = fair_t.div_ceil(u64::from(tacsim::surface::mine::class_yield(
        &drill,
        Word::Fair,
    ))) * u64::from(
        tacsim::surface::mine::cycle_secs(&drill, Footing::Regolith).expect("a regolith cycle"),
    );
    let mut lines = vec![
        PLAN_PREVIEW_HEAD
            .replace("{n}", "1")
            .replace("{mark}", "OWP-TV-01")
            .replace("{site}", "Ahuna Flank")
            .replace("{ore}", &ore_words("metal_ore")),
    ];
    let (mut est, mut tonnes) = (0_u64, 0_u64);
    for (k, step) in steps.iter().enumerate() {
        let (words, secs) = match step {
            PlanStep::Mine { pocket } => {
                tonnes += fair_t;
                (
                    format!(
                        "mine {}",
                        pocket_address(pocket[0], pocket[1]).expect("an address")
                    ),
                    fair_secs,
                )
            }
            PlanStep::Sound { field } => {
                let (fx, fy) = field_of_index(*field).expect("a field");
                (
                    format!("sound {}", field_name(fx, fy).expect("a field name")),
                    u64::from(scanner.sound_s),
                )
            }
            PlanStep::Block { letter } => (
                format!("mine every pocket of block {letter} that reads fair or better"),
                0,
            ),
        };
        est += secs;
        let hours = if secs > 0 {
            PLAN_STEP_HOURS.replace("{h}", &secs.div_ceil(3_600).to_string())
        } else {
            String::new()
        };
        lines.push(
            PLAN_PREVIEW_STEP
                .replace("{k}", &(k + 1).to_string())
                .replace("{words}", &words)
                .replace("{hours}", &hours)
                .replace("{far}", ""),
        );
    }
    est += tonnes.div_ceil(12)
        * u64::from(tacsim::surface::excursion::RECOVER_S + tacsim::surface::excursion::UNLOAD_S);
    lines.push(
        PLAN_PREVIEW_TAIL
            .replace("{name}", HULL)
            .replace("{room}", &(ore_limit_t(&rig.tw) - seeded).to_string())
            .replace("{shared}", "")
            .replace("{hours}", &est.div_ceil(3_600).to_string())
            .replace("{n}", "1")
            .replace("{list}", &format!("{list} family=metal_ore")),
    );
    lines
}

/// **The preview as `out` says it, checked whole against `want`** ([`preview_lines`]): the head
/// exact but for its `{real}` span (", about …", the crate's real-time helper's words), every other
/// line exact, in order.
fn assert_preview(out: &str, want: &[String]) {
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), want.len(), "{out}");
    let (before, after) = want[0].split_once("{real}").expect("the head's {real}");
    assert!(
        lines[0].starts_with(before) && lines[0].ends_with(after),
        "{out}"
    );
    let real = &lines[0][before.len()..lines[0].len() - after.len()];
    assert!(
        real.starts_with(", about ") && real.len() > ", about ".len(),
        "the head's real-time span: {out}"
    );
    assert_eq!(&lines[1..], &want[1..], "{out}");
}

/// The start's message for a start no plan backs.
const UNPLANNED: &str = "that act was not planned; preview it again";

// ---------------------------------------------------------------------------
// A first run (the smoke the others build on)
// ---------------------------------------------------------------------------

/// **A plan works two read pockets to the end**: filed from the ground, the tick mines the first
/// pocket, drives to the second and mines it, recovers (the queued unload follows) and the plan
/// ends `WorkedOut` from the bay; every start the tick submitted carried `by: plan`.
#[test]
fn sm11_a_plan_works_its_list_to_the_end() {
    let mut rig = stocked();
    let (pilot, row, tv, a, b) = read_two(&mut rig);
    let _ = file_plan(
        &mut rig,
        pilot,
        tv,
        &format!("+tv/1 plan {},{}", a.addr, b.addr),
    );
    let landed = work(&mut rig, &[tv], 400, &mut |_| {});
    println!("landed: {landed:?}");
    let plan = plan_of(&mut rig.tw, tv).expect("the plan");
    println!("plan: {plan:?}");
    assert!(matches!(plan.state, FillState::Ended { .. }), "{plan:?}");
    // Every start the tick submitted carried `by: plan` (the reviewer's T6) — SM.8's queued
    // unload, the recover's second half, carries none — and the outcome kept it.
    assert!(!landed.is_empty());
    for l in &landed {
        if l.kind == ActKind::Unload {
            assert_eq!(l.by, None, "{landed:?}");
        } else {
            assert!(
                matches!(l.by, Some(ActBy::Plan { no: 1, .. })),
                "{landed:?}"
            );
        }
    }
    // The plan's own mined-out pockets are worked, never "passed over" (the builder's decision 1,
    // read off the outcome's `by`).
    assert!(
        plan.notes
            .iter()
            .all(|note| !matches!(note.note, PlanNoteKind::Passed { .. })),
        "{plan:?}"
    );
    let vault = hopper(&rig.tw, tv);
    assert_eq!(
        pool(&mut rig.tw, vault, "metal_ore"),
        0,
        "the hopper is empty"
    );
    let hold = hold_vault(&rig.tw, &row);
    assert_eq!(
        pool(&mut rig.tw, hold, "metal_ore"),
        u64::from(plan.tally.tonnes),
        "every tonne mined is in the hold"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// More helpers: the chart, the projection, the queue, the hold
// ---------------------------------------------------------------------------

/// A principal's chart row for Ahuna.
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
        _ => None,
    }
}

/// The hull's sailing plan.
fn orders(tw: &TestWorld, hull: ObjId) -> Option<Orders> {
    match store(tw).orders_state(hull) {
        OrdersState::Present(orders) => Some(orders.clone()),
        OrdersState::Absent | OrdersState::Unreadable => None,
    }
}

/// `+ship/order <line>` as `who`, filed.
fn order(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let out = run(&mut rig.tw, who, &format!("+ship/order {line}"));
    assert!(out.contains("Order "), "{line}: {out}");
    out
}

/// The block letter of field `index`.
fn block_of_field(index: u8) -> char {
    let (fx, fy) = tacsim::surface::address::field_of_index(index).expect("a field");
    tacsim::surface::address::letter_of_block(fx / 2, fy / 2).expect("a block")
}

/// A scan by hand where the rig stands.
fn scanned_here(rig: &mut Rig, pilot: ObjId, slot: u8, tv: ObjId) {
    let _ = act_go(rig, pilot, &format!("+tv/{slot} scan go"));
    let _ = finish(rig, tv, pilot);
}

/// The start submissions in `queued` for `tv`.
fn starts_for(queued: &[AuthoritativeSubmit], tv: ObjId) -> Vec<AuthoritativeSubmit> {
    queued
        .iter()
        .filter(|submit| submit.action.key().id.as_str() == SURFACE_ACT_ACTION)
        .filter(|submit| submit.parameters["tv"] == json!(tv.0.to_string()))
        .cloned()
        .collect()
}

/// **The hold and the hoppers never pass the hold's limit** (`unloading.rs`'s A3 rule): the hold's
/// metal ore, every hopper's and every run's outstanding cycles.
fn within_limits(rig: &mut Rig, hold: VaultId, tvs: &[ObjId], limit: u64) {
    let mut total = pool(&mut rig.tw, hold, "metal_ore");
    for tv in tvs {
        let vault = hopper(&rig.tw, *tv);
        let held = pool(&mut rig.tw, vault, "metal_ore");
        assert!(held <= 12, "a hopper over its 12 t: {held}");
        total += held + run_of(&mut rig.tw, *tv).map_or(0, |run| run.outstanding_t());
    }
    assert!(total <= limit, "{total} t against the hold's {limit} t");
}

/// [`work`] until `stop` holds after a step (or the rigs have no work).
fn work_until(
    rig: &mut Rig,
    tvs: &[ObjId],
    max: usize,
    stop: &mut dyn FnMut(&mut Rig) -> bool,
) -> Vec<Landed> {
    let mut seen: BTreeMap<ObjId, u32> = tvs
        .iter()
        .map(|tv| (*tv, ordinal_of(&mut rig.tw, *tv)))
        .collect();
    let mut landed = Vec::new();
    for _ in 0..max {
        if !tvs.iter().any(|tv| busy(&mut rig.tw, *tv)) || stop(rig) {
            return landed;
        }
        let target = next_instant(rig, tvs);
        let (_, queued) = advance_to(rig, target);
        land_all(rig, &queued);
        for tv in tvs {
            let Some(record) = record_of(&mut rig.tw, *tv) else {
                continue;
            };
            if seen.get(tv).copied() != Some(record.ordinal) {
                seen.insert(*tv, record.ordinal);
                let (kind, by) = match (&record.act, &record.run, &record.last) {
                    (Some(act), _, _) if act.ordinal == record.ordinal => (act.kind, act.by),
                    (_, Some(run), _) if run.ordinal == record.ordinal => (ActKind::Mine, run.by),
                    (_, _, Some(last)) => (last.kind, last.by),
                    _ => continue,
                };
                landed.push(Landed { tv: *tv, kind, by });
            }
        }
    }
    panic!("the rigs still had work after {max} steps: {landed:?}");
}

// ---------------------------------------------------------------------------
// P1 — the offline story (A1)
// ---------------------------------------------------------------------------

/// What one arm of the offline story ends with (the brief's P1 post-image; the reviewer's F2/T3:
/// the worked blocks' vaults, their cursors, the mined ring and the Trust's and the Exchange's
/// books beside the rig, the hold, the chart, the queue and the notices).
#[derive(Debug, PartialEq)]
struct Story {
    record: Option<RigRecord>,
    projection: Option<TvRec>,
    hold_ore: u64,
    hopper_ore: u64,
    /// The worked blocks' vault pools of metal ore (the two named pockets' blocks and `X`).
    vaults: Vec<u64>,
    /// The worked blocks' metal deposits' pocket cursors.
    cursors: Vec<Option<PocketCursor>>,
    /// Ahuna's `tacsim:mined:` ring.
    mined: Vec<MinedFact>,
    chart: Option<ChartRecord>,
    orders: Option<Orders>,
    pending: Vec<String>,
    balance: i64,
    /// The Co-op Trust's posted credit.
    trust: i64,
    /// The Freeport Exchange's posted credit.
    exchange: i64,
    /// The sale: its quote's unit price and quantity, and its settlement's lines for the pilot,
    /// the Trust and the Exchange.
    sale: Sale,
    ic: i64,
    landed: Vec<Landed>,
}

/// One sale as the books hold it.
#[derive(Debug, PartialEq)]
struct Sale {
    price: i64,
    qty: u64,
    pilot: i64,
    trust: i64,
    exchange: i64,
}

/// The economy's store.
fn economy(tw: &TestWorld) -> &mush_economy::EconomyStore {
    tw.world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy")
}

/// An org of Ceres's map: its credit account.
fn org_account(tw: &TestWorld, org: &str) -> mush_economy::AccountId {
    mush_economy::AccountId(
        port_map(tw, "ceres_port").orgs[&cid(org)]
            .account
            .expect("the org's account"),
    )
}

/// An account's posted credit.
fn posted(tw: &TestWorld, account: mush_economy::AccountId) -> i64 {
    economy(tw)
        .snapshot()
        .account(account)
        .expect("the account")
        .posted_balance
        .minor()
}

/// The pilot's latest sale's quote and its contract, if any.
fn last_sale(tw: &TestWorld, pilot: ObjId) -> Option<(i64, u64, mush_economy::ContractId)> {
    tw.world()
        .module_state::<mush_market::MarketStore>()
        .expect("market")
        .quotes_of_seller(&mush_org::PrincipalId::Object(pilot))
        .last()
        .and_then(|quote| {
            quote
                .contract
                .map(|contract| (quote.price.minor(), quote.quantity, contract))
        })
}

/// A settled contract's lines, by account (`None` while unsettled).
fn settlement(
    tw: &TestWorld,
    contract: mush_economy::ContractId,
) -> Option<BTreeMap<mush_economy::AccountId, i64>> {
    let Some(mush_economy::ContractState::Settled { transaction }) =
        economy(tw).snapshot().contract(contract).map(|c| c.state)
    else {
        return None;
    };
    Some(
        economy(tw)
            .transaction(transaction)
            .expect("the settlement")
            .lines
            .iter()
            .map(|line| (line.account, line.delta.minor()))
            .collect(),
    )
}

/// A deposit's pocket cursor (`mining.rs`'s helper).
fn cursor_of(tw: &mut TestWorld, deposit: OreDepositId) -> Option<PocketCursor> {
    let pockets = handles(tw).families.pockets;
    typed(tw, &pockets, &deposit)
}

/// A block's vault on Ahuna (`mining.rs`'s helper).
fn block_vault(tw: &TestWorld, letter: char) -> VaultId {
    VaultId(
        port_map(tw, AHUNA)
            .site
            .and_then(|row| row.vaults.get(&letter.to_string()).copied())
            .expect("the block vault"),
    )
}

/// Ahuna's mined facts (`mining.rs`'s `ring`).
fn mined_ring(tw: &TestWorld) -> Vec<MinedFact> {
    match store(tw).mined(&cid(AHUNA)) {
        Some(MinedState::Present(ring)) => ring.entries.clone(),
        Some(MinedState::Unreadable) => panic!("the ring is unreadable"),
        None => Vec::new(),
    }
}

/// **Whether `line` is `template` filled** (pure): every literal run between `{placeholders}` is
/// in `line`, in order, the first at its start and the last at its end — a queued line named by
/// its constant whose figures the test does not recompute.
fn fills(line: &str, template: &str) -> bool {
    let mut parts: Vec<&str> = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        parts.push(&rest[..open]);
        let close = rest[open..]
            .find('}')
            .map_or(rest.len(), |at| open + at + 1);
        rest = &rest[close..];
    }
    parts.push(rest);
    let (Some(first), Some(last)) = (parts.first(), parts.last()) else {
        return false;
    };
    if !line.starts_with(first) || !line.ends_with(last) || line.len() < first.len() + last.len() {
        return false;
    }
    let mut at = first.len();
    let end = line.len() - last.len();
    for part in &parts[1..parts.len() - 1] {
        match line[at..end].find(part) {
            Some(found) => at += found + part.len(),
            None => return false,
        }
    }
    true
}

/// A reboot point of the story: no wait the reboot could drop is cached; then (arm B) the world
/// reboots offline and both arms warm Ahuna and stand at the same real instant `now + 120`.
fn reboot_point(rig: Rig, tv: ObjId, rebooting: bool) -> Rig {
    assert_eq!(
        tacsim::surface::tick::plan_wait_for_tests(rig.tw.world(), tv),
        None,
        "no cached wait at a reboot point"
    );
    let at = rig.tw.world().now() + 120;
    let mut rig = if rebooting { reboot(rig, &[]) } else { rig };
    warm(&mut rig);
    assert!(rig.tw.world().now() < at, "the warm ran past the point");
    rig.tw.set_now(at);
    let _ = rig.tw.fire_due(at.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    rig
}

/// **One arm of the offline story** (P1): the pilot reads two pockets by hand and scans the
/// field around the second, files `+tv/1 plan <A>,<B>,<X> family=metal_ore go` (`X` a block the
/// scan read), files `await tv all; depart ceres; sell metal_ore all` and logs off, the hold seeded
/// to two hoppers' room; the clock runs until the sale lands — rebooting (arm B) once the plan's
/// first recover is in flight and again mid-run.
#[allow(clippy::too_many_lines)]
fn story(rebooting: bool) -> (Story, String, i64, i64, u64) {
    let mut rig = stocked();
    let (pilot, row, tv, a, b) = read_two(&mut rig);
    let hull = ObjId(row.hull);
    let all = picks(&mut rig.tw);
    let c = pick(&all, "BLOCK_X", |p| {
        p.footing != Footing::Rock && p.letter() != a.letter() && p.letter() != b.letter()
    });
    to_cell(&mut rig, tv, c.stand);
    scanned_here(&mut rig, pilot, 1, tv);
    let x = c.letter();
    let chart = chart_of(&mut rig.tw, pilot).expect("the pilot's chart");
    assert!(
        chart.fields.keys().any(|index| block_of_field(*index) == x),
        "the scan read a field of block {x}"
    );
    println!("P1 block {x} (pockets {} and {})", a.addr, b.addr);
    let limit = ore_limit_t(&rig.tw);
    let hold = hold_vault(&rig.tw, &row);
    let seeded = limit - 24;
    seed_pool(&mut rig, hold, "metal_ore", seeded, LotId(9_901));
    let list = format!("{},{},{x}", a.addr, b.addr);
    let (preview, queued) = command(
        &mut rig,
        pilot,
        &format!("+tv/1 plan {list} family=metal_ore"),
    );
    println!("P1 preview:\n{preview}");
    assert!(queued.is_empty());
    let _ = file_plan(
        &mut rig,
        pilot,
        tv,
        &format!("+tv/1 plan {list} family=metal_ore"),
    );
    // The preview exactly first (the reviewer's F8/T9): the record's steps are the preview's (P5),
    // so the expected lines are built from them — the two named pockets, the block's soundings
    // nearest first, the block — with the hold's 24 t of room.
    let filed = plan_of(&mut rig.tw, tv).expect("the plan");
    assert_eq!(
        filed.steps.first(),
        Some(&PlanStep::Mine {
            pocket: [a.pocket.0, a.pocket.1]
        })
    );
    assert_eq!(filed.steps.last(), Some(&PlanStep::Block { letter: x }));
    assert_preview(&preview, &preview_lines(&rig, &filed.steps, &list, seeded));
    let filed_ic = filed.filed_ic;
    let _ = order(&mut rig, pilot, "await tv all");
    let _ = order(&mut rig, pilot, "depart ceres");
    let _ = order(&mut rig, pilot, "sell metal_ore all");
    disconnect(&mut rig.tw, pilot);
    // To the first recover in flight.
    let mut landed = work_until(&mut rig, &[tv], 400, &mut |rig| {
        record_of(&mut rig.tw, tv)
            .and_then(|record| record.act)
            .is_some_and(|act| act.kind == ActKind::Recover && act.due_ic > ic_now(&rig.tw) + 150)
    });
    assert!(
        record_of(&mut rig.tw, tv)
            .and_then(|record| record.act)
            .is_some_and(|act| act.kind == ActKind::Recover),
        "the first reboot point: a recover in flight"
    );
    let mut rig = reboot_point(rig, tv, rebooting);
    // To a run with a cycle in flight well ahead.
    landed.extend(work_until(&mut rig, &[tv], 400, &mut |rig| {
        record_of(&mut rig.tw, tv)
            .and_then(|record| record.run)
            .is_some_and(|run| run.k >= 2 && run.due_ic > ic_now(&rig.tw) + 150)
    }));
    assert!(
        record_of(&mut rig.tw, tv)
            .and_then(|record| record.run)
            .is_some_and(|run| run.k >= 2),
        "the second reboot point: mid-run"
    );
    let mut rig = reboot_point(rig, tv, rebooting);
    landed.extend(work(&mut rig, &[tv], 2_000, &mut |_| {}));
    // The queue: await, depart, sell.
    for _ in 0..600 {
        let done = orders(&rig.tw, hull).is_some_and(|doc| {
            doc.orders
                .iter()
                .all(|o| matches!(o.state, OrderState::Done | OrderState::Skipped))
        });
        if done {
            break;
        }
        let at = ic_now(&rig.tw) + 600;
        let (_, queued) = advance_to(&mut rig, at);
        land_all(&mut rig, &queued);
    }
    // The sale settles once the goods are in.
    let (price, qty, contract) = last_sale(&rig.tw, pilot).expect("the queue's sale");
    let mut lines = settlement(&rig.tw, contract);
    for _ in 0..40 {
        if lines.is_some() {
            break;
        }
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
        lines = settlement(&rig.tw, contract);
    }
    let lines = lines.expect("the sale settled");
    let me = tacsim::port::credit::own_account(rig.tw.world(), pilot)
        .map(|account| account.id)
        .expect("the pilot's account");
    let (trust, exchange) = (
        org_account(&rig.tw, "coop_trust"),
        org_account(&rig.tw, "freeport_exchange"),
    );
    let sale = Sale {
        price,
        qty,
        pilot: lines.get(&me).copied().unwrap_or(0),
        trust: lines.get(&trust).copied().unwrap_or(0),
        exchange: lines.get(&exchange).copied().unwrap_or(0),
    };
    let letters = [a.letter(), b.letter(), x];
    let story = Story {
        record: record_of(&mut rig.tw, tv),
        projection: projection(&rig.tw, tv),
        hold_ore: pool(&mut rig.tw, hold, "metal_ore"),
        hopper_ore: {
            let vault = hopper(&rig.tw, tv);
            pool(&mut rig.tw, vault, "metal_ore")
        },
        vaults: letters
            .iter()
            .map(|letter| {
                let vault = block_vault(&rig.tw, *letter);
                pool(&mut rig.tw, vault, "metal_ore")
            })
            .collect(),
        cursors: [&a, &b, &c]
            .iter()
            .map(|at| cursor_of(&mut rig.tw, at.deposit(OreFamily::Metals)))
            .collect(),
        mined: mined_ring(&rig.tw),
        chart: chart_of(&mut rig.tw, pilot),
        orders: orders(&rig.tw, hull),
        pending: pending(&rig.tw, pilot),
        balance: tacsim::port::credit::own_account(rig.tw.world(), pilot)
            .and_then(mush_economy::Account::available)
            .map_or(0, mush_economy::Amount::minor),
        trust: posted(&rig.tw, trust),
        exchange: posted(&rig.tw, exchange),
        sale,
        ic: ic_now(&rig.tw),
        landed,
    };
    // Reconnect: the heading, then the queued lines in queue order.
    let events = rig.tw.connect(pilot);
    let heard = said(&rig.tw, &events, pilot);
    no_refused_writes(&rig.tw);
    (
        story,
        heard,
        filed_ic,
        i64::try_from(seeded).unwrap_or(0),
        limit,
    )
}

/// **P1 — the offline story fills the hold, parks, and the queue sells** (A1; lead ruling 9):
/// two arms on the same key and script, arm B rebooted twice mid-plan; in both the plan ended with
/// the hopper empty, at least two trips, `tally.trips` the number of unloads, no launch after the
/// last unload, every reading the plan made `via: Plan`, the await done and the queue's depart and
/// sale run; on reconnect the surface's queued lines are one `UNLOADED` a trip and one
/// `PLAN_ENDED`, nothing else; and the two arms' post-images are equal.
#[test]
#[allow(clippy::too_many_lines)]
fn sm11_the_offline_story_fills_the_hold_parks_and_the_queue_sells() {
    let started = std::time::Instant::now();
    let (straight, heard_a, filed_ic, seeded, limit) = story(false);
    let (rebooted, heard_b, _, _, _) = story(true);
    println!("P1 landed: {:?}", straight.landed);
    println!("P1 pending: {:#?}", straight.pending);
    let record = straight.record.clone().expect("the record");
    let plan = record.plan.clone().expect("the plan");
    println!("P1 plan: {plan:?}");
    println!(
        "P1 IC span {} s ({} IC h), {} trips, {} t, ended {:?}, wall {:?}",
        straight.ic - filed_ic,
        (straight.ic - filed_ic) / 3_600,
        plan.tally.trips,
        plan.tally.tonnes,
        plan.state,
        started.elapsed()
    );
    // The story's IC span, pinned (the lead's pre-commit round, item 7; the judge's last
    // residual). The builder's run read 47,485 IC s and the fixer round's 46,297 with the plan's
    // record byte-identical (its steps, its 13 cycles over 3 trips, 24 t, its `HoldFull` end at IC
    // 46,343): the move was never the per-measure hold gate. It is this harness's own — `work`
    // gained its two trailing 6 s fires (the last outcome's tell and the plan's end told before it
    // returns), which shifts where the queue's tail below starts its 600 IC s steps; the await
    // and the departure then land two steps sooner (1,188 = 2 × 600 − 12), the arrival at 14:32
    // UTC instead of 14:52. Re-running with those two fires removed gives 47,485 again. A
    // change here is a change to the plan's steps or to the queue's timing: find which.
    assert_eq!(straight.ic - filed_ic, 46_297, "the story's IC span");
    assert!(
        matches!(
            plan.state,
            FillState::Ended {
                why: PlanEnd::HoldFull | PlanEnd::WorkedOut,
                ..
            }
        ),
        "{plan:?}"
    );
    assert_eq!(straight.hopper_ore, 0, "the hopper is empty");
    assert!(plan.tally.trips >= 2, "at least two trips: {plan:?}");
    let unloads = straight
        .landed
        .iter()
        .filter(|l| l.kind == ActKind::Unload)
        .count();
    assert_eq!(
        usize::from(plan.tally.trips),
        unloads,
        "every trip unloaded"
    );
    let last_unload = straight
        .landed
        .iter()
        .rposition(|l| l.kind == ActKind::Unload)
        .expect("an unload");
    assert!(
        straight.landed[last_unload..].iter().all(|l| !matches!(
            l.kind,
            ActKind::Launch | ActKind::Drive | ActKind::Mine | ActKind::Sound
        )),
        "no empty excursion after the last unload: {:?}",
        straight.landed
    );
    // Every reading made after the filing is the plan's.
    let chart = straight.chart.clone().expect("the chart");
    let stamps = chart
        .fields
        .values()
        .flat_map(|row| row.provenance.iter())
        .chain(chart.blocks.values().flat_map(|row| row.provenance.iter()));
    for stamp in stamps {
        let ChartProvenance::Local { at, via, .. } = stamp;
        if *at > filed_ic {
            assert_eq!(*via, Via::Plan, "{stamp:?}");
        }
    }
    // The queue ran: every order done, the await's done line logged, the hull docked at Ceres.
    let doc = straight.orders.clone().expect("the sailing plan");
    assert!(
        doc.orders.iter().all(|o| o.state == OrderState::Done),
        "{doc:?}"
    );
    assert!(
        doc.log.iter().any(|line| line
            .text
            .ends_with("Every rig parked; the sailing plan runs on.")),
        "{:?}",
        doc.log
    );
    // The sale: the whole hold (the seeded load and every tonne mined) at the Exchange's bid, the
    // Trust's share of the margin over the lane's reference (SM.5's arithmetic), the Exchange
    // paying the gross.
    let held = u64::try_from(seeded).unwrap_or(0) + u64::from(plan.tally.tonnes);
    let sale = &straight.sale;
    assert_eq!(sale.qty, held, "the queue sold the whole hold: {sale:?}");
    assert_eq!(straight.hold_ore, 0, "the hold sold");
    let programme = match lane().entry(&cid("ceres_port")) {
        Some(PackEntry::Port(def)) => def.programme.clone().expect("the programme"),
        other => panic!("ceres_port is a port entry: {other:?}"),
    };
    let reference =
        i64::try_from(programme.reference_minor[&cid("metal_ore")]).expect("the reference");
    let gross = sale.price * i64::try_from(sale.qty).expect("a quantity");
    let share = (sale.price - reference).max(0)
        * i64::try_from(sale.qty).expect("a quantity")
        * i64::from(programme.share_permille)
        / 1_000;
    assert_eq!(
        (sale.pilot, sale.trust, sale.exchange),
        (gross - share, share, -gross),
        "the settlement: {sale:?}"
    );
    // The queued lines, exactly and in the log's order (the reviewer's F12/T2): the surface's —
    // one `UNLOADED` a trip, then `PLAN_ENDED` with the tally and the hold at the end — and the
    // executor's told lines, each by its constant: the await's wait (once), its done line, the
    // depart's clearance, the arrival and the sale. Nothing else: no cycle, reading, drive,
    // launch, recover or pass-over line.
    let trips = usize::from(plan.tally.trips);
    let why = match plan.state {
        FillState::Ended {
            why: PlanEnd::HoldFull,
            ..
        } => tacsim::port::tv::END_HOLD_FULL.replace("{name}", HULL),
        _ => tacsim::port::tv::END_WORKED_OUT.to_owned(),
    };
    let ended = PLAN_ENDED
        .replace("{n}", "1")
        .replace("{site}", "Ahuna Flank")
        .replace("{why}", &why)
        .replace("{t}", &plan.tally.tonnes.to_string())
        .replace("{ore}", &ore_words("metal_ore"))
        .replace(
            "{c}",
            &tacsim::port::tv::count_words(u64::from(plan.tally.cycles), "cycle", "cycles"),
        )
        .replace(
            "{trips}",
            &tacsim::port::tv::count_words(u64::from(plan.tally.trips), "trip", "trips"),
        )
        .replace("{name}", HULL)
        .replace("{held}", &held.to_string())
        .replace("{room}", &(limit - held).to_string());
    let waits = tacsim::port::executor::WAITING_LINE
        .replace("{n}", "1")
        .replace(
            "{reason}",
            &tacsim::port::executor::AWAIT_WAITING.replace("{which}", "every rig"),
        );
    let done = tacsim::port::executor::AWAIT_DONE.replace("{which}", "Every rig");
    let pending = &straight.pending;
    assert_eq!(pending.len(), trips + 6, "{pending:#?}");
    assert_eq!(pending[0], waits, "{pending:#?}");
    let mut unloaded_t = 0_u64;
    for line in &pending[1..=trips] {
        assert!(
            fills(line, UNLOADED) && line.starts_with("Rig 1 unloads "),
            "{pending:#?}"
        );
        assert!(line.ends_with(" hold; hopper empty."), "{line}");
        unloaded_t += line["Rig 1 unloads ".len()..]
            .split(' ')
            .next()
            .and_then(|t| t.parse::<u64>().ok())
            .expect("the unload's tonnes");
    }
    assert_eq!(
        unloaded_t,
        u64::from(plan.tally.tonnes),
        "every tonne mined unloaded"
    );
    assert_eq!(pending[trips + 1], ended, "{pending:#?}");
    assert_eq!(pending[trips + 2], done, "{pending:#?}");
    assert!(
        fills(&pending[trips + 3], tacsim::port::voyage::CLEARANCE_GRANTED),
        "{pending:#?}"
    );
    assert!(
        fills(&pending[trips + 4], tacsim::port::voyage::ARRIVAL_NOTICE),
        "{pending:#?}"
    );
    assert!(
        fills(&pending[trips + 5], tacsim::port::proceeds::SALE_LINE),
        "{pending:#?}"
    );
    // The reconnect renders the heading, then exactly the queued lines in their order.
    let heard: Vec<&str> = heard_a.lines().collect();
    let at = heard
        .iter()
        .position(|line| *line == tacsim::port_tick::NOTICE_HEADING)
        .expect("the heading");
    assert_eq!(
        heard[at + 1..at + 1 + pending.len()]
            .iter()
            .map(|line| (*line).to_owned())
            .collect::<Vec<String>>(),
        *pending,
        "{heard_a}"
    );
    assert!(
        heard[at + 1 + pending.len()..]
            .iter()
            .all(|line| !pending.iter().any(|queued| queued == line)),
        "each queued line rendered once: {heard_a}"
    );
    // Every start the plan's tick submitted carried `by: plan` (the reviewer's T6); the queued
    // unload, the recover's second half, none.
    for l in &straight.landed {
        if l.kind == ActKind::Unload {
            assert_eq!(l.by, None, "{:?}", straight.landed);
        } else {
            assert!(
                matches!(l.by, Some(ActBy::Plan { no: 1, .. })),
                "{:?}",
                straight.landed
            );
        }
    }
    assert!(
        plan.notes
            .iter()
            .all(|note| !matches!(note.note, PlanNoteKind::Passed { .. })),
        "{plan:?}"
    );
    // The two arms.
    assert_eq!(heard_a, heard_b, "the same reconnect");
    assert_eq!(
        straight, rebooted,
        "a restart changes nothing in the post-image"
    );
}

// ---------------------------------------------------------------------------
// P2 — two rigs' plans share one hold (A2)
// ---------------------------------------------------------------------------

/// **P2 — two rigs' plans fill one hold and neither strands ore** (A2): rig 1's plan names `FAIR7`,
/// rig 2's another fair pocket; the second preview names the first plan (`PLAN_SHARED`); the hold
/// is seeded so its unreserved room is less than both pockets' ore; after every step the hold, both
/// hoppers and every outstanding cycle stay inside the hold's limit; at the end both plans have
/// ended (at least one `HoldFull`), both hoppers are empty, the hold holds the seeded load plus
/// every tonne mined, the `HoldFull` plan made no launch after its last unload, and `+ship/plan
/// ceres` names no loaded hopper and no rig working a plan.
#[test]
#[allow(clippy::too_many_lines)]
fn sm11_two_rigs_plans_fill_one_hold_and_neither_strands_ore() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 2);
    let all = picks(&mut rig.tw);
    let a = fair7(&all);
    let b = fair_other(&all, "FAIR (rig 2)", &[&a]);
    launched(&mut rig, pilot, 1, tvs[0]);
    launched(&mut rig, pilot, 2, tvs[1]);
    sounded_at(&mut rig, pilot, 1, tvs[0], &a);
    sounded_at(&mut rig, pilot, 2, tvs[1], &b);
    let limit = ore_limit_t(&rig.tw);
    let hold = hold_vault(&rig.tw, &row);
    let both = u64::from(a.tonnes[0]) + u64::from(b.tonnes[0]);
    let seeded = limit - both + 3;
    seed_pool(&mut rig, hold, "metal_ore", seeded, LotId(9_921));
    let _ = file_plan(&mut rig, pilot, tvs[0], &format!("+tv/1 plan {}", a.addr));
    let (preview, queued) = command(&mut rig, pilot, &format!("+tv/2 plan {}", b.addr));
    assert!(queued.is_empty());
    assert!(
        preview.contains(&PLAN_SHARED.replace("{m}", "1")),
        "the second preview names the first plan: {preview}"
    );
    let _ = file_plan(&mut rig, pilot, tvs[1], &format!("+tv/2 plan {}", b.addr));
    let landed = work(&mut rig, &tvs, 2_000, &mut |rig| {
        within_limits(rig, hold, &tvs, limit);
    });
    println!("P2 landed: {landed:?}");
    let plans: Vec<FillPlan> = tvs
        .iter()
        .map(|tv| plan_of(&mut rig.tw, *tv).expect("a plan"))
        .collect();
    println!("P2 plans: {plans:#?}");
    assert!(
        plans
            .iter()
            .all(|plan| matches!(plan.state, FillState::Ended { .. }))
    );
    assert!(
        plans.iter().any(|plan| matches!(
            plan.state,
            FillState::Ended {
                why: PlanEnd::HoldFull,
                ..
            }
        )),
        "{plans:?}"
    );
    for tv in &tvs {
        let vault = hopper(&rig.tw, *tv);
        assert_eq!(pool(&mut rig.tw, vault, "metal_ore"), 0, "hopper empty");
        let mine: Vec<&Landed> = landed.iter().filter(|l| l.tv == *tv).collect();
        if let Some(last) = mine.iter().rposition(|l| l.kind == ActKind::Unload) {
            assert!(
                mine[last..]
                    .iter()
                    .all(|l| !matches!(l.kind, ActKind::Launch | ActKind::Drive | ActKind::Mine)),
                "no empty excursion after the last unload: {mine:?}"
            );
        }
    }
    let mined: u64 = plans.iter().map(|plan| u64::from(plan.tally.tonnes)).sum();
    assert_eq!(pool(&mut rig.tw, hold, "metal_ore"), seeded + mined);
    // The hull can lift (the reviewer's T7): the passage plans with no rig holding her, and
    // `+ship/lift ceres` files the clearance (its burn left unlanded).
    let ship = run(&mut rig.tw, pilot, "+ship/plan ceres");
    assert_eq!(
        ship.lines().next(),
        Some("Plan for Ceres Freeport:"),
        "{ship}"
    );
    for gate in [
        "hopper holds",
        "working its plan",
        "is out on",
        "lifts once it is done",
    ] {
        assert!(!ship.contains(gate), "{gate}: {ship}");
    }
    let (lift, burn) = command(&mut rig, pilot, "+ship/lift ceres");
    assert!(lift.contains("Clearance filed for "), "she lifts: {lift}");
    assert_eq!(burn.len(), 1, "one burn: {lift}");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// P3 — a lapsed licence parks the plan (A3)
// ---------------------------------------------------------------------------

/// A pilot's gateway account.
fn account_of(rig: &Rig, who: ObjId) -> AccountId {
    rig.oracle
        .0
        .lock()
        .ok()
        .and_then(|map| map.get(&who).copied())
        .expect("an account")
}

/// The Registry's principal (`unloading.rs`'s).
fn registry(tw: &TestWorld) -> mush_org::PrincipalId {
    mush_org::PrincipalId::Organization(mush_org::OrgId(
        port_map(tw, "ceres_port").orgs[&cid("freeport_registry")].org,
    ))
}

/// **Staff file a claim** over every opened deposit of `at`'s block, held by `holder` under the
/// Registry, `term` IC seconds (`unloading.rs`'s helper: industry's own writer, `@claim/file`).
fn claim_block(rig: &mut Rig, at: &Pick, holder: ObjId, term: i64) {
    let issuer = mush_economy::principal_token(&registry(&rig.tw));
    let families = rig
        .tw
        .world()
        .module_state::<mush_industry::IndustryHandles>()
        .expect("industry")
        .0
        .clone();
    let mut filed = 0;
    for family in OreFamily::ALL {
        let deposit = at.deposit(family);
        if typed(&mut rig.tw, &families.deposits, &deposit).is_none() {
            continue;
        }
        let _ = run(
            &mut rig.tw,
            GOD,
            &format!(
                "@claim/file deposit:{deposit}=org:{}/obj:{}/exclusive/1/{term}/transferable",
                issuer.trim_start_matches("org:"),
                holder.0
            ),
        );
        let queued = drain(&mut rig.tw);
        land_all(rig, &queued);
        filed += 1;
    }
    assert!(filed > 0, "the block has an opened deposit");
}

/// **P3 — a plan whose licence lapses recovers, unloads and parks with its reason** (A3; lead
/// ruling 3): the plan mines its first pocket; with ore in the hopper and the pilot offline the
/// licence lapses (`reboot_editing` of `tacsim:licence:<account>`); at the next step's start the
/// plan parks `Licence`, the rig recovers, SM.8's queued unload empties the hopper and the plan
/// ends with the licence's reason; the queued notices hold exactly one `PLAN_PARKING` and one
/// `PLAN_ENDED`. In a second world a claim filed on the plan's next block parks it `Claimed`.
#[test]
#[allow(clippy::too_many_lines)]
fn sm11_a_lapsed_licence_parks_the_plan_with_its_reason() {
    let mut rig = stocked();
    let (pilot, _, tv, a, b) = read_two(&mut rig);
    let _ = file_plan(
        &mut rig,
        pilot,
        tv,
        &format!("+tv/1 plan {},{}", a.addr, b.addr),
    );
    disconnect(&mut rig.tw, pilot);
    // The first pocket mined out, ore in the hopper, the rig waiting for its next step.
    let _ = work_until(&mut rig, &[tv], 400, &mut |rig| {
        plan_of(&mut rig.tw, tv).is_some_and(|plan| plan.cursor >= 1)
            && record_of(&mut rig.tw, tv)
                .is_some_and(|record| record.act.is_none() && record.run.is_none())
    });
    let vault = hopper(&rig.tw, tv);
    assert!(
        pool(&mut rig.tw, vault, "metal_ore") > 0,
        "ore in the hopper"
    );
    let lapse = ic_now(&rig.tw) - 1;
    let key = licence_key(account_of(&rig, pilot));
    let mut rig = reboot_editing(rig, key.as_str(), &[], |doc| {
        doc["expires_ic"] = json!(lapse);
        doc["grace_secs"] = json!(0);
    });
    warm(&mut rig);
    let landed = work(&mut rig, &[tv], 400, &mut |_| {});
    println!("P3 landed: {landed:?}");
    let plan = plan_of(&mut rig.tw, tv).expect("the plan");
    assert!(
        matches!(
            plan.state,
            FillState::Ended {
                why: PlanEnd::Licence { .. },
                ..
            }
        ),
        "{plan:?}"
    );
    assert!(
        landed.iter().any(|l| l.kind == ActKind::Recover)
            && landed.iter().any(|l| l.kind == ActKind::Unload),
        "{landed:?}"
    );
    assert!(
        landed.iter().all(|l| l.kind != ActKind::Mine),
        "no mining after the lapse: {landed:?}"
    );
    let vault = hopper(&rig.tw, tv);
    assert_eq!(
        pool(&mut rig.tw, vault, "metal_ore"),
        0,
        "the hopper emptied"
    );
    let queued = pending(&rig.tw, pilot);
    println!("P3 pending: {queued:#?}");
    // Exactly one parking line, with `+licence`'s own lapsed line (the reviewer's T10), and one
    // end naming the same reason; the recover names no right (a recover line is never queued).
    let lapsed = LICENCE_LAPSED.replace("{lapses}", &ic_stamp(lapse));
    let parking = PLAN_PARKING.replace("{n}", "1").replace("{why}", &lapsed);
    assert_eq!(
        queued.iter().filter(|line| **line == parking).count(),
        1,
        "{parking}\n{queued:#?}"
    );
    assert_eq!(
        queued
            .iter()
            .filter(|line| line.starts_with("Rig 1's plan stops mining: "))
            .count(),
        1,
        "{queued:?}"
    );
    let ended_head = format!("Rig 1's plan at Ahuna Flank ended: {lapsed} It mined ");
    assert_eq!(
        queued
            .iter()
            .filter(|line| line.starts_with("Rig 1's plan at Ahuna Flank ended: "))
            .count(),
        1,
        "{queued:?}"
    );
    assert!(
        queued.iter().any(|line| line.starts_with(&ended_head)),
        "{ended_head}\n{queued:#?}"
    );
    let recovered = queued.iter().filter(|l| l.contains("is back in")).count();
    assert_eq!(recovered, 0, "a recover is never queued offline");
    no_refused_writes(&rig.tw);

    // SM.7's R9 (the reviewer's T10): the licence lapses with a run in flight; the run is ended by
    // its own rule at its next cycle, and once it has ended nothing more is drawn — the pocket's
    // cursor and the block's vault stay as the run left them while the plan parks.
    let mut rig = stocked();
    let (pilot, _, tv, a, b) = read_two(&mut rig);
    let _ = file_plan(
        &mut rig,
        pilot,
        tv,
        &format!("+tv/1 plan {},{}", a.addr, b.addr),
    );
    disconnect(&mut rig.tw, pilot);
    let _ = work_until(&mut rig, &[tv], 400, &mut |rig| {
        run_of(&mut rig.tw, tv).is_some_and(|run| run.due_ic > ic_now(&rig.tw) + 60)
    });
    assert!(run_of(&mut rig.tw, tv).is_some(), "a run in flight");
    let lapse = ic_now(&rig.tw) - 1;
    let key = licence_key(account_of(&rig, pilot));
    let mut rig = reboot_editing(rig, key.as_str(), &[], |doc| {
        doc["expires_ic"] = json!(lapse);
        doc["grace_secs"] = json!(0);
    });
    warm(&mut rig);
    let _ = work_until(&mut rig, &[tv], 400, &mut |rig| {
        run_of(&mut rig.tw, tv).is_none()
    });
    assert!(run_of(&mut rig.tw, tv).is_none(), "the run ended");
    let vault = block_vault(&rig.tw, a.letter());
    let deposit = a.deposit(OreFamily::Metals);
    let at_end = (
        cursor_of(&mut rig.tw, deposit),
        pool(&mut rig.tw, vault, "metal_ore"),
    );
    let landed = work(&mut rig, &[tv], 400, &mut |_| {});
    println!("P3 (R9) landed: {landed:?}");
    assert!(
        landed.iter().all(|l| l.kind != ActKind::Mine),
        "no run after the lapse: {landed:?}"
    );
    assert!(
        matches!(
            plan_of(&mut rig.tw, tv).map(|plan| plan.state),
            Some(FillState::Ended {
                why: PlanEnd::Licence { .. },
                ..
            })
        ),
        "the plan parked for its licence"
    );
    assert_eq!(
        (
            cursor_of(&mut rig.tw, deposit),
            pool(&mut rig.tw, vault, "metal_ore")
        ),
        at_end,
        "nothing drawn after the run in flight ended"
    );
    no_refused_writes(&rig.tw);

    // A claim on the plan's next pocket's block parks it `Claimed`.
    let mut rig = stocked();
    let (pilot, _, tv, a, b) = read_two(&mut rig);
    let _ = file_plan(
        &mut rig,
        pilot,
        tv,
        &format!("+tv/1 plan {},{}", a.addr, b.addr),
    );
    let rival = pilot_on(&mut rig, "Rival", 7);
    let _ = work_until(&mut rig, &[tv], 400, &mut |rig| {
        plan_of(&mut rig.tw, tv).is_some_and(|plan| plan.cursor >= 1)
            && record_of(&mut rig.tw, tv)
                .is_some_and(|record| record.act.is_none() && record.run.is_none())
    });
    claim_block(&mut rig, &b, rival, 100_000_000);
    // While it parks, the status and `plan show` name it coming home, never the step it skips
    // (the fixer round's correctness/F4).
    let _ = work_until(&mut rig, &[tv], 400, &mut |rig| {
        plan_of(&mut rig.tw, tv).is_some_and(|plan| matches!(plan.state, FillState::Parking { .. }))
    });
    let status = run(&mut rig.tw, pilot, "+tv");
    assert!(
        status.contains("  Plan 1: step 2 of 2, coming home to park; until "),
        "{status}"
    );
    let shown = run(&mut rig.tw, pilot, "+tv/1 plan show");
    assert!(
        shown.contains(&format!("  2. mine {}: pending", b.addr)) && !shown.contains(": now"),
        "{shown}"
    );
    let _ = work(&mut rig, &[tv], 400, &mut |_| {});
    let plan = plan_of(&mut rig.tw, tv).expect("the plan");
    assert!(
        matches!(
            plan.state,
            FillState::Ended {
                why: PlanEnd::Claimed,
                ..
            }
        ),
        "{plan:?}"
    );
    let shown = run(&mut rig.tw, pilot, "+tv/1 plan show");
    assert!(
        shown.contains(&format!("{}.", MINE_CLAIMED.trim_end_matches('.'))),
        "{shown}"
    );
    no_refused_writes(&rig.tw);
}

/// A second character on a gateway account of their own (no charter).
fn pilot_on(rig: &mut Rig, who: &str, seed: i64) -> ObjId {
    pilot(rig, who, seed)
}

// ---------------------------------------------------------------------------
// P4 — never re-armed (A4)
// ---------------------------------------------------------------------------

/// **P4 — a plan never re-arms** (A4; Q-20-3): after a one-pocket plan ends, 48 IC hours of fires
/// submit nothing for the rig and leave the plan as it ended; `plan show` reads its end;
/// `plan clear` strikes it (`PLAN_DROPPED`, one `plan_strike` keyed `…/strike/<ordinal>`); a second
/// clear is `PLAN_NONE` with nothing submitted; a new filing is plan 2.
#[test]
#[allow(clippy::too_many_lines)]
fn sm11_a_plan_never_rearms() {
    let mut rig = stocked();
    let (pilot, _, tv, a, b) = read_two(&mut rig);
    let _ = file_plan(&mut rig, pilot, tv, &format!("+tv/1 plan {}", a.addr));
    let _ = work(&mut rig, &[tv], 400, &mut |_| {});
    let ended = plan_of(&mut rig.tw, tv).expect("the plan");
    assert!(matches!(ended.state, FillState::Ended { .. }), "{ended:?}");
    let until = ic_now(&rig.tw) + 48 * 3_600;
    while ic_now(&rig.tw) < until {
        let at = ic_now(&rig.tw) + 3_600;
        let (_, queued) = advance_to(&mut rig, at);
        assert!(
            starts_for(&queued, tv).is_empty(),
            "nothing starts: {queued:?}"
        );
        land_all(&mut rig, &queued);
    }
    assert_eq!(plan_of(&mut rig.tw, tv), Some(ended.clone()), "unchanged");
    let shown = run(&mut rig.tw, pilot, "+tv/1 plan show");
    assert!(shown.contains("\nIt ended IC "), "{shown}");
    let ordinal = ordinal_of(&mut rig.tw, tv);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 plan clear");
    assert_eq!(
        out.lines().next(),
        Some(
            PLAN_DROPPED
                .replace("{n}", "1")
                .replace("{no}", "1")
                .as_str()
        )
    );
    assert_eq!(queued.len(), 1);
    assert_eq!(act_word(&queued[0]), "plan_strike");
    assert_eq!(
        queued[0].key.as_str(),
        key_text(tacsim::surface::act::plan_strike_key(tv, 1, ordinal))
    );
    land_all(&mut rig, &queued);
    assert_eq!(plan_of(&mut rig.tw, tv), None, "struck");
    let (out, queued) = command(&mut rig, pilot, "+tv/1 plan clear");
    assert_eq!(out.trim(), PLAN_NONE.replace("{n}", "1"));
    assert!(queued.is_empty());
    let _ = file_plan(&mut rig, pilot, tv, &format!("+tv/1 plan {}", b.addr));
    assert_eq!(record_of(&mut rig.tw, tv).map(|r| r.plans), Some(2));
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// P6 — the compiler's gates through the verb
// ---------------------------------------------------------------------------

/// **SM.11r-2 — a plan is compiled on the hold's room in every measure** (doc 20 cut SM.11r; doc
/// 20a §10 "Defects at HEAD", the owner's external review of 2026-10-23, item 2): the Dray-P's
/// cargo compartment takes 86,000 kg and 103,200 L; 44 t of industrial parts (2,300 L a tonne)
/// leave 42,000 kg but only 2,000 L — room for 4 t of metal ore (500 L a tonne) and 2 t of ice
/// ore (1,000 L), where the mass alone reads 42 t of either. So the metals plan on A, its family
/// inferred from the pilot's reading, previews "(4 t unreserved"; an ice plan on an ice-bearing
/// pocket the same soundings read is refused `PLAN_HOLD_FULL` (2 t is under a blind core's 3 t),
/// so the compile asks the plan's own family; and 3 t of metals more (400 L a tonne) leave 800 L,
/// 1 t of metal ore, under a fair cycle's 2 t as well, and the metals plan is refused too — each
/// refusal at the preview and at `go`, writing nothing, where the mass alone (42 t, then 39 t)
/// filed every one. The ice pocket stands in for doc 20a's carbon-ore case (review of SM.11r).
#[test]
fn sm11r_a_plan_reads_the_holds_room_in_every_measure() {
    let mut rig = stocked();
    let (pilot, row, tv, a, _) = read_two(&mut rig);
    let chart = chart_of(&mut rig.tw, pilot).expect("the chart");
    let all = picks(&mut rig.tw);
    let ice = pick(&all, "ICE_READ", |p| {
        p.classes[1] != Word::Barren
            && !tacsim::surface::chart::sound_adds(Some(&chart), &[p.pocket])
    });
    let hold = hold_vault(&rig.tw, &row);
    let before = record_of(&mut rig.tw, tv);
    let refuses = |rig: &mut Rig, line: &str| {
        for plan_line in [line.to_owned(), format!("{line} go")] {
            let (out, queued) = command(rig, pilot, &plan_line);
            assert_eq!(
                out.trim(),
                PLAN_HOLD_FULL.replace("{name}", HULL),
                "{plan_line}"
            );
            assert!(queued.is_empty(), "{plan_line}");
        }
    };
    seed_pool(&mut rig, hold, "industrial_parts", 44, LotId(9_981));
    let metals = format!("+tv/1 plan {}", a.addr);
    let (out, queued) = command(&mut rig, pilot, &metals);
    assert!(queued.is_empty(), "a preview submits nothing");
    assert!(out.contains("(4 t unreserved"), "{out}");
    refuses(&mut rig, &format!("+tv/1 plan {} family=ice_ore", ice.addr));
    seed_pool(&mut rig, hold, "metals", 3, LotId(9_982));
    refuses(&mut rig, &metals);
    assert_eq!(record_of(&mut rig.tw, tv), before, "nothing written");
    no_refused_writes(&rig.tw);
}

/// **SM.11r-2 — a plan's end names the hold's room in every measure** (doc 20 cut SM.11r; review
/// of SM.11r, the tests lens's S4 and the correctness lens's S2): with 44 t of industrial parts
/// aboard (room for 4 t of metal ore by volume, 42 t by mass), a metals plan on A mines two fair
/// cycles, 4 t, recovers, unloads and ends `HoldFull`; its end line reads "holds 48 t, 0 t of it
/// unreserved." — the tonnes aboard by mass, the room in the plan's own ore in every measure —
/// where the mass alone read "38 t of it unreserved".
#[test]
fn sm11r_a_plan_ends_naming_the_holds_room_in_every_measure() {
    let mut rig = stocked();
    let (pilot, row, tv, a, _) = read_two(&mut rig);
    let hold = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, hold, "industrial_parts", 44, LotId(9_983));
    file_plan(&mut rig, pilot, tv, &format!("+tv/1 plan {}", a.addr));
    let (_, heard) = work_heard(&mut rig, &[tv], 60, Some(pilot), &mut |_| {});
    let plan = plan_of(&mut rig.tw, tv).expect("the plan");
    assert!(
        matches!(
            plan.state,
            FillState::Ended {
                why: PlanEnd::HoldFull,
                ..
            }
        ),
        "{:?}",
        plan.state
    );
    assert_eq!(plan.tally.tonnes, 4, "two fair cycles fill the room");
    assert!(
        heard.contains(&format!(
            "the {HULL}'s hold holds 48 t, 0 t of it unreserved."
        )),
        "{heard}"
    );
}

/// **P6 — the compiler names only your own knowledge and rights**: a pocket never read answers
/// `"{addr}: You have no reading there."`; a pocket in another's claimed block `"{addr}: That block
/// is under claim."`; no licence → the mining act's own line (`MINE_NO_LICENCE`); a field name → `PLAN_USAGE`; seventeen
/// addresses → `PLAN_TOO_MANY`; a full hold → `PLAN_HOLD_FULL`; every refusal writes nothing; and
/// `family=` is frozen into the record.
#[test]
#[allow(clippy::too_many_lines)]
fn sm11_the_compiler_names_only_your_own_knowledge_and_rights() {
    let mut rig = stocked();
    let (pilot, row, tv, a, b) = read_two(&mut rig);
    let all = picks(&mut rig.tw);
    let chart = chart_of(&mut rig.tw, pilot).expect("the chart");
    let unread = pick(&all, "UNREAD", |p| {
        let field = tacsim::surface::address::field_of_pocket(p.pocket)
            .and_then(|(fx, fy)| tacsim::surface::address::field_index(fx, fy));
        field.is_some_and(|index| !chart.fields.contains_key(&index))
            && !chart.blocks.contains_key(&p.letter().to_string())
    });
    let before = record_of(&mut rig.tw, tv);
    let refuses = |rig: &mut Rig, line: &str, want: &str| {
        for typed_line in [line.to_owned(), format!("{line} go")] {
            let (out, queued) = command(rig, pilot, &typed_line);
            assert_eq!(out.trim(), want, "{typed_line}");
            assert!(queued.is_empty(), "{typed_line}");
        }
    };
    refuses(
        &mut rig,
        &format!("+tv/1 plan {},{}", a.addr, unread.addr),
        &format!("{}: {NO_READING}", unread.addr),
    );
    refuses(&mut rig, "+tv/1 plan M-NW", PLAN_USAGE);
    let seventeen: Vec<String> = all.iter().take(17).map(|p| p.addr.clone()).collect();
    refuses(
        &mut rig,
        &format!("+tv/1 plan {}", seventeen.join(",")),
        &PLAN_TOO_MANY.replace("{max}", "16"),
    );
    // Another's claim on B's block.
    let rival = pilot_on(&mut rig, "Rival", 7);
    claim_block(&mut rig, &b, rival, 100_000_000);
    refuses(
        &mut rig,
        &format!("+tv/1 plan {}", b.addr),
        &format!("{}: {MINE_CLAIMED}", b.addr),
    );
    // A full hold.
    let hold = hold_vault(&rig.tw, &row);
    let limit = ore_limit_t(&rig.tw);
    seed_pool(&mut rig, hold, "metal_ore", limit, LotId(9_961));
    refuses(
        &mut rig,
        &format!("+tv/1 plan {}", a.addr),
        &PLAN_HOLD_FULL.replace("{name}", HULL),
    );
    assert_eq!(record_of(&mut rig.tw, tv), before, "nothing written");
    no_refused_writes(&rig.tw);

    // No licence: the account swapped for a fresh one (the licence is keyed by the account).
    let mut rig = stocked();
    let (pilot, _, tv, a, _) = read_two(&mut rig);
    let own = account_of(&rig, pilot);
    if let Ok(mut map) = rig.oracle.0.lock() {
        map.insert(
            pilot,
            AccountId::mint("NobodyAcct", 77).expect("an account"),
        );
    }
    let (out, queued) = command(&mut rig, pilot, &format!("+tv/1 plan {}", a.addr));
    assert_eq!(out.trim(), MINE_NO_LICENCE, "the mining act's own line");
    assert!(queued.is_empty());
    if let Ok(mut map) = rig.oracle.0.lock() {
        map.insert(pilot, own);
    }
    // family= frozen.
    let _ = file_plan(
        &mut rig,
        pilot,
        tv,
        &format!("+tv/1 plan {} family=metal_ore", a.addr),
    );
    assert_eq!(
        plan_of(&mut rig.tw, tv).map(|plan| plan.family),
        Some(OreFamily::Metals)
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// P7 — a live plan holds its rig; clear releases it
// ---------------------------------------------------------------------------

/// **P7 — a live plan holds the rig and `clear` releases it** (D6, D8, D17): with a plan live the
/// hand acts answer `PLAN_LIVE` with nothing queued, a hand-built start refuses `plan_live`, and
/// `+ship/lift ceres` waits on the step in flight; `plan clear` (no `go`) is `PLAN_CLEARED`
/// with one `plan_clear` keyed `…/clear/<seq>`; the step in flight lands; the hand acts work again;
/// a second clear strikes it (`PLAN_DROPPED`, `…/strike/<ordinal>`, a different key); a new plan
/// files as plan 2, and two clears typed back to back on it are one intent (the second a HIT).
#[test]
#[allow(clippy::too_many_lines)]
fn sm11_a_live_plan_holds_the_rig_and_clear_releases_it() {
    let mut rig = stocked();
    let (pilot, _, tv, a, b) = read_two(&mut rig);
    let _ = file_plan(
        &mut rig,
        pilot,
        tv,
        &format!("+tv/1 plan {},{}", a.addr, b.addr),
    );
    // Let the plan's first step start.
    let _ = work_until(&mut rig, &[tv], 50, &mut |rig| {
        record_of(&mut rig.tw, tv)
            .is_some_and(|record| record.act.is_some() || record.run.is_some())
    });
    let live = PLAN_LIVE.replace("{n}", "1").replace("{no}", "1");
    for line in [
        "+tv/1 scan go",
        "+tv/1 drive ne 100 go",
        "+tv/1 mine go",
        "+tv/1 mine stop",
        "+tv/1 dump go",
        "+tv/1 unload go",
    ] {
        let (out, queued) = command(&mut rig, pilot, line);
        assert_eq!(out.trim(), live, "{line}");
        assert!(queued.is_empty(), "{line}");
    }
    let next = ordinal_of(&mut rig.tw, tv) + 1;
    let built = AuthoritativeSubmit {
        actor: pilot,
        action: handles(&rig.tw).surface_act.expect("the start"),
        subject: PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters: json!({ "tv": tv.0.to_string(), "act": "scan", "ordinal": next }),
        key: tacsim::surface::act_key(tv, next).expect("a key"),
    };
    let refused = land(&mut rig, &built).expect_err("a hand start under a live plan");
    assert!(
        refused.contains("ProviderRejected") && refused.contains("that rig is working a plan"),
        "plan_live at the prepare: {refused}"
    );
    // The departure: the step in flight holds her (an act outranks the plan; P8 reads the plan's
    // own hold with the rig in the bay).
    let (lift, queued) = command(&mut rig, pilot, "+ship/lift ceres");
    assert!(
        lift.contains("; the Salt Moth lifts once it is done."),
        "{lift}"
    );
    assert!(queued.is_empty());
    // Clear: the step in flight finishes.
    let seq = plan_of(&mut rig.tw, tv).expect("the plan").seq;
    let (out, queued) = command(&mut rig, pilot, "+tv/1 plan clear");
    assert_eq!(
        out.lines().next(),
        Some(
            PLAN_CLEARED
                .replace("{n}", "1")
                .replace("{no}", "1")
                .as_str()
        )
    );
    assert_eq!(queued.len(), 1);
    assert_eq!(act_word(&queued[0]), "plan_clear");
    let clear_key = key_text(tacsim::surface::act::plan_clear_key(tv, 1, seq));
    assert_eq!(queued[0].key.as_str(), clear_key);
    land_all(&mut rig, &queued);
    let landed = work(&mut rig, &[tv], 100, &mut |_| {});
    assert!(
        landed
            .iter()
            .all(|l| l.kind == ActKind::Mine || l.kind == ActKind::Drive),
        "only the step in flight: {landed:?}"
    );
    // The step in flight lands with `by: Plan` (the reviewer's T6): its outcome keeps the plan's
    // number and step though the plan was cleared under it.
    let last = record_of(&mut rig.tw, tv)
        .and_then(|record| record.last)
        .expect("the step's outcome");
    assert_eq!(last.by, Some(ActBy::Plan { no: 1, step: 0 }), "{last:?}");
    let plan = plan_of(&mut rig.tw, tv).expect("the plan");
    assert!(
        matches!(
            plan.state,
            FillState::Ended {
                why: PlanEnd::Cleared,
                ..
            }
        ),
        "{plan:?}"
    );
    let (out, _) = command(&mut rig, pilot, "+tv/1 scan");
    assert_ne!(out.trim(), live, "the hand acts work again");
    // Strike it.
    let ordinal = ordinal_of(&mut rig.tw, tv);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 plan clear");
    assert_eq!(
        out.lines().next(),
        Some(
            PLAN_DROPPED
                .replace("{n}", "1")
                .replace("{no}", "1")
                .as_str()
        )
    );
    assert_eq!(queued.len(), 1);
    assert_eq!(act_word(&queued[0]), "plan_strike");
    let strike_key = key_text(tacsim::surface::act::plan_strike_key(tv, 1, ordinal));
    assert_eq!(queued[0].key.as_str(), strike_key);
    assert_ne!(strike_key, clear_key);
    land_all(&mut rig, &queued);
    assert_eq!(plan_of(&mut rig.tw, tv), None);
    // Plan 2, and two clears back to back.
    let _ = file_plan(&mut rig, pilot, tv, &format!("+tv/1 plan {}", b.addr));
    let (_, first) = command(&mut rig, pilot, "+tv/1 plan clear");
    let (_, second) = command(&mut rig, pilot, "+tv/1 plan clear");
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert_eq!(first[0].key, second[0].key, "the same key");
    assert_eq!(first[0].parameters, second[0].parameters, "the same intent");
    land_all(&mut rig, &first);
    assert_eq!(
        land(&mut rig, &second[0]),
        Err(HIT.to_owned()),
        "one intent, one write"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// P8 — span of control (A5)
// ---------------------------------------------------------------------------

/// **P8 — a fifth excursion for one operator refuses** (A5; lead ruling 8): four other rigs'
/// records seeded with open excursions naming the pilot (two on the pilot's hull, two on another),
/// the pilot's own rig in the bay with a plan filed: the plan's launch waits (`PLAN_WAITS` with the
/// cap's words, told once to the connected pilot; nothing submitted over many fires; no drive leg
/// planned while the cap holds); `+tv/1 launch` answers `CAP_OPERATOR` before the filing (and
/// `PLAN_LIVE` after it: the plan holds its rig); `+ship/lift ceres` reads `TV_BUSY` "working its
/// plan" with `PLAN_REMEDY`; one seeded excursion closed, the plan's first move lands.
#[test]
#[allow(clippy::too_many_lines)]
fn sm11_a_fifth_excursion_for_one_operator_refuses() {
    let mut rig = stocked();
    let (pilot, row, tv, a, _) = read_two(&mut rig);
    let hull = ObjId(row.hull);
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let _ = finish(&mut rig, tv, pilot);
    let _ = work(&mut rig, &[tv], 50, &mut |_| {});
    let others: Vec<ObjId> = port_map(&rig.tw, "ceres_port")
        .vehicles
        .expect("the rig pool")
        .rigs
        .iter()
        .map(|rig| ObjId(rig.tv))
        .filter(|other| *other != tv)
        .take(4)
        .collect();
    assert_eq!(others.len(), 4);
    for (k, other) in others.iter().enumerate() {
        let mut record = RigRecord::fresh(*other, &battery());
        record.excursion = Some(tacsim::surface::Excursion {
            id: tacsim::surface::ExcursionId {
                tv: *other,
                launch_ic: ic_now(&rig.tw),
            },
            operator: pilot,
            hull: if k < 2 { hull } else { ObjId(hull.0 + 999_999) },
            slot: 1,
            site: cid(AHUNA),
        });
        put_record(&mut rig, record);
    }
    // The hand launch meets the cap (before a plan holds the rig: a live plan answers PLAN_LIVE).
    let (out, queued) = command(&mut rig, pilot, "+tv/1 launch");
    assert_eq!(out.trim(), CAP_OPERATOR.replace("{cap}", "4"));
    assert!(queued.is_empty());
    let _ = file_plan(&mut rig, pilot, tv, &format!("+tv/1 plan {}", a.addr));
    let seq = plan_of(&mut rig.tw, tv).expect("the plan").seq;
    let (_, decides_before, legs_before) =
        tacsim::surface::act::plan_counts_for_tests(rig.tw.world());
    let decides = |rig: &Rig| tacsim::surface::act::plan_counts_for_tests(rig.tw.world()).1;
    let mut heard = String::new();
    let mut cached: Option<(u32, i64)> = None;
    // The cap's wait is cached for PLAN_WAIT_IC_SECS (300 IC s; 6 IC s a fire at rate 1): the
    // interpreter runs once on the first fire, not again inside the wait, and once more when it
    // runs out (the reviewer's T4: D16's "at most one decide every 300 IC s").
    for k in 0..60 {
        let (events, queued) = fire(&mut rig);
        heard.push_str(&said(&rig.tw, &events, pilot));
        assert!(
            starts_for(&queued, tv).is_empty(),
            "nothing starts under the cap"
        );
        land_all(&mut rig, &queued);
        if k == 0 {
            cached = tacsim::surface::tick::plan_wait_for_tests(rig.tw.world(), tv);
            assert_eq!(
                decides(&rig) - decides_before,
                1,
                "one decide on the first fire"
            );
        }
        let (_, until) = cached.expect("the cap's wait is cached");
        let want = if ic_now(&rig.tw) >= until { 2 } else { 1 };
        assert_eq!(
            decides(&rig) - decides_before,
            want,
            "decides at IC {} (the wait runs to {until})",
            ic_now(&rig.tw)
        );
    }
    let (cached_seq, until) = cached.expect("the cap's wait is cached");
    assert_eq!(cached_seq, seq, "cached under the plan's seq");
    assert!(
        ic_now(&rig.tw) >= until,
        "the run of fires passed the first wait"
    );
    let waits = PLAN_WAITS.replace("{n}", "1").replace("{why}", WAIT_CAP);
    assert_eq!(count(&heard, &waits), 1, "told once: {heard}");
    let (_, _, legs_after) = tacsim::surface::act::plan_counts_for_tests(rig.tw.world());
    assert_eq!(
        legs_after, legs_before,
        "no drive leg planned while the cap holds"
    );
    // A hand-built plan start under the cap refuses at the prepare: the offer's decision waits.
    let next = ordinal_of(&mut rig.tw, tv) + 1;
    let built = AuthoritativeSubmit {
        actor: pilot,
        action: handles(&rig.tw).surface_act.expect("the start"),
        subject: PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters: json!({
            "tv": tv.0.to_string(),
            "act": "drive",
            "ordinal": next,
            "by": "plan",
            "plan": 1,
            "step": 0,
        }),
        key: tacsim::surface::act::plan_step_key(tv, next, 0, "drive").expect("a key"),
    };
    let refused = land(&mut rig, &built).expect_err("a plan start under the cap");
    assert!(
        refused.contains("that rig's plan is waiting"),
        "plan_waits at the prepare: {refused}"
    );
    assert_eq!(ordinal_of(&mut rig.tw, tv) + 1, next, "nothing started");
    let (out, queued) = command(&mut rig, pilot, "+tv/1 launch");
    assert_eq!(
        out.trim(),
        PLAN_LIVE.replace("{n}", "1").replace("{no}", "1"),
        "the plan holds its rig"
    );
    assert!(queued.is_empty());
    // The departure: a plan whose rig waits in the bay holds her too (the brief's 3.9).
    let (lift, queued) = command(&mut rig, pilot, "+ship/lift ceres");
    assert!(
        lift.contains("Rig 1 is working its plan; the Salt Moth lifts once it is done.")
            && lift.contains(&PLAN_REMEDY.replace("{n}", "1")),
        "{lift}"
    );
    assert!(queued.is_empty());
    // One excursion closes: the plan's launch (its drive from the bay) lands.
    let mut freed = record_of(&mut rig.tw, others[0]).expect("seeded");
    freed.excursion = None;
    put_record(&mut rig, freed);
    let landed = work_until(&mut rig, &[tv], 200, &mut |rig| {
        record_of(&mut rig.tw, tv).is_some_and(|record| record.act.is_some())
    });
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert!(
        record
            .act
            .as_ref()
            .is_some_and(|act| act.by == Some(ActBy::Plan { no: 1, step: 0 })),
        "{landed:?} {record:?}"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// P11, P12 — the idle return and the stranded rig
// ---------------------------------------------------------------------------

/// **P11 — an idle rig comes home after a day** (lead ruling 6): a rig left on the ground with ore
/// after a hand run, the pilot offline: at 24 IC h less a minute nothing starts; at 24 IC h the
/// tick submits `recover` with `by: idle` keyed `…/idle`; it lands and SM.8's queued unload
/// follows; the queued notices hold exactly `IDLE_HOME` (with its unload clause) and `UNLOADED`.
#[test]
fn sm11_an_idle_rig_comes_home_after_a_day() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let all = picks(&mut rig.tw);
    let a = fair7(&all);
    launched(&mut rig, pilot, 1, tv);
    let t = mined(&mut rig, pilot, 1, tv, &a);
    assert!(t > 0);
    disconnect(&mut rig.tw, pilot);
    let since = record_of(&mut rig.tw, tv)
        .and_then(|record| record.idle_since())
        .expect("idle since");
    let (_, queued) = advance_to(&mut rig, since + 24 * 3_600 - 60);
    assert!(starts_for(&queued, tv).is_empty(), "not yet");
    land_all(&mut rig, &queued);
    let ordinal = ordinal_of(&mut rig.tw, tv) + 1;
    let (_, queued) = advance_to(&mut rig, since + 24 * 3_600);
    let starts = starts_for(&queued, tv);
    assert_eq!(starts.len(), 1, "{queued:?}");
    assert_eq!(act_word(&starts[0]), "recover");
    assert_eq!(starts[0].parameters["by"], json!("idle"));
    assert_eq!(starts[0].actor, pilot, "as the operator");
    assert_eq!(
        starts[0].key.as_str(),
        key_text(tacsim::surface::act::idle_key(tv, ordinal))
    );
    land_all(&mut rig, &queued);
    let landed = work(&mut rig, &[tv], 100, &mut |_| {});
    assert!(
        landed.iter().any(|l| l.kind == ActKind::Unload),
        "{landed:?}"
    );
    let queued = pending(&rig.tw, pilot);
    println!("P11 pending: {queued:#?}");
    assert_eq!(queued.len(), 2, "{queued:?}");
    assert!(
        queued[0].starts_with("Rig 1 stood a day idle on Ahuna Flank and came home into the Salt Moth's bay; it unloads its hopper next."),
        "{queued:?}"
    );
    assert!(queued[1].starts_with("Rig 1 unloads "), "{queued:?}");
    no_refused_writes(&rig.tw);
}

/// **P12 — a stranded rig is parked where it stands and told once** (lead ruling 6): a rig on the
/// ground whose charge staff set below its way home, the pilot offline: at 24 IC h the projection
/// reads `Parked` and the queued notices hold one `STRANDED`; no recover is submitted over the next
/// IC day; after it the tick tries once more, still `Parked`, and tells nothing again.
#[test]
fn sm11_a_stranded_rig_is_parked_where_it_stands_and_told_once() {
    let mut rig = stocked();
    let (pilot, _, tv, _, _) = read_two(&mut rig);
    disconnect(&mut rig.tw, pilot);
    let mut record = record_of(&mut rig.tw, tv).expect("the record");
    record.charge = Charge {
        value_cu: 1,
        at_ic: ic_now(&rig.tw),
        in_bay: false,
    };
    put_record(&mut rig, record);
    let since = record_of(&mut rig.tw, tv)
        .and_then(|record| record.idle_since())
        .expect("idle since");
    let (_, queued) = advance_to(&mut rig, since + 24 * 3_600);
    assert!(
        starts_for(&queued, tv).is_empty(),
        "no recover it cannot pay"
    );
    land_all(&mut rig, &queued);
    let (_, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    assert_eq!(
        projection(&rig.tw, tv).map(|rec| rec.state),
        Some(TvState::Parked)
    );
    let stranded = |rig: &Rig| {
        pending(&rig.tw, pilot)
            .iter()
            .filter(|line| line.contains("it is parked where it stands on Ahuna Flank"))
            .count()
    };
    assert_eq!(stranded(&rig), 1);
    let until = ic_now(&rig.tw) + 24 * 3_600 - 120;
    while ic_now(&rig.tw) < until {
        let at = (ic_now(&rig.tw) + 3_600).min(until);
        let (_, queued) = advance_to(&mut rig, at);
        assert!(
            starts_for(&queued, tv).is_empty(),
            "no recover inside the day"
        );
        land_all(&mut rig, &queued);
    }
    let at = ic_now(&rig.tw) + 600;
    let (_, queued) = advance_to(&mut rig, at);
    land_all(&mut rig, &queued);
    let (_, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    assert_eq!(
        projection(&rig.tw, tv).map(|rec| rec.state),
        Some(TvState::Parked)
    );
    assert_eq!(stranded(&rig), 1, "told once");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// P13 — the notice budget
// ---------------------------------------------------------------------------

/// **P13 — the notice budget queues a trip, not a cycle** (lead ruling 7, D13): offline, a hand run
/// of four cycles, a recover and its unload queue exactly one line (`UNLOADED`); connected, the
/// same sequence is heard line by line (four cycles, the recover's `RECOVERED_UNLOADING`, the
/// unload's `UNLOADED` — the lead's pre-commit round, item 6, the judge's T11) and queues none.
#[test]
fn sm11_the_notice_budget_queues_a_trip_not_a_cycle() {
    for online in [false, true] {
        let mut rig = stocked();
        let (pilot, _, tvs) = on_pad(&mut rig, 1);
        let tv = tvs[0];
        let all = picks(&mut rig.tw);
        let b = pick(&all, "EIGHT", |p| {
            p.footing != Footing::Rock
                && p.families() == 1
                && p.classes[0] == Word::Fair
                && p.tonnes[0] == 8
        });
        launched(&mut rig, pilot, 1, tv);
        sounded_at(&mut rig, pilot, 1, tv, &b);
        let _ = act_go(&mut rig, pilot, "+tv/1 mine go");
        if !online {
            disconnect(&mut rig.tw, pilot);
        }
        let mut heard = String::new();
        while run_of(&mut rig.tw, tv).is_some() {
            heard.push_str(&cycle(&mut rig, tv, pilot));
        }
        let (out, queued) = command(&mut rig, pilot, "+tv/1 recover go");
        assert_eq!(queued.len(), 1, "{out}");
        land_all(&mut rig, &queued);
        let (landed, tail) = work_heard(&mut rig, &[tv], 50, Some(pilot), &mut |_| {});
        heard.push_str(&tail);
        assert!(landed.iter().any(|l| l.kind == ActKind::Unload));
        let queued = pending(&rig.tw, pilot);
        println!("P13 online={online} pending: {queued:#?}");
        if online {
            assert!(queued.is_empty(), "connected: nothing queued: {queued:?}");
            assert_eq!(heard.matches("Rig 1, cycle ").count(), 4, "{heard}");
            let back = RECOVERED_UNLOADING
                .replace("{n}", "1")
                .replace("{name}", HULL);
            assert_eq!(count(&heard, &back), 1, "the recover heard: {heard}");
            let unloaded: Vec<&str> = heard
                .lines()
                .filter(|line| fills(line, UNLOADED) && line.starts_with("Rig 1 unloads "))
                .collect();
            assert_eq!(unloaded.len(), 1, "the unload heard: {heard}");
        } else {
            assert_eq!(queued.len(), 1, "{queued:?}");
            assert!(queued[0].starts_with("Rig 1 unloads "), "{queued:?}");
        }
        no_refused_writes(&rig.tw);
    }
}

// ---------------------------------------------------------------------------
// P9 — via Plan, never a stake (A6)
// ---------------------------------------------------------------------------

/// Port fires until the queue is quiet for two fires (bounded at 40), every submission landed
/// (`claims.rs`'s `settle`).
fn settle(rig: &mut Rig) {
    let mut quiet = 0;
    for _ in 0..40 {
        let (_, queued) = fire(rig);
        if queued.is_empty() {
            quiet += 1;
            if quiet >= 2 {
                return;
            }
        } else {
            quiet = 0;
            land_all(rig, &queued);
        }
    }
}

/// **`+claim/<act> … go` as `who`, exactly one tenure act queued**, landed and settled; the reply.
fn claim_go(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let (out, queued) = command(rig, who, line);
    assert_eq!(queued.len(), 1, "{line}: one tenure act: {out}");
    land_all(rig, &queued);
    settle(rig);
    out
}

/// Every hand stamp in `row`'s rings, as `(row key, stamp)`.
fn hand_stamps(row: &ChartRecord) -> Vec<(String, i64)> {
    let fields = row.fields.iter().flat_map(|(index, field)| {
        field
            .provenance
            .iter()
            .filter_map(move |stamp| match stamp {
                ChartProvenance::Local {
                    at, via: Via::Hand, ..
                } => Some((format!("field {index}"), *at)),
                ChartProvenance::Local { .. } => None,
            })
    });
    let blocks = row.blocks.iter().flat_map(|(letter, block)| {
        block
            .provenance
            .iter()
            .filter_map(move |stamp| match stamp {
                ChartProvenance::Local {
                    at, via: Via::Hand, ..
                } => Some((format!("block {letter}"), *at)),
                ChartProvenance::Local { .. } => None,
            })
    });
    fields.chain(blocks).collect()
}

/// Every row's `first_hand_at`.
fn first_hands(row: &ChartRecord) -> Vec<(String, Option<i64>)> {
    row.fields
        .iter()
        .map(|(index, field)| (format!("field {index}"), field.first_hand_at))
        .chain(
            row.blocks
                .iter()
                .map(|(letter, block)| (format!("block {letter}"), block.first_hand_at)),
        )
        .collect()
}

/// **P9 — a plan's readings carry `via: Plan` and never stake** (A6; lead ruling 4; the
/// reviewer's F6, F7): (1) pilot A reads block `X` by hand — a scan and a quiet sounding — before
/// (2) the block's epoch moves on: pilot B sounds `X` loudly, stakes it and releases it; (3) A's
/// plan on `X` sounds and cores it: every stamp it writes is `via: Plan`, no `first_hand_at`
/// moves and every hand stamp of (1) is still in its ring; (4) A drives back into `X` by hand and
/// `+claim/stake` is `STAKE_NO_EVIDENCE` — the plan's readings are A's own but never a stake's;
/// (5) none of the plan's loud soundings is heard evidence in A's row; (7) a hand sounding repeated
/// from the plan's last sounding stand learns nothing new (the plan's reading is A's own); (6) a
/// hand sounding from another stand in `X` qualifies the stake.
#[test]
#[allow(clippy::too_many_lines)]
fn sm11_plan_made_readings_carry_via_plan_and_never_stake() {
    let mut rig = stocked();
    let (a_pilot, a_row, tvs) = on_pad(&mut rig, 1);
    let a_tv = tvs[0];
    let all = picks(&mut rig.tw);
    let fair = fair7(&all);
    let x = fair.letter();
    // (1) A's hand readings of X.
    launched(&mut rig, a_pilot, 1, a_tv);
    to_cell(&mut rig, a_tv, fair.stand);
    scanned_here(&mut rig, a_pilot, 1, a_tv);
    let _ = act_go(&mut rig, a_pilot, "+tv/1 sound quiet go");
    let _ = finish(&mut rig, a_tv, a_pilot);
    // (2) B sounds X loudly, stakes it and releases it.
    let (b_pilot, _, b_tvs) = on_pad_as(&mut rig, ("Bryn", 2, "Grey Heron"), 1);
    let b_tv = b_tvs[0];
    launched(&mut rig, b_pilot, 1, b_tv);
    let other = pick(&all, "B_STAND", |p| {
        p.letter() == x && p.pocket != fair.pocket && p.footing != Footing::Rock
    });
    to_cell(&mut rig, b_tv, other.stand);
    let _ = act_go(&mut rig, b_pilot, "+tv/1 sound go");
    let _ = finish(&mut rig, b_tv, b_pilot);
    let staked = claim_go(&mut rig, b_pilot, "+claim/stake go");
    assert!(
        staked.contains(&format!("You stake block {x} at ")),
        "{staked}"
    );
    let released = claim_go(&mut rig, b_pilot, &format!("+claim/release {x} go"));
    println!("P9 release: {released}");
    let since = ic_now(&rig.tw);
    // B's rig home, so its open excursion never matters.
    let _ = act_go(&mut rig, b_pilot, "+tv/1 recover go");
    let _ = finish(&mut rig, b_tv, b_pilot);
    // (3) A's plan on X, the hold seeded to one hopper's room.
    let before = chart_of(&mut rig.tw, a_pilot).expect("A's chart");
    let hands = hand_stamps(&before);
    let firsts = first_hands(&before);
    assert!(!hands.is_empty());
    let hold = hold_vault(&rig.tw, &a_row);
    let limit = ore_limit_t(&rig.tw);
    seed_pool(&mut rig, hold, "metal_ore", limit - 6, LotId(9_991));
    let _ = file_plan(
        &mut rig,
        a_pilot,
        a_tv,
        &format!("+tv/1 plan {x} family=metal_ore"),
    );
    let filed_ic = plan_of(&mut rig.tw, a_tv).expect("the plan").filed_ic;
    let landed = work(&mut rig, &[a_tv], 1_000, &mut |_| {});
    println!("P9 landed: {landed:?}");
    let sounded_fields: Vec<u8> = plan_of(&mut rig.tw, a_tv)
        .expect("the plan")
        .steps
        .iter()
        .filter_map(|step| match step {
            PlanStep::Sound { field } => Some(*field),
            PlanStep::Mine { .. } | PlanStep::Block { .. } => None,
        })
        .collect();
    assert!(
        landed.iter().any(|l| l.kind == ActKind::Sound),
        "the plan sounded"
    );
    assert!(
        landed.iter().any(|l| l.kind == ActKind::Mine),
        "the plan cored"
    );
    let after = chart_of(&mut rig.tw, a_pilot).expect("A's chart");
    let stamps: Vec<&ChartProvenance> = after
        .fields
        .values()
        .flat_map(|row| row.provenance.iter())
        .chain(after.blocks.values().flat_map(|row| row.provenance.iter()))
        .collect();
    assert!(
        stamps.iter().any(|stamp| stamp.at() > filed_ic),
        "the plan wrote stamps"
    );
    for stamp in &stamps {
        let ChartProvenance::Local { at, via, .. } = stamp;
        if *at > filed_ic {
            assert_eq!(*via, Via::Plan, "{stamp:?}");
        }
    }
    let after_firsts: BTreeMap<String, Option<i64>> = first_hands(&after).into_iter().collect();
    for (key, first) in &firsts {
        assert_eq!(after_firsts.get(key), Some(first), "{key}'s first_hand_at");
    }
    let after_hands = hand_stamps(&after);
    for hand in &hands {
        assert!(after_hands.contains(hand), "{hand:?} kept in its ring");
    }
    // (4) Back into X by hand; the stake has no evidence.
    if record_of(&mut rig.tw, a_tv).is_some_and(|record| record.excursion.is_none()) {
        launched(&mut rig, a_pilot, 1, a_tv);
    }
    let _ = act_go(&mut rig, a_pilot, &format!("+tv/1 drive {} go", fair.addr));
    let _ = finish(&mut rig, a_tv, a_pilot);
    let (out, queued) = command(&mut rig, a_pilot, "+claim/stake");
    assert_eq!(
        out.trim(),
        tacsim::port::claims::STAKE_NO_EVIDENCE.replace("{block}", &x.to_string())
    );
    assert!(queued.is_empty());
    // (5) No plan sounding is evidence.
    let row = chart_of(&mut rig.tw, a_pilot).expect("A's chart");
    assert_eq!(
        tacsim::surface::tenure::evidence_at(&row, x, Some(since)),
        None
    );
    assert!(tacsim::surface::tenure::heard_before(&row, x, Some(since), i64::MAX).is_empty());
    // (7) The plan's sounding stand is A's own: a hand repeat there learns nothing new. The stand
    // is a field middle the plan sounded from (the reviewer's T8: never skipped).
    let middles: Vec<[i64; 2]> = sounded_fields
        .iter()
        .filter_map(|field| tacsim::surface::address::field_of_index(*field))
        .map(|(fx, fy)| [fx * 32 + 16, fy * 32 + 16])
        .collect();
    let [sx, sy] = row.sound_stand.expect("the plan's last sounding stand");
    assert!(
        middles.contains(&[sx, sy]),
        "the stand {:?} is a field middle the plan sounded from: {middles:?}",
        [sx, sy]
    );
    to_cell(&mut rig, a_tv, (sx, sy));
    let (out, queued) = command(&mut rig, a_pilot, "+tv/1 sound go");
    assert!(out.contains("You learn nothing new here."), "{out}");
    assert!(queued.is_empty());
    // (6) A hand sounding from another stand qualifies the stake.
    to_cell(&mut rig, a_tv, (fair.stand.0 + 1, fair.stand.1));
    let _ = act_go(&mut rig, a_pilot, "+tv/1 sound quiet go");
    let _ = finish(&mut rig, a_tv, a_pilot);
    let (out, queued) = command(&mut rig, a_pilot, "+claim/stake");
    assert!(
        out.starts_with(&format!("Block {x}, your readings: ")),
        "{out}"
    );
    assert!(queued.is_empty());
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// A previous lessee's plan (the observer rule)
// ---------------------------------------------------------------------------

/// **A previous lessee's plan is never shown to the next** (the fixer round's conformance/F1; doc
/// 20 §5 and §16, "no reply names what the asker may not see"): a plan another pilot filed left on
/// the rig's record — live or ended — shows in neither the next lessee's `+tv`, `plan show`, `plan
/// clear` (`PLAN_NONE`, nothing submitted) nor their projection; it holds nothing (their hand acts
/// run); their own filing replaces it, and their own plan is theirs to read.
#[test]
#[allow(clippy::too_many_lines)]
fn sm11_a_previous_lessees_plan_is_never_shown() {
    use tacsim::surface::plan::{PLAN_BOUND_IC_SECS, PlanTally};
    let mut rig = stocked();
    let (pilot, row, tv, a, _) = read_two(&mut rig);
    let previous = pilot_on(&mut rig, "Previous", 9);
    let now = ic_now(&rig.tw);
    let theirs = |state: FillState| FillPlan {
        no: 1,
        author: previous,
        hull: ObjId(row.hull),
        slot: 1,
        site: cid(AHUNA),
        family: OreFamily::Metals,
        filed_ic: now - 10,
        deadline_ic: now - 10 + PLAN_BOUND_IC_SECS,
        list: vec![a.addr.clone()],
        steps: vec![PlanStep::Mine {
            pocket: [a.pocket.0, a.pocket.1],
        }],
        cursor: 0,
        passed: Vec::new(),
        state,
        seq: 1,
        tally: PlanTally {
            tonnes: 6,
            cycles: 3,
            trips: 1,
        },
        notes: Vec::new(),
    };
    for state in [
        FillState::Live,
        FillState::Ended {
            why: PlanEnd::HoldFull,
            at_ic: now - 5,
        },
    ] {
        let mut record = record_of(&mut rig.tw, tv).expect("the record");
        record.plan = Some(theirs(state));
        record.plans = 1;
        put_record(&mut rig, record);
        for _ in 0..2 {
            let (_, queued) = fire(&mut rig);
            assert!(
                starts_for(&queued, tv).is_empty(),
                "an orphaned plan is inert"
            );
            land_all(&mut rig, &queued);
        }
        let status = run(&mut rig.tw, pilot, "+tv");
        assert!(!status.contains("  Plan 1"), "{status}");
        for line in ["+tv/1 plan show", "+tv/1/plan/show"] {
            let (out, queued) = command(&mut rig, pilot, line);
            assert_eq!(out.trim(), PLAN_NONE.replace("{n}", "1"), "{line}");
            assert!(queued.is_empty());
        }
        let (out, queued) = command(&mut rig, pilot, "+tv/1 plan clear");
        assert_eq!(out.trim(), PLAN_NONE.replace("{n}", "1"));
        assert!(queued.is_empty(), "nothing submitted");
        assert_eq!(
            projection(&rig.tw, tv).and_then(|rec| rec.plan),
            None,
            "the projection carries none of it"
        );
        let (out, _) = command(&mut rig, pilot, "+tv/1 scan");
        assert_ne!(
            out.trim(),
            PLAN_LIVE.replace("{n}", "1").replace("{no}", "1"),
            "it holds nothing"
        );
    }
    // Their own filing replaces it; their own plan is theirs to read.
    let _ = file_plan(&mut rig, pilot, tv, &format!("+tv/1 plan {}", a.addr));
    let plan = plan_of(&mut rig.tw, tv).expect("the plan");
    assert_eq!((plan.no, plan.author), (2, pilot));
    let status = run(&mut rig.tw, pilot, "+tv");
    assert!(
        status.contains(&format!("  Plan 2: step 1 of 1, mine {}; until ", a.addr)),
        "{status}"
    );
    let shown = run(&mut rig.tw, pilot, "+tv/1 plan show");
    assert!(
        shown.starts_with("Rig 1's plan 2 on Ahuna Flank"),
        "{shown}"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// P10 — await tv
// ---------------------------------------------------------------------------

/// The orders log's texts.
fn log_of(tw: &TestWorld, hull: ObjId) -> Vec<String> {
    orders(tw, hull)
        .map(|doc| doc.log.iter().map(|line| line.text.clone()).collect())
        .unwrap_or_default()
}

/// Whether `hull` is still settled on Ahuna's pad.
fn on_ahuna(tw: &TestWorld, hull: ObjId) -> bool {
    matches!(
        tacsim::port::ship::berth_of(tw.world(), hull),
        tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == AHUNA
    )
}

/// One order's state.
fn order_state(tw: &TestWorld, hull: ObjId, id: u32) -> Option<OrderState> {
    orders(tw, hull).and_then(|doc| doc.order(id).map(|o| o.state))
}

/// **P10 — `await tv` waits for the rigs** (lead ruling 5; the reviewer's F3, F4): (a) `await tv
/// all; depart ceres` with rig 1 working a plan and rig 2 in the bay — the depart never fires while
/// the plan is live, and the await completes once it ends; (b) `await tv 2` completes while rig 1
/// is out, and the depart then waits on `TV_OUT`; (c) `await tv until=1` with a rig out times out
/// after one IC hour, `awaited` cleared, the depart waiting on `TV_OUT`; (d) the dry run (`+ship/plan`): behind
/// `await tv all` the depart reads no `TV_OUT` and carries `AWAIT_DRY_NOTE`, behind `await tv 2`
/// with rig 1 out it reads rig 1's `TV_OUT`.
#[test]
#[allow(clippy::too_many_lines)]
fn sm11_await_tv_waits_for_the_rigs() {
    // (a)
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 2);
    let hull = ObjId(row.hull);
    let all = picks(&mut rig.tw);
    let a = fair7(&all);
    launched(&mut rig, pilot, 1, tvs[0]);
    sounded_at(&mut rig, pilot, 1, tvs[0], &a);
    let _ = file_plan(&mut rig, pilot, tvs[0], &format!("+tv/1 plan {}", a.addr));
    let _ = order(&mut rig, pilot, "await tv all");
    let _ = order(&mut rig, pilot, "depart ceres");
    let _ = work(&mut rig, &tvs, 400, &mut |rig| {
        assert!(
            on_ahuna(&rig.tw, hull),
            "the depart never fires while the plan works"
        );
    });
    assert!(
        matches!(
            plan_of(&mut rig.tw, tvs[0]).map(|plan| plan.state),
            Some(FillState::Ended { .. })
        ),
        "the plan ended"
    );
    for _ in 0..6 {
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
    }
    let log = log_of(&rig.tw, hull);
    assert!(
        log.iter()
            .any(|line| line.ends_with("Every rig parked; the sailing plan runs on.")),
        "{log:?}"
    );
    assert_eq!(order_state(&rig.tw, hull, 1), Some(OrderState::Done));
    assert!(
        orders(&rig.tw, hull)
            .and_then(|doc| doc.order(1).map(|o| o.awaited))
            .flatten()
            .is_none(),
        "awaited cleared with the Done"
    );
    no_refused_writes(&rig.tw);

    // (b) and (d): rig 1 out by hand, rig 2 in the bay.
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 2);
    let hull = ObjId(row.hull);
    launched(&mut rig, pilot, 1, tvs[0]);
    let _ = order(&mut rig, pilot, "await tv 2");
    let _ = order(&mut rig, pilot, "depart ceres");
    let dry = run(&mut rig.tw, pilot, "+ship/plan");
    println!("P10 dry (await tv 2): {dry}");
    let depart = dry
        .lines()
        .find(|line| line.starts_with("  Order 2, depart for Ceres Freeport: "))
        .expect("the depart's line");
    assert!(
        depart.contains("it would hold: Rig 1 is out on Ahuna Flank"),
        "the gate over every rig but slot 2: {dry}"
    );
    for _ in 0..6 {
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
    }
    let log = log_of(&rig.tw, hull);
    assert!(
        log.iter()
            .any(|line| line.ends_with("Rig 2 parked; the sailing plan runs on.")),
        "{log:?}"
    );
    assert!(on_ahuna(&rig.tw, hull), "the depart waits for rig 1");
    assert!(
        log.iter()
            .any(|line| line.contains("Rig 1 is out on Ahuna Flank")),
        "{log:?}"
    );
    no_refused_writes(&rig.tw);

    // (c) and (d): an hour's await with rig 1 out.
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let hull = ObjId(row.hull);
    launched(&mut rig, pilot, 1, tvs[0]);
    let _ = order(&mut rig, pilot, "await tv all");
    let _ = order(&mut rig, pilot, "depart ceres");
    let dry = run(&mut rig.tw, pilot, "+ship/plan");
    println!("P10 dry (await tv all): {dry}");
    let depart = dry
        .lines()
        .find(|line| line.starts_with("  Order 2, depart for Ceres Freeport: "))
        .expect("the depart's line");
    assert!(
        depart.ends_with(tacsim::port::executor::AWAIT_DRY_NOTE) && !depart.contains("is out on"),
        "behind await tv all the rigs' half of the gate is skipped: {dry}"
    );
    let _ = run(&mut rig.tw, pilot, "+ship/order clear");
    let _ = order(&mut rig, pilot, "await tv until=1");
    let _ = order(&mut rig, pilot, "depart ceres");
    let (_, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    let started = orders(&rig.tw, hull)
        .and_then(|doc| doc.orders.first().and_then(|o| o.awaited))
        .expect("the await's start");
    let at = started + 3_600 + 60;
    let (_, queued) = advance_to(&mut rig, at);
    land_all(&mut rig, &queued);
    for _ in 0..4 {
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
    }
    let log = log_of(&rig.tw, hull);
    assert!(
        log.iter().any(|line| line.ends_with(
            &tacsim::port::executor::AWAIT_TIMED_OUT
                .replace("{h}", "1")
                .replace("{which}", "every rig")
                .replace("{out}", "rig 1 is")
        )),
        "{log:?}"
    );
    let doc = orders(&rig.tw, hull).expect("the plan");
    assert!(
        doc.orders.iter().all(|o| o.awaited.is_none()),
        "awaited cleared"
    );
    assert!(on_ahuna(&rig.tw, hull), "the depart waits for the rig out");
    assert!(
        log.iter()
            .any(|line| line.contains("Rig 1 is out on Ahuna Flank")),
        "{log:?}"
    );
    no_refused_writes(&rig.tw);
}

/// **P10 (e) and (f) — what `await tv` counts as parked** (the reviewer's F3): (e) a rig out by
/// hand with yesterday's ended plan on its record keeps `await tv all` waiting, and it completes
/// once that rig is recovered into the bay; (f) an idle rig stranded with no plan (its projection
/// `Parked`) completes `await tv all` where it stands, and the depart behind it waits on `TV_OUT`.
#[test]
#[allow(clippy::too_many_lines)]
fn sm11_await_tv_reads_a_stale_plan_and_a_stranded_rig() {
    // (e)
    let mut rig = stocked();
    let (pilot, row, tv, a, _) = read_two(&mut rig);
    let hull = ObjId(row.hull);
    let _ = file_plan(&mut rig, pilot, tv, &format!("+tv/1 plan {}", a.addr));
    let _ = work(&mut rig, &[tv], 400, &mut |_| {});
    assert!(matches!(
        plan_of(&mut rig.tw, tv).map(|plan| plan.state),
        Some(FillState::Ended { .. })
    ));
    launched(&mut rig, pilot, 1, tv);
    let _ = order(&mut rig, pilot, "await tv all");
    for _ in 0..6 {
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
    }
    assert_ne!(
        order_state(&rig.tw, hull, 1),
        Some(OrderState::Done),
        "a rig out by hand since its plan ended is not parked"
    );
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let _ = finish(&mut rig, tv, pilot);
    for _ in 0..6 {
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
    }
    assert_eq!(order_state(&rig.tw, hull, 1), Some(OrderState::Done));
    no_refused_writes(&rig.tw);

    // (f)
    let mut rig = stocked();
    let (pilot, row, tv, _, _) = read_two(&mut rig);
    let hull = ObjId(row.hull);
    let mut record = record_of(&mut rig.tw, tv).expect("the record");
    record.charge = Charge {
        value_cu: 1,
        at_ic: ic_now(&rig.tw),
        in_bay: false,
    };
    put_record(&mut rig, record);
    let since = record_of(&mut rig.tw, tv)
        .and_then(|record| record.idle_since())
        .expect("idle since");
    let (_, queued) = advance_to(&mut rig, since + 24 * 3_600);
    land_all(&mut rig, &queued);
    let (_, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    assert_eq!(
        projection(&rig.tw, tv).map(|rec| rec.state),
        Some(TvState::Parked)
    );
    let _ = order(&mut rig, pilot, "await tv all");
    let _ = order(&mut rig, pilot, "depart ceres");
    for _ in 0..6 {
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
    }
    assert_eq!(
        order_state(&rig.tw, hull, 1),
        Some(OrderState::Done),
        "parked where it stands"
    );
    assert!(on_ahuna(&rig.tw, hull), "the depart waits for the rig out");
    let log = log_of(&rig.tw, hull);
    assert!(
        log.iter()
            .any(|line| line.contains("Rig 1 is out on Ahuna Flank")),
        "{log:?}"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// The lead's pre-commit round
// ---------------------------------------------------------------------------

/// A `MakeWriter` that collects formatted log lines (`knowing.rs`'s).
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    /// Everything captured, as text.
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

/// A dispatcher writing WARN and up into `capture` (`knowing.rs`'s).
fn capture_dispatch(capture: Capture) -> tracing::Dispatch {
    tracing::Dispatch::new(
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .with_writer(capture)
            .finish(),
    )
}

/// Staff's own inventory write in one transaction (`unloading.rs`'s helper).
fn staff_inventory(
    rig: &mut Rig,
    body: impl FnOnce(
        &mush_inventory::InventoryFamilies,
        &mut mush_core::ActionTxn<'_>,
    ) -> Result<(), mush_inventory::InventoryError>,
) {
    let families = rig
        .tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    let engine = rig.tw.engine_mut();
    let prepared = {
        let mut txn = engine
            .action_transaction(&families.pools)
            .expect("an inventory transaction");
        body(&families, &mut txn).expect("staff's inventory write");
        txn.prepare().expect("prepares")
    };
    let (ops, install) = prepared.into_parts();
    engine.install_prepared_action(install).expect("installs");
    keep(rig, ops);
}

/// **Fill `tv`'s hopper's marker ring with young markers** (`unloading.rs`'s U4 rig): staff seed
/// `MAX_MARKER_RING` tonnes into the hopper and move them out one marked tonne at a time to a
/// scratch endpoint no hull reads — the hopper ends as it began, its ring full at this real second.
fn fill_ring(rig: &mut Rig, tv: ObjId, lot: u64) {
    let vault = hopper(&rig.tw, tv);
    let ring = u64::try_from(mush_inventory::MAX_MARKER_RING).expect("fits");
    seed_pool(rig, vault, "metal_ore", ring, LotId(lot));
    let scratch = EndpointRef(VaultId(9_000_000_011));
    let stamp = rig.tw.world().world_ic_now();
    let wall = rig.tw.world().now();
    for n in 0..ring {
        let id =
            mush_inventory::derived_marker_id(1, &json!({ "sm11.young": n })).expect("a marker id");
        staff_inventory(rig, |families, txn| {
            families
                .transfer_marked(
                    txn,
                    EndpointRef(vault),
                    scratch,
                    &cid("metal_ore"),
                    1,
                    &id,
                    GOD,
                    &stamp,
                    wall,
                )
                .map(|_| ())
        });
    }
}

/// **A full unload log is a wait the plan comes back from by itself** (the lead's pre-commit
/// round, item 1; the judge's first residual): a plan's first recover queues its unload; the
/// queued start lands, then staff fill the hopper's marker ring to its cap of young markers, so
/// the unload's finish is refused `MarkersFull` — and the finish **re-files the queue** after it.
/// With the pilot offline: the refusal is told once, queued for the reconnect with
/// `WHY_MARKERS_QUEUED` (never a park, never the plan's end); the tick reads the ring before it
/// starts the re-filed queue (the reviewer's R1) and submits **nothing** into it — no start over
/// seventy fires (the ring re-read on the queue's back-off), no page, the record unmoved, the
/// line not told a second time; `+tv/1 plan show` names `WAIT_MARKERS`; a hand `+tv/1 unload go`
/// is still held by the live plan (D8). A real day on, the ring has aged: the next re-read
/// starts the unload, which empties the hopper (`UNLOADED` queued), and the plan's next step
/// starts by itself.
#[test]
#[allow(clippy::too_many_lines)]
fn sm11_a_full_unload_log_waits_and_the_plan_goes_on_by_itself() {
    let mut rig = stocked();
    let (pilot, _, tv, a, b) = read_two(&mut rig);
    let _ = file_plan(
        &mut rig,
        pilot,
        tv,
        &format!("+tv/1 plan {},{}", a.addr, b.addr),
    );
    // To the plan's first recover landed: its queued unload stands.
    let _ = work_until(&mut rig, &[tv], 400, &mut |rig| {
        record_of(&mut rig.tw, tv).is_some_and(|record| record.queued.is_some())
    });
    assert!(
        record_of(&mut rig.tw, tv).is_some_and(|record| record.queued.is_some()),
        "the plan's recover queued its unload"
    );
    disconnect(&mut rig.tw, pilot);
    // The tell's fire: the queued unload starts (the ring still has room).
    let (_, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    let unload = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .expect("the queued unload in flight");
    assert_eq!(unload.kind, ActKind::Unload);
    fill_ring(&mut rig, tv, 9_311);
    let vault = hopper(&rig.tw, tv);
    let loaded = pool(&mut rig.tw, vault, "metal_ore");
    assert!(loaded > 0, "the hopper holds the trip's ore");
    // The finish: refused, the queue re-filed after it, the plan live.
    let (_, queued) = advance_to(&mut rig, unload.due_ic);
    assert_eq!(finishes(&rig.tw, &queued).len(), 1);
    land_all(&mut rig, &queued);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(
        record.last.as_ref().map(|last| last.result),
        Some(tacsim::surface::ActResult::Refused {
            code: tacsim::surface::RefusedCode::MarkersFull
        })
    );
    assert!(
        record
            .queued
            .as_ref()
            .is_some_and(|queued| queued.after == unload.ordinal && queued.operator == pilot),
        "the queue is re-filed after the refused unload: {record:?}"
    );
    assert!(record.plan.as_ref().is_some_and(FillPlan::live), "no park");
    let ordinal = record.ordinal;
    // The reviewer's R1: the re-filed queue is held on the full ring — no start is submitted, so
    // none is refused (no generic error line reaches a connected pilot) and nothing pages.
    let capture = Capture::default();
    let mut starts_by_fire: Vec<usize> = Vec::new();
    tracing::dispatcher::with_default(&capture_dispatch(capture.clone()), || {
        for _ in 0..70 {
            let (_, queued) = fire(&mut rig);
            starts_by_fire.push(starts_for(&queued, tv).len());
            land_all(&mut rig, &queued);
        }
    });
    assert!(
        starts_by_fire.iter().all(|n| *n == 0),
        "no start into a full ring: {starts_by_fire:?}"
    );
    assert_eq!(
        capture.count("its queued unload has not started after 8 tries"),
        0,
        "nothing pages: {}",
        capture.text()
    );
    assert_eq!(ordinal_of(&mut rig.tw, tv), ordinal, "nothing landed");
    assert!(
        record_of(&mut rig.tw, tv).is_some_and(|record| record.queued.is_some()),
        "the queue stands"
    );
    let shown = run(&mut rig.tw, pilot, "+tv/1 plan show");
    assert!(shown.contains(WAIT_MARKERS), "the markers wait: {shown}");
    // D8: a live plan still holds the hand unload.
    let (_, queued) = command(&mut rig, pilot, "+tv/1 unload go");
    assert!(queued.is_empty(), "the plan holds the hand unload");
    // A real day on, the ring has aged: the next retry lands and the unload empties the hopper.
    let t = rig.tw.world().now() + mush_inventory::MARKER_RETENTION_SECS;
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    for _ in 0..70 {
        if record_of(&mut rig.tw, tv)
            .and_then(|record| record.act)
            .is_some_and(|act| act.kind == ActKind::Unload)
        {
            break;
        }
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
    }
    let retried = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .expect("the queued unload started again");
    assert_eq!(retried.kind, ActKind::Unload);
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(
        pool(&mut rig.tw, vault, "metal_ore"),
        0,
        "the hopper is empty"
    );
    // The plan's next step starts by itself.
    let landed = work_until(&mut rig, &[tv], 100, &mut |rig| {
        record_of(&mut rig.tw, tv).is_some_and(|record| {
            record.act.as_ref().is_some_and(|act| act.by.is_some())
                || record.run.as_ref().is_some_and(|run| run.by.is_some())
        })
    });
    let record = record_of(&mut rig.tw, tv).expect("the record");
    let by = record
        .act
        .as_ref()
        .map(|act| act.by)
        .or(record.run.as_ref().map(|run| run.by))
        .flatten();
    assert!(
        matches!(by, Some(ActBy::Plan { no: 1, .. })),
        "the plan goes on: {landed:?} {record:?}"
    );
    // Told once, queued for the reconnect: the refusal, then the unload.
    let pending = pending(&rig.tw, pilot);
    println!("markers pending: {pending:#?}");
    let failed = ACT_FAILED
        .replace("{n}", "1")
        .replace("{act}", "unload")
        .replace("{why}", WHY_MARKERS_QUEUED);
    assert_eq!(
        pending.iter().filter(|line| **line == failed).count(),
        1,
        "{pending:#?}"
    );
    let at = pending
        .iter()
        .position(|line| *line == failed)
        .expect("the refusal");
    assert!(
        pending
            .get(at + 1..)
            .is_some_and(|later| later.iter().any(|line| fills(line, UNLOADED))),
        "the unload after it: {pending:#?}"
    );
    no_refused_writes(&rig.tw);
}

/// How [`markers_before_the_start`] fills the ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RingArm {
    /// Before the tell's fire, the pilot connected.
    Online,
    /// Before the tell's fire, the pilot offline.
    Offline,
    /// The pilot offline; the tell's fire submits the queued start (the ring has room), then
    /// staff fill the ring before it lands — the offer's backstop refuses it.
    Backstop,
}

/// **A ring full before the queued start holds the queue, told once** (the reviewer's R1): the
/// common way a ring fills is earlier unloads, so it is full when the recover queues the next one.
/// Staff fill the hopper's marker ring to `MAX_MARKER_RING` young markers after the plan's first
/// recover lands and before the tell's fire. Pinned: no start is submitted while the ring is full
/// (so none is refused, and no generic error line can reach a connected pilot) — seventy fires, the
/// ring re-read on the queue's back-off, nothing paged, the record unmoved; `ACT_FAILED` with
/// `WHY_MARKERS_QUEUED` told exactly once — said to the connected pilot, or queued for the offline
/// one's reconnect; `+tv/1 plan show` reads `WAIT_MARKERS`, never `WAIT_BUSY`; then the ring ages
/// a real day, the unload lands and empties the hopper, and the plan's next step starts by itself.
/// The `Backstop` arm keeps the offer's own check honest: a start the tick submitted while the ring
/// had room, landed after staff filled it, is refused `unload_markers` (nothing lands), and the
/// tick then holds the queue the same way, telling the line once.
#[allow(clippy::too_many_lines)]
fn markers_before_the_start(arm: RingArm) {
    let mut rig = stocked();
    let (pilot, _, tv, a, b) = read_two(&mut rig);
    let _ = file_plan(
        &mut rig,
        pilot,
        tv,
        &format!("+tv/1 plan {},{}", a.addr, b.addr),
    );
    let _ = work_until(&mut rig, &[tv], 400, &mut |rig| {
        record_of(&mut rig.tw, tv).is_some_and(|record| record.queued.is_some())
    });
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert!(
        record.queued.is_some() && record.act.is_none() && record.run.is_none(),
        "the plan's recover queued its unload: {record:?}"
    );
    if arm != RingArm::Online {
        disconnect(&mut rig.tw, pilot);
    }
    let mut heard = String::new();
    if arm == RingArm::Backstop {
        // The tell's fire submits the start into a ring with room; the ring fills under it.
        let (events, queued) = fire(&mut rig);
        heard.push_str(&said(&rig.tw, &events, pilot));
        let starts = starts_for(&queued, tv);
        assert_eq!(starts.len(), 1, "the queued start: {queued:?}");
        assert_eq!(starts[0].parameters["how"], json!("queued"));
        fill_ring(&mut rig, tv, 9_411);
        let refused = land(&mut rig, &starts[0]).expect_err("the offer's backstop");
        assert!(
            refused.contains("the rig's unload log is full; try again later"),
            "unload_markers at the offer: {refused}"
        );
        let rest: Vec<AuthoritativeSubmit> = queued
            .into_iter()
            .filter(|submit| submit.key != starts[0].key)
            .collect();
        land_all(&mut rig, &rest);
    } else {
        fill_ring(&mut rig, tv, 9_411);
    }
    let vault = hopper(&rig.tw, tv);
    assert!(
        pool(&mut rig.tw, vault, "metal_ore") > 0,
        "the hopper holds the trip's ore"
    );
    let ordinal = ordinal_of(&mut rig.tw, tv);
    let capture = Capture::default();
    let mut starts_by_fire: Vec<usize> = Vec::new();
    tracing::dispatcher::with_default(&capture_dispatch(capture.clone()), || {
        for _ in 0..70 {
            let (events, queued) = fire(&mut rig);
            heard.push_str(&said(&rig.tw, &events, pilot));
            heard.push('\n');
            starts_by_fire.push(starts_for(&queued, tv).len());
            land_all(&mut rig, &queued);
        }
    });
    assert!(
        starts_by_fire.iter().all(|n| *n == 0),
        "no start into a full ring: {starts_by_fire:?}"
    );
    assert_eq!(
        capture.count("its queued unload has not started after 8 tries"),
        0,
        "nothing pages: {}",
        capture.text()
    );
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(record.ordinal, ordinal, "nothing landed");
    assert!(record.queued.is_some(), "the queue stands: {record:?}");
    assert!(record.plan.as_ref().is_some_and(FillPlan::live), "no park");
    let failed = ACT_FAILED
        .replace("{n}", "1")
        .replace("{act}", "unload")
        .replace("{why}", WHY_MARKERS_QUEUED);
    if arm == RingArm::Online {
        assert_eq!(count(&heard, &failed), 1, "said once: {heard}");
        assert_eq!(
            count(
                &heard,
                &RECOVERED_UNLOADING
                    .replace("{n}", "1")
                    .replace("{name}", HULL)
            ),
            1,
            "the recover's own line first: {heard}"
        );
        assert!(
            !heard.contains("did not go through"),
            "no generic refusal: {heard}"
        );
    } else {
        let queued_lines = pending(&rig.tw, pilot);
        assert_eq!(
            queued_lines.iter().filter(|line| **line == failed).count(),
            1,
            "queued once for the reconnect: {queued_lines:#?}"
        );
    }
    let shown = run(&mut rig.tw, pilot, "+tv/1 plan show");
    assert!(shown.contains(WAIT_MARKERS), "the markers wait: {shown}");
    assert!(!shown.contains(WAIT_BUSY), "never the busy wait: {shown}");
    // A real day on, the ring has aged: the queue's next re-read starts the unload.
    let t = rig.tw.world().now() + mush_inventory::MARKER_RETENTION_SECS;
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    for _ in 0..70 {
        if record_of(&mut rig.tw, tv)
            .and_then(|record| record.act)
            .is_some_and(|act| act.kind == ActKind::Unload)
        {
            break;
        }
        let (events, queued) = fire(&mut rig);
        heard.push_str(&said(&rig.tw, &events, pilot));
        land_all(&mut rig, &queued);
    }
    let started = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .expect("the queued unload started");
    assert_eq!(started.kind, ActKind::Unload);
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(
        pool(&mut rig.tw, vault, "metal_ore"),
        0,
        "the hopper is empty"
    );
    let landed = work_until(&mut rig, &[tv], 100, &mut |rig| {
        record_of(&mut rig.tw, tv).is_some_and(|record| {
            record.act.as_ref().is_some_and(|act| act.by.is_some())
                || record.run.as_ref().is_some_and(|run| run.by.is_some())
        })
    });
    let record = record_of(&mut rig.tw, tv).expect("the record");
    let by = record
        .act
        .as_ref()
        .map(|act| act.by)
        .or(record.run.as_ref().map(|run| run.by))
        .flatten();
    assert!(
        matches!(by, Some(ActBy::Plan { no: 1, .. })),
        "the plan goes on: {landed:?} {record:?}"
    );
    // Still told once, whichever way it was delivered.
    if arm == RingArm::Online {
        assert_eq!(count(&heard, &failed), 1, "said once: {heard}");
    } else {
        let queued_lines = pending(&rig.tw, pilot);
        assert_eq!(
            queued_lines.iter().filter(|line| **line == failed).count(),
            1,
            "queued once: {queued_lines:#?}"
        );
        let at = queued_lines
            .iter()
            .position(|line| *line == failed)
            .expect("the refusal");
        assert!(
            queued_lines
                .get(at + 1..)
                .is_some_and(|later| later.iter().any(|line| fills(line, UNLOADED))),
            "the unload after it: {queued_lines:#?}"
        );
    }
    no_refused_writes(&rig.tw);
}

/// [`markers_before_the_start`], the pilot connected.
#[test]
fn sm11_a_ring_full_before_the_queued_start_is_said_once_online() {
    markers_before_the_start(RingArm::Online);
}

/// [`markers_before_the_start`], the pilot offline.
#[test]
fn sm11_a_ring_full_before_the_queued_start_is_queued_once_offline() {
    markers_before_the_start(RingArm::Offline);
}

/// [`markers_before_the_start`], the ring filled under a submitted start: the offer's backstop.
#[test]
fn sm11_a_ring_filled_under_a_submitted_start_is_refused_at_the_offer() {
    markers_before_the_start(RingArm::Backstop);
}

/// **A clear typed again across a queued unload's landing is one intent** (the lead's pre-commit
/// round, item 2; the judge's correctness/F6): with the plan's first recover landed and its queued
/// unload standing, `+tv/1 plan clear` is typed (not yet landed); the queued unload's start lands
/// (the record's ordinal moves, the plan's `seq` does not); the clear typed again carries the same
/// key AND the same parameters (`ordinal` is `PLAN_CLEAR_ORDINAL`, never the record's) — so the
/// first lands and the second is the ledger's HIT: no idempotency conflict, no second write.
#[test]
fn sm11_a_clear_retyped_across_a_queued_unload_is_one_intent() {
    let mut rig = stocked();
    let (pilot, _, tv, a, b) = read_two(&mut rig);
    let _ = file_plan(
        &mut rig,
        pilot,
        tv,
        &format!("+tv/1 plan {},{}", a.addr, b.addr),
    );
    let _ = work_until(&mut rig, &[tv], 400, &mut |rig| {
        record_of(&mut rig.tw, tv).is_some_and(|record| record.queued.is_some())
    });
    let before = record_of(&mut rig.tw, tv).expect("the record");
    let seq = before.plan.as_ref().expect("the plan").seq;
    let (_, first) = command(&mut rig, pilot, "+tv/1 plan clear");
    assert_eq!(first.len(), 1);
    assert_eq!(act_word(&first[0]), "plan_clear");
    assert_eq!(
        first[0].parameters["ordinal"],
        json!(tacsim::surface::act::PLAN_CLEAR_ORDINAL)
    );
    // The queued unload's start lands under the typed clear.
    let (_, queued) = fire(&mut rig);
    let starts = starts_for(&queued, tv);
    assert_eq!(starts.len(), 1, "{queued:?}");
    assert_eq!(starts[0].parameters["how"], json!("queued"));
    land_all(&mut rig, &queued);
    let moved = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(moved.ordinal, before.ordinal + 1, "the ordinal moved");
    assert_eq!(
        moved.plan.as_ref().map(|plan| plan.seq),
        Some(seq),
        "the plan's seq did not"
    );
    let (_, second) = command(&mut rig, pilot, "+tv/1 plan clear");
    assert_eq!(second.len(), 1);
    assert_eq!(first[0].key, second[0].key, "the same key");
    assert_eq!(
        first[0].parameters, second[0].parameters,
        "the same parameters"
    );
    land_all(&mut rig, &first);
    assert_eq!(
        land(&mut rig, &second[0]),
        Err(HIT.to_owned()),
        "a replay, not a conflict"
    );
    let plan = plan_of(&mut rig.tw, tv).expect("the plan");
    assert!(
        matches!(
            plan.state,
            FillState::Ended {
                why: PlanEnd::Cleared,
                ..
            }
        ),
        "{plan:?}"
    );
    assert_eq!(
        plan.notes
            .iter()
            .filter(|note| matches!(note.note, PlanNoteKind::Ended { .. }))
            .count(),
        1,
        "one write: {plan:?}"
    );
    no_refused_writes(&rig.tw);
}

/// **The bound precedes every wait: a plan past its deadline ends from the `Ashore` wait** (the
/// lead's pre-commit round, item 3; the judge's correctness/F5): a one-pocket plan filed with the
/// rig on the ground, its filing staff-shifted so its 288 IC hours run out ten IC minutes on; staff
/// move the pilot out of the hull's airlock space into its bay (not aboard by `voyage::aboard`'s
/// rule, `excursions.rs`'s gate test): the plan waits `Ashore` (told once to the connected pilot,
/// nothing submitted); at the deadline the tick submits the plan's `plan_end` as the pilot — still
/// not aboard — and it lands: the plan ends `Bound` where the rig stands (no recover: the way home
/// waits for the pilot, or the idle return), told once.
#[test]
#[allow(clippy::too_many_lines)]
fn sm11_the_bound_ends_a_plan_waiting_ashore() {
    let mut rig = stocked();
    let (pilot, row, tv, a, _) = read_two(&mut rig);
    let _ = file_plan(&mut rig, pilot, tv, &format!("+tv/1 plan {}", a.addr));
    let mut record = record_of(&mut rig.tw, tv).expect("the record");
    let deadline = ic_now(&rig.tw) + 600;
    if let Some(plan) = record.plan.as_mut() {
        plan.deadline_ic = deadline;
        plan.filed_ic = deadline - tacsim::surface::plan::PLAN_BOUND_IC_SECS;
    }
    put_record(&mut rig, record);
    let bay = ObjId(row.rooms[&cid("bay")]);
    support::with_ctx(&mut rig.tw, |ctx| {
        mush_sim::place_interior(ctx, pilot, bay, None)
    })
    .expect("staff move the pilot into the bay");
    let mut heard = String::new();
    let mut ended_at: Option<i64> = None;
    for _ in 0..200 {
        let (events, queued) = fire(&mut rig);
        heard.push_str(&said(&rig.tw, &events, pilot));
        heard.push('\n');
        let starts = starts_for(&queued, tv);
        if ic_now(&rig.tw) < deadline {
            assert!(starts.is_empty(), "nothing starts ashore: {starts:?}");
        }
        for start in &starts {
            assert_eq!(act_word(start), "plan_end", "only the end: {start:?}");
            assert_eq!(start.actor, pilot, "as the author");
        }
        land_all(&mut rig, &queued);
        if ended_at.is_none() && plan_of(&mut rig.tw, tv).is_some_and(|plan| !plan.live()) {
            ended_at = Some(ic_now(&rig.tw));
        }
        if ended_at.is_some_and(|at| ic_now(&rig.tw) > at + 12) {
            break;
        }
    }
    let ended_at = ended_at.expect("the plan ended");
    assert!(ended_at >= deadline, "not before its bound");
    assert!(
        ended_at <= deadline + 12,
        "at its bound, not a cached wait later"
    );
    let plan = plan_of(&mut rig.tw, tv).expect("the plan");
    assert!(
        matches!(
            plan.state,
            FillState::Ended {
                why: PlanEnd::Bound,
                ..
            }
        ),
        "{plan:?}"
    );
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert!(
        record.excursion.is_some() && record.act.is_none(),
        "the rig stands where it was: {record:?}"
    );
    let waits = PLAN_WAITS
        .replace("{n}", "1")
        .replace("{why}", &WAIT_ASHORE.replace("{name}", HULL));
    assert_eq!(count(&heard, &waits), 1, "the wait told once: {heard}");
    let ended: Vec<&str> = heard
        .lines()
        .filter(|line| fills(line, PLAN_ENDED) && line.contains(END_BOUND))
        .collect();
    assert_eq!(ended.len(), 1, "the end told once: {heard}");
    no_refused_writes(&rig.tw);
}

/// **The author shows and clears their plan off the helm** (the lead's pre-commit round, item 3;
/// D22 "stays clearable"; the judge's correctness/F5): with a plan live, staff cancel the pilot's
/// charter — the helm is gone, the pilot still aboard and still the rig's lessee. `+tv/1 plan show`
/// reads the plan with its helm wait; `+tv/1 plan clear` is `PLAN_CLEARED` with one `plan_clear`
/// keyed `…/clear/<seq>`, which lands: the plan ends `Cleared`.
#[test]
fn sm11_the_author_clears_their_plan_off_the_helm() {
    let mut rig = stocked();
    let (pilot, _, tv, a, b) = read_two(&mut rig);
    let _ = file_plan(
        &mut rig,
        pilot,
        tv,
        &format!("+tv/1 plan {},{}", a.addr, b.addr),
    );
    let charter = tacsim::port::live_charter(rig.tw.world(), pilot).expect("a charter");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@contract/cancel #{}", charter.contract.0),
    );
    assert_eq!(
        tacsim::port::live_charter(rig.tw.world(), pilot),
        None,
        "the charter is cancelled: {out}"
    );
    assert!(
        rigs_of(&rig.tw, pilot).contains(&tv),
        "the rig's lease stands"
    );
    let show = run(&mut rig.tw, pilot, "+tv/1 plan show");
    assert!(
        show.contains(&WAIT_HELM.replace("{name}", HULL)),
        "the helm's wait: {show}"
    );
    let seq = plan_of(&mut rig.tw, tv).expect("the plan").seq;
    let (out, queued) = command(&mut rig, pilot, "+tv/1 plan clear");
    assert_eq!(
        out.lines().next(),
        Some(
            PLAN_CLEARED
                .replace("{n}", "1")
                .replace("{no}", "1")
                .as_str()
        ),
        "{out}"
    );
    assert_eq!(queued.len(), 1);
    assert_eq!(act_word(&queued[0]), "plan_clear");
    assert_eq!(
        queued[0].key.as_str(),
        key_text(tacsim::surface::act::plan_clear_key(tv, 1, seq))
    );
    land_all(&mut rig, &queued);
    let plan = plan_of(&mut rig.tw, tv).expect("the plan");
    assert!(
        matches!(
            plan.state,
            FillState::Ended {
                why: PlanEnd::Cleared,
                ..
            }
        ),
        "{plan:?}"
    );
    no_refused_writes(&rig.tw);
}

/// **The bound ends a plan waiting off the helm, as its author** (the reviewer's R2; the lead's
/// pre-commit round, item 3; D22): with a plan live and its filing staff-shifted so its 288 IC hours
/// run out ten IC minutes on, staff cancel the pilot's charter — the helm is gone (so the rig is in
/// no `operator_rigs`, and only the off-helm offer carries it), the pilot still the rig's lessee.
/// Pinned: the `Helm` wait is told once to the connected pilot and nothing starts before the
/// deadline; then exactly one `plan_end` is submitted as the author and lands within one fire of
/// the deadline — through the off-helm offer's `plan` variant; the plan is `Ended { Bound }`;
/// `PLAN_ENDED` with `END_BOUND` is told once. With `aboard` false staff also move the pilot from
/// the airlock into the hull's bay (not aboard by `voyage::aboard`'s rule): the off-helm offer then
/// fixes the plan's variants with `controls == false` — a `plan_clear` the author typed while
/// still aboard, landed after the move, is refused and the plan stays live — while its `plan`
/// variant is still offered, so the `Bound` end lands as the author with no helm and not aboard.
#[allow(clippy::too_many_lines)]
fn bound_off_the_helm(aboard: bool) {
    let mut rig = stocked();
    let (pilot, row, tv, a, b) = read_two(&mut rig);
    let _ = file_plan(
        &mut rig,
        pilot,
        tv,
        &format!("+tv/1 plan {},{}", a.addr, b.addr),
    );
    let mut record = record_of(&mut rig.tw, tv).expect("the record");
    let deadline = ic_now(&rig.tw) + 600;
    if let Some(plan) = record.plan.as_mut() {
        plan.deadline_ic = deadline;
        plan.filed_ic = deadline - tacsim::surface::plan::PLAN_BOUND_IC_SECS;
    }
    put_record(&mut rig, record);
    let charter = tacsim::port::live_charter(rig.tw.world(), pilot).expect("a charter");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@contract/cancel #{}", charter.contract.0),
    );
    assert_eq!(
        tacsim::port::live_charter(rig.tw.world(), pilot),
        None,
        "the charter is cancelled: {out}"
    );
    assert!(
        rigs_of(&rig.tw, pilot).contains(&tv),
        "the rig's lease stands"
    );
    if !aboard {
        // The clear typed while still aboard (the off-helm offer's controls were then true).
        let (out, clear) = command(&mut rig, pilot, "+tv/1 plan clear");
        assert_eq!(clear.len(), 1, "{out}");
        assert_eq!(act_word(&clear[0]), "plan_clear");
        let bay = ObjId(row.rooms[&cid("bay")]);
        support::with_ctx(&mut rig.tw, |ctx| {
            mush_sim::place_interior(ctx, pilot, bay, None)
        })
        .expect("staff move the pilot into the bay");
        let refused = land(&mut rig, &clear[0]).expect_err("the clear is not offered off the hull");
        assert!(
            refused.contains("the operator is not aboard the hull"),
            "refused not_aboard by the off-helm offer's controls: {refused}"
        );
        assert!(
            plan_of(&mut rig.tw, tv).is_some_and(|plan| plan.live()),
            "the plan stays live"
        );
    }
    // One fire's IC span (each `fire` moves the real clock 6 s).
    let one_fire = 6 * i64::from(world_rate(&rig.tw).max(1));
    let mut heard = String::new();
    let mut ends: Vec<i64> = Vec::new();
    let mut ended_at: Option<i64> = None;
    for _ in 0..200 {
        let (events, queued) = fire(&mut rig);
        heard.push_str(&said(&rig.tw, &events, pilot));
        heard.push('\n');
        let starts = starts_for(&queued, tv);
        if ic_now(&rig.tw) < deadline {
            assert!(starts.is_empty(), "nothing starts off the helm: {starts:?}");
        }
        for start in &starts {
            assert_eq!(act_word(start), "plan_end", "only the end: {start:?}");
            assert_eq!(start.actor, pilot, "as the author");
            ends.push(ic_now(&rig.tw));
        }
        land_all(&mut rig, &queued);
        if ended_at.is_none() && plan_of(&mut rig.tw, tv).is_some_and(|plan| !plan.live()) {
            ended_at = Some(ic_now(&rig.tw));
        }
        if ended_at.is_some_and(|at| ic_now(&rig.tw) > at + one_fire) {
            break;
        }
    }
    assert_eq!(ends.len(), 1, "exactly one plan_end: {ends:?}");
    let ended_at = ended_at.expect("the plan ended");
    assert_eq!(
        ends[0], ended_at,
        "the one end submitted is the one that landed"
    );
    assert!(ended_at >= deadline, "not before its bound");
    assert!(
        ended_at <= deadline + one_fire,
        "within one fire of its bound: {ended_at} vs {deadline}"
    );
    let plan = plan_of(&mut rig.tw, tv).expect("the plan");
    assert!(
        matches!(
            plan.state,
            FillState::Ended {
                why: PlanEnd::Bound,
                ..
            }
        ),
        "{plan:?}"
    );
    let waits = PLAN_WAITS
        .replace("{n}", "1")
        .replace("{why}", &WAIT_HELM.replace("{name}", HULL));
    assert_eq!(
        count(&heard, &waits),
        1,
        "the helm's wait told once: {heard}"
    );
    let ended: Vec<&str> = heard
        .lines()
        .filter(|line| fills(line, PLAN_ENDED) && line.contains(END_BOUND))
        .collect();
    assert_eq!(ended.len(), 1, "the end told once: {heard}");
    no_refused_writes(&rig.tw);
}

/// [`bound_off_the_helm`], the author still aboard.
#[test]
fn sm11_the_bound_ends_a_plan_waiting_off_the_helm() {
    bound_off_the_helm(true);
}

/// [`bound_off_the_helm`], the author neither at the helm nor aboard.
#[test]
fn sm11_the_bound_ends_a_plan_off_the_helm_and_off_the_hull() {
    bound_off_the_helm(false);
}

/// **An idle return refused eight times pages once** (the lead's pre-commit round, item 4; the
/// judge's correctness/F9): a rig a day idle on the ground, the pilot offline; every idle recover
/// the tick submits is dropped before it lands (a start that does not land); after eight tries the
/// tick pages once — and only once over the fires after — as the plan's and the queue's arms do.
#[test]
fn sm11_an_idle_return_refused_eight_times_pages_once() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, 1, tv);
    disconnect(&mut rig.tw, pilot);
    let since = record_of(&mut rig.tw, tv)
        .and_then(|record| record.idle_since())
        .expect("idle since");
    let capture = Capture::default();
    let mut tries = 0_usize;
    tracing::dispatcher::with_default(&capture_dispatch(capture.clone()), || {
        let (_, queued) = advance_to(&mut rig, since + 24 * 3_600);
        tries += starts_for(&queued, tv).len();
        for _ in 0..20 {
            let (_, queued) = fire(&mut rig);
            let starts = starts_for(&queued, tv);
            for start in &starts {
                assert_eq!(start.parameters["by"], json!("idle"));
            }
            tries += starts.len();
        }
    });
    assert_eq!(tries, 8, "eight tries, then the back-off");
    assert_eq!(
        capture.count("its idle return has not started after 8 tries"),
        1,
        "one page: {}",
        capture.text()
    );
    assert!(
        record_of(&mut rig.tw, tv).is_some_and(|record| record.excursion.is_some()),
        "the rig is still out"
    );
}
