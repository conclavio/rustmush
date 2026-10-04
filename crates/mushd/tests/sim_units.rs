//! **Two pilots fielding two different machines, over real SSH** (16b cut Q.8;
//! doc 16 §3.3 "The stock tanks are persistent objects", §3.8 "Lifecycle", §9
//! items 1 and 2; 16b §9's cut Q row).
//!
//! Cuts X.1 to X.5 built five extension units and no player could fly one: the
//! pods are bound to the arena pack's `line_tank` slots and the extension
//! content only tests ever loaded. This is the lens for the cut that fielded
//! them, end to end and over the production composition `sim_rematch.rs` boots:
//!
//! 1. Ada takes `SIM UNIT biped_mk1` and Bo takes `SIM UNIT line_tank_laser`;
//! 2. `SIM START` refits both pods' tanks **before the pods lock**, and `BOARD`
//!    and `STARTUP` open cockpits on the machines they chose — Ada's `STATUS`
//!    prints the biped's own six sections, Bo's prints the heat the beam makes,
//!    and neither of those lines exists on a stock tank;
//! 3. they fight, and a hit lands;
//! 4. and then the pods are pods again: after the match, the next `SIM TEAM` is
//!    what puts both tanks back to `line_tank` at spec, with no staff step.
//!
//! # The observer rule, first and last
//!
//! Doc 16 §1 and §5: *nothing tells a pilot anything about a unit their platform
//! has not detected*. A chosen unit type is exactly the kind of thing that would
//! leak — the lobby knows it, the pack knows it, and the other pilot must not
//! until their own sensors have told them apart. So:
//!
//! - before either of them has classified anything, no cockpit surface carries
//!   the other machine's object name, its type word, or a contact id;
//! - `SIM STATUS` is readable from inside a pod, and once the match is on it
//!   prints no machine's type — the bracket is a `Selection` affordance;
//! - `SCAN` on an unidentified return is the one refusal, and the biped's own
//!   section words appear **only** in a reading Bo's own platform took.
//!
//! The whole-transcript sweep at the end reads every byte either session saw,
//! which is the claim that does not race the sensor stagger. Two things are
//! deliberately **outside** it, and are asserted as present rather than absent:
//!
//! - during `Selection` the board names both sides' chosen machines, and
//!   `SIM UNIT <type>` is announced to the centre's rooms and pods the way a
//!   claim is. That is a lobby negotiation over an OOC board, before any unit is
//!   in the arena, and it is what lets an opponent answer a choice;
//! - the board names both sides' pilots in every state, which is the owner's
//!   ruling on doc 16 §5 (cut Q, open question 2: the centre is an OOC
//!   simulator). So the sweep tests the other machine's **object name** and
//!   **type word**, and not a pilot's name.

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
/// How many rounds of the fight the test is allowed before it calls it. The
/// cannon cycles in seconds, so this is a wall-clock budget as much as a shot
/// count; the loop breaks on the first hit either pilot sees.
const MAX_SHOTS: usize = 40;

/// The two pilots, created before the world task spawns so their ids are stable.
const ADA: ObjId = ObjId(2);
const BO: ObjId = ObjId(3);

/// Slot 1 of each side, as the shipped arena pack names them. The object name is
/// a property of the OBJECT and a refit never touches one, which is why these
/// still find the pods' tanks after both have changed type.
const RED_TANK: &str = "Red Tank 1";
const BLUE_TANK: &str = "Blue Tank 1";

/// Where staff parks the two tanks, in centimetres of the arena's own frame —
/// `sim_solo.rs`'s muster, for its reasons: row 97 is the east-west road, and two
/// hundred metres is inside the cannon's band and inside the close-look range a
/// `SCAN` reading needs.
const RED_X_CM: i64 = 100_500;
const BLUE_X_CM: i64 = 120_500;
const ROW_Y_CM: i64 = 97_500;
const EAST_MDEG: u32 = 90_000;
const WEST_MDEG: u32 = 270_000;

/// The cannon's authored load (`ammunition/he_105.json`), which is what "at
/// spec" means for a `line_tank`'s magazine.
const LOAD: i64 = 40;

/// A line the parser refuses, used to bracket a multi-line answer.
const SENTINEL: &str = "zzz-sentinel";
/// The parser's refusal, which is the sentinel's answer.
const HUH: &str = "Huh?";

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

/// A fast arena: a short startup and a frequent summary, so what the test waits
/// for is the fight and not the clock. The AI fill stays **off** — this file is
/// about two humans choosing two machines.
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
    cfg
}

/// The production module list, in production order, with the starter area on.
fn module_list(
    onboarding: &mush_core::OnboardingConfig,
    llm: &Arc<LlmService>,
    cfg: tacsim::TacsimConfig,
) -> Vec<Box<dyn GameModule>> {
    let mut mods = modules::server_modules();
    mods.push(modules::worldgen_module(Arc::clone(llm)));
    mods.push(modules::npc_module(modules::npc_agent_adapter()));
    mods.extend(modules::sheet_modules(onboarding, &Default::default()));
    mods.push(modules::tacsim_module(cfg));
    mods.push(modules::content_module_with(true, true).expect("the content module composes"));
    mods
}

async fn boot(tag: &str) -> Server {
    boot_with(tag, tacsim_config()).await
}

/// **[`boot`] on a named `[tacsim]`** (16b cut AE.3): the drone can only board
/// a match whose side also carries a gun, and with two pilots and the fill off
/// that is no match at all, so one test boots with the fill on.
async fn boot_with(tag: &str, cfg: tacsim::TacsimConfig) -> Server {
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
    for (id, name) in [(ADA, "Ada"), (BO, "Bo")] {
        let made = world
            .create(ObjKind::Player, name, GOD, Some(LIMBO))
            .expect("a pilot");
        assert_eq!(made, id, "pilot id assumption");
        store
            .create_account(AccountRec {
                id: None,
                name: name.to_lowercase(),
                argon2_hash: hash_password(PILOT_PASSWORD).expect("a hash"),
                created: i64::from(id.0),
                characters: vec![CharacterRef {
                    id,
                    name: name.to_string(),
                }],
                ssh_keys: vec![],
                discord: None,
            })
            .await
            .expect("a pilot account");
    }

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

    let mut engine = Engine::with_modules(world, registry, module_list(&onboarding, &llm, cfg))
        .expect("modules");
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

/// **A whole answer, never half of one** — `arena_story.rs`' device: the line,
/// then a sentinel the parser refuses, so the whole of a multi-line answer has
/// certainly arrived by the time the refusal does.
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

/// One cell's current value out of an `@unit/show` listing
/// (`cell component/cannon/ammo: 40/40` answers `40`).
fn cell_current(listing: &str, path: &str) -> i64 {
    listing
        .lines()
        .filter_map(|line| {
            let rest = line.split_once(&format!("cell {path}: "))?.1;
            rest.split('/').next()?.trim().parse::<i64>().ok()
        })
        .next_back()
        .unwrap_or_else(|| panic!("no cell {path:?} in:\n{listing}"))
}

/// **The short designator in a `CONTACTS` listing** (16b cut Q.1): the first word
/// of a contact row, which is `X<n>` while the return is unresolved and the
/// team's letter with the target's centre slot once it has been told apart.
fn designator_of(listing: &str) -> Option<String> {
    listing
        .lines()
        .find(|line| line.contains("bearing ") && line.contains("(rel "))
        .and_then(|line| line.split_whitespace().next())
        .map(str::to_owned)
}

/// Put staff and both pilots in the lobby, and park the two tanks in sight of
/// each other on the muster road.
async fn open_the_centre(god: &mut Term) {
    let listing = ask(god, "@find Simulator Lobby", "END OF LIST").await;
    let lobby = dbref_of(&listing, "Simulator Lobby");
    for pilot in [ADA, BO] {
        say(
            god,
            &format!("@teleport #{}=#{lobby}", pilot.0),
            "Teleported",
        )
        .await;
    }
    say(
        god,
        &format!("@teleport #{}=#{lobby}", GOD.0),
        "Simulator Lobby",
    )
    .await;

    let listing = ask(god, "@find Proving Ground", "END OF LIST").await;
    let ground = dbref_of(&listing, "Proving Ground");
    for (name, x_cm, heading) in [
        (RED_TANK, RED_X_CM, EAST_MDEG),
        (BLUE_TANK, BLUE_X_CM, WEST_MDEG),
    ] {
        let listing = ask(god, &format!("@find {name}"), "END OF LIST").await;
        let tank = dbref_of(&listing, name);
        say(
            god,
            &format!(
                "@spatial/place/domain #{tank}=#{ground},{{\"x_cm\":{x_cm},\"y_cm\":{ROW_Y_CM},\
                 \"heading_mdeg\":{heading}}}"
            ),
            "Placed",
        )
        .await;
    }
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
/// Wait until `CONTACTS` has something in it, and answer its designator.
async fn wait_for_contact(term: &mut Term) -> String {
    let mut contact = None;
    wait_until(
        STEP_TIMEOUT,
        "the sensor to acquire a contact",
        async || {
            // Read WHOLE: a poll that returned at the first row would leave
            // the rest of the listing unread in the channel, to open the
            // caller's next mark on a stray contact row. Only a row carries
            // `(rel ` — the acquisition notice's bearing is absolute.
            let seen = ask_whole(term, "CONTACTS").await;
            if seen.contains("(rel ") {
                // Keep the classified callsign: a provisional X-number may be
                // renamed during subsequent status/weapon assertions.
                if seen.contains("blue") || seen.contains("red") {
                    contact = designator_of(&seen).filter(|id| !id.starts_with('X'));
                }
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

fn count(events: &[Value], kind: &str) -> usize {
    events
        .iter()
        .filter(|event| event.get("kind").and_then(Value::as_str) == Some(kind))
        .count()
}

/// **Nothing ever printed a bare contact id** (doc 16 §5).
fn no_sixteen_digit_word(seen: &str) {
    for word in seen.split(|c: char| !c.is_ascii_hexdigit()) {
        assert!(
            word.len() != 16 || u64::from_str_radix(word, 16).is_err(),
            "a line carried a sixteen-digit contact id ({word})"
        );
    }
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_pilots_field_two_different_machines_and_the_pods_are_pods_again() {
    let server = boot("sim-units").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;
    let listing = ask(&mut god, &format!("@find {RED_TANK}"), "END OF LIST").await;
    let red = dbref_of(&listing, RED_TANK);
    let listing = ask(&mut god, &format!("@find {BLUE_TANK}"), "END OF LIST").await;
    let blue = dbref_of(&listing, BLUE_TANK);

    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;

    // ---- the catalog the shipped server fields -----------------------------
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    // 16b cut AE.1: ten types wrap the sentence at the terminal's width, so the
    // answer is read whole — a needle from its opening could return before the
    // wrapped rest has arrived.
    let seen = ask_whole(&mut ada, "SIM UNIT").await;
    assert!(seen.contains("Fieldable units:"), "{seen}");
    for word in [
        "line_tank",
        "line_tank_laser",
        "biped_mk1",
        "line_tank_thermal",
        "line_tank_ecm",
        "line_tank_eccm",
    ] {
        assert!(
            seen.contains(word),
            "doc 16 section 9's items are not fielded: {seen}"
        );
    }
    // And with the wraps undone.
    assert!(
        seen.split_whitespace()
            .collect::<Vec<&str>>()
            .join(" ")
            .contains("Yours: line_tank."),
        "a slot that chose nothing flies its pod's own tank: {seen}"
    );

    // ---- two pilots, two machines ------------------------------------------
    say(
        &mut ada,
        "SIM UNIT biped_mk1",
        "Slot red 1 will field biped_mk1.",
    )
    .await;
    say(
        &mut bo,
        "SIM UNIT line_tank_laser",
        "Slot blue 1 will field line_tank_laser.",
    )
    .await;
    let seen = ask_whole(&mut ada, "SIM STATUS").await;
    assert!(seen.contains("[biped_mk1]"), "{seen}");
    assert!(seen.contains("[line_tank_laser]"), "{seen}");

    say(&mut ada, "SIM START", "is boarding").await;
    // The refit is done by the time boarding opens, and staff can read it.
    let seen = ask_whole(&mut god, &format!("@unit/show #{red}")).await;
    assert!(
        seen.contains("section left_leg") && seen.contains("section torso"),
        "red 1's tank was not refitted into a biped: {seen}"
    );

    say(&mut ada, "red", "Red Room").await;
    say(&mut bo, "blue", "Blue Room").await;
    say(&mut ada, "BOARD", "climb into").await;
    say(&mut bo, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;

    // ---- the observer rule, before anything is identified -------------------
    //
    // A guessed designator nobody is flying is **one** refusal: doc 16 §5 wants
    // it byte-identical whoever asks and whatever they guessed, because a
    // refusal that varied would itself be a probe. So the two answers are
    // compared to each other, not merely to the needle they were waited on.
    let mut refusals = Vec::new();
    for term in [&mut ada, &mut bo] {
        let seen = ask(term, "LOCK RD", "No such contact.").await;
        refusals.push(
            seen.lines()
                .find(|line| line.contains("No such contact."))
                .map(str::trim)
                .unwrap_or_default()
                .to_owned(),
        );
    }
    assert_eq!(
        refusals[0], refusals[1],
        "the refusal for a contact nobody holds is one line, not a per-pilot one"
    );

    // The WHOLE window — the title row, the grid, the legend and the
    // beyond-window sentence. `ask` would have returned at the title, which is
    // the first row `view_window` writes and none of the rows a leak would be
    // in.
    let seen = ask_whole(&mut bo, "VIEW").await;
    assert!(seen.contains("Arena window"), "{seen}");
    for leak in [RED_TANK, "biped", "Ada"] {
        assert!(!seen.contains(leak), "the window carried `{leak}`: {seen}");
    }

    // And the board, read from inside the pod, where `SIM` is still admitted:
    // the match is on, so it names nobody's machine.
    let seen = ask_whole(&mut bo, "SIM STATUS").await;
    assert!(
        seen.contains("Simulator centre:"),
        "the board answers from a pod: {seen}"
    );
    for leak in ["biped_mk1", "line_tank_laser", "[line_tank"] {
        assert!(
            !seen.contains(leak),
            "the board named a machine once the match was on (`{leak}`): {seen}"
        );
    }

    // ---- the machines they actually chose ----------------------------------
    power_up(&mut ada).await;
    power_up(&mut bo).await;

    let seen = ask_whole(&mut ada, "STATUS").await;
    for section in ["torso", "head", "left_leg", "right_leg"] {
        assert!(
            seen.contains(&format!("  {section}  intact")),
            "the biped's own sections are not in its pilot's STATUS: {seen}"
        );
    }
    assert!(
        !seen.contains("  turret  "),
        "it is not a tank any more: {seen}"
    );
    assert!(
        !seen.contains("  heat "),
        "the biped mounts nothing that makes heat: {seen}"
    );

    let seen = ask_whole(&mut bo, "STATUS").await;
    assert!(
        seen.contains("  heat 0 of 60"),
        "the laser tank's heat line is not in its pilot's STATUS: {seen}"
    );
    assert!(
        seen.contains("  turret  intact"),
        "it is still a tank: {seen}"
    );
    let seen = ask(&mut bo, "WEAPONSTATUS", "laser").await;
    assert!(
        seen.contains("laser") && !seen.contains("cannon"),
        "the laser tank carries a beam and no cannon: {seen}"
    );

    // ---- the fight ----------------------------------------------------------
    //
    // Ada's biped mounts the stock cannon, so her bare `FIRE` fires it. Bo's
    // tank carries no cannon at all, so his bare `FIRE` is his beam: the recorded
    // limit is that `DEFAULT_WEAPON` is `cannon` and the fallback is the first
    // weapon the platform carries, which for `line_tank_laser` is the laser.
    //
    // Both of them shoot, because the cannon's cycle is measured in seconds and
    // one gun firing into a per-mille roll is a test waiting out its own budget
    // for an outcome that is a draw. Whether a *given* round lands is not the
    // claim; the claim is that rounds from these two machines land at all, and
    // the ledger at the end is what carries it.
    let contact = wait_for_contact(&mut ada).await;
    say(&mut ada, &format!("LOCK {contact}"), "Locking").await;
    poll_whole(&mut ada, "WEAPONSTATUS", "Locked on").await;
    let theirs = wait_for_contact(&mut bo).await;
    say(&mut bo, &format!("LOCK {theirs}"), "Locking").await;

    let ada_start = ada.mark();
    let bo_start = bo.mark();
    for _ in 0..MAX_SHOTS {
        // Each line is followed by a wait on its own **echo**, which always comes
        // back whatever the answer is: the client's SSH session only advances
        // while somebody is awaiting the channel, so a loop that sent without
        // reading would stall the window and hang the test rather than fail it.
        for (term, line) in [(&mut ada, "FIRE"), (&mut bo, "FIRE")] {
            let mark = term.mark();
            term.send_line(line).await;
            term.expect_after(mark, line).await;
        }
        tokio::time::sleep(Duration::from_millis(800)).await;
        let fight = format!(
            "{}{}",
            strip_ansi(&ada.seen[ada_start..]),
            strip_ansi(&bo.seen[bo_start..])
        );
        if fight.contains("Hit on ") || fight.contains("Hit.") || fight.contains("is over") {
            break;
        }
    }

    // ---- the reading Bo's OWN platform took ---------------------------------
    //
    // `SCAN` names the biped's sections and nothing else does. Whether Bo has a
    // reading yet is a fact about his own sensors, so the two answers the rule
    // admits are the reading and the refusal that names no section at all.
    // The listing is read with the sentinel, so the answer is certainly whole
    // and "no designator" means Bo's board is really empty rather than slow —
    // and the empty case asserts that instead of asserting nothing.
    let listing = ask_whole(&mut bo, "CONTACTS").await;
    match designator_of(&listing) {
        Some(designator) => {
            let scanned = ask_whole(&mut bo, &format!("SCAN {designator}")).await;
            if scanned.contains("Scan of ") {
                assert!(
                    scanned.contains("torso"),
                    "a reading of a biped names the biped's sections: {scanned}"
                );
            } else {
                for word in ["torso", "left_leg", "right_leg"] {
                    assert!(
                        !scanned.contains(word),
                        "a refusal named a section anyway: {scanned}"
                    );
                }
            }
        }
        None => assert!(
            listing.contains("No contacts."),
            "CONTACTS answered neither a contact nor an empty board: {listing}"
        ),
    }

    // ---- and the pods are pods again ----------------------------------------
    //
    // No staff step: 16b cut F.1's claim-clears-the-board, now putting the TYPE
    // back as well as the cells.
    // The abort is asynchronous — the arena's `Ended`, the egress pump, the
    // check-in, the eject — so the wait is on the observable the teardown
    // publishes and not on a guess at how long it takes. A fixed sleep here is a
    // flake that reads as a product failure: `say` retries only the two
    // transients, and "you are still seated" is not one of them.
    ada.send_line("SIM ABORT").await;
    poll_whole(&mut ada, "SIM STATUS", "aborted").await;
    say(&mut ada, "lobby", "Simulator Lobby").await;
    say(&mut bo, "lobby", "Simulator Lobby").await;
    poll_whole(&mut ada, "SIM STATUS", "SIM TEAM red").await;
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;

    for (unit, who) in [(red, RED_TANK), (blue, BLUE_TANK)] {
        let seen = ask_whole(&mut god, &format!("@unit/show #{unit}")).await;
        assert!(
            seen.contains("section turret") && seen.contains("section hull"),
            "{who} is not a line tank again: {seen}"
        );
        assert!(
            !seen.contains("section left_leg"),
            "{who} is still last match's biped: {seen}"
        );
        assert!(
            !seen.contains("component/laser"),
            "{who} is still last match's laser tank: {seen}"
        );
        assert_eq!(
            cell_current(&seen, "component/cannon/ammo"),
            LOAD,
            "{who} did not come back with a full magazine: {seen}"
        );
        assert!(
            seen.contains("section hull: intact"),
            "{who}'s hull is not intact: {seen}"
        );
    }

    // ---- the ledger, and the observer rule over every byte -------------------
    let mut events = Vec::new();
    wait_until(
        STEP_TIMEOUT,
        "the unit match's hit to persist",
        async || {
            events = ledger_events(&server.store, 1).await;
            count(&events, "hit") > 0
        },
    )
    .await;
    assert!(
        count(&events, "hit") > 0,
        "the match ledger recorded no hit: {:?}",
        events
            .iter()
            .filter_map(|event| event.get("kind").and_then(Value::as_str))
            .take(30)
            .collect::<Vec<_>>()
    );

    for (whole, theirs, kind) in [
        (strip_ansi(&ada.seen), BLUE_TANK, "line_tank_laser"),
        (strip_ansi(&bo.seen), RED_TANK, "biped_mk1"),
    ] {
        assert!(
            !whole.contains(theirs),
            "a surface named the other machine by its object name"
        );
        // The pilot's OWN lobby lines carry their own choice, and the announced
        // `SIM UNIT` line carries the other side's — both deliberately, and both
        // during `Selection` only (see this file's header). So the sweep starts
        // where the arena does: from the moment either of them climbed into a
        // pod, no surface either session saw names the other machine's type.
        let cockpit = whole
            .rfind("climb into")
            .map_or_else(|| whole.clone(), |at| whole[at..].to_owned());
        assert!(
            !cockpit.contains(kind),
            "a cockpit surface named the other machine's type ({kind})"
        );
        no_sixteen_digit_word(&whole);
    }
}

// ---------------------------------------------------------------------------
// 16b cut O.2 — the pod-carrying tank, switched off over SSH (16c N12)
// ---------------------------------------------------------------------------

/// Where staff poses the two slot-1 tanks for the ECM match (16b cut O.2): on
/// the east-west road, four cells apart, so the stock tank stands inside the
/// Mk1 pod's 60 m bubble from the first substep.
const O2_RED_POSE: &str = "@arena/pose RA=100 97 90";
const O2_BLUE_POSE: &str = "@arena/pose BA=104 97 270";

/// **The pod's switch, over the production composition** (16b cut O.2; 16c
/// N12). No mushd test flew the ECM tank before this cut, so the setup is its
/// own: Ada takes `SIM UNIT line_tank_ecm` in `Selection`, Bo keeps the stock
/// tank on the other side, and staff poses both through `@arena/pose` before
/// `SIM START` with Bo inside the pod's 60 m. Ada reads her switch on at
/// check-out, powers up (a pod radiates only with power up) and reads her own
/// receiver degraded — a pod is in its own bubble — then types `EMCON off`: the
/// reply, the notice that says what it costs, her receiver clearing by the
/// ordinary path, the `STATUS` line; and after `SIM ABORT` the tape carries the
/// one `emcon` line for her tank.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn o2_a_pilot_in_the_ecm_tank_switches_the_pod_off_and_the_tape_says_so() {
    let server = boot("sim-units-emcon").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let listing = ask(&mut god, "@find Simulator Lobby", "END OF LIST").await;
    let lobby = dbref_of(&listing, "Simulator Lobby");
    for pilot in [ADA, BO] {
        say(
            &mut god,
            &format!("@teleport #{}=#{lobby}", pilot.0),
            "Teleported",
        )
        .await;
    }
    let listing = ask(&mut god, &format!("@find {RED_TANK}"), "END OF LIST").await;
    let red = dbref_of(&listing, RED_TANK);

    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(
        &mut ada,
        "SIM UNIT line_tank_ecm",
        "Slot red 1 will field line_tank_ecm.",
    )
    .await;
    say(
        &mut god,
        O2_RED_POSE,
        "RA opens at cell (100, 97) heading 90 for the next match.",
    )
    .await;
    say(
        &mut god,
        O2_BLUE_POSE,
        "BA opens at cell (104, 97) heading 270 for the next match.",
    )
    .await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    say(&mut bo, "blue", "Blue Room").await;
    say(&mut ada, "BOARD", "climb into").await;
    say(&mut bo, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;

    // On at check-out, whatever the power.
    poll_whole(&mut ada, "EMCON", "EMCON on: the pod radiates.").await;
    let seen = ask_whole(&mut bo, "EMCON").await;
    assert!(
        seen.contains("This tank carries no jamming pod."),
        "the stock tank has a pod: {seen}"
    );
    power_up(&mut ada).await;
    let seen = poll_whole(&mut ada, "STATUS", "receiver degraded").await;
    assert!(
        seen.contains("  emcon on: the pod radiates"),
        "STATUS does not carry the switch: {seen}"
    );

    // The switch.
    let mark = ada.mark();
    say(
        &mut ada,
        "EMCON off",
        "EMCON going off; the pod is silent from the next pulse.",
    )
    .await;
    ada.expect_after(
        mark,
        "EMCON off: the pod is silent; your own sight clears, and nothing hides you.",
    )
    .await;
    ada.expect_after(mark, "Receiver clear.").await;
    let seen = poll_whole(&mut ada, "STATUS", "emcon off: the pod is silent").await;
    assert!(
        !seen.contains("receiver degraded"),
        "the carrier's own sight did not clear: {seen}"
    );
    let seen = ask_whole(&mut ada, "EMCON off").await;
    assert!(seen.contains("EMCON is already off."), "{seen}");

    // The tape.
    ada.send_line("SIM ABORT").await;
    poll_whole(&mut ada, "SIM STATUS", "aborted").await;
    let mut events = Vec::new();
    wait_until(STEP_TIMEOUT, "the emcon line to persist", async || {
        events = ledger_events(&server.store, 1).await;
        count(&events, "emcon") > 0
    })
    .await;
    let switches: Vec<&Value> = events
        .iter()
        .filter(|event| event.get("kind").and_then(Value::as_str) == Some("emcon"))
        .collect();
    assert_eq!(switches.len(), 1, "{switches:?}");
    assert_eq!(
        switches
            .first()
            .and_then(|event| event.get("unit"))
            .and_then(Value::as_u64),
        Some(u64::from(red)),
        "{switches:?}"
    );
    assert_eq!(
        switches
            .first()
            .and_then(|event| event.get("on"))
            .and_then(Value::as_bool),
        Some(false),
        "{switches:?}"
    );
    no_sixteen_digit_word(&strip_ansi(&ada.seen));
    no_sixteen_digit_word(&strip_ansi(&bo.seen));
}

// ---------------------------------------------------------------------------
// 16b cut AE.1 — the scout car and the heavy tank, flown (16c N15)
// ---------------------------------------------------------------------------

/// **Two pilots field 16b cut AE.1's scout car and heavy tank over the
/// production composition** (16c N15; doc 16 §1 and §5's observer rule).
///
/// Ada takes `SIM UNIT scout_mk1` and Bo `SIM UNIT heavy_mk1`; the board names
/// both brackets during `Selection`, `SIM START` refits both pods' tanks, and
/// once they are aboard each pilot's `STATUS` prints its own chassis — the
/// car's pair of wheels, the heavy's pair of tracks — and `WEAPONSTATUS` its own
/// guns: since 16b cut AE.2 the car's two rows, the autocannon and then the
/// coaxial (`machine_gun (mg)  ready  2000 of 2000 round(s)`), and the heavy's
/// three, the cannon, the coaxial and the rack. From the moment either climbed
/// in, neither session saw the other machine's type word or its object name.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ae1_two_pilots_field_the_scout_car_and_the_heavy_tank() {
    let server = boot("sim-units-ae1").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;

    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    let seen = ask_whole(&mut ada, "SIM UNIT").await;
    for word in ["scout_mk1", "heavy_mk1", "biped_mk2"] {
        assert!(
            seen.contains(word),
            "16b cut AE.1's {word} is not fielded: {seen}"
        );
    }
    say(
        &mut ada,
        "SIM UNIT scout_mk1",
        "Slot red 1 will field scout_mk1.",
    )
    .await;
    say(
        &mut bo,
        "SIM UNIT heavy_mk1",
        "Slot blue 1 will field heavy_mk1.",
    )
    .await;
    let seen = ask_whole(&mut ada, "SIM STATUS").await;
    assert!(seen.contains("[scout_mk1]"), "{seen}");
    assert!(seen.contains("[heavy_mk1]"), "{seen}");

    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    say(&mut bo, "blue", "Blue Room").await;
    say(&mut ada, "BOARD", "climb into").await;
    say(&mut bo, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;
    power_up(&mut ada).await;
    power_up(&mut bo).await;

    // Each pilot's own chassis.
    let seen = ask_whole(&mut ada, "STATUS").await;
    for section in ["hull", "turret", "left_wheels", "right_wheels", "engine"] {
        assert!(
            seen.contains(&format!("  {section}  intact")),
            "the car's {section} is not in its pilot's STATUS: {seen}"
        );
    }
    assert!(
        !seen.contains("left_track"),
        "the car has no tracks: {seen}"
    );
    let seen = ask_whole(&mut bo, "STATUS").await;
    for section in ["hull", "turret", "left_track", "right_track", "engine"] {
        assert!(
            seen.contains(&format!("  {section}  intact")),
            "the heavy's {section} is not in its pilot's STATUS: {seen}"
        );
    }

    // Each pilot's own guns. **16b cut AE.2** gave the car its autocannon,
    // first in component order, and the heavy its rack: two rows and three.
    let seen = ask_whole(&mut ada, "WEAPONSTATUS").await;
    assert!(
        seen.contains("autocannon  ready  240 of 240 round(s)")
            && seen.contains("machine_gun (mg)  ready  2000 of 2000 round(s)"),
        "the car's two guns: {seen}"
    );
    assert!(
        !seen
            .lines()
            .any(|line| line.trim_start().starts_with("cannon")),
        "the car carries no cannon: {seen}"
    );
    let seen = ask_whole(&mut bo, "WEAPONSTATUS").await;
    assert!(
        seen.contains("cannon  ready  40 of 40 round(s)")
            && seen.contains("machine_gun (mg)  ready  2000 of 2000 round(s)")
            && seen.contains("missiles  ready  6 of 6 round(s)"),
        "the heavy's three guns: {seen}"
    );

    // The observer rule over every byte either session saw from the pods on.
    for (whole, theirs, kind) in [
        (strip_ansi(&ada.seen), BLUE_TANK, "heavy_mk1"),
        (strip_ansi(&bo.seen), RED_TANK, "scout_mk1"),
    ] {
        let cockpit = whole
            .rfind("climb into")
            .map_or_else(|| whole.clone(), |at| whole[at..].to_owned());
        assert!(
            !cockpit.contains(kind) && !cockpit.contains(theirs),
            "a cockpit surface named the other machine ({kind}, {theirs})"
        );
        no_sixteen_digit_word(&whole);
    }
}

// ---------------------------------------------------------------------------
// 16b cut AE.2 — the autocannon and the rack, fired (16c N15)
// ---------------------------------------------------------------------------

/// The lines of one kind a unit is the `unit` of, from one match's ledger.
fn lines_of<'a>(events: &'a [Value], kind: &str, unit: u32) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|event| {
            event.get("kind").and_then(Value::as_str) == Some(kind)
                && event.get("unit").and_then(Value::as_u64) == Some(u64::from(unit))
        })
        .collect()
}

/// **The scout car's bare `FIRE` is its autocannon, and the arena spends a
/// burst of three; the heavy carries a rack beside its cannon** (16b cut AE.2;
/// 16c N15), over the production composition.
///
/// Ada takes the scout car and Bo the heavy, parked 200 m apart on the road.
/// Bo's `WEAPONSTATUS` is three rows in component order, verbatim. Ada locks
/// Bo, types the bare `FIRE` and reads `Firing autocannon.` — HEAD's rule, the
/// cannon if carried else the first weapon, on a car whose author listed the
/// autocannon first — and a pulse later the arena has accepted it: her
/// `WEAPONSTATUS` reads three of 240 spent, and the tape a `fired` line of three
/// rounds from the autocannon.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ae2_the_scout_car_fires_its_autocannon_and_the_heavy_carries_a_rack() {
    let server = boot("sim-units-ae2").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;
    let listing = ask(&mut god, &format!("@find {RED_TANK}"), "END OF LIST").await;
    let red = dbref_of(&listing, RED_TANK);

    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(
        &mut ada,
        "SIM UNIT scout_mk1",
        "Slot red 1 will field scout_mk1.",
    )
    .await;
    say(
        &mut bo,
        "SIM UNIT heavy_mk1",
        "Slot blue 1 will field heavy_mk1.",
    )
    .await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    say(&mut bo, "blue", "Blue Room").await;
    say(&mut ada, "BOARD", "climb into").await;
    say(&mut bo, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;
    power_up(&mut ada).await;

    // The heavy's three rows, verbatim and in component order.
    let seen = poll_whole(&mut bo, "WEAPONSTATUS", "missiles").await;
    let at = |row: &str| {
        seen.find(row)
            .unwrap_or_else(|| panic!("no `{row}` in the heavy's WEAPONSTATUS: {seen}"))
    };
    let rows = [
        at("cannon  ready  40 of 40 round(s)"),
        at("machine_gun (mg)  ready  2000 of 2000 round(s)"),
        at("missiles  ready  6 of 6 round(s)"),
    ];
    assert!(
        rows.windows(2).all(|pair| pair[0] < pair[1]),
        "the heavy's rows are not in component order: {seen}"
    );

    // The car's bare `FIRE`.
    let contact = wait_for_contact(&mut ada).await;
    let locking = ada.mark();
    say(&mut ada, &format!("LOCK {contact}"), "Locking").await;
    ada.expect_after(locking, "Locked on").await;
    let fired = ada.mark();
    ada.send_line("FIRE").await;
    ada.expect_after(fired, "Firing autocannon.").await;
    let seen = poll_whole(&mut ada, "WEAPONSTATUS", "237 of 240 round(s)").await;
    assert!(
        seen.contains("machine_gun (mg)") && seen.contains("2000 of 2000 round(s)"),
        "the coaxial is untouched: {seen}"
    );
    assert!(
        !strip_ansi(&ada.seen[fired..]).contains("refused"),
        "the burst was refused: {}",
        strip_ansi(&ada.seen[fired..])
    );

    // The tape: one burst of three, from the autocannon.
    ada.send_line("SIM ABORT").await;
    poll_whole(&mut ada, "SIM STATUS", "aborted").await;
    let mut events = Vec::new();
    wait_until(STEP_TIMEOUT, "the burst to persist", async || {
        events = ledger_events(&server.store, 1).await;
        !lines_of(&events, "fired", red).is_empty()
    })
    .await;
    let bursts = lines_of(&events, "fired", red);
    assert_eq!(
        bursts
            .iter()
            .map(|event| (
                event.get("weapon").and_then(Value::as_str),
                event.get("rounds").and_then(Value::as_u64)
            ))
            .collect::<Vec<_>>(),
        vec![(Some("autocannon"), Some(3))],
        "{bursts:?}"
    );
    no_sixteen_digit_word(&strip_ansi(&ada.seen));
    no_sixteen_digit_word(&strip_ansi(&bo.seen));
}

/// **A heavy on the range fires its rack: `Firing missiles.`, and the round
/// lands 2,000 arena ms after the shot** (16b cut AE.2; 16c N15; doc 16 §3.5
/// "Scheduled impact").
///
/// Ada claims red, sets `SIM RANGE 1` and takes the heavy; staff poses the
/// target and her tank on the ford road, 50 cells — 500 m, inside her 600 m
/// sight — apart (the `sim_range.rs` pattern), her tank facing east and the
/// target facing west, toward her. The target is the stock hull, and its
/// **front** is the one band no single missile can put out of action: hull 400
/// and turret 350 of armour both take the 350 whole, where its rear (hull 150
/// over 200 internals, the engine struck through it) would be killed by one
/// round on most hits and end the range by elimination before the test reads
/// the rack cycling and aborts it. Her
/// `WEAPONSTATUS` is three rows, verbatim; she holds the target as `BA`, locks
/// it and types `FIRE missiles`, reads `Firing missiles.`, is told the outcome
/// at the impact, and reads the rack cycling with five of six left. The tape's
/// `impact` for that shot is 2,000 arena ms after its `fired`: 50,000 cm at
/// 25,000 cm/s.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ae2_a_heavy_on_the_range_fires_its_rack_and_the_round_lands_two_seconds_later() {
    let server = boot("sim-units-ae2-range").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let listing = ask(&mut god, "@find Simulator Lobby", "END OF LIST").await;
    let lobby = dbref_of(&listing, "Simulator Lobby");
    say(
        &mut god,
        &format!("@teleport #{}=#{lobby}", ADA.0),
        "Teleported",
    )
    .await;
    let listing = ask(&mut god, &format!("@find {RED_TANK}"), "END OF LIST").await;
    let red = dbref_of(&listing, RED_TANK);

    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(
        &mut ada,
        "SIM RANGE 1",
        "Range set: 1 target stands on the blue side;",
    )
    .await;
    say(
        &mut ada,
        "SIM UNIT heavy_mk1",
        "Slot red 1 will field heavy_mk1.",
    )
    .await;
    say(
        &mut god,
        "@arena/pose BA=130 97 270",
        "BA opens at cell (130, 97) heading 270 for the next match.",
    )
    .await;
    say(
        &mut god,
        "@arena/pose RA=80 97 90",
        "RA opens at cell (80, 97) heading 90 for the next match.",
    )
    .await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    say(&mut ada, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;
    power_up(&mut ada).await;

    // Three rows, verbatim.
    let seen = poll_whole(&mut ada, "WEAPONSTATUS", "missiles").await;
    for row in [
        "cannon  ready  40 of 40 round(s)",
        "machine_gun (mg)  ready  2000 of 2000 round(s)",
        "missiles  ready  6 of 6 round(s)",
    ] {
        assert!(seen.contains(row), "no `{row}`: {seen}");
    }

    // The rack.
    let contact = wait_for_contact(&mut ada).await;
    assert_eq!(contact, "BA", "the target answers to its pod's callsign");
    let locking = ada.mark();
    say(&mut ada, "LOCK BA", "Locking").await;
    ada.expect_after(locking, "Locked on BA").await;
    let fired = ada.mark();
    ada.send_line("FIRE missiles").await;
    ada.expect_after(fired, "Firing missiles.").await;
    let outcome = ada
        .expect_one_of_after(fired, &["Hit on BA", "Miss on BA", "refused"])
        .await;
    assert_ne!(
        outcome,
        2,
        "the missile was refused: {}",
        strip_ansi(&ada.seen[fired..])
    );
    let seen = ask_whole(&mut ada, "WEAPONSTATUS").await;
    assert!(
        seen.contains("missiles  cycling (") && seen.contains("5 of 6 round(s)"),
        "the rack is cycling with five left: {seen}"
    );
    assert!(
        seen.contains("cannon  ready  40 of 40 round(s)"),
        "the cannon is its own magazine: {seen}"
    );

    // The tape: the impact 2,000 arena ms after the shot.
    ada.send_line("SIM ABORT").await;
    poll_whole(&mut ada, "SIM STATUS", "aborted").await;
    let mut events = Vec::new();
    wait_until(
        STEP_TIMEOUT,
        "the missile's impact to persist",
        async || {
            events = ledger_events(&server.store, 1).await;
            !lines_of(&events, "impact", red).is_empty()
        },
    )
    .await;
    let shots = lines_of(&events, "fired", red);
    assert_eq!(shots.len(), 1, "one missile: {shots:?}");
    let shot = shots.first().copied().unwrap_or(&Value::Null);
    assert_eq!(shot.get("weapon").and_then(Value::as_str), Some("missiles"));
    assert_eq!(
        shot.get("range_cm").and_then(Value::as_u64),
        Some(50_000),
        "500 m"
    );
    let seq = shot.get("shot_seq").and_then(Value::as_u64);
    let impact = lines_of(&events, "impact", red)
        .into_iter()
        .find(|event| event.get("shot_seq").and_then(Value::as_u64) == seq)
        .unwrap_or_else(|| panic!("no impact for shot {seq:?}"));
    let (Some(fired_ms), Some(impact_ms)) = (
        shot.get("t_ms").and_then(Value::as_i64),
        impact.get("t_ms").and_then(Value::as_i64),
    ) else {
        panic!("no timestamps: {shot:?} {impact:?}");
    };
    assert_eq!(
        impact_ms - fired_ms,
        2_000,
        "50,000 cm at 25,000 cm/s is 2,000 ms"
    );
    no_sixteen_digit_word(&strip_ansi(&ada.seen));
}

// ---------------------------------------------------------------------------
// 16b cut AE.3 — one drone a side, and a side needs a gun (doc 16 §2 W2)
// ---------------------------------------------------------------------------

/// The cap's refusal at the shipped default, word for word.
const AE3_CAP: &str = "A side fields at most 1 drone; pick another type.";

/// The `NoGun` refusal for red, word for word.
const AE3_NO_GUN: &str = "The red side fields no gun; a side needs a machine that carries one.";

/// The drone's weapon line, in `WEAPONSTATUS` and for a bare `FIRE`.
const AE3_NO_WEAPON: &str = "This unit carries no weapon.";

/// **The lobby's two drone refusals over real SSH** (16b cut AE.3; doc 16 §2
/// W2), with the fill off as the file ships it.
///
/// Ada and Bo both claim red. Ada's `SIM UNIT drone_mk1` is announced as any
/// choice is; Bo's is refused with the cap's sentence, to Bo alone, and his
/// slot keeps what it had. Bo moves to blue, which leaves red a lone drone,
/// and Ada's `SIM START` is refused at once with the `NoGun` sentence — before
/// anything is written, so the centre is still in `Selection`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ae3_the_second_drone_and_a_gunless_side_are_refused_at_the_lobby() {
    let server = boot("sim-units-ae3-lobby").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;

    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut bo, "SIM TEAM red", "takes red slot 2").await;
    // The catalog lists the drone, in its sorted place.
    let seen = ask_whole(&mut ada, "SIM UNIT").await;
    assert!(
        seen.split_whitespace()
            .collect::<Vec<&str>>()
            .join(" ")
            .contains("biped_mk2, drone_mk1, heavy_mk1,"),
        "{seen}"
    );
    say(
        &mut ada,
        "SIM UNIT drone_mk1",
        "Slot red 1 will field drone_mk1.",
    )
    .await;
    let mark = ada.mark();
    let seen = ask(&mut bo, "SIM UNIT drone_mk1", AE3_CAP).await;
    // Ada's own announcement may still be arriving in Bo's slice; his slot's is
    // the one that must not.
    assert!(!seen.contains("Slot red 2 will field"), "{seen}");
    let board = ask_whole(&mut bo, "SIM STATUS").await;
    assert!(
        board.contains("[drone_mk1]"),
        "Ada's choice stands: {board}"
    );
    assert_eq!(
        board.matches("[drone_mk1]").count(),
        1,
        "and Bo's slot fields no second one: {board}"
    );
    // Re-choosing the class a slot already holds is never the cap's business.
    say(
        &mut ada,
        "SIM UNIT drone_mk1",
        "Slot red 1 will field drone_mk1.",
    )
    .await;
    assert!(
        !strip_ansi(&ada.seen[mark..]).contains(AE3_CAP),
        "the refusal reached a pilot who did not ask"
    );

    // A lone drone, the fill off: refused before anything is written.
    say(&mut bo, "SIM LEAVE", "gives up red slot 2").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    let mark = bo.mark();
    let seen = ask(&mut ada, "SIM START", AE3_NO_GUN).await;
    assert!(!seen.contains("is boarding"), "{seen}");
    let board = ask_whole(&mut ada, "SIM STATUS").await;
    assert!(board.contains("selection"), "nothing was written: {board}");
    // Bo's own whole answer drains everything the server sent him since the
    // mark, so the check below reads what he was actually told.
    let bo_board = ask_whole(&mut bo, "SIM STATUS").await;
    assert!(bo_board.contains("selection"), "{bo_board}");
    let bo_seen = strip_ansi(&bo.seen[mark..]);
    assert!(
        !bo_seen.contains(AE3_NO_GUN) && !bo_seen.contains("fields no gun"),
        "a refusal of SIM START is the caller's: {bo_seen}"
    );
}

/// **A pilot flies the drone** (16b cut AE.3; 16c N15), on a server whose fill
/// brings red a gun beside it (`SIM AI 2`).
///
/// Ada takes `SIM UNIT drone_mk1`, Bo the stock tank; both board. Ada's
/// `STATUS` brackets the fitted type and prints the drone's one section;
/// `WEAPONSTATUS` and a bare `FIRE` both read `This unit carries no weapon.`
/// — the bare `FIRE` no longer names a cannon the pilot never mentioned — and
/// Bo's cockpit never names the drone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ae3_a_pilot_flies_the_drone_and_it_carries_no_weapon() {
    let mut cfg = tacsim_config();
    cfg.match_.ai_fill = true;
    let server = boot_with("sim-units-ae3-fly", cfg).await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;

    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(
        &mut ada,
        "SIM UNIT drone_mk1",
        "Slot red 1 will field drone_mk1.",
    )
    .await;
    say(&mut ada, "SIM AI 2", "AI fill set to 2 a side.").await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    say(&mut bo, "blue", "Blue Room").await;
    let mark = bo.mark();
    let boarding = ada.mark();
    say(&mut ada, "BOARD", "climb into").await;
    say(&mut bo, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;
    // **16b cut AH.1**: the first frame, before the `STATUS` below. `Good
    // hunting` is said at the check-out and the arena's first frame can land a
    // moment after it, when `STATUS` answers `The panel has told you nothing
    // yet.` and carries no bracket at all. Ada is a fresh account, so her first
    // frame carries the panel hint in the same drain that writes her mirror —
    // the fence.
    ada.expect_after(boarding, "Hint: the panel").await;

    let seen = ask_whole(&mut ada, "STATUS").await;
    assert!(seen.contains("[drone_mk1]"), "the bracket: {seen}");
    assert!(seen.contains("  body  intact"), "its one section: {seen}");
    assert!(!seen.contains("  turret  "), "it is not a tank: {seen}");
    let seen = ask_whole(&mut ada, "WEAPONSTATUS").await;
    assert!(seen.contains(AE3_NO_WEAPON), "{seen}");
    assert!(!seen.contains("round(s)"), "{seen}");
    let seen = ask_whole(&mut ada, "FIRE").await;
    assert!(seen.contains(AE3_NO_WEAPON), "{seen}");
    assert!(!seen.contains("called `cannon`"), "{seen}");
    assert!(!seen.contains("Firing"), "{seen}");
    // A named weapon still names what was typed.
    let seen = ask_whole(&mut ada, "FIRE cannon").await;
    assert!(
        seen.contains("That unit carries no weapon called `cannon`."),
        "{seen}"
    );

    // Bo's cockpit, from the moment he climbed in, never named the drone:
    // his own `CONTACTS` (a whole answer, so everything sent to him before it
    // has been read) and every frame and notice that reached him meanwhile.
    let contacts = ask_whole(&mut bo, "CONTACTS").await;
    assert!(!contacts.contains("drone"), "{contacts}");
    let bo_seen = strip_ansi(&bo.seen[mark..]);
    assert!(
        !bo_seen.contains("drone"),
        "the other side's machine was named: {bo_seen}"
    );
    no_sixteen_digit_word(&strip_ansi(&ada.seen));
    no_sixteen_digit_word(&strip_ansi(&bo.seen));
}
