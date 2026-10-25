//! **Doc 20 cut SM.9 acceptance — the quarry's reads** (doc 20 §1 beat 2, §3.1, §5, §10, §16;
//! D-20-13; the SM.9 brief's §1 acceptance and its tests Q1–Q10 in 3.12).
//!
//! The ground chart: `+chart/ground [<body>]` lists a body's sites with the Registry's survey
//! words, the tenure, the passage now and the viewer's own best reading, and `+chart/ground map`
//! draws the 56-by-28 wash — the `body.ground` frame the TUI's quarry screen and the web's draw
//! too. The `site.map` frame's vector keys the screens read (the addressed rig, the family words,
//! the grid, the marks, the readings' targets, the rigs' lines and runs from the projection, the
//! hold's line, the log, the clock and the glyphs under a moving rig), the pushed `tv.status`
//! keyframe, and the projection's log kept to its current operator's excursion.
//!
//! Composed on the composition root (`mining.rs`'s rig, copied: the per-file precedent),
//! **booted keyed** with the geology key handed in through `tacsim::surface::install_geology`
//! (save the one keyless reboot Q1 takes on purpose). Every act goes through the real verb, the
//! real land path and the real tick; rigs are put in place by the tick-batch placement seam a
//! test may call directly (`mush_sim::place`), never a fake drive verb.

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
use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde_json::{Value, json};
use smol_str::SmolStr;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK};
use tacsim::pack::{PackEntry, RulesPack, load_pack};
use tacsim::port::ground_chart::{
    BODY_GROUND_FEED, GROUND_BEST, GROUND_BEST_DARK, GROUND_BEST_NONE, GROUND_DARK, GROUND_FOOTER,
    GROUND_HEAD, GROUND_HERE, GROUND_NO_BODY, GROUND_UNSURVEYED, GROUND_USAGE, WASH, frame_cell,
};
use tacsim::port::site_map::{MAP_LOG_LINES, MAP_NO_READINGS, TV_STATUS_FEED};
use tacsim::port::tv::{MAP_FAMILY_UNKNOWN, QUEUED_LINE};
use tacsim::port::{HullRow, PortHandles, PortMap, PortMapState, Ticket, TicketId, room_key};
use tacsim::surface::address::pocket_address;
use tacsim::surface::geology::{OreFamily, Word, province_word};
use tacsim::surface::ground::{SiteGeologySpec, SiteGround, site_ground_sync};
use tacsim::surface::reading::Footing;
use tacsim::surface::{
    GeologyBoot, GeologyState, RigKey, RigRecord, SURFACE_ACT_ACTION, SURFACE_DONE_ACTION, TvRec,
    TvRecState,
};
use tacsim::{TacsimConfig, TacsimStore};

use geology_keyed::{KEY_1, mac, secret};

/// The ramp's cell.
const RAMP_CELL: (i64, i64) = (128, 124);
/// The pad's centre cell, where the site map draws the hull's `H` (`port::site_map`'s private
/// `PAD_CELL`).
const PAD_CELL: (i64, i64) = (128, 128);
/// Ahuna Flank's port entry.
const AHUNA: &str = "ahuna_port";
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

/// Boot keyed, settle and stock every port, at world rate 1.
fn stocked() -> Rig {
    let (world, _) = mush_testkit::boot_world();
    let mut tw = boot_over(world, HashMap::new(), true);
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
    reboot_with(rig, reconnect, true)
}

/// [`reboot`] with no geology key handed in: the layer reads dark.
fn reboot_dark(rig: Rig, reconnect: &[ObjId]) -> Rig {
    reboot_with(rig, reconnect, false)
}

/// [`reboot`], keyed or not.
fn reboot_with(rig: Rig, reconnect: &[ObjId], keyed: bool) -> Rig {
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
        ledger,
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

/// A rig's projection.
fn projection(tw: &TestWorld, tv: ObjId) -> Option<TvRec> {
    match store(tw).tv_rec(tv) {
        Some(TvRecState::Present(rec)) => Some(rec.clone()),
        Some(TvRecState::Unreadable) => panic!("the projection is unreadable"),
        None => None,
    }
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

/// Staff place `tv` `Direct` on `entry`'s domain at site cell `(x, y)`'s centre, facing south —
/// and keep the placement's writes for a reboot (doc 20 cut SM.12b, `claims.rs`'s SM.11 precedent:
/// `support::with_ctx` drops its outbox, so a staff placement's recovery-ledger append never
/// reached the store; harmless until a placement sealed a ledger chunk, which the house yard's
/// lay-up and its drones' placements made happen here).
fn to_cell_on(rig: &mut Rig, entry: &str, tv: ObjId, cell: (i64, i64)) {
    let domain = mush_sim::DomainId(port_map(&rig.tw, entry).domain.expect("a domain"));
    let staff = support::place_keeping(&mut rig.tw, domain, tv, cell);
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    fold_module_ops_into(&mut rig.store, staff);
}

/// Staff move `tv` to Ahuna's site cell `(x, y)`.
fn to_cell(rig: &mut Rig, tv: ObjId, cell: (i64, i64)) {
    to_cell_on(rig, AHUNA, tv, cell);
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

// ---------------------------------------------------------------------------
// Doc 20 cut SM.9: the quarry's reads
// ---------------------------------------------------------------------------

/// **The `body.ground` frame `who` gets** through the feed's own query.
fn ground(rig: &mut Rig, who: ObjId, args: Value) -> Value {
    let (mut out, mut fx) = (mush_core::Outbox::new(), mush_core::Effects::new());
    rig.tw.engine_mut().step(
        mush_core::Inbound::Query {
            conn: 7,
            actor: who,
            id: 1,
            name: SmolStr::new(BODY_GROUND_FEED),
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
        .expect("a body.ground frame")
}

/// A frame's `ground.<key>` lines.
fn ground_lines(frame: &Value, key: &str) -> Vec<String> {
    frame["ground"][key]
        .as_array()
        .unwrap_or_else(|| panic!("ground.{key}: {frame}"))
        .iter()
        .map(|line| line.as_str().expect("a line").to_owned())
        .collect()
}

/// What `who` sees of `line`, one printed line each.
fn printed(rig: &mut Rig, who: ObjId, line: &str) -> Vec<String> {
    let ran = rig.tw.run(who, line);
    rig.tw
        .seen_by(&ran, who)
        .iter()
        .flat_map(|text| text.lines().map(str::to_owned).collect::<Vec<_>>())
        .collect()
}

/// A lane location's frame cell on its body's chart (the brief's 3.5: `site_province`, then
/// `frame_cell`).
fn marker_cell(tw: &TestWorld, body: &str, location: &str) -> [u32; 2] {
    let geology = tacsim::surface::body_geology(tw.world(), &cid(body)).expect("a table");
    let lane = lane();
    let Some(PackEntry::Location(def)) = lane.entry(&cid(location)) else {
        panic!("{location} is a location");
    };
    frame_cell(tacsim::port::site::site_province(def, &geology).expect("a province"))
}

/// The `tv.status` keyframes among `events`, with their audience.
fn statuses(events: &[(Audience, OutputEvent)]) -> Vec<(Audience, Value)> {
    events
        .iter()
        .filter_map(|(audience, event)| match event {
            OutputEvent::Panel {
                feed,
                kind: mush_proto::PanelFrameKind::Keyframe,
                data,
            } if feed.as_str() == TV_STATUS_FEED => Some((audience.clone(), data.clone())),
            _ => None,
        })
        .collect()
}

/// **Finish `tv`'s act** as [`finish`] does, every event of both fires returned.
fn finish_events(rig: &mut Rig, tv: ObjId) -> Vec<(Audience, OutputEvent)> {
    let due = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .map(|act| act.due_ic)
        .expect("an act in flight");
    let (mut events, queued) = advance_to(rig, due);
    land_all(rig, &queued);
    let (more, queued) = fire(rig);
    land_all(rig, &queued);
    events.extend(more);
    events
}

/// Every object key anywhere in `value`.
fn keys_of(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                out.push(key.clone());
                keys_of(inner, out);
            }
        }
        Value::Array(items) => {
            for inner in items {
                keys_of(inner, out);
            }
        }
        _ => {}
    }
}

/// An Ahuna pocket with metal ore at a soft stand.
fn ore_pick(all: &[Pick]) -> Pick {
    pick(all, "ORE", |p| {
        p.footing != Footing::Rock && p.classes[0] != Word::Barren
    })
}

// ---------------------------------------------------------------------------
// Q1. The wash
// ---------------------------------------------------------------------------

/// **Q1 — the ground frame is the Registry's wash** (doc 20 §3.1, §5's R0; the brief's 3.5): a
/// walker's Ceres frame is 56 × 28 at `cell_columns 1`; every frame cell not under a marker is
/// `WASH`'s glyph of `province_word(key, ceres, (x, y), metals)` (row 0 the south edge, as the
/// grid shape's every frame — decision D17 as built); the Freeport `F`, Ahuna `A`, Juling `J`
/// and Kerwan `K` stand at their own provinces, Freeport Commons `C` and Occator Rim `O` share
/// the Freeport's and are listed in `also`; a northern place is in a row `≥ 14`, a southern one
/// below; `family=volatiles` changes the wash; the legend is the four wash pairs then the
/// markers; Vesta's frame marks `R`; Mars answers `GROUND_UNSURVEYED`, Pluto `GROUND_NO_BODY`
/// naming the surveyed bodies, and a keyless reboot `GROUND_DARK`.
#[test]
#[allow(clippy::too_many_lines)]
fn sm9_the_ground_frame_is_the_registry_wash() {
    let mut rig = stocked();
    let walker = pilot(&mut rig, "Walker", 3);
    let frame = ground(&mut rig, walker, json!({ "body": "ceres" }));
    assert_eq!(frame["schema"]["id"], BODY_GROUND_FEED);
    assert_eq!(frame["schema"]["shape"], "grid");
    assert_eq!(
        (&frame["width"], &frame["height"], &frame["cell_columns"]),
        (&json!(56), &json!(28), &json!(1))
    );
    assert_eq!(frame["cell_cm"], 0);
    let geology = tacsim::surface::body_geology(rig.tw.world(), &cid("ceres")).expect("ceres");
    let key = mac(KEY_1);
    let rows = frame_rows(&frame);
    let f = marker_cell(&rig.tw, "ceres", "ceres_freeport");
    let a = marker_cell(&rig.tw, "ceres", "ahuna_flank");
    let c = marker_cell(&rig.tw, "ceres", "freeport_commons");
    let j = marker_cell(&rig.tw, "ceres", "juling_shadow");
    let k = marker_cell(&rig.tw, "ceres", "kerwan_floor");
    let o = marker_cell(&rig.tw, "ceres", "occator_rim");
    assert_eq!(
        (c, o),
        (f, f),
        "the Commons and Occator share the Freeport's province"
    );
    let at = |cell: [u32; 2]| {
        glyph_at(
            &rows,
            (
                usize::try_from(cell[0]).expect("fits"),
                usize::try_from(cell[1]).expect("fits"),
            ),
        )
    };
    for (cell, glyph) in [(f, 'F'), (a, 'A'), (j, 'J'), (k, 'K')] {
        assert_eq!(at(cell), Some(glyph), "{glyph} at {cell:?}");
    }
    assert!(
        f[1] >= 14,
        "the Freeport (19.7 N) is in a northern row: {f:?}"
    );
    assert!(a[1] < 14, "Ahuna (10.5 S) is in a southern row: {a:?}");
    let markers: Vec<[u32; 2]> = vec![f, a, j, k];
    for y in 0..28_u32 {
        for x in 0..56_u32 {
            if markers.contains(&[x, y]) {
                continue;
            }
            let word = province_word(&key, &geology, (x, y), OreFamily::Metals).expect("a word");
            let glyph = WASH
                .iter()
                .find(|(w, _)| *w == word)
                .map(|(_, g)| *g)
                .expect("a wash glyph");
            assert_eq!(at([x, y]), Some(glyph), "({x}, {y})");
        }
    }
    assert_eq!(
        frame["ground"]["also"],
        json!(["C Freeport Commons", "O Occator Rim"])
    );
    assert_eq!(
        frame["legend"],
        json!([
            [".", "barren"],
            ["-", "lean"],
            ["+", "fair"],
            ["*", "rich"],
            ["F", "Ceres Freeport"],
            ["A", "Ahuna Flank"],
            ["C", "Freeport Commons"],
            ["J", "Juling Shadow"],
            ["K", "Kerwan Floor"],
            ["O", "Occator Rim"],
        ])
    );
    let drawn: Vec<(String, bool)> = frame["ground"]["sites"]
        .as_array()
        .expect("sites")
        .iter()
        .map(|s| {
            (
                s["letter"].as_str().expect("a letter").to_owned(),
                s["drawn"].as_bool().expect("drawn"),
            )
        })
        .collect();
    assert_eq!(
        drawn,
        [
            ("A".to_owned(), true),
            ("C".to_owned(), false),
            ("J".to_owned(), true),
            ("K".to_owned(), true),
            ("O".to_owned(), false)
        ]
    );
    assert_eq!(frame["ground"]["ports"][0]["glyph"], "F");
    assert_eq!(frame["ground"]["family_word"], "metals");
    assert_eq!(
        frame["ground"]["families"],
        json!(["metals", "volatiles", "carbon", "strategic"])
    );
    // Another family, another wash.
    let ice = ground(
        &mut rig,
        walker,
        json!({ "body": "ceres", "family": "volatiles" }),
    );
    assert_ne!(ice["rows"], frame["rows"]);
    assert_eq!(ice["ground"]["family_word"], "volatiles");
    // Vesta marks Rheasilvia.
    let vesta = ground(&mut rig, walker, json!({ "body": "vesta" }));
    assert_eq!(vesta["ground"]["sites"][0]["letter"], "R");
    let r = marker_cell(&rig.tw, "vesta", "rheasilvia_commons");
    assert_eq!(
        glyph_at(
            &frame_rows(&vesta),
            (
                usize::try_from(r[0]).expect("fits"),
                usize::try_from(r[1]).expect("fits")
            )
        ),
        Some('R')
    );
    // The refusals, each the reason frame's title.
    let title = |frame: &Value| frame["title"].as_str().expect("a title").to_owned();
    let mars = ground(&mut rig, walker, json!({ "body": "mars" }));
    assert_eq!(title(&mars), GROUND_UNSURVEYED.replace("{body}", "Mars"));
    assert_eq!(mars["width"], 0);
    let pluto = ground(&mut rig, walker, json!({ "body": "pluto" }));
    assert_eq!(
        title(&pluto),
        GROUND_NO_BODY.replace("{list}", "ceres, vesta")
    );
    let unknown = ground(
        &mut rig,
        walker,
        json!({ "body": "ceres", "family": "gold" }),
    );
    assert_eq!(
        title(&unknown),
        MAP_FAMILY_UNKNOWN.replace("{word}", "gold")
    );
    let mut rig = reboot_dark(rig, &[walker]);
    let dark = ground(&mut rig, walker, json!({ "body": "ceres" }));
    assert_eq!(title(&dark), GROUND_DARK);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// Q2. The wash is everyone's; a best reading is its viewer's
// ---------------------------------------------------------------------------

/// **Q2 — the wash is the same for two viewers and a best reading is its viewer's** (§5, §16;
/// lead ruling 5): A on Ahuna's pad sounds an ore pocket; A's and the walker B's Ceres frames have
/// equal `rows` and `legend`; A's Ahuna `best` is `best_reading`'s text naming A's pocket, B's is
/// `null` and B's lines say `GROUND_BEST_NONE`; B's whole frame and B's `+chart/ground ceres`
/// print are byte-identical before and after A's sounding. And for one viewer at one instant
/// the feed's frame and the command's print agree (D5: one frame, three renderers).
#[test]
fn sm9_the_wash_is_the_same_for_two_viewers_and_a_best_reading_is_its_viewers() {
    let mut rig = stocked();
    let (a, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let b = pilot(&mut rig, "Walker", 3);
    let all = picks(&mut rig.tw, AHUNA);
    let ore = ore_pick(&all);
    let ceres = json!({ "body": "ceres" });
    let b_before = ground(&mut rig, b, ceres.clone());
    let b_print_before = printed(&mut rig, b, "+chart/ground ceres");
    launched(&mut rig, a, 1, tv);
    let _ = sounded_at(&mut rig, a, 1, tv, &ore);
    let a_frame = ground(&mut rig, a, ceres.clone());
    let b_after = ground(&mut rig, b, ceres.clone());
    assert_eq!(a_frame["rows"], b_after["rows"], "one wash for everyone");
    assert_eq!(a_frame["legend"], b_after["legend"]);
    let ahuna = |frame: &Value| {
        frame["ground"]["sites"]
            .as_array()
            .expect("sites")
            .iter()
            .find(|s| s["entry"] == AHUNA)
            .cloned()
            .expect("Ahuna's row")
    };
    let best = ahuna(&a_frame)["best"]
        .as_str()
        .expect("A's best reading")
        .to_owned();
    // The best names a pocket A's own sounding read: one of A's map readings in that family,
    // the door's own words.
    let family = [
        "metals",
        "volatiles",
        "carbon feedstock",
        "strategic elements",
    ]
    .into_iter()
    .zip(["metals", "volatiles", "carbon", "strategic"])
    .find(|(label, _)| best.ends_with(label))
    .map(|(_, word)| word)
    .expect("a family label");
    let a_map = map_frame(
        &mut rig,
        a,
        json!({ "view": "drive", "rig": 1, "family": family }),
    );
    assert!(
        a_map["site"]["readings"]
            .as_array()
            .expect("readings")
            .iter()
            .any(|r| r["text"] == best.as_str()),
        "{best} is one of A's own readings"
    );
    assert_eq!(ahuna(&a_frame)["here"], true);
    assert!(
        ground_lines(&a_frame, "lines")
            .iter()
            .any(|line| line == &GROUND_BEST.replace("{reading}", &best))
    );
    assert_eq!(ahuna(&b_after)["best"], Value::Null);
    let b_lines = ground_lines(&b_after, "lines");
    let at_ahuna = b_lines
        .iter()
        .position(|line| line.contains(" Ahuna Flank "))
        .expect("Ahuna's line");
    assert_eq!(
        b_lines.get(at_ahuna + 3).map(String::as_str),
        Some(GROUND_BEST_NONE)
    );
    // Each site's `best_line` is the list's own sentence, unindented — what a screen shows.
    assert_eq!(
        ahuna(&a_frame)["best_line"],
        GROUND_BEST.replace("{reading}", &best).trim_start()
    );
    assert_eq!(ahuna(&b_after)["best_line"], GROUND_BEST_NONE.trim_start());
    assert_eq!(
        serde_json::to_string(&b_after).expect("encodes"),
        serde_json::to_string(&b_before).expect("encodes"),
        "B's frame is byte-identical after A's sounding"
    );
    assert_eq!(printed(&mut rig, b, "+chart/ground ceres"), b_print_before);
    // One viewer, one instant: the feed's frame is the command's print.
    for (who, frame) in [(a, &a_frame), (b, &b_after)] {
        assert_eq!(
            printed(&mut rig, who, "+chart/ground ceres"),
            ground_lines(frame, "lines")
        );
        assert_eq!(
            printed(&mut rig, who, "+chart/ground map ceres"),
            ground_lines(frame, "print")
        );
    }
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// Q3. The verb
// ---------------------------------------------------------------------------

/// **Q3 — `+chart/ground` prints the list and the map** (§1 beat 2, §3.1; decision D3): from
/// Ahuna's pad the bare verb defaults to Ceres and marks Ahuna `GROUND_HERE`, its passage at the
/// charter's load; the list is the frame's `ground.lines` and the map its `ground.print`, every
/// line within 118 columns; `map ceres ice` shows volatiles; extra words print `GROUND_USAGE`;
/// nothing is written or submitted.
#[test]
fn sm9_chart_ground_prints_the_list_and_the_map() {
    let mut rig = stocked();
    let (a, _, _) = on_pad(&mut rig, 1);
    drop(rig.tw.take_module_ops());
    let frame = ground(&mut rig, a, json!({}));
    assert_eq!(
        frame["ground"]["body"], "ceres",
        "the docked body by default"
    );
    let listed = printed(&mut rig, a, "+chart/ground");
    assert_eq!(listed, ground_lines(&frame, "lines"));
    assert_eq!(printed(&mut rig, a, "+chart/ground ceres"), listed);
    assert_eq!(
        listed.first().map(String::as_str),
        Some(GROUND_HEAD.replace("{body}", "Ceres").as_str())
    );
    let here = listed
        .iter()
        .find(|line| line.contains(" Ahuna Flank "))
        .expect("Ahuna's line");
    // The site the hull stands at: where you are, and no passage to itself.
    assert!(here.ends_with(&format!("{GROUND_HERE}.")), "{here}");
    let ahuna = frame["ground"]["sites"]
        .as_array()
        .expect("sites")
        .iter()
        .find(|s| s["entry"] == AHUNA)
        .cloned()
        .expect("Ahuna's row");
    assert_eq!(ahuna["here"], true);
    assert_eq!(ahuna["passage"], Value::Null);
    // A site's pad clears only for the Freeport: every other site reads no passage from here.
    let heads: Vec<&String> = listed
        .iter()
        .filter(|line| line.starts_with("  ") && !line.starts_with("    "))
        .skip(1)
        .filter(|line| !line.contains(" Ahuna Flank "))
        .collect();
    assert!(!heads.is_empty());
    assert!(
        heads
            .iter()
            .all(|line| line.ends_with(": no passage from Ahuna Flank.")),
        "{listed:?}"
    );
    assert!(
        listed.iter().all(|line| !line.contains("where you are: ")),
        "{listed:?}"
    );
    let map = printed(&mut rig, a, "+chart/ground map ceres");
    assert_eq!(map, ground_lines(&frame, "print"));
    assert_eq!(
        map.first().map(String::as_str),
        Some("Ceres: the Registry's survey of metals")
    );
    assert!(map.iter().any(|line| line == GROUND_FOOTER));
    for line in listed.iter().chain(map.iter()) {
        assert!(line.chars().count() <= 118, "{line}");
    }
    let ice = printed(&mut rig, a, "+chart/ground map ceres ice");
    assert_eq!(
        ice.first().map(String::as_str),
        Some("Ceres: the Registry's survey of volatiles")
    );
    for bad in [
        "+chart/ground ceres extra words",
        "+chart/ground ceres ice",
        "+chart/ground map ceres ice metals",
    ] {
        assert_eq!(printed(&mut rig, a, bad), [GROUND_USAGE], "{bad}");
    }
    assert!(drain(&mut rig.tw).is_empty(), "nothing submitted");
    let written: Vec<SmolStr> = rig
        .tw
        .take_module_ops()
        .into_iter()
        .filter(|op| op.ns.as_str() == tacsim::NS)
        .map(|op| op.ns)
        .collect();
    assert!(written.is_empty(), "nothing written: {written:?}");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// Q4. `+chart/body` untouched
// ---------------------------------------------------------------------------

/// **Q4 — `+chart/body ceres` still prints the elements** (the brief's §1 acceptance): its first
/// line names Ceres's position and an `Elements at` line follows.
#[test]
fn sm9_chart_body_is_untouched() {
    let mut rig = stocked();
    let walker = pilot(&mut rig, "Walker", 3);
    let lines = printed(&mut rig, walker, "+chart/body ceres");
    let first = lines.first().expect("a line");
    assert!(
        first.starts_with("ceres on ")
            && first.contains(": r ")
            && first.contains(" AU, ecliptic longitude "),
        "{first}"
    );
    assert!(
        lines.iter().any(|line| line.starts_with("  Elements at ")),
        "{lines:?}"
    );
}

// ---------------------------------------------------------------------------
// Q5. The blessed ground fixture
// ---------------------------------------------------------------------------

/// Where a blessed fixture lives (`tests/driving.rs`'s precedent).
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
             cargo test -p tacsim --test quarry sm9_the_ground_fixture_is_blessed"
        );
    }
}

/// **Q5 — the ground fixture is blessed** (the brief's 3.11): the Ceres metals frame for a
/// walker in the Freeport's berth gallery (no helm, no rows: `best` null everywhere) under the
/// fixed key is `web/src/lib/fixtures/site/body_ground.json`, byte for byte; its print's 28 grid
/// rows are its own rows through `mush_ansi::panel::window_rows`, north up, one character a cell.
#[test]
fn sm9_the_ground_fixture_is_blessed() {
    let mut rig = stocked();
    let walker = pilot(&mut rig, "Walker", 3);
    let gallery = station_room(&rig.tw, "ceres_port", "room.c01_berth_gallery");
    teleport(&mut rig.tw, walker, gallery);
    let frame = ground(
        &mut rig,
        walker,
        json!({ "body": "ceres", "family": "metals" }),
    );
    for site in frame["ground"]["sites"].as_array().expect("sites") {
        assert_eq!(site["best"], Value::Null, "{site}");
    }
    let print = ground_lines(&frame, "print");
    let legend_lines = print
        .iter()
        .skip(1)
        .take_while(|line| line.starts_with("legend:") || line.starts_with("  "))
        .count();
    let grid: Vec<String> = frame_rows(&frame).into_iter().rev().collect();
    assert_eq!(grid.len(), 28);
    assert_eq!(
        print[1 + legend_lines..1 + legend_lines + 28].to_vec(),
        grid,
        "the print is the rows, north up"
    );
    assert!(grid.iter().all(|row| row.chars().count() == 56));
    let pretty = |value: &Value| serde_json::to_string_pretty(value).expect("encodes") + "\n";
    bless("body_ground.json", &pretty(&frame));
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// Q6. What the screens read off `site.map`
// ---------------------------------------------------------------------------

/// **Q6 — the `site.map` frame carries what the screens read** (the brief's 3.3; the 0z
/// ruling 1): with rig 1 drilling a pocket it sounded and rig 2 driving, the drive view names the
/// addressed rig, the family word and the four words, a grid that maps the addressed rig's frame
/// cell back to its own site cell, the marks (each only where the grid shows that glyph; the
/// slot digits with two rigs out), the readings' targets and lines, each rig's line equal to the
/// one `site.lines` prints, rig 1's run equal to the **projection's** run with its clause in
/// `MAP_RUN_CLAUSE`'s words and no class, tonnes or yield key anywhere in the frame, the hold's
/// line, the log, and — while rig 2's leg is in flight — the clock and the glyphs under the rigs,
/// rig 2's `@` where `leg_position` at the frame's own instant puts it; `+tv/map`'s lines carry
/// no run clause; the legend pairs name `v`, `V` and `#`; a second principal's frame has no ore
/// glyph and lists the rigs as `other-{k}` only.
#[test]
#[allow(clippy::too_many_lines)]
fn sm9_the_site_map_frame_carries_what_the_screens_read() {
    let mut rig = stocked();
    let (a, _, tvs) = on_pad(&mut rig, 2);
    let (tv1, tv2) = (tvs[0], tvs[1]);
    let all = picks(&mut rig.tw, AHUNA);
    let fair = fair7(&all);
    let far = barren(&all, &[&fair]);
    launched(&mut rig, a, 1, tv1);
    launched(&mut rig, a, 2, tv2);
    let _ = sounded_at(&mut rig, a, 1, tv1, &fair);
    let _ = act_go(&mut rig, a, "+tv/1 mine go");
    // The started run is projected on the next fire.
    let (_, queued) = fire(&mut rig);
    land_quietly(&mut rig, &queued);
    let projected = projection(&rig.tw, tv1).expect("rig 1's projection");
    let run_view = projected.run.clone().expect("the projected run");
    let _ = act_go(&mut rig, a, &format!("+tv/2 drive {} go", far.addr));
    let (_, queued) = fire(&mut rig);
    land_quietly(&mut rig, &queued);
    let act = record_of(&mut rig.tw, tv2)
        .and_then(|record| record.act)
        .expect("rig 2 drives");
    assert!(ic_now(&rig.tw) < act.due_ic, "still under way");
    let frame = map_frame(
        &mut rig,
        a,
        json!({ "view": "drive", "rig": 1, "family": "metals" }),
    );
    let site = &frame["site"];
    assert_eq!(site["addressed"], 1);
    assert_eq!(site["family_word"], "metals");
    assert_eq!(
        site["families"],
        json!(["metals", "volatiles", "carbon", "strategic"])
    );
    // The grid: the addressed rig's frame cell maps back to its own site cell.
    let (x0, y0, span) = (
        site["grid"]["x0"].as_i64().expect("x0"),
        site["grid"]["y0"].as_i64().expect("y0"),
        site["grid"]["span"].as_i64().expect("span"),
    );
    assert_eq!(span, 1);
    assert_eq!(site["grid"]["cell_cm"], 1_000);
    let you = (
        frame["you"][0].as_i64().expect("fx"),
        frame["you"][1].as_i64().expect("fy"),
    );
    assert_eq!(
        (x0 + you.0 * span + span / 2, y0 + you.1 * span + span / 2),
        fair.stand,
        "a clicked frame cell drives where the click said"
    );
    // The marks: each where the grid shows that very glyph.
    let rows = frame_rows(&frame);
    let cell = |v: &Value| {
        (
            usize::try_from(v[0].as_u64().expect("x")).expect("fits"),
            usize::try_from(v[1].as_u64().expect("y")).expect("fits"),
        )
    };
    for letter in site["marks"]["letters"].as_array().expect("letters") {
        let glyph = letter[2].as_str().and_then(|s| s.chars().next());
        assert_eq!(glyph_at(&rows, cell(letter)), glyph, "{letter}");
    }
    for slot in site["marks"]["slots"].as_array().expect("slots") {
        let digit = char::from_digit(
            u32::try_from(slot[2].as_u64().expect("n")).expect("fits"),
            10,
        );
        assert_eq!(glyph_at(&rows, cell(slot)), digit, "{slot}");
    }
    // The drive view around rig 1 shows neither the pad nor rig 2 (far off, driving): no hull
    // mark and — one of the viewer's rigs in the window — no slot digit.
    assert_eq!(site["marks"]["hull"], Value::Null);
    assert_eq!(site["marks"]["slots"], json!([]));
    // The whole site: the letters at the block corners, the hull at the pad, both rigs out and
    // so their slot digits — each mark where the grid shows that very glyph.
    let whole = map_frame(&mut rig, a, json!({ "view": "site", "family": "metals" }));
    let whole_rows = frame_rows(&whole);
    let marks = &whole["site"]["marks"];
    let (wx0, wy0, wspan) = (
        whole["site"]["grid"]["x0"].as_i64().expect("x0"),
        whole["site"]["grid"]["y0"].as_i64().expect("y0"),
        whole["site"]["grid"]["span"].as_i64().expect("span"),
    );
    assert_eq!(wspan, 8);
    let pad = (
        usize::try_from((PAD_CELL.0 - wx0).div_euclid(wspan)).expect("fits"),
        usize::try_from((PAD_CELL.1 - wy0).div_euclid(wspan)).expect("fits"),
    );
    assert_eq!(cell(&marks["hull"]), pad, "the hull at the pad: {marks}");
    assert_eq!(glyph_at(&whole_rows, pad), Some('H'));
    let letters = marks["letters"].as_array().expect("letters");
    assert!(!letters.is_empty(), "the block letters: {marks}");
    for letter in letters {
        let glyph = letter[2].as_str().and_then(|s| s.chars().next());
        assert!(glyph.is_some_and(|g| g.is_ascii_uppercase()));
        assert_eq!(glyph_at(&whole_rows, cell(letter)), glyph, "{letter}");
    }
    let slots = marks["slots"].as_array().expect("slots");
    assert!(!slots.is_empty(), "two rigs out: their digits: {marks}");
    for slot in slots {
        let digit = char::from_digit(
            u32::try_from(slot[2].as_u64().expect("n")).expect("fits"),
            10,
        );
        assert_eq!(glyph_at(&whole_rows, cell(slot)), digit, "{slot}");
    }
    // The readings: their targets and lines, as the readings line joins them.
    let readings = site["readings"].as_array().expect("readings");
    assert!(!readings.is_empty());
    let joined = site["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    for reading in readings {
        let line = reading["line"].as_str().expect("a line");
        assert!(line.starts_with(reading["text"].as_str().expect("text")));
        assert!(
            reading["text"]
                .as_str()
                .expect("text")
                .starts_with(reading["target"].as_str().expect("a target"))
        );
        assert!(joined.contains(line), "{line} in {joined}");
    }
    assert!(
        readings.iter().any(|r| r["target"] == fair.addr.as_str()),
        "the sounded pocket"
    );
    // The rigs: each line is the one `site.lines` prints; rig 1's run is the projection's.
    for rig_row in site["rigs"].as_array().expect("rigs") {
        let line = rig_row["line"].as_str().expect("a rig line");
        assert!(joined.contains(line), "{line}");
        assert!(
            !line.contains("drilling"),
            "no run clause in the rig's line"
        );
    }
    let run = &site["rigs"][0]["run"];
    assert_eq!(run["addr"], run_view.addr.as_str());
    assert_eq!(run["k"], run_view.k);
    assert_eq!(run["cycles"], run_view.cycles);
    assert_eq!(run["due_ic"], run_view.due_ic);
    let text = run["text"].as_str().expect("the run clause");
    assert!(
        text.starts_with(&format!(
            "drilling {} for metal ore, cycle 1 of {}, this cycle done ",
            fair.addr, run_view.cycles
        )),
        "{text}"
    );
    let mut keys = Vec::new();
    keys_of(&frame, &mut keys);
    for truth in ["class_t", "pocket_t", "reserve_t", "yield", "tonnes"] {
        assert!(!keys.iter().any(|k| k == truth), "{truth} in the frame");
    }
    assert!(
        site["hold"]["line"]
            .as_str()
            .is_some_and(|line| line.starts_with("Hold: ") && joined.contains(line))
    );
    let log = site["log"].as_array().expect("a log");
    assert!(!log.is_empty() && log.len() <= MAP_LOG_LINES, "{log:?}");
    let launch = record_of(&mut rig.tw, tv1)
        .and_then(|record| record.excursion)
        .map(|excursion| excursion.id.launch_ic)
        .expect("rig 1's excursion");
    assert!(
        log.iter()
            .all(|l| l["at_ic"].as_i64().expect("at") >= launch)
    );
    // The moving rig: the clock, the glyphs under the rigs, the `@` where its leg says.
    let clock = &site["clock"];
    assert_eq!(clock["now_ic"], ic_now(&rig.tw));
    assert_eq!(clock["rate"], world_rate(&rig.tw));
    let leg = &site["rigs"][1]["leg"];
    let waypoints: Vec<tacsim::surface::LegWaypoint> =
        serde_json::from_value(leg["waypoints"].clone()).expect("waypoints");
    let started = leg["started_ic"].as_i64().expect("started");
    let (lx, ly) = tacsim::port::site_map::leg_position(&waypoints, ic_now(&rig.tw) - started)
        .expect("a position");
    let site_cell = (lx.div_euclid(1_000), ly.div_euclid(1_000));
    assert_eq!(
        site["rigs"][1]["cell"],
        json!([site_cell.0, site_cell.1]),
        "the server places rig 2 where interpolation at the frame's instant does"
    );
    assert!(!site["under"].as_array().expect("under").is_empty());
    // `+tv/map` prints no run clause; its print is the frame's.
    let map = printed(&mut rig, a, "+tv/1 map");
    assert!(
        map.iter().all(|line| !line.contains("drilling ")),
        "{map:?}"
    );
    let legend: Vec<&str> = frame["legend"]
        .as_array()
        .expect("legend")
        .iter()
        .filter_map(|pair| pair[0].as_str())
        .collect();
    for glyph in ["v", "V", "#"] {
        assert!(legend.contains(&glyph), "{glyph}");
    }
    // A second principal: no ore glyph, the rigs as `other-{k}`.
    let (b, _, _) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    let theirs = map_frame(&mut rig, b, json!({ "view": "site" }));
    assert!(
        frame_rows(&theirs)
            .iter()
            .all(|row| !row.chars().any(|c| ORE_GLYPHS.contains(&c)))
    );
    for unit in theirs["units"].as_array().expect("units") {
        if unit["glyph"] == "o" {
            assert!(
                unit["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("other-"))
            );
        }
    }
    assert_eq!(theirs["site"]["log"], json!([]));
    // The side panes' two sentences ride as keys, whole (the pre-commit round; the judge's
    // correctness/F10): A reads pockets, so no no-readings sentence; B reads none, so B's is
    // `MAP_NO_READINGS` exactly, the very line `lines` prints; A's two rigs are B's other rigs,
    // and `lines`' other-rigs run, its continuations unwrapped, is that sentence word for word.
    assert_eq!(site["no_readings"], Value::Null);
    let theirs_lines: Vec<&str> = theirs["site"]["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let none = MAP_NO_READINGS.replace(
        "{family}",
        theirs["site"]["family"].as_str().expect("a family"),
    );
    assert_eq!(theirs["site"]["no_readings"], none.as_str());
    assert_eq!(theirs_lines.first().copied(), Some(none.as_str()));
    let other_rigs = theirs["site"]["other_rigs"]
        .as_str()
        .expect("A's rigs are B's other rigs");
    let at = theirs_lines
        .iter()
        .position(|line| line.starts_with("Other rigs: "))
        .expect("the other-rigs line");
    let mut unwrapped = theirs_lines[at].to_owned();
    for more in theirs_lines[at + 1..]
        .iter()
        .take_while(|line| line.starts_with("  "))
    {
        unwrapped.push(' ');
        unwrapped.push_str(more.trim_start());
    }
    assert_eq!(other_rigs, unwrapped);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// Q7. `tv.status`
// ---------------------------------------------------------------------------

/// **Q7 — `tv.status` is pushed on each projection change** (the brief's 3.4, D13): a scan's due
/// step and its tell, and a drive's first fire (the start the tick projects at once: an act with
/// a leg, SM.4b's P8 — a scan has none, so its first fire changes nothing and pushes nothing),
/// each push the operator exactly one keyframe whose `rig` is the stored projection right after
/// that fire, marked `retain: "latest"` (a whole-state keyframe: the session keeps only the
/// newest beside the resume ring); a walker hears none; a fire that writes nothing pushes
/// nothing; a line-mode render of the keyframe prints nothing (shape `status`).
#[test]
#[allow(clippy::too_many_lines)]
fn sm9_tv_status_is_pushed_on_each_projection_change() {
    let mut rig = stocked();
    let (a, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let b = pilot(&mut rig, "Walker", 3);
    launched(&mut rig, a, 1, tv);
    let _ = act_go(&mut rig, a, "+tv/1 scan go");
    let due = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .map(|act| act.due_ic)
        .expect("a scan in flight");
    let stored_now = |rig: &Rig| {
        serde_json::to_value(projection(&rig.tw, tv).expect("the projection")).expect("encodes")
    };
    // Each step: exactly one keyframe, to the operator, carrying the projection as stored.
    let one = |pushed: &[(Audience, Value)], stored: &Value, step: &str| {
        assert_eq!(pushed.len(), 1, "one keyframe at {step}: {pushed:?}");
        let (audience, data) = &pushed[0];
        assert_eq!(*audience, Audience::Actor(a), "only the operator at {step}");
        assert_ne!(*audience, Audience::Actor(b));
        assert_eq!(
            &data["rig"], stored,
            "the keyframe is the stored projection at {step}"
        );
        assert_eq!(
            data["schema"],
            json!({ "id": TV_STATUS_FEED, "shape": "status", "version": 1, "retain": "latest" }),
            "a whole-state keyframe at {step}"
        );
    };
    // A scan's first fire: no leg to project, nothing changed, nothing pushed.
    let (events, queued) = fire(&mut rig);
    assert!(ic_now(&rig.tw) < due, "the first fire is before the due");
    land_all(&mut rig, &queued);
    assert!(
        statuses(&events).is_empty(),
        "a scan's first fire writes no projection"
    );
    // The due step.
    let (events, queued) = advance_to(&mut rig, due);
    let at_due = statuses(&events);
    one(&at_due, &stored_now(&rig), "the due");
    land_all(&mut rig, &queued);
    // The tell.
    let (events, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    let at_tell = statuses(&events);
    one(&at_tell, &stored_now(&rig), "the tell");
    let data = at_tell[0].1.clone();
    let (events, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    assert!(
        statuses(&events).is_empty(),
        "a fire that writes nothing pushes nothing"
    );
    // A drive: its first fire projects the start (the leg) at once — exactly one keyframe.
    let all = picks(&mut rig.tw, AHUNA);
    let far = barren(&all, &[]);
    let _ = act_go(&mut rig, a, &format!("+tv/1 drive {} go", far.addr));
    let arrives = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .map(|act| act.due_ic)
        .expect("a drive in flight");
    let (events, queued) = fire(&mut rig);
    assert!(ic_now(&rig.tw) < arrives, "still under way");
    land_all(&mut rig, &queued);
    let at_start = statuses(&events);
    one(&at_start, &stored_now(&rig), "the drive's first fire");
    assert_eq!(at_start[0].1["rig"]["act"]["kind"], "drive");
    assert!(
        !at_start[0].1["rig"]["leg"].is_null(),
        "the leg is projected"
    );
    // Under way, nothing changes: nothing pushed.
    let (events, queued) = fire(&mut rig);
    assert!(ic_now(&rig.tw) < arrives, "still under way");
    land_all(&mut rig, &queued);
    assert!(
        statuses(&events).is_empty(),
        "a fire under way writes nothing and pushes nothing"
    );
    // Line mode prints nothing of it.
    let event = OutputEvent::Panel {
        feed: SmolStr::new(TV_STATUS_FEED),
        kind: mush_proto::PanelFrameKind::Keyframe,
        data: data.clone(),
    };
    let mut bytes = Vec::new();
    mush_ansi::render(
        &event,
        &mush_proto::ClientCaps {
            kind: mush_proto::ClientKind::Ssh,
            color: mush_proto::ColorDepth::Ansi256,
            width: 120,
            height: 40,
            utf8: true,
            structured: false,
        },
        &mush_ansi::AnsiTheme::default(),
        &mut bytes,
    );
    assert!(bytes.is_empty(), "{}", String::from_utf8_lossy(&bytes));
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// Q8. Measured
// ---------------------------------------------------------------------------

/// **Q8 — the ground frame is measured** (the brief's 3.12; informational, no time assertion):
/// the Ceres frame for a viewer whose Ahuna row holds a scan's fields and a sounding, five
/// samples, the least and the median printed.
#[test]
fn sm9_the_ground_frame_is_measured() {
    let mut rig = stocked();
    let (a, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let all = picks(&mut rig.tw, AHUNA);
    let ore = ore_pick(&all);
    launched(&mut rig, a, 1, tv);
    let _ = act_go(&mut rig, a, "+tv/1 scan go");
    let _ = finish(&mut rig, tv, a);
    let _ = sounded_at(&mut rig, a, 1, tv, &ore);
    let mut samples: Vec<f64> = (0..5)
        .map(|_| {
            let started = std::time::Instant::now();
            let frame = ground(&mut rig, a, json!({ "body": "ceres" }));
            let ms = started.elapsed().as_secs_f64() * 1_000.0;
            assert_eq!(frame["width"], 56);
            ms
        })
        .collect();
    samples.sort_by(f64::total_cmp);
    eprintln!(
        "sm9: the Ceres ground frame (one scan's fields and a sounding at Ahuna): least {:.2} ms, \
         median {:.2} ms ({} build)",
        samples.first().copied().unwrap_or(0.0),
        samples.get(2).copied().unwrap_or(0.0),
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
}

// ---------------------------------------------------------------------------
// Q9 and Q10. A lease's log, and a reading that outlives the hull's stay
// ---------------------------------------------------------------------------

/// A rig's projection log as `(at_ic, text)` pairs.
fn logged(tw: &TestWorld, tv: ObjId) -> Vec<(i64, String)> {
    projection(tw, tv)
        .map(|rec| {
            rec.log
                .iter()
                .map(|line| (line.at_ic, line.text.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// What A did with a counter rig, for the tests that hand it on: the pilot, the hull, the rig,
/// the pocket A sounded, every line A heard, and every log line A's projection held — before the
/// recover (the excursion's launch, scan and sounding) and after it.
struct Worked {
    a: ObjId,
    tv: ObjId,
    ore: Pick,
    heard: String,
    logged: Vec<(i64, String)>,
}

/// A works Ahuna with one rig — a launch, a scan, a quiet sounding of an ore pocket — recovers it
/// and lifts to the Freeport.
fn worked_and_home(rig: &mut Rig) -> Worked {
    let (a, row, tvs) = on_pad(rig, 1);
    let tv = tvs[0];
    let all = picks(&mut rig.tw, AHUNA);
    let ore = ore_pick(&all);
    let _ = act_go(rig, a, "+tv/1 launch go");
    let mut heard = finish(rig, tv, a);
    let _ = act_go(rig, a, "+tv/1 scan go");
    heard.push_str(&finish(rig, tv, a));
    heard.push_str(&sounded_at(rig, a, 1, tv, &ore));
    let mut logs = logged(&rig.tw, tv);
    let _ = act_go(rig, a, "+tv/1 recover go");
    heard.push_str(&finish(rig, tv, a));
    for line in logged(&rig.tw, tv) {
        if !logs.contains(&line) {
            logs.push(line);
        }
    }
    let t = file_and_launch(rig, a, ObjId(row.hull), "+ship/undock ceres");
    let _ = to_ic(rig, t.plan.depart_ic + t.plan.total_s);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), ObjId(row.hull)),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == "ceres_port"
        ),
        "A's hull is back at the Freeport"
    );
    Worked {
        a,
        tv,
        ore,
        heard,
        logged: logs,
    }
}

/// **Q9 — a rig's log is its current operator's** (D16; reviewer F1): A works a counter rig and
/// returns it; B leases the same TV, takes it to Ahuna and scans. B's `site.map` log, every
/// `tv.status` keyframe B receives and the stored projection's log hold none of A's log lines —
/// the excursion's launch, scan and sounding as well as the recover's (compared as `(at_ic,
/// text)`, so B's own scan from the same ramp is not mistaken for A's) — no line older than B's
/// launch, and none of the sounding A heard; they hold B's scan line; A receives no `tv.status`
/// while B works.
#[test]
#[allow(clippy::too_many_lines)]
fn sm9_a_rigs_log_is_its_current_operators() {
    let mut rig = stocked();
    let Worked {
        a,
        tv,
        heard,
        logged: a_log,
        ..
    } = worked_and_home(&mut rig);
    assert!(
        a_log
            .iter()
            .any(|(_, text)| text.starts_with("Rig 1 sounded ")),
        "A's sounding is in A's log: {a_log:?}"
    );
    // What A heard and logged of the sounding: A's knowledge of the ground, which B never had
    // (B only launches and scans, so no line of B's says "sounded").
    let a_sounding: Vec<String> = heard
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("Rig 1 sounded "))
        .map(str::to_owned)
        .chain(
            a_log
                .iter()
                .filter(|(_, text)| text.starts_with("Rig 1 sounded "))
                .map(|(_, text)| text.clone()),
        )
        .collect();
    assert!(!a_sounding.is_empty(), "A heard the sounding: {heard}");
    let left = run(&mut rig.tw, a, "+ship/leave");
    let counter = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, a, counter);
    let out = run(&mut rig.tw, a, "+tv/1 lease return go");
    assert!(!left.is_empty());
    assert!(rigs_of(&rig.tw, a).is_empty(), "A returned the rig: {out}");
    // B leases the same TV (the counter hands out the first free rig).
    let (b, b_row) = prospector(&mut rig, "Bryn", 2, "Tally Moth");
    lease_one(&mut rig, b);
    let b_tvs = rigs_of(&rig.tw, b);
    assert_eq!(b_tvs, vec![tv], "B leases the TV A returned");
    load(&mut rig, &b_row, 20, 20, 9_300);
    board(&mut rig, b);
    let t = file_and_launch(&mut rig, b, ObjId(b_row.hull), "+ship/undock ahuna");
    let _ = to_ic(&mut rig, t.plan.depart_ic + t.plan.total_s);
    warm(&mut rig, AHUNA);
    let mut events = Vec::new();
    let _ = act_go(&mut rig, b, "+tv/1 launch go");
    events.extend(finish_events(&mut rig, tv));
    let b_launch = record_of(&mut rig.tw, tv)
        .and_then(|record| record.excursion)
        .map(|excursion| excursion.id.launch_ic)
        .expect("B's excursion");
    let _ = act_go(&mut rig, b, "+tv/1 scan go");
    events.extend(finish_events(&mut rig, tv));
    // Every log B can see: none of A's entries, nothing older than B's launch, nothing of A's
    // sounding.
    let none_of_a = |log: &[(i64, String)], surface: &str| {
        for entry in log {
            assert!(!a_log.contains(entry), "A's {entry:?} on {surface}");
            assert!(
                entry.0 >= b_launch,
                "{entry:?} predates B's launch on {surface}"
            );
            assert!(
                !a_sounding.contains(&entry.1),
                "A's sounding {entry:?} on {surface}"
            );
        }
    };
    let entries = |log: &Value| -> Vec<(i64, String)> {
        log.as_array()
            .expect("a log")
            .iter()
            .map(|line| {
                (
                    line["at_ic"].as_i64().expect("at_ic"),
                    line["text"].as_str().expect("text").to_owned(),
                )
            })
            .collect()
    };
    // Doc 20 cut SM.12b: the house's drones push their own panels to their dispatcher (a dark
    // Thing, never connected), whose work at Occator Rim shares these fires; every other push is
    // B's.
    let house = tacsim::port::yard::dispatchers(rig.tw.world());
    let pushed: Vec<(Audience, Value)> = statuses(&events)
        .into_iter()
        .filter(|(audience, _)| !matches!(audience, Audience::Actor(who) if house.contains(who)))
        .collect();
    assert!(!pushed.is_empty(), "B's work pushes");
    for (audience, data) in &pushed {
        assert_eq!(*audience, Audience::Actor(b), "never A");
        none_of_a(&entries(&data["rig"]["log"]), "a tv.status keyframe");
    }
    let stored = logged(&rig.tw, tv);
    none_of_a(&stored, "the stored projection");
    assert!(
        stored
            .iter()
            .any(|(_, line)| line.starts_with("Rig 1 scanned ")),
        "{stored:?}"
    );
    let frame = map_frame(&mut rig, b, json!({ "view": "drive", "rig": 1 }));
    let log = entries(&frame["site"]["log"]);
    none_of_a(&log, "the site map");
    assert!(
        log.iter()
            .any(|(_, line)| line.starts_with("Rig 1 scanned "))
    );
    no_refused_writes(&rig.tw);
}

/// **Q10 — the best reading survives a reboot with no hull there** (D14; reviewer F3): A scans
/// Ahuna's fields and sounds an ore pocket, recovers and lifts to the Freeport; after a keyed
/// reboot, with nothing at Ahuna, A's `+chart/ground ceres` still prints `GROUND_BEST` naming a
/// **pocket** A sounded — never a field, though A's row holds scanned fields too (pockets rank
/// before fields in the real keyed context), at least as rich as the ore pocket A sounded, and
/// the very class the keyed ground holds there — and never a dark or "still being mapped" line.
#[test]
#[allow(clippy::too_many_lines)]
fn sm9_the_best_reading_survives_a_reboot_with_no_hull_there() {
    let mut rig = stocked();
    let Worked { a, ore, .. } = worked_and_home(&mut rig);
    let all = picks(&mut rig.tw, AHUNA);
    let before = printed(&mut rig, a, "+chart/ground ceres");
    let mut rig = reboot(rig, &[a]);
    let after = printed(&mut rig, a, "+chart/ground ceres");
    let best = after
        .iter()
        .find(|line| line.starts_with("    Your best reading: "))
        .expect("a best reading");
    // "{addr} {word} {family label}": a pocket address (a block letter then a number, never a
    // field's "C-NW"), a class word, a family.
    let reading = best
        .trim_start_matches("    Your best reading: ")
        .trim_end_matches('.');
    let (addr, rest) = reading.split_once(' ').expect("an address");
    let (word, label) = rest.split_once(' ').expect("a word and a family");
    assert!(
        addr.len() >= 2
            && addr.starts_with(|c: char| c.is_ascii_uppercase())
            && addr[1..].chars().all(|c| c.is_ascii_digit()),
        "a pocket, not a field: {best} (A sounded {})",
        ore.addr
    );
    let family = [
        "metals",
        "volatiles",
        "carbon feedstock",
        "strategic elements",
    ]
    .iter()
    .position(|l| *l == label)
    .expect("a family label");
    let rank = |w: Word| match w {
        Word::Barren => 0,
        Word::Lean => 1,
        Word::Fair => 2,
        Word::Rich => 3,
    };
    let named = match word {
        "lean" => Word::Lean,
        "fair" => Word::Fair,
        "rich" => Word::Rich,
        other => panic!("a class word: {other}"),
    };
    // The keyed ground holds that class there (best_reading reads the real classes) …
    let pocket = all
        .iter()
        .find(|p| p.addr == addr)
        .unwrap_or_else(|| panic!("{addr} is a pocket of Ahuna"));
    assert_eq!(pocket.classes[family], named, "{best}");
    // … and it is no poorer than the ore pocket A sounded (the richest sounded pocket wins).
    let ore_best = ore.classes.iter().copied().map(rank).max().unwrap_or(0);
    assert!(
        rank(named) >= ore_best,
        "{best} against {} {:?}",
        ore.addr,
        ore.classes
    );
    // Back at the Freeport, the charter's passages are planned at its load.
    assert!(
        after
            .iter()
            .any(|line| line.contains(" at this load, burn ")),
        "{after:?}"
    );
    assert!(
        after
            .iter()
            .all(|line| !line.contains("still being mapped") && line != GROUND_BEST_DARK),
        "{after:?}"
    );
    assert_eq!(
        before
            .iter()
            .filter(|l| l.starts_with("    Your "))
            .collect::<Vec<_>>(),
        after
            .iter()
            .filter(|l| l.starts_with("    Your "))
            .collect::<Vec<_>>(),
        "the same readings before and after"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// Q11. A rig with an unload queued
// ---------------------------------------------------------------------------

/// **Q11 — a rig with an unload queued reads so, never idle** (doc 20 cut SM.8's queue, its lead
/// note for this cut: "a rig with a queue is in the bay, unload queued, never idle"; the
/// record-backed rig states the screens show): A's rig comes home with ore in its hopper and the
/// recover queues the unload. Before the fire that starts it, the frame's rig row is in the bay,
/// with no act and no run, and its `queued` is `+tv` status's own queue line (`QUEUED_LINE`,
/// trimmed), read from the record — and no `site.lines` line gains it (`+tv/map` unchanged).
/// Once the tick starts the unload, the queue is gone and the act in flight is the unload.
#[test]
fn sm9_a_rig_with_an_unload_queued_reads_queued_never_idle() {
    let mut rig = stocked();
    let (a, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    launched(&mut rig, a, 1, tv);
    let hopper = tacsim::port::vehicle::hopper_of(rig.tw.world(), tv).expect("a hopper vault");
    seed_pool(&mut rig, hopper, "metal_ore", 4, LotId(9_901));
    // The recover, its finish landed, stopping before the fire that tells it.
    let _ = act_go(&mut rig, a, "+tv/1 recover go");
    let due = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .map(|act| act.due_ic)
        .expect("the recover in flight");
    let (_, queued) = advance_to(&mut rig, due);
    assert_eq!(finishes(&rig.tw, &queued).len(), 1, "the recover's finish");
    land_all(&mut rig, &queued);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert!(
        record.act.is_none() && record.queued.is_some(),
        "{record:?}"
    );
    let frame = map_frame(&mut rig, a, json!({ "view": "drive", "rig": 1 }));
    let row = &frame["site"]["rigs"][0];
    assert_eq!(row["slot"], 1);
    assert_eq!(row["queued"], QUEUED_LINE.trim());
    assert_eq!(row["act"], Value::Null);
    assert_eq!(row["run"], Value::Null);
    assert_eq!(row["cell"], Value::Null);
    let line = row["line"].as_str().expect("the rig's line");
    assert!(line.starts_with("Rig 1: in the bay, "), "{line}");
    let lines: Vec<&str> = frame["site"]["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        lines.contains(&line),
        "the rig's line is the one `+tv/map` prints"
    );
    assert!(
        lines.iter().all(|l| !l.contains(QUEUED_LINE.trim())),
        "{lines:?}"
    );
    // The next fire tells the recover and starts the queued unload as the operator.
    let (_, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert!(record.queued.is_none(), "the queue is spent: {record:?}");
    let frame = map_frame(&mut rig, a, json!({ "view": "drive", "rig": 1 }));
    let row = &frame["site"]["rigs"][0];
    assert_eq!(row["queued"], Value::Null);
    assert_eq!(row["act"]["kind"], "unload", "{row}");
    no_refused_writes(&rig.tw);
}
