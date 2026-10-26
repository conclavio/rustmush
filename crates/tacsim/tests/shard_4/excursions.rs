//! **Doc 20 cut SM.3b acceptance — the clock** (doc 20 §1 beats 4 and 9, §4.1, §4.3, §8,
//! §10, §11; D-20-6, D-20-7, D-20-11; the SM.3b brief's acceptance A1–A11, its tests 1–16 and
//! decisions 1–26; tests 17 and 18 and the extensions of 1, 4, 6, 8, 9 and 13 answer the
//! cut's review findings tests/T1–T6).
//!
//! A First Captain whose Dray-P is settled on a site's pad, aboard, sends a leased rig down
//! the stern ramp (`+tv/1 launch` previews, `+tv/1 launch go` commits), brings it back up
//! from beside the ramp (`+tv/1 recover`), and empties its hopper onto the site's spoil
//! (`+tv/1 dump`). Every act **starts** as one `tacsim_surface_act` submission by the
//! operator, whose prepare writes the rig's `tacsim.excursion` record (the act, its due
//! instant, the charge reserved for it), and **finishes** on the `tacsim.surface` tick, which
//! at the due instant writes the placement and the `tacsim:tv:<tv>` projection and submits
//! `tacsim_surface_done` as the operator. Two typed departure refusals, `TV_OUT` and
//! `HOPPER_LOADED`, hold the hull.
//!
//! Composed on the composition root (`vehicles.rs`'s rig: `commerce.rs`'s `Roster`, the test
//! world's layer with the game content, `voyage.rs`'s helpers; the per-file precedent). Every
//! boot asserts no namespace write was refused; every drain asserts no kept tacsim submission
//! is GOD's. The world tests run at world rate 1, so every IC instant a test names is the
//! instant the world reads.

use crate::support;

/// Doc 20 cut SM.4a: test 8 boots keyed, so a scan and a sounding plan (the retired probe's
/// replacement).
use crate::geology_keyed;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use mush_core::{
    AccountId, AccountIdentity, ActionStateBootRecord, Audience, AuthoritativeSubmit, CanonicalId,
    CommitToken, GOD, IdempotencyKey, IdempotencyScope, ModuleWrite, ObjId, OfferExecutionContext,
    OfferSubmission, PerceivedRef, RequestUid, ScopedIdempotencyKey,
};
use mush_economy::{EconomyConfig, EconomyStore, HoldId, VaultId};
use mush_inventory::{EndpointRef, Grade, InventoryHandles, LotId};
use mush_org::{PrincipalId, SystemPrincipalId};
use mush_proto::OutputEvent;
use mush_sim::data::{SpatialParent, SpatialState};
use mush_testkit::{TestWorld, fold_module_ops_into};
use smol_str::SmolStr;
use tacsim::definitions::content::LANE_PACK;
use tacsim::port::tv::{
    ACT_FAILED, CAP_OPERATOR, DUMP_EMPTY, DUMP_NO_SPOIL, DUMP_NOT_SITE, DUMPED, GROUND_DARK_LINE,
    LAUNCH_CLEARED, LAUNCH_ENDED, LAUNCH_FORFEIT, LAUNCH_NOT_SITE, LAUNCH_OPEN, LAUNCH_OUT,
    LAUNCH_PREVIEW, LAUNCH_UNDER_WAY, LAUNCHED, LEASE_FAILED, NO_RIG, RECOVER_ELSEWHERE,
    RECOVER_IN_BAY, RECOVER_PREVIEW_UNREAD, RECOVERED, RIG_BUSY, RIG_LOGGED_OUT, TV_DARK,
    TV_NOT_ABOARD, TV_UNWIRED, TvLeaseStatus, WHERE_AWAY, WHICH_RIG_ACT, WHY_HOPPER_CHANGED,
    WHY_NEVER_LEFT, WHY_NOT_BESIDE, WHY_SPOIL_GONE,
};
use tacsim::port::voyage::{HOPPER_LOADED, TV_AWAY, TV_BUSY, TV_OUT, Voyage, VoyageState};
use tacsim::port::{HullRow, PortHandles, PortMap, PortMapState, Ticket, TicketId, room_key};
use tacsim::surface::{
    Act, ActKind, ActParams, ActResult, Charge, Excursion, ExcursionId, RefusedCode, RigKey,
    RigRecord, SURFACE_ACT_ACTION, SURFACE_DONE_ACTION, TvRec, TvRecState, TvState,
};
use tacsim::{TacsimConfig, TacsimStore};

/// The Dray-P's stern ramp on a site's grid (the brief's decision 3).
const RAMP: (i64, i64, u32) = (128_000, 124_500, 180_000);
/// A full Mk1 battery, cu.
const FULL: u32 = 100_000;

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

// ---------------------------------------------------------------------------
// The rig (vehicles.rs's: commerce.rs's Roster, voyage.rs's helpers)
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

/// Boot, settle and stock every port, at world rate 1 (every IC instant exact).
fn stocked() -> Rig {
    let mut tw = TestWorld::with_modules(Roster.modules());
    no_refused_writes(&tw);
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

/// Fold every write since the first boot and reboot into the same roster.
fn reboot(rig: Rig) -> Rig {
    let Rig {
        mut tw,
        oracle,
        mut store,
        typed,
    } = rig;
    fold_module_ops_into(&mut store, tw.take_module_ops());
    let mut tw = tw
        .reboot_into(Roster.modules(), store.clone())
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
        store,
        typed,
    }
}

/// [`reboot`], then `reconnect` come back (a real restart drops every session).
fn reboot_with(rig: Rig, reconnect: &[ObjId]) -> Rig {
    let mut rig = reboot(rig);
    for who in reconnect {
        if !rig.tw.world().is_connected(*who) {
            let _ = rig.tw.connect(*who);
        }
    }
    rig
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
    match store(tw).port_map(&cid(LANE_PACK), &cid(entry)) {
        PortMapState::Present(map) => map.clone(),
        other => panic!("{entry}'s map: {other:?}"),
    }
}

fn station_room(tw: &TestWorld, entry: &str, local: &str) -> ObjId {
    let def = match tacsim::pack::load_pack(tacsim::definitions::content::LANE_JSON)
        .expect("the lane verifies")
        .entry(&cid(entry))
    {
        Some(tacsim::pack::PackEntry::Port(def)) => def.clone(),
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
    let mut bytes = [0x5b_u8; 16];
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

/// Drive one queued submission the way the world task does, keeping its typed post-images
/// for a reboot; the error carries the prepare's own message.
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

/// A chartered pilot whose hull the yard has made a Dray-P.
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
    // Doc 20 cut SM.5: a lease needs a prospecting licence in force; bought once here.
    support::licence::buy_licence(&mut rig.tw, pilot);
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

/// Seed `qty` of `good` into `vault`'s pool through `lot` (additive; the inventory's own
/// path, staff's hand).
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

/// Sweep to the first real second whose IC instant is at or past `target`; the events and
/// the submissions queued (not landed).
fn advance_to(
    rig: &mut Rig,
    target: i64,
) -> (Vec<(Audience, OutputEvent)>, Vec<AuthoritativeSubmit>) {
    let rate = i64::from(world_rate(&rig.tw).max(1));
    let ahead = target.saturating_sub(ic_now(&rig.tw)).max(0);
    // At least one real second, so the 1 s surface tick is due again.
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

/// File a passage with `line`, land its burn, launch it on the next fire; the ticket.
fn file_and_launch(rig: &mut Rig, pilot: ObjId, hull: ObjId, line: &str) -> Ticket {
    let (out, queued) = command(rig, pilot, line);
    assert!(out.contains("Clearance filed for "), "{line}: {out}");
    assert_eq!(queued.len(), 1, "one burn: {out}");
    let seq = voyage(&rig.tw, hull)
        .and_then(|v| v.pending)
        .map(|p| p.seq)
        .expect("a filed passage");
    land_all(rig, &queued);
    let t = ticket(&mut rig.tw, hull, seq).expect("the ticket");
    let (_, queued) = fire(rig);
    land_quietly(rig, &queued);
    t
}

/// The arrival instant of a ticket's plan.
fn arrival_of(t: &Ticket) -> i64 {
    t.plan.depart_ic + t.plan.total_s
}

/// Lease one rig with `+tv/lease go` at Occator's counter (the Exchange Hall).
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

/// **A Dray-P on Ahuna Flank's pad**, its pilot aboard: chartered, refitted, `n_rigs` rigs
/// leased, 20 t of reaction mass and 20 crew-days loaded, boarded and hopped there with
/// `+ship/undock ahuna`; the pilot, the hull's row and the rigs by slot.
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
    // deposit (an unkeyed site reads Ready at once, and this fires nothing).
    open_ahuna(rig);
    let t = file_and_launch(rig, pilot, hull, "+ship/undock ahuna");
    let _ = to_ic(rig, arrival_of(&t));
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == "ahuna_port"
        ),
        "the hull is settled on Ahuna Flank's pad"
    );
    let tvs = rigs_of(&rig.tw, pilot);
    assert_eq!(tvs.len(), n_rigs);
    (pilot, row, tvs)
}

/// **Open Ahuna Flank** (doc 20 cut SM.6, D2; `support::sites::open_site`), keeping the landed
/// bundles' typed post-images for a reboot; the port fires it took (0 when it is open already,
/// as an unkeyed site always is).
fn open_ahuna(rig: &mut Rig) -> u32 {
    let mut kept = Vec::new();
    let fires =
        support::sites::open_site_keeping(&mut rig.tw, "ahuna_port", &mut |ops| kept.push(ops));
    for ops in kept {
        keep(rig, ops);
    }
    fires
}

/// [`on_pad_as`] for the usual pilot.
fn on_pad(rig: &mut Rig, n_rigs: usize) -> (ObjId, HullRow, Vec<ObjId>) {
    on_pad_as(rig, "Pilot", 1, "Salt Moth", n_rigs)
}

fn parent(tw: &TestWorld, obj: ObjId) -> SpatialParent {
    tw.world()
        .module_state::<SpatialState>()
        .expect("the simulation")
        .parent_of(obj)
}

/// Whether `tv` stands in `row`'s bay's `rigs` compartment.
fn in_bay(tw: &TestWorld, tv: ObjId, row: &HullRow) -> bool {
    let bay = ObjId(row.rooms[&cid("bay")]);
    matches!(parent(tw, tv), SpatialParent::Interior { space, slot: Some(slot) }
        if space == bay && slot.as_str() == "rigs")
}

/// Ahuna Flank's passive domain.
fn ahuna_domain(tw: &TestWorld) -> mush_sim::DomainId {
    mush_sim::DomainId(port_map(tw, "ahuna_port").domain.expect("a domain"))
}

/// Whether `tv` stands `Direct` on Ahuna's domain at `(x, y, heading)`.
fn on_ground_at(tw: &TestWorld, tv: ObjId, at: (i64, i64, u32)) -> bool {
    matches!(parent(tw, tv), SpatialParent::Direct { domain, pose }
        if domain == ahuna_domain(tw)
            && (pose.x_cm, pose.y_cm, pose.heading_mdeg) == at)
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

/// Staff debit `qty` of `good` from `vault`'s pool through the inventory's own path
/// (`vehicles.rs`'s `take_pool`).
fn take_pool(rig: &mut Rig, vault: VaultId, good: &str, qty: u64) {
    let endpoint = EndpointRef(vault);
    let good = cid(good);
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
            .debit(&mut txn, endpoint, &good, qty)
            .map(|_| ())
            .expect("the pool is debited");
        txn.prepare().expect("prepares")
    };
    let (ops, install) = prepared.into_parts();
    engine.install_prepared_action(install).expect("installs");
    keep(rig, ops);
}

/// A pool's quantity, through the inventory's own read.
fn pool(tw: &mut TestWorld, vault: VaultId, good: &str) -> u64 {
    let families = tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    let key = mush_inventory::pool_key(EndpointRef(vault), &cid(good)).expect("a pool key");
    typed(tw, &families.pools, &key).map_or(0, |scalar| {
        u64::try_from(scalar.current()).expect("a pool fits u64")
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

/// **Finish `tv`'s act**: to its due instant (the fire there queues the finish, landed),
/// then one more fire for the tell; what `who` heard over both.
fn finish(rig: &mut Rig, tv: ObjId, who: ObjId) -> String {
    let due = due_of(&mut rig.tw, tv);
    let (events, queued) = advance_to(rig, due);
    let mut heard = said(&rig.tw, &events, who);
    let done = finishes(&queued);
    assert_eq!(done.len(), 1, "one finish at the due instant: {queued:?}");
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

// ---------------------------------------------------------------------------
// 1. Launch (A1)
// ---------------------------------------------------------------------------

/// **A bare launch previews; `go` sends the rig down the ramp** (A1): the preview queues and
/// writes nothing; `go` queues exactly one start as the pilot; landed, the record holds the
/// act (`due = start + 300`) and the excursion; at the due instant one fire places the TV
/// `Direct` on Ahuna's passive domain at the ramp pose and submits the finish as the pilot;
/// landed, the act clears, the charge is out of the bay, the projection reads `Out` with its
/// pose, `applied`, `told` and one log line stamped at the due instant, the pilot hears
/// `LAUNCHED` exactly once over two further fires, and `+tv` reads the rig at the ramp with a
/// full charge. The landed finish replayed refuses "that act is already finished" and
/// changes nothing (decision 5).
#[test]
#[allow(clippy::too_many_lines)]
fn sm3b_launch_previews_bare_and_go_sends_the_rig_down_the_ramp() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hull = ObjId(row.hull);
    assert_eq!(
        record_of(&mut rig.tw, tv),
        None,
        "no record before the first act"
    );

    // Bare: the preview, nothing queued, nothing written.
    let (out, queued) = command(&mut rig, pilot, "+tv/1 launch");
    let preview = LAUNCH_PREVIEW
        .replace("{n}", "1")
        .replace("{mark}", "OWP-TV-01")
        .replace("{ramp}", "stern ramp")
        .replace("{site}", "Ahuna Flank")
        .replace("{secs}", "300")
        .replace("{real}", ", about 5 min")
        .replace("{now}", "100")
        .replace("{after}", "100");
    assert_eq!(out.trim(), preview);
    assert!(queued.is_empty());
    assert_eq!(record_of(&mut rig.tw, tv), None);

    // Go: one start as the pilot.
    let started = ic_now(&rig.tw);
    let out = act_go(&mut rig, pilot, "+tv/1 launch go");
    assert!(
        out.starts_with("Rig 1 starts down the stern ramp: on the ground ")
            && out.contains(" UTC (IC "),
        "{out}"
    );
    let record = record_of(&mut rig.tw, tv).expect("the record");
    let act = record.act.clone().expect("the act in flight");
    assert_eq!(record.ordinal, 1);
    assert_eq!(act.ordinal, 1);
    assert_eq!(act.kind, ActKind::Launch);
    assert_eq!((act.operator, act.hull, act.slot), (pilot, hull, 1));
    assert_eq!(act.site, cid("ahuna_port"));
    assert_eq!(act.started_ic, started);
    assert_eq!(act.due_ic, started + 300);
    assert_eq!(act.reserved_cu, 0);
    assert_eq!(act.params, ActParams::None {});
    let opened = Excursion {
        id: ExcursionId {
            tv,
            launch_ic: started,
        },
        operator: pilot,
        hull,
        slot: 1,
        site: cid("ahuna_port"),
    };
    assert_eq!(record.excursion, Some(opened.clone()));
    assert_eq!(
        record.charge,
        Charge {
            value_cu: FULL,
            at_ic: 0,
            in_bay: true
        },
        "the charge is not written at the start (lazy)"
    );
    assert!(
        in_bay(&rig.tw, tv, &row),
        "still in the bay until it is due"
    );

    // Before the due instant nothing finishes.
    let (_, queued) = fire(&mut rig);
    assert!(finishes(&queued).is_empty());
    land_all(&mut rig, &queued);

    // At the due instant: the placement, then one finish as the pilot.
    let due = act.due_ic;
    let (_, queued) = advance_to(&mut rig, due);
    assert!(on_ground_at(&rig.tw, tv, RAMP), "{:?}", parent(&rig.tw, tv));
    let done = finishes(&queued);
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].actor, pilot, "the finish is the pilot's");
    let applied = projection(&rig.tw, tv).expect("the projection beside the placement");
    assert_eq!((applied.applied, applied.told), (1, 0));
    assert_eq!(applied.state, TvState::Out);
    land_all(&mut rig, &queued);
    // A replayed finish refuses on the ordinal and changes nothing (decision 5; review
    // finding tests/T4).
    let landed = record_of(&mut rig.tw, tv);
    let replayed = land(&mut rig, done[0]).expect_err("a replayed finish refuses");
    assert!(
        replayed.contains("that act is already finished"),
        "{replayed}"
    );
    assert_eq!(
        record_of(&mut rig.tw, tv),
        landed,
        "unchanged by the replay"
    );
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(record.act, None);
    assert_eq!(record.ordinal, 1);
    assert_eq!(record.excursion, Some(opened));
    assert_eq!(
        record.charge,
        Charge {
            value_cu: FULL,
            at_ic: due,
            in_bay: false
        }
    );
    let last = record.last.expect("the outcome");
    assert_eq!(
        (last.ordinal, last.kind, last.result),
        (1, ActKind::Launch, ActResult::Done)
    );
    assert_eq!(last.at_ic, due, "stamped at the act's own instant");

    // The tell, once.
    let (events, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    let mut heard = said(&rig.tw, &events, pilot);
    let (events, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    heard.push_str(&said(&rig.tw, &events, pilot));
    let launched = LAUNCHED
        .replace("{n}", "1")
        .replace("{ramp}", "stern ramp")
        .replace("{site}", "Ahuna Flank");
    assert_eq!(count(&heard, &launched), 1, "{heard}");
    let rec = projection(&rig.tw, tv).expect("the projection");
    assert_eq!(rec.state, TvState::Out);
    assert_eq!(
        rec.pose.map(|p| (p.x_cm, p.y_cm, p.heading_mdeg)),
        Some(RAMP)
    );
    assert_eq!((rec.applied, rec.told), (1, 1));
    assert_eq!(
        rec.excursion,
        Some(ExcursionId {
            tv,
            launch_ic: started
        })
    );
    assert_eq!(rec.site, Some(cid("ahuna_port")));
    assert_eq!(rec.act, None);
    assert_eq!(rec.operator, Some(pilot));
    assert_eq!(rec.log.len(), 1);
    assert_eq!(rec.log[0].at_ic, due);
    assert_eq!(rec.log[0].text, launched);

    let status = run(&mut rig.tw, pilot, "+tv");
    assert!(
        status.contains(
            "Rig 1: OWP-TV-01, on the ground at the stern ramp on Ahuna Flank; charge 100 %; "
        ),
        "{status}"
    );
    // Doc 20 cut SM.4a: the rig on the ground gains the status's ground line; this world is
    // unkeyed, so the instruments read dark (the ramp cell is block K's, field K-NW).
    let dark = GROUND_DARK_LINE
        .replace("{block}", "K")
        .replace("{tenure}", "open")
        .replace("{field}", "K-NW");
    assert!(status.contains(&dark), "{status}");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 2. Where a launch is refused (A2)
// ---------------------------------------------------------------------------

/// **A launch is refused at a port, with a clearance filed and under way** (A2): docked at
/// Ceres Freeport `LAUNCH_NOT_SITE`; on the pad with `+ship/lift ceres` filed (before the
/// launch arm departs) `LAUNCH_CLEARED`; under way `LAUNCH_UNDER_WAY`; each the whole reply,
/// nothing queued.
#[test]
fn sm3b_launch_is_refused_under_way_at_a_port_and_with_a_clearance_filed() {
    let mut rig = stocked();
    // At Ceres, docked and aboard.
    let (other, row_o) = prospector(&mut rig, "Other", 2, "Tally Moth");
    lease_one(&mut rig, other);
    load(&mut rig, &row_o, 20, 20, 9_700);
    board(&mut rig, other);
    for line in ["+tv/1 launch", "+tv/1 launch go"] {
        let (out, queued) = command(&mut rig, other, line);
        assert_eq!(
            out.trim(),
            LAUNCH_NOT_SITE.replace("{name}", "Tally Moth"),
            "{line}"
        );
        assert!(queued.is_empty(), "{line}");
    }
    // On the pad, with a clearance filed home.
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let hull = ObjId(row.hull);
    let (out, burn) = command(&mut rig, pilot, "+ship/lift ceres");
    assert!(out.contains("Clearance filed for "), "{out}");
    let (out, queued) = command(&mut rig, pilot, "+tv/1 launch go");
    assert_eq!(
        out.trim(),
        LAUNCH_CLEARED
            .replace("{name}", "Salt Moth")
            .replace("{port}", "Ceres Freeport")
    );
    assert!(queued.is_empty());
    // Under way.
    land_all(&mut rig, &burn);
    let (_, queued) = fire(&mut rig);
    land_quietly(&mut rig, &queued);
    assert!(matches!(
        tacsim::port::ship::berth_of(rig.tw.world(), hull),
        tacsim::port::ship::Berth::UnderWay
    ));
    let (out, queued) = command(&mut rig, pilot, "+tv/1 launch");
    assert_eq!(out.trim(), LAUNCH_UNDER_WAY.replace("{name}", "Salt Moth"));
    assert!(queued.is_empty());
    assert_eq!(record_of(&mut rig.tw, tvs[0]), None, "nothing was written");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 3. Recover (A3)
// ---------------------------------------------------------------------------

/// Staff place `tv` `Direct` on Ahuna's domain at `(x, y)`, facing south.
fn staff_place(rig: &mut Rig, tv: ObjId, x: i64, y: i64) {
    let domain = ahuna_domain(&rig.tw);
    staff_place_on(rig, tv, domain, x, y);
}

/// **A staff write that keeps its writes for a reboot** (doc 20 cut SM.12b, `claims.rs`'s SM.11
/// precedent): [`support::with_ctx`] drops its outbox, so a staff placement's recovery-ledger
/// append never reached the store — harmless until a placement sealed a ledger chunk, which the
/// house yard's lay-up and its drones' placements made happen here (a reboot then refused the
/// unsealed chunk 0 and the gap after it). The world's own writes fold first, then these.
fn keeping<T>(rig: &mut Rig, f: impl FnOnce(&mut mush_core::Ctx<'_>) -> T) -> T {
    let mut out = mush_core::Outbox::new();
    let result = {
        let engine = rig.tw.engine_mut();
        let mut ctx = mush_core::Ctx {
            w: &mut engine.world,
            actor: mush_core::GOD,
            switches: mush_core::SwitchSet::default(),
            args: mush_core::ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(SmolStr::new(tacsim::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        f(&mut ctx)
    };
    let staff: Vec<mush_core::ModuleOp> = out
        .take_deferred()
        .into_iter()
        .filter_map(|deferred| match deferred {
            mush_core::Deferred::Persist(op) => Some(op),
            _ => None,
        })
        .collect();
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    fold_module_ops_into(&mut rig.store, staff);
    result
}

/// Staff place `tv` `Direct` on `domain` at `(x, y)`, facing south.
fn staff_place_on(rig: &mut Rig, tv: ObjId, domain: mush_sim::DomainId, x: i64, y: i64) {
    let placed = keeping(rig, |ctx| {
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

/// Staff place `tv` back `Interior` in `row`'s bay compartment.
fn staff_bay(rig: &mut Rig, tv: ObjId, row: &HullRow) {
    let bay = ObjId(row.rooms[&cid("bay")]);
    let slot = mush_sim::SlotId::new("rigs").expect("a slot");
    let placed = keeping(rig, |ctx| {
        mush_sim::place_interior(ctx, tv, bay, Some(slot))
    });
    placed.expect("staff place the rig in the bay");
}

/// **A recover from beside the ramp; driven home from two cells** (A3): staff place the launched
/// rig two cells south of the ramp — since doc 20 cut SM.4b the preview drives the one cell (10 m)
/// to beside the ramp first (it was `RECOVER_FAR`), nothing queued; one cell off —
/// the preview, `go`, and at the due instant the rig stands `Interior` in the bay's `rigs`
/// compartment, the excursion is closed, the charge is back in the bay and the pilot hears
/// `RECOVERED` once; a recover with the rig in the bay answers `RECOVER_IN_BAY`.
#[test]
fn sm3b_recover_from_beside_the_ramp_and_driven_home_from_two_cells() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    staff_place(&mut rig, tv, RAMP.0, RAMP.1 - 2_000);
    // Doc 20 cut SM.4b (lead ruling 5): two cells off, the recover no longer refuses
    // `RECOVER_FAR` — it drives the one cell to beside the ramp first, in the same one act.
    let (out, queued) = command(&mut rig, pilot, "+tv/1 recover");
    assert!(
        out.trim().starts_with(
            "Rig 1 (OWP-TV-01) would drive 10 m to the stern ramp and climb into the Salt Moth's \
             bay: "
        ),
        "{out}"
    );
    assert!(!out.contains("climbs aboard from beside it"), "{out}");
    assert!(queued.is_empty());
    staff_place(&mut rig, tv, RAMP.0, RAMP.1 - 1_000);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 recover");
    assert!(
        out.trim().starts_with(
            "Rig 1 (OWP-TV-01) would climb the stern ramp into the Salt Moth's bay: 300 IC s, \
             about 5 min; charge 100 → 100 %; hopper empty."
        ),
        "{out}"
    );
    assert!(queued.is_empty());
    let out = act_go(&mut rig, pilot, "+tv/1 recover go");
    assert!(
        out.starts_with("Rig 1 starts up the stern ramp: in the bay "),
        "{out}"
    );
    let due = due_of(&mut rig.tw, tv);
    let heard = finish(&mut rig, tv, pilot);
    assert!(in_bay(&rig.tw, tv, &row), "{:?}", parent(&rig.tw, tv));
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(record.excursion, None, "the excursion is closed");
    assert_eq!(
        record.charge,
        Charge {
            value_cu: FULL,
            at_ic: due,
            in_bay: true
        }
    );
    assert_eq!(
        record.last.map(|last| (last.kind, last.result)),
        Some((ActKind::Recover, ActResult::Done))
    );
    let recovered = RECOVERED.replace("{n}", "1").replace("{name}", "Salt Moth");
    assert_eq!(count(&heard, &recovered), 1, "{heard}");
    let rec = projection(&rig.tw, tv).expect("the projection");
    assert_eq!(rec.state, TvState::Bay);
    assert_eq!((rec.pose, rec.excursion), (None, None));
    let (out, queued) = command(&mut rig, pilot, "+tv/1 recover");
    assert_eq!(out.trim(), RECOVER_IN_BAY.replace("{n}", "1"));
    assert!(queued.is_empty());
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 4. A restart mid-launch (A4)
// ---------------------------------------------------------------------------

/// **A restart mid-launch completes at the act's own due instant** (A4; lead ruling 2): world
/// A runs the launch uninterrupted; world B, built the same way, lands the start, folds and
/// reboots before the due instant and first fires 40 IC s after it. The typed record, the
/// projection and the rig's placement are equal across A and B, and `LAUNCHED` is told once
/// in B; a second reboot after the tell reloads the projection unchanged and tells nothing
/// again.
#[test]
fn sm3b_a_restart_mid_launch_completes_at_the_due_instant() {
    // World A.
    let mut a = stocked();
    let (pilot_a, _, tvs_a) = on_pad(&mut a, 1);
    let _ = act_go(&mut a, pilot_a, "+tv/1 launch go");
    let _ = finish(&mut a, tvs_a[0], pilot_a);
    let record_a = record_of(&mut a.tw, tvs_a[0]).expect("A's record");
    let rec_a = projection(&a.tw, tvs_a[0]).expect("A's projection");

    // World B.
    let mut b = stocked();
    let (pilot_b, _, tvs_b) = on_pad(&mut b, 1);
    let tv = tvs_b[0];
    let _ = act_go(&mut b, pilot_b, "+tv/1 launch go");
    let due = due_of(&mut b.tw, tv);
    // Doc 20 cut SM.11 (the brief's §6, the reviewer's F17): the pilot reconnects after the
    // restart, so "told exactly once across a restart" is pinned on what they hear — offline, a
    // `LAUNCHED` line is never queued (the notice budget; `sm11_an_offline_launch_line_is_not_queued`).
    let mut b = reboot_with(b, &[pilot_b]);
    assert_eq!(due_of(&mut b.tw, tv), due, "the act survives the restart");
    let (events, queued) = advance_to(&mut b, due + 40);
    let mut heard = said(&b.tw, &events, pilot_b);
    assert_eq!(finishes(&queued).len(), 1);
    land_all(&mut b, &queued);
    for _ in 0..2 {
        let (events, queued) = fire(&mut b);
        land_all(&mut b, &queued);
        heard.push_str(&said(&b.tw, &events, pilot_b));
    }
    let record_b = record_of(&mut b.tw, tv).expect("B's record");
    let rec_b = projection(&b.tw, tv).expect("B's projection");
    assert_eq!(record_b, record_a, "the typed record");
    assert_eq!(rec_b, rec_a, "the projection");
    assert_eq!(
        record_b.charge.at_ic, due,
        "the overdue act applied at its own stamp"
    );
    assert!(on_ground_at(&a.tw, tvs_a[0], RAMP));
    assert!(on_ground_at(&b.tw, tv, RAMP));
    let launched = LAUNCHED
        .replace("{n}", "1")
        .replace("{ramp}", "stern ramp")
        .replace("{site}", "Ahuna Flank");
    let queued_notice = tacsim::port_tick::pending_notices(b.tw.world(), pilot_b)
        .iter()
        .filter(|line| **line == launched)
        .count();
    assert_eq!(count(&heard, &launched), 1, "{heard}");
    assert_eq!(queued_notice, 0, "heard, never queued");
    // A told act stays told across a restart (review finding tests/T5): the tick's projection
    // folds, reloads and still reads `told`, so a second reboot re-tells nothing — no
    // `LAUNCHED` heard, none queued beyond the first — and the projection is unchanged.
    let mut b = reboot_with(b, &[pilot_b]);
    assert_eq!(
        projection(&b.tw, tv).as_ref(),
        Some(&rec_b),
        "the projection reloads"
    );
    let mut again = String::new();
    for _ in 0..2 {
        let (events, queued) = fire(&mut b);
        land_all(&mut b, &queued);
        again.push_str(&said(&b.tw, &events, pilot_b));
    }
    assert_eq!(count(&again, &launched), 0, "{again}");
    let queued_again = tacsim::port_tick::pending_notices(b.tw.world(), pilot_b)
        .iter()
        .filter(|line| **line == launched)
        .count();
    assert!(queued_again <= queued_notice, "nothing re-queued");
    assert_eq!(projection(&b.tw, tv), Some(rec_b), "told once, still");
    no_refused_writes(&b.tw);
}
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

/// **An offline launch line is not queued** (doc 20 cut SM.11, lead ruling 7 and D13; the
/// brief's §6, the reviewer's F17): the pilot launches and logs off before the act is due; the
/// launch lands and its line goes to the rig's projection log, but nothing is queued for the
/// reconnect — a launch is a `Live` line, and offline only a trip-level line is queued. **The
/// budget's other half** (the lead's pre-commit round, item 6, the judge's T12): the same rig left
/// a day on the ground comes home by itself, and that trip-level line (`IDLE_HOME`) IS queued —
/// exactly it, the launch's line still not — and read on reconnect after the heading.
#[test]
fn sm11_an_offline_launch_line_is_not_queued() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    disconnect(&mut rig.tw, pilot);
    let due = due_of(&mut rig.tw, tv);
    let (_, queued) = advance_to(&mut rig, due);
    assert_eq!(finishes(&queued).len(), 1);
    land_all(&mut rig, &queued);
    for _ in 0..2 {
        let (_, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
    }
    assert!(on_ground_at(&rig.tw, tv, RAMP), "the launch landed");
    let launched = LAUNCHED
        .replace("{n}", "1")
        .replace("{ramp}", "stern ramp")
        .replace("{site}", "Ahuna Flank");
    let pending = tacsim::port_tick::pending_notices(rig.tw.world(), pilot);
    assert!(
        !pending.iter().any(|line| line.contains(&launched)),
        "{pending:?}"
    );
    let rec = projection(&rig.tw, tv).expect("the projection");
    assert!(
        rec.log.iter().any(|line| line.text.contains(&launched)),
        "the projection still logs it: {:?}",
        rec.log
    );
    // The positive control: a day idle, the rig's return is a trip-level line, queued offline.
    let since = record_of(&mut rig.tw, tv)
        .and_then(|record| record.idle_since())
        .expect("idle since");
    let (_, queued) = advance_to(&mut rig, since + tacsim::surface::plan::IDLE_PARK_IC_SECS);
    land_all(&mut rig, &queued);
    let _ = finish(&mut rig, tv, pilot);
    assert!(in_bay(&rig.tw, tv, &row), "the idle return brought it in");
    let home = tacsim::port::tv::IDLE_HOME
        .replace("{n}", "1")
        .replace("{site}", "Ahuna Flank")
        .replace("{name}", "Salt Moth")
        .replace("{then}", "");
    let pending = tacsim::port_tick::pending_notices(rig.tw.world(), pilot);
    assert_eq!(pending, vec![home.clone()], "exactly the trip's line");
    let events = rig.tw.connect(pilot);
    let heard = said(&rig.tw, &events, pilot);
    let lines: Vec<&str> = heard.lines().collect();
    let at = lines
        .iter()
        .position(|line| *line == tacsim::port_tick::NOTICE_HEADING)
        .expect("the heading");
    assert_eq!(lines.get(at + 1).copied(), Some(home.as_str()), "{heard}");
    assert!(!heard.contains(&launched), "{heard}");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 5. The departure refusals (A5)
// ---------------------------------------------------------------------------

/// **A rig out or a loaded hopper holds the hull** (A5): with a landed launch in flight,
/// `+ship/lift ceres` and `+ship/plan ceres` read `TV_BUSY` (the brief's case; the fixer
/// round's item 10b); with the rig on the ground,
/// `+ship/plan ceres` prints `TV_OUT` in its block, `+ship/lift ceres` refuses with it and
/// files nothing, and a queued `depart ceres` reads "it would hold: …" in the dry run and
/// holds with the reason when it falls due; with a recover in flight, `TV_BUSY`; recovered
/// with 3 t of staff-placed ore in its hopper, all three read `HOPPER_LOADED`; after `+tv/1
/// dump go` lands and finishes the hopper holds nothing, the spoil exactly 3 t more (the
/// two pools' sum unchanged), `DUMPED` is heard once and `+ship/lift ceres` files.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3b_departure_is_refused_with_a_rig_out_or_a_loaded_hopper() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hull = ObjId(row.hull);
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    // A landed launch in flight: TV_BUSY, in the lift's reply and the plan's block.
    let launching = TV_BUSY
        .replace("{n}", "1")
        .replace("{doing}", "going down the stern ramp")
        .replace("{name}", "Salt Moth");
    let (out, queued) = command(&mut rig, pilot, "+ship/lift ceres");
    assert_eq!(out.trim(), launching);
    assert!(queued.is_empty());
    let plan = run(&mut rig.tw, pilot, "+ship/plan ceres");
    assert_eq!(plan.lines().nth(1), Some(launching.as_str()), "{plan}");
    assert!(voyage(&rig.tw, hull).and_then(|v| v.pending).is_none());
    let _ = finish(&mut rig, tv, pilot);
    let out_line = TV_OUT
        .replace("{n}", "1")
        .replace("{site}", "Ahuna Flank")
        .replace("{name}", "Salt Moth");
    let plan = run(&mut rig.tw, pilot, "+ship/plan ceres");
    assert_eq!(plan.lines().nth(1), Some(out_line.as_str()), "{plan}");
    let (out, queued) = command(&mut rig, pilot, "+ship/lift ceres");
    assert_eq!(out.trim(), out_line);
    assert!(queued.is_empty());
    assert!(voyage(&rig.tw, hull).and_then(|v| v.pending).is_none());
    // The queue's depart: the dry run, then the hold when it falls due.
    let out = run(&mut rig.tw, pilot, "+ship/order depart ceres at=+1h");
    assert!(
        out.contains("Order 1 filed: depart for Ceres Freeport"),
        "{out}"
    );
    let dry = run(&mut rig.tw, pilot, "+ship/plan");
    let would = format!(
        "  Order 1, depart for Ceres Freeport: it would hold: {}.",
        out_line.trim_end_matches('.')
    );
    assert!(
        dry.lines().any(|line| line == would),
        "{dry}\nwanted: {would}"
    );
    sweep(&mut rig.tw, 3_600);
    for _ in 0..2 {
        let (_, queued) = fire(&mut rig);
        assert!(
            queued
                .iter()
                .all(|s| s.action.key().id.as_str() != tacsim::port::BURN_ACTION),
            "nothing is burned"
        );
        land_all(&mut rig, &queued);
    }
    let waits = tacsim::port::executor::WAITING_LINE
        .replace("{n}", "1")
        .replace("{reason}", &out_line);
    let log: Vec<String> = match store(&rig.tw).orders_state(hull) {
        tacsim::port::orders::OrdersState::Present(orders) => {
            orders.log.iter().map(|l| l.text.clone()).collect()
        }
        other => panic!("the sailing plan: {other:?}"),
    };
    assert!(log.contains(&waits), "{log:?}");
    let out = run(&mut rig.tw, pilot, "+ship/order clear");
    assert!(out.contains("cleared"), "{out}");

    // A recover in flight: TV_BUSY.
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let busy = TV_BUSY
        .replace("{n}", "1")
        .replace("{doing}", "climbing the stern ramp")
        .replace("{name}", "Salt Moth");
    let (out, queued) = command(&mut rig, pilot, "+ship/lift ceres");
    assert_eq!(out.trim(), busy);
    assert!(queued.is_empty());
    // The verb mirrors the prepare's one-act rule (finding F10).
    let (out, queued) = command(&mut rig, pilot, "+tv/1 dump go");
    let rig_busy = RIG_BUSY
        .replace("{n}", "1")
        .replace("{doing}", "climbing the stern ramp");
    let head = rig_busy.split("{when}").next().unwrap_or_default();
    assert!(
        out.trim().starts_with(head) && out.contains(" UTC (IC "),
        "{out}"
    );
    assert!(queued.is_empty());
    let _ = finish(&mut rig, tv, pilot);
    assert!(in_bay(&rig.tw, tv, &row));

    // In the bay with 3 t of ore: HOPPER_LOADED on all three.
    fill_hopper(&mut rig, tv, 3, 9_800);
    let loaded = HOPPER_LOADED
        .replace("{n}", "1")
        .replace("{t}", "3")
        .replace("{name}", "Salt Moth");
    let plan = run(&mut rig.tw, pilot, "+ship/plan ceres");
    assert_eq!(plan.lines().nth(1), Some(loaded.as_str()), "{plan}");
    let (out, queued) = command(&mut rig, pilot, "+ship/lift ceres");
    assert_eq!(out.trim(), loaded);
    assert!(queued.is_empty());
    let _ = run(&mut rig.tw, pilot, "+ship/order depart ceres at=+1h");
    let dry = run(&mut rig.tw, pilot, "+ship/plan");
    let would = format!(
        "  Order 2, depart for Ceres Freeport: it would hold: {}.",
        loaded.trim_end_matches('.')
    );
    assert!(
        dry.lines().any(|line| line == would),
        "{dry}\nwanted: {would}"
    );
    let _ = run(&mut rig.tw, pilot, "+ship/order clear");

    // The dump: conserved.
    let hopper_v = hopper(&rig.tw, tv);
    let spoil = VaultId(
        port_map(&rig.tw, "ahuna_port")
            .site
            .and_then(|row| row.spoil_vault)
            .expect("the spoil vault"),
    );
    let before = (
        pool(&mut rig.tw, hopper_v, "metal_ore"),
        pool(&mut rig.tw, spoil, "metal_ore"),
    );
    assert_eq!(before.0, 3);
    let out = act_go(&mut rig, pilot, "+tv/1 dump go");
    assert_eq!(
        out.lines().next(),
        Some(tacsim::port::tv::DUMP_FILED.replace("{n}", "1").as_str()),
        "{out}"
    );
    let heard = finish(&mut rig, tv, pilot);
    let after = (
        pool(&mut rig.tw, hopper_v, "metal_ore"),
        pool(&mut rig.tw, spoil, "metal_ore"),
    );
    assert_eq!(after, (0, before.1 + 3));
    assert_eq!(after.0 + after.1, before.0 + before.1, "conserved");
    let dumped = DUMPED
        .replace("{n}", "1")
        .replace("{load}", "3 t of metal ore, run-of-mine")
        .replace("{site}", "Ahuna Flank");
    assert_eq!(count(&heard, &dumped), 1, "{heard}");
    let (out, queued) = command(&mut rig, pilot, "+ship/lift ceres");
    assert!(out.contains("Clearance filed for "), "{out}");
    assert_eq!(queued.len(), 1);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 6. The dump
// ---------------------------------------------------------------------------

/// **A dump is conserved from the bay and from the ground, and refused away from a site**:
/// at Ceres `DUMP_NOT_SITE`; on the pad an empty hopper is `DUMP_EMPTY`; from the bay 2 t go
/// to the spoil — and a tonne staff put in between the start and the finish stays in the
/// hopper (the frozen quantity is the start's); a dump whose hopper staff lightened before the
/// finish refuses `HopperChanged`, moves nothing and is told `ACT_FAILED`; from the ground at
/// the ramp the rest goes.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3b_dump_is_conserved_in_and_out_of_the_bay_and_refused_away_from_a_site() {
    let mut rig = stocked();
    // At Ceres.
    let (other, row_o) = prospector(&mut rig, "Other", 2, "Tally Moth");
    lease_one(&mut rig, other);
    load(&mut rig, &row_o, 20, 20, 9_700);
    board(&mut rig, other);
    let (out, queued) = command(&mut rig, other, "+tv/1 dump");
    assert_eq!(
        out.trim(),
        DUMP_NOT_SITE
            .replace("{n}", "1")
            .replace("{name}", "Tally Moth")
    );
    assert!(queued.is_empty());
    // On the pad.
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let (out, queued) = command(&mut rig, pilot, "+tv/1 dump go");
    assert_eq!(out.trim(), DUMP_EMPTY.replace("{n}", "1"));
    assert!(queued.is_empty());
    let hopper_v = hopper(&rig.tw, tv);
    let spoil = VaultId(
        port_map(&rig.tw, "ahuna_port")
            .site
            .and_then(|row| row.spoil_vault)
            .expect("the spoil vault"),
    );
    // From the bay: the preview names the load; a tonne added after the start stays.
    fill_hopper(&mut rig, tv, 2, 9_810);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 dump");
    assert_eq!(
        out.trim(),
        "Rig 1 (OWP-TV-02) would dump 2 t of metal ore, run-of-mine onto the spoil at Ahuna \
         Flank; it cannot be taken back. +tv/1 dump go dumps it."
    );
    assert!(queued.is_empty());
    let spoil_before = pool(&mut rig.tw, spoil, "metal_ore");
    let _ = act_go(&mut rig, pilot, "+tv/1 dump go");
    let record = record_of(&mut rig.tw, tv).expect("the record");
    match record.act.map(|act| act.params) {
        Some(ActParams::Dump { goods, .. }) => {
            assert_eq!(
                goods.into_iter().collect::<Vec<_>>(),
                vec![(cid("metal_ore"), 2)]
            );
        }
        other => panic!("a dump's frozen goods: {other:?}"),
    }
    fill_hopper(&mut rig, tv, 1, 9_820);
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(
        pool(&mut rig.tw, hopper_v, "metal_ore"),
        1,
        "the late tonne stays"
    );
    assert_eq!(pool(&mut rig.tw, spoil, "metal_ore"), spoil_before + 2);
    // A hopper that changed under the dump (review finding tests/T4): 2 t frozen at the
    // start, staff take a tonne before the finish; the finish refuses `HopperChanged`,
    // moves nothing, and the pilot hears `ACT_FAILED` once.
    fill_hopper(&mut rig, tv, 1, 9_825);
    let _ = act_go(&mut rig, pilot, "+tv/1 dump go");
    take_pool(&mut rig, hopper_v, "metal_ore", 1);
    let heard = finish(&mut rig, tv, pilot);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(record.act, None);
    assert_eq!(
        record
            .last
            .map(|last| (last.kind, last.result, last.moved.is_empty())),
        Some((
            ActKind::Dump,
            ActResult::Refused {
                code: RefusedCode::HopperChanged
            },
            true
        ))
    );
    assert_eq!(pool(&mut rig.tw, hopper_v, "metal_ore"), 1, "nothing moved");
    assert_eq!(pool(&mut rig.tw, spoil, "metal_ore"), spoil_before + 2);
    let failed = ACT_FAILED
        .replace("{n}", "1")
        .replace("{act}", "dump")
        .replace("{why}", WHY_HOPPER_CHANGED);
    assert_eq!(count(&heard, &failed), 1, "{heard}");
    // From the ground at the ramp.
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    assert!(on_ground_at(&rig.tw, tv, RAMP));
    let _ = act_go(&mut rig, pilot, "+tv/1 dump go");
    let heard = finish(&mut rig, tv, pilot);
    assert_eq!(pool(&mut rig.tw, hopper_v, "metal_ore"), 0);
    assert_eq!(pool(&mut rig.tw, spoil, "metal_ore"), spoil_before + 3);
    let dumped = DUMPED
        .replace("{n}", "1")
        .replace("{load}", "1 t of metal ore, run-of-mine")
        .replace("{site}", "Ahuna Flank");
    assert_eq!(count(&heard, &dumped), 1, "{heard}");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 7. A bare act commits nothing (A6)
// ---------------------------------------------------------------------------

/// **A bare act commits nothing** (A6): `+tv/1 launch` with the rig in the bay, then (the rig
/// launched, its hopper loaded) `+tv/1 recover` and `+tv/1 dump`, each queue nothing, leave
/// the typed record as it was and write no `tacsim:` document.
#[test]
fn sm3b_a_bare_act_commits_nothing() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let bare = |rig: &mut Rig, line: &str| {
        let before = record_of(&mut rig.tw, tv);
        let _ = rig.tw.take_module_ops();
        let (out, queued) = command(rig, pilot, line);
        assert!(queued.is_empty(), "{line}: {out}");
        assert!(out.contains(" would "), "{line} previews: {out}");
        assert_eq!(record_of(&mut rig.tw, tv), before, "{line}");
        let ops = rig.tw.take_module_ops();
        assert!(
            !ops.iter().any(|op| matches!(&op.op,
                ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key }
                    if key.starts_with("tacsim:"))),
            "{line} wrote a tacsim document: {ops:?}"
        );
    };
    bare(&mut rig, "+tv/1 launch");
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    fill_hopper(&mut rig, tv, 1, 9_830);
    bare(&mut rig, "+tv/1 recover");
    bare(&mut rig, "+tv/1 dump");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 8. Two acts cannot spend one margin (A7)
// ---------------------------------------------------------------------------

/// **Boot, settle and stock every port with the geology key handed in** (doc 20 cut SM.4a):
/// `stocked`'s rig, composed, keyed between composition and the boot pass (`build_engine`'s
/// order), so a scan and a sounding plan.
fn stocked_keyed() -> Rig {
    let (world, registry) = mush_testkit::boot_world();
    let mut engine = mush_core::Engine::with_modules(world, registry, Roster.modules())
        .expect("the roster composes");
    assert!(tacsim::surface::install_geology(
        &mut engine.world,
        tacsim::surface::GeologyBoot::Key(geology_keyed::secret(geology_keyed::KEY_1))
    ));
    let (mut out, mut fx) = (mush_core::Outbox::new(), mush_core::Effects::new());
    engine
        .modules_loaded(HashMap::new(), &mut out, &mut fx)
        .expect("the world boots");
    let mut tw = TestWorld::from_engine(engine);
    no_refused_writes(&tw);
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

/// Set a rig's charge out of the bay at the world's IC now (staff's hand, `put_record`).
fn put_charge(rig: &mut Rig, tv: ObjId, value_cu: u32) {
    let mut record = record_of(&mut rig.tw, tv).expect("the record");
    record.charge = Charge {
        value_cu,
        at_ic: ic_now(&rig.tw),
        in_bay: false,
    };
    put_record(rig, record);
}

/// Staff move `tv` to the centre of site cell `(x, y)`.
fn to_cell(rig: &mut Rig, tv: ObjId, (x, y): (i64, i64)) {
    staff_place(rig, tv, x * 1_000 + 500, y * 1_000 + 500);
}

/// Fire until Ahuna Flank's ground is warm (the arrival hook asked for it; the surface tick
/// builds one site a fire) — and, since doc 20 cut SM.4b, its home field (a scan or a sounding
/// from the ground reserves the real trip home; the tick builds the home the fire after).
fn warm_ahuna(rig: &mut Rig) {
    for _ in 0..8 {
        let warm = rig
            .tw
            .world()
            .module_state::<tacsim::surface::GeologyState>()
            .is_some_and(|state| !state.sites().is_empty() && !state.homes().is_empty());
        if warm {
            return;
        }
        let (_, queued) = fire(rig);
        land_quietly(rig, &queued);
    }
    panic!("Ahuna Flank's ground was not warmed");
}

/// **The charge home from a site cell** (doc 20 cut SM.4b): the Dray-P's ramp cell's home field
/// on Ahuna Flank and the TV's rates (300 cm/s, 15 cu a nominal cell) — what the reserve rule
/// now adds to a scan's or a sounding's spend from the ground.
fn home_cu_from(rig: &Rig, cell: (i64, i64)) -> u32 {
    let field = rig
        .tw
        .world()
        .module_state::<tacsim::surface::GeologyState>()
        .and_then(|state| state.home_field(&cid("ahuna_port"), (128, 124)))
        .expect("Ahuna Flank's home field is built");
    tacsim::surface::drive::home_cu_to(
        &field,
        cell,
        &tacsim::surface::drive::DriveRates {
            max_speed_cms: 300,
            cu_per_nominal_cell: 15,
        },
    )
    .expect("a way home")
}

/// **Two acts cannot spend one margin** (A7; §4.1, §19 item 4; decision 4) — **rewritten by doc
/// 20 cut SM.4a on the real kinds** (the test-world probe is retired, D15; a keyed boot so the
/// scan and the sounding plan): on a launched rig (100,000 cu, out of the bay, so the charge is
/// flat) at ordinal N: (a) two soundings both at N+1, built before either lands — the first
/// lands, the second refuses "that rig's orders moved on" and changes nothing; (b) the first
/// finished, staff set the charge to 6,500 cu and move the rig to a new cell (the same-cell
/// rule refuses a repeat before the reserve is read): a sounding (2,000) refuses "the rig's
/// charge is under the reserve" (4,500 < 5,000), while a scan (1,000) lands and finishes leaving
/// 5,500; (c) staff set 7,000 plus the real trip home from the next stand (doc 20 cut SM.4b:
/// the reserve counts it) and move it there: a sounding lands and finishes leaving exactly the
/// 5,000 floor plus that trip home, and a scan from a new cell then refuses with the reserve
/// message (4,000);
/// (d) staff leave the rig at the ramp **under** the floor (4,000 cu): a scan refuses, and a
/// recover (a zero spend) previews, lands and finishes — the exemption, in the world.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3b_two_acts_cannot_spend_one_margin() {
    let mut rig = stocked_keyed();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    warm_ahuna(&mut rig);
    let n = record_of(&mut rig.tw, tv).expect("the record").ordinal;
    // (a)
    let first = start(&rig, pilot, tv, "sound", n + 1);
    let second = start(&rig, pilot, tv, "sound", n + 1);
    land(&mut rig, &first).expect("the first sounding lands");
    let landed = record_of(&mut rig.tw, tv);
    let refused = land(&mut rig, &second).expect_err("the second refuses");
    assert!(refused.contains("that rig's orders moved on"), "{refused}");
    assert_eq!(
        record_of(&mut rig.tw, tv),
        landed,
        "unchanged by the second"
    );
    // (b)
    let _ = finish(&mut rig, tv, pilot);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(record.charge.value_cu, FULL - 2_000);
    assert!(!record.charge.in_bay);
    put_charge(&mut rig, tv, 6_500);
    to_cell(&mut rig, tv, (133, 124));
    let over = start(&rig, pilot, tv, "sound", n + 2);
    let refused = land(&mut rig, &over).expect_err("reserve");
    assert!(
        refused.contains("the rig's charge is under the reserve"),
        "{refused}"
    );
    let scan = start(&rig, pilot, tv, "scan", n + 2);
    land(&mut rig, &scan).expect("a scan of 1,000 fits");
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(
        record_of(&mut rig.tw, tv).map(|r| r.charge.value_cu),
        Some(5_500)
    );
    // (c) Doc 20 cut SM.4b (lead ruling 4): the reserve now counts the real trip home from the
    // stand, so "exactly the floor" is 7,000 plus that trip: the sounding leaves the floor plus
    // the charge home, which the reserve kept back.
    let home = home_cu_from(&rig, (138, 124));
    assert!(home > 0, "a real trip home from ten cells off");
    put_charge(&mut rig, tv, 7_000 + home);
    to_cell(&mut rig, tv, (138, 124));
    let fits = start(&rig, pilot, tv, "sound", n + 3);
    land(&mut rig, &fits).expect("2,000 and the trip home from 7,000 + home fits");
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(
        record_of(&mut rig.tw, tv).map(|r| r.charge.value_cu),
        Some(5_000 + home),
        "exactly the floor after the trip home"
    );
    to_cell(&mut rig, tv, (143, 124));
    let one = start(&rig, pilot, tv, "scan", n + 4);
    let refused = land(&mut rig, &one).expect_err("4,000");
    assert!(
        refused.contains("the rig's charge is under the reserve"),
        "{refused}"
    );
    // (d) The way home is never refused by the rule (decision 21; review finding tests/T2):
    // at exactly the floor a zero spend fits without the exemption, so staff leave the rig
    // **under** it (4,000 cu, out of the bay, as a tools-data change might), at the ramp. A
    // scan there refuses — the rule is live — yet the recover (a zero spend) previews, lands
    // and finishes `Done`.
    staff_place(&mut rig, tv, RAMP.0, RAMP.1);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 recover");
    assert!(out.contains("charge 5 → 5 %"), "{out}");
    assert!(queued.is_empty());
    put_charge(&mut rig, tv, 4_000);
    let one = start(&rig, pilot, tv, "scan", n + 4);
    let refused = land(&mut rig, &one).expect_err("under the floor");
    assert!(
        refused.contains("the rig's charge is under the reserve"),
        "{refused}"
    );
    let (out, queued) = command(&mut rig, pilot, "+tv/1 recover");
    assert!(
        out.contains("charge 4 → 4 %") && !out.contains("reserve"),
        "{out}"
    );
    assert!(queued.is_empty());
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(
        record_of(&mut rig.tw, tv)
            .and_then(|r| r.last)
            .map(|l| (l.kind, l.result)),
        Some((ActKind::Recover, ActResult::Done))
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 9. The caps (A8)
// ---------------------------------------------------------------------------

/// Write `record` into the family through a typed transaction (staff's hand; `seed_pool`'s
/// shape).
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

/// **The caps refuse at the prepare** (A8; lead ruling 8): four other Occator rigs' records
/// seeded with open excursions at Ahuna naming the pilot as operator; the pilot's launch is
/// refused at the command with `CAP_OPERATOR`, the whole reply, and — built and landed
/// directly — at the prepare. Then the same four re-seeded as **another** pilot's (review
/// finding tests/T3): the site and the world now count four open excursions, the pilot none,
/// so the pilot's launch previews and its built start lands — the operator count is the
/// actor's own, not the site's or the world's.
#[test]
fn sm3b_caps_refuse_at_the_prepare() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hull = ObjId(row.hull);
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
    let battery = tacsim::definitions::tools::tools_table()
        .expect("the tools table")
        .values()
        .find_map(tacsim::definitions::tools::ToolDef::battery)
        .cloned()
        .expect("a battery row");
    for other in &others {
        let mut record = RigRecord::fresh(*other, &battery);
        record.excursion = Some(Excursion {
            id: ExcursionId {
                tv: *other,
                launch_ic: 0,
            },
            operator: pilot,
            hull,
            slot: 1,
            site: cid("ahuna_port"),
        });
        put_record(&mut rig, record);
    }
    let cap = CAP_OPERATOR.replace("{cap}", "4");
    for line in ["+tv/1 launch", "+tv/1 launch go"] {
        let (out, queued) = command(&mut rig, pilot, line);
        assert_eq!(out.trim(), cap, "{line}");
        assert!(queued.is_empty());
    }
    let built = AuthoritativeSubmit {
        actor: pilot,
        action: handles(&rig.tw).surface_act.expect("registered"),
        subject: PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters: serde_json::json!({ "tv": tv.0.to_string(), "act": "launch", "ordinal": 1 }),
        key: tacsim::surface::act_key(tv, 1).expect("a key"),
    };
    let refused = land(&mut rig, &built).expect_err("the cap");
    assert!(
        refused.contains("you have as many rigs out as one pilot may run"),
        "{refused}"
    );
    assert_eq!(record_of(&mut rig.tw, tv), None, "nothing written");
    // The same four excursions, another operator's: the pilot runs none.
    let someone = rig.tw.spawn_player("Someone");
    for other in &others {
        let mut record = record_of(&mut rig.tw, *other).expect("the seeded record");
        if let Some(excursion) = record.excursion.as_mut() {
            excursion.operator = someone;
        }
        put_record(&mut rig, record);
    }
    let (out, queued) = command(&mut rig, pilot, "+tv/1 launch");
    assert!(
        out.trim()
            .starts_with("Rig 1 (OWP-TV-01) would go down the stern ramp onto Ahuna Flank"),
        "{out}"
    );
    assert!(queued.is_empty());
    land(&mut rig, &built).expect("the pilot's own count is 0");
    assert_eq!(
        record_of(&mut rig.tw, tv)
            .and_then(|record| record.act)
            .map(|act| act.kind),
        Some(ActKind::Launch)
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 10. One act a principal a fire (A9)
// ---------------------------------------------------------------------------

/// **An operator finishes one act a fire** (A9; §8, D-18-32): two rigs launched at one
/// instant are due at one instant; the first due fire queues one finish, the next fire the
/// other; both land and both are told.
#[test]
fn sm3b_one_operator_finishes_one_act_a_fire() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 2);
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = act_go(&mut rig, pilot, "+tv/2 launch go");
    let due = due_of(&mut rig.tw, tvs[0]);
    assert_eq!(due_of(&mut rig.tw, tvs[1]), due, "due at one instant");
    let (events, queued) = advance_to(&mut rig, due);
    let mut heard = said(&rig.tw, &events, pilot);
    assert_eq!(finishes(&queued).len(), 1, "one finish the first fire");
    land_all(&mut rig, &queued);
    let (events, queued) = fire(&mut rig);
    heard.push_str(&said(&rig.tw, &events, pilot));
    assert_eq!(finishes(&queued).len(), 1, "the other the next");
    land_all(&mut rig, &queued);
    for _ in 0..2 {
        let (events, queued) = fire(&mut rig);
        land_all(&mut rig, &queued);
        heard.push_str(&said(&rig.tw, &events, pilot));
    }
    for (n, tv) in [(1, tvs[0]), (2, tvs[1])] {
        let record = record_of(&mut rig.tw, tv).expect("the record");
        assert_eq!(record.act, None);
        assert_eq!(
            record.last.map(|l| (l.result, l.at_ic)),
            Some((ActResult::Done, due)),
            "rig {n}"
        );
        let launched = LAUNCHED
            .replace("{n}", &n.to_string())
            .replace("{ramp}", "stern ramp")
            .replace("{site}", "Ahuna Flank");
        assert_eq!(count(&heard, &launched), 1, "rig {n}: {heard}");
    }
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 11. The slot selector (A10)
// ---------------------------------------------------------------------------

/// **The slot selector** (A10; lead ruling 6): `+tv/3` on the two-slot bay answers `NO_RIG`;
/// `+tv/5` is the dispatcher's unknown switch; with two rigs and nothing named, `+tv/launch`
/// answers `WHICH_RIG_ACT`; after `+tv/2` it previews rig 2; after a reboot (the pointer is
/// RAM) `+tv/launch go` answers `WHICH_RIG_ACT` again and queues nothing.
#[test]
fn sm3b_the_slot_selector() {
    let mut rig = stocked();
    let (pilot, _, _) = on_pad(&mut rig, 2);
    let out = run(&mut rig.tw, pilot, "+tv/3");
    assert_eq!(
        out.trim(),
        NO_RIG.replace("{n}", "3").replace("{name}", "Salt Moth")
    );
    let out = run(&mut rig.tw, pilot, "+tv/5");
    assert_eq!(out.trim(), "Unknown switch \"/5\" for +tv.");
    let which = WHICH_RIG_ACT
        .replace("{act}", "launch")
        .replace("{slots}", "1 or 2");
    let (out, queued) = command(&mut rig, pilot, "+tv/launch");
    assert_eq!(out.trim(), which);
    assert!(queued.is_empty());
    let _ = run(&mut rig.tw, pilot, "+tv/2");
    let (out, queued) = command(&mut rig, pilot, "+tv/launch");
    assert!(
        out.trim()
            .starts_with("Rig 2 (OWP-TV-02) would go down the stern ramp"),
        "{out}"
    );
    assert!(queued.is_empty());
    let mut rig = reboot(rig);
    let (out, queued) = command(&mut rig, pilot, "+tv/launch go");
    assert_eq!(out.trim(), which);
    assert!(queued.is_empty());
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 12. The spoil (A11)
// ---------------------------------------------------------------------------

/// **A site lays up its spoil, and no player may take from it** (A11; decision 6): each of
/// the six sites' map rows records a spoil Thing (`DARK`, in the host room) and a vault owned
/// by `System(tacsim.sites)` with no scope, backed by `Container(spoil)` and Active; the site
/// line reads Ready with the spoil; `live_endpoints` never lists a spoil vault for a pilot
/// and `may_take` refuses it; a reboot lays nothing twice, and nor does the real lay-up pass
/// run again — the `PortApplier`'s update over each of the six site entries of the shipped
/// lane, the path a lane version that touches a site takes (the fixer round's item 10a):
/// objects, vaults, the six spoil vaults and the map rows all as they were. Since doc 20 cut
/// SM.6 the same holds for each site's sixteen block vaults (D3), each backed by its block place.
#[test]
fn sm3b_a_site_lays_up_its_spoil_and_no_player_may_take_from_it() {
    let mut rig = stocked();
    let owner = PrincipalId::System(SystemPrincipalId::new("tacsim.sites").expect("an id"));
    let report = tacsim::port::readiness(rig.tw.world());
    let mut spoils = Vec::new();
    let mut blocks = Vec::new();
    for entry in [
        "commons_port",
        "kerwan_port",
        "ahuna_port",
        "juling_port",
        "occator_port",
    ]
    .into_iter()
    .chain(
        report
            .sites
            .iter()
            .map(CanonicalId::as_str)
            .filter(|entry| {
                ![
                    "commons_port",
                    "kerwan_port",
                    "ahuna_port",
                    "juling_port",
                    "occator_port",
                ]
                .contains(entry)
            }),
    ) {
        let map = port_map(&rig.tw, entry);
        let row = map.site.clone().expect("a site row");
        let thing = ObjId(row.spoil.expect("a spoil Thing"));
        let obj = rig.tw.world().get(thing).expect("live");
        assert!(obj.flags.contains(mush_core::Flags::DARK), "{entry}");
        assert_eq!(
            obj.location,
            map.host.map(ObjId),
            "{entry}: in the host room"
        );
        let vault = economy(&rig.tw)
            .snapshot()
            .vault(VaultId(row.spoil_vault.expect("a spoil vault")))
            .cloned()
            .expect("the vault");
        assert_eq!(vault.owner, owner, "{entry}");
        assert_eq!(vault.scope, None, "{entry}");
        assert_eq!(vault.backing, mush_economy::CustodyRef::Container(thing));
        assert_eq!(vault.status, mush_economy::VaultStatus::Active);
        let line = report
            .lines
            .iter()
            .find(|line| {
                line.binding == tacsim::port::Binding::Site
                    && line.port.as_ref().is_some_and(|p| p.as_str() == entry)
            })
            .expect("the site line");
        // Doc 20 cut SM.6 (D2, D3): the line counts the block vaults too, and this unkeyed
        // world's layer is dark, so nothing opens and the pad stays open.
        assert_eq!(
            line.text,
            format!(
                "{entry} site: 16 block places, the site clerk, the spoil and 16 block vaults \
                 laid up; no deposits while the geology layer is dark."
            )
        );
        spoils.push(vault.id);
        // Doc 20 cut SM.6 (D3; review T7): the sixteen block vaults — every surface
        // deposit's pool — are the sites' too, each backed by its own block place.
        assert_eq!(row.vaults.len(), 16, "{entry}: sixteen block vaults");
        for (letter, id) in &row.vaults {
            let place = ObjId(*row.blocks.get(letter).expect("the block's place"));
            let vault = economy(&rig.tw)
                .snapshot()
                .vault(VaultId(*id))
                .cloned()
                .expect("the block vault");
            assert_eq!(vault.owner, owner, "{entry} {letter}");
            assert_eq!(vault.scope, None, "{entry} {letter}");
            assert_eq!(
                vault.backing,
                mush_economy::CustodyRef::Container(place),
                "{entry} {letter}"
            );
            assert_eq!(vault.status, mush_economy::VaultStatus::Active);
            blocks.push(vault.id);
        }
    }
    assert_eq!(spoils.len(), 6, "six sites");
    assert_eq!(blocks.len(), 6 * 16, "96 block vaults");
    let (pilot, _) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    let stranger = rig.tw.spawn_player("Stranger");
    for who in [pilot, stranger] {
        let endpoints = mush_inventory::provider::live_endpoints(rig.tw.world(), who);
        for vault in &spoils {
            assert!(!endpoints.contains(vault), "#{} reaches a spoil", who.0);
            assert!(!mush_economy::may_take_from_vault(
                rig.tw.world(),
                who,
                *vault
            ));
        }
        for vault in &blocks {
            assert!(
                !endpoints.contains(vault),
                "#{} reaches a block vault",
                who.0
            );
            assert!(!mush_economy::may_take_from_vault(
                rig.tw.world(),
                who,
                *vault
            ));
        }
    }
    // A reboot lays nothing twice.
    let entries: Vec<String> = report.sites.iter().map(ToString::to_string).collect();
    assert_eq!(entries.len(), 6);
    let census = |tw: &TestWorld| {
        let snapshot = economy(tw).snapshot();
        let ours = snapshot
            .vaults()
            .filter(|vault| vault.owner == owner)
            .map(|vault| vault.id)
            .collect::<Vec<_>>();
        (
            tw.world().iter().count(),
            snapshot.vaults().count(),
            ours,
            entries
                .iter()
                .map(|entry| port_map(tw, entry).site)
                .collect::<Vec<_>>(),
        )
    };
    let before = census(&rig.tw);
    // Doc 20 cut SM.6 (D3): six spoil vaults and sixteen block vaults a site, owned by the
    // sites.
    assert_eq!(
        before.2.len(),
        6 * 17,
        "six spoil vaults and 96 block vaults, owned by the sites"
    );
    let mut rig = reboot(rig);
    assert_eq!(census(&rig.tw), before, "a reboot lays nothing");
    // The real lay-up pass, run again over every site entry.
    let pack = mush_content::load_pack(tacsim::definitions::content::LANE_JSON)
        .expect("the lane verifies");
    let placed = rig
        .tw
        .world()
        .module_state::<mush_content::ContentState>()
        .and_then(|state| state.latches.applied(&cid(LANE_PACK)))
        .map(mush_content::Applied::placed)
        .expect("the lane is applied");
    for entry in &entries {
        let id = cid(entry);
        let raw = pack.entry(&id).cloned().expect("a site entry");
        let applied = support::with_ctx(&mut rig.tw, |ctx| {
            mush_content::EntryApplier::update(
                &tacsim::port::apply::PortApplier,
                ctx,
                &pack,
                &id,
                &raw,
                None,
                &placed,
            )
        });
        assert!(applied.is_ok(), "{entry}: {applied:?}");
    }
    assert_eq!(
        census(&rig.tw),
        before,
        "the lay-up pass lays nothing twice"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 13. A charter's end with a rig out (decision 13)
// ---------------------------------------------------------------------------

/// **A rig out on a site holds the revoke** (decision 13; review finding tests/T1). A revoke
/// reaches `repossess` only at the programme port (`recall_block` check 1 answers
/// `RECALL_NOT_AT_PORT` anywhere else), so the test drives both halves:
///
/// - the pilot's hull settled on Ahuna's pad with rig 1 out on the ground: the revoke stops
///   at check 1 — exactly `NOT_REVOKED` + `RECALL_NOT_AT_PORT` — and the rig, its record,
///   its lease and the charter stand;
/// - **the bay path**: a second pilot's Dray-P docked at Ceres with its rig in the bay and a
///   `tacsim.excursion` record that (a) holds an open excursion, then (b) an act in flight:
///   each revoke reaches check 7, `repossess` refuses on the record, and the reply is exactly
///   `NOT_REVOKED` + `RECALL_BAY` — the rig still in the bay, its lease Active, its record
///   unchanged, the charter live. As the control, (c) the record with neither — the same
///   revoke repossesses the rig and revokes the charter.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3b_a_rig_on_the_ground_holds_the_revoke() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    let record = record_of(&mut rig.tw, tv);
    let out = run(&mut rig.tw, GOD, "@coop/revoke Pilot=test");
    let not_at_port = format!(
        "{}{}",
        tacsim::port::recall::NOT_REVOKED,
        tacsim::port::recall::RECALL_NOT_AT_PORT
            .replace("{name}", "Salt Moth")
            .replace("{port}", "Ceres Freeport")
    );
    assert_eq!(out.trim(), not_at_port);
    assert!(on_ground_at(&rig.tw, tv, RAMP), "the rig stays where it is");
    assert_eq!(record_of(&mut rig.tw, tv), record, "the record stands");
    assert!(record.and_then(|r| r.excursion).is_some());
    let lease = store(&rig.tw)
        .tv_lease_on(tv)
        .cloned()
        .expect("the lease record stays live");
    assert_eq!(lease.status, TvLeaseStatus::Active);
    assert!(tacsim::port::live_charter(rig.tw.world(), pilot).is_some());

    // The bay path, at Ceres.
    let (other, row_o) = prospector(&mut rig, "Other", 2, "Tally Moth");
    lease_one(&mut rig, other);
    let tv_o = rigs_of(&rig.tw, other)[0];
    let hull_o = ObjId(row_o.hull);
    assert!(in_bay(&rig.tw, tv_o, &row_o));
    let battery = tacsim::definitions::tools::tools_table()
        .expect("the tools table")
        .values()
        .find_map(tacsim::definitions::tools::ToolDef::battery)
        .cloned()
        .expect("a battery row");
    let now = ic_now(&rig.tw);
    let mut open = RigRecord::fresh(tv_o, &battery);
    open.excursion = Some(Excursion {
        id: ExcursionId {
            tv: tv_o,
            launch_ic: now,
        },
        operator: other,
        hull: hull_o,
        slot: 1,
        site: cid("ahuna_port"),
    });
    let mut busy = RigRecord::fresh(tv_o, &battery);
    busy.ordinal = 1;
    busy.act = Some(Act {
        ordinal: 1,
        kind: ActKind::Recover,
        operator: other,
        hull: hull_o,
        slot: 1,
        site: cid("ahuna_port"),
        started_ic: now,
        // Far off, so no surface fire finishes it while the test reads.
        due_ic: now + 1_000_000,
        reserved_cu: 0,
        params: ActParams::None {},
        by: None,
    });
    let held = format!(
        "{}{}",
        tacsim::port::recall::NOT_REVOKED,
        tacsim::port::recall::RECALL_BAY.replace("{name}", "Tally Moth")
    );
    for (case, seeded) in [("an open excursion", open), ("an act in flight", busy)] {
        put_record(&mut rig, seeded.clone());
        let out = run(&mut rig.tw, GOD, "@coop/revoke Other=test");
        assert_eq!(out.trim(), held, "{case}");
        assert!(
            in_bay(&rig.tw, tv_o, &row_o),
            "{case}: the rig stays aboard"
        );
        assert_eq!(
            store(&rig.tw).tv_lease_on(tv_o).map(|lease| lease.status),
            Some(TvLeaseStatus::Active),
            "{case}: the lease stands"
        );
        assert_eq!(
            record_of(&mut rig.tw, tv_o),
            Some(seeded),
            "{case}: the record stands"
        );
        assert!(
            tacsim::port::live_charter(rig.tw.world(), other).is_some(),
            "{case}: the charter stands"
        );
    }
    // The control: neither an excursion nor an act — the revoke repossesses the rig.
    put_record(&mut rig, RigRecord::fresh(tv_o, &battery));
    let out = run(&mut rig.tw, GOD, "@coop/revoke Other=test");
    assert!(
        out.contains("Charter of the Tally Moth (") && out.contains(") revoked from Other: test."),
        "{out}"
    );
    assert!(!in_bay(&rig.tw, tv_o, &row_o), "repossessed");
    assert!(tacsim::port::live_charter(rig.tw.world(), other).is_none());
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 15. A rig out of place can still come home (decision 21)
// ---------------------------------------------------------------------------

/// **A rig a staff move left out of place can still come home** (decision 21): (i) launched
/// and finished, then staff put back in the bay with its excursion open — `+ship/lift ceres`
/// reads `TV_OUT`, `+tv/1 launch` `LAUNCH_OPEN`, `+tv/1 recover go` lands and finishes
/// `Done`, the excursion closes and `+ship/lift ceres` files; (ii) a rig never launched, staff
/// placed on the site at the ramp pose — `+tv` reads it at the ramp, `+tv/1 recover go` lands
/// with no excursion and finishes `Done`, the rig in the bay.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3b_a_rig_a_staff_move_left_out_of_place_can_still_come_home() {
    // (i)
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    staff_bay(&mut rig, tv, &row);
    assert!(in_bay(&rig.tw, tv, &row));
    let (out, queued) = command(&mut rig, pilot, "+ship/lift ceres");
    assert_eq!(
        out.trim(),
        TV_OUT
            .replace("{n}", "1")
            .replace("{site}", "Ahuna Flank")
            .replace("{name}", "Salt Moth")
    );
    assert!(queued.is_empty());
    let (out, queued) = command(&mut rig, pilot, "+tv/1 launch");
    assert_eq!(out.trim(), LAUNCH_OPEN.replace("{n}", "1"));
    assert!(queued.is_empty());
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let _ = finish(&mut rig, tv, pilot);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(record.excursion, None);
    assert_eq!(
        record.last.map(|l| (l.kind, l.result)),
        Some((ActKind::Recover, ActResult::Done))
    );
    let (out, queued) = command(&mut rig, pilot, "+ship/lift ceres");
    assert!(out.contains("Clearance filed for "), "{out}");
    assert_eq!(queued.len(), 1);

    // (ii)
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    staff_place(&mut rig, tv, RAMP.0, RAMP.1);
    let status = run(&mut rig.tw, pilot, "+tv");
    assert!(
        status.contains(
            "Rig 1: OWP-TV-01, on the ground at the stern ramp on Ahuna Flank; charge 100 %; "
        ),
        "{status}"
    );
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    assert_eq!(
        record_of(&mut rig.tw, tv).and_then(|r| r.excursion),
        None,
        "no excursion was needed"
    );
    let _ = finish(&mut rig, tv, pilot);
    assert!(in_bay(&rig.tw, tv, &row));
    assert_eq!(
        record_of(&mut rig.tw, tv)
            .and_then(|r| r.last)
            .map(|l| (l.kind, l.result)),
        Some((ActKind::Recover, ActResult::Done))
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 16. A finish that does not land (finding F2)
// ---------------------------------------------------------------------------

/// **A finish that does not land backs off and retries** (finding F2; the world half of the
/// attempt rule, whose binding half is `tick.rs`'s unit test): a launch lands; before its due
/// instant its record is rewritten so the act's operator is a second player who holds no rig
/// lease, so the finish is offered to nobody and never lands. Past the due instant the tick
/// submits it each fire; on the eighth counted attempt it backs off (the act still in flight,
/// the fire passing it over); the operator restored, the retry after `DONE_BACKOFF_FIRES`
/// lands and finishes.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3b_a_finish_that_does_not_land_backs_off_and_retries() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let stranger = rig.tw.spawn_player("Stranger");
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let mut record = record_of(&mut rig.tw, tv).expect("the record");
    if let Some(act) = record.act.as_mut() {
        act.operator = stranger;
    }
    put_record(&mut rig, record.clone());
    let due = due_of(&mut rig.tw, tv);
    let (_, queued) = advance_to(&mut rig, due);
    let done = finishes(&queued);
    assert_eq!(done.len(), 1);
    assert_eq!(
        done[0].actor, stranger,
        "the finish is the act's operator's"
    );
    assert!(land(&mut rig, done[0]).is_err(), "offered to nobody");
    // Fires until the back-off: every visit submits until the eighth counted attempt.
    let mut fires = 0;
    loop {
        let (_, queued) = fire(&mut rig);
        fires += 1;
        let stats = tacsim::surface::last_surface_fire(rig.tw.world()).expect("a fire");
        if stats.passed_over == 1 {
            assert!(finishes(&queued).is_empty());
            break;
        }
        for submit in finishes(&queued) {
            assert!(land(&mut rig, submit).is_err());
        }
        land_quietly(&mut rig, &queued);
        assert!(fires < 20, "the back-off never began");
    }
    assert_eq!(
        fires,
        usize::from(tacsim::surface::MAX_DONE_ATTEMPTS),
        "eight counted attempts"
    );
    assert!(
        record_of(&mut rig.tw, tv).and_then(|r| r.act).is_some(),
        "the act is still in flight"
    );
    // The operator restored; the back-off runs out and the retry lands.
    if let Some(act) = record.act.as_mut() {
        act.operator = pilot;
    }
    put_record(&mut rig, record);
    let mut landed = false;
    for _ in 1..=tacsim::surface::DONE_BACKOFF_FIRES {
        let (_, queued) = fire(&mut rig);
        let done = finishes(&queued);
        if let Some(submit) = done.first() {
            assert_eq!(submit.actor, pilot);
            land_all(&mut rig, &queued);
            landed = true;
            break;
        }
        land_quietly(&mut rig, &queued);
        let stats = tacsim::surface::last_surface_fire(rig.tw.world()).expect("a fire");
        assert_eq!(stats.passed_over, 1, "passed over in the back-off");
    }
    assert!(landed, "the retry after the back-off lands");
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(record.act, None);
    assert_eq!(
        record.last.map(|l| (l.result, l.at_ic)),
        Some((ActResult::Done, due))
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 17. A finish the world refuses (review finding tests/T4)
// ---------------------------------------------------------------------------

/// **A finish the world refuses is told and keeps what it should** (decision 5; the finish's
/// `Refused` arms; review finding tests/T4): (i) a recover started from beside the ramp, then
/// staff move the rig two cells off before it is due — the tick places nothing, the finish
/// records `Refused { NotBeside }`, the excursion stays open, the charge is untouched and the
/// pilot hears `ACT_FAILED` naming the ramp once; (ii) a launch started from the bay, then
/// staff move the rig onto the ground three cells off before it is due — the finish records
/// `Refused { NeverLeft }`, the excursion closes (the reservation released), the charge is
/// untouched and the pilot hears `ACT_FAILED` once.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3b_a_finish_the_world_refuses_is_told_and_keeps_what_it_should() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    let launched = record_of(&mut rig.tw, tv).expect("the record");
    assert!(launched.excursion.is_some());

    // (i) NotBeside.
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    staff_place(&mut rig, tv, RAMP.0, RAMP.1 - 2_000);
    let heard = finish(&mut rig, tv, pilot);
    assert!(
        on_ground_at(&rig.tw, tv, (RAMP.0, RAMP.1 - 2_000, 180_000)),
        "the tick placed nothing: {:?}",
        parent(&rig.tw, tv)
    );
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(record.act, None);
    assert_eq!(
        record.last.as_ref().map(|last| (last.kind, last.result)),
        Some((
            ActKind::Recover,
            ActResult::Refused {
                code: RefusedCode::NotBeside
            }
        ))
    );
    assert_eq!(
        record.excursion, launched.excursion,
        "the excursion stays open"
    );
    assert_eq!(record.charge, launched.charge, "the charge untouched");
    let failed = ACT_FAILED
        .replace("{n}", "1")
        .replace("{act}", "recover")
        .replace("{why}", &WHY_NOT_BESIDE.replace("{ramp}", "stern ramp"));
    assert_eq!(count(&heard, &failed), 1, "{heard}");
    assert_eq!(
        projection(&rig.tw, tv).map(|rec| (rec.state, rec.told)),
        Some((TvState::Out, 2))
    );

    // Home the ordinary way, then (ii) NeverLeft.
    staff_place(&mut rig, tv, RAMP.0, RAMP.1);
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let _ = finish(&mut rig, tv, pilot);
    assert!(in_bay(&rig.tw, tv, &row));
    let home = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(home.excursion, None);
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    assert!(
        record_of(&mut rig.tw, tv)
            .and_then(|r| r.excursion)
            .is_some(),
        "the start opens the excursion"
    );
    staff_place(&mut rig, tv, RAMP.0, RAMP.1 - 3_000);
    let heard = finish(&mut rig, tv, pilot);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(
        record.last.as_ref().map(|last| (last.kind, last.result)),
        Some((
            ActKind::Launch,
            ActResult::Refused {
                code: RefusedCode::NeverLeft
            }
        ))
    );
    assert_eq!(record.excursion, None, "the excursion closes");
    assert_eq!(record.charge, home.charge, "the charge untouched");
    // The rig stands on the ground with no excursion open: the projection reads the ground,
    // not the record (the fixer round's item 6).
    assert!(on_ground_at(&rig.tw, tv, (RAMP.0, RAMP.1 - 3_000, 180_000)));
    assert_eq!(
        projection(&rig.tw, tv).map(|rec| rec.state),
        Some(TvState::Out),
        "a rig on the ground projects Out"
    );
    let failed = ACT_FAILED
        .replace("{n}", "1")
        .replace("{act}", "launch")
        .replace("{why}", WHY_NEVER_LEFT);
    assert_eq!(count(&heard, &failed), 1, "{heard}");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 18. The planners' other gates (review finding tests/T6)
// ---------------------------------------------------------------------------

/// Fold every write and reboot with the `tacsim:tvlease:` record of `lease` edited
/// (`vehicles.rs`'s helper; staff's hand on the document).
fn reboot_editing_lease(
    mut rig: Rig,
    lease: u64,
    edit: impl FnOnce(&mut serde_json::Value),
) -> Rig {
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

/// A start of `act` for `tv` at `ordinal`, built as the client would.
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

/// [`start`], landed; the error carries the prepare's own message.
fn land_start(
    rig: &mut Rig,
    pilot: ObjId,
    tv: ObjId,
    act: &str,
    ordinal: u32,
) -> Result<(), String> {
    let submit = start(rig, pilot, tv, act, ordinal);
    land(rig, &submit)
}

/// `line` answers exactly `want` and queues nothing.
fn refuses(rig: &mut Rig, who: ObjId, line: &str, want: &str) {
    let (out, queued) = command(rig, who, line);
    assert_eq!(out.trim(), want, "{line}");
    assert!(queued.is_empty(), "{line} queued nothing");
}

/// **The planners' other gates refuse at the command and at the prepare** (the brief's 3.3;
/// decisions 11, 14 and 21; review finding tests/T6), each the whole reply with nothing
/// queued, and — where the gate is the offer's fixed plan — a built start refused at the
/// prepare by its message: an empty slot within the bay (`+tv/2 launch` with one rig) is
/// `NO_RIG`; the pilot off the hull hears `TV_NOT_ABOARD` for launch, recover and dump (the
/// control lease read as the lessee aboard, decision 14); a rig on the ground answers a launch
/// `LAUNCH_OUT`; a rig staff moved onto another site's ground answers a recover and a dump
/// `RECOVER_ELSEWHERE` and `+tv` reads it `WHERE_AWAY`; a staff-ended rig lease refuses the
/// launch `LAUNCH_ENDED` but never the dump (decision 11); a forfeit record refuses the
/// launch `LAUNCH_FORFEIT`; a site whose spoil vault the map no longer records refuses the
/// dump `DUMP_NO_SPOIL`.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3b_the_act_gates_refuse_at_the_command_and_at_the_prepare() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    // An empty slot within the bay.
    refuses(
        &mut rig,
        pilot,
        "+tv/2 launch",
        &NO_RIG.replace("{n}", "2").replace("{name}", "Salt Moth"),
    );

    // Off the bridge: staff move the pilot from the hull's airlock space down into its
    // bay (aboard is the airlock interior, `voyage::aboard`); every act refuses, and so does
    // the built start at the prepare.
    let SpatialParent::Interior {
        space: airlock,
        slot: airlock_slot,
    } = parent(&rig.tw, pilot)
    else {
        panic!("the pilot is aboard: {:?}", parent(&rig.tw, pilot));
    };
    let bay = ObjId(row.rooms[&cid("bay")]);
    keeping(&mut rig, |ctx| {
        mush_sim::place_interior(ctx, pilot, bay, None)
    })
    .expect("staff move the pilot into the bay");
    let not_aboard = TV_NOT_ABOARD.replace("{name}", "Salt Moth");
    for line in ["+tv/1 launch", "+tv/1 recover go", "+tv/1 dump"] {
        refuses(&mut rig, pilot, line, &not_aboard);
    }
    let refused = land_start(&mut rig, pilot, tv, "launch", 1).expect_err("not aboard");
    assert!(
        refused.contains("the operator is not aboard the hull"),
        "{refused}"
    );
    assert_eq!(record_of(&mut rig.tw, tv), None, "nothing written");
    keeping(&mut rig, |ctx| {
        mush_sim::place_interior(ctx, pilot, airlock, airlock_slot)
    })
    .expect("staff put the pilot back aboard");

    // A rig on the ground: a launch is LAUNCH_OUT, at the command and at the prepare.
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    refuses(
        &mut rig,
        pilot,
        "+tv/1 launch go",
        &LAUNCH_OUT.replace("{n}", "1"),
    );
    let refused = land_start(&mut rig, pilot, tv, "launch", 2).expect_err("out");
    assert!(
        refused.contains("that rig is already out of the bay"),
        "{refused}"
    );

    // Staff move it onto Kerwan's ground: elsewhere.
    let kerwan = mush_sim::DomainId(port_map(&rig.tw, "kerwan_port").domain.expect("a domain"));
    staff_place_on(&mut rig, tv, kerwan, RAMP.0, RAMP.1);
    let elsewhere = RECOVER_ELSEWHERE
        .replace("{n}", "1")
        .replace("{name}", "Salt Moth");
    refuses(&mut rig, pilot, "+tv/1 recover", &elsewhere);
    refuses(&mut rig, pilot, "+tv/1 dump go", &elsewhere);
    let refused = land_start(&mut rig, pilot, tv, "recover", 2).expect_err("elsewhere");
    assert!(
        refused.contains("that rig is not beside the hull"),
        "{refused}"
    );
    let status = run(&mut rig.tw, pilot, "+tv");
    let away = format!(
        "Rig 1: OWP-TV-01, {}; charge 100 %; ",
        WHERE_AWAY.replace("{name}", "Salt Moth")
    );
    assert!(status.contains(&away), "{status}\nwanted: {away}");

    // Home, then staff end Occator's lease: the launch refuses, the dump does not.
    staff_place(&mut rig, tv, RAMP.0, RAMP.1);
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let _ = finish(&mut rig, tv, pilot);
    assert!(in_bay(&rig.tw, tv, &row));
    let lease = store(&rig.tw)
        .tv_lease_on(tv)
        .cloned()
        .expect("the lease record");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@contract/cancel #{}", lease.lease),
    );
    let counter = rig
        .tw
        .world()
        .get(station_room(&rig.tw, "ceres_port", "room.c07_exchange"))
        .map(|o| o.name.clone())
        .expect("the counter");
    let at_counter = |template: &str| {
        template
            .replace("{n}", "1")
            .replace("{counter}", &counter)
            .replace("{port}", "Ceres Freeport")
    };
    refuses(&mut rig, pilot, "+tv/1 launch", &at_counter(LAUNCH_ENDED));
    let ordinal = record_of(&mut rig.tw, tv).expect("the record").ordinal + 1;
    let refused = land_start(&mut rig, pilot, tv, "launch", ordinal).expect_err("ended");
    assert!(refused.contains("that rig's lease has ended"), "{refused}");
    fill_hopper(&mut rig, tv, 1, 9_840);
    let (out, queued) = command(&mut rig, pilot, "+tv/1 dump");
    assert!(
        out.trim()
            .starts_with("Rig 1 (OWP-TV-01) would dump 1 t of metal ore, run-of-mine"),
        "the dump is never lease-gated: {out}"
    );
    assert!(queued.is_empty());

    // A forfeit record: LAUNCH_FORFEIT (checked before the ended lease).
    let mut rig = reboot_editing_lease(rig, lease.lease, |doc| {
        doc["status"] = serde_json::json!("forfeit");
    });
    assert_eq!(
        store(&rig.tw).tv_lease_on(tv).map(|record| record.status),
        Some(TvLeaseStatus::Forfeit)
    );
    refuses(
        &mut rig,
        pilot,
        "+tv/1 launch go",
        &at_counter(LAUNCH_FORFEIT),
    );
    let refused = land_start(&mut rig, pilot, tv, "launch", ordinal).expect_err("forfeit");
    assert!(
        refused.contains("that rig's deposit is forfeit"),
        "{refused}"
    );

    // A site whose map no longer records its spoil vault: DUMP_NO_SPOIL.
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    let spoil_vault = port_map(&rig.tw, "ahuna_port")
        .site
        .and_then(|row| row.spoil_vault)
        .expect("the spoil vault");
    let edited = rig
        .store
        .values_mut()
        .flat_map(|data| data.kv.iter_mut())
        .filter_map(|(_, value)| value.get_mut("site"))
        .filter(|site| site.get("spoil_vault") == Some(&serde_json::json!(spoil_vault)))
        .map(|site| site.as_object_mut().map(|row| row.remove("spoil_vault")))
        .count();
    assert_eq!(edited, 1, "one map row records the spoil vault");
    let mut rig = reboot(rig);
    refuses(
        &mut rig,
        pilot,
        "+tv/1 dump go",
        &DUMP_NO_SPOIL.replace("{site}", "Ahuna Flank"),
    );
    let refused = land_start(&mut rig, pilot, tv, "dump", ordinal).expect_err("no spoil");
    assert!(
        refused.contains("the site has no spoil laid out"),
        "{refused}"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 19. A rig still logged out at Occator's counter (the fixer round's items 1 and 2)
// ---------------------------------------------------------------------------

/// Write `record` under `key` — which need not be its own (staff's hand; a record that
/// disagrees with its key is the families rule's unreadable case).
fn put_record_under(rig: &mut Rig, key: RigKey, record: RigRecord) {
    let family = handles(&rig.tw).families.excursions;
    let engine = rig.tw.engine_mut();
    let prepared = {
        let mut txn = engine
            .action_transaction(&family)
            .expect("an excursion transaction");
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

/// The status of hold `id`.
fn hold_status(tw: &TestWorld, id: u64) -> mush_economy::HoldStatus {
    economy(tw)
        .snapshot()
        .hold(HoldId(id))
        .map(|hold| hold.status.clone())
        .expect("the hold")
}

/// **A rig still logged out holds the return and names a remedy that works** (the fixer
/// round's items 1 and 2; decisions 13 and 21). Rig 1 is launched and finished on Ahuna, then
/// staff put it back in the bay with its excursion still open and move the hull onto Ceres's
/// berth — the one way a rig can be out while its hull is at a port:
///
/// - the departure, at a port: `+ship/undock ahuna` and `+ship/plan ahuna` read `TV_AWAY`
///   naming the excursion's site (a recover cannot work at a port, so the remedy is staff's);
///   the rig staff placed on Ceres's own ground with no excursion open reads it `away`;
/// - the lease return at Occator's counter, bare and with `go`, answers exactly
///   `RIG_LOGGED_OUT` and changes nothing: the lease record, the deposit hold, the typed
///   record and the rig's place in the bay all as they were;
/// - a record that will not read (one that disagrees with its own key) refuses the return
///   `LEASE_FAILED` (fail closed), again changing nothing.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3b_a_rig_still_logged_out_holds_the_return_and_the_hull_at_a_port() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hull = ObjId(row.hull);
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    // Staff: the rig back in the bay with its excursion open, the hull onto Ceres's berth.
    staff_bay(&mut rig, tv, &row);
    let ceres = mush_sim::DomainId(
        port_map(&rig.tw, "ceres_port")
            .domain
            .expect("Ceres's domain"),
    );
    staff_place_on(&mut rig, hull, ceres, 0, 0);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == "ceres_port"
        ),
        "the hull stands at Ceres"
    );
    let logged = record_of(&mut rig.tw, tv).expect("the record");
    assert!(logged.excursion.is_some() && logged.act.is_none());
    assert!(in_bay(&rig.tw, tv, &row));

    // Item 2, the port branch: the remedy is staff's.
    let away = TV_AWAY
        .replace("{n}", "1")
        .replace("{where}", "on Ahuna Flank")
        .replace("{name}", "Salt Moth");
    let (out, queued) = command(&mut rig, pilot, "+ship/undock ahuna");
    assert_eq!(out.trim(), away);
    assert!(queued.is_empty());
    let plan = run(&mut rig.tw, pilot, "+ship/plan ahuna");
    assert_eq!(plan.lines().nth(1), Some(away.as_str()), "{plan}");
    // No site can be named: the rig on Ceres's own ground, no excursion open.
    let mut idle = logged.clone();
    idle.excursion = None;
    put_record(&mut rig, idle);
    staff_place_on(&mut rig, tv, ceres, 1_000, 1_000);
    let (out, queued) = command(&mut rig, pilot, "+ship/undock ahuna");
    assert_eq!(
        out.trim(),
        TV_AWAY
            .replace("{n}", "1")
            .replace("{where}", "away from her bay")
            .replace("{name}", "Salt Moth")
    );
    assert!(queued.is_empty());
    staff_bay(&mut rig, tv, &row);
    put_record(&mut rig, logged.clone());

    // Item 1: the lease return at the counter refuses and changes nothing.
    let lease = store(&rig.tw)
        .tv_lease_on(tv)
        .cloned()
        .expect("the live lease record");
    let hold = hold_status(&rig.tw, lease.hold);
    let out = run(&mut rig.tw, pilot, "+ship/leave");
    assert!(
        rig.tw.world().get(pilot).and_then(|o| o.location)
            != Some(ObjId(row.rooms[&cid("bridge")])),
        "off the hull: {out}"
    );
    let counter = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, pilot, counter);
    let unchanged = |rig: &mut Rig, case: &str| {
        assert_eq!(
            store(&rig.tw).tv_lease_on(tv),
            Some(&lease),
            "{case}: the lease record"
        );
        assert_eq!(
            hold_status(&rig.tw, lease.hold),
            hold,
            "{case}: the deposit hold"
        );
        assert!(in_bay(&rig.tw, tv, &row), "{case}: the rig stays aboard");
    };
    let out_line = RIG_LOGGED_OUT
        .replace("{n}", "1")
        .replace("{site}", "Ahuna Flank");
    for line in ["+tv/1 lease return", "+tv/1 lease return go"] {
        refuses(&mut rig, pilot, line, &out_line);
        unchanged(&mut rig, line);
        assert_eq!(record_of(&mut rig.tw, tv).as_ref(), Some(&logged), "{line}");
    }
    // A record that will not read: fail closed.
    let mut foreign = RigRecord::fresh(ObjId(tv.0 + 1_000_000), &battery());
    foreign.excursion = None;
    put_record_under(&mut rig, RigKey(tv), foreign);
    refuses(&mut rig, pilot, "+tv/1 lease return go", LEASE_FAILED);
    unchanged(&mut rig, "an unreadable record");
    no_refused_writes(&rig.tw);
}

/// The shipped Mk1 battery row.
fn battery() -> tacsim::definitions::tools::BatteryDef {
    tacsim::definitions::tools::tools_table()
        .expect("the tools table")
        .values()
        .find_map(tacsim::definitions::tools::ToolDef::battery)
        .cloned()
        .expect("a battery row")
}

// ---------------------------------------------------------------------------
// 20. Never stranded by its battery (the fixer round's item 3)
// ---------------------------------------------------------------------------

/// Staff take every battery out of the one tools table the layer reads (`PortHandles`
/// `.surface_tools`): the rig's battery row no longer resolves.
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

/// **A rig can always come home without its battery row** (§4.1 "never stranded by its
/// battery"; the fixer round's item 3): rig 1 launched and finished, standing at the ramp;
/// then its battery row stops resolving. `+tv` reads its charge `?`; `+tv/1 recover`
/// previews `RECOVER_PREVIEW_UNREAD`, and `go` starts, lands and finishes `Done`, the rig in
/// the bay and its charge back on the umbilical at the same value; a hopper of 2 t dumps,
/// started, landed and finished `Done`, the spoil 2 t up; and a launch still fails closed —
/// `TV_DARK` at the command and "that rig cannot be read" at the prepare.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3b_a_rig_is_never_stranded_by_its_battery() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let _ = act_go(&mut rig, pilot, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, pilot);
    let launched = record_of(&mut rig.tw, tv).expect("the record");
    assert!(!launched.charge.in_bay);
    unbattery(&mut rig);
    let status = run(&mut rig.tw, pilot, "+tv");
    assert!(
        status.contains(
            "Rig 1: OWP-TV-01, on the ground at the stern ramp on Ahuna Flank; charge ? %; "
        ),
        "{status}"
    );
    // The recover: previewed, started, finished.
    let (out, queued) = command(&mut rig, pilot, "+tv/1 recover");
    assert_eq!(
        out.trim(),
        RECOVER_PREVIEW_UNREAD
            .replace("{n}", "1")
            .replace("{mark}", "OWP-TV-01")
            .replace("{ramp}", "stern ramp")
            .replace("{name}", "Salt Moth")
            .replace("{secs}", "300")
            .replace("{real}", ", about 5 min")
            .replace("{hopper}", "empty")
            // Doc 20 cut SM.8: an empty hopper's recover names no unload.
            .replace("{then}", "")
    );
    assert!(queued.is_empty());
    let _ = act_go(&mut rig, pilot, "+tv/1 recover go");
    let due = due_of(&mut rig.tw, tv);
    let heard = finish(&mut rig, tv, pilot);
    assert!(in_bay(&rig.tw, tv, &row));
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(
        record.last.as_ref().map(|last| (last.kind, last.result)),
        Some((ActKind::Recover, ActResult::Done))
    );
    assert_eq!(record.excursion, None);
    assert_eq!(
        record.charge,
        Charge {
            value_cu: launched.charge.value_cu,
            at_ic: due,
            in_bay: true,
        },
        "back on the umbilical at the value it had"
    );
    let recovered = RECOVERED.replace("{n}", "1").replace("{name}", "Salt Moth");
    assert_eq!(count(&heard, &recovered), 1, "{heard}");
    // The dump, from the bay.
    fill_hopper(&mut rig, tv, 2, 9_850);
    let spoil = VaultId(
        port_map(&rig.tw, "ahuna_port")
            .site
            .and_then(|row| row.spoil_vault)
            .expect("the spoil vault"),
    );
    let spoil_before = pool(&mut rig.tw, spoil, "metal_ore");
    let _ = act_go(&mut rig, pilot, "+tv/1 dump go");
    let _ = finish(&mut rig, tv, pilot);
    let hopper_v = hopper(&rig.tw, tv);
    assert_eq!(pool(&mut rig.tw, hopper_v, "metal_ore"), 0);
    assert_eq!(pool(&mut rig.tw, spoil, "metal_ore"), spoil_before + 2);
    assert_eq!(
        record_of(&mut rig.tw, tv)
            .and_then(|record| record.last)
            .map(|last| (last.kind, last.result)),
        Some((ActKind::Dump, ActResult::Done))
    );
    // The launch fails closed, at the command and at the prepare.
    refuses(
        &mut rig,
        pilot,
        "+tv/1 launch",
        &TV_DARK.replace("{name}", "Salt Moth"),
    );
    let ordinal = record_of(&mut rig.tw, tv).expect("the record").ordinal + 1;
    let before = record_of(&mut rig.tw, tv);
    let refused = land_start(&mut rig, pilot, tv, "launch", ordinal).expect_err("no battery");
    assert!(refused.contains("that rig cannot be read"), "{refused}");
    assert_eq!(record_of(&mut rig.tw, tv), before, "nothing written");
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 21. A dump the spoil will not take (the fixer round's item 4)
// ---------------------------------------------------------------------------

/// **A dump the spoil will not take records a refusal and never wedges** (the fixer round's
/// item 4):
///
/// - (a) the spoil broken between the start and the finish (its Thing flagged `GOING`): the
///   finish lands `Refused { SpoilGone }` at the act's own instant, nothing moved, the pilot
///   hears `ACT_FAILED` with `WHY_SPOIL_GONE` once, the rig is free (a later dump, the spoil
///   mended, starts and finishes `Done`) and the hull reads `HOPPER_LOADED`, never `TV_BUSY`;
/// - (b1) **doc 20 cut SM.6 (lead ruling 5; SM.3b's round F3, fixed inventory-side)**: two ores
///   the site's spoil has never held now land in **one** bundle (`transfer_many` stages the
///   spoil's pool census once): the first finish lands `Done`, the spoil holds 2 and 1, the
///   hopper is empty;
/// - (b2) a transfer the spoil side refuses mid-bundle, by another real cause: the spoil's pool
///   census filled to `MAX_POOLS_PER_ENDPOINT − 1` with seeded test commodities, two ores it has
///   never held would make it 65: the first finish does not land and writes nothing (the
///   hopper's two pools exact, the act still in flight); the next fire's finish records
///   `SpoilGone` and lands, the hopper still exact, the act released, the hull held only by
///   `HOPPER_LOADED`.
#[test]
#[allow(clippy::too_many_lines)]
fn sm3b_a_dump_the_spoil_will_not_take_is_refused_and_never_wedges() {
    // (a)
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hopper_v = hopper(&rig.tw, tv);
    let site = port_map(&rig.tw, "ahuna_port").site.expect("the site row");
    let spoil = VaultId(site.spoil_vault.expect("the spoil vault"));
    let spoil_thing = ObjId(site.spoil.expect("the spoil Thing"));
    fill_hopper(&mut rig, tv, 2, 9_860);
    let _ = act_go(&mut rig, pilot, "+tv/1 dump go");
    let due = due_of(&mut rig.tw, tv);
    rig.tw
        .world_mut()
        .set_flag(spoil_thing, mush_core::Flags::GOING, true)
        .expect("flagged");
    let heard = finish(&mut rig, tv, pilot);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(record.act, None, "the act is released");
    assert_eq!(
        record.last.as_ref().map(|last| (
            last.kind,
            last.result,
            last.at_ic,
            last.moved.is_empty()
        )),
        Some((
            ActKind::Dump,
            ActResult::Refused {
                code: RefusedCode::SpoilGone
            },
            due,
            true
        ))
    );
    assert_eq!(
        pool(&mut rig.tw, hopper_v, "metal_ore"),
        2,
        "the load stays"
    );
    assert_eq!(pool(&mut rig.tw, spoil, "metal_ore"), 0, "nothing moved");
    let failed = ACT_FAILED
        .replace("{n}", "1")
        .replace("{act}", "dump")
        .replace("{why}", WHY_SPOIL_GONE);
    assert_eq!(count(&heard, &failed), 1, "{heard}");
    let loaded = HOPPER_LOADED
        .replace("{n}", "1")
        .replace("{t}", "2")
        .replace("{name}", "Salt Moth");
    refuses(&mut rig, pilot, "+ship/lift ceres", &loaded);
    // The rig is free: the spoil mended, the next dump runs.
    rig.tw
        .world_mut()
        .set_flag(spoil_thing, mush_core::Flags::GOING, false)
        .expect("mended");
    let _ = act_go(&mut rig, pilot, "+tv/1 dump go");
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(pool(&mut rig.tw, hopper_v, "metal_ore"), 0);
    assert_eq!(pool(&mut rig.tw, spoil, "metal_ore"), 2);

    // (b1) Doc 20 cut SM.6: two new pools on the spoil land in one bundle.
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hopper_v = hopper(&rig.tw, tv);
    let spoil = VaultId(
        port_map(&rig.tw, "ahuna_port")
            .site
            .and_then(|row| row.spoil_vault)
            .expect("the spoil vault"),
    );
    fill_hopper(&mut rig, tv, 2, 9_870);
    seed_pool(&mut rig, hopper_v, "ice_ore", 1, LotId(9_875));
    let held = |rig: &mut Rig| {
        (
            pool(&mut rig.tw, hopper_v, "metal_ore"),
            pool(&mut rig.tw, hopper_v, "ice_ore"),
            pool(&mut rig.tw, spoil, "metal_ore"),
            pool(&mut rig.tw, spoil, "ice_ore"),
        )
    };
    assert_eq!(held(&mut rig), (2, 1, 0, 0));
    let _ = act_go(&mut rig, pilot, "+tv/1 dump go");
    let due = due_of(&mut rig.tw, tv);
    let (_, queued) = advance_to(&mut rig, due);
    let done = finishes(&queued);
    assert_eq!(done.len(), 1);
    land(&mut rig, done[0]).expect("a two-ore dump onto a fresh spoil lands in one bundle");
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(record.act, None, "the act is released");
    assert_eq!(
        record.last.map(|last| last.result),
        Some(ActResult::Done),
        "the dump is done"
    );
    assert_eq!(
        held(&mut rig),
        (0, 0, 2, 1),
        "the spoil holds both, the hopper is empty"
    );

    // (b2) The spoil's pool census one short of the cap: two new ores refuse mid-bundle.
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hopper_v = hopper(&rig.tw, tv);
    let spoil = VaultId(
        port_map(&rig.tw, "ahuna_port")
            .site
            .and_then(|row| row.spoil_vault)
            .expect("the spoil vault"),
    );
    for n in 0..(mush_inventory::MAX_POOLS_PER_ENDPOINT - 1) {
        seed_pool(
            &mut rig,
            spoil,
            &format!("filler_{n:02}"),
            1,
            LotId(9_900 + u64::try_from(n).expect("small")),
        );
    }
    fill_hopper(&mut rig, tv, 2, 9_870);
    seed_pool(&mut rig, hopper_v, "ice_ore", 1, LotId(9_875));
    let held = |rig: &mut Rig| {
        (
            pool(&mut rig.tw, hopper_v, "metal_ore"),
            pool(&mut rig.tw, hopper_v, "ice_ore"),
            pool(&mut rig.tw, spoil, "metal_ore"),
            pool(&mut rig.tw, spoil, "ice_ore"),
        )
    };
    assert_eq!(held(&mut rig), (2, 1, 0, 0));
    let _ = act_go(&mut rig, pilot, "+tv/1 dump go");
    let due = due_of(&mut rig.tw, tv);
    let (_, queued) = advance_to(&mut rig, due);
    let done = finishes(&queued);
    assert_eq!(done.len(), 1);
    let refused = land(&mut rig, done[0]).expect_err("the spoil side refuses the transfer");
    assert_eq!(held(&mut rig), (2, 1, 0, 0), "nothing written: {refused}");
    assert!(
        record_of(&mut rig.tw, tv).and_then(|r| r.act).is_some(),
        "the act is still in flight"
    );
    // The next fire: the finish records the refusal.
    let (_, queued) = fire(&mut rig);
    assert_eq!(finishes(&queued).len(), 1, "the retry");
    land_all(&mut rig, &queued);
    let record = record_of(&mut rig.tw, tv).expect("the record");
    assert_eq!(record.act, None, "the act is released");
    assert_eq!(
        record.last.map(|last| (last.result, last.at_ic)),
        Some((
            ActResult::Refused {
                code: RefusedCode::SpoilGone
            },
            due
        ))
    );
    assert_eq!(held(&mut rig), (2, 1, 0, 0), "the hopper is exact");
    let (_, queued) = fire(&mut rig);
    land_all(&mut rig, &queued);
    refuses(
        &mut rig,
        pilot,
        "+ship/lift ceres",
        &HOPPER_LOADED
            .replace("{n}", "1")
            .replace("{t}", "3")
            .replace("{name}", "Salt Moth"),
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 22. The dump's preview names what moves (the fixer round's item 9)
// ---------------------------------------------------------------------------

/// **The dump's preview and its start name one load** (D-20-9: `go` commits exactly the
/// preview; the fixer round's item 9): 3 t in the hopper, 1 t of it held by a live hold.
/// The preview names 2 t, the start freezes 2, the finish moves 2 — the held tonne stays.
#[test]
fn sm3b_the_dump_preview_names_what_the_start_freezes() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hopper_v = hopper(&rig.tw, tv);
    let spoil = VaultId(
        port_map(&rig.tw, "ahuna_port")
            .site
            .and_then(|row| row.spoil_vault)
            .expect("the spoil vault"),
    );
    fill_hopper(&mut rig, tv, 3, 9_880);
    // Staff's hand: a live hold of 1 t on the hopper's pool, through the inventory's own path.
    let families = rig
        .tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    let stamp = rig.tw.world().world_ic_now();
    let holder = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: mush_core::PrincipalId::Object { id: GOD },
            owner: mush_core::OwnerId::new("inventory").expect("an owner"),
            namespace: cid("staff_hold"),
        },
        0,
        IdempotencyKey::Request(RequestUid::from_bytes([0x5a; 16])),
    );
    let engine = rig.tw.engine_mut();
    let prepared = {
        let mut txn = engine
            .action_transaction(&families.pools)
            .expect("inventory transaction");
        families
            .reserve(
                &mut txn,
                EndpointRef(hopper_v),
                &cid("metal_ore"),
                &holder,
                1,
                &stamp,
            )
            .expect("the hold is placed");
        txn.prepare().expect("prepares")
    };
    let (ops, install) = prepared.into_parts();
    engine.install_prepared_action(install).expect("installs");
    keep(&mut rig, ops);
    // The preview names the available 2 t.
    let (out, queued) = command(&mut rig, pilot, "+tv/1 dump");
    assert_eq!(
        out.trim(),
        "Rig 1 (OWP-TV-01) would dump 2 t of metal ore, run-of-mine onto the spoil at Ahuna \
         Flank; it cannot be taken back. +tv/1 dump go dumps it."
    );
    assert!(queued.is_empty());
    // The start freezes the same figure.
    let _ = act_go(&mut rig, pilot, "+tv/1 dump go");
    match record_of(&mut rig.tw, tv).and_then(|record| record.act) {
        Some(Act {
            params: ActParams::Dump { goods, .. },
            ..
        }) => assert_eq!(
            goods.into_iter().collect::<Vec<_>>(),
            vec![(cid("metal_ore"), 2)],
            "the preview's load is the frozen load"
        ),
        other => panic!("a dump in flight: {other:?}"),
    }
    let _ = finish(&mut rig, tv, pilot);
    assert_eq!(
        pool(&mut rig.tw, hopper_v, "metal_ore"),
        1,
        "the held tonne stays"
    );
    assert_eq!(pool(&mut rig.tw, spoil, "metal_ore"), 2);
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 23. A roster without the tools table (the fixer round's item 10c)
// ---------------------------------------------------------------------------

/// **Every `+tv` act answers `TV_UNWIRED` on a roster without the tools table** (the
/// brief's 3.2: the tools table is built once and a bad table leaves the acts unwired, never a
/// `ModError`; the fixer round's item 10c). The table is compiled-in content a test cannot
/// break, so the world is given exactly the handles such a roster registers
/// (`port_tick::register`: `surface_tools` `None`, so neither action is registered and
/// `surface_act` / `surface_done` are `None`). Launch, recover and dump, bare and with `go`,
/// each answer `TV_UNWIRED` whole, queue nothing and write no record; `+tv` still reads.
#[test]
fn sm3b_every_act_answers_tv_unwired_without_the_tools_table() {
    let mut rig = stocked();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    fill_hopper(&mut rig, tv, 1, 9_890);
    {
        let handles = rig
            .tw
            .world_mut()
            .module_state_mut::<PortHandles>()
            .expect("port handles");
        handles.surface_tools = None;
        handles.surface_act = None;
        handles.surface_done = None;
    }
    for line in [
        "+tv/1 launch",
        "+tv/1 launch go",
        "+tv/1 recover",
        "+tv/1 recover go",
        "+tv/1 dump",
        "+tv/1 dump go",
    ] {
        let _ = rig.tw.take_module_ops();
        refuses(&mut rig, pilot, line, TV_UNWIRED);
        assert_eq!(record_of(&mut rig.tw, tv), None, "{line}: no record");
        let ops = rig.tw.take_module_ops();
        assert!(
            !ops.iter().any(|op| matches!(&op.op,
                ModuleWrite::JsonSet { key, .. } | ModuleWrite::Del { key }
                    if key.starts_with("tacsim:"))),
            "{line} wrote a tacsim document: {ops:?}"
        );
    }
    let status = run(&mut rig.tw, pilot, "+tv");
    assert!(
        status.contains("Rig 1: OWP-TV-01, in the bay; charge ? %; "),
        "{status}"
    );
    no_refused_writes(&rig.tw);
}

// ---------------------------------------------------------------------------
// 21. A drilling rig is busy to the return and the repossession (doc 20 cut SM.7)
// ---------------------------------------------------------------------------

/// **A drilling rig holds the return and the repossession** (doc 20 cut SM.7; the SM.7 lead's
/// pre-commit round, item 2): a second pilot's Dray-P docked at Ceres, its rig in the bay with a
/// staff-seeded `tacsim.excursion` record holding a mining run and **no excursion** — the one way
/// a run can meet Occator's counter, and a record `rig_flight` once read as idle. It reads busy
/// now: the lease return at the counter, bare and with `go`, answers `RIG_BUSY` with `drilling
/// <pocket>` and the `+tv/1 mine stop` remedy and changes nothing (the lease record, the deposit
/// hold, the rig aboard, the record); the revoke reaches `repossess`, which holds — exactly
/// `NOT_REVOKED` + `RECALL_BAY`, the rig aboard, its lease Active, the record and the charter
/// standing. The same record with the run cleared is the control: the revoke repossesses.
#[test]
#[allow(clippy::too_many_lines)]
fn sm7_a_drilling_rig_holds_the_return_and_the_repossession() {
    use tacsim::surface::address::{block_letter, pocket_address, pocket_number};
    use tacsim::surface::{LockRow, OreFamily, Run};
    let mut rig = stocked();
    let (other, row_o) = prospector(&mut rig, "Other", 2, "Tally Moth");
    lease_one(&mut rig, other);
    let tv_o = rigs_of(&rig.tw, other)[0];
    let hull_o = ObjId(row_o.hull);
    assert!(in_bay(&rig.tw, tv_o, &row_o));
    let now = ic_now(&rig.tw);
    let (px, py) = (36_i64, 56_i64);
    let addr = pocket_address(px, py).expect("an address");
    let number = pocket_number(px, py).expect("a number");
    let mut drilling = RigRecord::fresh(tv_o, &battery());
    drilling.ordinal = 1;
    drilling.run = Some(Run {
        ordinal: 1,
        operator: other,
        hull: hull_o,
        slot: 1,
        site: cid("ahuna_port"),
        body: cid("ceres"),
        stand: [px * 4 + 1, py * 4 + 1],
        pocket: [px, py],
        number,
        letter: block_letter(px, py).expect("a letter"),
        family: OreFamily::Metals,
        deposit: mush_industry::OreDepositId(77),
        good: cid("metal_ore"),
        source: Some(12),
        hopper: 13,
        cycles: 4,
        k: 1,
        secs: 1_800,
        cost_cu: 2_000,
        reserve_t: vec![2, 2, 2, 1],
        class_t: 2,
        pocket_t: 7,
        blind: false,
        stop: false,
        lapses_ic: None,
        lock_rows: vec![LockRow {
            deposit: mush_industry::OreDepositId(77),
            pocket: number,
        }],
        // Far off, so no surface fire lands a cycle while the test reads.
        due_ic: now + 1_000_000,
        reserved_cu: 8_000,
        by: None,
    });
    assert_eq!(drilling.check(), Ok(()), "a well-formed staff record");
    put_record(&mut rig, drilling.clone());

    // The lease return at the counter: busy, with the stop as the remedy.
    let lease = store(&rig.tw)
        .tv_lease_on(tv_o)
        .cloned()
        .expect("the live lease record");
    let hold = hold_status(&rig.tw, lease.hold);
    let counter = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, other, counter);
    let busy = RIG_BUSY
        .replace("{n}", "1")
        .replace("{doing}", &format!("drilling {addr}"));
    let head = busy.split("{when}").next().unwrap_or_default();
    let remedy = tacsim::port::tv::RUN_STOP_REMEDY.replace("{n}", "1");
    assert_eq!(
        remedy, " +tv/1 mine stop ends the run after this cycle.",
        "the remedy names the stop"
    );
    for line in ["+tv/1 lease return", "+tv/1 lease return go"] {
        let (out, queued) = command(&mut rig, other, line);
        let out = out.trim();
        assert!(
            out.starts_with(head) && out.contains(" UTC (IC ") && out.ends_with(&remedy),
            "{line}: {out}"
        );
        assert!(queued.is_empty(), "{line} queued nothing");
        assert_eq!(
            store(&rig.tw).tv_lease_on(tv_o),
            Some(&lease),
            "{line}: the lease record"
        );
        assert_eq!(
            hold_status(&rig.tw, lease.hold),
            hold,
            "{line}: the deposit hold"
        );
        assert!(
            in_bay(&rig.tw, tv_o, &row_o),
            "{line}: the rig stays aboard"
        );
        assert_eq!(
            record_of(&mut rig.tw, tv_o).map(|record| record.run),
            Some(drilling.run.clone()),
            "{line}: the run stands"
        );
    }

    // The repossession holds.
    let held = format!(
        "{}{}",
        tacsim::port::recall::NOT_REVOKED,
        tacsim::port::recall::RECALL_BAY.replace("{name}", "Tally Moth")
    );
    let out = run(&mut rig.tw, GOD, "@coop/revoke Other=test");
    assert_eq!(out.trim(), held);
    assert!(in_bay(&rig.tw, tv_o, &row_o), "the rig stays aboard");
    assert_eq!(
        store(&rig.tw).tv_lease_on(tv_o).map(|lease| lease.status),
        Some(TvLeaseStatus::Active),
        "the lease stands"
    );
    assert_eq!(
        record_of(&mut rig.tw, tv_o).map(|record| record.run),
        Some(drilling.run.clone()),
        "the run stands"
    );
    assert!(
        tacsim::port::live_charter(rig.tw.world(), other).is_some(),
        "the charter stands"
    );
    // The control: the run cleared — the revoke repossesses the rig.
    put_record(&mut rig, RigRecord::fresh(tv_o, &battery()));
    let out = run(&mut rig.tw, GOD, "@coop/revoke Other=test");
    assert!(
        out.contains("Charter of the Tally Moth (") && out.contains(") revoked from Other: test."),
        "{out}"
    );
    assert!(!in_bay(&rig.tw, tv_o, &row_o), "repossessed");
    no_refused_writes(&rig.tw);
}
