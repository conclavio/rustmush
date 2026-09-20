//! **The rematch lens, over real SSH** (16b cut F.1; doc 16 §3.1 "Custody",
//! §3.3 "The stock tanks are persistent objects", §3.8; §1's *a second match can
//! start*).
//!
//! `arena_story.rs` proves doc 16 §1's A1 up to a second `SIM TEAM` claim. This
//! is the half that was never proved and was wrong: what a **second match** is
//! fought on, and whether two players can start one with nobody at a staff
//! terminal.
//!
//! Two tests, both driving the whole production stack the way `sim_weapons.rs`
//! and `arena_story.rs` do — real accounts, real SSH, the shipped content:
//!
//! 1. **The rematch, with no staff between the matches.** Staff musters the two
//!    slot-1 tanks close together *before* match one (and does nothing after it),
//!    the two players fight to a kill, and then the two players **alone** claim,
//!    start and board again. A staff read taken after the second claim finds both
//!    tanks at their spec — the wrecked hull intact, the magazine full — and the
//!    cockpits of match two open at the arena's own **authored** muster, 2.4 km
//!    apart, which is where the pack puts them and not where match one ended.
//! 2. **A restart mid-match, through the lobby.** Staff spends part of one tank's
//!    magazine and parks it off its authored cell — so the boot pass has
//!    something to undo and its "at spec, at muster" assertions are claims rather
//!    than facts about a world nobody touched — the two players board through
//!    `BOARD`, and the server is stopped while the match runs. After the reboot no
//!    body is inside a pod, no cockpit billet is held, the roster tanks are at
//!    their spec and at their authored cells, and a second `SIM TEAM` + `SIM
//!    START` + `BOARD` succeeds and opens on a full magazine at the authored
//!    muster. `arena_custody.rs`'s restart test drives the **staff** path
//!    (`@arena/start`, no pods, no seats) and therefore proves none of that.
//!
//! Both tests read the store directly for the durable `assignment:<unit>` billet,
//! and every such read is **waited for** ([`wait_for_cockpit`]): no line sent to
//! the server orders that write, and after a reboot the pod half of it is owed by
//! the module's own first tick.

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
/// `BOARD`'s one transient: the control-lease mirror is a 1 Hz projection.
const CONTROL_UNKNOWN: &str = "control state unknown";
/// How many times a line is re-sent on a transient before the test gives up.
const RETRY_BUDGET: usize = 40;
/// How many shots the fight is allowed before the test calls it (the reasoning is
/// `arena_story.rs`'s: about a dozen rounds end it, this is the tail).
const MAX_SHOTS: usize = 30;

/// The two pilots, created before the world task spawns so their ids are stable.
const ADA: ObjId = ObjId(2);
const BO: ObjId = ObjId(3);

/// Slot 1 of each side, as the shipped arena pack names them.
const RED_TANK: &str = "Red Tank 1";
const BLUE_TANK: &str = "Blue Tank 1";

/// Where the pack musters them: column 6 and column 249, both on row 97 — the
/// east-west road, since 16b cut F.4 (`content/arenas/proving_ground.json`).
/// In whole metres of easting, as `STATUS` prints them; the columns did not move
/// with the rows.
const RED_MUSTER_M: i64 = 65;
const BLUE_MUSTER_M: i64 = 2_495;

/// Where staff puts the blue tank for match one: Red Tank 4's own authored cell,
/// four cells south of Red Tank 1's, in centimetres of the window's frame.
const BLUE_CLOSE_X_CM: i64 = 6_500;
const BLUE_CLOSE_Y_CM: i64 = 93_500;

/// The cannon's authored load (`ammunition/he_105.json`).
const LOAD: i64 = 40;

/// The line every witness in the centre reads when a match is settled.
const OVER: &str = "is over";
/// The one line the eleven cockpit selectors refuse with once the hatches open.
const OUTSIDE_COCKPIT: &str = "You are not at the station of a deployed unit.";
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

/// A fast arena: a short startup so a pilot is flying within a test's patience,
/// and a boarding window long enough that nothing races it.
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

/// Boot the stack over SSH. `store` reuses an earlier server's — the reboot half
/// of the second lens — and a reused store already carries the accounts and the
/// characters, so they are made only on a first boot.
async fn boot(tag: &str, store: Option<Arc<dyn ObjectStore>>) -> Server {
    let dir = scratch_dir(tag);
    let fresh = store.is_none();
    let store: Arc<dyn ObjectStore> = match store {
        Some(store) => store,
        None => Arc::new(InMemoryStore::new()),
    };
    if fresh {
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
    }

    let mut world = load_or_seed_world(&*store).await.expect("boot world");
    if fresh {
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
    for _ in 0..RETRY_BUDGET {
        let seen = ask_whole(term, line).await;
        if seen.contains(needle) {
            return seen;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("`{line}` never said `{needle}`")
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
/// (`cell section/hull/internals: 200/200` answers `200`).
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

/// One tank's easting in whole metres, off its pilot's own `STATUS` line
/// (`  at 1085 m east, 975 m north; …`).
async fn easting_m(term: &mut Term) -> i64 {
    let seen = ask_whole(term, "STATUS").await;
    seen.lines()
        .find_map(|line| {
            let rest = line.split_once(" at ")?.1;
            let east = rest.split_once(" m east")?.0;
            east.trim().parse::<i64>().ok()
        })
        .unwrap_or_else(|| panic!("no easting in:\n{seen}"))
}

/// **The short designator in a `CONTACTS` listing** (16b cut Q.1): the first
/// word of a contact row, which is `X<n>` while the return is unresolved and
/// the team's letter with the target's centre slot once it has been told apart.
///
/// It replaces the sixteen-digit scraper this helper used to be. The digits are
/// not printed anywhere a pilot can see any more, and a designator is what
/// `LOCK`, `RANGE` and `BEARING` take.
fn designator_of(listing: &str) -> Option<String> {
    listing
        .lines()
        .find(|line| line.contains("bearing ") && line.contains("(rel "))
        .and_then(|line| line.split_whitespace().next())
        .map(str::to_owned)
}

/// Walk both pilots into the simulator lobby and put staff there with them.
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
}

/// The two slot-1 tanks' dbrefs.
async fn tank_refs(god: &mut Term) -> (u32, u32) {
    let listing = ask(god, &format!("@find {RED_TANK}"), "END OF LIST").await;
    let red = dbref_of(&listing, RED_TANK);
    let listing = ask(god, &format!("@find {BLUE_TANK}"), "END OF LIST").await;
    let blue = dbref_of(&listing, BLUE_TANK);
    (red, blue)
}

/// Put the blue tank four cells south of the red one, the way `sim_weapons.rs`
/// does.
///
/// Test 1 uses it so a fight happens in a test's patience, and **before match one
/// only**: the point of that test is that nothing is typed at the staff terminal
/// between the matches. Test 2 uses it for the other reason — a tank off its
/// authored cell is something the boot reset has to put back.
async fn muster_them_together(god: &mut Term, blue: u32) {
    let listing = ask(god, "@find Proving Ground", "END OF LIST").await;
    let ground = dbref_of(&listing, "Proving Ground");
    say(
        god,
        &format!(
            "@spatial/place/domain #{blue}=#{ground},{{\"x_cm\":{BLUE_CLOSE_X_CM},\"y_cm\":{BLUE_CLOSE_Y_CM},\"heading_mdeg\":270000}}"
        ),
        "Placed",
    )
    .await;
}

/// Claim, start, walk and board — the whole lobby walk, by the players alone.
async fn take_off(ada: &mut Term, bo: &mut Term) {
    say(ada, "SIM TEAM red", "takes red slot 1").await;
    say(bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(ada, "SIM START", "is boarding").await;
    say(ada, "red", "Red Room").await;
    say(bo, "blue", "Blue Room").await;
    say(ada, "BOARD", "climb into").await;
    say(bo, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;
}

/// Walk one pilot from a team room back to the lobby.
///
/// `lobby`, not `sim`: 16b cut F.4 took the `sim` alias off the two team rooms'
/// return exits, because a bare `SIM` in a team room walked the pilot out of the
/// room `BOARD` needs them in instead of printing the status board. The word
/// belongs to the verb, and this helper was one of the places that depended on
/// it not doing.
async fn back_to_the_lobby(term: &mut Term) {
    say(term, "lobby", "Simulator Lobby").await;
}

/// Bring power up, waiting for the arena to say it is online.
async fn power_up(term: &mut Term) {
    say(term, "STARTUP", "Startup sequence").await;
    poll_whole(term, "STATUS", "power online").await;
}

/// Wait until `CONTACTS` has something in it, and answer its opaque id.
async fn wait_for_contact(term: &mut Term) -> String {
    for _ in 0..RETRY_BUDGET {
        let mark = term.mark();
        term.send_line("CONTACTS").await;
        if term
            .expect_one_of_after(mark, &["bearing", "No contacts."])
            .await
            == 0
            && let Some(id) = designator_of(&strip_ansi(&term.seen[mark..]))
        {
            return id;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("nothing was ever detected");
}

/// Shoot the lock until the match ends. Answers how many rounds went out.
///
/// The lock is waited for first: the cockpit's `FIRE` pre-check reads the **last
/// frame** (16b cut K.2's mirror), so a `FIRE` sent in the same breath as the
/// `LOCK` is refused "No lock" against a frame one pulse behind.
async fn fight_to_the_end(ada: &mut Term) -> usize {
    poll_whole(ada, "WEAPONSTATUS", "Locked on").await;
    let mark = ada.mark();
    let mut shots = 0_usize;
    for _ in 0..MAX_SHOTS {
        let mut fired = false;
        for _ in 0..RETRY_BUDGET {
            let at = ada.mark();
            ada.send_line("FIRE").await;
            match ada
                .expect_one_of_after(
                    at,
                    &[
                        "Firing cannon",
                        "Still cycling",
                        COMMIT_PENDING,
                        CONTROL_UNKNOWN,
                        OUTSIDE_COCKPIT,
                        OVER,
                    ],
                )
                .await
            {
                0 => {
                    fired = true;
                    break;
                }
                1..=3 => tokio::time::sleep(Duration::from_millis(500)).await,
                _ => break,
            }
        }
        if !fired {
            break;
        }
        shots += 1;
        if strip_ansi(&ada.seen[mark..]).contains(OVER) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    shots
}

// ---------------------------------------------------------------------------
// (1) The rematch
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_players_start_a_second_match_with_no_staff_and_find_both_tanks_at_spec() {
    let server = boot("sim-rematch", None).await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;
    let (red, blue) = tank_refs(&mut god).await;
    // The ONE staff action of this test, and it happens before match one.
    muster_them_together(&mut god, blue).await;

    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;

    // ---- match one, to a kill -------------------------------------------
    take_off(&mut ada, &mut bo).await;
    power_up(&mut ada).await;
    power_up(&mut bo).await;
    let contact = wait_for_contact(&mut ada).await;
    say(&mut ada, &format!("LOCK {contact}"), "Locking").await;
    let shots = fight_to_the_end(&mut ada).await;
    assert!(shots > 0, "the red tank never got a shot away");
    ada.expect(OVER).await;
    ada.expect("Its units are checked in.").await;

    // The evidence a check-in is supposed to leave: the loser's record IS the
    // wreck, and the winner's magazine IS down. Doc 16 §1's acceptance reads
    // exactly this, which is why 16b cut F.1 does **not** reset at check-in.
    let wreck = ask_whole(&mut god, &format!("@unit/show #{blue}")).await;
    assert!(
        wreck.contains("section hull: wreck"),
        "the loser's tank came back a wreck: {wreck}"
    );
    let survivor = ask_whole(&mut god, &format!("@unit/show #{red}")).await;
    let left = cell_current(&survivor, "component/cannon/ammo");
    assert!(
        left < LOAD,
        "the winner's magazine is down after {shots} shot(s): {survivor}"
    );

    // ---- match two, by the two players alone -----------------------------
    //
    // Nothing is typed at the staff terminal from here on. The claim itself is
    // what clears the board (16b cut F.1), which is the whole point: on a fresh
    // store there is no front door to a staff character at all.
    back_to_the_lobby(&mut ada).await;
    back_to_the_lobby(&mut bo).await;
    let seen = poll_whole(&mut ada, "SIM STATUS", "ended").await;
    assert!(
        seen.contains("— red"),
        "the board carries the outcome: {seen}"
    );
    assert!(
        seen.contains("`SIM TEAM red` or `SIM TEAM blue` opens the next match."),
        "and says how to get past it: {seen}"
    );
    let seen = ask(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    assert!(
        !seen.contains("that is not something you can do to it now"),
        "the terminal refusal is still what a player gets: {seen}"
    );
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;

    // **The staff read, after the second claim and before the check-out**: both
    // tanks are at their spec. Nothing between the matches asked for this.
    for (unit, who) in [(red, RED_TANK), (blue, BLUE_TANK)] {
        let seen = ask_whole(&mut god, &format!("@unit/show #{unit}")).await;
        assert!(
            !seen.contains("section hull: wreck"),
            "{who} is still last match's wreck: {seen}"
        );
        assert_eq!(
            cell_current(&seen, "component/cannon/ammo"),
            LOAD,
            "{who} did not come back with a full magazine: {seen}"
        );
        assert!(
            seen.contains("section hull: intact"),
            "{who}'s hull is not intact again: {seen}"
        );
        // And the hull's internals are back above their floor, which is what lets
        // `plan_hit`'s transition to zero fire again — a hull carried in at zero
        // can never be destroyed a second time.
        assert!(
            cell_current(&seen, "section/hull/internals") > 0,
            "{who}'s hull internals are still spent: {seen}"
        );
    }

    // ---- and the second match really runs, from the authored muster ------
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    say(&mut bo, "blue", "Blue Room").await;
    say(&mut ada, "BOARD", "climb into").await;
    say(&mut bo, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;

    // The cockpits: the arena opened on the pack's own muster, not on the kill
    // site, and on a full magazine.
    assert_eq!(
        easting_m(&mut ada).await,
        RED_MUSTER_M,
        "the red tank did not open at its authored cell"
    );
    assert_eq!(
        easting_m(&mut bo).await,
        BLUE_MUSTER_M,
        "the blue tank opened where staff put it for match one, not at its authored cell"
    );
    let seen = ask(&mut bo, "WEAPONSTATUS", "cannon").await;
    assert!(
        seen.contains(&format!("{LOAD} of {LOAD} round(s)")),
        "the formerly destroyed tank's cannon is not full: {seen}"
    );
    // And it is a tank again, not a hulk: it starts up and takes an order.
    power_up(&mut bo).await;
    say(&mut bo, "HEADING 270", "Steering to 270").await;

    say(&mut ada, "SIM ABORT", "aborts match").await;
}

// ---------------------------------------------------------------------------
// (2) A restart mid-match, through the lobby
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restart_mid_match_through_the_lobby_leaves_no_body_in_a_pod() {
    let first = boot("sim-restart-a", None).await;
    let (_god_handle, mut god) = login(first.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;
    let (red, blue) = tank_refs(&mut god).await;
    // **Something for the reset to undo.** Before this the reboot's "at spec, at
    // muster" assertions below were true of the record the world booted with:
    // nothing damaged a tank and nothing moved one, so deleting the boot reset
    // left every one of them green. A spent magazine and a tank parked off its
    // authored cell are the two cheapest facts that make them claims — both are
    // committed before the check-out, so the snapshot a restart restores carries
    // them, exactly as a fought match's damage would.
    say(
        &mut god,
        &format!(
            "@unit/damage #{blue}={{\"ops\":[{{\"AdjustCell\":{{\"addr\":{{\"Component\":{{\"component\":\"cannon\",\"cell\":\"ammo\"}}}},\"delta\":-13}}}}]}}"
        ),
        "Applied",
    )
    .await;
    muster_them_together(&mut god, blue).await;

    let (_a, mut ada) = login(first.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(first.addr, "bo", PILOT_PASSWORD, "Bo").await;
    take_off(&mut ada, &mut bo).await;
    // Both cockpit billets are held, on the tanks' own `assignment:<unit>`
    // documents. That half is **durable** — it is what a restart used to leave
    // behind, and what `@spatial/unassign` was the only cure for.
    for (unit, who) in [(red, RED_TANK), (blue, BLUE_TANK)] {
        wait_for_cockpit(&first.store, unit, true, who).await;
    }
    // Both bodies are inside their pods: `EXIT` is the verb that knows.
    say(&mut ada, "STATUS", "power").await;

    // Stop the server where it stands, mid-`Running`.
    let store = Arc::clone(&first.store);
    drop(ada);
    drop(bo);
    drop(god);
    drop(first);

    let second = boot("sim-restart-b", Some(Arc::clone(&store))).await;
    let (_god2, mut god) = login(second.addr, "boss", GM_PASSWORD, "God").await;

    // (1) No cockpit billet is held, and (2) the tanks are at their spec —
    //     including the magazine staff spent before the match, which the
    //     check-out snapshot carried through the restart.
    //
    //     Both reads are **waited for**, and neither is a gate on the other. The
    //     billet goes back in `boot_matches`' own `release_seats_after_restart`
    //     (for a unit that carried a marker); the record is put back by the reset
    //     in the same pass, and the **pod** half of the sweep — the bodies at (3)
    //     below — is the one that waits for the module's first 1 s tick. All
    //     three then travel their own persist lane, so an unwaited `@unit/show`
    //     still catches the restored check-out snapshot on its way past.
    for (unit, who) in [(red, RED_TANK), (blue, BLUE_TANK)] {
        wait_for_cockpit(&store, unit, false, who).await;
        let seen = poll_unit_show(&mut god, unit, who, |seen| {
            cell_current(seen, "component/cannon/ammo") == LOAD
        })
        .await;
        assert!(
            seen.contains("section hull: intact"),
            "{who} is not intact after the restart: {seen}"
        );
        assert_eq!(
            cell_current(&seen, "component/cannon/ammo"),
            LOAD,
            "{who} did not come back with a full magazine: {seen}"
        );
    }

    // (3) No body is inside a pod. `EXIT` is admitted on `pod_of_body` alone, so
    //     its refusal is the exact statement that nobody is in one — and a `look`
    //     says which room the pilot is standing in instead.
    let (_a2, mut ada) = login(second.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b2, mut bo) = login(second.addr, "bo", PILOT_PASSWORD, "Bo").await;
    for term in [&mut ada, &mut bo] {
        say(term, "EXIT", "You are not in a pod.").await;
    }
    say(&mut ada, "look", "Red Room").await;
    say(&mut bo, "look", "Blue Room").await;

    // (4) And the centre runs a second match, through the lobby, with nobody's
    //     help.
    back_to_the_lobby(&mut ada).await;
    back_to_the_lobby(&mut bo).await;
    take_off(&mut ada, &mut bo).await;
    assert_eq!(
        easting_m(&mut ada).await,
        RED_MUSTER_M,
        "the second match did not open at the authored muster"
    );
    assert_eq!(
        easting_m(&mut bo).await,
        BLUE_MUSTER_M,
        "the blue tank opened where staff put it before the restart, not at its authored cell"
    );
    let seen = ask(&mut bo, "WEAPONSTATUS", "cannon").await;
    assert!(
        seen.contains(&format!("{LOAD} of {LOAD} round(s)")),
        "the rounds staff spent before the restart were never put back: {seen}"
    );
    say(&mut ada, "SIM ABORT", "aborts match").await;
}

/// Whether one unit's `assignment:<unit>` document carries a `cockpit` seat, as
/// the store holds it.
async fn holds_a_cockpit(store: &Arc<dyn ObjectStore>, unit: u32) -> bool {
    store
        .module_json_get("sim", &format!("assignment:{unit}"))
        .await
        .expect("the store read")
        .is_some_and(|document| document.to_string().contains("cockpit"))
}

/// `@unit/show <unit>`, repeated until `settled` is happy with the listing.
///
/// The boot reset happens inside `boot_matches`, before the front door serves a
/// line — but its write then travels the world task's persist lane, which no line
/// this test sends orders, so a read that happens to land first sees the restored
/// check-out snapshot. The budget is bounded and the panic names the last
/// listing, so a reset that never happens still fails — loudly, with the
/// evidence.
async fn poll_unit_show(
    god: &mut Term,
    unit: u32,
    who: &str,
    settled: impl Fn(&str) -> bool,
) -> String {
    let mut seen = String::new();
    for _ in 0..RETRY_BUDGET {
        seen = ask_whole(god, &format!("@unit/show #{unit}")).await;
        if settled(&seen) {
            return seen;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("{who}'s record never settled after the restart:\n{seen}")
}

/// The same read, **waited for** — `arena_custody.rs`'s `until(..)` in the shape
/// this file needs.
///
/// A store read is not a command: nothing about sending a line to the server
/// orders the write this is looking for. Before the restart the billet arrives
/// through `BOARD`'s own persist lane; after it, a marked unit's billet goes back
/// in `boot_matches` and an unmarked one's on the module's **first 1 s tick**
/// (the pod sweep cannot run in `on_world_loaded`, which is before the content
/// latch exists), and either then travels `vacate_station` →
/// `persist_assignments` → the world task's persist lane. On a loaded machine a
/// warm `login` beats all of that, and an unwaited assertion fails against
/// correct code.
async fn wait_for_cockpit(store: &Arc<dyn ObjectStore>, unit: u32, want: bool, who: &str) {
    for _ in 0..100 {
        if holds_a_cockpit(store, unit).await == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!(
        "{who}'s `assignment:{unit}` document never {} a cockpit billet",
        if want { "carried" } else { "gave up" }
    );
}
