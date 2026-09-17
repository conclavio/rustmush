//! **The custody lens** (16b cut T.3; 16b §7 "Verify lenses"; doc 16 §3.1
//! "Custody", "Failure", §3.8, §8 "Restart mid-match").
//!
//! Everything cut T.3 claims is a claim about a *running server*: that a match
//! takes custody of real unit records and gives them back, that a unit nothing
//! happened to comes back byte-identical, that a driven one comes back where the
//! arena left it, that a panicking pulse voids the match without taking the world
//! with it, and that a restart in the middle of a match is a void the next boot
//! reconciles. None of that can be shown by calling a function. So this drives
//! the whole stack over **real SSH**, the way `onboarding_e2e.rs` does, and reads
//! the **store** for what landed.
//!
//! # No fixture (16b cut C.2)
//!
//! This file used to compose a test-local unit spec and a test-local definition
//! provider, because until cut C the production `TacticsModule` carried an
//! **empty** catalog and `@unit/create` could name no schema at all. Cut C ships
//! `crates/tacsim/content/units/line_tank.json`, registers it through the C1
//! registry and hands the catalog to the production tactics module, so the two
//! fixture modules are gone and the list here is the production article in
//! production order, with `[tacsim].enabled` on so the game's own boot packs
//! apply the arena's ground.
//!
//! The two tanks this lens drives are still **its own** objects (`#3` and `#4`,
//! created before the world task spawns, so their ids are stable) rather than
//! the arena pack's eight: what is under test is custody of a unit the lens can
//! read in the store before and after, and naming a pack-made object would put
//! the applier's own bookkeeping between the assertion and the property.
//!
//! One number moved with the real schema. The fixture's tank did 50 m/s at
//! 100 m/s²; doc 16 §3.3's does 15 m/s at 1 m/s², so the drive below is ten
//! seconds of arena time for three cells where it used to be three seconds for
//! ten. The property is the same one — the driven unit comes back where the
//! *arena* left it — and it is the shipped tank that decides how far that is.

mod support;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_core::module::GameModule;
use mush_core::{Engine, GOD, ObjId, ObjKind};
use mush_llm::{LlmProvider, LlmService, MockProvider};
use mush_session::{SessionConfig, SessionManager};
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use mushd::accounts::{Accounts, hash_password};
use mushd::ipguard::IpGuard;
use mushd::modules;
use mushd::persist::load_or_seed_world;
use mushd::ssh_bridge::{SshBridge, SshListener, start_listener};
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::world_task::{self, PersistHandles, ServerDeps};
use serde_json::Value;

use support::{Term, play_client, scratch_dir};

const GM_PASSWORD: &str = "gm-secret-1";
/// The world's transient authoritative refusal (`mush_server`'s
/// `RefusalReason::CommitPending`): a commit is in flight, so try again.
const COMMIT_PENDING: &str = "please retry in a moment";
/// How many times a line is re-sent on that refusal before the test gives up.
const RETRY_BUDGET: usize = 8;
/// The shipped unit schema (16b cut C.2's `content/units/line_tank.json`).
const SCHEMA: &str = "tacsim.unit.line_tank";
/// The arena `@arena/start` names.
const ARENA: &str = "tacsim.arena.proving_ground";
/// The domain host room, created before the world task spawns.
const FIELD: ObjId = ObjId(2);
/// The two tanks.
const RED: ObjId = ObjId(3);
const BLUE: ObjId = ObjId(4);
/// One arena cell, centimetres (doc 16 §3.2).
const CELL_CM: i64 = 1_000;

// ---------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------

struct Server {
    addr: SocketAddr,
    dir: PathBuf,
    store: Arc<dyn ObjectStore>,
    _listener: SshListener,
    _handles: PersistHandles,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A fast tacsim layer: a 100 ms pulse at every tempo and a 200 ms startup, so a
/// lens runs in seconds. Every other `[tacsim]` value is the shipped default.
fn tacsim_config() -> tacsim::TacsimConfig {
    let mut cfg = tacsim::TacsimConfig::default();
    cfg.arena.tempo.quiet_ms = 100;
    cfg.arena.tempo.alert_ms = 100;
    cfg.arena.tempo.engaged_ms = 100;
    cfg.match_.startup_ms = 200;
    // The black box is also the check-in's source of the arena's final pose when
    // no pilot is in the cockpit (cut T.3's `CustodyIn` seam), so the summary
    // cadence is turned up with the tempo.
    cfg.arena.ledger.summary_ms = 100;
    cfg
}

/// The production module list, in production order (16b cut C.2: there is no
/// fixture module in it any more — see this file's header).
fn module_list(
    onboarding: &mush_core::OnboardingConfig,
    llm: &Arc<LlmService>,
) -> Vec<Box<dyn GameModule>> {
    let mut mods = modules::server_modules();
    mods.push(modules::worldgen_module(Arc::clone(llm)));
    mods.push(modules::npc_module(modules::npc_agent_adapter()));
    mods.extend(modules::ruleset_modules(onboarding, &Default::default()));
    mods.push(modules::tacsim_module(tacsim_config()));
    // 16b cut C.2: the content module last, with `[game].starter_area` off and
    // the game's own boot packs on — the arena's ground has to be in this
    // world's atlas records before `@arena/start` can build its window.
    mods.push(modules::content_module_with(false, true).expect("the content module composes"));
    mods
}

/// Boot the stack over SSH. `store` reuses an earlier server's — the reboot half
/// of the lens — and `seed` creates the room and the two tanks, which a reboot
/// must not do again.
async fn boot(tag: &str, store: Option<Arc<dyn ObjectStore>>, seed: bool) -> Server {
    let dir = scratch_dir(tag);
    let store: Arc<dyn ObjectStore> = match store {
        Some(store) => store,
        None => {
            let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
            store
                .create_account(AccountRec {
                    id: None,
                    name: "boss".to_string(),
                    argon2_hash: hash_password(GM_PASSWORD).expect("a hash"),
                    created: 0,
                    characters: vec![CharacterRef {
                        id: GOD,
                        name: "God".to_string(),
                    }],
                    ssh_keys: vec![],
                    discord: None,
                })
                .await
                .expect("the account");
            store
        }
    };

    let mut world = load_or_seed_world(&*store).await.expect("boot world");
    if seed {
        let field = world
            .create(ObjKind::Room, "Proving Ground", GOD, None)
            .expect("the domain host");
        assert_eq!(field, FIELD, "domain-host id assumption");
        for (id, name) in [(RED, "Red One"), (BLUE, "Blue One")] {
            let obj = world
                .create(ObjKind::Thing, name, GOD, None)
                .expect("a tank");
            assert_eq!(obj, id, "tank id assumption");
        }
    }

    let onboarding = mush_core::OnboardingConfig {
        chargen_required: false,
        start_room: mush_core::LIMBO,
        start_room_unapproved: mush_core::LIMBO,
    };
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).expect("server commands");
    let mock: Arc<dyn LlmProvider> = Arc::new(MockProvider::new("openrouter"));
    let llm = Arc::new(
        LlmService::builder()
            .provider("openrouter", mock)
            .global_enabled(false)
            .build(),
    );

    let mut engine =
        Engine::with_modules(world, registry, module_list(&onboarding, &llm)).expect("modules");
    modules::boot_modules(&mut engine, &*store)
        .await
        .expect("module boot");

    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(120),
        ..SessionConfig::default()
    }));
    let guard = Arc::new(IpGuard::new());
    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            store: Arc::clone(&store),
            guard: Arc::clone(&guard),
            staff,
            llm: Some(Arc::clone(&llm)),
            quarantine_spool: mushd::quarantine::QuarantineSpool::in_memory(),
            metrics: Arc::new(mush_net_web::Metrics::new()),
            outbox_adapters: Vec::new(),
        },
    );
    let accounts = Arc::new(Accounts::new(Arc::clone(&store), handles.handle.clone(), 3));
    let bridge = Arc::new(SshBridge::new(
        accounts,
        Arc::clone(&sessions),
        handles.handle.clone(),
        Arc::clone(&guard),
    ));
    let listener = start_listener(
        "127.0.0.1:0".parse().expect("a loopback address"),
        &dir,
        "Welcome to the proving ground.".to_string(),
        bridge,
    )
    .await
    .expect("ssh listener");
    let addr = listener.addr();
    Server {
        addr,
        dir,
        store,
        _listener: listener,
        _handles: handles,
    }
}

/// Connect as staff.
async fn staff_session(addr: SocketAddr) -> (russh::client::Handle<support::AcceptAll>, Term) {
    let (handle, mut term) = play_client(addr).await;
    term.expect("Welcome to the proving ground.").await;
    term.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    term.expect("Connected as God").await;
    (handle, term)
}

/// Run one line and wait for `needle` to come back on the same span, retrying
/// while the world answers the **transient** authoritative refusal.
///
/// Two rules, both learned the hard way:
///
/// * Every needle is chosen so it cannot appear in the **echo** of the line that
///   produced it. A terminal echoes what was typed, so `@behavior/add
///   #2=sim.domain` matched on `sim.domain` would pass before the world had even
///   seen the command, and the next line would race it.
/// * `@arena`'s switches are on the authoritative lane, which answers "the world
///   is committing another action; please retry in a moment" when a commit is in
///   flight. That is a real answer a real client retries, not a failure.
async fn say(term: &mut Term, line: &str, needle: &str) {
    assert!(
        !line.contains(needle),
        "`{needle}` appears in `{line}`'s own echo; pick a needle from the ANSWER"
    );
    for _ in 0..RETRY_BUDGET {
        let mark = term.mark();
        term.send_line(line).await;
        if term
            .expect_one_of_after(mark, &[needle, COMMIT_PENDING])
            .await
            == 0
        {
            return;
        }
    }
    panic!("`{line}` was refused as commit-pending {RETRY_BUDGET} times running");
}

/// The world, ready to run a match: a domain on the field and both tanks placed
/// in it as units.
async fn prepare(term: &mut Term) {
    say(
        term,
        &format!("@behavior/add #{}=sim.domain", FIELD.0),
        "Attached",
    )
    .await;
    say(
        term,
        &format!("@domain/create #{}=sim.passive", FIELD.0),
        "Created domain",
    )
    .await;
    // Two DIFFERENT cells, deliberately: doc 16 §3.4's occupancy rule is one
    // unit per cell, and two tanks stacked on the domain's origin would have the
    // check-out move one of them into an entry zone — which is correct, and
    // which would make "the unit nothing happened to never moved" false for a
    // reason that has nothing to do with custody.
    for (unit, x_cm) in [(RED, 500_i64), (BLUE, 20_500_i64)] {
        say(
            term,
            &format!("@unit/create #{}={SCHEMA}", unit.0),
            "is now a tactical unit",
        )
        .await;
        say(
            term,
            &format!("@behavior/add #{}=sim.participant", unit.0),
            "Attached",
        )
        .await;
        say(
            term,
            &format!(
                "@spatial/place/domain #{}=0,{{\"x_cm\":{x_cm},\"y_cm\":500}}",
                unit.0
            ),
            "Placed",
        )
        .await;
    }
}

/// One unit's authoritative record, as the store holds it.
async fn record(store: &Arc<dyn ObjectStore>, unit: ObjId) -> Option<Value> {
    store
        .module_json_get("tactics", &format!("unit:{}", unit.0))
        .await
        .expect("the store read")
}

/// One unit's placement, as the store holds it.
async fn placement(store: &Arc<dyn ObjectStore>, unit: ObjId) -> Option<Value> {
    store
        .module_json_get("sim", &format!("placement:{}", unit.0))
        .await
        .expect("the store read")
}

fn pose_x_cm(placement: &Value) -> i64 {
    placement
        .pointer("/parent/pose/x_cm")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| panic!("a direct placement carries a pose: {placement}"))
}

fn pose_heading(placement: &Value) -> u64 {
    placement
        .pointer("/parent/pose/heading_mdeg")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("a direct placement carries a heading: {placement}"))
}

/// Poll until `check` answers, or fail.
async fn until(bound: Duration, label: &str, mut check: impl AsyncFnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + bound;
    while tokio::time::Instant::now() < deadline {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{label} did not happen within {bound:?}");
}

// ---------------------------------------------------------------------------
// The lens
// ---------------------------------------------------------------------------

/// **Custody in and out** (16b §7): two units checked out, one driven ten cells,
/// the match aborted — and afterwards the untouched unit's record is byte-
/// identical to what it was before the match, while the driven one's world
/// placement is where the arena left it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_match_takes_custody_of_two_units_and_gives_them_back() {
    let server = boot("arena-custody", None, true).await;
    let (_handle, mut term) = staff_session(server.addr).await;
    prepare(&mut term).await;

    let before_red = record(&server.store, RED).await.expect("the red record");
    let before_blue = record(&server.store, BLUE).await.expect("the blue record");
    let start_x = pose_x_cm(&placement(&server.store, RED).await.expect("placed"));
    let blue_placement = placement(&server.store, BLUE).await.expect("placed");
    assert!(
        before_red.get("deployed").is_none(),
        "no marker before a match: {before_red}"
    );

    // --- check-out -------------------------------------------------------
    say(
        &mut term,
        &format!("@arena/start {ARENA}=#{}/#{}", RED.0, BLUE.0),
        "is running on",
    )
    .await;
    until(Duration::from_secs(10), "the markers to land", async || {
        record(&server.store, RED)
            .await
            .is_some_and(|document| document.get("deployed").is_some())
            && record(&server.store, BLUE)
                .await
                .is_some_and(|document| document.get("deployed").is_some())
    })
    .await;
    // The marker is the ONLY difference: doc 16 §3.1's check-out writes one
    // field and touches no cell.
    let deployed_red = record(&server.store, RED).await.expect("the red record");
    let mut stripped = deployed_red.clone();
    if let Some(object) = stripped.as_object_mut() {
        object.remove("deployed");
    }
    assert_eq!(stripped, before_red, "the marker is the only change");
    // And the admission sites refuse, by the one named error.
    say(
        &mut term,
        &format!("@unit/damage #{}={{\"ops\":[]}}", RED.0),
        "deployed in arena match",
    )
    .await;
    say(
        &mut term,
        &format!("@spatial/unplace #{}", RED.0),
        "deployed in arena match",
    )
    .await;

    // --- drive east, at the shipped tank's own pace ------------------------
    say(
        &mut term,
        &format!("@arena/intent #{}=startup", RED.0),
        "Ordered",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    say(
        &mut term,
        &format!("@arena/intent #{}=heading 90", RED.0),
        "Ordered",
    )
    .await;
    say(
        &mut term,
        &format!("@arena/intent #{}=speed 5000", RED.0),
        "Ordered",
    )
    .await;
    // Ten seconds of arena time. The order is above the tank's ceiling on
    // purpose — the arena caps it at `max_speed_cms` — so what is being timed is
    // doc 16 §3.4's acceleration and nothing else: 100 cm/s² for ten seconds is
    // 1,000 cm/s and fifty metres, which is five cells, and the assertion below
    // asks for three.
    tokio::time::sleep(Duration::from_millis(10_000)).await;

    // --- check-in --------------------------------------------------------
    say(&mut term, "@arena/abort", "is stopping").await;
    until(
        Duration::from_secs(20),
        "the check-in to land",
        async || {
            record(&server.store, RED)
                .await
                .is_some_and(|document| document.get("deployed").is_none())
                && record(&server.store, BLUE)
                    .await
                    .is_some_and(|document| document.get("deployed").is_none())
        },
    )
    .await;

    // (1) The unit nothing happened to is byte-identical, record and placement.
    assert_eq!(
        record(&server.store, BLUE).await,
        Some(before_blue),
        "a unit nothing happened to comes back byte-identical"
    );
    assert_eq!(
        placement(&server.store, BLUE).await,
        Some(blue_placement),
        "and it never moved"
    );

    // (2) The driven unit's world placement is where the arena left it: several
    //     cells east of where it started, on the heading it was ordered onto.
    //     The pose the world writes is the arena's own last frame (cut T.3's
    //     `CustodyIn` seam), so this is a statement about the arena's number,
    //     not about the world's arithmetic.
    let after = placement(&server.store, RED).await.expect("still placed");
    let travelled = pose_x_cm(&after) - start_x;
    assert!(
        travelled >= 3 * CELL_CM,
        "the driven unit came back {travelled} cm east of where it went out; three cells is \
         {} cm (placement: {after})",
        3 * CELL_CM
    );
    assert_eq!(
        pose_heading(&after),
        90_000,
        "and on the bearing it was ordered onto: {after}"
    );

    // (3) The match left exactly one result document, and it is an abort.
    let result = server
        .store
        .module_json_get("tacsim", "tacsim:match:1")
        .await
        .expect("the store read")
        .expect("one result document per match (doc 16 §4)");
    assert_eq!(
        result.get("outcome").and_then(Value::as_str),
        Some("abort"),
        "the result names the outcome: {result}"
    );
    assert_eq!(
        result.pointer("/ledger/log").and_then(Value::as_str),
        Some("match.1"),
        "and the ledger it left behind: {result}"
    );
    assert!(
        result
            .pointer("/ledger/digest")
            .and_then(Value::as_str)
            .is_some_and(|digest| digest.len() == 64),
        "with its digest: {result}"
    );
}

/// **A panic inside `step` voids the match, and the world keeps serving**
/// (doc 16 §3.1 "Failure").
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_injected_panic_voids_the_match_and_the_world_keeps_serving() {
    let server = boot("arena-fault", None, true).await;
    let (_handle, mut term) = staff_session(server.addr).await;
    prepare(&mut term).await;
    let before_red = record(&server.store, RED).await.expect("the red record");

    say(
        &mut term,
        &format!("@arena/start {ARENA}=#{}/#{}", RED.0, BLUE.0),
        "is running on",
    )
    .await;
    until(Duration::from_secs(10), "the markers to land", async || {
        record(&server.store, RED)
            .await
            .is_some_and(|document| document.get("deployed").is_some())
    })
    .await;

    say(&mut term, "@arena/fault 1", "will panic").await;

    // The match is voided and its units are checked back in — restored from
    // their check-out snapshots, with no arena-derived cell committed.
    until(Duration::from_secs(20), "the void check-in", async || {
        record(&server.store, RED)
            .await
            .is_some_and(|document| document.get("deployed").is_none())
    })
    .await;
    assert_eq!(
        record(&server.store, RED).await,
        Some(before_red),
        "a voided match restores the check-out snapshot exactly"
    );
    let result = server
        .store
        .module_json_get("tacsim", "tacsim:match:1")
        .await
        .expect("the store read")
        .expect("a voided match still leaves its result");
    assert_eq!(
        result.get("outcome").and_then(Value::as_str),
        Some("void"),
        "with the reason: {result}"
    );

    // And the world is still a world: the same connection still serves
    // commands, and the units are mutable again.
    say(&mut term, "look", "Limbo").await;
    say(
        &mut term,
        &format!("@unit/damage #{}={{\"ops\":[]}}", RED.0),
        "Applied 0 op(s)",
    )
    .await;
}

/// **Restart mid-match** (doc 16 §3.1 "Boot reconciliation", §8): a server that
/// stops while a match is running leaves `deployed` markers behind, and the next
/// boot restores them, releases the seats and records an `aborted(restart)`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restart_mid_match_restores_the_units_and_records_an_aborted_restart() {
    let first = boot("arena-restart-a", None, true).await;
    let (_handle, mut term) = staff_session(first.addr).await;
    prepare(&mut term).await;
    let before_red = record(&first.store, RED).await.expect("the red record");

    say(
        &mut term,
        &format!("@arena/start {ARENA}=#{}/#{}", RED.0, BLUE.0),
        "is running on",
    )
    .await;
    until(Duration::from_secs(10), "the markers to land", async || {
        record(&first.store, RED)
            .await
            .is_some_and(|document| document.get("deployed").is_some())
    })
    .await;

    // Stop the server where it stands, mid-`Running`, with both markers on
    // disk and no result document. An OOC match holds no durable state and
    // never resumes (D-16-3), so this is exactly the state the next boot has
    // to reconcile.
    let store = Arc::clone(&first.store);
    drop(term);
    drop(first);

    let second = boot("arena-restart-b", Some(Arc::clone(&store)), false).await;
    let (_handle2, mut term2) = staff_session(second.addr).await;

    // (1) The marker is gone and the snapshot is back.
    until(
        Duration::from_secs(20),
        "the boot reconciliation",
        async || {
            record(&store, RED)
                .await
                .is_some_and(|document| document.get("deployed").is_none())
        },
    )
    .await;
    assert_eq!(
        record(&store, RED).await,
        Some(before_red),
        "the restored record is the check-out snapshot exactly"
    );

    // (2) The units are mutable again.
    say(
        &mut term2,
        &format!("@unit/damage #{}={{\"ops\":[]}}", RED.0),
        "Applied 0 op(s)",
    )
    .await;

    // (3) And the match it was in is on the record as an aborted restart.
    let result = store
        .module_json_get("tacsim", "tacsim:match:1")
        .await
        .expect("the store read")
        .expect("boot reconciliation writes the match's result");
    assert_eq!(
        result.get("outcome").and_then(Value::as_str),
        Some("abort"),
        "{result}"
    );
    assert_eq!(
        result.get("reason").and_then(Value::as_str),
        Some("restart"),
        "{result}"
    );

    // (4) The next match this world runs gets a NEW id: a counter that
    //     restarted at zero would write over `log:match.1` and `tacsim:match:1`.
    // The first match's check-in re-placed both tanks, so they are still
    // participants; nothing to re-attach.
    say(
        &mut term2,
        &format!("@arena/start {ARENA}=#{}/#{}", RED.0, BLUE.0),
        "Match 2 is running",
    )
    .await;
    say(&mut term2, "@arena/abort 2", "is stopping").await;
}
