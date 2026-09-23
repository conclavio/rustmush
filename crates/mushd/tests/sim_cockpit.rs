//! **The cockpit lens** (16b cut K.2; 16b §8 cut K, K2–K4; doc 16 §3.7, §3.8,
//! §5).
//!
//! `sim_lobby.rs` proved the half a cockpit is not needed for. This is the other
//! half, and every claim in it is a claim about a real player on a real account
//! in a real pod, so it drives the whole stack over **real SSH** the way
//! `arena_custody.rs` and `sim_lobby.rs` do.
//!
//! 1. two pilots walk `Selection → Boarding → Running → Aborted → Idle` twice in
//!    a row, boarding through `BOARD` and leaving through `EXIT`;
//! 2. a pilot who types `FIRE` and `EXIT` in one burst fires nothing: no `fired`
//!    event reaches the match's ledger, and nothing refuses after the `EXIT`;
//! 3. a pilot whose line drops mid-match keeps their tank's heading and speed,
//!    reclaims the seat on reconnect inside the grace (a fresh `HEADING` is
//!    taken), and — past the grace — finds the seat gone (16b cut N.1 is what the
//!    *tank* does next: `sim_reflex.rs`);
//! 4. `HEADING` before `STARTUP` is refused with the cold tank's own word;
//! 5. `VIEW` prints the arena window as a map with the platform on it;
//! 6. every cockpit verb outside a cockpit, and `BOARD` outside a team room,
//!    refuses with one line.
//!
//! # How a line is cut
//!
//! By dropping the client: the transport tears down without a `QUIT`, the
//! gateway reports `DisconnectKind::Lost`, and the engine announces
//! "has gone link-dead." and fires cut K.2's new `HookPoint::Linkdead`. Staff
//! stand in the team room precisely so that announce is something a test can
//! wait for.

mod support;

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

use support::{Term, play_client, scratch_dir, strip_ansi};

const GM_PASSWORD: &str = "gm-secret-1";
const PILOT_PASSWORD: &str = "pilot-secret-1";

/// The world's transient authoritative refusal: a commit is in flight.
const COMMIT_PENDING: &str = "please retry in a moment";
/// `BOARD`'s one transient: the control-lease mirror is refreshed by a 1 Hz
/// tick, and a server that has not ticked since boot fails closed rather than
/// reading ignorance as a vacancy.
const CONTROL_UNKNOWN: &str = "control state unknown";
/// How many times a line is re-sent on a transient before the test gives up.
const RETRY_BUDGET: usize = 40;

/// The two pilot characters, created before the world task spawns so their ids
/// are stable.
const ADA: ObjId = ObjId(2);
const BO: ObjId = ObjId(3);

/// The one line the cockpit refuses a verb with outside a cockpit.
const OUTSIDE_COCKPIT: &str = "You are not at the station of a deployed unit.";
/// The one line `BOARD` refuses with outside a team room.
const OUTSIDE_TEAM_ROOM: &str = "Boarding is through one of the simulator's team rooms.";
/// The one line `EXIT` refuses with outside a pod.
const NOT_IN_A_POD: &str = "You are not in a pod.";
/// The one line a motion order refuses with while the tank is cold.
const POWER_NOT_ONLINE: &str = "Power not online";
/// The same refusal once `STARTUP` has been accepted and the ten seconds are
/// running (16b cut F.1): a pilot who has already asked is told to wait, not told
/// to ask again.
const POWER_STARTING: &str = "Power is still coming up";

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

/// A fast arena: a short startup so a pilot is flying within a test's patience,
/// and a boarding window long enough that no test races it.
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
) -> Vec<Box<dyn GameModule>> {
    let mut mods = modules::server_modules();
    mods.push(modules::worldgen_module(Arc::clone(llm)));
    mods.push(modules::npc_module(modules::npc_agent_adapter()));
    mods.extend(modules::ruleset_modules(onboarding, &Default::default()));
    mods.push(modules::tacsim_module(tacsim_config()));
    mods.push(modules::content_module_with(true, true).expect("the content module composes"));
    mods
}

async fn boot(tag: &str, grace: Duration) -> Server {
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
        linkdead_grace: grace,
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

/// Walk every pilot into the simulator lobby, and put staff wherever `staff_at`
/// names (by `@find` name), so a test can wait on a room announce.
async fn open_the_centre(god: &mut Term, staff_at: &str) {
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
    let listing = ask(god, &format!("@find {staff_at}"), "END OF LIST").await;
    let room = dbref_of(&listing, staff_at);
    // Teleporting YOURSELF renders the room you land in rather than saying
    // "Teleported.", so the room's own name is the needle.
    say(god, &format!("@teleport #{}=#{room}", GOD.0), staff_at).await;
}

/// Claim, start, walk and board: the state every cockpit test starts from.
async fn take_off(ada: &mut Term, bo: &mut Term) {
    say(ada, "SIM TEAM red", "takes red slot 1").await;
    say(bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(ada, "SIM START", "is boarding").await;
    say(ada, "red", "Red Room").await;
    say(bo, "blue", "Blue Room").await;
    say(ada, "BOARD", "climb into").await;
    say(bo, "BOARD", "climb into").await;
    // The second seat fires the trigger, and the centre says so in every room.
    ada.expect("Good hunting").await;
}

/// The speed `STATUS` last reported, in kph (`0` when it reported none).
fn kph_of(seen: &str) -> i32 {
    seen.lines()
        .rev()
        .find_map(|line| {
            let (head, _) = line.split_once(" kph")?;
            head.rsplit(|c: char| !c.is_ascii_digit() && c != '-')
                .find(|word| !word.is_empty())?
                .parse::<i32>()
                .ok()
        })
        .unwrap_or(0)
}

/// Every ledger chunk one match wrote, as one string.
async fn ledger_text(store: &Arc<dyn ObjectStore>, match_id: u64) -> String {
    let log = format!("match.{match_id}");
    let mut text = String::new();
    for chunk in 0..16 {
        let key = mush_core::chunk_key(&log, chunk);
        match store
            .module_json_get("tacsim", key.as_str())
            .await
            .expect("the store read")
        {
            Some(value) => text.push_str(&value.to_string()),
            None => break,
        }
    }
    text
}

// ---------------------------------------------------------------------------
// (1) The whole walk, twice, through BOARD and EXIT
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_pilots_board_fly_and_climb_out_twice_in_a_row() {
    let server = boot("sim-cockpit-walk", Duration::from_secs(120)).await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god, "Simulator Lobby").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;

    for run in 1..=2 {
        take_off(&mut ada, &mut bo).await;

        // A pilot in a pod is flying: the cockpit answers.
        let seen = ask(&mut ada, "STATUS", "power").await;
        assert!(seen.contains("Red Tank 1"), "run {run}: {seen}");

        // And climbs out again: the seat, the lease and the focus go back, and
        // the body stands in the team room.
        say(&mut ada, "EXIT", "climb out").await;
        say(&mut bo, "EXIT", "climb out").await;
        let seen = ask(&mut ada, "STATUS", OUTSIDE_COCKPIT).await;
        assert!(seen.contains(OUTSIDE_COCKPIT), "run {run}: {seen}");

        // `Running -> Aborted -> Idle`, by a pilot who still holds the slot. The
        // check-in is the arena's own answer, so the reset waits for it — staff
        // watch from the lobby, where the centre says so.
        let mark = god.mark();
        say(&mut ada, "SIM ABORT", "aborts match").await;
        god.expect_after(mark, "is over").await;
        say(&mut god, "@arena/reset", "is reset").await;
        // Back in the lobby for the next run.
        say(&mut ada, "lobby", "Simulator Lobby").await;
        say(&mut bo, "lobby", "Simulator Lobby").await;
    }
}

// ---------------------------------------------------------------------------
// (2) `FIRE` then `EXIT` in one burst
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pilot_who_types_fire_then_exit_in_one_burst_fires_nothing() {
    let server = boot("sim-cockpit-burst", Duration::from_secs(120)).await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god, "Simulator Lobby").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    take_off(&mut ada, &mut bo).await;
    say(&mut ada, "STARTUP", "Startup sequence").await;

    // The burst: two lines, no wait between them.
    let mark = ada.mark();
    ada.send_line("FIRE").await;
    ada.send_line("EXIT").await;
    ada.expect_after(mark, "climb out").await;
    let seen = strip_ansi(&ada.seen[mark..]);
    let after = seen
        .split_once("climb out")
        .map(|(_, tail)| tail.to_owned())
        .unwrap_or_default();
    assert!(
        !after.contains("Refused") && !after.contains("No lock"),
        "nothing refuses after the EXIT — the order was purged, not answered: {after:?}"
    );

    // And the arena's own record says no shot was ever resolved.
    tokio::time::sleep(Duration::from_millis(600)).await;
    say(&mut ada, "SIM ABORT", "aborts match").await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let ledger = ledger_text(&server.store, 1).await;
    assert!(
        !ledger.contains("\\\"fired\\\"") && !ledger.contains("\"fired\""),
        "the match ledger records no shot: {ledger}"
    );
}

// ---------------------------------------------------------------------------
// (3) A dropped line
// ---------------------------------------------------------------------------

/// A pilot in a pod is in a room of their own — the pod — so the engine's
/// "has gone link-dead." announce has nobody to reach and this test cannot wait
/// on it. What it waits on instead is the **reclaim's own line**, which the
/// arena's seat hook sends only on a revival: its arrival is the proof that the
/// drop was a link-dead gap and the reconnect was a reclaim, and not two
/// ordinary sessions in a row.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dropped_line_keeps_the_tank_rolling_and_the_seat_is_reclaimed_on_reconnect() {
    let server = boot("sim-cockpit-linkdead", Duration::from_secs(120)).await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god, "Simulator Lobby").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (ada_handle, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    take_off(&mut ada, &mut bo).await;
    say(&mut ada, "STARTUP", "Startup sequence").await;
    // Power comes online behind the startup; the orders are taken once it is.
    for _ in 0..RETRY_BUDGET {
        let mark = ada.mark();
        ada.send_line("HEADING 90").await;
        if ada
            .expect_one_of_after(mark, &["Steering to 90", POWER_NOT_ONLINE, POWER_STARTING])
            .await
            == 0
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    say(&mut ada, "SPEED 20", "Making 20 kph").await;
    // The tank accelerates at a metre a second squared, so it takes a moment to
    // be **making way** rather than merely ordered to. The drop waits for that,
    // because an order still in flight when the line goes is one the epoch raise
    // purges — doc 16 §3.8 keeps the orders a tank is *carrying out*, and that
    // is the claim under test.
    let mut rolling = 0;
    for _ in 0..RETRY_BUDGET {
        rolling = kph_of(&ask(&mut ada, "STATUS", "kph").await);
        if rolling > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(rolling > 0, "the tank is making way before the line drops");

    // The line drops: no QUIT, just a socket that stops being there.
    drop(ada);
    drop(ada_handle);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Inside the grace, the seat is reclaimed — the arena says so — and the tank
    // is still rolling on the heading it was given.
    let (_a2, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    ada.expect("back at your station").await;
    let seen = ask(&mut ada, "STATUS", "heading").await;
    assert!(
        seen.contains("heading 90"),
        "the tank kept its continuous orders: {seen}"
    );
    assert!(
        kph_of(&seen) > 0,
        "and it never stopped rolling ({rolling} kph when the line went): {seen}"
    );
    // And the reclaimed seat takes a fresh order under its new epoch.
    say(&mut ada, "HEADING 180", "Steering to 180").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn past_the_grace_the_seat_is_released_and_the_pilot_cannot_reclaim_it() {
    let server = boot("sim-cockpit-grace", Duration::from_secs(2)).await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god, "Simulator Lobby").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (ada_handle, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    take_off(&mut ada, &mut bo).await;

    drop(ada);
    drop(ada_handle);
    // The grace is two seconds and the reaper sleeps exactly that; four is the
    // whole of the margin.
    tokio::time::sleep(Duration::from_secs(4)).await;

    // The seat went with the session: the pilot comes back to a cockpit that is
    // not theirs any more — no reclaim line, and the verbs mean nothing — and
    // the match runs on without her.
    let (_a2, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let seen = ask(&mut ada, "STATUS", OUTSIDE_COCKPIT).await;
    assert!(seen.contains(OUTSIDE_COCKPIT), "{seen}");
    assert!(
        !ada.seen.contains("back at your station"),
        "past the grace there is nothing to reclaim: {}",
        strip_ansi(&ada.seen)
    );
    let seen = ask(&mut god, "@arena/status", "match 1").await;
    assert!(
        seen.contains("running"),
        "the match runs on without her: {seen}"
    );
    // **16b cut N.1 amended what happens to the tank** (doc 16 §3.8 and its §11
    // amendment): K4's "and the tank idles" was standing in for doc 07 §1.1's
    // fallback controller until it existed, and the released seat now hands the
    // tank to the reflex controller in `Withdraw`. What this test is about — the
    // seat is gone and there is nothing to reclaim — is unchanged; what the tank
    // does with itself afterwards is `crates/mushd/tests/sim_reflex.rs`'s.
}

// ---------------------------------------------------------------------------
// (4), (5), (6) The refusals, and the window
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_cockpit_verbs_are_contextual_and_the_window_is_a_map() {
    let server = boot("sim-cockpit-verbs", Duration::from_secs(120)).await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god, "Simulator Lobby").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;

    // (6) In the lobby, flying nothing: one line each.
    for verb in [
        "STARTUP",
        "HEADING 90",
        "SPEED 20",
        "VIEW",
        "CONTACTS",
        "LOCK 1",
        "FIRE",
        "WEAPONSTATUS",
        "STATUS",
    ] {
        let seen = ask(&mut ada, verb, OUTSIDE_COCKPIT).await;
        assert!(seen.contains(OUTSIDE_COCKPIT), "{verb}: {seen}");
    }
    let seen = ask(&mut ada, "BOARD", OUTSIDE_TEAM_ROOM).await;
    assert!(seen.contains(OUTSIDE_TEAM_ROOM), "{seen}");
    let seen = ask(&mut ada, "EXIT", NOT_IN_A_POD).await;
    assert!(seen.contains(NOT_IN_A_POD), "{seen}");

    take_off(&mut ada, &mut bo).await;

    // (4) A cold tank refuses to steer, in the arena's own words.
    let seen = ask(&mut ada, "HEADING 90", POWER_NOT_ONLINE).await;
    assert!(seen.contains(POWER_NOT_ONLINE), "{seen}");

    // (5) The window, as a map: the line renderer draws the grid frame the same
    // verb hands the web panel.
    let seen = ask(&mut ada, "VIEW", "Arena window").await;
    assert!(seen.contains("legend:"), "the map has a legend: {seen}");
    assert!(seen.contains("@ you"), "and the platform's glyph: {seen}");
    assert!(seen.contains("you at ("), "and says where it is: {seen}");
    assert!(
        seen.contains('@'),
        "the platform is drawn on the map: {seen}"
    );

    // And `WEAPONSTATUS` reads the tank's own components.
    let seen = ask(&mut ada, "WEAPONSTATUS", "cannon").await;
    assert!(seen.contains("round(s)"), "{seen}");
}

// ---------------------------------------------------------------------------
// (7) The gates stand aside outside the centre (16b cut F.4)
// ---------------------------------------------------------------------------

/// **A command gate runs before exits, so a gate that refuses everywhere eats
/// content** (16b cut F.4).
///
/// `mush_core::cmd::dispatch` consults gates at step 2 and matches exits at step
/// 5. `CockpitGate` refused `BOARD` anywhere but a team room, so the OOC tour
/// bus's own exit — authored `Board the Bus <BUS>;bus;board` since the lot was
/// written — answered "Boarding is through one of the simulator's team rooms."
/// at the Bus Stop and did not move the player. The `SIM` gate did the same to
/// the `sim` alias on the Welcome Hall's exit, and inside the team rooms the
/// collision ran the other way: the gate stood aside, the back exit's own `sim`
/// alias matched first, and a pilot who typed `SIM` to read the board walked out
/// of the room `BOARD` needs them in.
///
/// Three lines, three rooms, one rule: outside the centre the gates decide
/// nothing, and inside it they are what they were.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn board_takes_the_bus_outside_the_centre_and_sim_reads_the_board_inside_it() {
    let server = boot("sim-cockpit-gates", Duration::from_secs(120)).await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;

    // (a) The tour bus. Ada stands at the Bus Stop and types the word the room's
    // own exit list advertises.
    let listing = ask(&mut god, "@find Bus Stop", "END OF LIST").await;
    let stop = dbref_of(&listing, "Bus Stop");
    say(
        &mut god,
        &format!("@teleport #{}=#{stop}", ADA.0),
        "Teleported",
    )
    .await;
    say(&mut ada, "board", "Studio Tour Bus").await;
    // And back off it, which is the alias the bus's own exit carries.
    say(&mut ada, "off", "Bus Stop").await;

    // The stand-aside is one early-out over all eleven verbs, not a `BOARD`
    // special case, so the other ten are typed here too: out at the Bus Stop the
    // gate decides nothing and the HANDLER says what the gate used to, word for
    // word. `EXIT` is the one that is not obviously equivalent — the gate
    // refused `NOT_IN_A_POD` outright, while `exit()` runs the abandonment
    // guardrail first — so it is asserted, not assumed.
    for verb in [
        "STARTUP",
        "HEADING 90",
        "SPEED 20",
        "VIEW",
        "CONTACTS",
        "LOCK 1",
        "FIRE",
        "WEAPONSTATUS",
        "STATUS",
    ] {
        let seen = ask(&mut ada, verb, OUTSIDE_COCKPIT).await;
        assert!(
            seen.contains(OUTSIDE_COCKPIT),
            "{verb} at the Bus Stop: {seen}"
        );
    }
    let seen = ask(&mut ada, "EXIT", NOT_IN_A_POD).await;
    assert!(seen.contains(NOT_IN_A_POD), "EXIT at the Bus Stop: {seen}");

    // (b) Inside the centre the cockpit gate is unchanged: `BOARD` in the lobby
    // is still refused with the sentence that says where boarding happens.
    open_the_centre(&mut god, "Simulator Lobby").await;
    let seen = ask(&mut ada, "BOARD", OUTSIDE_TEAM_ROOM).await;
    assert!(seen.contains(OUTSIDE_TEAM_ROOM), "{seen}");

    // And the one thing `help simulator` tells a newcomer to type in this room
    // actually resolves. (`look selector` does not: the item carries no alias
    // and `matchit` takes a whole `;`-segment or a segment prefix, which is why
    // the topic names the console.)
    let seen = ask(&mut ada, "look Simulator Selector", "console").await;
    assert!(seen.contains("SIM TEAM red"), "{seen}");

    // (c) A bare `SIM` in a team room reads the board instead of walking the
    // pilot back to the lobby.
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    let seen = ask(&mut ada, "SIM", "Simulator centre:").await;
    assert!(seen.contains("boarding"), "{seen}");
    assert!(
        !seen.contains("Simulator Lobby"),
        "bare SIM walked the pilot out of the team room: {seen}"
    );

    // Which the next line proves outright: `BOARD` only works from in here.
    say(&mut ada, "BOARD", "climb into").await;

    // And the walking words the pack kept still walk.
    say(&mut bo, "blue", "Blue Room").await;
    say(&mut bo, "lobby", "Simulator Lobby").await;
    say(&mut bo, "blue", "Blue Room").await;
    say(&mut bo, "BOARD", "climb into").await;
}

// ---------------------------------------------------------------------------
// 16b cut B.4 — a pilot sees where they land (`B-38`)
// ---------------------------------------------------------------------------

/// The raw byte offset just past the first `needle` after `mark`, so a later
/// `expect_after` can ask what arrived AFTER that sentence.
fn past(term: &Term, mark: usize, needle: &str) -> usize {
    term.seen[mark..]
        .find(needle)
        .map(|at| mark + at + needle.len())
        .unwrap_or_else(|| panic!("{needle:?} after the mark in: {:?}", &term.seen[mark..]))
}

/// **`BOARD` shows the pod and `EXIT` the team room, each after the sentence
/// that explains the move** (16b cut B.4, `B-38`).
///
/// Before this cut both moved the pilot with no room view at all — a line
/// client that climbed out was standing in a room it had not been shown. The
/// room is shown by the callers, AFTER their own sentence, never inside the
/// move: `You climb into …` and then the pod; `You climb out of …` and then the
/// team room.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b4_board_and_exit_show_the_room_after_the_sentence() {
    let server = boot("sim-cockpit-b4-rooms", Duration::from_secs(120)).await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god, "Simulator Lobby").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;

    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    say(&mut bo, "blue", "Blue Room").await;

    // BOARD: the sentence, then the pod's own name again as the room view's
    // head — the interior `look` would show from inside it.
    let mark = ada.mark();
    say(&mut ada, "BOARD", "climb into").await;
    ada.expect_after(mark, "EXIT to get out").await;
    let said = strip_ansi(&ada.seen[mark..]);
    let pod = said
        .split_once("You climb into ")
        .and_then(|(_, rest)| rest.split_once(". The panel wakes"))
        .map(|(pod, _)| pod.to_owned())
        .unwrap_or_else(|| panic!("the boarding sentence names the pod: {said}"));
    let after_sentence = past(&ada, mark, "for the rest of it.");
    ada.expect_after(after_sentence, &pod).await;

    say(&mut bo, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;

    // EXIT: the sentence, then the team room's view.
    let mark = ada.mark();
    say(&mut ada, "EXIT", "climb out").await;
    ada.expect_after(mark, "cannot take it back.").await;
    let after_sentence = past(&ada, mark, "cannot take it back.");
    ada.expect_after(after_sentence, "Red Room").await;
    // The team room is shown whole, its exits and all.
    ada.expect_after(after_sentence, "Exits").await;
}
