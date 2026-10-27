//! **Doc 20 §13's SM.8 acceptance — the mining story, on the production composition** (doc 20
//! §1's twelve beats; §6, §7.1, §7.2, §7.4, §8; the SM.8 brief's A1 and its 3.13, lead ruling 5,
//! decisions R11 and R17).
//!
//! The test boots **`mush_server::boot::build_engine`** — the split server's own root: the
//! roster from `boot::game_modules`, the help topics loaded from `docs/help`, the world timeline
//! seeded from `[game]`, the account oracle the world task links, the durable key rings — over a
//! kept in-memory store, with `[game].ic_ratio = 24` (the world boots at the play rate, so no staff
//! act runs at all), the sky at rate 0, and **a FIXED geology key written into the key directory
//! before the first boot** (the tacsim suites' `KEY_1` bytes; an empty directory would provision a
//! fresh random key and the ground would change run to run — the brief's reviewer finding F1).
//! `ic_slice_loop.rs`'s rig is copied (the file-local precedent every mush-server story follows):
//! **every queued authoritative submission lands as the world task lands it** and **every restart
//! is a plain `build_engine` re-boot over the kept store**.
//!
//! The story, played twice — once straight, once with a restart inside the mining run — on the
//! one shared key: a pilot walks in from Limbo, enrols, refits a Dray-P, buys a prospecting
//! licence, leases a rig and reads `+chart`; bunkers to the planner's figures for Ahuna Flank (the
//! leg home held back) and lifts; on the pad scans and sounds, picks the richest metals pocket of
//! their own sounding, drives there and mines until the hopper is full; recovers, and the recover's
//! queued unload moves the hopper into the hold; lifts home; sells the hold at the Exchange's ore
//! bid with the Trust's quarter of the upper half. Money is conserved over the pilot and every org
//! of the `ceres_port` and `stickney_port` maps; every hold and hopper write stays inside both
//! limits of the Dray-P's and the prospector's own stowage; no page opens; the world rate is 24
//! throughout; and the two post-images are equal.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mush_config::config::Config;
use mush_core::{
    AccountId, Audience, AuthoritativeSubmit, CanonicalId, GOD, LIMBO, ObjId, OfferSubmission,
    PrincipalId as CorePrincipal,
};
use mush_economy::{ContractId, ContractState, EconomyStore, VaultId};
use mush_inventory::{EndpointRef, InventoryHandles};
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
use tacsim::definitions::content::{
    DRAY_P, DRAY_P_JSON, LANE_JSON, LANE_PACK, TV_PROSPECTOR, TV_PROSPECTOR_JSON,
};
use tacsim::pack::{PackEntry, load_pack};
use tacsim::port::apply::room_by_key;
use tacsim::port::capacity::Compartment;
use tacsim::port::{HullRow, PortDef, PortHandles, PortMap, PortMapState, Ticket, TicketId};
use tacsim::surface::geology::OreFamily;
use tacsim::surface::{ActKind, ChartKey, RigKey, RigRecord, Run};

/// **The story's geology key** (the brief's decision R17): the tacsim suites' `KEY_1` bytes
/// (`tests/support/geology_keyed.rs`, which a mush-server test cannot import), so the ground is
/// the one SM.7's and SM.8's tacsim tests measured.
const STORY_KEY: [u8; 32] = *b"rustmush-sm1-geology-test-key-01";
/// Ahuna Flank's port entry.
const AHUNA: &str = "ahuna_port";
/// The hull's name.
const HULL: &str = "Salt Moth";
/// The most mining runs the story makes before it fails, naming what it got.
const MAX_RUNS: usize = 4;
/// **The real second each arm's story starts at** (unix seconds, later than any boot): the IC
/// clock is a pure function of the real one (`[game].ic_epoch` is the IC datetime at unix 0), so
/// two arms booted seconds apart read the same IC instants only from a shared real start.
const STORY_T0: i64 = 1_900_000_000;
/// **The step a later start moves by** (the cut SM.8 fix round's tests/T7): 365 IC days at rate
/// 24 in real seconds, so a start moved past [`STORY_T0`] keeps its IC time of day and, near
/// enough, its IC date.
const STORY_STEP: i64 = 365 * 86_400 / 24;
/// The real seconds a start lies past the first boot at least: both arms boot before it.
const STORY_MARGIN: i64 = 3_600;

/// **The shared real start of both arms** (tests/T7: never a date bomb): [`STORY_T0`] while the
/// box's clock is [`STORY_MARGIN`] short of it, else the first [`STORY_STEP`] past it that is —
/// read once, before either boot, so both arms start at the same instant.
fn story_t0(now: i64) -> i64 {
    let earliest = now.saturating_add(STORY_MARGIN);
    if earliest <= STORY_T0 {
        return STORY_T0;
    }
    let steps = earliest
        .saturating_sub(STORY_T0)
        .saturating_add(STORY_STEP - 1)
        / STORY_STEP;
    STORY_T0.saturating_add(steps.saturating_mul(STORY_STEP))
}

/// **The shared start is never in the past** (tests/T7): before 2030 it is [`STORY_T0`]; after,
/// a whole number of [`STORY_STEP`]s past it, at least [`STORY_MARGIN`] ahead of the clock.
#[test]
fn sm8_the_story_start_is_never_in_the_past() {
    assert_eq!(story_t0(1_800_000_000), STORY_T0);
    for now in [
        STORY_T0 - STORY_MARGIN + 1,
        STORY_T0,
        2_000_000_000,
        4_000_000_000,
    ] {
        let t0 = story_t0(now);
        assert!(t0 >= now + STORY_MARGIN, "{now} → {t0}");
        assert!(t0 < now + STORY_MARGIN + STORY_STEP, "{now} → {t0}");
        assert_eq!((t0 - STORY_T0) % STORY_STEP, 0, "{now} → {t0}");
    }
}

// ---------------------------------------------------------------------------
// The rig: `build_engine` over a kept in-memory store (`ic_slice_loop.rs`'s)
// ---------------------------------------------------------------------------

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

/// A fresh scratch dir (the `boot_parity.rs` pattern).
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mush-smstory-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    dir
}

/// **Write a fixed geology key into `dir`** in the `DurableKeyStore` layout, by hand (the
/// `geology_keys.rs` tests' way): `geology-key-0000000001.hex` holding the 32 bytes as 64
/// lowercase hex digits and a newline, and `geology-ring-0000000001.json` naming it current; both
/// mode 0600 and the directory 0700 on unix (the reader refuses a wider file or directory).
fn write_fixed_key(dir: &Path, key: [u8; 32]) {
    use std::fmt::Write as _;
    std::fs::create_dir_all(dir).expect("the key directory");
    let mut hex = String::with_capacity(65);
    for byte in key {
        let _ = write!(hex, "{byte:02x}");
    }
    hex.push('\n');
    let key_path = dir.join("geology-key-0000000001.hex");
    let ring_path = dir.join("geology-ring-0000000001.json");
    std::fs::write(&key_path, hex.as_bytes()).expect("the key file");
    std::fs::write(
        &ring_path,
        br#"{"format_version":1,"current":1,"retained":[]}"#,
    )
    .expect("the ring manifest");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        for path in [&key_path, &ring_path] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .expect("private key files");
        }
        // The durable key path itself must be 0700 (the store refuses a wider
        // directory); `create_dir_all` leaves it at the umask's 0755.
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .expect("a private key directory");
    }
}

/// **The config dev1's rate-24 deploy renders** (`ic_slice_loop.rs`'s), the geology key
/// directory the shared fixed one.
fn config(tag: &str, key_dir: &Path) -> Config {
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
    cfg.tacsim.geology.key_dir = Some(key_dir.to_path_buf());
    cfg
}

/// What a landing came to.
enum Landing {
    /// Staged, made durable and installed.
    Landed,
    /// The coordinator's HIT.
    Replay,
    /// Refused before any durable intent.
    Refused(String),
}

/// One booted engine over a kept store.
struct Rig {
    tw: TestWorld,
    cfg: Config,
    store: Arc<dyn ObjectStore>,
    spool: QuarantineSpool,
    serial: u64,
    epoch: u64,
    accounts: BTreeMap<ObjId, AccountId>,
    /// The hull's hold, once it is a Dray-P (every landing re-checks it).
    hold: Option<VaultId>,
    /// The rigs' hoppers (every landing re-checks them).
    hoppers: BTreeSet<u64>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.cfg.server.data_dir);
    }
}

/// **Boot the production root** over a fresh in-memory store, the key directory shared.
async fn boot(tag: &str, key_dir: &Path) -> Rig {
    let cfg = config(tag, key_dir);
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
    let tw = TestWorld::from_engine(booted.engine);
    let mut rig = Rig {
        tw,
        cfg,
        store,
        spool,
        serial: 1,
        epoch: 1,
        accounts: BTreeMap::new(),
        hold: None,
        hoppers: BTreeSet::new(),
    };
    flush(&mut rig).await;
    rig
}

/// **Persist the step's legacy work** as the world task does after every step.
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

/// **A server-half restart over the kept store** (`ic_slice_loop.rs`'s): the legacy work
/// flushed; the engine dropped; `build_engine` over the same store, spool and key dir; a new
/// process epoch; the clock put back; every pilot's account back through the stored accounts;
/// `probe` read back through the store; the pilots reconnected (a restart has no sessions, and a
/// player who logs back in hears what is told after).
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
    let pilots: Vec<ObjId> = rig.accounts.keys().copied().collect();
    for pilot in pilots {
        let _ = rig.tw.connect(pilot);
    }
    flush(rig).await;
}

/// The handler's selector turned into the actor's own opaque offer (`world_task.rs`'s
/// `command_offer_submission`).
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

/// The permit's actor's query and projection, the resolve, then
/// `ActionCoordinator::prepare_and_stage_resolved` (`world_task.rs`'s `resolve_and_stage_offer`).
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
                mush_core::OfferExecutionContext {
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

/// **Land one queued submission exactly as the world task does** (`ic_slice_loop.rs`'s): legacy
/// work flushed; `command_ingress`; `preflight`; on `Vacant` the projection, the resolve, the
/// stage and the K3 sequence. Every tracked hold and hopper is then checked against its limits.
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
                if let Err(detail) = resolve_and_stage(&mut rig.tw, permit, submission) {
                    return Landing::Refused(detail);
                }
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
                let _ = rig.tw.install_composed_action_announcing(installation);
                auth.checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
                    .await
                    .expect("the bundle checkpoints");
                if let Some(hold) = rig.hold {
                    let hoppers: Vec<VaultId> = rig.hoppers.iter().copied().map(VaultId).collect();
                    assert_stowage_within_limits(rig, hold, &hoppers);
                }
                return Landing::Landed;
            }
        }
    }
    panic!("the idempotency retirement watermark changed repeatedly")
}

/// Every queued submission, in the order the engine queued them. GOD never submits from a tick.
fn drain(tw: &mut TestWorld) -> Vec<AuthoritativeSubmit> {
    let mut kept = Vec::new();
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        if submit.action.key().owner.as_str() == tacsim::NS {
            assert_ne!(submit.actor, GOD, "GOD never submits from a tick");
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

/// Land each; refusals and replays pass.
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

/// One fire (6 s on), its submissions drained, not landed.
async fn fire(rig: &mut Rig) -> (Vec<(Audience, OutputEvent)>, Vec<AuthoritativeSubmit>) {
    let events = sweep(rig, 6).await;
    let queued = drain_players(rig).await;
    (events, queued)
}

/// One fire, its submissions landed (each must land or be a replay).
async fn fire_landing(rig: &mut Rig) -> Vec<(Audience, OutputEvent)> {
    let (events, queued) = fire(rig).await;
    land_all(rig, &queued).await;
    events
}

/// One typed line as `who`; what they saw.
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

/// The port tick until the whole readiness report reads Ready (the six sites' clerks opening
/// their deposits included; `ic_slice_loop.rs`'s bound of 300 fires).
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

/// **A character on a gateway account of their own** (the world task's create path).
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

// ---------------------------------------------------------------------------
// Reads
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
    let key =
        tacsim::port::room_key(&def.area, &format!("{}.{local}", def.area)).expect("a room key");
    room_by_key(tw.world(), &key).expect("the room is on this world")
}

fn location(tw: &TestWorld, obj: ObjId) -> Option<ObjId> {
    tw.world().get(obj).and_then(|o| o.location)
}

fn handles(tw: &TestWorld) -> PortHandles {
    tw.world()
        .module_state::<PortHandles>()
        .cloned()
        .expect("port handles")
}

/// A vault's pool of `good`, through an owner-stamped transaction.
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
        .expect("reads")
        .map_or(0, |scalar| {
            u64::try_from(scalar.current()).expect("non-negative")
        })
}

/// A typed record, read through an owner-stamped transaction.
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

/// One hull's ticket at `seq`.
fn ticket(tw: &mut TestWorld, hull: ObjId, seq: u32) -> Option<Ticket> {
    let handles = handles(tw);
    typed(tw, &handles.families.tickets, &TicketId { hull, seq })
}

/// A rig's typed record.
fn record_of(tw: &mut TestWorld, tv: ObjId) -> Option<RigRecord> {
    let handles = handles(tw);
    typed(tw, &handles.families.excursions, &RigKey(tv))
}

/// A rig's run.
fn run_of(tw: &mut TestWorld, tv: ObjId) -> Option<Run> {
    record_of(tw, tv).and_then(|record| record.run)
}

fn ic_now(tw: &TestWorld) -> i64 {
    tw.world().world_ic_now().at
}

fn rate(tw: &TestWorld) -> u32 {
    tw.world().world_timeline().current().rate
}

/// Run the clock to IC instant `target` (one fire at the real second it falls on); the events
/// and what the fire queued, landed.
async fn to_ic(rig: &mut Rig, target: i64) -> Vec<(Audience, OutputEvent)> {
    let rate = i64::from(rate(&rig.tw).max(1));
    let ahead = target.saturating_sub(ic_now(&rig.tw)).max(0);
    let real = ((ahead + rate - 1) / rate).max(1);
    let events = sweep(rig, real).await;
    let queued = drain_players(rig).await;
    land_all(rig, &queued).await;
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

/// `+market/quote <listing>=<qty>`, `+market/accept <quote>`, the Draw landed.
async fn buy(rig: &mut Rig, pilot: ObjId, listing: ListingId, qty: u64) {
    let quoted = run(rig, pilot, &format!("+market/quote {listing}={qty}")).await;
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = newest_quote(&rig.tw, pilot);
    let accepted = run(rig, pilot, &format!("+market/accept {quote}")).await;
    assert!(accepted.contains("Agreed."), "{accepted}");
    let queued = drain_players(rig).await;
    assert_eq!(queued.len(), 1, "{accepted}");
    land_all(rig, &queued).await;
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

/// `Leg home from {port}: burn {b} t, {d} crew-days at …` → `(b, d)`: the leg home the filing
/// holds back (`SITE_HOME`, the plan's first leg-home line).
fn leg_home(plan: &str) -> Option<(u64, u64)> {
    let line = plan.lines().find(|l| l.starts_with("Leg home from "))?;
    leg_home_figures(line)
}

/// The same for the leg home with a full hold (`SITE_HOME_LADEN`, doc 20b RK-1).
fn leg_home_full(plan: &str) -> Option<(u64, u64)> {
    let line = plan
        .lines()
        .find(|l| l.starts_with("Leg home from ") && l.contains(" with a full hold ("))?;
    leg_home_figures(line)
}

/// A leg-home line's `: burn {b} t, {d} crew-days` → `(b, d)`.
fn leg_home_figures(line: &str) -> Option<(u64, u64)> {
    let rest = line.split(": burn ").nth(1)?;
    let burn = rest.split_whitespace().next()?.parse().ok()?;
    let days = rest
        .split(", ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some((burn, days))
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

/// **The conservation set**: posted credit over `pilots` and every org of the `ceres_port` and
/// `stickney_port` maps (the Trust, the Registry, Occator, the Exchange, the chandlery's `coop`,
/// Ceres Clearing and Tycho Mutual among them).
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

/// The Co-op's `(basis, share)` of one sale: the margin over the reference, and its permille
/// floored.
fn margin_and_share(price: i64, reference: i64, qty: u64, permille: u32) -> (i64, i64) {
    let basis = (price - reference).max(0) * i64::try_from(qty).expect("qty");
    (basis, basis * i64::from(permille) / 1_000)
}

fn goods() -> GoodsTable {
    tacsim::definitions::goods::goods_table().expect("the goods table")
}

/// A unit's stowage compartments from its shipped pack.
fn compartments_of(json: &str, unit: &str) -> Vec<Compartment> {
    let pack = load_pack(json).expect("the pack");
    let Some(PackEntry::Unit(spec)) = pack.entry(&cid(unit)) else {
        panic!("the {unit} unit");
    };
    spec.logistics
        .as_ref()
        .expect("its logistics")
        .stowage
        .iter()
        .map(|(id, def)| Compartment::from_stowage(id, def))
        .collect()
}

/// **Every hold and hopper write inside both limits** (A1; the brief's 3.13, reviewer finding
/// F5): the Dray-P's compartments from `DRAY_P`'s own pack, the prospector's `hopper` from
/// `TV_PROSPECTOR`'s — never the hull's limits as literals; each vault's pools read per good,
/// routed with `capacity::compartment_for`, and each compartment's `capacity::load` at or under
/// every limit.
fn assert_stowage_within_limits(rig: &mut Rig, hold: VaultId, hoppers: &[VaultId]) {
    let goods = goods();
    let checks = [
        (compartments_of(DRAY_P_JSON, DRAY_P), vec![hold]),
        (
            compartments_of(TV_PROSPECTOR_JSON, TV_PROSPECTOR),
            hoppers.to_vec(),
        ),
    ];
    for (compartments, vaults) in checks {
        for vault in vaults {
            let mut contents: BTreeMap<CanonicalId, BTreeMap<CanonicalId, u64>> = BTreeMap::new();
            for good in &goods.goods {
                let qty = pool(&mut rig.tw, vault, good.id.as_str());
                if qty == 0 {
                    continue;
                }
                let comp = tacsim::port::capacity::compartment_for(&goods, &compartments, &good.id)
                    .unwrap_or_else(|| {
                        panic!("vault #{} holds {}, which stows nowhere", vault.0, good.id)
                    });
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
                        "vault #{} {} is over its {measure}: {used} of {limit} ({held:?})",
                        vault.0,
                        comp.id
                    );
                }
            }
        }
    }
}

/// No escalation of any kind is open, and no namespace write was refused.
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

/// **The geology readiness line the fixed key reads** (the brief's 3.13): `GeologySecret`'s
/// fingerprint, its first twelve lowercase hex digits.
fn geology_line() -> String {
    let secret = mush_core::GeologySecret::new(1, STORY_KEY).expect("the story key");
    let hex: String = secret
        .fingerprint()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!(
        "geology: key v1, fingerprint {}, pinned; reading rule v{}.",
        &hex[..12],
        tacsim::surface::READING_RULE_VERSION
    )
}

// ---------------------------------------------------------------------------
// The story's own steps
// ---------------------------------------------------------------------------

/// **File a passage with `line` and fly it**: the clearance filed, its burn landed, the launch
/// landed on the next fire, the clock run to the arrival; what `pilot` heard at the arrival.
async fn fly(rig: &mut Rig, pilot: ObjId, hull: ObjId, line: &str) -> String {
    let filed = run(rig, pilot, line).await;
    assert!(filed.contains("Clearance filed for "), "{line}: {filed}");
    let burn = drain_players(rig).await;
    assert_eq!(burn.len(), 1, "one burn: {filed}");
    let seq = match store(&rig.tw).voyage_state(hull) {
        tacsim::port::voyage::VoyageState::Present(voyage) => voyage
            .pending
            .as_ref()
            .map(|p| p.seq)
            .expect("a filed passage"),
        other => panic!("the sailing plan: {other:?}"),
    };
    land_all(rig, &burn).await;
    let ticket = ticket(&mut rig.tw, hull, seq).expect("the ticket");
    let _ = fire_landing(rig).await;
    let events = to_ic(rig, ticket.plan.depart_ic + ticket.plan.total_s).await;
    let mut heard = said(&rig.tw, &events, pilot);
    let events = fire_landing(rig).await;
    heard.push('\n');
    heard.push_str(&said(&rig.tw, &events, pilot));
    heard
}

/// **An act's `go` and its finish**: `line` queues exactly one start, landed; then the clock to
/// the act's due (its finish landed) and one more fire (the tell, and whatever it queued, landed);
/// what `pilot` heard over both.
async fn act(rig: &mut Rig, pilot: ObjId, tv: ObjId, line: &str) -> String {
    let out = run(rig, pilot, line).await;
    let queued = drain_players(rig).await;
    assert_eq!(queued.len(), 1, "{line}: one start: {out}");
    land_all(rig, &queued).await;
    finish(rig, pilot, tv).await
}

/// **The act in flight, finished and told**; what `pilot` heard.
async fn finish(rig: &mut Rig, pilot: ObjId, tv: ObjId) -> String {
    let due = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .map(|act| act.due_ic)
        .expect("an act in flight");
    let events = to_ic(rig, due).await;
    let mut heard = said(&rig.tw, &events, pilot);
    let events = fire_landing(rig).await;
    heard.push('\n');
    heard.push_str(&said(&rig.tw, &events, pilot));
    heard
}

/// **The run's cycle in flight, landed and told**; what `pilot` heard.
async fn cycle(rig: &mut Rig, pilot: ObjId, tv: ObjId) -> String {
    let due = run_of(&mut rig.tw, tv)
        .map(|run| run.due_ic)
        .expect("a run");
    let events = to_ic(rig, due).await;
    let mut heard = said(&rig.tw, &events, pilot);
    let events = fire_landing(rig).await;
    heard.push('\n');
    heard.push_str(&said(&rig.tw, &events, pilot));
    heard
}

/// **The metals pockets the pilot's own sounding names**, richest first (`rich`, `fair`, `lean`;
/// ties in the block's order): the `  metals: …` line of the sounding's block.
fn sounded_metals(block: &str) -> Vec<String> {
    let Some(list) = block
        .lines()
        .find_map(|line| line.strip_prefix("  metals: "))
    else {
        return Vec::new();
    };
    let mut pockets: Vec<(u8, usize, String)> = list
        .split(", ")
        .enumerate()
        .filter_map(|(at, entry)| {
            let (addr, class) = entry.split_once(' ')?;
            let rank = match class {
                "rich" => 0,
                "fair" => 1,
                "lean" => 2,
                _ => return None,
            };
            Some((rank, at, addr.to_owned()))
        })
        .collect();
    pockets.sort();
    pockets.into_iter().map(|(_, _, addr)| addr).collect()
}

/// Every account's `(posted, available)` the post-image names.
fn balances(tw: &TestWorld, pilot: ObjId) -> Vec<(String, i64, i64)> {
    let mut out = Vec::new();
    let me = credit_account(tw, &PrincipalId::Object(pilot));
    out.push(("pilot".to_owned(), posted(tw, me), available(tw, me)));
    for org in [
        "coop_trust",
        "freeport_registry",
        "occator",
        "coop",
        "freeport_exchange",
    ] {
        let account = mush_economy::AccountId(
            port_map(tw, "ceres_port").orgs[&cid(org)]
                .account
                .expect("the org's account"),
        );
        out.push((org.to_owned(), posted(tw, account), available(tw, account)));
    }
    out
}

/// A vault's pools by good, every good of the table.
fn pools(tw: &mut TestWorld, vault: VaultId) -> BTreeMap<String, u64> {
    goods()
        .goods
        .iter()
        .filter_map(|good| {
            let qty = pool(tw, vault, good.id.as_str());
            (qty > 0).then(|| (good.id.to_string(), qty))
        })
        .collect()
}

/// **The story's post-image**, by name, never by object id.
#[derive(Debug, PartialEq, Eq)]
struct PostImage {
    balances: Vec<(String, i64, i64)>,
    hold: BTreeMap<String, u64>,
    hopper: BTreeMap<String, u64>,
    blocks: BTreeMap<String, BTreeMap<String, u64>>,
    record: String,
    chart: String,
    cursors: String,
    mined: String,
    bid: Option<u64>,
    geology: String,
    ic_now: i64,
}

/// **The story, played once** (doc 20 §1's beats; the brief's 3.13 steps 1–8). With `restart`,
/// a plain `build_engine` re-boot over the kept store falls between the first and the second cycle
/// of the first mining run.
#[allow(clippy::too_many_lines)]
async fn play(rig: &mut Rig, t0: i64, restart_mid_run: bool) -> PostImage {
    // 1. The world ready at rate 24, the geology key the fixed one; both arms from one real start.
    assert!(rig.tw.world().now() < t0, "the story starts after the boot");
    rig.tw.set_now(t0);
    let _ = sweep(rig, 120).await;
    stock(rig).await;
    let report = run(rig, GOD, "@tacsim/port status").await;
    assert!(
        report.contains("world rate: 24 IC seconds per real second. [ready]"),
        "{report}"
    );
    let geology = geology_line();
    assert!(
        report
            .lines()
            .any(|line| line == format!("{geology} [ready]")),
        "{report}\nwanted: {geology} [ready]"
    );
    // 2. The desk: enrol, refit, licence; Occator's counter: the lease; the chart.
    let pilot = pilot(rig, "Vex", "vexacct").await;
    assert_eq!(location(&rig.tw, pilot), Some(LIMBO));
    let c01 = station_room(&rig.tw, "ceres_port", "room.c01_berth_gallery");
    let c02 = station_room(&rig.tw, "ceres_port", "room.c02_pressure");
    let c03 = station_room(&rig.tw, "ceres_port", "room.c03_arrivals");
    let c04 = station_room(&rig.tw, "ceres_port", "room.c04_concourse");
    let c05 = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    let c07 = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    let c08 = station_room(&rig.tw, "ceres_port", "room.c08_chandlery");
    walk(rig, pilot, "c", c03).await;
    walk(rig, pilot, "registry", c05).await;
    let enrolled = run(rig, pilot, &format!("+coop/enrol {HULL}")).await;
    assert!(enrolled.contains(&format!("now the {HULL}")), "{enrolled}");
    let base = closed_sum(&rig.tw, &[pilot]);
    let refit = run(rig, pilot, "+coop/refit prospector").await;
    assert!(refit.contains("as a Dray-P"), "{refit}");
    let ship = run(rig, pilot, "+ship").await;
    assert!(ship.contains(&format!("{HULL} (Dray-P)")), "{ship}");
    let row = hull_row(&rig.tw, pilot);
    let hull = ObjId(row.hull);
    let hold = hold_of(&rig.tw, &row);
    rig.hold = Some(hold);
    assert_stowage_within_limits(rig, hold, &[]);
    let preview = run(rig, pilot, "+licence/buy").await;
    assert!(!preview.is_empty(), "the licence's terms");
    let _ = run(rig, pilot, "+licence/buy go").await;
    let queued = drain_players(rig).await;
    land_all(rig, &queued).await;
    let licence = run(rig, pilot, "+licence").await;
    assert!(licence.contains("in force"), "{licence}");
    walk(rig, pilot, "out", c03).await;
    walk(rig, pilot, "concourse", c04).await;
    walk(rig, pilot, "exchange", c07).await;
    let terms = run(rig, pilot, "+tv/lease").await;
    assert!(terms.contains("+tv/lease go signs it."), "{terms}");
    let leased = run(rig, pilot, "+tv/lease go").await;
    assert!(
        leased.contains("Occator Water & Propellant leases you "),
        "{leased}"
    );
    let tv = store(&rig.tw)
        .tv_leases_of(pilot)
        .find(|record| record.live() && record.slot == 1)
        .map(|record| record.tv)
        .expect("rig 1");
    let hopper = tacsim::port::vehicle::hopper_of(rig.tw.world(), tv).expect("the hopper");
    rig.hoppers.insert(hopper.0);
    let rigs = run(rig, pilot, "+tv").await;
    assert!(rigs.contains("Rig 1: "), "{rigs}");
    let chart = run(rig, pilot, "+chart").await;
    assert!(chart.contains("Ahuna Flank"), "{chart}");
    assert_eq!(
        closed_sum(&rig.tw, &[pilot]),
        base,
        "the fees stayed inside the set"
    );
    // 3. The chandlery: bunker to the planner's own figures for Ahuna, the leg home with a full
    // hold (doc 20b RK-1: the leg home the filing holds back is priced with no ore aboard).
    walk(rig, pilot, "chandlery", c08).await;
    let fuel_ask = listing_of(&rig.tw, "ceres_port", "occator", "propellant");
    let food_ask = listing_of(&rig.tw, "ceres_port", "coop", "provisions");
    for _ in 0..6 {
        let plan = run(rig, pilot, "+ship/plan ahuna").await;
        let (burn, tank) = plan_numbers(&plan, "Burn ", "(tank ").expect(&plan);
        let (days, locker) = plan_numbers(&plan, "Stores ", "(locker ").expect(&plan);
        let (home_burn, home_days) = leg_home_full(&plan).expect(&plan);
        let (want_tank, want_locker) = (burn + home_burn, days + home_days);
        if tank >= want_tank && locker >= want_locker {
            break;
        }
        if tank < want_tank {
            buy(rig, pilot, fuel_ask, want_tank - tank).await;
        }
        if locker < want_locker {
            buy(rig, pilot, food_ask, want_locker - locker).await;
        }
    }
    // The market's sweep sees the Draws' receipts before the hull may lift.
    let _ = sweep(rig, 61).await;
    let _ = sweep(rig, 61).await;
    assert_eq!(closed_sum(&rig.tw, &[pilot]), base);
    walk(rig, pilot, "exchange", c07).await;
    walk(rig, pilot, "concourse", c04).await;
    walk(rig, pilot, "arrivals", c03).await;
    walk(rig, pilot, "lobby", c02).await;
    walk(rig, pilot, "gallery", c01).await;
    let boarded = run(rig, pilot, "+ship/board").await;
    assert!(
        boarded.contains(&format!("You cycle the lock into the {HULL}.")),
        "{boarded}"
    );
    let plan = run(rig, pilot, "+ship/plan ahuna").await;
    assert!(plan.starts_with("Plan for Ahuna Flank"), "{plan}");
    // The planner's bare figures and what the hull carries at the lift, for step 7's measure.
    let (burn_out, tank_at_lift) = plan_numbers(&plan, "Burn ", "(tank ").expect(&plan);
    let (days_out, locker_at_lift) = plan_numbers(&plan, "Stores ", "(locker ").expect(&plan);
    let (home_burn, home_days) = leg_home(&plan).expect(&plan);
    let (full_burn, full_days) = leg_home_full(&plan).expect(&plan);
    let _ = fly(rig, pilot, hull, "+ship/lift ahuna").await;
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == AHUNA
        ),
        "settled on Ahuna's pad"
    );
    // 4. On the pad: the site warm, the scan, the sounding, the drive to the pilot's own pick.
    let mut warm = false;
    for _ in 0..20 {
        let out = run(rig, pilot, "+tv/1 scan").await;
        if !out.contains("is still being mapped") {
            warm = true;
            break;
        }
        let _ = fire_landing(rig).await;
    }
    assert!(warm, "Ahuna's ground warmed within 20 fires");
    let scanned = act(rig, pilot, tv, "+tv/1 scan go").await;
    assert!(scanned.contains("Rig 1 scanned "), "{scanned}");
    let sounded = act(rig, pilot, tv, "+tv/1 sound go").await;
    assert!(sounded.contains("Rig 1 sounded "), "{sounded}");
    let mut targets = sounded_metals(&sounded).into_iter();
    // 5. Mine to full: at most `MAX_RUNS` runs, each on the next pocket the sounding named.
    let mut got: Vec<(String, u64)> = Vec::new();
    let mut restarted = false;
    let full = loop {
        assert!(
            got.len() < MAX_RUNS,
            "the hopper did not fill in {MAX_RUNS} runs: {got:?}"
        );
        let target = targets
            .next()
            .unwrap_or_else(|| panic!("no more metals pockets to try: {got:?}\n{sounded}"));
        let arrived = act(rig, pilot, tv, &format!("+tv/1 drive {target} go")).await;
        assert!(
            arrived.contains(&format!("Rig 1 arrives at {target}")),
            "{arrived}"
        );
        let status = run(rig, pilot, "+tv").await;
        assert!(status.contains("open"), "the block's tenure: {status}");
        let preview = run(rig, pilot, "+tv/1 mine").await;
        assert!(preview.contains(" would mine "), "{preview}");
        let before = pool(&mut rig.tw, hopper, "metal_ore");
        let out = run(rig, pilot, "+tv/1 mine go").await;
        assert!(out.contains("Rig 1 starts drilling "), "{out}");
        let queued = drain_players(rig).await;
        land_all(rig, &queued).await;
        let heard = cycle(rig, pilot, tv).await;
        assert!(heard.contains("Rig 1, cycle 1 of "), "{heard}");
        if !restarted && run_of(&mut rig.tw, tv).is_some() {
            restarted = true;
            if restart_mid_run {
                restart(rig, |tw| run_of(tw, tv).is_some_and(|run| run.k == 2)).await;
            }
        }
        while run_of(&mut rig.tw, tv).is_some() {
            let heard = cycle(rig, pilot, tv).await;
            assert!(heard.contains("Rig 1, cycle "), "{heard}");
        }
        let after = pool(&mut rig.tw, hopper, "metal_ore");
        got.push((target.clone(), after - before));
        if preview.contains(" of 12 t; ") && after == 12 {
            break after;
        }
    };
    assert!(
        restarted,
        "the run was long enough to restart inside: {got:?}"
    );
    assert_eq!(full, 12, "the hopper is full: {got:?}");
    // The warm after the restart point (the same fires in both arms).
    for _ in 0..8 {
        let _ = fire_landing(rig).await;
    }
    // 6. The recover, and its queued unload.
    let limit_t = compartments_of(DRAY_P_JSON, DRAY_P)
        .into_iter()
        .find(|c| c.id.as_str() == "cargo")
        .and_then(|c| c.limits.get(&cid("si.mass_kg")).copied())
        .expect("the cargo hold's mass limit")
        / 1_000;
    let label = goods()
        .good(&cid("metal_ore"))
        .map(|g| g.label.clone())
        .expect("metal ore");
    let preview = run(rig, pilot, "+tv/1 recover").await;
    let then = tacsim::port::tv::RECOVER_THEN
        .replace("{load}", &format!("12 t of {label}"))
        .replace("{room}", &(limit_t - 12).to_string());
    assert!(
        preview.contains(&format!(
            "; hopper 12 of 12 t{then}. +tv/1 recover go brings it in."
        )),
        "{preview}\nwanted: …{then}"
    );
    let heard = act(rig, pilot, tv, "+tv/1 recover go").await;
    let back = tacsim::port::tv::RECOVERED_UNLOADING
        .replace("{n}", "1")
        .replace("{name}", HULL);
    assert!(heard.contains(&back), "{heard}\nwanted: {back}");
    let unloading = record_of(&mut rig.tw, tv)
        .and_then(|record| record.act)
        .expect("the queued unload started");
    assert_eq!(unloading.kind, ActKind::Unload);
    let heard = finish(rig, pilot, tv).await;
    let unloaded = tacsim::port::tv::UNLOADED
        .replace("{n}", "1")
        .replace("{load}", &format!("12 t of {label}"))
        .replace("{name}", HULL)
        .replace("{hopper}", "empty");
    assert!(heard.contains(&unloaded), "{heard}\nwanted: {unloaded}");
    assert_eq!(pool(&mut rig.tw, hopper, "metal_ore"), 0);
    assert_eq!(pool(&mut rig.tw, hold, "metal_ore"), 12);
    // 7. Home: the plan names no loaded hopper and no rig out; the lift; the sale.
    let plan = run(rig, pilot, "+ship/plan ceres").await;
    assert!(
        !plan.contains("hopper holds") && !plan.contains("is out on"),
        "{plan}"
    );
    // What the way home needed, measured: what the passage out and the pad days spent, plus what
    // the leg home burns and eats with the 12 t of ore aboard; the leg home with a full hold, which
    // the story bunkered to, covers it (doc 20b RK-1). SM.8 measured a crew-day past the bare
    // figures when the story bunkered a margin: that was the pad stay's charge, which stops at the
    // leg home's crew-days (`landed_charge`), so bunkered to the figures there is none.
    let (laden_burn, tank_on_pad) = plan_numbers(&plan, "Burn ", "(tank ").expect(&plan);
    let (laden_days, locker_on_pad) = plan_numbers(&plan, "Stores ", "(locker ").expect(&plan);
    let burn_need = tank_at_lift - tank_on_pad + laden_burn;
    let days_need = locker_at_lift - locker_on_pad + laden_days;
    let (bare_burn, bare_days) = (burn_out + home_burn, days_out + home_days);
    let figures = format!(
        "the planner's bare figures: burn {burn_out} + {home_burn} t, stores {days_out} + \
         {home_days} crew-days; with a full hold: burn {burn_out} + {full_burn} t, stores \
         {days_out} + {full_days} crew-days; at the lift: tank {tank_at_lift} t, locker \
         {locker_at_lift}; on the pad: tank {tank_on_pad} t, locker {locker_on_pad}; the laden \
         leg home: burn {laden_burn} t, stores {laden_days} crew-days\n{plan}"
    );
    assert_eq!(
        (burn_need, days_need),
        (bare_burn, bare_days),
        "bunkered to the figures, the pad stay costs nothing: {figures}"
    );
    assert!(
        burn_need <= burn_out + full_burn && days_need <= days_out + full_days,
        "the full-hold figures do not cover the way home: {figures}"
    );
    let _ = fly(rig, pilot, hull, "+ship/lift ceres").await;
    let ashore = run(rig, pilot, "+ship/leave").await;
    assert_eq!(location(&rig.tw, pilot), Some(c01), "{ashore}");
    walk(rig, pilot, "lobby", c02).await;
    walk(rig, pilot, "arrivals", c03).await;
    walk(rig, pilot, "concourse", c04).await;
    walk(rig, pilot, "exchange", c07).await;
    let bid = ListingId(
        port_map(&rig.tw, "ceres_port").vendors[&cid("freeport_exchange")].bids[&cid("metal_ore")]
            .listing,
    );
    let price = price_of(&rig.tw, bid);
    let wanted = listed_qty(&rig.tw, bid).expect("the bid stands");
    let preview = run(rig, pilot, &format!("+market/sell {bid}")).await;
    let lines: Vec<&str> = preview.lines().collect();
    assert_eq!(lines.len(), 2, "{preview}");
    assert!(
        lines[0].ends_with(&format!(
            ": Freeport Exchange wants {wanted} more at {} each; you hold 12 there.",
            cr(price)
        )),
        "{preview}"
    );
    assert_eq!(
        lines[1],
        format!("+market/sell {bid}=all sells 12 for {}.", cr(price * 12))
    );
    let programme = port_def("ceres_port").programme.expect("the programme");
    let reference = i64::try_from(programme.reference_minor[&cid("metal_ore")]).expect("fits");
    let (_, share) = margin_and_share(price, reference, 12, programme.share_permille);
    let sold = run(rig, pilot, &format!("+market/sell {bid}=all")).await;
    assert!(
        sold.contains(&format!(
            "Sold 12 t of {label} to Freeport Exchange at {}/t: {}; Co-op share {}; you receive {} \
             when the goods are in.",
            cr(price),
            cr(price * 12),
            cr(share),
            cr(price * 12 - share)
        )),
        "{sold}"
    );
    let queued = drain_players(rig).await;
    land_all(rig, &queued).await;
    let contract = market(&rig.tw)
        .quotes_of_seller(&PrincipalId::Object(pilot))
        .last()
        .and_then(|quote| quote.contract)
        .expect("the sale's contract");
    for _ in 0..12 {
        if matches!(
            economy(&rig.tw)
                .snapshot()
                .contract(contract)
                .map(|c| c.state),
            Some(ContractState::Settled { .. })
        ) {
            break;
        }
        let _ = sweep(rig, 31).await;
        let queued = drain_players(rig).await;
        land_quietly(rig, &queued).await;
    }
    let lines = settlement(&rig.tw, contract);
    let me = credit_account(&rig.tw, &PrincipalId::Object(pilot));
    let account = |org: &str| {
        mush_economy::AccountId(
            port_map(&rig.tw, "ceres_port").orgs[&cid(org)]
                .account
                .expect("the org's account"),
        )
    };
    assert_eq!(
        lines.get(&me).copied(),
        Some(price * 12 - share),
        "{lines:?}"
    );
    assert_eq!(
        lines.get(&account("coop_trust")).copied(),
        Some(share),
        "{lines:?}"
    );
    assert_eq!(
        lines.get(&account("freeport_exchange")).copied(),
        Some(-(price * 12)),
        "{lines:?}"
    );
    assert_eq!(pool(&mut rig.tw, hold, "metal_ore"), 0, "the hold sold");
    // 8. Conservation, the limits, no page, the rate.
    assert_eq!(
        closed_sum(&rig.tw, &[pilot]),
        base,
        "money is conserved over the set"
    );
    assert_stowage_within_limits(rig, hold, &[hopper]);
    assert_no_pages(&rig.tw);
    assert_eq!(
        rate(&rig.tw),
        24,
        "the world rate is 24 throughout; no staff act ran"
    );
    // The post-image.
    let record = record_of(&mut rig.tw, tv).expect("the rig's record");
    let handles = handles(&rig.tw);
    let chart_row = typed(
        &mut rig.tw,
        &handles.families.charts,
        &ChartKey {
            principal: pilot,
            site: cid(AHUNA),
        },
    )
    .map(|row| format!("{:?} {:?}", row.fields, row.blocks))
    .unwrap_or_default();
    let mut cursors = String::new();
    let mut blocks = BTreeMap::new();
    let ahuna = port_map(&rig.tw, AHUNA);
    for (target, _) in &got {
        let letter = target.chars().next().expect("a block letter");
        let deposit = mush_industry::derive_system_deposit_id(&tacsim::surface::opening::receipt(
            &cid(AHUNA),
            letter,
            OreFamily::Metals,
        ))
        .expect("a deposit id");
        let cursor = typed(&mut rig.tw, &handles.families.pockets, &deposit);
        cursors.push_str(&format!("{letter}: {cursor:?}\n"));
        let vault = ahuna
            .site
            .as_ref()
            .and_then(|site| site.vaults.get(&letter.to_string()).copied())
            .map(VaultId)
            .expect("the block vault");
        blocks.insert(letter.to_string(), pools(&mut rig.tw, vault));
    }
    let mined = match store(&rig.tw).mined(&cid(AHUNA)) {
        Some(tacsim::surface::mined::MinedState::Present(ring)) => ring
            .entries
            .iter()
            .map(|fact| {
                format!(
                    "{} {} {} {} {} {}",
                    fact.at_ic, fact.pocket, fact.good, fact.tonnes, fact.ordinal, fact.k
                )
            })
            .collect::<Vec<_>>()
            .join("; "),
        other => format!("{other:?}"),
    };
    PostImage {
        balances: balances(&rig.tw, pilot),
        hold: pools(&mut rig.tw, hold),
        hopper: pools(&mut rig.tw, hopper),
        blocks,
        record: format!(
            "ordinal {} charge {:?} last {:?} queued {:?}",
            record.ordinal,
            record.charge,
            record
                .last
                .as_ref()
                .map(|l| (l.kind, l.result, l.moved.clone())),
            record.queued.as_ref().map(|q| (q.after, q.at_ic)),
        ),
        chart: chart_row,
        cursors,
        mined,
        bid: listed_qty(&rig.tw, bid),
        geology,
        ic_now: ic_now(&rig.tw),
    }
}

/// **A1 — the story on the production composition** (doc 20 §13's SM.8 row): twice, on one
/// fixed geology key — straight, then with a restart inside the mining run — every line asserted
/// in both arms; the two post-images equal; the key file byte-unchanged by both boots.
#[tokio::test]
async fn sm8_the_story_on_the_production_composition() {
    let keys = scratch_dir("keys");
    write_fixed_key(&keys, STORY_KEY);
    let key_file = keys.join("geology-key-0000000001.hex");
    let before = std::fs::read(&key_file).expect("the key file");
    let unix_now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        });
    let t0 = story_t0(unix_now);
    let straight = {
        let mut rig = boot("a", &keys).await;
        play(&mut rig, t0, false).await
    };
    let restarted = {
        let mut rig = boot("b", &keys).await;
        play(&mut rig, t0, true).await
    };
    assert_eq!(
        std::fs::read(&key_file).expect("the key file"),
        before,
        "the fixed key is never rewritten"
    );
    let _ = std::fs::remove_dir_all(&keys);
    assert_eq!(
        straight, restarted,
        "the restart's post-image is the straight run's"
    );
    assert!(
        !straight.hold.keys().any(|good| good.ends_with("_ore")),
        "the hold sold all its ore: {:?}",
        straight.hold
    );
    assert!(straight.hopper.is_empty(), "the hopper is empty");
    assert!(straight.balances.iter().all(|(_, posted, _)| *posted >= 0));
}
