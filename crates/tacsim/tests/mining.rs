//! **Doc 20 cut SM.7 acceptance — mining** (doc 20 §1 beat 8, §3.4, §4.1, §4.2, §5, §6, §8,
//! §11; D-20-5, D-20-9, D-20-11; the SM.7 brief's acceptance A1–A11 and its tests M1–M16 in
//! 3.12, with the fixer round's M12b and M17–M19).
//!
//! A First Captain whose Dray-P is settled on a site's pad mines: `+tv/1 mine` previews a run from
//! the operator's **own** reading of the pocket the rig stands on — its class, each cycle's tonnes,
//! the time, the hopper and the hold's room, the charge after the run and after the trip home —
//! or, on a pocket they have never read, one blind cycle; `go` commits ONE `tacsim_surface_act`
//! (`act: mine`) whose prepare writes the run into the rig's cycle slot and locks the pocket's
//! cursor rows; the `tacsim.surface` tick lands each cycle at its own due through
//! `tacsim_surface_done` (keyed per cycle), moving the family's ore from the block vault to the
//! hopper, drawing the cursor, spending the charge and writing the operator's core; the operator
//! hears each cycle's line, and the site's `tacsim.surface.mined` ring gains a fact.
//!
//! Composed on the composition root (`driving.rs`'s rig, copied: the per-file precedent),
//! **booted keyed** with the geology key handed in through `tacsim::surface::install_geology`;
//! Ahuna Flank (or the site a test names) opened once a world through `support::sites`. Every act
//! goes through the real verb, the real land path and the real tick; rigs are put in place by the
//! tick-batch placement seam a test may call directly (`mush_sim::place`, `staff_place`), never a
//! fake drive verb. **Pockets are chosen from the keyed truth** (`picks`) and printed
//! (`--nocapture`) for the report. Landings keep a ledger of every landed submission's identity
//! (`ledger_identity`, the world task's idempotency inputs), so a redelivered submission is the
//! ledger's HIT as in the world task; `resolve`/`prepare` split a landing in two for the races.

mod support;

#[path = "support/geology_keyed.rs"]
mod geology_keyed;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use mush_core::{
    AccountId, AccountIdentity, ActionStateBootRecord, Audience, AuthoritativeSubmit, CanonicalId,
    CommitToken, GOD, IdempotencyKey, IdempotencyScope, ModuleWrite, ObjId, OfferExecutionContext,
    OfferSubmission, RequestUid, ScopedIdempotencyKey, World,
};
use mush_economy::{EconomyConfig, VaultId};
use mush_industry::OreDepositId;
use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde_json::{Value, json};
use smol_str::SmolStr;
use tacsim::definitions::content::{DRAY_P, LANE_JSON, LANE_PACK, TV_PROSPECTOR};
use tacsim::definitions::tools::{BatteryDef, ToolDef, tools_table};
use tacsim::pack::{PackEntry, RulesPack, load_pack};
use tacsim::port::licence::{
    LICENCE_LAPSED, LICENCE_OTHER, LICENCE_SUSPENDED, ic_stamp, licence_key,
};
use tacsim::port::tv::{
    CORED, CYCLE_DONE, CYCLE_FAILED, MINE_ALL_BARREN, MINE_ALSO, MINE_BLIND_COUNT, MINE_BLIND_ROOM,
    MINE_CLAIMED, MINE_ENDED, MINE_FORFEIT, MINE_HOLD_FULL, MINE_HOPPER_FULL, MINE_IN_BAY,
    MINE_LOCKED, MINE_NO_LICENCE, MINE_NO_ORE, MINE_NOT_RUNNING, MINE_PREVIEW_BLIND, MINE_RUNNING,
    MINE_STOPPING, MINE_STOPPING_ALREADY, MINE_WORKED_OUT, NO_READING, NOTHING_LIFTED, PIT_WORKED,
    PIT_WORKED_OUT, RUN_DARK, RUN_RIGHTS, RUN_STOP_REMEDY, RUN_STOPPED, RUN_WORKED_OUT, WHY_MOVED,
    WHY_UNWORKED, ore_words,
};
use tacsim::port::voyage::TV_BUSY;
use tacsim::port::{HullRow, PortHandles, PortMap, PortMapState, Ticket, TicketId, room_key};
use tacsim::surface::address::{pocket_address, pocket_number};
use tacsim::surface::cursor::PocketCursor;
use tacsim::surface::drive::{DriveRates, HomeField, home_cu_here};
use tacsim::surface::geology::{OreFamily, Word};
use tacsim::surface::ground::{SiteGeologySpec, SiteGround, site_ground_sync};
use tacsim::surface::mined::MinedState;
use tacsim::surface::reading::Footing;
use tacsim::surface::{
    ActKind, ActResult, Charge, ChartKey, ChartRecord, GeologyBoot, GeologyState, Instrument,
    PIT_SEEN, RigKey, RigRecord, Run, RunEnd, SURFACE_ACT_ACTION, SURFACE_DONE_ACTION, TIER_R2,
    TvRec, TvRecState, cell_tier,
};
use tacsim::{TacsimConfig, TacsimStore};

use geology_keyed::{KEY_1, secret};

/// The ramp's cell.
const RAMP_CELL: (i64, i64) = (128, 124);
/// Ahuna Flank's port entry.
const AHUNA: &str = "ahuna_port";
/// The Freeport Registry's name.
const REGISTRY: &str = "Freeport Registry";

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

// ---------------------------------------------------------------------------
// The rig (driving.rs's, booted keyed)
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
    /// **The harness's idempotency ledger** (A7; the SM.7 fixer's T1): the identity of every
    /// submission that landed, over exactly the fields `mush_server`'s
    /// `ActionCoordinator::command_ingress` digests into the request's idempotency key (actor,
    /// action, subject, targets, parameters, key). A second landing of an identity already here is
    /// the world task's HIT — answered from the record, never prepared ([`land`] returns [`HIT`]).
    /// It survives a restart, as the store's ledger does.
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
    let mut bytes = [0x6d_u8; 16];
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

/// **A submission's ledger identity** — the fields `ActionCoordinator::command_ingress`
/// (`crates/mush-server/src/action_coordinator.rs`) digests into its request uid, so two
/// submissions share an identity exactly when the world task's ledger would answer the second
/// from the first's record.
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

/// A submission resolved against the world as it stands — the offer re-planned at "landing" —
/// and not yet prepared (M16 holds one across another's landing).
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
/// bundle installed and its typed post-images kept; the action outcome's JSON. The error
/// carries the prepare's own message.
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
/// that already landed is the HIT: [`HIT`], nothing prepared), then [`resolve`] and [`prepare`]
/// — recording the identity once it lands; the action outcome's JSON.
fn land_outcome(rig: &mut Rig, submit: &AuthoritativeSubmit) -> Result<Value, String> {
    let identity = ledger_identity(submit);
    if rig.ledger.contains(&identity) {
        return Err(HIT.to_owned());
    }
    let resolved = resolve(rig, submit)?;
    let outcome = prepare(rig, &resolved)?;
    rig.ledger.insert(identity);
    Ok(outcome)
}

/// [`land_outcome`], the outcome dropped.
fn land(rig: &mut Rig, submit: &AuthoritativeSubmit) -> Result<(), String> {
    land_outcome(rig, submit).map(|_| ())
}

/// **Past the ledger**: a submission resolved and prepared even though its identity already
/// landed — what the prepare's own guard sees if the HIT were ever missed (A7's second guard).
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

/// Land every submission; each must land or be the ledger's HIT (the world task answers a HIT
/// from its record: `ic_slice_loop.rs`'s `Landing::Replay`).
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

/// **A Dray-P on a site's pad** (`venue` the undock word, `entry` the site's port entry), its
/// pilot aboard, `n_rigs` rigs leased; the site opened first (its pad is closed until then) and
/// its ground and home warmed.
fn on_pad_at(
    rig: &mut Rig,
    (who, seed, name): (&str, i64, &str),
    (venue, entry): (&str, &str),
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
    warm(rig, entry);
    let tvs = rigs_of(&rig.tw, pilot);
    assert_eq!(tvs.len(), n_rigs);
    (pilot, row, tvs)
}

/// [`on_pad_at`] on Ahuna Flank.
fn on_pad_as(
    rig: &mut Rig,
    who: &str,
    seed: i64,
    name: &str,
    n_rigs: usize,
) -> (ObjId, HullRow, Vec<ObjId>) {
    on_pad_at(rig, (who, seed, name), ("ahuna", AHUNA), n_rigs)
}

/// [`on_pad_as`] for the usual pilot.
fn on_pad(rig: &mut Rig, n_rigs: usize) -> (ObjId, HullRow, Vec<ObjId>) {
    on_pad_as(rig, "Pilot", 1, "Salt Moth", n_rigs)
}

/// Fire until `entry`'s ground is cached and its home field built (a cycle's preview reserves
/// the real trip home; the surface tick builds the home the fire after the site).
fn warm(rig: &mut Rig, entry: &str) {
    let (spec, location) = spec_of(&rig.tw, entry);
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
                    .any(|(site, ramp, _)| site.as_str() == entry && *ramp == RAMP_CELL)
            });
        if cached && homed {
            return;
        }
        let (_, queued) = fire(rig);
        land_quietly(rig, &queued);
    }
    panic!("{entry}'s ground was not warmed");
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
fn spec_of(tw: &TestWorld, entry: &str) -> (SiteGeologySpec, String) {
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
    (spec, def.location.to_string())
}

/// A site's keyed ground, as a test reads it (a cache hit once warm).
fn ground_of(tw: &mut TestWorld, entry: &str) -> Arc<SiteGround> {
    let (spec, location) = spec_of(tw, entry);
    site_ground_sync(tw.world_mut(), &site_frame(&location), &spec).expect("the site builds")
}

/// **One pocket of a site, from the keyed truth** (the brief's 3.12): its address, the pocket,
/// the stand cell at its middle, the classes and tonnes per family, the footing at the stand.
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

    /// The pocket's number in its block.
    fn number(&self) -> u16 {
        pocket_number(self.pocket.0, self.pocket.1).expect("a number")
    }

    /// The deposit of `family` at this pocket's block.
    fn deposit(&self, entry: &str, family: OreFamily) -> OreDepositId {
        mush_industry::derive_system_deposit_id(&tacsim::surface::opening::receipt(
            &cid(entry),
            self.letter(),
            family,
        ))
        .expect("a deposit id")
    }
}

/// **Every pocket of `entry` with a footing at its middle**, read from the site's keyed table and
/// its composite — the truth a test may read, never a player.
fn picks(tw: &mut TestWorld, entry: &str) -> Vec<Pick> {
    let ground = ground_of(tw, entry);
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
                addr: pocket_address(px, py).expect("an address"),
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

/// **`FAIR7`** (A1): fair metals, 7 t, the only family, standing on regolith or ice — else any
/// such fair pocket with an odd tonnage (the brief's fallback).
fn fair7(all: &[Pick]) -> Pick {
    let soft = |p: &Pick| p.footing != Footing::Rock;
    if all
        .iter()
        .any(|p| soft(p) && p.families() == 1 && p.classes[0] == Word::Fair && p.tonnes[0] == 7)
    {
        pick(all, "FAIR7", |p| {
            soft(p) && p.families() == 1 && p.classes[0] == Word::Fair && p.tonnes[0] == 7
        })
    } else {
        pick(all, "FAIR7 (fallback)", |p| {
            soft(p) && p.families() == 1 && p.classes[0] == Word::Fair && p.tonnes[0] % 2 == 1
        })
    }
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

/// **`BARREN`**: every family barren, on soft ground.
fn barren(all: &[Pick], not: &[&Pick]) -> Pick {
    pick(all, "BARREN", |p| {
        p.footing != Footing::Rock && p.families() == 0 && not.iter().all(|n| n.pocket != p.pocket)
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

/// A deposit's pocket cursor.
fn cursor_of(tw: &mut TestWorld, deposit: OreDepositId) -> Option<PocketCursor> {
    let pockets = handles(tw).families.pockets;
    typed(tw, &pockets, &deposit)
}

/// A principal's chart row for a site.
fn chart_of(tw: &mut TestWorld, who: ObjId, entry: &str) -> Option<ChartRecord> {
    let h = handles(tw);
    typed(
        tw,
        &h.families.charts,
        &ChartKey {
            principal: who,
            site: cid(entry),
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

/// A block's vault on `entry`.
fn block_vault(tw: &TestWorld, entry: &str, letter: char) -> VaultId {
    VaultId(
        port_map(tw, entry)
            .site
            .and_then(|row| row.vaults.get(&letter.to_string()).copied())
            .expect("the block vault"),
    )
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

/// **Finish `tv`'s act** (launch, scan, sounding): to its due instant (the fire there queues the
/// finish, landed), then one more fire for the tell; what `who` heard over both.
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

/// **Land `tv`'s cycle in flight**: to its due instant (the fire there queues the cycle's
/// finish, which is checked to name the run's ordinal and cycle and is landed), then one more fire
/// for the tell; what `who` heard over both, and the finish's submission.
fn cycle(rig: &mut Rig, tv: ObjId, who: ObjId) -> (String, AuthoritativeSubmit) {
    let run = run_of(&mut rig.tw, tv).expect("a run in flight");
    let (events, queued) = advance_to(rig, run.due_ic);
    let mut heard = said(&rig.tw, &events, who);
    let done = finishes(&queued);
    assert_eq!(done.len(), 1, "one finish at the cycle's due: {queued:?}");
    let submit = done[0].clone();
    assert_eq!(
        submit.parameters,
        json!({ "tv": tv.0.to_string(), "ordinal": run.ordinal, "cycle": run.k })
    );
    assert_eq!(
        submit.key.as_str(),
        format!("tacsim.surface/{}/{}/cycle/{}", tv.0, run.ordinal, run.k)
    );
    land_all(rig, &queued);
    let (events, queued) = fire(rig);
    land_all(rig, &queued);
    heard.push('\n');
    heard.push_str(&said(&rig.tw, &events, who));
    (heard, submit)
}

/// How many times `line` appears in `text`.
fn count(text: &str, line: &str) -> usize {
    text.matches(line).count()
}

/// Staff place `tv` `Direct` on `entry`'s domain at site cell `(x, y)`'s centre, facing south.
fn to_cell_on(rig: &mut Rig, entry: &str, tv: ObjId, (x, y): (i64, i64)) {
    let domain = mush_sim::DomainId(port_map(&rig.tw, entry).domain.expect("a domain"));
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

/// Staff move `tv` to Ahuna's site cell `(x, y)`.
fn to_cell(rig: &mut Rig, tv: ObjId, cell: (i64, i64)) {
    to_cell_on(rig, AHUNA, tv, cell);
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

/// Put a rig's charge at `value_cu` out of the bay at the world's IC now (its record otherwise
/// kept, a run in flight included).
fn put_charge(rig: &mut Rig, tv: ObjId, value_cu: u32) -> Charge {
    let charge = Charge {
        value_cu,
        at_ic: ic_now(&rig.tw),
        in_bay: false,
    };
    let mut record = record_of(&mut rig.tw, tv).unwrap_or_else(|| RigRecord::fresh(tv, &battery()));
    record.charge = charge;
    put_record(rig, record);
    charge
}

/// Launch rig `slot` of `pilot` and finish it (it stands at the ramp).
fn launched(rig: &mut Rig, pilot: ObjId, slot: u8, tv: ObjId) {
    let _ = act_go(rig, pilot, &format!("+tv/{slot} launch go"));
    let _ = finish(rig, tv, pilot);
}

/// Launch `tv`, put it on `at`'s stand and sound there (its operator reads the pocket); the
/// sounding's lines.
fn sounded_at(rig: &mut Rig, pilot: ObjId, slot: u8, tv: ObjId, at: &Pick) -> String {
    to_cell(rig, tv, at.stand);
    let _ = act_go(rig, pilot, &format!("+tv/{slot} sound quiet go"));
    finish(rig, tv, pilot)
}

/// The TV's driving figures, from content.
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

/// The reserve floor (5,000 cu on the Mk1).
fn floor() -> u32 {
    let b = battery();
    b.capacity_cu * b.reserve_permille / 1_000
}

/// Ahuna Flank's home field for the Dray-P's ramp cell, as the world built it.
fn home_field(tw: &TestWorld) -> Arc<HomeField> {
    tw.world()
        .module_state::<GeologyState>()
        .and_then(|state| state.home_field(&cid(AHUNA), RAMP_CELL))
        .expect("the home field is built")
}

/// The charge home from a stand a rig occupies (the plan's figure: 0 beside the ramp).
fn home_cu(tw: &TestWorld, stand: (i64, i64)) -> u32 {
    if (stand.0 - RAMP_CELL.0).abs() <= 1 && (stand.1 - RAMP_CELL.1).abs() <= 1 {
        return 0;
    }
    home_cu_here(
        &home_field(tw),
        stand,
        &rates(tw),
        mush_atlas::physics::LocomotionTable::shipped(),
    )
}

/// Whole percent of the Mk1 battery.
fn pct(cu: u32) -> u32 {
    cu * 100 / battery().capacity_cu
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

/// **The `site.map` frame `who` gets** through the feed's own query.
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
    let width = usize::try_from(frame["width"].as_u64().expect("a width")).expect("fits");
    let height = usize::try_from(frame["height"].as_u64().expect("a height")).expect("fits");
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

/// The ore glyphs no second principal's grid may hold.
const ORE_GLYPHS: [char; 5] = ['1', '2', '3', '+', '.'];

/// A rig's hopper load and capacity words (`hopper_text`).
fn hopper_words(kg: u64) -> String {
    if kg == 0 {
        "empty".to_owned()
    } else {
        format!("{} of 12 t", kg / 1_000)
    }
}

/// The cell byte of `pocket` in a chart row.
fn cell_byte(row: &ChartRecord, pocket: (i64, i64)) -> Option<u8> {
    let (letter, index) = tacsim::surface::address::pocket_in_block(pocket.0, pocket.1)?;
    row.blocks
        .get(letter.to_string().as_str())
        .and_then(|block| block.cell(index))
}

/// The Core stamps in the row's block of `pocket`.
fn core_stamps(row: &ChartRecord, pocket: (i64, i64)) -> Vec<i64> {
    let Some((letter, _)) = tacsim::surface::address::pocket_in_block(pocket.0, pocket.1) else {
        return Vec::new();
    };
    row.blocks
        .get(letter.to_string().as_str())
        .map(|block| {
            block
                .provenance
                .iter()
                .filter_map(|entry| match entry {
                    tacsim::surface::ChartProvenance::Local {
                        instrument: Instrument::Core,
                        at,
                        ..
                    } => Some(*at),
                    tacsim::surface::ChartProvenance::Local { .. } => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The expected yields of a class's tonnes a cycle over `tonnes`.
fn yields_of(per: u32, tonnes: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut left = tonnes;
    while left > 0 {
        let take = left.min(per);
        out.push(take);
        left -= take;
    }
    out
}

/// The rig's charge value.
fn value_of(tw: &mut TestWorld, tv: ObjId) -> u32 {
    record_of(tw, tv)
        .map(|record| record.charge.value_cu)
        .expect("a record")
}

// ---------------------------------------------------------------------------
// M1 (A1). A fair seven-tonne pocket
// ---------------------------------------------------------------------------

/// **M1 — a fair seven-tonne pocket yields 2, 2, 2, 1, then is worked out** (A1; §1 beat 8, §6):
/// after a sounding from `FAIR7`'s stand, `+tv/1 mine` prints the preview from the operator's own
/// reading — `2/2/2/1 = 7 t`, the run's seconds, the hopper `0 → 7 of 12 t`, the hold's room after
/// (the hold's mass room less 7 t, from the hull's own compartment) and the charge words — and
/// queues nothing; `go` queues exactly one start (`act: mine`); landed, the record's run is cycle 1
/// of 4 with 8,000 cu reserved and due 1,800 IC s on, and the metal cursor's row of the pocket is
/// locked by the rig with nothing drawn; four cycles land at their dues, each keyed `…/cycle/k`,
/// and the operator hears exactly the four cycle lines (`hopper 2 of 12 t` … `7 of 12 t`), the last
/// with `RUN_WORKED_OUT`; the block vault fell by 7, the hopper rose by 7, the cursor drew 7 with no
/// lock, the run is gone, the charge fell by 8,000, the operator's cell reads `TIER_R2 | PIT_SEEN`
/// with Core stamps; a fresh `+tv/1 mine` refuses `MINE_WORKED_OUT`.
#[test]
#[allow(clippy::too_many_lines)]
fn sm7_a_fair_seven_tonne_pocket_yields_two_two_two_one_then_worked_out() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let all = picks(&mut rig.tw, AHUNA);
    let fair = fair7(&all);
    launched(&mut rig, pilot, 1, tv);
    let _ = sounded_at(&mut rig, pilot, 1, tv, &fair);
    // The operator's R2 text of the pocket through the render door, as the sounding left it.
    let sounded_row = chart_of(&mut rig.tw, pilot, AHUNA);
    let read_sounded = tacsim::surface::pocket_reading(
        rig.tw.world(),
        &cid(AHUNA),
        sounded_row.as_ref(),
        fair.pocket,
    );
    assert!(
        read_sounded.starts_with(&fair.addr) && read_sounded != NO_READING,
        "{read_sounded}"
    );
    let tonnes = u32::from(fair.tonnes[0]);
    let yields = yields_of(2, tonnes);
    let cycles = yields.len();
    let before = value_of(&mut rig.tw, tv);
    let home = home_cu(&rig.tw, fair.stand);
    let cost = 2_000 * u32::try_from(cycles).expect("fits");
    let (out, queued) = command(&mut rig, pilot, "+tv/1 mine");
    assert!(queued.is_empty(), "a preview queues nothing");
    let joined: Vec<String> = yields.iter().map(u32::to_string).collect();
    let head = format!(
        "Rig 1 (OWP-TV-01) would mine {} fair metal ore: {cycles} cycles, {} = {tonnes} t, {} IC s",
        fair.addr,
        joined.join("/"),
        1_800 * cycles
    );
    let room = (ore_mass_limit(&rig.tw) - u64::from(tonnes) * 1_000) / 1_000;
    let tail = format!(
        "; hopper 0 → {tonnes} of 12 t; hold room after: {room} t; charge {} → {} % ({} % after the \
         trip home). +tv/1 mine go starts it.",
        pct(before),
        pct(before - cost),
        pct(before - cost - home)
    );
    assert!(
        out.trim().starts_with(&head) && out.trim().ends_with(&tail),
        "{out}\n{head}\n{tail}"
    );
    if tonnes == 7 {
        assert!(
            out.contains(": 4 cycles, 2/2/2/1 = 7 t, 7200 IC s"),
            "{out}"
        );
    }
    // The go.
    let now = ic_now(&rig.tw);
    let out = act_go(&mut rig, pilot, "+tv/1 mine go");
    assert!(
        out.contains(&format!(
            "Rig 1 starts drilling {}: the first cycle ",
            fair.addr
        )),
        "{out}"
    );
    let run = run_of(&mut rig.tw, tv).expect("the run");
    assert_eq!(
        (run.cycles, run.k, run.reserved_cu, run.due_ic),
        (u8::try_from(cycles).expect("fits"), 1, cost, now + 1_800)
    );
    assert!(
        record_of(&mut rig.tw, tv).is_some_and(|r| r.act.is_none()),
        "the act slot is free"
    );
    let deposit = fair.deposit(AHUNA, OreFamily::Metals);
    let cursor = cursor_of(&mut rig.tw, deposit).expect("the metal cursor");
    let row = cursor
        .row(fair.number())
        .cloned()
        .expect("the pocket's row");
    assert_eq!(row.drawn_t, 0);
    assert_eq!(
        row.lock.map(|lock| (lock.tv, lock.ordinal)),
        Some((tv, run.ordinal))
    );
    let vault = block_vault(&rig.tw, AHUNA, fair.letter());
    let vault_before = pool(&mut rig.tw, vault, "metal_ore");
    let hopper_vault = hopper(&rig.tw, tv);
    assert_eq!(pool(&mut rig.tw, hopper_vault, "metal_ore"), 0);
    // The cycles.
    let mut heard = String::new();
    let mut held = 0_u32;
    for (k, y) in yields.iter().enumerate() {
        let due = run_of(&mut rig.tw, tv).expect("a run").due_ic;
        assert_eq!(due, now + 1_800 * i64::try_from(k + 1).expect("fits"));
        let (lines, _) = cycle(&mut rig, tv, pilot);
        held += y;
        let mut line = CYCLE_DONE
            .replace("{n}", "1")
            .replace("{k}", &(k + 1).to_string())
            .replace("{cycles}", &cycles.to_string())
            .replace("{t}", &y.to_string())
            .replace("{ore}", "metal ore")
            .replace("{hopper}", &hopper_words(u64::from(held) * 1_000));
        if k + 1 == cycles {
            line.push_str(
                &RUN_WORKED_OUT
                    .replace("{addr}", &fair.addr)
                    .replace("{ore}", "metal ore"),
            );
        }
        assert_eq!(count(&lines, &line), 1, "{lines}\nwanted: {line}");
        heard.push_str(&lines);
    }
    assert_eq!(
        count(&heard, ", cycle "),
        cycles,
        "exactly the cycle lines: {heard}"
    );
    assert_eq!(
        pool(&mut rig.tw, vault, "metal_ore"),
        vault_before - u64::from(tonnes)
    );
    assert_eq!(
        pool(&mut rig.tw, hopper_vault, "metal_ore"),
        u64::from(tonnes)
    );
    let cursor = cursor_of(&mut rig.tw, deposit).expect("the metal cursor");
    let row = cursor
        .row(fair.number())
        .cloned()
        .expect("the pocket's row");
    assert_eq!((row.drawn_t, row.lock), (tonnes, None));
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert!(record.run.is_none());
    assert_eq!(record.charge.value_cu, before - cost);
    let last = record.last.expect("the outcome");
    assert_eq!(last.kind, ActKind::Mine);
    let mined = last.mined.expect("what the cycle did");
    assert_eq!(mined.end, Some(RunEnd::WorkedOut));
    let chart = chart_of(&mut rig.tw, pilot, AHUNA).expect("the operator's row");
    assert_eq!(cell_byte(&chart, fair.pocket), Some(TIER_R2 | PIT_SEEN));
    assert_eq!(cell_tier(TIER_R2 | PIT_SEEN), TIER_R2);
    assert!(!core_stamps(&chart, fair.pocket).is_empty(), "a Core stamp");
    // The SM.7 lead's pre-commit round, item 8: the pit bit beside the tier reads the same R2 text
    // through the render door — the core is a reading, never a second kind of cell.
    assert_eq!(
        tacsim::surface::pocket_reading(rig.tw.world(), &cid(AHUNA), Some(&chart), fair.pocket),
        read_sounded,
        "the cored cell reads its R2 text"
    );
    // Worked out.
    let (out, queued) = command(&mut rig, pilot, "+tv/1 mine");
    assert_eq!(
        out.trim(),
        MINE_WORKED_OUT
            .replace("{addr}", &fair.addr)
            .replace("{ore}", "metal ore")
    );
    assert!(queued.is_empty());
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M2 (A2). The sweep finds the books balanced
// ---------------------------------------------------------------------------

/// Empty `tv`'s hopper onto the spoil when it holds anything (its operator's dump, finished).
fn emptied(rig: &mut Rig, pilot: ObjId, slot: u8, tv: ObjId) {
    let vault = hopper(&rig.tw, tv);
    if pool(&mut rig.tw, vault, "metal_ore") > 0 {
        let _ = act_go(rig, pilot, &format!("+tv/{slot} dump go"));
        let _ = finish(rig, tv, pilot);
    }
}

/// **M2 — the sweep finds the books balanced** (A2's world half; §3.4; lead ruling 6): a read run
/// worked out (`FAIR7`), then a blind core of an unread ore pocket and a blind core of a barren
/// one, then industry's sweep: nothing quarantined, and for every worked deposit the cursor's
/// total — `SurfaceDraws`' answer — equals `initial_reserve − pool`. (The 10,000 cycles are
/// `surface::mine`'s `sm7_ten_thousand_cycles_conserve`, the arithmetic half.)
#[test]
fn sm7_the_sweep_finds_the_books_balanced() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let all = picks(&mut rig.tw, AHUNA);
    let fair = fair7(&all);
    let ore = pick(&all, "UNREAD_ORE", |p| {
        p.footing != Footing::Rock
            && p.families() == 1
            && p.classes[0] != Word::Barren
            && p.pocket != fair.pocket
    });
    let empty = barren(&all, &[&fair, &ore]);
    launched(&mut rig, pilot, 1, tv);
    let _ = sounded_at(&mut rig, pilot, 1, tv, &fair);
    let _ = act_go(&mut rig, pilot, "+tv/1 mine go");
    while run_of(&mut rig.tw, tv).is_some() {
        let _ = cycle(&mut rig, tv, pilot);
    }
    for at in [&ore, &empty] {
        emptied(&mut rig, pilot, 1, tv);
        to_cell(&mut rig, tv, at.stand);
        let out = act_go(&mut rig, pilot, "+tv/1 mine go");
        assert!(out.contains("starts coring"), "{out}");
        let _ = cycle(&mut rig, tv, pilot);
    }
    sweep(&mut rig.tw, 61);
    let quarantined = rig
        .tw
        .world()
        .module_state::<mush_industry::IndustryStore>()
        .expect("the industry store")
        .quarantined();
    assert!(quarantined.is_empty(), "{quarantined:?}");
    let families = rig
        .tw
        .world()
        .module_state::<mush_industry::IndustryHandles>()
        .expect("industry")
        .0
        .clone();
    for at in [&fair, &ore] {
        let deposit = at.deposit(AHUNA, OreFamily::Metals);
        let body = typed(&mut rig.tw, &families.deposits, &deposit).expect("the deposit");
        let cursor = cursor_of(&mut rig.tw, deposit).expect("a drawn cursor");
        let at_vault = block_vault(&rig.tw, AHUNA, at.letter());
        let left = pool(&mut rig.tw, at_vault, "metal_ore");
        assert_eq!(
            cursor.drawn_total(),
            Some(body.initial_reserve() - left),
            "{}",
            at.addr
        );
        assert!(cursor.drawn_total().is_some_and(|t| t > 0));
    }
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M3 (A3, A4). The hopper and the hold bound every run
// ---------------------------------------------------------------------------

/// **M3 — the hopper and the hold bound every run** (A3, A4; §6, §19 item 1; lead ruling 4): (a)
/// rig 1 on `RICH` with 6 t already in its hopper: the bare preview stops at the hopper, two
/// cycles of 3 t; `+tv/1 mine 3` refuses `MINE_NO_FIT` naming the count that fits ("the hopper is
/// full after the second — +tv/1 mine 2"); the hopper filled to 12 t refuses `MINE_HOPPER_FULL`;
/// (b) the hold seeded to leave 5 t unreserved (rig 1's 12 t counted): rig 2 on a fair pocket
/// previews two cycles (4 t) and `+tv/2 mine 3` names the hold. (c), two rigs filling one hold, is
/// `sm7_the_hold_counts_every_outstanding_cycle`; (d), carbon bound by packed volume before mass,
/// is `surface::mine`'s `sm7_the_fit_names_its_bound`.
#[test]
#[allow(clippy::too_many_lines)]
fn sm7_the_hopper_and_the_hold_bound_every_run() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 2);
    let all = picks(&mut rig.tw, AHUNA);
    let rich = pick(&all, "RICH", |p| {
        p.footing != Footing::Rock && p.classes[0] == Word::Rich && p.tonnes[0] >= 12
    });
    let fair = fair7(&all);
    launched(&mut rig, pilot, 1, tvs[0]);
    launched(&mut rig, pilot, 2, tvs[1]);
    let _ = sounded_at(&mut rig, pilot, 1, tvs[0], &rich);
    let _ = sounded_at(&mut rig, pilot, 2, tvs[1], &fair);
    // (a) The hopper.
    let vault = hopper(&rig.tw, tvs[0]);
    seed_pool(&mut rig, vault, "metal_ore", 6, LotId(8_801));
    let (out, queued) = command(&mut rig, pilot, "+tv/1 mine");
    assert!(queued.is_empty());
    assert!(
        out.contains(&format!(
            "would mine {} rich metal ore: 2 cycles, 3/3 = 6 t, 3600 IC s",
            rich.addr
        )) && out.contains("; hopper 6 → 12 of 12 t; "),
        "{out}"
    );
    let (out, queued) = command(&mut rig, pilot, "+tv/1 mine 3");
    assert!(queued.is_empty());
    assert_eq!(
        out.trim(),
        "3 cycles do not fit: the hopper is full after the second — +tv/1 mine 2, or +tv/1 mine \
         with no number."
    );
    let (out, queued) = command(&mut rig, pilot, "+tv/1 mine 3 go");
    assert!(
        out.starts_with("3 cycles do not fit") && queued.is_empty(),
        "go commits nothing: {out}"
    );
    assert!(run_of(&mut rig.tw, tvs[0]).is_none(), "no run written");
    let vault = hopper(&rig.tw, tvs[0]);
    seed_pool(&mut rig, vault, "metal_ore", 6, LotId(8_803));
    let (out, queued) = command(&mut rig, pilot, "+tv/1 mine");
    assert!(queued.is_empty());
    assert_eq!(
        out.trim(),
        MINE_HOPPER_FULL
            .replace("{n}", "1")
            .replace("{hopper}", "12 of 12 t")
    );
    // (b) The hold: 5 t left once rig 1's 12 t are counted.
    let limit_t = ore_mass_limit(&rig.tw) / 1_000;
    let vault = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, vault, "metal_ore", limit_t - 12 - 5, LotId(8_805));
    let (out, queued) = command(&mut rig, pilot, "+tv/2 mine");
    assert!(queued.is_empty());
    assert!(
        out.contains(&format!(
            "would mine {} fair metal ore: 2 cycles, 2/2 = 4 t, 3600 IC s",
            fair.addr
        )) && out.contains("; hold room after: 1 t; "),
        "{out}"
    );
    let (out, queued) = command(&mut rig, pilot, "+tv/2 mine 3");
    assert!(queued.is_empty());
    assert_eq!(
        out.trim(),
        "3 cycles do not fit: the hold's room is used after the second — +tv/2 mine 2, or +tv/2 \
         mine with no number."
    );
    no_refused_writes(&rig.tw);
}

/// The hold's unreserved room the `site.map` frame shows `who` (whole tonnes), the one function
/// both the map and the plan read (`port::ship::hold_unreserved`).
fn hold_room_t(rig: &mut Rig, who: ObjId) -> u64 {
    map_frame(rig, who, json!({ "view": "site" }))["site"]["hold"]["room_t"]
        .as_u64()
        .expect("the hold's room")
}

/// **The hold counts every outstanding cycle** (M3 (c), A3; §19 item 1, decision R8): two rigs of
/// one hull on two fair pockets, the hold seeded to leave 6 t. Rig 1 starts three cycles (6 t);
/// rig 2's preview and start both refuse `MINE_HOLD_FULL`. The unreserved room reads 6 t before,
/// 0 the moment rig 1's run starts (its three cycles reserved) and 0 after each cycle (each moves
/// 2 t from outstanding into the hopper); after every fire the hold's load, both hoppers and every
/// outstanding cycle stay within the hold's mass limit and each hopper within 12,000 kg.
#[test]
#[allow(clippy::too_many_lines)]
fn sm7_the_hold_counts_every_outstanding_cycle() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 2);
    let all = picks(&mut rig.tw, AHUNA);
    let fair = fair7(&all);
    let other = fair_other(&all, "FAIR (rig 2)", &[&fair]);
    launched(&mut rig, pilot, 1, tvs[0]);
    launched(&mut rig, pilot, 2, tvs[1]);
    let _ = sounded_at(&mut rig, pilot, 1, tvs[0], &fair);
    let _ = sounded_at(&mut rig, pilot, 2, tvs[1], &other);
    let limit = ore_mass_limit(&rig.tw);
    let hold = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, hold, "metal_ore", limit / 1_000 - 6, LotId(8_901));
    assert_eq!(hold_room_t(&mut rig, pilot), 6, "before");
    let (out, _) = command(&mut rig, pilot, "+tv/1 mine");
    assert!(out.contains(": 3 cycles, 2/2/2 = 6 t, "), "{out}");
    let _ = act_go(&mut rig, pilot, "+tv/1 mine go");
    assert_eq!(hold_room_t(&mut rig, pilot), 0, "the run's cycles reserved");
    let full = MINE_HOLD_FULL.replace("{name}", "Salt Moth");
    for line in ["+tv/2 mine", "+tv/2 mine go"] {
        let (out, queued) = command(&mut rig, pilot, line);
        assert_eq!(out.trim(), full, "{line}");
        assert!(queued.is_empty());
    }
    let check = |rig: &mut Rig| {
        let held = pool(&mut rig.tw, hold, "metal_ore") * 1_000;
        let hoppers: u64 = tvs
            .iter()
            .map(|tv| {
                let vault = hopper(&rig.tw, *tv);
                pool(&mut rig.tw, vault, "metal_ore") * 1_000
            })
            .sum();
        let outstanding: u64 = tvs
            .iter()
            .filter_map(|tv| run_of(&mut rig.tw, *tv))
            .map(|run| run.outstanding_t() * 1_000)
            .sum();
        assert!(
            held + hoppers + outstanding <= limit,
            "{held} {hoppers} {outstanding}"
        );
        for tv in &tvs {
            let vault = hopper(&rig.tw, *tv);
            assert!(pool(&mut rig.tw, vault, "metal_ore") * 1_000 <= 12_000);
        }
    };
    check(&mut rig);
    while run_of(&mut rig.tw, tvs[0]).is_some() {
        let _ = cycle(&mut rig, tvs[0], pilot);
        check(&mut rig);
        assert_eq!(
            hold_room_t(&mut rig, pilot),
            0,
            "outstanding moved to the hopper"
        );
    }
    let vault = hopper(&rig.tw, tvs[0]);
    assert_eq!(pool(&mut rig.tw, vault, "metal_ore"), 6);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M4 (A5). A blind core never tells the class
// ---------------------------------------------------------------------------

/// `text` with `addr` and `mark` read as placeholders.
fn neutral(text: &str, addr: &str, mark: &str) -> String {
    text.replace(addr, "<addr>").replace(mark, "<mark>")
}

/// **M4 — an unread pocket cores blind and never tells the class** (A5; lead ruling 2, SM.6's
/// D14): two operators' rigs, A's on `BARREN` and B's on an unread metal pocket, at charges that
/// leave the same whole percents after the trip home. With 2 t of room (each hopper holding 10 t)
/// both refuse `MINE_BLIND_ROOM` identically; emptied, the previews (`MINE_PREVIEW_BLIND`, no pit
/// clause), `+tv/1 mine 3` (`MINE_BLIND_COUNT`), the filed lines (`CORE_FILED`), the runs' views
/// and due instants, the projections' run views and the `+tv` status's coring lines are
/// byte-identical with the address and the mark substituted; both cores land at one instant and
/// spend one cycle's charge; the ore core moved its class's tonnes and locked its row at the
/// start, the barren core moved nothing and wrote no cursor row; both rows read `TIER_R2 |
/// PIT_SEEN` with a Core stamp, and `CORED` prints each operator's own new reading.
#[test]
#[allow(clippy::too_many_lines)]
fn sm7_an_unread_pocket_cores_blind_and_never_tells_the_class() {
    let mut rig = stocked();
    let (a, _, a_tvs) = on_pad(&mut rig, 1);
    let (b, _, b_tvs) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    let (a_tv, b_tv) = (a_tvs[0], b_tvs[0]);
    let all = picks(&mut rig.tw, AHUNA);
    // A barren pocket and an unread single-metal pocket whose ways home differ by under 1,000 cu
    // (so the whole percents after the trip home agree).
    let mut chosen: Option<(Pick, Pick, u32, u32)> = None;
    for empty in all
        .iter()
        .filter(|p| p.footing != Footing::Rock && p.families() == 0)
        .take(40)
    {
        let h_empty = home_cu(&rig.tw, empty.stand);
        if let Some(ore) = all.iter().find(|p| {
            p.footing != Footing::Rock
                && p.families() == 1
                && p.classes[0] != Word::Barren
                && home_cu(&rig.tw, p.stand).abs_diff(h_empty) < 1_000
        }) {
            chosen = Some((
                empty.clone(),
                ore.clone(),
                h_empty,
                home_cu(&rig.tw, ore.stand),
            ));
            break;
        }
    }
    let (empty, ore, h_empty, h_ore) = chosen.expect("a barren and an ore pocket alike");
    println!(
        "BARREN: {} footing {:?}; UNREAD_ORE: {} classes {:?} tonnes {:?} footing {:?}",
        empty.addr, empty.footing, ore.addr, ore.classes, ore.tonnes, ore.footing
    );
    launched(&mut rig, a, 1, a_tv);
    launched(&mut rig, b, 1, b_tv);
    to_cell(&mut rig, a_tv, empty.stand);
    to_cell(&mut rig, b_tv, ore.stand);
    let value = 52_000 + h_empty.max(h_ore);
    let _ = put_charge(&mut rig, a_tv, value);
    let _ = put_charge(&mut rig, b_tv, value);
    let marks = ("OWP-TV-01", "OWP-TV-02");
    // 2 t of room: the blind room refuses, the same line for both.
    let vault = hopper(&rig.tw, a_tv);
    seed_pool(&mut rig, vault, "metal_ore", 10, LotId(9_001));
    let vault = hopper(&rig.tw, b_tv);
    seed_pool(&mut rig, vault, "metal_ore", 10, LotId(9_003));
    for line in ["+tv/1 mine", "+tv/1 mine go"] {
        let (a_out, a_q) = command(&mut rig, a, line);
        let (b_out, b_q) = command(&mut rig, b, line);
        assert_eq!(
            a_out.trim(),
            MINE_BLIND_ROOM.replace("{room}", "3"),
            "{line}"
        );
        assert_eq!(a_out, b_out, "{line}");
        assert!(a_q.is_empty() && b_q.is_empty(), "{line} queues nothing");
    }
    emptied(&mut rig, a, 1, a_tv);
    emptied(&mut rig, b, 1, b_tv);
    let _ = put_charge(&mut rig, a_tv, value);
    let _ = put_charge(&mut rig, b_tv, value);
    // The previews, the count, the filed lines.
    for line in ["+tv/1 mine", "+tv/1 mine 3", "+tv/1 mine metal"] {
        let (a_out, a_q) = command(&mut rig, a, line);
        let (b_out, b_q) = command(&mut rig, b, line);
        assert!(a_q.is_empty() && b_q.is_empty());
        assert_eq!(
            neutral(&a_out, &empty.addr, marks.0),
            neutral(&b_out, &ore.addr, marks.1),
            "{line}"
        );
        if line == "+tv/1 mine" {
            assert!(
                a_out.starts_with("No reading here: the first cycle will tell. ")
                    && a_out.starts_with(&MINE_PREVIEW_BLIND[..44])
                    && !a_out.contains("pit"),
                "{a_out}"
            );
            assert!(a_out.contains("3 t of room needed to core an unread pocket"));
        }
        if line == "+tv/1 mine 3" {
            assert_eq!(
                a_out.trim(),
                MINE_BLIND_COUNT.replace("{n}", "1").replace("{fam}", "")
            );
        }
    }
    let (a_out, a_q) = command(&mut rig, a, "+tv/1 mine go");
    let (b_out, b_q) = command(&mut rig, b, "+tv/1 mine go");
    assert!(a_out.starts_with("Rig 1 starts coring "), "{a_out}");
    assert_eq!(
        neutral(&a_out, &empty.addr, marks.0),
        neutral(&b_out, &ore.addr, marks.1)
    );
    assert_eq!((a_q.len(), b_q.len()), (1, 1));
    // The start outcomes' JSON: identical once the actor and the rig are substituted (the
    // SM.7 fixer's T8; nothing of the pocket, the class or the deposit is in it).
    let mut a_start = land_outcome(&mut rig, &a_q[0]).expect("A's core starts");
    let mut b_start = land_outcome(&mut rig, &b_q[0]).expect("B's core starts");
    assert_eq!(
        (a_start["actor"].clone(), a_start["tv"].clone()),
        (json!(a.0), json!(a_tv.0))
    );
    for start in [&mut a_start, &mut b_start] {
        start["actor"] = json!("<actor>");
        start["tv"] = json!("<tv>");
    }
    assert_eq!(a_start, b_start, "the start outcomes");
    assert_eq!(a_start["act"], json!("mine"));
    let (a_run, b_run) = (
        run_of(&mut rig.tw, a_tv).expect("A's core"),
        run_of(&mut rig.tw, b_tv).expect("B's core"),
    );
    assert_eq!(
        (
            a_run.ordinal,
            a_run.cycles,
            a_run.due_ic,
            a_run.reserved_cu,
            a_run.blind
        ),
        (
            b_run.ordinal,
            b_run.cycles,
            b_run.due_ic,
            b_run.reserved_cu,
            b_run.blind
        )
    );
    assert_eq!(a_run.reserve_t, vec![3]);
    assert_eq!(b_run.reserve_t, vec![3]);
    // The ore core locked its row at the start; the barren core wrote none.
    let ore_cursor = cursor_of(&mut rig.tw, ore.deposit(AHUNA, OreFamily::Metals))
        .expect("the ore pocket's cursor");
    assert!(
        ore_cursor
            .row(ore.number())
            .and_then(|row| row.lock.as_ref())
            .is_some_and(|lock| lock.tv == b_tv)
    );
    for family in OreFamily::ALL {
        if let Some(cursor) = cursor_of(&mut rig.tw, empty.deposit(AHUNA, family)) {
            assert!(cursor.row(empty.number()).is_none(), "{family:?}");
        }
    }
    // The status lines.
    let a_status = run(&mut rig.tw, a, "+tv");
    let b_status = run(&mut rig.tw, b, "+tv");
    let coring = |text: &str| -> String {
        text.lines()
            .find(|line| line.starts_with("  Coring "))
            .map(str::to_owned)
            .expect("a coring line")
    };
    assert_eq!(
        neutral(&coring(&a_status), &empty.addr, marks.0),
        neutral(&coring(&b_status), &ore.addr, marks.1)
    );
    // The due fire: the projections' run views, then the cores land.
    let (_, queued) = advance_to(&mut rig, a_run.due_ic);
    let a_view = projection(&rig.tw, a_tv).and_then(|rec| rec.run);
    assert!(a_view.is_some(), "the projection shows the core");
    assert_eq!(a_view, projection(&rig.tw, b_tv).and_then(|rec| rec.run));
    let done = finishes(&queued);
    assert_eq!(done.len(), 2, "both cores at one instant");
    land_all(&mut rig, &queued);
    let (events, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    let a_heard = said(&rig.tw, &events, a);
    let b_heard = said(&rig.tw, &events, b);
    assert_eq!(value_of(&mut rig.tw, a_tv), value - 2_000);
    assert_eq!(value_of(&mut rig.tw, b_tv), value - 2_000);
    let a_vault = hopper(&rig.tw, a_tv);
    let b_vault = hopper(&rig.tw, b_tv);
    assert_eq!(pool(&mut rig.tw, a_vault, "metal_ore"), 0, "nothing lifted");
    let lifted = u64::from(match ore.classes[0] {
        Word::Lean => 1_u8,
        Word::Fair => 2,
        Word::Rich => 3,
        Word::Barren => 0,
    })
    .min(u64::from(ore.tonnes[0]));
    assert_eq!(pool(&mut rig.tw, b_vault, "metal_ore"), lifted);
    for (who, at) in [(a, &empty), (b, &ore)] {
        let row = chart_of(&mut rig.tw, who, AHUNA).expect("a row");
        assert_eq!(cell_byte(&row, at.pocket), Some(TIER_R2 | PIT_SEEN));
        assert_eq!(core_stamps(&row, at.pocket), vec![a_run.due_ic]);
    }
    let barren_core = CORED
        .replace("{n}", "1")
        .replace("{reading}", &format!("{} barren", empty.addr))
        .replace("{lift}", NOTHING_LIFTED)
        .replace("{hopper}", "empty");
    assert_eq!(count(&a_heard, &barren_core), 1, "{a_heard}");
    let class = match ore.classes[0] {
        Word::Lean => "lean",
        Word::Fair => "fair",
        Word::Rich => "rich",
        Word::Barren => "barren",
    };
    let ore_core = CORED
        .replace("{n}", "1")
        .replace("{reading}", &format!("{} {class} metals", ore.addr))
        .replace("{lift}", &format!("{lifted} t {}", ore_words("metal_ore")))
        .replace("{hopper}", &hopper_words(lifted * 1_000));
    assert_eq!(count(&b_heard, &ore_core), 1, "{b_heard}");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M5 (A6). Stop
// ---------------------------------------------------------------------------

/// **M5 — `stop` lets the running cycle yield** (A6; §6; lead ruling 8, decision R7): a
/// four-cycle run; `+tv/1 mine stop` during cycle 1 queues one `mine_stop` start (committed with
/// no `go`) and says `MINE_STOPPING`; a second stop says `MINE_STOPPING_ALREADY` and queues
/// nothing; cycle 1 lands with its tonnes and `RUN_STOPPED`; the run is gone, the charge fell by
/// exactly one cycle's 2,000 cu, the lock is released, and no cycle 2 is ever submitted. With no
/// run, `stop` says `MINE_NOT_RUNNING`.
#[test]
fn sm7_stop_lets_the_running_cycle_yield() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let all = picks(&mut rig.tw, AHUNA);
    let fair = fair7(&all);
    launched(&mut rig, pilot, 1, tv);
    let _ = sounded_at(&mut rig, pilot, 1, tv, &fair);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 mine stop");
    assert_eq!(out.trim(), MINE_NOT_RUNNING.replace("{n}", "1"));
    assert!(queued.is_empty());
    let before = value_of(&mut rig.tw, tv);
    let _ = act_go(&mut rig, pilot, "+tv/1 mine go");
    let run = run_of(&mut rig.tw, tv).expect("the run");
    assert!(run.cycles >= 2);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 mine stop");
    assert!(
        out.trim().starts_with(
            &MINE_STOPPING
                .replace("{n}", "1")
                .replace("{addr}", &fair.addr)
                .replace(" done {when}.", "")
        ),
        "{out}"
    );
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].parameters["act"], json!("mine_stop"));
    assert_eq!(
        queued[0].key.as_str(),
        format!("tacsim.surface/{}/{}/stop", tv.0, run.ordinal)
    );
    land_all(&mut rig, &queued);
    assert!(run_of(&mut rig.tw, tv).is_some_and(|run| run.stop));
    let (out, queued) = command(&mut rig, pilot, "+tv/1 mine stop");
    assert_eq!(out.trim(), MINE_STOPPING_ALREADY.replace("{n}", "1"));
    assert!(queued.is_empty());
    let status = crate::run(&mut rig.tw, pilot, "+tv/1");
    assert!(status.contains("; it stops after this cycle."), "{status}");
    let (heard, _) = cycle(&mut rig, tv, pilot);
    let line = CYCLE_DONE
        .replace("{n}", "1")
        .replace("{k}", "1")
        .replace("{cycles}", &run.cycles.to_string())
        .replace("{t}", "2")
        .replace("{ore}", "metal ore")
        .replace("{hopper}", "2 of 12 t")
        + &*RUN_STOPPED.replace("{n}", "1");
    assert_eq!(count(&heard, &line), 1, "{heard}\nwanted: {line}");
    assert!(run_of(&mut rig.tw, tv).is_none());
    assert_eq!(
        value_of(&mut rig.tw, tv),
        before - 2_000,
        "one cycle, never refunded"
    );
    let cursor = cursor_of(&mut rig.tw, fair.deposit(AHUNA, OreFamily::Metals)).expect("drawn");
    assert_eq!(
        cursor
            .row(fair.number())
            .map(|row| (row.drawn_t, row.lock.is_none())),
        Some((2, true))
    );
    for _ in 0..3 {
        let at = ic_now(&rig.tw) + 1_800;
        let (_, queued) = advance_to(&mut rig, at);
        assert!(finishes(&queued).is_empty(), "no cycle 2");
    }
    let vault = hopper(&rig.tw, tv);
    assert_eq!(pool(&mut rig.tw, vault, "metal_ore"), 2);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M6 (A7). A replayed cycle
// ---------------------------------------------------------------------------

/// **M6 — a replayed cycle is a HIT** (A7; decision R2; the SM.7 fixer's T1): the cycle-1
/// finish the tick submits at the due (`S1`) is held unlanded across a restart; the restarted
/// tick submits the same cycle again (`S2`) with the same key, parameters, actor and action — so
/// the same ledger identity (`ActionCoordinator::command_ingress`'s digest inputs) — and `S2`
/// lands. `S1` redelivered is then the harness ledger's HIT, answered from the record with
/// nothing prepared: the pools, the cursor, the charge and the record are unchanged. Past the
/// ledger, the prepare's own guard refuses it `Conflict` "that cycle is already finished" (the
/// second guard), and so does a finish of `(ordinal, 1)` landed after cycle 2's due — each with
/// nothing moved.
#[test]
#[allow(clippy::too_many_lines)]
fn sm7_a_replayed_cycle_is_a_hit() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let all = picks(&mut rig.tw, AHUNA);
    let fair = fair7(&all);
    launched(&mut rig, pilot, 1, tv);
    let _ = sounded_at(&mut rig, pilot, 1, tv, &fair);
    let _ = act_go(&mut rig, pilot, "+tv/1 mine go");
    let run = run_of(&mut rig.tw, tv).expect("the run");
    // S1, held.
    let (_, queued) = advance_to(&mut rig, run.due_ic);
    let held = finishes(&queued);
    assert_eq!(held.len(), 1, "one finish at the due: {queued:?}");
    let s1 = held[0].clone();
    assert_eq!(
        s1.key.as_str(),
        format!("tacsim.surface/{}/{}/cycle/1", tv.0, run.ordinal)
    );
    // The restart: the tick's retry counts are RAM, so the first fire after it submits the cycle
    // afresh (staff placement is no persisted write, so the rig is placed again first).
    let mut rig = reboot(rig, &[pilot]);
    to_cell(&mut rig, tv, fair.stand);
    let (_, queued) = fire(&mut rig);
    let again = finishes(&queued);
    assert_eq!(
        again.len(),
        1,
        "the restarted tick submits the cycle: {queued:?}"
    );
    let s2 = again[0].clone();
    assert_eq!(
        ledger_identity(&s2),
        ledger_identity(&s1),
        "one cycle, one identity across the restart"
    );
    land_all(&mut rig, &queued);
    let deposit = fair.deposit(AHUNA, OreFamily::Metals);
    let vault = block_vault(&rig.tw, AHUNA, fair.letter());
    let hopper_vault = hopper(&rig.tw, tv);
    let snapshot = |rig: &mut Rig| {
        (
            record_of(&mut rig.tw, tv),
            cursor_of(&mut rig.tw, deposit),
            pool(&mut rig.tw, vault, "metal_ore"),
            pool(&mut rig.tw, hopper_vault, "metal_ore"),
        )
    };
    let before = snapshot(&mut rig);
    assert_eq!(before.3, 2, "cycle 1 moved its 2 t");
    assert_eq!(
        before.0.as_ref().and_then(|r| r.run.as_ref()).map(|r| r.k),
        Some(2),
        "cycle 2 armed"
    );
    // S1 redelivered: the HIT.
    assert_eq!(land(&mut rig, &s1), Err(HIT.to_owned()), "the ledger's HIT");
    assert_eq!(snapshot(&mut rig), before, "nothing moved twice");
    // Past the ledger: the prepare's `(ordinal, k)` guard.
    let refused = land_past_the_ledger(&mut rig, &s1).expect_err("a replay");
    assert!(
        refused.contains("Conflict") && refused.contains("that cycle is already finished"),
        "{refused}"
    );
    assert_eq!(snapshot(&mut rig), before, "nothing moved twice");
    // A finish of `(ordinal, 1)` after cycle 2's due: the same guard, nothing moved.
    let run = run_of(&mut rig.tw, tv).expect("cycle 2 armed");
    let at = run.due_ic + 1;
    let rate = i64::from(world_rate(&rig.tw).max(1));
    let real = (at - ic_now(&rig.tw) + rate - 1) / rate;
    let t = rig.tw.world().now().saturating_add(real.max(1));
    rig.tw.set_now(t);
    let refused = land_past_the_ledger(&mut rig, &s1).expect_err("an old cycle");
    assert!(
        refused.contains("Conflict") && refused.contains("that cycle is already finished"),
        "{refused}"
    );
    assert_eq!(snapshot(&mut rig), before, "nothing moved by the old cycle");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M7 (A8). A second rig on the pocket
// ---------------------------------------------------------------------------

/// **M7 — a second rig on the pocket refuses with the public line** (A8; lead ruling 3, decision
/// R4): A's rig 1 drills `TWO` (an ore pocket of two families) and both families' cursor rows
/// carry its lock; B's rig on `TWO` hears `MINE_LOCKED`, preview and `go` alike, nothing queued.
/// B's rig then cores `BARREN` (no cursor row anywhere) and A's rig 2 there hears the same line
/// byte for byte (the run record is what refuses). A **stale** lock — A's rig 1's run removed by a
/// staff edit of its record, the record closed without a release — is taken by A's rig 2's start:
/// the rows' lock is rig 2's.
#[test]
#[allow(clippy::too_many_lines)]
fn sm7_a_second_rig_on_the_pocket_refuses_with_the_public_line() {
    let mut rig = stocked();
    let (a, _, a_tvs) = on_pad(&mut rig, 2);
    let (b, _, b_tvs) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    let all = picks(&mut rig.tw, AHUNA);
    let two = pick(&all, "TWO", |p| {
        p.footing != Footing::Rock && p.families() == 2
    });
    let empty = barren(&all, &[&two]);
    launched(&mut rig, a, 1, a_tvs[0]);
    launched(&mut rig, a, 2, a_tvs[1]);
    launched(&mut rig, b, 1, b_tvs[0]);
    let _ = sounded_at(&mut rig, a, 1, a_tvs[0], &two);
    let _ = act_go(&mut rig, a, "+tv/1 mine go");
    let a_run = run_of(&mut rig.tw, a_tvs[0]).expect("A's run");
    let ore_families: Vec<OreFamily> = OreFamily::ALL
        .into_iter()
        .filter(|f| two.classes[usize::from(f.code())] != Word::Barren)
        .collect();
    assert_eq!(ore_families.len(), 2);
    assert_eq!(a_run.lock_rows.len(), 2, "both families' rows");
    for family in &ore_families {
        let cursor = cursor_of(&mut rig.tw, two.deposit(AHUNA, *family)).expect("a cursor");
        assert_eq!(
            cursor
                .row(two.number())
                .and_then(|row| row.lock.as_ref())
                .map(|lock| (lock.tv, lock.ordinal)),
            Some((a_tvs[0], a_run.ordinal)),
            "{family:?}"
        );
    }
    // B on `TWO`.
    to_cell(&mut rig, b_tvs[0], two.stand);
    for line in ["+tv/1 mine", "+tv/1 mine go"] {
        let (out, queued) = command(&mut rig, b, line);
        assert_eq!(out.trim(), MINE_LOCKED, "{line}");
        assert!(queued.is_empty());
    }
    let ore_line = run(&mut rig.tw, b, "+tv/1 mine");
    // B cores `BARREN`; A's rig 2 there.
    to_cell(&mut rig, b_tvs[0], empty.stand);
    let _ = act_go(&mut rig, b, "+tv/1 mine go");
    to_cell(&mut rig, a_tvs[1], empty.stand);
    let barren_line = run(&mut rig.tw, a, "+tv/2 mine");
    assert_eq!(barren_line, ore_line, "byte for byte");
    // The stale lock: A's rig 1's run removed by staff, its rows' lock left behind.
    let mut record = record_of(&mut rig.tw, a_tvs[0]).expect("A's record");
    record.run = None;
    put_record(&mut rig, record);
    to_cell(&mut rig, a_tvs[1], two.stand);
    let out = act_go(&mut rig, a, "+tv/2 mine go");
    assert!(out.contains("Rig 2 starts drilling "), "{out}");
    let taken = run_of(&mut rig.tw, a_tvs[1]).expect("rig 2's run");
    for family in &ore_families {
        let cursor = cursor_of(&mut rig.tw, two.deposit(AHUNA, *family)).expect("a cursor");
        assert_eq!(
            cursor
                .row(two.number())
                .and_then(|row| row.lock.as_ref())
                .map(|lock| (lock.tv, lock.ordinal)),
            Some((a_tvs[1], taken.ordinal)),
            "the stale lock taken: {family:?}"
        );
    }
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M8 (A9). Rights gate mining and never the way home
// ---------------------------------------------------------------------------

/// The Registry's org principal (the sites' issuer).
fn registry(tw: &TestWorld) -> mush_org::PrincipalId {
    mush_org::PrincipalId::Organization(mush_org::OrgId(
        port_map(tw, "ceres_port").orgs[&cid("freeport_registry")].org,
    ))
}

/// **Staff file a claim** (industry's own writer, `@claim/file`; SM.6's precedent — no test
/// fixture writes an industry family) over every opened deposit of `at`'s block, held by `holder`
/// under the Registry, its term `term` IC seconds on (the verb adds the number to its own now).
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
        let deposit = at.deposit(AHUNA, family);
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

/// Occator's counter's name and port, as the lines name them.
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

/// Recover `tv` home (its operator's `+tv/1 recover go`, finished), asserting it lands `Done`.
fn recovered(rig: &mut Rig, pilot: ObjId, tv: ObjId) {
    let _ = act_go(rig, pilot, "+tv/1 recover go");
    let _ = finish(rig, tv, pilot);
    let last = record_of(&mut rig.tw, tv).and_then(|r| r.last);
    assert_eq!(
        last.map(|l| (l.kind, l.result)),
        Some((ActKind::Recover, ActResult::Done))
    );
}

/// **M8 — rights gate mining and never the way home** (A9; §8, §9; lead ruling 5, decisions R15,
/// R19, R20): (a) a suspended entitlement → `LICENCE_SUSPENDED` exactly as `+licence` prints it;
/// the licence held by another character of the account → `LICENCE_OTHER` exactly as `+licence`
/// prints it (never `MINE_NO_LICENCE`); no licence → `MINE_NO_LICENCE`; (b) a four-cycle run whose
/// licence lapses during cycle 2: cycle 2 yields, its line ends `RUN_RIGHTS` with `LICENCE_LAPSED`'s
/// filled text, no cycle 3; a fresh `+tv/1 mine` refuses `LICENCE_LAPSED`; `+tv/1 recover` drives
/// home; (c) staff file every deposit of a block to another pilot under the Registry: `+tv/1 mine`
/// and `+tv/1 mine <a barren family>` both refuse `MINE_CLAIMED` byte-identically; the holder's own
/// rig previews there, then mines it — a landed start and a landed cycle that re-arms with no
/// rights stop; a claim filed **mid-run** stops the run after the cycle in flight with
/// `RUN_RIGHTS` + `MINE_CLAIMED`, and the rig recovers; (d) a claim whose term has passed and that
/// nothing lapsed no longer bars the block; (e) the lease forfeit (the lease document edited to
/// `forfeit` across a restart — SM.3b's seam) mid-run stops the run after the cycle in flight with
/// `RUN_RIGHTS` + `MINE_FORFEIT`, a fresh `+tv/1 mine` refuses `MINE_FORFEIT`, and the rig recovers.
#[test]
#[allow(clippy::too_many_lines)]
fn sm7_rights_gate_mining_and_never_the_way_home() {
    let mut rig = stocked();
    let (p, _, p_tvs) = on_pad(&mut rig, 1);
    let (q, _, q_tvs) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    let (tv, q_tv) = (p_tvs[0], q_tvs[0]);
    let all = picks(&mut rig.tw, AHUNA);
    // Four blocks, a soft single-metal pocket of at least two fair cycles in each.
    let mut blocks: Vec<Pick> = Vec::new();
    for p in &all {
        if p.footing != Footing::Rock
            && p.families() == 1
            && p.classes[0] == Word::Fair
            && p.tonnes[0] >= 7
            && !blocks.iter().any(|b| b.letter() == p.letter())
        {
            blocks.push(p.clone());
        }
        if blocks.len() == 5 {
            break;
        }
    }
    assert_eq!(blocks.len(), 5, "five blocks with a fair pocket");
    let (w, x, y, z, v) = (
        blocks[0].clone(),
        blocks[1].clone(),
        blocks[2].clone(),
        blocks[3].clone(),
        blocks[4].clone(),
    );
    for (name, at) in [("W", &w), ("X", &x), ("Y", &y), ("Z", &z), ("V", &v)] {
        println!(
            "{name}: {} tonnes {:?} footing {:?}",
            at.addr, at.tonnes, at.footing
        );
    }
    launched(&mut rig, p, 1, tv);
    launched(&mut rig, q, 1, q_tv);
    to_cell(&mut rig, tv, w.stand);
    // (a) Suspended.
    let entitlement = store(&rig.tw)
        .licence_of_holder(p)
        .map(|record| record.entitlement)
        .expect("P's licence");
    let freeze = |rig: &mut Rig, status: mush_economy::AccountStatus| {
        let ok = support::with_ctx(&mut rig.tw, |ctx| {
            mush_economy::set_account_status(ctx, mush_economy::AccountId(entitlement), status)
        });
        assert!(ok, "staff set the entitlement account's status");
    };
    freeze(&mut rig, mush_economy::AccountStatus::Frozen);
    let suspended = LICENCE_SUSPENDED
        .replace("{issuer}", REGISTRY)
        .replace("{account}", &entitlement.to_string());
    let (out, queued) = command(&mut rig, p, "+tv/1 mine");
    assert_eq!(out.trim(), suspended);
    assert_eq!(out, run(&mut rig.tw, p, "+licence"), "+licence's own line");
    assert!(queued.is_empty());
    freeze(&mut rig, mush_economy::AccountStatus::Active);
    // Other: the account's licence held by another character.
    let key = licence_key(account_of(&rig, p));
    let mut rig = reboot_editing(rig, key.as_str(), &[p, q], |doc| {
        doc["holder"] = json!(q.0);
    });
    warm(&mut rig, AHUNA);
    let other = LICENCE_OTHER.replace("{issuer}", REGISTRY);
    let (out, _) = command(&mut rig, p, "+tv/1 mine");
    assert_eq!(out.trim(), other);
    assert_eq!(out, run(&mut rig.tw, p, "+licence"));
    assert_ne!(out.trim(), MINE_NO_LICENCE);
    // Restored to P; then no licence at all: P's gateway account swapped for a fresh one (the
    // licence is keyed by the account, so nothing is found), then put back.
    let mut rig = reboot_editing(rig, key.as_str(), &[p, q], |doc| {
        doc["holder"] = json!(p.0);
    });
    warm(&mut rig, AHUNA);
    let own = account_of(&rig, p);
    let swap = |rig: &Rig, account: AccountId| {
        if let Ok(mut map) = rig.oracle.0.lock() {
            map.insert(p, account);
        }
    };
    swap(
        &rig,
        AccountId::mint("NobodyAcct", 77).expect("an account id"),
    );
    let (out, queued) = command(&mut rig, p, "+tv/1 mine");
    assert_eq!(out.trim(), MINE_NO_LICENCE);
    assert!(queued.is_empty());
    swap(&rig, own);
    // (b) A lapse during cycle 2.
    let _ = sounded_at(&mut rig, p, 1, tv, &w);
    let _ = act_go(&mut rig, p, "+tv/1 mine go");
    let run0 = run_of(&mut rig.tw, tv).expect("the run");
    assert!(run0.cycles >= 3);
    let lapse = run0.due_ic + 900;
    let mut rig = reboot_editing(rig, key.as_str(), &[p, q], |doc| {
        doc["expires_ic"] = json!(lapse);
        doc["grace_secs"] = json!(0);
    });
    warm(&mut rig, AHUNA);
    // Staff placement (`with_ctx`) is no persisted write: the restart puts the rig back where its
    // last persisted move left it, so staff place it again (M12 drives there for real).
    to_cell(&mut rig, tv, w.stand);
    let (first, _) = cycle(&mut rig, tv, p);
    assert!(first.contains("Rig 1, cycle 1 of "), "{first}");
    let (second, _) = cycle(&mut rig, tv, p);
    let lapsed = LICENCE_LAPSED.replace("{lapses}", &ic_stamp(lapse));
    let line = CYCLE_DONE
        .replace("{n}", "1")
        .replace("{k}", "2")
        .replace("{cycles}", &run0.cycles.to_string())
        .replace("{t}", "2")
        .replace("{ore}", "metal ore")
        .replace("{hopper}", "4 of 12 t")
        + &*RUN_RIGHTS
            .replace("{n}", "1")
            .replace("{addr}", &w.addr)
            .replace("{line}", &lapsed);
    assert_eq!(count(&second, &line), 1, "{second}\nwanted: {line}");
    assert!(run_of(&mut rig.tw, tv).is_none(), "no cycle 3");
    let (out, _) = command(&mut rig, p, "+tv/1 mine");
    assert_eq!(out.trim(), lapsed);
    recovered(&mut rig, p, tv);
    let (out, queued) = command(&mut rig, p, "+tv/1 mine");
    assert_eq!(
        out.trim(),
        MINE_IN_BAY.replace("{n}", "1"),
        "home in the bay"
    );
    assert!(queued.is_empty());
    support::licence::buy_licence(&mut rig.tw, p);
    emptied(&mut rig, p, 1, tv);
    launched(&mut rig, p, 1, tv);
    // (c) A block under claim: X's, held by Q.
    let term = 30 * 86_400;
    claim_block(&mut rig, &x, q, term);
    to_cell(&mut rig, tv, x.stand);
    let (out, queued) = command(&mut rig, p, "+tv/1 mine");
    assert_eq!(out.trim(), MINE_CLAIMED);
    assert!(queued.is_empty());
    let (barren_word, _) = [("ice", 1_usize), ("carbon", 2), ("strategic", 3)]
        .into_iter()
        .find(|(_, code)| x.classes[*code] == Word::Barren)
        .expect("a barren family at X");
    let (out_barren, _) = command(&mut rig, p, &format!("+tv/1 mine {barren_word}"));
    assert_eq!(out_barren, out, "the same line whatever the family");
    to_cell(&mut rig, q_tv, x.stand);
    let (out, queued) = command(&mut rig, q, "+tv/1 mine");
    assert!(
        out.contains("No reading here: the first cycle will tell."),
        "{out}"
    );
    assert!(queued.is_empty());
    // The holder's own rig mines it (the SM.7 fixer's F2/T6): Q reads the pocket, starts a run
    // — the start's offer re-plans the tenure through the engine's reads — and its cycle lands
    // with the done offer's rights read at landing: the run re-arms (no `RUN_RIGHTS`), the ore
    // moved; then Q stops it.
    let _ = sounded_at(&mut rig, q, 1, q_tv, &x);
    let out = act_go(&mut rig, q, "+tv/1 mine go");
    assert!(out.contains("Rig 1 starts drilling "), "{out}");
    let q_run = run_of(&mut rig.tw, q_tv).expect("Q's run under its own claim");
    assert!(q_run.cycles >= 2);
    let (heard, _) = cycle(&mut rig, q_tv, q);
    let line = CYCLE_DONE
        .replace("{n}", "1")
        .replace("{k}", "1")
        .replace("{cycles}", &q_run.cycles.to_string())
        .replace("{t}", "2")
        .replace("{ore}", "metal ore")
        .replace("{hopper}", "2 of 12 t");
    assert_eq!(count(&heard, &line), 1, "{heard}\nwanted: {line}");
    assert!(!heard.contains("stops drilling"), "no rights stop: {heard}");
    assert_eq!(
        run_of(&mut rig.tw, q_tv).map(|run| run.k),
        Some(2),
        "the holder's run re-arms"
    );
    let q_hopper = hopper(&rig.tw, q_tv);
    assert_eq!(pool(&mut rig.tw, q_hopper, "metal_ore"), 2);
    let (_, queued) = command(&mut rig, q, "+tv/1 mine stop");
    land_all(&mut rig, &queued);
    let (heard, _) = cycle(&mut rig, q_tv, q);
    assert!(
        heard.contains(RUN_STOPPED.replace("{n}", "1").as_str()),
        "{heard}"
    );
    assert!(run_of(&mut rig.tw, q_tv).is_none());
    to_cell(&mut rig, q_tv, (RAMP_CELL.0, RAMP_CELL.1 + 1));
    // A claim filed mid-run, over Y's block.
    let _ = sounded_at(&mut rig, p, 1, tv, &y);
    let _ = act_go(&mut rig, p, "+tv/1 mine go");
    let cycles = run_of(&mut rig.tw, tv).expect("the run").cycles;
    claim_block(&mut rig, &y, q, term);
    let (heard, _) = cycle(&mut rig, tv, p);
    let line = CYCLE_DONE
        .replace("{n}", "1")
        .replace("{k}", "1")
        .replace("{cycles}", &cycles.to_string())
        .replace("{t}", "2")
        .replace("{ore}", "metal ore")
        .replace("{hopper}", "2 of 12 t")
        + &*RUN_RIGHTS
            .replace("{n}", "1")
            .replace("{addr}", &y.addr)
            .replace("{line}", MINE_CLAIMED);
    assert_eq!(count(&heard, &line), 1, "{heard}\nwanted: {line}");
    assert!(run_of(&mut rig.tw, tv).is_none());
    recovered(&mut rig, p, tv);
    emptied(&mut rig, p, 1, tv);
    launched(&mut rig, p, 1, tv);
    // (d) A claim past its term that nothing lapsed: open ground again.
    claim_block(&mut rig, &z, q, 100);
    let at = ic_now(&rig.tw) + 101;
    let _ = to_ic(&mut rig, at);
    to_cell(&mut rig, tv, z.stand);
    let (out, _) = command(&mut rig, p, "+tv/1 mine");
    assert!(
        out.contains("No reading here: the first cycle will tell."),
        "{out}"
    );
    // (e) The lease forfeit, mid-run.
    let _ = sounded_at(&mut rig, p, 1, tv, &v);
    let _ = act_go(&mut rig, p, "+tv/1 mine go");
    let cycles = run_of(&mut rig.tw, tv).expect("the run").cycles;
    let lease = tacsim::port::tv::tv_lease_key(lease_of(&rig.tw, tv));
    let mut rig = reboot_editing(rig, lease.as_str(), &[p, q], |doc| {
        doc["status"] = json!("forfeit");
    });
    warm(&mut rig, AHUNA);
    to_cell(&mut rig, tv, v.stand);
    let (heard, _) = cycle(&mut rig, tv, p);
    let line = CYCLE_DONE
        .replace("{n}", "1")
        .replace("{k}", "1")
        .replace("{cycles}", &cycles.to_string())
        .replace("{t}", "2")
        .replace("{ore}", "metal ore")
        .replace("{hopper}", "2 of 12 t")
        + &*RUN_RIGHTS
            .replace("{n}", "1")
            .replace("{addr}", &v.addr)
            .replace("{line}", &at_counter(&rig.tw, MINE_FORFEIT));
    assert_eq!(count(&heard, &line), 1, "{heard}\nwanted: {line}");
    let (out, _) = command(&mut rig, p, "+tv/1 mine");
    assert_eq!(out.trim(), at_counter(&rig.tw, MINE_FORFEIT));
    recovered(&mut rig, p, tv);
    // An ended lease reads `MINE_ENDED` (the launch's own seam: staff cancel the contract).
    let q_lease = lease_of(&rig.tw, q_tv);
    let _ = run(&mut rig.tw, GOD, &format!("@contract/cancel #{q_lease}"));
    to_cell(&mut rig, q_tv, w.stand);
    let (out, _) = command(&mut rig, q, "+tv/1 mine");
    assert_eq!(out.trim(), at_counter(&rig.tw, MINE_ENDED));
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M9. Held and commons blocks
// ---------------------------------------------------------------------------

/// **M9 — held and commons blocks** (§9.2; decision R5): on Occator Rim, an authored **held**
/// block (F, Occator's) with no claim filed refuses `MINE_CLAIMED` (SM.10 seeds its claims); on
/// Freeport Commons a commons block mines under a licence — a blind core lands and lifts its ore.
#[test]
fn sm7_held_and_commons_blocks() {
    let mut rig = stocked();
    let (p, _, p_tvs) = on_pad_at(
        &mut rig,
        ("Pilot", 1, "Salt Moth"),
        ("occator", "occator_port"),
        1,
    );
    let (c, _, c_tvs) = on_pad_at(
        &mut rig,
        ("Other", 2, "Tally Moth"),
        ("commons", "commons_port"),
        1,
    );
    let occator = picks(&mut rig.tw, "occator_port");
    let held = pick(&occator, "OCCATOR F", |p| {
        p.letter() == 'F' && p.footing != Footing::Rock
    });
    launched(&mut rig, p, 1, p_tvs[0]);
    to_cell_on(&mut rig, "occator_port", p_tvs[0], held.stand);
    let (out, queued) = command(&mut rig, p, "+tv/1 mine");
    assert_eq!(out.trim(), MINE_CLAIMED);
    assert!(queued.is_empty());
    let commons = picks(&mut rig.tw, "commons_port");
    let ore = pick(&commons, "COMMONS", |p| {
        p.footing != Footing::Rock && p.families() == 1 && p.classes[0] == Word::Rich
    });
    launched(&mut rig, c, 1, c_tvs[0]);
    to_cell_on(&mut rig, "commons_port", c_tvs[0], ore.stand);
    let out = act_go(&mut rig, c, "+tv/1 mine go");
    assert!(out.contains("starts coring"), "{out}");
    let (heard, _) = cycle(&mut rig, c_tvs[0], c);
    assert!(
        heard.contains(&format!("Rig 1 cored {}", ore.addr)),
        "{heard}"
    );
    let vault = hopper(&rig.tw, c_tvs[0]);
    assert!(
        pool(&mut rig.tw, vault, "metal_ore") > 0,
        "the commons core lifted ore"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M10. The run's reservation, and M11. The pace
// ---------------------------------------------------------------------------

/// **M10 — `reserved_total` counts the run** (lead ruling 1; SM.3b's owed acceptance): a
/// four-cycle run started with exactly the floor plus 999 cu left after the trip home; a scan
/// from the drilling rig (1,000 cu) refuses `RESERVE_LOW` though alone it would fit; with 1 cu
/// more it starts **during the run**, both land — the scan's chart write and the cycles' lines —
/// and the final charge is exact.
#[test]
fn sm7_reserved_total_counts_the_run() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let all = picks(&mut rig.tw, AHUNA);
    let fair = fair7(&all);
    launched(&mut rig, pilot, 1, tv);
    let _ = sounded_at(&mut rig, pilot, 1, tv, &fair);
    let cycles = u32::try_from(yields_of(2, u32::from(fair.tonnes[0])).len()).expect("fits");
    let run_cost = 2_000 * cycles;
    let home = home_cu(&rig.tw, fair.stand);
    let value = floor() + 999 + run_cost + home;
    let _ = put_charge(&mut rig, tv, value);
    let _ = act_go(&mut rig, pilot, "+tv/1 mine go");
    let (out, queued) = command(&mut rig, pilot, "+tv/1 scan");
    assert!(queued.is_empty());
    assert!(
        out.trim().starts_with("Rig 1 has ") && out.trim().ends_with(" % reserve."),
        "{out}"
    );
    // Alone (no run), the scan would fit: the record's reservation is what refuses it.
    let mut alone = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(tacsim::surface::reserved_total(&alone), run_cost);
    alone.run = None;
    assert_eq!(tacsim::surface::reserved_total(&alone), 0);
    let _ = put_charge(&mut rig, tv, value + 1);
    let _ = act_go(&mut rig, pilot, "+tv/1 scan go");
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert!(
        record.act.is_some() && record.run.is_some(),
        "a scan beside the run"
    );
    let _ = finish(&mut rig, tv, pilot);
    let row = chart_of(&mut rig.tw, pilot, AHUNA).expect("the row");
    assert!(!row.fields.is_empty(), "the scan's chart write");
    let mut lines = String::new();
    while run_of(&mut rig.tw, tv).is_some() {
        lines.push_str(&cycle(&mut rig, tv, pilot).0);
    }
    assert_eq!(
        count(&lines, ", cycle "),
        usize::try_from(cycles).expect("fits")
    );
    assert_eq!(value_of(&mut rig.tw, tv), value + 1 - 1_000 - run_cost);
    no_refused_writes(&rig.tw);
}

/// **M11 — terrain sets the pace** (§6; lead ruling 4): a run on a rock stand is due every 3,600
/// IC s, one on regolith or ice every 1,800.
#[test]
fn sm7_terrain_sets_the_pace() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 2);
    let all = picks(&mut rig.tw, AHUNA);
    let rock = pick(&all, "ROCK", |p| {
        p.footing == Footing::Rock
            && p.families() == 1
            && p.classes[0] != Word::Barren
            && u32::from(p.tonnes[0])
                > match p.classes[0] {
                    Word::Lean => 1,
                    Word::Fair => 2,
                    _ => 3,
                }
    });
    let fair = fair7(&all);
    launched(&mut rig, pilot, 1, tvs[0]);
    launched(&mut rig, pilot, 2, tvs[1]);
    let _ = sounded_at(&mut rig, pilot, 1, tvs[0], &rock);
    let _ = sounded_at(&mut rig, pilot, 2, tvs[1], &fair);
    for (slot, tv, secs) in [(1_u8, tvs[0], 3_600), (2, tvs[1], 1_800)] {
        let now = ic_now(&rig.tw);
        let _ = act_go(&mut rig, pilot, &format!("+tv/{slot} mine go"));
        let first = run_of(&mut rig.tw, tv).expect("the run");
        assert_eq!(first.secs, secs);
        assert_eq!(first.due_ic, now + i64::from(secs));
        let _ = cycle(&mut rig, tv, pilot);
        let second = run_of(&mut rig.tw, tv).expect("cycle 2");
        assert_eq!(second.due_ic - first.due_ic, i64::from(secs), "slot {slot}");
    }
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M12. A restart mid-run
// ---------------------------------------------------------------------------

/// Everything M12 compares, as one debug string: the rig's record and projection, the pocket's
/// cursor, the block vault's and the hopper's pools, the operator's chart row and the site's
/// mined ring.
fn books(rig: &mut Rig, pilot: ObjId, tv: ObjId, at: &Pick) -> String {
    let record = record_of(&mut rig.tw, tv);
    let projected = projection(&rig.tw, tv);
    let cursor = cursor_of(&mut rig.tw, at.deposit(AHUNA, OreFamily::Metals));
    let vault = block_vault(&rig.tw, AHUNA, at.letter());
    let block = pool(&mut rig.tw, vault, "metal_ore");
    let vault = hopper(&rig.tw, tv);
    let held = pool(&mut rig.tw, vault, "metal_ore");
    let chart = chart_of(&mut rig.tw, pilot, AHUNA);
    let ring = store(&rig.tw).mined(&cid(AHUNA)).cloned();
    format!("{record:#?}\n{projected:#?}\n{cursor:#?}\n{block} {held}\n{chart:#?}\n{ring:#?}")
}

/// **M12 — a restart mid-run equals an uninterrupted run** (§8): two worlds; in each the rig
/// drives to `FAIR7` (a real drive, so its place is a persisted write), sounds, starts the run and
/// lands cycle 1; the clock then passes cycle 2's due unfired, and one world reboots keyed (every
/// write folded, the operator reconnected) while the other does not; both then fire alike and
/// finish the run. The records, cursors, pools, charts, projections and rings are equal, and so
/// are the lines the operator heard after the restart point.
#[test]
fn sm7_a_restart_mid_run_equals_an_uninterrupted_run() {
    let mut seen: Vec<(String, String)> = Vec::new();
    for restart in [false, true] {
        let mut rig = stocked();
        let (pilot, _, tvs) = on_pad(&mut rig, 1);
        let tv = tvs[0];
        let all = picks(&mut rig.tw, AHUNA);
        let fair = fair7(&all);
        launched(&mut rig, pilot, 1, tv);
        let _ = act_go(
            &mut rig,
            pilot,
            &format!("+tv/1 drive {},{} go", fair.stand.0, fair.stand.1),
        );
        let _ = finish(&mut rig, tv, pilot);
        let _ = act_go(&mut rig, pilot, "+tv/1 sound quiet go");
        let _ = finish(&mut rig, tv, pilot);
        let _ = act_go(&mut rig, pilot, "+tv/1 mine go");
        let (first, _) = cycle(&mut rig, tv, pilot);
        assert!(first.contains("Rig 1, cycle 1 of "), "{first}");
        // Past cycle 2's due, nothing fired.
        let due = run_of(&mut rig.tw, tv).expect("cycle 2 armed").due_ic;
        let rate = i64::from(world_rate(&rig.tw).max(1));
        let real = (due - ic_now(&rig.tw)) / rate + 2;
        let t = rig.tw.world().now().saturating_add(real);
        rig.tw.set_now(t);
        let mut rig = if restart { reboot(rig, &[pilot]) } else { rig };
        let mut heard = String::new();
        for _ in 0..4 {
            let (events, queued) = fire(&mut rig);
            heard.push_str(&said(&rig.tw, &events, pilot));
            land_all(&mut rig, &queued);
        }
        warm(&mut rig, AHUNA);
        while run_of(&mut rig.tw, tv).is_some() {
            heard.push_str(&cycle(&mut rig, tv, pilot).0);
        }
        assert!(heard.contains("Rig 1, cycle 2 of "), "{heard}");
        seen.push((books(&mut rig, pilot, tv, &fair), heard));
        no_refused_writes(&rig.tw);
    }
    assert_eq!(seen[0].0, seen[1].0, "the books");
    assert_eq!(seen[0].1, seen[1].1, "the lines");
}

// ---------------------------------------------------------------------------
// M13 (A10). Pits render for everyone
// ---------------------------------------------------------------------------

/// The `+tv/map site` print `who` gets, line by line.
fn map_print(rig: &mut Rig, who: ObjId) -> Vec<String> {
    run(&mut rig.tw, who, "+tv/map site")
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Every `(line, column, before, after)` at which two prints differ (both the same shape).
fn print_diff(before: &[String], after: &[String]) -> Vec<(usize, usize, char, char)> {
    assert_eq!(before.len(), after.len());
    let mut out = Vec::new();
    for (i, (b, a)) in before.iter().zip(after).enumerate() {
        assert_eq!(b.chars().count(), a.chars().count(), "line {i}");
        for (j, (cb, ca)) in b.chars().zip(a.chars()).enumerate() {
            if cb != ca {
                out.push((i, j, cb, ca));
            }
        }
    }
    out
}

/// Every frame cell at which two frames' rows differ.
fn frame_diff(before: &Value, after: &Value) -> Vec<((usize, usize), char, char)> {
    let (b, a) = (frame_rows(before), frame_rows(after));
    assert_eq!(b.len(), a.len());
    let mut out = Vec::new();
    for (y, (rb, ra)) in b.iter().zip(&a).enumerate() {
        for (x, (cb, ca)) in rb.chars().zip(ra.chars()).enumerate() {
            if cb != ca {
                out.push(((x, y), cb, ca));
            }
        }
    }
    out
}

/// A pocket's frame cell in the site view.
fn frame_cell(pocket: (i64, i64)) -> (usize, usize) {
    (
        usize::try_from(pocket.0 / 2).expect("x"),
        usize::try_from(pocket.1 / 2).expect("y"),
    )
}

/// **M13 — pits render for everyone** (A10; §5, decisions R16, F18): on the single-family
/// `FAIR7`, B — another principal with no reading, on the same pad, its rig in the bay — sees its
/// `site.map` frame and its `+tv/map site` print change in exactly one place: `v` at the pocket's
/// cell after A's stopped run drew one cycle, `V` once A worked it out, no ore glyph ever (A's rig
/// is staff-parked 40 cells east and 40 north of the pocket's stand for every frame). Then B's own
/// rig on the pocket previews `MINE_PREVIEW_BLIND` with `PIT_WORKED_OUT`; on a pocket a stopped
/// run left partly drawn, with `PIT_WORKED`; while A's run holds that pocket, `MINE_LOCKED` first;
/// and on a never-drawn unread ore pocket and on `BARREN` with no pit clause, byte-identical
/// between them with the address substituted.
#[test]
#[allow(clippy::too_many_lines)]
fn sm7_pits_render_for_everyone() {
    let mut rig = stocked();
    let (a, _, a_tvs) = on_pad(&mut rig, 1);
    let (b, _, b_tvs) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    let (a_tv, b_tv) = (a_tvs[0], b_tvs[0]);
    let all = picks(&mut rig.tw, AHUNA);
    let fair = fair7(&all);
    let part = pick(&all, "PARTLY", |p| {
        p.footing != Footing::Rock
            && p.families() == 1
            && p.classes[0] == Word::Fair
            && p.tonnes[0] >= 3
            && frame_cell(p.pocket) != frame_cell(fair.pocket)
    });
    let away = (fair.stand.0 + 40, fair.stand.1 + 40);
    launched(&mut rig, a, 1, a_tv);
    let _ = sounded_at(&mut rig, a, 1, a_tv, &fair);
    to_cell(&mut rig, a_tv, away);
    let frame0 = map_frame(&mut rig, b, json!({ "view": "site" }));
    let print0 = map_print(&mut rig, b);
    let cell = frame_cell(fair.pocket);
    let under = glyph_at(&frame_rows(&frame0), cell).expect("the pocket's cell");
    // A's run, stopped after its first cycle: drawn, not worked out.
    to_cell(&mut rig, a_tv, fair.stand);
    let _ = act_go(&mut rig, a, "+tv/1 mine go");
    let (_, queued) = command(&mut rig, a, "+tv/1 mine stop");
    land_all(&mut rig, &queued);
    let _ = cycle(&mut rig, a_tv, a);
    assert!(run_of(&mut rig.tw, a_tv).is_none());
    to_cell(&mut rig, a_tv, away);
    let frame1 = map_frame(&mut rig, b, json!({ "view": "site" }));
    let print1 = map_print(&mut rig, b);
    assert_eq!(frame_diff(&frame0, &frame1), vec![(cell, under, 'v')]);
    let changed = print_diff(&print0, &print1);
    assert!(
        !changed.is_empty()
            && changed
                .iter()
                .all(|(_, _, was, now)| *was == under && *now == 'v'),
        "{changed:?}"
    );
    // Worked out.
    to_cell(&mut rig, a_tv, fair.stand);
    let _ = act_go(&mut rig, a, "+tv/1 mine go");
    while run_of(&mut rig.tw, a_tv).is_some() {
        let _ = cycle(&mut rig, a_tv, a);
    }
    to_cell(&mut rig, a_tv, away);
    let frame2 = map_frame(&mut rig, b, json!({ "view": "site" }));
    let print2 = map_print(&mut rig, b);
    assert_eq!(frame_diff(&frame0, &frame2), vec![(cell, under, 'V')]);
    let changed = print_diff(&print1, &print2);
    assert!(
        !changed.is_empty()
            && changed
                .iter()
                .all(|(_, _, was, now)| *was == 'v' && *now == 'V'),
        "{changed:?}"
    );
    for frame in [&frame0, &frame1, &frame2] {
        for row in frame_rows(frame) {
            assert!(
                !row.chars().any(|c| ORE_GLYPHS.contains(&c)),
                "an ore glyph in B's grid: {row}"
            );
        }
    }
    // A partly drawn pocket, then B's rig on the pockets.
    let _ = sounded_at(&mut rig, a, 1, a_tv, &part);
    let _ = act_go(&mut rig, a, "+tv/1 mine go");
    let (_, queued) = command(&mut rig, a, "+tv/1 mine stop");
    land_all(&mut rig, &queued);
    let _ = cycle(&mut rig, a_tv, a);
    to_cell(&mut rig, a_tv, away);
    launched(&mut rig, b, 1, b_tv);
    to_cell(&mut rig, b_tv, fair.stand);
    let (out, queued) = command(&mut rig, b, "+tv/1 mine");
    assert!(queued.is_empty());
    assert!(
        out.trim()
            .starts_with("No reading here: the first cycle will tell. ")
            && out.contains(&format!("{PIT_WORKED_OUT} +tv/1 mine go starts it.")),
        "{out}"
    );
    to_cell(&mut rig, b_tv, part.stand);
    let (out, _) = command(&mut rig, b, "+tv/1 mine");
    assert!(
        out.trim()
            .starts_with("No reading here: the first cycle will tell. ")
            && out.contains(&format!("{PIT_WORKED} +tv/1 mine go starts it.")),
        "{out}"
    );
    // While A's run holds it, the lock refuses first.
    to_cell(&mut rig, a_tv, part.stand);
    let _ = act_go(&mut rig, a, "+tv/1 mine go");
    let (out, _) = command(&mut rig, b, "+tv/1 mine");
    assert_eq!(out.trim(), MINE_LOCKED);
    // A never-drawn unread ore pocket and `BARREN`: no pit clause, one line.
    let fresh: Vec<&Pick> = all
        .iter()
        .filter(|p| {
            p.footing != Footing::Rock && p.pocket != fair.pocket && p.pocket != part.pocket
        })
        .collect();
    let mut pair: Option<(Pick, Pick, u32)> = None;
    'pairs: for empty in fresh.iter().filter(|p| p.families() == 0).take(40) {
        let h_empty = home_cu(&rig.tw, empty.stand);
        for ore in fresh
            .iter()
            .filter(|p| p.families() == 1 && p.classes[0] != Word::Barren)
        {
            let h_ore = home_cu(&rig.tw, ore.stand);
            let value = 52_000 + h_empty.max(h_ore);
            if pct(value - 2_000 - h_empty) == pct(value - 2_000 - h_ore) {
                pair = Some(((*empty).clone(), (*ore).clone(), value));
                break 'pairs;
            }
        }
    }
    let (empty, ore, value) = pair.expect("a barren and an ore pocket alike");
    let mut previews = Vec::new();
    for at in [&ore, &empty] {
        to_cell(&mut rig, b_tv, at.stand);
        let _ = put_charge(&mut rig, b_tv, value);
        let (out, _) = command(&mut rig, b, "+tv/1 mine");
        assert!(!out.contains("Every map shows"), "{out}");
        assert!(out.trim().starts_with("No reading here"), "{out}");
        previews.push(neutral(&out, &at.addr, "OWP-TV-02"));
    }
    assert_eq!(previews[0], previews[1]);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M14 (A11's world half). The cycle line fixture
// ---------------------------------------------------------------------------

/// Where a blessed site fixture lives (`tests/chart.rs`'s precedent).
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
             cargo test -p tacsim --test mining sm7_the_cycle_line_fixture_is_blessed"
        );
    }
}

/// [`cycle`], returning every event of the landing and the tell.
fn cycle_events(rig: &mut Rig, tv: ObjId) -> Vec<(Audience, OutputEvent)> {
    let due = run_of(&mut rig.tw, tv).expect("a run in flight").due_ic;
    let (mut events, queued) = advance_to(rig, due);
    land_all(rig, &queued);
    let (more, queued) = fire(rig);
    land_all(rig, &queued);
    events.extend(more);
    events
}

/// The one event `who` heard exactly as `wanted` (trimmed), or starting with it when `prefix`.
fn the_event(
    tw: &TestWorld,
    events: &[(Audience, OutputEvent)],
    who: ObjId,
    wanted: &str,
    prefix: bool,
) -> OutputEvent {
    let found: Vec<OutputEvent> = events
        .iter()
        .filter(|(audience, event)| {
            let text = said(tw, &[(audience.clone(), event.clone())], who);
            if prefix {
                text.trim().starts_with(wanted)
            } else {
                text.trim() == wanted
            }
        })
        .map(|(_, event)| event.clone())
        .collect();
    assert_eq!(found.len(), 1, "one event {wanted:?}: {found:?}");
    found.into_iter().next().expect("one")
}

/// **M14 — the cycle line fixture is blessed** (A11's world half): the operator's real
/// `CYCLE_DONE` event (cycle 1 of `FAIR7`'s run, no clause) and real `CORED` event (a blind core
/// of an unread ore pocket) serialised into `web/src/lib/fixtures/site/mine_events.json` as
/// `{ "cycle": …, "cored": … }`; `TACSIM_BLESS=1` writes, every other run compares byte for byte.
#[test]
fn sm7_the_cycle_line_fixture_is_blessed() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let all = picks(&mut rig.tw, AHUNA);
    let fair = fair7(&all);
    let ore = pick(&all, "UNREAD_ORE", |p| {
        p.footing != Footing::Rock
            && p.families() == 1
            && p.classes[0] != Word::Barren
            && p.pocket != fair.pocket
    });
    launched(&mut rig, pilot, 1, tv);
    let _ = sounded_at(&mut rig, pilot, 1, tv, &fair);
    let _ = act_go(&mut rig, pilot, "+tv/1 mine go");
    let cycles = run_of(&mut rig.tw, tv).expect("the run").cycles;
    let events = cycle_events(&mut rig, tv);
    let line = CYCLE_DONE
        .replace("{n}", "1")
        .replace("{k}", "1")
        .replace("{cycles}", &cycles.to_string())
        .replace("{t}", "2")
        .replace("{ore}", "metal ore")
        .replace("{hopper}", "2 of 12 t");
    let cycle_event = the_event(&rig.tw, &events, pilot, &line, false);
    let (_, queued) = command(&mut rig, pilot, "+tv/1 mine stop");
    land_all(&mut rig, &queued);
    let _ = cycle(&mut rig, tv, pilot);
    emptied(&mut rig, pilot, 1, tv);
    to_cell(&mut rig, tv, ore.stand);
    let _ = act_go(&mut rig, pilot, "+tv/1 mine go");
    let events = cycle_events(&mut rig, tv);
    let cored = the_event(
        &rig.tw,
        &events,
        pilot,
        &format!("Rig 1 cored {}", ore.addr),
        true,
    );
    let pretty = |value: &Value| serde_json::to_string_pretty(value).expect("encodes") + "\n";
    bless(
        "mine_events.json",
        &pretty(&json!({ "cycle": cycle_event, "cored": cored })),
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M15. The mined ring
// ---------------------------------------------------------------------------

/// The site's mined facts.
fn ring(tw: &TestWorld) -> Vec<tacsim::surface::mined::MinedFact> {
    match store(tw).mined(&cid(AHUNA)) {
        Some(MinedState::Present(ring)) => ring.entries.clone(),
        Some(MinedState::Unreadable) => panic!("the ring is unreadable"),
        None => Vec::new(),
    }
}

/// **M15 — a cycle writes one mined fact** (§5's public record; decision R3): one `Done` cycle
/// with tonnes appends one fact to `tacsim:mined:<site>` (its pocket, tonnes, operator, run and
/// cycle); a retell — the projection's told pair put back across a restart, as a refused
/// projection write leaves it — retells the line and does not double the fact; the next cycle
/// appends the second; a barren core appends none.
#[test]
#[allow(clippy::too_many_lines)]
fn sm7_a_cycle_writes_one_mined_fact() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let all = picks(&mut rig.tw, AHUNA);
    let fair = fair7(&all);
    let empty = barren(&all, &[&fair]);
    launched(&mut rig, pilot, 1, tv);
    let _ = sounded_at(&mut rig, pilot, 1, tv, &fair);
    assert!(ring(&rig.tw).is_empty());
    let _ = act_go(&mut rig, pilot, "+tv/1 mine go");
    let started = run_of(&mut rig.tw, tv).expect("the run");
    let _ = cycle(&mut rig, tv, pilot);
    let facts = ring(&rig.tw);
    assert_eq!(facts.len(), 1, "{facts:?}");
    assert_eq!(
        (
            facts[0].tv,
            facts[0].operator,
            facts[0].pocket.as_str(),
            facts[0].good.as_str(),
            facts[0].tonnes,
            facts[0].ordinal,
            facts[0].k,
            facts[0].at_ic
        ),
        (
            tv,
            pilot,
            fair.addr.as_str(),
            "metal_ore",
            2,
            started.ordinal,
            1,
            started.due_ic
        )
    );
    let told = projection(&rig.tw, tv).map(|rec| (rec.told, rec.told_cycle));
    assert_eq!(told, Some((started.ordinal, 1)));
    // The retell.
    let key = format!("tacsim:tv:{}", tv.0);
    let mut rig = reboot_editing(rig, &key, &[pilot], |doc| {
        doc["told_cycle"] = json!(0);
    });
    let mut heard = String::new();
    for _ in 0..3 {
        let (events, queued) = fire(&mut rig);
        heard.push_str(&said(&rig.tw, &events, pilot));
        land_quietly(&mut rig, &queued);
    }
    warm(&mut rig, AHUNA);
    assert!(
        heard.contains("Rig 1, cycle 1 of "),
        "the retold line: {heard}"
    );
    assert_eq!(
        projection(&rig.tw, tv).map(|rec| (rec.told, rec.told_cycle)),
        Some((started.ordinal, 1))
    );
    assert_eq!(ring(&rig.tw), facts, "never doubled");
    // The next cycle (its rig placed again: staff placement is no persisted write).
    to_cell(&mut rig, tv, fair.stand);
    let (_, queued) = command(&mut rig, pilot, "+tv/1 mine stop");
    land_all(&mut rig, &queued);
    let _ = cycle(&mut rig, tv, pilot);
    let facts = ring(&rig.tw);
    assert_eq!(facts.len(), 2, "{facts:?}");
    assert_eq!((facts[1].k, facts[1].tonnes), (2, 2));
    // A barren core: nothing.
    to_cell(&mut rig, tv, empty.stand);
    let _ = act_go(&mut rig, pilot, "+tv/1 mine go");
    let _ = cycle(&mut rig, tv, pilot);
    assert_eq!(ring(&rig.tw), facts, "a barren core appends none");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M16. Two starts race at the landing
// ---------------------------------------------------------------------------

/// **M16 — two starts race at the prepare** (reviewer F10; the SM.7 fixer's T2): both starts are
/// planned (queued by their commands) before either lands, and the second's offer is
/// **resolved** (`resolve`, the landing's first half) before the first lands and **prepared**
/// (`prepare`) after it — so the prepare's own guards are what refuse, never a hand-built offer;
/// a fresh landing of the same submission then shows the offer's re-plan refusing too. (a) **The
/// hold**: one pilot's two rigs on two fair pockets, the hold seeded so its unreserved room fits
/// one run but not both; the second's prepare refuses `Conflict` "the hold's room changed" and its
/// landing `hold_full`, with nothing written (the record, the cursor, the charge, compared as
/// documents). (b) **The lock**: two pilots' rigs on one pocket; the second's prepare refuses
/// `locked` (the cursor row's live lock on ore, the other rig's run on `BARREN`) and so does its
/// landing; on an ore pocket and on `BARREN` the second-start refusals are byte-identical with
/// the address substituted (decision R4). (c) **The drawn row**: a run's offer resolved, then
/// another rig's one-cycle core draws the pocket and releases it; the prepare refuses `Conflict`
/// "that pocket changed" and the landing `mine_changed`.
#[test]
#[allow(clippy::too_many_lines)]
fn sm7_two_starts_race_at_the_prepare() {
    let mut rig = stocked();
    let (a, row, a_tvs) = on_pad(&mut rig, 2);
    let (b, _, b_tvs) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    let all = picks(&mut rig.tw, AHUNA);
    let one = fair7(&all);
    let two = fair_other(&all, "FAIR_TWO", &[&one]);
    let empty = barren(&all, &[&one, &two]);
    launched(&mut rig, a, 1, a_tvs[0]);
    launched(&mut rig, a, 2, a_tvs[1]);
    launched(&mut rig, b, 1, b_tvs[0]);
    let _ = sounded_at(&mut rig, a, 1, a_tvs[0], &one);
    let _ = sounded_at(&mut rig, a, 2, a_tvs[1], &two);
    // (a) The hold: room for the larger run, not for both.
    let need = |p: &Pick| u64::from(p.tonnes[0]);
    let limit_t = ore_mass_limit(&rig.tw) / 1_000;
    let room = need(&one).max(need(&two));
    let vault = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, vault, "metal_ore", limit_t - room, LotId(9_501));
    let (_, first) = command(&mut rig, a, "+tv/1 mine go");
    let (_, second) = command(&mut rig, a, "+tv/2 mine go");
    assert_eq!((first.len(), second.len()), (1, 1), "both planned");
    let deposit = two.deposit(AHUNA, OreFamily::Metals);
    let before = (
        record_of(&mut rig.tw, a_tvs[1]),
        cursor_of(&mut rig.tw, deposit),
        value_of(&mut rig.tw, a_tvs[1]),
    );
    // At the prepare (the SM.7 fixer's T2): the second's offer resolved before the first lands,
    // then prepared after it — the prepare's own snapshot of the hull's runs refuses.
    let held = resolve(&rig, &second[0]).expect("the second's offer resolves first");
    land(&mut rig, &first[0]).expect("the first lands");
    let refused = prepare(&mut rig, &held).expect_err("the second's prepare refuses");
    assert!(
        refused.contains("Conflict")
            && refused.contains("the hold's room changed; preview it again"),
        "{refused}"
    );
    let snapshot = |rig: &mut Rig| {
        format!(
            "{:?}",
            (
                record_of(&mut rig.tw, a_tvs[1]),
                cursor_of(&mut rig.tw, deposit),
                value_of(&mut rig.tw, a_tvs[1]),
            )
        )
    };
    assert_eq!(format!("{before:?}"), snapshot(&mut rig), "nothing written");
    // At the landing: a fresh landing re-plans the start (the offer), which counts rig 1's run
    // in the hold and refuses before the prepare.
    let refused = land(&mut rig, &second[0]).expect_err("the second refuses");
    assert!(
        refused.contains("ProviderRejected")
            && refused.contains("the hold has no unreserved room for another cycle"),
        "{refused}"
    );
    assert_eq!(format!("{before:?}"), snapshot(&mut rig), "nothing written");
    // (b) The lock, on an ore pocket (`two`, unread by B) and on `BARREN`; rig 1's run cleared by
    // staff first so the hold has room for rig 2 (the lock, gate 7, is what refuses).
    let mut record = record_of(&mut rig.tw, a_tvs[0]).expect("rig 1's record");
    record.run = None;
    put_record(&mut rig, record);
    let mut seconds = Vec::new();
    for at in [&two, &empty] {
        to_cell(&mut rig, a_tvs[1], at.stand);
        to_cell(&mut rig, b_tvs[0], at.stand);
        let (_, first) = command(&mut rig, b, "+tv/1 mine go");
        let (_, second) = command(&mut rig, a, "+tv/2 mine go");
        assert_eq!(
            (first.len(), second.len()),
            (1, 1),
            "both planned at {}",
            at.addr
        );
        let before = format!("{:?}", record_of(&mut rig.tw, a_tvs[1]));
        // At the prepare: the second's offer resolved before the first lands; the prepare's
        // in-transaction lock (`pocket_taken`: the cursor row's live lock on ore, the other
        // rig's run on a barren pocket with no row) refuses `locked`.
        let held = resolve(&rig, &second[0]).expect("the second's offer resolves first");
        land(&mut rig, &first[0]).expect("the first lands");
        let at_prepare = prepare(&mut rig, &held).expect_err("the second's prepare refuses");
        assert!(
            at_prepare.contains("ProviderRejected")
                && at_prepare.contains("another rig is working that pocket"),
            "{at_prepare}"
        );
        assert_eq!(before, format!("{:?}", record_of(&mut rig.tw, a_tvs[1])));
        // At the landing: the offer's re-plan refuses the same way.
        let refused = land(&mut rig, &second[0]).expect_err("the second refuses");
        assert!(
            refused.contains("another rig is working that pocket"),
            "{refused}"
        );
        assert_eq!(before, format!("{:?}", record_of(&mut rig.tw, a_tvs[1])));
        seconds.push((
            at_prepare.replace(&at.addr, "<addr>"),
            refused.replace(&at.addr, "<addr>"),
        ));
        // B's run off the pocket for the next round (its record's run cleared by staff).
        let mut record = record_of(&mut rig.tw, b_tvs[0]).expect("B's record");
        record.run = None;
        put_record(&mut rig, record);
    }
    assert_eq!(
        seconds[0], seconds[1],
        "the barren race refuses the same way"
    );
    // (c) The cursor's drawn, as previewed: A's rig 1 plans a run on `one` (drawn 0) and its
    // offer is resolved; B's rig cores `one` blind meanwhile — a one-cycle run that draws and
    // releases its lock — so A's prepare finds no lock but a drawn row, and refuses `Conflict`
    // "that pocket changed"; a fresh landing re-plans and refuses `mine_changed` at the offer.
    to_cell(&mut rig, a_tvs[0], one.stand);
    to_cell(&mut rig, b_tvs[0], one.stand);
    let (_, first) = command(&mut rig, a, "+tv/1 mine go");
    assert_eq!(first.len(), 1, "A's run planned");
    let held = resolve(&rig, &first[0]).expect("A's offer resolves");
    let out = act_go(&mut rig, b, "+tv/1 mine go");
    assert!(out.contains("starts coring"), "{out}");
    let _ = cycle(&mut rig, b_tvs[0], b);
    let one_deposit = one.deposit(AHUNA, OreFamily::Metals);
    let drawn_now = cursor_of(&mut rig.tw, one_deposit);
    assert_eq!(
        drawn_now
            .as_ref()
            .and_then(|cursor| cursor.row(one.number()))
            .map(|row| (row.drawn_t, row.lock.is_none())),
        Some((2, true)),
        "B's core drew and released"
    );
    let before = format!("{:?}", record_of(&mut rig.tw, a_tvs[0]));
    let refused = prepare(&mut rig, &held).expect_err("A's prepare refuses");
    assert!(
        refused.contains("Conflict") && refused.contains("that pocket changed; preview it again"),
        "{refused}"
    );
    let refused = land(&mut rig, &first[0]).expect_err("A's landing refuses");
    assert!(
        refused.contains("that run changed since its preview; preview it again"),
        "{refused}"
    );
    assert_eq!(before, format!("{:?}", record_of(&mut rig.tw, a_tvs[0])));
    assert_eq!(
        cursor_of(&mut rig.tw, one_deposit),
        drawn_now,
        "nothing written"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M12b. A late landing judges the lapse at the cycle's own due
// ---------------------------------------------------------------------------

/// **M12b — a late landing judges the lapse at the cycle's own due** (§8's restart acceptance;
/// decision R9; the SM.7 fixer's F1): cycle 1 lands; the clock passes cycle 2's due unfired and
/// the world restarts with the licence lapsing at `L` = cycle 2's due + 100 — so cycle 2's finish
/// lands after `L` and reads the standing `Lapsed { L }`. As in the uninterrupted world (which
/// lands it at its due, the licence still in force), the next cycle began before `L`: cycle 2's
/// line carries no rights stop and the run re-arms; cycle 3, beginning after `L`, ends the run
/// with `RUN_RIGHTS` and `LICENCE_LAPSED` filled with `L`.
#[test]
fn sm7_a_late_landing_judges_the_lapse_at_the_due() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let all = picks(&mut rig.tw, AHUNA);
    let fair = fair7(&all);
    launched(&mut rig, pilot, 1, tv);
    let _ = sounded_at(&mut rig, pilot, 1, tv, &fair);
    let _ = act_go(&mut rig, pilot, "+tv/1 mine go");
    let cycles = run_of(&mut rig.tw, tv).expect("the run").cycles;
    assert!(cycles >= 3);
    let _ = cycle(&mut rig, tv, pilot);
    let due = run_of(&mut rig.tw, tv).expect("cycle 2 armed").due_ic;
    let lapse = due + 100;
    // Past the lapse, nothing fired; then the restart with the licence lapsing at `lapse`.
    let rate = i64::from(world_rate(&rig.tw).max(1));
    let real = (lapse - ic_now(&rig.tw)) / rate + 2;
    let t = rig.tw.world().now().saturating_add(real);
    rig.tw.set_now(t);
    let key = licence_key(account_of(&rig, pilot));
    let mut rig = reboot_editing(rig, key.as_str(), &[pilot], |doc| {
        doc["expires_ic"] = json!(lapse);
        doc["grace_secs"] = json!(0);
    });
    to_cell(&mut rig, tv, fair.stand);
    assert!(ic_now(&rig.tw) > lapse, "the landing is late");
    let mut heard = String::new();
    for _ in 0..3 {
        let (events, queued) = fire(&mut rig);
        heard.push_str(&said(&rig.tw, &events, pilot));
        land_all(&mut rig, &queued);
    }
    let line = CYCLE_DONE
        .replace("{n}", "1")
        .replace("{k}", "2")
        .replace("{cycles}", &cycles.to_string())
        .replace("{t}", "2")
        .replace("{ore}", "metal ore")
        .replace("{hopper}", "4 of 12 t");
    assert_eq!(count(&heard, &line), 1, "{heard}\nwanted: {line}");
    assert!(
        !heard.contains("stops drilling"),
        "no stop after cycle 2: {heard}"
    );
    assert_eq!(
        run_of(&mut rig.tw, tv).map(|run| run.k),
        Some(3),
        "re-armed"
    );
    let (third, _) = cycle(&mut rig, tv, pilot);
    let lapsed = LICENCE_LAPSED.replace("{lapses}", &ic_stamp(lapse));
    let line = CYCLE_DONE
        .replace("{n}", "1")
        .replace("{k}", "3")
        .replace("{cycles}", &cycles.to_string())
        .replace("{t}", "2")
        .replace("{ore}", "metal ore")
        .replace("{hopper}", "6 of 12 t")
        + &*RUN_RIGHTS
            .replace("{n}", "1")
            .replace("{addr}", &fair.addr)
            .replace("{line}", &lapsed);
    assert_eq!(count(&third, &line), 1, "{third}\nwanted: {line}");
    assert!(run_of(&mut rig.tw, tv).is_none(), "no cycle 4");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M17. A run holds the rig: only knowledge acts beside it
// ---------------------------------------------------------------------------

/// **M17 — a run admits only knowledge acts** (D-20-9; the brief's 3.4 last bullet, decision R1;
/// the SM.7 fixer's T3): with a run in flight (cycle 1 landed, so the hopper holds ore),
/// `+tv/1 mine` (bare, with a count, with `go`), `+tv/1 recover` and `+tv/1 dump` (bare and with
/// `go`) and `+tv/1 drive <x,y> go` each answer `MINE_RUNNING` filled — the pocket, the cycle in
/// flight, the run's count — with nothing queued, while the bare `+tv/1 drive <x,y>` previews and
/// a scan or a sounding may start (M10 lands one). The run is unchanged by all of it. Then, the
/// run stopped and closed, a `+tv/1 mine go` whose RAM order a restart lost refuses at its
/// landing with the start's `unplanned` line and writes nothing.
#[test]
#[allow(clippy::too_many_lines)]
fn sm7_a_run_admits_only_knowledge_acts() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let all = picks(&mut rig.tw, AHUNA);
    let fair = fair7(&all);
    launched(&mut rig, pilot, 1, tv);
    let _ = sounded_at(&mut rig, pilot, 1, tv, &fair);
    let _ = act_go(&mut rig, pilot, "+tv/1 mine go");
    let (first, _) = cycle(&mut rig, tv, pilot);
    assert!(first.contains("Rig 1, cycle 1 of "), "{first}");
    let run = run_of(&mut rig.tw, tv).expect("cycle 2 armed");
    assert_eq!(run.k, 2);
    let vault = hopper(&rig.tw, tv);
    assert_eq!(pool(&mut rig.tw, vault, "metal_ore"), 2, "ore to dump");
    let head = MINE_RUNNING
        .replace("{n}", "1")
        .replace("{addr}", &fair.addr)
        .replace("{k}", "2")
        .replace("{cycles}", &run.cycles.to_string());
    let (prefix, suffix) = head.split_once("{when}").expect("the due words");
    let target = format!("{},{}", RAMP_CELL.0, RAMP_CELL.1 + 1);
    let mut lines: Vec<String> = [
        "+tv/1 mine",
        "+tv/1 mine 2",
        "+tv/1 mine go",
        "+tv/1 drill",
        "+tv/1 recover",
        "+tv/1 recover go",
        "+tv/1 dump",
        "+tv/1 dump go",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    lines.push(format!("+tv/1 drive {target} go"));
    let mut answers = Vec::new();
    for line in &lines {
        let (out, queued) = command(&mut rig, pilot, line);
        assert!(queued.is_empty(), "{line} queues nothing");
        let out = out.trim().to_owned();
        assert!(
            out.starts_with(prefix) && out.ends_with(suffix),
            "{line}: {out}\nwanted: {head}"
        );
        answers.push(out);
    }
    assert!(
        answers.iter().all(|out| *out == answers[0]),
        "one line: {answers:?}"
    );
    // The drive's bare preview is legal during a run (§6: "the next drive's preview").
    let (out, queued) = command(&mut rig, pilot, &format!("+tv/1 drive {target}"));
    assert!(queued.is_empty());
    assert!(
        out.trim().starts_with("Rig 1 (OWP-TV-01) would drive "),
        "{out}"
    );
    assert_eq!(
        run_of(&mut rig.tw, tv),
        Some(run.clone()),
        "the run unchanged"
    );
    // The run closed; then a start whose RAM order a restart lost.
    let (_, queued) = command(&mut rig, pilot, "+tv/1 mine stop");
    land_all(&mut rig, &queued);
    let _ = cycle(&mut rig, tv, pilot);
    assert!(run_of(&mut rig.tw, tv).is_none());
    let (out, queued) = command(&mut rig, pilot, "+tv/1 mine go");
    assert!(out.contains("Rig 1 starts drilling "), "{out}");
    assert_eq!(queued.len(), 1);
    let mut rig = reboot(rig, &[pilot]);
    to_cell(&mut rig, tv, fair.stand);
    let before = format!("{:?}", record_of(&mut rig.tw, tv));
    let refused = land(&mut rig, &queued[0]).expect_err("an order lost to the restart");
    assert!(
        refused.contains("that act was not planned; preview it again"),
        "{refused}"
    );
    assert_eq!(
        before,
        format!("{:?}", record_of(&mut rig.tw, tv)),
        "nothing written"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M18. A cycle that cannot happen ends the run
// ---------------------------------------------------------------------------

/// **M18 — a cycle that cannot happen ends the run** (the brief's 3.7 steps 2–4 and 3.8; the SM.7
/// fixer's T4). (a) **Moved**: cycle 1 lands; staff move the drilling rig one cell off its stand
/// before cycle 2's due; cycle 2 lands `Refused { Moved }` and the operator hears `CYCLE_FAILED`
/// with `WHY_MOVED` — the run is gone, no charge spent, nothing moved, the cursor's row keeps its
/// draw and its lock is released. (b) **Unworked** (the two-pass protocol, the dump's `SpoilGone`
/// precedent): a fresh run's first finish is submitted and never lands (the world task dropped
/// it); the tick's next fire counts the attempt and submits the same cycle again, and its landing
/// — the offer naming the cycle in `failed` — records `Refused { Unworked }`: `CYCLE_FAILED` with
/// `WHY_UNWORKED`, nothing moved, nothing spent, the run gone and the lock released. The first,
/// dropped submission redelivered is then the ledger's HIT.
#[test]
#[allow(clippy::too_many_lines)]
fn sm7_a_cycle_that_cannot_happen_ends_the_run() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let all = picks(&mut rig.tw, AHUNA);
    let fair = fair7(&all);
    launched(&mut rig, pilot, 1, tv);
    let _ = sounded_at(&mut rig, pilot, 1, tv, &fair);
    let deposit = fair.deposit(AHUNA, OreFamily::Metals);
    let block = block_vault(&rig.tw, AHUNA, fair.letter());
    let vault = hopper(&rig.tw, tv);
    // (a) Moved.
    let _ = act_go(&mut rig, pilot, "+tv/1 mine go");
    let cycles = run_of(&mut rig.tw, tv).expect("the run").cycles;
    let _ = cycle(&mut rig, tv, pilot);
    let charge = value_of(&mut rig.tw, tv);
    let (in_block, in_hopper) = (
        pool(&mut rig.tw, block, "metal_ore"),
        pool(&mut rig.tw, vault, "metal_ore"),
    );
    assert_eq!(in_hopper, 2);
    to_cell(&mut rig, tv, (fair.stand.0 + 1, fair.stand.1));
    let (heard, _) = cycle(&mut rig, tv, pilot);
    let failed = |k: u8, why: &str| {
        CYCLE_FAILED
            .replace("{n}", "1")
            .replace("{k}", &k.to_string())
            .replace("{cycles}", &cycles.to_string())
            .replace("{addr}", &fair.addr)
            .replace("{why}", why)
    };
    assert_eq!(count(&heard, &failed(2, WHY_MOVED)), 1, "{heard}");
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert!(record.run.is_none(), "the run ends");
    assert_eq!(
        record.last.map(|last| last.result),
        Some(ActResult::Refused {
            code: tacsim::surface::RefusedCode::Moved
        })
    );
    assert_eq!(value_of(&mut rig.tw, tv), charge, "nothing spent");
    assert_eq!(
        (
            pool(&mut rig.tw, block, "metal_ore"),
            pool(&mut rig.tw, vault, "metal_ore")
        ),
        (in_block, in_hopper),
        "nothing moved"
    );
    let row = |rig: &mut Rig| {
        cursor_of(&mut rig.tw, deposit)
            .and_then(|cursor| cursor.row(fair.number()).cloned())
            .map(|row| (row.drawn_t, row.lock.is_none()))
    };
    assert_eq!(row(&mut rig), Some((2, true)), "drawn kept, lock released");
    // (b) Unworked, by the two-pass protocol.
    to_cell(&mut rig, tv, fair.stand);
    let _ = act_go(&mut rig, pilot, "+tv/1 mine go");
    let run = run_of(&mut rig.tw, tv).expect("the second run");
    assert_eq!(row(&mut rig), Some((2, false)), "locked by the second run");
    let charge = value_of(&mut rig.tw, tv);
    let (_, queued) = advance_to(&mut rig, run.due_ic);
    let dropped = finishes(&queued);
    assert_eq!(dropped.len(), 1, "the first finish: {queued:?}");
    let dropped = dropped[0].clone();
    let (_, queued) = fire(&mut rig);
    let again = finishes(&queued);
    assert_eq!(again.len(), 1, "the retried finish: {queued:?}");
    assert_eq!(ledger_identity(again[0]), ledger_identity(&dropped));
    land_all(&mut rig, &queued);
    let (events, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    let heard = said(&rig.tw, &events, pilot);
    let unworked = CYCLE_FAILED
        .replace("{n}", "1")
        .replace("{k}", "1")
        .replace("{cycles}", &run.cycles.to_string())
        .replace("{addr}", &fair.addr)
        .replace("{why}", WHY_UNWORKED);
    assert_eq!(count(&heard, &unworked), 1, "{heard}");
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert!(record.run.is_none(), "the run ends");
    assert_eq!(
        record.last.map(|last| last.result),
        Some(ActResult::Refused {
            code: tacsim::surface::RefusedCode::Unworked
        })
    );
    assert_eq!(value_of(&mut rig.tw, tv), charge, "nothing spent");
    assert_eq!(
        (
            pool(&mut rig.tw, block, "metal_ore"),
            pool(&mut rig.tw, vault, "metal_ore")
        ),
        (in_block, in_hopper),
        "nothing moved"
    );
    assert_eq!(row(&mut rig), Some((2, true)), "the lock released");
    assert_eq!(
        land(&mut rig, &dropped),
        Err(HIT.to_owned()),
        "redelivered: the HIT"
    );
    // Nothing more is submitted for the closed run.
    for _ in 0..3 {
        let (_, queued) = fire(&mut rig);
        assert!(finishes(&queued).is_empty(), "the run is closed");
    }
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M19. The reading names the family
// ---------------------------------------------------------------------------

/// A word's rank as the R1 reading orders it (barren 0 … rich 3).
fn word_code(word: &str) -> Option<u8> {
    match word {
        "barren" => Some(0),
        "lean" => Some(1),
        "fair" => Some(2),
        "rich" => Some(3),
        _ => None,
    }
}

/// **The richest family of a field's reading as the operator sees it** — `field_reading`'s
/// text parsed (`"{field} {words} {family} ({lo}–{hi} pockets); …"`, a band `low–high`), each
/// family ranked by its upper word then its lower, ties to `OreFamily::ALL`'s order, `None` when
/// nothing reads above barren (the mine preview's decision R6, read from the player's side).
fn richest_of_text(text: &str) -> Option<OreFamily> {
    let (_, rest) = text.split_once(' ')?;
    let mut best: Option<((u8, u8), OreFamily)> = None;
    for part in rest.split("; ") {
        let part = part.find(" (").map_or(part, |at| &part[..at]);
        let (words, label) = part.split_once(' ')?;
        let Some(family) = OreFamily::ALL.into_iter().find(|f| f.label() == label) else {
            continue;
        };
        let rank = match words.split_once('–') {
            Some((low, high)) => (word_code(high)?, word_code(low)?),
            None => {
                let word = word_code(words)?;
                (word, word)
            }
        };
        if best.is_none_or(|(kept, _)| rank > kept) {
            best = Some((rank, family));
        }
    }
    best.filter(|((high, _), _)| *high > 0)
        .map(|(_, family)| family)
}

/// **M19 — the reading names the family** (§6; decisions R6 and the read default; the SM.7
/// fixer's T5). (a) A read pocket of two families (`TWO`): the bare preview mines the family with
/// the most tonnes left (ties to the richer class, then `OreFamily::ALL`'s order) and adds the
/// other family's `MINE_ALSO` clause; a family the reading shows barren refuses `MINE_NO_ORE`.
/// (b) A read barren pocket refuses `MINE_ALL_BARREN`, bare and with `go`, nothing queued. (c)
/// **The blind default** (R6, owner-critical): after a scan, an unread pocket in a scanned field
/// whose reading is richest in a family other than metals cores for that family — the family the
/// operator's own R1 row shows richest, never metals by default.
#[test]
#[allow(clippy::too_many_lines)]
fn sm7_the_reading_names_the_family() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let all = picks(&mut rig.tw, AHUNA);
    let two = pick(&all, "TWO", |p| {
        p.footing != Footing::Rock && p.families() == 2
    });
    let empty = barren(&all, &[&two]);
    launched(&mut rig, pilot, 1, tv);
    // (a) Two families, read.
    let _ = sounded_at(&mut rig, pilot, 1, tv, &two);
    let class_word = |word: Word| match word {
        Word::Barren => "barren",
        Word::Lean => "lean",
        Word::Fair => "fair",
        Word::Rich => "rich",
    };
    let ore_of = |family: OreFamily| ore_words(tacsim::surface::opening::ore_good(family));
    let verb_word = |family: OreFamily| match family {
        OreFamily::Metals => "metal",
        OreFamily::Volatiles => "ice",
        OreFamily::CarbonFeedstock => "carbon",
        OreFamily::StrategicElements => "strategic",
    };
    let mut best: Option<((u8, u8), OreFamily)> = None;
    for family in OreFamily::ALL {
        let i = usize::from(family.code());
        if two.classes[i] == Word::Barren {
            continue;
        }
        let rank = (two.tonnes[i], two.classes[i].code());
        if best.is_none_or(|(kept, _)| rank > kept) {
            best = Some((rank, family));
        }
    }
    let (_, chosen) = best.expect("an ore family");
    let other = OreFamily::ALL
        .into_iter()
        .find(|f| *f != chosen && two.classes[usize::from(f.code())] != Word::Barren)
        .expect("the second family");
    let (out, queued) = command(&mut rig, pilot, "+tv/1 mine");
    assert!(queued.is_empty());
    let head = format!(
        "Rig 1 (OWP-TV-01) would mine {} {} {}: ",
        two.addr,
        class_word(two.classes[usize::from(chosen.code())]),
        ore_of(chosen)
    );
    let also = MINE_ALSO
        .replace(
            "{class}",
            class_word(two.classes[usize::from(other.code())]),
        )
        .replace("{ore}", &ore_of(other))
        .replace("{n}", "1")
        .replace("{word}", verb_word(other));
    assert!(
        out.trim().starts_with(&head) && out.contains(&format!("{also} +tv/1 mine go starts it.")),
        "{out}\nwanted: {head} … {also}"
    );
    let barren_family = OreFamily::ALL
        .into_iter()
        .find(|f| two.classes[usize::from(f.code())] == Word::Barren)
        .expect("a barren family at TWO");
    for go in ["", " go"] {
        let (out, queued) = command(
            &mut rig,
            pilot,
            &format!("+tv/1 mine {}{go}", verb_word(barren_family)),
        );
        assert!(queued.is_empty());
        assert_eq!(
            out.trim(),
            MINE_NO_ORE
                .replace("{addr}", &two.addr)
                .replace("{ore}", &ore_of(barren_family))
        );
    }
    // (b) A read barren pocket.
    let _ = sounded_at(&mut rig, pilot, 1, tv, &empty);
    for line in ["+tv/1 mine", "+tv/1 mine go"] {
        let (out, queued) = command(&mut rig, pilot, line);
        assert!(queued.is_empty(), "{line}");
        assert_eq!(
            out.trim(),
            MINE_ALL_BARREN.replace("{addr}", &empty.addr),
            "{line}"
        );
    }
    // (c) The blind default: scan until a scanned field reads richest in a family other than
    // metals and holds a pocket the operator has not sounded.
    let fields_side = 64 / tacsim::surface::geology::POCKETS_PER_FIELD_SIDE;
    let mut found: Option<(Pick, OreFamily)> = None;
    for stand in all
        .iter()
        .filter(|p| p.footing != Footing::Ice)
        .step_by(97)
        .take(8)
    {
        to_cell(&mut rig, tv, stand.stand);
        let _ = act_go(&mut rig, pilot, "+tv/1 scan go");
        let _ = finish(&mut rig, tv, pilot);
        let row = chart_of(&mut rig.tw, pilot, AHUNA).expect("the scan's row");
        'fields: for fy in 0..fields_side {
            for fx in 0..fields_side {
                let text = tacsim::surface::field_reading(
                    rig.tw.world(),
                    &cid(AHUNA),
                    Some(&row),
                    (fx, fy),
                );
                let Some(family) = richest_of_text(&text) else {
                    continue;
                };
                if family == OreFamily::Metals {
                    continue;
                }
                if let Some(pocket) = all.iter().find(|p| {
                    tacsim::surface::address::field_of_pocket(p.pocket) == Some((fx, fy))
                        && !row.pocket_sounded(p.pocket)
                }) {
                    println!(
                        "R6: field ({fx},{fy}) reads {text:?}; blind core at {}",
                        pocket.addr
                    );
                    found = Some((pocket.clone(), family));
                    break 'fields;
                }
            }
        }
        if found.is_some() {
            break;
        }
    }
    let (at, family) = found.expect("a scanned field richest in a family other than metals");
    to_cell(&mut rig, tv, at.stand);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 mine");
    assert!(queued.is_empty());
    assert!(
        out.trim()
            .starts_with("No reading here: the first cycle will tell. ")
            && out.contains(&format!(
                "would core {} for {}: 1 cycle, ",
                at.addr,
                ore_of(family)
            )),
        "{out}"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M20. A started run is seen at once, its order leaves, and it holds the lift
// ---------------------------------------------------------------------------

/// **M20 — a started run is seen at once, its order leaves the slot, and it holds the lift** (the
/// SM.7 lead's pre-commit round, items 2, 4 and 5): after `+tv/1 mine go` lands on `FAIR7`, (a)
/// the RAM order is still in the slot and a forged start of the next ordinal refuses "that act
/// was not planned; preview it again" — the offer dropped the landed order **without re-planning
/// it** (a re-plan would answer "that rig is drilling"); (b) one fire, well before the first
/// cycle's due, writes the run into the projection — ordinal, cycle 1 of the run's cycles, the
/// cycle's due — with no cycle landed, and clears the order from the slot; (c) `+ship/lift ceres`
/// refuses `TV_BUSY` with `drilling <pocket>` and the `+tv/1 mine stop` remedy, never `TV_OUT`'s
/// recover, and queues nothing.
#[test]
fn sm7_a_started_run_is_seen_at_once_and_holds_the_lift() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let all = picks(&mut rig.tw, AHUNA);
    let fair = fair7(&all);
    launched(&mut rig, pilot, 1, tv);
    let _ = sounded_at(&mut rig, pilot, 1, tv, &fair);
    let (_, queued) = command(&mut rig, pilot, "+tv/1 mine go");
    assert_eq!(queued.len(), 1, "one start");
    land_all(&mut rig, &queued);
    let run = run_of(&mut rig.tw, tv).expect("the run");
    let last_before = record_of(&mut rig.tw, tv).and_then(|record| record.last);
    // (a) The landed order is still in the slot; the offer drops it without a re-plan.
    assert_eq!(
        tacsim::surface::act::mine_order_for_tests(rig.tw.world(), tv),
        Some(run.ordinal),
        "the order outlives its landing until the tick prunes it"
    );
    let mut forged = queued[0].clone();
    forged.parameters["ordinal"] = json!(run.ordinal + 1);
    forged.key = tacsim::surface::act_key(tv, run.ordinal + 1).expect("a key");
    let refused = land(&mut rig, &forged).expect_err("no live order");
    assert!(
        refused.contains("that act was not planned; preview it again"),
        "{refused}"
    );
    assert_eq!(
        run_of(&mut rig.tw, tv),
        Some(run.clone()),
        "nothing written"
    );
    // (b) One fire, before the first cycle's due: the projection shows the run; nothing landed.
    let (_, queued) = fire(&mut rig);
    assert!(ic_now(&rig.tw) < run.due_ic, "before the first cycle's due");
    assert!(finishes(&queued).is_empty(), "no cycle is due");
    land_all(&mut rig, &queued);
    let view = projection(&rig.tw, tv)
        .and_then(|rec| rec.run)
        .expect("the projection shows the run from its first fire");
    assert_eq!(
        (view.ordinal, view.k, view.cycles, view.due_ic),
        (run.ordinal, 1, run.cycles, run.due_ic)
    );
    assert_eq!(
        record_of(&mut rig.tw, tv).and_then(|record| record.last),
        last_before,
        "no cycle landed"
    );
    assert_eq!(
        tacsim::surface::act::mine_order_for_tests(rig.tw.world(), tv),
        None,
        "the tick pruned the landed order"
    );
    // (c) The departure: busy drilling, the stop as its remedy.
    let busy = format!(
        "{}{}",
        TV_BUSY
            .replace("{n}", "1")
            .replace("{doing}", &format!("drilling {}", fair.addr))
            .replace("{name}", "Salt Moth"),
        RUN_STOP_REMEDY.replace("{n}", "1")
    );
    let (out, queued) = command(&mut rig, pilot, "+ship/lift ceres");
    assert_eq!(out.trim(), busy);
    assert!(queued.is_empty());
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// M21. A dark registry ends the run with the dark line
// ---------------------------------------------------------------------------

/// **M21 — a registry that cannot be read ends the run with the dark line, never a claim** (the
/// SM.7 lead's pre-commit round, item 3): a run on `FAIR7` (open ground) lands cycle 1; then the
/// Registry — Ahuna's issuer — is dropped from every lane port map (RAM only, the store's test
/// hook, SM.6's F3 precedent), so the tenure read at cycle 2's landing is dark. Cycle 2 still
/// yields (rights are per act), its line ends `RUN_DARK` and never `MINE_CLAIMED`; the outcome's
/// end is `Dark { issuer }`; the run is gone and its lock released; and nothing moves after — the
/// hopper, the block vault and the cursor stand through a later sweep past the next dues.
#[test]
fn sm7_a_dark_registry_ends_the_run_with_the_dark_line() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let all = picks(&mut rig.tw, AHUNA);
    let fair = fair7(&all);
    launched(&mut rig, pilot, 1, tv);
    let _ = sounded_at(&mut rig, pilot, 1, tv, &fair);
    let _ = act_go(&mut rig, pilot, "+tv/1 mine go");
    let run = run_of(&mut rig.tw, tv).expect("the run");
    assert!(run.cycles >= 3, "a run long enough to end early");
    let (first, _) = cycle(&mut rig, tv, pilot);
    assert!(!first.contains(MINE_CLAIMED), "{first}");
    let dropped = rig
        .tw
        .world_mut()
        .module_state_mut::<TacsimStore>()
        .expect("tacsim state")
        .drop_lane_org_for_tests(&cid("freeport_registry"));
    assert!(dropped > 0, "the Registry's row was on the maps");
    let (second, _) = cycle(&mut rig, tv, pilot);
    let dark = RUN_DARK.replace("{addr}", &fair.addr);
    assert_eq!(count(&second, &dark), 1, "{second}");
    assert!(
        second.contains(", cycle 2 of ") && second.contains("2 t metal ore; hopper "),
        "the running cycle yields: {second}"
    );
    assert!(
        !second.contains(MINE_CLAIMED),
        "never the claim line: {second}"
    );
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert!(record.run.is_none(), "the run ended");
    let mined = record
        .last
        .and_then(|last| last.mined)
        .expect("what the cycle did");
    assert_eq!(
        mined.end,
        Some(RunEnd::Dark {
            why: tacsim::surface::excursion::DarkCause::Issuer
        })
    );
    assert_eq!(mined.tonnes, 2);
    let deposit = fair.deposit(AHUNA, OreFamily::Metals);
    let cursor = cursor_of(&mut rig.tw, deposit);
    assert_eq!(
        cursor
            .as_ref()
            .and_then(|cursor| cursor.row(fair.number()))
            .map(|row| (row.drawn_t, row.lock.is_none())),
        Some((4, true)),
        "two cycles drawn, the lock released"
    );
    // Nothing moves after.
    let hopper_vault = hopper(&rig.tw, tv);
    let vault = block_vault(&rig.tw, AHUNA, fair.letter());
    let held = pool(&mut rig.tw, hopper_vault, "metal_ore");
    let left = pool(&mut rig.tw, vault, "metal_ore");
    let events = to_ic(&mut rig, run.due_ic + i64::from(run.secs) * 3);
    let heard = said(&rig.tw, &events, pilot);
    assert!(!heard.contains(MINE_CLAIMED), "{heard}");
    assert!(!heard.contains(", cycle 3 of "), "{heard}");
    assert_eq!(pool(&mut rig.tw, hopper_vault, "metal_ore"), held);
    assert_eq!(pool(&mut rig.tw, vault, "metal_ore"), left);
    assert_eq!(cursor_of(&mut rig.tw, deposit), cursor);
    no_refused_writes(&rig.tw);
}
