//! **The custody lens** (16b cut T.3; 16b §7 "Verify lenses"; doc 16 §3.1
//! "Custody", "Failure", §3.8, §8 "Restart mid-match").
//!
//! Everything cut T.3 claims is a claim about a *running server*: that a match
//! takes custody of real unit records and gives them back, that a unit nothing
//! happened to comes back byte-identical, that a driven one comes back where the
//! arena left it, that a panicking pulse voids the match without taking the world
//! with it, and that a restart in the middle of a match is resumed by the next
//! boot from the match's last snapshot (16b cut AF.3) — or, when it cannot be,
//! voided by name. None of that can be shown by calling a function. So this drives
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
//! the arena pack's stock tanks: what is under test is custody of a unit the lens can
//! read in the store before and after, and naming a pack-made object would put
//! the applier's own bookkeeping between the assertion and the property.
//!
//! One number moved with the real schema. The fixture's tank did 50 m/s at
//! 100 m/s²; doc 16 §3.3's does 15 m/s at 1 m/s², so the drive below is ten
//! seconds of arena time for three cells where it used to be three seconds for
//! ten. The property is the same one — the driven unit comes back where the
//! *arena* left it — and it is the shipped tank that decides how far that is.

mod support;

use support::{wait_arena_seconds, wait_until};

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
    // **16b cut N.1**: `@arena/start` deploys units with **no seated pilot**, and
    // the check-out rule makes a unit with no pilot the reflex controller's (doc
    // 16 §6). That is correct — a staff-started match is an AI match — but it is
    // not what this file is about: two machines that patrol, close and shoot make
    // "the unit nothing happened to comes back byte-identical" false for a reason
    // that has nothing to do with custody. So the controller's cadence is pushed
    // past this test's lifetime.
    //
    // **17b cut M4.3** moved the premise again: a unit with no pilot is a unit
    // MIND's (doc 17 §9, §12), which thinks on its class row's grid
    // (`STOCK_THINK_MS`, one second) and reads no cadence knob, so `decide_ms`
    // idles nothing any more. The device is now the hulls' own: the custody
    // lens below makes the idle unit immobile AND dry before the match (its
    // engine destroyed, its magazines emptied — its guns kept, because a hull
    // that cannot move and carries no weapon is doc 16 §3.8's *disabled with
    // no weapons* and ends the match at 0 ms by elimination; a destroyed
    // sensor component blinds nothing at the arena), so its mind starts it
    // up, can drive nothing and has nothing to fire; and takes the driven
    // unit's weapons off, so an unarmed mind patrols and never closes (doc 17
    // §17's fourteenth divergence) and neither hull can shoot the other.
    // Nothing but the staff's orders and the red hull's own patrol moves
    // anything. The cadence ceiling is kept for the reflex rows no production
    // hull lays any more.
    //
    // **16b cut U.1** moved the premise. The **first** tick lands at the unit's
    // seeded stagger inside one cadence (`stagger_ms(seed, id, decide_ms)`; it
    // was `id % decide_ms`, which put it in the first second), and the match
    // seed folds the world clock and a process salt, so it moves every run. At
    // an hour's cadence a first tick inside this file's twenty-odd seconds was
    // about one run in two hundred per machine: a tank leaving `Startup`
    // mid-drive, planning its own route and moving the pose the check-in
    // compares. The cadence is therefore the largest the field holds (about
    // forty-nine days of arena time), which puts that chance near five in a
    // million per machine; the arithmetic that reads `decide_ms` is `i64` and
    // saturating throughout, and the module is handed this config directly, so
    // `ArenaConfig::validate`'s cross-key checks (the server's config load runs
    // them), which this value fails exactly as the hour did, are not in the
    // way. A tank may never come online
    // here, and nothing in this file reads whether it did. What the controller
    // does with a released seat and an AI-filled lobby is `sim_reflex.rs`'s.
    cfg.arena.reflex.decide_ms = u32::MAX;
    cfg
}

/// **[`tacsim_config`] with the layer on** (16b cut AF.3): the resume's tests
/// need it, because a boot voids every running match on a server whose
/// simulator is off (`the simulator is off on this server`) — and this file's
/// other lenses run `@arena/start` with the layer at its shipped default, off.
fn resume_config() -> tacsim::TacsimConfig {
    let mut cfg = tacsim_config();
    cfg.enabled = true;
    cfg
}

/// The production module list, in production order (16b cut C.2: there is no
/// fixture module in it any more — see this file's header).
fn module_list(
    onboarding: &mush_core::OnboardingConfig,
    llm: &Arc<LlmService>,
    cfg: tacsim::TacsimConfig,
) -> Vec<Box<dyn GameModule>> {
    // The loaded game content includes the foundry. Share the production inventory
    // capability so a restart can audit its receipts through their actual owner.
    let (mut mods, inventory) = modules::server_modules_with_tacsim(
        Default::default(),
        Default::default(),
        Default::default(),
        &Default::default(),
        &Default::default(),
        Default::default(),
    );
    mods.push(modules::worldgen_module(Arc::clone(llm)));
    mods.push(modules::npc_module(modules::npc_agent_adapter()));
    mods.extend(modules::sheet_modules(onboarding, &Default::default()));
    mods.push(modules::tacsim_module_wired(cfg, false, inventory));
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
    boot_with(tag, store, seed, tacsim_config()).await
}

/// [`boot`] on a named `[tacsim]` (16b cut AF.3: the config edited between two
/// boots, which only a real boot over the store can show).
async fn boot_with(
    tag: &str,
    store: Option<Arc<dyn ObjectStore>>,
    seed: bool,
    cfg: tacsim::TacsimConfig,
) -> Server {
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

    let mut engine = Engine::with_modules(world, registry, module_list(&onboarding, &llm, cfg))
        .expect("modules");
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

fn pose_y_cm(placement: &Value) -> i64 {
    placement
        .pointer("/parent/pose/y_cm")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| panic!("a direct placement carries a pose: {placement}"))
}

/// **One of a running match's two documents** (16b cut AF.2), as the store
/// holds it: `tacsim:snapshot:<n>` or `tacsim:live:<n>`.
async fn running_document(store: &Arc<dyn ObjectStore>, key: &str) -> Option<Value> {
    store
        .module_json_get("tacsim", key)
        .await
        .expect("the store read")
}

/// Every ledger line one match wrote, parsed, in the order the arena wrote them
/// (16b cut M.1) — `crates/mushd/tests/arena_story.rs`'s reader: a chunk document
/// carries the chunk's exact bytes in its `lines` fields, collected in chunk order
/// and split.
///
/// **16b cut AF.3**: through `tacsim::arena_bridge::tape_in_order`, so a resumed
/// match's abandoned tail is not read as the match.
async fn ledger_events(store: &Arc<dyn ObjectStore>, match_id: u64) -> Vec<Value> {
    let kept = kept_tape(store, match_id).await;
    let mut events = Vec::new();
    for value in kept {
        let mut lines: Vec<String> = Vec::new();
        collect_lines(&value, &mut lines);
        for blob in lines {
            for line in blob.lines().filter(|line| !line.trim().is_empty()) {
                events.push(
                    serde_json::from_str::<Value>(line).unwrap_or_else(|error| {
                        panic!("a ledger line is not JSON ({error}): {line}")
                    }),
                );
            }
        }
    }
    events
}

/// Every `"lines"` string in one chunk document, in document order.
fn collect_lines(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                match (key.as_str(), child.as_str()) {
                    ("lines", Some(text)) => out.push(text.to_owned()),
                    _ => collect_lines(child, out),
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_lines(item, out);
            }
        }
        _ => {}
    }
}

/// `unit`'s `(x_cm, y_cm, heading_mdeg)` in every `position_summary` line, in
/// order (16b cut M.1). A row is `[unit, x_cm, y_cm, z_cm, heading_mdeg]`.
fn summaries_of(events: &[Value], unit: ObjId) -> Vec<(i64, i64, u64)> {
    events
        .iter()
        .filter(|event| event.get("kind").and_then(Value::as_str) == Some("position_summary"))
        .filter_map(|event| {
            event
                .get("units")
                .and_then(Value::as_array)?
                .iter()
                .filter_map(Value::as_array)
                .find(|row| row.first().and_then(Value::as_u64) == Some(u64::from(unit.0)))
                .and_then(|row| {
                    Some((
                        row.get(1).and_then(Value::as_i64)?,
                        row.get(2).and_then(Value::as_i64)?,
                        row.get(4).and_then(Value::as_u64)?,
                    ))
                })
        })
        .collect()
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

    // **17b cut M4.3**: both hulls are a mind's once checked out (the AI fill
    // is a unit mind), so the idle unit is made immobile and dry first — its
    // engine destroyed through the staff's own damage verb in the shape
    // `tests/support`'s `engine_struck` writes, its magazines emptied as
    // `shot_dry` empties them — and the driven unit is unarmed, so neither can
    // shoot the other and the idle one cannot move. (Not inert: an immobile
    // hull with no weapon is out of action, and the match would end at once.)
    // The records are captured AFTER the damage: what custody must give back
    // byte-identical is the unit as it went out.
    let destroyed = |section: &str| {
        format!(r#"{{"SetSectionCondition":{{"section":"{section}","condition":"destroyed"}}}}"#)
    };
    let dead = |component: &str| {
        format!(
            r#"{{"SetComponentCondition":{{"component":"{component}","condition":"destroyed"}}}}"#
        )
    };
    let unarmed = [destroyed("turret"), dead("cannon"), dead("machine_gun")];
    let parked = [
        r#"{"SetCell":{"addr":{"Facet":{"section":"hull","facet":"rear","cell":"armor"}},"value":0}}"#
            .to_owned(),
        r#"{"SetCell":{"addr":{"Section":{"section":"engine","cell":"internals"}},"value":0}}"#
            .to_owned(),
        destroyed("engine"),
        r#"{"SetCell":{"addr":{"Component":{"component":"cannon","cell":"ammo"}},"value":0}}"#
            .to_owned(),
        r#"{"SetCell":{"addr":{"Component":{"component":"machine_gun","cell":"ammo"}},"value":0}}"#
            .to_owned(),
    ];
    for (unit, ops) in [(RED, unarmed.to_vec()), (BLUE, parked.to_vec())] {
        let plan = format!("{{\"ops\":[{}]}}", ops.join(","));
        say(
            &mut term,
            &format!("@unit/damage #{}={plan}", unit.0),
            "Applied",
        )
        .await;
    }
    let before_red = record(&server.store, RED).await.expect("the red record");
    let before_blue = record(&server.store, BLUE).await.expect("the blue record");
    let start = placement(&server.store, RED).await.expect("placed");
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
    wait_until(Duration::from_secs(10), "the markers to land", async || {
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
    // The staff shortcut has no pilot frame; one reported arena second covers
    // this fixture's 300 ms startup before accepting motion orders.
    wait_arena_seconds(&mut term, 1).await;
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
    // Ten observed seconds of arena time. The order is above the tank's ceiling on
    // purpose — the arena caps it at `max_speed_cms`. **17b cut M4.3**: the red
    // hull's mind patrols the ground at the same time (a unit with no pilot is
    // a mind's), so what is being asserted below is custody's promise alone —
    // the pose the world gets back is the arena's own last frame, somewhere
    // other than where the hull went out — and no longer doc 16 §3.4's
    // acceleration on one bearing, which `tacsim`'s own motor tests hold.
    wait_arena_seconds(&mut term, 10).await;
    // **16b cut M.1** (`T-42`): then the staff's stop, as before; the mind may
    // drive on, which is why the pose is compared with the arena's last
    // summary — written every pulse here (`summary_ms` is the tempo's period)
    // — and not with a standstill.
    say(
        &mut term,
        &format!("@arena/intent #{}=speed 0", RED.0),
        "Ordered",
    )
    .await;
    wait_arena_seconds(&mut term, 7).await;

    // --- check-in --------------------------------------------------------
    say(&mut term, "@arena/abort", "is stopping").await;
    wait_until(
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

    // (1) The unit nothing happened to is byte-identical, record and placement
    //     (17b cut M4.3: immobile and dry, so its mind starts it up — power is
    //     the arena's, not the record's — and can neither drive it nor fire).
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

    // (2) The driven unit's world placement is where the arena left it. The
    //     pose the world writes is the arena's own last frame (cut T.3's
    //     `CustodyIn` seam), so this is a statement about the arena's number,
    //     not about the world's arithmetic. How far it went is not asserted
    //     since 17b cut M4.3: the staff's orders and the hull's own mind drive
    //     it by turns, and the distance is the match seed's; the equality
    //     below is custody's whatever the distance.
    let after = placement(&server.store, RED).await.expect("still placed");
    assert!(
        after.pointer("/parent/domain").is_some(),
        "a direct placement names its domain: {after}"
    );
    assert_eq!(
        after.pointer("/parent/domain"),
        start.pointer("/parent/domain"),
        "the driven unit came back to the domain it went out from: {after} / {start}"
    );
    // **16b cut M.1** (`T-42`): and it is the arena's number verbatim. A chunk
    // reaches the store only at 16 KiB or at the end's flush, so the black box is
    // read after the end — waiting for its terminal `aborted` line — and its last
    // two `position_summary` rows for the red tank agree (it had stopped), and the
    // placement IS the last one: a stopped tank's final pose is its last summary,
    // and the check-in writes the pose the arena returned at `Ended`.
    wait_until(
        Duration::from_secs(20),
        "the flushed black box",
        async || {
            ledger_events(&server.store, 1)
                .await
                .iter()
                .any(|event| event.get("kind").and_then(Value::as_str) == Some("aborted"))
        },
    )
    .await;
    let summaries = summaries_of(&ledger_events(&server.store, 1).await, RED);
    let Some(last) = summaries.last() else {
        panic!("the black box has no summary of the red tank: {summaries:?}");
    };
    assert_eq!(
        (pose_x_cm(&after), pose_y_cm(&after), pose_heading(&after)),
        *last,
        "the placement is the arena's last summary, verbatim: {after}"
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
    wait_until(Duration::from_secs(10), "the markers to land", async || {
        record(&server.store, RED)
            .await
            .is_some_and(|document| document.get("deployed").is_some())
    })
    .await;

    say(&mut term, "@arena/fault 1", "will panic").await;

    // The match is voided and its units are checked back in — restored from
    // their check-out snapshots, with no arena-derived cell committed.
    wait_until(Duration::from_secs(20), "the void check-in", async || {
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

/// **Every event the tracing layer is handed, with its fields** (16b cut
/// AF.3): set as the thread's default around a boot, which runs on the test's
/// own thread, so the boot's one void line and its `reason` can be read.
#[derive(Clone, Default)]
struct Captured(Arc<std::sync::Mutex<Vec<std::collections::BTreeMap<String, String>>>>);

/// [`Captured`]'s field visitor: strings as they are, the rest by `Debug`.
struct FieldsOf<'a>(&'a mut std::collections::BTreeMap<String, String>);

impl tracing::field::Visit for FieldsOf<'_> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
}

impl tracing::Subscriber for Captured {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() <= tracing::Level::WARN
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut fields = std::collections::BTreeMap::new();
        event.record(&mut FieldsOf(&mut fields));
        if let Ok(mut held) = self.0.lock() {
            held.push(fields);
        }
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

impl Captured {
    /// The reason the boot's void line gave for match `id`, if it gave one.
    fn void_reason(&self, id: u64) -> Option<String> {
        let held = self.0.lock().ok()?;
        held.iter()
            .find(|fields| {
                fields
                    .get("message")
                    .is_some_and(|message| message.contains("could not resume"))
                    && fields.get("match_id").map(String::as_str) == Some(id.to_string().as_str())
            })
            .and_then(|fields| fields.get("reason").cloned())
    }
}

/// **Stop a server the way a crash stops it** (16b cut AF.3): its arena tasks
/// told to stop, then its world task and its store writer aborted and joined,
/// before any of its sessions is dropped. Dropping the `Server` alone detaches
/// those tasks rather than ending them, and a world that is still running when
/// the pilots' sessions go handles their disconnects — releasing every seat and
/// taking one more snapshot — which is a clean shutdown the resume would then
/// read, not the crash under test.
async fn crash(server: &mut Server) {
    if let Some(arenas) = &server._handles.arenas {
        arenas.signal_stop_all();
    }
    server._handles.world_join.abort();
    let _ = (&mut server._handles.world_join).await;
    server._handles.writer_join.abort();
    let _ = (&mut server._handles.writer_join).await;
}

/// **The tape as `tape_in_order` counts it** (16b cut AF.3) — and, on a log no
/// resume forked, **exactly its raw chunk entries**, asserted: the reader only
/// truncates at a marker, so a chunk the world appended twice or out of order
/// on an unforked tape still breaks the digest check (`B-48`'s lens) instead of
/// being folded away by the last-write rule.
async fn kept_tape(store: &Arc<dyn ObjectStore>, match_id: u64) -> Vec<Value> {
    let entries = ledger_entries(store, match_id).await;
    let (kept, markers) = tacsim::arena_bridge::tape_in_order(&entries);
    if markers.is_empty() {
        let raw: Vec<Value> = entries
            .into_iter()
            .filter(|entry| entry.get("chunk").and_then(Value::as_u64).is_some())
            .collect();
        assert_eq!(
            raw, kept,
            "an unforked tape is its raw chunk entries, each once and in order"
        );
    }
    kept
}

/// Every entry of one match's log, in document order (16b cut AF.3): arena
/// chunks and resume markers alike.
async fn ledger_entries(store: &Arc<dyn ObjectStore>, match_id: u64) -> Vec<Value> {
    let log = format!("match.{match_id}");
    let mut entries = Vec::new();
    for chunk in 0..256 {
        let key = mush_core::chunk_key(&log, chunk);
        let Some(value) = store
            .module_json_get("tacsim", key.as_str())
            .await
            .expect("the store read")
        else {
            break;
        };
        entries.extend(
            value
                .get("entries")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|entry| entry.get("value").cloned()),
        );
    }
    entries
}

/// Start match 1 with both tanks, and wait for the markers and both of the
/// running match's documents to be in the store — the state a server that
/// stops here leaves behind.
async fn start_and_wait_for_the_snapshot(term: &mut Term, store: &Arc<dyn ObjectStore>) {
    say(
        term,
        &format!("@arena/start {ARENA}=#{}/#{}", RED.0, BLUE.0),
        "is running on",
    )
    .await;
    wait_until(
        Duration::from_secs(10),
        "the markers and the two documents",
        async || {
            record(store, RED)
                .await
                .is_some_and(|document| document.get("deployed").is_some())
                && record(store, BLUE)
                    .await
                    .is_some_and(|document| document.get("deployed").is_some())
                && running_document(store, "tacsim:snapshot:1").await.is_some()
                && running_document(store, "tacsim:live:1").await.is_some()
        },
    )
    .await;
}

/// **Restart mid-match: the match resumes** (16b cut AF.3; doc 16 §3.1
/// "Durability", "Boot reconciliation", §8 "Restart mid-match"): a server
/// stopped while a staff match runs, with both markers and both of the running
/// match's documents on disk, and booted again over the same store.
///
/// The markers stay on both records and `@unit/damage` is refused as deployed;
/// `@arena/status` lists match 1 running with its `resumed:` line; there is no
/// result document; `@arena/abort 1` stops it and the result is an ordinary
/// `abort` with a tape; the markers go with it. The raw log holds one resume
/// marker, with the old server's chunks each once and in order before it, the
/// chunk it names among them, and the resumed server's chunks each once and in
/// order from the one after it; `tape_in_order` reads a contiguous tape. The
/// next match is match 2.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restart_mid_match_resumes_the_match_and_the_units_stay_deployed() {
    let mut first = boot_with("arena-resume-a", None, true, resume_config()).await;
    let (_handle, mut term) = staff_session(first.addr).await;
    prepare(&mut term).await;
    start_and_wait_for_the_snapshot(&mut term, &first.store).await;

    // Stop the server where it stands, mid-`Running`.
    let store = Arc::clone(&first.store);
    crash(&mut first).await;
    drop(term);
    drop(first);

    let second = boot_with(
        "arena-resume-b",
        Some(Arc::clone(&store)),
        false,
        resume_config(),
    )
    .await;
    let (_handle2, mut term2) = staff_session(second.addr).await;

    // (1) The markers are still on both records, and the units still refused.
    for unit in [RED, BLUE] {
        assert!(
            record(&store, unit)
                .await
                .is_some_and(|document| document.get("deployed").is_some()),
            "#{} lost its marker at the resume",
            unit.0
        );
    }
    say(
        &mut term2,
        &format!("@unit/damage #{}={{\"ops\":[]}}", RED.0),
        "is deployed in arena match 1",
    )
    .await;

    // (2) The match runs, and staff read that it resumed.
    say(&mut term2, "@arena/status", "resumed: from a snapshot at").await;
    assert!(
        running_document(&store, "tacsim:match:1").await.is_none(),
        "a resumed match writes no result"
    );

    // (3) An abort ends it the ordinary way.
    say(&mut term2, "@arena/abort 1", "is stopping").await;
    wait_until(Duration::from_secs(20), "the result", async || {
        running_document(&store, "tacsim:match:1").await.is_some()
    })
    .await;
    let result = running_document(&store, "tacsim:match:1")
        .await
        .expect("the result");
    assert_eq!(
        result.get("reason").and_then(Value::as_str),
        Some("abort"),
        "{result}"
    );
    assert!(
        result
            .pointer("/ledger/chunks")
            .and_then(Value::as_u64)
            .is_some_and(|chunks| chunks >= 1),
        "{result}"
    );
    wait_until(Duration::from_secs(10), "the markers to go", async || {
        record(&store, RED)
            .await
            .is_some_and(|document| document.get("deployed").is_none())
            && record(&store, BLUE)
                .await
                .is_some_and(|document| document.get("deployed").is_none())
    })
    .await;

    // (4) The tape: one resume marker, the raw chunks around it, and a
    // contiguous tape once read in the order it counts.
    let entries = ledger_entries(&store, 1).await;
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.get("resumed").is_some())
            .count(),
        1,
        "{entries:?}"
    );
    // The raw log, around its one marker: the old server's chunks each once
    // and in order from 1; a marker naming a chunk the old server wrote; and
    // the resumed server's chunks each once and in order from the one after it
    // — so the only sequences written twice are the abandoned tail's, above
    // the marker's chunk.
    let chunks_of = |entries: &[Value]| -> Vec<u64> {
        entries
            .iter()
            .filter_map(|entry| entry.get("chunk").and_then(Value::as_u64))
            .collect()
    };
    let at = entries
        .iter()
        .position(|entry| entry.get("resumed").is_some())
        .expect("the resume marker");
    let (old, rest) = entries.split_at(at);
    let from = rest
        .first()
        .and_then(|marker| marker.pointer("/resumed/from_chunk"))
        .and_then(Value::as_u64)
        .expect("the marker's from_chunk");
    let before = chunks_of(old);
    let after = chunks_of(rest.get(1..).unwrap_or_default());
    assert_eq!(
        before,
        (1..=u64::try_from(before.len()).unwrap_or(0)).collect::<Vec<u64>>(),
        "the old server's chunks, each once and in order: {entries:?}"
    );
    assert!(
        from <= before.last().copied().unwrap_or(0),
        "the marker names chunk {from}, which the old server never wrote: {before:?}"
    );
    assert_eq!(
        after,
        (from + 1..=from + u64::try_from(after.len()).unwrap_or(0)).collect::<Vec<u64>>(),
        "the resumed chunks, each once and in order from {}: {entries:?}",
        from + 1
    );
    let (kept, markers) = tacsim::arena_bridge::tape_in_order(&entries);
    assert_eq!(markers.len(), 1);
    let sequences: Vec<u64> = chunks_of(&kept);
    assert_eq!(
        sequences,
        (1..=u64::try_from(sequences.len()).unwrap_or(0)).collect::<Vec<u64>>(),
        "a contiguous tape"
    );

    // (5) The next match is match 2.
    say(
        &mut term2,
        &format!("@arena/start {ARENA}=#{}/#{}", RED.0, BLUE.0),
        "Match 2 is running",
    )
    .await;
    say(&mut term2, "@arena/abort 2", "is stopping").await;
}

/// **Restart mid-match with a snapshot that is not what the arena wrote: a
/// void by name** (16b cut AF.3): the same stop, then `tacsim:snapshot:1`
/// rewritten in the store with a hashed field moved and its digest stamped
/// again, so the boot passes every check up to the decode's hash. The old
/// answer, now for this arm alone: the restored record is the check-out
/// snapshot exactly, `@unit/damage` is accepted, `tacsim:match:1` is an
/// `abort` for `restart`, both documents are gone — and the log names the
/// reason.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restart_mid_match_voids_a_match_whose_snapshot_is_wrong() {
    let mut first = boot_with("arena-void-a", None, true, resume_config()).await;
    let (_handle, mut term) = staff_session(first.addr).await;
    prepare(&mut term).await;
    let before_red = record(&first.store, RED).await.expect("the red record");
    start_and_wait_for_the_snapshot(&mut term, &first.store).await;
    let store = Arc::clone(&first.store);
    crash(&mut first).await;
    drop(term);
    drop(first);

    // The snapshot, a hashed field moved and the digest stamped again.
    let mut snapshot = running_document(&store, "tacsim:snapshot:1")
        .await
        .expect("the snapshot");
    let state = snapshot
        .get("state")
        .and_then(Value::as_str)
        .expect("a state string");
    let mut parsed: Value = serde_json::from_str(state).expect("a JSON state");
    let now = parsed.get("now_ms").and_then(Value::as_i64).unwrap_or(0);
    parsed["now_ms"] = serde_json::json!(now + 1);
    let state = serde_json::to_string(&parsed).expect("a state encodes");
    {
        use sha2::Digest as _;
        snapshot["state_digest"] = Value::String(tacsim::arena::snapshot::digest_text(
            &sha2::Sha256::digest(state.as_bytes()).into(),
        ));
    }
    snapshot["state"] = Value::String(state);
    store
        .write(mush_store::WriteBatch {
            serial: 1,
            durable: true,
            module_ops: vec![mush_core::ModuleOp {
                ns: smol_str::SmolStr::new("tacsim"),
                op: mush_core::ModuleWrite::JsonSet {
                    key: smol_str::SmolStr::new("tacsim:snapshot:1"),
                    value: snapshot,
                },
            }],
            ..mush_store::WriteBatch::default()
        })
        .await
        .expect("the store takes the edit");

    let captured = Captured::default();
    let second = {
        let _guard = tracing::subscriber::set_default(captured.clone());
        boot_with(
            "arena-void-b",
            Some(Arc::clone(&store)),
            false,
            resume_config(),
        )
        .await
    };
    assert!(
        captured
            .void_reason(1)
            .is_some_and(|reason| reason.starts_with("state hash mismatch (recorded ")),
        "{:?}",
        captured.void_reason(1)
    );
    let (_handle2, mut term2) = staff_session(second.addr).await;
    assert_eq!(
        record(&store, RED).await,
        Some(before_red),
        "the restored record is the check-out snapshot exactly"
    );
    say(
        &mut term2,
        &format!("@unit/damage #{}={{\"ops\":[]}}", RED.0),
        "Applied 0 op(s)",
    )
    .await;
    let result = running_document(&store, "tacsim:match:1")
        .await
        .expect("the void writes the match's result");
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
    assert!(
        running_document(&store, "tacsim:snapshot:1")
            .await
            .is_none()
    );
    assert!(running_document(&store, "tacsim:live:1").await.is_none());
}

/// **The two void arms only a real boot can reach** (16b cut AF.3): the
/// config edited between the boots — `[tacsim.arena].snapshot_ms = 0`, the
/// operator's kill switch, and `[tacsim].enabled = false` — each voids a
/// running match by name, with its units restored.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restart_with_snapshots_or_the_layer_off_voids_by_name() {
    for (tag, reason, edit) in [
        (
            "snapshots-off",
            "snapshots are off on this server",
            (|cfg: &mut tacsim::TacsimConfig| cfg.arena.snapshot_ms = 0)
                as fn(&mut tacsim::TacsimConfig),
        ),
        (
            "layer-off",
            "the simulator is off on this server",
            |cfg: &mut tacsim::TacsimConfig| cfg.enabled = false,
        ),
    ] {
        let mut first = boot_with(&format!("arena-{tag}-a"), None, true, resume_config()).await;
        let (_handle, mut term) = staff_session(first.addr).await;
        prepare(&mut term).await;
        let before_red = record(&first.store, RED).await.expect("the red record");
        start_and_wait_for_the_snapshot(&mut term, &first.store).await;
        let store = Arc::clone(&first.store);
        crash(&mut first).await;
        drop(term);
        drop(first);

        let mut cfg = resume_config();
        edit(&mut cfg);
        let captured = Captured::default();
        let _second = {
            let _guard = tracing::subscriber::set_default(captured.clone());
            boot_with(
                &format!("arena-{tag}-b"),
                Some(Arc::clone(&store)),
                false,
                cfg,
            )
            .await
        };
        assert_eq!(captured.void_reason(1).as_deref(), Some(reason), "{tag}");
        assert_eq!(record(&store, RED).await, Some(before_red), "{tag}");
        assert!(
            record(&store, BLUE)
                .await
                .is_some_and(|document| document.get("deployed").is_none()),
            "{tag}"
        );
        let result = running_document(&store, "tacsim:match:1")
            .await
            .expect("the void writes the match's result");
        assert_eq!(
            result.get("reason").and_then(Value::as_str),
            Some("restart"),
            "{tag}: {result}"
        );
    }
}

/// **The two documents exist while a match runs and go with its result** (16b
/// cut AF.2; 16c N16): over SSH, a staff `@arena/start` — the store holds the
/// arena's snapshot (`tacsim.snapshot`, and no `match` key: the store key names
/// it) and the world's live record (it decodes, it names match 1 and its record
/// runs); `@arena/abort 1`, and both are gone while the result document is
/// there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn af2_the_two_documents_exist_while_a_match_runs_and_go_with_its_result() {
    let server = boot("arena-af2", None, true).await;
    let (_handle, mut term) = staff_session(server.addr).await;
    prepare(&mut term).await;
    say(
        &mut term,
        &format!("@arena/start {ARENA}=#{}/#{}", RED.0, BLUE.0),
        "is running on",
    )
    .await;
    wait_until(
        Duration::from_secs(10),
        "the snapshot and the live record",
        async || {
            running_document(&server.store, "tacsim:snapshot:1")
                .await
                .is_some()
                && running_document(&server.store, "tacsim:live:1")
                    .await
                    .is_some()
        },
    )
    .await;
    let snapshot = running_document(&server.store, "tacsim:snapshot:1")
        .await
        .expect("the snapshot");
    assert_eq!(
        snapshot.get("record").and_then(Value::as_str),
        Some("tacsim.snapshot"),
        "the arena's own document"
    );
    assert!(snapshot.get("match").is_none(), "the key names the match");
    let live = running_document(&server.store, "tacsim:live:1")
        .await
        .expect("the live record");
    let live = tacsim::match_::LiveRecord::from_document(&live)
        .unwrap_or_else(|error| panic!("the live record the store gave back: {error}"));
    assert_eq!(live.match_id, tacsim::arena_task::MatchId(1));
    assert_eq!(live.record.state, tacsim::match_::MatchState::Running);

    say(&mut term, "@arena/abort 1", "is stopping").await;
    wait_until(
        Duration::from_secs(20),
        "the result, and the two documents gone",
        async || {
            running_document(&server.store, "tacsim:match:1")
                .await
                .is_some()
                && running_document(&server.store, "tacsim:snapshot:1")
                    .await
                    .is_none()
                && running_document(&server.store, "tacsim:live:1")
                    .await
                    .is_none()
        },
    )
    .await;
}
