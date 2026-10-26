//! **Doc 20 cut SM.12a acceptance — the mill** (doc 20 §7.3 "The mill", §11, §12.3, §13's SM.12
//! row; D-20-10; Q-20-1; doc 18 §5's P5; the SM.12a brief's B12).
//!
//! The lane's two mills: the Freeport Exchange mills the metal ore it buys into metal
//! concentrate, Occator its ice ore into water ice, five tonnes to two, 20 t of ore a batch and
//! at most 40 t an IC day — the two `mill` rows of the shipped lane, v14 (with the relay seed
//! `tacsim.relay.luna_ceres` re-signed at v3 beside it, since it requires the lane at an exact
//! version). **Every test boots the production roster** (the shipped lane and relay v3) unless
//! it needs lane rows the shipped lane does not have — a changed ratio, a removed row, 30 t of
//! seeded concentrate, a mill-less world — and then boots that lane as an operator seed or loads
//! it as the next version; a seeded lane boots the historical relay seed
//! (`support::boot_packs_with_lane`), so a seeded rig is compared only with another seeded rig.
//! On the port tick's stock arm, after the intake drip and before the lapsed-bid re-post, the
//! Ceres clerk makes one mill act a fire: a **batch** (`tacsim_port_mill`: the ore debited and the product credited at the storefront, the mill's ledger advanced, one
//! bundle) or a **restock** (`mush_market::restock_ask`: the house's drawn ask raised by the
//! units milled and not yet listed, the vendor row's `mill_listed` counter in the same outbox).
//!
//! Composed on the composition root (`plumbing.rs`'s rig with `commerce.rs`'s `Roster`: the test
//! world's layer with the game content, or an operator lane, an optional shelf and an optional
//! extra lane the layer loads). Readiness is read through `@tacsim/port status` and the desk is
//! proved through `+coop/enrol` (the brief's D26); every lane variant the clerk must see is both
//! shelved and loaded (D25); a mill ledger is damaged through the key rule, never by an
//! undecodable record (D19); a test whose span passes six real days runs at world rate 24 (D18).
//! Every test ends asserting no namespace write was refused.

use crate::support;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use mush_core::{
    AccountId, ActionStateBootRecord, Audience, AuthoritativeSubmit, CanonicalId, CommitToken, GOD,
    IdempotencyKey, IdempotencyScope, ModuleWrite, ObjId, OfferExecutionContext, OfferSubmission,
    RequestUid, ScopedIdempotencyKey, StateFamilyHandle,
};
use mush_economy::{ContractId, ContractState, EconomyConfig, EconomyStore, VaultId};
use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
use mush_market::{Listing, ListingId, MarketStore};
use mush_org::PrincipalId;
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde_json::Value;
use smol_str::SmolStr;
use tacsim::config::PackSource;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK, LANE_VERSION};
use tacsim::pack::{PackEntry, load_pack};
use tacsim::port::apply::room_by_key;
use tacsim::port::mill::{MILL_ACTION, MillId, MillLedger};
use tacsim::port::{Binding, HullRow, PortDef, PortMap, PortMapState, gated, ic_stamp, room_key};
use tacsim::{TacsimConfig, TacsimStore};

use support::claims::{MapOracle, install_oracle};
use support::ground::{UNSIGNED, sign};
use support::mill::strip_mill;

/// Lane v13's published digest (relay cut R9b), which `strip_mill` rebuilds.
const LANE_V13_DIGEST: &str = "34d47907dd42dbcd5192aae05d6eaf76c89dd40abd877845ac83ec35968e47c1";

/// IC seconds in an IC day.
const DAY: i64 = 86_400;

/// The Exchange's org and Occator's.
const EXCHANGE: &str = "freeport_exchange";
const OCCATOR: &str = "occator";

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

fn lane_id() -> CanonicalId {
    cid(LANE_PACK)
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
    match load_pack(LANE_JSON)
        .expect("the lane verifies")
        .entry(&cid(entry))
    {
        Some(PackEntry::Port(def)) => def.clone(),
        other => panic!("{entry} is a port entry: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// The rig (plumbing.rs's, with commerce.rs's Roster)
// ---------------------------------------------------------------------------

/// How a test roster is composed: the shipped game content, or an operator lane in its place;
/// an optional shelf; an optional extra lane version the layer loads (so the clerk and the
/// report read an applied update — `commerce.rs`'s `extra_lane`).
#[derive(Clone, Default)]
struct Roster {
    seed_lane: Option<String>,
    shelf: Option<Vec<mush_content::Pack>>,
    extra_lane: Option<String>,
}

impl Roster {
    fn seeded(lane: String) -> Roster {
        Roster {
            seed_lane: Some(lane),
            ..Roster::default()
        }
    }

    /// The shipped content with `next` both shelved and loaded (D25; `commerce.rs`'s
    /// `ic8_an_ask_reprice_withdraws_and_the_arm_relists` shape).
    fn next(next: &str) -> Roster {
        Roster {
            seed_lane: None,
            shelf: Some(vec![
                mush_content::load_pack(next).expect("the next lane verifies"),
            ]),
            extra_lane: Some(next.to_owned()),
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
        cfg.time.ephemeris.rate = 0;
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

/// A booted world, its roster, its oracle, the cumulative store, the typed post-images, and
/// the keys the last fire wrote of the market's listings, its meta and `ceres_port`'s map.
struct Rig {
    tw: TestWorld,
    oracle: Arc<MapOracle>,
    roster: Roster,
    store: HashMap<SmolStr, mush_core::ModuleBootData>,
    typed: BTreeMap<(SmolStr, SmolStr), Value>,
    last_keys: Vec<String>,
    /// How many times `ceres_port`'s map document was written since the boot.
    map_sets: usize,
}

fn no_refused_writes(tw: &TestWorld) {
    let refused = tw.world().refused_namespace_writes();
    assert!(refused.is_empty(), "refused namespace writes: {refused:?}");
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

/// Boot `roster` and sweep once, so the capital settles.
fn boot(roster: Roster) -> Rig {
    let mut tw = TestWorld::with_modules(roster.modules());
    no_refused_writes(&tw);
    with_offer_keys(&mut tw);
    let oracle = Arc::new(MapOracle::default());
    install_oracle(&mut tw, &oracle);
    let mut rig = Rig {
        tw,
        oracle,
        roster,
        store: HashMap::new(),
        typed: BTreeMap::new(),
        last_keys: Vec::new(),
        map_sets: 0,
    };
    sweep(&mut rig, 120);
    rig
}

/// Boot the production roster, settle and stock every port, at world rate 1 (every IC instant
/// exact).
fn stocked() -> Rig {
    stocked_with(Roster::default())
}

fn stocked_with(roster: Roster) -> Rig {
    let mut rig = boot(roster);
    stock(&mut rig);
    set_rate(&mut rig.tw, 1);
    rig
}

/// Fold every write since the first boot into the rig's store and reboot into its roster, in
/// place (plumbing's unkeyed `reboot`: the mill reads no geology).
fn reboot(rig: &mut Rig) {
    fold(rig);
    let tw = std::mem::take(&mut rig.tw);
    let mut tw = tw
        .reboot_into(rig.roster.modules(), rig.store.clone())
        .expect("the reboot boots");
    no_refused_writes(&tw);
    tw.engine_mut()
        .hydrate_action_state(
            rig.typed
                .iter()
                .map(|((ns, key), value)| {
                    ActionStateBootRecord::new(ns.clone(), key.clone(), value.clone())
                })
                .collect::<Vec<_>>(),
        )
        .expect("the typed state hydrates");
    with_offer_keys(&mut tw);
    install_oracle(&mut tw, &rig.oracle);
    rig.tw = tw;
    rig.last_keys.clear();
}

/// Fold the module ops queued so far into the store; return the keys among them the mill's
/// acts write — a listing, the market's meta and `ceres_port`'s map (T11).
fn fold(rig: &mut Rig) -> Vec<String> {
    let ops = rig.tw.take_module_ops();
    let map_key = tacsim::port::port_map_key(&lane_id(), &cid("ceres_port"));
    rig.map_sets += ops
        .iter()
        .filter(|op| matches!(&op.op, ModuleWrite::JsonSet { key, .. } if *key == map_key))
        .count();
    let keys = ops
        .iter()
        .filter_map(|op| match &op.op {
            ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key }
                if key.starts_with("market:listing:")
                    || key.as_str() == "market:meta"
                    || *key == map_key =>
            {
                Some(key.to_string())
            }
            _ => None,
        })
        .collect();
    fold_module_ops_into(&mut rig.store, ops);
    keys
}

/// Everything due `secs` on; the writes it made are the rig's `last_keys`.
fn sweep(rig: &mut Rig, secs: i64) {
    fold(rig);
    let t = rig.tw.world().now().saturating_add(secs);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1000));
    rig.last_keys = fold(rig);
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

fn market(tw: &TestWorld) -> &MarketStore {
    tw.world().module_state::<MarketStore>().expect("market")
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

fn teleport(tw: &mut TestWorld, who: ObjId, room: ObjId) {
    let _ = run(tw, GOD, &format!("@teleport #{}=#{}", who.0, room.0));
    assert_eq!(
        tw.world().get(who).and_then(|o| o.location),
        Some(room),
        "the teleport landed"
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

/// The registered action and the resolved request of one queued submission.
type Resolved = (
    mush_core::DefinitionRef,
    mush_core::ResolvedActionInvocation,
    [u8; 16],
);

/// Resolve one queued submission the way the world task does: regenerate the actor's offer and
/// resolve the parameters against it.
fn resolve(rig: &Rig, submit: &AuthoritativeSubmit) -> Result<Resolved, String> {
    let token = next_token();
    let engine = rig.tw.engine();
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
    Ok((action, resolved, token))
}

/// Drive one queued submission, keeping its typed post-images for a reboot; the count of the
/// bundle's staged writes.
fn land_counting(rig: &mut Rig, submit: &AuthoritativeSubmit) -> Result<usize, String> {
    let (action, resolved, token) = resolve(rig, submit)?;
    let (prepared, _) = rig
        .tw
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(
            &action,
            &resolved,
            CommitToken::from_bytes(token),
        )
        .map_err(|error| format!("{error:?}"))?;
    let (ops, install) = prepared.into_parts();
    let count = ops.len();
    rig.tw
        .engine_mut()
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
    Ok(count)
}

fn land(rig: &mut Rig, submit: &AuthoritativeSubmit) -> Result<(), String> {
    land_counting(rig, submit).map(|_| ())
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

fn is_mill(submit: &AuthoritativeSubmit) -> bool {
    submit.action.key().id.as_str() == MILL_ACTION
}

/// Land `queued`: a mill batch must land; every other submission lands quietly.
fn land_queue(rig: &mut Rig, queued: &[AuthoritativeSubmit]) {
    for submit in queued {
        let landed = land(rig, submit);
        if is_mill(submit) {
            landed.unwrap_or_else(|error| panic!("a mill batch did not land: {error}"));
        }
    }
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

/// One port-tick fire (6 s on), its submissions drained (not landed); the writes it made are
/// the rig's `last_keys`.
fn fire(rig: &mut Rig) -> Vec<AuthoritativeSubmit> {
    fold(rig);
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    rig.last_keys = fold(rig);
    drain(&mut rig.tw)
}

/// One fire, its queue landed (a mill batch must land).
fn fire_landing(rig: &mut Rig) -> Vec<AuthoritativeSubmit> {
    let queued = fire(rig);
    land_queue(rig, &queued);
    queued
}

/// Fire and land until a mill batch lands (at most `max` fires); that batch.
fn fire_until_batch(rig: &mut Rig, max: usize) -> AuthoritativeSubmit {
    for _ in 0..max {
        let queued = fire_landing(rig);
        if let Some(batch) = queued.into_iter().find(is_mill) {
            return batch;
        }
    }
    panic!("no mill batch landed within {max} fires");
}

/// Fire and land until a fire queues nothing and writes nothing the Ceres clerk's acts write
/// (at most `max` fires); the fires made.
fn fire_until_idle(rig: &mut Rig, max: usize) -> usize {
    for n in 1..=max {
        let queued = fire_landing(rig);
        if queued.is_empty() && rig.last_keys.is_empty() {
            return n;
        }
    }
    panic!("the clerk was not idle within {max} fires");
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
        let queued = fire(rig);
        land_all(rig, &queued);
    }
    panic!("the ports were not stocked within 40 fires");
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

/// A chartered pilot whose hull the yard has made a Dray-P (86 t of bulk and ore), with its
/// hold vault.
fn dray_p(rig: &mut Rig, who: &str, seed: i64, name: &str) -> (ObjId, VaultId) {
    let (pilot, row) = chartered(rig, who, seed, name);
    let out = run(&mut rig.tw, pilot, "+coop/refit prospector");
    assert!(out.contains("as a Dray-P"), "{out}");
    let stowage = ObjId(row.rooms[&cid("stowage")]);
    let hold = economy(&rig.tw)
        .snapshot()
        .vault_by_backing(&mush_economy::CustodyRef::SpatialInterior(stowage))
        .expect("the hold vault")
        .id;
    (pilot, hold)
}

/// Seed `qty` of `good` into `vault`'s pool through `lot` (additive; staff's path).
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

/// The pool of `good` at `vault`.
fn pool(tw: &mut TestWorld, vault: VaultId, good: &str) -> u64 {
    let families = tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    let key = mush_inventory::pool_key(EndpointRef(vault), &cid(good)).expect("a pool key");
    let mut txn = tw
        .engine_mut()
        .action_transaction(&families.pools)
        .expect("a transaction");
    txn.get(&families.pools, &key)
        .expect("the family reads")
        .map_or(0, |scalar| {
            u64::try_from(scalar.current()).expect("a non-negative pool")
        })
}

/// The marker ring of `vault`, as the inventory keeps it.
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

/// Move the world, without firing, to `offset` IC seconds past the next IC-day boundary, at
/// whatever rate the world runs (D18's rate-aware helper).
fn past_next_boundary(rig: &mut Rig, offset: i64) {
    let now = ic_now(&rig.tw);
    let next = (now.div_euclid(DAY) + 1) * DAY + offset;
    let rate = i64::from(world_rate(&rig.tw).max(1));
    let real = (next - now + rate - 1) / rate;
    let t = rig.tw.world().now().saturating_add(real);
    rig.tw.set_now(t);
    assert!(ic_now(&rig.tw) >= next, "past the boundary");
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

/// The Exchange Hall (C07): the ore bids' board, the asks' counter.
fn exchange(tw: &TestWorld) -> ObjId {
    station_room(tw, "ceres_port", "room.c07_exchange")
}

/// Every economy account's `(posted, held)`, minor units.
fn money(tw: &TestWorld) -> BTreeMap<u64, (i64, i64)> {
    economy(tw)
        .snapshot()
        .accounts()
        .map(|a| (a.id.0, (a.posted_balance.minor(), a.held_amount.minor())))
        .collect()
}

/// A listing's quantity, if it stands.
fn quantity(tw: &TestWorld, listing: u64) -> Option<u64> {
    market(tw).listing(ListingId(listing)).map(|l| l.quantity)
}

/// A standing ore bid's listing.
fn bid_listing(tw: &TestWorld, org: &str, good: &str) -> u64 {
    port_map(tw, "ceres_port").vendors[&cid(org)].bids[&cid(good)].listing
}

/// `+market/sell <bid>=<what>` at the Exchange Hall, the deliver leg landed; the sale's
/// contract.
fn sell(rig: &mut Rig, pilot: ObjId, org: &str, good: &str, what: &str) -> ContractId {
    let hall = exchange(&rig.tw);
    teleport(&mut rig.tw, pilot, hall);
    let listing = bid_listing(&rig.tw, org, good);
    let out = run(
        &mut rig.tw,
        pilot,
        &format!("+market/sell {listing}={what}"),
    );
    assert!(out.contains("Sold "), "{out}");
    let queued = drain(&mut rig.tw);
    land_all(rig, &queued);
    market(&rig.tw)
        .quotes_of_seller(&PrincipalId::Object(pilot))
        .last()
        .and_then(|q| q.contract)
        .expect("a sale contract")
}

/// Seed `qty` of `ore` into `hold` and sell it all into `org`'s bid; the contract.
fn seed_and_sell(
    rig: &mut Rig,
    pilot: ObjId,
    hold: VaultId,
    org: &str,
    ore: &str,
    qty: u64,
    lot: u64,
) -> ContractId {
    seed_pool(rig, hold, ore, qty, LotId(lot));
    sell(rig, pilot, org, ore, &qty.to_string())
}

fn state_of(tw: &TestWorld, c: ContractId) -> Option<ContractState> {
    economy(tw).snapshot().contract(c).map(|c| c.state)
}

fn settled(tw: &TestWorld, c: ContractId) -> bool {
    matches!(state_of(tw, c), Some(ContractState::Settled { .. }))
}

/// **The mill-aware settle** (the brief's B5): sweep 31 s at a time until `contract` settles,
/// landing each sweep's queue one by one; with `stop_at_batch`, return the first mill batch
/// the moment its sweep's queue has landed (the contract perhaps still open).
fn settle_until(
    rig: &mut Rig,
    contract: ContractId,
    stop_at_batch: bool,
) -> Option<AuthoritativeSubmit> {
    for _ in 0..12 {
        if settled(&rig.tw, contract) && !stop_at_batch {
            return None;
        }
        sweep(rig, 31);
        let queued = drain(&mut rig.tw);
        land_queue(rig, &queued);
        if stop_at_batch && let Some(batch) = queued.into_iter().find(is_mill) {
            return Some(batch);
        }
        if stop_at_batch && settled(&rig.tw, contract) {
            return None;
        }
    }
    if settled(&rig.tw, contract) {
        return None;
    }
    panic!(
        "contract #{contract} did not settle: {:?}",
        state_of(&rig.tw, contract)
    );
}

fn settle(rig: &mut Rig, contract: ContractId) {
    settle_until(rig, contract, false);
}

/// Settle a sale, then fire until its batch lands; that batch.
fn sale_batch(rig: &mut Rig, contract: ContractId) -> AuthoritativeSubmit {
    let batch = settle_until(rig, contract, true);
    match batch {
        Some(batch) => batch,
        None => fire_until_batch(rig, 24),
    }
}

/// Mint `minor` into `who`'s own credit account from the treasury.
fn top_up(tw: &mut TestWorld, who: ObjId, minor: i64) {
    let instrument = mush_economy::InstrumentId::new(tacsim::CREDIT).expect("credit");
    let account = economy(tw)
        .snapshot()
        .accounts_of(&PrincipalId::Object(who))
        .find(|a| a.instrument == instrument)
        .map(|a| a.id)
        .expect("the pilot's account");
    let treasury = port_map(tw, "ceres_port").treasury.expect("a treasury");
    let out = run(tw, GOD, &format!("@econ/mint {treasury}={account} {minor}"));
    assert!(!out.contains("Usage"), "{out}");
}

/// The newest quote `buyer` holds.
fn newest_quote(tw: &TestWorld, buyer: ObjId) -> mush_market::QuoteId {
    market(tw)
        .quotes_of_buyer(&PrincipalId::Object(buyer))
        .map(|q| q.id)
        .max()
        .expect("a quote")
}

/// Quote `qty` of `listing` and accept it with no destination; the accept's reply and the
/// quote.
fn buy(
    rig: &mut Rig,
    pilot: ObjId,
    listing: ListingId,
    qty: u64,
) -> (String, mush_market::QuoteId) {
    let hall = exchange(&rig.tw);
    teleport(&mut rig.tw, pilot, hall);
    let quoted = run(
        &mut rig.tw,
        pilot,
        &format!("+market/quote {listing}={qty}"),
    );
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = newest_quote(&rig.tw, pilot);
    let accepted = run(&mut rig.tw, pilot, &format!("+market/accept {quote}"));
    (accepted, quote)
}

/// Buy and land the Draw; the purchase's contract.
fn buy_and_land(rig: &mut Rig, pilot: ObjId, listing: ListingId, qty: u64) -> ContractId {
    let (accepted, quote) = buy(rig, pilot, listing, qty);
    let queued = drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "{accepted}");
    assert_eq!(queued[0].actor, pilot, "the Draw lands as the pilot");
    land_all(rig, &queued);
    contract_of(&rig.tw, quote)
}

fn contract_of(tw: &TestWorld, quote: mush_market::QuoteId) -> ContractId {
    market(tw)
        .quote(quote)
        .and_then(|q| q.contract)
        .expect("the accept drafted a contract")
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

/// A dispatcher writing `level` and up into `capture` (`boot_parity.rs`'s registration anchor).
fn capture_dispatch(capture: Capture, level: tracing::Level) -> tracing::Dispatch {
    static ANCHOR: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
    ANCHOR.get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
    tracing::Dispatch::new(
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(level)
            .with_writer(capture)
            .finish(),
    )
}

/// Every write so far folded into the rig's store; then the stored document at `key`.
fn stored(rig: &mut Rig, key: &str) -> Option<Value> {
    fold(rig);
    rig.store
        .values()
        .find_map(|data| data.kv.iter().find(|(k, _)| k.as_str() == key))
        .map(|(_, value)| value.clone())
}

/// Overwrite the stored document at `key` as a damaged store would hold it; the next reboot
/// loads it.
fn damage(rig: &mut Rig, key: &str, value: Value) {
    fold(rig);
    let slot = rig
        .store
        .values_mut()
        .find_map(|data| data.kv.iter_mut().find(|(k, _)| k.as_str() == key))
        .expect("the document is stored");
    slot.1 = value;
}

/// Edit `ceres_port`'s stored map with `edit`; the next reboot loads it.
fn damage_map(rig: &mut Rig, edit: impl FnOnce(&mut Value)) {
    let key = tacsim::port::port_map_key(&lane_id(), &cid("ceres_port"));
    let mut map = stored(rig, &key).expect("the port map is stored");
    edit(&mut map);
    damage(rig, &key, map);
}

// ---------------------------------------------------------------------------
// The mill's own reads
// ---------------------------------------------------------------------------

fn mill_handle(tw: &TestWorld) -> StateFamilyHandle<MillId, MillLedger> {
    tw.world()
        .module_state::<tacsim::port::PortHandles>()
        .expect("the port handles")
        .families
        .mills
        .clone()
}

fn mill_id(org: &str, ore: &str) -> MillId {
    MillId::derive(&lane_id(), &cid("ceres_port"), &cid(org), &cid(ore)).expect("a mill id")
}

/// The ledger of `org`'s mill of `ore`, as the engine holds it.
fn ledger(tw: &TestWorld, org: &str, ore: &str) -> Option<MillLedger> {
    let handle = mill_handle(tw);
    tw.engine()
        .action_state()
        .get(&handle, &mill_id(org, ore))
        .ok()
        .flatten()
        .cloned()
}

/// The rig's kept typed record of `org`'s mill of `ore` — its `(ns, key)`.
fn ledger_slot(rig: &Rig, org: &str, ore: &str) -> (SmolStr, SmolStr) {
    let text = mill_id(org, ore).key_text();
    rig.typed
        .iter()
        .find(|(_, value)| {
            value["family"]["id"] == tacsim::port::mill::MILL_FAMILY
                && value["key"].as_str() == Some(text.as_str())
        })
        .map(|(slot, _)| slot.clone())
        .expect("the ledger's typed record is kept")
}

/// `org`'s storefront vault.
fn storefront(tw: &TestWorld, org: &str) -> VaultId {
    VaultId(port_map(tw, "ceres_port").vendors[&cid(org)].vault)
}

/// The live drawn ask on `org`'s storefront pool of `good`.
fn ask(tw: &TestWorld, org: &str, good: &str) -> Option<Listing> {
    let market = market(tw);
    market
        .lot_listing_of(EndpointRef(storefront(tw, org)), &cid(good))
        .and_then(|id| market.listing(id).cloned())
}

/// `org`'s `mill_listed` counter for its mill of `ore`.
fn counter(tw: &TestWorld, org: &str, ore: &str) -> Option<u64> {
    port_map(tw, "ceres_port").vendors[&cid(org)]
        .mill_listed
        .get(&cid(ore))
        .copied()
}

/// The Ceres stock clerk.
fn clerk(tw: &TestWorld) -> ObjId {
    ObjId(
        port_map(tw, "ceres_port")
            .clerk
            .expect("the clerk is made")
            .thing,
    )
}

/// The `@tacsim/port status` line that starts with `head` (a per-port line, indented).
fn line(tw: &mut TestWorld, head: &str) -> String {
    let report = status(tw);
    report
        .lines()
        .find(|l| l.starts_with(&format!("  {head}")))
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("no line `{head}`:\n{report}"))
}

fn mill_line(tw: &mut TestWorld, org: &str, ore: &str) -> String {
    line(tw, &format!("ceres_port mill {org} {ore}:"))
}

fn ask_line(tw: &mut TestWorld, org: &str, good: &str) -> String {
    line(tw, &format!("ceres_port ask {org} {good}:"))
}

/// A whole line's state word, parsed from its suffix.
fn state_word(line: &str) -> &str {
    line.rsplit_once(" [")
        .map(|(_, state)| state.trim_end_matches(']'))
        .unwrap_or_default()
}

/// Whether a line of `binding` in this state shuts the desk (`coop::shuts`' rule: a gated line
/// shuts in any state but Ready; the Bids line on Missing alone).
fn shuts(binding: Binding, line: &str) -> bool {
    let word = state_word(line);
    gated(binding)
        && match binding {
            Binding::Bids => word == "missing",
            _ => word != "ready",
        }
}

/// A fresh pilot at the Registry desk runs `+coop/enrol`: `Ok` with the charter, `Err` with the
/// refusal and the gate's captured INFO line.
fn enrol(rig: &mut Rig, who: &str, seed: i64) -> Result<String, (String, String)> {
    let pilot = pilot(rig, who, seed);
    let capture = Capture::default();
    let dispatch = capture_dispatch(capture.clone(), tracing::Level::INFO);
    let out = tracing::dispatcher::with_default(&dispatch, || {
        run(&mut rig.tw, pilot, &format!("+coop/enrol {who} Ship"))
    });
    if out.contains(&format!("now the {who} Ship")) {
        Ok(out)
    } else {
        Err((out, capture.text()))
    }
}

// ---------------------------------------------------------------------------
// 1. The lane
// ---------------------------------------------------------------------------

/// **`strip_mill` rebuilds the published v13** (F9): the shipped lane is v15 and carries
/// exactly two `mill` rows — the Exchange's and Occator's, with B2's figures — read from the
/// shipped pack; the shipped lane stripped (yard and mills: doc 20 cut SM.12b's `strip_yard` runs
/// first inside `strip_mill`) and re-signed at 13 is relay R9b's published v13, digest for
/// digest; and `strip_mill` removes exactly those two rows and the yard, idempotently.
#[test]
fn sm12a_strip_mill_rebuilds_the_published_v13() {
    assert_eq!(LANE_VERSION, 15, "lane v15 since doc 20 cut SM.12b");
    let shipped = load_pack(LANE_JSON).expect("the lane verifies");
    assert_eq!(shipped.manifest().version(), LANE_VERSION);
    let mut mills = Vec::new();
    for (id, entry) in shipped.entries() {
        let PackEntry::Port(def) = entry else {
            continue;
        };
        for vendor in &def.vendors {
            for mill in &vendor.mill {
                mills.push((
                    id.to_string(),
                    vendor.org.to_string(),
                    mill.ore.to_string(),
                    mill.into.to_string(),
                    (mill.ratio.ore, mill.ratio.product),
                    mill.batch_t,
                    mill.per_ic_day_t,
                ));
            }
        }
    }
    let row = |org: &str, ore: &str, into: &str| {
        (
            "ceres_port".to_owned(),
            org.to_owned(),
            ore.to_owned(),
            into.to_owned(),
            (5, 2),
            20,
            40,
        )
    };
    assert_eq!(
        mills,
        vec![
            row(EXCHANGE, "metal_ore", "metals"),
            row(OCCATOR, "ice_ore", "volatiles"),
        ],
        "the shipped lane's two mills, 5 : 2, 20 t a batch, at most 40 t an IC day"
    );
    let pack = load_pack(&lane_variant(13, strip_mill)).expect("the v13 shape verifies");
    assert_eq!(
        pack.manifest().digest().to_hex(),
        LANE_V13_DIGEST,
        "the shipped lane less its mills is the published v13"
    );
    let mut doc: Value = serde_json::from_str(LANE_JSON).expect("JSON");
    let vendors = |doc: &Value| -> Vec<bool> {
        doc["entries"]["ceres_port"]["def"]["vendors"]
            .as_array()
            .expect("vendors")
            .iter()
            .map(|v| v.get("mill").is_some())
            .collect()
    };
    assert_eq!(vendors(&doc), vec![true, true, false]);
    strip_mill(&mut doc);
    assert_eq!(vendors(&doc), vec![false, false, false]);
    // Doc 20 cut SM.12b: the yard goes with the mills (a yard names a mill row).
    assert!(
        doc["entries"]["occator_port"]["def"]["site"]
            .get("yard")
            .is_none()
    );
    let once = doc.clone();
    strip_mill(&mut doc);
    assert_eq!(doc, once, "idempotent");
}

// ---------------------------------------------------------------------------
// 2–5. The batch, the remainder, the rate, the cadence
// ---------------------------------------------------------------------------

/// **A batch mills ore into the ask and writes its ledger** (§7.3 "The mill"; C9): 20 t of
/// metal ore sold, one batch (ordinal 1) lands — the storefront's ore 0, its concentrate +8,
/// the ledger at one batch — and before its restock the `metals` Listings line names the 8 t
/// milled and not yet listed, whole; the next fire restocks by exactly 8 with the revision
/// bumped and the counter at 8, and the Mill line reads the ledger. Then with a Draw accepted
/// and not yet drawn, a second batch's line names both the hand-over and the milled tonnes.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12a_a_batch_mills_ore_into_the_ask_and_writes_its_ledger() {
    let mut rig = stocked();
    let (miner, hold) = dray_p(&mut rig, "Miner", 1, "Salt Moth");
    let vault = storefront(&rig.tw, EXCHANGE);
    let before = ask(&rig.tw, EXCHANGE, "metals").expect("the seed listing");
    assert_eq!(before.quantity, 4_000);
    let contract = seed_and_sell(&mut rig, miner, hold, EXCHANGE, "metal_ore", 20, 901);
    let batch = sale_batch(&mut rig, contract);
    assert_eq!(batch.actor, clerk(&rig.tw), "the clerk mills");
    assert_eq!(batch.parameters["ordinal"], 1);
    assert_eq!(pool(&mut rig.tw, vault, "metal_ore"), 0);
    assert_eq!(pool(&mut rig.tw, vault, "metals"), 4_008);
    let milled = ledger(&rig.tw, EXCHANGE, "metal_ore").expect("the ledger");
    assert_eq!(
        (
            milled.batches,
            milled.ore_t,
            milled.product_t,
            milled.recent.len()
        ),
        (1, 20, 8, 1)
    );
    let id = before.id;
    assert_eq!(
        ask_line(&mut rig.tw, EXCHANGE, "metals"),
        format!(
            "  ceres_port ask freeport_exchange metals: listing #{id} at 200.00cr/t, 4000 t \
             listed; the storefront holds 4008 t, 8 t milled and not yet listed. [ready]"
        )
    );
    // The next fire restocks.
    let queued = fire_landing(&mut rig);
    assert!(queued.is_empty(), "a restock submits nothing");
    let after = ask(&rig.tw, EXCHANGE, "metals").expect("the listing");
    assert_eq!(after.id, id);
    assert_eq!(after.quantity, 4_008);
    assert_eq!(after.revision, before.revision + 1);
    assert_eq!(counter(&rig.tw, EXCHANGE, "metal_ore"), Some(8));
    assert!(tacsim::port_tick::drift_pages(rig.tw.world()).is_empty());
    let stamp = ic_stamp(milled.recent[0].at_ic);
    assert_eq!(
        mill_line(&mut rig.tw, EXCHANGE, "metal_ore"),
        format!(
            "  ceres_port mill freeport_exchange metal_ore: 1 batch has milled 20 t of ore into 8 t \
             of metals, the last at {stamp}; 0 t of ore waits; 5 to 2, 20 t a batch, at most 40 t \
             an IC day. [ready]"
        )
    );
    settle(&mut rig, contract);

    // The hand-over variant: a Draw of 10 t accepted and kept aside, then a second batch.
    top_up(&mut rig.tw, miner, 300_000);
    let (accepted, quote) = buy(&mut rig, miner, ListingId(id.0), 10);
    assert!(accepted.contains("Contract #"), "{accepted}");
    let draw = drain(&mut rig.tw);
    assert_eq!(draw.len(), 1, "{accepted}");
    assert_eq!(quantity(&rig.tw, id.0), Some(3_998));
    let second = seed_and_sell(&mut rig, miner, hold, EXCHANGE, "metal_ore", 20, 902);
    let batch = sale_batch(&mut rig, second);
    assert_eq!(batch.parameters["ordinal"], 2);
    assert_eq!(
        ask_line(&mut rig.tw, EXCHANGE, "metals"),
        format!(
            "  ceres_port ask freeport_exchange metals: listing #{id} at 200.00cr/t, 3998 t \
             listed; the storefront holds 4016 t, 10 t in hand-over, 8 t milled and not yet \
             listed. [ready]"
        )
    );
    land_all(&mut rig, &draw);
    let purchase = contract_of(&rig.tw, quote);
    settle(&mut rig, purchase);
    settle(&mut rig, second);
    fire_until_idle(&mut rig, 24);
    assert_eq!(quantity(&rig.tw, id.0), Some(4_006), "3998 + 8 restocked");
    assert_eq!(counter(&rig.tw, EXCHANGE, "metal_ore"), Some(16));
    assert!(tacsim::port_tick::drift_pages(rig.tw.world()).is_empty());
    no_refused_writes(&rig.tw);
}

/// **Less than a batch waits, and the remainder is kept** (§7.3): 19 t sold mills nothing
/// however many fires ("19 t of ore waits"); one tonne more makes a batch; 27 t more make one
/// batch and leave 7 t waiting, which the Mill line says.
#[test]
fn sm12a_less_than_a_batch_waits_and_the_remainder_is_kept() {
    let mut rig = stocked();
    past_next_boundary(&mut rig, 60);
    let (miner, hold) = dray_p(&mut rig, "Miner", 1, "Salt Moth");
    let contract = seed_and_sell(&mut rig, miner, hold, EXCHANGE, "metal_ore", 19, 901);
    settle(&mut rig, contract);
    for _ in 0..12 {
        let queued = fire_landing(&mut rig);
        assert!(!queued.iter().any(is_mill), "19 t is less than a batch");
    }
    assert!(ledger(&rig.tw, EXCHANGE, "metal_ore").is_none());
    assert_eq!(
        mill_line(&mut rig.tw, EXCHANGE, "metal_ore"),
        "  ceres_port mill freeport_exchange metal_ore: nothing milled yet; 19 t of ore waits; 5 \
         to 2, 20 t a batch, at most 40 t an IC day. [ready]"
    );
    let contract = seed_and_sell(&mut rig, miner, hold, EXCHANGE, "metal_ore", 1, 902);
    let batch = sale_batch(&mut rig, contract);
    assert_eq!(batch.parameters["ordinal"], 1);
    settle(&mut rig, contract);
    fire_until_idle(&mut rig, 24);
    let contract = seed_and_sell(&mut rig, miner, hold, EXCHANGE, "metal_ore", 27, 903);
    let batch = sale_batch(&mut rig, contract);
    assert_eq!(batch.parameters["ordinal"], 2);
    settle(&mut rig, contract);
    fire_until_idle(&mut rig, 24);
    let milled = ledger(&rig.tw, EXCHANGE, "metal_ore").expect("the ledger");
    assert_eq!(
        (milled.batches, milled.ore_t, milled.product_t),
        (2, 40, 16)
    );
    let vault = storefront(&rig.tw, EXCHANGE);
    assert_eq!(pool(&mut rig.tw, vault, "metal_ore"), 7);
    let stamp = ic_stamp(milled.recent[1].at_ic);
    assert_eq!(
        mill_line(&mut rig.tw, EXCHANGE, "metal_ore"),
        format!(
            "  ceres_port mill freeport_exchange metal_ore: 2 batches have milled 40 t of ore into \
             16 t of metals, the last at {stamp}; 7 t of ore waits; 5 to 2, 20 t a batch, at most \
             40 t an IC day. [ready]"
        )
    );
    no_refused_writes(&rig.tw);
}

/// **The daily rate binds, and the next IC day resumes** (§7.3; the review's B12): the IC time
/// of day anchored first; 86 t sold make two batches that IC day (40 t) and no more however
/// many fires; past the next boundary two more; 6 t remain.
#[test]
fn sm12a_the_daily_rate_binds_and_the_next_ic_day_resumes() {
    let mut rig = stocked();
    past_next_boundary(&mut rig, 60);
    let (miner, hold) = dray_p(&mut rig, "Miner", 1, "Salt Moth");
    let contract = seed_and_sell(&mut rig, miner, hold, EXCHANGE, "metal_ore", 86, 901);
    settle(&mut rig, contract);
    for _ in 0..30 {
        fire_landing(&mut rig);
    }
    let day_one = ledger(&rig.tw, EXCHANGE, "metal_ore").expect("the ledger");
    assert_eq!((day_one.batches, day_one.day_ore_t), (2, 40));
    let vault = storefront(&rig.tw, EXCHANGE);
    assert_eq!(pool(&mut rig.tw, vault, "metal_ore"), 46);
    past_next_boundary(&mut rig, 60);
    for _ in 0..30 {
        fire_landing(&mut rig);
    }
    let day_two = ledger(&rig.tw, EXCHANGE, "metal_ore").expect("the ledger");
    assert_eq!(
        (day_two.batches, day_two.ore_t, day_two.day_ore_t),
        (4, 80, 40)
    );
    assert_eq!(day_two.day, day_one.day + 1);
    assert_eq!(pool(&mut rig.tw, vault, "metal_ore"), 6);
    assert!(tacsim::port::mill::conserves(
        day_two.ore_t,
        day_two.product_t,
        day_two.ratio
    ));
    no_refused_writes(&rig.tw);
}

/// One fire with the one-act check (12a-M11): at most one queued submission by the Ceres
/// clerk, and when there is one, no listing and no `ceres_port` map document written.
fn checked_fire(rig: &mut Rig) -> Vec<AuthoritativeSubmit> {
    let queued = fire(rig);
    one_act(rig, &queued);
    queued
}

fn one_act(rig: &Rig, queued: &[AuthoritativeSubmit]) {
    let clerk = clerk(&rig.tw);
    let by_clerk = queued.iter().filter(|s| s.actor == clerk).count();
    assert!(
        by_clerk <= 1,
        "the clerk made {by_clerk} submissions in one fire"
    );
    if by_clerk == 1 {
        assert!(
            rig.last_keys.is_empty(),
            "the clerk submitted and wrote {:?} in one fire",
            rig.last_keys
        );
    }
}

/// Settle with the one-act check on every sweep.
fn checked_settle(rig: &mut Rig, contract: ContractId) {
    for _ in 0..12 {
        if settled(&rig.tw, contract) {
            return;
        }
        sweep(rig, 31);
        let queued = drain(&mut rig.tw);
        one_act(rig, &queued);
        land_queue(rig, &queued);
    }
    panic!("contract #{contract} did not settle");
}

/// Move the world, without firing, to `secs` real seconds before the next IC-day boundary (at
/// world rate 1).
fn before_next_boundary(rig: &mut Rig, secs: i64) {
    assert_eq!(world_rate(&rig.tw), 1);
    let now = ic_now(&rig.tw);
    let next = (now.div_euclid(DAY) + 1) * DAY;
    let t = rig.tw.world().now().saturating_add(next - now - secs);
    rig.tw.set_now(t);
    assert_eq!(ic_now(&rig.tw), next - secs);
}

/// **The drip acts first, and one act a fire** (D4; 12a-M11): ore waiting and a drip due on the
/// boundary fire — the first fire past the boundary drips (SM.5's timing unmoved) and the next
/// mills. And a batch in flight at the boundary: the boundary fire's stock step defers, so it
/// drips nothing; the batch lands, and the next fire drips. Over every fire the Ceres clerk
/// makes at most one act.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12a_the_drip_acts_first_and_one_act_a_fire() {
    let mut rig = stocked();
    past_next_boundary(&mut rig, 60);
    let (miner, hold) = dray_p(&mut rig, "Miner", 1, "Salt Moth");
    // 60 t sold: two batches today, 20 t wait for tomorrow's room; the bid's drip is due at
    // the boundary.
    seed_pool(&mut rig, hold, "metal_ore", 60, LotId(901));
    let contract = sell(&mut rig, miner, EXCHANGE, "metal_ore", "60");
    checked_settle(&mut rig, contract);
    for _ in 0..30 {
        let queued = checked_fire(&mut rig);
        land_queue(&mut rig, &queued);
    }
    assert_eq!(
        ledger(&rig.tw, EXCHANGE, "metal_ore").map(|l| l.batches),
        Some(2)
    );
    let row = |tw: &TestWorld| {
        port_map(tw, "ceres_port").vendors[&cid(EXCHANGE)].bids[&cid("metal_ore")].clone()
    };
    before_next_boundary(&mut rig, 3);
    let waiting = row(&rig.tw);
    // The first fire past the boundary drips, and mills nothing.
    let queued = checked_fire(&mut rig);
    assert!(!queued.iter().any(is_mill), "the drip acts first");
    land_queue(&mut rig, &queued);
    let dripped = row(&rig.tw);
    assert_ne!(
        dripped.listing, waiting.listing,
        "the drip re-posted the bid"
    );
    assert_eq!(
        dripped.posted_ic.map(|p| p.div_euclid(DAY)),
        Some(ic_now(&rig.tw).div_euclid(DAY))
    );
    // The next fire mills.
    let queued = checked_fire(&mut rig);
    assert!(queued.iter().any(is_mill), "the next fire mills");
    land_queue(&mut rig, &queued);
    assert_eq!(
        ledger(&rig.tw, EXCHANGE, "metal_ore").map(|l| (l.batches, l.day_ore_t)),
        Some((3, 20))
    );
    for _ in 0..6 {
        let queued = checked_fire(&mut rig);
        land_queue(&mut rig, &queued);
    }

    // A batch in flight at the boundary: 20 t sold late in the day; the last fire before the
    // boundary submits the batch and is not drained.
    before_next_boundary(&mut rig, 12);
    seed_pool(&mut rig, hold, "metal_ore", 20, LotId(902));
    let late = sell(&mut rig, miner, EXCHANGE, "metal_ore", "20");
    fold(&mut rig);
    let t = rig.tw.world().now().saturating_add(6);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    rig.last_keys = fold(&mut rig);
    assert!(
        rig.tw.engine().has_authoritative_submit_for(clerk(&rig.tw)),
        "the batch is in flight"
    );
    let before = row(&rig.tw);
    // The boundary fire: the stock step defers, so nothing drips.
    let queued = checked_fire(&mut rig);
    assert!(
        ic_now(&rig.tw).div_euclid(DAY) > before.posted_ic.unwrap_or_default().div_euclid(DAY),
        "past the boundary"
    );
    assert_eq!(row(&rig.tw), before, "the boundary fire drips nothing");
    assert_eq!(
        queued.iter().filter(|s| is_mill(s)).count(),
        1,
        "the batch in flight"
    );
    land_queue(&mut rig, &queued);
    // The next fire drips.
    let queued = checked_fire(&mut rig);
    land_queue(&mut rig, &queued);
    assert_ne!(
        row(&rig.tw).listing,
        before.listing,
        "the drip, one fire late"
    );
    checked_settle(&mut rig, late);
    for _ in 0..6 {
        let queued = checked_fire(&mut rig);
        land_queue(&mut rig, &queued);
    }
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 6–8. The replay guard, the restart, the first listing
// ---------------------------------------------------------------------------

/// **A replayed batch is refused and moves nothing** (D3): ordinal 1's kept submission landed
/// again is refused "that batch is already milled" and ordinal 2 with no ore waiting "nothing
/// is due at that mill" (both at the prepare); a submission naming a mill that is not among the
/// clerk's rows is refused at the resolve, and one made as another port's clerk naming the
/// Exchange's mill is never offered. Each leaves both ledgers and the storefront's pools as
/// they were, read from the engine.
#[test]
fn sm12a_a_replayed_batch_is_refused_and_moves_nothing() {
    let mut rig = stocked();
    let (miner, hold) = dray_p(&mut rig, "Miner", 1, "Salt Moth");
    let contract = seed_and_sell(&mut rig, miner, hold, EXCHANGE, "metal_ore", 20, 901);
    let batch = sale_batch(&mut rig, contract);
    settle(&mut rig, contract);
    let vault = storefront(&rig.tw, EXCHANGE);
    // What a refused landing must leave as it was, read from the engine (a refused prepare
    // stages nothing, so the rig's own kept typed map could not show a write): both mills'
    // ledgers and the Exchange storefront's ore and product pools.
    let milled = ledger(&rig.tw, EXCHANGE, "metal_ore");
    assert!(milled.is_some());
    let icy = ledger(&rig.tw, OCCATOR, "ice_ore");
    let pools = |tw: &mut TestWorld| (pool(tw, vault, "metal_ore"), pool(tw, vault, "metals"));
    let before = pools(&mut rig.tw);
    assert_eq!(before.0, 0, "the batch milled every tonne");
    let unchanged = |rig: &mut Rig| {
        assert_eq!(ledger(&rig.tw, EXCHANGE, "metal_ore"), milled);
        assert_eq!(ledger(&rig.tw, OCCATOR, "ice_ore"), icy);
        assert_eq!(pools(&mut rig.tw), before);
    };
    // The replay: refused at the prepare by the ordinal guard.
    let refused = land(&mut rig, &batch).expect_err("a replay is refused");
    assert!(
        refused.contains("that batch is already milled"),
        "{refused}"
    );
    unchanged(&mut rig);
    // The ledger's next ordinal with no ore waiting: refused at the prepare, nothing due.
    let mut idle = batch.clone();
    idle.parameters["ordinal"] = serde_json::json!(2);
    let refused = land(&mut rig, &idle).expect_err("nothing is due");
    assert!(refused.contains("nothing is due at that mill"), "{refused}");
    unchanged(&mut rig);
    // A mill key that is not among the clerk's rows: refused at the resolve (the offer's
    // `mill` parameter is a choice of the clerk's own mills).
    let mut stranger = batch.clone();
    stranger.parameters["mill"] = serde_json::json!("0".repeat(64));
    stranger.parameters["ordinal"] = serde_json::json!(2);
    let refused = land(&mut rig, &stranger).expect_err("an unknown mill is refused");
    assert_eq!(
        refused, "Resolve(InvalidParameters)",
        "refused at the resolve"
    );
    unchanged(&mut rig);
    // Another port's clerk naming the Exchange's mill: Stickney's clerk has no mill, so the
    // action is not offered to it at all.
    let mut elsewhere = batch.clone();
    elsewhere.actor = ObjId(
        port_map(&rig.tw, "stickney_port")
            .clerk
            .expect("Stickney's clerk")
            .thing,
    );
    elsewhere.parameters["ordinal"] = serde_json::json!(2);
    let refused = land(&mut rig, &elsewhere).expect_err("another port's clerk is refused");
    assert_eq!(refused, "that action is not currently offered");
    unchanged(&mut rig);
    no_refused_writes(&rig.tw);
}

/// **A restart between the batch and its restock restocks once** (F6; the restart argument):
/// the world reboots right after the batch lands and before any further fire or sweep; then
/// exactly one restock of 8 (the counter equal to the ledger's total), and over 24 more fires
/// neither the listing, the counter nor the ledger moves again.
#[test]
fn sm12a_a_restart_between_the_batch_and_its_restock_restocks_once() {
    let mut rig = stocked();
    let (miner, hold) = dray_p(&mut rig, "Miner", 1, "Salt Moth");
    let contract = seed_and_sell(&mut rig, miner, hold, EXCHANGE, "metal_ore", 20, 901);
    let batch = settle_until(&mut rig, contract, true).expect("the batch lands in the settle");
    assert!(is_mill(&batch));
    let listed = ask(&rig.tw, EXCHANGE, "metals")
        .expect("the listing")
        .quantity;
    reboot(&mut rig);
    set_rate(&mut rig.tw, 1);
    assert_eq!(world_rate(&rig.tw), 1);
    assert_eq!(
        counter(&rig.tw, EXCHANGE, "metal_ore"),
        None,
        "not yet listed"
    );
    settle(&mut rig, contract);
    fire_until_idle(&mut rig, 24);
    let milled = ledger(&rig.tw, EXCHANGE, "metal_ore").expect("the ledger");
    let after = ask(&rig.tw, EXCHANGE, "metals").expect("the listing");
    assert_eq!(after.quantity, listed + 8, "exactly one restock of 8");
    assert_eq!(
        counter(&rig.tw, EXCHANGE, "metal_ore"),
        Some(milled.product_t)
    );
    assert_eq!(milled.product_t, 8);
    for _ in 0..24 {
        fire_landing(&mut rig);
    }
    assert_eq!(
        ask(&rig.tw, EXCHANGE, "metals").map(|l| l.quantity),
        Some(after.quantity)
    );
    assert_eq!(counter(&rig.tw, EXCHANGE, "metal_ore"), Some(8));
    assert_eq!(ledger(&rig.tw, EXCHANGE, "metal_ore"), Some(milled));
    no_refused_writes(&rig.tw);
}

/// **A first listing writes no counter** (F1; 12a-T1; N6): two worlds booted through one
/// `Roster` path at the shipped version — the shipped content with its mills, and the same
/// content with the mills stripped — stock identically: neither stored `ceres_port` map has a
/// `mill_listed` key, the map is written the same number of times, and the two documents are
/// equal.
#[test]
fn sm12a_a_first_listing_writes_no_counter() {
    // Both seeded, so both boot the same historical relay seed (`boot_packs_with_lane`).
    let mut a = stocked_with(Roster::seeded(lane_variant(LANE_VERSION, |_| {})));
    let mut b = stocked_with(Roster::seeded(lane_variant(LANE_VERSION, strip_mill)));
    let key = tacsim::port::port_map_key(&lane_id(), &cid("ceres_port"));
    let doc_a = stored(&mut a, &key).expect("A's map");
    let doc_b = stored(&mut b, &key).expect("B's map");
    for (doc, rig) in [(&doc_a, "A"), (&doc_b, "B")] {
        for (org, row) in doc["vendors"].as_object().expect("vendor rows") {
            assert!(
                row.get("mill_listed").is_none(),
                "{rig}: {org}'s row carries no counter"
            );
        }
    }
    assert!(a.map_sets > 0);
    assert_eq!(
        a.map_sets, b.map_sets,
        "the map is written as often in both worlds"
    );
    assert_eq!(doc_a, doc_b, "the two maps are equal");
    no_refused_writes(&a.tw);
    no_refused_writes(&b.tw);
}

// ---------------------------------------------------------------------------
// 9–12. Faults: the terms, the ledger, the sell-out, a removed row
// ---------------------------------------------------------------------------

/// The Exchange mill's batch and restock on a fresh rig; the batch.
fn one_exchange_batch(rig: &mut Rig, miner: ObjId, hold: VaultId) -> AuthoritativeSubmit {
    let contract = seed_and_sell(rig, miner, hold, EXCHANGE, "metal_ore", 20, 901);
    let batch = sale_batch(rig, contract);
    settle(rig, contract);
    fire_until_idle(rig, 24);
    assert_eq!(counter(&rig.tw, EXCHANGE, "metal_ore"), Some(8));
    batch
}

/// Occator's ice ore sold mills and restocks: its ledger advances and its `volatiles` ask
/// rises by what it made.
fn occator_mills(rig: &mut Rig, icer: ObjId, hold: VaultId, lot: u64) {
    let before = ledger(&rig.tw, OCCATOR, "ice_ore").map_or(0, |l| l.product_t);
    let listed = ask(&rig.tw, OCCATOR, "volatiles")
        .expect("Occator's ask")
        .quantity;
    let contract = seed_and_sell(rig, icer, hold, OCCATOR, "ice_ore", 20, lot);
    settle(rig, contract);
    fire_until_idle(rig, 24);
    let after = ledger(&rig.tw, OCCATOR, "ice_ore").expect("Occator milled");
    assert_eq!(after.product_t, before + 8, "Occator milled one batch");
    assert_eq!(
        ask(&rig.tw, OCCATOR, "volatiles").map(|l| l.quantity),
        Some(listed + 8),
        "and restocked it"
    );
}

/// **A changed ratio or product stops that mill and nothing else** (F2, D9, O-5's default):
/// (i) a later version whose Exchange mill reads 5 : 1, shelved and loaded, applied after one
/// batch: no Exchange batch lands (a hand-built ordinal 2 is refused on its terms), the Mill
/// line reads Flagged, the ledger is unchanged, and Occator mills; (ii) a ledger whose product
/// no longer matches, its counter set back to 0 so that only the ledger's `into` keeps it out
/// of `metals`: the Mill line reads so, the `metals` ask reads none unlisted (Ready, no
/// drift), and Occator mills.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12a_a_changed_ratio_or_product_stops_that_mill_and_nothing_else() {
    // (i) The ratio.
    let next = lane_variant(LANE_VERSION + 1, |doc| {
        doc["entries"]["ceres_port"]["def"]["vendors"][0]["mill"][0]["ratio"]["product"] =
            serde_json::json!(1);
    });
    let capture = Capture::default();
    let dispatch = capture_dispatch(capture.clone(), tracing::Level::WARN);
    tracing::dispatcher::with_default(&dispatch, || {
        let mut rig = stocked_with(Roster::next(&next));
        let (miner, hold) = dray_p(&mut rig, "Miner", 1, "Salt Moth");
        let (icer, ice_hold) = dray_p(&mut rig, "Icer", 2, "Rime Moth");
        let batch = one_exchange_batch(&mut rig, miner, hold);
        let milled = ledger(&rig.tw, EXCHANGE, "metal_ore").expect("the ledger");
        let (plan, apply) = plan_and_apply(&mut rig.tw);
        assert!(
            apply.contains(&format!("Applied {LANE_PACK}@{}", LANE_VERSION + 1)),
            "{plan}\n{apply}"
        );
        let report = status(&mut rig.tw);
        let lane_line = format!(
            "lane: {LANE_PACK}@{} applied; this server ships @{LANE_VERSION}. [flagged]",
            LANE_VERSION + 1
        );
        assert!(report.lines().any(|l| l == lane_line), "{report}");
        let contract = seed_and_sell(&mut rig, miner, hold, EXCHANGE, "metal_ore", 20, 902);
        settle(&mut rig, contract);
        for _ in 0..12 {
            let queued = fire_landing(&mut rig);
            assert!(!queued.iter().any(is_mill), "no Exchange batch lands");
        }
        let mut forged = batch.clone();
        forged.parameters["ordinal"] = serde_json::json!(2);
        let refused = land(&mut rig, &forged).expect_err("the ratio is refused at the prepare");
        assert!(
            refused.contains("the mill's ratio or product no longer matches its ledger"),
            "{refused}"
        );
        assert_eq!(
            mill_line(&mut rig.tw, EXCHANGE, "metal_ore"),
            "  ceres_port mill freeport_exchange metal_ore: its ratio no longer matches its ledger. \
             [flagged]"
        );
        let kept = ledger(&rig.tw, EXCHANGE, "metal_ore").expect("the ledger");
        assert_eq!(kept, milled, "the ledger is unchanged");
        assert!(tacsim::port::mill::conserves(
            kept.ore_t,
            kept.product_t,
            kept.ratio
        ));
        occator_mills(&mut rig, icer, ice_hold, 903);
        no_refused_writes(&rig.tw);
    });
    let warned = capture.text();
    assert_eq!(
        warned
            .matches("a mill's authored ratio or product no longer matches its ledger")
            .count(),
        1,
        "{warned}"
    );

    // (ii) The product.
    let mut rig = stocked();
    let (miner, hold) = dray_p(&mut rig, "Miner", 1, "Salt Moth");
    let (icer, ice_hold) = dray_p(&mut rig, "Icer", 2, "Rime Moth");
    one_exchange_batch(&mut rig, miner, hold);
    let slot = ledger_slot(&rig, EXCHANGE, "metal_ore");
    rig.typed
        .get_mut(&slot)
        .expect("the record")
        .get_mut("value")
        .expect("its value")["into"] = serde_json::json!("volatiles");
    // The counter set back to 0 as well, so the readings part: summed by the ledger's
    // `into` (D9) the mill now owes `volatiles`, which the Exchange does not ask, and
    // `metals` reads none unlisted; summed by the row's authored `into`, or by leftover
    // `mill_listed` keys (12a-C6), `metals` would read 8 unlisted against a pool its
    // listing already carries in full, and page drift and read Flagged.
    damage_map(&mut rig, |map| {
        map["vendors"][EXCHANGE]["mill_listed"]["metal_ore"] = serde_json::json!(0);
    });
    reboot(&mut rig);
    set_rate(&mut rig.tw, 1);
    let vault = storefront(&rig.tw, EXCHANGE);
    assert_eq!(
        ask(&rig.tw, EXCHANGE, "metals").map(|l| l.quantity),
        Some(pool(&mut rig.tw, vault, "metals")),
        "the listing carries the whole pool"
    );
    assert_eq!(
        mill_line(&mut rig.tw, EXCHANGE, "metal_ore"),
        "  ceres_port mill freeport_exchange metal_ore: its product no longer matches its ledger. \
         [flagged]"
    );
    let contract = seed_and_sell(&mut rig, miner, hold, EXCHANGE, "metal_ore", 20, 902);
    settle(&mut rig, contract);
    for _ in 0..12 {
        let queued = fire_landing(&mut rig);
        assert!(!queued.iter().any(is_mill), "no Exchange batch lands");
    }
    let metals = ask_line(&mut rig.tw, EXCHANGE, "metals");
    assert_eq!(state_word(&metals), "ready", "{metals}");
    assert!(
        !metals.contains("milled"),
        "the counter dropped out: {metals}"
    );
    assert!(tacsim::port_tick::drift_pages(rig.tw.world()).is_empty());
    occator_mills(&mut rig, icer, ice_hold, 903);
    no_refused_writes(&rig.tw);
}

/// A valid ledger that names Occator's mill, for storing under the Exchange mill's key (D19).
fn occator_ledger(at: i64, vault: VaultId, clerk: ObjId) -> Value {
    serde_json::to_value(MillLedger {
        pack: lane_id(),
        entry: cid("ceres_port"),
        org: cid(OCCATOR),
        ore: cid("ice_ore"),
        into: cid("volatiles"),
        ratio: tacsim::port::MillRatio { ore: 5, product: 2 },
        vault,
        clerk,
        batches: 1,
        ore_t: 20,
        product_t: 8,
        day: tacsim::port::mill::ic_day(at),
        day_ore_t: 20,
        recent: vec![tacsim::port::mill::MillBatch {
            ordinal: 1,
            at_ic: at,
            ore_t: 20,
            product_t: 8,
        }],
    })
    .expect("a ledger serializes")
}

/// Damage the Exchange mill's ledger through the key rule: store a valid ledger naming another
/// mill under its key (D19).
fn damage_exchange_ledger(rig: &mut Rig) {
    let slot = ledger_slot(rig, EXCHANGE, "metal_ore");
    let other = occator_ledger(
        ic_now(&rig.tw),
        storefront(&rig.tw, OCCATOR),
        clerk(&rig.tw),
    );
    rig.typed
        .get_mut(&slot)
        .expect("the record")
        .as_object_mut()
        .expect("an envelope")
        .insert("value".to_owned(), other);
}

/// The Listings lines of the whole report but `except`.
fn ask_lines_but(tw: &mut TestWorld, except: &str) -> Vec<String> {
    status(tw)
        .lines()
        .filter(|l| {
            l.contains(" ask ") && l.starts_with("  ") && !l.starts_with(&format!("  {except}"))
        })
        .map(str::to_owned)
        .collect()
}

/// **An unreadable mill ledger stalls nothing else** (F3, D8, D15; the review's B8): after one
/// Exchange batch and its restock the ledger is damaged by the key rule and the world reboots.
/// The prepare refuses the mill's next batch on the key rule; its Mill line and its `metals`
/// Listings line read Flagged, every other Listings line reads as it did, and a fresh pilot's
/// `+coop/enrol` is shut by that Listings line; then Occator's
/// ice ore mills and restocks, and Occator's drip posts at the next boundary. The Ceres clerk
/// is never the port tick's refused agent.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12a_an_unreadable_mill_ledger_stalls_nothing_else() {
    let capture = Capture::default();
    let dispatch = capture_dispatch(capture.clone(), tracing::Level::WARN);
    let clerk_id = tracing::dispatcher::with_default(&dispatch, || {
        let mut rig = stocked();
        let (miner, hold) = dray_p(&mut rig, "Miner", 1, "Salt Moth");
        let (icer, ice_hold) = dray_p(&mut rig, "Icer", 2, "Rime Moth");
        let batch = one_exchange_batch(&mut rig, miner, hold);
        let head = "ceres_port ask freeport_exchange metals:";
        let others = ask_lines_but(&mut rig.tw, head);
        assert!(others.len() >= 4, "{others:?}");
        damage_exchange_ledger(&mut rig);
        reboot(&mut rig);
        set_rate(&mut rig.tw, 1);
        // The prepare fails closed on the key rule too: the ledger's next ordinal, built from
        // the kept batch, is refused and stages nothing.
        let damaged = ledger(&rig.tw, EXCHANGE, "metal_ore");
        let mut forged = batch.clone();
        forged.parameters["ordinal"] = serde_json::json!(2);
        let refused = land(&mut rig, &forged).expect_err("the key rule refuses");
        assert!(
            refused.contains("a mill ledger is stored under another mill's key"),
            "{refused}"
        );
        assert_eq!(ledger(&rig.tw, EXCHANGE, "metal_ore"), damaged);
        assert_eq!(
            mill_line(&mut rig.tw, EXCHANGE, "metal_ore"),
            "  ceres_port mill freeport_exchange metal_ore: its ledger cannot be read. [flagged]"
        );
        let metals = ask_line(&mut rig.tw, EXCHANGE, "metals");
        assert_eq!(
            metals,
            "  ceres_port ask freeport_exchange metals: its mill ledger cannot be read. [flagged]"
        );
        assert!(shuts(Binding::Listings, &metals));
        assert_eq!(
            ask_lines_but(&mut rig.tw, head),
            others,
            "every other ask unchanged"
        );
        let (refused, info) = enrol(&mut rig, "Newcomer", 9).expect_err("the desk is shut");
        assert!(refused.contains(tacsim::port::coop::NOT_OPEN), "{refused}");
        assert!(
            info.contains("closed by the readiness gate")
                && info.contains(
                    "ceres_port ask freeport_exchange metals: its mill ledger cannot be read."
                ),
            "{info}"
        );
        // Occator mills and restocks; its drip posts at the next boundary.
        occator_mills(&mut rig, icer, ice_hold, 902);
        let ice = bid_listing(&rig.tw, OCCATOR, "ice_ore");
        let unfilled = quantity(&rig.tw, ice).expect("the ice bid stands");
        past_next_boundary(&mut rig, 60);
        let mut rose = false;
        for _ in 0..6 {
            fire_landing(&mut rig);
            let now = bid_listing(&rig.tw, OCCATOR, "ice_ore");
            if quantity(&rig.tw, now).is_some_and(|q| q > unfilled) {
                rose = true;
                break;
            }
        }
        assert!(rose, "Occator's drip posted at the boundary");
        no_refused_writes(&rig.tw);
        clerk(&rig.tw)
    });
    let warned = capture.text();
    assert!(
        !warned
            .lines()
            .any(|l| l.contains("a port agent was refused")
                && l.contains(&format!("agent={}", clerk_id.0))),
        "{warned}"
    );
}

/// The `metals` ask of a 30 t Exchange bought to 0 by one Draw accepted and not yet drawn,
/// then 20 t of metal ore sold and milled: the rig, the kept Draw and its quote, and the sale.
fn sold_out_with_a_batch(
    rig: &mut Rig,
) -> (Vec<AuthoritativeSubmit>, mush_market::QuoteId, ContractId) {
    let (buyer, _) = dray_p(rig, "Buyer", 3, "Long Haul");
    top_up(&mut rig.tw, buyer, 1_000_000);
    let listing = ask(&rig.tw, EXCHANGE, "metals").expect("the seed listing");
    assert_eq!(listing.quantity, 30);
    let (accepted, quote) = buy(rig, buyer, listing.id, 30);
    assert!(accepted.contains("Contract #"), "{accepted}");
    let draw = drain(&mut rig.tw);
    assert_eq!(draw.len(), 1, "{accepted}");
    assert!(
        market(&rig.tw).listing(listing.id).is_none(),
        "a sold-out drawn ask is deleted"
    );
    let (miner, hold) = dray_p(rig, "Miner", 1, "Salt Moth");
    let sale = seed_and_sell(rig, miner, hold, EXCHANGE, "metal_ore", 20, 901);
    let batch = sale_batch(rig, sale);
    assert_eq!(batch.parameters["ordinal"], 1);
    (draw, quote, sale)
}

/// The 30 t seed lane (`commerce.rs:1380-1386`'s edit).
fn thirty_tonnes() -> String {
    lane_variant(LANE_VERSION, |doc| {
        doc["entries"]["ceres_port"]["def"]["vendors"][0]["stock"][0]["qty"] =
            serde_json::json!(30);
    })
}

/// **A sold-out ask is listed again with the milled stock** (12a-M4; D11 as amended): bought to
/// 0 with one Draw not yet drawn, a batch reads Ready "SOLD OUT … 8 t milled and not yet
/// listed" and the desk stays open; no restock is made (no listing); once the Draw lands and
/// settles, the list step lists the whole pool and sets the counter to the ledger's total.
#[test]
fn sm12a_a_sold_out_ask_is_listed_again_with_the_milled_stock() {
    let mut rig = stocked_with(Roster::seeded(thirty_tonnes()));
    let (draw, quote, sale) = sold_out_with_a_batch(&mut rig);
    assert_eq!(
        ask_line(&mut rig.tw, EXCHANGE, "metals"),
        "  ceres_port ask freeport_exchange metals: SOLD OUT: seeded 30 t, 0 left, 8 t milled and \
         not yet listed. [ready]"
    );
    enrol(&mut rig, "Newcomer", 9).expect("the desk stays open across the gap");
    for _ in 0..4 {
        fire_landing(&mut rig);
    }
    assert!(
        ask(&rig.tw, EXCHANGE, "metals").is_none(),
        "no restock without a listing"
    );
    assert_eq!(counter(&rig.tw, EXCHANGE, "metal_ore"), None);
    land_all(&mut rig, &draw);
    let relisting = relist_after_the_draw(&mut rig, quote, sale);
    assert_eq!(relisting, 3, "the relisting fire's writes");
    no_refused_writes(&rig.tw);
}

/// Settle the kept Draw's purchase and the sale, then fire until the list step lists the
/// milled pool; the relisting fire's counted writes (listing, meta, map).
fn relist_after_the_draw(rig: &mut Rig, quote: mush_market::QuoteId, sale: ContractId) -> usize {
    let purchase = contract_of(&rig.tw, quote);
    let mut relisting = None;
    for _ in 0..24 {
        if settled(&rig.tw, purchase)
            && settled(&rig.tw, sale)
            && ask(&rig.tw, EXCHANGE, "metals").is_some()
        {
            break;
        }
        let had = ask(&rig.tw, EXCHANGE, "metals").is_some();
        sweep(rig, 31);
        let keys = rig.last_keys.clone();
        let queued = drain(&mut rig.tw);
        land_queue(rig, &queued);
        if !had && ask(&rig.tw, EXCHANGE, "metals").is_some() {
            relisting = Some(keys.len());
        }
    }
    let listing = ask(&rig.tw, EXCHANGE, "metals").expect("relisted");
    assert_eq!(listing.quantity, 8, "the whole pool");
    let milled = ledger(&rig.tw, EXCHANGE, "metal_ore").expect("the ledger");
    assert_eq!(
        counter(&rig.tw, EXCHANGE, "metal_ore"),
        Some(milled.product_t)
    );
    assert!(tacsim::port_tick::drift_pages(rig.tw.world()).is_empty());
    relisting.expect("one sweep relisted")
}

/// **A removed mill row pages until staff withdraw** (12a-C6, D17, O-7's default): a later
/// version without the Exchange's `mill` row, shelved and loaded, applied after a batch and
/// before its restock: the next fire pages drift and the `metals` Listings line reads Flagged,
/// a line that shuts the desk; `@market/vendor withdraw <listing>` as GOD and the next fire
/// lists the whole pool, Ready, with no counter read for the removed mill. (Asserted on the
/// line and its gated state: past the shipped version the Lane line shuts the desk anyway, D22.)
#[test]
fn sm12a_a_removed_mill_row_pages_until_staff_withdraw() {
    let next = lane_variant(LANE_VERSION + 1, |doc| {
        if let Some(vendor) = doc["entries"]["ceres_port"]["def"]["vendors"][0].as_object_mut() {
            vendor.remove("mill");
        }
    });
    let mut rig = stocked_with(Roster::next(&next));
    let (miner, hold) = dray_p(&mut rig, "Miner", 1, "Salt Moth");
    let contract = seed_and_sell(&mut rig, miner, hold, EXCHANGE, "metal_ore", 20, 901);
    let _batch = settle_until(&mut rig, contract, true).expect("the batch lands in the settle");
    let listing = ask(&rig.tw, EXCHANGE, "metals").expect("the listing");
    assert_eq!(listing.quantity, 4_000, "not yet restocked");
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@{}", LANE_VERSION + 1)),
        "{plan}\n{apply}"
    );
    settle(&mut rig, contract);
    fire_landing(&mut rig);
    let vault = storefront(&rig.tw, EXCHANGE);
    assert!(
        tacsim::port_tick::drift_pages(rig.tw.world()).contains(&(vault.0, cid("metals"))),
        "the drift is paged"
    );
    let id = listing.id;
    let flagged = ask_line(&mut rig.tw, EXCHANGE, "metals");
    assert_eq!(
        flagged,
        format!(
            "  ceres_port ask freeport_exchange metals: listing #{id} lists 4000 t but the \
             storefront holds 4008 t. [flagged]"
        )
    );
    assert!(
        shuts(Binding::Listings, &flagged),
        "a line that shuts the desk"
    );
    let out = run(&mut rig.tw, GOD, &format!("@market/vendor withdraw {id}"));
    assert!(ask(&rig.tw, EXCHANGE, "metals").is_none(), "{out}");
    fire_until_idle(&mut rig, 24);
    let relisted = ask(&rig.tw, EXCHANGE, "metals").expect("relisted");
    assert_eq!(relisted.quantity, 4_008, "the whole pool");
    assert_eq!(
        state_word(&ask_line(&mut rig.tw, EXCHANGE, "metals")),
        "ready"
    );
    assert_eq!(
        counter(&rig.tw, EXCHANGE, "metal_ore"),
        None,
        "no counter for the removed mill"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 13–15. In kind: no money, no marker; the counted writes
// ---------------------------------------------------------------------------

/// Fire until idle, asserting every org account unchanged around each batch's landing and each
/// restock fire (no contract is open); the batches landed and the restocks seen.
fn mill_in_kind(rig: &mut Rig, org: &str, good: &str, max: usize) -> (usize, usize) {
    let (mut batches, mut restocks) = (0, 0);
    for _ in 0..max {
        let listed = ask(&rig.tw, org, good).map(|l| l.quantity);
        let before = money(&rig.tw);
        let queued = fire(rig);
        if ask(&rig.tw, org, good).map(|l| l.quantity) > listed {
            restocks += 1;
            assert_eq!(money(&rig.tw), before, "a restock fire moves no money");
        }
        for submit in &queued {
            let before = money(&rig.tw);
            land(rig, submit).expect("lands");
            if is_mill(submit) {
                batches += 1;
                assert_eq!(money(&rig.tw), before, "a batch's landing moves no money");
            }
        }
        if queued.is_empty() && rig.last_keys.is_empty() {
            break;
        }
    }
    (batches, restocks)
}

/// **Occator mills ice into water, in kind** (the review's B5): 40 t of ice ore seeded straight
/// into Occator's storefront (no sale, so no contract is open) make two batches and 16 t of
/// water; its ask is restocked twice; no account moves around either.
#[test]
fn sm12a_occator_mills_ice_into_water_in_kind() {
    let mut rig = stocked();
    past_next_boundary(&mut rig, 60);
    fire_until_idle(&mut rig, 24);
    let vault = storefront(&rig.tw, OCCATOR);
    let listed = ask(&rig.tw, OCCATOR, "volatiles")
        .expect("the ask")
        .quantity;
    assert_eq!(listed, 9_600);
    seed_pool(&mut rig, vault, "ice_ore", 40, LotId(901));
    let (batches, restocks) = mill_in_kind(&mut rig, OCCATOR, "volatiles", 24);
    assert_eq!((batches, restocks), (2, 2));
    let milled = ledger(&rig.tw, OCCATOR, "ice_ore").expect("the ledger");
    assert_eq!(
        (milled.batches, milled.ore_t, milled.product_t),
        (2, 40, 16)
    );
    assert_eq!(pool(&mut rig.tw, vault, "ice_ore"), 0);
    assert_eq!(pool(&mut rig.tw, vault, "volatiles"), 9_616);
    assert_eq!(
        ask(&rig.tw, OCCATOR, "volatiles").map(|l| l.quantity),
        Some(9_616)
    );
    assert_eq!(counter(&rig.tw, OCCATOR, "ice_ore"), Some(16));
    no_refused_writes(&rig.tw);
}

/// **A mill moves no money and writes no marker** (D2; the review's B5): metal ore seeded
/// straight into the Exchange's storefront; every account and both storefronts' marker rings
/// are unchanged around the batch's landing and around its restock fire.
#[test]
fn sm12a_a_mill_moves_no_money_and_writes_no_marker() {
    let mut rig = stocked();
    let exchange_vault = storefront(&rig.tw, EXCHANGE);
    let occator_vault = storefront(&rig.tw, OCCATOR);
    seed_pool(&mut rig, exchange_vault, "metal_ore", 20, LotId(901));
    let rings = |tw: &mut TestWorld| {
        (
            marker_ring(tw, exchange_vault),
            marker_ring(tw, occator_vault),
        )
    };
    let before = (money(&rig.tw), rings(&mut rig.tw));
    let (batches, restocks) = mill_in_kind(&mut rig, EXCHANGE, "metals", 24);
    assert_eq!((batches, restocks), (1, 1));
    assert_eq!((money(&rig.tw), rings(&mut rig.tw)), before);
    no_refused_writes(&rig.tw);
}

/// **The mill counts its writes** (12a-M10; the coordinator's ruling on C2; T11): measured step
/// by step and pinned — (a) the batch bundle into an existing product pool, (b) into a fresh one
/// (in a world whose `metals` stock row never minted), (c) the restock's legacy writes on its
/// fire, (d) the list step with a counter write on the relisting fire.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12a_the_mill_counts_its_writes() {
    // (a) and (c).
    let mut rig = stocked();
    let vault = storefront(&rig.tw, EXCHANGE);
    seed_pool(&mut rig, vault, "metal_ore", 20, LotId(901));
    let mut batch = None;
    for _ in 0..12 {
        let queued = fire(&mut rig);
        let (mills, rest): (Vec<_>, Vec<_>) = queued.into_iter().partition(is_mill);
        land_queue(&mut rig, &rest);
        if let Some(submit) = mills.into_iter().next() {
            batch = Some(submit);
            break;
        }
    }
    let batch = batch.expect("a batch is submitted");
    let a = land_counting(&mut rig, &batch).expect("the batch lands");
    let queued = fire(&mut rig);
    assert!(queued.is_empty(), "the restock submits nothing");
    let c = rig.last_keys.len();
    assert_eq!(
        counter(&rig.tw, EXCHANGE, "metal_ore"),
        Some(8),
        "{:?}",
        rig.last_keys
    );

    // (b) A fresh product pool: the Exchange's `metals` stock row cannot mint (its recorded lot
    // is not its own issue's), so the clerk skips it after three attempts and goes on to mill.
    let mut fresh = boot(Roster::default());
    damage_map(&mut fresh, |map| {
        map["vendors"][EXCHANGE]["stock"][0]["lot"] = serde_json::json!(12_345);
    });
    reboot(&mut fresh);
    let fresh_vault = storefront(&fresh.tw, EXCHANGE);
    seed_pool(&mut fresh, fresh_vault, "metal_ore", 20, LotId(902));
    let mut fresh_batch = None;
    for _ in 0..80 {
        let queued = fire(&mut fresh);
        let (mills, rest): (Vec<_>, Vec<_>) = queued.into_iter().partition(is_mill);
        for submit in &rest {
            let _ = land(&mut fresh, submit);
        }
        if let Some(submit) = mills.into_iter().next() {
            fresh_batch = Some(submit);
            break;
        }
    }
    let fresh_batch = fresh_batch.expect("the clerk mills past the unmintable row");
    assert_eq!(
        pool(&mut fresh.tw, fresh_vault, "metals"),
        0,
        "never minted"
    );
    let b = land_counting(&mut fresh, &fresh_batch).expect("the batch lands");
    assert_eq!(
        pool(&mut fresh.tw, fresh_vault, "metals"),
        8,
        "a fresh pool"
    );

    // (d) The relisting fire, test 11's.
    let mut sold = stocked_with(Roster::seeded(thirty_tonnes()));
    let (draw, quote, sale) = sold_out_with_a_batch(&mut sold);
    land_all(&mut sold, &draw);
    let d = relist_after_the_draw(&mut sold, quote, sale);

    println!("SM.12a writes: batch {a}, batch into a fresh pool {b}, restock {c}, relisting {d}");
    assert_eq!(
        a, 5,
        "ore pool scalar and book, product pool scalar and book, the ledger"
    );
    assert_eq!(b, 6, "and the endpoint census of the fresh pool");
    assert_eq!(c, 2, "the listing and `ceres_port`'s map");
    assert_eq!(
        d, 3,
        "the new listing, `market:meta` and `ceres_port`'s map"
    );
    no_refused_writes(&rig.tw);
    no_refused_writes(&fresh.tw);
    no_refused_writes(&sold.tw);
}

// ---------------------------------------------------------------------------
// 16. The Mill line
// ---------------------------------------------------------------------------

/// **The Mill line reads the ledger** (O-2b's staff texts; 12a-T1): whole-line world pins of
/// nothing yet, one batch and two, its product changed, its ledger unreadable, its ratio
/// changed (a variant shelved and loaded), and Pending "not seeded yet" when the vendor's map
/// row is gone. ("its storefront cannot be read." is pinned by the unit test alone, D19.)
#[test]
#[allow(clippy::too_many_lines)]
fn sm12a_the_mill_line_reads_the_ledger() {
    let mut rig = stocked();
    past_next_boundary(&mut rig, 60);
    let head = "  ceres_port mill freeport_exchange metal_ore:";
    let terms = "5 to 2, 20 t a batch, at most 40 t an IC day.";
    assert_eq!(
        mill_line(&mut rig.tw, EXCHANGE, "metal_ore"),
        format!("{head} nothing milled yet; 0 t of ore waits; {terms} [ready]")
    );
    let occator = mill_line(&mut rig.tw, OCCATOR, "ice_ore");
    assert_eq!(
        occator,
        format!(
            "  ceres_port mill occator ice_ore: nothing milled yet; 0 t of ore waits; {terms} [ready]"
        )
    );
    assert!(!gated(Binding::Mill));
    let (miner, hold) = dray_p(&mut rig, "Miner", 1, "Salt Moth");
    one_exchange_batch(&mut rig, miner, hold);
    let one = ledger(&rig.tw, EXCHANGE, "metal_ore").expect("the ledger");
    assert_eq!(
        mill_line(&mut rig.tw, EXCHANGE, "metal_ore"),
        format!(
            "{head} 1 batch has milled 20 t of ore into 8 t of metals, the last at {}; 0 t of \
             ore waits; {terms} [ready]",
            ic_stamp(one.recent[0].at_ic)
        )
    );
    let contract = seed_and_sell(&mut rig, miner, hold, EXCHANGE, "metal_ore", 25, 902);
    sale_batch(&mut rig, contract);
    settle(&mut rig, contract);
    fire_until_idle(&mut rig, 24);
    let two = ledger(&rig.tw, EXCHANGE, "metal_ore").expect("the ledger");
    assert_eq!(
        mill_line(&mut rig.tw, EXCHANGE, "metal_ore"),
        format!(
            "{head} 2 batches have milled 40 t of ore into 16 t of metals, the last at {}; 5 t \
             of ore waits; {terms} [ready]",
            ic_stamp(two.recent[1].at_ic)
        )
    );
    // Its product changed.
    let slot = ledger_slot(&rig, EXCHANGE, "metal_ore");
    let kept = rig.typed[&slot].clone();
    rig.typed
        .get_mut(&slot)
        .expect("the record")
        .get_mut("value")
        .expect("its value")["into"] = serde_json::json!("volatiles");
    reboot(&mut rig);
    assert_eq!(
        mill_line(&mut rig.tw, EXCHANGE, "metal_ore"),
        format!("{head} its product no longer matches its ledger. [flagged]")
    );
    // Its ledger unreadable (the key rule).
    rig.typed.insert(slot, kept);
    damage_exchange_ledger(&mut rig);
    reboot(&mut rig);
    assert_eq!(
        mill_line(&mut rig.tw, EXCHANGE, "metal_ore"),
        format!("{head} its ledger cannot be read. [flagged]")
    );
    // Not seeded: the vendor's map row gone (`sm5_an_unrecorded_intake…`'s shape).
    damage_map(&mut rig, |map| {
        let removed = map["vendors"]
            .as_object_mut()
            .and_then(|vendors| vendors.remove(EXCHANGE));
        assert!(removed.is_some(), "the row was recorded");
    });
    reboot(&mut rig);
    assert!(
        !port_map(&rig.tw, "ceres_port")
            .vendors
            .contains_key(&cid(EXCHANGE)),
        "the reboot keeps the row absent"
    );
    assert_eq!(
        mill_line(&mut rig.tw, EXCHANGE, "metal_ore"),
        format!("{head} not seeded yet. [pending]")
    );
    no_refused_writes(&rig.tw);

    // Its ratio changed: a later version, shelved and loaded, applied after one batch.
    let next = lane_variant(LANE_VERSION + 1, |doc| {
        doc["entries"]["ceres_port"]["def"]["vendors"][0]["mill"][0]["ratio"]["product"] =
            serde_json::json!(1);
    });
    let mut rig = stocked_with(Roster::next(&next));
    let (miner, hold) = dray_p(&mut rig, "Miner", 1, "Salt Moth");
    one_exchange_batch(&mut rig, miner, hold);
    plan_and_apply(&mut rig.tw);
    assert_eq!(
        mill_line(&mut rig.tw, EXCHANGE, "metal_ore"),
        format!("{head} its ratio no longer matches its ledger. [flagged]")
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 17–18. Source probes
// ---------------------------------------------------------------------------

fn crates_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .to_path_buf()
}

/// Every `.rs` file under `crates/<dir>`, as `(relative path, production text with its comment
/// lines dropped)` — the text before the file's first `#[cfg(test)]` (e7r2's `production()`).
fn production_files(dir: &str) -> Vec<(String, String)> {
    fn walk(path: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(p);
            }
        }
    }
    let root = crates_dir();
    let mut files = Vec::new();
    walk(&root.join(dir), &mut files);
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("readable source");
            let production = match text.find("#[cfg(test)]") {
                Some(at) => &text[..at],
                None => text.as_str(),
            };
            let kept: Vec<&str> = production
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect();
            let relative = path
                .strip_prefix(&root)
                .expect("under crates/")
                .to_string_lossy()
                .replace('\\', "/");
            (relative, kept.join("\n"))
        })
        .collect()
}

/// `(file, count)` of every file whose production text names `needle`, by path.
fn occurrences(files: &[(String, String)], needle: &str) -> Vec<(String, usize)> {
    let mut found: Vec<(String, usize)> = files
        .iter()
        .filter_map(|(path, text)| {
            let n = text.matches(needle).count();
            (n > 0).then(|| (path.clone(), n))
        })
        .collect();
    found.sort();
    found
}

/// **Only the mill restocks** (12a-M3): across the production text of `tacsim`, `mush-server`
/// (the one other crate with a production edge to the market) and `mush-market`, comment lines
/// ignored, `restock_ask` is defined once, in the market's `api.rs`, and called at exactly one
/// other place, the mill's step.
#[test]
fn sm12a_only_the_mill_restocks() {
    let mut files = production_files("tacsim/src");
    files.extend(production_files("mush-server/src"));
    files.extend(production_files("mush-market/src"));
    assert_eq!(
        occurrences(&files, "pub fn restock_ask("),
        vec![("mush-market/src/api.rs".to_owned(), 1)]
    );
    assert_eq!(
        occurrences(&files, "restock_ask("),
        vec![
            ("mush-market/src/api.rs".to_owned(), 1),
            ("tacsim/src/port/mill.rs".to_owned(), 1),
        ],
        "one definition and one call"
    );
}

/// **Only the clerk, the foundry and the mill create matter in tacsim** (12a-M2, as corrected;
/// the tacsim-side twin of `mush-market`'s `e7r2_p1_no_path_in_the_market_mints`): over
/// `crates/tacsim/src`'s production text, comment lines ignored, `.mint_origin(` occurs once, in
/// the stock clerk's mint; `.credit(` once in the mill and otherwise only in the relay foundry
/// (present, not counted: that clause is co-owned with the relay track, relay R9a's finite
/// foundry stock); `.debit(` only in the burn (twice) and the mill (once). SM.12b's delivery is
/// a `transfer`, so it needs no re-bless here.
#[test]
fn sm12a_only_the_clerk_the_foundry_and_the_mill_create_matter() {
    let files = production_files("tacsim/src");
    assert_eq!(
        occurrences(&files, ".mint_origin("),
        vec![("tacsim/src/port/stock.rs".to_owned(), 1)]
    );
    let credits = occurrences(&files, ".credit(");
    let (foundry, others): (Vec<_>, Vec<_>) = credits
        .into_iter()
        .partition(|(path, _)| path == "tacsim/src/relay_foundry.rs");
    assert_eq!(others, vec![("tacsim/src/port/mill.rs".to_owned(), 1)]);
    assert!(
        foundry.iter().any(|(_, n)| *n >= 1),
        "the relay foundry's credit (co-owned with the relay track) is present"
    );
    assert_eq!(
        occurrences(&files, ".debit("),
        vec![
            ("tacsim/src/port/burn.rs".to_owned(), 2),
            ("tacsim/src/port/mill.rs".to_owned(), 1),
        ]
    );
}

// ---------------------------------------------------------------------------
// 19. A pack apply on a kept world
// ---------------------------------------------------------------------------

/// **A pack apply adds the mills to a kept world** (D17, D22, D25): a mill-less world at the
/// shipped version, the shipped content (with its mills) as the next version, shelved and
/// loaded. Ore sold before the apply mills nothing and no counter is written; the apply is
/// refused by nothing and writes no counter; the next fires mill and restock, the counter
/// appearing with the first restock. Then a version that only re-sizes the Exchange's mill
/// (10 t a batch, 20 t a day), applied after a reboot and before any fire: the counter and the
/// ledger survive byte-identically and the next batch is 10 t.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12a_a_pack_apply_adds_the_mills_to_a_kept_world() {
    let next = lane_variant(LANE_VERSION + 1, |_| {});
    let mut rig = stocked_with(Roster {
        seed_lane: Some(lane_variant(LANE_VERSION, strip_mill)),
        shelf: Some(vec![
            mush_content::load_pack(&next).expect("the next lane verifies"),
        ]),
        extra_lane: Some(next.clone()),
    });
    let (miner, hold) = dray_p(&mut rig, "Miner", 1, "Salt Moth");
    let contract = seed_and_sell(&mut rig, miner, hold, EXCHANGE, "metal_ore", 40, 901);
    settle(&mut rig, contract);
    for _ in 0..12 {
        let queued = fire_landing(&mut rig);
        assert!(!queued.iter().any(is_mill), "no mill rows, no batch");
    }
    let vault = storefront(&rig.tw, EXCHANGE);
    assert_eq!(pool(&mut rig.tw, vault, "metal_ore"), 40);
    let key = tacsim::port::port_map_key(&lane_id(), &cid("ceres_port"));
    let no_counter = |map: &Value| {
        map["vendors"]
            .as_object()
            .expect("vendor rows")
            .values()
            .all(|row| row.get("mill_listed").is_none())
    };
    assert!(no_counter(&stored(&mut rig, &key).expect("the map")));
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@{}", LANE_VERSION + 1)),
        "{plan}\n{apply}"
    );
    let report = status(&mut rig.tw);
    let lane_line = format!(
        "lane: {LANE_PACK}@{} applied; this server ships @{LANE_VERSION}. [flagged]",
        LANE_VERSION + 1
    );
    assert!(report.lines().any(|l| l == lane_line), "{report}");
    assert!(
        no_counter(&stored(&mut rig, &key).expect("the map")),
        "the apply writes none"
    );
    let batch = fire_until_batch(&mut rig, 12);
    assert_eq!(batch.parameters["ordinal"], 1);
    assert_eq!(
        counter(&rig.tw, EXCHANGE, "metal_ore"),
        None,
        "not before its restock"
    );
    fire_until_idle(&mut rig, 24);
    let milled = ledger(&rig.tw, EXCHANGE, "metal_ore").expect("the ledger");
    assert_eq!((milled.batches, milled.ore_t), (2, 40));
    assert_eq!(counter(&rig.tw, EXCHANGE, "metal_ore"), Some(16));

    // The +2 version re-sizes the Exchange's mill; applied after a reboot, before any fire.
    let resized = lane_variant(LANE_VERSION + 2, |doc| {
        let mill = &mut doc["entries"]["ceres_port"]["def"]["vendors"][0]["mill"][0];
        mill["batch_t"] = serde_json::json!(10);
        mill["per_ic_day_t"] = serde_json::json!(20);
    });
    rig.roster.shelf = Some(vec![
        mush_content::load_pack(&resized).expect("the resized lane verifies"),
    ]);
    rig.roster.extra_lane = Some(resized);
    // The engine's ledger, and its wire, before the reboot (the rig's own kept typed map is
    // not read back: it moves only when the rig lands a bundle).
    let ledger_before = ledger(&rig.tw, EXCHANGE, "metal_ore").expect("the ledger");
    let wire_before = serde_json::to_string(&ledger_before).expect("a ledger serializes");
    let counter_before = counter(&rig.tw, EXCHANGE, "metal_ore");
    reboot(&mut rig);
    set_rate(&mut rig.tw, 1);
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@{}", LANE_VERSION + 2)),
        "{plan}\n{apply}"
    );
    let ledger_after = ledger(&rig.tw, EXCHANGE, "metal_ore").expect("the ledger survives");
    assert_eq!(ledger_after, ledger_before, "the ledger survives");
    assert_eq!(
        serde_json::to_string(&ledger_after).expect("a ledger serializes"),
        wire_before,
        "the ledger survives byte-identically"
    );
    assert_eq!(counter(&rig.tw, EXCHANGE, "metal_ore"), counter_before);
    // Today's tally holds 40 t, past the new 20 t rate: the next IC day mills at 10 t.
    past_next_boundary(&mut rig, 60);
    let contract = seed_and_sell(&mut rig, miner, hold, EXCHANGE, "metal_ore", 20, 902);
    let batch = sale_batch(&mut rig, contract);
    assert_eq!(batch.parameters["ordinal"], 3);
    let resized = ledger(&rig.tw, EXCHANGE, "metal_ore").expect("the ledger");
    assert_eq!(
        resized.recent.last().map(|b| (b.ore_t, b.product_t)),
        Some((10, 4)),
        "the next batch is 10 t"
    );
    settle(&mut rig, contract);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 20. The month
// ---------------------------------------------------------------------------

/// Metal ore the miner offers the Exchange each IC day (0..=60 t, averaging under the drip's 40).
const MONTH_METAL: [u64; 30] = [
    0, 40, 19, 55, 30, 45, 0, 60, 25, 40, 35, 50, 20, 41, 38, 44, 30, 0, 52, 33, 40, 27, 48, 36,
    19, 60, 22, 40, 31, 45,
];
/// Ice ore the icer offers Occator each IC day.
const MONTH_ICE: [u64; 30] = [
    30, 0, 45, 19, 60, 40, 35, 0, 50, 38, 41, 25, 40, 33, 0, 58, 44, 37, 20, 40, 36, 19, 52, 40,
    30, 45, 0, 39, 42, 35,
];
/// Metal concentrate the miner buys each IC day (0 until its first sale has settled).
const MONTH_BUY: [u64; 30] = [
    0, 0, 1, 2, 0, 2, 1, 2, 2, 0, 1, 2, 2, 1, 0, 2, 2, 1, 2, 0, 1, 2, 2, 1, 2, 0, 2, 1, 2, 2,
];

/// The month's running state.
struct Month {
    day: usize,
    rebooted: bool,
    fires: usize,
    pages_before_reboot: Option<Vec<(u64, CanonicalId)>>,
}

impl Month {
    /// Land one queue: a mill batch must land, every account unchanged around it and each
    /// mill's day tally within 40 t; on IC day 15 the first batch to land is followed at once
    /// by a reboot (before the next fire or sweep).
    fn land(&mut self, rig: &mut Rig, queued: &[AuthoritativeSubmit]) {
        let mut reboot_now = false;
        for submit in queued {
            if !is_mill(submit) {
                let _ = land(rig, submit);
                continue;
            }
            let before = money(&rig.tw);
            land(rig, submit).unwrap_or_else(|error| panic!("a batch did not land: {error}"));
            assert_eq!(money(&rig.tw), before, "a batch's landing moves no money");
            for (org, ore) in [(EXCHANGE, "metal_ore"), (OCCATOR, "ice_ore")] {
                if let Some(l) = ledger(&rig.tw, org, ore) {
                    assert!(l.day_ore_t <= 40, "{org}: {} t in one IC day", l.day_ore_t);
                }
            }
            if self.day == 15 && !self.rebooted {
                reboot_now = true;
            }
        }
        if reboot_now {
            self.pages_before_reboot = Some(tacsim::port_tick::drift_pages(rig.tw.world()));
            reboot(rig);
            // T6: a reboot keeps the world rate (the world timeline is durable), which
            // SM.12b's and SM.12c's rate-24 rigs rely on; `set_rate` is then a no-op.
            assert_eq!(world_rate(&rig.tw), 24, "a reboot keeps the world rate");
            set_rate(&mut rig.tw, 24);
            assert_eq!(world_rate(&rig.tw), 24, "the rate after the reboot");
            self.rebooted = true;
        }
    }

    /// Settle `contracts` with the mill-aware settle.
    fn settle(&mut self, rig: &mut Rig, contracts: &[ContractId]) {
        for _ in 0..16 {
            if contracts.iter().all(|c| settled(&rig.tw, *c)) {
                return;
            }
            sweep(rig, 31);
            self.fires += 1;
            let queued = drain(&mut rig.tw);
            self.land(rig, &queued);
        }
        panic!("day {}: the contracts did not settle", self.day);
    }

    /// Fire densely until the clerk reads idle; no contract is open, so every restock fire
    /// moves no money.
    fn dense(&mut self, rig: &mut Rig) {
        for _ in 0..24 {
            let listed = (
                ask(&rig.tw, EXCHANGE, "metals").map(|l| l.quantity),
                ask(&rig.tw, OCCATOR, "volatiles").map(|l| l.quantity),
            );
            let before = money(&rig.tw);
            let queued = fire(rig);
            self.fires += 1;
            let after = (
                ask(&rig.tw, EXCHANGE, "metals").map(|l| l.quantity),
                ask(&rig.tw, OCCATOR, "volatiles").map(|l| l.quantity),
            );
            if after.0 > listed.0 || after.1 > listed.1 {
                assert_eq!(money(&rig.tw), before, "day {}: a restock fire", self.day);
            }
            let idle = queued.is_empty() && rig.last_keys.is_empty();
            self.land(rig, &queued);
            if idle {
                return;
            }
        }
        panic!("day {}: the clerk was not idle within 24 fires", self.day);
    }

    /// IC-hour strides (150 real seconds at rate 24) to 60 IC seconds past the next boundary,
    /// each fire's queue landed.
    fn stride(&mut self, rig: &mut Rig) {
        let now = ic_now(&rig.tw);
        let target = (now.div_euclid(DAY) + 1) * DAY + 60;
        while ic_now(&rig.tw) < target {
            let left = (target - ic_now(&rig.tw) + 23) / 24;
            fold(rig);
            let t = rig.tw.world().now().saturating_add(left.min(150));
            rig.tw.set_now(t);
            let _ = rig.tw.fire_due(t.saturating_mul(1_000));
            rig.last_keys = fold(rig);
            self.fires += 1;
            let queued = drain(&mut rig.tw);
            self.land(rig, &queued);
        }
    }
}

/// **A month of milling conserves and restocks only by the mill** — doc 20 §13's SM.12
/// acceptance (§1 of the brief; D18; BC-7): 30 IC days at world rate 24 of a fixed schedule of
/// real ore sales into the Exchange's and Occator's bids and real purchases from the Exchange's
/// `metals` ask, with one reboot on IC day 15 between a batch landing and its restock. Each IC
/// day the `metals` ask moves by exactly what was milled less what was bought, and each mill's
/// day tally stays within 40 t; at the end each ledger conserves 5 : 2, the ore sold less the
/// ore still in the storefront is what was milled, each ask is its seed less purchases plus
/// the milled product, each product pool is its listing, no drift is paged and no Listings
/// line is Flagged; no batch's landing and no contract-free restock fire moves money.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12a_a_month_of_milling_conserves_and_restocks_only_by_the_mill() {
    let started = std::time::Instant::now();
    println!("metal ore offered: {MONTH_METAL:?}");
    println!("ice ore offered:   {MONTH_ICE:?}");
    println!("metals bought:     {MONTH_BUY:?}");
    let mut rig = stocked();
    set_rate(&mut rig.tw, 24);
    assert_eq!(world_rate(&rig.tw), 24);
    let (miner, miner_hold) = dray_p(&mut rig, "Miner", 1, "Salt Moth");
    let (icer, icer_hold) = dray_p(&mut rig, "Icer", 2, "Rime Moth");
    let mut month = Month {
        day: 0,
        rebooted: false,
        fires: 0,
        pages_before_reboot: None,
    };
    past_next_boundary(&mut rig, 60);
    month.dense(&mut rig);
    let exchange_vault = storefront(&rig.tw, EXCHANGE);
    let occator_vault = storefront(&rig.tw, OCCATOR);
    let metals_id = ask(&rig.tw, EXCHANGE, "metals").expect("the metals ask").id;
    assert_eq!(quantity(&rig.tw, metals_id.0), Some(4_000));
    assert_eq!(
        ask(&rig.tw, OCCATOR, "volatiles").map(|l| l.quantity),
        Some(9_600)
    );
    let (mut metal_sold, mut ice_sold, mut bought) = (0_u64, 0_u64, 0_u64);
    let mut prev_product = 0_u64;
    let mut prev_listed = 4_000_u64;
    for day in 0..30 {
        month.day = day;
        let mut contracts = Vec::new();
        for (pilot, hold, org, ore, table, sold) in [
            (
                miner,
                miner_hold,
                EXCHANGE,
                "metal_ore",
                MONTH_METAL,
                &mut metal_sold,
            ),
            (
                icer,
                icer_hold,
                OCCATOR,
                "ice_ore",
                MONTH_ICE,
                &mut ice_sold,
            ),
        ] {
            let qty = table[day];
            if qty == 0 {
                continue;
            }
            seed_pool(
                &mut rig,
                hold,
                ore,
                qty,
                LotId(20_000 + (day as u64) * 2 + u64::from(org == OCCATOR)),
            );
            let before = pool(&mut rig.tw, hold, ore);
            contracts.push(sell(&mut rig, pilot, org, ore, "all"));
            *sold += before - pool(&mut rig.tw, hold, ore);
        }
        month.settle(&mut rig, &contracts);
        let mut bought_today = 0;
        if MONTH_BUY[day] > 0 {
            let purchase = buy_and_land(&mut rig, miner, metals_id, MONTH_BUY[day]);
            month.settle(&mut rig, &[purchase]);
            assert!(
                settled(&rig.tw, purchase),
                "day {day}: the purchase settled"
            );
            bought_today = MONTH_BUY[day];
            bought += bought_today;
        }
        month.dense(&mut rig);
        month.stride(&mut rig);
        month.dense(&mut rig);
        // The day's checkpoint: the clerk is idle, so every milled tonne is listed.
        let exchange = ledger(&rig.tw, EXCHANGE, "metal_ore");
        let product = exchange.as_ref().map_or(0, |l| l.product_t);
        let listed = quantity(&rig.tw, metals_id.0).expect("the metals ask stands");
        assert_eq!(
            i128::from(listed) - i128::from(prev_listed),
            i128::from(product - prev_product) - i128::from(bought_today),
            "day {day}: the ask moved by the milled less the bought"
        );
        for (org, ore) in [(EXCHANGE, "metal_ore"), (OCCATOR, "ice_ore")] {
            if let Some(l) = ledger(&rig.tw, org, ore) {
                assert!(l.check().is_ok(), "day {day}: {org}'s ledger");
                assert!(tacsim::port::mill::conserves(l.ore_t, l.product_t, l.ratio));
                assert!(l.day_ore_t <= 40);
            }
        }
        prev_product = product;
        prev_listed = listed;
    }
    assert!(month.rebooted, "the month rebooted on IC day 15");
    assert_eq!(
        month.pages_before_reboot,
        Some(Vec::new()),
        "no drift page before the reboot"
    );
    assert!(tacsim::port_tick::drift_pages(rig.tw.world()).is_empty());
    // The end.
    let exchange = ledger(&rig.tw, EXCHANGE, "metal_ore").expect("the Exchange milled");
    let occator = ledger(&rig.tw, OCCATOR, "ice_ore").expect("Occator milled");
    for (l, sold, vault, ore, product, seed) in [
        (
            &exchange,
            metal_sold,
            exchange_vault,
            "metal_ore",
            "metals",
            4_000_i128,
        ),
        (
            &occator,
            ice_sold,
            occator_vault,
            "ice_ore",
            "volatiles",
            9_600_i128,
        ),
    ] {
        assert_eq!(l.ore_t * 2, l.product_t * 5, "{ore}: five to two");
        assert_eq!(
            sold - pool(&mut rig.tw, vault, ore),
            l.ore_t,
            "{ore}: milled is sold less waiting"
        );
        let org = if ore == "metal_ore" {
            EXCHANGE
        } else {
            OCCATOR
        };
        let listing = ask(&rig.tw, org, product).expect("the ask stands");
        let bought_here = if ore == "metal_ore" {
            i128::from(bought)
        } else {
            0
        };
        assert_eq!(
            i128::from(listing.quantity),
            seed - bought_here + i128::from(l.product_t),
            "{product}: the seed less purchases plus the milled"
        );
        let (undrawn, _) = market(&rig.tw).draw_reading(EndpointRef(vault), &cid(product));
        assert_eq!(
            pool(&mut rig.tw, vault, product),
            listing.quantity + undrawn,
            "{product}: the pool is the listing"
        );
    }
    let report = status(&mut rig.tw);
    assert!(
        !report
            .lines()
            .any(|l| l.contains(" ask ") && l.ends_with("[flagged]")),
        "{report}"
    );
    println!(
        "SM.12a month: {} fires, {:.1} s wall, metal ore sold {metal_sold} t, ice ore sold \
         {ice_sold} t, metals bought {bought} t, the Exchange milled {} t into {} t, Occator {} t \
         into {} t",
        month.fires,
        started.elapsed().as_secs_f64(),
        exchange.ore_t,
        exchange.product_t,
        occator.ore_t,
        occator.product_t
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 21–23. The fault paths: a counter, a stall, a refused restock
// ---------------------------------------------------------------------------

/// **A counter past its ledger flags only its good** (the review's B9; D21, D27): (i) the
/// Exchange row's counter damaged past its ledger's total: the `metals` Listings line and the
/// Mill line read Flagged, one WARN `Counter` names the Exchange's mill, the stock arm does not
/// return early (Occator mills and restocks, and its drip posts at the boundary), and Occator's
/// and the Co-op's lines are unchanged; (ii) in the 30 t world, after the milled tonnes are
/// relisted and bought out, the counter damaged to 0 (so more is unlisted than the pool
/// holds): the same Flagged line, the ask skipped alone with one WARN `Underflow` by the good,
/// and the arm goes on (Occator mills and restocks, its drip posts at the boundary, and the
/// Ceres clerk is never the port tick's refused agent).
#[test]
#[allow(clippy::too_many_lines)]
fn sm12a_a_counter_past_its_ledger_flags_only_its_good() {
    // (i) The counter past its ledger.
    let capture = Capture::default();
    let dispatch = capture_dispatch(capture.clone(), tracing::Level::WARN);
    tracing::dispatcher::with_default(&dispatch, || {
        let mut rig = stocked();
        let (miner, hold) = dray_p(&mut rig, "Miner", 1, "Salt Moth");
        let (icer, ice_hold) = dray_p(&mut rig, "Icer", 2, "Rime Moth");
        one_exchange_batch(&mut rig, miner, hold);
        let others: Vec<String> = [
            "ceres_port ask occator volatiles:",
            "ceres_port ask occator propellant:",
            "ceres_port ask coop provisions:",
        ]
        .iter()
        .map(|head| line(&mut rig.tw, head))
        .collect();
        damage_map(&mut rig, |map| {
            map["vendors"][EXCHANGE]["mill_listed"]["metal_ore"] = serde_json::json!(9);
        });
        reboot(&mut rig);
        set_rate(&mut rig.tw, 1);
        assert_eq!(
            ask_line(&mut rig.tw, EXCHANGE, "metals"),
            "  ceres_port ask freeport_exchange metals: its mill ledger cannot be read. [flagged]"
        );
        assert_eq!(
            mill_line(&mut rig.tw, EXCHANGE, "metal_ore"),
            "  ceres_port mill freeport_exchange metal_ore: its ledger cannot be read. [flagged]"
        );
        let now: Vec<String> = [
            "ceres_port ask occator volatiles:",
            "ceres_port ask occator propellant:",
            "ceres_port ask coop provisions:",
        ]
        .iter()
        .map(|head| line(&mut rig.tw, head))
        .collect();
        assert_eq!(now, others, "Occator's and the Co-op's lines are unchanged");
        occator_mills(&mut rig, icer, ice_hold, 902);
        let ice = bid_listing(&rig.tw, OCCATOR, "ice_ore");
        let unfilled = quantity(&rig.tw, ice).expect("the ice bid stands");
        past_next_boundary(&mut rig, 60);
        let mut rose = false;
        for _ in 0..6 {
            fire_landing(&mut rig);
            if quantity(&rig.tw, bid_listing(&rig.tw, OCCATOR, "ice_ore"))
                .is_some_and(|q| q > unfilled)
            {
                rose = true;
                break;
            }
        }
        assert!(rose, "Occator's drip posted at the boundary");
        no_refused_writes(&rig.tw);
    });
    let warned = capture.text();
    let counters: Vec<&str> = warned
        .lines()
        .filter(|l| l.contains("a mill's counter is past its ledger"))
        .collect();
    assert_eq!(counters.len(), 1, "{warned}");
    assert!(counters[0].contains("ore=metal_ore"), "{warned}");

    // (ii) The underflow.
    let capture = Capture::default();
    let dispatch = capture_dispatch(capture.clone(), tracing::Level::WARN);
    let clerk_id = tracing::dispatcher::with_default(&dispatch, || {
        let mut rig = stocked_with(Roster::seeded(thirty_tonnes()));
        let (draw, quote, sale) = sold_out_with_a_batch(&mut rig);
        let (icer, ice_hold) = dray_p(&mut rig, "Icer", 2, "Rime Moth");
        land_all(&mut rig, &draw);
        relist_after_the_draw(&mut rig, quote, sale);
        assert_eq!(counter(&rig.tw, EXCHANGE, "metal_ore"), Some(8));
        // Buy and draw the 8 milled tonnes: the pool is empty and the ask deleted.
        let buyer = rig
            .tw
            .world()
            .iter()
            .find(|o| o.name == "Buyer")
            .map(|o| o.id)
            .expect("the buyer");
        let listing = ask(&rig.tw, EXCHANGE, "metals").expect("relisted");
        let purchase = buy_and_land(&mut rig, buyer, listing.id, 8);
        settle(&mut rig, purchase);
        let vault = storefront(&rig.tw, EXCHANGE);
        assert_eq!(pool(&mut rig.tw, vault, "metals"), 0);
        assert!(ask(&rig.tw, EXCHANGE, "metals").is_none());
        damage_map(&mut rig, |map| {
            map["vendors"][EXCHANGE]["mill_listed"]["metal_ore"] = serde_json::json!(0);
        });
        reboot(&mut rig);
        set_rate(&mut rig.tw, 1);
        assert_eq!(
            ask_line(&mut rig.tw, EXCHANGE, "metals"),
            "  ceres_port ask freeport_exchange metals: its mill ledger cannot be read. [flagged]"
        );
        let volatiles = line(&mut rig.tw, "ceres_port ask occator volatiles:");
        for _ in 0..6 {
            fire_landing(&mut rig);
        }
        assert_eq!(
            line(&mut rig.tw, "ceres_port ask occator volatiles:"),
            volatiles
        );
        // The skip is a `continue`, never a return from the arm: Occator's ice ore sold
        // now mills and restocks, and its drip posts at the next boundary.
        occator_mills(&mut rig, icer, ice_hold, 902);
        let ice = bid_listing(&rig.tw, OCCATOR, "ice_ore");
        let unfilled = quantity(&rig.tw, ice).expect("the ice bid stands");
        past_next_boundary(&mut rig, 60);
        let mut rose = false;
        for _ in 0..6 {
            fire_landing(&mut rig);
            if quantity(&rig.tw, bid_listing(&rig.tw, OCCATOR, "ice_ore"))
                .is_some_and(|q| q > unfilled)
            {
                rose = true;
                break;
            }
        }
        assert!(rose, "Occator's drip posted at the boundary");
        no_refused_writes(&rig.tw);
        clerk(&rig.tw)
    });
    let warned = capture.text();
    let underflows: Vec<&str> = warned
        .lines()
        .filter(|l| l.contains("holds less than its mills count as not yet listed"))
        .collect();
    assert_eq!(underflows.len(), 1, "{warned}");
    assert!(underflows[0].contains("good=metals"), "{warned}");
    assert!(
        !warned
            .lines()
            .any(|l| l.contains("a port agent was refused")
                && l.contains(&format!("agent={}", clerk_id.0))),
        "the Ceres clerk is never the refused agent: {warned}"
    );
}

/// **A stalled batch is passed over for the day** (the review's B9; the attempt rule): with 20 t
/// waiting, three fires each submit the batch and each submission is discarded; the fourth fire
/// submits none (one WARN `Stalled`), nor does any later fire that IC day; past the next
/// boundary the batch is submitted again at the same ordinal and lands.
#[test]
fn sm12a_a_stalled_batch_is_passed_over_for_the_day() {
    let capture = Capture::default();
    let dispatch = capture_dispatch(capture.clone(), tracing::Level::WARN);
    tracing::dispatcher::with_default(&dispatch, || {
        let mut rig = stocked();
        past_next_boundary(&mut rig, 60);
        fire_until_idle(&mut rig, 24);
        let vault = storefront(&rig.tw, EXCHANGE);
        seed_pool(&mut rig, vault, "metal_ore", 20, LotId(901));
        let mut submitted = 0;
        for _ in 0..3 {
            let queued = fire(&mut rig);
            let (mills, rest): (Vec<_>, Vec<_>) = queued.into_iter().partition(is_mill);
            land_queue(&mut rig, &rest);
            assert_eq!(mills.len(), 1, "the batch is submitted");
            assert_eq!(mills[0].parameters["ordinal"], 1);
            submitted += 1;
        }
        assert_eq!(submitted, 3);
        for _ in 0..12 {
            let queued = fire_landing(&mut rig);
            assert!(
                !queued.iter().any(is_mill),
                "passed over for the rest of the IC day"
            );
        }
        assert!(ledger(&rig.tw, EXCHANGE, "metal_ore").is_none());
        past_next_boundary(&mut rig, 60);
        let batch = fire_until_batch(&mut rig, 12);
        assert_eq!(batch.parameters["ordinal"], 1, "the same ordinal");
        assert_eq!(
            ledger(&rig.tw, EXCHANGE, "metal_ore").map(|l| l.batches),
            Some(1)
        );
        no_refused_writes(&rig.tw);
    });
    let warned = capture.text();
    assert_eq!(
        warned.matches("a mill's batch never lands").count(),
        1,
        "{warned}"
    );
}

/// **A refused restock stalls nothing else** (the review's B9; D8): the Exchange's org suspended
/// after a batch lands and before its restock — the market refuses the restock, warned once
/// `Refused`, and the Exchange's restock is passed over for the day; meanwhile Occator's ice ore
/// mills and restocks and its drip posts at the boundary; the org made active again and the
/// boundary passed, the Exchange's restock lands (+8) and its counter reads 8.
#[test]
fn sm12a_a_refused_restock_stalls_nothing_else() {
    let capture = Capture::default();
    let dispatch = capture_dispatch(capture.clone(), tracing::Level::WARN);
    tracing::dispatcher::with_default(&dispatch, || {
        let mut rig = stocked();
        let (miner, hold) = dray_p(&mut rig, "Miner", 1, "Salt Moth");
        let (icer, ice_hold) = dray_p(&mut rig, "Icer", 2, "Rime Moth");
        let contract = seed_and_sell(&mut rig, miner, hold, EXCHANGE, "metal_ore", 20, 901);
        settle_until(&mut rig, contract, true).expect("the batch lands in the settle");
        let listed = ask(&rig.tw, EXCHANGE, "metals").expect("the ask").quantity;
        let org = port_map(&rig.tw, "ceres_port").orgs[&cid(EXCHANGE)].org;
        let out = run(&mut rig.tw, GOD, &format!("@org/status #{org}=suspended"));
        assert!(!out.contains("Usage"), "{out}");
        settle(&mut rig, contract);
        for _ in 0..6 {
            fire_landing(&mut rig);
        }
        assert_eq!(
            ask(&rig.tw, EXCHANGE, "metals").map(|l| l.quantity),
            Some(listed)
        );
        assert_eq!(counter(&rig.tw, EXCHANGE, "metal_ore"), None);
        occator_mills(&mut rig, icer, ice_hold, 902);
        let ice = bid_listing(&rig.tw, OCCATOR, "ice_ore");
        let unfilled = quantity(&rig.tw, ice).expect("the ice bid stands");
        past_next_boundary(&mut rig, 60);
        let mut rose = false;
        for _ in 0..6 {
            fire_landing(&mut rig);
            if quantity(&rig.tw, bid_listing(&rig.tw, OCCATOR, "ice_ore"))
                .is_some_and(|q| q > unfilled)
            {
                rose = true;
                break;
            }
        }
        assert!(rose, "Occator's drip posted at the boundary");
        let out = run(&mut rig.tw, GOD, &format!("@org/status #{org}=active"));
        assert!(!out.contains("Usage"), "{out}");
        past_next_boundary(&mut rig, 60);
        fire_until_idle(&mut rig, 24);
        assert_eq!(
            ask(&rig.tw, EXCHANGE, "metals").map(|l| l.quantity),
            Some(listed + 8)
        );
        assert_eq!(counter(&rig.tw, EXCHANGE, "metal_ore"), Some(8));
        no_refused_writes(&rig.tw);
    });
    let warned = capture.text();
    let refused: Vec<&str> = warned
        .lines()
        .filter(|l| l.contains("the clerk's restock of a milled ask was refused"))
        .collect();
    assert_eq!(refused.len(), 1, "{warned}");
    assert!(
        refused[0].contains("not trading") || refused[0].contains("No such listing"),
        "{warned}"
    );
}

/// **A milled pool its house cannot list stalls nothing else** (D8; lead ruling L24's rule as
/// the list step now applies it): the 30 t Exchange's `metals` ask bought to 0 and a batch
/// milled into its pool; the Exchange's org suspended and the purchase drawn and settled, so
/// the pool holds 8 milled tonnes with no listing and nothing in flight. The list step's
/// listing is refused ("That vendor's organization is not trading.") once that IC day and the
/// ask is passed over for the rest of it, so Occator's ice ore still mills and restocks and
/// its banked drip posts; the next IC day the ask is tried, and refused, once more; the org
/// made active again and the next boundary passed, the list step lists the whole pool and the
/// counter reads the ledger's total.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12a_a_milled_pool_its_house_cannot_list_stalls_nothing_else() {
    let capture = Capture::default();
    let dispatch = capture_dispatch(capture.clone(), tracing::Level::WARN);
    tracing::dispatcher::with_default(&dispatch, || {
        let mut rig = stocked_with(Roster::seeded(thirty_tonnes()));
        // Anchor the IC time of day, so the refusals below fall on two known IC days.
        past_next_boundary(&mut rig, 60);
        fire_until_idle(&mut rig, 24);
        let (draw, quote, sale) = sold_out_with_a_batch(&mut rig);
        let (icer, ice_hold) = dray_p(&mut rig, "Icer", 2, "Rime Moth");
        land_all(&mut rig, &draw);
        let org = port_map(&rig.tw, "ceres_port").orgs[&cid(EXCHANGE)].org;
        let out = run(&mut rig.tw, GOD, &format!("@org/status #{org}=suspended"));
        assert!(!out.contains("Usage"), "{out}");
        let purchase = contract_of(&rig.tw, quote);
        for _ in 0..24 {
            if settled(&rig.tw, purchase) && settled(&rig.tw, sale) {
                break;
            }
            sweep(&mut rig, 31);
            let queued = drain(&mut rig.tw);
            land_queue(&mut rig, &queued);
        }
        assert!(settled(&rig.tw, purchase), "the purchase settles");
        assert!(settled(&rig.tw, sale), "the sale settles");
        let vault = storefront(&rig.tw, EXCHANGE);
        assert_eq!(pool(&mut rig.tw, vault, "metals"), 8, "the milled tonnes");
        for _ in 0..6 {
            fire_landing(&mut rig);
        }
        assert!(
            ask(&rig.tw, EXCHANGE, "metals").is_none(),
            "a suspended house lists nothing"
        );
        // The clerk's acts go on: Occator mills and restocks, and its intake, at its cap
        // since the anchor's boundary (one banked drip, decision 4b), spends that drip
        // after the sale into it: the bid is re-posted at its cap.
        let ice = bid_listing(&rig.tw, OCCATOR, "ice_ore");
        assert_eq!(quantity(&rig.tw, ice), Some(240), "at its cap");
        occator_mills(&mut rig, icer, ice_hold, 902);
        let dripped = bid_listing(&rig.tw, OCCATOR, "ice_ore");
        assert_ne!(dripped, ice, "Occator's drip posted");
        assert_eq!(quantity(&rig.tw, dripped), Some(240));
        // The next IC day: the ask is tried, and refused, once more, and passed over again.
        past_next_boundary(&mut rig, 60);
        for _ in 0..6 {
            fire_landing(&mut rig);
        }
        assert!(ask(&rig.tw, EXCHANGE, "metals").is_none());
        // Active again, past the next boundary: the whole pool is listed.
        let out = run(&mut rig.tw, GOD, &format!("@org/status #{org}=active"));
        assert!(!out.contains("Usage"), "{out}");
        past_next_boundary(&mut rig, 60);
        fire_until_idle(&mut rig, 24);
        let listing = ask(&rig.tw, EXCHANGE, "metals").expect("relisted");
        assert_eq!(listing.quantity, 8, "the whole pool");
        let milled = ledger(&rig.tw, EXCHANGE, "metal_ore").expect("the ledger");
        assert_eq!(
            counter(&rig.tw, EXCHANGE, "metal_ore"),
            Some(milled.product_t)
        );
        assert_eq!(
            state_word(&ask_line(&mut rig.tw, EXCHANGE, "metals")),
            "ready"
        );
        no_refused_writes(&rig.tw);
    });
    let warned = capture.text();
    let refused: Vec<&str> = warned
        .lines()
        .filter(|l| l.contains("the stock clerk could not list an ask") && l.contains("metals"))
        .collect();
    assert_eq!(
        refused.len(),
        2,
        "tried once on each of the two IC days it was suspended: {warned}"
    );
    assert!(
        refused.iter().all(|l| l.contains("not trading")),
        "{warned}"
    );
}

// ---------------------------------------------------------------------------
// The restock's wait (the lead's ruling on conformance F2; the tests lens's T7)
// ---------------------------------------------------------------------------

/// **An undrawn Draw holds the restock, and never loses it** (the lead's ruling on SM.12a's
/// conformance F2, the builder's form; the tests lens's T7): while any Draw quote on the
/// Exchange's `metals` pool is unsettled the restock is deferred — a delay, never a loss. Over
/// many fires with a 10 t Draw accepted and not yet drawn, a milled batch's 8 t are not
/// restocked: the listing and the counter stand, the Listings line reads Ready with its "10 t
/// in hand-over, 8 t milled and not yet listed" clauses, no drift page and no mill warning
/// appear. (i) The Draw drawn: the restock lands on the fire whose market sweep settles the
/// quote, or on the first fire after it, never later. (ii) A second Draw never drawn: held
/// across the whole window, the restock lands on the fire whose sweep lapses the quote (its
/// 10 t refunded to the listing), or on the first fire after it.
#[test]
#[allow(clippy::too_many_lines)]
fn sm12a_an_undrawn_draw_holds_the_restock_and_never_loses_it() {
    let capture = Capture::default();
    let dispatch = capture_dispatch(capture.clone(), tracing::Level::WARN);
    tracing::dispatcher::with_default(&dispatch, || {
        let mut rig = stocked();
        // Anchor the IC time of day, so no drip or day boundary falls inside the test.
        past_next_boundary(&mut rig, 60);
        fire_until_idle(&mut rig, 24);
        let (miner, hold) = dray_p(&mut rig, "Miner", 1, "Salt Moth");
        let vault = storefront(&rig.tw, EXCHANGE);
        let metals = cid("metals");
        let in_flight = |tw: &TestWorld| market(tw).draw_reading(EndpointRef(vault), &metals).1;
        let id = ask(&rig.tw, EXCHANGE, "metals")
            .expect("the seed listing")
            .id;
        assert_eq!(quantity(&rig.tw, id.0), Some(4_000));
        top_up(&mut rig.tw, miner, 600_000);
        let held = |rig: &mut Rig, listed: u64, pool: u64| {
            let queued = fire_landing(rig);
            assert!(!queued.iter().any(is_mill), "no ore waits, so no batch");
            assert!(in_flight(&rig.tw), "the Draw is still in flight");
            assert_eq!(quantity(&rig.tw, id.0), Some(listed), "the restock waits");
            assert_eq!(
                ask_line(&mut rig.tw, EXCHANGE, "metals"),
                format!(
                    "  ceres_port ask freeport_exchange metals: listing #{id} at 200.00cr/t, \
                     {listed} t listed; the storefront holds {pool} t, 10 t in hand-over, 8 t \
                     milled and not yet listed. [ready]"
                )
            );
            assert!(tacsim::port_tick::drift_pages(rig.tw.world()).is_empty());
        };
        // Fire until the 8 t are restocked onto `listed`; the restock comes on the fire whose
        // sweep settles or lapses the quote, or on the first fire after it. The fires made.
        let restocked = |rig: &mut Rig, listed: u64| -> usize {
            let mut clear = None;
            for n in 1..=24 {
                fire_landing(rig);
                let now = quantity(&rig.tw, id.0);
                if now == Some(listed + 8) {
                    assert!(
                        clear.is_none_or(|c| n == c + 1),
                        "restocked on fire {n}, the quote settled on fire {clear:?}"
                    );
                    return n;
                }
                assert_eq!(now, Some(listed), "the restock waits, and loses nothing");
                if !in_flight(&rig.tw) {
                    assert!(clear.is_none(), "a fire with nothing in flight restocks");
                    clear = Some(n);
                }
            }
            panic!("the restock never landed (the quote settled on fire {clear:?})");
        };

        // (i) A Draw of 10 t accepted and kept aside; a batch of 8 t lands behind it.
        let (accepted, quote) = buy(&mut rig, miner, ListingId(id.0), 10);
        assert!(accepted.contains("Contract #"), "{accepted}");
        let draw = drain(&mut rig.tw);
        assert_eq!(draw.len(), 1, "{accepted}");
        assert_eq!(quantity(&rig.tw, id.0), Some(3_990));
        let sale = seed_and_sell(&mut rig, miner, hold, EXCHANGE, "metal_ore", 20, 901);
        let batch = sale_batch(&mut rig, sale);
        assert_eq!(batch.parameters["ordinal"], 1);
        settle(&mut rig, sale);
        assert_eq!(counter(&rig.tw, EXCHANGE, "metal_ore"), None);
        for _ in 0..12 {
            held(&mut rig, 3_990, 4_008);
        }
        assert_eq!(
            counter(&rig.tw, EXCHANGE, "metal_ore"),
            None,
            "nothing listed"
        );
        land_all(&mut rig, &draw);
        let fires = restocked(&mut rig, 3_990);
        println!("SM.12a wait: the drawn quote's restock landed on fire {fires} after the draw");
        assert_eq!(counter(&rig.tw, EXCHANGE, "metal_ore"), Some(8));
        let purchase = contract_of(&rig.tw, quote);
        settle(&mut rig, purchase);
        assert_eq!(
            ask_line(&mut rig.tw, EXCHANGE, "metals"),
            format!(
                "  ceres_port ask freeport_exchange metals: listing #{id} at 200.00cr/t, 3998 t \
                 listed; the storefront holds 3998 t. [ready]"
            )
        );

        // (ii) A second Draw of 10 t accepted and never drawn; a second batch behind it.
        let (accepted, _lapsing) = buy(&mut rig, miner, ListingId(id.0), 10);
        assert!(accepted.contains("Contract #"), "{accepted}");
        assert_eq!(drain(&mut rig.tw).len(), 1, "{accepted}");
        assert_eq!(quantity(&rig.tw, id.0), Some(3_988));
        let sale = seed_and_sell(&mut rig, miner, hold, EXCHANGE, "metal_ore", 20, 902);
        let batch = sale_batch(&mut rig, sale);
        assert_eq!(batch.parameters["ordinal"], 2);
        settle(&mut rig, sale);
        for _ in 0..12 {
            held(&mut rig, 3_988, 4_006);
        }
        // Half the window in one stride: still held.
        sweep(&mut rig, mush_market::DRAW_WINDOW_SECS / 2);
        let queued = drain(&mut rig.tw);
        land_queue(&mut rig, &queued);
        held(&mut rig, 3_988, 4_006);
        assert_eq!(counter(&rig.tw, EXCHANGE, "metal_ore"), Some(8), "unmoved");
        // Past the window: the sweep lapses the quote and refunds its 10 t; then the restock.
        sweep(&mut rig, mush_market::DRAW_WINDOW_SECS / 2 + 60);
        let queued = drain(&mut rig.tw);
        land_queue(&mut rig, &queued);
        let listed = quantity(&rig.tw, id.0).expect("the listing stands");
        let fires = if listed == 3_998 + 8 {
            0
        } else {
            assert_eq!(listed, 3_998, "the lapse refunded the 10 t");
            assert!(!in_flight(&rig.tw), "the lapse settles the quote");
            restocked(&mut rig, 3_998)
        };
        println!("SM.12a wait: the lapsed quote's restock landed {fires} fire(s) after the lapse");
        assert!(fires <= 1, "the first fire after the lapse restocks");
        assert_eq!(counter(&rig.tw, EXCHANGE, "metal_ore"), Some(16));
        let milled = ledger(&rig.tw, EXCHANGE, "metal_ore").expect("the ledger");
        assert_eq!((milled.batches, milled.product_t), (2, 16), "nothing lost");
        assert!(tacsim::port_tick::drift_pages(rig.tw.world()).is_empty());
        no_refused_writes(&rig.tw);
    });
    let warned = capture.text();
    assert!(
        !warned.contains("tacsim: a mill"),
        "the wait warns nothing:\n{warned}"
    );
}
