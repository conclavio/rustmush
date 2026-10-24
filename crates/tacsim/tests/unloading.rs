//! **Doc 20 cut SM.8 acceptance — unloading** (doc 20 §1 beats 9 and 12, §4.1, §4.3, §6, §7.1,
//! §7.2, §8, §19 items 1 and 3; D-20-7, D-20-10, D-20-11; the SM.8 brief's acceptance A2–A4 and its
//! tests U1–U12 in 3.12).
//!
//! A rig home in its Dray-P's bay with ore in its hopper unloads into the hold: a `Done` recover
//! files the unload on the rig's record (a durable queue) and the `tacsim.surface` tick starts it
//! as the operator on its next visit, keyed `tacsim.surface/{tv}/{ordinal}/queued`; or the operator
//! files one by hand with `+tv/<n> unload [<t>] go`. The finish moves the hopper's goods into the
//! hold by one inventory-owned marked transfer, one `inventory.transferred` marker a good. The
//! departure names the unload on a site's pad and staff at a port; the hopper never sells.
//!
//! Composed on the composition root (`mining.rs`'s rig, copied: the per-file precedent), **booted
//! keyed** with the tacsim suites' fixed geology key; Ahuna Flank opened once a world through
//! `support::sites`. Every act goes through the real verb, the real land path and the real tick;
//! hoppers and holds are seeded through the inventory's own path (`seed_pool`) where a test is not
//! about mining, and rigs are put in place by the tick-batch placement seam (`mush_sim::place`)
//! a test may call directly. Pockets are chosen from the keyed truth (`picks`) and printed
//! (`--nocapture`) for the report. Landings keep a ledger of every landed submission's identity, so
//! a redelivered submission is the ledger's HIT as in the world task; `resolve`/`prepare` split a
//! landing in two for the races.

mod support;

#[path = "support/geology_keyed.rs"]
mod geology_keyed;

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
use mush_market::CapacityProvider;
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde_json::{Value, json};
use smol_str::SmolStr;
use tacsim::definitions::content::{DRAY_P, LANE_JSON, LANE_PACK};
use tacsim::definitions::tools::{BatteryDef, ToolDef, tools_table};
use tacsim::pack::{PackEntry, RulesPack, load_pack};
use tacsim::port::licence::licence_key;
use tacsim::port::tv::{
    ACT_FAILED, DUMP_EMPTY, MINE_CLAIMED, MINE_FORFEIT, QUEUED_LINE, RECOVER_THEN,
    RECOVER_THEN_PART, RECOVERED, RECOVERED_UNLOADING, TV_NOT_ABOARD, UNLOAD_NO_FIT,
    UNLOAD_NO_ROOM, UNLOAD_NO_STOCK, UNLOAD_NOT_SITE, UNLOAD_OUT, UNLOAD_PREVIEW, UNLOAD_STAYS,
    UNLOAD_UNDER_WAY, UNLOAD_USAGE, UNLOADED, UNLOADED_NONE, ore_words,
};
use tacsim::port::voyage::{HOPPER_LOADED, HOPPER_LOADED_AWAY, TV_BUSY};
use tacsim::port::{HullRow, PortHandles, PortMap, PortMapState, Ticket, TicketId, room_key};
use tacsim::surface::geology::{OreFamily, Word};
use tacsim::surface::ground::{SiteGeologySpec, SiteGround, site_ground_sync};
use tacsim::surface::reading::Footing;
use tacsim::surface::{
    ActKind, ActParams, ActResult, Charge, GeologyBoot, GeologyState, Queued, RefusedCode, RigKey,
    RigRecord, Run, SURFACE_ACT_ACTION, SURFACE_DONE_ACTION, UNLOAD_S, UnloadHow, act_key,
    charge_at, queued_key,
};
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

/// **Past the ledger**: a submission resolved and prepared even though its identity already
/// landed — what the prepare's own guard sees if the HIT were ever missed.
fn land_past_the_ledger(rig: &mut Rig, submit: &AuthoritativeSubmit) -> Result<(), String> {
    let resolved = resolve(rig, submit)?;
    prepare(rig, &resolved).map(|_| ())
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

/// The surface finishes a fire queued.
fn finishes(queued: &[AuthoritativeSubmit]) -> Vec<&AuthoritativeSubmit> {
    queued
        .iter()
        .filter(|submit| submit.action.key().id.as_str() == SURFACE_DONE_ACTION)
        .collect()
}

/// The surface starts a fire queued.
fn starts(queued: &[AuthoritativeSubmit]) -> Vec<&AuthoritativeSubmit> {
    queued
        .iter()
        .filter(|submit| submit.action.key().id.as_str() == SURFACE_ACT_ACTION)
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

/// A Dray-P docked at Ceres, its pilot aboard, one rig leased (U4, U10, U12: the port).
fn at_ceres(rig: &mut Rig, who: &str, seed: i64, name: &str) -> (ObjId, HullRow, ObjId) {
    let (pilot, row) = prospector(rig, who, seed, name);
    lease_one(rig, pilot);
    board(rig, pilot);
    let tv = rigs_of(&rig.tw, pilot)[0];
    (pilot, row, tv)
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

/// Whether an `inventory.transferred` applied marker stands under `id`.
fn applied(tw: &mut TestWorld, id: &SmolStr) -> bool {
    let families = tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    typed(tw, &families.applied, id).is_some()
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
    assert_eq!(finishes(&queued).len(), 1, "one finish: {queued:?}");
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
    assert_eq!(finishes(&queued).len(), 1, "one finish at the cycle's due");
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

/// Staff place `tv` `Direct` on Ahuna's domain at site cell `(x, y)`'s centre, facing south.
fn to_cell(rig: &mut Rig, tv: ObjId, (x, y): (i64, i64)) {
    let domain = mush_sim::DomainId(port_map(&rig.tw, AHUNA).domain.expect("a domain"));
    let placed = support::with_ctx(&mut rig.tw, |ctx| {
        let posture = mush_sim::data::passive_rules_value(serde_json::json!({}))
            .map_err(|e| e.to_string())?;
        mush_sim::place(
            ctx,
            tv,
            domain,
            mush_sim::data::Pose {
                x_cm: x * 1_000 + 500,
                y_cm: y * 1_000 + 500,
                z_cm: 0,
                heading_mdeg: 180_000,
                posture,
            },
        )
    });
    placed.expect("staff place the rig");
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

/// **A load in `load_words`' words** (`"{q} t of {label}"` per good, goods in id order, joined as
/// a list — `a`, `a and b`, `a, b and c`: the lead's pre-commit round, item 5, since goods v3's
/// labels carry commas), the labels read from the store's goods table — never a literal.
fn words(tw: &TestWorld, load: &[(&str, u64)]) -> String {
    let table = store(tw).goods().expect("the goods table");
    let items: Vec<String> = load
        .iter()
        .map(|(good, qty)| {
            format!(
                "{qty} t of {}",
                table.good(&cid(good)).expect("a good").label
            )
        })
        .collect();
    match items.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

/// Seed `qty` of `good` into `tv`'s hopper.
fn fill(rig: &mut Rig, tv: ObjId, good: &str, qty: u64, lot: u64) {
    let vault = hopper(&rig.tw, tv);
    seed_pool(rig, vault, good, qty, LotId(lot));
}

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

/// **A forged start** for `tv` at `ordinal` — what a client could submit by hand — with the
/// unload's `extra` parameters, keyed `key`.
fn forged(
    tw: &TestWorld,
    who: ObjId,
    tv: ObjId,
    ordinal: u32,
    extra: &Value,
    key: Option<IdempotencyKeyText>,
) -> AuthoritativeSubmit {
    let mut parameters = json!({ "tv": tv.0.to_string(), "act": "unload", "ordinal": ordinal });
    if let (Some(map), Some(more)) = (parameters.as_object_mut(), extra.as_object()) {
        map.extend(more.clone());
    }
    AuthoritativeSubmit {
        actor: who,
        action: handles(tw).surface_act.expect("the start is registered"),
        subject: PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters,
        key: key.expect("the key forms"),
    }
}

/// The rig's record's ordinal (0 with none).
fn ordinal_of(tw: &mut TestWorld, tv: ObjId) -> u32 {
    record_of(tw, tv).map_or(0, |record| record.ordinal)
}

/// **Recover `tv` and land its finish** — `+tv/1 recover go`, then to its due with the finish
/// landed — stopping before the fire that tells it; the recover's ordinal and due.
fn recover_landed(rig: &mut Rig, pilot: ObjId, slot: u8, tv: ObjId) -> (u32, i64) {
    let _ = act_go(rig, pilot, &format!("+tv/{slot} recover go"));
    let act = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .expect("the recover in flight");
    let (_, queued) = advance_to(rig, act.due_ic);
    assert_eq!(finishes(&queued).len(), 1, "the recover's finish");
    land_all(rig, &queued);
    (act.ordinal, act.due_ic)
}

// ---------------------------------------------------------------------------
// U1. The recover files the unload
// ---------------------------------------------------------------------------

/// **U1 — the recover files the unload and the hopper empties** (lead ruling 1; §1 beat 9): a rig
/// on the ground with ore from a real run (`FAIR7`), the hold seeded with 10 t: `+tv/1 recover`
/// previews the way home and `RECOVER_THEN` exactly — the load and the room after computed from
/// the hull's compartment — and queues nothing; `go` → one start; at its due the finish lands
/// `Done` and the record holds a queued unload after the recover's ordinal, from the excursion it
/// closed; the next fire tells exactly `RECOVERED_UNLOADING` and queues exactly one
/// `tacsim_surface_act` as the operator keyed `tacsim.surface/{tv}/{ordinal + 1}/queued` with `act:
/// unload, how: queued` and no `qty`; landed, the act is a queued unload of the hopper's goods due
/// 600 IC s on and the queue is gone; at its due every tonne moves (hopper 0, hold + mined) with one
/// `inventory.transferred` marker under `unload::marker`'s id; the next fire tells exactly
/// `UNLOADED`; `+ship/plan ceres` names no loaded hopper.
#[test]
#[allow(clippy::too_many_lines)]
fn sm8_recover_files_the_unload_and_the_hopper_empties() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let all = picks(&mut rig.tw);
    let fair = fair7(&all);
    launched(&mut rig, pilot, 1, tv);
    let hold = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, hold, "metal_ore", 10, LotId(8_801));
    let t = mined(&mut rig, pilot, 1, tv, &fair);
    assert!(t > 0, "the run lifted ore");
    let hop = hopper(&rig.tw, tv);
    // The preview: the way home, then the unload and the hold's room after.
    let (out, queued) = command(&mut rig, pilot, "+tv/1 recover");
    assert!(queued.is_empty(), "a preview queues nothing");
    let then = RECOVER_THEN
        .replace("{load}", &words(&rig.tw, &[("metal_ore", t)]))
        .replace("{room}", &(ore_limit_t(&rig.tw) - 10 - t).to_string());
    assert!(
        out.trim().ends_with(&format!(
            "; hopper {t} of 12 t{then}. +tv/1 recover go brings it in."
        )),
        "{out}\nwanted: …{then}"
    );
    let excursion = record_of(&mut rig.tw, tv)
        .and_then(|record| record.excursion)
        .map(|open| open.id)
        .expect("the excursion is open");
    let (recover, due) = recover_landed(&mut rig, pilot, 1, tv);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(
        record.last.as_ref().map(|l| (l.kind, l.result)),
        Some((ActKind::Recover, ActResult::Done))
    );
    assert!(record.excursion.is_none(), "the excursion closed");
    let queued_unload = record.queued.clone().expect("the unload is queued");
    assert_eq!(
        (
            queued_unload.kind,
            queued_unload.operator,
            queued_unload.slot,
            queued_unload.after,
            queued_unload.at_ic,
            queued_unload.excursion
        ),
        (ActKind::Unload, pilot, 1, recover, due, Some(excursion))
    );
    assert_eq!(pool(&mut rig.tw, hop, "metal_ore"), t, "nothing moved yet");
    let status = run(&mut rig.tw, pilot, "+tv");
    assert!(status.lines().any(|line| line == QUEUED_LINE), "{status}");
    // The next fire: the tell and the queued start, as the operator.
    let (events, queued) = fire(&mut rig);
    let heard = said(&rig.tw, &events, pilot);
    let back = RECOVERED_UNLOADING
        .replace("{n}", "1")
        .replace("{name}", HULL);
    assert_eq!(count(&heard, &back), 1, "{heard}");
    assert_eq!(
        count(
            &heard,
            &RECOVERED.replace("{n}", "1").replace("{name}", HULL)
        ),
        0
    );
    let filed = starts(&queued);
    assert_eq!(filed.len(), 1, "one queued start: {queued:?}");
    assert_eq!(filed[0].actor, pilot);
    assert_eq!(
        filed[0].key.as_str(),
        format!("tacsim.surface/{}/{}/queued", tv.0, recover + 1)
    );
    assert_eq!(
        filed[0].parameters,
        json!({
            "tv": tv.0.to_string(),
            "act": "unload",
            "ordinal": recover + 1,
            "how": "queued",
        })
    );
    land_all(&mut rig, &queued);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert!(record.queued.is_none(), "the start cleared the queue");
    let act = record.act.clone().expect("the unload in flight");
    assert_eq!(act.kind, ActKind::Unload);
    assert_eq!(act.due_ic, act.started_ic + i64::from(UNLOAD_S));
    assert_eq!(
        act.params,
        ActParams::Unload {
            hopper: hop.0,
            hold: hold.0,
            goods: [(cid("metal_ore"), t)].into_iter().collect(),
            how: UnloadHow::Queued,
            excursion: Some(excursion),
        }
    );
    // The finish at its due: every tonne, one marker.
    let (_, queued) = advance_to(&mut rig, act.due_ic);
    assert_eq!(finishes(&queued).len(), 1);
    land_all(&mut rig, &queued);
    assert_eq!(pool(&mut rig.tw, hop, "metal_ore"), 0);
    assert_eq!(pool(&mut rig.tw, hold, "metal_ore"), 10 + t);
    let marker = tacsim::surface::unload::marker(
        tv,
        Some(excursion),
        act.ordinal,
        &cid("metal_ore"),
        act.due_ic,
    )
    .expect("a marker id");
    assert!(applied(&mut rig.tw, &marker), "the move's marker stands");
    let last = record_of(&mut rig.tw, tv)
        .and_then(|record| record.last)
        .expect("the outcome");
    assert_eq!((last.kind, last.result), (ActKind::Unload, ActResult::Done));
    assert_eq!(last.moved, [(cid("metal_ore"), t)].into_iter().collect());
    assert!(last.stays.is_empty());
    let (events, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    let heard = said(&rig.tw, &events, pilot);
    let unloaded = UNLOADED
        .replace("{n}", "1")
        .replace("{load}", &words(&rig.tw, &[("metal_ore", t)]))
        .replace("{name}", HULL)
        .replace("{hopper}", "empty");
    assert_eq!(count(&heard, &unloaded), 1, "{heard}\nwanted: {unloaded}");
    let plan = run(&mut rig.tw, pilot, "+ship/plan ceres");
    assert!(!plan.contains("hopper holds"), "{plan}");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// U2. An empty hopper queues nothing
// ---------------------------------------------------------------------------

/// **U2 — a recover with an empty hopper queues nothing**: `Done`, no queue, `RECOVERED` told, and
/// no start submitted over the next fires.
#[test]
fn sm8_a_recover_with_an_empty_hopper_queues_nothing() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, pilot, 1, tv);
    let _ = recover_landed(&mut rig, pilot, 1, tv);
    assert!(
        record_of(&mut rig.tw, tv).is_some_and(|record| record.queued.is_none()),
        "nothing queued"
    );
    let mut heard = String::new();
    for _ in 0..4 {
        let (events, queued) = fire(&mut rig);
        heard.push_str(&said(&rig.tw, &events, pilot));
        assert!(starts(&queued).is_empty(), "no unload starts: {queued:?}");
        land_all(&mut rig, &queued);
    }
    assert_eq!(
        count(
            &heard,
            &RECOVERED.replace("{n}", "1").replace("{name}", HULL)
        ),
        1,
        "{heard}"
    );
    assert!(record_of(&mut rig.tw, tv).is_some_and(|record| record.act.is_none()));
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// U3. The hand unload, whole and in part
// ---------------------------------------------------------------------------

/// **U3 — the hand unload, whole and in part** (lead ruling 2; decisions R10, R15, R18): a two-ore
/// hopper in the bay: the bare preview exactly (`UNLOAD_PREVIEW`, both goods in id order), nothing
/// queued; `+tv/1 unload 3` previews 3 t of the first good; its `go` queues a start carrying `how:
/// part, qty: 3` — a forged start for the same order with `how: whole`, or with `qty: 4`, refused
/// `Conflict` with nothing written — and lands moving exactly 3 t of the first good (no stays
/// clause told); `+tv/1 unload 99` past the hopper's 1 t of ice is `UNLOAD_NO_STOCK` naming the
/// stock (the lead's pre-commit round, item 4 — the hold had room for 99 t, so "do not fit" was
/// wrong); `0` and `x` are the usage; the bare `go` moves the rest of both. Then the hold seeded
/// to 1 t of room under a 2 t hopper: `+tv/1 unload 2` is exactly `UNLOAD_NO_FIT` naming the 1 t
/// that fits (the room bound it); and with the hold full, so the first good fits not at all,
/// exactly `UNLOAD_NO_ROOM`. The two-ore preview's load reads "4 t of ice ore, run-of-mine and 8 t
/// of metal ore, run-of-mine" byte for byte (item 5).
#[test]
#[allow(clippy::too_many_lines)]
fn sm8_the_hand_unload_whole_and_in_part() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let (hop, hold) = (hopper(&rig.tw, tv), hold_vault(&rig.tw, &row));
    fill(&mut rig, tv, "ice_ore", 4, 8_811);
    fill(&mut rig, tv, "metal_ore", 8, 8_812);
    let limit = ore_limit_t(&rig.tw);
    let real = format!(", about {} min", UNLOAD_S / 60);
    let preview = |load: &str, hop1: u64, room: u64, arg: &str| {
        UNLOAD_PREVIEW
            .replace("{n}", "1")
            .replace("{mark}", "OWP-TV-01")
            .replace("{load}", load)
            .replace("{name}", HULL)
            .replace("{secs}", &UNLOAD_S.to_string())
            .replace("{real}", &real)
            .replace("{hop0}", "12")
            .replace("{hop1}", &hop1.to_string())
            .replace("{cap}", "12")
            .replace("{room}", &room.to_string())
            .replace("{stays}", "")
            .replace("{arg}", arg)
    };
    let (out, queued) = command(&mut rig, pilot, "+tv/1 unload");
    let both = words(&rig.tw, &[("ice_ore", 4), ("metal_ore", 8)]);
    assert_eq!(
        both, "4 t of ice ore, run-of-mine and 8 t of metal ore, run-of-mine",
        "two goods read as a list, never four comma-joined words"
    );
    assert_eq!(out.trim(), preview(&both, 0, limit - 12, ""));
    assert!(queued.is_empty());
    let (out, queued) = command(&mut rig, pilot, "+tv/1 unload 3");
    assert_eq!(
        out.trim(),
        preview(&words(&rig.tw, &[("ice_ore", 3)]), 9, limit - 3, " 3")
    );
    assert!(queued.is_empty());
    // The part `go`: the start carries how and qty; forged starts for the same order refuse.
    let (out, queued) = command(&mut rig, pilot, "+tv/1 unload 3 go");
    assert!(
        out.starts_with("Rig 1 starts unloading into the hold: done "),
        "{out}"
    );
    assert_eq!(queued.len(), 1);
    let next = ordinal_of(&mut rig.tw, tv) + 1;
    assert_eq!(
        queued[0].parameters,
        json!({ "tv": tv.0.to_string(), "act": "unload", "ordinal": next, "how": "part", "qty": 3 })
    );
    let before = record_of(&mut rig.tw, tv);
    for extra in [
        json!({ "how": "whole" }),
        json!({ "how": "part", "qty": 4 }),
    ] {
        let fake = forged(&rig.tw, pilot, tv, next, &extra, act_key(tv, next));
        let refused = land(&mut rig, &fake).expect_err("a start held to its order");
        assert!(refused.contains("that rig's orders moved on"), "{refused}");
        assert_eq!(record_of(&mut rig.tw, tv), before, "nothing written");
    }
    land_all(&mut rig, &queued);
    let act = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .expect("the unload in flight");
    assert!(matches!(
        &act.params,
        ActParams::Unload { how: UnloadHow::Part, goods, .. }
            if *goods == [(cid("ice_ore"), 3)].into_iter().collect()
    ));
    let heard = finish(&mut rig, tv, pilot);
    let told = UNLOADED
        .replace("{n}", "1")
        .replace("{load}", &words(&rig.tw, &[("ice_ore", 3)]))
        .replace("{name}", HULL)
        .replace("{hopper}", "9 of 12 t");
    assert_eq!(count(&heard, &told), 1, "{heard}\nwanted: {told}");
    assert!(!heard.contains("stays in the hopper"), "{heard}");
    assert_eq!(
        (
            pool(&mut rig.tw, hop, "ice_ore"),
            pool(&mut rig.tw, hop, "metal_ore")
        ),
        (1, 8)
    );
    assert_eq!(pool(&mut rig.tw, hold, "ice_ore"), 3);
    // Past the hopper's stock (the hold has room for far more): the stock is named, never "do not
    // fit"; 0 and a word are the usage.
    let (out, queued) = command(&mut rig, pilot, "+tv/1 unload 99");
    assert_eq!(
        out.trim(),
        UNLOAD_NO_STOCK
            .replace("{n}", "1")
            .replace("{fit}", "1")
            .replace("{ore}", &ore_words("ice_ore"))
    );
    assert_eq!(
        out.trim(),
        "Rig 1's hopper holds only 1 t of ice ore: +tv/1 unload 1, or +tv/1 unload with no number."
    );
    assert!(queued.is_empty());
    for bad in ["+tv/1 unload 0", "+tv/1 unload x"] {
        let (out, queued) = command(&mut rig, pilot, bad);
        assert_eq!(out.trim(), UNLOAD_USAGE.replace("{n}", "1"), "{bad}");
        assert!(queued.is_empty());
    }
    // The bare `go`: the rest of both.
    let (_, queued) = command(&mut rig, pilot, "+tv/1 unload go");
    assert_eq!(
        queued.first().map(|s| s.parameters["how"].clone()),
        Some(json!("whole"))
    );
    land_all(&mut rig, &queued);
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(
        (
            pool(&mut rig.tw, hop, "ice_ore"),
            pool(&mut rig.tw, hop, "metal_ore")
        ),
        (0, 0)
    );
    assert_eq!(
        (
            pool(&mut rig.tw, hold, "ice_ore"),
            pool(&mut rig.tw, hold, "metal_ore")
        ),
        (4, 8)
    );
    // The hold's room 1 t under a 2 t hopper: the room binds — UNLOAD_NO_FIT names the 1 t.
    seed_pool(&mut rig, hold, "metal_ore", limit - 13, LotId(8_813));
    fill(&mut rig, tv, "metal_ore", 2, 8_814);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 unload 2");
    assert_eq!(
        out.trim(),
        UNLOAD_NO_FIT
            .replace("{q}", "2")
            .replace("{n}", "1")
            .replace("{fit}", "1")
            .replace("{ore}", &ore_words("metal_ore"))
    );
    assert!(queued.is_empty());
    // The hold full: the first good fits not at all — UNLOAD_NO_ROOM, never "can unload 0 t".
    seed_pool(&mut rig, hold, "metal_ore", 1, LotId(8_815));
    let (out, queued) = command(&mut rig, pilot, "+tv/1 unload 2");
    assert_eq!(
        out.trim(),
        UNLOAD_NO_ROOM
            .replace("{name}", HULL)
            .replace("{n}", "1")
            .replace("{load}", &words(&rig.tw, &[("metal_ore", 2)]))
    );
    assert!(queued.is_empty());
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// U4 (A2). Where an unload cannot work
// ---------------------------------------------------------------------------

/// The start's message for an unload the offer has no plan for (`REFUSAL_CODES`' `unplanned`).
const UNPLANNED: &str = "that act was not planned; preview it again";
/// The start's message when the hopper or the hold's room moved under a planned unload.
const UNLOAD_CHANGED_CODE: &str = "the hopper or the hold's room changed; preview it again";

/// `line` and `line go` both refuse with a line `want` accepts, queue nothing and leave the
/// record as it was; then a forged whole start for the rig's next ordinal is refused at the
/// prepare with nothing written — **"that act was not planned"**: the command wrote no order and
/// the record holds no queue, so the offer has no plan to run (the fix round's conformance/F5:
/// the refusal is named, never any error). The state's own refusal AT THE START is proven where
/// a plan exists: [`queued_refused`] (the offer's queued variant re-running the gates) and U4b (a
/// hand order filed while the state allowed it, then overtaken).
fn refused_everywhere(
    rig: &mut Rig,
    (who, tv): (ObjId, ObjId),
    line: &str,
    want: &dyn Fn(&str) -> bool,
) {
    let before = record_of(&mut rig.tw, tv);
    for typed_line in [line.to_owned(), format!("{line} go")] {
        let (out, queued) = command(rig, who, &typed_line);
        assert!(want(out.trim()), "{typed_line}: {out}");
        assert!(queued.is_empty(), "{typed_line}: nothing queued");
        assert_eq!(
            record_of(&mut rig.tw, tv),
            before,
            "{typed_line}: nothing written"
        );
    }
    let next = ordinal_of(&mut rig.tw, tv) + 1;
    let fake = forged(
        &rig.tw,
        who,
        tv,
        next,
        &json!({ "how": "whole" }),
        act_key(tv, next),
    );
    let refused = land(rig, &fake).expect_err("the prepare refuses a forged start");
    assert!(
        refused.contains("ProviderRejected") && refused.contains(UNPLANNED),
        "{line}: {refused}"
    );
    assert_eq!(
        record_of(&mut rig.tw, tv),
        before,
        "{line}: the prepare wrote nothing"
    );
}

/// **A queued unload the state refuses at the start** (the fix round's correctness/F1, tests/T1):
/// staff put a queue on `tv`'s record (`hull`, slot 1, after its ordinal — a fresh record is put
/// at ordinal 1), so the offer's queued variant re-runs `unload_plan(…, true)`; the tick's start
/// for it, forged, is refused `ProviderRejected` with the state's own message `code`, the record
/// (queue and all) unchanged; then the record goes back as it was before the queue.
fn queued_refused(rig: &mut Rig, (who, tv, hull): (ObjId, ObjId, ObjId), code: &str) {
    let original = record_of(&mut rig.tw, tv);
    let mut record = original.clone().unwrap_or_else(|| {
        let mut fresh = RigRecord::fresh(tv, &battery());
        fresh.ordinal = 1;
        fresh
    });
    record.queued = Some(Queued {
        kind: ActKind::Unload,
        operator: who,
        hull,
        slot: 1,
        site: cid(AHUNA),
        after: record.ordinal,
        at_ic: ic_now(&rig.tw),
        excursion: None,
    });
    put_record(rig, record.clone());
    let next = record.ordinal + 1;
    let fake = forged(
        &rig.tw,
        who,
        tv,
        next,
        &json!({ "how": "queued" }),
        queued_key(tv, next),
    );
    let refused = land(rig, &fake).expect_err("the start refuses the queued unload");
    assert!(
        refused.contains("ProviderRejected") && refused.contains(code),
        "wanted `{code}`: {refused}"
    );
    assert_eq!(
        record_of(&mut rig.tw, tv),
        Some(record.clone()),
        "nothing written"
    );
    record.queued = None;
    if let Some(original) = original {
        put_record(rig, original);
    } else {
        put_record(rig, record);
    }
}

/// **U4 — the unload refuses where it cannot work** (A2; decisions R5, R6, R9): under way →
/// `UNLOAD_UNDER_WAY`; the hull docked at a port with a loaded hopper → `UNLOAD_NOT_SITE`; on
/// Ahuna's pad: an empty hopper → `DUMP_EMPTY`; the operator not aboard → `TV_NOT_ABOARD`; another
/// unload in flight → `RIG_BUSY` ("unloading its hopper into the hold"); a full hold →
/// `UNLOAD_NO_ROOM`; the rig on the ground → `UNLOAD_OUT` — each bare and with `go`, nothing
/// queued, the record unchanged, and a forged start with no plan refused "that act was not
/// planned" with nothing written. Where a queue makes a plan (under way, not on a site's pad, the
/// rig out), its start is refused with the state's own word (`queued_refused`); a hand order
/// overtaken by the state is U4b.
#[test]
#[allow(clippy::too_many_lines)]
fn sm8_the_unload_refuses_where_it_cannot_work() {
    let mut rig = stocked();
    // Under way, then docked at Stickney: a port, not a site's pad.
    let (sailor, sailor_row, sailor_tv) = at_ceres(&mut rig, "Sailor", 3, "Tally Moth");
    load(&mut rig, &sailor_row, 60, 20, 9_300);
    let t = file_and_launch(
        &mut rig,
        sailor,
        ObjId(sailor_row.hull),
        "+ship/undock stickney",
    );
    fill(&mut rig, sailor_tv, "metal_ore", 2, 8_821);
    let under_way = UNLOAD_UNDER_WAY.replace("{name}", "Tally Moth");
    refused_everywhere(&mut rig, (sailor, sailor_tv), "+tv/1 unload", &|out| {
        out == under_way
    });
    let sailor_hull = ObjId(sailor_row.hull);
    queued_refused(
        &mut rig,
        (sailor, sailor_tv, sailor_hull),
        "the hull is under way",
    );
    to_ic(&mut rig, t.plan.depart_ic + t.plan.total_s);
    let not_site = UNLOAD_NOT_SITE
        .replace("{name}", "Tally Moth")
        .replace("{n}", "1");
    refused_everywhere(&mut rig, (sailor, sailor_tv), "+tv/1 unload", &|out| {
        out == not_site
    });
    queued_refused(
        &mut rig,
        (sailor, sailor_tv, sailor_hull),
        "the hull is not on a site's pad",
    );
    // On Ahuna's pad.
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let empty = DUMP_EMPTY.replace("{n}", "1");
    refused_everywhere(&mut rig, (pilot, tv), "+tv/1 unload", &|out| out == empty);
    fill(&mut rig, tv, "metal_ore", 3, 8_822);
    // Not aboard: staff move the pilot from the hull's airlock space down into its bay
    // (`excursions.rs`'s seam: aboard is the airlock interior), then back.
    let mush_sim::data::SpatialParent::Interior {
        space: airlock,
        slot: airlock_slot,
    } = rig
        .tw
        .world()
        .module_state::<mush_sim::data::SpatialState>()
        .expect("the simulation")
        .parent_of(pilot)
    else {
        panic!("the pilot is aboard");
    };
    let bay = ObjId(row.rooms[&cid("bay")]);
    support::with_ctx(&mut rig.tw, |ctx| {
        mush_sim::place_interior(ctx, pilot, bay, None)
    })
    .expect("staff move the pilot into the bay");
    let ashore = TV_NOT_ABOARD.replace("{name}", HULL);
    refused_everywhere(&mut rig, (pilot, tv), "+tv/1 unload", &|out| out == ashore);
    support::with_ctx(&mut rig.tw, |ctx| {
        mush_sim::place_interior(ctx, pilot, airlock, airlock_slot)
    })
    .expect("staff put the pilot back aboard");
    // Busy: another unload in flight.
    let _ = act_go(&mut rig, pilot, "+tv/1 unload go");
    refused_everywhere(&mut rig, (pilot, tv), "+tv/1 unload", &|out| {
        out.starts_with("Rig 1 is unloading its hopper into the hold: done ")
    });
    let _ = finish(&mut rig, tv, pilot);
    // A full hold.
    let hold = hold_vault(&rig.tw, &row);
    let held = pool(&mut rig.tw, hold, "metal_ore");
    let room = ore_limit_t(&rig.tw) - held;
    seed_pool(&mut rig, hold, "metal_ore", room, LotId(8_823));
    fill(&mut rig, tv, "metal_ore", 3, 8_824);
    let no_room = UNLOAD_NO_ROOM
        .replace("{name}", HULL)
        .replace("{n}", "1")
        .replace("{load}", &words(&rig.tw, &[("metal_ore", 3)]));
    refused_everywhere(&mut rig, (pilot, tv), "+tv/1 unload", &|out| out == no_room);
    // The rig on the ground.
    launched(&mut rig, pilot, 1, tv);
    let out_there = UNLOAD_OUT
        .replace("{n}", "1")
        .replace("{site}", "Ahuna Flank");
    refused_everywhere(&mut rig, (pilot, tv), "+tv/1 unload", &|out| {
        out == out_there
    });
    queued_refused(
        &mut rig,
        (pilot, tv, ObjId(row.hull)),
        "the rig is not in the bay",
    );
    no_refused_writes(&rig.tw);
}

/// Stage `body` in one inventory transaction and install it (staff's hand).
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

/// Staff move `qty` of `good` from `from` to `to` (no marker).
fn staff_move(rig: &mut Rig, from: VaultId, to: VaultId, good: &str, qty: u64) {
    staff_inventory(rig, |families, txn| {
        families
            .transfer(txn, EndpointRef(from), EndpointRef(to), &cid(good), qty)
            .map(|_| ())
    });
}

/// **`line` filed while the state allows it**: exactly one start queued, NOT landed — a start
/// in the world task's queue when the state changes under it.
fn filed(rig: &mut Rig, who: ObjId, line: &str) -> AuthoritativeSubmit {
    let (out, mut queued) = command(rig, who, line);
    assert!(
        out.starts_with("Rig ") && out.contains(" starts unloading into the hold: done "),
        "{line}: {out}"
    );
    assert_eq!(queued.len(), 1, "{line}: one start");
    queued.remove(0)
}

/// **`start` lands into a changed state and is refused** with `code` (a substring of the error)
/// — the record, `tv`'s hopper and the hold exactly as they were.
fn overtaken(rig: &mut Rig, start: &AuthoritativeSubmit, (tv, hold): (ObjId, VaultId), code: &str) {
    let hop = hopper(&rig.tw, tv);
    let image = |rig: &mut Rig| {
        (
            record_of(&mut rig.tw, tv),
            pool(&mut rig.tw, hop, "metal_ore"),
            pool(&mut rig.tw, hold, "metal_ore"),
        )
    };
    let before = image(rig);
    let refused = land(rig, start).expect_err("the start meets the changed state");
    assert!(refused.contains(code), "wanted `{code}`: {refused}");
    assert_eq!(image(rig), before, "nothing written");
}

/// **U4b — a start filed while the state allowed it is refused at the start once the state
/// changes** (A2's "at the prepare"; the fix round's correctness/F1, conformance/F5, tests/T1):
/// each `+tv/<n> unload go` below is filed with room and the operator aboard, its start held in
/// the queue while staff change the world, then landed: the hopper emptied under it → `ProviderRejected`
/// `unload_changed` (the prepare's re-check of the hopper); the operator moved off the bridge →
/// `not_aboard` (the offer re-runs `unload_plan`); a queued unload resolved, then the hopper
/// emptied before its prepare → `unload_changed` (the queue kept); rig 2 put on the ground →
/// `unload_out`; a SIBLING's hopper filled so the hold's unreserved room no longer takes the
/// frozen load → `unload_changed` (the re-check counts every other rig of the hull); the hold
/// filled → `unload_changed`. Every time nothing is written.
#[test]
#[allow(clippy::too_many_lines)]
fn sm8_a_start_overtaken_by_the_state_is_refused_at_the_start() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 2);
    let (tv1, tv2) = (tvs[0], tvs[1]);
    let (hop1, hold) = (hopper(&rig.tw, tv1), hold_vault(&rig.tw, &row));
    let limit = ore_limit_t(&rig.tw);
    // The hopper emptied between the go and its landing.
    fill(&mut rig, tv1, "metal_ore", 3, 8_941);
    let start = filed(&mut rig, pilot, "+tv/1 unload go");
    staff_move(&mut rig, hop1, hold, "metal_ore", 3);
    overtaken(&mut rig, &start, (tv1, hold), UNLOAD_CHANGED_CODE);
    // The operator off the bridge.
    fill(&mut rig, tv1, "metal_ore", 3, 8_942);
    let start = filed(&mut rig, pilot, "+tv/1 unload go");
    let mush_sim::data::SpatialParent::Interior {
        space: airlock,
        slot: airlock_slot,
    } = rig
        .tw
        .world()
        .module_state::<mush_sim::data::SpatialState>()
        .expect("the simulation")
        .parent_of(pilot)
    else {
        panic!("the pilot is aboard");
    };
    let bay = ObjId(row.rooms[&cid("bay")]);
    support::with_ctx(&mut rig.tw, |ctx| {
        mush_sim::place_interior(ctx, pilot, bay, None)
    })
    .expect("staff move the pilot into the bay");
    overtaken(
        &mut rig,
        &start,
        (tv1, hold),
        "the operator is not aboard the hull",
    );
    support::with_ctx(&mut rig.tw, |ctx| {
        mush_sim::place_interior(ctx, pilot, airlock, airlock_slot)
    })
    .expect("staff put the pilot back aboard");
    // A queued unload resolved, then the hopper emptied before its prepare: the prepare's own
    // re-check (the offer froze 3 t).
    let mut record =
        record_of(&mut rig.tw, tv1).unwrap_or_else(|| RigRecord::fresh(tv1, &battery()));
    record.ordinal = record.ordinal.max(1);
    record.queued = Some(Queued {
        kind: ActKind::Unload,
        operator: pilot,
        hull: ObjId(row.hull),
        slot: 1,
        site: cid(AHUNA),
        after: record.ordinal,
        at_ic: ic_now(&rig.tw),
        excursion: None,
    });
    put_record(&mut rig, record.clone());
    let next = record.ordinal + 1;
    let start = forged(
        &rig.tw,
        pilot,
        tv1,
        next,
        &json!({ "how": "queued" }),
        queued_key(tv1, next),
    );
    let resolved = resolve(&rig, &start).expect("the queued start resolves");
    staff_move(&mut rig, hop1, hold, "metal_ore", 1);
    let refused = prepare(&mut rig, &resolved).expect_err("the hopper moved under the queue");
    assert!(
        refused.contains("ProviderRejected") && refused.contains(UNLOAD_CHANGED_CODE),
        "{refused}"
    );
    assert_eq!(
        record_of(&mut rig.tw, tv1),
        Some(record.clone()),
        "nothing written; the queue stands"
    );
    record.queued = None;
    put_record(&mut rig, record);
    // Rig 2 put on the ground under its go.
    fill(&mut rig, tv2, "metal_ore", 2, 8_943);
    let start = filed(&mut rig, pilot, "+tv/2 unload go");
    to_cell(&mut rig, tv2, (RAMP_CELL.0, RAMP_CELL.1 - 3));
    overtaken(&mut rig, &start, (tv2, hold), "the rig is not in the bay");
    // A sibling's hopper filled under rig 1's go: rig 1's 2 t fit the 4 t of room rig 2's 2 t
    // leave, and no longer once rig 2 holds 5 t (1 t of room).
    let hop2 = hopper(&rig.tw, tv2);
    let held = pool(&mut rig.tw, hold, "metal_ore");
    seed_pool(
        &mut rig,
        hold,
        "metal_ore",
        limit - held - 2 - 4,
        LotId(8_944),
    );
    assert_eq!(pool(&mut rig.tw, hop1, "metal_ore"), 2);
    let start = filed(&mut rig, pilot, "+tv/1 unload go");
    fill(&mut rig, tv2, "metal_ore", 3, 8_945);
    overtaken(&mut rig, &start, (tv1, hold), UNLOAD_CHANGED_CODE);
    // The hold filled under rig 1's go (1 t of its 2 t fits the 1 t of room left).
    let start = filed(&mut rig, pilot, "+tv/1 unload go");
    seed_pool(&mut rig, hold, "metal_ore", 1, LotId(8_946));
    overtaken(&mut rig, &start, (tv1, hold), UNLOAD_CHANGED_CODE);
    assert_eq!(
        pool(&mut rig.tw, hold, "metal_ore") + pool(&mut rig.tw, hop2, "metal_ore"),
        limit,
        "the hold's unreserved room is spent"
    );
    no_refused_writes(&rig.tw);
}

/// **An unload start's `how` and `qty` follow one rule** (the brief's 3.7, decision R15; the fix
/// round's tests/T3): with a whole hand order planned for the next ordinal, a start with no
/// `how`, `how: whole` with a `qty`, and `how: part` with none are each refused `InvalidRequest`
/// "an unload names how it was planned"; a dump start carrying `how` and a scan start carrying
/// `qty` are refused the same way — nothing written, and the planned start then lands.
#[test]
fn sm8_an_unload_start_names_how_it_was_planned() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    fill(&mut rig, tv, "metal_ore", 3, 8_951);
    let start = filed(&mut rig, pilot, "+tv/1 unload go");
    let next = ordinal_of(&mut rig.tw, tv) + 1;
    let before = record_of(&mut rig.tw, tv);
    let malformed = |act: &str, extra: Value| {
        let mut parameters = json!({ "tv": tv.0.to_string(), "act": act, "ordinal": next });
        if let (Some(map), Some(more)) = (parameters.as_object_mut(), extra.as_object()) {
            map.extend(more.clone());
        }
        parameters
    };
    for parameters in [
        malformed("unload", json!({})),
        malformed("unload", json!({ "how": "whole", "qty": 3 })),
        malformed("unload", json!({ "how": "part" })),
        malformed("unload", json!({ "how": "queued", "qty": 1 })),
        malformed("dump", json!({ "how": "whole" })),
        malformed("scan", json!({ "qty": 2 })),
    ] {
        let mut fake = start.clone();
        fake.parameters = parameters.clone();
        let refused = land(&mut rig, &fake).expect_err("a malformed start");
        assert!(
            refused.contains("InvalidRequest")
                && refused.contains("an unload names how it was planned"),
            "{parameters}: {refused}"
        );
        assert_eq!(
            record_of(&mut rig.tw, tv),
            before,
            "{parameters}: nothing written"
        );
    }
    land(&mut rig, &start).expect("the planned start lands");
    assert!(matches!(
        record_of(&mut rig.tw, tv)
            .and_then(|record| record.act)
            .map(|act| act.params),
        Some(ActParams::Unload {
            how: UnloadHow::Whole,
            ..
        })
    ));
    no_refused_writes(&rig.tw);
}

/// `ACT_FAILED` for rig `n`'s unload, `why` filled.
fn unload_failed(n: u8, why: &str) -> String {
    ACT_FAILED
        .replace("{n}", &n.to_string())
        .replace("{act}", "unload")
        .replace("{why}", why)
}

/// The record's last outcome's result.
fn last_result(tw: &mut TestWorld, tv: ObjId) -> Option<ActResult> {
    record_of(tw, tv).and_then(|record| record.last.map(|last| last.result))
}

/// **The unload's finish refuses what changed under it** (the brief's 3.8, decision R8; the fix
/// round's tests/T2 and correctness/F2), each told exactly with `ACT_FAILED` and its `WHY_*`, the
/// hopper and the hold untouched, the act released:
/// - ore taken out of the hopper after the start → `HopperChanged`;
/// - the finish's marker already applied (staff wrote it) → the transfer refuses the whole
///   bundle, nothing written, and the tick's next finish of the same act — `failed` naming it —
///   records `HoldRefused` (the dump's `SpoilGone` precedent);
/// - the hopper's marker ring full of young markers → `MarkersFull`, decided before anything is
///   staged, never told as a hold that would not take the load;
/// - the rig put on the ground → `LeftBay`;
/// - the hold filled to its limit after the start → `HoldRefused` (the admit check).
#[test]
#[allow(clippy::too_many_lines)]
fn sm8_the_unload_finish_refuses_what_changed_under_it() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 2);
    let (tv1, tv2) = (tvs[0], tvs[1]);
    let (hop1, hop2, hold) = (
        hopper(&rig.tw, tv1),
        hopper(&rig.tw, tv2),
        hold_vault(&rig.tw, &row),
    );
    let limit = ore_limit_t(&rig.tw);
    let held = |rig: &mut Rig, hop: VaultId| {
        (
            pool(&mut rig.tw, hop, "metal_ore"),
            pool(&mut rig.tw, hold, "metal_ore"),
        )
    };
    // HopperChanged: a tonne taken out after the start froze three.
    fill(&mut rig, tv1, "metal_ore", 3, 8_961);
    let _ = act_go(&mut rig, pilot, "+tv/1 unload go");
    staff_move(&mut rig, hop1, hold, "metal_ore", 1);
    let image = held(&mut rig, hop1);
    let heard = finish(&mut rig, tv1, pilot);
    assert_eq!(
        count(
            &heard,
            &unload_failed(1, tacsim::port::tv::WHY_HOPPER_CHANGED)
        ),
        1,
        "{heard}"
    );
    assert_eq!(
        last_result(&mut rig.tw, tv1),
        Some(ActResult::Refused {
            code: RefusedCode::HopperChanged
        })
    );
    assert_eq!(held(&mut rig, hop1), image, "nothing moved");
    // The marker already applied: the bundle refuses mid-transfer; the retry records it.
    fill(&mut rig, tv1, "metal_ore", 1, 8_962);
    let _ = act_go(&mut rig, pilot, "+tv/1 unload 2 go");
    let act = record_of(&mut rig.tw, tv1)
        .and_then(|record| record.act)
        .expect("the unload in flight");
    let id = tacsim::surface::unload::marker(tv1, None, act.ordinal, &cid("metal_ore"), act.due_ic)
        .expect("a marker id");
    let stamp = rig.tw.world().world_ic_now();
    let wall = rig.tw.world().now();
    staff_inventory(&mut rig, |families, txn| {
        families
            .transfer_marked(
                txn,
                EndpointRef(hop1),
                EndpointRef(hold),
                &cid("metal_ore"),
                1,
                &id,
                GOD,
                &stamp,
                wall,
            )
            .map(|_| ())
    });
    let image = held(&mut rig, hop1);
    let (_, queued) = advance_to(&mut rig, act.due_ic);
    let done = finishes(&queued);
    assert_eq!(done.len(), 1);
    let refused = land(&mut rig, done[0]).expect_err("the marked transfer refuses");
    assert!(
        refused.contains("the hopper could not be unloaded into the hold"),
        "{refused}"
    );
    assert_eq!(held(&mut rig, hop1), image, "nothing written");
    assert!(record_of(&mut rig.tw, tv1).is_some_and(|record| record.act.is_some()));
    let (_, queued) = fire(&mut rig);
    assert_eq!(finishes(&queued).len(), 1, "the retry");
    land_all(&mut rig, &queued);
    assert_eq!(
        last_result(&mut rig.tw, tv1),
        Some(ActResult::Refused {
            code: RefusedCode::HoldRefused
        })
    );
    let (events, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    let heard = said(&rig.tw, &events, pilot);
    assert_eq!(
        count(
            &heard,
            &unload_failed(1, tacsim::port::tv::WHY_HOLD_REFUSED)
        ),
        1,
        "{heard}"
    );
    assert_eq!(held(&mut rig, hop1), image, "the hopper is exact");
    // MarkersFull: rig 2's hopper ring filled with young markers after its start, by staff moves
    // to a scratch endpoint no hull reads (the hold's room is not touched).
    let ring = u64::try_from(mush_inventory::MAX_MARKER_RING).expect("fits");
    fill(&mut rig, tv2, "metal_ore", 2 + ring, 8_963);
    let _ = act_go(&mut rig, pilot, "+tv/2 unload 2 go");
    let scratch = EndpointRef(VaultId(9_000_000_001));
    let stamp = rig.tw.world().world_ic_now();
    let wall = rig.tw.world().now();
    for n in 0..ring {
        let id =
            mush_inventory::derived_marker_id(1, &json!({ "sm8.young": n })).expect("a marker id");
        staff_inventory(&mut rig, |families, txn| {
            families
                .transfer_marked(
                    txn,
                    EndpointRef(hop2),
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
    let image = held(&mut rig, hop2);
    assert_eq!(image.0, 2);
    let heard = finish(&mut rig, tv2, pilot);
    assert_eq!(
        count(
            &heard,
            &unload_failed(2, tacsim::port::tv::WHY_MARKERS_FULL)
        ),
        1,
        "{heard}"
    );
    assert!(
        !heard.contains(tacsim::port::tv::WHY_HOLD_REFUSED),
        "{heard}"
    );
    assert_eq!(
        last_result(&mut rig.tw, tv2),
        Some(ActResult::Refused {
            code: RefusedCode::MarkersFull
        })
    );
    assert_eq!(held(&mut rig, hop2), image, "nothing moved");
    // LeftBay: rig 1 put on the ground under its unload.
    let _ = act_go(&mut rig, pilot, "+tv/1 unload go");
    to_cell(&mut rig, tv1, (RAMP_CELL.0, RAMP_CELL.1 - 3));
    let image = held(&mut rig, hop1);
    let heard = finish(&mut rig, tv1, pilot);
    assert_eq!(
        count(&heard, &unload_failed(1, tacsim::port::tv::WHY_LEFT_BAY)),
        1,
        "{heard}"
    );
    assert_eq!(
        last_result(&mut rig.tw, tv1),
        Some(ActResult::Refused {
            code: RefusedCode::LeftBay
        })
    );
    assert_eq!(held(&mut rig, hop1), image, "nothing moved");
    // HoldRefused by the admit: the hold filled to its limit after rig 2's start.
    let _ = act_go(&mut rig, pilot, "+tv/2 unload go");
    let now_held = pool(&mut rig.tw, hold, "metal_ore");
    seed_pool(&mut rig, hold, "metal_ore", limit - now_held, LotId(8_964));
    let image = held(&mut rig, hop2);
    let heard = finish(&mut rig, tv2, pilot);
    assert_eq!(
        count(
            &heard,
            &unload_failed(2, tacsim::port::tv::WHY_HOLD_REFUSED)
        ),
        1,
        "{heard}"
    );
    assert_eq!(
        last_result(&mut rig.tw, tv2),
        Some(ActResult::Refused {
            code: RefusedCode::HoldRefused
        })
    );
    assert_eq!(held(&mut rig, hop2), image, "nothing moved");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// U5. A shrunk hold
// ---------------------------------------------------------------------------

/// **U5 — a shrunk hold unloads what fits and says what stays** (lead ruling 3; decision R7):
/// the hold staff-filled to leave 5 t and a 12 t hopper recovered: the recover's preview carries
/// `RECOVER_THEN_PART`; the queued unload moves 5 t and leaves 7, told `UNLOADED` with
/// `UNLOAD_STAYS`; the departure reads `HOPPER_LOADED`; `+tv/1 dump go` empties it. Then with no
/// room at all: the queued unload is a 0 s act with no goods, told `UNLOADED_NONE` once, the queue
/// cleared, no retry; the hand `+tv/1 unload` and `+tv/1 unload 1` both answer exactly
/// `UNLOAD_NO_ROOM`.
#[test]
#[allow(clippy::too_many_lines)]
fn sm8_a_shrunk_hold_unloads_what_fits_and_says_what_stays() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let (hop, hold) = (hopper(&rig.tw, tv), hold_vault(&rig.tw, &row));
    let limit = ore_limit_t(&rig.tw);
    seed_pool(&mut rig, hold, "metal_ore", limit - 5, LotId(8_831));
    launched(&mut rig, pilot, 1, tv);
    fill(&mut rig, tv, "metal_ore", 12, 8_832);
    let (out, _) = command(&mut rig, pilot, "+tv/1 recover");
    let part = RECOVER_THEN_PART
        .replace("{load}", &words(&rig.tw, &[("metal_ore", 5)]))
        .replace("{stays}", &words(&rig.tw, &[("metal_ore", 7)]));
    assert!(
        out.trim()
            .ends_with(&format!("{part}. +tv/1 recover go brings it in.")),
        "{out}\nwanted: …{part}"
    );
    let _ = recover_landed(&mut rig, pilot, 1, tv);
    let (_, queued) = fire(&mut rig);
    assert_eq!(starts(&queued).len(), 1, "the queued unload starts");
    land_all(&mut rig, &queued);
    let heard = finish(&mut rig, tv, pilot);
    let told = format!(
        "{}{}",
        UNLOADED
            .replace("{n}", "1")
            .replace("{load}", &words(&rig.tw, &[("metal_ore", 5)]))
            .replace("{name}", HULL)
            .replace("{hopper}", "7 of 12 t"),
        UNLOAD_STAYS.replace("{stays}", &words(&rig.tw, &[("metal_ore", 7)]))
    );
    assert_eq!(count(&heard, &told), 1, "{heard}\nwanted: {told}");
    assert_eq!(pool(&mut rig.tw, hop, "metal_ore"), 7);
    assert_eq!(pool(&mut rig.tw, hold, "metal_ore"), limit);
    let loaded = HOPPER_LOADED
        .replace("{n}", "1")
        .replace("{t}", "7")
        .replace("{name}", HULL);
    let (out, queued) = command(&mut rig, pilot, "+ship/lift ceres");
    assert_eq!(out.trim(), loaded);
    assert!(queued.is_empty());
    let _ = act_go(&mut rig, pilot, "+tv/1 dump go");
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(pool(&mut rig.tw, hop, "metal_ore"), 0);
    // No room at all: a 0 s queued unload, told once, nothing retried.
    launched(&mut rig, pilot, 1, tv);
    fill(&mut rig, tv, "metal_ore", 3, 8_833);
    let _ = recover_landed(&mut rig, pilot, 1, tv);
    let (_, queued) = fire(&mut rig);
    assert_eq!(starts(&queued).len(), 1);
    land_all(&mut rig, &queued);
    let act = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .expect("the empty unload in flight");
    assert_eq!(act.due_ic, act.started_ic, "a 0 s act");
    assert!(matches!(
        &act.params,
        ActParams::Unload { how: UnloadHow::Queued, goods, .. } if goods.is_empty()
    ));
    let heard = finish(&mut rig, tv, pilot);
    let none = UNLOADED_NONE
        .replace("{n}", "1")
        .replace("{stays}", &words(&rig.tw, &[("metal_ore", 3)]))
        .replace("{name}", HULL);
    assert_eq!(count(&heard, &none), 1, "{heard}\nwanted: {none}");
    for _ in 0..3 {
        let (events, queued) = fire(&mut rig);
        assert!(starts(&queued).is_empty() && finishes(&queued).is_empty());
        assert!(!said(&rig.tw, &events, pilot).contains("hopper still holds"));
    }
    assert!(record_of(&mut rig.tw, tv).is_some_and(|r| r.queued.is_none() && r.act.is_none()));
    let no_room = UNLOAD_NO_ROOM
        .replace("{name}", HULL)
        .replace("{n}", "1")
        .replace("{load}", &words(&rig.tw, &[("metal_ore", 3)]));
    for line in ["+tv/1 unload", "+tv/1 unload 1"] {
        let (out, queued) = command(&mut rig, pilot, line);
        assert_eq!(out.trim(), no_room, "{line}");
        assert!(queued.is_empty());
    }
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// U6. A replayed finish
// ---------------------------------------------------------------------------

/// **U6 — a replayed unload finish is a HIT** (decision R3): the unload's finish captured and
/// landed twice under its key: the second is the ledger's HIT; pools, marker and record unchanged;
/// the same finish prepared past the ledger refuses "that act is already finished".
#[test]
fn sm8_a_replayed_unload_finish_is_a_hit() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let (hop, hold) = (hopper(&rig.tw, tv), hold_vault(&rig.tw, &row));
    fill(&mut rig, tv, "metal_ore", 4, 8_841);
    let _ = act_go(&mut rig, pilot, "+tv/1 unload go");
    let act = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .expect("the unload in flight");
    let (_, queued) = advance_to(&mut rig, act.due_ic);
    let done = finishes(&queued);
    assert_eq!(done.len(), 1);
    let submit = done[0].clone();
    land(&mut rig, &submit).expect("the finish lands");
    let marker =
        tacsim::surface::unload::marker(tv, None, act.ordinal, &cid("metal_ore"), act.due_ic)
            .expect("a marker id");
    let image = |rig: &mut Rig| {
        (
            pool(&mut rig.tw, hop, "metal_ore"),
            pool(&mut rig.tw, hold, "metal_ore"),
            applied(&mut rig.tw, &marker),
            record_of(&mut rig.tw, tv),
        )
    };
    let after = image(&mut rig);
    assert_eq!((after.0, after.1, after.2), (0, 4, true));
    assert_eq!(land(&mut rig, &submit), Err(HIT.to_owned()));
    assert_eq!(image(&mut rig), after, "the HIT changed nothing");
    let past = land_past_the_ledger(&mut rig, &submit).expect_err("the act is finished");
    assert!(past.contains("that act is already finished"), "{past}");
    assert_eq!(
        image(&mut rig),
        after,
        "the prepare's guard changed nothing"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// U7. A restart between the recover and the unload
// ---------------------------------------------------------------------------

/// **U7 — a restart between the recover and the unload files it after boot** (§8 "nothing is
/// RAM-only"): two worlds; in each a rig at the ramp with 5 t in its hopper is recovered and its
/// finish lands; one world then reboots keyed (every write folded, the operator reconnected) before
/// the tick has started the unload, the other does not; both fire alike until the unload is told.
/// The records, the pools, the marker's presence, the projections and the lines the operator heard
/// after the restart point are equal, and the hopper is empty.
#[test]
fn sm8_a_restart_between_the_recover_and_the_unload_files_it_after_boot() {
    let mut seen: Vec<(String, String)> = Vec::new();
    for restart in [false, true] {
        let mut rig = stocked();
        let (pilot, row, tvs) = on_pad(&mut rig, 1);
        let tv = tvs[0];
        launched(&mut rig, pilot, 1, tv);
        fill(&mut rig, tv, "metal_ore", 5, 8_851);
        let _ = recover_landed(&mut rig, pilot, 1, tv);
        let excursion = record_of(&mut rig.tw, tv)
            .and_then(|record| record.queued)
            .map(|queued| queued.excursion)
            .expect("the unload is queued");
        let mut rig = if restart { reboot(rig, &[pilot]) } else { rig };
        let mut heard = String::new();
        for _ in 0..3 {
            let (events, queued) = fire(&mut rig);
            heard.push_str(&said(&rig.tw, &events, pilot));
            land_all(&mut rig, &queued);
        }
        let due = record_of(&mut rig.tw, tv)
            .and_then(|record| record.act)
            .map(|act| act.due_ic)
            .expect("the unload in flight after the restart point");
        let (events, queued) = advance_to(&mut rig, due);
        heard.push_str(&said(&rig.tw, &events, pilot));
        land_all(&mut rig, &queued);
        for _ in 0..2 {
            let (events, queued) = fire(&mut rig);
            heard.push_str(&said(&rig.tw, &events, pilot));
            land_all(&mut rig, &queued);
        }
        let (hop, hold) = (hopper(&rig.tw, tv), hold_vault(&rig.tw, &row));
        let record = record_of(&mut rig.tw, tv).expect("the record");
        let act = record.last.clone().expect("the unload's outcome");
        assert_eq!((act.kind, act.result), (ActKind::Unload, ActResult::Done));
        let marker = tacsim::surface::unload::marker(
            tv,
            excursion,
            act.ordinal,
            &cid("metal_ore"),
            act.at_ic,
        )
        .expect("a marker id");
        let projected = match store(&rig.tw).tv_rec(tv) {
            Some(tacsim::surface::TvRecState::Present(rec)) => Some(rec.clone()),
            _ => None,
        };
        let books = format!(
            "{record:#?}\n{projected:#?}\n{} {} {}",
            pool(&mut rig.tw, hop, "metal_ore"),
            pool(&mut rig.tw, hold, "metal_ore"),
            applied(&mut rig.tw, &marker),
        );
        assert_eq!(pool(&mut rig.tw, hop, "metal_ore"), 0, "every tonne moved");
        assert!(applied(&mut rig.tw, &marker), "the move's marker stands");
        assert!(
            heard.contains(
                &RECOVERED_UNLOADING
                    .replace("{n}", "1")
                    .replace("{name}", HULL)
            )
        );
        seen.push((books, heard));
        no_refused_writes(&rig.tw);
    }
    assert_eq!(seen[0].0, seen[1].0, "the books");
    assert_eq!(seen[0].1, seen[1].1, "the lines");
}

// ---------------------------------------------------------------------------
// U8. Rights never bar the way home or the unload
// ---------------------------------------------------------------------------

/// The Freeport Registry as a principal (Ahuna's issuer).
fn registry(tw: &TestWorld) -> mush_org::PrincipalId {
    mush_org::PrincipalId::Organization(mush_org::OrgId(
        port_map(tw, "ceres_port").orgs[&cid("freeport_registry")].org,
    ))
}

/// **Staff file a claim** (industry's own writer, `@claim/file`; `mining.rs`'s helper) over
/// every opened deposit of `at`'s block, held by `holder` under the Registry, `term` IC seconds.
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

/// The rig's lease record's contract id.
fn lease_of(tw: &TestWorld, tv: ObjId) -> u64 {
    store(tw)
        .tv_lease_on(tv)
        .map(|record| record.lease)
        .expect("a lease")
}

/// A pilot's gateway account.
fn account_of(rig: &Rig, who: ObjId) -> AccountId {
    rig.oracle
        .0
        .lock()
        .ok()
        .and_then(|map| map.get(&who).copied())
        .expect("an account")
}

/// Occator's counter's name and port, as the lines name them (`mining.rs`'s).
fn at_counter(tw: &TestWorld, template: &str) -> String {
    let counter = tw
        .world()
        .get(station_room(tw, "ceres_port", "room.c07_exchange"))
        .map(|o| o.name.clone())
        .expect("the counter");
    template
        .replace("{n}", "1")
        .replace("{counter}", &counter)
        .replace("{port}", "Ceres Freeport")
}

/// No line names the licence, the claim or the lease (§8 "rights are per act", §19 item 3).
fn silent_on_rights(text: &str) {
    for word in [
        "licence", "claim", "lease", "forfeit", "Registry", "Occator",
    ] {
        assert!(!text.contains(word), "`{word}` named: {text}");
    }
}

/// **U8 — rights never bar the way home or the unload** (SM.7's owed acceptance; §8, §19 item 3;
/// decision R14): a rig with ore from a real run (`FAIR7`); then a claim filed over its block for
/// another pilot (staff `@claim/file`), the licence lapsed and the lease set forfeit (the stored
/// documents edited across restarts — SM.3b's and SM.7's seams), each proven taken before the
/// way home is tried (the claim answers `MINE_CLAIMED`, the licence reads lapsed, the stored
/// lease is forfeit and answers `MINE_FORFEIT`): `+tv/1 recover` previews the unload, its `go`
/// lands, the queued unload starts and lands, every tonne moves, and `+tv/1 unload` previews a
/// fresh tonne — no line of theirs names the licence, the claim or the lease.
#[test]
#[allow(clippy::too_many_lines)]
fn sm8_rights_never_bar_the_way_home_or_the_unload() {
    let mut rig = stocked();
    let (p, row, tvs) = on_pad(&mut rig, 1);
    let q = pilot(&mut rig, "Other", 2);
    let tv = tvs[0];
    let all = picks(&mut rig.tw);
    let fair = fair7(&all);
    launched(&mut rig, p, 1, tv);
    let t = mined(&mut rig, p, 1, tv, &fair);
    assert!(t > 0);
    claim_block(&mut rig, &fair, q, 30 * 86_400);
    // The positive controls (the fix round's tests/T5): each right is really gone before the way
    // home is tried — the claim bars the operator's own mining there (the licence and the lease
    // still good, so the tenure gate answers) …
    let (out, queued) = command(&mut rig, p, "+tv/1 mine");
    assert_eq!(out.trim(), MINE_CLAIMED, "the claim took");
    assert!(queued.is_empty());
    let key = licence_key(account_of(&rig, p));
    let lapse = ic_now(&rig.tw) - 1;
    let rig = reboot_editing(rig, key.as_str(), &[p], |doc| {
        doc["expires_ic"] = json!(lapse);
        doc["grace_secs"] = json!(0);
    });
    let lease = tacsim::port::tv::tv_lease_key(lease_of(&rig.tw, tv));
    let mut rig = reboot_editing(rig, lease.as_str(), &[p], |doc| {
        doc["status"] = json!("forfeit");
    });
    warm(&mut rig);
    // … the licence reads lapsed, and the lease is forfeit (the store's record, and the lease
    // gate — mining's first — answering for it).
    let licence = run(&mut rig.tw, p, "+licence");
    assert!(licence.contains("lapsed"), "the licence lapsed: {licence}");
    assert!(
        store(&rig.tw)
            .tv_lease_on(tv)
            .is_some_and(|record| record.status == tacsim::port::tv::TvLeaseStatus::Forfeit),
        "the lease is forfeit"
    );
    let (out, queued) = command(&mut rig, p, "+tv/1 mine");
    assert_eq!(
        out.trim(),
        at_counter(&rig.tw, MINE_FORFEIT),
        "the lease bars mining"
    );
    assert!(queued.is_empty());
    let (out, queued) = command(&mut rig, p, "+tv/1 recover");
    assert!(queued.is_empty());
    assert!(out.contains("; then it unloads "), "{out}");
    silent_on_rights(&out);
    let _ = recover_landed(&mut rig, p, 1, tv);
    let (events, queued) = fire(&mut rig);
    silent_on_rights(&said(&rig.tw, &events, p));
    assert_eq!(starts(&queued).len(), 1, "the queued unload starts");
    land_all(&mut rig, &queued);
    let heard = finish(&mut rig, tv, p);
    silent_on_rights(&heard);
    let (hop, hold) = (hopper(&rig.tw, tv), hold_vault(&rig.tw, &row));
    assert_eq!(pool(&mut rig.tw, hop, "metal_ore"), 0);
    assert_eq!(pool(&mut rig.tw, hold, "metal_ore"), t);
    fill(&mut rig, tv, "metal_ore", 1, 8_861);
    let (out, queued) = command(&mut rig, p, "+tv/1 unload");
    assert!(out.starts_with("Rig 1 (OWP-TV-01) would unload "), "{out}");
    assert!(queued.is_empty());
    silent_on_rights(&out);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// U9 and U9b. A queued unload superseded; a stale hand order
// ---------------------------------------------------------------------------

/// **U9 — any start supersedes a queued unload** (lead ruling 1; decisions R1, R15): with a
/// queued unload pending, the operator's `+tv/1 dump go` is pending as the tick fires — the tick
/// defers, the dump lands, the queue is gone, the tick submits nothing for it afterwards, and a
/// stale queued start landing later is refused with nothing written. Then the race: with a queued
/// unload pending, the operator's own `+tv/1 unload go` (`how: whole`, `act_key`) and the tick's
/// queued start (`how: queued`, `queued_key`) both resolved for the same ordinal: the hand start
/// prepares first and lands, the tick's is refused `Conflict` ("that rig's orders moved on") with
/// nothing written, and the hopper moves once.
#[test]
#[allow(clippy::too_many_lines)]
fn sm8_any_start_supersedes_a_queued_unload() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let (hop, hold) = (hopper(&rig.tw, tv), hold_vault(&rig.tw, &row));
    launched(&mut rig, pilot, 1, tv);
    fill(&mut rig, tv, "metal_ore", 3, 8_871);
    let (recover, _) = recover_landed(&mut rig, pilot, 1, tv);
    // The dump, typed before the tick's next fire: pending as it fires.
    let out = run(&mut rig.tw, pilot, "+tv/1 dump go");
    assert!(
        out.contains("Rig 1 opens its hopper over the spoil."),
        "{out}"
    );
    let (_, queued) = fire(&mut rig);
    let filed = starts(&queued);
    assert_eq!(filed.len(), 1, "the dump's start alone: {queued:?}");
    assert_eq!(filed[0].parameters["act"], json!("dump"));
    land_all(&mut rig, &queued);
    assert!(
        record_of(&mut rig.tw, tv).is_some_and(|r| r.queued.is_none()),
        "the dump superseded the queued unload"
    );
    let _ = finish(&mut rig, tv, pilot);
    for _ in 0..3 {
        let (_, queued) = fire(&mut rig);
        assert!(
            starts(&queued).is_empty(),
            "nothing more starts: {queued:?}"
        );
        land_all(&mut rig, &queued);
    }
    assert_eq!(pool(&mut rig.tw, hop, "metal_ore"), 0);
    assert_eq!(
        pool(&mut rig.tw, hold, "metal_ore"),
        0,
        "nothing reached the hold"
    );
    let before = record_of(&mut rig.tw, tv);
    let stale = forged(
        &rig.tw,
        pilot,
        tv,
        recover + 1,
        &json!({ "how": "queued" }),
        queued_key(tv, recover + 1),
    );
    // The queue is gone and no order names this ordinal: the offer has no plan (the brief's
    // `Conflict` is the race loser's, below; a start with no plan is refused at the offer).
    let refused = land(&mut rig, &stale).expect_err("a stale queued start");
    assert!(
        refused.contains("ProviderRejected") && refused.contains(UNPLANNED),
        "{refused}"
    );
    assert_eq!(record_of(&mut rig.tw, tv), before, "nothing written");
    // The race.
    launched(&mut rig, pilot, 1, tv);
    fill(&mut rig, tv, "metal_ore", 4, 8_872);
    let (recover, _) = recover_landed(&mut rig, pilot, 1, tv);
    let (_, hand) = command(&mut rig, pilot, "+tv/1 unload go");
    assert_eq!(hand.len(), 1);
    assert_eq!(
        hand[0].key.as_str(),
        format!("tacsim.surface/{}/{}/act", tv.0, recover + 1)
    );
    let (_, queued) = fire(&mut rig);
    let tick = starts(&queued);
    assert_eq!(tick.len(), 1, "the tick's queued start: {queued:?}");
    assert_eq!(
        tick[0].key.as_str(),
        format!("tacsim.surface/{}/{}/queued", tv.0, recover + 1)
    );
    let first = resolve(&rig, &hand[0]).expect("the hand start resolves");
    let second = resolve(&rig, tick[0]).expect("the tick's start resolves");
    prepare(&mut rig, &first).expect("the hand start prepares first and lands");
    let after = record_of(&mut rig.tw, tv);
    let refused = prepare(&mut rig, &second).expect_err("the tick's start loses the race");
    assert!(
        refused.contains("Conflict") && refused.contains("that rig's orders moved on"),
        "{refused}"
    );
    assert_eq!(record_of(&mut rig.tw, tv), after, "nothing written");
    assert!(matches!(
        after.and_then(|r| r.act).map(|act| act.params),
        Some(ActParams::Unload {
            how: UnloadHow::Whole,
            ..
        })
    ));
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(pool(&mut rig.tw, hop, "metal_ore"), 0);
    assert_eq!(
        pool(&mut rig.tw, hold, "metal_ore"),
        4,
        "the hopper moved once"
    );
    no_refused_writes(&rig.tw);
}

/// **U9b — a stale hand order never captures a queued unload** (the brief's reviewer finding
/// F2a): `+tv/1 unload 3 go` lands (a part unload at ordinal N); **before any fire** its RAM order
/// still sits in the slot, and a start for N + 1 carrying the same `how: part, qty: 3` is refused
/// "that act was not planned" — the offer's own ordinal filter, not the prune (without the filter
/// the stale order would plan N + 1 and the start would meet "that rig is busy"; the fix round's
/// tests/T4); the next fire's complete scan prunes the order. The unload finishes; then a launch
/// and a recover with ore: the tick's queued start lands on its first attempt, its act a queued
/// unload of the hopper's whole contents, every tonne moves, nothing is told as changed.
#[test]
fn sm8_a_stale_hand_order_never_captures_a_queued_unload() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let (hop, hold) = (hopper(&rig.tw, tv), hold_vault(&rig.tw, &row));
    fill(&mut rig, tv, "metal_ore", 5, 8_881);
    let _ = act_go(&mut rig, pilot, "+tv/1 unload 3 go");
    let landed = ordinal_of(&mut rig.tw, tv);
    assert_eq!(
        tacsim::surface::act::unload_order_for_tests(rig.tw.world(), tv),
        Some(landed),
        "the landed order is still in the slot before any fire"
    );
    let before = record_of(&mut rig.tw, tv);
    let stale = forged(
        &rig.tw,
        pilot,
        tv,
        landed + 1,
        &json!({ "how": "part", "qty": 3 }),
        act_key(tv, landed + 1),
    );
    let refused = land(&mut rig, &stale).expect_err("a stale order plans nothing");
    assert!(
        refused.contains("ProviderRejected") && refused.contains(UNPLANNED),
        "{refused}"
    );
    assert_eq!(record_of(&mut rig.tw, tv), before, "nothing written");
    let (_, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    assert_eq!(
        tacsim::surface::act::unload_order_for_tests(rig.tw.world(), tv),
        None,
        "the tick's complete scan pruned the landed order"
    );
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(pool(&mut rig.tw, hop, "metal_ore"), 2);
    launched(&mut rig, pilot, 1, tv);
    let _ = recover_landed(&mut rig, pilot, 1, tv);
    let (_, queued) = fire(&mut rig);
    let filed = starts(&queued);
    assert_eq!(filed.len(), 1);
    land(&mut rig, filed[0]).expect("the queued start lands on its first attempt");
    let act = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .expect("the queued unload in flight");
    assert!(matches!(
        &act.params,
        ActParams::Unload { how: UnloadHow::Queued, goods, .. }
            if *goods == [(cid("metal_ore"), 2)].into_iter().collect()
    ));
    let heard = finish(&mut rig, tv, pilot);
    assert!(!heard.contains("changed since the preview"), "{heard}");
    assert_eq!(pool(&mut rig.tw, hop, "metal_ore"), 0);
    assert_eq!(pool(&mut rig.tw, hold, "metal_ore"), 5);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// U10. The departure names the unload
// ---------------------------------------------------------------------------

/// **U10 — the departure names the unload** (decision R5; reviewer finding F6): on Ahuna's pad a
/// loaded hopper reads exactly `HOPPER_LOADED` (now naming the unload) from `+ship/lift`,
/// `+ship/plan`'s second line and a queued depart's dry run; an unload in flight reads `TV_BUSY`
/// "unloading its hopper into the hold"; landed, no loaded hopper is named. At Ceres's dock a
/// staff-filled hopper reads exactly `HOPPER_LOADED_AWAY` from `+ship/undock` and `+ship/lift`,
/// and there `+tv/1 unload` answers `UNLOAD_NOT_SITE` — the port line names no remedy that refuses.
#[test]
#[allow(clippy::too_many_lines)]
fn sm8_the_departure_names_the_unload() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    fill(&mut rig, tv, "metal_ore", 3, 8_891);
    let loaded = HOPPER_LOADED
        .replace("{n}", "1")
        .replace("{t}", "3")
        .replace("{name}", HULL);
    let plan = run(&mut rig.tw, pilot, "+ship/plan ceres");
    assert_eq!(plan.lines().nth(1), Some(loaded.as_str()), "{plan}");
    let (out, queued) = command(&mut rig, pilot, "+ship/lift ceres");
    assert_eq!(out.trim(), loaded);
    assert!(queued.is_empty());
    let _ = run(&mut rig.tw, pilot, "+ship/order depart ceres at=+1h");
    let dry = run(&mut rig.tw, pilot, "+ship/plan");
    let would = format!(
        "depart for Ceres Freeport: it would hold: {}.",
        loaded.trim_end_matches('.')
    );
    assert!(
        dry.lines().any(|line| line.ends_with(&would)),
        "{dry}\nwanted: …{would}"
    );
    let _ = run(&mut rig.tw, pilot, "+ship/order clear");
    let _ = act_go(&mut rig, pilot, "+tv/1 unload go");
    let busy = TV_BUSY
        .replace("{n}", "1")
        .replace("{doing}", "unloading its hopper into the hold")
        .replace("{name}", HULL);
    let (out, queued) = command(&mut rig, pilot, "+ship/lift ceres");
    assert_eq!(out.trim(), busy);
    assert!(queued.is_empty());
    let _ = finish(&mut rig, tv, pilot);
    let plan = run(&mut rig.tw, pilot, "+ship/plan ceres");
    assert!(!plan.contains("hopper holds"), "{plan}");
    // At Ceres's dock: only staff can empty it.
    let (docker, docker_row, docker_tv) = at_ceres(&mut rig, "Docker", 3, "Tally Moth");
    load(&mut rig, &docker_row, 20, 20, 9_300);
    fill(&mut rig, docker_tv, "metal_ore", 2, 8_892);
    let away = HOPPER_LOADED_AWAY
        .replace("{n}", "1")
        .replace("{t}", "2")
        .replace("{name}", "Tally Moth");
    for line in ["+ship/undock stickney", "+ship/lift stickney"] {
        let (out, queued) = command(&mut rig, docker, line);
        assert_eq!(out.trim(), away, "{line}");
        assert!(queued.is_empty());
    }
    let (out, _) = command(&mut rig, docker, "+tv/1 unload");
    assert_eq!(
        out.trim(),
        UNLOAD_NOT_SITE
            .replace("{name}", "Tally Moth")
            .replace("{n}", "1")
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// U11. The bay recharges an unloading rig
// ---------------------------------------------------------------------------

/// **U11 — an unload regains charge in the bay** (lead ruling 2; doc 20 §4.1 "a 600 IC s unload
/// regains 8,300 cu"): a rig in the bay at half charge unloads; its charge after reads, at the
/// unload's due, exactly the bay's recharge over the act — `600 × recharge / 3,600` cu from the
/// battery row, capped at the capacity — and the unload spent nothing.
#[test]
fn sm8_an_unload_regains_charge_in_the_bay() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let b = battery();
    let start = Charge {
        value_cu: b.capacity_cu / 2,
        at_ic: ic_now(&rig.tw),
        in_bay: true,
    };
    let mut record = record_of(&mut rig.tw, tv).unwrap_or_else(|| RigRecord::fresh(tv, &b));
    record.charge = start;
    put_record(&mut rig, record);
    fill(&mut rig, tv, "metal_ore", 2, 8_901);
    let _ = act_go(&mut rig, pilot, "+tv/1 unload go");
    let act = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .expect("the unload in flight");
    assert_eq!((act.started_ic, act.reserved_cu), (start.at_ic, 0));
    let _ = finish(&mut rig, tv, pilot);
    let after = record_of(&mut rig.tw, tv)
        .map(|record| record.charge)
        .expect("the record");
    let gained = u64::from(UNLOAD_S) * u64::from(b.bay_recharge_cu_per_ic_hour) / 3_600;
    let expected = (u64::from(start.value_cu) + gained).min(u64::from(b.capacity_cu));
    assert_eq!(
        after,
        Charge {
            value_cu: u32::try_from(expected).expect("fits"),
            at_ic: act.due_ic,
            in_bay: true,
        }
    );
    assert_eq!(u64::from(charge_at(&after, act.due_ic, &b)), expected);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// U12 (A4). The hopper never sells
// ---------------------------------------------------------------------------

/// **U12 — the hopper never sells** (A4; D-20-10; decision R12): a Dray-P docked at Ceres with ore
/// staff-seeded in a rig's hopper: the pilot may not take from it; the pilot's `+market/sell <metal
/// ore bid>=1 from=<hopper>` at the Exchange answers exactly `NO_SUCH_LISTING`; a Royalty-flagged
/// character, who may take from it, answers exactly `HOLD_ONLY_SOURCE`; `governed_hold` and the
/// market's capacity plan answer nothing for it; no quote, no contract, the pools unchanged. The
/// positive control: a tonne seeded into the same hull's hold sells at the bid.
#[test]
fn sm8_the_hopper_never_sells() {
    let mut rig = stocked();
    // Docked at Ceres, the pilot left at Occator's counter in the Exchange Hall after the lease.
    let (pilot, row) = prospector(&mut rig, "Pilot", 1, HULL);
    lease_one(&mut rig, pilot);
    let tv = rigs_of(&rig.tw, pilot)[0];
    fill(&mut rig, tv, "metal_ore", 2, 8_911);
    let hop = hopper(&rig.tw, tv);
    let endpoint = EndpointRef(hop);
    assert!(!mush_inventory::may_take(rig.tw.world(), pilot, endpoint));
    assert!(tacsim::port::governed_hold(rig.tw.world(), endpoint).is_none());
    assert!(
        tacsim::port::PortCapacity
            .plan(rig.tw.world(), endpoint)
            .is_none()
    );
    let listing = port_map(&rig.tw, "ceres_port").vendors[&cid("freeport_exchange")].bids
        [&cid("metal_ore")]
        .listing;
    let hall = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    let line = format!("+market/sell {listing}=1 from={}", hop.0);
    let (out, queued) = command(&mut rig, pilot, &line);
    assert_eq!(out.trim(), mush_market::NO_SUCH_LISTING);
    assert!(queued.is_empty());
    let royal = rig.tw.spawn_player("Auditor");
    rig.tw
        .world_mut()
        .get_mut(royal)
        .expect("the auditor exists")
        .flags
        .insert(mush_core::Flags::ROYALTY);
    teleport(&mut rig.tw, royal, hall);
    assert!(mush_inventory::may_take(rig.tw.world(), royal, endpoint));
    let (out, queued) = command(&mut rig, royal, &line);
    assert_eq!(out.trim(), mush_market::HOLD_ONLY_SOURCE);
    assert!(queued.is_empty());
    let market = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .expect("market");
    for who in [pilot, royal] {
        assert!(
            market
                .quotes_of_seller(&mush_org::PrincipalId::Object(who))
                .next()
                .is_none(),
            "no quote"
        );
    }
    assert_eq!(
        pool(&mut rig.tw, hop, "metal_ore"),
        2,
        "the hopper is as it was"
    );
    // The positive control: the hold sells.
    let hold = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, hold, "metal_ore", 1, LotId(8_912));
    let (out, queued) = command(&mut rig, pilot, &format!("+market/sell {listing}=1"));
    let label = &store(&rig.tw)
        .goods()
        .expect("the goods table")
        .good(&cid("metal_ore"))
        .expect("metal ore")
        .label;
    assert!(out.contains(&format!("Sold 1 t of {label} ")), "{out}");
    land_all(&mut rig, &queued);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// A3. Two rigs fill one hold
// ---------------------------------------------------------------------------

/// The hold's load, both hoppers and every outstanding cycle stay inside the hold's mass limit,
/// and each hopper inside its own.
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

/// **A3 — two rigs fill one hold and both unload everything** (§19 item 1: a full hold never
/// strands the departure): two rigs of one Dray-P on two fair pockets, the hold seeded so their two
/// runs use its unreserved room to the tonne; both runs end, both recover, both queued unloads land
/// on successive fires; after every fire the hold, both hoppers and every outstanding cycle stay
/// inside both limits; at the end both hoppers are empty, the hold holds the seeded load plus every
/// tonne mined, and `+ship/plan ceres` names no loaded hopper.
#[test]
#[allow(clippy::too_many_lines)]
fn sm8_two_rigs_fill_one_hold_and_both_unload_everything() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 2);
    let all = picks(&mut rig.tw);
    let fair = fair7(&all);
    let other = fair_other(&all, "FAIR (rig 2)", &[&fair]);
    launched(&mut rig, pilot, 1, tvs[0]);
    launched(&mut rig, pilot, 2, tvs[1]);
    sounded_at(&mut rig, pilot, 1, tvs[0], &fair);
    sounded_at(&mut rig, pilot, 2, tvs[1], &other);
    // Fair metals lift 2 t a cycle: a run takes the pocket's tonnes, at most the hopper's 12.
    let a = u64::from(fair.tonnes[0]).min(12);
    let b = u64::from(other.tonnes[0]).min(12);
    let limit = ore_limit_t(&rig.tw);
    let hold = hold_vault(&rig.tw, &row);
    let seeded = limit - a - b;
    seed_pool(&mut rig, hold, "metal_ore", seeded, LotId(8_921));
    let _ = act_go(&mut rig, pilot, "+tv/1 mine go");
    let _ = act_go(&mut rig, pilot, "+tv/2 mine go");
    for (tv, want) in [(tvs[0], a), (tvs[1], b)] {
        let run = run_of(&mut rig.tw, tv).expect("a run");
        assert_eq!(
            run.reserve_t.iter().map(|t| u64::from(*t)).sum::<u64>(),
            want,
            "the run takes the room to the tonne"
        );
    }
    within_limits(&mut rig, hold, &tvs, limit);
    for _ in 0..64 {
        let next = tvs
            .iter()
            .filter_map(|tv| run_of(&mut rig.tw, *tv).map(|run| run.due_ic))
            .min();
        let Some(due) = next else {
            break;
        };
        let (_, queued) = advance_to(&mut rig, due);
        land_all(&mut rig, &queued);
        within_limits(&mut rig, hold, &tvs, limit);
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
        within_limits(&mut rig, hold, &tvs, limit);
    }
    assert!(tvs.iter().all(|tv| run_of(&mut rig.tw, *tv).is_none()));
    let (hop_a, hop_b) = (hopper(&rig.tw, tvs[0]), hopper(&rig.tw, tvs[1]));
    let mined_a = pool(&mut rig.tw, hop_a, "metal_ore");
    let mined_b = pool(&mut rig.tw, hop_b, "metal_ore");
    assert_eq!((mined_a, mined_b), (a, b), "both hoppers full to the plan");
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let _ = act_go(&mut rig, pilot, "+tv/2 recover go");
    for _ in 0..64 {
        let mut done = true;
        let mut next: Option<i64> = None;
        for tv in &tvs {
            let record = record_of(&mut rig.tw, *tv).expect("a record");
            let unloaded = record.act.is_none()
                && record.queued.is_none()
                && record
                    .last
                    .as_ref()
                    .is_some_and(|l| (l.kind, l.result) == (ActKind::Unload, ActResult::Done));
            done &= unloaded;
            if let Some(act) = &record.act {
                next = Some(next.map_or(act.due_ic, |n| n.min(act.due_ic)));
            }
        }
        if done {
            break;
        }
        let (_, queued) = match next {
            Some(due) if due > ic_now(&rig.tw) => advance_to(&mut rig, due),
            _ => fire(&mut rig),
        };
        land_all(&mut rig, &queued);
        within_limits(&mut rig, hold, &tvs, limit);
    }
    for tv in &tvs {
        let vault = hopper(&rig.tw, *tv);
        assert_eq!(pool(&mut rig.tw, vault, "metal_ore"), 0);
    }
    assert_eq!(pool(&mut rig.tw, hold, "metal_ore"), seeded + a + b);
    assert_eq!(
        pool(&mut rig.tw, hold, "metal_ore"),
        limit,
        "the hold is full"
    );
    let plan = run(&mut rig.tw, pilot, "+ship/plan ceres");
    assert!(!plan.contains("hopper holds"), "{plan}");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// The finish's writes, counted (the brief's 3.8, reviewer finding F14)
// ---------------------------------------------------------------------------

/// **Land `submit`, counting the bundle's staged writes** (each `ModuleOp` the prepare staged).
fn land_counting(rig: &mut Rig, submit: &AuthoritativeSubmit) -> usize {
    let resolved = resolve(rig, submit).expect("the finish resolves");
    let (prepared, _) = rig
        .tw
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(
            &resolved.action,
            &resolved.resolved,
            CommitToken::from_bytes(resolved.token),
        )
        .expect("the finish prepares");
    let (ops, install) = prepared.into_parts();
    let count = ops.len();
    rig.tw
        .engine_mut()
        .install_prepared_action(install)
        .expect("the finish installs");
    keep(rig, ops);
    rig.ledger.insert(ledger_identity(submit));
    count
}

/// `+tv/<slot> unload go`, landed; the finish at its due counted, then told.
fn counted_unload(rig: &mut Rig, pilot: ObjId, slot: u8, tv: ObjId) -> usize {
    let _ = act_go(rig, pilot, &format!("+tv/{slot} unload go"));
    let due = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .map(|act| act.due_ic)
        .expect("the unload in flight");
    let (_, queued) = advance_to(rig, due);
    let done = finishes(&queued);
    assert_eq!(done.len(), 1);
    let submit = done[0].clone();
    let count = land_counting(rig, &submit);
    let (_, queued) = fire(rig);
    land_all(rig, &queued);
    count
}

/// **The unload finish's writes, counted against §11's Tick row "≤ 8 writes a completion"**
/// (the brief's 3.8, reviewer finding F14): rig 1's one-good finish into a fresh hold pool, a
/// two-good finish (one fresh pool), and a one-good finish into an existing pool; then rig 2,
/// whose hopper's marker ring holds forty markers staff-written two real days ago through the
/// inventory's own marked move, unloads one good into an existing pool and its marker push evicts
/// aged markers, then two goods into existing pools, evicting more. The figures are printed for
/// the report (`--nocapture`) and **pinned** (the fix round's tests/T6), every one over the row;
/// nothing is trimmed to meet it — SM.12's cost lane gates on it.
#[test]
#[allow(clippy::too_many_lines)]
fn sm8_the_unload_finish_counts_its_writes() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 2);
    let (tv1, tv2) = (tvs[0], tvs[1]);
    let hold = hold_vault(&rig.tw, &row);
    fill(&mut rig, tv1, "metal_ore", 2, 8_931);
    let fresh = counted_unload(&mut rig, pilot, 1, tv1);
    fill(&mut rig, tv1, "ice_ore", 1, 8_932);
    fill(&mut rig, tv1, "metal_ore", 1, 8_933);
    let two = counted_unload(&mut rig, pilot, 1, tv1);
    fill(&mut rig, tv1, "metal_ore", 1, 8_934);
    let existing = counted_unload(&mut rig, pilot, 1, tv1);
    // Forty aged markers at the front of rig 2's hopper ring.
    let hop2 = hopper(&rig.tw, tv2);
    fill(&mut rig, tv2, "metal_ore", 41, 8_935);
    let families = rig
        .tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    let aged = rig.tw.world().now() - 2 * mush_inventory::MARKER_RETENTION_SECS;
    let stamp = rig.tw.world().world_ic_now();
    let mut ids = Vec::new();
    for n in 0..40_u64 {
        let id =
            mush_inventory::derived_marker_id(1, &json!({ "sm8.aged": n })).expect("a marker id");
        let engine = rig.tw.engine_mut();
        let prepared = {
            let mut txn = engine
                .action_transaction(&families.pools)
                .expect("an inventory transaction");
            families
                .transfer_marked(
                    &mut txn,
                    EndpointRef(hop2),
                    EndpointRef(hold),
                    &cid("metal_ore"),
                    1,
                    &id,
                    GOD,
                    &stamp,
                    aged,
                )
                .expect("an aged marked move");
            txn.prepare().expect("prepares")
        };
        let (ops, install) = prepared.into_parts();
        engine.install_prepared_action(install).expect("installs");
        keep(&mut rig, ops);
        ids.push(id);
    }
    let evicting = counted_unload(&mut rig, pilot, 2, tv2);
    let evicted = ids.iter().filter(|id| !applied(&mut rig.tw, id)).count();
    // A two-good finish into existing pools whose ring push evicts more aged markers.
    fill(&mut rig, tv2, "ice_ore", 1, 8_936);
    fill(&mut rig, tv2, "metal_ore", 1, 8_937);
    let evicting_two = counted_unload(&mut rig, pilot, 2, tv2);
    let evicted_two = ids
        .iter()
        .filter(|id| !applied(&mut rig.tw, id))
        .count()
        .saturating_sub(evicted);
    println!(
        "SM.8 unload finish writes: one good into a fresh hold pool {fresh}; two goods (one fresh \
         pool) {two}; one good into an existing pool {existing}; one good into an existing pool \
         evicting {evicted} aged markers {evicting}; two goods into existing pools evicting \
         {evicted_two} aged markers {evicting_two} (§11 Tick row: ≤ 8 a completion)"
    );
    // **Pinned** (the fix round's tests/T6): every figure is over §11's Tick row of ≤ 8 writes
    // a completion — a residual for SM.12's cost lane, which gates on it; a change that adds a
    // write to the finish moves one of these and must say why.
    assert_eq!(fresh, 10, "one good into a fresh hold pool");
    assert_eq!(two, 17, "two goods, one fresh hold pool");
    assert_eq!(existing, 9, "one good into an existing pool");
    assert_eq!(evicted, 8, "the one-good push evicts eight aged markers");
    assert_eq!(
        evicting, 17,
        "one good into an existing pool, eight evictions"
    );
    assert_eq!(
        evicting,
        existing + evicted,
        "each eviction is one inventory.applied removal"
    );
    assert_eq!(evicted_two, 3, "the two-good push evicts three more");
    assert_eq!(
        evicting_two, 19,
        "two goods into existing pools, three evictions"
    );
    assert_eq!(pool(&mut rig.tw, hop2, "metal_ore"), 0);
    assert_eq!(pool(&mut rig.tw, hop2, "ice_ore"), 0);
    no_refused_writes(&rig.tw);
}
