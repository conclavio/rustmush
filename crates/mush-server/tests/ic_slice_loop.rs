//! **Doc 18 §6 "The story test", on the production composition** (doc 18 §2, §4.1–§4.4,
//! §6; cut IC10 decision 15, promoted by cut IC12 — its brief's item 1, lead rulings
//! L1–L4).
//!
//! Every test here boots **`mush_server::boot::build_engine`** — the split server's
//! own root: the roster from `boot::game_modules`, the help topics loaded from
//! `docs/help`, the world timeline seeded from `[game]`, the account oracle the world
//! task links, the three durable key rings — over an in-memory store, with
//! `[game].ic_ratio = 24` (the rendered `IC_RATIO` of an owner-approved rate-24
//! deploy: the world boots at the play rate, no staff act) and the sky built at rate 0
//! explicitly (`[tacsim.time.ephemeris].rate = 0`). The production composition is
//! never a test world (IC3 L15), so the Rate line gates `+coop/enrol` as on dev1: the
//! pilots enrol at 24, and the one staff act, `@tacsim/time rate=3600`, comes after
//! the enrolments (lead ruling L1; doc 18 §6's "none after boot" cannot hold on this
//! composition — owner item O6).
//!
//! **Every queued authoritative submission lands as the world task lands it**
//! (`CommandSubmitDriver::drive_one`, `world_task.rs:3329-3433`; the in-process
//! precedent `production_action_migrations.rs:150-265`): the step's legacy work is
//! persisted first (`persist::batch_from_dirty` + `ObjectStore::write` — staging
//! refuses beside dirty state), then `ActionCoordinator::command_ingress` →
//! `preflight` → the actor's own projection and resolve → `prepare_and_stage_resolved`
//! → `take_staged_action` → `reserve_authoritative_intent` → `reconcile_authoritative`
//! → `install_prepared_action_announcing` → `checkpoint_authoritative`. So the typed
//! state lives in the store, a same-epoch resubmission is the coordinator's HIT (a
//! replay, never landed), and **every restart is a plain `build_engine` re-boot over
//! the kept store**, the typed partition hydrated before any module's boot pass as in
//! production, the process epoch bumped (production draws it at random per process).
//!
//! The tests:
//! - [`ic12_part_one_a_leg_by_hand`] — §2 day 1 and day 2's walk, one leg by hand
//!   (IC10's test, promoted), with IC9's `Deferred` launch (T7) and the same-boot HIT;
//! - [`ic12_part_two_the_queue_runs_the_return`] — the sailing plan run unattended
//!   through Stickney and back to Ceres, with §6's three restarts;
//! - [`ic12_a_rate_change_mid_passage_relabels_the_arrival`] — §6's rate change and
//!   IC9 T9 (`leg_s` frozen across a generation);
//! - [`ic12_negative_checks_refuse_in_fiction_and_move_nothing`] — §6's negative checks;
//! - [`ic12_the_sailing_plan_skips_and_holds`] — IC11's untested executor paths.
//!
//! Every expected figure is computed from the posted prices, the goods table and the
//! planner's own output (D-18-39, risk 16), never a literal; money is conserved over
//! the pilot, the Trust and every port org; every hold write leaves both cargo limits
//! met (and the tank's and the locker's); and no escalation of any kind is open.

#[path = "../../tacsim/tests/support/vertex.rs"]
mod vertex;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mush_config::config::Config;
use mush_core::{
    AccountId, Audience, AuthoritativeSubmit, CanonicalId, GOD, LIMBO, ObjId,
    OfferExecutionContext, OfferSubmission, PrincipalId as CorePrincipal,
};
use mush_economy::{ContractId, ContractState, EconomyStore, VaultId};
use mush_inventory::{EndpointRef, InventoryHandles, LotId};
use mush_market::{ListingId, MarketStore};
use mush_org::PrincipalId;
use mush_proto::OutputEvent;
use mush_server::action_coordinator::{
    ActionCoordinator, ActionPreflight, StagedAuthoritativeAction,
};
use mush_server::boot;
use mush_server::persist;
use mush_server::quarantine::QuarantineSpool;
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use mush_testkit::TestWorld;
use tacsim::TacsimStore;
use tacsim::definitions::GoodsTable;
use tacsim::definitions::content::{DRAY, DRAY_JSON, LANE_JSON, LANE_PACK};
use tacsim::pack::{PackEntry, load_pack};
use tacsim::port::apply::room_by_key;
use tacsim::port::capacity::Compartment;
use tacsim::port::{
    HullRow, OrderState, Orders, OrdersState, PlanState, PortDef, PortHandles, PortMap,
    PortMapState, StockIssueId, Ticket, TicketId, room_key,
};

// ---------------------------------------------------------------------------
// The rig: `build_engine` over a kept in-memory store
// ---------------------------------------------------------------------------

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

/// A fresh scratch data dir (the `boot_parity.rs` pattern).
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mush-icslice-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    dir
}

/// **The config dev1's rate-24 deploy renders** (doc 18 §4.7 "Config flips"): the
/// memory store on a scratch data dir, the shipped help topics, `[tacsim]` on with
/// no operator pack (the game's own content), `[game].ic_ratio = 24` and the sky's
/// rate 0 set explicitly.
fn config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg.server.help_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/help");
    cfg.tacsim.enabled = true;
    cfg.tacsim.pack_paths = Vec::new();
    cfg.tacsim.seed_paths = Vec::new();
    cfg.game.ic_ratio = 24;
    cfg.tacsim.time.ephemeris.rate = 0;
    // Doc 20 cut SM.1: a production-shaped config carries the geology key (the shipped
    // template names one), so the readiness report's geology line reads Ready and
    // `stock()` can wait on "Ready: every binding above is in place."
    cfg.tacsim.geology.key_dir = Some(cfg.server.data_dir.join("geology-keys"));
    cfg
}

/// What a landing came to.
enum Landing {
    /// Staged, made durable and installed (the provider's summary is in `Rig::told`).
    Landed,
    /// The coordinator's HIT: the same request in this epoch already owns or has
    /// finished a durable intent; nothing was landed.
    Replay,
    /// Refused before any durable intent (the world task's generic refusal).
    Refused(String),
}

/// One booted engine over a kept store.
struct Rig {
    tw: TestWorld,
    cfg: Config,
    store: Arc<dyn ObjectStore>,
    spool: QuarantineSpool,
    /// The next legacy batch serial (the world task's counter; boot wrote serial 0).
    serial: u64,
    /// The process's idempotency epoch: 1 at the first boot, bumped by a restart.
    epoch: u64,
    boots: u32,
    /// Every pilot made here and its gateway account.
    accounts: BTreeMap<ObjId, AccountId>,
    /// What landings made observers hear (`install_prepared_action_announcing`).
    heard: Vec<(Audience, OutputEvent)>,
    /// The providers' summaries, as the world task tells the actor.
    told: Vec<(ObjId, String)>,
    /// Coordinator HITs.
    replays: u32,
    /// The hull holds whose limits every landing re-checks.
    holds: BTreeSet<u64>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.cfg.server.data_dir);
    }
}

/// **Boot the production root** over a fresh in-memory store.
async fn boot(tag: &str) -> Rig {
    let cfg = config(tag);
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let spool = QuarantineSpool::open(cfg.server.data_dir.join("quarantine")).expect("a spool");
    let booted = boot::build_engine(&cfg, &store, &spool)
        .await
        .expect("the split half boots");
    assert_eq!(
        booted.engine.world.world_timeline().current().rate,
        24,
        "the world timeline is seeded from [game].ic_ratio"
    );
    assert!(
        !booted
            .engine
            .world
            .module_state::<TacsimStore>()
            .expect("the layer composed")
            .test_world(),
        "the production composition is never a test world"
    );
    let tw = TestWorld::from_engine(booted.engine);
    let mut rig = Rig {
        tw,
        cfg,
        store,
        spool,
        serial: 1,
        epoch: 1,
        boots: 1,
        accounts: BTreeMap::new(),
        heard: Vec::new(),
        told: Vec::new(),
        replays: 0,
        holds: BTreeSet::new(),
    };
    flush(&mut rig).await;
    rig
}

/// **Persist the step's legacy work** as the world task does after every step — the
/// dirty objects and the queued module ops in one batch (the world-timeline ops a rate
/// change wrote among them) — and the precondition of staging (`DirtyWorld`,
/// `LegacyModuleOps`).
async fn flush(rig: &mut Rig) {
    let dirty = rig.tw.world_mut().take_dirty();
    let ops = rig.tw.take_module_ops();
    if dirty.is_empty() && ops.is_empty() {
        return;
    }
    let mut batch = persist::batch_from_dirty(rig.tw.world(), dirty, rig.serial, true);
    batch.module_ops = ops;
    let receipt = rig
        .store
        .write(batch)
        .await
        .expect("the legacy batch commits");
    assert_eq!(receipt.serial, rig.serial);
    rig.serial = rig.serial.saturating_add(1);
}

/// **A server-half restart over the kept store** (doc 18 §6 "Restarts"): the legacy
/// work flushed; the engine dropped with whatever submissions it still queued (a kill
/// loses them); `build_engine` over the same store, spool and key dirs — the typed
/// partition hydrated before any module boots; a new process epoch; the clock put back
/// where the test had reached. Asserts every pilot's account came back through the
/// stored accounts (the oracle), and that `probe` — a typed document the caller knows
/// — reads back through the store. A restart has no sessions.
async fn restart(rig: &mut Rig, probe: impl FnOnce(&mut TestWorld) -> bool) {
    flush(rig).await;
    let now = rig.tw.world().now();
    let old = std::mem::replace(&mut rig.tw, TestWorld::new());
    drop(old);
    let booted = boot::build_engine(&rig.cfg, &rig.store, &rig.spool)
        .await
        .expect("the restart boots over the kept store");
    rig.tw = TestWorld::from_engine(booted.engine);
    rig.tw.set_now(now);
    rig.epoch = rig.epoch.saturating_add(1);
    rig.boots = rig.boots.saturating_add(1);
    for (pilot, account) in &rig.accounts {
        assert_eq!(
            rig.tw.engine().account_of(*pilot),
            Some(*account),
            "the oracle re-hydrated #{} from the stored accounts",
            pilot.0
        );
    }
    assert!(
        probe(&mut rig.tw),
        "the typed state came back through the store"
    );
    flush(rig).await;
}

// ---------------------------------------------------------------------------
// Landing as the world task lands
// ---------------------------------------------------------------------------

/// `command_offer_submission` (`world_task.rs:3187`, private): the handler's selector
/// turned into the actor's own opaque offer — the actor's projection, the offer for
/// the action, the subject checked equal, the submission's own targets.
fn offer_submission(
    tw: &TestWorld,
    submit: &AuthoritativeSubmit,
) -> Result<OfferSubmission, String> {
    let engine = tw.engine();
    let query = engine
        .legal_action_query(
            CorePrincipal::Object { id: submit.actor },
            submit.actor,
            submit.subject.clone(),
        )
        .map_err(|error| format!("{error:?}"))?;
    let offers = engine
        .project_legal_actions(&query)
        .map_err(|error| error.to_string())?;
    let projected = offers
        .projected_for_action(&submit.action)
        .ok_or_else(|| "that action is not currently available".to_owned())?;
    if projected.subject() != &submit.subject {
        return Err("that action's subject is not the one the command named".to_owned());
    }
    Ok(OfferSubmission {
        offer: projected.offer(),
        subject: projected.subject().clone(),
        targets: submit.targets.clone(),
        parameters: submit.parameters.clone(),
    })
}

/// `resolve_and_stage_offer` (`world_task.rs:3142`, private): the permit's actor's
/// query and projection, the resolve under the permit's request and idempotency, then
/// `ActionCoordinator::prepare_and_stage_resolved`.
fn resolve_and_stage(
    tw: &mut TestWorld,
    permit: Box<mush_server::action_coordinator::ActionPermit>,
    submission: OfferSubmission,
) -> Result<StagedAuthoritativeAction, String> {
    let CorePrincipal::Object { id: actor } = permit.ingress().principal().clone() else {
        return Err("coordinator principal is not an object actor".to_owned());
    };
    let resolved = {
        let engine = tw.engine();
        let query = engine
            .legal_action_query(
                CorePrincipal::Object { id: actor },
                actor,
                submission.subject.clone(),
            )
            .map_err(|error| format!("{error:?}"))?;
        let offers = engine
            .project_legal_actions(&query)
            .map_err(|error| error.to_string())?;
        engine
            .resolve_legal_action(
                &offers,
                OfferExecutionContext {
                    request: permit.ingress().request(),
                    principal: CorePrincipal::Object { id: actor },
                    actor,
                    idempotency: permit.ingress().idempotency().clone(),
                    viewer_scope: query.viewer_scope(),
                    now: None,
                },
                submission,
            )
            .map_err(|error| error.to_string())?
    };
    ActionCoordinator::prepare_and_stage_resolved(permit, tw.engine_mut(), resolved)
        .map_err(|error| error.to_string())
}

/// **Land one queued submission exactly as the world task does** (lead ruling L2;
/// `CommandSubmitDriver::drive_one`): legacy work flushed first; `command_ingress` in
/// this process's epoch; `preflight` against the store — `Retired` raises the epoch
/// and retries (≤ 3, as the driver), `Pending`/`Finalized` is the HIT (counted, never
/// landed), `Quarantined`/`Conflict` is an integrity fault (a test failure); on
/// `Vacant`, the projection, the resolve and the stage, then the K3 sequence and the
/// announcing install. Every hold write is then checked against both limits.
async fn land(rig: &mut Rig, submit: &AuthoritativeSubmit) -> Landing {
    flush(rig).await;
    let store = Arc::clone(&rig.store);
    let auth = store
        .authoritative_store()
        .expect("the store is K3-durable");
    for _ in 0..3 {
        let envelope = match ActionCoordinator::command_ingress(submit, rig.epoch) {
            Ok(envelope) => envelope,
            Err(detail) => return Landing::Refused(detail),
        };
        let preflight = ActionCoordinator::preflight(auth, envelope)
            .await
            .expect("the preflight reads the store");
        match preflight {
            ActionPreflight::Retired {
                minimum_accepted_epoch,
            } => {
                rig.epoch = rig.epoch.max(minimum_accepted_epoch);
            }
            ActionPreflight::Pending { .. } | ActionPreflight::Finalized(_) => {
                rig.replays = rig.replays.saturating_add(1);
                return Landing::Replay;
            }
            ActionPreflight::Quarantined(receipt) => {
                panic!("an integrity fault: quarantined at {}", receipt.token())
            }
            ActionPreflight::Conflict => panic!("an integrity fault: an idempotency conflict"),
            ActionPreflight::Vacant(permit) => {
                let submission = match offer_submission(&rig.tw, submit) {
                    Ok(submission) => submission,
                    Err(detail) => return Landing::Refused(detail),
                };
                let staged = match resolve_and_stage(&mut rig.tw, permit, submission) {
                    Ok(staged) => staged,
                    Err(detail) => return Landing::Refused(detail),
                };
                let (bundle, installation) = rig
                    .tw
                    .engine_mut()
                    .take_staged_action()
                    .expect("the staged action's guards are current")
                    .expect("one staged action")
                    .into_parts();
                let bundle = *bundle;
                auth.reserve_authoritative_intent(bundle.clone())
                    .await
                    .expect("the intent reserves");
                let receipt = auth
                    .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
                    .await
                    .expect("the bundle reconciles");
                assert!(receipt.matches_bundle(&bundle));
                let heard = rig.tw.install_prepared_action_announcing(installation);
                rig.heard.extend(heard);
                auth.checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
                    .await
                    .expect("the bundle checkpoints");
                if let Some(summary) = &staged.summary {
                    rig.told.push((submit.actor, summary.clone()));
                }
                assert_holds_within_limits(rig);
                return Landing::Landed;
            }
        }
    }
    panic!("the idempotency retirement watermark changed repeatedly")
}

/// Every queued submission, in the order the engine queued them. GOD never submits
/// from the port tick (`mush-campaign/src/tick.rs:28-37`).
fn drain(tw: &mut TestWorld) -> Vec<AuthoritativeSubmit> {
    let mut kept = Vec::new();
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        if submit.action.key().owner.as_str() == tacsim::NS {
            assert_ne!(submit.actor, GOD, "GOD never submits from the port tick");
        }
        kept.push(*submit);
    }
    kept
}

/// Land each; a refusal fails the test.
async fn land_all(rig: &mut Rig, queued: &[AuthoritativeSubmit]) {
    for submit in queued {
        if let Landing::Refused(error) = land(rig, submit).await {
            panic!(
                "{} by #{} ({}) did not land: {error}",
                submit.action.key().id,
                submit.actor.0,
                submit.key.as_str()
            );
        }
    }
}

/// Land each; refusals and replays pass (a re-staged leg the clock raced).
async fn land_quietly(rig: &mut Rig, queued: &[AuthoritativeSubmit]) {
    for submit in queued {
        let _ = land(rig, submit).await;
    }
}

/// Everything due `secs` on; legacy work flushed.
async fn sweep(rig: &mut Rig, secs: i64) -> Vec<(Audience, OutputEvent)> {
    let t = rig.tw.world().now().saturating_add(secs);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1000));
    flush(rig).await;
    events
}

/// One port-tick fire (6 s on), its submissions drained, not landed.
async fn fire(rig: &mut Rig) -> (Vec<(Audience, OutputEvent)>, Vec<AuthoritativeSubmit>) {
    let events = sweep(rig, 6).await;
    let queued = drain(&mut rig.tw);
    (events, queued)
}

/// One fire, its submissions landed quietly.
async fn fire_quietly(rig: &mut Rig) -> Vec<(Audience, OutputEvent)> {
    let (events, queued) = fire(rig).await;
    land_quietly(rig, &queued).await;
    events
}

/// One typed line as `who`; what they saw. Legacy work is flushed after it.
async fn run(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let events = rig.tw.run(who, line);
    flush(rig).await;
    said(&rig.tw, &events, who)
}

fn said(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    tw.seen_by(events, who).join("\n")
}

/// One typed line as the pilot, asserting where it leaves them.
async fn walk(rig: &mut Rig, pilot: ObjId, word: &str, to: ObjId) {
    let out = run(rig, pilot, word).await;
    assert_eq!(location(&rig.tw, pilot), Some(to), "`{word}`: {out}");
}

/// The port tick, until the whole readiness report reads Ready (the clerks stock and
/// list both ports; background work, not a staff act). **Doc 20 cut SM.6 (D2):** with the
/// geology key configured, Ready also waits for the six sites' clerks to open every deposit
/// and move the Commons's worked share, one act a clerk a fire — 97 fires for the worked
/// Commons on the shipped tables, the warm included — so the bound is 300 fires: twice the
/// worst case of 64 opens and 64 transfers, plus the warms and slack for the shared
/// `MAX_AGENTS_WORKED_PER_FIRE`.
async fn stock(rig: &mut Rig) {
    for _ in 0..300 {
        let out = run(rig, GOD, "@tacsim/port status").await;
        if out.lines().last() == Some("Ready: every binding above is in place.") {
            return;
        }
        let (_, queued) = fire(rig).await;
        land_all(rig, &queued).await;
    }
    let out = run(rig, GOD, "@tacsim/port status").await;
    panic!("the report did not read Ready within 300 fires:\n{out}");
}

/// **A character on a gateway account of their own** (the world task's create path:
/// the account written, the character linked to the oracle).
async fn pilot(rig: &mut Rig, name: &str, account_name: &str) -> ObjId {
    let pilot = rig.tw.spawn_player(name);
    let created = 1_760_000_000;
    let account = AccountId::mint(account_name, created).expect("an account id");
    rig.store
        .create_account(AccountRec {
            id: Some(account),
            name: account_name.to_owned(),
            argon2_hash: "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA"
                .to_owned(),
            created,
            characters: vec![CharacterRef {
                id: pilot,
                name: name.to_owned(),
            }],
            ssh_keys: Vec::new(),
            discord: None,
        })
        .await
        .expect("the account is written");
    assert!(
        rig.tw.engine().link_account_character(pilot, account),
        "the oracle takes the new character"
    );
    rig.accounts.insert(pilot, account);
    flush(rig).await;
    pilot
}

/// A second character on an existing gateway account.
async fn alt(rig: &mut Rig, name: &str, account_name: &str, of: ObjId) -> ObjId {
    let account = rig.accounts[&of];
    let who = rig.tw.spawn_player(name);
    rig.store
        .link_character(account_name, who, name)
        .await
        .expect("the character links");
    assert!(rig.tw.engine().link_account_character(who, account));
    rig.accounts.insert(who, account);
    flush(rig).await;
    who
}

// ---------------------------------------------------------------------------
// Reads (IC10's)
// ---------------------------------------------------------------------------

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
    match store(tw).port_map(&cid(LANE_PACK), &cid(entry)) {
        PortMapState::Present(map) => map.clone(),
        other => panic!("{entry}'s map: {other:?}"),
    }
}

fn port_def(entry: &str) -> PortDef {
    let pack = load_pack(LANE_JSON).expect("the lane verifies");
    match pack.entry(&cid(entry)) {
        Some(PackEntry::Port(def)) => def.clone(),
        _ => panic!("{entry} is a port entry"),
    }
}

/// A station room, re-derived from the port entry's area and the room's local id.
fn station_room(tw: &TestWorld, entry: &str, local: &str) -> ObjId {
    let def = port_def(entry);
    let key = room_key(&def.area, &format!("{}.{local}", def.area)).expect("a room key");
    room_by_key(tw.world(), &key).expect("the room is on this world")
}

fn location(tw: &TestWorld, obj: ObjId) -> Option<ObjId> {
    tw.world().get(obj).and_then(|o| o.location)
}

fn families(tw: &TestWorld) -> mush_inventory::InventoryFamilies {
    tw.world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone()
}

fn handles(tw: &TestWorld) -> PortHandles {
    tw.world()
        .module_state::<PortHandles>()
        .cloned()
        .expect("port handles")
}

/// A vault's pool of `good`, through an owner-stamped transaction.
fn pool(tw: &mut TestWorld, vault: VaultId, good: &str) -> u64 {
    let families = families(tw);
    let key = mush_inventory::pool_key(EndpointRef(vault), &cid(good)).expect("a pool key");
    let mut txn = tw
        .engine_mut()
        .action_transaction(&families.pools)
        .expect("a transaction");
    txn.get(&families.pools, &key)
        .expect("reads")
        .map_or(0, |scalar| {
            u64::try_from(scalar.current()).expect("non-negative")
        })
}

/// One hull's ticket at `seq`, read through the typed family.
fn ticket(tw: &mut TestWorld, hull: ObjId, seq: u32) -> Option<Ticket> {
    let handles = handles(tw);
    let mut txn = tw
        .engine_mut()
        .action_transaction(&handles.families.tickets)
        .expect("a transaction");
    txn.get(&handles.families.tickets, &TicketId { hull, seq })
        .expect("reads")
        .cloned()
}

/// Every ticket the hull holds, from seq 1 up to the first gap past `max`.
fn tickets(tw: &mut TestWorld, hull: ObjId) -> Vec<Ticket> {
    (1..=16).filter_map(|seq| ticket(tw, hull, seq)).collect()
}

fn ic_now(tw: &TestWorld) -> i64 {
    tw.world().world_ic_now().at
}

/// Run the clock to IC instant `target` (one fire at the real second it falls on),
/// landing what the fire queued.
async fn to_ic(rig: &mut Rig, target: i64) -> Vec<(Audience, OutputEvent)> {
    let rate = i64::from(rig.tw.world().world_timeline().current().rate.max(1));
    let ahead = target.saturating_sub(ic_now(&rig.tw)).max(0);
    let real = (ahead + rate - 1) / rate;
    let events = sweep(rig, real).await;
    let queued = drain(&mut rig.tw);
    land_quietly(rig, &queued).await;
    events
}

/// [`to_ic`], the fire's submissions drained and returned, not landed.
async fn to_ic_queued(rig: &mut Rig, target: i64) -> Vec<AuthoritativeSubmit> {
    let rate = i64::from(rig.tw.world().world_timeline().current().rate.max(1));
    let ahead = target.saturating_sub(ic_now(&rig.tw)).max(0);
    let real = (ahead + rate - 1) / rate;
    let _ = sweep(rig, real).await;
    drain(&mut rig.tw)
}

fn credit_account(tw: &TestWorld, owner: &PrincipalId) -> mush_economy::AccountId {
    let credit = mush_economy::InstrumentId::new(tacsim::CREDIT).expect("credit");
    economy(tw)
        .snapshot()
        .accounts_of(owner)
        .find(|a| a.instrument == credit)
        .map(|a| a.id)
        .expect("a credit account")
}

fn posted(tw: &TestWorld, account: mush_economy::AccountId) -> i64 {
    economy(tw)
        .snapshot()
        .account(account)
        .expect("the account")
        .posted_balance
        .minor()
}

/// An account's available funds (posted − held). A contract's escrow is a hold
/// (`mush-economy`'s `held_amount`): it moves this and never `posted_balance`, so a
/// check that an escrow came back, or was never taken, reads this.
fn available(tw: &TestWorld, account: mush_economy::AccountId) -> i64 {
    economy(tw)
        .snapshot()
        .account(account)
        .and_then(mush_economy::Account::available)
        .map(mush_economy::Amount::minor)
        .expect("the account's available funds")
}

/// The money render (`5600.00cr`).
fn cr(minor: i64) -> String {
    format!("{}.{:02}cr", minor / 100, minor % 100)
}

fn listing_of(tw: &TestWorld, entry: &str, org: &str, good: &str) -> ListingId {
    let vault = VaultId(port_map(tw, entry).vendors[&cid(org)].vault);
    market(tw)
        .lot_listing_of(EndpointRef(vault), &cid(good))
        .unwrap_or_else(|| panic!("{entry}/{org} lists {good}"))
}

fn newest_quote(tw: &TestWorld, buyer: ObjId) -> mush_market::QuoteId {
    market(tw)
        .quotes_of_buyer(&PrincipalId::Object(buyer))
        .map(|q| q.id)
        .max()
        .expect("a quote")
}

fn listed_qty(tw: &TestWorld, id: ListingId) -> Option<u64> {
    market(tw).listing(id).map(|l| l.quantity)
}

fn price_of(tw: &TestWorld, id: ListingId) -> i64 {
    market(tw)
        .listing(id)
        .map(|l| l.price.minor())
        .expect("the listing")
}

/// `+market/quote <listing>=<qty>`, `+market/accept <quote>`, the Draw landed; the
/// price paid (minor).
async fn buy(rig: &mut Rig, pilot: ObjId, listing: ListingId, qty: u64) -> i64 {
    let price = price_of(&rig.tw, listing);
    let quoted = run(rig, pilot, &format!("+market/quote {listing}={qty}")).await;
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = newest_quote(&rig.tw, pilot);
    let accepted = run(rig, pilot, &format!("+market/accept {quote}")).await;
    assert!(accepted.contains("Agreed."), "{accepted}");
    let queued = drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "{accepted}");
    land_all(rig, &queued).await;
    price * i64::try_from(qty).expect("qty")
}

/// `Burn B t (tank T t; …)` → `(B, T)`; `Stores C crew-days (locker L; …)` → `(C, L)`.
fn plan_numbers(plan: &str, head: &str, open: &str) -> Option<(u64, u64)> {
    let line = plan.lines().find(|l| l.starts_with(head))?;
    let first = line
        .strip_prefix(head)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let second = line
        .split(open)
        .nth(1)?
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    Some((first, second))
}

/// The settlement transaction's `(account, delta)` lines.
fn settlement(tw: &TestWorld, contract: ContractId) -> BTreeMap<mush_economy::AccountId, i64> {
    let Some(ContractState::Settled { transaction }) =
        economy(tw).snapshot().contract(contract).map(|c| c.state)
    else {
        panic!("contract #{contract} did not settle");
    };
    economy(tw)
        .transaction(transaction)
        .expect("the settlement")
        .lines
        .iter()
        .map(|l| (l.account, l.delta.minor()))
        .collect()
}

/// The sum of posted credit over `pilots`, the Trust and every port org.
fn closed_sum(tw: &TestWorld, pilots: &[ObjId]) -> i64 {
    let mut sum: i64 = pilots
        .iter()
        .map(|p| posted(tw, credit_account(tw, &PrincipalId::Object(*p))))
        .sum();
    for entry in ["ceres_port", "stickney_port"] {
        for row in port_map(tw, entry).orgs.values() {
            if let Some(account) = row.account {
                sum += posted(tw, mush_economy::AccountId(account));
            }
        }
    }
    sum
}

/// `{n}` with thousands grouped (`1,234`), the board's and the sale line's render.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The Co-op's `(basis, share)` of one sale, computed here from the posted prices:
/// the margin over the reference ask, and its permille floored (D-18-24).
fn margin_and_share(price: i64, reference: i64, qty: u64, permille: u32) -> (i64, i64) {
    let basis = (price - reference).max(0) * i64::try_from(qty).expect("qty");
    (basis, basis * i64::from(permille) / 1_000)
}

fn goods() -> GoodsTable {
    tacsim::definitions::goods::goods_table().expect("the goods table")
}

/// The Dray's compartments from its shipped pack (cargo, tank, locker).
fn dray_compartments() -> Vec<Compartment> {
    let dray = load_pack(DRAY_JSON).expect("the Dray pack");
    let Some(PackEntry::Unit(spec)) = dray.entry(&cid(DRAY)) else {
        panic!("the Dray unit");
    };
    spec.logistics
        .as_ref()
        .expect("the Dray's logistics")
        .stowage
        .iter()
        .map(|(id, def)| Compartment::from_stowage(id, def))
        .collect()
}

fn cargo_compartment() -> Compartment {
    dray_compartments()
        .into_iter()
        .find(|c| c.id.as_str() == "cargo")
        .expect("the cargo hold")
}

/// The both-limits load (derived from the goods table, never named — D-18-39).
fn vertex_load() -> (u64, u64) {
    vertex::both_limits_vertex(
        &goods(),
        &cargo_compartment(),
        &cid("metals"),
        &cid("volatiles"),
    )
    .expect("an integral vertex")
}

/// **Every tracked hold leaves every compartment within its limits** (doc 18 §6:
/// "every hold write leaves both cargo limits met"): for each hold, each good's pool
/// read, routed to the compartment that accepts it, and each compartment's load (mass
/// and packed volume; the tank and the locker by their own measures) at or under its
/// limit. Called after every landed write.
fn assert_holds_within_limits(rig: &mut Rig) {
    let goods = goods();
    let compartments = dray_compartments();
    let holds: Vec<u64> = rig.holds.iter().copied().collect();
    for hold in holds {
        let mut contents: BTreeMap<CanonicalId, BTreeMap<CanonicalId, u64>> = BTreeMap::new();
        for good in &goods.goods {
            let qty = pool(&mut rig.tw, VaultId(hold), good.id.as_str());
            if qty == 0 {
                continue;
            }
            let comp = tacsim::port::capacity::compartment_for(&goods, &compartments, &good.id)
                .unwrap_or_else(|| panic!("{} stows somewhere aboard", good.id));
            contents
                .entry(comp.id.clone())
                .or_default()
                .insert(good.id.clone(), qty);
        }
        for comp in &compartments {
            let held = contents.get(&comp.id).cloned().unwrap_or_default();
            let load = tacsim::port::capacity::load(&goods, comp, &held).expect("the load");
            for (measure, limit) in &comp.limits {
                let used = load.get(measure).copied().unwrap_or(0);
                assert!(
                    used <= *limit,
                    "hold #{hold} {} is over its {measure}: {used} of {limit} ({held:?})",
                    comp.id
                );
            }
        }
    }
}

/// No `SettlementRefused`, no stale-contract page, no open escalation of any kind.
fn assert_no_pages(tw: &TestWorld) {
    let pages: Vec<_> = economy(tw).escalations().collect();
    assert!(pages.is_empty(), "no escalation of any kind: {pages:?}");
    assert!(tw.world().refused_namespace_writes().is_empty());
}

/// The live charter's hull row for `pilot`.
fn hull_row(tw: &TestWorld, pilot: ObjId) -> HullRow {
    let charter = tacsim::port::live_charter(tw.world(), pilot).expect("a charter");
    port_map(tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|row| row.hull == charter.hull.0)
        .expect("the hull's row")
}

fn hold_of(tw: &TestWorld, row: &HullRow) -> VaultId {
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

/// The receipts every Ceres and Stickney storefront's seeded stock rows hold.
fn receipts(tw: &mut TestWorld) -> Vec<(StockIssueId, tacsim::port::StockReceipt)> {
    let handles = handles(tw);
    let mut out = Vec::new();
    for entry in ["ceres_port", "stickney_port"] {
        for row in port_map(tw, entry).vendors.values() {
            for stock in &row.stock {
                let id = StockIssueId::parse(&stock.id).expect("an issue id");
                let mut txn = tw
                    .engine_mut()
                    .action_transaction(&handles.families.port_stock)
                    .expect("a transaction");
                if let Some(receipt) = txn
                    .get(&handles.families.port_stock, &id)
                    .expect("reads")
                    .cloned()
                {
                    out.push((id, receipt));
                }
            }
        }
    }
    out
}

/// Whether every good in `hold` traces to a seeded Origin receipt.
fn traces_to_origin(tw: &mut TestWorld, hold: VaultId, goods: &[&str]) {
    let receipts = receipts(tw);
    let families = families(tw);
    for good in goods {
        let mut txn = tw
            .engine_mut()
            .action_transaction(&families.pools)
            .expect("a transaction");
        let (_, root) = families
            .tranche_for(&mut txn, EndpointRef(hold), &cid(good))
            .expect("reads")
            .unwrap_or_else(|| panic!("the hold's {good} root"));
        let traced = root.inherited().iter().any(|parent| {
            let Ok(Some(source)) = txn.get(&families.tranches, parent) else {
                return false;
            };
            receipts
                .iter()
                .any(|(_, r)| source.absorbed().contains(&LotId(r.lot)))
        });
        assert!(
            traced,
            "the hold's {good} traces to a seeded Origin receipt"
        );
    }
}

/// The hull's sailing plan.
fn orders(tw: &TestWorld, hull: ObjId) -> Orders {
    match store(tw).orders_state(hull) {
        OrdersState::Present(orders) => orders.clone(),
        other => panic!("the sailing plan: {other:?}"),
    }
}

fn order_state(tw: &TestWorld, hull: ObjId, id: u32) -> Option<OrderState> {
    match store(tw).orders_state(hull) {
        OrdersState::Present(orders) => orders.order(id).map(|o| o.state),
        OrdersState::Absent | OrdersState::Unreadable => None,
    }
}

fn log_texts(tw: &TestWorld, hull: ObjId) -> Vec<String> {
    orders(tw, hull)
        .log
        .iter()
        .map(|l| l.text.clone())
        .collect()
}

fn burn_keys(queued: &[AuthoritativeSubmit]) -> Vec<String> {
    queued
        .iter()
        .filter(|s| s.action.key().id.as_str() == tacsim::port::BURN_ACTION)
        .map(|s| s.key.as_str().to_owned())
        .collect()
}

/// Drop `who`'s every connection (`orders.rs`'s idiom).
fn disconnect(tw: &mut TestWorld, who: ObjId) {
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    let conns: Vec<u64> = (1..=64)
        .filter(|conn| tw.engine().actor_of(*conn) == Some(who))
        .collect();
    assert!(!conns.is_empty(), "#{} holds a connection", who.0);
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

/// The journey `hull` is on (or last flew: an arrived journey stays in the book).
fn journey(tw: &TestWorld, hull: ObjId) -> Option<mush_sim::CarrierJourney> {
    tw.world()
        .module_state::<mush_sim::JourneyBook>()
        .and_then(|book| book.get(hull).cloned())
}

fn under_way(tw: &TestWorld, hull: ObjId) -> bool {
    matches!(
        tacsim::port::ship::berth_of(tw.world(), hull),
        tacsim::port::ship::Berth::UnderWay
    )
}

/// What a single-good compartment can hold of `good`: the least of its limits over
/// the good's per-unit measures.
fn single_cap(goods: &GoodsTable, compartment: &str, good: &str) -> u64 {
    let comp = dray_compartments()
        .into_iter()
        .find(|c| c.id.as_str() == compartment)
        .expect("the compartment");
    let row = goods.good(&cid(good)).expect("the good");
    comp.limits
        .iter()
        .filter_map(|(measure, limit)| row.measures.get(measure).map(|per| limit / per))
        .min()
        .expect("a limited measure")
}

/// The sky line of `@tacsim/port status`, asserted in doc 18 §6's form: constant at
/// rate 0, the lane's epoch, a 12-hex revision, `[ready]`, and no duration anywhere
/// on it (D-18-50).
fn assert_sky_line(report: &str) {
    let sky = report
        .lines()
        .find(|l| l.starts_with("sky: "))
        .unwrap_or_else(|| panic!("a sky line:\n{report}"));
    let rev = sky
        .strip_prefix("sky: constant (rate 0), kepler, epoch 2351-05-13, rev ")
        .and_then(|rest| rest.strip_suffix(" [ready]"))
        .unwrap_or_else(|| panic!("the sky line's form: {sky}"));
    assert_eq!(rev.len(), 12, "{sky}");
    assert!(
        rev.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "{sky}"
    );
    for word in [" h ", "hours", "day", "IC days", "real"] {
        assert!(!sky.contains(word), "no duration on the sky line: {sky}");
    }
}

/// The help topics a fresh tester reads, each answering its topic in the booted
/// registry (doc 18 §5 IC12 row; Pass 1 item 3).
async fn assert_help(rig: &mut Rig, who: ObjId) {
    for (topic, title) in [
        ("ports", "# Ports"),
        ("ships", "# Ships"),
        ("coop", "# The Co-op's First Captain programme"),
        ("orders", "# The sailing plan"),
        ("trade", "# Trading"),
    ] {
        let out = run(rig, who, &format!("help {topic}")).await;
        let heading = title.trim_start_matches("# ");
        assert!(out.contains(heading), "`help {topic}`:\n{out}");
    }
}

// ---------------------------------------------------------------------------
// Part 1: one leg by hand
// ---------------------------------------------------------------------------

/// **Doc 18 §6 part 1: one leg of the trade run, played by hand, line by line** (doc
/// 18 §2 day 1 and day 2's walk; IC10's test, promoted to the production root).
///
/// A fresh character on a gateway account of their own walks in from Limbo through
/// the IC door, enrols at the play rate, buys the both-limits cargo at the Ceres
/// Exchange (derived from the goods table, never named — D-18-39), buys reaction mass
/// and provisions at exactly the planner's burn and crew-days, and — after the one
/// staff act, `@tacsim/time rate=3600` — boards, plans, undocks (IC9 T7: the tick
/// defers a launch whose burn has not landed, and the same burn resubmitted in this
/// boot is the coordinator's HIT), flies the lane hearing the Kirkwood boundary and
/// the docking in the Crew Compartment, walks to Kasei's Receiving Bay by the Stickney
/// pack's own words, sells both goods with doc 18's exact lines, and walks back to the
/// hull by §2 day 2's words. Every figure is computed; money is conserved; every tonne
/// traces to a seeded receipt; every hold write leaves the limits met.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn ic12_part_one_a_leg_by_hand() {
    let mut rig = boot("part1").await;
    let _ = sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let report = run(&mut rig, GOD, "@tacsim/port status").await;
    assert_sky_line(&report);
    assert!(
        report.contains("world rate: 24 IC seconds per real second. [ready]"),
        "{report}"
    );

    // The character, on a gateway account of their own, in Limbo.
    let pilot = pilot(&mut rig, "Vex", "vexacct").await;
    assert_eq!(location(&rig.tw, pilot), Some(LIMBO));
    let c01 = station_room(&rig.tw, "ceres_port", "room.c01_berth_gallery");
    let c02 = station_room(&rig.tw, "ceres_port", "room.c02_pressure");
    let c03 = station_room(&rig.tw, "ceres_port", "room.c03_arrivals");
    let c04 = station_room(&rig.tw, "ceres_port", "room.c04_concourse");
    let c05 = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    let c07 = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    let c08 = station_room(&rig.tw, "ceres_port", "room.c08_chandlery");

    // Steps 2–5: the IC door, the desk, the terms, the enrolment at the play rate.
    walk(&mut rig, pilot, "c", c03).await;
    walk(&mut rig, pilot, "registry", c05).await;
    let drays = port_map(&rig.tw, "ceres_port").hulls.len();
    let terms = run(&mut rig, pilot, "+coop").await;
    assert!(
        terms.contains(&format!("Drays free: {drays} of {drays}.")),
        "{terms}"
    );
    let enrolled = run(&mut rig, pilot, "+coop/enrol Salt Moth").await;
    assert!(enrolled.contains("now the Salt Moth"), "{enrolled}");
    let ship = run(&mut rig, pilot, "+ship").await;
    assert!(ship.contains("Salt Moth (Dray): docked "), "{ship}");
    assert!(ship.contains("Programme: 0 sales;"), "{ship}");
    let row = hull_row(&rig.tw, pilot);
    let hull = ObjId(row.hull);
    let hold = hold_of(&rig.tw, &row);
    rig.holds.insert(hold.0);
    let me = credit_account(&rig.tw, &PrincipalId::Object(pilot));
    let stake = posted(&rig.tw, me);
    assert!(
        terms.contains(&format!("Terms: a {} stake; ", cr(stake))),
        "{terms}"
    );
    let books_before = closed_sum(&rig.tw, &[pilot]);

    // Step 6: the Exchange, the both-limits cargo.
    walk(&mut rig, pilot, "arrivals", c03).await;
    walk(&mut rig, pilot, "concourse", c04).await;
    walk(&mut rig, pilot, "exchange", c07).await;
    let listed = run(&mut rig, pilot, "+market/list").await;
    let metals_ask = listing_of(&rig.tw, "ceres_port", "freeport_exchange", "metals");
    let ice_ask = listing_of(&rig.tw, "ceres_port", "occator", "volatiles");
    assert!(listed.contains(&format!("#{metals_ask} ")), "{listed}");
    assert!(listed.contains(&format!("#{ice_ask} ")), "{listed}");
    let goods = goods();
    let (metals, ice) = vertex_load();
    let (metals_seeded, ice_seeded) = (
        listed_qty(&rig.tw, metals_ask).expect("listed"),
        listed_qty(&rig.tw, ice_ask).expect("listed"),
    );
    let mut costs = buy(&mut rig, pilot, metals_ask, metals).await;
    costs += buy(&mut rig, pilot, ice_ask, ice).await;

    // Step 7: the chandlery, reaction mass and provisions at the planner's figures.
    walk(&mut rig, pilot, "chandlery", c08).await;
    let fuel_ask = listing_of(&rig.tw, "ceres_port", "occator", "propellant");
    let food_ask = listing_of(&rig.tw, "ceres_port", "coop", "provisions");
    let (fuel_seeded, food_seeded) = (
        listed_qty(&rig.tw, fuel_ask).expect("listed"),
        listed_qty(&rig.tw, food_ask).expect("listed"),
    );
    let (mut fuel, mut food) = (0_u64, 0_u64);
    for _ in 0..6 {
        let plan = run(&mut rig, pilot, "+ship/plan stickney").await;
        let (burn, tank) = plan_numbers(&plan, "Burn ", "(tank ").expect(&plan);
        let (days, locker) = plan_numbers(&plan, "Stores ", "(locker ").expect(&plan);
        if tank >= burn && locker >= days {
            break;
        }
        if tank < burn {
            costs += buy(&mut rig, pilot, fuel_ask, burn - tank).await;
            fuel += burn - tank;
        }
        if locker < days {
            costs += buy(&mut rig, pilot, food_ask, days - locker).await;
            food += days - locker;
        }
    }
    let plan = run(&mut rig, pilot, "+ship/plan stickney").await;
    let (burn, tank) = plan_numbers(&plan, "Burn ", "(tank ").expect(&plan);
    let (days, locker) = plan_numbers(&plan, "Stores ", "(locker ").expect(&plan);
    assert_eq!((tank, locker), (fuel, food), "{plan}");
    assert!(tank >= burn && locker >= days, "{plan}");
    // `+ship/hold`: the hold at both limits, the tank and the locker at what was bought.
    let held = run(&mut rig, pilot, "+ship/hold").await;
    let cargo = cargo_compartment();
    let (mass, volume) = (
        cargo.limits[&cid("si.mass_kg")],
        cargo.limits[&cid("si.volume_l")],
    );
    let cell = |good: &str, qty: u64| {
        let row = goods.good(&cid(good)).expect("the good");
        format!(
            "    {}: {qty} t · {} m³",
            row.label,
            tacsim::port::capacity::format_m3(qty * row.measures[&cid("si.volume_l")])
        )
    };
    let label = |good: &str| goods.good(&cid(good)).expect("the good").label.clone();
    let want = [
        format!("Hold of the Salt Moth ({}):", row.mark),
        "  HOLD".to_owned(),
        cell("metals", metals),
        cell("volatiles", ice),
        cell("structural_alloys", 0),
        cell("industrial_parts", 0),
        // Doc 19 cut A.8: the moon goods.
        cell("instrument_boards", 0),
        cell("carbon_feedstock", 0),
        format!(
            "    TOTAL: {} t · {} m³",
            tacsim::port::capacity::format_tonnes(mass),
            tacsim::port::capacity::format_m3(volume)
        ),
        format!("    LIMIT: {} t · {} m³", mass / 1_000, volume / 1_000),
        "    FULL: mass and packed volume".to_owned(),
        format!(
            "  TANKAGE {}, {fuel} of {} t",
            label("propellant"),
            single_cap(&goods, "propellant", "propellant")
        ),
        format!(
            "  STORES {}, {food} of {} crew-days",
            label("provisions"),
            single_cap(&goods, "provisions", "provisions")
        ),
    ];
    assert!(
        held.contains(&want.join("\n")),
        "{held}\nwanted:\n{}",
        want.join("\n")
    );
    let credit = run(&mut rig, pilot, "+credit").await;
    assert!(
        credit.contains(&format!("{} available", cr(stake - costs))),
        "{credit}"
    );
    traces_to_origin(
        &mut rig.tw,
        hold,
        &["metals", "volatiles", "propellant", "provisions"],
    );

    // The walk takes a couple of real minutes, in which the market's sweep sees the
    // Draws' receipts (a hull waits for promised goods until then: `GOODS_COMING`).
    let _ = sweep(&mut rig, 61).await;
    let _ = sweep(&mut rig, 61).await;
    // The one staff act during play: the lane flown at 3600, after the enrolment
    // (whose readiness gate reads the production Rate line; lead ruling L1).
    let out = run(&mut rig, GOD, "@tacsim/time rate=3600").await;
    assert_eq!(
        rig.tw.world().world_timeline().current().rate,
        3_600,
        "{out}"
    );

    // Step 8: board and walk the ship.
    walk(&mut rig, pilot, "exchange", c07).await;
    walk(&mut rig, pilot, "concourse", c04).await;
    walk(&mut rig, pilot, "arrivals", c03).await;
    walk(&mut rig, pilot, "lobby", c02).await;
    walk(&mut rig, pilot, "gallery", c01).await;
    let boarded = run(&mut rig, pilot, "+ship/board").await;
    assert!(
        boarded.contains("You cycle the lock into the Salt Moth."),
        "{boarded}"
    );
    let airlock = room_of(&row, "airlock");
    let passage = room_of(&row, "passage");
    let bridge = room_of(&row, "bridge");
    let crew = room_of(&row, "crew");
    assert_eq!(location(&rig.tw, pilot), Some(airlock));
    walk(&mut rig, pilot, "in", passage).await;
    walk(&mut rig, pilot, "up", bridge).await;
    // IC11's untested Bridge render: the status line, docked, with the mark.
    let look = run(&mut rig, pilot, "look").await;
    let status = tacsim::port::ship::STATUS_DOCKED
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark)
        .replace("{port}", "Ceres Freeport");
    assert!(
        look.lines().any(|l| l == status),
        "{look}\nwanted: {status}"
    );

    // Step 9: the plan, as the planner prints it. The tank was bunkered to the
    // planner's own burn, so the burn is that fixed point with nothing left (doc 18 §2
    // step 9's 41 t was the burn of a 42 t tank under the coast law; since doc 19 cut
    // A.2 the flip law's fixed point is about 52 t), and the stores are 19 crew-days.
    let plan = run(&mut rig, pilot, "+ship/plan stickney").await;
    assert!(plan.starts_with("Plan for Stickney Yards"), "{plan}");
    let (burn, tank) = plan_numbers(&plan, "Burn ", "(tank ").expect(&plan);
    let (days, locker) = plan_numbers(&plan, "Stores ", "(locker ").expect(&plan);
    assert_eq!(
        (burn, tank),
        (fuel, fuel),
        "the burn is the bunkered fixed point: {plan}"
    );
    assert_eq!(
        (days, locker),
        (19, food),
        "the flip law's 19 crew-days (doc 19 cut A.2; 22 under the coast law on lane v4): {plan}"
    );

    // The undock, and IC9 T7: a fire while the burn is still queued in the engine
    // (the world task's deferred submit: legacy work owns the next commit) defers the
    // launch — `launch_step`'s `Step::Deferred` (`ctx.authoritative_pending` true for
    // the pilot), never the lapse grace. The burn is left in the engine's slot
    // across the fire and drained only after it.
    let filed = run(&mut rig, pilot, "+ship/undock stickney").await;
    assert!(
        filed.contains("Clearance filed for Stickney Yards"),
        "{filed}"
    );
    let events = sweep(&mut rig, 6).await;
    let stats = tacsim::port_tick::last_fire(rig.tw.world()).expect("the port tick fired");
    assert!(
        stats.deferred >= 1,
        "the launch deferred to the queued burn: {stats:?}"
    );
    let burn_leg = drain(&mut rig.tw);
    assert_eq!(
        burn_keys(&burn_leg),
        vec![format!("tacsim.port.burn/{}/undock/1", hull.0)],
        "the undock's burn, and nothing the fire queued: {filed}"
    );
    assert_eq!(burn_leg.len(), 1, "{filed}");
    let heard_now = said(&rig.tw, &events, pilot);
    assert!(
        journey(&rig.tw, hull).is_none(),
        "no journey before the burn"
    );
    assert!(!under_way(&rig.tw, hull));
    let lapsed = tacsim::port::voyage::CLEARANCE_LAPSED.replace("{port}", "Stickney Yards");
    assert!(!heard_now.contains(&lapsed), "{heard_now}");
    let look = run(&mut rig, pilot, "look").await;
    let cleared = tacsim::port::ship::STATUS_CLEARED
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark)
        .replace("{port}", "Stickney Yards");
    assert!(
        look.lines().any(|l| l == cleared),
        "{look}\nwanted: {cleared}"
    );
    let tank_before = pool(&mut rig.tw, hold, "propellant");
    assert!(matches!(
        land(&mut rig, &burn_leg[0]).await,
        Landing::Landed
    ));
    // The same drained burn again in the same boot: the coordinator's HIT.
    let replays = rig.replays;
    assert!(matches!(
        land(&mut rig, &burn_leg[0]).await,
        Landing::Replay
    ));
    assert_eq!(rig.replays, replays + 1);
    assert_eq!(tickets(&mut rig.tw, hull).len(), 1, "one ticket");
    let ticket = ticket(&mut rig.tw, hull, 1).expect("the ticket");
    assert_eq!(
        pool(&mut rig.tw, hold, "propellant"),
        tank_before - ticket.plan.burn_t,
        "one debit"
    );
    walk(&mut rig, pilot, "down", passage).await;
    walk(&mut rig, pilot, "crew", crew).await;
    // The next fire launches; the fire after it reads the journey and tells the pilot
    // (`launch_verify`).
    let events = fire_quietly(&mut rig).await;
    assert!(under_way(&rig.tw, hull), "the next fire launches");
    let mut granted = said(&rig.tw, &events, pilot);
    let events = fire_quietly(&mut rig).await;
    granted.push_str(&said(&rig.tw, &events, pilot));
    assert!(granted.contains("Clearance granted."), "{granted}");
    // The burn and the crew-days left the hold at departure: bought − burned.
    assert_eq!(
        pool(&mut rig.tw, hold, "propellant"),
        fuel - ticket.plan.burn_t
    );
    assert_eq!(
        pool(&mut rig.tw, hold, "provisions"),
        food - u64::from(ticket.plan.crew_days)
    );

    // The flight, heard in the Crew Compartment: the boundary at lane v3's first leg
    // (≈ 266.5 s real at 3600 for the doc's full outbound; this load's frozen first
    // leg is a little shorter) and the docking at its total (≈ 470.8 s). Each is
    // heard AT its instant: not by the fire a few real seconds before it (7 s at
    // 3600), and by the fire at it — as part 2 pins the return's docking.
    let boundary = ticket.plan.depart_ic + i64::from(ticket.plan.legs[0].secs);
    let arrival = ticket.plan.depart_ic + ticket.plan.total_s;
    let kirkwood = "Kirkwood Depot slides past";
    let docks = "Salt Moth docks at Stickney Yards.";
    assert!(!granted.contains(kirkwood), "not at the launch: {granted}");
    let events = to_ic(&mut rig, boundary - 7 * 3_600).await;
    let early = said(&rig.tw, &events, pilot);
    assert!(
        !early.contains(kirkwood),
        "not a fire before the boundary (IC {}, due {boundary}): {early}",
        ic_now(&rig.tw)
    );
    let events = to_ic(&mut rig, boundary).await;
    let at_boundary = said(&rig.tw, &events, pilot);
    assert!(
        at_boundary.contains(kirkwood),
        "at the boundary (IC {}, due {boundary}): {at_boundary}",
        ic_now(&rig.tw)
    );
    let events = to_ic(&mut rig, arrival - 7 * 3_600).await;
    let early = said(&rig.tw, &events, pilot);
    assert!(
        under_way(&rig.tw, hull) && !early.contains(docks),
        "under way a fire before the arrival (due {arrival}): {early}"
    );
    let events = to_ic(&mut rig, arrival).await;
    let at_arrival = said(&rig.tw, &events, pilot);
    assert!(
        !under_way(&rig.tw, hull) && at_arrival.contains(docks),
        "docked at the ticket's arrival (IC {}, due {arrival}): {at_arrival}",
        ic_now(&rig.tw)
    );

    // Steps 12–13 by hand: ashore, the walk to Kasei's Receiving Bay.
    let p01 = station_room(&rig.tw, "stickney_port", "room.p01_berth_gallery");
    let p02 = station_room(&rig.tw, "stickney_port", "room.p02_pressure");
    let p03 = station_room(&rig.tw, "stickney_port", "room.p03_customs");
    let p04 = station_room(&rig.tw, "stickney_port", "room.p04_concourse");
    let p07 = station_room(&rig.tw, "stickney_port", "room.p07_receiving");
    let p08 = station_room(&rig.tw, "stickney_port", "room.p08_exchange");
    let p12 = station_room(&rig.tw, "stickney_port", "room.p12_observation");
    walk(&mut rig, pilot, "passage", passage).await;
    walk(&mut rig, pilot, "airlock", airlock).await;
    let ashore = run(&mut rig, pilot, "+ship/leave").await;
    assert_eq!(location(&rig.tw, pilot), Some(p01), "{ashore}");
    walk(&mut rig, pilot, "lobby", p02).await;
    walk(&mut rig, pilot, "customs", p03).await;
    walk(&mut rig, pilot, "concourse", p04).await;
    walk(&mut rig, pilot, "exchange", p08).await;
    walk(&mut rig, pilot, "down", p07).await;

    // Kasei's two bids, and both goods sold into them with doc 18's lines.
    let kasei = port_map(&rig.tw, "stickney_port").vendors[&cid("kasei")].clone();
    let metals_bid = ListingId(kasei.bids[&cid("metals")].listing);
    let ice_bid = ListingId(kasei.bids[&cid("volatiles")].listing);
    let listed = run(&mut rig, pilot, "+market/list").await;
    assert!(listed.contains(&format!("#{metals_bid} ")), "{listed}");
    assert!(listed.contains(&format!("#{ice_bid} ")), "{listed}");
    let (metals_intake, ice_intake) = (
        listed_qty(&rig.tw, metals_bid).expect("bid"),
        listed_qty(&rig.tw, ice_bid).expect("bid"),
    );
    let permille = store(&rig.tw)
        .enrolment_of(pilot)
        .map(|e| e.share_permille)
        .expect("the enrolment");
    let mut contracts: Vec<(ContractId, i64)> = Vec::new();
    let (mut margins, mut shares_billed) = (0_i64, 0_i64);
    for (bid, ask, good, qty) in [
        (metals_bid, metals_ask, "metals", metals),
        (ice_bid, ice_ask, "volatiles", ice),
    ] {
        let price = price_of(&rig.tw, bid);
        // The reference is the origin ask's live price (IC10 decision 8).
        let reference = price_of(&rig.tw, ask);
        let gross = price * i64::try_from(qty).expect("qty");
        let (margin, share) = margin_and_share(price, reference, qty, permille);
        margins += margin;
        shares_billed += share;
        let line = format!(
            "Sold {} t of {} to Kasei Metals at {}/t: {}; Co-op share {}; you receive {} when \
             the goods are in. Jonas Achterberg-Nkemelu countersigns.",
            grouped(qty),
            label(good),
            cr(price),
            cr(gross),
            cr(share),
            cr(gross - share)
        );
        let sold = run(&mut rig, pilot, &format!("+market/sell {bid}={qty}")).await;
        assert!(sold.lines().any(|l| l == line), "{sold}\nwanted: {line}");
        let q = market(&rig.tw)
            .quotes_of_seller(&PrincipalId::Object(pilot))
            .last()
            .cloned()
            .expect("the sale");
        let leg = drain(&mut rig.tw);
        assert_eq!(leg.len(), 1, "{sold}");
        land_all(&mut rig, &leg).await;
        contracts.push((q.contract.expect("contract"), gross));
    }
    for _ in 0..12 {
        let _ = sweep(&mut rig, 31).await;
    }

    // Journals: pilot + Trust = Kasei's payment on each sale.
    let kasei_account = mush_economy::AccountId(
        port_map(&rig.tw, "stickney_port").orgs[&cid("kasei")]
            .account
            .expect("Kasei's account"),
    );
    let trust = mush_economy::AccountId(
        port_map(&rig.tw, "ceres_port").orgs[&cid("coop_trust")]
            .account
            .expect("the Trust's account"),
    );
    let (mut nets, mut shares) = (0_i64, 0_i64);
    for (contract, gross) in &contracts {
        let lines = settlement(&rig.tw, *contract);
        let paid = -lines.get(&kasei_account).copied().unwrap_or(0);
        let net = lines.get(&me).copied().unwrap_or(0);
        let share = lines.get(&trust).copied().unwrap_or(0);
        assert_eq!(paid, *gross);
        assert_eq!(net + share, paid, "pilot + Trust = Kasei's payment");
        assert_eq!(lines.values().sum::<i64>(), 0);
        nets += net;
        shares += share;
    }
    let credit = run(&mut rig, pilot, "+credit").await;
    assert!(
        credit.contains(&format!("{} available", cr(stake - costs + nets))),
        "{credit}"
    );
    assert_eq!(
        shares, shares_billed,
        "the Trust was paid what each line billed"
    );
    assert_eq!(
        closed_sum(&rig.tw, &[pilot]),
        books_before,
        "money is conserved"
    );

    // §2 day 2's walk (steps 16–17): back to the hull by the pack's own words, and
    // the statement on the way.
    walk(&mut rig, pilot, "exchange", p08).await;
    walk(&mut rig, pilot, "concourse", p04).await;
    walk(&mut rig, pilot, "observation", p12).await;
    walk(&mut rig, pilot, "concourse", p04).await;
    let statement = run(&mut rig, pilot, "+coop/statement").await;
    assert!(
        statement.contains(&format!(
            "  Sales: 2; margin {}; Co-op share paid {}.",
            cr(margins),
            cr(shares)
        )),
        "{statement}"
    );
    walk(&mut rig, pilot, "customs", p03).await;
    walk(&mut rig, pilot, "lobby", p02).await;
    walk(&mut rig, pilot, "gallery", p01).await;
    let boarded = run(&mut rig, pilot, "+ship/board").await;
    assert!(
        boarded.contains("You cycle the lock into the Salt Moth."),
        "{boarded}"
    );
    assert_eq!(location(&rig.tw, pilot), Some(airlock));

    // Seeded − sold = remaining on every touched listing and pool; Kasei's pools are
    // the delivered intake.
    let ceres = port_map(&rig.tw, "ceres_port");
    let exchange_store = VaultId(ceres.vendors[&cid("freeport_exchange")].vault);
    let occator_store = VaultId(ceres.vendors[&cid("occator")].vault);
    let coop_store = VaultId(ceres.vendors[&cid("coop")].vault);
    for (listing, seeded, bought) in [
        (metals_ask, metals_seeded, metals),
        (ice_ask, ice_seeded, ice),
        (fuel_ask, fuel_seeded, fuel),
        (food_ask, food_seeded, food),
    ] {
        assert_eq!(listed_qty(&rig.tw, listing), Some(seeded - bought));
    }
    assert_eq!(
        pool(&mut rig.tw, exchange_store, "metals"),
        metals_seeded - metals
    );
    assert_eq!(
        pool(&mut rig.tw, occator_store, "volatiles"),
        ice_seeded - ice
    );
    assert_eq!(
        pool(&mut rig.tw, occator_store, "propellant"),
        fuel_seeded - fuel
    );
    assert_eq!(
        pool(&mut rig.tw, coop_store, "provisions"),
        food_seeded - food
    );
    assert_eq!(
        listed_qty(&rig.tw, metals_bid),
        Some(metals_intake - metals)
    );
    assert_eq!(listed_qty(&rig.tw, ice_bid), Some(ice_intake - ice));
    let kasei_store = VaultId(kasei.vault);
    assert_eq!(pool(&mut rig.tw, kasei_store, "metals"), metals);
    assert_eq!(pool(&mut rig.tw, kasei_store, "volatiles"), ice);
    assert_eq!(pool(&mut rig.tw, hold, "metals"), 0);
    assert_eq!(pool(&mut rig.tw, hold, "volatiles"), 0);
    assert_no_pages(&rig.tw);

    // A fresh tester's help (doc 18 §5 IC12 row).
    assert_help(&mut rig, pilot).await;
}

// ---------------------------------------------------------------------------
// Shared set-up: enrolment and the Ceres outfit at the play rate
// ---------------------------------------------------------------------------

/// What [`outfit`] bought at Ceres.
struct Outfit {
    costs: i64,
    metals: u64,
    ice: u64,
    fuel: u64,
    food: u64,
}

/// **Steps 2–4 at the play rate**: the IC door, the desk, `+coop/enrol <name>`.
/// Returns the hull's row; the hold is tracked for the limit checks.
async fn enrol(rig: &mut Rig, pilot: ObjId, name: &str) -> HullRow {
    let c03 = station_room(&rig.tw, "ceres_port", "room.c03_arrivals");
    let c05 = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    walk(rig, pilot, "c", c03).await;
    walk(rig, pilot, "registry", c05).await;
    let enrolled = run(rig, pilot, &format!("+coop/enrol {name}")).await;
    assert!(enrolled.contains(&format!("now the {name}")), "{enrolled}");
    let row = hull_row(&rig.tw, pilot);
    let hold = hold_of(&rig.tw, &row);
    rig.holds.insert(hold.0);
    row
}

/// **Steps 6–7 from the Registry**: `out`, `concourse`, `exchange`, the both-limits
/// cargo, `chandlery`, reaction mass and provisions to the planner's own figures for
/// Stickney (IC10's loop), then the market sweep's two laps for the Draws' receipts.
async fn outfit(rig: &mut Rig, pilot: ObjId) -> Outfit {
    let c03 = station_room(&rig.tw, "ceres_port", "room.c03_arrivals");
    let c04 = station_room(&rig.tw, "ceres_port", "room.c04_concourse");
    let c07 = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    let c08 = station_room(&rig.tw, "ceres_port", "room.c08_chandlery");
    walk(rig, pilot, "out", c03).await;
    walk(rig, pilot, "concourse", c04).await;
    walk(rig, pilot, "exchange", c07).await;
    let (metals, ice) = vertex_load();
    let metals_ask = listing_of(&rig.tw, "ceres_port", "freeport_exchange", "metals");
    let ice_ask = listing_of(&rig.tw, "ceres_port", "occator", "volatiles");
    let mut costs = buy(rig, pilot, metals_ask, metals).await;
    costs += buy(rig, pilot, ice_ask, ice).await;
    walk(rig, pilot, "chandlery", c08).await;
    let fuel_ask = listing_of(&rig.tw, "ceres_port", "occator", "propellant");
    let food_ask = listing_of(&rig.tw, "ceres_port", "coop", "provisions");
    let (mut fuel, mut food) = (0_u64, 0_u64);
    for _ in 0..6 {
        let plan = run(rig, pilot, "+ship/plan stickney").await;
        let (burn, tank) = plan_numbers(&plan, "Burn ", "(tank ").expect(&plan);
        let (days, locker) = plan_numbers(&plan, "Stores ", "(locker ").expect(&plan);
        if tank >= burn && locker >= days {
            break;
        }
        if tank < burn {
            costs += buy(rig, pilot, fuel_ask, burn - tank).await;
            fuel += burn - tank;
        }
        if locker < days {
            costs += buy(rig, pilot, food_ask, days - locker).await;
            food += days - locker;
        }
    }
    let _ = sweep(rig, 61).await;
    let _ = sweep(rig, 61).await;
    Outfit {
        costs,
        metals,
        ice,
        fuel,
        food,
    }
}

/// From the chandlery to the Airlock by §2 step 8's words, and `+ship/board`.
async fn board_from_chandlery(rig: &mut Rig, pilot: ObjId, name: &str) {
    for (word, local) in [
        ("exchange", "room.c07_exchange"),
        ("concourse", "room.c04_concourse"),
        ("arrivals", "room.c03_arrivals"),
        ("lobby", "room.c02_pressure"),
        ("gallery", "room.c01_berth_gallery"),
    ] {
        let to = station_room(&rig.tw, "ceres_port", local);
        walk(rig, pilot, word, to).await;
    }
    let boarded = run(rig, pilot, "+ship/board").await;
    assert!(
        boarded.contains(&format!("You cycle the lock into the {name}.")),
        "{boarded}"
    );
}

/// `@tacsim/time rate=<rate>` as GOD, asserted.
async fn set_rate(rig: &mut Rig, rate: u32) {
    let out = run(rig, GOD, &format!("@tacsim/time rate={rate}")).await;
    assert_eq!(
        rig.tw.world().world_timeline().current().rate,
        rate,
        "{out}"
    );
}

/// Every sale `who` made (the market's seller quotes).
fn sales_of(tw: &TestWorld, who: ObjId) -> Vec<mush_market::Quote> {
    market(tw)
        .quotes_of_seller(&PrincipalId::Object(who))
        .cloned()
        .collect()
}

/// Every lot listing's quantity, by id.
fn listing_quantities(tw: &TestWorld) -> BTreeMap<u64, u64> {
    market(tw)
        .lot_listings()
        .map(|l| (l.id.0, l.quantity))
        .collect()
}

/// The ask listings of one port's vendors, as `(listing, good, quantity now)`.
fn asks_of(tw: &TestWorld, entry: &str) -> Vec<(ListingId, String, u64)> {
    let map = port_map(tw, entry);
    let mut out = Vec::new();
    for row in map.vendors.values() {
        let vault = EndpointRef(VaultId(row.vault));
        for good in [
            "metals",
            "volatiles",
            "propellant",
            "provisions",
            "industrial_parts",
            "structural_alloys",
        ] {
            if let Some(listing) = market(tw).lot_listing_of(vault, &cid(good)) {
                let seeded = listed_qty(tw, listing).unwrap_or(0);
                out.push((listing, good.to_owned(), seeded));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Part 2: the queue runs the return
// ---------------------------------------------------------------------------

/// **Doc 18 §6 part 2: a pilot's queue run unattended through Stickney and back to
/// Ceres** (doc 18 §2 days 1–2, D-18-38; one pilot alone in this world, F5).
///
/// Wren enrols and loads at the play rate, boards, and — after the one staff act —
/// files §2 step 10's queue with `at=+10m` for the return (the shipped grammar has
/// no seconds form; lead ruling L12) and disconnects. No line is typed by the pilot
/// until the reconnect. The three §6 restarts are real `build_engine` re-boots over
/// the kept store: (a) between the depart's burn landing and its launch — the burn
/// replayed in the new epoch before any fire is refused by the provider (the ticket
/// at its `seq` is the guard), then one launch, one journey, no new burn under any
/// key; (b) with a sale in flight and its deliver leg lost — re-staged under its
/// contract key, settled once, "verified, not re-run"; (c) the kept world after the
/// return — nothing submitted, no stock re-minted, no listing moved, no stake
/// re-posted, no capital re-issued, no object made. The Stickney buys are asserted
/// line by line against figures computed from the boards and the planning core (the
/// fill to the return's fixed point; the alloys "short of 55 t: funds"). The pilot
/// reconnects at Ceres to the watch log; the statement reads two sales.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn ic12_part_two_the_queue_runs_the_return() {
    let mut rig = boot("part2").await;
    let _ = sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let ceres_asks = asks_of(&rig.tw, "ceres_port");
    let stickney_asks = asks_of(&rig.tw, "stickney_port");
    let receipts_at_stock = receipts(&mut rig.tw).len();

    let wren = pilot(&mut rig, "Wren", "wrenacct").await;
    let row = enrol(&mut rig, wren, "Salt Moth").await;
    let hull = ObjId(row.hull);
    let hold = hold_of(&rig.tw, &row);
    let me = credit_account(&rig.tw, &PrincipalId::Object(wren));
    let stake = posted(&rig.tw, me);
    let books = closed_sum(&rig.tw, &[wren]);
    let outfit = outfit(&mut rig, wren).await;
    assert_eq!(posted(&rig.tw, me), stake - outfit.costs);
    board_from_chandlery(&mut rig, wren, "Salt Moth").await;
    let passage = room_of(&row, "passage");
    let crew = room_of(&row, "crew");
    walk(&mut rig, wren, "in", passage).await;
    walk(&mut rig, wren, "crew", crew).await;
    set_rate(&mut rig, 3_600).await;

    // Step 10: the queue, filed aboard (the return at `+10m`).
    let queue = [
        "depart stickney at=now",
        "sell metals all min=250",
        "sell volatiles all min=150",
        "buy propellant fill max=60",
        "buy provisions fill",
        "buy industrial_parts 49 max=400",
        "buy structural_alloys 55 max=400",
        "depart ceres at=+10m",
    ];
    for (n, line) in (1_u32..).zip(queue) {
        let out = run(&mut rig, wren, &format!("+ship/order {line}")).await;
        assert!(out.contains(&format!("Order {n} filed")), "`{line}`: {out}");
        if n == 1 {
            assert!(
                out.contains(&tacsim::port::orders::ORDER_FIRES_NEXT.replace("{n}", "1")),
                "{out}"
            );
        }
    }
    let shown = run(&mut rig, wren, "+ship/orders").await;
    for n in 1..=8 {
        assert!(
            shown.lines().any(|l| l.starts_with(&format!("  {n}. "))),
            "order {n} listed:\n{shown}"
        );
    }
    // Step 11: log off aboard.
    disconnect(&mut rig.tw, wren);

    // ---- Restart (a): between the depart's burn and its launch ----
    let key = format!("tacsim.port.burn/{}/1/1", hull.0);
    let mut kept: Option<AuthoritativeSubmit> = None;
    for _ in 0..10 {
        let (_, queued) = fire(&mut rig).await;
        for submit in &queued {
            if submit.key.as_str() == key {
                kept = Some(submit.clone());
            }
        }
        land_all(&mut rig, &queued).await;
        if kept.is_some() {
            break;
        }
    }
    let burn = kept.expect("the order arm submitted the depart's burn");
    assert_eq!(tickets(&mut rig.tw, hull).len(), 1, "the burn landed");
    assert!(!under_way(&rig.tw, hull), "not launched before the restart");
    let tank_after_burn = pool(&mut rig.tw, hold, "propellant");
    restart(&mut rig, |tw| ticket(tw, hull, 1).is_some()).await;
    // The kept burn, landed in the new epoch before any fire — the hull still docked,
    // the passage still filed, so the offer projects and the prepare runs: the
    // coordinator preflights it Vacant (cross-restart replay is the mechanic's), and
    // the provider's own guard refuses it because the ticket at its `seq` exists.
    assert!(!under_way(&rig.tw, hull), "still docked after the restart");
    match land(&mut rig, &burn).await {
        Landing::Refused(detail) => {
            assert!(
                detail.contains("that passage is already burned"),
                "the ticket's seq guard: {detail}"
            );
        }
        Landing::Replay => panic!("a new epoch never preflights the old request as a HIT"),
        Landing::Landed => panic!("the kept burn landed a second time"),
    }
    assert_eq!(tickets(&mut rig.tw, hull).len(), 1, "not a second burn");
    assert_eq!(
        pool(&mut rig.tw, hold, "propellant"),
        tank_after_burn,
        "nothing debited twice"
    );
    let mut after: Vec<AuthoritativeSubmit> = Vec::new();
    for _ in 0..4 {
        let (_, queued) = fire(&mut rig).await;
        after.extend(queued.iter().cloned());
        land_all(&mut rig, &queued).await;
    }
    assert!(
        burn_keys(&after).is_empty(),
        "no new burn under any key: {:?}",
        burn_keys(&after)
    );
    assert!(under_way(&rig.tw, hull), "one launch");
    let out_ticket = ticket(&mut rig.tw, hull, 1).expect("the ticket");
    let flown = journey(&rig.tw, hull).expect("one journey");
    assert_eq!(
        flown.departed().at,
        out_ticket.plan.depart_ic,
        "the ticket's journey"
    );
    assert_eq!(tickets(&mut rig.tw, hull).len(), 1, "still one ticket");
    assert_eq!(
        pool(&mut rig.tw, hold, "propellant"),
        tank_after_burn,
        "the launch debits nothing more"
    );
    assert_eq!(order_state(&rig.tw, hull, 1), Some(OrderState::Done));

    // ---- The passage, and restart (b) with a sale in flight ----
    // The arrival fire docks the hull and, in the same fire, the order arm takes its
    // next step: the sale, whose deliver leg is queued and must not land here.
    let arrival = out_ticket.plan.depart_ic + out_ticket.plan.total_s;
    let mut first = Some(to_ic_queued(&mut rig, arrival).await);
    let mut deliver: Option<AuthoritativeSubmit> = None;
    for _ in 0..20 {
        let queued = match first.take() {
            Some(queued) => queued,
            None => fire(&mut rig).await.1,
        };
        let (legs, rest): (Vec<_>, Vec<_>) = queued
            .into_iter()
            .partition(|s| s.key.as_str().starts_with("market.lot.deliver."));
        land_all(&mut rig, &rest).await;
        if let Some(leg) = legs.into_iter().next() {
            deliver = Some(leg);
            break;
        }
    }
    let deliver = deliver.expect("the first sale's deliver leg is queued");
    // The tank and the locker as the hull docked: where the fill later starts (the
    // sales take only cargo).
    let tank_docked = pool(&mut rig.tw, hold, "propellant");
    let locker_docked = pool(&mut rig.tw, hold, "provisions");
    let inflight = orders(&rig.tw, hull).inflight.expect("the sale in flight");
    assert_eq!(inflight.order, 2);
    let contract = inflight.refs.contract.expect("the sale's contract");
    assert_eq!(
        deliver.key.as_str(),
        format!("market.lot.deliver.{contract:x}")
    );
    let kasei = port_map(&rig.tw, "stickney_port").vendors[&cid("kasei")].clone();
    let kasei_store = VaultId(kasei.vault);
    let metals_bid = ListingId(kasei.bids[&cid("metals")].listing);
    let bid_before = listed_qty(&rig.tw, metals_bid).expect("the bid");
    // The deliver leg dies with the old engine's queue, as a kill would lose it.
    restart(&mut rig, |tw| ticket(tw, hull, 1).is_some()).await;
    let mut market_keys: Vec<String> = Vec::new();
    for _ in 0..40 {
        if order_state(&rig.tw, hull, 2) == Some(OrderState::Done) {
            break;
        }
        let (_, queued) = fire(&mut rig).await;
        market_keys.extend(
            queued
                .iter()
                .filter(|s| s.action.key().owner.as_str() == "market")
                .map(|s| s.key.as_str().to_owned()),
        );
        land_all(&mut rig, &queued).await;
    }
    assert_eq!(order_state(&rig.tw, hull, 2), Some(OrderState::Done));
    // The step in flight is re-staged once under its contract's key; the fire that
    // verifies it may already take order 3's sale (a different contract, the water-ice
    // bid's), and nothing else is submitted.
    let restaged = format!("market.lot.deliver.{contract:x}");
    assert_eq!(
        market_keys.iter().filter(|k| **k == restaged).count(),
        1,
        "the re-staged deliver, once: {market_keys:?}"
    );
    let ice_bid = ListingId(kasei.bids[&cid("volatiles")].listing);
    for key in market_keys.iter().filter(|k| **k != restaged) {
        let other = sales_of(&rig.tw, wren)
            .into_iter()
            .find(|q| {
                q.contract
                    .is_some_and(|c| *key == format!("market.lot.deliver.{:x}", c.0))
            })
            .unwrap_or_else(|| panic!("{key} is a sale's leg"));
        assert_eq!(
            other.listing, ice_bid,
            "{key} is order 3's sale, not a re-run"
        );
    }
    assert_eq!(
        sales_of(&rig.tw, wren)
            .iter()
            .filter(|q| q.listing == metals_bid)
            .count(),
        1,
        "no second sale into the metals bid"
    );
    assert!(matches!(
        economy(&rig.tw)
            .snapshot()
            .contract(ContractId(contract))
            .map(|c| c.state),
        Some(ContractState::Settled { .. })
    ));
    assert_eq!(
        listed_qty(&rig.tw, metals_bid),
        Some(bid_before),
        "one bid decrement"
    );
    assert_eq!(
        pool(&mut rig.tw, kasei_store, "metals"),
        outfit.metals,
        "Kasei +20 once"
    );
    assert_eq!(store(&rig.tw).enrolment_of(wren).map(|e| e.sales), Some(1));

    // ---- The queue runs: the second sale, the fill, the fittings, the alloys ----
    for _ in 0..200 {
        if order_state(&rig.tw, hull, 8) == Some(OrderState::Done) && under_way(&rig.tw, hull) {
            break;
        }
        let _ = fire_quietly(&mut rig).await;
    }
    assert_eq!(
        order_state(&rig.tw, hull, 8),
        Some(OrderState::Done),
        "{:#?}",
        log_texts(&rig.tw, hull)
    );
    assert!(under_way(&rig.tw, hull), "the return launched");
    assert_eq!(store(&rig.tw).enrolment_of(wren).map(|e| e.sales), Some(2));
    let back = ticket(&mut rig.tw, hull, 2).expect("the return's ticket");
    assert_eq!(tickets(&mut rig.tw, hull).len(), 2);
    let log = log_texts(&rig.tw, hull);
    for id in 4..=7 {
        assert_eq!(
            order_state(&rig.tw, hull, id),
            Some(OrderState::Done),
            "order {id}: {log:#?}"
        );
    }
    // ---- The Stickney buys, every figure computed (D-18-39, risk 16) ----
    // The prices from the live boards; the fill's targets from the planning core's
    // own pure functions (`project_cargo`, `fill_targets`) over the hold as it stood
    // at the fill — the cargo sold, the tank and the locker as they docked — with the
    // passage planner over the world's own sky for the return's route; the fittings
    // and the alloys from Wren's funds after the earlier buys and the hold's room by
    // both limits.
    let expected = stickney_buys(
        &rig,
        hull,
        (tank_docked, locker_docked),
        wren,
        me,
        stake - outfit.costs,
    );
    for line in &expected.lines {
        assert!(
            log.iter().any(|l| l == line),
            "{line}\nin the log:\n{log:#?}"
        );
    }
    // The return docks at Ceres at its frozen instant (≈ 418 s real at 3600 for this
    // load): still under way a fire before it, docked by the fire at it.
    let home = back.plan.depart_ic + back.plan.total_s;
    let _ = to_ic(&mut rig, home - 7 * 3_600).await;
    assert!(
        under_way(&rig.tw, hull),
        "under way a fire before the arrival"
    );
    let _ = to_ic(&mut rig, home).await;
    assert!(
        !under_way(&rig.tw, hull),
        "docked at Ceres at the frozen instant"
    );
    assert_eq!(
        journey(&rig.tw, hull).map(|j| j.departed().at),
        Some(back.plan.depart_ic),
        "the return flew its ticket"
    );

    // ---- The pilot reconnects at Ceres ----
    let notices = tacsim::port_tick::pending_notices(rig.tw.world(), wren);
    assert!(!notices.is_empty(), "the watch log queued notices");
    let events = rig.tw.connect(wren);
    flush(&mut rig).await;
    let heard = said(&rig.tw, &events, wren);
    assert_eq!(
        heard.matches(tacsim::port_tick::NOTICE_HEADING).count(),
        1,
        "{heard}"
    );
    let mut at = heard
        .find(tacsim::port_tick::NOTICE_HEADING)
        .expect("the heading");
    for notice in &notices {
        assert_eq!(
            heard.matches(notice.as_str()).count(),
            1,
            "{notice}:\n{heard}"
        );
        let found = heard[at..].find(notice.as_str()).map(|i| i + at);
        let found = found.unwrap_or_else(|| panic!("{notice} in queue order:\n{heard}"));
        at = found;
    }
    let look = run(&mut rig, wren, "look").await;
    let status = tacsim::port::ship::STATUS_DOCKED
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark)
        .replace("{port}", "Ceres Freeport");
    assert!(
        look.lines().any(|l| l == status),
        "{look}\nwanted: {status}"
    );
    let shown = run(&mut rig, wren, "+ship/orders").await;
    assert!(
        shown.contains(", signed Wren:"),
        "the signatory rule:\n{shown}"
    );

    // Money: Wren + Trust = Kasei's payment on every sale; Σ posted unchanged.
    let trust = mush_economy::AccountId(
        port_map(&rig.tw, "ceres_port").orgs[&cid("coop_trust")]
            .account
            .expect("the Trust's account"),
    );
    let kasei_account = mush_economy::AccountId(
        port_map(&rig.tw, "stickney_port").orgs[&cid("kasei")]
            .account
            .expect("Kasei's account"),
    );
    let permille = store(&rig.tw)
        .enrolment_of(wren)
        .map(|e| e.share_permille)
        .expect("the enrolment");
    let mut shares = 0_i64;
    let sold = sales_of(&rig.tw, wren);
    assert_eq!(sold.len(), 2, "two sales");
    for q in &sold {
        let lines = settlement(&rig.tw, q.contract.expect("a contract"));
        let paid = -lines.get(&kasei_account).copied().unwrap_or(0);
        let net = lines.get(&me).copied().unwrap_or(0);
        let share = lines.get(&trust).copied().unwrap_or(0);
        assert_eq!(net + share, paid, "Wren + Trust = Kasei's payment");
        assert_eq!(lines.values().sum::<i64>(), 0);
        shares += share;
    }
    // The statement's margin and share, computed from the posted prices: each sale's
    // margin over the Ceres reference ask, its share floored at the frozen permille.
    let (mut margins, mut billed) = (0_i64, 0_i64);
    for (bid, ask, qty) in [
        (
            metals_bid,
            listing_of(&rig.tw, "ceres_port", "freeport_exchange", "metals"),
            outfit.metals,
        ),
        (
            ListingId(kasei.bids[&cid("volatiles")].listing),
            listing_of(&rig.tw, "ceres_port", "occator", "volatiles"),
            outfit.ice,
        ),
    ] {
        let (margin, share) = margin_and_share(
            price_of(&rig.tw, bid),
            price_of(&rig.tw, ask),
            qty,
            permille,
        );
        margins += margin;
        billed += share;
    }
    assert_eq!(shares, billed, "the Trust was paid what each line billed");
    let statement = run(&mut rig, wren, "+coop/statement").await;
    assert!(
        statement.contains(&format!(
            "  Sales: 2; margin {}; Co-op share paid {}.",
            cr(margins),
            cr(shares)
        )),
        "{statement}"
    );
    assert_eq!(closed_sum(&rig.tw, &[wren]), books, "money is conserved");

    // Goods: seeded − bought = remaining on every ask; Kasei's pools are the delivered
    // intake; the hold is bought − sold − burned, both legs' burns from the tickets.
    // A listing gone from the market's book reads 0 remaining.
    let remaining: BTreeMap<u64, u64> = ceres_asks
        .iter()
        .chain(stickney_asks.iter())
        .map(|(id, _, _)| (id.0, listed_qty(&rig.tw, *id).unwrap_or(0)))
        .collect();
    let bought = |asks: &[(ListingId, String, u64)], good: &str| -> u64 {
        asks.iter()
            .filter(|(_, g, _)| g == good)
            .map(|(id, _, seeded)| seeded - remaining[&id.0])
            .sum()
    };
    assert_eq!(bought(&ceres_asks, "metals"), outfit.metals);
    assert_eq!(bought(&ceres_asks, "volatiles"), outfit.ice);
    assert_eq!(bought(&ceres_asks, "propellant"), outfit.fuel);
    assert_eq!(bought(&ceres_asks, "provisions"), outfit.food);
    assert_eq!(pool(&mut rig.tw, kasei_store, "metals"), outfit.metals);
    assert_eq!(pool(&mut rig.tw, kasei_store, "volatiles"), outfit.ice);
    assert_eq!(pool(&mut rig.tw, hold, "metals"), 0);
    assert_eq!(pool(&mut rig.tw, hold, "volatiles"), 0);
    for (good, qty) in [
        ("propellant", expected.fuel),
        ("provisions", expected.food),
        ("industrial_parts", expected.parts),
        ("structural_alloys", expected.alloys),
    ] {
        assert_eq!(
            bought(&stickney_asks, good),
            qty,
            "{good}: the Stickney asks sold what the clamp computed"
        );
    }
    for good in ["industrial_parts", "structural_alloys"] {
        assert_eq!(
            pool(&mut rig.tw, hold, good),
            bought(&stickney_asks, good),
            "{good} aboard is what the Stickney asks sold"
        );
    }
    // The fill bought the tank to the return's fixed point, which covers the burn the
    // return flew (sized on the heaviest projected load, so never short).
    assert_eq!(tank_docked + expected.fuel, expected.tank_target);
    assert!(
        back.plan.burn_t <= expected.tank_target,
        "the fill covers the return's burn: {} against {}",
        back.plan.burn_t,
        expected.tank_target
    );
    assert!(
        u64::from(back.plan.crew_days) <= locker_docked + expected.food,
        "the fill covers the return's crew-days"
    );
    assert_eq!(
        pool(&mut rig.tw, hold, "propellant"),
        outfit.fuel + bought(&stickney_asks, "propellant")
            - out_ticket.plan.burn_t
            - back.plan.burn_t,
        "the tank is bought − burned over both legs"
    );
    assert_eq!(
        pool(&mut rig.tw, hold, "provisions"),
        outfit.food + bought(&stickney_asks, "provisions")
            - u64::from(out_ticket.plan.crew_days)
            - u64::from(back.plan.crew_days),
        "the locker is bought − eaten over both legs"
    );
    traces_to_origin(
        &mut rig.tw,
        hold,
        &["industrial_parts", "structural_alloys"],
    );
    assert_no_pages(&rig.tw);

    // ---- Restart (c): the kept world ----
    let receipts_before = receipts(&mut rig.tw).len();
    assert_eq!(
        receipts_before, receipts_at_stock,
        "the run minted no stock"
    );
    let listings_before = listing_quantities(&rig.tw);
    let balance = posted(&rig.tw, me);
    let enrolment = store(&rig.tw).enrolment_of(wren).cloned();
    let obligations = economy(&rig.tw).snapshot().obligations().count();
    let objects = rig.tw.world().iter().count();
    restart(&mut rig, |tw| ticket(tw, hull, 2).is_some()).await;
    // The kept world submits nothing on its own: no stock mint, no stake or capital
    // re-posted, no order step re-run — every fire's queue is empty (and anything it
    // did queue would have to land, so a guard refusing in a loop fails here).
    let mut after_c: Vec<AuthoritativeSubmit> = Vec::new();
    for _ in 0..4 {
        let (_, queued) = fire(&mut rig).await;
        after_c.extend(queued.iter().cloned());
        land_all(&mut rig, &queued).await;
    }
    assert!(
        after_c.is_empty(),
        "the kept world submits nothing: {:?}",
        after_c
            .iter()
            .map(|s| (
                s.action.key().id.as_str().to_owned(),
                s.key.as_str().to_owned()
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        receipts(&mut rig.tw).len(),
        receipts_before,
        "no stock re-minted"
    );
    assert_eq!(
        listing_quantities(&rig.tw),
        listings_before,
        "no listing moved"
    );
    assert_eq!(posted(&rig.tw, me), balance, "the stake is not re-posted");
    assert_eq!(store(&rig.tw).enrolment_of(wren).cloned(), enrolment);
    assert_eq!(
        economy(&rig.tw).snapshot().obligations().count(),
        obligations,
        "no capital re-issued"
    );
    assert_eq!(rig.tw.world().iter().count(), objects, "no object made");
    assert_eq!(rig.boots, 4);
    assert_no_pages(&rig.tw);
}

/// What part 2's Stickney buys must come to, computed (never read back from the
/// log): each quantity and the log line the executor writes for it.
struct StickneyBuys {
    fuel: u64,
    food: u64,
    parts: u64,
    alloys: u64,
    tank_target: u64,
    lines: Vec<String>,
}

/// **Part 2's Stickney buys, computed from the live boards and the planning core**
/// (doc 18 §2 step 14; D-18-39, D-18-41; the brief's "the alloys clamped by funds").
///
/// The fill: `project_cargo` over orders 4–7 from the hold as it stood at the fill
/// (the cargo sold, the tank and the locker as they docked), then `fill_targets` with
/// the passage planner over the world's sky (`[tacsim.time.ephemeris]`) for the port's
/// listed return route. The fittings: 49 t, no bound biting. The alloys: Wren's funds
/// after the fill and the fittings (`start`, the stake less Ceres, plus the two sales' nets
/// from their settlement journals) over the alloy ask, which must be below the 55 t
/// wanted, the listing and the hold's room by both limits after the fittings — so
/// the line reads "short of 55 t: funds".
fn stickney_buys(
    rig: &Rig,
    hull: ObjId,
    (tank_docked, locker_docked): (u64, u64),
    wren: ObjId,
    me: mush_economy::AccountId,
    start: i64,
) -> StickneyBuys {
    use tacsim::port::executor::{BOUGHT, BOUGHT_FILL_LOCKER, BOUGHT_FILL_TANK, BOUGHT_SHORT};
    let tw = &rig.tw;
    let goods = goods();
    let label = |good: &str| goods.good(&cid(good)).expect("the good").label.clone();
    let ask = |org: &str, good: &str| listing_of(tw, "stickney_port", org, good);
    let fuel_price = price_of(tw, ask("kasei", "propellant"));
    let food_price = price_of(tw, ask("yard_commissary", "provisions"));
    let parts_listing = ask("stickney_yards", "industrial_parts");
    let alloy_listing = ask("kasei", "structural_alloys");
    let (parts_price, alloy_price) = (price_of(tw, parts_listing), price_of(tw, alloy_listing));
    assert!(fuel_price <= 6_000, "under the fill's max=60");
    assert!(
        parts_price <= 40_000 && alloy_price <= 40_000,
        "under max=400"
    );

    // The fill's fixed point.
    let comps = dray_compartments();
    let docked: BTreeMap<CanonicalId, u64> = [
        (cid("propellant"), tank_docked),
        (cid("provisions"), locker_docked),
    ]
    .into_iter()
    .filter(|(_, qty)| *qty > 0)
    .collect();
    let readings: Vec<tacsim::port::gauges::Reading> = comps
        .iter()
        .map(|comp| {
            let contents = docked
                .iter()
                .filter(|(good, _)| {
                    tacsim::port::capacity::compartment_for(&goods, &comps, good)
                        .is_some_and(|c| c.id == comp.id)
                })
                .map(|(good, qty)| (good.clone(), *qty))
                .collect();
            (comp.id.clone(), comp.clone(), contents)
        })
        .collect();
    let doc = orders(tw, hull);
    let between: Vec<&tacsim::port::What> = doc
        .orders
        .iter()
        .filter(|order| (4..=7).contains(&order.id))
        .map(|order| &order.what)
        .collect();
    let (projected, extra) =
        tacsim::port::project_cargo(&goods, &readings, &between).expect("the projection");
    let lane = load_pack(LANE_JSON).expect("the lane verifies");
    let sky = tacsim::Sky::from_pack(
        &lane,
        &tacsim::EphemerisConfig {
            epoch: rig.cfg.tacsim.time.ephemeris.epoch,
            rate: rig.cfg.tacsim.time.ephemeris.rate,
        },
    );
    let catalog = tacsim::definitions::Catalog::build_fielded(&[], tacsim::NS)
        .expect("the fielded content binds");
    let spec = catalog.specs.get(&cid(DRAY)).expect("the Dray is fielded");
    let logistics = spec.logistics.as_ref().expect("the Dray's logistics");
    let drive = tacsim::port::HullDrive {
        class: logistics
            .drive
            .as_ref()
            .expect("the Dray's drive")
            .class
            .clone(),
        law: *catalog
            .drive_for_class(&logistics.drive.as_ref().expect("the Dray's drive").class)
            .expect("the Dray's class flies a law"),
        payload_ceiling_kg: logistics.payload_ceiling_kg,
        throttle_permille: tacsim::port::DEFAULT_THROTTLE_PERMILLE,
    };
    let dry_kg = u64::from(logistics.dry_mass_t) * 1_000;
    let routes = port_def("stickney_port").routes[&cid("ceres")].clone();
    let depart_ic = ic_now(tw);
    let fill = tacsim::port::fill_targets(
        &goods,
        &readings,
        &projected,
        extra,
        tank_docked,
        locker_docked,
        1,
        logistics.occupant_allowance_kg,
        |payload| {
            tacsim::port::choose_route(
                &sky,
                &lane,
                &routes,
                &drive,
                dry_kg,
                payload,
                1,
                depart_ic,
                tacsim::port::RoutePolicy::default(),
                None,
            )
        },
    )
    .expect("the fill's fixed point");
    let fuel = fill.tank - tank_docked;
    let food = fill.locker - locker_docked;
    let fill_tank = BOUGHT_FILL_TANK
        .replace("{qty}", &fuel.to_string())
        .replace("{label}", &label("propellant"))
        .replace("{price}", &cr(fuel_price))
        .replace(
            "{total}",
            &cr(fuel_price * i64::try_from(fuel).expect("qty")),
        )
        .replace("{after}", &fill.tank.to_string())
        .replace(
            "{limit}",
            &single_cap(&goods, "propellant", "propellant").to_string(),
        )
        .replace("{cargo}", &fill.cargo_t.to_string());
    let fill_locker = BOUGHT_FILL_LOCKER
        .replace("{qty}", &food.to_string())
        .replace(
            "{total}",
            &cr(food_price * i64::try_from(food).expect("qty")),
        );

    // Wren's funds when the cargo buys ran: the stake less Ceres, plus both nets,
    // less the fill.
    let nets: i64 = sales_of(tw, wren)
        .iter()
        .map(|q| {
            settlement(tw, q.contract.expect("a contract"))
                .get(&me)
                .copied()
                .unwrap_or(0)
        })
        .sum();
    let after_fill = start + nets
        - fuel_price * i64::try_from(fuel).expect("qty")
        - food_price * i64::try_from(food).expect("qty");

    // The fittings: 49 t, nothing biting.
    let cargo = cargo_compartment();
    let parts = 49_u64;
    let (parts_room, _) = tacsim::port::capacity::max_more(
        &goods,
        &cargo,
        &BTreeMap::new(),
        &cid("industrial_parts"),
    )
    .expect("the room");
    assert!(
        parts <= parts_room
            && Some(parts) <= listed_qty(tw, parts_listing)
            && parts_price * 49 <= after_fill,
        "the fittings buy is not clamped"
    );
    let parts_line = BOUGHT
        .replace("{qty}", &parts.to_string())
        .replace("{unit}", "t")
        .replace("{label}", &label("industrial_parts"))
        .replace("{price}", &cr(parts_price))
        .replace("{total}", &cr(parts_price * 49));

    // The alloys: clamped by funds alone.
    let left = after_fill - parts_price * 49;
    let funds_units = u64::try_from(left / alloy_price).expect("non-negative");
    let fitted: BTreeMap<CanonicalId, u64> = [(cid("industrial_parts"), parts)].into();
    let (alloy_room, _) =
        tacsim::port::capacity::max_more(&goods, &cargo, &fitted, &cid("structural_alloys"))
            .expect("the room");
    let alloy_stock = listed_qty(tw, alloy_listing).unwrap_or(0) + funds_units;
    assert!(
        funds_units < 55 && funds_units < alloy_room && funds_units < alloy_stock,
        "funds bind alone: {funds_units} t affordable, room {alloy_room} t, stock \
         {alloy_stock} t"
    );
    let alloys_line = BOUGHT_SHORT
        .replace("{qty}", &funds_units.to_string())
        .replace("{unit}", "t")
        .replace("{label}", &label("structural_alloys"))
        .replace("{price}", &cr(alloy_price))
        .replace(
            "{total}",
            &cr(alloy_price * i64::try_from(funds_units).expect("qty")),
        )
        .replace("{want}", "55")
        .replace("{bounds}", "funds");
    StickneyBuys {
        fuel,
        food,
        parts,
        alloys: funds_units,
        tank_target: fill.tank,
        lines: vec![fill_tank, fill_locker, parts_line, alloys_line],
    }
}

// ---------------------------------------------------------------------------
// A rate change mid-passage (§6; IC9 T9)
// ---------------------------------------------------------------------------

/// A unix second's UTC wall time, `HH:MM` (the renders' form).
fn hhmm(unix: i64) -> String {
    let of_day = unix.rem_euclid(86_400);
    format!("{:02}:{:02}", of_day / 3_600, (of_day % 3_600) / 60)
}

/// The real second IC instant `target` falls at, on the live generation (the
/// renders' `real_until`: ceiling).
fn real_at(tw: &TestWorld, target: i64) -> i64 {
    let rate = i64::from(tw.world().world_timeline().current().rate).max(1);
    let ic = target.saturating_sub(ic_now(tw)).max(0);
    tw.world().now() + (ic + rate - 1) / rate
}

/// The IC civil date a queued departure due at `due_utc` is labelled with
/// (`when.rs::plan_instant`: never in the past, through the live generation).
fn due_label(tw: &TestWorld, due_utc: i64) -> String {
    let at = ic_now(tw).max(tw.world().world_timeline().current().ic_at(due_utc));
    format!("IC {}", mush_core::CivilDate::from_ic_seconds(at))
}

/// **A rate change mid-passage relabels the arrival and the queue's IC stamps** (doc
/// 18 §6; IC9 T9: `leg_s` frozen across a generation; D-18-46). A pilot's queued
/// depart flies; a later `daily HH:MM` depart waits behind it. Mid-passage GOD sets
/// the rate to 1800: the journey's arrival instant in IC (its frozen leg seconds) is
/// unchanged; the real-time ETA on the status line and on `+ship` is re-derived and
/// the time left doubles; the queued depart's `Next due:` keeps its UTC hour and its
/// IC label is re-derived through the new generation (never stored); and the hull
/// docks at the frozen IC instant.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn ic12_a_rate_change_mid_passage_relabels_the_arrival() {
    let mut rig = boot("rate").await;
    let _ = sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let kit = pilot(&mut rig, "Kit", "kitacct").await;
    let row = enrol(&mut rig, kit, "Salt Moth").await;
    let hull = ObjId(row.hull);
    let _ = outfit(&mut rig, kit).await;
    board_from_chandlery(&mut rig, kit, "Salt Moth").await;
    set_rate(&mut rig, 3_600).await;
    // A daily hour two real hours on (to the minute), the next occurrence of it.
    let now = rig.tw.world().now();
    let daily = (now + 7_200).div_euclid(60) * 60;
    let out = run(&mut rig, kit, "+ship/order depart stickney at=now").await;
    assert!(out.contains("Order 1 filed"), "{out}");
    let out = run(
        &mut rig,
        kit,
        &format!("+ship/order depart ceres at=daily {}", hhmm(daily)),
    )
    .await;
    assert!(out.contains("Order 2 filed"), "{out}");
    for _ in 0..12 {
        if under_way(&rig.tw, hull) {
            break;
        }
        let _ = fire_quietly(&mut rig).await;
    }
    assert!(under_way(&rig.tw, hull), "order 1 flew");
    let flown = ticket(&mut rig.tw, hull, 1).expect("the ticket");
    let arrival = flown.plan.depart_ic + flown.plan.total_s;
    let boundary = flown.plan.depart_ic + i64::from(flown.plan.legs[0].secs);
    // Mid-passage: past the Kirkwood boundary.
    let _ = to_ic(&mut rig, boundary + 3_600).await;
    assert!(under_way(&rig.tw, hull));
    let before = journey(&rig.tw, hull).expect("the journey");
    let eta_3600 = real_at(&rig.tw, arrival);
    let look = run(&mut rig, kit, "look").await;
    assert!(
        look.contains(&format!("ETA {} UTC.", hhmm(eta_3600))),
        "{look}\nwanted ETA {}",
        hhmm(eta_3600)
    );
    let ship = run(&mut rig, kit, "+ship").await;
    assert!(
        ship.contains(&format!("about {} UTC", hhmm(eta_3600))),
        "{ship}"
    );
    let orders_3600 = run(&mut rig, kit, "+ship/orders").await;
    let label_3600 = due_label(&rig.tw, daily);
    assert!(
        orders_3600.contains(&format!("{} UTC", hhmm(daily))) && orders_3600.contains(&label_3600),
        "{orders_3600}\nwanted {label_3600}"
    );

    // The rate change.
    let left_3600 = eta_3600 - rig.tw.world().now();
    set_rate(&mut rig, 1_800).await;
    let after = journey(&rig.tw, hull).expect("the journey");
    assert_eq!(
        after.departed().at,
        before.departed().at,
        "the departure instant is frozen"
    );
    let frozen: Vec<u32> = flown.plan.legs.iter().map(|leg| leg.secs).collect();
    assert_eq!(
        before.leg_times(),
        Some(frozen.as_slice()),
        "the journey flies the ticket's frozen leg seconds"
    );
    assert_eq!(
        after.leg_times(),
        before.leg_times(),
        "the frozen leg seconds survive the generation"
    );
    assert_eq!(
        (before.arrival_at(), after.arrival_at()),
        (Some(arrival), Some(arrival)),
        "the arrival instant in IC is the ticket's, unchanged"
    );
    let eta_1800 = real_at(&rig.tw, arrival);
    let left_1800 = eta_1800 - rig.tw.world().now();
    assert!(
        (left_1800 - 2 * left_3600).abs() <= 1,
        "the time left doubles: {left_3600} s at 3600, {left_1800} s at 1800"
    );
    let look = run(&mut rig, kit, "look").await;
    assert!(
        look.contains(&format!("ETA {} UTC.", hhmm(eta_1800))),
        "{look}\nwanted ETA {}",
        hhmm(eta_1800)
    );
    let ship = run(&mut rig, kit, "+ship").await;
    assert!(
        ship.contains(&format!("about {} UTC", hhmm(eta_1800))),
        "{ship}"
    );
    let orders_1800 = run(&mut rig, kit, "+ship/orders").await;
    let label_1800 = due_label(&rig.tw, daily);
    assert_ne!(label_3600, label_1800, "the IC label is re-derived");
    assert!(
        orders_1800.contains(&format!("{} UTC", hhmm(daily))) && orders_1800.contains(&label_1800),
        "{orders_1800}\nwanted {label_1800}"
    );
    assert!(
        !orders_1800.contains(&label_3600),
        "no stored IC stamp: {orders_1800}"
    );

    // The hull docks at the frozen IC instant: under way a fire before it, docked at it.
    let _ = to_ic(&mut rig, arrival - 7 * 1_800).await;
    assert!(
        under_way(&rig.tw, hull),
        "under way a fire before the frozen instant"
    );
    let _ = to_ic(&mut rig, arrival).await;
    assert!(!under_way(&rig.tw, hull), "docked at the frozen instant");
    assert!(ic_now(&rig.tw) >= arrival);
    assert_no_pages(&rig.tw);
}

// Relay gate 3: the same real trade/departure path, with radio in both directions.
fn radio_envelope(rig: &Rig, id: u64) -> mush_propagation::Envelope {
    rig.tw
        .world()
        .module_state::<mush_propagation::PropagationStore>()
        .unwrap()
        .envelope(id)
        .unwrap()
        .clone()
}

async fn radio_issue(rig: &mut Rig, pilot: ObjId, body: &str) -> u64 {
    let out = run(rig, GOD, &format!("+order {pilot}={body}")).await;
    assert!(out.contains("Queued"), "{out}");
    rig.tw
        .world()
        .module_state::<mush_propagation::PropagationStore>()
        .unwrap()
        .envelopes()
        .last()
        .unwrap()
        .id()
}

async fn radio_pass(rig: &mut Rig) {
    let _ = run(rig, GOD, "@prop/pass").await;
}

async fn radio_arrive(rig: &mut Rig, id: u64) {
    for _ in 0..16 {
        radio_pass(rig).await;
        let e = radio_envelope(rig, id);
        if e.state() == mush_propagation::EnvelopeState::Delivered {
            return;
        }
        if let Some(f) = e.transit().and_then(|r| r.flight.as_ref()) {
            to_ic(rig, f.due.at).await;
        } else {
            to_ic(rig, ic_now(&rig.tw) + 1).await;
        }
    }
    panic!("radio did not arrive: {:?}", radio_envelope(rig, id));
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn r5_real_dray_radio_freezes_forwards_and_recovers() {
    use mush_propagation::EnvelopeState;
    let mut rig = boot("relay-r5").await;
    sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let pilot = pilot(&mut rig, "RadioPilot", "radioacct").await;
    let row = enrol(&mut rig, pilot, "Radio Dray").await;
    let hull = ObjId(row.hull);
    outfit(&mut rig, pilot).await;
    let luna = rig
        .tw
        .world()
        .module_state::<mush_sim::relay::RelayStore>()
        .unwrap()
        .registry()
        .unwrap()
        .nodes[&cid("malapert")]
        .place;
    run(&mut rig, GOD, &format!("@teleport me={luna}")).await;
    run(&mut rig, GOD, "@relay/disable luna_ceres.luna").await;
    let boarding = radio_issue(&mut rig, pilot, "board-and-forward").await;
    radio_pass(&mut rig).await;
    board_from_chandlery(&mut rig, pilot, "Radio Dray").await;
    radio_arrive(&mut rig, boarding).await;
    assert_eq!(
        radio_envelope(&rig, boarding)
            .transit()
            .unwrap()
            .forwardings,
        1
    );
    assert!(
        run(&mut rig, pilot, "+ship")
            .await
            .contains("conventional radio remains available")
    );
    run(&mut rig, GOD, "@relay/enable luna_ceres.luna").await;
    let ceres = station_room(&rig.tw, "ceres_port", "room.c01_berth_gallery");
    run(&mut rig, GOD, &format!("@teleport me={ceres}")).await;
    set_rate(&mut rig, 3_600).await;
    let out = run(&mut rig, pilot, "+ship/undock stickney").await;
    assert!(out.contains("Clearance filed"), "{out}");
    for _ in 0..16 {
        fire_quietly(&mut rig).await;
        if under_way(&rig.tw, hull) {
            break;
        }
    }
    assert!(under_way(&rig.tw, hull));
    let flown = ticket(&mut rig.tw, hull, 1).unwrap();
    let leg = &flown.plan.legs[0];
    set_rate(&mut rig, 1).await;
    // The actual departure has a resolvable endpoint, and a positive radio leg.
    let departure = radio_issue(&mut rig, pilot, "departure-radio").await;
    radio_arrive(&mut rig, departure).await;
    assert_eq!(
        radio_envelope(&rig, departure)
            .transit()
            .unwrap()
            .destination,
        hull
    );

    to_ic(&mut rig, leg.t0 + i64::from(leg.secs) / 4).await;
    let id = radio_issue(&mut rig, pilot, "mid-leg-private").await;
    radio_pass(&mut rig).await;
    let frozen = radio_envelope(&rig, id)
        .transit()
        .unwrap()
        .flight
        .clone()
        .unwrap();
    assert!(frozen.seconds > 0 && frozen.basis.starts_with("ship.v1:"));
    let elapsed = u64::try_from(frozen.started.at - leg.t0).unwrap();
    let km = (u128::from(leg.dist_km) * u128::from(elapsed)).div_ceil(u128::from(leg.secs));
    let expected = (km * 1_000)
        .div_ceil(u128::from(tacsim::sky::LIGHT_M_PER_S))
        .max(1);
    assert_eq!(u128::try_from(frozen.seconds).unwrap(), expected);
    assert!(
        !run(&mut rig, pilot, "+orders")
            .await
            .contains("mid-leg-private")
    );
    let status = run(&mut rig, pilot, "+ship").await;
    assert!(
        status.contains("IC seconds one way") && status.contains("returning acknowledgements"),
        "{status}"
    );
    restart(&mut rig, |tw| ticket(tw, hull, 1).as_ref() == Some(&flown)).await;
    assert_eq!(
        radio_envelope(&rig, id).transit().unwrap().flight.as_ref(),
        Some(&frozen)
    );
    // Walking within the moving hull is not a new receiving endpoint.
    walk(&mut rig, pilot, "in", room_of(&row, "passage")).await;
    set_rate(&mut rig, 2).await;
    radio_pass(&mut rig).await;
    let reanchored = radio_envelope(&rig, id)
        .transit()
        .unwrap()
        .flight
        .clone()
        .unwrap();
    assert_eq!(
        (reanchored.started.at, reanchored.due.at, reanchored.basis),
        (frozen.started.at, frozen.due.at, frozen.basis)
    );
    to_ic(&mut rig, frozen.due.at - 2).await;
    radio_pass(&mut rig).await;
    assert_eq!(radio_envelope(&rig, id).state(), EnvelopeState::Transmitted);
    radio_arrive(&mut rig, id).await;
    assert_eq!(radio_envelope(&rig, id).transit().unwrap().forwardings, 0);
    assert!(
        run(&mut rig, pilot, "+orders")
            .await
            .contains("mid-leg-private")
    );
    let out = run(&mut rig, pilot, &format!("+orders/ack {id}")).await;
    assert!(out.contains("Acknowledgement queued"), "{out}");
    let ack = rig
        .tw
        .world()
        .module_state::<mush_propagation::PropagationStore>()
        .unwrap()
        .envelopes()
        .find(|e| e.ack_of() == Some(id))
        .unwrap()
        .id();
    radio_pass(&mut rig).await;
    assert_eq!(
        radio_envelope(&rig, id).state(),
        EnvelopeState::Delivered,
        "no early returning ACK"
    );
    assert!(
        radio_envelope(&rig, ack)
            .transit()
            .unwrap()
            .flight
            .as_ref()
            .unwrap()
            .seconds
            > 0
    );
    radio_arrive(&mut rig, ack).await;
    assert_eq!(
        radio_envelope(&rig, id).state(),
        EnvelopeState::Acknowledged
    );

    // The second half of a leg uses its OTHER endpoint, with deterministic
    // distance from the frozen plan, not a new ship position in the sky.
    to_ic(&mut rig, leg.t0 + i64::from(leg.secs) * 3 / 4).await;
    run(&mut rig, GOD, "@relay/disable luna_ceres.luna").await;
    let far = radio_issue(&mut rig, pilot, "nearer-end-switched").await;
    radio_pass(&mut rig).await;
    let far_flight = radio_envelope(&rig, far)
        .transit()
        .unwrap()
        .flight
        .clone()
        .unwrap();
    let lane = store(&rig.tw)
        .pack(&flown.route_pack, flown.route_version)
        .unwrap();
    let sky = tacsim::sky::Sky::from_pack(&lane.pack, &store(&rig.tw).config().time.ephemeris);
    let hop = sky
        .distance_km(
            &cid("ceres"),
            far_flight.started.at,
            &leg.body,
            far_flight.started.at,
        )
        .unwrap();
    let remaining = u64::try_from(leg.t1 - far_flight.started.at).unwrap();
    let offset = (u128::from(leg.dist_km) * u128::from(remaining)).div_ceil(u128::from(leg.secs));
    let expected =
        ((u128::from(hop) + offset) * 1000).div_ceil(u128::from(tacsim::sky::LIGHT_M_PER_S));
    assert_eq!(u128::try_from(far_flight.seconds).unwrap(), expected);
    radio_arrive(&mut rig, far).await;
    run(&mut rig, GOD, "@relay/enable luna_ceres.luna").await;

    // R6a: the same real underway hull receives a letter only after its physical
    // flight, and sender knowledge waits for a returning receipt.
    let sent = run(
        &mut rig,
        GOD,
        &format!("@mail/quick {pilot}=Underway letter/private cargo note"),
    )
    .await;
    assert!(sent.contains("queued for IC transit"), "{sent}");
    let mail_id = rig
        .tw
        .world()
        .module_state::<mush_mail::MailState>()
        .unwrap()
        .next_id()
        - 1;
    assert!(
        rig.tw
            .world()
            .module_state::<mush_mail::MailState>()
            .unwrap()
            .entry(pilot, mail_id)
            .is_none()
    );
    sweep(&mut rig, 1).await;
    let key = format!("mail:v1:{mail_id}:{}:out", pilot.0);
    let mailed = mush_propagation::dispatch::find(rig.tw.world(), &key)
        .unwrap()
        .clone();
    let flight = mailed.transit().unwrap().flight.clone().unwrap();
    assert!(flight.seconds > 0 && flight.basis.starts_with("ship.v1:"));
    assert!(
        !run(&mut rig, pilot, "@mail")
            .await
            .contains("Underway letter")
    );
    restart(&mut rig, |tw| ticket(tw, hull, 1).is_some()).await;
    assert_eq!(
        mush_propagation::dispatch::find(rig.tw.world(), &key)
            .unwrap()
            .transit()
            .unwrap()
            .flight
            .as_ref(),
        Some(&flight)
    );
    radio_arrive(&mut rig, mailed.id()).await;
    sweep(&mut rig, 1).await;
    assert!(
        rig.tw
            .world()
            .module_state::<mush_mail::MailState>()
            .unwrap()
            .entry(pilot, mail_id)
            .is_some()
    );
    assert!(
        rig.tw
            .world()
            .module_state::<mush_mail::MailState>()
            .unwrap()
            .message(mail_id)
            .unwrap()
            .delivery
            .as_ref()
            .unwrap()
            .is_empty()
    );
    sweep(&mut rig, 1).await;
    let returning = mush_propagation::dispatch::find(
        rig.tw.world(),
        &format!("mail:v1:{mail_id}:{}:return", pilot.0),
    )
    .unwrap()
    .id();
    radio_arrive(&mut rig, returning).await;
    sweep(&mut rig, 1).await;
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_mail::MailState>()
            .unwrap()
            .message(mail_id)
            .unwrap()
            .delivery
            .as_ref()
            .unwrap()[&pilot],
        mush_mail::transit::Outcome::Delivered
    );

    // Missing typed ticket is a bounded wait, including a ship-local room.
    let handles = handles(&rig.tw);
    let mut txn = rig
        .tw
        .engine_mut()
        .action_transaction(&handles.families.tickets)
        .unwrap();
    txn.remove(&handles.families.tickets, &TicketId { hull, seq: 1 })
        .unwrap();
    let (_, install) = txn.prepare().unwrap().into_parts();
    rig.tw
        .engine_mut()
        .install_prepared_action(install)
        .unwrap();
    let missing = radio_issue(&mut rig, pilot, "missing-ticket").await;
    radio_pass(&mut rig).await;
    assert!(radio_envelope(&rig, missing).transit().is_none());
    assert!(
        run(&mut rig, pilot, "+ship")
            .await
            .contains("physical lag unavailable")
    );
    let mut txn = rig
        .tw
        .engine_mut()
        .action_transaction(&handles.families.tickets)
        .unwrap();
    txn.insert(
        &handles.families.tickets,
        &TicketId { hull, seq: 1 },
        flown.clone(),
    )
    .unwrap();
    let (_, install) = txn.prepare().unwrap().into_parts();
    rig.tw
        .engine_mut()
        .install_prepared_action(install)
        .unwrap();
    radio_arrive(&mut rig, missing).await;

    // Cross a real journey boundary; then freeze a radio leg spanning docking.
    to_ic(&mut rig, leg.t1 + 12).await;
    assert!(journey(&rig.tw, hull).unwrap().cursor() >= 1);
    let boundary = radio_issue(&mut rig, pilot, "boundary-radio").await;
    radio_arrive(&mut rig, boundary).await;
    let arrival = flown.plan.depart_ic + flown.plan.total_s;
    to_ic(&mut rig, arrival - 10).await;
    let landing = radio_issue(&mut rig, pilot, "dock-and-leave").await;
    radio_pass(&mut rig).await;
    to_ic(&mut rig, arrival + 12).await;
    assert!(!under_way(&rig.tw, hull));
    walk(&mut rig, pilot, "airlock", room_of(&row, "airlock")).await;
    let out = run(&mut rig, pilot, "+ship/leave").await;
    assert!(!out.contains("under way"), "{out}");
    assert_ne!(location(&rig.tw, pilot), Some(room_of(&row, "airlock")));
    radio_arrive(&mut rig, landing).await;
    assert_eq!(
        radio_envelope(&rig, landing).transit().unwrap().forwardings,
        1
    );
    assert_no_pages(&rig.tw);
}

// ---------------------------------------------------------------------------
// Negative checks (§6)
// ---------------------------------------------------------------------------

/// What must not move under a refusal: the pilot's credit — posted and available
/// (an escrow hold moves only the second) — the hold's pools, the pilot's location,
/// every lot listing's quantity, the number of quotes and contracts, and the hull's
/// tickets.
#[derive(Debug, PartialEq, Eq)]
struct Still {
    balance: i64,
    available: i64,
    pools: Vec<u64>,
    at: Option<ObjId>,
    listings: BTreeMap<u64, u64>,
    quotes: usize,
    contracts: usize,
    tickets: usize,
}

fn still(rig: &mut Rig, pilot: ObjId, hold: VaultId, hull: ObjId) -> Still {
    let goods = goods();
    let pools = goods
        .goods
        .iter()
        .map(|g| pool(&mut rig.tw, hold, g.id.as_str()))
        .collect();
    let account = credit_account(&rig.tw, &PrincipalId::Object(pilot));
    Still {
        balance: posted(&rig.tw, account),
        available: available(&rig.tw, account),
        pools,
        at: location(&rig.tw, pilot),
        listings: listing_quantities(&rig.tw),
        quotes: market(&rig.tw).quotes().count(),
        contracts: economy(&rig.tw).snapshot().contracts().count(),
        tickets: tickets(&mut rig.tw, hull).len(),
    }
}

/// Type `line` as `who`, assert the reply carries `want`, nothing was queued, and
/// nothing in [`Still`] moved.
async fn refused(
    rig: &mut Rig,
    who: ObjId,
    line: &str,
    want: &str,
    snapshot: &dyn Fn(&mut Rig) -> Still,
) {
    let before = snapshot(rig);
    let out = run(rig, who, line).await;
    assert!(out.contains(want), "`{line}`: wanted `{want}`:\n{out}");
    assert!(drain(&mut rig.tw).is_empty(), "`{line}` queued nothing");
    assert_eq!(snapshot(rig), before, "`{line}` moved nothing");
}

/// The line `capacity_line` renders for `qty` of `good` into `contents` (the shipped
/// renderer over the shipped admission; doc 18 §4.3).
fn capacity_refusal(contents: &[(&str, u64)], good: &str, qty: u64) -> String {
    let goods = goods();
    let cargo = cargo_compartment();
    let held: BTreeMap<CanonicalId, u64> = contents.iter().map(|(g, q)| (cid(g), *q)).collect();
    let tacsim::port::capacity::Admission::Refused(refusal) =
        tacsim::port::capacity::admits(&goods, &cargo, &held, (&cid(good), qty))
    else {
        panic!("{qty} {good} over {contents:?} is refused");
    };
    let mut after = held.clone();
    *after.entry(cid(good)).or_insert(0) += qty;
    let load = tacsim::port::capacity::load(&goods, &cargo, &after).expect("the load");
    tacsim::port::capacity_line(&goods, &cargo, &cid(good), qty, &refusal, &load)
}

/// `+market/quote` then `+market/accept` as `who`, the accept asserted refused with
/// `want` and nothing moved (no contract on the quote, nothing queued).
async fn accept_refused(rig: &mut Rig, who: ObjId, listing: ListingId, qty: u64, want: &str) {
    let quoted = run(rig, who, &format!("+market/quote {listing}={qty}")).await;
    if !quoted.contains("Quote #") {
        assert!(
            quoted.contains(want),
            "the quote: wanted `{want}`:\n{quoted}"
        );
        assert!(drain(&mut rig.tw).is_empty());
        return;
    }
    let quote = newest_quote(&rig.tw, who);
    let account = credit_account(&rig.tw, &PrincipalId::Object(who));
    let (balance, free) = (posted(&rig.tw, account), available(&rig.tw, account));
    let accepted = run(rig, who, &format!("+market/accept {quote}")).await;
    assert!(accepted.contains(want), "wanted `{want}`:\n{accepted}");
    assert!(
        market(&rig.tw)
            .quote(quote)
            .is_some_and(|q| q.contract.is_none()),
        "no contract"
    );
    assert_eq!(
        (posted(&rig.tw, account), available(&rig.tw, account)),
        (balance, free),
        "nothing charged and nothing held in escrow"
    );
    assert!(drain(&mut rig.tw).is_empty(), "nothing staged");
}

/// Mint `minor` into `who`'s credit account from the treasury (a set-up act, so the
/// capacity checks are the refusals under test and never the funds).
async fn top_up(rig: &mut Rig, who: ObjId, minor: i64) {
    let account = credit_account(&rig.tw, &PrincipalId::Object(who));
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let out = run(
        rig,
        GOD,
        &format!("@econ/mint {treasury}={} {minor}", account.0),
    )
    .await;
    assert!(!out.contains("denied"), "{out}");
}

/// From the chandlery: reaction mass and provisions bought until the planner's burn
/// and crew-days for `dest` are aboard (IC10's loop).
async fn bunker(rig: &mut Rig, who: ObjId, dest: &str, fuel: ListingId, food: ListingId) {
    for _ in 0..6 {
        let plan = run(rig, who, &format!("+ship/plan {dest}")).await;
        let (burn, tank) = plan_numbers(&plan, "Burn ", "(tank ").expect(&plan);
        let (days, locker) = plan_numbers(&plan, "Stores ", "(locker ").expect(&plan);
        if tank >= burn && locker >= days {
            return;
        }
        if tank < burn {
            let _ = buy(rig, who, fuel, burn - tank).await;
        }
        if locker < days {
            let _ = buy(rig, who, food, days - locker).await;
        }
    }
    panic!("the planner's burn and stores did not converge");
}

/// `@inv/move <hold>/metals:1=<store>` as `who`: queued, then refused at the
/// coordinator before any intent; the refusal's detail.
async fn inv_move_out(rig: &mut Rig, who: ObjId, hold: VaultId, store: u64) -> String {
    let out = run(rig, who, &format!("@inv/move {}/metals:1={store}", hold.0)).await;
    let queued = drain(&mut rig.tw);
    assert!(out.contains("Queued."), "{out}");
    assert_eq!(queued.len(), 1, "{out}");
    match land(rig, &queued[0]).await {
        Landing::Refused(detail) => {
            assert_eq!(detail, "offered action parameters are invalid");
            detail
        }
        Landing::Replay | Landing::Landed => panic!("the move out of the hold landed"),
    }
}

/// Walk `who` through `steps` of `(word, port entry, room local)`.
async fn walk_path(rig: &mut Rig, who: ObjId, steps: &[(&str, &str, &str)]) {
    for (word, entry, local) in steps {
        let to = station_room(&rig.tw, entry, local);
        walk(rig, who, word, to).await;
    }
}

const C: &str = "ceres_port";
const P: &str = "stickney_port";

/// **§6's negative checks, each refused in fiction with nothing moved** (doc 18 §6
/// "Negative checks"; cut IC12 item 1.7). The enrolment checks come before the rate
/// switch, so the refusal under test is the one asserted and never the Rate line; a
/// gate whose player-facing answer is generic asserts the generic answer, and reads
/// the gate's own line through the staff view or the function.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn ic12_negative_checks_refuse_in_fiction_and_move_nothing() {
    let mut rig = boot("negative").await;
    let _ = sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let vex = pilot(&mut rig, "Vex", "vexacct").await;
    let row = enrol(&mut rig, vex, "Salt Moth").await;
    let hull = ObjId(row.hull);
    let hold = hold_of(&rig.tw, &row);
    let snap = move |rig: &mut Rig| still(rig, vex, hold, hull);

    // 1. A second enrolment from another character on the same gateway account.
    let vexa = alt(&mut rig, "Vexa", "vexacct", vex).await;
    walk_path(
        &mut rig,
        vexa,
        &[
            ("c", C, "room.c03_arrivals"),
            ("registry", C, "room.c05_registry"),
        ],
    )
    .await;
    refused(
        &mut rig,
        vexa,
        "+coop/enrol Long Haul",
        tacsim::port::coop::ENROLLED_ONCE,
        &snap,
    )
    .await;
    assert!(tacsim::port::live_charter(rig.tw.world(), vexa).is_none());

    // 3. Another player's `+ship/board` on the Moth.
    let ruth = pilot(&mut rig, "Ruth", "ruthacct").await;
    walk_path(
        &mut rig,
        ruth,
        &[
            ("c", C, "room.c03_arrivals"),
            ("lobby", C, "room.c02_pressure"),
            ("gallery", C, "room.c01_berth_gallery"),
        ],
    )
    .await;
    refused(
        &mut rig,
        ruth,
        "+ship/board",
        tacsim::port::coop::NO_CHARTER,
        &snap,
    )
    .await;

    // 2. After the one staff act: a would-be pilot hears only NOT_OPEN; the staff view
    // carries the flagged Rate line; the production gate names the Rate binding.
    set_rate(&mut rig, 3_600).await;
    let cass = pilot(&mut rig, "Cass", "cassacct").await;
    walk_path(
        &mut rig,
        cass,
        &[
            ("c", C, "room.c03_arrivals"),
            ("registry", C, "room.c05_registry"),
        ],
    )
    .await;
    let out = run(&mut rig, cass, "+coop/enrol Grey Heron").await;
    assert_eq!(
        out.trim(),
        tacsim::port::coop::NOT_OPEN,
        "the player sees only NOT_OPEN"
    );
    assert!(tacsim::port::live_charter(rig.tw.world(), cass).is_none());
    let report = run(&mut rig, GOD, "@tacsim/port status").await;
    assert!(
        report.contains("world rate: 3600 IC seconds per real second; play runs at 24. [flagged]"),
        "{report}"
    );
    let gate = tacsim::port::readiness(rig.tw.world());
    let line = tacsim::port::enrol_gate(&gate).expect_err("the gate is shut");
    assert_eq!(line.binding, tacsim::port::Binding::Rate);

    // 6 (Ceres). 86 t of ice (volume); 81 t of ice after 19 t of concentrate; ice into
    // the tank (the cargo full at both limits, the tank empty: ice stows only as bulk).
    top_up(&mut rig, vex, 10_000_000).await;
    walk_path(
        &mut rig,
        vex,
        &[
            ("arrivals", C, "room.c03_arrivals"),
            ("concourse", C, "room.c04_concourse"),
            ("exchange", C, "room.c07_exchange"),
        ],
    )
    .await;
    let ice = listing_of(&rig.tw, C, "occator", "volatiles");
    let metals = listing_of(&rig.tw, C, "freeport_exchange", "metals");
    let (m, v) = vertex_load();
    accept_refused(
        &mut rig,
        vex,
        ice,
        86,
        &capacity_refusal(&[], "volatiles", 86),
    )
    .await;
    let _ = buy(&mut rig, vex, metals, m - 1).await;
    accept_refused(
        &mut rig,
        vex,
        ice,
        v + 1,
        &capacity_refusal(&[("metals", m - 1)], "volatiles", v + 1),
    )
    .await;
    let _ = buy(&mut rig, vex, metals, 1).await;
    let _ = buy(&mut rig, vex, ice, v).await;
    assert_eq!(
        pool(&mut rig.tw, hold, "propellant"),
        0,
        "the tank is empty"
    );
    accept_refused(
        &mut rig,
        vex,
        ice,
        1,
        &capacity_refusal(&[("metals", m), ("volatiles", v)], "volatiles", 1),
    )
    .await;

    // 5 (Limbo). A quote taken at the chandlery, accepted from Limbo.
    walk_path(&mut rig, vex, &[("chandlery", C, "room.c08_chandlery")]).await;
    let fuel = listing_of(&rig.tw, C, "occator", "propellant");
    let food = listing_of(&rig.tw, C, "coop", "provisions");
    let quoted = run(&mut rig, vex, &format!("+market/quote {fuel}=1")).await;
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = newest_quote(&rig.tw, vex);
    walk_path(
        &mut rig,
        vex,
        &[
            ("exchange", C, "room.c07_exchange"),
            ("concourse", C, "room.c04_concourse"),
            ("arrivals", C, "room.c03_arrivals"),
        ],
    )
    .await;
    walk(&mut rig, vex, "x", LIMBO).await;
    refused(
        &mut rig,
        vex,
        &format!("+market/accept {quote}"),
        &tacsim::port::NOT_IN_PORT.replace("{port}", "Ceres Freeport"),
        &snap,
    )
    .await;
    walk_path(&mut rig, vex, &[("c", C, "room.c03_arrivals")]).await;

    // 7. The tank one tonne short of the planner's burn (lane v3: the vertex with 39 t
    // aboard burns 40 — the 105‰ law counts the fuel's own mass), and not aboard.
    walk_path(
        &mut rig,
        vex,
        &[
            ("concourse", C, "room.c04_concourse"),
            ("exchange", C, "room.c07_exchange"),
            ("chandlery", C, "room.c08_chandlery"),
        ],
    )
    .await;
    // One tonne short: 51 t aboard against the flip law's 52 t burn at that load (doc
    // 19 cut A.2; the coast law's case was 39 t against 40).
    let _ = buy(&mut rig, vex, fuel, 51).await;
    let plan = run(&mut rig, vex, "+ship/plan stickney").await;
    let (days, _) = plan_numbers(&plan, "Stores ", "(locker ").expect(&plan);
    let _ = buy(&mut rig, vex, food, days).await;
    let plan = run(&mut rig, vex, "+ship/plan stickney").await;
    let (burn, tank) = plan_numbers(&plan, "Burn ", "(tank ").expect(&plan);
    assert_eq!((burn, tank), (52, 51), "one tonne short: {plan}");
    let _ = sweep(&mut rig, 61).await;
    let _ = sweep(&mut rig, 61).await;
    walk_path(
        &mut rig,
        vex,
        &[
            ("exchange", C, "room.c07_exchange"),
            ("concourse", C, "room.c04_concourse"),
            ("arrivals", C, "room.c03_arrivals"),
            ("lobby", C, "room.c02_pressure"),
            ("gallery", C, "room.c01_berth_gallery"),
        ],
    )
    .await;
    refused(
        &mut rig,
        vex,
        "+ship/undock stickney",
        &tacsim::port::voyage::NOT_ABOARD.replace("{name}", "Salt Moth"),
        &snap,
    )
    .await;
    let boarded = run(&mut rig, vex, "+ship/board").await;
    assert!(
        boarded.contains("You cycle the lock into the Salt Moth."),
        "{boarded}"
    );
    refused(
        &mut rig,
        vex,
        "+ship/undock stickney",
        &tacsim::port::voyage::TANK_SHORT
            .replace("{port}", "Stickney Yards")
            .replace("{burn}", &burn.to_string())
            .replace("{tank}", &tank.to_string()),
        &snap,
    )
    .await;

    // 4. A third soul into the berths slot (two berths: the pilot, then a visitor).
    let airlock = room_of(&row, "airlock");
    let mut placed = Vec::new();
    for name in ["Visitor", "Third"] {
        let who = rig.tw.spawn_player(name);
        flush(&mut rig).await;
        let out = run(
            &mut rig,
            GOD,
            &format!("@behavior/add #{}=sim.participant", who.0),
        )
        .await;
        assert!(!out.contains("denied"), "{out}");
        let out = run(
            &mut rig,
            GOD,
            &format!("@spatial/place/interior #{}=#{},berths", who.0, airlock.0),
        )
        .await;
        placed.push((who, out));
    }
    let (visitor, out) = &placed[0];
    assert!(
        out.contains("Placed"),
        "the second berth takes the visitor: {out}"
    );
    // The placement gate's own refusal (`mush-sim/src/transition.rs:63`), said to
    // the staffer who tried.
    let (third, out) = &placed[1];
    assert!(
        out.contains("compartment `berths` is full (2 max)"),
        "the third soul is refused: {out}"
    );
    assert_ne!(location(&rig.tw, *third), Some(airlock), "nothing moved");
    let out = run(&mut rig, GOD, &format!("@spatial/unplace #{}", visitor.0)).await;
    assert!(out.contains("Unplaced Visitor."), "{out}");
    let _ = run(
        &mut rig,
        GOD,
        &format!("@teleport #{}=#{}", visitor.0, LIMBO.0),
    )
    .await;

    // 11. `o`, `s`, `home`, `@teleport` by an occupant aboard: the boundary holds.
    // The boundary's line (`mush-sim/src/lib.rs:333`) for the three exits; `@teleport`
    // is no player's verb, and its generic answer is what the occupant hears.
    let boundary = "Vex is aboard Salt Moth; it moves only between Salt Moth's rooms, or leaves \
                    through the simulation or staff recovery (@spatial).";
    for line in ["o", "s", "home"] {
        refused(&mut rig, vex, line, boundary, &snap).await;
    }
    refused(
        &mut rig,
        vex,
        &format!("@teleport me=#{}", LIMBO.0),
        "Permission denied.",
        &snap,
    )
    .await;
    assert_eq!(location(&rig.tw, vex), Some(airlock));

    // 14. The pilot's raw `@journey/depart` on the hull: the pilot holds no authority
    // over a Trust hull (the generic answer); staff read the lane's own refusal.
    refused(
        &mut rig,
        vex,
        &format!("@journey/depart #{}=ceres_stickney", hull.0),
        "Permission denied",
        &snap,
    )
    .await;
    let out = run(
        &mut rig,
        GOD,
        &format!("@journey/depart #{}=ceres_stickney", hull.0),
    )
    .await;
    assert!(
        out.contains(&tacsim::port::voyage::LANE_UNPLANNED.replace("{route}", "ceres_stickney")),
        "{out}"
    );
    assert!(journey(&rig.tw, hull).is_none());

    // The missing tonne, then 15 (IC9 T6): a held launch, a non-staff clear refused, a
    // staff clear standing it down, the pilot's `+ship/leave` landing in the gallery.
    let out = run(&mut rig, vex, "+ship/leave").await;
    assert_eq!(
        location(&rig.tw, vex),
        Some(station_room(&rig.tw, C, "room.c01_berth_gallery")),
        "{out}"
    );
    walk_path(
        &mut rig,
        vex,
        &[
            ("lobby", C, "room.c02_pressure"),
            ("arrivals", C, "room.c03_arrivals"),
            ("concourse", C, "room.c04_concourse"),
            ("exchange", C, "room.c07_exchange"),
            ("chandlery", C, "room.c08_chandlery"),
        ],
    )
    .await;
    // The tonne short, and the stores its mass now asks for (crew-days step with the
    // payload).
    bunker(&mut rig, vex, "stickney", fuel, food).await;
    let _ = sweep(&mut rig, 61).await;
    let _ = sweep(&mut rig, 61).await;
    let to_gallery: [(&str, &str, &str); 5] = [
        ("exchange", C, "room.c07_exchange"),
        ("concourse", C, "room.c04_concourse"),
        ("arrivals", C, "room.c03_arrivals"),
        ("lobby", C, "room.c02_pressure"),
        ("gallery", C, "room.c01_berth_gallery"),
    ];
    walk_path(&mut rig, vex, &to_gallery).await;
    let _ = run(&mut rig, vex, "+ship/board").await;
    let filed = run(&mut rig, vex, "+ship/undock stickney").await;
    assert!(filed.contains("Clearance filed"), "{filed}");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued).await;
    let stickney = port_map(&rig.tw, P).domain.expect("a domain");
    let _ = run(&mut rig, GOD, &format!("@domain/pause {stickney}")).await;
    let held = tacsim::port::voyage::LAUNCH_HELD
        .replace("{name}", "Salt Moth")
        .replace("{port}", "Stickney Yards");
    let mut told = String::new();
    for _ in 0..3 {
        let events = fire_quietly(&mut rig).await;
        told.push_str(&said(&rig.tw, &events, vex));
    }
    assert!(told.contains(&held), "the held launch is told: {told}");
    assert!(journey(&rig.tw, hull).is_none(), "no journey while held");
    refused(
        &mut rig,
        vex,
        &format!("@tacsim/port clear #{}", hull.0),
        "Permission denied.",
        &snap,
    )
    .await;
    let events = rig.tw.run(GOD, &format!("@tacsim/port clear #{}", hull.0));
    flush(&mut rig).await;
    assert!(
        said(&rig.tw, &events, vex).contains(
            &tacsim::port::voyage::CLEARANCE_STOOD_DOWN
                .replace("{name}", "Salt Moth")
                .replace("{port}", "Stickney Yards")
        ),
        "the pilot is told"
    );
    let _ = run(&mut rig, GOD, &format!("@domain/resume {stickney}")).await;
    let out = run(&mut rig, vex, "+ship/leave").await;
    assert_eq!(
        location(&rig.tw, vex),
        Some(station_room(&rig.tw, C, "room.c01_berth_gallery")),
        "the stood-down pilot leaves into the gallery: {out}"
    );

    // Bunker again (nothing burned is returned) and fly; 7 (under way, twice).
    walk_path(
        &mut rig,
        vex,
        &[
            ("lobby", C, "room.c02_pressure"),
            ("arrivals", C, "room.c03_arrivals"),
            ("concourse", C, "room.c04_concourse"),
            ("exchange", C, "room.c07_exchange"),
            ("chandlery", C, "room.c08_chandlery"),
        ],
    )
    .await;
    bunker(&mut rig, vex, "stickney", fuel, food).await;
    let _ = sweep(&mut rig, 61).await;
    let _ = sweep(&mut rig, 61).await;
    walk_path(&mut rig, vex, &to_gallery).await;
    let _ = run(&mut rig, vex, "+ship/board").await;
    // 10's docked counterpart: the pilot's own move out of the docked hold is
    // refused the same way (the endpoint rule, not the hull's berth).
    let exchange_store = port_map(&rig.tw, C).vendors[&cid("freeport_exchange")].vault;
    let before = snap(&mut rig);
    let docked_move = inv_move_out(&mut rig, vex, hold, exchange_store).await;
    assert_eq!(snap(&mut rig), before, "nothing left the docked hold");
    let filed = run(&mut rig, vex, "+ship/undock stickney").await;
    assert!(filed.contains("Clearance filed"), "{filed}");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued).await;
    let _ = fire_quietly(&mut rig).await;
    assert!(under_way(&rig.tw, hull), "launched");
    for _ in 0..2 {
        refused(
            &mut rig,
            vex,
            "+ship/undock stickney",
            &tacsim::port::voyage::UNDOCK_UNDER_WAY.replace("{name}", "Salt Moth"),
            &snap,
        )
        .await;
    }
    // 10. `@inv/move` out of the hold under way — as built, the ENDPOINT-AUTHORITY
    // refusal: the pilot acts on exactly one endpoint (the hull's own hold), so a
    // move between it and a storefront has nowhere to land, refused at the
    // coordinator before any intent (the player hears the world task's generic
    // refusal) whether the hull is docked (the counterpart before the launch, above)
    // or under way. No under-way rule of its own exists: a staff move out of a hold
    // under way lands (D-18-37's deferral to P-H; Q-18-62), so §6's check 10 as
    // written stays open there.
    let before = snap(&mut rig);
    let refused_move = inv_move_out(&mut rig, vex, hold, exchange_store).await;
    assert_eq!(
        refused_move, docked_move,
        "the same refusal docked and under way"
    );
    assert_eq!(snap(&mut rig), before, "nothing left the hold");

    // To Stickney.
    let flown = tickets(&mut rig.tw, hull)
        .last()
        .cloned()
        .expect("the ticket");
    let _ = to_ic(&mut rig, flown.plan.depart_ic + flown.plan.total_s).await;
    let _ = fire_quietly(&mut rig).await;
    assert!(!under_way(&rig.tw, hull), "docked at Stickney");
    for word in ["passage", "airlock"] {
        let to = room_of(&row, word);
        walk(&mut rig, vex, word, to).await;
    }
    let out = run(&mut rig, vex, "+ship/leave").await;
    assert_eq!(
        location(&rig.tw, vex),
        Some(station_room(&rig.tw, P, "room.p01_berth_gallery")),
        "{out}"
    );
    walk_path(
        &mut rig,
        vex,
        &[
            ("lobby", P, "room.p02_pressure"),
            ("customs", P, "room.p03_customs"),
            ("concourse", P, "room.p04_concourse"),
            ("exchange", P, "room.p08_exchange"),
            ("down", P, "room.p07_receiving"),
        ],
    )
    .await;
    let kasei = port_map(&rig.tw, P).vendors[&cid("kasei")].clone();
    let metals_bid = ListingId(kasei.bids[&cid("metals")].listing);
    let ice_bid = ListingId(kasei.bids[&cid("volatiles")].listing);

    // 13. Selling more than held.
    refused(
        &mut rig,
        vex,
        &format!("+market/sell {metals_bid}={}", m + 1),
        &mush_market::SELL_SHORT
            .replace("{held}", &m.to_string())
            .replace("{qty}", &(m + 1).to_string()),
        &snap,
    )
    .await;
    let sold = run(&mut rig, vex, &format!("+market/sell {metals_bid}={m}")).await;
    assert!(sold.contains(&format!("Sold {m} t")), "{sold}");
    let leg = drain(&mut rig.tw);
    land_all(&mut rig, &leg).await;

    // 9. An unfunded bid in the queue: Kasei's account drained by staff to 100.00cr —
    // under one tonne's price, not nothing (the house's units are floored) — the
    // queued sale skipped with `HOUSE_CANNOT_PAY_SKIP` (the planner's pre-check, never
    // the market's `UNFUNDED`), the ice kept; the interactive sale answers `UNFUNDED`.
    let kasei_account = port_map(&rig.tw, P).orgs[&cid("kasei")]
        .account
        .expect("Kasei's account");
    let treasury = port_map(&rig.tw, C).treasury.expect("a treasury");
    let funds = economy(&rig.tw)
        .snapshot()
        .account(mush_economy::AccountId(kasei_account))
        .and_then(mush_economy::Account::available)
        .map(mush_economy::Amount::minor)
        .expect("Kasei's available balance");
    let left = 10_000;
    assert!(
        funds > left && left < price_of(&rig.tw, ice_bid),
        "{funds} available; the ice bid {}",
        price_of(&rig.tw, ice_bid)
    );
    let drained = funds - left;
    let out = run(
        &mut rig,
        GOD,
        &format!("@econ/pay #{kasei_account}=#{treasury} {drained}"),
    )
    .await;
    assert!(out.contains("Paid: transaction #"), "{out}");
    assert_eq!(
        available(&rig.tw, mush_economy::AccountId(kasei_account)),
        left,
        "the house is left under one tonne"
    );
    let out = run(&mut rig, vex, "+ship/order sell volatiles all").await;
    assert!(out.contains("Order 1 filed"), "{out}");
    assert_eq!(pool(&mut rig.tw, hold, "volatiles"), v);
    for _ in 0..3 {
        let _ = fire_quietly(&mut rig).await;
    }
    let skip = tacsim::port::executor::SKIP_LINE
        .replace("{n}", "1")
        .replace(
            "{reason}",
            &tacsim::port::executor::HOUSE_CANNOT_PAY_SKIP
                .replace("{house}", "Kasei Metals")
                .replace("{label}", "water ice"),
        );
    let log = log_texts(&rig.tw, hull);
    assert!(log.contains(&skip), "which refusal fired: {log:#?}");
    assert!(
        !log.iter().any(|l| l.contains(mush_market::UNFUNDED)),
        "{log:#?}"
    );
    assert_eq!(order_state(&rig.tw, hull, 1), Some(OrderState::Skipped));
    assert_eq!(pool(&mut rig.tw, hold, "volatiles"), v, "the cargo is kept");
    refused(
        &mut rig,
        vex,
        &format!("+market/sell {ice_bid}={v}"),
        mush_market::UNFUNDED,
        &snap,
    )
    .await;
    let out = run(
        &mut rig,
        GOD,
        &format!("@econ/pay #{treasury}=#{kasei_account} {drained}"),
    )
    .await;
    assert!(out.contains("Paid: transaction #"), "{out}");
    let sold = run(&mut rig, vex, &format!("+market/sell {ice_bid}={v}")).await;
    assert!(sold.contains(&format!("Sold {v} t")), "{sold}");
    let leg = drain(&mut rig.tw);
    land_all(&mut rig, &leg).await;
    for _ in 0..12 {
        let _ = sweep(&mut rig, 31).await;
    }

    // 6 (Stickney). 101 t of alloy stock (mass), the cargo hold empty.
    walk_path(&mut rig, vex, &[("exchange", P, "room.p08_exchange")]).await;
    let alloys = listing_of(&rig.tw, P, "kasei", "structural_alloys");
    accept_refused(
        &mut rig,
        vex,
        alloys,
        101,
        &capacity_refusal(&[], "structural_alloys", 101),
    )
    .await;

    // 5 (the other port). The pilot at the Ceres Exchange (staff carry them there: no
    // player reaches Ceres without the hull) while the hull is berthed at Stickney.
    let c07 = station_room(&rig.tw, C, "room.c07_exchange");
    let _ = run(&mut rig, GOD, &format!("@teleport #{}=#{}", vex.0, c07.0)).await;
    accept_refused(
        &mut rig,
        vex,
        metals,
        1,
        &tacsim::port::HOLD_ELSEWHERE
            .replace("{name}", "Salt Moth")
            .replace("{other}", "Stickney Yards")
            .replace("{port}", "Ceres Freeport"),
    )
    .await;
    let p08 = station_room(&rig.tw, P, "room.p08_exchange");
    let _ = run(&mut rig, GOD, &format!("@teleport #{}=#{}", vex.0, p08.0)).await;

    // 8. A queued depart with the pilot ashore: it holds, and burns nothing.
    let tickets_before = tickets(&mut rig.tw, hull).len();
    let tank_before = pool(&mut rig.tw, hold, "propellant");
    let out = run(&mut rig, vex, "+ship/order depart ceres at=now").await;
    assert!(out.contains("Order 2 filed"), "{out}");
    for _ in 0..3 {
        let _ = fire_quietly(&mut rig).await;
    }
    let ashore = tacsim::port::executor::HELD_ASHORE
        .replace("{name}", "Salt Moth")
        .replace("{port}", "Stickney Yards");
    let log = log_texts(&rig.tw, hull);
    assert!(log.iter().any(|l| l.contains(&ashore)), "{log:#?}");
    assert_eq!(tickets(&mut rig.tw, hull).len(), tickets_before, "no burn");
    assert_eq!(
        pool(&mut rig.tw, hold, "propellant"),
        tank_before,
        "nothing burned"
    );
    assert!(!under_way(&rig.tw, hull));

    // 12. `o` → `x` → `c` from a Stickney-stamped position: the `+ic` hint.
    let _ = run(&mut rig, vex, "o").await;
    assert_ne!(location(&rig.tw, vex), Some(p08), "`o` stepped out");
    walk(&mut rig, vex, "x", LIMBO).await;
    let out = run(&mut rig, vex, "c").await;
    assert!(
        out.contains(&tacsim::ic_return::left_at_line(
            &tacsim::ic_return::PORTS[1]
        )),
        "{out}"
    );
    assert_eq!(location(&rig.tw, vex), Some(LIMBO), "the door held");
    let out = run(&mut rig, vex, "+ic").await;
    assert_eq!(
        location(&rig.tw, vex),
        Some(station_room(&rig.tw, P, "room.p01_berth_gallery")),
        "{out}"
    );
    let _ = ruth;
    assert_no_pages(&rig.tw);
}

// ---------------------------------------------------------------------------
// The sailing plan's skips and holds (IC11's untested executor paths)
// ---------------------------------------------------------------------------

/// `+ship/order <line>` as `who`, asserted filed.
async fn file(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let out = run(rig, who, &format!("+ship/order {line}")).await;
    assert!(out.contains("filed"), "`{line}`: {out}");
    out
}

/// Fire (landing everything but what `keep` holds back) until `done`, at most `max`
/// fires; the held-back submissions are returned, never landed.
async fn fire_until(
    rig: &mut Rig,
    max: usize,
    keep: impl Fn(&AuthoritativeSubmit) -> bool,
    done: impl Fn(&Rig) -> bool,
) -> Vec<AuthoritativeSubmit> {
    let mut held = Vec::new();
    for _ in 0..max {
        if done(rig) {
            return held;
        }
        let (_, queued) = fire(rig).await;
        let (back, rest): (Vec<_>, Vec<_>) = queued.into_iter().partition(|s| keep(s));
        held.extend(back);
        land_quietly(rig, &rest).await;
    }
    assert!(done(rig), "the condition did not hold within {max} fires");
    held
}

fn has_log(tw: &TestWorld, hull: ObjId, line: &str) -> bool {
    log_texts(tw, hull).iter().any(|l| l == line)
}

/// **IC11's untested executor paths, on the production composition** (doc 18 cut
/// IC12 item 1.8; lead note 3e). Two worlds: a hull at Ceres for the consumable holds
/// (`HELD_SHORT_FOR_DEPART`, `HELD_NO_DEPART`, `FILL_ENOUGH`), `NO_BID` and three
/// dropped burns (`HELD_UNCONFIRMED`); a hull at Stickney for `UNDER_MIN`,
/// `NOTHING_ABOARD` and the two late hand-overs (`AWAITING_REFUND` after
/// `DELIVER_TOO_LATE` and after `DRAW_TOO_LATE`). Each asserts the executor's log line
/// (the constant), the hold shape, and that nothing moved. `REFUSED_RETRY`,
/// `INTAKE_TAKEN`, `HELD_FILL` and `HELD_FILL_PLAN` are pinned at the unit level
/// (`executor.rs`'s `ic12_*` tests: no step on the shipped packs reaches them).
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn ic12_the_sailing_plan_skips_and_holds() {
    use tacsim::port::executor as ex;

    // ---- Ceres: the consumable holds, NO_BID, three dropped burns ----
    let mut rig = boot("skips-ceres").await;
    let _ = sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let ada = pilot(&mut rig, "Ada", "adaacct").await;
    let row = enrol(&mut rig, ada, "Salt Moth").await;
    let hull = ObjId(row.hull);
    let hold = hold_of(&rig.tw, &row);
    walk_path(
        &mut rig,
        ada,
        &[
            ("arrivals", C, "room.c03_arrivals"),
            ("concourse", C, "room.c04_concourse"),
            ("exchange", C, "room.c07_exchange"),
        ],
    )
    .await;
    let (m, v) = vertex_load();
    let metals = listing_of(&rig.tw, C, "freeport_exchange", "metals");
    let ice = listing_of(&rig.tw, C, "occator", "volatiles");
    let _ = buy(&mut rig, ada, metals, m).await;
    let _ = buy(&mut rig, ada, ice, v).await;
    walk_path(&mut rig, ada, &[("chandlery", C, "room.c08_chandlery")]).await;
    let plan = run(&mut rig, ada, "+ship/plan stickney").await;
    let (days, _) = plan_numbers(&plan, "Stores ", "(locker ").expect(&plan);
    let food = listing_of(&rig.tw, C, "coop", "provisions");
    let _ = buy(&mut rig, ada, food, days + 2).await;
    let _ = sweep(&mut rig, 61).await;
    let _ = sweep(&mut rig, 61).await;
    board_from_chandlery(&mut rig, ada, "Salt Moth").await;
    set_rate(&mut rig, 3_600).await;
    assert_eq!(pool(&mut rig.tw, hold, "propellant"), 0);

    // NO_BID: no house at Ceres buys concentrate — the sale is passed over.
    let _ = file(&mut rig, ada, "sell metals all").await;
    let _ = fire_until(
        &mut rig,
        4,
        |_| false,
        |rig| order_state(&rig.tw, hull, 1) == Some(OrderState::Skipped),
    )
    .await;
    let skip = ex::SKIP_LINE.replace("{n}", "1").replace(
        "{reason}",
        &ex::NO_BID.replace("{label}", "metal concentrate"),
    );
    assert!(
        has_log(&rig.tw, hull, &skip),
        "{:#?}",
        log_texts(&rig.tw, hull)
    );
    assert_eq!(pool(&mut rig.tw, hold, "metals"), m, "the cargo is kept");

    // HELD_SHORT_FOR_DEPART: an explicit tonne of reaction mass ahead of a depart
    // whose burn it cannot cover holds, buying nothing.
    let _ = file(&mut rig, ada, "buy propellant 1").await;
    let _ = file(&mut rig, ada, "depart stickney at=+2h").await;
    let _ = fire_until(
        &mut rig,
        4,
        |_| false,
        |rig| orders(&rig.tw, hull).state == PlanState::Held,
    )
    .await;
    let doc = orders(&rig.tw, hull);
    let reason = doc
        .hold
        .as_ref()
        .map(|h| h.reason.clone())
        .expect("the hold");
    let prefix = "1 t would leave the passage to Stickney Yards short; it needs ";
    assert!(
        reason.starts_with(prefix) && reason.ends_with(" t."),
        "{reason}"
    );
    let need: u64 = reason
        .trim_start_matches(prefix)
        .trim_end_matches(" t.")
        .parse()
        .expect("the need");
    assert_eq!(doc.order(2).map(|o| o.state), Some(OrderState::Held));
    assert!(has_log(
        &rig.tw,
        hull,
        &ex::HELD_LINE
            .replace("{n}", "2")
            .replace("{reason}", &reason)
    ));
    assert_eq!(pool(&mut rig.tw, hold, "propellant"), 0, "nothing bought");
    let out = run(&mut rig, ada, "+ship/order del 2").await;
    assert!(out.contains("Order 2"), "{out}");
    let _ = run(&mut rig, ada, "+ship/order del 3").await;

    // HELD_NO_DEPART: `fill` with no depart after it.
    let _ = file(&mut rig, ada, "buy propellant fill").await;
    let _ = fire_until(
        &mut rig,
        4,
        |_| false,
        |rig| orders(&rig.tw, hull).state == PlanState::Held,
    )
    .await;
    assert!(has_log(
        &rig.tw,
        hull,
        &ex::HELD_LINE
            .replace("{n}", "4")
            .replace("{reason}", ex::HELD_NO_DEPART)
    ));
    assert_eq!(pool(&mut rig.tw, hold, "propellant"), 0, "nothing bought");
    let _ = run(&mut rig, ada, "+ship/order del 4").await;

    // `fill` ahead of the depart buys to the burn the held order named; a second
    // `fill` then finds enough aboard (FILL_ENOUGH).
    let _ = file(&mut rig, ada, "depart stickney at=+2h").await;
    let _ = file(&mut rig, ada, "buy propellant fill").await;
    let _ = run(&mut rig, ada, "+ship/order move 6=2").await;
    let _ = fire_until(
        &mut rig,
        20,
        |_| false,
        |rig| order_state(&rig.tw, hull, 6) == Some(OrderState::Done),
    )
    .await;
    // `fill` buys to the burn at its own fixed point — the planner's figure — which
    // the held explicit order's need brackets (that projection carried its own tonne).
    let filled = pool(&mut rig.tw, hold, "propellant");
    let plan = run(&mut rig, ada, "+ship/plan stickney").await;
    let (burn, tank) = plan_numbers(&plan, "Burn ", "(tank ").expect(&plan);
    assert_eq!(
        (tank, burn),
        (filled, filled),
        "filled to the planner's burn: {plan}"
    );
    assert!(
        need.abs_diff(filled) <= 1,
        "the held need {need} against the fill {filled}"
    );
    let _ = file(&mut rig, ada, "buy propellant fill").await;
    let _ = run(&mut rig, ada, "+ship/order move 7=3").await;
    let _ = fire_until(
        &mut rig,
        6,
        |_| false,
        |rig| order_state(&rig.tw, hull, 7) == Some(OrderState::Done),
    )
    .await;
    assert!(
        has_log(
            &rig.tw,
            hull,
            &ex::FILL_ENOUGH
                .replace("{label}", "reaction mass")
                .replace("{port}", "Stickney Yards")
        ),
        "{:#?}",
        log_texts(&rig.tw, hull)
    );
    assert_eq!(
        pool(&mut rig.tw, hold, "propellant"),
        filled,
        "nothing more bought"
    );

    // Three dropped burns: the depart at once, every burn it files lost.
    // The stores first (the locker's own fill, landed), then the depart at once,
    // every burn it files lost.
    let _ = run(&mut rig, ada, "+ship/order del 5").await;
    let _ = file(&mut rig, ada, "depart stickney at=now").await;
    let _ = file(&mut rig, ada, "buy provisions fill").await;
    let _ = run(&mut rig, ada, "+ship/order move 9=4").await;
    disconnect(&mut rig.tw, ada);
    let is_burn = |s: &AuthoritativeSubmit| s.action.key().id.as_str() == tacsim::port::BURN_ACTION;
    let mut burns = fire_until(&mut rig, 20, is_burn, |rig| {
        order_state(&rig.tw, hull, 9) == Some(OrderState::Done)
    })
    .await;
    let tank = pool(&mut rig.tw, hold, "propellant");
    for _ in 0..200 {
        if orders(&rig.tw, hull).state == PlanState::Held {
            break;
        }
        let (_, queued) = fire(&mut rig).await;
        let (back, rest): (Vec<_>, Vec<_>) = queued
            .into_iter()
            .partition(|s| s.action.key().id.as_str() == tacsim::port::BURN_ACTION);
        burns.extend(back);
        land_quietly(&mut rig, &rest).await;
    }
    let doc = orders(&rig.tw, hull);
    assert_eq!(
        doc.hold.as_ref().map(|h| h.reason.as_str()),
        Some(ex::HELD_UNCONFIRMED)
    );
    assert!(
        burns.len() >= 3,
        "a burn per attempt: {:?}",
        burn_keys(&burns)
    );
    let dropped = ex::NOT_ADMITTED.replace("{n}", "8");
    assert!(
        log_texts(&rig.tw, hull)
            .iter()
            .filter(|l| **l == dropped)
            .count()
            >= 3,
        "{:#?}",
        log_texts(&rig.tw, hull)
    );
    assert!(tickets(&mut rig.tw, hull).is_empty(), "no ticket");
    assert_eq!(
        pool(&mut rig.tw, hold, "propellant"),
        tank,
        "nothing burned"
    );
    assert!(!under_way(&rig.tw, hull));
    assert_no_pages(&rig.tw);
    drop(rig);

    // ---- Stickney: UNDER_MIN, NOTHING_ABOARD, the late hand-overs ----
    let mut rig = boot("skips-stickney").await;
    let _ = sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let ben = pilot(&mut rig, "Ben", "benacct").await;
    let row = enrol(&mut rig, ben, "Salt Moth").await;
    let hull = ObjId(row.hull);
    let hold = hold_of(&rig.tw, &row);
    let outfit = outfit(&mut rig, ben).await;
    top_up(&mut rig, ben, 10_000_000).await;
    board_from_chandlery(&mut rig, ben, "Salt Moth").await;
    set_rate(&mut rig, 3_600).await;
    let filed = run(&mut rig, ben, "+ship/undock stickney").await;
    assert!(filed.contains("Clearance filed"), "{filed}");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued).await;
    let _ = fire_quietly(&mut rig).await;
    let flown = ticket(&mut rig.tw, hull, 1).expect("the ticket");
    let _ = to_ic(&mut rig, flown.plan.depart_ic + flown.plan.total_s).await;
    let _ = fire_quietly(&mut rig).await;
    assert!(!under_way(&rig.tw, hull), "docked at Stickney");

    // UNDER_MIN: a floor over Kasei's bid passes the sale over.
    let kasei = port_map(&rig.tw, P).vendors[&cid("kasei")].clone();
    let ice_bid = ListingId(kasei.bids[&cid("volatiles")].listing);
    let _ = file(&mut rig, ben, "sell volatiles all min=1000").await;
    disconnect(&mut rig.tw, ben);
    let _ = fire_until(
        &mut rig,
        4,
        |_| false,
        |rig| order_state(&rig.tw, hull, 1) == Some(OrderState::Skipped),
    )
    .await;
    let under = ex::UNDER_MIN
        .replace("{house}", "Kasei Metals")
        .replace("{price}", &cr(price_of(&rig.tw, ice_bid)))
        .replace("{unit}", "t")
        .replace("{min}", &cr(100_000));
    assert!(
        has_log(
            &rig.tw,
            hull,
            &ex::SKIP_LINE
                .replace("{n}", "1")
                .replace("{reason}", &under)
        ),
        "{:#?}",
        log_texts(&rig.tw, hull)
    );
    assert_eq!(
        pool(&mut rig.tw, hold, "volatiles"),
        outfit.ice,
        "the cargo is kept"
    );

    // AWAITING_REFUND after DELIVER_TOO_LATE: the sale's leg lost, and the first
    // re-stage comes after the sale's cutoff (a downtime): the hand-over is left to
    // refund, and the order is tried again after it.
    let _ = rig.tw.connect(ben);
    let _ = file(&mut rig, ben, "sell metals all").await;
    disconnect(&mut rig.tw, ben);
    let lost = fire_until(
        &mut rig,
        4,
        |s| s.key.as_str().starts_with("market.lot.deliver."),
        |rig| order_state(&rig.tw, hull, 2) == Some(OrderState::Inflight),
    )
    .await;
    let contract = orders(&rig.tw, hull)
        .inflight
        .and_then(|i| i.refs.contract)
        .expect("the sale's contract");
    let _ = lost;
    let _ = sweep(&mut rig, 3_100).await;
    let _ = drain(&mut rig.tw);
    let late = fire_until(
        &mut rig,
        4,
        |s| s.action.key().owner.as_str() == "market",
        |rig| has_log(&rig.tw, hull, &ex::AWAITING_REFUND.replace("{n}", "2")),
    )
    .await;
    assert!(
        late.is_empty(),
        "nothing re-staged past the cutoff: {late:?}"
    );
    assert_eq!(
        pool(&mut rig.tw, hold, "metals"),
        outfit.metals,
        "nothing left the hold"
    );
    assert!(
        orders(&rig.tw, hull).hold.is_none(),
        "the late hand-over awaits its refund, no hold"
    );
    // Past the hour the contract expires with its refund; the order is tried again.
    let _ = sweep(&mut rig, 3_700).await;
    let _ = fire_until(
        &mut rig,
        60,
        |_| false,
        |rig| {
            !economy(&rig.tw)
                .snapshot()
                .contract(ContractId(contract))
                .is_some_and(|c| !c.state.is_terminal())
        },
    )
    .await;

    // NOTHING_ABOARD: once the concentrate is sold, another sale of it is passed over.
    let _ = fire_until(
        &mut rig,
        80,
        |_| false,
        |rig| order_state(&rig.tw, hull, 2) == Some(OrderState::Done),
    )
    .await;
    assert_eq!(
        pool(&mut rig.tw, hold, "metals"),
        0,
        "the retried sale sold it"
    );
    let _ = rig.tw.connect(ben);
    let _ = file(&mut rig, ben, "sell metals all").await;
    let _ = fire_until(
        &mut rig,
        4,
        |_| false,
        |rig| order_state(&rig.tw, hull, 3) == Some(OrderState::Skipped),
    )
    .await;
    assert!(has_log(
        &rig.tw,
        hull,
        &ex::SKIP_LINE.replace("{n}", "3").replace(
            "{reason}",
            &ex::NOTHING_ABOARD.replace("{label}", "metal concentrate")
        )
    ));

    // AWAITING_REFUND after DRAW_TOO_LATE: a buy whose Draw is never landed, its first
    // re-stage past the Draw cutoff (`DRAW_WINDOW_SECS − DRAW_SETTLE_MARGIN_SECS`,
    // 3,000 s after the accept). The escrow is a hold: the accept leaves the posted
    // balance alone and takes the price out of the available funds; the expiry gives
    // it back. Both are read before the order is filed (order 2's sale has settled).
    let me = credit_account(&rig.tw, &PrincipalId::Object(ben));
    let (balance, free) = (posted(&rig.tw, me), available(&rig.tw, me));
    let cutoff = mush_market::DRAW_WINDOW_SECS - mush_market::DRAW_SETTLE_MARGIN_SECS;
    let alloys = listing_of(&rig.tw, P, "kasei", "structural_alloys");
    let escrow = price_of(&rig.tw, alloys) * 5;
    let _ = file(&mut rig, ben, "buy structural_alloys 5").await;
    let _ = fire_until(
        &mut rig,
        4,
        |s| s.key.as_str().starts_with("market.lot.draw."),
        |rig| order_state(&rig.tw, hull, 4) == Some(OrderState::Inflight),
    )
    .await;
    let draw = orders(&rig.tw, hull)
        .inflight
        .and_then(|i| i.refs.contract)
        .expect("the buy's contract");
    assert_eq!(
        (posted(&rig.tw, me), available(&rig.tw, me)),
        (balance, free - escrow),
        "the accept holds the price in escrow"
    );
    let _ = sweep(&mut rig, cutoff + 100).await;
    let _ = drain(&mut rig.tw);
    let late = fire_until(
        &mut rig,
        4,
        |s| s.action.key().owner.as_str() == "market",
        |rig| has_log(&rig.tw, hull, &ex::AWAITING_REFUND.replace("{n}", "4")),
    )
    .await;
    assert!(
        late.is_empty(),
        "nothing re-staged past the cutoff: {late:?}"
    );
    assert_eq!(
        pool(&mut rig.tw, hold, "structural_alloys"),
        0,
        "nothing drawn"
    );
    // Within the hour the escrow comes back.
    let _ = sweep(&mut rig, 3_700).await;
    let _ = fire_until(
        &mut rig,
        60,
        |_| false,
        |rig| {
            economy(&rig.tw)
                .snapshot()
                .contract(ContractId(draw))
                .is_some_and(|c| c.state.is_terminal())
        },
    )
    .await;
    assert!(
        matches!(
            economy(&rig.tw)
                .snapshot()
                .contract(ContractId(draw))
                .map(|c| c.state),
            Some(ContractState::Expired)
        ),
        "the Draw's contract expired (its refund)"
    );
    assert_eq!(
        (posted(&rig.tw, me), available(&rig.tw, me)),
        (balance, free),
        "the escrow is released exactly: nothing paid, nothing held"
    );
    assert_no_pages(&rig.tw);
}

/// **Doc 18 cut IC14 row 40 (lead ruling L23): a counter refusal is retried, on the
/// shipped packs.** A hull docked at Stickney with metal concentrate aboard queues
/// `sell metals all` while staff have suspended Kasei Metals (`@org/status
/// #<org>=suspended`, a real staff act): the market refuses the sale at the counter
/// (`NOT_BUYING`), the plan's log carries `REFUSED_RETRY` once and the order waits a
/// minute; staff lift the suspension and the retry sells. `INTAKE_TAKEN` stays at the
/// unit level: no staff act on the shipped packs exhausts a bid's intake (Kasei's 4,000
/// and 9,600 t against a 100 t hold).
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn ic14_a_counter_refusal_is_retried_on_the_shipped_packs() {
    use tacsim::port::executor as ex;

    let mut rig = boot("refusedretry").await;
    let _ = sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let cy = pilot(&mut rig, "Cy", "cyacct").await;
    let row = enrol(&mut rig, cy, "Salt Moth").await;
    let hull = ObjId(row.hull);
    let hold = hold_of(&rig.tw, &row);
    let outfit = outfit(&mut rig, cy).await;
    top_up(&mut rig, cy, 10_000_000).await;
    board_from_chandlery(&mut rig, cy, "Salt Moth").await;
    set_rate(&mut rig, 3_600).await;
    let filed = run(&mut rig, cy, "+ship/undock stickney").await;
    assert!(filed.contains("Clearance filed"), "{filed}");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued).await;
    let _ = fire_quietly(&mut rig).await;
    let flown = ticket(&mut rig.tw, hull, 1).expect("the ticket");
    let _ = to_ic(&mut rig, flown.plan.depart_ic + flown.plan.total_s).await;
    let _ = fire_quietly(&mut rig).await;
    assert!(!under_way(&rig.tw, hull), "docked at Stickney");
    assert_eq!(pool(&mut rig.tw, hold, "metals"), outfit.metals);

    // Staff suspend Kasei; the queued sale is refused at the counter and retried.
    let kasei = port_map(&rig.tw, P).orgs[&cid("kasei")].org;
    let out = run(&mut rig, GOD, &format!("@org/status #{kasei}=suspended")).await;
    assert!(!out.contains("Usage"), "{out}");
    let _ = file(&mut rig, cy, "sell metals all").await;
    disconnect(&mut rig.tw, cy);
    let retry = ex::REFUSED_RETRY
        .replace("{n}", "1")
        .replace("{line}", mush_market::NOT_BUYING);
    let _ = fire_until(&mut rig, 6, |_| false, |rig| has_log(&rig.tw, hull, &retry)).await;
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.order(1).map(|o| o.refusals), Some(1), "{:#?}", doc.log);
    assert_eq!(doc.order(1).map(|o| o.state), Some(OrderState::Pending));
    assert_eq!(
        pool(&mut rig.tw, hold, "metals"),
        outfit.metals,
        "nothing sold"
    );
    // Lifted: the retry sells.
    let out = run(&mut rig, GOD, &format!("@org/status #{kasei}=active")).await;
    assert!(!out.contains("Usage"), "{out}");
    let _ = fire_until(
        &mut rig,
        60,
        |_| false,
        |rig| order_state(&rig.tw, hull, 1) == Some(OrderState::Done),
    )
    .await;
    assert_eq!(pool(&mut rig.tw, hold, "metals"), 0, "sold after the lift");
    assert_eq!(
        log_texts(&rig.tw, hull)
            .iter()
            .filter(|l| **l == retry)
            .count(),
        1,
        "REFUSED_RETRY logged once"
    );
}
