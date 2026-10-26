//! **Doc 20 cut SM.12b acceptance — Occator's drones** (doc 20 §12.3 "Occator's drones", §9.6,
//! §8 "Leaving it running", §13's SM.12 row: "NPC excursions never exceed their caps nor sell into
//! player intakes"; the SM.12b brief's acceptance A1–A4 and its tests T0–T16).
//!
//! Lane v15 lays up Occator's yard at Occator Rim — the Dray-P "Occator Yard" on the pad, two
//! rigs of Occator's pool in its bay, a `DARK` dispatcher in the host room. The site clerk seeds
//! the house's standing licence and surveys its held blocks into the dispatcher's chart; the
//! dispatcher then delivers the yard hold's ice ore in kind to Occator's storefront for SM.12a's
//! mill, or files SM.11 fill plans for its idle rigs through the player's own compiler, re-armed
//! by its own rule.
//!
//! Composed on the composition root (`plans.rs`'s rig, copied: the per-file precedent), **booted
//! keyed** with the tacsim suites' fixed geology key (`KEY_1`); every act goes through the real
//! offer, the real land path and the real ticks; readiness is read through `@tacsim/port status`
//! only (never `support::with_ctx`, whose lens is detached). The keyed figures each test reads are
//! printed (`--nocapture`) for the report.

mod support;

#[path = "support/geology_keyed.rs"]
mod geology_keyed;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use mush_core::{
    AccountId, AccountIdentity, ActionStateBootRecord, Audience, AuthoritativeSubmit, CanonicalId,
    CommitToken, GOD, IdempotencyKey, IdempotencyKeyText, IdempotencyScope, ModuleWrite, ObjId,
    OfferExecutionContext, OfferSubmission, RequestUid, ScopedIdempotencyKey, World,
};
use mush_economy::{EconomyConfig, VaultId};
use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
use mush_org::PrincipalId;
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde_json::{Value, json};
use smol_str::SmolStr;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK, LANE_VERSION};
use tacsim::pack::{PackEntry, RulesPack, load_pack};
use tacsim::port::yard::{
    DISPATCH_ACTION, DispatchKey, DispatchLedger, HOUSE_RIG_LINE, YARD_STAYS,
};
use tacsim::port::{HullRow, PortHandles, PortMap, PortMapState, TicketId, YardRow, room_key};
use tacsim::surface::geology::{OreFamily, Word};
use tacsim::surface::ground::{SiteGeologySpec, SiteGround, site_ground_sync};
use tacsim::surface::plan::{FillPlan, FillState, PlanEnd, PlanStep, PlanTally};
use tacsim::surface::reading::Footing;
use tacsim::surface::{
    ChartKey, ChartProvenance, ChartRecord, GeologyBoot, GeologyState, RigKey, RigRecord,
    SURFACE_ACT_ACTION, Via,
};
use tacsim::{TacsimConfig, TacsimStore};

use geology_keyed::{KEY_1, secret};

/// The ramp's cell.
const RAMP_CELL: (i64, i64) = (128, 124);
/// Occator Rim's port entry.
const OCCATOR: &str = "occator_port";

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

// ---------------------------------------------------------------------------
// The rig (plans.rs's, booted keyed)
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
    /// The harness's idempotency ledger (`plans.rs`'s): every submission identity that landed.
    ledger: BTreeSet<String>,
    /// Actors whose submissions [`fire`] and `advance_to` drop (never land): a clerk held while a
    /// pilot flies in, its acts retried by its own rule on a later fire.
    held: Vec<ObjId>,
}

/// Whether a heard line names a rig: the word itself, never the "rig" inside `HEARD_HERE`'s
/// "right under you".
fn names_a_rig(line: &str) -> bool {
    line.split(|c: char| !c.is_ascii_alphabetic())
        .any(|word| word.eq_ignore_ascii_case("rig"))
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

/// Compose over `world`, hand the key in, then run the boot pass over `store`.
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
        ledger: BTreeSet::new(),
        held: Vec::new(),
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
        held,
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
        held,
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
    let mut bytes = [0x77_u8; 16];
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

/// A submission's ledger identity (`ActionCoordinator::command_ingress`'s fields).
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

/// A submission resolved against the world as it stands, not yet prepared.
struct Resolved {
    action: mush_core::DefinitionRef,
    resolved: mush_core::ResolvedActionInvocation,
    token: [u8; 16],
}

/// The landing's first half: the offer projected and resolved under a fresh request token.
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

/// The landing's second half: the prepare, the bundle installed, its typed post-images kept;
/// the outcome's JSON and the bundle's staged writes.
fn prepare(
    rig: &mut Rig,
    resolved: &Resolved,
) -> Result<(Value, Vec<mush_core::ModuleOp>), String> {
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
    keep(rig, ops.clone());
    Ok((outcome, ops))
}

/// Drive one queued submission the way the world task does — the ledger first, then resolve
/// and prepare; the outcome and the bundle's writes.
fn land_full(
    rig: &mut Rig,
    submit: &AuthoritativeSubmit,
) -> Result<(Value, Vec<mush_core::ModuleOp>), String> {
    let identity = ledger_identity(submit);
    if rig.ledger.contains(&identity) {
        return Err(HIT.to_owned());
    }
    let resolved = resolve(rig, submit)?;
    let landed = prepare(rig, &resolved)?;
    rig.ledger.insert(identity);
    Ok(landed)
}

fn land(rig: &mut Rig, submit: &AuthoritativeSubmit) -> Result<(), String> {
    land_full(rig, submit).map(|_| ())
}

/// Every queued submission of the port's lanes and industry's, dropping other modules'.
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
                    "{} by #{} did not land: {error} ({})",
                    submit.action.key().id,
                    submit.actor.0,
                    submit.parameters
                )
            });
    }
}

fn land_quietly(rig: &mut Rig, queued: &[AuthoritativeSubmit]) {
    for submit in queued {
        let _ = land(rig, submit);
    }
}

/// One fire (6 s on), its submissions drained (not landed); a held actor's dropped.
fn fire(rig: &mut Rig) -> (Vec<(Audience, OutputEvent)>, Vec<AuthoritativeSubmit>) {
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1_000));
    let mut queued = drain(&mut rig.tw);
    queued.retain(|submit| !rig.held.contains(&submit.actor));
    (events, queued)
}

/// One fire, everything it queued landed; what it queued.
fn settle(rig: &mut Rig) -> Vec<AuthoritativeSubmit> {
    let (_, queued) = fire(rig);
    land_all(rig, &queued);
    queued
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

fn handles(tw: &TestWorld) -> PortHandles {
    tw.world()
        .module_state::<PortHandles>()
        .cloned()
        .expect("port handles")
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

/// Debit `qty` of `good` from `vault`'s pool (staff's hand, the inventory's own path).
fn debit_pool(rig: &mut Rig, vault: VaultId, good: &str, qty: u64) {
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
            .expect("inventory transaction");
        families
            .debit(&mut txn, EndpointRef(vault), &cid(good), qty)
            .expect("the debit stages");
        txn.prepare().expect("prepares")
    };
    let (ops, install) = prepared.into_parts();
    engine.install_prepared_action(install).expect("installs");
    keep(rig, ops);
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

/// A rig's typed record.
fn record_of(tw: &mut TestWorld, tv: ObjId) -> Option<RigRecord> {
    let h = handles(tw);
    typed(tw, &h.families.excursions, &RigKey(tv))
}

/// Write `record` into the family through a typed transaction (staff's hand; `plans.rs`'s).
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

/// The yard's delivery ledger.
fn ledger_of(tw: &mut TestWorld) -> Option<DispatchLedger> {
    let h = handles(tw);
    typed(
        tw,
        &h.families.dispatch,
        &DispatchKey { site: cid(OCCATOR) },
    )
}

// ---------------------------------------------------------------------------
// Occator Rim, its yard and its ground
// ---------------------------------------------------------------------------

/// Occator Rim's yard row.
fn yard(tw: &TestWorld) -> YardRow {
    port_map(tw, OCCATOR)
        .site
        .and_then(|site| site.yard)
        .expect("Occator Rim's yard is laid up")
}

/// The yard's dispatcher.
fn dispatcher(tw: &TestWorld) -> ObjId {
    ObjId(yard(tw).dispatcher.expect("a dispatcher"))
}

/// The yard hull.
fn yard_hull(tw: &TestWorld) -> ObjId {
    ObjId(yard(tw).hulls.first().expect("the yard hull").hull)
}

/// The yard's rigs, slot order.
fn yard_rigs(tw: &TestWorld) -> Vec<ObjId> {
    let mut rigs = yard(tw).rigs;
    rigs.sort_by_key(|rig| rig.n);
    rigs.into_iter().map(|rig| ObjId(rig.tv)).collect()
}

/// The yard hold's vault.
fn yard_hold(tw: &TestWorld) -> VaultId {
    VaultId(yard(tw).hold.expect("the yard hold"))
}

/// Occator's Ceres storefront vault.
fn storefront(tw: &TestWorld) -> VaultId {
    VaultId(
        port_map(tw, "ceres_port")
            .vendors
            .get(&cid("occator"))
            .expect("Occator's vendor row")
            .vault,
    )
}

/// The house, `Organization(occator)`.
fn house(tw: &TestWorld) -> PrincipalId {
    tacsim::port::yard::operator_principal(tw.world(), dispatcher(tw))
}

/// Occator Rim's site clerk.
fn clerk(tw: &TestWorld) -> ObjId {
    ObjId(
        port_map(tw, OCCATOR)
            .site
            .and_then(|site| site.clerk)
            .expect("a site clerk"),
    )
}

/// Fire until `entry`'s ground is cached and its home field built.
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

/// A lane site's frame.
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

/// The dispatch line `@tacsim/port status` reads for Occator Rim.
fn dispatch_line(tw: &mut TestWorld) -> String {
    let out = run(tw, GOD, "@tacsim/port status");
    out.lines()
        .find(|line| line.contains("occator_port dispatch occator: "))
        .map(|line| line.trim().to_owned())
        .unwrap_or_else(|| panic!("no Dispatch line:\n{out}"))
}

/// Open Occator Rim (its clerk's opening acts landed; every other submission dropped), keeping
/// the typed post-images; the fires taken.
fn open_occator(rig: &mut Rig) -> u32 {
    let mut kept = Vec::new();
    let fires = support::sites::open_site_keeping(&mut rig.tw, OCCATOR, &mut |ops| kept.push(ops));
    for ops in kept {
        keep(rig, ops);
    }
    fires
}

/// The submissions of `queued` with action id `action` by `actor`.
fn of<'q>(
    queued: &'q [AuthoritativeSubmit],
    action: &str,
    actor: ObjId,
) -> Vec<&'q AuthoritativeSubmit> {
    queued
        .iter()
        .filter(|submit| submit.action.key().id.as_str() == action && submit.actor == actor)
        .collect()
}

/// The idempotency key text of a submission.
fn key_of(submit: &AuthoritativeSubmit) -> String {
    serde_json::to_value(&submit.key)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{:?}", submit.key))
}

/// **The seeds**: from an open Occator Rim, fire and land everything until the clerk has landed
/// the house's licence and four surveys; the fires taken.
fn seeded(rig: &mut Rig) -> u32 {
    for fires in 0..200_u32 {
        if yard(&rig.tw).licence.is_some()
            && ['F', 'G', 'J', 'K']
                .iter()
                .all(|letter| surveyed(&mut rig.tw, *letter))
        {
            return fires;
        }
        let _ = settle(rig);
    }
    panic!("the house was not seeded within 200 fires");
}

/// Whether the dispatcher's chart reads all four of `letter`'s fields.
fn surveyed(tw: &mut TestWorld, letter: char) -> bool {
    let d = dispatcher(tw);
    chart_of(tw, d, OCCATOR).is_some_and(|row| {
        block_field_indexes(letter)
            .iter()
            .all(|index| row.fields.contains_key(index))
    })
}

/// A block's four field indexes (`fy · 8 + fx`), south-west first.
fn block_field_indexes(letter: char) -> Vec<u8> {
    let (bx, by) = tacsim::surface::address::block_of_letter(letter).expect("a block letter");
    let mut out = Vec::new();
    for dy in 0..2 {
        for dx in 0..2 {
            out.push(
                tacsim::surface::address::field_index(bx * 2 + dx, by * 2 + dy)
                    .expect("a field index"),
            );
        }
    }
    out
}

// ---------------------------------------------------------------------------
// T0 — the keyed ground (O-9)
// ---------------------------------------------------------------------------

/// **T0 — `KEY_1` gives the drones ice** (doc 20 cut SM.12b; O-9, the lead's probe at `9df93246`):
/// under the suites' fixed key, Occator Rim's held blocks hold exactly these figures of
/// fair-or-better ice, and F + G (the yard's worked blocks) hold at least two hold-fills (172 t).
/// A pin of known values: if a figure moves, the ore-dependent tests are not trusted until the
/// lead re-reads the key — `yard.blocks`, `yard.ore`, the key and the charts are never changed to
/// get green.
#[test]
fn sm12b_key_1_gives_the_drones_ice() {
    let mut rig = stocked();
    let ground = ground_of(&mut rig.tw, OCCATOR);
    let v = 1; // Volatiles in the geology's family order (metals, volatiles, carbon, strategic).
    let mut figures = BTreeMap::new();
    for letter in ['F', 'G', 'J', 'K'] {
        let (mut fair_t, mut fair_n) = (0_u64, 0_u32);
        let mut by_word = [0_u32; 4];
        for py in 0..64_i64 {
            for px in 0..64_i64 {
                if tacsim::surface::address::block_letter(px, py) != Some(letter) {
                    continue;
                }
                let Some(row) = ground.geology.pocket(px, py) else {
                    continue;
                };
                let word = row.classes[v];
                let at = Word::ALL.iter().position(|w| *w == word).expect("a word");
                by_word[at] += 1;
                if word >= Word::Fair {
                    fair_t += u64::from(row.tonnes[v]);
                    fair_n += 1;
                }
            }
        }
        println!(
            "SM.12b T0 occator_port block {letter}: fair-or-better ice {fair_t} t in {fair_n} \
             pockets; barren/lean/fair/rich {by_word:?}"
        );
        figures.insert(letter, (fair_t, fair_n, by_word));
    }
    assert_eq!(figures[&'F'], (271, 22, [226, 8, 14, 8]));
    assert_eq!(figures[&'G'], (256, 20, [228, 8, 11, 9]));
    assert_eq!(figures[&'J'], (252, 23, [227, 6, 18, 5]));
    assert_eq!(figures[&'K'], (252, 19, [232, 5, 12, 7]));
    let fg = figures[&'F'].0 + figures[&'G'].0;
    println!("SM.12b T0 F+G fair-or-better ice {fg} t (two hold-fills: 172 t)");
    assert!(fg >= 172);
}

// ---------------------------------------------------------------------------
// T1 — the lay-up
// ---------------------------------------------------------------------------

/// **T1 — the yard is laid up and submits nothing** (doc 20 cut SM.12b; B4): after the boot no
/// authoritative submission is queued; the yard hull, its hold, its two rigs in its bay and its
/// dark dispatcher stand as the brief names them; the programme's eight hulls and the pool's eight
/// rigs are untouched; a re-apply creates nothing.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12b_the_yard_is_laid_up_and_submits_nothing() {
    let (world, _) = mush_testkit::boot_world();
    let mut tw = boot_over(world, HashMap::new());
    assert_eq!(
        tw.engine().queued_authoritative_actors().count(),
        0,
        "the applier and the boot pass submit nothing"
    );
    let w = tw.world();
    let row = yard(&tw);
    assert_eq!(row.hulls.len(), 1);
    let hull_row = &row.hulls[0];
    let hull = ObjId(hull_row.hull);
    assert_eq!(hull_row.mark, "OWP-YD-01");
    assert_eq!(
        w.get(hull).map(|o| o.name.clone()).as_deref(),
        Some("Occator Yard")
    );
    let view = tacsim::port::ship::hull_view(w, hull).expect("the yard's view");
    assert!(view.yard);
    assert_eq!(view.unit.as_str(), "tacsim.unit.dray_p");
    let (_, bay_room) = view.bay().expect("the yard's bay is laid up");
    assert_eq!(hull_row.rooms.get(&cid("bay")).copied(), Some(bay_room.0));
    let ceres = port_map(&tw, "ceres_port");
    assert_eq!(ceres.hulls.len(), 8, "the programme's eight hulls");
    assert!(
        ceres
            .hulls
            .iter()
            .all(|programme| !programme.rooms.values().any(|room| *room == bay_room.0)),
        "the yard's bay is on the yard's row and no programme row"
    );
    assert_eq!(
        ceres
            .vehicles
            .as_ref()
            .map(|v| v.rigs.iter().map(|r| r.mark.clone()).collect::<Vec<_>>()),
        Some((1..=8).map(|n| format!("OWP-TV-{n:02}")).collect())
    );
    // Placed Direct on Occator Rim's domain at the pad's arrival pose, titled to the house.
    let occator = port_map(&tw, OCCATOR);
    let (domain, pose) = mush_sim::pose_of(w, hull).expect("the yard is placed");
    assert_eq!(Some(domain.0), occator.domain);
    assert_eq!((pose.x_cm, pose.y_cm), (0, 0));
    let house = house(&tw);
    assert_eq!(
        house,
        PrincipalId::Organization(mush_org::OrgId(
            ceres.orgs.get(&cid("occator")).expect("Occator's row").org
        ))
    );
    let economy = w
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy");
    let asset = economy.snapshot().asset(hull).expect("the yard is titled");
    assert_eq!(asset.title_owner, house);
    assert!(matches!(
        tacsim::port::ship::berth_of(w, hull),
        tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == OCCATOR
    ));
    // The hold vault.
    let hold = economy
        .snapshot()
        .vault(yard_hold(&tw))
        .expect("the hold vault")
        .clone();
    assert_eq!(hold.owner, house);
    assert_eq!(
        hold.scope.map(|scope| scope.0),
        ceres.orgs.get(&cid("occator")).map(|row| row.root_scope)
    );
    assert_eq!(
        hold.backing,
        mush_economy::CustodyRef::SpatialInterior(view.stowage().expect("the stowage"))
    );
    assert_eq!(hold.status, mush_economy::VaultStatus::Active);
    // The rigs: in the bay, slots 1 and 2, the vehicles principal's hoppers, titled to the house.
    let rigs = yard_rigs(&tw);
    let marks: Vec<String> = row.rigs.iter().map(|rig| rig.mark.clone()).collect();
    assert_eq!(marks, vec!["OWP-YT-01".to_owned(), "OWP-YT-02".to_owned()]);
    let in_bay = tacsim::port::vehicle::rigs_in_bay(w, &view);
    assert_eq!(
        in_bay
            .iter()
            .map(|rig| (rig.slot, rig.tv, rig.mark.clone(), rig.lease))
            .collect::<Vec<_>>(),
        vec![
            (1, rigs[0], "OWP-YT-01".to_owned(), Some(0)),
            (2, rigs[1], "OWP-YT-02".to_owned(), Some(0)),
        ]
    );
    for tv in &rigs {
        let hopper = tacsim::port::vehicle::hopper_of(w, *tv).expect("a hopper");
        let vault = economy.snapshot().vault(hopper).expect("the hopper vault");
        assert_eq!(
            vault.owner,
            PrincipalId::System(
                mush_org::SystemPrincipalId::new(tacsim::port::site::VEHICLES_PRINCIPAL)
                    .expect("a principal")
            )
        );
        assert_eq!(
            economy.snapshot().asset(*tv).map(|a| a.title_owner.clone()),
            Some(house.clone())
        );
        let spatial = w
            .module_state::<mush_sim::data::SpatialState>()
            .expect("the simulation");
        assert!(matches!(
            spatial.parent_of(*tv),
            mush_sim::data::SpatialParent::Interior { space, .. } if space == bay_room
        ));
    }
    // The house views (the census `live_on_hull` appends): both rigs, slot order, never stored.
    let d = dispatcher(&tw);
    let census = tacsim::port::yard::house_rigs(w, d);
    assert_eq!(
        census
            .iter()
            .map(|lease| (
                lease.mark.clone(),
                lease.slot,
                lease.hull,
                lease.pilot,
                lease.is_house()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("OWP-YT-01".to_owned(), 1, hull, d, true),
            ("OWP-YT-02".to_owned(), 2, hull, d, true),
        ]
    );
    assert_eq!(
        tacsim::port::yard::rig_lease(w, rigs[1]).map(|lease| lease.slot),
        Some(2)
    );
    // The queries (B3 item 11) over the laid-up yard: the dispatcher's yard and hull, which hull
    // it operates and is aboard, the principal it operates for — and nobody else's.
    let programme_hull = ObjId(ceres.hulls[0].hull);
    assert!(tacsim::port::yard::yard_of_dispatcher(w, d).is_some());
    assert!(tacsim::port::yard::yard_of_dispatcher(w, hull).is_none());
    assert!(tacsim::port::yard::yard_of_dispatcher(w, rigs[0]).is_none());
    assert!(tacsim::port::yard::yard_of_hull(w, hull).is_some());
    assert!(tacsim::port::yard::yard_of_hull(w, programme_hull).is_none());
    assert_eq!(tacsim::port::yard::operating_hull(w, d), Some(hull));
    assert!(tacsim::port::yard::operates(w, d, hull));
    assert!(!tacsim::port::yard::operates(w, d, programme_hull));
    assert!(!tacsim::port::yard::operates(w, rigs[0], hull));
    assert!(tacsim::port::yard::is_dispatcher_of(w, hull, d));
    assert!(!tacsim::port::yard::is_dispatcher_of(w, programme_hull, d));
    assert!(!tacsim::port::yard::is_dispatcher_of(w, hull, rigs[0]));
    assert_eq!(
        tacsim::port::yard::operator_principal(w, rigs[0]),
        PrincipalId::Object(rigs[0]),
        "anyone but a dispatcher operates for itself"
    );
    assert_eq!(tacsim::port::yard::dispatchers(w), vec![d]);
    // The dispatcher: dark, in the host room, named for the yard.
    let thing = w.get(d).expect("the dispatcher");
    assert!(thing.flags.contains(mush_core::Flags::DARK));
    assert_eq!(thing.name, "Occator Yard dispatcher");
    assert_eq!(thing.location.map(|room| room.0), occator.host);
    // A re-apply creates nothing.
    let before = port_map(&tw, OCCATOR);
    let objects = tw.world().iter().count();
    let out = run(&mut tw, GOD, &format!("@atlas/pack plan {LANE_PACK}"));
    assert!(out.contains("Nothing to do."), "{out}");
    assert_eq!(port_map(&tw, OCCATOR), before);
    assert_eq!(tw.world().iter().count(), objects);
    no_refused_writes(&tw);
}

// ---------------------------------------------------------------------------
// The player's side (plans.rs's and claims.rs's helpers, copied)
// ---------------------------------------------------------------------------

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

/// Board from C01.
fn board(rig: &mut Rig, pilot: ObjId) {
    let gallery = station_room(&rig.tw, "ceres_port", "room.c01_berth_gallery");
    teleport(&mut rig.tw, pilot, gallery);
    let out = run(&mut rig.tw, pilot, "+ship/board");
    assert!(out.contains("You cycle the lock into the "), "{out}");
}

/// A command and the submissions it queued.
fn command(rig: &mut Rig, who: ObjId, line: &str) -> (String, Vec<AuthoritativeSubmit>) {
    let out = run(&mut rig.tw, who, line);
    let queued = drain(&mut rig.tw);
    (out, queued)
}

/// Sweep to the first real second whose IC instant is at or past `target`; the submissions
/// queued (a held actor's dropped), landed quietly.
fn to_ic(rig: &mut Rig, target: i64) {
    let rate = i64::from(world_rate(&rig.tw).max(1));
    let ahead = target.saturating_sub(ic_now(&rig.tw)).max(0);
    let real = ((ahead + rate - 1) / rate).max(1);
    let t = rig.tw.world().now().saturating_add(real);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let mut queued = drain(&mut rig.tw);
    queued.retain(|submit| !rig.held.contains(&submit.actor));
    land_quietly(rig, &queued);
}

/// File a passage with `line`, land its burn, launch it on the next fire; its arrival IC.
fn file_and_launch(rig: &mut Rig, pilot: ObjId, hull: ObjId, line: &str) -> i64 {
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
    let h = handles(&rig.tw);
    let ticket =
        typed(&mut rig.tw, &h.families.tickets, &TicketId { hull, seq }).expect("the ticket");
    let (_, queued) = fire(rig);
    land_quietly(rig, &queued);
    ticket.plan.depart_ic + ticket.plan.total_s
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

/// **A licensed pilot's Dray-P settled on Occator Rim's pad** (the site already open), `n_rigs`
/// rigs leased, the pilot aboard and connected; the site's ground and home warmed.
fn player_on_occator(
    rig: &mut Rig,
    (who, seed, name): (&str, i64, &str),
    n_rigs: usize,
) -> (ObjId, HullRow, Vec<ObjId>) {
    let (pilot, row) = prospector(rig, who, seed, name);
    for _ in 0..n_rigs {
        lease_one(rig, pilot);
    }
    let vault = hold_vault(&rig.tw, &row);
    let lot = 9_100 + u64::from(row.hull) * 10;
    seed_pool(rig, vault, "propellant", 20, LotId(lot));
    seed_pool(rig, vault, "provisions", 20, LotId(lot + 1));
    board(rig, pilot);
    let hull = ObjId(row.hull);
    let arrives = file_and_launch(rig, pilot, hull, "+ship/undock occator");
    to_ic(rig, arrives);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == OCCATOR
        ),
        "the hull is settled on Occator Rim's pad"
    );
    warm(rig, OCCATOR);
    let tvs = rigs_of(&rig.tw, pilot);
    assert_eq!(tvs.len(), n_rigs);
    (pilot, row, tvs)
}

/// **`line` as `who`, and exactly one start queued**, landed; the reply.
fn act_go(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let (out, queued) = command(rig, who, line);
    assert_eq!(queued.len(), 1, "{line}: one start: {out}");
    assert_eq!(queued[0].actor, who, "the start is the operator's");
    land_all(rig, &queued);
    out
}

/// **Finish `tv`'s act**: to its due instant (the fire there queues the finish, landed), then one
/// more fire; what `who` heard.
fn finish(rig: &mut Rig, tv: ObjId, who: ObjId) -> String {
    let due = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .map(|act| act.due_ic)
        .expect("an act in flight");
    to_ic(rig, due);
    let (events, queued) = fire(rig);
    land_all(rig, &queued);
    said(&rig.tw, &events, who)
}

/// Staff place `tv` `Direct` on `entry`'s domain at site cell `cell` (claims.rs's).
fn to_cell_on(rig: &mut Rig, entry: &str, tv: ObjId, cell: (i64, i64)) {
    let domain = mush_sim::DomainId(port_map(&rig.tw, entry).domain.expect("a domain"));
    let staff = support::place_keeping(&mut rig.tw, domain, tv, cell);
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    fold_module_ops_into(&mut rig.store, staff);
}

/// The soft stands of block `letter` of `entry` (claims.rs's), nearest the block's middle first,
/// each with its pocket.
fn stands_in(tw: &mut TestWorld, entry: &str, letter: char) -> Vec<((i64, i64), (i64, i64))> {
    let ground = ground_of(tw, entry);
    let surface = ground
        .composite
        .surface(&mush_atlas::SurfaceRef::Ground)
        .expect("a ground surface");
    let (bx, by) = tacsim::surface::address::block_of_letter(letter).expect("a letter");
    let mut out = Vec::new();
    for py in by * 16..by * 16 + 16 {
        for px in bx * 16..bx * 16 + 16 {
            let stand = (px * 4 + 2, py * 4 + 2);
            if tacsim::surface::chart::footing_at(surface, stand)
                .is_ok_and(|footing| footing != Footing::Rock)
            {
                out.push((stand, (px, py)));
            }
        }
    }
    let middle = (bx * 16 + 8, by * 16 + 8);
    out.sort_by_key(|(_, (px, py))| ((px - middle.0).abs().max((py - middle.1).abs()), *px, *py));
    out
}

/// The marker ring of `vault`.
fn marker_ring(tw: &mut TestWorld, vault: VaultId) -> Option<mush_inventory::record::MarkerRing> {
    let families = tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    let mut txn = tw
        .engine_mut()
        .action_transaction(&families.pools)
        .expect("a transaction");
    txn.get(&families.marker_ring, &EndpointRef(vault))
        .expect("the family reads")
        .cloned()
}

// ---------------------------------------------------------------------------
// The drones' own helpers
// ---------------------------------------------------------------------------

/// Stocked at world rate 24 (set before Occator Rim opens, so the house's seeded claims stand on
/// the clock generation the run reads), Occator Rim open.
fn yard_world() -> Rig {
    let mut rig = stocked();
    set_rate(&mut rig.tw, 24);
    assert_eq!(world_rate(&rig.tw), 24);
    let opened = open_occator(&mut rig);
    println!("SM.12b Occator Rim opened in {opened} fires");
    rig
}

/// Every account's posted and held amounts, and the economy journal's length.
fn balances(tw: &TestWorld) -> (BTreeMap<u64, (i128, i128)>, u64) {
    let economy = tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy");
    (
        economy
            .snapshot()
            .accounts()
            .map(|a| (a.id.0, (a.posted_balance.widen(), a.held_amount.widen())))
            .collect(),
        economy.journal_len(),
    )
}

/// **No money moved** between `before` and `after` ([`balances`]): every account's posted and
/// held amounts unchanged, any account opened since at zero, and no journal entry posted.
fn unmoved(
    before: &(BTreeMap<u64, (i128, i128)>, u64),
    after: &(BTreeMap<u64, (i128, i128)>, u64),
    what: &str,
) {
    assert_eq!(after.1, before.1, "{what}: no journal entry");
    for (id, amounts) in &after.0 {
        assert_eq!(
            Some(amounts),
            Some(before.0.get(id).unwrap_or(&(0, 0))),
            "{what}: account {id}"
        );
    }
    assert!(
        before.0.keys().all(|id| after.0.contains_key(id)),
        "{what}: no account closed"
    );
}

/// How many contracts the economy holds.
fn contract_count(tw: &TestWorld) -> usize {
    tw.world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy")
        .snapshot()
        .contracts()
        .count()
}

/// A submission's `act` parameter.
fn act_word(submit: &AuthoritativeSubmit) -> String {
    submit.parameters["act"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// Land each submission; the refusals (the ledger's HITs left out) as
/// `"{action} by #{actor} {act}: {error}"`.
fn land_each(rig: &mut Rig, queued: &[AuthoritativeSubmit]) -> Vec<String> {
    let mut refused = Vec::new();
    for submit in queued {
        match land(rig, submit) {
            Ok(()) => {}
            Err(error) if error == HIT => {}
            Err(error) => refused.push(format!(
                "{} by #{} {}: {error}",
                submit.action.key().id,
                submit.actor.0,
                act_word(submit)
            )),
        }
    }
    refused
}

/// A copy of `submit` under another idempotency key (a hand-built retry).
fn rekeyed(submit: &AuthoritativeSubmit, suffix: &str) -> AuthoritativeSubmit {
    let mut again = submit.clone();
    again.key =
        IdempotencyKeyText::parse_canonical(format!("{}/{suffix}", key_of(submit))).expect("a key");
    again
}

/// The total ice ore in a set of vaults.
fn ice(tw: &mut TestWorld, vaults: &[VaultId]) -> u64 {
    vaults.iter().map(|vault| pool(tw, *vault, "ice_ore")).sum()
}

/// The yard rigs' hopper vaults.
fn hoppers(tw: &TestWorld) -> Vec<VaultId> {
    yard_rigs(tw)
        .into_iter()
        .map(|tv| tacsim::port::vehicle::hopper_of(tw.world(), tv).expect("a hopper"))
        .collect()
}

/// Occator's ice-ore mill ledger.
fn mill_ledger(tw: &mut TestWorld) -> Option<tacsim::port::mill::MillLedger> {
    let id = tacsim::port::mill::MillId::derive(
        &cid(LANE_PACK),
        &cid("ceres_port"),
        &cid("occator"),
        &cid("ice_ore"),
    )
    .expect("a mill id");
    let h = handles(tw);
    typed(tw, &h.families.mills, &id)
}

/// Occator's `volatiles` ask's listed quantity.
fn volatiles_ask(tw: &TestWorld) -> u64 {
    let market = tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .expect("market");
    market
        .lot_listing_of(EndpointRef(storefront(tw)), &cid("volatiles"))
        .and_then(|id| market.listing(id))
        .map_or(0, |listing| listing.quantity)
}

/// Occator's `volatiles` units milled and not yet listed (the ledger less the counter).
fn volatiles_unlisted(tw: &mut TestWorld) -> u64 {
    let product = mill_ledger(tw).map_or(0, |ledger| ledger.product_t);
    let counter = port_map(tw, "ceres_port").vendors[&cid("occator")]
        .mill_listed
        .get(&cid("ice_ore"))
        .copied()
        .unwrap_or(0);
    product.saturating_sub(counter)
}

/// The ice ore in Occator Rim's blocks `letters`' vaults.
fn block_ice(tw: &mut TestWorld, letters: &[char]) -> u64 {
    let map = port_map(tw, OCCATOR);
    let vaults: Vec<VaultId> = letters
        .iter()
        .map(|letter| {
            VaultId(
                *map.site
                    .as_ref()
                    .and_then(|site| site.vaults.get(letter.to_string().as_str()))
                    .expect("a block vault"),
            )
        })
        .collect();
    ice(tw, &vaults)
}

/// **Fire until K's survey lands** (the clerk's acts and everyone's landed); the fire it landed
/// on is the last.
fn until_surveyed(rig: &mut Rig) {
    for _ in 0..200 {
        let (_, queued) = fire(rig);
        land_all(rig, &queued);
        if surveyed(&mut rig.tw, 'K') {
            return;
        }
    }
    panic!("K's survey never landed");
}

/// Fire until `cond`, landing everything; the fires taken.
fn until(rig: &mut Rig, max: u32, what: &str, cond: &dyn Fn(&mut Rig) -> bool) -> u32 {
    for n in 0..max {
        if cond(rig) {
            return n;
        }
        let (_, queued) = fire(rig);
        land_all(rig, &queued);
    }
    panic!("{what} within {max} fires");
}

/// A rig's plan.
fn plan_of(tw: &mut TestWorld, tv: ObjId) -> Option<FillPlan> {
    record_of(tw, tv).and_then(|record| record.plan)
}

/// The dispatcher's plan filings in `queued`.
fn filings(queued: &[AuthoritativeSubmit], d: ObjId) -> Vec<&AuthoritativeSubmit> {
    of(queued, SURFACE_ACT_ACTION, d)
        .into_iter()
        .filter(|submit| act_word(submit) == "plan_file")
        .collect()
}

/// The house's licence account.
fn licence_account(tw: &TestWorld) -> mush_economy::AccountId {
    mush_economy::AccountId(yard(tw).licence.expect("the house licence").account)
}

// ---------------------------------------------------------------------------
// T2, T2b — the clerk's seeds
// ---------------------------------------------------------------------------

/// **T2 — the clerk seeds the house's licence and surveys** (doc 20 cut SM.12b; B9; D16): after
/// SM.10's four held seeds, the next five clerk fires record the house's standing licence (no
/// money, no contract) and survey F, G, J and K one a fire, each survey one write (the
/// dispatcher's chart row); only then the papers. The dispatcher's row reads all sixteen fields at
/// R1 on rock, each with one plan stamp; no plan is filed before K's survey; a second survey of F
/// refuses.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12b_the_clerk_seeds_the_house_licence_and_surveys() {
    let mut rig = yard_world();
    let (c, d) = (clerk(&rig.tw), dispatcher(&rig.tw));
    let mut log: Vec<(u32, String)> = Vec::new();
    let mut surveys: Vec<AuthoritativeSubmit> = Vec::new();
    let mut k_landed = false;
    for n in 0..60_u32 {
        let had = yard(&rig.tw).licence.is_some();
        let (books, contracts) = (balances(&rig.tw), contract_count(&rig.tw));
        let (_, queued) = fire(&mut rig);
        assert!(
            k_landed || filings(&queued, d).is_empty(),
            "no plan is filed before K's survey (fire {n})"
        );
        for submit in &queued {
            if submit.actor == c {
                log.push((n, key_of(submit)));
            }
        }
        for submit in &queued {
            if submit.actor == c && submit.action.key().id.as_str() == DISPATCH_ACTION {
                let (outcome, ops) = land_full(&mut rig, submit).expect("the survey lands");
                assert_eq!(outcome["act"], "survey");
                assert_eq!(outcome["t"], 0);
                assert_eq!(
                    ops.len(),
                    1,
                    "a survey writes the dispatcher's chart row only"
                );
                surveys.push(submit.clone());
                if key_of(submit).ends_with("/k") {
                    k_landed = true;
                }
            } else {
                land_all(&mut rig, std::slice::from_ref(submit));
            }
        }
        if !had && yard(&rig.tw).licence.is_some() {
            log.push((n, "licence".to_owned()));
            unmoved(&books, &balances(&rig.tw), "the licence moves no money");
            assert_eq!(
                contract_count(&rig.tw),
                contracts,
                "the licence is no contract"
            );
        }
        if log
            .iter()
            .filter(|(_, key)| key.ends_with("/pallas_syndicate"))
            .count()
            >= 2
        {
            break;
        }
    }
    println!("SM.12b T2 clerk log: {log:?}");
    let keys: Vec<&str> = log.iter().map(|(_, key)| key.as_str()).collect();
    let seed = |l: &str| format!("tacsim.claim.seed/occator_port/{l}/freeport_registry");
    let survey = |l: &str| format!("tacsim.dispatch/occator_port/survey/{l}");
    let want: Vec<String> = ["f", "g", "j", "k"]
        .iter()
        .map(|l| seed(l))
        .chain(std::iter::once("licence".to_owned()))
        .chain(["f", "g", "j", "k"].iter().map(|l| survey(l)))
        .collect();
    assert_eq!(
        keys[..9].to_vec(),
        want.iter().map(String::as_str).collect::<Vec<_>>()
    );
    assert!(
        keys[9].ends_with("/pallas_syndicate"),
        "the papers follow K's survey: {keys:?}"
    );
    for pair in log[..10].windows(2) {
        assert_eq!(pair[0].0 + 1, pair[1].0, "one item a clerk fire: {log:?}");
    }
    // The licence: the house's, Active, in the licence instrument, a century.
    let licence = yard(&rig.tw).licence.expect("the house licence");
    assert_eq!(
        licence.until_ic,
        licence.from_ic + tacsim::surface::tenure::SEEDED_TERM_IC_SECS
    );
    let account = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy")
        .snapshot()
        .account(mush_economy::AccountId(licence.account))
        .expect("the account")
        .clone();
    assert_eq!(account.owner, house(&rig.tw));
    assert_eq!(account.status, mush_economy::AccountStatus::Active);
    assert_eq!(
        account.instrument.as_str(),
        tacsim::economy::LICENCE_FREEPORT
    );
    assert_eq!(
        account.name.as_str(),
        "Occator Water & Propellant standing prospecting licence"
    );
    assert!(matches!(
        tacsim::port::licence_standing(rig.tw.world(), d, ic_now(&rig.tw)),
        tacsim::port::LicenceStanding::InForce { .. }
    ));
    // The dispatcher's chart row: sixteen fields at R1 on rock, one plan stamp each.
    let row = chart_of(&mut rig.tw, d, OCCATOR).expect("the dispatcher's chart row");
    let fields: Vec<u8> = ['F', 'G', 'J', 'K']
        .iter()
        .flat_map(|letter| block_field_indexes(*letter))
        .collect();
    assert_eq!(row.fields.len(), 16);
    for index in &fields {
        let field = row.fields.get(index).expect("a surveyed field");
        assert_eq!(field.footing, Footing::Rock);
        assert_eq!(field.provenance.len(), 1);
        assert!(matches!(
            field.provenance[0],
            ChartProvenance::Local {
                instrument: tacsim::surface::Instrument::Scan,
                via: Via::Plan,
                ..
            }
        ));
        assert_eq!(field.first_hand_at, None);
    }
    assert_eq!(row.scan_stand, None);
    assert_eq!(
        chart_of(&mut rig.tw, c, OCCATOR),
        None,
        "the clerk keeps no chart"
    );
    // A second survey of F refuses, and writes nothing.
    let f = surveys
        .iter()
        .find(|submit| key_of(submit).ends_with("/f"))
        .expect("F's survey");
    let refused = land(&mut rig, &rekeyed(f, "again")).expect_err("a second survey of F");
    assert!(
        refused.contains("that block is already surveyed"),
        "{refused}"
    );
    no_refused_writes(&rig.tw);
}

/// **T2b — the papers never wait on the house** (doc 20 cut SM.12b; D26; the reviewer's F5):
/// every house survey is dropped (never landed) from the first on. F's survey is submitted on
/// exactly `MAX_SITE_ATTEMPTS` clerk fires; then the twelve papers land one a fire, no survey
/// submitted while a paper is due; then the clerk resubmits F's survey, and landed from then on
/// the four surveys land one a fire and the dispatcher files.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12b_the_papers_never_wait_on_the_house() {
    let mut rig = yard_world();
    let (c, d) = (clerk(&rig.tw), dispatcher(&rig.tw));
    let f_key = "tacsim.dispatch/occator_port/survey/f";
    let mut f_fires: Vec<u32> = Vec::new();
    let mut paper_fires: Vec<u32> = Vec::new();
    let mut landed_surveys: Vec<(u32, String)> = Vec::new();
    let mut filed_at: Option<u32> = None;
    for n in 0..120_u32 {
        let dropping = paper_fires.len() < 12;
        let (_, queued) = fire(&mut rig);
        let mut kept = Vec::new();
        for submit in queued {
            if submit.actor == c && submit.action.key().id.as_str() == DISPATCH_ACTION {
                let key = key_of(&submit);
                if key == f_key {
                    f_fires.push(n);
                }
                if dropping {
                    assert!(
                        !rig.tw
                            .engine()
                            .queued_authoritative_actors()
                            .any(|a| a == c),
                        "a drained, unlanded survey leaves nothing pending for the clerk"
                    );
                    continue;
                }
                landed_surveys.push((n, key));
            }
            if submit.actor == c && key_of(&submit).ends_with("/pallas_syndicate") {
                paper_fires.push(n);
            }
            if !filings(std::slice::from_ref(&submit), d).is_empty() && filed_at.is_none() {
                filed_at = Some(n);
            }
            kept.push(submit);
        }
        land_all(&mut rig, &kept);
        if filed_at.is_some() {
            break;
        }
    }
    println!(
        "SM.12b T2b F's survey at fires {f_fires:?}; papers at {paper_fires:?}; surveys landed \
         {landed_surveys:?}; first filing at {filed_at:?}"
    );
    let max = usize::from(tacsim::port::site::MAX_SITE_ATTEMPTS);
    assert_eq!(
        f_fires.len(),
        max + 1,
        "three dropped tries, then one after the papers"
    );
    for pair in f_fires[..max].windows(2) {
        assert_eq!(pair[0] + 1, pair[1], "tried on consecutive clerk fires");
    }
    assert_eq!(paper_fires.len(), 12);
    assert_eq!(
        paper_fires[0],
        f_fires[max - 1] + 1,
        "the papers go on at once"
    );
    for pair in paper_fires.windows(2) {
        assert_eq!(pair[0] + 1, pair[1], "one paper a clerk fire");
    }
    assert_eq!(
        f_fires[max],
        paper_fires[11] + 1,
        "F's survey again on the fire after the last paper"
    );
    let landed: Vec<&str> = landed_surveys.iter().map(|(_, key)| key.as_str()).collect();
    assert_eq!(
        landed,
        vec![
            "tacsim.dispatch/occator_port/survey/f",
            "tacsim.dispatch/occator_port/survey/g",
            "tacsim.dispatch/occator_port/survey/j",
            "tacsim.dispatch/occator_port/survey/k",
        ]
    );
    for pair in landed_surveys.windows(2) {
        assert_eq!(pair[0].0 + 1, pair[1].0, "one survey a fire");
    }
    assert!(filed_at.is_some(), "the dispatcher files once surveyed");
}

// ---------------------------------------------------------------------------
// T3 — the seeded readings are no evidence and no leak
// ---------------------------------------------------------------------------

/// **T3 — seeded readings never stake and never leak** (doc 20 cut SM.12b; B6; 12b-L1): the
/// dispatcher's survey is no stake evidence (`evidence_at` reads nothing for F); a licensed
/// player with a leased rig standing in F reads `NO_READING` there, byte-identically before and
/// after the survey — through the pocket's door, `+tv`'s ground line, `+chart/ground occator`
/// and the readings of the player's `site.map` frame.
#[test]
fn sm12b_seeded_readings_never_stake_and_never_leak() {
    let mut rig = yard_world();
    let c = clerk(&rig.tw);
    // The clerk held while the pilot flies in: its seeds, its licence and its surveys wait.
    rig.held = vec![c];
    let (pilot, _, tvs) = player_on_occator(&mut rig, ("Pip", 3, "Salt Moth"), 1);
    let tv = tvs[0];
    assert!(
        !surveyed(&mut rig.tw, 'F'),
        "nothing is surveyed while the clerk is held"
    );
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    let (stand, pocket) = stands_in(&mut rig.tw, OCCATOR, 'F')[0];
    to_cell_on(&mut rig, OCCATOR, tv, stand);
    let read = |rig: &mut Rig| {
        let row = chart_of(&mut rig.tw, pilot, OCCATOR);
        let door = tacsim::surface::chart::pocket_reading(
            rig.tw.world(),
            &cid(OCCATOR),
            row.as_ref(),
            pocket,
        );
        let status = run(&mut rig.tw, pilot, "+tv");
        let ground = status
            .lines()
            .find(|line| line.contains(tacsim::port::tv::NO_READINGS_HERE))
            .map(str::to_owned)
            .unwrap_or_else(|| panic!("no ground line: {status}"));
        let chart = run(&mut rig.tw, pilot, "+chart/ground occator");
        let frame = map_frame(rig, pilot, json!({ "view": "site" }));
        let readings = frame["site"]["readings"].clone();
        (door, ground, chart, readings)
    };
    let before = read(&mut rig);
    assert_eq!(before.0, tacsim::port::tv::NO_READING);
    assert_eq!(
        before.3,
        json!([]),
        "the player reads nothing on the site yet"
    );
    rig.held.clear();
    let fires = seeded(&mut rig);
    println!("SM.12b T3 seeded {fires} fires after the clerk was let go");
    let after = read(&mut rig);
    assert_eq!(after, before, "the survey leaks nothing to the player");
    let d = dispatcher(&rig.tw);
    let row = chart_of(&mut rig.tw, d, OCCATOR).expect("the dispatcher's row");
    assert_eq!(
        tacsim::surface::tenure::evidence_at(&row, 'F', Some(0)),
        None
    );
    assert!(tacsim::surface::tenure::heard_before(&row, 'F', None, i64::MAX).is_empty());
}

// ---------------------------------------------------------------------------
// T4 — the dispatcher files through the player's compiler
// ---------------------------------------------------------------------------

/// **T4 — the dispatcher files through the player's compiler** (doc 20 cut SM.12b; B10; D7): the
/// Dispatch line reads Pending while the house's items are owed, Ready "nothing filed yet" the
/// moment K's survey lands, and Ready with its figures once plan 1's filing lands; plan 1 is filed
/// on rig 1, then rig 2's on a later fire; each plan is the dispatcher's, `F,G` for ice, and its
/// steps are exactly what `+tv/2 plan F,G family=ice_ore` previews when typed as the dispatcher.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12b_the_dispatcher_files_through_the_players_compiler() {
    let mut rig = yard_world();
    let d = dispatcher(&rig.tw);
    let rigs = yard_rigs(&rig.tw);
    let head = "occator_port dispatch occator: ";
    assert_eq!(
        dispatch_line(&mut rig.tw),
        format!("{head}waiting on the Registry's seeds, licence and survey. [pending]")
    );
    until_surveyed(&mut rig);
    assert_eq!(
        dispatch_line(&mut rig.tw),
        format!("{head}2 rigs in the Occator Yard's bay; nothing filed yet. [ready]"),
        "Ready the moment K's survey lands"
    );
    // Plan 1 on rig 1.
    let mut filed: Vec<(u32, ObjId)> = Vec::new();
    for n in 0..30_u32 {
        let (_, queued) = fire(&mut rig);
        for submit in filings(&queued, d) {
            let tv = ObjId(
                submit.parameters["tv"]
                    .as_str()
                    .and_then(|tv| tv.parse().ok())
                    .expect("a tv"),
            );
            assert_eq!(
                key_of(submit),
                format!("tacsim.surface/{}/plan/1/file", tv.0),
                "plan 1's key"
            );
            filed.push((n, tv));
        }
        land_all(&mut rig, &queued);
        if filed.len() == 1 && filed[0].1 == rigs[0] && plan_of(&mut rig.tw, rigs[0]).is_some() {
            break;
        }
    }
    assert_eq!(
        filed.first().map(|(_, tv)| *tv),
        Some(rigs[0]),
        "rig 1 first"
    );
    let line = dispatch_line(&mut rig.tw);
    println!("SM.12b T4 after plan 1: {line}");
    assert_eq!(
        line,
        format!(
            "{head}0 of 2 out, 1 working a plan, 1 plan filed in all; 0 t of ice ore delivered \
             to its storefront today, 0 t in all. [ready]"
        )
    );
    let plan = plan_of(&mut rig.tw, rigs[0]).expect("plan 1");
    assert_eq!(plan.author, d);
    assert_eq!(plan.no, 1);
    assert_eq!(plan.family, OreFamily::Volatiles);
    assert_eq!(plan.list, vec!["F".to_owned(), "G".to_owned()]);
    assert_eq!(record_of(&mut rig.tw, rigs[0]).map(|r| r.plans), Some(1));
    // The same compile typed as the dispatcher, on rig 2 in the same bay, before its filing.
    let preview = run(&mut rig.tw, d, "+tv/2 plan F,G family=ice_ore");
    println!("SM.12b T4 the dispatcher's preview:\n{preview}");
    let lines: Vec<&str> = preview
        .lines()
        .filter(|line| line.starts_with("  ") && line.contains(". "))
        .collect();
    assert_eq!(lines.len(), plan.steps.len(), "{preview}");
    for (k, step) in plan.steps.iter().enumerate() {
        let want = format!(
            "  {}. {}",
            k + 1,
            tacsim::surface::plan::step_words(step, true)
        );
        assert!(lines[k].starts_with(&want), "{} vs {want}", lines[k]);
    }
    // Rig 2's plan on a later fire.
    let second_rig = rigs[1];
    until(&mut rig, 30, "rig 2's plan", &|rig| {
        plan_of(&mut rig.tw, second_rig).is_some()
    });
    let second = plan_of(&mut rig.tw, rigs[1]).expect("rig 2's plan");
    assert_eq!((second.no, second.author), (1, d));
    assert_eq!(
        second.steps, plan.steps,
        "the same compile from the same bay"
    );
}

// ---------------------------------------------------------------------------
// T5 — the drones mine, deliver and never sell
// ---------------------------------------------------------------------------

/// T5's span: **4,000 rig fires** of 6 real s — 24,000 real s, 576,000 IC s at world rate 24,
/// about 6.7 IC days. At rate 1 the same fires are 24,000 IC s, less than the 46,297 IC s one
/// player's plan took to fill one hold (`plans.rs` P1), so rate 1 could not hold one plan.
const T5_FIRES: u32 = 4_000;

/// **T5 — the drones mine, deliver and never sell** (doc 20 cut SM.12b; A1, A2, A3): at world rate
/// 24, over a fixed span of fires, with a chartered pilot settled on the pad to hear them. After
/// every fire: the caps over every rig record (the pool's and the yard's), the hoppers, the hold's
/// census within its limit in every measure and equal to the room the production reader gives the
/// dispatcher's `site.map` frame, the orphan rule's seam (no yard plan reads its author off the
/// helm, no queued unload orphaned), and — on a fire whose yard records are those of the fire
/// before — no tell, the projection carrying each plan's view and told pair. Over the run: the
/// soundings bounded where the code bounds them, the heard lines in SM.4a's form and measured
/// (never asserted), the deliveries in kind (each its storefront rise; no money, no contract by
/// any house act; their writes the two pools', the census and the ledger only), every ore bid
/// unmoved by every landing but the Ceres stock clerk's, no refusal, no notice for the dispatcher;
/// at the end, conservation through the mill and **the first run's figures pinned**.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12b_the_drones_mine_deliver_and_never_sell() {
    let mut rig = yard_world();
    let (pilot, _, _) = player_on_occator(&mut rig, ("Hark", 4, "Salt Moth"), 0);
    assert_eq!(world_rate(&rig.tw), 24);
    let d = dispatcher(&rig.tw);
    let rigs = yard_rigs(&rig.tw);
    let (hold, front) = (yard_hold(&rig.tw), storefront(&rig.tw));
    let hops = hoppers(&rig.tw);
    // Every rig record the caps' census reads: the pool's rigs and the yard's.
    let mut every_rig: Vec<ObjId> = port_map(&rig.tw, "ceres_port")
        .vehicles
        .expect("the pool")
        .rigs
        .iter()
        .map(|row| ObjId(row.tv))
        .collect();
    every_rig.extend(&rigs);
    let start_ic = ic_now(&rig.tw);
    let fg_start = block_ice(&mut rig.tw, &['F', 'G']);
    let front_start = pool(&mut rig.tw, front, "ice_ore");
    let held_start = ice(&mut rig.tw, &[hold]) + ice(&mut rig.tw, &hops);
    let (product_start, ore_start, batches_start) = mill_ledger(&mut rig.tw)
        .map_or((0, 0, 0), |ledger| {
            (ledger.product_t, ledger.ore_t, ledger.batches)
        });
    let ask_start = volatiles_ask(&rig.tw);
    let unlisted_start = volatiles_unlisted(&mut rig.tw);
    // Every ore bid, keyed (vendor, good).
    let ore_bids: Vec<((CanonicalId, CanonicalId), u64)> = port_map(&rig.tw, "ceres_port")
        .vendors
        .iter()
        .flat_map(|(vendor, row)| {
            row.bids
                .iter()
                .filter(|(good, _)| good.as_str().ends_with("_ore"))
                .map(|(good, bid)| ((vendor.clone(), good.clone()), bid.listing))
        })
        .collect();
    assert!(
        ore_bids
            .iter()
            .any(|((vendor, good), _)| vendor.as_str() == "occator" && good.as_str() == "ice_ore"),
        "Occator's ice-ore bid is among them"
    );
    let bids_of = |tw: &TestWorld| -> BidTerms {
        ore_bids
            .iter()
            .map(|(key, listing)| (key.clone(), listing_terms(tw, *listing)))
            .collect()
    };
    let clerk_ceres = ObjId(
        port_map(&rig.tw, "ceres_port")
            .clerk
            .as_ref()
            .expect("the Ceres clerk")
            .thing,
    );
    // A delivery's whole write set: the two vaults' ice-ore books and pools, the inventory's
    // census, and the yard's ledger — nothing else (mined ore is unrooted: no lineage).
    let ice_ore = cid("ice_ore");
    let book = |vault: VaultId| {
        mush_inventory::pool_key(EndpointRef(vault), &ice_ore)
            .expect("a pool key")
            .0
            .to_string()
    };
    let (hold_book, front_book) = (book(hold), book(front));
    let ledger_key = format!(
        "{}:action-state/v1/{}/{OCCATOR}",
        tacsim::NS,
        tacsim::port::yard::DISPATCH_FAMILY
    );
    let delivery_may_write = |key: &str| {
        key == ledger_key
            || key.starts_with("inventory:action-state/v1/inventory_census/")
            || [&hold_book, &front_book].iter().any(|hash| {
                key == format!("inventory:action-state/v1/inventory_books/{hash}")
                    || key == format!("inventory:action-state/v1/inventory_pools/{hash}")
            })
    };
    let mut delivered: u64 = 0;
    let mut deliveries: u32 = 0;
    let mut sounds: u32 = 0;
    let mut unloads: u32 = 0;
    let mut idle_returns: u32 = 0;
    let mut heard_total: u32 = 0;
    let mut heard_hours: BTreeMap<i64, u32> = BTreeMap::new();
    let mut refused: Vec<String> = Vec::new();
    let mut deliver_writes: Vec<(usize, Vec<String>)> = Vec::new();
    let mut hold_fills: u32 = 0;
    let mut ended_seen: BTreeSet<(ObjId, u32)> = BTreeSet::new();
    let mut pockets_mined: BTreeSet<String> = BTreeSet::new();
    let mut plans_seen: BTreeMap<(ObjId, u32), FillPlan> = BTreeMap::new();
    let mut markers: Vec<String> = Vec::new();
    let mut last_sound: Option<u32> = None;
    let mut all_read: Option<u32> = None;
    let mut plan_two_filed: Option<u32> = None;
    let mut seams: BTreeMap<&'static str, u32> = BTreeMap::new();
    let mut ring_peak: usize = 0;
    let (mut room_census, mut room_checks): (Option<u64>, u32) = (None, 0);
    let (mut quiet_fires, mut bid_landings, mut house_landings) = (0_u32, 0_u32, 0_u32);
    // The yard records each fire takes in, and the fire before's.
    let mut records_in: Vec<Option<RigRecord>> =
        rigs.iter().map(|tv| record_of(&mut rig.tw, *tv)).collect();
    let mut records_before: Option<Vec<Option<RigRecord>>> = None;
    for n in 0..T5_FIRES {
        let quiet = records_before.as_ref() == Some(&records_in);
        let (events, queued) = fire(&mut rig);
        // A yard rig's plan reads told after one fire and is never re-told on a second quiet fire
        // (B5 item 19: the projection's view is the lessee's, `yard::rig_lease`'s — else the tick
        // would compare a view with an always-empty projection and re-tell it every fire).
        if quiet {
            quiet_fires += 1;
            let stats =
                tacsim::surface::last_surface_fire(rig.tw.world()).expect("the surface tick fired");
            assert_eq!(
                stats.told, 0,
                "nothing is re-told on a quiet fire (fire {n})"
            );
            for (tv, record) in rigs.iter().zip(&records_in) {
                let Some(plan) = record.as_ref().and_then(|record| record.plan.as_ref()) else {
                    continue;
                };
                let rec = match store(&rig.tw).tv_rec(*tv) {
                    Some(tacsim::surface::TvRecState::Present(rec)) => rec.clone(),
                    other => panic!("rig {}'s projection (fire {n}): {other:?}", tv.0),
                };
                assert_eq!(
                    rec.plan_told,
                    Some(tacsim::surface::excursion::PlanTold {
                        no: plan.no,
                        seq: plan.seq
                    }),
                    "rig {}'s plan is told (fire {n})",
                    tv.0
                );
                assert_eq!(
                    rec.plan,
                    tacsim::surface::plan::plan_view_for(Some(plan), Some(d)),
                    "rig {}'s projection carries its plan's view (fire {n})",
                    tv.0
                );
            }
        }
        for line in said(&rig.tw, &events, pilot).lines() {
            if line.starts_with("A sounding thumps through the ground") {
                heard_total += 1;
                *heard_hours.entry(ic_now(&rig.tw) / 3_600).or_insert(0) += 1;
                assert!(!names_a_rig(line), "a heard line names no rig: {line}");
                for forbidden in ["OWP", "Occator", "ice", "#", "F1", "G1"] {
                    assert!(
                        !line.contains(forbidden),
                        "a heard line names nothing: {line}"
                    );
                }
            }
        }
        for submit in &queued {
            assert!(
                !(submit.action.key().owner.as_str() == "market"
                    && (submit.actor == d || rigs.contains(&submit.actor))),
                "the house never trades: {}",
                submit.action.key().id
            );
            if submit.actor == d && submit.parameters["by"] == "idle" {
                idle_returns += 1;
            }
            if submit.actor == d && submit.action.key().id.as_str() == SURFACE_ACT_ACTION {
                match act_word(submit).as_str() {
                    "sound" => {
                        assert_ne!(submit.parameters["quiet"], true, "the house sounds loud");
                        sounds += 1;
                        last_sound = Some(n);
                    }
                    "plan_file" if key_of(submit).ends_with("/plan/2/file") => {
                        plan_two_filed = plan_two_filed.or(Some(n));
                    }
                    "unload" => {
                        unloads += 1;
                        assert!(
                            key_of(submit).ends_with("/queued"),
                            "a plan's unload is SM.8's queued unload: {}",
                            key_of(submit)
                        );
                    }
                    _ => {}
                }
            }
        }
        for submit in &queued {
            let bids_before = (submit.actor != clerk_ceres).then(|| bids_of(&rig.tw));
            let house = submit.actor == d;
            let (journal, contracts) = (journal_len(&rig.tw), contract_count(&rig.tw));
            if house && submit.action.key().id.as_str() == DISPATCH_ACTION {
                let before = pool(&mut rig.tw, front, "ice_ore");
                match land_full(&mut rig, submit) {
                    Ok((outcome, ops)) => {
                        let t = outcome["t"].as_u64().expect("tonnes");
                        let after = pool(&mut rig.tw, front, "ice_ore");
                        assert_eq!(after - before, t, "a delivery's storefront rise is its t");
                        assert!(after <= 40, "the storefront holds at most a day of milling");
                        delivered += t;
                        deliveries += 1;
                        let keys: Vec<String> = ops
                            .iter()
                            .map(|op| format!("{}:{}", op.ns, op_key(op)))
                            .collect();
                        for key in &keys {
                            assert!(
                                delivery_may_write(key),
                                "a delivery writes the two ice-ore pools, the census and its \
                                 ledger only: {key} in {keys:?}"
                            );
                        }
                        deliver_writes.push((ops.len(), keys));
                    }
                    Err(error) if error == HIT => {}
                    Err(error) => refused.push(format!("deliver: {error}")),
                }
            } else {
                refused.extend(land_each(&mut rig, std::slice::from_ref(submit)));
            }
            if house {
                house_landings += 1;
                assert_eq!(journal_len(&rig.tw), journal, "a house act posts no money");
                assert_eq!(contract_count(&rig.tw), contracts, "nor a contract");
            }
            if let Some(bids_before) = bids_before {
                bid_landings += 1;
                assert_eq!(
                    bids_of(&rig.tw),
                    bids_before,
                    "an ore bid moves only by the Ceres stock clerk ({} by #{})",
                    submit.action.key().id,
                    submit.actor.0
                );
            }
        }
        // A1 after every fire: the caps over every rig record the census reads.
        let mut house_out: u32 = 0;
        for tv in &every_rig {
            if record_of(&mut rig.tw, *tv)
                .and_then(|record| record.excursion)
                .is_some_and(|excursion| excursion.operator == d)
            {
                house_out += 1;
            }
        }
        assert!(
            house_out <= tacsim::surface::MAX_EXCURSIONS_PER_OPERATOR.min(2),
            "the dispatcher's open excursions within its cap and its rigs (fire {n}): {house_out}"
        );
        let mut outstanding: u64 = 0;
        let mut records_out: Vec<Option<RigRecord>> = Vec::with_capacity(rigs.len());
        for tv in &rigs {
            let record = record_of(&mut rig.tw, *tv);
            if let Some(record) = &record {
                if let Some(excursion) = &record.excursion {
                    assert_eq!(excursion.operator, d);
                }
                // The orphan rule's one seam (B5 item 18): never the helm wait, never an
                // orphaned queue; the markers wait recorded, never a failure.
                match tacsim::surface::plan::plan_seam_for_tests(rig.tw.world(), record) {
                    Some(seam @ ("helm" | "orphan")) => {
                        panic!("rig {}'s plan reads {seam} (fire {n}): {record:?}", tv.0)
                    }
                    Some(seam) => *seams.entry(seam).or_insert(0) += 1,
                    None => {}
                }
                if let Some(run) = &record.run {
                    outstanding += run.outstanding_t();
                    pockets_mined.insert(
                        tacsim::surface::address::pocket_address(run.pocket[0], run.pocket[1])
                            .unwrap_or_default(),
                    );
                }
                if let Some(plan) = &record.plan {
                    plans_seen.insert((*tv, plan.no), plan.clone());
                    if let FillState::Ended { why, .. } = plan.state
                        && ended_seen.insert((*tv, plan.no))
                    {
                        if why == PlanEnd::HoldFull {
                            hold_fills += 1;
                        }
                        println!(
                            "SM.12b T5 fire {n}: rig {} plan {} ended {why:?} {:?}",
                            tv.0, plan.no, plan.tally
                        );
                    }
                }
            }
            records_out.push(record);
        }
        if all_read.is_none() {
            let row = chart_of(&mut rig.tw, d, OCCATOR);
            let side = tacsim::surface::geology::POCKETS_PER_FIELD_SIDE;
            let whole = row.is_some_and(|row| {
                ['F', 'G'].iter().all(|letter| {
                    let (bx, by) =
                        tacsim::surface::address::block_of_letter(*letter).expect("a letter");
                    (bx * 2 * side..(bx * 2 + 2) * side).all(|px| {
                        (by * 2 * side..(by * 2 + 2) * side).all(|py| row.pocket_sounded((px, py)))
                    })
                })
            });
            if whole {
                all_read = Some(n);
            }
        }
        for hopper in &hops {
            assert!(
                pool(&mut rig.tw, *hopper, "ice_ore") <= 12,
                "a hopper holds 12 t"
            );
            ring_peak =
                ring_peak.max(marker_ring(&mut rig.tw, *hopper).map_or(0, |ring| ring.len()));
        }
        // The hold's census within its limit in every measure — the Dray-P's ore hold, 86,000 kg
        // and 103,200 L (`dray_p.json`), ice ore 1,000 kg and 1,000 L a tonne — and the room the
        // production reader gives the dispatcher (`hold_unreserved` over `live_on_hull`'s census,
        // the yard's house views included, through its `site.map` frame) equal to it.
        let census = ice(&mut rig.tw, &[hold]) + ice(&mut rig.tw, &hops) + outstanding;
        let mass_room = 86_000_i64 - i64::try_from(census * 1_000).expect("fits");
        let volume_room = 103_200_i64 - i64::try_from(census * 1_000).expect("fits");
        assert!(
            mass_room >= 0 && volume_room >= 0,
            "the hold's census within its 86 t in every measure (fire {n}): {census} t"
        );
        // The frame is read whenever the census moves (the room is a function of it alone: one
        // ore, one hold) — every fire would only re-read an unchanged room at a frame's cost.
        if room_census != Some(census) {
            room_census = Some(census);
            room_checks += 1;
            let frame = map_frame(&mut rig, d, json!({ "view": "site" }));
            assert_eq!(
                frame["site"]["hold"]["room_t"].as_i64(),
                Some(mass_room / 1_000),
                "the dispatcher's hold room is the limit less the hold, both hoppers and every \
                 outstanding cycle (fire {n})"
            );
        }
        if n == T5_FIRES / 2 {
            for (tv, hopper) in rigs.iter().zip(&hops) {
                let ring = marker_ring(&mut rig.tw, *hopper).map_or(0, |ring| ring.len());
                markers.push(format!("rig {} hopper ring {ring}", tv.0));
            }
        }
        records_before = Some(std::mem::replace(&mut records_in, records_out));
    }
    let span = ic_now(&rig.tw) - start_ic;
    let heard_max = heard_hours.values().copied().max().unwrap_or(0);
    // O-15 (D29): measured and reported, never asserted.
    println!("SM.12b heard: total {heard_total}, max {heard_max} an IC hour");
    assert!(heard_total >= 1, "the pilot hears the house sound");
    println!(
        "SM.12b T5 span {T5_FIRES} fires, {span} IC s; {deliveries} deliveries, {delivered} t; \
         {hold_fills} hold-fills; {sounds} loud soundings; {unloads} queued unloads; {idle_returns} \
         idle returns; pockets mined {pockets_mined:?}; refused {refused:?}"
    );
    println!(
        "SM.12b T5 checks: {quiet_fires} quiet fires, {room_checks} hold-room reads, \
         {bid_landings} landings with every ore bid unmoved, {house_landings} house landings \
         with no money and no contract; seams {seams:?}; hopper rings peaked at {ring_peak} of {}",
        mush_inventory::record::MAX_MARKER_RING
    );
    assert!(quiet_fires > 0 && room_checks > 1 && bid_landings > 0 && house_landings > 0);
    for (tv, hopper) in rigs.iter().zip(&hops) {
        let ring = marker_ring(&mut rig.tw, *hopper).map_or(0, |ring| ring.len());
        markers.push(format!("rig {} hopper ring {ring} at the end", tv.0));
    }
    println!("SM.12b T5 markers: {markers:?}; unloads {unloads} over {span} IC s");
    if let Some((writes, keys)) = deliver_writes.first() {
        println!("SM.12b T5 first delivery writes {writes}: {keys:?}");
    }
    if let Some((writes, keys)) = deliver_writes.last() {
        println!("SM.12b T5 last delivery writes {writes}: {keys:?}");
    }
    assert!(
        refused.is_empty(),
        "every house submission lands: {refused:?}"
    );
    // The delivery bundle's writes (§11's Tick row: the overrun this cut adds, never trimmed):
    // into a fresh storefront pool 6 — both pools' scalars and books and the pool census, and the
    // ledger — else 5; mined ore is unrooted, so no lineage is carried.
    assert_eq!(deliver_writes.first().map(|(writes, _)| *writes), Some(6));
    assert!(
        deliver_writes
            .iter()
            .all(|(writes, _)| *writes == 5 || *writes == 6),
        "{deliver_writes:?}"
    );
    assert!(
        sounds <= 16,
        "each of F's and G's eight fields sounded at most once a plan"
    );
    // No loud sounding once the house's row reads every pocket of F's and G's eight fields at
    // R2 (`chart::sound_adds` is then false for each, so no compile emits a Sound step).
    let all = all_read.expect("the house reads F and G whole at R2 within the span");
    assert!(
        last_sound.is_some_and(|last| last < all),
        "no sounding once the house reads the blocks at R2: last sound {last_sound:?}, all read \
         by fire {all}"
    );
    println!(
        "SM.12b T5 soundings: {sounds}, the last at fire {last_sound:?}; F and G read whole at R2 \
         by fire {all_read:?}; plan 2 first filed at {plan_two_filed:?}"
    );
    assert!(
        plans_seen.keys().any(|(_, no)| *no >= 2),
        "the house re-arms after its first holds"
    );
    // Every reading the plans wrote is a plan's (or the survey's).
    let row = chart_of(&mut rig.tw, d, OCCATOR).expect("the dispatcher's row");
    assert!(
        row.fields
            .values()
            .flat_map(|field| field.provenance.iter())
            .chain(
                row.blocks
                    .values()
                    .flat_map(|block| block.provenance.iter())
            )
            .all(|entry| matches!(entry, ChartProvenance::Local { via: Via::Plan, .. }))
    );
    // No notice is queued for the dispatcher or its rigs (`queue_notice` is players' only; D12).
    let notices: Vec<String> = rig
        .tw
        .take_module_ops()
        .iter()
        .filter(|op| op.ns.as_str() == tacsim::NS)
        .map(op_key)
        .filter(|key| key.starts_with(tacsim::port_tick::NOTICE_PREFIX))
        .collect();
    for who in std::iter::once(d).chain(rigs.iter().copied()) {
        assert!(
            !notices.contains(&tacsim::port_tick::notice_key(who).to_string()),
            "no notice document for #{}: {notices:?}",
            who.0
        );
    }
    // A3: conservation end to end (the run sells no ore).
    let fg_end = block_ice(&mut rig.tw, &['F', 'G']);
    let held_end = ice(&mut rig.tw, &[hold]) + ice(&mut rig.tw, &hops);
    let ledger = mill_ledger(&mut rig.tw).expect("the mill milled");
    let front_end = pool(&mut rig.tw, front, "ice_ore");
    let ledger_d = ledger_of(&mut rig.tw).expect("the dispatch ledger");
    assert_eq!(ledger_d.delivered_t, delivered);
    assert_eq!(
        fg_start - fg_end,
        held_end - held_start + delivered,
        "every tonne mined is in the hold, a hopper or delivered"
    );
    assert_eq!(
        delivered,
        (ledger.ore_t - ore_start) + front_end - front_start,
        "every tonne delivered is milled or waits at the storefront"
    );
    let rise = volatiles_ask(&rig.tw) - ask_start;
    let unlisted = volatiles_unlisted(&mut rig.tw);
    assert_eq!(
        rise + unlisted - unlisted_start,
        ledger.product_t - product_start,
        "the volatiles ask rises by what the mill made"
    );
    let (milled_ore, milled_water, batches) = (
        ledger.ore_t - ore_start,
        ledger.product_t - product_start,
        ledger.batches - batches_start,
    );
    println!(
        "SM.12b T5 milled {milled_ore} t of ore into {milled_water} t of water ({batches} \
         batches); ask +{rise}"
    );
    // **The first run's figures** (the brief's T5: "pin the run's figures the first run shows"),
    // a pure function of `KEY_1`, the lane, world rate 24 and the 4,000-fire span: 22 deliveries
    // of 320 t, 12 hold-fills, 16 loud soundings, 38 queued unloads, 14 mill batches of 280 t of
    // ore into 112 t of water. A move here is a behaviour change to explain, never to re-pin
    // blindly.
    assert_eq!(
        (deliveries, delivered, hold_fills, sounds, unloads),
        (22, 320, 12, 16, 38),
        "the first run's deliveries, tonnes, hold-fills, soundings and unloads"
    );
    assert_eq!(
        (batches, milled_ore, milled_water),
        (14, 280, 112),
        "the first run's mill batches, ore and water"
    );
    no_refused_writes(&rig.tw);
}

/// Every ore bid's quantity and revision, keyed `(vendor, good)`.
type BidTerms = Vec<((CanonicalId, CanonicalId), Option<(u64, u64)>)>;

/// A listing's quantity and revision, if it stands.
fn listing_terms(tw: &TestWorld, listing: u64) -> Option<(u64, u64)> {
    tw.world()
        .module_state::<mush_market::MarketStore>()
        .expect("market")
        .listing(mush_market::ListingId(listing))
        .map(|listing| (listing.quantity, listing.revision))
}

/// **The `site.map` frame `who` gets** through the feed's own query (`quarry.rs`'s).
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

/// The economy journal's length.
fn journal_len(tw: &TestWorld) -> u64 {
    tw.world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy")
        .journal_len()
}

/// A module op's key.
fn op_key(op: &mush_core::ModuleOp) -> String {
    match &op.op {
        ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key } => key.to_string(),
        ModuleWrite::StreamAppend { .. } => "(stream)".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// T6, T6b — the full storefront and the stalled rig
// ---------------------------------------------------------------------------

/// **T6 — a full storefront parks the drones** (doc 20 cut SM.12b; O-10; 12b-T2; D21): Occator's
/// storefront seeded to two IC days of milling. No delivery lands while it holds a day or more;
/// the plans fill the hold and end `HoldFull` (each with a landed cycle and an empty hopper); no
/// plan is filed while the hold is full and nothing can be delivered — the dispatcher's compile
/// refuses and backs off, **at most one compile a rig a re-arm interval** (25 fires at rate 24),
/// never one a fire; once the mill draws the storefront below a day, one delivery lands and a
/// later plan 2 (`plans == 2`) lands a cycle.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12b_a_full_storefront_parks_the_drones() {
    let mut rig = yard_world();
    let d = dispatcher(&rig.tw);
    let rigs = yard_rigs(&rig.tw);
    let (hold, front) = (yard_hold(&rig.tw), storefront(&rig.tw));
    let hops = hoppers(&rig.tw);
    seed_pool(&mut rig, front, "ice_ore", 80, LotId(9_801));
    let bound = 40;
    let mut first_delivery: Option<u32> = None;
    let mut holds_full: BTreeSet<ObjId> = BTreeSet::new();
    let mut ended: BTreeSet<(ObjId, u32)> = BTreeSet::new();
    let mut plan_two: Option<u32> = None;
    let mut done = false;
    // The dispatcher's compiles from the first plan's `HoldFull` end to plan 2's filing (the
    // window the full hold and the full storefront park it in): `(fire, compiles)` at each end.
    let mut window_from: Option<(u32, u64)> = None;
    let mut window_to: Option<(u32, u64)> = None;
    for n in 0..4_000_u32 {
        let front_before = pool(&mut rig.tw, front, "ice_ore");
        let (_, queued) = fire(&mut rig);
        let compiles = tacsim::surface::plan::house_compiles_for_tests(rig.tw.world());
        if !of(&queued, DISPATCH_ACTION, d).is_empty() {
            assert!(
                front_before < bound,
                "no delivery while the storefront holds a day (fire {n}: {front_before} t)"
            );
            first_delivery = first_delivery.or(Some(n));
        }
        for submit in filings(&queued, d) {
            assert!(
                holds_full.len() < rigs.len() || first_delivery.is_some(),
                "nothing filed into a full hold before a delivery (fire {n})"
            );
            if key_of(submit).ends_with("/plan/2/file") {
                plan_two = plan_two.or(Some(n));
                window_to = window_to.or(Some((n, compiles)));
            }
        }
        land_all(&mut rig, &queued);
        for (tv, hopper) in rigs.iter().zip(&hops) {
            let Some(plan) = plan_of(&mut rig.tw, *tv) else {
                continue;
            };
            if let FillState::Ended { why, .. } = plan.state
                && ended.insert((*tv, plan.no))
            {
                println!(
                    "SM.12b T6 fire {n}: rig {} plan {} ended {why:?} {:?}",
                    tv.0, plan.no, plan.tally
                );
                if why == PlanEnd::HoldFull {
                    assert!(plan.tally.cycles >= 1, "a HoldFull plan landed a cycle");
                    assert_eq!(
                        pool(&mut rig.tw, *hopper, "ice_ore"),
                        0,
                        "its hopper is empty"
                    );
                    if plan.no == 1 {
                        holds_full.insert(*tv);
                        window_from = window_from.or(Some((n, compiles)));
                    }
                }
            }
            if plan.no == 2 {
                assert_eq!(
                    record_of(&mut rig.tw, *tv).map(|record| record.plans),
                    Some(2),
                    "plan 2 is the rig's second filing"
                );
            }
            if plan.no == 2 && plan.tally.cycles >= 1 {
                done = true;
            }
        }
        if done {
            println!(
                "SM.12b T6: hold full by fire {:?}, first delivery at {first_delivery:?}, plan 2 \
                 filed at {plan_two:?}, its first cycle by fire {n}; hold {} t, storefront {} t",
                holds_full,
                pool(&mut rig.tw, hold, "ice_ore"),
                pool(&mut rig.tw, front, "ice_ore")
            );
            break;
        }
    }
    assert!(done, "plan 2 landed a cycle after the storefront drained");
    let delivered = first_delivery.expect("a delivery");
    assert!(
        plan_two.is_some_and(|filed| filed > delivered),
        "plan 2 after the delivery"
    );
    // D21's back-off: over the parked window the dispatcher compiled at most once a rig an
    // interval (25 fires at rate 24) — never once a fire — and at least once more than the one
    // compile plan 2's filing took (the refused compile it backed off from).
    let ((from, at_from), (to, at_to)) = (
        window_from.expect("a plan ended HoldFull"),
        window_to.expect("plan 2 was filed"),
    );
    let (fires, compiles) = (to - from, at_to - at_from);
    let interval = 3_600 / (24 * 6);
    let bound = 2 * (u64::from(fires) / interval + 1);
    println!(
        "SM.12b T6 back-off: {compiles} dispatcher compiles over the {fires} parked fires \
         {from}..{to} (bound {bound})"
    );
    assert!(
        compiles >= 2,
        "the full hold refused at least one compile before plan 2's: {compiles}"
    );
    assert!(
        compiles <= bound,
        "at most one compile a rig an interval, never one a fire: {compiles} over {fires} fires"
    );
}

/// **T6b — a stalled rig is named and resumes after a hand unload** (doc 20 cut SM.12b; D27; the
/// reviewer's F4; a rule fixture): the yard hold staff-seeded to a tonne short of its 86 t and
/// rig 1's hopper to 4 t, the dispatcher's hand unload leaves ore in the hopper (the last unload's
/// stays): no plan is ever filed on rig 1 while it stays so, rig 2 still files once a delivery
/// makes room, and the Dispatch line reads order 5d; a second hand unload as the dispatcher empties
/// the hopper and rig 1's plan is filed on the dispatcher's next eligible fire.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12b_a_stalled_rig_is_named_and_resumes_after_a_hand_unload() {
    let mut rig = yard_world();
    let d = dispatcher(&rig.tw);
    let rigs = yard_rigs(&rig.tw);
    let (hold, hops) = (yard_hold(&rig.tw), hoppers(&rig.tw));
    seed_pool(&mut rig, hold, "ice_ore", 85, LotId(9_811));
    seed_pool(&mut rig, hops[0], "ice_ore", 4, LotId(9_812));
    let out = act_go(&mut rig, d, "+tv/1 unload go");
    println!("SM.12b T6b the dispatcher's first unload: {out}");
    let _ = finish(&mut rig, rigs[0], d);
    let left = pool(&mut rig.tw, hops[0], "ice_ore");
    let last = record_of(&mut rig.tw, rigs[0])
        .and_then(|record| record.last)
        .expect("the unload's outcome");
    println!("SM.12b T6b the unload left {left} t: {last:?}");
    assert!(left > 0, "the unload left ore in the hopper");
    assert!(
        !last.stays.is_empty() || matches!(last.result, tacsim::surface::ActResult::Refused { .. })
    );
    until_surveyed(&mut rig);
    let stalled = "occator_port dispatch occator: rig 1 (OWP-YT-01) holds ore its last unload left \
                   in the hopper. Staff unload it as the dispatcher (+tv/1 unload go) to start \
                   it again. [flagged]";
    assert_eq!(dispatch_line(&mut rig.tw), stalled);
    // Three re-arm intervals (25 fires each at rate 24): rig 1 never filed, rig 2 files.
    let mut rig_two_filed = false;
    for _ in 0..80 {
        let (_, queued) = fire(&mut rig);
        for submit in filings(&queued, d) {
            assert_ne!(
                submit.parameters["tv"],
                json!(rigs[0].0.to_string()),
                "never re-armed into a known dead end"
            );
            rig_two_filed = true;
        }
        land_all(&mut rig, &queued);
    }
    assert!(rig_two_filed, "rig 2 still files");
    assert_eq!(dispatch_line(&mut rig.tw), stalled);
    // Staff's remedy: a hand unload as the dispatcher, which now lands whole.
    let first = rigs[0];
    until(&mut rig, 200, "rig 1 idle in the bay", &|rig| {
        let hold_t = pool(&mut rig.tw, hold, "ice_ore");
        hold_t + 4 <= 86
            && record_of(&mut rig.tw, first)
                .is_some_and(|record| record.act.is_none() && record.queued.is_none())
    });
    let _ = act_go(&mut rig, d, "+tv/1 unload go");
    let _ = finish(&mut rig, rigs[0], d);
    assert_eq!(
        pool(&mut rig.tw, hops[0], "ice_ore"),
        0,
        "the hopper is empty"
    );
    let fires = until(&mut rig, 60, "rig 1's plan", &|rig| {
        plan_of(&mut rig.tw, first).is_some()
    });
    println!("SM.12b T6b rig 1 filed {fires} fires after its unload");
    assert_ne!(dispatch_line(&mut rig.tw), stalled, "the line leaves 5d");
}

// ---------------------------------------------------------------------------
// T7 — the re-arm rule
// ---------------------------------------------------------------------------

/// **T7 — the dispatcher re-arms only on its rule** (doc 20 cut SM.12b; O-14; D28; at world rate
/// 24; **a rule fixture**: a chart row holds coverage, never classes, so no chart edit can make
/// fair ice read lean — the fixture writes the rigs' records through a staff typed write).
/// (a) Both rigs' last plans ended `WorkedOut` with no cycle: nothing is filed over three re-arm
/// intervals and the line reads order 8; with a cycle landed but the house's row reading F and G
/// at R1 only, still nothing (the house re-arms only on ore it has read). (b) A strike re-arms; a
/// clear holds (order 5c); a second strike re-arms. (c) A frozen licence stops the house: its
/// plan parks `Licence` and ends, nothing is re-filed, and the line reads order 4.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12b_the_dispatcher_rearms_only_on_its_rule() {
    let mut rig = yard_world();
    let d = dispatcher(&rig.tw);
    let rigs = yard_rigs(&rig.tw);
    let hull = yard_hull(&rig.tw);
    until_surveyed(&mut rig);
    let now = ic_now(&rig.tw);
    let worked_out = |slot: u8, cycles: u16| FillPlan {
        no: 1,
        author: d,
        hull,
        slot,
        site: cid(OCCATOR),
        family: OreFamily::Volatiles,
        filed_ic: now - 10,
        deadline_ic: now - 10 + tacsim::surface::plan::PLAN_BOUND_IC_SECS,
        list: vec!["F".to_owned(), "G".to_owned()],
        steps: vec![PlanStep::Block { letter: 'F' }],
        cursor: 1,
        passed: Vec::new(),
        state: FillState::Ended {
            why: PlanEnd::WorkedOut,
            at_ic: now,
        },
        seq: 1,
        tally: PlanTally {
            tonnes: 0,
            cycles,
            trips: 0,
        },
        notes: Vec::new(),
    };
    for (k, tv) in rigs.iter().enumerate() {
        let mut record =
            record_of(&mut rig.tw, *tv).unwrap_or_else(|| RigRecord::fresh_unread(*tv));
        record.plan = Some(worked_out(u8::try_from(k + 1).expect("a slot"), 0));
        record.plans = 1;
        put_record(&mut rig, record);
    }
    let quiet = |rig: &mut Rig, what: &str| {
        for _ in 0..80 {
            let (_, queued) = fire(rig);
            assert!(filings(&queued, d).is_empty(), "nothing re-armed: {what}");
            land_all(rig, &queued);
        }
    };
    quiet(&mut rig, "worked out with no cycle");
    let worked_line = "occator_port dispatch occator: its blocks read worked out in ice ore; 2 rigs \
                       parked in the Occator Yard's bay, 0 t delivered in all. [ready]";
    assert_eq!(dispatch_line(&mut rig.tw), worked_line);
    let mut record = record_of(&mut rig.tw, rigs[0]).expect("rig 1's record");
    record.plan = Some(worked_out(1, 1));
    put_record(&mut rig, record);
    quiet(&mut rig, "worked out with a cycle and nothing read at R2");
    assert_eq!(dispatch_line(&mut rig.tw), worked_line);
    // (b) A strike re-arms.
    let (out, queued) = command(&mut rig, d, "+tv/1 plan clear");
    println!("SM.12b T7 strike: {out}");
    land_all(&mut rig, &queued);
    assert_eq!(plan_of(&mut rig.tw, rigs[0]), None, "struck");
    let first = rigs[0];
    until(&mut rig, 30, "plan 2 on rig 1", &|rig| {
        plan_of(&mut rig.tw, first).is_some_and(|plan| plan.no == 2)
    });
    assert_eq!(record_of(&mut rig.tw, rigs[0]).map(|r| r.plans), Some(2));
    // A clear holds.
    let (out, queued) = command(&mut rig, d, "+tv/1 plan clear");
    println!("SM.12b T7 clear: {out}");
    land_all(&mut rig, &queued);
    until(&mut rig, 60, "plan 2 cleared", &|rig| {
        plan_of(&mut rig.tw, first).is_some_and(|plan| {
            matches!(
                plan.state,
                FillState::Ended {
                    why: PlanEnd::Cleared,
                    ..
                }
            )
        })
    });
    quiet(&mut rig, "a cleared plan holds");
    assert_eq!(
        dispatch_line(&mut rig.tw),
        "occator_port dispatch occator: rig 1 (OWP-YT-01) is held by staff; strike its plan to \
         start it again. [flagged]"
    );
    let home = until(&mut rig, 900, "rig 1 back in the bay", &|rig| {
        record_of(&mut rig.tw, first).is_some_and(|record| {
            record.excursion.is_none() && record.act.is_none() && record.run.is_none()
        })
    });
    println!("SM.12b T7 rig 1 home {home} fires after the clear");
    let (out, queued) = command(&mut rig, d, "+tv/1 plan clear");
    println!("SM.12b T7 second strike: {out}");
    land_all(&mut rig, &queued);
    until(&mut rig, 60, "plan 3 on rig 1", &|rig| {
        plan_of(&mut rig.tw, first).is_some_and(|plan| plan.no == 3)
    });
    // (c) A frozen licence stops the house.
    let account = licence_account(&rig.tw);
    let ok = support::with_ctx(&mut rig.tw, |ctx| {
        mush_economy::set_account_status(ctx, account, mush_economy::AccountStatus::Frozen)
    });
    assert!(ok, "staff froze the house's licence account");
    until(&mut rig, 1_500, "plan 3 ends on the licence", &|rig| {
        plan_of(&mut rig.tw, first).is_some_and(|plan| {
            matches!(
                plan.state,
                FillState::Ended {
                    why: PlanEnd::Licence { .. },
                    ..
                }
            )
        })
    });
    quiet(&mut rig, "a frozen licence");
    assert_eq!(
        dispatch_line(&mut rig.tw),
        "occator_port dispatch occator: its licence account is not active. [flagged]"
    );
}

// ---------------------------------------------------------------------------
// T8 — the caps
// ---------------------------------------------------------------------------

/// **T8 — four excursions bind the dispatcher** (doc 20 cut SM.12b; A1; P8's seeding): four of
/// the pool's rigs seeded with open excursions on Occator Rim naming the dispatcher as operator.
/// A hand-built launch as the dispatcher refuses at the prepare on the operator's cap (the
/// prepare's census over every rig record, the yard's included); the dispatcher's plans are
/// filed but no launch starts, and a hand-built plan start refuses on the plan's wait. One seeded
/// excursion closed, exactly one yard rig launches and the other stays in the bay — three seeded
/// and one yard rig bind the cap of four only if the yard rigs are counted; a second closed, both
/// are out and the site counts them.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12b_four_excursions_bind_the_dispatcher() {
    assert_eq!(tacsim::surface::MAX_EXCURSIONS_PER_OPERATOR, 4);
    assert_eq!(tacsim::surface::MAX_EXCURSIONS_PER_SITE, 32);
    assert_eq!(tacsim::surface::MAX_EXCURSIONS_WORLD, 1_024);
    let mut rig = yard_world();
    let d = dispatcher(&rig.tw);
    let rigs = yard_rigs(&rig.tw);
    let hull = yard_hull(&rig.tw);
    let others: Vec<ObjId> = port_map(&rig.tw, "ceres_port")
        .vehicles
        .expect("the pool")
        .rigs
        .iter()
        .take(4)
        .map(|r| ObjId(r.tv))
        .collect();
    for other in &others {
        let mut record = RigRecord::fresh_unread(*other);
        record.excursion = Some(tacsim::surface::Excursion {
            id: tacsim::surface::ExcursionId {
                tv: *other,
                launch_ic: ic_now(&rig.tw),
            },
            operator: d,
            hull,
            slot: 1,
            site: cid(OCCATOR),
        });
        put_record(&mut rig, record);
    }
    // A hand-built launch as the dispatcher, before any plan is filed (a hand act beside a live
    // plan refuses on the plan first): the prepare's cap refuses it, naming the operator's cap.
    let first = rigs[0];
    let launch = AuthoritativeSubmit {
        actor: d,
        action: handles(&rig.tw).surface_act.expect("the start"),
        subject: mush_core::PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters: json!({
            "tv": first.0.to_string(),
            "act": "launch",
            "ordinal": record_of(&mut rig.tw, first).map_or(0, |r| r.ordinal) + 1,
        }),
        key: IdempotencyKeyText::parse_canonical(format!("sm12b.t8/{}/launch", first.0))
            .expect("a key"),
    };
    let refused = land(&mut rig, &launch).expect_err("a hand launch under the cap");
    println!("SM.12b T8 the hand-built launch: {refused}");
    assert!(
        refused.contains("you have as many rigs out as one pilot may run"),
        "the operator's cap refuses it at the prepare: {refused}"
    );
    assert!(
        record_of(&mut rig.tw, first).is_none_or(|record| record.excursion.is_none()),
        "nothing launched"
    );
    until_surveyed(&mut rig);
    let launches = |queued: &[AuthoritativeSubmit]| {
        of(queued, SURFACE_ACT_ACTION, d)
            .into_iter()
            .filter(|s| matches!(act_word(s).as_str(), "drive" | "launch"))
            .count()
    };
    for _ in 0..60 {
        let (_, queued) = fire(&mut rig);
        assert_eq!(launches(&queued), 0, "nothing launches under the cap");
        land_all(&mut rig, &queued);
    }
    assert!(
        plan_of(&mut rig.tw, first).is_some(),
        "the plan is filed (a filing is no excursion)"
    );
    let next = record_of(&mut rig.tw, first).map_or(0, |r| r.ordinal) + 1;
    let built = AuthoritativeSubmit {
        actor: d,
        action: handles(&rig.tw).surface_act.expect("the start"),
        subject: mush_core::PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters: json!({
            "tv": first.0.to_string(),
            "act": "drive",
            "ordinal": next,
            "by": "plan",
            "plan": 1,
            "step": 0,
        }),
        key: tacsim::surface::act::plan_step_key(first, next, 0, "drive").expect("a key"),
    };
    let refused = land(&mut rig, &built).expect_err("a plan start under the cap");
    println!("SM.12b T8 the hand-built start: {refused}");
    assert!(
        refused.contains("that rig's plan is waiting"),
        "the plan's start refuses on the plan's own wait: {refused}"
    );
    assert_eq!(
        record_of(&mut rig.tw, first).map_or(0, |r| r.ordinal) + 1,
        next
    );
    let site_open = |rig: &mut Rig| {
        let mut all = rigs.clone();
        all.extend(&others);
        all.iter()
            .filter(|tv| {
                record_of(&mut rig.tw, **tv).is_some_and(|record| {
                    record
                        .excursion
                        .is_some_and(|excursion| excursion.site.as_str() == OCCATOR)
                })
            })
            .count()
    };
    assert_eq!(site_open(&mut rig), 4);
    let close = |rig: &mut Rig, other: ObjId| {
        let mut freed = record_of(&mut rig.tw, other).expect("seeded");
        freed.excursion = None;
        put_record(rig, freed);
    };
    let out_now = |rig: &mut Rig| -> Vec<ObjId> {
        rigs.iter()
            .copied()
            .filter(|tv| {
                record_of(&mut rig.tw, *tv).is_some_and(|record| record.excursion.is_some())
            })
            .collect()
    };
    // One seeded excursion closes: one yard rig launches (three seeded and one yard rig are the
    // cap), and never a second beside it however long it waits.
    close(&mut rig, others[0]);
    until(&mut rig, 120, "one yard rig out", &|rig| {
        !out_now(rig).is_empty()
    });
    for _ in 0..60 {
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
        assert!(
            out_now(&mut rig).len() <= 1,
            "the cap holds the second yard rig: the yard's own excursion counts"
        );
    }
    let yard_out = out_now(&mut rig).len();
    assert_eq!(
        site_open(&mut rig),
        3 + yard_out,
        "three seeded and the yard's out"
    );
    // A second closes: both yard rigs are out.
    close(&mut rig, others[1]);
    let both = rigs.clone();
    until(&mut rig, 120, "both yard rigs out", &|rig| {
        both.iter()
            .all(|tv| record_of(&mut rig.tw, *tv).is_some_and(|record| record.excursion.is_some()))
    });
    assert_eq!(site_open(&mut rig), 4, "two seeded and the yard's two");
    for tv in &rigs {
        assert_eq!(
            record_of(&mut rig.tw, *tv)
                .and_then(|r| r.excursion)
                .map(|e| e.operator),
            Some(d)
        );
    }
}

// ---------------------------------------------------------------------------
// T9 — the restart
// ---------------------------------------------------------------------------

/// T9's script span, rig fires after the house is seeded.
const T9_FIRES: u32 = 800;
/// T9's reboot window, real seconds (`plans.rs`'s `reboot_point`'s shape).
const T9_WINDOW: i64 = 60;
/// **T9's reboot points** (fire indexes; doc 20 cut SM.12c, the lead's ruling, lead note item 7):
/// re-scouted at SM.12c-lane's base (`d842fdd0`; the first delivery at fire 125). The first is the
/// first quiet point after the first delivery (397, as SM.12b's scout found it at `1b5dd371`); the
/// second is scouted **with the first point's jump in place** — the arms pass the window at the
/// first point, so SM.12b's second point (422, read off a run that never jumped) is not quiet in
/// either arm (an act due in the window) — as the first quiet point inside a mining run more than
/// 20 fires after the first. If the world moves, the test fails naming the point and what is due:
/// re-scout by running `t9_arm(&[], false)` (the scout this constant replaced) for the first and
/// `t9_arm(&[first], false)` for the second, reading the quiet points each returns.
const T9_POINTS: [u32; 2] = [397, 441];

/// What T9 compares.
#[derive(Debug, PartialEq)]
struct Post {
    records: Vec<Option<RigRecord>>,
    projections: Vec<Option<tacsim::surface::TvRec>>,
    pools: Vec<u64>,
    dispatch: Option<DispatchLedger>,
    mill: Option<tacsim::port::mill::MillLedger>,
    chart: Option<ChartRecord>,
    cursors: Vec<Option<tacsim::surface::cursor::PocketCursor>>,
    mined: Vec<tacsim::surface::mined::MinedFact>,
    map: PortMap,
    ic: i64,
}

fn post(rig: &mut Rig) -> Post {
    let rigs = yard_rigs(&rig.tw);
    let d = dispatcher(&rig.tw);
    let mut vaults = vec![yard_hold(&rig.tw), storefront(&rig.tw)];
    vaults.extend(hoppers(&rig.tw));
    let mut pools: Vec<u64> = vaults
        .iter()
        .map(|vault| pool(&mut rig.tw, *vault, "ice_ore"))
        .collect();
    let front = storefront(&rig.tw);
    pools.push(pool(&mut rig.tw, front, "volatiles"));
    let h = handles(&rig.tw);
    let cursors = ['F', 'G']
        .iter()
        .map(|letter| {
            let id = mush_industry::derive_system_deposit_id(&tacsim::surface::opening::receipt(
                &cid(OCCATOR),
                *letter,
                OreFamily::Volatiles,
            ))
            .expect("a deposit id");
            typed(&mut rig.tw, &h.families.pockets, &id)
        })
        .collect();
    Post {
        records: rigs.iter().map(|tv| record_of(&mut rig.tw, *tv)).collect(),
        projections: rigs
            .iter()
            .map(|tv| match store(&rig.tw).tv_rec(*tv) {
                Some(tacsim::surface::TvRecState::Present(rec)) => Some(rec.clone()),
                _ => None,
            })
            .collect(),
        pools,
        dispatch: ledger_of(&mut rig.tw),
        mill: mill_ledger(&mut rig.tw),
        chart: chart_of(&mut rig.tw, d, OCCATOR),
        cursors,
        mined: match store(&rig.tw).mined(&cid(OCCATOR)) {
            Some(tacsim::surface::mined::MinedState::Present(ring)) => ring.entries.clone(),
            _ => Vec::new(),
        },
        map: port_map(&rig.tw, OCCATOR),
        ic: ic_now(&rig.tw),
    }
}

/// **Whether nothing the restart could perturb falls due within the reboot window**: no cached
/// plan wait; each yard rig either mid-act or mid-run, due past the window, or idle in the bay
/// with its plan ended and not re-armed within the window; nothing deliverable (the yard hold
/// empty, or the storefront holding a day); Occator's mill with nothing to mill (less than a batch
/// at the storefront, or the IC day's room spent) and nothing unlisted; no IC day boundary within
/// the window (the mill's day and the drip's). `Err` names what is due.
fn quiet_point(rig: &mut Rig) -> Result<(), &'static str> {
    let rate = i64::from(world_rate(&rig.tw));
    let now = ic_now(&rig.tw);
    let horizon = now + (T9_WINDOW + 30) * rate;
    for tv in yard_rigs(&rig.tw) {
        if tacsim::surface::tick::plan_wait_for_tests(rig.tw.world(), tv).is_some() {
            return Err("a cached wait");
        }
        let record = record_of(&mut rig.tw, tv).ok_or("no record")?;
        if record.queued.is_some() {
            return Err("a queued unload");
        }
        let due = record
            .act
            .as_ref()
            .map(|act| act.due_ic)
            .or(record.run.as_ref().map(|run| run.due_ic));
        match due {
            Some(due) if due > horizon => {}
            Some(_) => return Err("an act due in the window"),
            None => match record.plan.as_ref().map(|plan| plan.state) {
                Some(FillState::Ended { why, at_ic }) => {
                    let rearms = matches!(why, PlanEnd::HoldFull | PlanEnd::Bound);
                    if rearms && at_ic + 3_600 <= horizon {
                        return Err("a re-arm due in the window");
                    }
                }
                _ => return Err("an idle rig with a live plan or none"),
            },
        }
    }
    let front = storefront(&rig.tw);
    let hold = yard_hold(&rig.tw);
    let front_t = pool(&mut rig.tw, front, "ice_ore");
    if pool(&mut rig.tw, hold, "ice_ore") > 0 && front_t < 40 {
        return Err("a delivery due");
    }
    let day = now.div_euclid(86_400);
    let spent = mill_ledger(&mut rig.tw)
        .is_some_and(|ledger| ledger.day == day && ledger.day_ore_t + 20 > 40);
    if front_t >= 20 && !spent {
        return Err("a batch due");
    }
    if volatiles_unlisted(&mut rig.tw) > 0 {
        return Err("a restock due");
    }
    if day != horizon.div_euclid(86_400) {
        return Err("a day boundary");
    }
    Ok(())
}

/// One arm of T9's script: the seeded yard run for [`T9_FIRES`] fires; at each of `points` (fire
/// indexes) the window passes in one step — rebooting first when `rebooting`, then warming
/// Occator Rim; the post-image. With fewer than two points, the quiet points the run passed (the
/// scout, kept callable for [`T9_POINTS`]' re-scout recipe). **Doc 20 cut SM.12c**: at each
/// pinned point, before any reboot, both arms assert the point is still quiet ([`quiet_point`]),
/// that a delivery has landed by the first and that a yard rig is inside a mining run at the
/// second — loudly, naming the re-scout.
fn t9_arm(points: &[u32], rebooting: bool) -> (Post, Vec<(u32, bool)>, Vec<u32>) {
    let mut rig = yard_world();
    let _ = seeded(&mut rig);
    let mut quiet = Vec::new();
    let mut deliveries = Vec::new();
    let mut reasons: BTreeMap<&str, u32> = BTreeMap::new();
    for n in 0..T9_FIRES {
        if points.contains(&n) {
            if let Err(why) = quiet_point(&mut rig) {
                panic!("T9 point {n} is no longer quiet: {why}; re-scout (see T9_POINTS)");
            }
            if points.first() == Some(&n) && deliveries.is_empty() {
                panic!(
                    "T9 point {n} is no longer quiet: no delivery has landed before it; re-scout \
                     (see T9_POINTS)"
                );
            }
            let mining = yard_rigs(&rig.tw)
                .iter()
                .any(|tv| record_of(&mut rig.tw, *tv).is_some_and(|record| record.run.is_some()));
            if points.get(1) == Some(&n) && !mining {
                panic!(
                    "T9 point {n} is no longer quiet: no yard rig is inside a mining run; \
                     re-scout (see T9_POINTS)"
                );
            }
            let at = rig.tw.world().now() + T9_WINDOW;
            if rebooting {
                rig = reboot(rig, &[]);
                assert_eq!(world_rate(&rig.tw), 24);
            }
            // Staff read the dispatcher's map (both arms alike): on a cold site it asks for the
            // site's ground and its home, which the warm then builds.
            let d = dispatcher(&rig.tw);
            let _ = run(&mut rig.tw, d, "+tv/map site");
            warm(&mut rig, OCCATOR);
            assert!(rig.tw.world().now() < at, "the warm ran past the point");
            rig.tw.set_now(at);
            let _ = rig.tw.fire_due(at.saturating_mul(1_000));
            let queued = drain(&mut rig.tw);
            land_all(&mut rig, &queued);
            continue;
        }
        if points.len() < 2 {
            match quiet_point(&mut rig) {
                Ok(()) => {
                    let mining = yard_rigs(&rig.tw).iter().any(|tv| {
                        record_of(&mut rig.tw, *tv).is_some_and(|record| record.run.is_some())
                    });
                    quiet.push((n, mining));
                }
                Err(why) => *reasons.entry(why).or_insert(0_u32) += 1,
            }
        }
        let (_, queued) = fire(&mut rig);
        if !of(&queued, DISPATCH_ACTION, dispatcher(&rig.tw)).is_empty() {
            deliveries.push(n);
        }
        land_all(&mut rig, &queued);
    }
    if points.len() < 2 {
        println!(
            "SM.12b T9 scout: {} quiet points; the rest {reasons:?}",
            quiet.len()
        );
    }
    (post(&mut rig), quiet, deliveries)
}

/// **T9 — the restart changes nothing** (doc 20 cut SM.12b; A4; D13): two arms on one key and one
/// script; arm B reboots twice — once after the first delivery lands, once inside a mining run —
/// each at a point where nothing it could perturb falls due within the window (`reboot_point`'s
/// rule, sharpened for the port tick's mill and drip); both arms warm Occator Rim and stand at the
/// same instant after each. The post-images are equal: the yard rigs' records and projections,
/// the pools, the dispatch and mill ledgers, the dispatcher's chart row, the cursors, the mined
/// ring, the site map and the IC clock. **Doc 20 cut SM.12c** (the lead's ruling, lead note item
/// 7): the points are pinned ([`T9_POINTS`]) and the scout arm is dropped; each arm asserts each
/// point still quiet, after the first delivery and inside a mining run.
#[test]
fn sm12b_the_restart_changes_nothing() {
    println!(
        "SM.12b T9 reboot points {} and {} (pinned)",
        T9_POINTS[0], T9_POINTS[1]
    );
    let (a, _, _) = t9_arm(&T9_POINTS, false);
    let (b, _, _) = t9_arm(&T9_POINTS, true);
    assert_eq!(a, b, "the restart changes nothing");
    assert!(a.dispatch.is_some_and(|ledger| ledger.deliveries >= 1));
}

// ---------------------------------------------------------------------------
// T10, T11 — what players and staff read
// ---------------------------------------------------------------------------

/// **T10 — players see the drones as other rigs** (doc 20 cut SM.12b; §12.3): a licensed pilot's
/// Dray-P on the pad with a leased rig: the house's first loud sounding reaches the pilot as
/// SM.4a's bearing-and-band line, naming nothing; `+tv/1 map` lists a yard rig among "Other rigs:"
/// while it is out, and the player's `site.map` frame carries one `site.others` row for each
/// yard rig on the ground; standing in F the player's rig is refused mining and staking ("That
/// block is under claim.") and the contest (`CONTEST_HELD`); it drives home from F.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12b_players_see_the_drones_as_other_rigs() {
    let mut rig = yard_world();
    let (pilot, _, tvs) = player_on_occator(&mut rig, ("Wren", 5, "Salt Moth"), 1);
    let tv = tvs[0];
    let yard_rigs = yard_rigs(&rig.tw);
    let mut heard: Option<String> = None;
    for n in 0..800_u32 {
        let out = yard_rigs
            .iter()
            .any(|tv| record_of(&mut rig.tw, *tv).is_some_and(|record| record.excursion.is_some()));
        if out && heard.is_some() {
            println!("SM.12b T10 a yard rig out and a sounding heard by fire {n}");
            break;
        }
        let (events, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
        heard = heard.or_else(|| {
            said(&rig.tw, &events, pilot)
                .lines()
                .find(|line| line.starts_with("A sounding thumps through the ground"))
                .map(str::to_owned)
        });
    }
    let heard = heard.expect("the house's sounding is heard on the pad");
    println!("SM.12b T10 heard: {heard}");
    let bearings = [
        "north",
        "northeast",
        "east",
        "southeast",
        "south",
        "southwest",
        "west",
        "northwest",
    ];
    let prefix = "A sounding thumps through the ground from the ";
    assert!(
        heard == tacsim::port::tv::HEARD_HERE
            || heard.strip_prefix(prefix).is_some_and(|rest| {
                bearings
                    .iter()
                    .any(|bearing| rest.starts_with(&format!("{bearing}, ")))
                    && rest.ends_with('.')
            }),
        "SM.4a's bearing-and-band line: {heard}"
    );
    assert!(!names_a_rig(&heard), "a heard line names no rig: {heard}");
    for forbidden in ["OWP", "Occator", "ice", "#", "dispatcher"] {
        assert!(
            !heard.contains(forbidden),
            "a heard line names nothing: {heard}"
        );
    }
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    let map = run(&mut rig.tw, pilot, "+tv/1 map");
    assert!(map.contains("Other rigs: "), "{map}");
    let frame = map_frame(&mut rig, pilot, json!({ "view": "site" }));
    let others = frame["site"]["others"]
        .as_array()
        .expect("the others")
        .len();
    let occator_domain = port_map(&rig.tw, OCCATOR).domain.expect("a domain");
    let on_ground = yard_rigs
        .iter()
        .filter(|tv| {
            mush_sim::pose_of(rig.tw.world(), **tv)
                .is_some_and(|(domain, _)| domain.0 == occator_domain)
        })
        .count();
    println!("SM.12b T10 the frame's others: {others}; yard rigs on the ground: {on_ground}");
    assert!(on_ground >= 1, "a yard rig is out on the ground");
    assert_eq!(
        others, on_ground,
        "the frame's other rigs are the yard's on the ground"
    );
    assert!(
        frame["site"]["other_rigs"]
            .as_str()
            .is_some_and(|line| line.starts_with("Other rigs: ")),
        "{}",
        frame["site"]
    );
    let (stand, _) = stands_in(&mut rig.tw, OCCATOR, 'F')[0];
    to_cell_on(&mut rig, OCCATOR, tv, stand);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 mine");
    println!("SM.12b T10 mine in F: {out}");
    assert!(out.contains(tacsim::port::tv::MINE_CLAIMED), "{out}");
    assert!(queued.is_empty());
    let (out, queued) = command(&mut rig, pilot, "+claim/stake");
    assert_eq!(out.trim(), tacsim::port::tv::MINE_CLAIMED);
    assert!(queued.is_empty());
    let (out, queued) = command(&mut rig, pilot, "+claim/contest F");
    assert_eq!(
        out.trim(),
        tacsim::port::claims::CONTEST_HELD
            .replace("{block}", "F")
            .replace("{org}", "Occator Water & Propellant")
    );
    assert!(queued.is_empty());
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let _ = finish(&mut rig, tv, pilot);
}

/// **T11 — staff read the dispatcher as its operator** (doc 20 cut SM.12b; B5 item 21): as the
/// dispatcher, `+tv` lists both rigs — each `HOUSE_RIG_SLOT` filled, then `HOUSE_RIG_LINE` — with
/// SM.11's plan line under rig 1's; `+tv/1 plan show` reads plan 1; the dispatcher's `site.map`
/// frame (the viewer's operating hull, the yard) carries both rigs, rig 1's with its plan; a
/// chartered player's `+tv` lists none of them.
#[test]
fn sm12b_staff_read_the_dispatcher_as_its_operator() {
    let mut rig = yard_world();
    let d = dispatcher(&rig.tw);
    let first = yard_rigs(&rig.tw)[0];
    until_surveyed(&mut rig);
    until(&mut rig, 30, "plan 1", &|rig| {
        plan_of(&mut rig.tw, first).is_some()
    });
    let status = run(&mut rig.tw, d, "+tv");
    println!("SM.12b T11 +tv as the dispatcher:\n{status}");
    let lines: Vec<&str> = status.lines().collect();
    let house = HOUSE_RIG_LINE.replace("{org}", "Occator Water & Propellant");
    for (n, mark) in [(1, "OWP-YT-01"), (2, "OWP-YT-02")] {
        let at = lines
            .iter()
            .position(|line| line.starts_with(&format!("Rig {n}: {mark}, ")))
            .unwrap_or_else(|| panic!("rig {n}'s line: {status}"));
        assert!(
            lines[at].ends_with('.') && lines[at].contains("; hopper "),
            "{}",
            lines[at]
        );
        assert_eq!(lines[at + 1], house);
        if n == 1 {
            assert!(
                lines
                    .get(at + 2)
                    .is_some_and(|line| line.starts_with("  Plan 1: step ")),
                "SM.11's plan line under rig 1's: {status}"
            );
        }
    }
    let show = run(&mut rig.tw, d, "+tv/1 plan show");
    println!("SM.12b T11 +tv/1 plan show:\n{show}");
    assert!(show.contains("plan 1"), "{show}");
    let frame = map_frame(&mut rig, d, json!({ "view": "site" }));
    let rows = frame["site"]["rigs"]
        .as_array()
        .expect("the dispatcher's rigs");
    println!("SM.12b T11 the dispatcher's frame rigs: {rows:?}");
    assert_eq!(
        rows.iter()
            .map(|row| (row["slot"].as_u64(), row["mark"].as_str()))
            .collect::<Vec<_>>(),
        vec![(Some(1), Some("OWP-YT-01")), (Some(2), Some("OWP-YT-02"))]
    );
    assert!(
        rows[0]["plan"].is_object(),
        "rig 1's row carries its plan: {}",
        rows[0]
    );
    let p = pilot(&mut rig, "Ash", 6);
    let out = run(&mut rig.tw, p, "+coop/enrol Salt Moth");
    assert!(out.contains("now the Salt Moth"), "{out}");
    let player = run(&mut rig.tw, p, "+tv");
    assert!(!player.contains("OWP-YT-"), "{player}");
}

// ---------------------------------------------------------------------------
// T12 — never trades, never sails
// ---------------------------------------------------------------------------

/// **T12 — the dispatcher never trades or sails** (doc 20 cut SM.12b; D2; A2): every ship verb
/// answers `YARD_STAYS` filled, queues nothing and writes nothing (no legacy write either); the
/// market refuses through its own gates — a sale `NAME_SOURCE` and a quote `NOT_IN_PORT` from the
/// host room, each writing nothing; in the Exchange Hall a quote stands (that arm's one write: the
/// quote record) but its accept names no endpoint and writes nothing — and nothing the house or
/// anyone holds moves: every account's amounts, the journal, every listing's quantity and
/// revision, the yard hold and the storefront.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12b_the_dispatcher_never_trades_or_sails() {
    let mut rig = yard_world();
    let _ = seeded(&mut rig);
    let d = dispatcher(&rig.tw);
    let stays = YARD_STAYS
        .replace("{name}", "Occator Yard")
        .replace("{org}", "Occator Water & Propellant")
        .replace("{site}", "Occator Rim");
    let (hold, front) = (yard_hold(&rig.tw), storefront(&rig.tw));
    let books = balances(&rig.tw);
    let listings = all_listings(&rig.tw);
    let ore = (
        pool(&mut rig.tw, hold, "ice_ore"),
        pool(&mut rig.tw, front, "ice_ore"),
    );
    // A command's reply, its queued submissions and every write it made (legacy and typed).
    let typed_at = |rig: &Rig| rig.typed.len();
    let quiet = |rig: &mut Rig, line: &str| -> (String, Vec<AuthoritativeSubmit>, Vec<String>) {
        fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
        let (out, queued) = command(rig, d, line);
        let writes = rig
            .tw
            .take_module_ops()
            .iter()
            .map(|op| format!("{}:{}", op.ns, op_key(op)))
            .collect();
        (out, queued, writes)
    };
    for line in [
        "+ship/lift ceres",
        "+ship/undock ceres",
        "+ship",
        "+ship/hold",
        "+ship/orders",
        "+ship/order 1 depart ceres",
    ] {
        let (out, queued, writes) = quiet(&mut rig, line);
        assert_eq!(out.trim(), stays, "{line}");
        assert!(queued.is_empty(), "{line}");
        assert!(writes.is_empty(), "{line} writes nothing: {writes:?}");
    }
    // `+ship/board` asks for a berth gallery before it asks for a charter: from the host room it
    // answers that, and boards nothing.
    let (out, queued, writes) = quiet(&mut rig, "+ship/board");
    assert_eq!(out.trim(), tacsim::port::ship::BOARD_WHERE);
    assert!(queued.is_empty());
    assert!(writes.is_empty(), "+ship/board writes nothing: {writes:?}");
    let vendors = port_map(&rig.tw, "ceres_port").vendors;
    let bid = vendors[&cid("occator")].bids[&cid("ice_ore")].listing;
    let (out, queued, writes) = quiet(&mut rig, &format!("+market/sell {bid}=10"));
    assert_eq!(out.trim(), mush_market::NAME_SOURCE, "{out}");
    assert!(queued.is_empty());
    assert!(writes.is_empty(), "the sale writes nothing: {writes:?}");
    let ask = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .expect("market")
        .lot_listing_of(EndpointRef(front), &cid("volatiles"))
        .expect("the volatiles ask");
    let (out, queued, writes) = quiet(&mut rig, &format!("+market/quote {ask}=1"));
    assert_eq!(
        out.trim(),
        tacsim::port::NOT_IN_PORT.replace("{port}", "Ceres Freeport"),
        "{out}"
    );
    assert!(queued.is_empty());
    assert!(
        writes.is_empty(),
        "the refused quote writes nothing: {writes:?}"
    );
    // In port: the quote stands (its one write); its accept names no endpoint.
    let host = rig
        .tw
        .world()
        .get(d)
        .and_then(|o| o.location)
        .expect("the host room");
    let hall = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, d, hall);
    let typed_before = typed_at(&rig);
    let (quoted, _, quote_writes) = quiet(&mut rig, &format!("+market/quote {ask}=1"));
    assert!(quoted.contains("Quote #"), "{quoted}");
    println!("SM.12b T12 the in-port quote's writes: {quote_writes:?}");
    assert!(
        quote_writes.iter().all(|key| key.starts_with("market:")),
        "the quote's writes are the market's quote record: {quote_writes:?}"
    );
    let quote = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .expect("market")
        .quotes_of_buyer(&PrincipalId::Object(d))
        .map(|q| q.id)
        .max()
        .expect("the quote");
    let (out, queued, writes) = quiet(&mut rig, &format!("+market/accept {quote}"));
    assert!(
        out.contains("Name the inventory endpoint this is delivered into"),
        "{out}"
    );
    assert!(queued.is_empty());
    assert!(
        writes.is_empty(),
        "the refused accept writes nothing: {writes:?}"
    );
    assert_eq!(typed_at(&rig), typed_before, "no typed bundle landed");
    teleport(&mut rig.tw, d, host);
    unmoved(&books, &balances(&rig.tw), "the dispatcher's verbs");
    assert_eq!(all_listings(&rig.tw), listings, "no listing moved");
    assert_eq!(
        (
            pool(&mut rig.tw, hold, "ice_ore"),
            pool(&mut rig.tw, front, "ice_ore"),
        ),
        ore,
        "the yard hold and the storefront unmoved"
    );
}

/// Every live listing's id, quantity and revision.
fn all_listings(tw: &TestWorld) -> Vec<(u64, u64, u64)> {
    tw.world()
        .module_state::<mush_market::MarketStore>()
        .expect("market")
        .live_listings()
        .map(|listing| (listing.id.0, listing.quantity, listing.revision))
        .collect()
}

/// **T12 — the yard never calls the market** (doc 20 cut SM.12b; A2's source probe): the yard's
/// module names nothing in `mush_market` and mints no origin.
#[test]
fn sm12b_the_yard_never_calls_the_market() {
    let source = include_str!("../src/port/yard.rs");
    let production = source
        .split("#[cfg(test)]")
        .next()
        .expect("the production half");
    assert!(
        !production.contains("mush_market::"),
        "the yard calls no market"
    );
    assert!(
        !production.contains(".mint_origin("),
        "the yard mints nothing"
    );
}

// ---------------------------------------------------------------------------
// T13, T14 — the writes and the season
// ---------------------------------------------------------------------------

/// **T13 — a delivery is one receipted bundle** (doc 20 cut SM.12b; B8; §11's Tick row): a
/// hand-built delivery under the landed ordinal refuses (`Conflict`, nothing written), one under
/// the next ordinal with nothing due refuses too; the bundles' writes counted and pinned — a
/// delivery into a fresh storefront pool, one into an existing pool, a survey, and the licence
/// item's legacy writes.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12b_a_delivery_is_one_receipted_bundle() {
    let mut rig = yard_world();
    let (c, d) = (clerk(&rig.tw), dispatcher(&rig.tw));
    let (hold, front) = (yard_hold(&rig.tw), storefront(&rig.tw));
    let map_key = tacsim::port::port_map_key(&cid(LANE_PACK), &cid(OCCATOR)).to_string();
    let mut survey_writes = Vec::new();
    let mut licence_writes: Option<usize> = None;
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    for _ in 0..40 {
        let had = yard(&rig.tw).licence.is_some();
        let (_, queued) = fire(&mut rig);
        let ops = rig.tw.take_module_ops();
        if !had && let Some(licence) = yard(&rig.tw).licence {
            // The licence fire's every write, counted whole (nothing filtered out first): the
            // account, the economy's meta and the site map — item 8's three, and nothing else.
            let keys: Vec<String> = ops
                .iter()
                .map(|op| format!("{}:{}", op.ns, op_key(op)))
                .collect();
            println!("SM.12b T13 the licence fire's writes: {keys:?}");
            let mut sorted = keys.clone();
            sorted.sort();
            let mut want = vec![
                format!("economy:account:{}", licence.account),
                "economy:meta".to_owned(),
                format!("{}:{map_key}", tacsim::NS),
            ];
            want.sort();
            assert_eq!(
                sorted, want,
                "the licence fire writes item 8's three and nothing else"
            );
            licence_writes = Some(ops.len());
        }
        fold_module_ops_into(&mut rig.store, ops);
        for submit in &queued {
            if submit.actor == c && submit.action.key().id.as_str() == DISPATCH_ACTION {
                let (_, ops) = land_full(&mut rig, submit).expect("the survey lands");
                survey_writes.push(ops.len());
            } else {
                land_all(&mut rig, std::slice::from_ref(submit));
            }
        }
        if surveyed(&mut rig.tw, 'K') {
            break;
        }
    }
    // Deliveries: the hold seeded 30 t (the storefront's ice-ore pool absent: a fresh pool).
    assert_eq!(pool(&mut rig.tw, front, "ice_ore"), 0);
    seed_pool(&mut rig, hold, "ice_ore", 30, LotId(9_821));
    let mut counted: Vec<usize> = Vec::new();
    let mut landed: Vec<AuthoritativeSubmit> = Vec::new();
    for _ in 0..20 {
        let (_, queued) = fire(&mut rig);
        for submit in &queued {
            if submit.actor == d && submit.action.key().id.as_str() == DISPATCH_ACTION {
                let (_, ops) = land_full(&mut rig, submit).expect("the delivery lands");
                println!(
                    "SM.12b T13 delivery writes: {:?}",
                    ops.iter()
                        .map(|op| format!("{}:{}", op.ns, op_key(op)))
                        .collect::<Vec<_>>()
                );
                counted.push(ops.len());
                landed.push(submit.clone());
            } else {
                land_all(&mut rig, std::slice::from_ref(submit));
            }
        }
        if !landed.is_empty() {
            break;
        }
    }
    // A second delivery into the existing pool.
    seed_pool(&mut rig, hold, "ice_ore", 5, LotId(9_822));
    for _ in 0..20 {
        let (_, queued) = fire(&mut rig);
        for submit in &queued {
            if submit.actor == d && submit.action.key().id.as_str() == DISPATCH_ACTION {
                let (_, ops) = land_full(&mut rig, submit).expect("the delivery lands");
                println!(
                    "SM.12b T13 delivery writes: {:?}",
                    ops.iter()
                        .map(|op| format!("{}:{}", op.ns, op_key(op)))
                        .collect::<Vec<_>>()
                );
                counted.push(ops.len());
                landed.push(submit.clone());
            } else {
                land_all(&mut rig, std::slice::from_ref(submit));
            }
        }
        if landed.len() >= 2 {
            break;
        }
    }
    assert_eq!(landed.len(), 2, "two deliveries");
    // The replay guard: the landed ordinal again refuses and writes nothing.
    let ledger = ledger_of(&mut rig.tw).expect("the ledger");
    let replay = rekeyed(&landed[1], "again");
    let refused = land(&mut rig, &replay).expect_err("the landed ordinal again");
    assert!(
        refused.contains("that delivery is already made"),
        "{refused}"
    );
    assert_eq!(ledger_of(&mut rig.tw), Some(ledger.clone()));
    // The next ordinal with nothing due.
    let hold_t = pool(&mut rig.tw, hold, "ice_ore");
    if hold_t > 0 {
        debit_pool(&mut rig, hold, "ice_ore", hold_t);
    }
    let mut next = rekeyed(&landed[1], "next");
    next.parameters["ordinal"] = json!(ledger.deliveries + 1);
    let refused = land(&mut rig, &next).expect_err("nothing is due");
    assert!(refused.contains("nothing is due for delivery"), "{refused}");
    assert_eq!(ledger_of(&mut rig.tw), Some(ledger));
    println!(
        "SM.12b writes: deliver {} (fresh pool {}), survey {}, licence {}",
        counted[1],
        counted[0],
        survey_writes.first().copied().unwrap_or(0),
        licence_writes.unwrap_or(0)
    );
    // Staff-seeded ore is rooted (an origin lot), so its transfer carries lineage: two writes
    // over a mined, unrooted tonne's (T5 measures those: 6 into a fresh pool, 5 after).
    assert_eq!(
        counted,
        vec![8, 7],
        "a fresh pool's census, then the pools, the lineage and the ledger"
    );
    assert!(survey_writes.iter().all(|writes| *writes == 1));
    assert_eq!(survey_writes.len(), 4);
    assert_eq!(
        licence_writes,
        Some(3),
        "the account, the economy's meta, the site map"
    );
}

/// **T14 — the drones' season is measured** (doc 20 cut SM.12b; O-16; informational, asserting
/// arithmetic only): the opened ice deposits' reserves over F and G and over all four held
/// blocks, the water they make at 5 : 2 and the real days that covers at §12.3's 320 t a real day,
/// beside §11's ceiling.
#[test]
fn sm12b_the_drones_season_is_measured() {
    let mut rig = yard_world();
    let industry = rig
        .tw
        .world()
        .module_state::<mush_industry::IndustryHandles>()
        .expect("industry")
        .0
        .clone();
    let mut reserves = BTreeMap::new();
    for letter in ['F', 'G', 'J', 'K'] {
        let id = mush_industry::derive_system_deposit_id(&tacsim::surface::opening::receipt(
            &cid(OCCATOR),
            letter,
            OreFamily::Volatiles,
        ))
        .expect("a deposit id");
        let reserve = typed(&mut rig.tw, &industry.deposits, &id)
            .map_or(0, |deposit| deposit.initial_reserve());
        reserves.insert(letter, reserve);
    }
    let fg = reserves[&'F'] + reserves[&'G'];
    let all: u64 = reserves.values().sum();
    let water = |ore: u64| ore * 2 / 5;
    let days = |ore: u64| water(ore) as f64 / 320.0;
    println!(
        "SM.12b T14 ice reserves {reserves:?}: F+G {fg} t ({} t of water, {:.2} real days at 320 t \
         a day); all four {all} t ({} t of water, {:.2} real days); the ceiling F+G <= 3072 t of \
         ore <= 1229 t of water ({:.2} days), all four {:.2} days",
        water(fg),
        days(fg),
        water(all),
        days(all),
        1229.0 / 320.0,
        2458.0 / 320.0
    );
    assert_eq!(water(5), 2);
    assert!(fg <= all);
    assert!(fg <= 3_072 && all <= 6_144, "within §11's ceiling");
}

// ---------------------------------------------------------------------------
// T15, T16 — the lane and the line
// ---------------------------------------------------------------------------

/// The published lane v14 (doc 20 cut SM.12a)'s digest, read before the re-bless.
const LANE_V14_DIGEST: &str = "c507f74c135562151b6dbf36fb1d977e14f4c8ac1884f67082be901fcd94a8f8";

/// The shipped lane at `version`, edited, re-signed.
fn lane_variant(version: u32, edit: impl FnOnce(&mut Value)) -> String {
    let mut doc: Value = serde_json::from_str(LANE_JSON).expect("the lane is JSON");
    doc["manifest"]["version"] = json!(version);
    doc["manifest"]["digest"] = json!(support::ground::UNSIGNED);
    edit(&mut doc);
    support::ground::sign(&serde_json::to_string_pretty(&doc).expect("serializes"))
}

/// **T15 — `strip_yard` rebuilds the published v14** (doc 20 cut SM.12b): the shipped lane is v15
/// and differs from the stripped shape in Occator Rim's yard alone; stripped and re-signed at 14
/// it is SM.12a's published v14, digest for digest; `strip_yard` is idempotent.
#[test]
fn sm12b_strip_yard_rebuilds_the_published_v14() {
    assert_eq!(LANE_VERSION, 15, "lane v15 since doc 20 cut SM.12b");
    let pack = load_pack(&lane_variant(14, support::drones::strip_yard)).expect("the v14 shape");
    assert_eq!(pack.manifest().digest().to_hex(), LANE_V14_DIGEST);
    let shipped: Value = serde_json::from_str(LANE_JSON).expect("JSON");
    let mut stripped = shipped.clone();
    support::drones::strip_yard(&mut stripped);
    assert!(
        shipped["entries"]["occator_port"]["def"]["site"]
            .get("yard")
            .is_some()
    );
    let mut restored = stripped.clone();
    restored["entries"]["occator_port"]["def"]["site"]["yard"] =
        shipped["entries"]["occator_port"]["def"]["site"]["yard"].clone();
    assert_eq!(restored, shipped, "the yard is the only difference");
    let once = stripped.clone();
    support::drones::strip_yard(&mut stripped);
    assert_eq!(stripped, once, "idempotent");
}

/// **T16 — the Dispatch line reads the yard** (doc 20 cut SM.12b; B11): through `@tacsim/port
/// status`, order 6 before Occator Rim opens, 7 while the house's items are owed, 10 once surveyed,
/// 9 while working with its figures, and order 2 once the delivery ledger is damaged by the key
/// rule (a valid ledger naming another site stored under the yard's key).
#[test]
#[allow(clippy::too_many_lines)]
fn sm12b_the_dispatch_line_reads_the_yard() {
    let mut rig = stocked();
    set_rate(&mut rig.tw, 24);
    let head = "occator_port dispatch occator: ";
    assert_eq!(
        dispatch_line(&mut rig.tw),
        format!(
            "{head}2 rigs in the Occator Yard's bay; the site is not open, so nothing is filed. \
             [ready]"
        )
    );
    let _ = open_occator(&mut rig);
    assert_eq!(
        dispatch_line(&mut rig.tw),
        format!("{head}waiting on the Registry's seeds, licence and survey. [pending]")
    );
    until_surveyed(&mut rig);
    assert_eq!(
        dispatch_line(&mut rig.tw),
        format!("{head}2 rigs in the Occator Yard's bay; nothing filed yet. [ready]")
    );
    let hold = yard_hold(&rig.tw);
    seed_pool(&mut rig, hold, "ice_ore", 12, LotId(9_831));
    until(&mut rig, 60, "a delivery and both plans", &|rig| {
        ledger_of(&mut rig.tw).is_some()
            && yard_rigs(&rig.tw)
                .iter()
                .all(|tv| plan_of(&mut rig.tw, *tv).is_some())
    });
    let line = dispatch_line(&mut rig.tw);
    println!("SM.12b T16 working: {line}");
    assert!(
        line.starts_with(head)
            && line.contains(
                " of 2 out, 2 working a plan, 2 plans filed in all; 12 t of ice ore \
                             delivered to its storefront today, 12 t in all. [ready]"
            ),
        "{line}"
    );
    // Damage: a valid ledger naming another site, under the yard's key.
    let slot = rig
        .typed
        .iter()
        .find(|(_, value)| {
            value["family"]["id"] == tacsim::port::yard::DISPATCH_FAMILY
                && value["key"].as_str() == Some(OCCATOR)
        })
        .map(|(slot, _)| slot.clone())
        .expect("the dispatch ledger's typed record");
    rig.typed
        .get_mut(&slot)
        .expect("the record")
        .get_mut("value")
        .expect("its value")["site"] = json!("juling_port");
    let mut rig = reboot(rig, &[]);
    assert_eq!(
        dispatch_line(&mut rig.tw),
        format!("{head}the delivery ledger cannot be read. [flagged]")
    );
}

// ---------------------------------------------------------------------------
// B7 — the house's standing
// ---------------------------------------------------------------------------

/// **The house standing reads the industry account** (doc 20 cut SM.12b; B7; 12b-E6; placed here,
/// beside the world it needs): an account of the house in the licence instrument pre-opened
/// below the clerk's — scoped, `Frozen` — is the one the industry predicate reads, so the clerk
/// adopts it, the dispatcher's standing reads `Suspended` and its plans' compile refuses on the
/// licence; the predicate itself (`mush_industry::extraction_authorized`, over F's ice deposit
/// and the house's seeded claim) answers `LicenceInactive` for the house; and an `Active` account
/// of the house opened later — a higher id — changes neither: the lowest id is the one read.
#[test]
fn sm12b_the_house_standing_reads_the_industry_account() {
    let mut rig = yard_world();
    let d = dispatcher(&rig.tw);
    let house = house(&rig.tw);
    let scope = port_map(&rig.tw, "ceres_port").orgs[&cid("occator")].root_scope;
    let opened = support::with_ctx(&mut rig.tw, |ctx| {
        let id = ctx
            .w
            .module_state_mut::<mush_economy::EconomyStore>()
            .and_then(mush_economy::EconomyStore::alloc_account)
            .expect("an account id");
        mush_economy::open_account(
            ctx,
            mush_economy::Account {
                id,
                owner: house.clone(),
                instrument: tacsim::economy::licence_instrument().expect("the instrument"),
                scope: Some(mush_org::OrgScopeId(scope)),
                name: SmolStr::new("pre-opened"),
                status: mush_economy::AccountStatus::Frozen,
                posted_balance: mush_economy::Amount::ZERO,
                held_amount: mush_economy::Amount::ZERO,
                last_hold_seq: 0,
                credit: mush_economy::CreditPolicy::forbid(),
                last_journal_seq: 0,
                revision: 1,
                approval: None,
            },
        );
        id
    });
    let _ = seeded(&mut rig);
    assert_eq!(
        licence_account(&rig.tw),
        opened,
        "the clerk adopts the industry's account"
    );
    assert_eq!(
        tacsim::port::licence_standing(rig.tw.world(), d, ic_now(&rig.tw)),
        tacsim::port::LicenceStanding::Suspended
    );
    let instrument = tacsim::economy::licence_instrument().expect("the instrument");
    assert_eq!(
        mush_industry::licence::licence_accounts(rig.tw.world(), &instrument, &house),
        Some(mush_economy::AccountStatus::Frozen)
    );
    // The predicate itself, for the house over F's ice and its seeded claim.
    let industry = rig
        .tw
        .world()
        .module_state::<mush_industry::IndustryHandles>()
        .expect("industry")
        .0
        .clone();
    let deposit = mush_industry::derive_system_deposit_id(&tacsim::surface::opening::receipt(
        &cid(OCCATOR),
        'F',
        OreFamily::Volatiles,
    ))
    .expect("a deposit id");
    let body = typed(&mut rig.tw, &industry.deposits, &deposit).expect("F's ice is open");
    let claims = {
        let mut txn = rig
            .tw
            .engine_mut()
            .action_transaction(&industry.deposits)
            .expect("a transaction");
        industry
            .claims_over(&mut txn, mush_industry::ClaimSubject::Deposit { deposit })
            .expect("the claims read")
    };
    assert!(
        claims.iter().any(|claim| claim.holder() == &house),
        "the house's seeded claim stands over F's ice"
    );
    let verdict = |tw: &TestWorld| {
        mush_industry::extraction_authorized(
            tw.world(),
            d,
            &house,
            &body,
            body.initial_reserve(),
            &claims,
            Some(&instrument),
            &tw.world().world_ic_now(),
        )
        .map(|authorized| authorized.claim)
    };
    assert_eq!(
        verdict(&rig.tw),
        Err(mush_industry::ExtractionRefusal::LicenceInactive),
        "the industry predicate refuses the house on its frozen licence"
    );
    // A later, higher-id `Active` account of the house in the instrument: the lowest id is read.
    let later = support::with_ctx(&mut rig.tw, |ctx| {
        let id = ctx
            .w
            .module_state_mut::<mush_economy::EconomyStore>()
            .and_then(mush_economy::EconomyStore::alloc_account)
            .expect("an account id");
        mush_economy::open_account(
            ctx,
            mush_economy::Account {
                id,
                owner: house.clone(),
                instrument: instrument.clone(),
                scope: Some(mush_org::OrgScopeId(scope)),
                name: SmolStr::new("opened later"),
                status: mush_economy::AccountStatus::Active,
                posted_balance: mush_economy::Amount::ZERO,
                held_amount: mush_economy::Amount::ZERO,
                last_hold_seq: 0,
                credit: mush_economy::CreditPolicy::forbid(),
                last_journal_seq: 0,
                revision: 1,
                approval: None,
            },
        );
        id
    });
    assert!(later.0 > opened.0, "the later account has the higher id");
    assert_eq!(
        mush_industry::licence::licence_accounts(rig.tw.world(), &instrument, &house),
        Some(mush_economy::AccountStatus::Frozen),
        "the lowest id is the one read"
    );
    assert_eq!(
        tacsim::port::licence_standing(rig.tw.world(), d, ic_now(&rig.tw)),
        tacsim::port::LicenceStanding::Suspended
    );
    assert_eq!(
        verdict(&rig.tw),
        Err(mush_industry::ExtractionRefusal::LicenceInactive)
    );
    assert_eq!(
        dispatch_line(&mut rig.tw),
        "occator_port dispatch occator: its licence account is not active. [flagged]"
    );
    for _ in 0..30 {
        let (_, queued) = fire(&mut rig);
        assert!(
            filings(&queued, d).is_empty(),
            "no plan compiles on a suspended licence"
        );
        land_all(&mut rig, &queued);
    }
}

// ---------------------------------------------------------------------------
// T1's upgrade clause — the yard on a kept world
// ---------------------------------------------------------------------------

/// **A pack apply lays the yard on a kept world** (doc 20 cut SM.12b; T1's upgrade clause, in
/// `mill.rs`'s shelf-and-extra-lane shape — SM.12a's own precedent, whose kept-world test seeds
/// `lane_variant(LANE_VERSION, strip_mill)` and takes it to `LANE_VERSION + 1`: the v14 shape is
/// the shipped lane less its yard (T15), so the shipped content less its yard, re-signed at 15,
/// taken to the shipped content at 16 is exactly the v14 → v15 upgrade, with no second copy of
/// the shipped version on the shelf): `@atlas/pack plan` reads one update, `occator_port`, and
/// nothing created or retired, and the apply makes exactly the yard's objects; nothing is
/// submitted.
#[test]
fn sm12b_a_pack_apply_lays_the_yard_on_a_kept_world() {
    let next = lane_variant(LANE_VERSION + 1, |_| {});
    let seed = lane_variant(LANE_VERSION, support::drones::strip_yard);
    let mut tw = TestWorld::with_modules(upgrade_modules(&seed, &next));
    no_refused_writes(&tw);
    sweep(&mut tw, 120);
    assert!(
        port_map(&tw, OCCATOR)
            .site
            .and_then(|site| site.yard)
            .is_none(),
        "no yard before the apply"
    );
    let objects = tw.world().iter().count();
    let _ = drain(&mut tw);
    let plan = run(&mut tw, GOD, &format!("@atlas/pack plan {LANE_PACK}"));
    println!("SM.12b the plan:\n{plan}");
    // One update — Occator Rim's entry — and nothing created, retired or otherwise updated.
    assert!(
        plan.contains(&format!(
            "Plan for {LANE_PACK}@{}: 0 create, 1 update, 0 retire, ",
            LANE_VERSION + 1
        )),
        "{plan}"
    );
    let rows: Vec<&str> = plan
        .lines()
        .filter(|line| {
            let word = line.split_whitespace().next().unwrap_or("");
            matches!(word, "create" | "update" | "retire")
        })
        .collect();
    assert_eq!(rows.len(), 1, "{plan}");
    assert_eq!(
        rows[0].split_whitespace().collect::<Vec<_>>(),
        vec!["update", "occator_port", "(port)"],
        "{plan}"
    );
    let fingerprint = plan
        .split("@atlas/pack apply ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_default()
        .to_owned();
    let apply = run(&mut tw, GOD, &format!("@atlas/pack apply {fingerprint}"));
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@{}", LANE_VERSION + 1)),
        "{apply}"
    );
    assert_eq!(
        tw.engine().queued_authoritative_actors().count(),
        0,
        "the apply submits nothing"
    );
    let row = yard(&tw);
    let made: usize = row
        .hulls
        .iter()
        .map(|hull| 1 + hull.rooms.len() + hull.exits.len())
        .sum::<usize>()
        + row
            .rigs
            .iter()
            .map(|rig| 1 + rig.rooms.len())
            .sum::<usize>()
        + usize::from(row.dispatcher.is_some());
    assert_eq!(
        tw.world().iter().count() - objects,
        made,
        "exactly the yard's objects"
    );
    assert_eq!(row.rigs.len(), 2);
    assert!(row.hold.is_some());
    no_refused_writes(&tw);
}

/// The production roster seeded with `seed` as its lane, with `next` shelved and loaded
/// (`mill.rs`'s `Roster`).
fn upgrade_modules(seed: &str, next: &str) -> Vec<Box<dyn mush_core::GameModule>> {
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
    cfg.time.ephemeris.rate = 0;
    cfg.seeds.push(tacsim::config::PackSource {
        name: "ops/lane_next.json".to_owned(),
        bytes: next.to_owned(),
    });
    cfg.seeds.push(tacsim::config::PackSource {
        name: "ops/lane.json".to_owned(),
        bytes: seed.to_owned(),
    });
    let (mut mods, inventory) = comp::server_modules_with_tacsim(
        mush_weather::WeatherConfig::default(),
        EconomyConfig::default(),
        mush_atlas::AtlasConfig::default(),
        &onboarding,
        &game,
        comp::recovery_policy(&cfg.recovery),
    );
    let at = mods
        .iter()
        .position(|m| m.name() == mush_atlas::NS)
        .expect("the atlas composes");
    let mut full = tacsim::definitions::content::station_packs().expect("the station packs");
    full.push(mush_content::load_pack(next).expect("the next lane verifies"));
    mods[at] = Box::new(
        mush_atlas::AtlasModule::with_config(mush_atlas::AtlasConfig::default()).with_shelf(full),
    );
    mods.push(comp::worldgen_module_with(
        Arc::clone(&llm),
        mush_worldgen::WorldgenConfig::default(),
    ));
    mods.push(comp::npc_module_with(
        comp::npc_agent_adapter(),
        &mush_config::config::NpcCfg::default(),
    ));
    mods.push(comp::tacsim_test_world_module_wired(cfg, false, inventory));
    let packs = support::boot_packs_with_lane(
        mush_content::load_pack(seed).expect("the seed lane verifies"),
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
    mods
}
