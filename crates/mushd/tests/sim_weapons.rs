//! **The weapon lens, over real SSH** (16b cut W.1, W1; doc 16 §3.5, §3.6, §3.7's
//! `LOCK`, `FIRE` and `WEAPONSTATUS`).
//!
//! `sim_cockpit.rs` proved a pilot can board, steer and climb out. This is the half
//! that shoots, and every claim in it is a claim about two real accounts in two real
//! pods on the shipped proving ground, so it drives the whole stack the way
//! `sim_cockpit.rs` and `arena_custody.rs` do.
//!
//! One test, and it is doc 16 §1's A1 minus the damage (which is cut W.3's): two
//! pilots board, bring power up, **see each other** as contacts, `LOCK` one of them
//! by the opaque id `CONTACTS` printed, `FIRE`, and then read `WEAPONSTATUS` on both
//! sides — the shooter a round down and cycling, the target untouched and ready —
//! while the match's own ledger records `locked`, `fired` and `impact`.
//!
//! # Why the blue tank is moved before the match starts
//!
//! The shipped `arenas/proving_ground.json` musters red at cell column 6 and blue at
//! column 249: 2.4 km apart, against a `visual_mk1` that reaches 600 m. Two tanks
//! closing that at 54 kph is sixty seconds of real time, which is not a test — so
//! staff puts the blue tank on **Red Tank 4's own authored cell**, four cells south of
//! Red Tank 1, and the match is fought there.
//!
//! That cell and the three between them are authored open ground — `Grass`, and the
//! road band itself at row 96 — at one flat elevation (the pack's `field.q0_0`
//! heightfield holds column 6 level from row 92 to row 102), so the line of sight is
//! clear and the signature factor is the open-ground one: the detection this test
//! waits for is the shipped
//! sensor's own answer on the shipped ground, not a contrivance. `opening_pose`
//! (16b cut T.3) is what carries the placement into the match: *the unit's own world
//! pose when it is on the window and its cell is free*.

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

/// The two pilot characters, created before the world task spawns so their ids are
/// stable.
const ADA: ObjId = ObjId(2);
const BO: ObjId = ObjId(3);

/// The one line a motion or fire order refuses with while the tank is cold.
const POWER_NOT_ONLINE: &str = "Power not online";

/// Where staff puts the blue tank: Red Tank 4's authored cell, four cells south of
/// Red Tank 1's, in centimetres of the arena window's own frame (cell centre).
/// 16b cut F.4 moved the muster into the ford's rows, so slot 4 is row 93 and not
/// row 130.
const BLUE_X_CM: i64 = 6_500;
const BLUE_Y_CM: i64 = 93_500;

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

/// A fast arena: a short startup so a pilot is flying within a test's patience, and
/// a boarding window long enough that nothing races it.
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

/// Run `line` until its answer carries `needle`, and hand that answer back.
///
/// The frame a read verb renders is the **last one the arena sent** (16b cut K.2's
/// mirror), so a fact the arena has just accepted — a lock, a spent round — reaches
/// the pilot on the next pulse and not in the same breath as the order. This is that
/// wait, and it is the honest one: a test that asserted the mirror immediately would
/// be asserting that the cockpit keeps a second model of the arena's state.
async fn poll(term: &mut Term, line: &str, echo: &str, needle: &str) -> String {
    for _ in 0..RETRY_BUDGET {
        let seen = ask(term, line, echo).await;
        if seen.contains(needle) {
            return seen;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!(
        "`{line}` never said `{needle}`:\n{}",
        strip_ansi(&term.seen[term.seen.len().saturating_sub(4_000)..])
    );
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

/// The opaque contact id in a `CONTACTS` listing: the sixteen base-16 digits
/// `ContactId` renders as, and the only name a pilot may use for a contact.
fn contact_id_of(listing: &str) -> Option<String> {
    listing
        .split_whitespace()
        .find(|word| word.len() == 16 && u64::from_str_radix(word, 16).is_ok())
        .map(str::to_owned)
}

/// Walk both pilots into the simulator lobby and put staff in the lobby with them.
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

/// Put the blue tank four cells south of the red one, on ground the pack authored.
/// See the module head for why.
async fn muster_them_together(god: &mut Term) {
    let listing = ask(god, "@find Proving Ground", "END OF LIST").await;
    let ground = dbref_of(&listing, "Proving Ground");
    let listing = ask(god, "@find Blue Tank 1", "END OF LIST").await;
    let blue = dbref_of(&listing, "Blue Tank 1");
    say(
        god,
        &format!(
            "@spatial/place/domain #{blue}=#{ground},{{\"x_cm\":{BLUE_X_CM},\"y_cm\":{BLUE_Y_CM},\"heading_mdeg\":270000}}"
        ),
        "Placed",
    )
    .await;
}

/// Claim, start, walk and board: the state the shot is taken from.
async fn take_off(ada: &mut Term, bo: &mut Term) {
    say(ada, "SIM TEAM red", "takes red slot 1").await;
    say(bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(ada, "SIM START", "is boarding").await;
    say(ada, "red", "Red Room").await;
    say(bo, "blue", "Blue Room").await;
    say(ada, "BOARD", "climb into").await;
    say(bo, "BOARD", "climb into").await;
    // The second seat fires cut K.1's trigger, and the centre says so in every room.
    ada.expect("Good hunting").await;
}

/// Bring power up, waiting for the arena to say it is online.
async fn power_up(term: &mut Term) {
    say(term, "STARTUP", "Startup sequence").await;
    for _ in 0..RETRY_BUDGET {
        let seen = ask(term, "STATUS", "power").await;
        if seen.contains("power online") {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("the tank never came online");
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
        {
            let seen = strip_ansi(&term.seen[mark..]);
            if let Some(id) = contact_id_of(&seen) {
                return id;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!(
        "nothing was ever detected:\n{}",
        strip_ansi(&term.seen[term.seen.len().saturating_sub(4_000)..])
    );
}

/// Every ledger chunk one match wrote, as one string.
async fn ledger_text(store: &Arc<dyn ObjectStore>, match_id: u64) -> String {
    let log = format!("match.{match_id}");
    let mut text = String::new();
    for chunk in 0..64 {
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
// The lens
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_pilots_detect_lock_and_fire_and_the_ledger_records_the_shot() {
    let server = boot("sim-weapons").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;
    muster_them_together(&mut god).await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    take_off(&mut ada, &mut bo).await;

    // A cold tank with no lock is told about the **lock** first: doc 16 §3.7's
    // refusal column puts power last, which is what makes `FIRE` useful advice.
    let seen = ask(&mut ada, "FIRE", "No lock").await;
    assert!(
        !seen.contains(POWER_NOT_ONLINE),
        "the lock comes first: {seen}"
    );

    power_up(&mut ada).await;
    power_up(&mut bo).await;

    // Forty metres of open grass: the shipped sensor sees it, and the pilot is
    // handed one opaque id and nothing else about what is out there.
    let contact = wait_for_contact(&mut ada).await;
    let seen = ask(&mut ada, "CONTACTS", "bearing").await;
    assert!(
        !seen.contains("Blue Tank"),
        "a contact is never named by its object: {seen}"
    );

    // Before the lock, `WEAPONSTATUS` is a full load and no band.
    let seen = ask(&mut ada, "WEAPONSTATUS", "cannon").await;
    assert!(seen.contains("40 of 40 round(s)"), "{seen}");
    assert!(seen.contains("ready"), "{seen}");
    assert!(!seen.contains("Locked on"), "{seen}");

    // The lock, and then the shot. The arena records the lock in its own phase 4 and
    // the pilot reads it back off the next frame, which is what `poll` waits for.
    say(&mut ada, &format!("LOCK {contact}"), "Locking").await;
    let seen = poll(&mut ada, "WEAPONSTATUS", "cannon", "Locked on").await;
    assert!(seen.contains(&contact), "the lock is the contact: {seen}");
    assert!(
        seen.contains("band 1 of 2"),
        "forty metres is the cannon's inner band: {seen}"
    );

    // **16b cut F.2**: the percentage is the **arena's** line, said once when the
    // lock resolves at its own timestamp. Cut W.2 printed it from the cockpit as
    // well, off the frame the world held *before* the order — so a pilot who
    // re-locked to watch the number close as the range shortened read two lines
    // with two different numbers, the first of them stale. One order, one line.
    let repeat = ada.mark();
    say(&mut ada, &format!("LOCK {contact}"), "Locking").await;
    ada.expect_after(repeat, "Locked on ").await;
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    let after = strip_ansi(&ada.seen[repeat..]);
    assert_eq!(
        after.matches("Locked on ").count(),
        1,
        "one LOCK, one percentage:\n{after}"
    );
    assert!(
        after.contains("% to hit"),
        "and it carries the chance doc 16 §3.5 step 5 promises:\n{after}"
    );

    // From here the shooter's own outcome line is waited for rather than read out
    // of the scrollback: the impact resolves a pulse after the `FIRE`, so a
    // synchronous scan would race the arena.
    let shot = ada.mark();
    say(&mut ada, "FIRE", "Firing cannon").await;

    // The shooter: a round down, and cycling. The second `FIRE` is refused for the
    // cycle and not for the lock, which is the arena's own `ready_at_ms` speaking.
    let spent = poll(&mut ada, "WEAPONSTATUS", "cannon", "39 of 40 round(s)").await;
    assert!(spent.contains("cycling"), "{spent}");
    let seen = ask(&mut ada, "FIRE", "Still cycling").await;
    assert!(!seen.contains("No lock"), "the lock is still held: {seen}");

    // The target: untouched in the magazine, and ready. Damage is below.
    let seen = ask(&mut bo, "WEAPONSTATUS", "cannon").await;
    assert!(seen.contains("40 of 40 round(s)"), "{seen}");
    assert!(seen.contains("ready"), "{seen}");
    assert!(
        !seen.contains("Locked on"),
        "the blue pilot locked nothing: {seen}"
    );

    // **16b cut F.2**: what the two pilots are told about a round.
    //
    // The shooter is told at the **impact** and not at the `FIRE`, so the outcome
    // line is waited for from the mark taken before the shot: one line, naming
    // the contact the shooter's own platform holds and nothing about the target's
    // tank.
    ada.expect_one_of_after(shot, &["Hit on ", "Miss on "])
        .await;
    let seen = strip_ansi(&ada.seen[shot..]);
    let outcome = seen
        .lines()
        .find(|line| line.starts_with("Hit on ") || line.starts_with("Miss on "))
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("the shooter was told nothing about their round:\n{seen}"));
    assert!(
        outcome.contains(&contact),
        "the outcome names the contact it holds and never the unit: {outcome}"
    );
    assert!(
        !outcome.contains("Blue Tank")
            && !outcome.contains("armour")
            && !outcome.contains("internals"),
        "and carries nothing about the target's own tank: {outcome}"
    );

    // Then the coaxial, whose pull is a ten-round burst: at this range one of
    // them lands, so the victim's half of the surface is exercised without the
    // test depending on a single draw. Both waits are taken from a mark, because
    // `Term::expect` scans the whole transcript from the start and the cannon's
    // own round has already put `Hit on ` in both of them — the burst would prove
    // nothing at all.
    let burst = ada.mark();
    let taking = bo.mark();
    say(&mut ada, "FIRE machine_gun", "Firing machine_gun").await;
    ada.expect_after(burst, "Hit on ").await;

    // The victim: their own section and facet, and what is left there, with
    // nothing whatever about Ada (`observer_rule.rs` asserts that absence surface
    // by surface).
    bo.expect_after(taking, "Hit on the ").await;

    // And doc 16 §5's status block, which until this cut did not exist at all: it
    // lists every section, and the facet the round passed is now below maximum.
    let seen = poll(&mut bo, "STATUS", "power", "internals").await;
    assert!(seen.contains("hull"), "every section is listed: {seen}");
    let moved = seen.lines().any(|line| {
        line.split_whitespace().any(|word| {
            word.split_once('/').is_some_and(|(now, max)| {
                match (now.parse::<i64>(), max.parse::<i64>()) {
                    (Ok(now), Ok(max)) => now < max,
                    _ => false,
                }
            })
        })
    });
    assert!(moved, "the status block never moved: {seen}");

    // And the black box: the lock, the shot and its arrival, in the match's own
    // ledger on the world's writer lane.
    tokio::time::sleep(Duration::from_millis(800)).await;
    say(&mut ada, "SIM ABORT", "aborts match").await;
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let ledger = ledger_text(&server.store, 1).await;
    for kind in ["locked", "fired", "impact"] {
        assert!(
            ledger.contains(&format!("kind\\\":\\\"{kind}"))
                || ledger.contains(&format!("kind\":\"{kind}")),
            "the match ledger records `{kind}`: {ledger}"
        );
    }
    assert!(
        ledger.contains("shot_seq"),
        "and the shot carries its sequence: {ledger}"
    );
}
