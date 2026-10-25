//! **Doc 18 cut IC13 — buyout, graduation and dormancy, on the production composition**
//! (doc 18 §4.1 "The buyout" and "Losses, dormancy and release", §5 IC13 row; D-18-21,
//! D-18-23, D-18-24, D-18-25; the IC13 brief's item 9 and lead rulings L1–L16).
//!
//! The rig is `ic_slice_loop.rs`'s, copied file-local (lead ruling L13: that file is not
//! edited): every test boots **`mush_server::boot::build_engine`** — the split server's
//! own root — over an in-memory store at `[game].ic_ratio = 24`, enrols at the play rate,
//! and lands every queued authoritative submission exactly as the world task does; a
//! restart is a plain `build_engine` re-boot over the kept store. Every expected amount
//! is computed (D-18-39) from the enrolment's frozen terms, the posted prices and the
//! accounts, never a literal beyond the programme's 90,000 and 110,000; each test ends
//! with no escalation of any kind open.
//!
//! The tests (the verify lens of the doc 18 §5 IC13 row, made concrete):
//! - [`ic13_a_buyout_passes_title_once_ends_the_lease_and_silences_the_share`] — the
//!   title moves once, the lease ends, the Trust takes exactly the price, a restart keeps
//!   it all, and the graduate's next sale at Stickney carries no Co-op share;
//! - [`ic13_a_buyout_is_refused_under_way_cleared_elsewhere_or_with_goods_moving`] —
//!   goods moving, a step at the counter, a filed clearance, under way and docked at
//!   Stickney each refuse and write nothing;
//! - [`ic13_a_refunded_buyout_re_leases`] — a staff-cancelled buyout refunds and the
//!   observer re-charters the same hull; a second buyout then graduates;
//! - [`ic13_seven_idle_days_recall_a_docked_empty_hull_and_the_next_enrolment_charters_it`]
//!   — the dormancy recall, the forfeit riding with the re-homed hold, and the next
//!   captain chartering that very hull;
//! - [`ic13_a_departure_keeps_the_charter_and_a_dormant_hull_with_cargo_holds_its_plan`]
//!   — a burn resets the clock; a dormant hull at Stickney with cargo holds its plan and
//!   pages once;
//! - [`ic13_release_and_revoke_recall_through_one_path`] — `+coop/release` and staff
//!   `@coop/revoke` (every pilot spelling, every refusal) through the one recall;
//! - [`ic13_a_queued_order_waits_out_the_buyout_and_runs_on_the_title`] — a queued buy
//!   waits under the buyout's own note through the window and runs on the title once it
//!   passes.

#[path = "../../tacsim/tests/support/vertex.rs"]
mod vertex;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mush_config::config::Config;
use mush_core::{
    AccountId, Audience, AuthoritativeSubmit, CanonicalId, GOD, ObjId, OfferExecutionContext,
    OfferSubmission, PrincipalId as CorePrincipal,
};
use mush_economy::{ContractId, ContractState, EconomyStore, VaultId};
use mush_inventory::{EndpointRef, InventoryHandles};
use mush_market::{ListingId, MarketStore};
use mush_org::{OrgId, PrincipalId};
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
    EndCause, Enrolment, EnrolmentStatus, HullRow, OrderState, Orders, OrdersState, PlanState,
    PortDef, PortHandles, PortMap, PortMapState, Ticket, TicketId, room_key,
};

// ---------------------------------------------------------------------------
// The rig: `ic_slice_loop.rs`'s, copied (L13)
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
    tw: Box<TestWorld>,
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
// Keep the production boot's large result out of the enclosing fixture future.
// The heap-backed future remains lazy and propagates the actual boot result.
fn boxed_boot_engine<'a>(
    cfg: &'a Config,
    store: &'a Arc<dyn ObjectStore>,
    spool: &'a QuarantineSpool,
) -> std::pin::Pin<
    Box<impl std::future::Future<Output = Result<Box<boot::BootedEngine>, String>> + 'a>,
> {
    Box::pin(async move { boot::build_engine(cfg, store, spool).await.map(Box::new) })
}

async fn boot(tag: &str) -> Rig {
    let cfg = config(tag);
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let spool = QuarantineSpool::open(cfg.server.data_dir.join("quarantine")).expect("a spool");
    let booted = boxed_boot_engine(&cfg, &store, &spool)
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
    let tw = Box::new(TestWorld::from_engine(booted.engine));
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
    let old = std::mem::replace(&mut rig.tw, Box::new(TestWorld::new()));
    drop(old);
    let booted = boxed_boot_engine(&rig.cfg, &rig.store, &rig.spool)
        .await
        .expect("the restart boots over the kept store");
    *rig.tw = TestWorld::from_engine(booted.engine);
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
                let heard = rig.tw.install_composed_action_announcing(installation);
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

/// **The queue drained, the house's share landed** (doc 20 cut SM.12b): the Occator Yard's
/// dispatcher works Occator Rim in every keyed world, its drones' plan steps and unloads queued by
/// the ticks; they are landed here, quietly, and the rest — the players' and the clerks' — handed
/// back, so what a typed line or a fire is counted to have queued is the players' world's.
async fn drain_players(rig: &mut Rig) -> Vec<AuthoritativeSubmit> {
    let house = tacsim::port::yard::dispatchers(rig.tw.world());
    let (theirs, rest): (Vec<_>, Vec<_>) = drain(&mut rig.tw)
        .into_iter()
        .partition(|submit| house.contains(&submit.actor));
    land_quietly(rig, &theirs).await;
    rest
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
    let queued = drain_players(rig).await;
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
    let queued = drain_players(rig).await;
    land_quietly(rig, &queued).await;
    events
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
    let queued = drain_players(rig).await;
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

/// The hull's sailing plan.
fn orders(tw: &TestWorld, hull: ObjId) -> Orders {
    match store(tw).orders_state(hull) {
        OrdersState::Present(orders) => orders.clone(),
        other => panic!("the sailing plan: {other:?}"),
    }
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

// ---------------------------------------------------------------------------
// Cut IC13's reads
// ---------------------------------------------------------------------------

/// The programme's buyout price, the stake and the 110,000 the lens tops a pilot up to.
const LENS_AVAILABLE: i64 = 11_000_000;

/// `who`'s enrolment document.
fn enrolment(tw: &TestWorld, who: ObjId) -> Enrolment {
    store(tw).enrolment_of(who).cloned().expect("the enrolment")
}

fn contract_state(tw: &TestWorld, id: u64) -> ContractState {
    economy(tw)
        .snapshot()
        .contract(ContractId(id))
        .map(|c| c.state)
        .expect("the contract")
}

fn contract_count(tw: &TestWorld) -> usize {
    economy(tw).snapshot().contracts().count()
}

/// The Trust's programme account and the Trust as a principal.
fn trust_account(tw: &TestWorld) -> mush_economy::AccountId {
    mush_economy::AccountId(
        port_map(tw, "ceres_port").orgs[&cid("coop_trust")]
            .account
            .expect("the Trust's account"),
    )
}

fn trust(tw: &TestWorld) -> PrincipalId {
    PrincipalId::Organization(OrgId(
        port_map(tw, "ceres_port").orgs[&cid("coop_trust")].org,
    ))
}

fn title_owner(tw: &TestWorld, hull: ObjId) -> Option<PrincipalId> {
    economy(tw)
        .snapshot()
        .asset(hull)
        .map(|a| a.title_owner.clone())
}

/// The journal transactions that moved `hull`'s title.
fn title_moves(tw: &TestWorld, hull: ObjId) -> usize {
    economy(tw)
        .transactions()
        .filter(|t| {
            t.asset_effects.iter().any(|a| {
                a.object == hull && a.kind == mush_economy::AssetEffectKind::TitleTransferred
            })
        })
        .count()
}

fn vault_owner(tw: &TestWorld, vault: VaultId) -> PrincipalId {
    economy(tw)
        .snapshot()
        .vault(vault)
        .map(|v| v.owner.clone())
        .expect("the vault")
}

/// The readiness report's Ceres hulls line.
async fn hulls_line(rig: &mut Rig) -> String {
    let report = run(rig, GOD, "@tacsim/port status").await;
    report
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("ceres_port hulls: "))
        .unwrap_or_else(|| panic!("a hulls line:\n{report}"))
        .to_owned()
}

/// A Ceres hull's row by its Thing.
fn row_of_hull(tw: &TestWorld, hull: ObjId) -> HullRow {
    port_map(tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|row| row.hull == hull.0)
        .expect("the hull's row")
}

fn name_of(tw: &TestWorld, obj: ObjId) -> String {
    tw.world()
        .get(obj)
        .map(|o| o.name.clone())
        .expect("an object")
}

/// Every text in `events`, whoever hears it.
fn all_texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| mush_testkit::text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}

/// From the chandlery to the Registry & Co-op desk.
async fn registry_from_chandlery(rig: &mut Rig, who: ObjId) {
    for (word, local) in [
        ("exchange", "room.c07_exchange"),
        ("concourse", "room.c04_concourse"),
        ("arrivals", "room.c03_arrivals"),
        ("registry", "room.c05_registry"),
    ] {
        let to = station_room(&rig.tw, "ceres_port", local);
        walk(rig, who, word, to).await;
    }
}

/// From the Registry & Co-op desk to the chandlery.
async fn chandlery_from_registry(rig: &mut Rig, who: ObjId) {
    for (word, local) in [
        ("out", "room.c03_arrivals"),
        ("concourse", "room.c04_concourse"),
        ("exchange", "room.c07_exchange"),
        ("chandlery", "room.c08_chandlery"),
    ] {
        let to = station_room(&rig.tw, "ceres_port", local);
        walk(rig, who, word, to).await;
    }
}

/// From the Registry & Co-op desk to the berth gallery.
async fn gallery_from_registry(rig: &mut Rig, who: ObjId) {
    for (word, local) in [
        ("out", "room.c03_arrivals"),
        ("lobby", "room.c02_pressure"),
        ("gallery", "room.c01_berth_gallery"),
    ] {
        let to = station_room(&rig.tw, "ceres_port", local);
        walk(rig, who, word, to).await;
    }
}

/// From the berth gallery to the Registry & Co-op desk.
async fn registry_from_gallery(rig: &mut Rig, who: ObjId) {
    for (word, local) in [
        ("lobby", "room.c02_pressure"),
        ("arrivals", "room.c03_arrivals"),
        ("registry", "room.c05_registry"),
    ] {
        let to = station_room(&rig.tw, "ceres_port", local);
        walk(rig, who, word, to).await;
    }
}

/// What a refused buyout must leave as it found: the contracts, the enrolment (Active,
/// no buyout), the lease (Active) and the pilot's available funds.
struct Untouched {
    contracts: usize,
    enrolment: Enrolment,
    available: i64,
}

fn untouched(tw: &TestWorld, who: ObjId) -> Untouched {
    let e = enrolment(tw, who);
    assert_eq!(e.status, EnrolmentStatus::Active);
    assert_eq!(e.buyout, None);
    assert_eq!(contract_state(tw, e.lease), ContractState::Active);
    Untouched {
        contracts: contract_count(tw),
        available: available(tw, credit_account(tw, &PrincipalId::Object(who))),
        enrolment: e,
    }
}

fn assert_untouched(tw: &TestWorld, who: ObjId, was: &Untouched, why: &str) {
    assert_eq!(contract_count(tw), was.contracts, "no contract: {why}");
    assert_eq!(enrolment(tw, who), was.enrolment, "the enrolment: {why}");
    assert_eq!(
        contract_state(tw, was.enrolment.lease),
        ContractState::Active,
        "the lease: {why}"
    );
    assert_eq!(
        available(tw, credit_account(tw, &PrincipalId::Object(who))),
        was.available,
        "the funds: {why}"
    );
}

/// Top `who` up so exactly `target` minor is available.
async fn top_up_to(rig: &mut Rig, who: ObjId, target: i64) {
    let me = credit_account(&rig.tw, &PrincipalId::Object(who));
    let short = target - available(&rig.tw, me);
    assert!(short > 0, "already {} available", available(&rig.tw, me));
    top_up(rig, who, short).await;
    assert_eq!(available(&rig.tw, me), target);
}

/// Land the burn the undock queued, then fire until she is under way and the launch is
/// told; the burn's real second and the ticket.
async fn launch(rig: &mut Rig, hull: ObjId) -> (i64, Ticket) {
    let burn = drain_players(rig).await;
    assert_eq!(burn.len(), 1, "the undock's burn");
    land_all(rig, &burn).await;
    let burned_at = rig.tw.world().now();
    for _ in 0..4 {
        let _ = fire_quietly(rig).await;
        if under_way(&rig.tw, hull) {
            break;
        }
    }
    assert!(under_way(&rig.tw, hull), "she launched");
    let _ = fire_quietly(rig).await;
    let ticket = ticket(&mut rig.tw, hull, 1).expect("the ticket");
    (burned_at, ticket)
}

/// Fly the ticket to its arrival.
async fn arrive(rig: &mut Rig, hull: ObjId, ticket: &Ticket) {
    let arrival = ticket.plan.depart_ic + ticket.plan.total_s;
    let _ = to_ic(rig, arrival).await;
    assert!(!under_way(&rig.tw, hull), "docked at the ticket's arrival");
}

// ---------------------------------------------------------------------------
// 1. The buyout
// ---------------------------------------------------------------------------

/// **The verify lens, item 1** (lead rulings L1, L2, L7, L10): a pilot with 110,000.00cr
/// available buys out. The title moves to the pilot exactly once, the lease is
/// Cancelled, the Trust's account rises by exactly the price; the graduate's next sale
/// carries no Co-op share, moves nothing of the Trust's and folds no counter.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn ic13_a_buyout_passes_title_once_ends_the_lease_and_silences_the_share() {
    let mut rig = boot("buyout").await;
    let _ = sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let vex = pilot(&mut rig, "Vex", "vexacct").await;
    let row = enrol(&mut rig, vex, "Salt Moth").await;
    let hull = ObjId(row.hull);
    let hold = hold_of(&rig.tw, &row);
    let outfit = outfit(&mut rig, vex).await;
    registry_from_chandlery(&mut rig, vex).await;
    let me = credit_account(&rig.tw, &PrincipalId::Object(vex));
    let trust_acct = trust_account(&rig.tw);
    let e = enrolment(&rig.tw, vex);
    let price = e.buyout_minor;
    let lease = e.lease;
    let moves_before = title_moves(&rig.tw, hull);

    // 2. Short of the price: refused with what is available; nothing written.
    let was = untouched(&rig.tw, vex);
    assert_eq!(
        was.available,
        e.stake_minor - outfit.costs,
        "the stake less the outfit"
    );
    assert!(was.available < price);
    let out = run(&mut rig, vex, "+coop/buyout").await;
    let short = tacsim::port::buyout::BUYOUT_FUNDS
        .replace("{buyout}", &cr(price))
        .replace("{available}", &cr(was.available));
    assert!(out.contains(&short), "{out}\nwanted: {short}");
    assert_untouched(&rig.tw, vex, &was, "short of the price");

    // 3. Topped up to exactly 110,000.00cr available.
    top_up_to(&mut rig, vex, LENS_AVAILABLE).await;
    let posted_before = posted(&rig.tw, me);
    let trust_before = posted(&rig.tw, trust_acct);
    let contracts_before = contract_count(&rig.tw);

    // 4. The buyout: the interim reply, the enrolment pending, the lease Cancelled, the
    // contract Active, the price held.
    let out = run(&mut rig, vex, "+coop/buyout").await;
    let filed = tacsim::port::buyout::BUYOUT_FILED
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark)
        .replace("{buyout}", &cr(price))
        .replace("{countersign}", tacsim::port::coop::COUNTERSIGN);
    assert!(out.contains(&filed), "{out}\nwanted: {filed}");
    let pending = enrolment(&rig.tw, vex);
    assert_eq!(pending.status, EnrolmentStatus::BuyoutPending);
    let c = pending.buyout.expect("the buyout contract");
    assert_eq!(contract_count(&rig.tw), contracts_before + 1);
    assert_eq!(contract_state(&rig.tw, lease), ContractState::Cancelled);
    assert_eq!(contract_state(&rig.tw, c), ContractState::Active);
    assert_eq!(available(&rig.tw, me), LENS_AVAILABLE - price);
    assert_eq!(
        posted(&rig.tw, me) - available(&rig.tw, me),
        price,
        "the price is held"
    );
    assert_eq!(posted(&rig.tw, me), posted_before, "an escrow is a hold");
    let line = hulls_line(&mut rig).await;
    assert!(
        line.contains(&format!(", 1 being bought out ({})", row.mark)) && line.ends_with("[ready]"),
        "{line}"
    );
    // The window (owner item O6): neither charter nor title, so no boarding.
    gallery_from_registry(&mut rig, vex).await;
    let out = run(&mut rig, vex, "+ship/board").await;
    assert!(
        out.contains(tacsim::port::coop::NO_CHARTER),
        "the window refuses boarding: {out}"
    );

    // 5. The next sweep settles: the title moves once, the money once.
    let events = sweep(&mut rig, 31).await;
    assert!(matches!(
        contract_state(&rig.tw, c),
        ContractState::Settled { .. }
    ));
    assert_eq!(title_owner(&rig.tw, hull), Some(PrincipalId::Object(vex)));
    assert_eq!(
        title_moves(&rig.tw, hull),
        moves_before + 1,
        "exactly one journal transaction moved the hull's title"
    );
    assert_eq!(posted(&rig.tw, trust_acct), trust_before + price);
    assert_eq!(posted(&rig.tw, me), posted_before - price);
    assert_eq!(available(&rig.tw, me), LENS_AVAILABLE - price);
    let graduated = enrolment(&rig.tw, vex);
    assert_eq!(graduated.status, EnrolmentStatus::Graduated);
    assert_eq!(graduated.ended, Some(EndCause::BoughtOut));
    assert!(graduated.ended_at.is_some());
    assert_eq!(graduated.buyout, Some(c));
    let notice = tacsim::port::buyout::GRADUATED
        .replace("{name}", "Salt Moth")
        .replace("{buyout}", &cr(price));
    assert_eq!(
        notice,
        format!(
            "The Registry writes your name over the Co-op's mark. The Salt Moth is yours: {} \
             paid; the charter ends; no further share.",
            cr(price)
        ),
        "doc 18 §2 step 19's line"
    );
    assert!(
        said(&rig.tw, &events, vex).contains(&notice),
        "{}",
        said(&rig.tw, &events, vex)
    );

    // 6. The renders.
    let terms = run(&mut rig, vex, "+coop").await;
    let grad_line = tacsim::port::coop::GRADUATED_LINE
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark);
    assert!(terms.contains(&grad_line), "{terms}");
    let statement = run(&mut rig, vex, "+coop/statement").await;
    assert!(
        statement.contains("  Charter: bought out; the hull is yours."),
        "{statement}"
    );
    let ship = run(&mut rig, vex, "+ship").await;
    let ship_line = tacsim::port::ship::SHIP_GRADUATED.replace("{name}", "Salt Moth");
    assert!(ship.contains(&ship_line), "{ship}");
    let again = run(&mut rig, vex, "+coop/buyout").await;
    let already = tacsim::port::buyout::ALREADY_GRADUATED
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark);
    assert!(again.contains(&already), "{again}");

    // 7. The hulls line counts the graduate and the gate stays open: a second pilot
    // enrols and is chartered the next hull; Vex's alt is refused for ever.
    let line = hulls_line(&mut rig).await;
    let made = port_map(&rig.tw, "ceres_port").hulls.len();
    assert!(
        line.contains(&format!("{made} of {made} made, {} titled to ", made - 1))
            && line.contains(&format!(", 1 graduated ({} to Vex)", row.mark))
            && line.ends_with("[ready]"),
        "{line}"
    );
    // Doc 18 cut IC14 row 34: the whole line, byte for byte, on the production root.
    assert_eq!(
        line,
        format!(
            "ceres_port hulls: {made} of {made} made, {} titled to {}; {} laid up and free, 0 \
             leased, 1 graduated ({} to Vex). [ready]",
            made - 1,
            trust_name(&rig.tw),
            made - 1,
            row.mark
        )
    );
    stock(&mut rig).await;
    let ash = pilot(&mut rig, "Ash", "ashacct").await;
    let ash_row = enrol(&mut rig, ash, "Tern").await;
    assert_eq!(ash_row.mark, "FPC-D-0402");
    let alt = alt(&mut rig, "Vexalt", "vexacct", vex).await;
    let c03 = station_room(&rig.tw, "ceres_port", "room.c03_arrivals");
    let c05 = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    walk(&mut rig, alt, "c", c03).await;
    walk(&mut rig, alt, "registry", c05).await;
    let out = run(&mut rig, alt, "+coop/enrol Second Wind").await;
    assert!(out.contains(tacsim::port::coop::ENROLLED_ONCE), "{out}");

    // 8. More sweeps and a restart leave the title, the one transfer and the document.
    for _ in 0..3 {
        let _ = sweep(&mut rig, 31).await;
    }
    restart(&mut rig, |tw| {
        tw.world()
            .module_state::<TacsimStore>()
            .and_then(|s| s.enrolment_of(vex))
            .is_some_and(|e| e.status == EnrolmentStatus::Graduated)
    })
    .await;
    assert_eq!(enrolment(&rig.tw, vex), graduated);
    assert_eq!(title_owner(&rig.tw, hull), Some(PrincipalId::Object(vex)));
    assert_eq!(title_moves(&rig.tw, hull), moves_before + 1);
    let _ = rig.tw.connect(vex);
    flush(&mut rig).await;

    // 9. On the title: board, plan, undock, fly to Stickney.
    set_rate(&mut rig, 3_600).await;
    let boarded = run(&mut rig, vex, "+ship/board").await;
    assert!(
        boarded.contains("You cycle the lock into the Salt Moth."),
        "{boarded}"
    );
    let plan = run(&mut rig, vex, "+ship/plan stickney").await;
    assert!(plan.starts_with("Plan for Stickney Yards"), "{plan}");
    let filed = run(&mut rig, vex, "+ship/undock stickney").await;
    assert!(
        filed.contains("Clearance filed for Stickney Yards"),
        "{filed}"
    );
    let (_, ticket) = launch(&mut rig, hull).await;
    // The graduate's burn left the hold as a charter's does: bought less burned.
    assert_eq!(
        pool(&mut rig.tw, hold, "propellant"),
        outfit.fuel - ticket.plan.burn_t
    );
    assert_eq!(
        pool(&mut rig.tw, hold, "provisions"),
        outfit.food - u64::from(ticket.plan.crew_days)
    );
    arrive(&mut rig, hull, &ticket).await;
    let p01 = station_room(&rig.tw, "stickney_port", "room.p01_berth_gallery");
    let ashore = run(&mut rig, vex, "+ship/leave").await;
    assert_eq!(location(&rig.tw, vex), Some(p01), "{ashore}");
    for (word, local) in [
        ("lobby", "room.p02_pressure"),
        ("customs", "room.p03_customs"),
        ("concourse", "room.p04_concourse"),
        ("exchange", "room.p08_exchange"),
        ("down", "room.p07_receiving"),
    ] {
        let to = station_room(&rig.tw, "stickney_port", local);
        walk(&mut rig, vex, word, to).await;
    }

    // 10. The graduate's sale: no Co-op share, the Trust unmoved, no counter folded.
    let kasei = port_map(&rig.tw, "stickney_port").vendors[&cid("kasei")].clone();
    let bid = ListingId(kasei.bids[&cid("metals")].listing);
    let price_t = price_of(&rig.tw, bid);
    let qty = outfit.metals;
    assert_eq!(pool(&mut rig.tw, hold, "metals"), qty);
    let gross = price_t * i64::try_from(qty).expect("qty");
    let label = goods().good(&cid("metals")).expect("metals").label.clone();
    let sale_line = format!(
        "Sold {} t of {label} to Kasei Metals at {}/t: {}; you receive {} when the goods are \
         in. Jonas Achterberg-Nkemelu countersigns.",
        grouped(qty),
        cr(price_t),
        cr(gross),
        cr(gross)
    );
    let trust_at_sale = posted(&rig.tw, trust_acct);
    let mine_at_sale = posted(&rig.tw, me);
    let counters = (
        graduated.sales,
        graduated.margin_minor,
        graduated.shares_minor,
    );
    let sold = run(&mut rig, vex, &format!("+market/sell {bid}={qty}")).await;
    assert!(
        sold.lines().any(|l| l == sale_line),
        "{sold}\nwanted: {sale_line}"
    );
    assert!(!sold.contains("Co-op share"), "{sold}");
    let leg = drain_players(&mut rig).await;
    assert_eq!(leg.len(), 1, "{sold}");
    land_all(&mut rig, &leg).await;
    for _ in 0..12 {
        let _ = sweep(&mut rig, 31).await;
    }
    assert_eq!(
        pool(&mut rig.tw, hold, "metals"),
        0,
        "the metals left the hold"
    );
    assert_eq!(
        pool(&mut rig.tw, hold, "volatiles"),
        outfit.ice,
        "the ice stays aboard"
    );
    assert_eq!(
        posted(&rig.tw, trust_acct),
        trust_at_sale,
        "the Trust is unmoved"
    );
    assert_eq!(
        posted(&rig.tw, me),
        mine_at_sale + gross,
        "Vex receives the gross"
    );
    let after = enrolment(&rig.tw, vex);
    assert_eq!(
        (after.sales, after.margin_minor, after.shares_minor),
        counters,
        "a graduate's sale folds nothing"
    );
    assert_eq!(after.status, EnrolmentStatus::Graduated);
    assert_no_pages(&rig.tw);
}

// ---------------------------------------------------------------------------
// 2. The refusals
// ---------------------------------------------------------------------------

/// **The verify lens, item 4** (lead ruling L14): a buyout while goods are moving, while
/// a queued step is at the counter, with a clearance filed, under way and docked at
/// Stickney each refuses, and nothing is written.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn ic13_a_buyout_is_refused_under_way_cleared_elsewhere_or_with_goods_moving() {
    let mut rig = boot("refusals").await;
    let _ = sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let vex = pilot(&mut rig, "Vex", "vexacct").await;
    let row = enrol(&mut rig, vex, "Salt Moth").await;
    let hull = ObjId(row.hull);
    top_up_to(&mut rig, vex, LENS_AVAILABLE).await;

    // Goods moving: one more Ceres ask accepted, its Draw queued but not landed.
    chandlery_from_registry(&mut rig, vex).await;
    let food = listing_of(&rig.tw, "ceres_port", "coop", "provisions");
    let quoted = run(&mut rig, vex, &format!("+market/quote {food}=1")).await;
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = newest_quote(&rig.tw, vex);
    let accepted = run(&mut rig, vex, &format!("+market/accept {quote}")).await;
    assert!(accepted.contains("Agreed."), "{accepted}");
    let draw = drain_players(&mut rig).await;
    assert_eq!(draw.len(), 1, "{accepted}");
    registry_from_chandlery(&mut rig, vex).await;
    let was = untouched(&rig.tw, vex);
    let out = run(&mut rig, vex, "+coop/buyout").await;
    let goods_wait = tacsim::port::buyout::GOODS_WAIT.replace("{name}", "Salt Moth");
    assert!(out.contains(&goods_wait), "{out}");
    assert_untouched(&rig.tw, vex, &was, "goods moving");
    land_all(&mut rig, &draw).await;
    let _ = sweep(&mut rig, 61).await;
    let _ = sweep(&mut rig, 61).await;

    // A queued step at the counter: a buy filed at the desk, staged by the order arm and
    // not yet landed.
    let filed = run(&mut rig, vex, "+ship/order buy provisions 1").await;
    let (_, queued) = fire(&mut rig).await;
    assert!(
        orders(&rig.tw, hull).inflight.is_some(),
        "the step is at the counter: {filed}"
    );
    let was = untouched(&rig.tw, vex);
    let out = run(&mut rig, vex, "+coop/buyout").await;
    let inflight = tacsim::port::buyout::INFLIGHT_WAIT.replace("{name}", "Salt Moth");
    assert!(out.contains(&inflight), "{out}");
    assert_untouched(&rig.tw, vex, &was, "a step at the counter");
    land_quietly(&mut rig, &queued).await;
    for _ in 0..20 {
        if orders(&rig.tw, hull).inflight.is_none() {
            break;
        }
        let _ = sweep(&mut rig, 61).await;
        let queued = drain_players(&mut rig).await;
        land_quietly(&mut rig, &queued).await;
    }
    assert!(
        orders(&rig.tw, hull).inflight.is_none(),
        "the step landed: {:#?}",
        orders(&rig.tw, hull)
    );
    let _ = sweep(&mut rig, 61).await;
    let _ = sweep(&mut rig, 61).await;

    // The outfit for Stickney (the planner's figures over everything aboard), then aboard.
    let _ = outfit(&mut rig, vex).await;
    set_rate(&mut rig, 3_600).await;
    board_from_chandlery(&mut rig, vex, "Salt Moth").await;

    // A filed clearance: aboard, after the undock and before the launch.
    let out = run(&mut rig, vex, "+ship/undock stickney").await;
    assert!(out.contains("Clearance filed for Stickney Yards"), "{out}");
    let burn = drain_players(&mut rig).await;
    assert_eq!(burn.len(), 1, "{out}");
    let was = untouched(&rig.tw, vex);
    let out = run(&mut rig, vex, "+coop/buyout").await;
    let cleared = tacsim::port::buyout::CLEARED_WAIT.replace("{name}", "Salt Moth");
    assert!(out.contains(&cleared), "{out}");
    assert_untouched(&rig.tw, vex, &was, "a clearance filed");

    // Under way.
    land_all(&mut rig, &burn).await;
    for _ in 0..4 {
        let _ = fire_quietly(&mut rig).await;
        if under_way(&rig.tw, hull) {
            break;
        }
    }
    assert!(under_way(&rig.tw, hull));
    let _ = fire_quietly(&mut rig).await;
    let was = untouched(&rig.tw, vex);
    let out = run(&mut rig, vex, "+coop/buyout").await;
    let away = tacsim::port::buyout::BUYOUT_UNDER_WAY
        .replace("{name}", "Salt Moth")
        .replace("{port}", "Ceres Freeport");
    assert!(out.contains(&away), "{out}");
    assert_untouched(&rig.tw, vex, &was, "under way");

    // Docked at Stickney.
    let ticket = ticket(&mut rig.tw, hull, 1).expect("the ticket");
    arrive(&mut rig, hull, &ticket).await;
    let was = untouched(&rig.tw, vex);
    let out = run(&mut rig, vex, "+coop/buyout").await;
    let elsewhere = tacsim::port::buyout::BUYOUT_NOT_HERE
        .replace("{name}", "Salt Moth")
        .replace("{port}", "Ceres Freeport");
    assert!(out.contains(&elsewhere), "{out}");
    assert_untouched(&rig.tw, vex, &was, "docked at Stickney");
    assert_eq!(title_owner(&rig.tw, hull), Some(trust(&rig.tw)));
    assert_no_pages(&rig.tw);
}

// ---------------------------------------------------------------------------
// 3. The refund
// ---------------------------------------------------------------------------

/// **The verify lens, item 2** (lead ruling L7): staff cancel the buyout before the sweep
/// settles it; the escrow comes back, the observer re-charters the same hull to the
/// pilot under a new lease, and a second buyout then graduates.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn ic13_a_refunded_buyout_re_leases() {
    let mut rig = boot("refund").await;
    let _ = sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let vex = pilot(&mut rig, "Vex", "vexacct").await;
    let row = enrol(&mut rig, vex, "Salt Moth").await;
    let hull = ObjId(row.hull);
    let hold = hold_of(&rig.tw, &row);
    let me = credit_account(&rig.tw, &PrincipalId::Object(vex));
    top_up_to(&mut rig, vex, LENS_AVAILABLE).await;
    let first = enrolment(&rig.tw, vex);
    let out = run(&mut rig, vex, "+coop/buyout").await;
    assert!(
        out.contains("The title passes to you at the next settlement."),
        "{out}"
    );
    let c = enrolment(&rig.tw, vex).buyout.expect("the buyout contract");
    assert_eq!(available(&rig.tw, me), LENS_AVAILABLE - first.buyout_minor);

    // Before any sweep: staff cancel it.
    let events = rig.tw.run(GOD, &format!("@contract/cancel #{c}"));
    flush(&mut rig).await;
    assert_eq!(contract_state(&rig.tw, c), ContractState::Cancelled);
    assert_eq!(
        available(&rig.tw, me),
        LENS_AVAILABLE,
        "the escrow came back"
    );
    let renewed = enrolment(&rig.tw, vex);
    assert_eq!(renewed.status, EnrolmentStatus::Active);
    assert_eq!(renewed.buyout, None);
    assert_ne!(renewed.lease, first.lease, "a new lease");
    assert_eq!(renewed.leased_at, rig.tw.world().now());
    assert_eq!(renewed.vault, first.vault);
    assert_eq!(
        contract_state(&rig.tw, renewed.lease),
        ContractState::Active
    );
    let charter = tacsim::port::live_charter(rig.tw.world(), vex).expect("a charter");
    assert_eq!(charter.contract.0, renewed.lease);
    assert_eq!(charter.hull, hull);
    let new_lease = economy(&rig.tw)
        .snapshot()
        .contract(ContractId(renewed.lease))
        .cloned()
        .expect("the new lease");
    assert!(
        new_lease
            .consent
            .parties
            .contains(&PrincipalId::Object(vex))
    );
    assert_eq!(new_lease.issuer, trust(&rig.tw));
    assert_eq!(title_owner(&rig.tw, hull), Some(trust(&rig.tw)));
    assert_eq!(hold_of(&rig.tw, &row), hold, "the same hold vault");
    assert_eq!(vault_owner(&rig.tw, hold), PrincipalId::Object(vex));
    let refunded = tacsim::port::buyout::BUYOUT_REFUNDED
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark)
        .replace("{buyout}", &cr(first.buyout_minor));
    assert!(
        said(&rig.tw, &events, vex).contains(&refunded),
        "{}",
        said(&rig.tw, &events, vex)
    );

    // A full charter again: she boards, and the hulls line reads her leased.
    gallery_from_registry(&mut rig, vex).await;
    let boarded = run(&mut rig, vex, "+ship/board").await;
    assert!(
        boarded.contains("You cycle the lock into the Salt Moth."),
        "{boarded}"
    );
    let line = hulls_line(&mut rig).await;
    assert!(
        line.contains(&format!(" 1 leased ({} to Vex)", row.mark)) && line.ends_with("[ready]"),
        "{line}"
    );
    let ashore = run(&mut rig, vex, "+ship/leave").await;
    let c01 = station_room(&rig.tw, "ceres_port", "room.c01_berth_gallery");
    assert_eq!(location(&rig.tw, vex), Some(c01), "{ashore}");

    // A second buyout graduates.
    registry_from_gallery(&mut rig, vex).await;
    let out = run(&mut rig, vex, "+coop/buyout").await;
    assert!(
        out.contains("The title passes to you at the next settlement."),
        "{out}"
    );
    let _ = sweep(&mut rig, 31).await;
    let graduated = enrolment(&rig.tw, vex);
    assert_eq!(graduated.status, EnrolmentStatus::Graduated);
    assert_eq!(title_owner(&rig.tw, hull), Some(PrincipalId::Object(vex)));
    assert_eq!(available(&rig.tw, me), LENS_AVAILABLE - first.buyout_minor);
    assert_no_pages(&rig.tw);
}

// ---------------------------------------------------------------------------
// 4. Dormancy
// ---------------------------------------------------------------------------

/// **The verify lens, item 3** (lead rulings L3, L4, L5, L15): seven idle real days
/// recall a docked hull with an empty cargo hold — at 7 days less an hour nothing
/// happens — and the next enrolment charters that very hull, its re-homed hold vault and
/// the fuel and stores the last captain left in it (owner item O1).
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn ic13_seven_idle_days_recall_a_docked_empty_hull_and_the_next_enrolment_charters_it() {
    const D: i64 = 7 * 86_400;
    let mut rig = boot("dormancy").await;
    let _ = sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let vex = pilot(&mut rig, "Vex", "vexacct").await;
    let row = enrol(&mut rig, vex, "Salt Moth").await;
    let hull = ObjId(row.hull);
    let hold = hold_of(&rig.tw, &row);
    chandlery_from_registry(&mut rig, vex).await;
    let fuel = listing_of(&rig.tw, "ceres_port", "occator", "propellant");
    let food = listing_of(&rig.tw, "ceres_port", "coop", "provisions");
    let _ = buy(&mut rig, vex, fuel, 5).await;
    let _ = buy(&mut rig, vex, food, 4).await;
    let _ = sweep(&mut rig, 61).await;
    let _ = sweep(&mut rig, 61).await;
    let (tank, locker) = (
        pool(&mut rig.tw, hold, "propellant"),
        pool(&mut rig.tw, hold, "provisions"),
    );
    assert_eq!((tank, locker), (5, 4));
    disconnect(&mut rig.tw, vex);
    let e = enrolment(&rig.tw, vex);
    let leased_at = e.leased_at;

    // Seven days less an hour: nothing.
    let now = rig.tw.world().now();
    let _ = sweep(&mut rig, leased_at + D - 3_600 - now).await;
    let _ = fire_quietly(&mut rig).await;
    assert_eq!(contract_state(&rig.tw, e.lease), ContractState::Active);
    assert_eq!(enrolment(&rig.tw, vex).status, EnrolmentStatus::Active);

    // Past seven days: the recall.
    let now = rig.tw.world().now();
    let _ = sweep(&mut rig, leased_at + D + 60 - now).await;
    let _ = fire_quietly(&mut rig).await;
    assert_eq!(contract_state(&rig.tw, e.lease), ContractState::Cancelled);
    let revoked = enrolment(&rig.tw, vex);
    assert_eq!(revoked.status, EnrolmentStatus::Revoked);
    assert_eq!(revoked.ended, Some(EndCause::Dormant));
    assert_eq!(hold_of(&rig.tw, &row), hold, "the same vault");
    assert_eq!(vault_owner(&rig.tw, hold), trust(&rig.tw));
    assert_eq!(
        (
            pool(&mut rig.tw, hold, "propellant"),
            pool(&mut rig.tw, hold, "provisions")
        ),
        (tank, locker),
        "the fuel and stores stay in the hold, forfeit with it"
    );
    assert_eq!(name_of(&rig.tw, hull), e.laid_up_name);
    assert_eq!(e.laid_up_name, "Slack Water");
    {
        let w = rig.tw.world();
        let port = tacsim::port::programme_port(w).expect("the programme port");
        assert!(
            tacsim::port::coop::free_hulls(w, &port)
                .iter()
                .any(|r| r.hull == hull.0),
            "the hull is back in the pool"
        );
    }
    let line = hulls_line(&mut rig).await;
    assert!(!line.contains("recalled"), "{line}");
    restart(&mut rig, |tw| {
        tw.world()
            .module_state::<TacsimStore>()
            .and_then(|s| s.enrolment_of(vex))
            .is_some_and(|e| e.status == EnrolmentStatus::Revoked)
    })
    .await;
    assert_eq!(enrolment(&rig.tw, vex), revoked);
    assert_eq!(vault_owner(&rig.tw, hold), trust(&rig.tw));
    assert_eq!(title_owner(&rig.tw, hull), Some(trust(&rig.tw)));
    // Vex reconnects to the notice.
    let events = rig.tw.connect(vex);
    flush(&mut rig).await;
    let heard = said(&rig.tw, &events, vex);
    let dormant = tacsim::port::recall::DORMANT_NOTICE
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark);
    let heading = heard
        .find(tacsim::port_tick::NOTICE_HEADING)
        .unwrap_or_else(|| panic!("the heading:\n{heard}"));
    assert!(
        heard[heading..].contains(&dormant),
        "{heard}\nwanted: {dormant}"
    );

    // Every gated line Ready again after the jump; the next captain charters that hull.
    stock(&mut rig).await;
    let ash = pilot(&mut rig, "Ash", "ashacct").await;
    let ash_row = enrol(&mut rig, ash, "Tern").await;
    assert_eq!(ash_row.hull, row.hull, "the very hull");
    assert_eq!(ash_row.mark, row.mark);
    assert_eq!(hold_of(&rig.tw, &ash_row), hold, "the same vault");
    assert_eq!(vault_owner(&rig.tw, hold), PrincipalId::Object(ash));
    let ash_e = enrolment(&rig.tw, ash);
    assert_eq!(ash_e.status, EnrolmentStatus::Active);
    assert_eq!(ash_e.vault, hold.0);
    let held = run(&mut rig, ash, "+ship/hold").await;
    let goods = goods();
    let label = |good: &str| goods.good(&cid(good)).expect("the good").label.clone();
    for want in [
        format!(
            "  TANKAGE {}, {tank} of {} t",
            label("propellant"),
            single_cap(&goods, "propellant", "propellant")
        ),
        format!(
            "  STORES {}, {locker} of {} crew-days",
            label("provisions"),
            single_cap(&goods, "provisions", "provisions")
        ),
    ] {
        assert!(held.contains(&want), "{held}\nwanted: {want}");
    }

    // Vex: refused for ever, and the charter reads ended.
    registry_from_chandlery(&mut rig, vex).await;
    let out = run(&mut rig, vex, "+coop/enrol Again").await;
    assert!(out.contains(tacsim::port::coop::ENROLLED_ONCE), "{out}");
    let terms = run(&mut rig, vex, "+coop").await;
    assert!(
        terms.contains("Your charter of the Salt Moth (FPC-D-0401) has ended."),
        "{terms}"
    );
    assert_no_pages(&rig.tw);
}

// ---------------------------------------------------------------------------
// 5. A departure keeps the charter
// ---------------------------------------------------------------------------

/// **Dormancy reads durable departures** (lead rulings L3, L12; lead 2f/2i): a burn
/// within the week keeps the charter whatever the lease's age; a hull gone a week since
/// her burn, docked at Stickney with cargo, cannot be recalled where she is — her plan is
/// held once (`HELD_DORMANT`), staff are paged once, and the lease stands.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn ic13_a_departure_keeps_the_charter_and_a_dormant_hull_with_cargo_holds_its_plan() {
    const D: i64 = 7 * 86_400;
    let mut rig = boot("departure").await;
    let _ = sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let vex = pilot(&mut rig, "Vex", "vexacct").await;
    let row = enrol(&mut rig, vex, "Salt Moth").await;
    let hull = ObjId(row.hull);
    let hold = hold_of(&rig.tw, &row);
    let outfit = outfit(&mut rig, vex).await;
    let e = enrolment(&rig.tw, vex);
    let leased_at = e.leased_at;

    // The clock moves on before the departure, so the burn falls well after the lease.
    let _ = sweep(&mut rig, 6 * 3_600).await;
    board_from_chandlery(&mut rig, vex, "Salt Moth").await;
    let out = run(&mut rig, vex, "+ship/undock stickney").await;
    assert!(out.contains("Clearance filed for Stickney Yards"), "{out}");
    let (burned_at, ticket) = launch(&mut rig, hull).await;
    assert!(burned_at >= leased_at + 6 * 3_600);
    arrive(&mut rig, hull, &ticket).await;

    // The waiting plan: ashore at Stickney, a depart filed from the gallery.
    let p01 = station_room(&rig.tw, "stickney_port", "room.p01_berth_gallery");
    let ashore = run(&mut rig, vex, "+ship/leave").await;
    assert_eq!(location(&rig.tw, vex), Some(p01), "{ashore}");
    let filed = run(&mut rig, vex, "+ship/order depart ceres at=now").await;
    let _ = fire_quietly(&mut rig).await;
    let doc = orders(&rig.tw, hull);
    let depart = doc
        .cursor
        .unwrap_or_else(|| panic!("a cursor: {filed}\n{doc:?}"));
    assert_eq!(doc.state, PlanState::Running, "{doc:?}");
    assert_eq!(
        doc.order(depart).map(|o| o.state),
        Some(OrderState::Pending)
    );
    assert!(
        doc.hold
            .as_ref()
            .is_some_and(|h| h.retry && h.order == depart),
        "a waiting note: {:?}",
        doc.hold
    );

    // More than a week after the lease, less than a week after the burn: nothing.
    let now = rig.tw.world().now();
    let _ = sweep(&mut rig, leased_at + D + 3_600 - now).await;
    let _ = fire_quietly(&mut rig).await;
    assert_eq!(contract_state(&rig.tw, e.lease), ContractState::Active);
    assert_eq!(enrolment(&rig.tw, vex).status, EnrolmentStatus::Active);
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.state, PlanState::Running);
    assert!(doc.hold.as_ref().is_some_and(|h| h.retry), "{:?}", doc.hold);
    assert!(tacsim::port_tick::recall_pages(rig.tw.world()).is_empty());

    // A week after the burn: dormant at Stickney with cargo — held and paged, not recalled.
    let now = rig.tw.world().now();
    let _ = sweep(&mut rig, burned_at + D + 3_600 - now).await;
    let _ = fire_quietly(&mut rig).await;
    assert_eq!(contract_state(&rig.tw, e.lease), ContractState::Active);
    let still = enrolment(&rig.tw, vex);
    assert_eq!(still.status, EnrolmentStatus::Active);
    assert_eq!(still.ended, None);
    let doc = orders(&rig.tw, hull);
    let reason = tacsim::port::recall::HELD_DORMANT
        .replace("{name}", "Salt Moth")
        .replace("{port}", "Ceres Freeport");
    assert_eq!(doc.state, PlanState::Held, "{doc:?}");
    assert!(
        doc.hold
            .as_ref()
            .is_some_and(|h| !h.retry && h.reason == reason && h.order == depart),
        "{:?}",
        doc.hold
    );
    let held_line = tacsim::port::executor::HELD_LINE
        .replace("{n}", &depart.to_string())
        .replace("{reason}", &reason);
    assert!(
        doc.log.iter().any(|l| l.text == held_line),
        "{:#?}",
        doc.log
    );
    assert_eq!(
        tacsim::port_tick::recall_pages(rig.tw.world()),
        vec![hull.0]
    );
    assert_eq!(
        pool(&mut rig.tw, hold, "metals"),
        outfit.metals,
        "the cargo stays"
    );
    assert_eq!(vault_owner(&rig.tw, hold), PrincipalId::Object(vex));

    // A second fire writes nothing new and pages nothing new.
    let _ = fire_quietly(&mut rig).await;
    assert_eq!(orders(&rig.tw, hull), doc);
    assert_eq!(
        tacsim::port_tick::recall_pages(rig.tw.world()),
        vec![hull.0]
    );
    assert_eq!(contract_state(&rig.tw, e.lease), ContractState::Active);
    assert_no_pages(&rig.tw);
}

// ---------------------------------------------------------------------------
// 6. Release and revoke
// ---------------------------------------------------------------------------

/// **`+coop/release` and `@coop/revoke` run the one recall** (lead rulings L9, L16; the
/// IC13 brief's reviewer F2 and F11): the pilot's release keeps the stake; staff's
/// revocation resolves the pilot anywhere in all three spellings, refuses a hull with
/// cargo, bounds its reason, and — on an empty hull — puts the pilot ashore, records and
/// tells the reason, and returns the Dray to the pool.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn ic13_release_and_revoke_recall_through_one_path() {
    let mut rig = boot("release").await;
    let _ = sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let vex = pilot(&mut rig, "Vex", "vexacct").await;
    let row = enrol(&mut rig, vex, "Salt Moth").await;
    let hold = hold_of(&rig.tw, &row);
    let me = credit_account(&rig.tw, &PrincipalId::Object(vex));
    chandlery_from_registry(&mut rig, vex).await;
    let food = listing_of(&rig.tw, "ceres_port", "coop", "provisions");
    let _ = buy(&mut rig, vex, food, 3).await;
    let _ = sweep(&mut rig, 61).await;
    let _ = sweep(&mut rig, 61).await;
    let locker = pool(&mut rig.tw, hold, "provisions");
    let c04 = station_room(&rig.tw, "ceres_port", "room.c04_concourse");
    let c05 = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    let c07 = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    walk(&mut rig, vex, "exchange", c07).await;
    walk(&mut rig, vex, "concourse", c04).await;
    let out = run(&mut rig, vex, "+coop/release").await;
    let where_ = tacsim::port::recall::RELEASE_WHERE
        .replace("{office}", &name_of(&rig.tw, c05))
        .replace("{port}", "Ceres Freeport");
    assert!(out.contains(&where_), "{out}\nwanted: {where_}");
    let c03 = station_room(&rig.tw, "ceres_port", "room.c03_arrivals");
    walk(&mut rig, vex, "arrivals", c03).await;
    walk(&mut rig, vex, "registry", c05).await;
    let posted_before = posted(&rig.tw, me);
    let out = run(&mut rig, vex, "+coop/release").await;
    let released = tacsim::port::recall::RELEASED
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark)
        .replace("{countersign}", tacsim::port::coop::COUNTERSIGN);
    let forfeit = tacsim::port::recall::FORFEIT_LINE
        .replace("{tank}", "0")
        .replace("{locker}", &locker.to_string());
    assert!(out.contains(&released), "{out}\nwanted: {released}");
    assert!(out.contains(&forfeit), "{out}\nwanted: {forfeit}");
    assert_eq!(posted(&rig.tw, me), posted_before, "the stake stays");
    let gone = enrolment(&rig.tw, vex);
    assert_eq!(gone.status, EnrolmentStatus::Revoked);
    assert_eq!(gone.ended, Some(EndCause::Released));
    assert_eq!(vault_owner(&rig.tw, hold), trust(&rig.tw));

    // Ash, with cargo aboard: every spelling of the pilot resolves, and the cargo refuses.
    stock(&mut rig).await;
    let ash = pilot(&mut rig, "Ash", "ashacct").await;
    let _ = enrol(&mut rig, ash, "Tern").await;
    let _ = outfit(&mut rig, ash).await;
    board_from_chandlery(&mut rig, ash, "Tern").await;
    assert_ne!(location(&rig.tw, GOD), location(&rig.tw, ash));
    let was = untouched(&rig.tw, ash);
    let cargo = tacsim::port::recall::RECALL_CARGO.replace("{name}", "Tern");
    for spelling in ["Ash".to_owned(), "*Ash".to_owned(), format!("#{}", ash.0)] {
        let out = run(
            &mut rig,
            GOD,
            &format!("@coop/revoke {spelling}=testing the recall"),
        )
        .await;
        assert_eq!(out, format!("Not revoked: {cargo}"), "{spelling}");
        assert_untouched(&rig.tw, ash, &was, &spelling);
    }
    let out = run(&mut rig, vex, "@coop/revoke Ash=testing the recall").await;
    assert_eq!(out, "Permission denied.");
    for (line, want) in [
        ("@coop/revoke Ash=", tacsim::port::recall::REVOKE_WHY_BOUNDS),
        (
            "@coop/revoke Ash=   ",
            tacsim::port::recall::REVOKE_WHY_BOUNDS,
        ),
        ("@coop/revoke Ash", tacsim::port::recall::REVOKE_USAGE),
        ("@coop/revoke =x", tacsim::port::recall::REVOKE_USAGE),
        ("@coop/revoke Nobody=x", tacsim::port::recall::NO_SUCH_PILOT),
    ] {
        let out = run(&mut rig, GOD, line).await;
        assert_eq!(out, want, "{line}");
    }
    let out = run(&mut rig, GOD, &format!("@coop/revoke #{}=x", c05.0)).await;
    assert_eq!(out, tacsim::port::recall::NO_SUCH_PILOT);
    let too_long = "x".repeat(tacsim::port::coop::MAX_REVOKE_WHY_BYTES + 1);
    let out = run(&mut rig, GOD, &format!("@coop/revoke Ash={too_long}")).await;
    assert_eq!(out, tacsim::port::recall::REVOKE_WHY_BOUNDS);
    assert_untouched(&rig.tw, ash, &was, "the refusals");

    // Bo, aboard an empty hull docked at Ceres: revoked, put ashore, told, returned.
    stock(&mut rig).await;
    let bo = pilot(&mut rig, "Bo", "boacct").await;
    let bo_row = enrol(&mut rig, bo, "Heron").await;
    let bo_hull = ObjId(bo_row.hull);
    let bo_hold = hold_of(&rig.tw, &bo_row);
    gallery_from_registry(&mut rig, bo).await;
    let boarded = run(&mut rig, bo, "+ship/board").await;
    assert!(
        boarded.contains("You cycle the lock into the Heron."),
        "{boarded}"
    );
    let c01 = station_room(&rig.tw, "ceres_port", "room.c01_berth_gallery");
    // Boarding out of C01 already stamped Ceres (IC1's observer), so Bo is given a
    // Stickney stamp first: the recall's put-ashore must move it to Ceres and clear
    // Stickney, or this proves nothing.
    let marks = |tw: &TestWorld| {
        (
            tw.world().has_marker(bo, "tacsim.last_port.ceres"),
            tw.world().has_marker(bo, "tacsim.last_port.stickney"),
        )
    };
    let p01 = station_room(&rig.tw, "stickney_port", "room.p01_berth_gallery");
    let stickney =
        tacsim::ic_return::port_of_room(rig.tw.world(), p01).expect("Stickney is an IC port");
    tacsim::ic_return::stamp(rig.tw.world_mut(), bo, stickney).expect("the Stickney stamp");
    assert_eq!(marks(&rig.tw), (false, true), "stamped at Stickney");
    let events = rig.tw.run(GOD, "@coop/revoke Bo=rules breach");
    flush(&mut rig).await;
    let staff = said(&rig.tw, &events, GOD);
    let reply = tacsim::port::recall::REVOKED_REPLY
        .replace("{name}", "Heron")
        .replace("{mark}", &bo_row.mark)
        .replace("{pilot}", "Bo")
        .replace("{why}", "rules breach");
    assert!(staff.contains(&reply), "{staff}\nwanted: {reply}");
    assert_eq!(
        location(&rig.tw, bo),
        Some(c01),
        "put ashore at the gallery"
    );
    assert_eq!(
        marks(&rig.tw),
        (true, false),
        "the recall's put-ashore stamped Ceres and cleared Stickney"
    );
    let heard = said(&rig.tw, &events, bo);
    let put_ashore = tacsim::port::recall::PUT_ASHORE.replace("{name}", "Heron");
    assert_eq!(heard.matches(&put_ashore).count(), 1, "{heard}");
    let everyone = all_texts(&events);
    assert_eq!(
        everyone.matches("Bo comes ashore from the Heron.").count(),
        1,
        "{everyone}"
    );
    let notice = tacsim::port::recall::REVOKED_NOTICE
        .replace("{name}", "Heron")
        .replace("{mark}", &bo_row.mark)
        .replace("{why}", "rules breach");
    assert!(heard.contains(&notice), "{heard}\nwanted: {notice}");
    let bo_e = enrolment(&rig.tw, bo);
    assert_eq!(bo_e.status, EnrolmentStatus::Revoked);
    assert_eq!(
        bo_e.ended,
        Some(EndCause::Revoked {
            by: GOD,
            why: "rules breach".to_owned(),
        })
    );
    assert_eq!(vault_owner(&rig.tw, bo_hold), trust(&rig.tw));
    assert_eq!(name_of(&rig.tw, bo_hull), bo_e.laid_up_name);
    {
        let w = rig.tw.world();
        let port = tacsim::port::programme_port(w).expect("the programme port");
        assert!(
            tacsim::port::coop::free_hulls(w, &port)
                .iter()
                .any(|r| r.hull == bo_hull.0),
            "the Dray is back in the pool"
        );
    }
    assert_eq!(row_of_hull(&rig.tw, bo_hull).mark, bo_row.mark);
    assert_no_pages(&rig.tw);
}

// ---------------------------------------------------------------------------
// 7. A queued order across the buyout
// ---------------------------------------------------------------------------

/// **The order arm across the buyout window, then on the title** (lead ruling L2; owner
/// item O6): a queued buy due between `+coop/buyout` and the settlement waits under the
/// buyout's own note (`HELD_BUYOUT`, logged and told once — never the lapsed charter's
/// `HELD_CHARTER`), stages nothing, and once the title has passed the note drops
/// silently and the order runs with the graduate as its pilot (`helm_of`'s Title arm at
/// the executor's per-step gate).
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn ic13_a_queued_order_waits_out_the_buyout_and_runs_on_the_title() {
    let mut rig = boot("window").await;
    let _ = sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let vex = pilot(&mut rig, "Vex", "vexacct").await;
    let row = enrol(&mut rig, vex, "Salt Moth").await;
    let hull = ObjId(row.hull);
    let hold = hold_of(&rig.tw, &row);
    top_up_to(&mut rig, vex, LENS_AVAILABLE).await;
    let food_before = pool(&mut rig.tw, hold, "provisions");

    // A sweep first, so the economy's next settlement is a full period away; then, with
    // no fire between them, the order and the buyout.
    let _ = sweep(&mut rig, 31).await;
    let filed = run(&mut rig, vex, "+ship/order buy provisions 1").await;
    let doc = orders(&rig.tw, hull);
    let order = doc
        .cursor
        .unwrap_or_else(|| panic!("a cursor: {filed}\n{doc:?}"));
    assert_eq!(doc.state, PlanState::Running, "{filed}");
    let out = run(&mut rig, vex, "+coop/buyout").await;
    assert!(
        out.contains("The title passes to you at the next settlement."),
        "{out}"
    );
    assert_eq!(
        enrolment(&rig.tw, vex).status,
        EnrolmentStatus::BuyoutPending
    );

    // The window: the step waits under the buyout's note, told once, nothing staged.
    let (events, queued) = fire(&mut rig).await;
    assert!(queued.is_empty(), "nothing staged in the window");
    assert_eq!(
        enrolment(&rig.tw, vex).status,
        EnrolmentStatus::BuyoutPending,
        "still inside the window"
    );
    let reason = tacsim::port::executor::HELD_BUYOUT.replace("{name}", "Salt Moth");
    let waits = tacsim::port::executor::WAITING_LINE
        .replace("{n}", &order.to_string())
        .replace("{reason}", &reason);
    let doc = orders(&rig.tw, hull);
    assert_eq!(doc.state, PlanState::Running);
    assert_eq!(doc.order(order).map(|o| o.state), Some(OrderState::Pending));
    assert!(
        doc.hold
            .as_ref()
            .is_some_and(|h| h.retry && h.order == order && h.reason == reason),
        "{:?}",
        doc.hold
    );
    assert_eq!(
        doc.log.iter().filter(|l| l.text == waits).count(),
        1,
        "{:#?}",
        doc.log
    );
    let charter = tacsim::port::executor::HELD_CHARTER.replace("{name}", "Salt Moth");
    assert!(
        !doc.log.iter().any(|l| l.text.contains(&charter)),
        "the buyout is not a lapsed charter: {:#?}",
        doc.log
    );
    let heard = said(&rig.tw, &events, vex);
    assert_eq!(heard.matches(&waits).count(), 1, "{heard}");
    // A second fire in the window writes and tells nothing new.
    let (events, queued) = fire(&mut rig).await;
    assert!(queued.is_empty());
    assert_eq!(orders(&rig.tw, hull), doc);
    assert!(!said(&rig.tw, &events, vex).contains(&waits));

    // The settlement graduates; the order then runs on the title.
    let _ = sweep(&mut rig, 31).await;
    assert_eq!(enrolment(&rig.tw, vex).status, EnrolmentStatus::Graduated);
    assert_eq!(title_owner(&rig.tw, hull), Some(PrincipalId::Object(vex)));
    for _ in 0..20 {
        if orders(&rig.tw, hull)
            .order(order)
            .is_some_and(|o| o.state == OrderState::Done)
        {
            break;
        }
        let _ = fire_quietly(&mut rig).await;
        let _ = sweep(&mut rig, 31).await;
        let queued = drain_players(&mut rig).await;
        land_quietly(&mut rig, &queued).await;
    }
    let doc = orders(&rig.tw, hull);
    assert_eq!(
        doc.order(order).map(|o| o.state),
        Some(OrderState::Done),
        "{doc:#?}"
    );
    assert!(doc.hold.is_none(), "the note dropped: {:?}", doc.hold);
    assert_eq!(
        doc.log.iter().filter(|l| l.text == waits).count(),
        1,
        "the note dropped without a second line"
    );
    assert_eq!(pool(&mut rig.tw, hold, "provisions"), food_before + 1);
    assert_no_pages(&rig.tw);
}

// ---------------------------------------------------------------------------
// Doc 18 cut IC14 — the convergence residuals of IC13 (rows 4, 12, 32–34)
// ---------------------------------------------------------------------------

/// The Trust's display name, read from the shipped Ceres entry (never typed).
fn trust_name(_tw: &TestWorld) -> String {
    let def = port_def("ceres_port");
    let programme = def.programme.as_ref().expect("the programme");
    def.orgs
        .iter()
        .find(|o| o.id == programme.org)
        .map(|o| o.name.clone())
        .expect("the Trust's entry")
}

/// **Write documents straight into the kept store** (the brief's "writing the document
/// directly in the folded store and rebooting"): the legacy work flushed, one batch of
/// `JsonSet`s in `ns`, then a restart reads them back. A test-side injection of a state
/// no staff act reaches; never a production switch.
async fn write_docs(rig: &mut Rig, ns: &str, docs: Vec<(smol_str::SmolStr, serde_json::Value)>) {
    flush(rig).await;
    let mut batch = persist::batch_from_dirty(rig.tw.world(), BTreeSet::new(), rig.serial, true);
    batch.module_ops = docs
        .into_iter()
        .map(|(key, value)| mush_core::ModuleOp {
            ns: smol_str::SmolStr::new(ns),
            op: mush_core::ModuleWrite::JsonSet { key, value },
        })
        .collect();
    let receipt = rig.store.write(batch).await.expect("the batch commits");
    assert_eq!(receipt.serial, rig.serial);
    rig.serial = rig.serial.saturating_add(1);
    restart(rig, |_| true).await;
}

/// Freeze an economy account through its stored document (no staff verb freezes one;
/// `@account/unfreeze` lifts it).
async fn freeze_account(rig: &mut Rig, account: mush_economy::AccountId) {
    let mut doc = serde_json::to_value(
        economy(&rig.tw)
            .snapshot()
            .account(account)
            .cloned()
            .expect("the account"),
    )
    .expect("an account document");
    doc["status"] = serde_json::json!("Frozen");
    write_docs(
        rig,
        mush_economy::NS,
        vec![(mush_economy::account_key(account), doc)],
    )
    .await;
    assert_eq!(
        economy(&rig.tw)
            .snapshot()
            .account(account)
            .map(|a| a.status),
        Some(mush_economy::AccountStatus::Frozen),
        "the account reads frozen after the restart"
    );
}

/// How many of `who`'s queued reconnect notices are exactly `line`.
fn notices_of(tw: &TestWorld, who: ObjId, line: &str) -> usize {
    tacsim::port_tick::pending_notices(tw.world(), who)
        .iter()
        .filter(|l| l.as_str() == line)
        .count()
}

/// **Doc 18 cut IC14 row 4 (lead ruling L10): a Revoked charter with a live lease
/// converges.** The state a recall whose `end_lease` returned `Err` leaves — the
/// enrolment Revoked, its lease still Active — written into the store and booted; the
/// recall arm retries `end_lease`: the lease is Cancelled, the hull goes back to the
/// pool (its hold the Trust's), the pilot is told once and holds no helm.
#[tokio::test]
async fn ic14_a_revoked_charter_with_a_live_lease_converges() {
    let mut rig = boot("livelease").await;
    let _ = sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let vex = pilot(&mut rig, "Vex", "vexacct").await;
    let row = enrol(&mut rig, vex, "Salt Moth").await;
    let hold = hold_of(&rig.tw, &row);
    let e = enrolment(&rig.tw, vex);
    let revoked = Enrolment {
        status: EnrolmentStatus::Revoked,
        ended_at: Some(rig.tw.world().now()),
        ended: Some(EndCause::Dormant),
        ..e.clone()
    };
    write_docs(
        &mut rig,
        tacsim::NS,
        vec![(
            tacsim::port::coop::enrolment_key(e.account),
            serde_json::to_value(&revoked).expect("an enrolment document"),
        )],
    )
    .await;
    assert_eq!(enrolment(&rig.tw, vex).status, EnrolmentStatus::Revoked);
    assert_eq!(
        contract_state(&rig.tw, e.lease),
        ContractState::Active,
        "the lease is still live"
    );
    for _ in 0..3 {
        let _ = fire_quietly(&mut rig).await;
    }
    assert_eq!(contract_state(&rig.tw, e.lease), ContractState::Cancelled);
    assert_eq!(
        vault_owner(&rig.tw, hold),
        trust(&rig.tw),
        "back in the pool"
    );
    assert!(tacsim::port::helm_of(rig.tw.world(), vex).is_none());
    let returned = tacsim::port::recall::RETURNED_NOTICE
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark);
    assert_eq!(notices_of(&rig.tw, vex, &returned), 1, "told once");
    for _ in 0..3 {
        let _ = fire_quietly(&mut rig).await;
    }
    assert_eq!(notices_of(&rig.tw, vex, &returned), 1, "and only once");
    assert_no_pages(&rig.tw);
}

/// Steps shared by the two re-charter tests: a pilot with a queued buy, the buyout
/// entered, the Trust's account frozen (so the re-charter's lease offer refuses: its
/// rent names that account), and staff cancelling the buyout. Returns the pilot, the
/// row, the plan's order and the pre-buyout enrolment.
async fn refunded_with_the_trust_frozen(rig: &mut Rig) -> (ObjId, HullRow, u32, Enrolment) {
    let _ = sweep(rig, 120).await;
    stock(rig).await;
    let vex = pilot(rig, "Vex", "vexacct").await;
    let row = enrol(rig, vex, "Salt Moth").await;
    let hull = ObjId(row.hull);
    top_up_to(rig, vex, LENS_AVAILABLE).await;
    let first = enrolment(&rig.tw, vex);
    let _ = sweep(rig, 31).await;
    let filed = run(rig, vex, "+ship/order buy provisions 1").await;
    let order = orders(&rig.tw, hull)
        .cursor
        .unwrap_or_else(|| panic!("a cursor: {filed}"));
    let out = run(rig, vex, "+coop/buyout").await;
    assert!(
        out.contains("The title passes to you at the next settlement."),
        "{out}"
    );
    let c = enrolment(&rig.tw, vex).buyout.expect("the buyout contract");
    freeze_account(rig, trust_account(&rig.tw)).await;
    let _ = rig.tw.run(GOD, &format!("@contract/cancel #{c}"));
    flush(rig).await;
    assert_eq!(contract_state(&rig.tw, c), ContractState::Cancelled);
    let pending = enrolment(&rig.tw, vex);
    assert_eq!(
        pending.status,
        EnrolmentStatus::BuyoutPending,
        "the failed re-charter leaves the enrolment buyout-pending"
    );
    let relet = tacsim::port::buyout::RELET_PENDING
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark)
        .replace("{buyout}", &cr(first.buyout_minor));
    assert_eq!(notices_of(&rig.tw, vex, &relet), 1, "told once");
    (vex, row, order, first)
}

/// **Doc 18 cut IC14 row 12 (lead ruling L11; proposed Q-18-71): a failed re-charter
/// retries until the pilot is chartered.** The observer's first re-charter fails (the
/// Trust's account frozen, so the lease offer refuses), the pilot is told
/// `RELET_PENDING` once, the recall arm pages once and the queued plan waits under
/// `HELD_RELET`; staff lift the freeze and the next due retry re-charters her — Active
/// on a new lease, `BUYOUT_REFUNDED` told once, and no Revoked state ever written.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn ic14_a_failed_relet_retries_until_the_pilot_is_chartered() {
    let mut rig = boot("relet").await;
    let (vex, row, order, first) = refunded_with_the_trust_frozen(&mut rig).await;
    let hull = ObjId(row.hull);
    let hold = hold_of(&rig.tw, &row);
    let food_before = pool(&mut rig.tw, hold, "provisions");
    // First sight: the arm records the observer's attempt and pages; the next fire the
    // order arm holds the step under the re-charter's own note.
    let _ = fire_quietly(&mut rig).await;
    assert_eq!(
        tacsim::port_tick::recall_pages(rig.tw.world()),
        vec![hull.0]
    );
    let _ = fire_quietly(&mut rig).await;
    let reason = tacsim::port::executor::HELD_RELET.replace("{name}", "Salt Moth");
    let doc = orders(&rig.tw, hull);
    assert!(
        doc.hold
            .as_ref()
            .is_some_and(|h| h.retry && h.order == order && h.reason == reason),
        "{:?}",
        doc.hold
    );
    let waits = tacsim::port::executor::WAITING_LINE
        .replace("{n}", &order.to_string())
        .replace("{reason}", &reason);
    assert_eq!(doc.log.iter().filter(|l| l.text == waits).count(), 1);
    // Staff lift the freeze; the next due retry re-charters.
    let trust_acct = trust_account(&rig.tw);
    let out = run(&mut rig, GOD, &format!("@account/unfreeze {trust_acct}")).await;
    assert!(out.contains("unfrozen"), "{out}");
    let mut chartered = false;
    for _ in 0..40 {
        let _ = fire_quietly(&mut rig).await;
        let e = enrolment(&rig.tw, vex);
        assert_ne!(e.status, EnrolmentStatus::Revoked, "never revoked");
        if e.status == EnrolmentStatus::Active {
            chartered = true;
            break;
        }
    }
    assert!(chartered, "re-chartered within the backoff");
    let renewed = enrolment(&rig.tw, vex);
    assert_ne!(renewed.lease, first.lease, "a new lease");
    assert_eq!(
        contract_state(&rig.tw, renewed.lease),
        ContractState::Active
    );
    let refunded = tacsim::port::buyout::BUYOUT_REFUNDED
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark)
        .replace("{buyout}", &cr(first.buyout_minor));
    assert_eq!(
        notices_of(&rig.tw, vex, &refunded),
        1,
        "BUYOUT_REFUNDED once"
    );
    let relet = tacsim::port::buyout::RELET_PENDING
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark)
        .replace("{buyout}", &cr(first.buyout_minor));
    assert_eq!(notices_of(&rig.tw, vex, &relet), 1, "RELET_PENDING once");
    assert_eq!(
        tacsim::port_tick::recall_pages(rig.tw.world()),
        vec![hull.0],
        "one page"
    );
    // Brief B3: the silent re-check drops `HELD_RELET`'s note once she is chartered
    // again, and the queued buy runs on the renewed charter.
    for _ in 0..20 {
        if orders(&rig.tw, hull)
            .order(order)
            .is_some_and(|o| o.state == OrderState::Done)
        {
            break;
        }
        let _ = fire_quietly(&mut rig).await;
        let _ = sweep(&mut rig, 31).await;
        let queued = drain_players(&mut rig).await;
        land_quietly(&mut rig, &queued).await;
    }
    let doc = orders(&rig.tw, hull);
    assert_eq!(
        doc.order(order).map(|o| o.state),
        Some(OrderState::Done),
        "{doc:#?}"
    );
    assert!(doc.hold.is_none(), "HELD_RELET dropped: {:?}", doc.hold);
    assert_eq!(
        doc.log.iter().filter(|l| l.text == waits).count(),
        1,
        "the note dropped without a second line"
    );
    assert_eq!(pool(&mut rig.tw, hold, "provisions"), food_before + 1);
}

/// **Doc 18 cut IC14 row 12: a re-charter that never lands gives up after six.** The
/// freeze is never lifted: the arm makes the observer's attempt plus five retries, each
/// after the doubling backoff (24, 48, 96, 192 and 384 fires), and only then ends the
/// enrolment exactly as cut IC13 built it — Revoked, `EndCause::LeaseEnded`,
/// `BUYOUT_UNLEASED` told once. The attempts are read from the arm's own record
/// (`port_tick::relet_attempts`) every fire: it counts 1 (the observer's, recorded at
/// first sight) through 5, each step due exactly its backoff after the last, and the
/// sixth attempt gives up and clears it — exactly `MAX_RELET_ATTEMPTS` re-charters.
/// Staff are paged at first sight through the recall arm's one latch, and the give-up
/// arm logs its own error. A refused offer mints no lease, so no lease contract was
/// made at all.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn ic14_a_relet_that_never_lands_gives_up_after_six() {
    let mut rig = boot("relet6").await;
    let (vex, row, _order, first) = refunded_with_the_trust_frozen(&mut rig).await;
    let hull = ObjId(row.hull);
    let contracts = contract_count(&rig.tw);
    assert_eq!(
        tacsim::port_tick::relet_attempts(rig.tw.world(), hull),
        None,
        "the arm has not seen the hull yet"
    );
    let _ = fire_quietly(&mut rig).await;
    let sighted = tacsim::port_tick::fires(rig.tw.world());
    assert_eq!(
        tacsim::port_tick::relet_attempts(rig.tw.world(), hull),
        Some((1, sighted + tacsim::port::buyout::relet_backoff(1))),
        "first sight records the observer's attempt"
    );
    assert_eq!(
        tacsim::port_tick::recall_pages(rig.tw.world()),
        vec![hull.0],
        "and pages once"
    );
    let wait: u64 = [24_u64, 48, 96, 192, 384].iter().sum();
    // Every change of the arm's record, in order: (attempts made, the fire it changed).
    let mut seen: Vec<(u8, u64)> = vec![(1, sighted)];
    let mut revoked_at = None;
    for _ in 0..(wait + 40) {
        let _ = fire_quietly(&mut rig).await;
        let at = tacsim::port_tick::fires(rig.tw.world());
        match tacsim::port_tick::relet_attempts(rig.tw.world(), hull) {
            Some((made, due)) => {
                if seen.last().map(|(m, _)| *m) != Some(made) {
                    assert_eq!(
                        due,
                        at + tacsim::port::buyout::relet_backoff(made),
                        "attempt {made} is due its backoff on"
                    );
                    seen.push((made, at));
                }
            }
            None => {
                assert_eq!(
                    enrolment(&rig.tw, vex).status,
                    EnrolmentStatus::Revoked,
                    "the record clears only when the arm gives up"
                );
                revoked_at = Some(at);
                break;
            }
        }
        assert_eq!(
            enrolment(&rig.tw, vex).status,
            EnrolmentStatus::BuyoutPending,
            "buyout-pending through every retry"
        );
    }
    let revoked_at = revoked_at.expect("the arm gave up");
    let made: Vec<u8> = seen.iter().map(|(m, _)| *m).collect();
    assert_eq!(
        made,
        (1..tacsim::port::buyout::MAX_RELET_ATTEMPTS).collect::<Vec<u8>>(),
        "five attempts that stayed pending; the sixth gave up"
    );
    for pair in seen.windows(2) {
        let ((before, at), (_, next)) = (pair[0], pair[1]);
        assert_eq!(
            next - at,
            tacsim::port::buyout::relet_backoff(before),
            "each retry waits its doubling backoff: {seen:?}"
        );
    }
    assert!(
        revoked_at >= sighted + wait,
        "not before the five backoffs: sighted at {sighted}, revoked at {revoked_at}"
    );
    let e = enrolment(&rig.tw, vex);
    assert_eq!(e.ended, Some(EndCause::LeaseEnded));
    assert_eq!(contract_count(&rig.tw), contracts, "no lease was minted");
    let unleased = tacsim::port::buyout::BUYOUT_UNLEASED
        .replace("{name}", "Salt Moth")
        .replace("{mark}", &row.mark)
        .replace("{buyout}", &cr(first.buyout_minor));
    assert_eq!(
        notices_of(&rig.tw, vex, &unleased),
        1,
        "BUYOUT_UNLEASED once"
    );
    for _ in 0..30 {
        let _ = fire_quietly(&mut rig).await;
    }
    assert_eq!(notices_of(&rig.tw, vex, &unleased), 1, "and never again");
    assert_eq!(
        tacsim::port_tick::relet_attempts(rig.tw.world(), hull),
        None,
        "and the arm never tries again"
    );
    assert_eq!(
        tacsim::port_tick::recall_pages(rig.tw.world()),
        vec![hull.0],
        "the first-sight page is the recall arm's one latch"
    );
}

/// **Doc 18 cut IC14 row 32: the buyout's failure residues — the one a staff act
/// reaches.** The pilot's escrow refused after the offer (an approval rule on her
/// account, a real staff act): the verb cancels its own contract through
/// `cancel_own`, answers `BUYOUT_FAILED`, and the charter stands — no BuyoutPending,
/// the lease Active, the buyout contract Cancelled, nothing held. The other residues
/// (`end_lease` Retrying → `BUYOUT_HELD`, `end_lease` Err, the Trust's acceptance not
/// activating) are not reachable on the production root without a production change
/// (cut IC14 report, NOT DONE); `recall_outcome`'s unit test pins the Retrying/Err
/// mapping the recall shares.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn ic14_the_buyout_residues() {
    let mut rig = boot("residues").await;
    let _ = sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let vex = pilot(&mut rig, "Vex", "vexacct").await;
    let row = enrol(&mut rig, vex, "Salt Moth").await;
    let hull = ObjId(row.hull);
    top_up_to(&mut rig, vex, LENS_AVAILABLE).await;
    let me = credit_account(&rig.tw, &PrincipalId::Object(vex));
    let first = enrolment(&rig.tw, vex);

    // (a) The escrow refused after the offer.
    let out = run(
        &mut rig,
        GOD,
        &format!("@account/approval {me}=100 threshold:1"),
    )
    .await;
    assert!(!out.contains("Usage"), "{out}");
    let before = contract_count(&rig.tw);
    let out = run(&mut rig, vex, "+coop/buyout").await;
    assert!(out.contains(tacsim::port::buyout::BUYOUT_FAILED), "{out}");
    let e = enrolment(&rig.tw, vex);
    assert_eq!(e.status, EnrolmentStatus::Active, "no BuyoutPending");
    assert_eq!(e.buyout, None);
    assert_eq!(contract_state(&rig.tw, e.lease), ContractState::Active);
    assert_eq!(contract_count(&rig.tw), before + 1, "the offer was made");
    let offered = economy(&rig.tw)
        .snapshot()
        .contracts()
        .map(|c| c.id)
        .max()
        .expect("the buyout contract");
    assert_eq!(
        contract_state(&rig.tw, offered.0),
        ContractState::Cancelled,
        "cancelled through its own cancel"
    );
    assert_eq!(available(&rig.tw, me), LENS_AVAILABLE, "nothing held");
    let out = run(&mut rig, GOD, &format!("@account/approval {me}=none")).await;
    assert!(out.contains("Cleared"), "{out}");
    // With the rule cleared the same buyout goes through: the residue blocked nothing.
    let out = run(&mut rig, vex, "+coop/buyout").await;
    assert!(
        out.contains("The title passes to you at the next settlement."),
        "{out}"
    );
    assert_eq!(
        enrolment(&rig.tw, vex).status,
        EnrolmentStatus::BuyoutPending
    );
    assert_eq!(available(&rig.tw, me), LENS_AVAILABLE - first.buyout_minor);
    let _ = hull;
}

/// **Doc 18 cut IC14 row 33: a blocked recall holds the plan once and pages once.** A
/// dormant hull docked at Stickney with cargo cannot be recalled; the arm holds her
/// plan (`HELD_DORMANT`) and pages once — and on each re-entry past
/// `RECALL_RECHECK_FIRES` writes nothing (`hold_plan`'s written-once rule), pages
/// nothing (`page_once`) and re-arms its backoff.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn ic14_a_blocked_recall_holds_the_plan_once_and_pages_once() {
    const D: i64 = 7 * 86_400;
    let mut rig = boot("reentry").await;
    let _ = sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let vex = pilot(&mut rig, "Vex", "vexacct").await;
    let row = enrol(&mut rig, vex, "Salt Moth").await;
    let hull = ObjId(row.hull);
    let _outfit = outfit(&mut rig, vex).await;
    board_from_chandlery(&mut rig, vex, "Salt Moth").await;
    let out = run(&mut rig, vex, "+ship/undock stickney").await;
    assert!(out.contains("Clearance filed for Stickney Yards"), "{out}");
    let (burned_at, ticket) = launch(&mut rig, hull).await;
    arrive(&mut rig, hull, &ticket).await;
    let p01 = station_room(&rig.tw, "stickney_port", "room.p01_berth_gallery");
    let ashore = run(&mut rig, vex, "+ship/leave").await;
    assert_eq!(location(&rig.tw, vex), Some(p01), "{ashore}");
    let _ = run(&mut rig, vex, "+ship/order depart ceres at=now").await;
    let _ = fire_quietly(&mut rig).await;
    let now = rig.tw.world().now();
    let _ = sweep(&mut rig, burned_at + D + 3_600 - now).await;
    let _ = fire_quietly(&mut rig).await;
    let held = orders(&rig.tw, hull);
    assert_eq!(held.state, PlanState::Held, "{held:?}");
    assert_eq!(
        tacsim::port_tick::recall_pages(rig.tw.world()),
        vec![hull.0]
    );
    let mut blocked =
        tacsim::port_tick::recall_blocked_at(rig.tw.world(), hull.0).expect("the backoff is armed");
    for _ in 0..2 {
        for _ in 0..(tacsim::port::recall::RECALL_RECHECK_FIRES + 1) {
            let _ = fire_quietly(&mut rig).await;
        }
        assert_eq!(orders(&rig.tw, hull), held, "written once");
        assert_eq!(
            tacsim::port_tick::recall_pages(rig.tw.world()),
            vec![hull.0],
            "paged once"
        );
        let again = tacsim::port_tick::recall_blocked_at(rig.tw.world(), hull.0).expect("re-armed");
        assert!(
            again > blocked,
            "the backoff re-armed: {blocked} -> {again}"
        );
        blocked = again;
    }
    assert_eq!(enrolment(&rig.tw, vex).status, EnrolmentStatus::Active);
}

/// **Doc 18 cut IC14 row 34: the recalled hulls line on the production root, byte for
/// byte.** Staff cancel a charter's lease; until the return arm's next fire the hulls
/// line reads `, 1 recalled, not yet returned ({mark})` exactly; then she is back.
#[tokio::test]
async fn ic14_the_recalled_hulls_line_on_the_production_root() {
    let mut rig = boot("recalledline").await;
    let _ = sweep(&mut rig, 120).await;
    stock(&mut rig).await;
    let vex = pilot(&mut rig, "Vex", "vexacct").await;
    let row = enrol(&mut rig, vex, "Salt Moth").await;
    let hold = hold_of(&rig.tw, &row);
    let e = enrolment(&rig.tw, vex);
    let _ = run(&mut rig, GOD, &format!("@contract/cancel #{}", e.lease)).await;
    assert_eq!(enrolment(&rig.tw, vex).status, EnrolmentStatus::Revoked);
    let made = port_map(&rig.tw, "ceres_port").hulls.len();
    let line = hulls_line(&mut rig).await;
    assert_eq!(
        line,
        format!(
            "ceres_port hulls: {made} of {made} made and titled to {}; {} laid up and free, 0 \
             leased, 1 recalled, not yet returned ({}). [ready]",
            trust_name(&rig.tw),
            made - 1,
            row.mark
        )
    );
    let _ = fire_quietly(&mut rig).await;
    assert_eq!(
        vault_owner(&rig.tw, hold),
        trust(&rig.tw),
        "back in the pool"
    );
}

// ---------------------------------------------------------------------------
// Doc 20 cut SM.2: a hop is a departure
// ---------------------------------------------------------------------------

/// One hop flown end to end: `line` files, the burn lands, the launch is fired and the
/// ticket flown to its arrival; the burn's real second.
async fn sm2_hop(rig: &mut Rig, pilot: ObjId, hull: ObjId, line: &str) -> i64 {
    let out = run(rig, pilot, line).await;
    assert!(out.contains("Clearance filed for "), "{line}: {out}");
    let seq = match rig
        .tw
        .world()
        .module_state::<TacsimStore>()
        .expect("tacsim state")
        .voyage_state(hull)
    {
        tacsim::port::VoyageState::Present(voyage) => {
            voyage.pending.as_ref().map(|pending| pending.seq)
        }
        tacsim::port::VoyageState::Absent | tacsim::port::VoyageState::Unreadable => None,
    }
    .expect("a filed passage");
    let burn = drain_players(rig).await;
    assert_eq!(burn.len(), 1, "the hop's burn");
    land_all(rig, &burn).await;
    let burned_at = rig.tw.world().now();
    for _ in 0..4 {
        let _ = fire_quietly(rig).await;
        if under_way(&rig.tw, hull) {
            break;
        }
    }
    assert!(under_way(&rig.tw, hull), "she lifted for {line}");
    let ticket = ticket(&mut rig.tw, hull, seq).expect("the hop's ticket");
    let _ = to_ic(rig, ticket.plan.depart_ic + ticket.plan.total_s).await;
    assert!(!under_way(&rig.tw, hull), "settled or docked after {line}");
    burned_at
}

/// A pilot enrolled, bunkered at the chandlery for hops, and aboard at Ceres.
async fn sm2_aboard_for_hops(rig: &mut Rig) -> (ObjId, ObjId, Enrolment) {
    let _ = sweep(rig, 120).await;
    stock(rig).await;
    let vex = pilot(rig, "Vex", "vexacct").await;
    let row = enrol(rig, vex, "Salt Moth").await;
    chandlery_from_registry(rig, vex).await;
    let fuel = listing_of(&rig.tw, "ceres_port", "occator", "propellant");
    let food = listing_of(&rig.tw, "ceres_port", "coop", "provisions");
    let _ = buy(rig, vex, fuel, 10).await;
    let _ = buy(rig, vex, food, 12).await;
    let _ = sweep(rig, 61).await;
    let _ = sweep(rig, 61).await;
    board_from_chandlery(rig, vex, "Salt Moth").await;
    let e = enrolment(&rig.tw, vex);
    (vex, ObjId(row.hull), e)
}

/// **A week of hops trips no recall** (doc 20 §13's SM.2 acceptance; §3.3: a hop burns a
/// tonne and mints a ticket, so it is a departure for the dormancy rule, which reads real
/// time): a hop to Ahuna and back on day 0 and again on day 6 keep the charter past a week
/// from the first return; the control, with only the day-0 round trip, is recalled a week
/// after its last burn — docked at Ceres, the hold empty, nothing in flight.
#[tokio::test]
async fn sm2_a_week_of_hops_trips_no_recall() {
    const D: i64 = 7 * 86_400;
    let mut rig = boot("sm2_hops").await;
    let (vex, hull, e) = sm2_aboard_for_hops(&mut rig).await;
    let _ = sm2_hop(&mut rig, vex, hull, "+ship/undock ahuna").await;
    let b0 = sm2_hop(&mut rig, vex, hull, "+ship/lift ceres").await;
    let now = rig.tw.world().now();
    let _ = sweep(&mut rig, e.leased_at + 6 * 86_400 - now).await;
    let _ = sm2_hop(&mut rig, vex, hull, "+ship/undock ahuna").await;
    let _ = sm2_hop(&mut rig, vex, hull, "+ship/lift ceres").await;
    let now = rig.tw.world().now();
    let _ = sweep(&mut rig, b0 + D + 3_600 - now).await;
    let _ = fire_quietly(&mut rig).await;
    assert_eq!(contract_state(&rig.tw, e.lease), ContractState::Active);
    assert_eq!(enrolment(&rig.tw, vex).status, EnrolmentStatus::Active);
    assert!(tacsim::port_tick::recall_pages(rig.tw.world()).is_empty());

    // The control: only the day-0 round trip.
    let mut rig = boot("sm2_hops_control").await;
    let (vex, hull, e) = sm2_aboard_for_hops(&mut rig).await;
    let _ = sm2_hop(&mut rig, vex, hull, "+ship/undock ahuna").await;
    let b0 = sm2_hop(&mut rig, vex, hull, "+ship/lift ceres").await;
    let now = rig.tw.world().now();
    let _ = sweep(&mut rig, b0 + D + 3_600 - now).await;
    let _ = fire_quietly(&mut rig).await;
    assert_eq!(contract_state(&rig.tw, e.lease), ContractState::Cancelled);
    let revoked = enrolment(&rig.tw, vex);
    assert_eq!(revoked.status, EnrolmentStatus::Revoked);
    assert_eq!(revoked.ended, Some(EndCause::Dormant));
    assert_eq!(title_owner(&rig.tw, hull), Some(trust(&rig.tw)));
    {
        let w = rig.tw.world();
        let port = tacsim::port::programme_port(w).expect("the programme port");
        assert!(
            tacsim::port::coop::free_hulls(w, &port)
                .iter()
                .any(|r| r.hull == hull.0),
            "the hull is back in the pool"
        );
    }
}
