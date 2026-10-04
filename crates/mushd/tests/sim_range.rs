//! **One pilot on the range, over real SSH** (16b cut O.3; 16c N13, N2; doc 16
//! §3.8 "Lifecycle"; plan item 31).
//!
//! A new pilot needs something to shoot at that does not shoot back. This is the
//! lens for the range end to end over the production composition
//! `sim_units.rs` boots, **with the AI fill off** — the shipped default, and
//! what the boot helper leaves it — because the range's whole claim about the
//! lobby is that one pilot can start it on such a server:
//!
//! 1. Ada claims red and types `SIM RANGE 1`; staff poses her tank four cells
//!    **behind** blue's first pod's tank with both facing east, so every round
//!    lands on the rear band (hull 40, engine 30, turret 30);
//! 2. `SIM START` is accepted with the fill off, and by the time boarding opens
//!    the target's pod is refitted to `target_mk1` — no gun, no sight;
//! 3. she boards, starts up, holds the target as `BA`, locks it and fires until
//!    one of her rounds is told to her as a hit — each wait bounded by one cycle
//!    and the flight, never a fixed sleep;
//! 4. and then `SIM ABORT`, never a wall wait to the terminal head: her
//!    `SIM REPORT` carries her own hit rate, `SIM HISTORY` reads the row
//!    `aborted [range]`, the tape shows the target never fired, and after the
//!    next `SIM TEAM` the target's pod is a pod again.
//!
//! The elimination end of a range is `crates/tacsim/tests/weapons.rs`' cross-
//! tempo test's and the cockpit rig's (paused time), where arena time is free.

mod support;

use support::{STEP_TIMEOUT, wait_until};

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_core::module::GameModule;
use mush_core::{Engine, GOD, LIMBO, ObjId, ObjKind};
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

use support::{Term, play_client, scratch_dir, strip_ansi};

const GM_PASSWORD: &str = "gm-secret-1";
const PILOT_PASSWORD: &str = "pilot-secret-1";

/// The world's transient authoritative refusal: a commit is in flight.
const COMMIT_PENDING: &str = "please retry in a moment";
/// `BOARD`'s one transient: the control-lease mirror is a 1 Hz projection.
const CONTROL_UNKNOWN: &str = "control state unknown";
/// How many times a line is re-sent on a transient before the test gives up.
const RETRY_BUDGET: usize = 40;
/// How many rounds the test fires before it calls it. Every one is aimed at a
/// stationary target's rear from forty metres; the loop ends at the first hit.
const MAX_SHOTS: usize = 20;
/// **One cycle and the flight**, and a margin for the pulses either side: the
/// stock cannon cycles in 6,000 ms (`weapons/cannon_105.json`) and a round
/// crosses forty metres in well under a pulse. What a wait for the cannon's
/// readiness is bounded by.
const CYCLE_AND_FLIGHT: Duration = Duration::from_secs(12);

/// The one pilot, created before the world task spawns so its id is stable.
const ADA: ObjId = ObjId(2);

/// Blue's first pod's tank, as the shipped arena pack names it — the range's one
/// target once `SIM START` has refitted it. The object name is a property of the
/// OBJECT and a refit never touches it.
const BLUE_TANK: &str = "Blue Tank 1";

/// Where staff opens the target and the shooter (16b cut U.4's `@arena/pose`):
/// on the east-west road, four cells apart, both facing east — the shooter
/// behind the target, looking at its rear.
const TARGET_POSE: &str = "@arena/pose BA=130 97 90";
const SHOOTER_POSE: &str = "@arena/pose RA=126 97 90";

/// A line the parser refuses, used to bracket a multi-line answer.
const SENTINEL: &str = "zzz-sentinel";
/// The parser's refusal, which is the sentinel's answer.
const HUH: &str = "Huh?";

// ---------------------------------------------------------------------------
// The rig (`sim_units.rs`')
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

/// A fast arena: a short startup and a frequent summary. The AI fill stays
/// **off** — the range starts without it, and that is the claim.
fn tacsim_config() -> tacsim::TacsimConfig {
    let mut cfg = tacsim::TacsimConfig {
        enabled: true,
        ..tacsim::TacsimConfig::default()
    };
    cfg.arena.tempo.quiet_ms = 100;
    cfg.arena.tempo.alert_ms = 100;
    cfg.arena.tempo.engaged_ms = 100;
    cfg.arena.ledger.summary_ms = 200;
    cfg.match_.startup_ms = 300;
    cfg.match_.board_timeout_ms = 120_000;
    assert!(!cfg.match_.ai_fill, "the range is proved with the fill off");
    cfg
}

/// The production module list, in production order, with the starter area on.
fn module_list(
    onboarding: &mush_core::OnboardingConfig,
    llm: &Arc<LlmService>,
) -> Vec<Box<dyn GameModule>> {
    let mut mods = modules::server_modules();
    mods.push(modules::worldgen_module(Arc::clone(llm)));
    mods.push(modules::npc_module(modules::npc_agent_adapter()));
    mods.extend(modules::sheet_modules(onboarding, &Default::default()));
    mods.push(modules::tacsim_module(tacsim_config()));
    mods.push(modules::content_module_with(true, true).expect("the content module composes"));
    mods
}

async fn boot(tag: &str) -> Server {
    let dir = scratch_dir(tag);
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
        .expect("the staff account");

    let mut world = load_or_seed_world(&*store).await.expect("boot world");
    let made = world
        .create(ObjKind::Player, "Ada", GOD, Some(LIMBO))
        .expect("a pilot");
    assert_eq!(made, ADA, "pilot id assumption");
    store
        .create_account(AccountRec {
            id: None,
            name: "ada".to_string(),
            argon2_hash: hash_password(PILOT_PASSWORD).expect("a hash"),
            created: i64::from(ADA.0),
            characters: vec![CharacterRef {
                id: ADA,
                name: "Ada".to_string(),
            }],
            ssh_keys: vec![],
            discord: None,
        })
        .await
        .expect("a pilot account");

    let onboarding = mush_core::OnboardingConfig {
        chargen_required: false,
        start_room: LIMBO,
        start_room_unapproved: LIMBO,
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
    let identity: Arc<dyn mush_core::AccountIdentity> = Arc::new(
        mush_server::accounts_identity::hydrate(&*store)
            .await
            .expect("the account map hydrates"),
    );
    engine.set_account_identity(Arc::clone(&identity));
    if let Some(slot) = engine.world.module_state_mut::<mush_sim::AccountOracle>() {
        *slot = mush_sim::AccountOracle::new(identity);
    }

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
    let accounts = Arc::new(Accounts::new(Arc::clone(&store), handles.handle.clone(), 6));
    let bridge = Arc::new(SshBridge::new(
        accounts,
        Arc::clone(&sessions),
        handles.handle.clone(),
        Arc::clone(&guard),
    ));
    let listener = start_listener(
        "127.0.0.1:0".parse().expect("a loopback address"),
        &dir,
        "Welcome to the simulator.".to_string(),
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

/// Connect one account and wait for its character to land.
async fn login(
    addr: SocketAddr,
    account: &str,
    password: &str,
    character: &str,
) -> (russh::client::Handle<support::AcceptAll>, Term) {
    let (handle, mut term) = play_client(addr).await;
    term.expect("Welcome to the simulator.").await;
    term.send_line(&format!("connect {account} {password}"))
        .await;
    term.expect(&format!("Connected as {character}")).await;
    (handle, term)
}

/// Run one line and wait for `needle`, retrying the two transients.
async fn say(term: &mut Term, line: &str, needle: &str) {
    assert!(
        !line.contains(needle),
        "`{needle}` appears in `{line}`'s own echo; pick a needle from the ANSWER"
    );
    for _ in 0..RETRY_BUDGET {
        let mark = term.mark();
        term.send_line(line).await;
        match term
            .expect_one_of_after(mark, &[needle, COMMIT_PENDING, CONTROL_UNKNOWN])
            .await
        {
            0 => return,
            _ => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    }
    panic!("`{line}` was refused as transient {RETRY_BUDGET} times running");
}

/// Run one line, wait for `needle`, and hand back everything that arrived.
async fn ask(term: &mut Term, line: &str, needle: &str) -> String {
    let mark = term.mark();
    say(term, line, needle).await;
    strip_ansi(&term.seen[mark..])
}

/// **A whole answer, never half of one**: the line, then a sentinel the parser
/// refuses, so the whole of a multi-line answer has arrived by the refusal.
async fn ask_whole(term: &mut Term, line: &str) -> String {
    for _ in 0..RETRY_BUDGET {
        let mark = term.mark();
        term.send_line(line).await;
        term.send_line(SENTINEL).await;
        if term.expect_one_of_after(mark, &[HUH, COMMIT_PENDING]).await == 0 {
            return strip_ansi(&term.seen[mark..]);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("`{line}` never produced a whole answer")
}

/// A read repeated until its answer carries `needle`.
async fn poll_whole(term: &mut Term, line: &str, needle: &str) -> String {
    let mut seen = String::new();
    wait_until(
        STEP_TIMEOUT,
        &format!("`{line}` to say `{needle}`"),
        async || {
            seen = ask_whole(term, line).await;
            seen.contains(needle)
        },
    )
    .await;
    seen
}

/// One object's dbref, out of an `@find` listing.
fn dbref_of(listing: &str, name: &str) -> u32 {
    listing
        .lines()
        .find_map(|line| {
            let rest = line.split_once(name)?.1;
            let inside = rest.split_once("(#")?.1;
            inside.split(')').next()?.trim().parse::<u32>().ok()
        })
        .unwrap_or_else(|| panic!("no dbref for {name:?} in:\n{listing}"))
}

/// **The short designator in a `CONTACTS` listing** (16b cut Q.1).
fn designator_of(listing: &str) -> Option<String> {
    listing
        .lines()
        .find(|line| line.contains("bearing ") && line.contains("(rel "))
        .and_then(|line| line.split_whitespace().next())
        .map(str::to_owned)
}

/// Bring power up, waiting for the arena to say it is online.
///
/// Each `STATUS` is read WHOLE ([`ask_whole`]). A needle of `power` is not
/// this answer's alone: the panel hint (`…brings your power up.`) and the
/// previous poll's own reply both carry it, so a poll could return on either
/// and leave its `STATUS` in flight, to land after whatever the caller marks
/// next.
async fn power_up(term: &mut Term) {
    say(term, "STARTUP", "Startup sequence").await;
    wait_until(STEP_TIMEOUT, "the tank to come online", async || {
        ask_whole(term, "STATUS").await.contains("power online")
    })
    .await;
}

/// Wait until `CONTACTS` holds a return told apart, and answer its designator.
async fn wait_for_contact(term: &mut Term) -> String {
    let mut contact = None;
    wait_until(
        STEP_TIMEOUT,
        "the sensor to tell a contact apart",
        async || {
            // Read WHOLE: a poll that returned at the first row would leave
            // the rest of the listing unread in the channel, to open the
            // caller's next mark on a stray contact row. Only a row carries
            // `(rel ` — the acquisition notice's bearing is absolute.
            let seen = ask_whole(term, "CONTACTS").await;
            if seen.contains("(rel ") {
                contact = designator_of(&seen).filter(|id| !id.starts_with('X'));
            }
            contact.is_some()
        },
    )
    .await;
    contact.expect("waited for a contact")
}

/// Every ledger line one match wrote, parsed.
async fn ledger_events(store: &Arc<dyn ObjectStore>, match_id: u64) -> Vec<Value> {
    let log = format!("match.{match_id}");
    let mut events = Vec::new();
    for chunk in 0..256 {
        let key = mush_core::chunk_key(&log, chunk);
        let Some(value) = store
            .module_json_get("tacsim", key.as_str())
            .await
            .expect("the store read")
        else {
            break;
        };
        let mut lines: Vec<String> = Vec::new();
        collect_lines(&value, &mut lines);
        for blob in lines {
            for line in blob.lines().filter(|line| !line.trim().is_empty()) {
                if let Ok(event) = serde_json::from_str::<Value>(line) {
                    events.push(event);
                }
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
        Value::Array(items) => items.iter().for_each(|item| collect_lines(item, out)),
        _ => {}
    }
}

/// The lines of one kind a unit is the `unit` of.
fn count_by(events: &[Value], kind: &str, unit: u32) -> usize {
    events
        .iter()
        .filter(|event| {
            event.get("kind").and_then(Value::as_str) == Some(kind)
                && event.get("unit").and_then(Value::as_u64) == Some(u64::from(unit))
        })
        .count()
}

/// **`SIM REPORT`, read whole**: the report document rides the result's batch,
/// so the first read may find it still being written; it is asked again until
/// the answer ends, then a sentinel brings in the rest of what it said.
async fn sim_report(term: &mut Term) -> String {
    let mut report = String::new();
    wait_until(STEP_TIMEOUT, "the report to be read whole", async || {
        let mark = term.mark();
        term.send_line("SIM REPORT").await;
        let at = term
            .expect_one_of_after(
                mark,
                &[
                    "the match was aborted.",
                    "still being written",
                    "already being read",
                    COMMIT_PENDING,
                ],
            )
            .await;
        if at != 0 {
            return false;
        }
        term.send_line(SENTINEL).await;
        term.expect_after(mark, HUH).await;
        report = strip_ansi(&term.seen[mark..]);
        true
    })
    .await;
    report
}

/// **The hit rate `You flew …` ends with** (16b cut O.3): the per cent in `(N%)`.
fn hit_rate(report: &str) -> Option<u64> {
    let line = report.lines().find(|line| line.contains("You flew RA: "))?;
    let (_, tail) = line.rsplit_once('(')?;
    tail.split_once("%)")?.0.trim().parse().ok()
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_pilot_on_the_range_hits_its_target_and_reads_her_own_rate() {
    let server = boot("sim-range").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let listing = ask(&mut god, "@find Simulator Lobby", "END OF LIST").await;
    let lobby = dbref_of(&listing, "Simulator Lobby");
    say(
        &mut god,
        &format!("@teleport #{}=#{lobby}", ADA.0),
        "Teleported",
    )
    .await;
    let listing = ask(&mut god, &format!("@find {BLUE_TANK}"), "END OF LIST").await;
    let target = dbref_of(&listing, BLUE_TANK);

    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;

    // ---- the range, set up by one pilot on a fill-off server ----------------
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    let seen = ask(&mut ada, "SIM AI", "AI fill").await;
    assert!(
        seen.contains("AI fill is off on this server."),
        "the fill is off here: {seen}"
    );
    say(
        &mut ada,
        "SIM RANGE 1",
        // The answer's head: a terminal wraps the whole sentence at eighty columns.
        "Range set: 1 target stands on the blue side;",
    )
    .await;
    let seen = ask_whole(&mut ada, "SIM STATUS").await;
    let flat = seen.split_whitespace().collect::<Vec<&str>>().join(" ");
    assert!(
        flat.contains("mode: range \u{2014} 1 target on the blue side"),
        "{seen}"
    );
    say(
        &mut god,
        TARGET_POSE,
        "BA opens at cell (130, 97) heading 90 for the next match.",
    )
    .await;
    say(
        &mut god,
        SHOOTER_POSE,
        "RA opens at cell (126, 97) heading 90 for the next match.",
    )
    .await;

    // ---- the start, with the fill off, and the refit -----------------------
    say(&mut ada, "SIM START", "is boarding").await;
    let seen = ask_whole(&mut god, &format!("@unit/show #{target}")).await;
    assert!(
        seen.contains("section engine") && !seen.contains("component/cannon"),
        "blue 1's tank was not refitted into a target: {seen}"
    );
    say(&mut ada, "red", "Red Room").await;
    say(&mut ada, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;

    // ---- the shooting --------------------------------------------------------
    power_up(&mut ada).await;
    let contact = wait_for_contact(&mut ada).await;
    assert_eq!(contact, "BA", "the target answers to its pod's callsign");
    // The lock is the arena's at its next pulse: a `FIRE` sent before it lands
    // is refused `No lock`, so the lock is waited for on the lock's own line.
    let locking = ada.mark();
    say(&mut ada, "LOCK BA", "Locking").await;
    ada.expect_after(locking, "Locked on BA").await;
    let mut hit = false;
    for _ in 0..MAX_SHOTS {
        // The cannon ready first, bounded by one cycle and the flight.
        wait_until(CYCLE_AND_FLIGHT, "the cannon to be ready", async || {
            ask_whole(&mut ada, "WEAPONSTATUS")
                .await
                .lines()
                .any(|line| line.contains("cannon") && line.contains("ready"))
        })
        .await;
        let shot = ada.mark();
        ada.send_line("FIRE").await;
        // The outcome is told at the impact, a pulse or two after the shot.
        let outcome = ada
            .expect_one_of_after(shot, &["Hit on BA", "Miss on BA", "refused"])
            .await;
        if outcome == 0 {
            hit = true;
            break;
        }
    }
    assert!(
        hit,
        "{MAX_SHOTS} rounds at a standing target and not one hit"
    );

    // ---- `SIM ABORT`, and what the pilot reads after ------------------------
    ada.send_line("SIM ABORT").await;
    poll_whole(&mut ada, "SIM STATUS", "aborted").await;

    let report = sim_report(&mut ada).await;
    let rate =
        hit_rate(&report).unwrap_or_else(|| panic!("`You flew RA:` carries no rate:\n{report}"));
    assert!(
        (1..=100).contains(&rate),
        "a hit rate of {rate}%:\n{report}"
    );
    for line in report.lines() {
        assert!(
            !line.contains("hit on ") && !line.contains("your hull"),
            "the report names a round the target fired: {line}"
        );
    }

    let history = ask_whole(&mut ada, "SIM HISTORY").await;
    let flat = history.split_whitespace().collect::<Vec<&str>>().join(" ");
    assert!(flat.contains("#1 aborted [range] "), "{history}");
    assert!(flat.contains("(you flew RA)"), "{history}");

    // The tape: Ada's rounds landed, and the target never fired one.
    let mut events = Vec::new();
    wait_until(STEP_TIMEOUT, "the range's hit to persist", async || {
        events = ledger_events(&server.store, 1).await;
        events
            .iter()
            .any(|event| event.get("kind").and_then(Value::as_str) == Some("hit"))
    })
    .await;
    assert_eq!(count_by(&events, "fired", target), 0, "a target fired");

    // ---- the pod is a pod again ---------------------------------------------
    say(&mut ada, "lobby", "Simulator Lobby").await;
    poll_whole(&mut ada, "SIM STATUS", "SIM TEAM red").await;
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    let seen = ask_whole(&mut god, &format!("@unit/show #{target}")).await;
    assert!(
        seen.contains("section turret") && seen.contains("component/cannon"),
        "blue 1's tank is still a target: {seen}"
    );
    let seen = ask_whole(&mut ada, "SIM RANGE").await;
    assert!(
        seen.contains("The range is not set; SIM RANGE <n> sets it in selection."),
        "the range went with the last match: {seen}"
    );
}
