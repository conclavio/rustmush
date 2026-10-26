//! **Doc 20 cut SM.5 acceptance — legal and market plumbing** (doc 20 §1 beats 1 and 12,
//! §5, §7.2, §7.3, §7.4, §9.1, §11, §12.3; D-20-10, D-20-12; Q-20-5, Q-20-11; the SM.5
//! brief's acceptance A1–A12).
//!
//! Lane v11 carries the Freeport Registry's prospecting licence (`+licence/buy go`: an
//! entitlement account, a Public fee contract the economy sweep settles and canon
//! chronicles, one `tacsim:licence:` record a gateway account), the Ceres ore bids and their
//! intake drip (a stock-arm step: one drip an IC day, within the house's funds, renewed
//! before the market's TTL), the programme's fixed reference for ore (the Co-op's Trust takes
//! a quarter of the margin over half the bid) and Occator's appended capital raise; `+tv/lease`
//! reads the licence; `+market/sell <listing>` previews a bid's unfilled intake and `=all`
//! sells what the bid takes.
//!
//! Composed on the composition root (`vehicles.rs`'s rig: `commerce.rs`'s `Roster`, the test
//! world's layer with the game content). Every boot asserts no namespace write was refused;
//! every drain asserts no kept tacsim submission is GOD's. The world tests run at world rate
//! 1, so every IC instant a test names is the instant the world reads.

use crate::support;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use mush_core::{
    AccountId, AccountIdentity, ActionStateBootRecord, Audience, AuthoritativeSubmit, CanonicalId,
    ClockRef, ClockStamp, CommitToken, GOD, IdempotencyKey, IdempotencyScope, ModuleWrite, ObjId,
    OfferExecutionContext, OfferSubmission, RequestUid, ScopedIdempotencyKey,
};
use mush_economy::{
    AccountStatus, ContractId, ContractState, EconomicVisibility, EconomyConfig, EconomyStore,
    VaultId,
};
use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
use mush_market::{ListingId, MarketStore};
use mush_org::PrincipalId;
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde_json::Value;
use smol_str::SmolStr;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK, LANE_VERSION};
use tacsim::pack::{PackEntry, RulesPack, load_pack};
use tacsim::port::apply::room_by_key;
use tacsim::port::licence::{
    LICENCE_BOOKS_DARK, LICENCE_BOUGHT, LICENCE_EXTENDED, LICENCE_FUNDS, LICENCE_GRACE,
    LICENCE_HELP, LICENCE_IN_FORCE, LICENCE_LAPSED, LICENCE_NO_CREDIT, LICENCE_NO_MATCH,
    LICENCE_NONE, LICENCE_NOT_HERE, LICENCE_OTHER, LICENCE_PREFIX, LICENCE_PREVIEW,
    LICENCE_PREVIEW_EXTEND, LICENCE_SUSPENDED, Licence, LicenceStanding, LicenceState, ic_stamp,
    licence_gate,
};
use tacsim::port::tv::LEASE_LICENCE;
use tacsim::port::{BidRow, HullRow, PortDef, PortMap, PortMapState, room_key};
use tacsim::{TacsimConfig, TacsimStore};

use support::ground::{UNSIGNED, sign};
use support::ore::{ORE_GOODS, strip_ore};

/// Lane v10's published digest (doc 20 cut SM.3a), which `strip_ore` rebuilds.
const LANE_V10_DIGEST: &str = "3f49d1764cf10ff5ac1b43f0f88b06d47ba9bd89d7633fa8808f95121b0156c1";

/// IC seconds in an IC day.
const DAY: i64 = 86_400;
/// The licence's fee as the economy renders it, and in minor units.
const FEE: &str = "2500.00cr";
const FEE_MINOR: i64 = 250_000;
/// The Registry's name in the world.
const REGISTRY: &str = "Freeport Registry";

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
// The rig (vehicles.rs's: commerce.rs's Roster, voyage.rs's helpers)
// ---------------------------------------------------------------------------

/// How a test roster is composed: `commerce.rs`'s (no sheet, the test world's layer, the
/// shipped game content), or an operator lane in place of the shipped one, with a shelf.
#[derive(Clone, Default)]
struct Roster {
    seed_lane: Option<String>,
    shelf: Option<Vec<mush_content::Pack>>,
}

impl Roster {
    fn bare() -> Roster {
        Roster::default()
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
        if let Some(lane) = &self.seed_lane {
            cfg.seeds.push(tacsim::config::PackSource {
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
    };
    sweep(&mut rig.tw, 120);
    rig
}

/// Boot, settle and stock every port, at world rate 1 (every IC instant exact).
fn stocked() -> Rig {
    let mut rig = boot(Roster::bare());
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
    let mut bytes = [0x5b_u8; 16];
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

/// One fire, its submissions landed quietly.
fn fire_quietly(rig: &mut Rig) {
    let (_, queued) = fire(rig);
    land_quietly(rig, &queued);
}

/// Let the world's ticks run a while where the clock stands — three port-tick fires,
/// then a minute's sweep (the surface tick, the economy sweep, the canon bridge) —
/// landing what they queue.
fn tick_through(rig: &mut Rig) {
    for _ in 0..3 {
        fire_quietly(rig);
    }
    sweep(&mut rig.tw, 61);
    let queued = drain(&mut rig.tw);
    land_quietly(rig, &queued);
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

/// A chartered pilot whose hull the yard has made a Dray-P — with **no** licence.
fn dray_p(rig: &mut Rig, who: &str, seed: i64, name: &str) -> (ObjId, HullRow) {
    let (pilot, row) = chartered(rig, who, seed, name);
    let out = run(&mut rig.tw, pilot, "+coop/refit prospector");
    assert!(out.contains("as a Dray-P"), "{out}");
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

/// Seed `qty` of `good` into `vault`'s pool through `lot` (additive; staff's path).
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

/// Move the world `days` IC days on without firing a tick (at rate 1, real days too).
fn skip_days(rig: &mut Rig, days: i64) {
    let t = rig.tw.world().now().saturating_add(days * DAY);
    rig.tw.set_now(t);
}

/// Move the world, without firing, to `offset` IC seconds past the next IC-day boundary.
fn past_next_boundary(rig: &mut Rig, offset: i64) {
    let now = ic_now(&rig.tw);
    let next = (now.div_euclid(DAY) + 1) * DAY + offset;
    let t = rig.tw.world().now().saturating_add(next - now);
    rig.tw.set_now(t);
    assert!(ic_now(&rig.tw) >= next, "world rate 1");
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

/// The Exchange Hall (C07): Occator's counter and the ore bids' board.
fn exchange(tw: &TestWorld) -> ObjId {
    station_room(tw, "ceres_port", "room.c07_exchange")
}

fn name_of(tw: &TestWorld, obj: ObjId) -> String {
    tw.world()
        .get(obj)
        .map(|o| o.name.clone())
        .unwrap_or_default()
}

/// `line` at Occator's counter.
fn at_occator(rig: &mut Rig, pilot: ObjId, line: &str) -> String {
    let counter = exchange(&rig.tw);
    teleport(&mut rig.tw, pilot, counter);
    run(&mut rig.tw, pilot, line)
}

/// The pilot's credit account.
fn credit_of(tw: &TestWorld, who: ObjId) -> mush_economy::AccountId {
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

/// An org of `ceres_port`'s account.
fn org_account(tw: &TestWorld, org: &str) -> mush_economy::AccountId {
    mush_economy::AccountId(
        port_map(tw, "ceres_port").orgs[&cid(org)]
            .account
            .expect("the org's account"),
    )
}

/// Pay `org`'s account down to `left` minor units available, into the port's treasury.
fn pay_house_down_to(rig: &mut Rig, org: &str, left: i64) {
    let account = org_account(&rig.tw, org);
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let (posted, held) = balances(&rig.tw, account);
    let out = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@econ/pay #{}=#{treasury} {}",
            account.0,
            posted - held - left
        ),
    );
    assert!(out.contains("Paid: transaction #"), "{out}");
    let (posted, held) = balances(&rig.tw, account);
    assert_eq!(posted - held, left);
}

/// The pilot's gateway account.
fn gateway(rig: &Rig, pilot: ObjId) -> AccountId {
    rig.oracle
        .0
        .lock()
        .expect("the oracle")
        .get(&pilot)
        .copied()
        .expect("a mapped pilot")
}

/// The licence record standing for `pilot`'s gateway account.
fn licence(rig: &Rig, pilot: ObjId) -> Licence {
    match store(&rig.tw).licence_state(gateway(rig, pilot)) {
        LicenceState::Present(record) => record.clone(),
        other => panic!("no readable licence: {other:?}"),
    }
}

/// The pilot's unscoped accounts in the licence instrument.
fn entitlements(tw: &TestWorld, who: ObjId) -> Vec<mush_economy::Account> {
    let instrument = tacsim::economy::licence_instrument().expect("the licence instrument");
    economy(tw)
        .snapshot()
        .accounts_of(&PrincipalId::Object(who))
        .filter(|a| a.instrument == instrument)
        .cloned()
        .collect()
}

/// How many `tacsim:licence:` documents the world wrote since the last take.
fn licence_writes(tw: &mut TestWorld) -> usize {
    tw.take_module_ops()
        .iter()
        .filter(|op| match &op.op {
            ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key } => {
                key.starts_with(LICENCE_PREFIX)
            }
            ModuleWrite::StreamAppend { .. } => false,
        })
        .count()
}

/// `+licence/buy go` as `pilot`, asserting the licence.
fn buy(rig: &mut Rig, pilot: ObjId) -> String {
    let out = run(&mut rig.tw, pilot, "+licence/buy go");
    assert!(
        out.contains(" licenses you to prospect at its sites until ")
            || out.contains(" extends your licence to "),
        "{out}"
    );
    out
}

/// A standing ore bid's map row.
fn bid_row(tw: &TestWorld, org: &str, good: &str) -> BidRow {
    port_map(tw, "ceres_port").vendors[&cid(org)].bids[&cid(good)].clone()
}

/// Every refresh row of the Exchange and Occator, by good.
fn ore_rows(tw: &TestWorld) -> BTreeMap<String, BidRow> {
    ORE_GOODS
        .iter()
        .map(|good| {
            let org = if *good == "ice_ore" {
                "occator"
            } else {
                "freeport_exchange"
            };
            ((*good).to_owned(), bid_row(tw, org, good))
        })
        .collect()
}

/// A listing's quantity, if it stands.
fn quantity(tw: &TestWorld, listing: u64) -> Option<u64> {
    market(tw).listing(ListingId(listing)).map(|l| l.quantity)
}

/// `+market/sell <metal_ore bid>=<what>` (or another good) at the Exchange Hall, the deliver
/// leg landed; what the pilot heard.
fn sell(rig: &mut Rig, pilot: ObjId, org: &str, good: &str, what: &str) -> String {
    let hall = exchange(&rig.tw);
    teleport(&mut rig.tw, pilot, hall);
    let listing = bid_row(&rig.tw, org, good).listing;
    let out = run(
        &mut rig.tw,
        pilot,
        &format!("+market/sell {listing}={what}"),
    );
    assert!(out.contains("Sold "), "{out}");
    let queued = drain(&mut rig.tw);
    land_all(rig, &queued);
    out
}

fn state_of(tw: &TestWorld, c: ContractId) -> Option<ContractState> {
    economy(tw).snapshot().contract(c).map(|c| c.state)
}

/// Sweep until `pilot`'s newest sale settles.
fn settle_last(rig: &mut Rig, pilot: ObjId) {
    let contract = market(&rig.tw)
        .quotes_of_seller(&PrincipalId::Object(pilot))
        .last()
        .and_then(|q| q.contract)
        .expect("a sale contract");
    settle(rig, contract);
}

/// Sweep until `contract` settles.
fn settle(rig: &mut Rig, contract: ContractId) {
    for _ in 0..12 {
        if matches!(
            state_of(&rig.tw, contract),
            Some(ContractState::Settled { .. })
        ) {
            return;
        }
        sweep(&mut rig.tw, 31);
        let queued = drain(&mut rig.tw);
        land_quietly(rig, &queued);
    }
    panic!(
        "contract #{contract} did not settle: {:?}",
        state_of(&rig.tw, contract)
    );
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

/// The world's IC clock as a claim's `ClockStamp` reads it.
fn stamp(at: i64) -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at,
    }
}

// ---------------------------------------------------------------------------
// The licence
// ---------------------------------------------------------------------------

/// **A1: a licence bought through the real verb authorizes extraction over a staff-seeded
/// claim** (§9.1; `crates/mush-industry/src/licence.rs`): the composition root names the
/// licence instrument to industry; the same pilot over the same in-memory claim and deposit
/// answers `NoLicence` before buying and `Ok` after.
#[test]
fn sm5_a_licence_authorizes_extraction_over_a_seeded_claim() {
    let mut rig = stocked();
    let (pilot, _) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let named = rig
        .tw
        .world()
        .module_state::<mush_industry::IndustryStore>()
        .and_then(|s| s.licence().cloned())
        .expect("the composition root names the licence");
    assert_eq!(named.as_str(), tacsim::economy::LICENCE_FREEPORT);
    let (deposit, claim) = seeded_claim(pilot);
    assert_eq!(
        authorized(&rig.tw, pilot, &deposit, &claim),
        Err(mush_industry::ExtractionRefusal::NoLicence)
    );
    buy(&mut rig, pilot);
    let after = authorized(&rig.tw, pilot, &deposit, &claim);
    assert!(after.is_ok(), "{after:?}");
}

/// A staff-seeded metal-ore deposit and a claim on it held by `pilot`, built in memory as
/// `crates/mush-industry/tests/e7c3.rs` builds them (A1).
fn seeded_claim(pilot: ObjId) -> (mush_industry::OreDeposit, mush_industry::ClaimRecord) {
    let deposit = mush_industry::OreDeposit::new(
        mush_industry::OreDepositId(1),
        ObjId(7),
        cid("metal_ore"),
        Grade::new(60).expect("grade"),
        mush_industry::Accessibility::new(100).expect("accessibility"),
        mush_industry::BandId::new("grade_b").expect("band"),
        mush_industry::BandId::new("reserve_major").expect("band"),
        mush_industry::YieldCurve::new(vec![mush_industry::CurvePoint {
            cumulative: 0,
            per_cycle: 500,
        }])
        .expect("curve"),
        EndpointRef(VaultId(1)),
        10_000,
    )
    .expect("a deposit");
    let claim = mush_industry::ClaimRecord::new(
        mush_industry::ClaimId(1),
        mush_industry::ClaimSubject::Deposit {
            deposit: deposit.id(),
        },
        PrincipalId::Object(GOD),
        PrincipalId::Object(pilot),
        ObjId(901),
        mush_industry::Exclusivity::Shared,
        1,
        stamp(i64::MAX / 2),
        mush_industry::Transferability::Transferable,
    )
    .expect("a staff-seeded claim");
    (deposit, claim)
}

/// `mush_industry::extraction_authorized` for `pilot` over the seeded claim, passing the
/// composed world's `IndustryStore::licence()` (A1).
fn authorized(
    tw: &TestWorld,
    pilot: ObjId,
    deposit: &mush_industry::OreDeposit,
    claim: &mush_industry::ClaimRecord,
) -> Result<mush_industry::licence::Authorized, mush_industry::ExtractionRefusal> {
    let licence = tw
        .world()
        .module_state::<mush_industry::IndustryStore>()
        .and_then(|s| s.licence().cloned());
    mush_industry::extraction_authorized(
        tw.world(),
        pilot,
        &PrincipalId::Object(pilot),
        deposit,
        10_000,
        std::slice::from_ref(claim),
        licence.as_ref(),
        &stamp(ic_now(tw)),
    )
}

/// **No unpaid entitlement is ever `Active`** (the lead's pre-commit round, item 1; residual
/// F3): staff put an approval rule under the fee on the pilot's credit account, so the escrow
/// at accept (`/buy go`'s step c) needs an approval nobody gives and the buy is refused there,
/// after step a opened the entitlement account. It is left `Frozen` with no record and
/// nothing paid: `licence_gate` reads `Unlicensed`, `+licence` the no-licence line, and
/// `extraction_authorized` refuses it over the staff-seeded claim (`LicenceInactive`), where
/// before this round it was `Active` and authorized. With the rule cleared, a second buy
/// reuses that account, activates it and licenses the pilot.
#[test]
fn sm5_a_refused_buy_leaves_no_active_entitlement() {
    let mut rig = stocked();
    let (pilot, _) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let (deposit, claim) = seeded_claim(pilot);
    let account = credit_of(&rig.tw, pilot);
    let rule = mush_economy::ApprovalRule {
        threshold: mush_economy::Amount::from_minor(100),
        policy: mush_org::ApprovalPolicy::Threshold {
            approvals: 1,
            from_role: None,
        },
        window: None,
    };
    let ruled = support::with_ctx(&mut rig.tw, |ctx| {
        mush_economy::set_account_approval(ctx, account, Some(rule))
    });
    assert!(ruled, "staff put an approval rule on the pilot's account");
    let before = balances(&rig.tw, account);
    let out = run(&mut rig.tw, pilot, "+licence/buy go");
    assert_eq!(
        out.lines().last(),
        Some(tacsim::port::licence::LICENCE_FAILED),
        "{out}"
    );
    assert!(matches!(
        store(&rig.tw).licence_state(gateway(&rig, pilot)),
        LicenceState::Absent
    ));
    let residue = entitlements(&rig.tw, pilot);
    assert_eq!(residue.len(), 1, "step a opened the entitlement account");
    assert_eq!(
        residue[0].status,
        AccountStatus::Frozen,
        "never Active unpaid"
    );
    assert_eq!(balances(&rig.tw, account), before, "nothing paid");
    assert_eq!(
        authorized(&rig.tw, pilot, &deposit, &claim),
        Err(mush_industry::ExtractionRefusal::LicenceInactive)
    );
    assert_eq!(
        licence_gate(rig.tw.world(), pilot, ic_now(&rig.tw)),
        Err(LicenceStanding::Unlicensed)
    );
    assert_eq!(
        run(&mut rig.tw, pilot, "+licence"),
        LICENCE_NONE
            .replace("{issuer}", REGISTRY)
            .replace("{fee}", FEE)
            .replace("{term}", "168")
    );

    // The rule cleared: the next buy reuses the residue and activates it.
    let cleared = support::with_ctx(&mut rig.tw, |ctx| {
        mush_economy::set_account_approval(ctx, account, None)
    });
    assert!(cleared);
    let t0 = ic_now(&rig.tw);
    let out = buy(&mut rig, pilot);
    let record = licence(&rig, pilot);
    assert_eq!(record.entitlement, residue[0].id.0, "the residue is reused");
    assert_eq!(record.expires_ic, t0 + 168 * DAY);
    assert_eq!(record.terms, 1);
    let bought = LICENCE_BOUGHT
        .replace("{issuer}", REGISTRY)
        .replace("{until}", &ic_stamp(t0 + 168 * DAY))
        .replace("{fee}", FEE)
        .replace("{account}", &record.entitlement.to_string());
    assert_eq!(out.lines().last(), Some(bought.as_str()), "{out}");
    let now = entitlements(&rig.tw, pilot);
    assert_eq!(now.len(), 1, "one entitlement account");
    assert_eq!(now[0].status, AccountStatus::Active);
    let after = authorized(&rig.tw, pilot, &deposit, &claim);
    assert!(after.is_ok(), "{after:?}");
    assert_eq!(licence_gate(rig.tw.world(), pilot, ic_now(&rig.tw)), Ok(()));
}

/// **A bare `+licence/buy` previews and commits nothing** (§9.1): the preview line exactly;
/// no record, no entitlement account, the balance unchanged, no licence document written.
#[test]
fn sm5_a_bare_buy_previews_and_commits_nothing() {
    let mut rig = stocked();
    let (pilot, _) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let account = credit_of(&rig.tw, pilot);
    let before = balances(&rig.tw, account);
    let _ = licence_writes(&mut rig.tw);
    let out = run(&mut rig.tw, pilot, "+licence/buy");
    let want = LICENCE_PREVIEW
        .replace("{issuer}", REGISTRY)
        .replace("{term}", "168")
        .replace("{until}", &ic_stamp(ic_now(&rig.tw) + 168 * DAY))
        .replace("{fee}", FEE);
    assert_eq!(out, want);
    assert!(matches!(
        store(&rig.tw).licence_state(gateway(&rig, pilot)),
        LicenceState::Absent
    ));
    assert!(entitlements(&rig.tw, pilot).is_empty());
    assert_eq!(balances(&rig.tw, account), before);
    assert_eq!(licence_writes(&mut rig.tw), 0);
    // The grammar: a word that is not `go`, or one after it, is the usage line.
    for line in [
        "+licence/buy now",
        "+licence/buy go now",
        "+licence everything",
    ] {
        let out = run(&mut rig.tw, pilot, line);
        assert_eq!(out, tacsim::port::licence::LICENCE_USAGE, "{line}");
    }
}

/// **A2: the licence lapses after its grace, read at the next act** (§9.1, lead ruling 5):
/// bought at `t0`, a lease at `t0 + 170 d` (grace) goes through; at `t0 + 172 d` the lease
/// refuses `LEASE_LICENCE`, `+licence` prints the lapse at `t0 + 171 d` and the gate errs; no
/// tick wrote the record in between; a buy then runs from now.
#[test]
fn sm5_the_licence_lapses_after_its_grace_at_the_next_act() {
    let mut rig = stocked();
    let (pilot, _) = dray_p(&mut rig, "Pilot", 1, "Salt Moth");
    let t0 = ic_now(&rig.tw);
    buy(&mut rig, pilot);
    let record = licence(&rig, pilot);
    assert_eq!(record.first_bought_ic, t0);
    assert_eq!(record.expires_ic, t0 + 168 * DAY);
    assert_eq!(record.lapses_ic(), Some(t0 + 171 * DAY));
    let _ = licence_writes(&mut rig.tw);

    // Day 170: in grace, which authorizes.
    skip_days(&mut rig, 170);
    assert_eq!(licence_gate(rig.tw.world(), pilot, ic_now(&rig.tw)), Ok(()));
    let out = at_occator(&mut rig, pilot, "+tv/lease go");
    assert!(
        out.contains("Occator Water & Propellant leases you "),
        "{out}"
    );

    // Day 172: lapsed since day 171.
    skip_days(&mut rig, 2);
    let out = at_occator(&mut rig, pilot, "+tv/lease");
    assert_eq!(out, LEASE_LICENCE);
    let out = at_occator(&mut rig, pilot, "+tv/lease go");
    assert_eq!(out, LEASE_LICENCE);
    // The ticks run past the expiry and the lapse (after the lease checks: across 172 IC
    // days their charter arm ends the idle charter, which the lease would read first):
    // the lapse is still read at the next act, never written by a tick.
    assert!(tacsim::port::live_charter(rig.tw.world(), pilot).is_some());
    tick_through(&mut rig);
    assert!(
        tacsim::port::live_charter(rig.tw.world(), pilot).is_none(),
        "the ticks ran: the charter arm ended the idle charter"
    );
    let out = run(&mut rig.tw, pilot, "+licence");
    assert_eq!(
        out,
        LICENCE_LAPSED.replace("{lapses}", &ic_stamp(t0 + 171 * DAY))
    );
    assert_eq!(
        licence_gate(rig.tw.world(), pilot, ic_now(&rig.tw)),
        Err(LicenceStanding::Lapsed {
            at_ic: t0 + 171 * DAY
        })
    );
    // Nothing wrote the record in between: not the port tick, the surface tick or the
    // economy sweep that ran across the expiry and the lapse, and no act.
    assert_eq!(licence(&rig, pilot), record);
    assert_eq!(
        licence_writes(&mut rig.tw),
        0,
        "the lapse is read, never written"
    );
    // A buy after the lapse runs from now.
    let now = ic_now(&rig.tw);
    let out = run(&mut rig.tw, pilot, "+licence/buy");
    assert_eq!(
        out,
        LICENCE_PREVIEW
            .replace("{issuer}", REGISTRY)
            .replace("{term}", "168")
            .replace("{until}", &ic_stamp(now + 168 * DAY))
            .replace("{fee}", FEE)
    );
    buy(&mut rig, pilot);
    let after = licence(&rig, pilot);
    assert_eq!(after.expires_ic, now + 168 * DAY);
    assert_eq!(after.terms, 2);
    assert_eq!(after.first_bought_ic, t0, "the first purchase is kept");
}

/// **A3: a buy while in force, or in grace, extends from the expiry** (§9.1): `expires_ic`
/// moves by exactly 168 IC days each time, `terms` counts 2 then 3.
#[test]
fn sm5_a_buy_while_in_force_or_in_grace_extends_from_the_expiry() {
    let mut rig = stocked();
    let (pilot, _) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let t0 = ic_now(&rig.tw);
    let out = buy(&mut rig, pilot);
    let first = licence(&rig, pilot);
    // The verb's reply is the last line; the economy tells the pilot of the contract's
    // offer and activation before it, as it does every party.
    let bought = LICENCE_BOUGHT
        .replace("{issuer}", REGISTRY)
        .replace("{until}", &ic_stamp(t0 + 168 * DAY))
        .replace("{fee}", FEE)
        .replace("{account}", &first.entitlement.to_string());
    assert_eq!(out.lines().last(), Some(bought.as_str()), "{out}");
    assert_eq!(first.terms, 1);
    // In force: from the expiry.
    let preview = run(&mut rig.tw, pilot, "+licence/buy");
    assert_eq!(
        preview,
        LICENCE_PREVIEW_EXTEND
            .replace("{issuer}", REGISTRY)
            .replace("{term}", "168")
            .replace("{from}", &ic_stamp(t0 + 168 * DAY))
            .replace("{until}", &ic_stamp(t0 + 336 * DAY))
            .replace("{fee}", FEE)
    );
    let out = run(&mut rig.tw, pilot, "+licence/buy go");
    let extended = LICENCE_EXTENDED
        .replace("{issuer}", REGISTRY)
        .replace("{until}", &ic_stamp(t0 + 336 * DAY))
        .replace("{fee}", FEE);
    assert_eq!(out.lines().last(), Some(extended.as_str()), "{out}");
    let second = licence(&rig, pilot);
    assert_eq!(second.terms, 2);
    assert_eq!(second.expires_ic, first.expires_ic + 168 * DAY);
    assert_eq!(
        second.entitlement, first.entitlement,
        "one entitlement account"
    );
    // In grace (a day past the expiry): still from the expiry.
    skip_days(&mut rig, 337);
    assert!(matches!(
        licence_gate(rig.tw.world(), pilot, ic_now(&rig.tw)),
        Ok(())
    ));
    buy(&mut rig, pilot);
    let third = licence(&rig, pilot);
    assert_eq!(third.terms, 3);
    assert_eq!(third.expires_ic, second.expires_ic + 168 * DAY);
    assert_eq!(third.first_bought_ic, t0);
    assert_eq!(entitlements(&rig.tw, pilot).len(), 1);
}

/// **A3: the fee is a Public contract the chronicle records** (§9.1; D-20-12): the contract
/// is `Public` and Ready at its activation, so it settles on the **first** economy sweep
/// after the buy — one sweep period on, pinned exactly (the lead's pre-commit round, item
/// 6(a)) — the Registry's account rises by exactly 250,000 minor and the pilot's falls by as
/// much, and canon holds one `economy.settled` line naming the contract — once.
#[test]
fn sm5_the_fee_is_a_public_contract_the_chronicle_records() {
    let mut rig = stocked();
    let (pilot, _) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let registry = org_account(&rig.tw, "freeport_registry");
    let account = credit_of(&rig.tw, pilot);
    let (registry_before, _) = balances(&rig.tw, registry);
    let (pilot_before, _) = balances(&rig.tw, account);
    buy(&mut rig, pilot);
    let contract = ContractId(licence(&rig, pilot).contract);
    let terms = economy(&rig.tw)
        .snapshot()
        .contract(contract)
        .cloned()
        .expect("the fee contract");
    assert_eq!(terms.consent.visibility, EconomicVisibility::Public);
    assert_eq!(terms.issuer, PrincipalId::Object(pilot));
    let settled = |tw: &TestWorld| {
        tw.world()
            .module_state::<mush_canon::CanonState>()
            .expect("canon")
            .recent()
            .into_iter()
            .filter(|e| e.kind.as_str() == "economy.settled" && e.detail["contract"] == contract.0)
            .count()
    };
    assert_eq!(settled(&rig.tw), 0, "nothing chronicled before the sweep");
    assert!(
        !matches!(
            state_of(&rig.tw, contract),
            Some(ContractState::Settled { .. })
        ),
        "the buy itself settles nothing"
    );
    // One sweep period on: the first economy sweep after the buy.
    sweep(&mut rig.tw, mush_economy::SWEEP_PERIOD_SECS);
    assert!(
        matches!(
            state_of(&rig.tw, contract),
            Some(ContractState::Settled { .. })
        ),
        "settled on the first sweep: {:?}",
        state_of(&rig.tw, contract)
    );
    assert_eq!(
        balances(&rig.tw, registry).0,
        registry_before + FEE_MINOR,
        "the Registry's account rises by the fee"
    );
    assert_eq!(balances(&rig.tw, account), (pilot_before - FEE_MINOR, 0));
    assert_eq!(settled(&rig.tw), 1, "one chronicle line");
    sweep(&mut rig.tw, 61);
    assert_eq!(settled(&rig.tw), 1, "and only one");
}

/// **One licence an account** (D-18-25's per-account rule; C23A): another character on the
/// same gateway account is refused `LICENCE_OTHER` on the preview and the buy, writes
/// nothing, and reads `LICENCE_OTHER` on `+licence`.
#[test]
fn sm5_one_licence_an_account() {
    let mut rig = stocked();
    let (pilot, _) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    buy(&mut rig, pilot);
    let alt = rig.tw.spawn_player("Alt");
    let account = gateway(&rig, pilot);
    if let Ok(mut map) = rig.oracle.0.lock() {
        map.insert(alt, account);
    }
    let _ = licence_writes(&mut rig.tw);
    let other = LICENCE_OTHER.replace("{issuer}", REGISTRY);
    for line in ["+licence/buy", "+licence/buy go", "+licence"] {
        assert_eq!(run(&mut rig.tw, alt, line), other, "{line}");
    }
    assert!(entitlements(&rig.tw, alt).is_empty());
    assert_eq!(licence_writes(&mut rig.tw), 0);
    assert_eq!(licence(&rig, pilot).holder, pilot);
}

/// **The `+licence` lines** (§5, §9.1): none, in force, grace, lapsed and suspended, whole
/// lines; the help names the lane's fee in the house form. (An unreadable record's line is
/// `sm5_an_unreadable_licence_reads_dark_and_bars_only_its_account`'s.)
#[test]
fn sm5_the_licence_show_lines() {
    let mut rig = stocked();
    let (pilot, _) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    assert_eq!(
        run(&mut rig.tw, pilot, "+licence"),
        LICENCE_NONE
            .replace("{issuer}", REGISTRY)
            .replace("{fee}", FEE)
            .replace("{term}", "168")
    );
    assert_eq!(
        run(&mut rig.tw, pilot, "+licence/show"),
        run(&mut rig.tw, pilot, "+licence")
    );
    let t0 = ic_now(&rig.tw);
    buy(&mut rig, pilot);
    let record = licence(&rig, pilot);
    let account = record.entitlement.to_string();
    skip_days(&mut rig, 10);
    assert_eq!(
        run(&mut rig.tw, pilot, "+licence"),
        LICENCE_IN_FORCE
            .replace("{issuer}", REGISTRY)
            .replace("{account}", &account)
            .replace("{until}", &ic_stamp(t0 + 168 * DAY))
            .replace("{days}", "158")
            .replace("{grace}", "3")
    );
    skip_days(&mut rig, 159);
    assert_eq!(
        run(&mut rig.tw, pilot, "+licence"),
        LICENCE_GRACE
            .replace("{issuer}", REGISTRY)
            .replace("{account}", &account)
            .replace("{until}", &ic_stamp(t0 + 168 * DAY))
            .replace("{lapses}", &ic_stamp(t0 + 171 * DAY))
    );
    skip_days(&mut rig, 2);
    assert_eq!(
        run(&mut rig.tw, pilot, "+licence"),
        LICENCE_LAPSED.replace("{lapses}", &ic_stamp(t0 + 171 * DAY))
    );
    // Suspended: staff freeze a second pilot's entitlement account.
    let (second, _) = chartered(&mut rig, "Second", 2, "Salt Wind");
    buy(&mut rig, second);
    let frozen = licence(&rig, second).entitlement;
    let ok = support::with_ctx(&mut rig.tw, |ctx| {
        mush_economy::set_account_status(
            ctx,
            mush_economy::AccountId(frozen),
            AccountStatus::Frozen,
        )
    });
    assert!(ok, "staff froze the entitlement account");
    let suspended = LICENCE_SUSPENDED
        .replace("{issuer}", REGISTRY)
        .replace("{account}", &frozen.to_string());
    assert_eq!(run(&mut rig.tw, second, "+licence"), suspended);
    assert_eq!(run(&mut rig.tw, second, "+licence/buy"), suspended);
    assert_eq!(
        licence_gate(rig.tw.world(), second, ic_now(&rig.tw)),
        Err(LicenceStanding::Suspended)
    );
    // The help spells the lane's fee and terms in the house form.
    let block = port_def("ceres_port").licence.expect("the licence block");
    assert!(LICENCE_HELP.contains(&format!("{}.00cr", block.fee_major)));
    assert!(LICENCE_HELP.contains(&format!("for {} IC days", block.term_ic_days)));
    assert!(LICENCE_HELP.contains(&format!("then {} IC days' grace", block.grace_ic_days)));
    assert!(!LICENCE_HELP.contains("2,500"));
}

/// **A lapsed licence refuses the lease, never the term settle** (doc 20b RK-1; SM.5's
/// decision 21 beside SM.3a's D8): a rig leased in the licence's grace runs past its term
/// while the licence lapses; at the pilot's next `+tv/lease` the preview shows the re-debit
/// before the licence's line and writes nothing, the `go` re-debits the term and closes with
/// the licence's line, nothing more leased, and a second `go` re-debits nothing (the plain
/// refusal).
#[test]
fn rk1_a_lapsed_licence_still_settles_a_past_term_rig() {
    let mut rig = stocked();
    let (pilot, _) = dray_p(&mut rig, "Pilot", 1, "Salt Moth");
    buy(&mut rig, pilot);
    skip_days(&mut rig, 170);
    let out = at_occator(&mut rig, pilot, "+tv/lease go");
    assert!(
        out.contains("Occator Water & Propellant leases you "),
        "{out}"
    );
    let leased = store(&rig.tw)
        .tv_leases_of(pilot)
        .next()
        .expect("the lease")
        .clone();
    // A day past the term; the licence lapsed at day 171.
    skip_days(&mut rig, 169);
    assert!(ic_now(&rig.tw) > leased.term_ends_ic);
    let account = credit_of(&rig.tw, pilot);
    let before = balances(&rig.tw, account);
    let preview = at_occator(&mut rig, pilot, "+tv/lease");
    assert!(preview.contains("Occator re-debits "), "{preview}");
    assert!(preview.ends_with(LEASE_LICENCE), "{preview}");
    assert_eq!(
        balances(&rig.tw, account),
        before,
        "a preview writes nothing"
    );
    let out = at_occator(&mut rig, pilot, "+tv/lease go");
    assert!(out.contains("Occator re-debits "), "{out}");
    assert!(out.ends_with(LEASE_LICENCE), "{out}");
    let renewed = store(&rig.tw)
        .tv_leases_of(pilot)
        .next()
        .expect("the lease")
        .clone();
    assert_eq!(renewed.terms_paid, leased.terms_paid + 1);
    assert_eq!(renewed.term_ends_ic, leased.term_ends_ic + leased.term_secs);
    assert_eq!(balances(&rig.tw, account).0, before.0 - leased.fee_minor);
    assert_eq!(store(&rig.tw).tv_leases_of(pilot).count(), 1);
    let again = at_occator(&mut rig, pilot, "+tv/lease go");
    assert_eq!(again, LEASE_LICENCE, "settled: nothing more is due");
    assert_eq!(balances(&rig.tw, account).0, before.0 - leased.fee_minor);
}

/// **A4: a lease needs a licence** (§4.3, §9.1; lead ruling 6): `+tv/lease go` at Occator's
/// counter without one refuses `LEASE_LICENCE` and writes nothing; with one it leases.
#[test]
fn sm5_a_lease_needs_a_licence() {
    let mut rig = stocked();
    let (pilot, _) = dray_p(&mut rig, "Pilot", 1, "Salt Moth");
    let account = credit_of(&rig.tw, pilot);
    let before = balances(&rig.tw, account);
    for line in ["+tv/lease", "+tv/lease go"] {
        assert_eq!(at_occator(&mut rig, pilot, line), LEASE_LICENCE, "{line}");
    }
    assert_eq!(store(&rig.tw).tv_leases_of(pilot).count(), 0);
    assert_eq!(balances(&rig.tw, account), before);
    buy(&mut rig, pilot);
    let out = at_occator(&mut rig, pilot, "+tv/lease go");
    assert!(
        out.contains("Occator Water & Propellant leases you "),
        "{out}"
    );
    assert_eq!(store(&rig.tw).tv_leases_of(pilot).count(), 1);
}

/// `+licence/buy` and `+licence/buy go` as `who` answer exactly `want`, and nothing is
/// written: no licence document, no record by holder, no entitlement account, the credit
/// account (when there is one) untouched.
fn refuses_to_buy(rig: &mut Rig, who: ObjId, want: &str) {
    let credit = tacsim::port::credit::own_account(rig.tw.world(), who).map(|a| a.id);
    let before = credit.map(|id| balances(&rig.tw, id));
    let _ = licence_writes(&mut rig.tw);
    for line in ["+licence/buy", "+licence/buy go"] {
        assert_eq!(run(&mut rig.tw, who, line), want, "{line}");
    }
    assert_eq!(licence_writes(&mut rig.tw), 0, "no licence document");
    assert!(store(&rig.tw).licence_of_holder(who).is_none(), "no record");
    assert!(
        entitlements(&rig.tw, who).is_empty(),
        "no entitlement account"
    );
    assert_eq!(
        credit.map(|id| balances(&rig.tw, id)),
        before,
        "nothing paid"
    );
}

/// **`/buy`'s refusal ladder writes nothing** (§9.1; the brief's checks 1 and 3, and the
/// lane without a licence block): a character the Registry cannot match to a gateway
/// account is refused `LICENCE_NO_MATCH` (fail closed — never the character's own account);
/// a matched pilot with no credit account `LICENCE_NO_CREDIT`; a chartered pilot short of
/// the fee `LICENCE_FUNDS` with what they have; and on a lane whose ports author no licence
/// block every form answers `LICENCE_NOT_HERE`. Each writes nothing.
#[test]
fn sm5_a_refused_buy_writes_nothing() {
    let mut rig = stocked();
    // 1. No gateway account: fail closed.
    let stray = rig.tw.spawn_player("Stray");
    refuses_to_buy(&mut rig, stray, LICENCE_NO_MATCH);
    // 3. A matched pilot who never enrolled holds no credit account.
    let walker = pilot(&mut rig, "Walker", 2);
    refuses_to_buy(&mut rig, walker, LICENCE_NO_CREDIT);
    assert!(matches!(
        store(&rig.tw).licence_state(gateway(&rig, walker)),
        LicenceState::Absent
    ));
    // 3. A chartered pilot with 1000.00cr available, short of the 2500.00cr fee.
    let (poor, _) = chartered(&mut rig, "Poor", 3, "Salt Moth");
    let account = credit_of(&rig.tw, poor);
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let (posted, held) = balances(&rig.tw, account);
    let out = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@econ/pay #{}=#{treasury} {}",
            account.0,
            posted - held - 100_000
        ),
    );
    assert!(out.contains("Paid: transaction #"), "{out}");
    let (posted, held) = balances(&rig.tw, account);
    assert_eq!(posted - held, 100_000);
    refuses_to_buy(
        &mut rig,
        poor,
        &LICENCE_FUNDS
            .replace("{fee}", FEE)
            .replace("{available}", "1000.00cr"),
    );

    // A lane whose ports author no licence block: the Registry issues none on this world.
    let unlicensed = lane_variant(LANE_VERSION, |doc| {
        if let Some(def) = doc["entries"]["ceres_port"]["def"].as_object_mut() {
            def.remove("licence");
        }
    });
    let mut rig = boot(Roster {
        seed_lane: Some(unlicensed),
        shelf: None,
    });
    let anyone = pilot(&mut rig, "Pilot", 1);
    refuses_to_buy(&mut rig, anyone, LICENCE_NOT_HERE);
    for line in ["+licence", "+licence/show"] {
        assert_eq!(run(&mut rig.tw, anyone, line), LICENCE_NOT_HERE, "{line}");
    }
}

/// Every write so far folded into the rig's store; then the stored document at `key`.
fn stored(rig: &mut Rig, key: &str) -> Option<Value> {
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    rig.store
        .values()
        .find_map(|data| data.kv.iter().find(|(k, _)| k.as_str() == key))
        .map(|(_, value)| value.clone())
}

/// Overwrite the stored document at `key` as a damaged store would hold it (every write so
/// far folded first); the next reboot loads it.
fn damage(rig: &mut Rig, key: &str, value: Value) {
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    let slot = rig
        .store
        .values_mut()
        .find_map(|data| data.kv.iter_mut().find(|(k, _)| k.as_str() == key))
        .expect("the document is stored");
    slot.1 = value;
}

/// **A licence reads back across a reboot; an unreadable one reads dark and bars only its
/// account** (§9.1; the store's R8 rule): the record bought through the verb loads back
/// whole. A `tacsim:licence:` document that does not decode (the minimal body the write
/// path leaves on a serialize fault), or one that names another account than its key's (the
/// identity rule), keeps its key claimed: that account's `+licence`, `+licence/buy`,
/// `+licence/buy go` and `+tv/lease go` all answer `LICENCE_BOOKS_DARK`, the gate errs
/// `Unreadable`, nothing is written over the damaged document, and a pilot of another
/// account still buys.
#[test]
#[allow(clippy::too_many_lines)]
fn sm5_an_unreadable_licence_reads_dark_and_bars_only_its_account() {
    let mut rig = stocked();
    let (pilot, _) = dray_p(&mut rig, "Pilot", 1, "Salt Moth");
    buy(&mut rig, pilot);
    let record = licence(&rig, pilot);
    let shown = run(&mut rig.tw, pilot, "+licence");
    assert!(
        shown.starts_with("Your prospecting licence from the Freeport Registry, account #"),
        "{shown}"
    );
    let key = tacsim::port::licence::licence_key(gateway(&rig, pilot));
    assert_eq!(
        stored(&mut rig, &key),
        Some(serde_json::to_value(&record).expect("the record serializes"))
    );

    // The round trip: the record loads back whole.
    let mut rig = reboot(rig);
    assert_eq!(licence(&rig, pilot), record);
    assert_eq!(run(&mut rig.tw, pilot, "+licence"), shown);
    assert_eq!(licence_gate(rig.tw.world(), pilot, ic_now(&rig.tw)), Ok(()));

    let elsewhere = Licence {
        account: AccountId::mint("ElsewhereAcct", 9).expect("an account id"),
        ..record.clone()
    };
    let damaged = [
        (
            "a body that does not decode",
            serde_json::json!({ "schema": record.schema, "account": record.account }),
            "Salt Wind",
        ),
        (
            "a body naming another account than its key's",
            serde_json::to_value(&elsewhere).expect("serializes"),
            "Salt Spray",
        ),
    ];
    for (seed, (why, body, ship)) in (10_i64..).zip(damaged) {
        damage(&mut rig, &key, body.clone());
        rig = reboot(rig);
        assert!(
            matches!(
                store(&rig.tw).licence_state(gateway(&rig, pilot)),
                LicenceState::Unreadable
            ),
            "{why}"
        );
        assert_eq!(
            licence_gate(rig.tw.world(), pilot, ic_now(&rig.tw)),
            Err(LicenceStanding::Unreadable),
            "{why}"
        );
        for line in [
            "+licence",
            "+licence/show",
            "+licence/buy",
            "+licence/buy go",
        ] {
            assert_eq!(
                run(&mut rig.tw, pilot, line),
                LICENCE_BOOKS_DARK,
                "{why}: {line}"
            );
        }
        assert_eq!(
            at_occator(&mut rig, pilot, "+tv/lease go"),
            LICENCE_BOOKS_DARK,
            "{why}: the lease reads the licence dark"
        );
        assert_eq!(
            stored(&mut rig, &key),
            Some(body),
            "{why}: nothing is written over the damaged document"
        );
        // The key bars only its own account: another account's pilot buys.
        let (other, _) = chartered(&mut rig, &format!("Other{seed}"), seed, ship);
        buy(&mut rig, other);
        assert_eq!(
            licence_gate(rig.tw.world(), other, ic_now(&rig.tw)),
            Ok(()),
            "{why}"
        );
    }
}

/// **One refused intake never stalls the others** (§7.3; lead ruling L24's rule): with the
/// Freeport Exchange suspended, each of its three renewal-due ore bids is withdrawn, its post
/// refused and the row marked lapsed, and the post refused again moves nothing — after which
/// the row is passed over for the rest of the IC day, so Occator's ice intake renews at its
/// standing quantity within a few fires. Reinstated, past the next IC-day boundary, each of
/// the Exchange's rows comes back at one drip.
#[test]
fn sm5_a_refused_intake_never_stalls_the_others() {
    let mut rig = stocked();
    let exchange_goods = ["metal_ore", "carbon_ore", "strategic_ore"];
    let metal = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    let ice = bid_row(&rig.tw, "occator", "ice_ore");
    let at = metal.expires - 43_200;
    for row in ore_rows(&rig.tw).values() {
        assert!(
            (0..=86_400).contains(&(row.expires - at)),
            "every intake's renewal is due at once"
        );
    }
    let org = port_map(&rig.tw, "ceres_port").orgs[&cid("freeport_exchange")].org;
    let out = run(&mut rig.tw, GOD, &format!("@org/status #{org}=suspended"));
    assert!(!out.contains("Usage"), "{out}");
    rig.tw.set_now(at);
    let mut renewed = ice.clone();
    for _ in 0..12 {
        fire_quietly(&mut rig);
        renewed = bid_row(&rig.tw, "occator", "ice_ore");
        if renewed.listing != ice.listing {
            break;
        }
    }
    assert_ne!(
        renewed.listing, ice.listing,
        "Occator's intake renews while the Exchange's are refused"
    );
    assert_eq!(quantity(&rig.tw, renewed.listing), Some(240));
    let marked: BTreeMap<String, BidRow> = ore_rows(&rig.tw)
        .into_iter()
        .filter(|(good, _)| exchange_goods.contains(&good.as_str()))
        .collect();
    for (good, row) in &marked {
        assert_eq!(quantity(&rig.tw, row.listing), None, "{good}: withdrawn");
        assert!(row.expires <= rig.tw.world().now(), "{good}: marked lapsed");
    }
    // Passed over for the rest of the IC day: later fires leave them as they are.
    for _ in 0..4 {
        fire_quietly(&mut rig);
    }
    for (good, row) in &marked {
        assert_eq!(
            bid_row(&rig.tw, "freeport_exchange", good),
            *row,
            "{good}: passed over"
        );
    }

    // Reinstated, past the next boundary: each comes back at one drip, one act a fire.
    let out = run(&mut rig.tw, GOD, &format!("@org/status #{org}=active"));
    assert!(!out.contains("Usage"), "{out}");
    past_next_boundary(&mut rig, 60);
    for _ in 0..8 {
        fire_quietly(&mut rig);
    }
    for (good, per) in [("metal_ore", 40), ("carbon_ore", 20), ("strategic_ore", 2)] {
        let back = bid_row(&rig.tw, "freeport_exchange", good);
        assert_ne!(back.listing, marked[good].listing, "{good}: re-posted");
        assert_eq!(
            quantity(&rig.tw, back.listing),
            Some(per),
            "{good}: one drip"
        );
    }
}

/// **A lane has one licence issuer, and every site is its** (§9.1, D-20-12; phase 1 has one
/// instrument): through the pack loader, a second port's licence block is refused naming
/// the first, and a site whose issuer is another org than the licence block's is refused;
/// with no licence block, the site rule has nothing to hold a site to.
#[test]
fn sm5_a_lane_has_one_licence_issuer_and_every_site_is_its() {
    let refused = |edit: &dyn Fn(&mut Value)| {
        let json = lane_variant(LANE_VERSION + 1, |doc| edit(doc));
        load_pack(&json)
            .err()
            .map_or_else(|| "accepted".to_owned(), |e| e.to_string())
    };
    let second = |doc: &mut Value| {
        doc["entries"]["stickney_port"]["def"]["licence"] = serde_json::json!({
            "org": "kasei", "fee_major": 100, "term_ic_days": 30, "grace_ic_days": 1,
            "claim_stake_major": 2000, "claim_term_major": 2000
        });
    };
    let got = refused(&second);
    let want = "licence: port stickney_port also authors a licence block (port ceres_port's); a \
                lane has one licence issuer in phase 1";
    assert!(got.contains(want), "{got}\nwanted: {want}");
    let occator_issues = |doc: &mut Value| {
        doc["entries"]["ahuna_port"]["def"]["site"]["issuer"] = serde_json::json!("occator");
    };
    let got = refused(&occator_issues);
    let want = "site: issuer occator is not the lane's licence issuer freeport_registry";
    assert!(got.contains(want), "{got}\nwanted: {want}");
    assert_eq!(
        refused(&|doc| {
            if let Some(def) = doc["entries"]["ceres_port"]["def"].as_object_mut() {
                def.remove("licence");
            }
            occator_issues(doc);
        }),
        "accepted"
    );
}

// ---------------------------------------------------------------------------
// The sale and the share
// ---------------------------------------------------------------------------

/// **A5: ten tonnes of ore sell at the bid, the Trust at a quarter of the upper half**
/// (§7.4; Q-20-5): 700.00cr gross, 87.50cr to the Trust (a quarter of 35.00cr a tonne over
/// the fixed 35.00cr reference), 612.50cr net — read from the sale line and, after the sweep,
/// from the Trust's account.
#[test]
fn sm5_ten_tonnes_of_ore_sell_at_the_bid_with_the_trust_at_a_quarter_of_the_upper_half() {
    let mut rig = stocked();
    let (pilot, row) = dray_p(&mut rig, "Pilot", 1, "Salt Moth");
    let hold = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, hold, "metal_ore", 10, LotId(901));
    let trust = org_account(&rig.tw, "coop_trust");
    let (trust_before, _) = balances(&rig.tw, trust);
    let signatory = name_of(
        &rig.tw,
        ObjId(port_map(&rig.tw, "ceres_port").vendors[&cid("freeport_exchange")].signatory),
    );
    let out = sell(&mut rig, pilot, "freeport_exchange", "metal_ore", "10");
    let line = format!(
        "Sold 10 t of metal ore, run-of-mine to Freeport Exchange at 70.00cr/t: 700.00cr; Co-op \
         share 87.50cr; you receive 612.50cr when the goods are in. {signatory} countersigns."
    );
    assert!(out.lines().any(|l| l == line), "{out}\nwanted: {line}");
    let quote = market(&rig.tw)
        .quotes_of_seller(&PrincipalId::Object(pilot))
        .last()
        .cloned()
        .expect("the sale quote");
    let contract = quote.contract.expect("the sale contract");
    settle(&mut rig, contract);
    assert_eq!(balances(&rig.tw, trust).0, trust_before + 8_750);
    assert_eq!(pool(&mut rig.tw, hold, "metal_ore"), 0);
}

/// **A7: a hold of ore sells in one act against the cap** (§7.2; lead ruling 8): the bare
/// `+market/sell <listing>` previews what the bid wants, what the hold holds and what `=all`
/// sells; `=all` sells the 86 t; the bid's `+market` line reads `wanted 154 ×`; nothing is
/// left in the hold.
#[test]
fn sm5_a_hold_of_ore_sells_in_one_act_against_the_cap() {
    let mut rig = stocked();
    let (pilot, row) = dray_p(&mut rig, "Pilot", 1, "Salt Moth");
    let hold = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, hold, "metal_ore", 86, LotId(901));
    let hall = exchange(&rig.tw);
    teleport(&mut rig.tw, pilot, hall);
    let listing = bid_row(&rig.tw, "freeport_exchange", "metal_ore").listing;
    let out = run(&mut rig.tw, pilot, &format!("+market/sell {listing}"));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    assert!(lines[0].starts_with(&format!("#{listing} ")), "{out}");
    assert!(
        lines[0]
            .ends_with(": Freeport Exchange wants 240 more at 70.00cr each; you hold 86 there."),
        "{out}"
    );
    assert_eq!(
        lines[1],
        format!("+market/sell {listing}=all sells 86 for 6020.00cr.")
    );
    assert_eq!(
        quantity(&rig.tw, listing),
        Some(240),
        "the preview sold nothing"
    );
    assert_eq!(pool(&mut rig.tw, hold, "metal_ore"), 86);
    let out = sell(&mut rig, pilot, "freeport_exchange", "metal_ore", "all");
    assert!(out.contains("Sold 86 t of metal ore, run-of-mine"), "{out}");
    let list = run(&mut rig.tw, pilot, "+market/list");
    assert!(
        list.lines()
            .any(|l| l.starts_with(&format!("#{listing} ")) && l.contains(" wanted 154 × ")),
        "{list}"
    );
    assert_eq!(
        pool(&mut rig.tw, hold, "metal_ore"),
        0,
        "nothing is left in the hold"
    );
    // A second `all` with nothing held refuses with the reason.
    let out = run(&mut rig.tw, pilot, &format!("+market/sell {listing}=all"));
    assert!(out.contains(mush_market::SELL_NONE_HELD), "{out}");
}

// ---------------------------------------------------------------------------
// The intake drip
// ---------------------------------------------------------------------------

/// **A6: a filled intake drips back** (§7.3; lead ruling 4): a row that waited at its cap
/// across an IC-day boundary keeps one banked drip, spent by the first fire after a sale into
/// it (exactly one drip); a sale on the IC day of the post drips nothing before the next
/// boundary; past it, the first fire posts `min(cap, unfilled + drip)` and rewrites only that
/// row's `listing`, `expires` and `posted_ic`; one act a fire.
#[test]
#[allow(clippy::too_many_lines)]
fn sm5_a_filled_intake_drips_back_within_the_hour() {
    let mut rig = stocked();
    let (pilot, row) = dray_p(&mut rig, "Pilot", 1, "Salt Moth");
    let hold = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, hold, "metal_ore", 126, LotId(901));
    seed_pool(&mut rig, hold, "carbon_ore", 40, LotId(902));
    // Past a boundary: every intake still stands at its cap, so nothing re-posts, and each
    // row's last post stays on the earlier day — a banked drip.
    let boot_rows = ore_rows(&rig.tw);
    past_next_boundary(&mut rig, 60);
    fire_quietly(&mut rig);
    fire_quietly(&mut rig);
    assert_eq!(
        ore_rows(&rig.tw),
        boot_rows,
        "a row at its cap is left alone"
    );

    // The banked drip: 50 t sold leaves 190; the next fire posts 230, one drip.
    sell(&mut rig, pilot, "freeport_exchange", "metal_ore", "50");
    let metal = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    assert_eq!(quantity(&rig.tw, metal.listing), Some(190));
    fire_quietly(&mut rig);
    let banked = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    assert_ne!(banked.listing, metal.listing, "a new listing");
    assert_eq!(quantity(&rig.tw, banked.listing), Some(230));
    assert_eq!(
        quantity(&rig.tw, metal.listing),
        None,
        "the old one is withdrawn"
    );
    assert_eq!(
        banked.posted_ic.map(|p| p.div_euclid(DAY)),
        Some(ic_now(&rig.tw).div_euclid(DAY))
    );
    assert_eq!(
        (banked.price_major, banked.intake),
        (metal.price_major, metal.intake),
        "the authored terms are never the drip's"
    );
    settle_last(&mut rig, pilot);

    // A sale on the IC day of the post: nothing before the next boundary (the settling
    // sweeps fire the port tick on the same IC day).
    sell(&mut rig, pilot, "freeport_exchange", "metal_ore", "76");
    assert_eq!(quantity(&rig.tw, banked.listing), Some(154));
    settle_last(&mut rig, pilot);
    for _ in 0..3 {
        fire_quietly(&mut rig);
    }
    assert_eq!(bid_row(&rig.tw, "freeport_exchange", "metal_ore"), banked);
    assert_eq!(quantity(&rig.tw, banked.listing), Some(154));
    // Carbon sold the same day spends its own banked drip at once: 30 sold leaves 90, the
    // next fire posts 110.
    sell(&mut rig, pilot, "freeport_exchange", "carbon_ore", "30");
    fire_quietly(&mut rig);
    let carbon = bid_row(&rig.tw, "freeport_exchange", "carbon_ore");
    assert_eq!(quantity(&rig.tw, carbon.listing), Some(110));
    settle_last(&mut rig, pilot);

    // Past the next boundary: one act a fire, authored order.
    let before = ore_rows(&rig.tw);
    let alloy = bid_row(&rig.tw, "freeport_exchange", "structural_alloys");
    past_next_boundary(&mut rig, 60);
    fire_quietly(&mut rig);
    let after = ore_rows(&rig.tw);
    let dripped = &after["metal_ore"];
    assert_eq!(quantity(&rig.tw, dripped.listing), Some(194), "154 + 40");
    assert_ne!(dripped.listing, banked.listing);
    assert_eq!(
        BidRow {
            listing: banked.listing,
            expires: banked.expires,
            posted_ic: banked.posted_ic,
            ..dripped.clone()
        },
        banked,
        "only the listing, the deadline and the last post move"
    );
    for good in ["carbon_ore", "strategic_ore", "ice_ore"] {
        assert_eq!(after[good], before[good], "{good}: one act a fire");
    }
    assert_eq!(
        bid_row(&rig.tw, "freeport_exchange", "structural_alloys"),
        alloy
    );
    fire_quietly(&mut rig);
    let carbon = bid_row(&rig.tw, "freeport_exchange", "carbon_ore");
    assert_eq!(
        quantity(&rig.tw, carbon.listing),
        Some(120),
        "110 + 20, at the cap"
    );
    assert_eq!(ore_rows(&rig.tw)["metal_ore"], *dripped);
}

/// **A6: the drip never posts past capital** (§7.3; reviewer F10): with the Exchange's
/// available funds cut to 100 tonnes' worth of metal ore, a standing 154 t re-posts smaller
/// at 100; under a tonne's worth the standing listing is left exactly as it was, nothing is
/// posted, and the house is warned once a boot.
#[test]
fn sm5_the_drip_never_posts_past_capital() {
    let mut rig = stocked();
    let (pilot, row) = dray_p(&mut rig, "Pilot", 1, "Salt Moth");
    let hold = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, hold, "metal_ore", 86, LotId(901));
    past_next_boundary(&mut rig, 60);
    let exchange_account = org_account(&rig.tw, "freeport_exchange");
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let pay_down_to = |rig: &mut Rig, left: i64| {
        let (posted, held) = balances(&rig.tw, exchange_account);
        let out = run(
            &mut rig.tw,
            GOD,
            &format!(
                "@econ/pay #{}=#{treasury} {}",
                exchange_account.0,
                posted - held - left
            ),
        );
        assert!(out.contains("Paid: transaction #"), "{out}");
        let (posted, held) = balances(&rig.tw, exchange_account);
        assert_eq!(posted - held, left);
    };
    // The sale escrows 86 × 70.00cr; 100 t's worth is left available beside it.
    pay_down_to(&mut rig, 86 * 7_000 + 100 * 7_000);
    sell(&mut rig, pilot, "freeport_exchange", "metal_ore", "all");
    let standing = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    assert_eq!(quantity(&rig.tw, standing.listing), Some(154));
    fire_quietly(&mut rig);
    let smaller = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    assert_ne!(smaller.listing, standing.listing);
    assert_eq!(
        quantity(&rig.tw, smaller.listing),
        Some(100),
        "a standing bid never promises more than the house can pay"
    );
    // The sale settles before the clock moves on (an unsettled one would lapse and
    // refund its escrow to the house).
    settle_last(&mut rig, pilot);

    // Under a tonne: left as it is, warned once a boot.
    pay_down_to(&mut rig, 6_999);
    past_next_boundary(&mut rig, 60);
    let capture = Capture::default();
    tracing::dispatcher::with_default(&capture_dispatch(capture.clone()), || {
        for _ in 0..4 {
            fire_quietly(&mut rig);
        }
    });
    let now_row = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    assert_eq!(
        now_row,
        smaller,
        "posted {:?}; the house's (posted, held) {:?}",
        quantity(&rig.tw, now_row.listing),
        balances(&rig.tw, exchange_account)
    );
    assert_eq!(quantity(&rig.tw, smaller.listing), Some(100));
    let warned = capture
        .text()
        .lines()
        .filter(|l| {
            l.contains("cannot fund a unit of its ore intake") && l.contains("good=metal_ore")
        })
        .count();
    assert_eq!(warned, 1, "once a boot:\n{}", capture.text());
}

/// **An untouched intake renews before the market's TTL** (reviewer F2; lead decisions 4a,
/// 4d, 4e, 20): an ore bid never sold into is re-posted at its standing quantity 12 real
/// hours before its recorded deadline, and still stands a day past the old deadline; one the
/// market removed past its deadline comes back at one drip (the readiness line Pending
/// before); one withdrawn by hand before its deadline waits for that deadline.
#[test]
#[allow(clippy::too_many_lines)]
fn sm5_an_untouched_intake_renews_before_the_market_ttl() {
    let mut rig = stocked();
    let first = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    assert_eq!(quantity(&rig.tw, first.listing), Some(240));
    rig.tw.set_now(first.expires - 43_200);
    fire_quietly(&mut rig);
    let renewed = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    assert_ne!(renewed.listing, first.listing);
    assert_eq!(
        quantity(&rig.tw, renewed.listing),
        Some(240),
        "at its standing quantity"
    );
    assert_eq!(
        renewed.expires,
        rig.tw.world().now() + mush_market::MAX_LISTING_TTL_SECS
    );
    rig.tw.set_now(first.expires + DAY);
    fire_quietly(&mut rig);
    assert_eq!(
        quantity(&rig.tw, renewed.listing),
        Some(240),
        "it still stands"
    );

    // The lapse: the market's TTL sweep removed it past its deadline.
    rig.tw.set_now(renewed.expires + 60);
    support::with_ctx(&mut rig.tw, |ctx| {
        mush_market::withdraw_listing(ctx, ListingId(renewed.listing))
    })
    .expect("the listing is removed");
    let report = status(&mut rig.tw);
    let gone = format!(
        "  ceres_port bid freeport_exchange metal_ore: listing #{} is gone; the clerk posts it \
         again at the next drip. [pending]",
        renewed.listing
    );
    assert!(report.lines().any(|l| l == gone), "{report}");
    let mut lapsed = renewed.clone();
    for _ in 0..8 {
        fire_quietly(&mut rig);
        lapsed = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
        if lapsed.listing != renewed.listing {
            break;
        }
    }
    assert_ne!(lapsed.listing, renewed.listing, "re-posted");
    assert_eq!(quantity(&rig.tw, lapsed.listing), Some(40), "one drip");

    // Withdrawn by hand before its deadline: not re-posted until that deadline.
    support::with_ctx(&mut rig.tw, |ctx| {
        mush_market::withdraw_listing(ctx, ListingId(lapsed.listing))
    })
    .expect("withdrawn by hand");
    // The readiness line names the recorded deadline it waits for, not the next drip (the
    // lead's pre-commit round, item 3(b)), its real span in `min`.
    let report = status(&mut rig.tw);
    let waits = format!(
        "  ceres_port bid freeport_exchange metal_ore: listing #{} was withdrawn; the clerk posts \
         it again at its deadline, ",
        lapsed.listing
    );
    assert!(
        report.lines().any(|l| l.starts_with(&waits)
            && l.contains(" UTC on ")
            && l.ends_with(" min). [pending]")),
        "{report}"
    );
    for _ in 0..4 {
        fire_quietly(&mut rig);
    }
    assert_eq!(bid_row(&rig.tw, "freeport_exchange", "metal_ore"), lapsed);
    rig.tw.set_now(lapsed.expires + 60);
    let mut back = lapsed.clone();
    for _ in 0..8 {
        fire_quietly(&mut rig);
        back = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
        if back.listing != lapsed.listing {
            break;
        }
    }
    assert_ne!(
        back.listing, lapsed.listing,
        "posted once its deadline passed"
    );
    assert_eq!(quantity(&rig.tw, back.listing), Some(40));
}

/// **A drip post refused after its withdraw is re-posted on the next fire** (case 5's
/// bookkeeping): the Exchange suspended for one fire, its renewal-due metal ore bid is
/// withdrawn and the post refused, so the row reads lapsed (`expires = now`); reinstated,
/// the next fire posts one drip.
#[test]
fn sm5_a_refused_drip_post_after_its_withdraw_is_reposted_next_fire() {
    let mut rig = stocked();
    let first = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    let org = port_map(&rig.tw, "ceres_port").orgs[&cid("freeport_exchange")].org;
    let out = run(&mut rig.tw, GOD, &format!("@org/status #{org}=suspended"));
    assert!(!out.contains("Usage"), "{out}");
    rig.tw.set_now(first.expires - 43_200);
    fire_quietly(&mut rig);
    let marked = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    assert_eq!(
        quantity(&rig.tw, first.listing),
        None,
        "the withdraw landed"
    );
    assert_eq!(marked.listing, first.listing);
    assert_eq!(marked.expires, rig.tw.world().now(), "the row reads lapsed");
    let out = run(&mut rig.tw, GOD, &format!("@org/status #{org}=active"));
    assert!(!out.contains("Usage"), "{out}");
    fire_quietly(&mut rig);
    let back = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    assert_ne!(back.listing, first.listing);
    assert_eq!(quantity(&rig.tw, back.listing), Some(40), "one drip");
}

// ---------------------------------------------------------------------------
// The lane
// ---------------------------------------------------------------------------

/// **A8: a fixed reference for an asked good is refused at load** (§7.4, Q-20-5), and so is
/// a good in both maps — through the pack loader, the exact sentences.
#[test]
fn sm5_a_fixed_reference_for_an_asked_good_is_refused_at_load() {
    let refused = |edit: &dyn Fn(&mut Value)| {
        let json = lane_variant(LANE_VERSION + 1, |doc| edit(doc));
        load_pack(&json)
            .err()
            .map_or_else(|| "accepted".to_owned(), |e| e.to_string())
    };
    assert_eq!(refused(&|_| {}), "accepted");
    // `metals` is asked (the Exchange's concentrate): moved from the listing reference to a
    // fixed one, the fixed one is refused naming the first port that asks it.
    let pack = lane();
    let asker = pack
        .entries()
        .iter()
        .find_map(|(id, entry)| match entry {
            PackEntry::Port(def)
                if def
                    .vendors
                    .iter()
                    .any(|v| v.asks.iter().any(|a| a.good.as_str() == "metals")) =>
            {
                Some(id.clone())
            }
            _ => None,
        })
        .expect("a port asks metals");
    let got = refused(&|doc| {
        let programme = &mut doc["entries"]["ceres_port"]["def"]["programme"];
        if let Some(map) = programme["reference"].as_object_mut() {
            map.remove("metals");
        }
        programme["reference_minor"]["metals"] = serde_json::json!(10_000);
    });
    let want = format!(
        "programme coop.first_captain: reference_minor metals: a fixed reference is only for a \
         good no port of this lane asks, and port {asker} asks it"
    );
    assert!(got.contains(&want), "{got}\nwanted: {want}");
    let got = refused(&|doc| {
        doc["entries"]["ceres_port"]["def"]["programme"]["reference_minor"]["volatiles"] =
            serde_json::json!(5_000);
    });
    let want = "programme coop.first_captain: volatiles has both a reference listing and a fixed \
                reference";
    assert!(got.contains(want), "{got}\nwanted: {want}");
}

/// **A11: Occator's capital rises by one appended tranche on a kept world** (§7.3, §12.3;
/// lead ruling 3): a world latched at v10 applying v11 makes exactly one new 250,000.00cr
/// tranche and records the raise; its Org line then reads 300000.00cr, 2 of 2; a fresh v11
/// world holds the same two; a later version that edits the recorded raise is refused as
/// drift and applies nothing.
#[test]
#[allow(clippy::too_many_lines)]
fn sm5_occators_capital_rises_by_a_tranche_on_a_kept_world() {
    // A fresh v11 world: two tranches, the raise recorded.
    let fresh = stocked();
    let occator = port_map(&fresh.tw, "ceres_port").orgs[&cid("occator")].clone();
    assert_eq!(occator.raises, vec![250_000]);
    assert_eq!(
        occator
            .capital
            .iter()
            .map(|t| (t.n, t.amount_minor))
            .collect::<Vec<_>>(),
        vec![(0, 5_000_000), (1, 25_000_000)]
    );
    drop(fresh);

    let roster = Roster {
        seed_lane: Some(lane_variant(10, strip_ore)),
        shelf: Some(vec![
            mush_content::load_pack(LANE_JSON).expect("the shipped lane verifies"),
        ]),
    };
    let mut rig = boot(roster);
    assert_eq!(
        store(&rig.tw).applied_in(rig.tw.world(), &lane_id()),
        Some(10)
    );
    let v10 = port_map(&rig.tw, "ceres_port").orgs[&cid("occator")].clone();
    assert_eq!(v10.capital.len(), 1);
    assert!(v10.raises.is_empty());
    let before: Vec<u64> = economy(&rig.tw)
        .snapshot()
        .obligations()
        .map(|o| o.id.0)
        .collect();
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@{LANE_VERSION}")),
        "{plan}\n{apply}"
    );
    let made: Vec<mush_economy::Obligation> = economy(&rig.tw)
        .snapshot()
        .obligations()
        .filter(|o| !before.contains(&o.id.0))
        .cloned()
        .collect();
    // Doc 20 cut SM.10: v12 also adds the Pallas Syndicate (capital 50,000), whose base
    // tranche joins R9b's 100,000-credit Fleet base; Occator's raise is still exactly one.
    assert_eq!(
        made.len(),
        3,
        "Occator's one new tranche, the Syndicate and the Fleet bases"
    );
    let made: Vec<mush_economy::Obligation> = made
        .into_iter()
        .filter(|o| ![5_000_000, 10_000_000].contains(&o.amount.minor()))
        .collect();
    assert_eq!(made.len(), 1, "exactly one new Occator tranche");
    assert_eq!(made[0].amount.minor(), 25_000_000);
    let row = port_map(&rig.tw, "ceres_port").orgs[&cid("occator")].clone();
    assert_eq!(row.raises, vec![250_000]);
    assert_eq!(row.capital.len(), 2);
    assert_eq!(row.capital[0], v10.capital[0], "the base tranche stands");
    assert_eq!(row.capital[1].obligation, made[0].id.0);
    sweep(&mut rig.tw, 120);
    let report = status(&mut rig.tw);
    let line = format!(
        "  ceres_port org occator: Occator Water & Propellant, org #{}, account #{}, capital \
         300000.00cr: 2 of 2 tranches settled. [ready]",
        row.org,
        row.account.expect("an account")
    );
    assert!(
        report.lines().any(|l| l == line),
        "{report}\nwanted: {line}"
    );

    // A later version that edits the recorded raise: drift, nothing applied.
    let edited = mush_content::load_pack(&lane_variant(LANE_VERSION + 1, |doc| {
        if let Some(orgs) = doc["entries"]["ceres_port"]["def"]["orgs"].as_array_mut() {
            for org in orgs.iter_mut() {
                if org["id"] == "occator" {
                    org["raises"] = serde_json::json!([200_000]);
                }
            }
        }
    }))
    .expect("the edited variant verifies");
    rig.roster.shelf = Some(vec![edited]);
    let mut rig = reboot(rig);
    let obligations = economy(&rig.tw).snapshot().obligations().count();
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    let want = "ceres_port: org occator's raises [200000] do not extend the [250000] already \
                issued; a port update never re-capitalizes";
    assert!(
        apply.contains(want) || plan.contains(want),
        "{plan}\n{apply}\nwanted: {want}"
    );
    assert!(!apply.contains(&format!("Applied {LANE_PACK}@")), "{apply}");
    assert_eq!(
        store(&rig.tw).applied_in(rig.tw.world(), &lane_id()),
        Some(LANE_VERSION)
    );
    assert_eq!(
        economy(&rig.tw).snapshot().obligations().count(),
        obligations
    );
    assert_eq!(port_map(&rig.tw, "ceres_port").orgs[&cid("occator")], row);
}

/// **A12: the readiness report names the licence and the ore intakes** (lead ruling 10):
/// the Licence line Ready, the four ore Bids lines Ready with their drip, and `+coop/enrol`
/// open.
#[test]
fn sm5_the_readiness_report_names_the_licence_and_the_ore_intakes() {
    let mut rig = stocked();
    let report = status(&mut rig.tw);
    let licence = "  ceres_port licence tacsim.licence.freeport: the Freeport Registry issues it \
                   at 2500.00cr for 168 IC days with 3 IC days' grace; extraction reads it. \
                   [ready]";
    assert!(report.lines().any(|l| l == licence), "{report}");
    for (org, good, price, cap, per) in [
        ("freeport_exchange", "metal_ore", "70.00cr", 240, 40),
        ("freeport_exchange", "carbon_ore", "50.00cr", 120, 20),
        ("freeport_exchange", "strategic_ore", "600.00cr", 12, 2),
        ("occator", "ice_ore", "35.00cr", 240, 40),
    ] {
        let listing = bid_row(&rig.tw, org, good).listing;
        let line = format!(
            "  ceres_port bid {org} {good}: listing #{listing} at {price}/t, {cap} of {cap} t \
             wanted; {per} t more each IC day. [ready]"
        );
        assert!(
            report.lines().any(|l| l == line),
            "{report}\nwanted: {line}"
        );
    }
    let readiness = tacsim::port::readiness(rig.tw.world());
    assert!(tacsim::port::enrol_gate(&readiness).is_ok());
    assert!(!tacsim::port::gated(tacsim::port::Binding::Licence));
    let (pilot, _) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    assert!(tacsim::port::live_charter(rig.tw.world(), pilot).is_some());
}

/// **`strip_ore` rebuilds the published v10** (lead decision 1): the v10 shape's digest is
/// SM.3a's.
#[test]
fn sm5_strip_ore_rebuilds_the_published_v10() {
    let pack = load_pack(&lane_variant(10, strip_ore)).expect("the v10 shape verifies");
    assert_eq!(pack.manifest().digest().to_hex(), LANE_V10_DIGEST);
}

/// **Only Ceres bids ore** (§7.3): the four ore bids are the Exchange's three and Occator's
/// one, each with its refresh; no other vendor bids, asks or stocks an ore good.
#[test]
fn sm5_only_ceres_bids_ore() {
    let pack = lane();
    let mut bids = Vec::new();
    for (id, entry) in pack.entries() {
        let PackEntry::Port(def) = entry else {
            continue;
        };
        for vendor in &def.vendors {
            for bid in &vendor.bids {
                if ORE_GOODS.contains(&bid.good.as_str()) {
                    bids.push((
                        id.to_string(),
                        vendor.org.to_string(),
                        bid.good.to_string(),
                        bid.price_major,
                        bid.intake,
                        bid.refresh.map(|r| (r.per_ic_day, r.cap)),
                    ));
                }
            }
            assert!(
                !vendor
                    .asks
                    .iter()
                    .any(|a| ORE_GOODS.contains(&a.good.as_str()))
                    && !vendor
                        .stock
                        .iter()
                        .any(|s| ORE_GOODS.contains(&s.good.as_str())),
                "{id} {}: nobody asks or stocks ore",
                vendor.org
            );
        }
    }
    let ceres = |org: &str, good: &str, price, intake, per, cap| {
        (
            "ceres_port".to_owned(),
            org.to_owned(),
            good.to_owned(),
            price,
            intake,
            Some((per, cap)),
        )
    };
    assert_eq!(
        bids,
        vec![
            ceres("freeport_exchange", "metal_ore", 70, 240, 40, 240),
            ceres("freeport_exchange", "carbon_ore", 50, 120, 20, 120),
            ceres("freeport_exchange", "strategic_ore", 600, 12, 2, 12),
            ceres("occator", "ice_ore", 35, 240, 40, 240),
        ]
    );
    let programme = port_def("ceres_port").programme.expect("the programme");
    assert_eq!(
        programme
            .reference_minor
            .iter()
            .map(|(good, minor)| (good.to_string(), *minor))
            .collect::<Vec<_>>(),
        vec![
            ("carbon_ore".to_owned(), 2_500),
            ("ice_ore".to_owned(), 1_750),
            ("metal_ore".to_owned(), 3_500),
            ("strategic_ore".to_owned(), 30_000),
        ]
    );
}

// ---------------------------------------------------------------------------
// The lead's pre-commit round
// ---------------------------------------------------------------------------

/// **A pack apply brings a lapsed intake back at one drip too** (decision 20; the lead's
/// pre-commit round, item 2; residual F4): on a world latched at the shipped lane, the
/// Exchange's metal ore intake lapses (past its deadline the market removed it, and no fire
/// ran since); staff then apply a later version that edits only the licence block's grace —
/// a test-only pack, never published — so the port applier updates `ceres_port` and its
/// part (e).8 re-posts the lapsed row. It comes back at exactly one drip (40 t), as the
/// drip's case 4 would post it, never at its 240 t cap; the row keeps its authored terms and
/// records the apply's IC instant, and the drip posts nothing more that IC day.
#[test]
fn sm5_a_pack_apply_brings_a_lapsed_intake_back_at_one_drip() {
    let later = mush_content::load_pack(&lane_variant(LANE_VERSION + 1, |doc| {
        doc["entries"]["ceres_port"]["def"]["licence"]["grace_ic_days"] = serde_json::json!(4);
    }))
    .expect("the later variant verifies");
    let mut rig = boot(Roster {
        seed_lane: None,
        shelf: Some(vec![later]),
    });
    stock(&mut rig);
    set_rate(&mut rig.tw, 1);
    assert_eq!(
        store(&rig.tw).applied_in(rig.tw.world(), &lane_id()),
        Some(LANE_VERSION)
    );
    let first = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    assert_eq!(quantity(&rig.tw, first.listing), Some(240));
    // The lapse: past its deadline the market removed it; no fire has run since.
    rig.tw.set_now(first.expires + 60);
    support::with_ctx(&mut rig.tw, |ctx| {
        mush_market::withdraw_listing(ctx, ListingId(first.listing))
    })
    .expect("the listing is removed");
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@{}", LANE_VERSION + 1)),
        "{plan}\n{apply}"
    );
    let back = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    assert_ne!(back.listing, first.listing, "the apply re-posted it");
    assert_eq!(
        quantity(&rig.tw, back.listing),
        Some(40),
        "one drip, not the 240 t cap"
    );
    assert_eq!(
        (back.price_major, back.intake),
        (first.price_major, first.intake),
        "the row keeps the authored terms"
    );
    assert_eq!(back.posted_ic, Some(ic_now(&rig.tw)));
    // The drip posts nothing more on the apply's IC day.
    for _ in 0..3 {
        fire_quietly(&mut rig);
    }
    assert_eq!(bid_row(&rig.tw, "freeport_exchange", "metal_ore"), back);
    assert_eq!(quantity(&rig.tw, back.listing), Some(40));
}

/// The lane edited to bid `price` major units for `good` at the Freeport Exchange.
fn exchange_bid_priced(doc: &mut Value, good: &str, price: u64) {
    let vendors = doc["entries"]["ceres_port"]["def"]["vendors"]
        .as_array_mut()
        .expect("the port's vendors");
    let bid = vendors
        .iter_mut()
        .find(|v| v["org"] == "freeport_exchange")
        .and_then(|v| v["bids"].as_array_mut())
        .and_then(|bids| bids.iter_mut().find(|b| b["good"] == good))
        .expect("the Exchange bids for the good");
    bid["price_major"] = serde_json::json!(price);
}

/// **An apply revises a refresh row within the house's funds** (doc 20b RK-1; SM.5's R5
/// (ii)): staff apply a later version that re-prices the Exchange's metal ore bid 70.00cr →
/// 80.00cr while the Exchange can fund only 50 t at that price. Part (e).8's revise arm
/// re-prices the standing 240 t bid at 50 t, as the drip's case 3 would. An unrecorded row
/// (the carbon ore row, its listing and row removed by hand) still posts its full 120 t
/// intake, as every first apply does before its houses' capital settles (R5 (i), kept).
#[test]
fn rk1_an_apply_revises_a_refresh_row_within_the_houses_funds() {
    let later = mush_content::load_pack(&lane_variant(LANE_VERSION + 1, |doc| {
        exchange_bid_priced(doc, "metal_ore", 80);
    }))
    .expect("the later variant verifies");
    let mut rig = boot(Roster {
        seed_lane: None,
        shelf: Some(vec![later]),
    });
    stock(&mut rig);
    set_rate(&mut rig.tw, 1);
    let metal = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    assert_eq!(quantity(&rig.tw, metal.listing), Some(240));
    let carbon = bid_row(&rig.tw, "freeport_exchange", "carbon_ore");
    let key = tacsim::port::port_map_key(&lane_id(), &cid("ceres_port"));
    let mut map = stored(&mut rig, &key).expect("the port map is stored");
    let removed = map["vendors"]["freeport_exchange"]["bids"]
        .as_object_mut()
        .and_then(|bids| bids.remove("carbon_ore"));
    assert!(removed.is_some(), "the row was recorded");
    damage(&mut rig, &key, map);
    let mut rig = reboot(rig);
    support::with_ctx(&mut rig.tw, |ctx| {
        mush_market::withdraw_listing(ctx, ListingId(carbon.listing))
    })
    .expect("the carbon ore listing is withdrawn");
    // 50 t at 80.00cr; 80 t at 50.00cr.
    pay_house_down_to(&mut rig, "freeport_exchange", 50 * 8_000);
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@{}", LANE_VERSION + 1)),
        "{plan}\n{apply}"
    );
    let revised = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    assert_eq!(revised.listing, metal.listing, "revised in place");
    assert_eq!(revised.price_major, 80);
    assert_eq!(
        quantity(&rig.tw, revised.listing),
        Some(50),
        "never more than the house can pay"
    );
    let opened = bid_row(&rig.tw, "freeport_exchange", "carbon_ore");
    assert_eq!(
        (opened.price_major, opened.intake),
        (50, 120),
        "the row records the authored terms"
    );
    assert_eq!(
        quantity(&rig.tw, opened.listing),
        Some(120),
        "an unrecorded row posts its full intake"
    );
}

/// **A lapsed refresh row an unfunded house cannot re-post keeps the new price** (doc 20b
/// RK-1; SM.5's R5 (iii)): the metal ore intake lapses, the Exchange is drained below a
/// tonne, and staff apply a version re-pricing it 70.00cr → 80.00cr. Part (e).8 posts
/// nothing (the drip owns it, and warns) but records the authored price, so the row and the
/// lane agree and the next apply reads no change.
#[test]
fn rk1_an_unfunded_lapsed_row_records_the_new_price() {
    let later = mush_content::load_pack(&lane_variant(LANE_VERSION + 1, |doc| {
        exchange_bid_priced(doc, "metal_ore", 80);
    }))
    .expect("the later variant verifies");
    let mut rig = boot(Roster {
        seed_lane: None,
        shelf: Some(vec![later]),
    });
    stock(&mut rig);
    set_rate(&mut rig.tw, 1);
    let first = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    rig.tw.set_now(first.expires + 60);
    support::with_ctx(&mut rig.tw, |ctx| {
        mush_market::withdraw_listing(ctx, ListingId(first.listing))
    })
    .expect("the listing is removed");
    pay_house_down_to(&mut rig, "freeport_exchange", 7_999);
    let (plan, apply) = plan_and_apply(&mut rig.tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@{}", LANE_VERSION + 1)),
        "{plan}\n{apply}"
    );
    let row = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    assert_eq!(row.listing, first.listing, "nothing was posted");
    assert!(quantity(&rig.tw, row.listing).is_none());
    assert_eq!(
        (row.price_major, row.intake, row.expires),
        (80, first.intake, first.expires),
        "the new price is recorded; the lapse stands"
    );
}

/// **A zero drip warns only when the house cannot fund a tonne** (doc 20b RK-1; SM.5's R4,
/// the world test owed beside `drip_zero_warning`'s unit test): the metal ore intake is sold
/// out, and its row is damaged to read a renewal due with no IC-day boundary since its post
/// (a world clock never does this within the market's 90-day listing life; the store does).
/// Three fires with the Exchange funded post nothing and warn nothing; drained below a
/// tonne, three more fires warn exactly once.
#[test]
fn rk1_a_zero_drip_warns_only_when_the_house_cannot_fund_a_tonne() {
    let mut rig = stocked();
    let (pilot, row) = dray_p(&mut rig, "Pilot", 1, "Salt Moth");
    let hold = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, hold, "metal_ore", 240, LotId(901));
    sell(&mut rig, pilot, "freeport_exchange", "metal_ore", "all");
    settle_last(&mut rig, pilot);
    let exhausted = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    assert_eq!(quantity(&rig.tw, exhausted.listing), Some(0));
    let key = tacsim::port::port_map_key(&lane_id(), &cid("ceres_port"));
    let mut map = stored(&mut rig, &key).expect("the port map is stored");
    let now = rig.tw.world().now();
    let ic = ic_now(&rig.tw);
    map["vendors"]["freeport_exchange"]["bids"]["metal_ore"]["expires"] =
        serde_json::json!(now + 3_600);
    map["vendors"]["freeport_exchange"]["bids"]["metal_ore"]["posted_ic"] = serde_json::json!(ic);
    damage(&mut rig, &key, map);
    let mut rig = reboot(rig);
    let warned = |capture: &Capture| {
        capture
            .text()
            .lines()
            .filter(|l| {
                l.contains("cannot fund a unit of its ore intake") && l.contains("good=metal_ore")
            })
            .count()
    };
    let funded = Capture::default();
    tracing::dispatcher::with_default(&capture_dispatch(funded.clone()), || {
        for _ in 0..3 {
            fire_quietly(&mut rig);
        }
    });
    assert_eq!(warned(&funded), 0, "funded:\n{}", funded.text());
    let row = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    assert_eq!(
        row.listing, exhausted.listing,
        "a zero target posts nothing"
    );
    pay_house_down_to(&mut rig, "freeport_exchange", 6_999);
    let drained = Capture::default();
    tracing::dispatcher::with_default(&capture_dispatch(drained.clone()), || {
        for _ in 0..3 {
            fire_quietly(&mut rig);
        }
    });
    assert_eq!(warned(&drained), 1, "drained, once:\n{}", drained.text());
    assert_eq!(
        ic_now(&rig.tw).div_euclid(DAY),
        ic.div_euclid(DAY),
        "one IC day"
    );
}

/// **A sale on the apply's own IC day drips nothing before the boundary** (§7.3; the lead's
/// pre-commit round, item 6(e)): the boot's apply posts each ore bid with `posted_ic` set
/// to its own IC instant (`post_bid`), so a sale into the metal ore intake that same IC day
/// is not met by a drip — were `posted_ic` unset, the first fire would read a boundary and
/// re-post at once. Past the next boundary the first fire posts one drip.
#[test]
fn sm5_a_sale_on_the_applys_ic_day_drips_nothing_before_the_boundary() {
    let mut rig = stocked();
    let (pilot, row) = dray_p(&mut rig, "Pilot", 1, "Salt Moth");
    let hold = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, hold, "metal_ore", 50, LotId(901));
    let posted = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    let day = posted.posted_ic.map(|p| p.div_euclid(DAY));
    assert_eq!(
        day,
        Some(ic_now(&rig.tw).div_euclid(DAY)),
        "the apply posted it today, and recorded so"
    );
    sell(&mut rig, pilot, "freeport_exchange", "metal_ore", "50");
    assert_eq!(quantity(&rig.tw, posted.listing), Some(190));
    settle_last(&mut rig, pilot);
    for _ in 0..3 {
        fire_quietly(&mut rig);
    }
    assert_eq!(
        ic_now(&rig.tw).div_euclid(DAY),
        day.unwrap_or_default(),
        "still the apply's IC day"
    );
    assert_eq!(
        bid_row(&rig.tw, "freeport_exchange", "metal_ore"),
        posted,
        "no drip on the post's own IC day"
    );
    assert_eq!(quantity(&rig.tw, posted.listing), Some(190));
    past_next_boundary(&mut rig, 60);
    fire_quietly(&mut rig);
    let dripped = bid_row(&rig.tw, "freeport_exchange", "metal_ore");
    assert_ne!(dripped.listing, posted.listing);
    assert_eq!(quantity(&rig.tw, dripped.listing), Some(230), "190 + 40");
}

/// **An exhausted intake reads so, and `=all` sells nothing into it** (§7.2, §7.3; the lead's
/// pre-commit round, items 6(b) and 6(c)): 12 t of strategic ore sold into its 12 t intake
/// leave the bid standing at 0; its funded house's readiness line reads Ready "not buying
/// until the next IC day"; with 3 t more aboard, `+market/sell <listing>=all` refuses
/// `NOT_BUYING` and moves nothing.
#[test]
fn sm5_an_exhausted_intake_takes_nothing_with_all() {
    let mut rig = stocked();
    let (pilot, row) = dray_p(&mut rig, "Pilot", 1, "Salt Moth");
    let hold = hold_vault(&rig.tw, &row);
    seed_pool(&mut rig, hold, "strategic_ore", 12, LotId(901));
    sell(&mut rig, pilot, "freeport_exchange", "strategic_ore", "12");
    let bid = bid_row(&rig.tw, "freeport_exchange", "strategic_ore");
    assert_eq!(
        quantity(&rig.tw, bid.listing),
        Some(0),
        "the intake is taken"
    );
    let report = status(&mut rig.tw);
    let line = format!(
        "  ceres_port bid freeport_exchange strategic_ore: listing #{} not buying until the next \
         IC day; its intake is taken. [ready]",
        bid.listing
    );
    assert!(
        report.lines().any(|l| l == line),
        "{report}\nwanted: {line}"
    );
    seed_pool(&mut rig, hold, "strategic_ore", 3, LotId(902));
    let out = run(
        &mut rig.tw,
        pilot,
        &format!("+market/sell {}=all", bid.listing),
    );
    assert!(out.contains(mush_market::NOT_BUYING), "{out}");
    assert!(!out.contains("Sold "), "{out}");
    assert_eq!(
        pool(&mut rig.tw, hold, "strategic_ore"),
        3,
        "nothing was sold"
    );
    assert_eq!(quantity(&rig.tw, bid.listing), Some(0));
}

/// **An unrecorded intake reads "no bid is posted yet"** (§7.3; the lead's pre-commit round,
/// item 6(c)): a port map whose Exchange row has lost its strategic ore `BidRow` (a damaged
/// store; the apply always records what it posts) loads back, and the readiness report reads
/// that intake Pending, "the clerk posts it at the next drip" — never Missing.
#[test]
fn sm5_an_unrecorded_intake_reads_no_bid_posted_yet() {
    let mut rig = stocked();
    let key = tacsim::port::port_map_key(&lane_id(), &cid("ceres_port"));
    let mut map = stored(&mut rig, &key).expect("the port map is stored");
    let removed = map["vendors"]["freeport_exchange"]["bids"]
        .as_object_mut()
        .and_then(|bids| bids.remove("strategic_ore"));
    assert!(removed.is_some(), "the row was recorded");
    damage(&mut rig, &key, map);
    let mut rig = reboot(rig);
    assert!(
        !port_map(&rig.tw, "ceres_port").vendors[&cid("freeport_exchange")]
            .bids
            .contains_key(&cid("strategic_ore"))
    );
    let report = status(&mut rig.tw);
    let line = "  ceres_port bid freeport_exchange strategic_ore: no bid is posted yet; the clerk \
                posts it at the next drip. [pending]";
    assert!(
        report.lines().any(|l| l == line),
        "{report}\nwanted: {line}"
    );
}

/// Name `instrument` (or none) as the industry layer's extraction licence, in RAM.
fn name_licence(tw: &mut TestWorld, instrument: Option<mush_economy::InstrumentId>) {
    tw.world_mut()
        .module_state_mut::<mush_industry::IndustryStore>()
        .expect("the industry layer is composed")
        .set_licence(instrument);
}

/// The readiness report's Licence line.
fn licence_status(tw: &mut TestWorld) -> String {
    let report = status(tw);
    report
        .lines()
        .find(|l| l.starts_with("  ceres_port licence "))
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("no Licence line:\n{report}"))
}

/// **The Licence line names what is missing** (lead ruling 10; the lead's pre-commit round,
/// item 6(d)): each of `licence_line`'s four Missing variants, whole — the industry layer
/// names no extraction licence, or another instrument (both set in RAM: the composition root
/// never builds them), the issuer has no account (a damaged port map naming an account the
/// economy does not hold), and the instrument is not an entitlement (a damaged instrument
/// document, which the create-only registration never overwrites); restored, Ready.
#[test]
fn sm5_the_licence_line_names_what_is_missing() {
    let mut rig = stocked();
    let head = "  ceres_port licence tacsim.licence.freeport: ";
    let ready = format!(
        "{head}the Freeport Registry issues it at 2500.00cr for 168 IC days with 3 IC days' \
         grace; extraction reads it. [ready]"
    );
    assert_eq!(licence_status(&mut rig.tw), ready);
    // 1. No extraction licence named.
    name_licence(&mut rig.tw, None);
    assert_eq!(
        licence_status(&mut rig.tw),
        format!("{head}the industry layer names no extraction licence. [missing]")
    );
    // 2. Another instrument named.
    name_licence(
        &mut rig.tw,
        mush_economy::InstrumentId::new("tacsim.credit"),
    );
    assert_eq!(
        licence_status(&mut rig.tw),
        format!("{head}the industry layer names tacsim.credit. [missing]")
    );
    name_licence(&mut rig.tw, tacsim::economy::licence_instrument());
    assert_eq!(licence_status(&mut rig.tw), ready);

    // 4. The issuer has no account the economy holds.
    let key = tacsim::port::port_map_key(&lane_id(), &cid("ceres_port"));
    let mut map = stored(&mut rig, &key).expect("the port map is stored");
    map["orgs"]["freeport_registry"]["account"] = serde_json::json!(999_999_999_u64);
    damage(&mut rig, &key, map);
    let mut rig = reboot(rig);
    assert_eq!(
        licence_status(&mut rig.tw),
        format!("{head}the Freeport Registry has no account. [missing]")
    );

    // 3. The instrument is not an entitlement (read before the account).
    let instrument = tacsim::economy::licence_instrument().expect("the licence instrument");
    let ikey = mush_economy::instrument_key(&instrument);
    let mut def = stored(&mut rig, &ikey).expect("the instrument is stored");
    def["kind"] =
        serde_json::to_value(mush_economy::InstrumentKind::Commodity).expect("the kind serializes");
    damage(&mut rig, &ikey, def);
    let mut rig = reboot(rig);
    assert_eq!(
        licence_status(&mut rig.tw),
        format!("{head}it is not registered as an entitlement. [missing]")
    );
}

/// **The in-verb help matches the verbs** (the lead's pre-commit round, item 4): `help
/// +market` documents the bare `+market/sell <listing>` preview and `=all` beside `=<qty>`,
/// as `help trade` does; `help +tv` says a lease needs a prospecting licence, as `help ships`
/// does.
#[test]
fn sm5_the_in_verb_help_names_the_preview_all_and_the_licence() {
    let mut rig = stocked();
    let pilot = pilot(&mut rig, "Pilot", 1);
    let market = run(&mut rig.tw, pilot, "help +market");
    assert!(
        market.lines().any(|l| l == "+market/sell <listing>"),
        "{market}"
    );
    assert!(
        market.contains("It sells nothing."),
        "the preview sells nothing: {market}"
    );
    assert!(
        market.contains("+market/sell <listing>=<qty>|all [from=<vault>]"),
        "{market}"
    );
    assert!(
        market.contains("all sells what the bid takes and leaves the rest"),
        "{market}"
    );
    let tv = run(&mut rig.tw, pilot, "help +tv");
    assert!(
        tv.contains("A lease needs a prospecting licence in force or in its grace (+licence)."),
        "{tv}"
    );
    assert!(tacsim::port::tv::TV_HELP.contains("(+licence)"));
}
