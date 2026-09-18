//! **A1, the story test** (16b cut W.3, W4; doc 16 §1 "A1", §3.6, §3.7, §3.8, §5,
//! §8).
//!
//! Doc 16 §1 names two acceptances, and this is the first of them, end to end, on
//! the **production composition** over **real SSH**:
//!
//! > Two connected accounts create characters, walk from the OOC welcome hall into
//! > the simulator lobby, pick opposite teams, board their pods, receive the default
//! > tank, start it up, drive across terrain, see each other only once line of sight
//! > and sensor range allow, lock, fire cannons, and one tank is destroyed. The
//! > match records a result, both pilots leave, and a second match can start.
//!
//! Everything below is one continuous script. No verb is simulated, no module is
//! swapped, no arena function is called directly: two pilots and a bystander type
//! at a terminal, and the assertions are about what each of them was shown.
//!
//! # The four witnesses, and what each is asked
//!
//! | witness | seat | what this file asserts about them |
//! |---|---|---|
//! | Ada | red pilot | the whole cockpit: contacts, the lock and its percentage, the rounds going down, the kill, the team room afterwards |
//! | Bo | blue pilot | the cockpit on the other side, and that it is told **nothing** about the shots that killed it beyond the result |
//! | Cy | a bystander standing in the lobby for the whole match | the centre's own announcements, and **nothing of the arena** |
//! | God | staff | `@arena/status` running then reset, `@unit/show` on the wreck, the ledger, the result document, the money |
//!
//! # What the shipped cockpit can and cannot show a pilot
//!
//! Doc 16 §5 lists *armor and internals per section as current over max* in the
//! status block. **The frame carries none** — [`tacsim::arena::step::Frame`] has the
//! pose, the speed, the power, the contacts, the lock and the weapons and no damage
//! at all — so neither pilot is told about a hit in the cockpit, and "the loser sees
//! its tank destroyed" is asserted on the surfaces that *do* carry it: the centre's
//! own "Match 1 is over (red)" line, the hatch that opens, the result document's
//! `destroyed` row, and `@unit/show`'s `section hull: wreck` on the checked-in tank.
//! That gap is recorded here rather than papered over: a damage panel is a change to
//! the frame protocol and belongs to the cut that adds it, not to the story test.
//!
//! # Why the two accounts are seeded before the world task spawns
//!
//! `mush_server::accounts_identity::hydrate` builds the character → account map
//! **once, at boot**, and nothing updates it afterwards; `SIM TEAM` fails closed on
//! an unmapped character (doc 10 R3). A player who types `create` at the welcome
//! screen therefore cannot claim a simulator slot until the next reboot. Every sim
//! lens in this directory seeds its accounts before the engine is built for that
//! reason and this one does too — the pilots still arrive through the front door,
//! build their sheets with `+chargen`, and **walk** `ooc` → `simulator` → `red`/`blue`
//! rather than being teleported.
//!
//! # Why the fight is frontal
//!
//! Doc 16 §1 asks for *one tank destroyed*, and only the **hull** can destroy a
//! `line_tank` (doc 16 §3.3: `destroy_unit` is the hull's consequence). Every
//! other facet band reaches a section that can end the match some other way — the
//! rear band reaches the engine, whose `disable_motion` plus a lost turret makes a
//! tank `disabled`, and doc 16 §3.8 eliminates a side for that just as readily as
//! for a wreck. A rear-quarter duel therefore ends in a wreck most of the time and
//! in a disablement the rest of it, which is correct engine behaviour and a story
//! test that tells a different story every third run. The **front** band reaches
//! only the hull and the turret, and a turret loss disables nothing, so a frontal
//! duel can end exactly one way: the blue tank blows up. It costs about twice the
//! rounds (400 cm of glacis against 150 cm of engine deck) and buys the acceptance
//! its own premise.
//!
//! # Why the two tanks are re-posed before `SIM START`
//!
//! The shipped `arenas/proving_ground.json` musters red at cell column 6 and blue at
//! column 249: 2.4 km apart against a `visual_mk1` that reaches 600 m, which is a
//! minute of wall time spent proving that a tank can cross a map. So staff re-poses
//! the two slot-1 tanks **62 cells apart** on the east-west road — just outside the
//! sensor's own 60 cells at every signature factor, so the first `CONTACTS` is
//! honestly empty and the detection this test waits for is earned by driving.
//! `arena_def::opening_pose` (16b cut T.3) is what carries the placement into the
//! match. `observer_rule.rs` uses the same device for the same reason.

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
/// How many times a poll waits for the **physics**: the shipped tank takes
/// fifteen seconds to reach its 15 m/s and another eleven to cover the road this
/// lens sends it down, so a budget measured in transient retries is the wrong one
/// for a wait measured in acceleration.
const DRIVE_BUDGET: usize = 120;

/// The two pilots and the bystander, created before the world task spawns so
/// their ids are stable (see the module head).
const ADA: ObjId = ObjId(2);
const BO: ObjId = ObjId(3);
const CY: ObjId = ObjId(4);

/// Slot 1 of each side, as the shipped arena pack names them.
const RED_TANK: &str = "Red Tank 1";
const BLUE_TANK: &str = "Blue Tank 1";
/// The arena's muster room, as the shipped pack labels it.
const MUSTER: &str = "The Proving Ground";

/// Where staff re-poses the two slot-1 tanks: 62 cells apart on the east-west
/// road, two cells beyond the stock sensor's 60 (see the module head).
const RED_CELL: (i64, i64) = (90, 97);
const BLUE_CELL: (i64, i64) = (152, 97);

/// The metre mark on the road at which both tanks are told to stop. Red opens at
/// 905 m and blue at 1,525 m; each has covered 180 m by then, which leaves 260 m
/// between them at the order and about 150 m once the shipped brake (2 m/s² from
/// 15 m/s) has taken hold — well inside the cannon's inner band and far enough
/// apart that doc 16 §3.4's one-unit-per-cell rule never comes into it.
const STOP_AT_M: i64 = 1_085;

/// The cannon's authored load (`ammunition/he_105.json`).
const LOAD: i64 = 40;

/// How many shots the fight is allowed before the test calls it.
///
/// Doc 16 §3.3's **front** hull is 400 armour over 200 internals against 120 a
/// round, so five hull hits end it, and the front facet table splits hits evenly
/// between the hull and the turret: about ten hits, eleven or twelve rounds at
/// the percentage this range resolves at. This is a ceiling for the tail of that
/// distribution and not a budget the ordinary run comes near, and a fight that
/// reaches it fails loudly below rather than waiting for an end that is not
/// coming.
const MAX_SHOTS: usize = 30;

/// The line every witness in the centre reads when the match is settled.
const OVER: &str = "is over (red)";
/// What `CONTACTS` says when the platform holds nothing.
const NO_CONTACTS: &str = "No contacts.";
/// The parser's refusal, which is the sentinel's answer.
const HUH: &str = "Huh?";
/// The one line the nine cockpit selectors refuse with once the pilot is no
/// longer at the station of a deployed unit — which is what they all become the
/// moment the check-in opens the hatches.
const OUTSIDE_COCKPIT: &str = "You are not at the station of a deployed unit.";

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
/// and a boarding window long enough that nothing races it. Every other
/// `[tacsim]` value — the cannon's 6,000 ms cycle included, because it is content
/// and not configuration — is the shipped one.
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
    for (id, name) in [(ADA, "Ada"), (BO, "Bo"), (CY, "Cy")] {
        let made = world
            .create(ObjKind::Player, name, GOD, Some(LIMBO))
            .expect("a character");
        assert_eq!(made, id, "character id assumption");
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
            .expect("a character account");
    }

    // The live grid is Limbo, which the OOC pack hangs its `ooc` exit off: the
    // pilots walk from there, which is what makes this the front door.
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
        linkdead_grace: Duration::from_secs(300),
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
    let accounts = Arc::new(Accounts::new(Arc::clone(&store), handles.handle.clone(), 8));
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

// ---------------------------------------------------------------------------
// The terminal
// ---------------------------------------------------------------------------

/// Connect one account through the welcome screen and wait for its character.
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

/// Run `line` until its **whole** answer carries `needle`, and hand that answer
/// back.
///
/// A read verb renders the **last frame the arena sent** (16b cut K.2's mirror),
/// so a fact the arena has just accepted reaches the pilot on the next pulse and
/// not in the same breath as the order. This is that wait.
async fn poll_whole(term: &mut Term, line: &str, needle: &str) -> String {
    for _ in 0..RETRY_BUDGET {
        let seen = ask_whole(term, line).await;
        if seen.contains(needle) {
            return seen;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!(
        "`{line}` never said `{needle}`:\n{}",
        tail(&strip_ansi(&term.seen))
    );
}

/// The last four thousand characters of a transcript, for a failure message.
fn tail(seen: &str) -> String {
    let start = seen.len().saturating_sub(4_000);
    seen[start..].to_owned()
}

/// **A whole answer, never half of one** — `observer_rule.rs`' device.
///
/// `look`'s room render is one event but several lines, and a needle taken from
/// its first line can return before the contents line has arrived, which would
/// make an absence assertion pass for the wrong reason. So the line is followed by
/// a deliberate nonsense command and the slice runs to *its* refusal.
const SENTINEL: &str = "zzq-story-sentinel";

async fn ask_whole(term: &mut Term, line: &str) -> String {
    for _ in 0..RETRY_BUDGET {
        let mark = term.mark();
        term.send_line(line).await;
        term.send_line(SENTINEL).await;
        // The sentinel is a parser refusal and needs no authority, but it rides the
        // same lane: a world mid-commit answers BOTH lines with the transient and
        // the `Huh?` never comes. That is a retry, not a failure.
        if term.expect_one_of_after(mark, &[HUH, COMMIT_PENDING]).await == 0 {
            return strip_ansi(&term.seen[mark..]);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("`{line}` never produced a whole answer")
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

/// The domain id whose host is `host`, out of an `@domain` listing.
fn domain_of(listing: &str, host: &str) -> u64 {
    listing
        .lines()
        .find(|line| line.contains(&format!("host {host}")))
        .and_then(|line| {
            let rest = line.split_once('#')?.1;
            rest.split_whitespace().next()?.parse::<u64>().ok()
        })
        .unwrap_or_else(|| panic!("no domain hosted on {host:?} in:\n{listing}"))
}

/// The opaque contact id in a `CONTACTS` listing: the sixteen base-16 digits
/// `ContactId` renders as, and the only name a pilot may use for a contact.
fn contact_id_of(listing: &str) -> Option<String> {
    listing
        .split(|c: char| !c.is_ascii_hexdigit())
        .find(|word| word.len() == 16 && u64::from_str_radix(word, 16).is_ok())
        .map(str::to_owned)
}

/// The first unsigned number after `prefix` in `hay`.
fn number_after(hay: &str, prefix: &str) -> u64 {
    let tail = hay
        .split_once(prefix)
        .unwrap_or_else(|| panic!("no {prefix:?} in:\n{hay}"))
        .1;
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    digits
        .parse()
        .unwrap_or_else(|_| panic!("no number after {prefix:?} in:\n{hay}"))
}

/// `WEAPONSTATUS`' cannon row, **or nothing** because the cockpit is gone.
///
/// The settling hit opens the hatch inside the same pulse, so a read issued a
/// breath after the last shot is answered by the nine selectors' one refusal
/// rather than by a weapon table. That is not a failure and it is not a retry: it
/// is the end of the fight, and this is the reader that can say so.
async fn read_cannon_rounds(term: &mut Term) -> Option<i64> {
    let seen = ask_whole(term, "WEAPONSTATUS").await;
    match cannon_rounds(&seen) {
        Some(rounds) => Some(rounds),
        None => {
            assert!(
                seen.contains(OUTSIDE_COCKPIT),
                "a weapon table with no cannon row and no refusal:\n{seen}"
            );
            None
        }
    }
}

/// The cannon's rounds, off a `WEAPONSTATUS` listing: `cannon  ready  39 of 40
/// round(s)  band 1 of 2` answers `39`. The coaxial's belt is two thousand, so
/// the load is what tells the two rows apart.
fn cannon_rounds(listing: &str) -> Option<i64> {
    let needle = format!(" of {LOAD} round(s)");
    listing
        .lines()
        .rfind(|line| line.contains(&needle))
        .and_then(|line| {
            line.split(&needle)
                .next()?
                .split_whitespace()
                .next_back()?
                .parse::<i64>()
                .ok()
        })
}

/// One `@unit/show` cell line's current value: `  cell component/cannon/ammo:
/// 37/40` answers `37`.
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

// ---------------------------------------------------------------------------
// The black box
// ---------------------------------------------------------------------------

/// Every ledger line one match wrote, parsed, in the order the arena wrote them.
///
/// A chunk document carries the chunk's **exact bytes** as one entry's `lines`
/// field (16b T4), so the black box is reassembled by collecting every `lines`
/// string in chunk order and splitting it.
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

/// The kinds of a ledger, in order.
fn kinds(events: &[Value]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| event.get("kind").and_then(Value::as_str))
        .collect()
}

/// The index of the first event of `kind`, for an ordering assertion.
fn first(events: &[Value], kind: &str) -> usize {
    kinds(events)
        .iter()
        .position(|word| *word == kind)
        .unwrap_or_else(|| panic!("the ledger records no `{kind}`: {:?}", kinds(events)))
}

// ---------------------------------------------------------------------------
// The script
// ---------------------------------------------------------------------------

/// Open two accounts, fund them, and hand back `(ada_account, bo_account)` and
/// the two balance lines as they read before a shot is fired.
async fn open_the_books(god: &mut Term) -> ((u64, u64), (String, String)) {
    // The settlement currency is the game's own (`tacsim.credit`, registered at
    // boot by the economy layer), so nothing here invents an instrument.
    let seen = ask(
        god,
        "@account/system faucet=tacsim.credit",
        "Opened account",
    )
    .await;
    let faucet = number_after(&seen, "Opened account #");
    let seen = ask(
        god,
        &format!("@account/open #{}=tacsim.credit", ADA.0),
        "Opened account",
    )
    .await;
    let ada = number_after(&seen, "Opened account #");
    let seen = ask(
        god,
        &format!("@account/open #{}=tacsim.credit", BO.0),
        "Opened account",
    )
    .await;
    let bo = number_after(&seen, "Opened account #");
    for account in [ada, bo] {
        say(
            god,
            &format!("@econ/mint {faucet}={account} 250000"),
            "Minted",
        )
        .await;
    }
    let before = (balance(god, ada).await, balance(god, bo).await);
    assert!(
        before.0.contains("2500") && before.1.contains("2500"),
        "both pilots start the day funded: {before:?}"
    );
    ((ada, bo), before)
}

/// One account's balance line, exactly as `@account/show` prints it.
async fn balance(god: &mut Term, account: u64) -> String {
    let seen = ask_whole(god, &format!("@account/show {account}")).await;
    seen.lines()
        .rfind(|line| line.trim_start().starts_with("balance "))
        .unwrap_or_else(|| panic!("no balance line for account {account}:\n{seen}"))
        .trim()
        .to_owned()
}

/// Re-pose the two slot-1 tanks 62 cells apart on the road, nose to nose (see the
/// module head), and answer their dbrefs.
async fn muster_the_tanks(god: &mut Term) -> (u32, u32) {
    let listing = ask(god, &format!("@find {RED_TANK}"), "END OF LIST").await;
    let red = dbref_of(&listing, RED_TANK);
    let listing = ask(god, &format!("@find {BLUE_TANK}"), "END OF LIST").await;
    let blue = dbref_of(&listing, BLUE_TANK);
    let listing = ask(god, "@domain", &format!("host {MUSTER}")).await;
    let domain = domain_of(&listing, MUSTER);
    for (unit, (x, y), heading) in [(red, RED_CELL, 90_000), (blue, BLUE_CELL, 270_000)] {
        let pose = format!(
            "{{\"x_cm\":{},\"y_cm\":{},\"heading_mdeg\":{heading}}}",
            x * 1_000 + 500,
            y * 1_000 + 500
        );
        say(
            god,
            &format!("@spatial/place/domain #{unit}={domain},{pose}"),
            "Placed",
        )
        .await;
    }
    (red, blue)
}

/// The front door, for one pilot: a sheet built with the three commands the game's
/// chargen has, then the walk from Limbo through the welcome hall into the lobby.
async fn arrive(term: &mut Term, name: &str, callsign: &str, gunnery: u32) {
    say(term, "+chargen", "A blank pilot sheet is yours").await;
    say(term, &format!("+stat name={name}"), &format!("= {name}.")).await;
    say(
        term,
        &format!("+stat callsign={callsign}"),
        &format!("= {callsign}."),
    )
    .await;
    say(
        term,
        &format!("+stat gunnery={gunnery}"),
        &format!("gunnery = {gunnery}."),
    )
    .await;
    say(term, "ooc", "Welcome Hall").await;
    say(term, "simulator", "Simulator Lobby").await;
}

/// Bring a tank's power up and wait for the arena to say it is online.
async fn power_up(term: &mut Term) {
    say(term, "STARTUP", "Startup sequence").await;
    poll_whole(term, "STATUS", "power online").await;
}

/// Point a tank down the road and open it up.
async fn drive(term: &mut Term, heading: u32) {
    for _ in 0..RETRY_BUDGET {
        let mark = term.mark();
        term.send_line(&format!("HEADING {heading}")).await;
        if term
            .expect_one_of_after(
                mark,
                &[
                    &format!("Steering to {heading}"),
                    "Power not online",
                    COMMIT_PENDING,
                    CONTROL_UNKNOWN,
                ],
            )
            .await
            == 0
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    say(term, "SPEED 54", "Making 54 kph").await;
}

/// One tank's easting, metres, off its own `STATUS` line (`  at 1085 m east,
/// 975 m north; …`).
async fn easting_m(term: &mut Term) -> i64 {
    let seen = ask_whole(term, "STATUS").await;
    let line = seen
        .lines()
        .rfind(|line| line.contains("m east"))
        .unwrap_or_else(|| panic!("no easting in:\n{seen}"));
    i64::try_from(number_after(line, "at ")).expect("an easting fits")
}

/// Everything a surface must never say about a unit an arena holds.
fn names_nothing(seen: &str, red: u32, blue: u32, what: &str) {
    never_names(
        seen,
        &[
            RED_TANK.to_string(),
            BLUE_TANK.to_string(),
            format!("#{red}"),
            format!("#{blue}"),
        ],
        what,
    );
}

/// `seen` says none of `leaks`.
fn never_names(seen: &str, leaks: &[String], what: &str) {
    for leak in leaks {
        assert!(!seen.contains(leak), "{what} leaked {leak:?}:\n{seen}");
    }
}

/// Everything a surface outside the two cockpits must never say about the
/// shooting (doc 16 §3.6: *nothing else about another unit crosses to a pilot*,
/// and nothing at all crosses to the world).
fn says_nothing_of_the_fight(seen: &str, contact: &str, what: &str) {
    for leak in [
        contact,
        "Firing",
        "Locked on",
        "Locking",
        "bearing",
        "round(s)",
    ] {
        assert!(
            !seen.contains(leak),
            "{what} leaked {leak:?} out of the arena:\n{seen}"
        );
    }
}

// ---------------------------------------------------------------------------
// A1
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_pilots_walk_in_fight_and_one_tank_is_destroyed() {
    let server = boot("arena-story").await;

    // -----------------------------------------------------------------
    // (0) Staff opens the books and musters the two tanks.
    // -----------------------------------------------------------------
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let ((ada_account, bo_account), money_before) = open_the_books(&mut god).await;
    let (red, blue) = muster_the_tanks(&mut god).await;
    say(&mut god, "ooc", "Welcome Hall").await;
    say(&mut god, "simulator", "Simulator Lobby").await;
    say(&mut god, "@arena/status", "No match is running.").await;

    // -----------------------------------------------------------------
    // (1) Two pilots and a bystander come in through the front door.
    // -----------------------------------------------------------------
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;
    arrive(&mut ada, "Ada Renard", "Ladder", 4).await;
    arrive(&mut bo, "Bo Nakamura", "Kestrel", 2).await;
    say(&mut cy, "ooc", "Welcome Hall").await;
    say(&mut cy, "simulator", "Simulator Lobby").await;
    // The bystander's own clock starts here: everything after this mark is what
    // the lobby showed somebody who never left it.
    let bystander_mark = cy.mark();

    // -----------------------------------------------------------------
    // (2) Opposite teams, and the centre says so to the room.
    // -----------------------------------------------------------------
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    cy.expect_after(bystander_mark, "Ada takes red slot 1")
        .await;
    cy.expect_after(bystander_mark, "Bo takes blue slot 1")
        .await;

    let status = ask(&mut ada, "SIM STATUS", "blue:").await;
    assert!(status.contains("selection"), "{status}");

    // -----------------------------------------------------------------
    // (3) `SIM START`, the walk to the team rooms, and the pods.
    // -----------------------------------------------------------------
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    say(&mut bo, "blue", "Blue Room").await;
    say(&mut ada, "BOARD", "climb into").await;
    say(&mut bo, "BOARD", "climb into").await;
    // The second seat fires cut K.1's `Boarding -> Running` trigger, and the
    // centre says so in every one of its rooms — the lobby the bystander is
    // standing in included.
    ada.expect("Good hunting").await;
    cy.expect_after(bystander_mark, "Good hunting").await;

    let status = ask(&mut god, "@arena/status", "ledger writes awaiting").await;
    assert!(status.contains("running"), "{status}");
    assert!(status.contains("tempo"), "{status}");

    // -----------------------------------------------------------------
    // (4) Power up. Before detection, neither scope holds anything.
    // -----------------------------------------------------------------
    power_up(&mut ada).await;
    power_up(&mut bo).await;
    let seen = ask(&mut ada, "CONTACTS", NO_CONTACTS).await;
    names_nothing(
        &seen,
        red,
        blue,
        "the red pilot's `CONTACTS` before detection",
    );
    let seen = ask(&mut bo, "CONTACTS", NO_CONTACTS).await;
    names_nothing(
        &seen,
        red,
        blue,
        "the blue pilot's `CONTACTS` before detection",
    );
    // And the world is told nothing about either of them while the arena holds
    // them: the muster room's own domain is gone from every staff read.
    say(
        &mut god,
        &format!("@unit/show #{red}"),
        "not a tactical unit",
    )
    .await;
    say(&mut god, "@domain", "No domains.").await;

    // -----------------------------------------------------------------
    // (5) Drive toward each other until the sensor pass finds something.
    // -----------------------------------------------------------------
    drive(&mut ada, 90).await;
    drive(&mut bo, 270).await;

    let mut held = String::new();
    for _ in 0..DRIVE_BUDGET {
        let mark = ada.mark();
        ada.send_line("CONTACTS").await;
        if ada
            .expect_one_of_after(
                mark,
                &["bearing", NO_CONTACTS, COMMIT_PENDING, CONTROL_UNKNOWN],
            )
            .await
            == 0
        {
            held = strip_ansi(&ada.seen[mark..]);
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        !held.is_empty(),
        "the two tanks never detected each other:\n{}",
        tail(&strip_ansi(&ada.seen))
    );
    let contact = contact_id_of(&held)
        .unwrap_or_else(|| panic!("a contact id in:\n{held}"))
        .to_owned();
    names_nothing(&held, red, blue, "the red pilot's held `CONTACTS`");

    // The notice came first, unasked, while she was driving — doc 16 §3.6's
    // `acquired` — and then, after `sensors.classify_after_ms`, the side.
    assert!(
        strip_ansi(&ada.seen).contains(&format!("New contact {contact}")),
        "the acquisition notice reached the pilot:\n{held}"
    );
    let classified = poll_whole(&mut ada, "CONTACTS", "blue").await;
    names_nothing(&classified, red, blue, "the classified `CONTACTS`");
    assert!(
        strip_ansi(&ada.seen).contains(&format!("Contact {contact} identified")),
        "and the classification notice did too:\n{classified}"
    );

    // -----------------------------------------------------------------
    // (6) Close and halt, nose to nose.
    // -----------------------------------------------------------------
    let mut closed = false;
    for _ in 0..DRIVE_BUDGET {
        if easting_m(&mut ada).await >= STOP_AT_M {
            closed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    assert!(
        closed,
        "the red tank never reached {STOP_AT_M} m east; the fight below would be \
         fought at a range this lens did not choose"
    );
    say(&mut ada, "SPEED 0", "Making 0 kph").await;
    say(&mut bo, "SPEED 0", "Making 0 kph").await;
    // Nose to nose, and neither tank turns: see the module head for why the fight
    // is fought on the front facet and not on the cheaper rear one.
    poll_whole(&mut bo, "STATUS", "heading 270").await;

    // While all of that happened, the bystander was looking at a lobby that says
    // nothing about it.
    let seen = ask_whole(&mut cy, "look").await;
    assert!(
        seen.contains("Simulator Lobby"),
        "the bystander really looked:\n{seen}"
    );
    names_nothing(&seen, red, blue, "the bystander's `look` mid-match");
    says_nothing_of_the_fight(&seen, &contact, "the bystander's `look` mid-match");

    // -----------------------------------------------------------------
    // (7) The lock, its percentage, and the fight.
    // -----------------------------------------------------------------
    let seen = ask_whole(&mut ada, "WEAPONSTATUS").await;
    assert!(seen.contains("40 of 40 round(s)"), "{seen}");
    assert!(!seen.contains("Locked on"), "{seen}");

    say(&mut ada, &format!("LOCK {contact}"), "Locking").await;
    let locked = poll_whole(&mut ada, "WEAPONSTATUS", "Locked on").await;
    assert!(
        locked.contains(&contact),
        "the lock is the contact: {locked}"
    );
    assert!(
        locked.contains("% to hit"),
        "doc 16 §3.5 step 5's percentage, before any shot: {locked}"
    );
    assert!(
        locked.contains("band 1 of 2"),
        "a hundred and fifty metres is the cannon's inner band: {locked}"
    );

    let fight_mark = ada.mark();
    let mut shots: i64 = 0;
    // Every `(shots fired, rounds the cockpit then showed)` pair this fight could
    // read back — the in-flight half of §8's ammunition lens.
    let mut readings: Vec<(i64, i64)> = Vec::new();
    for _ in 0..MAX_SHOTS {
        // `FIRE` while the breech is busy is refused for the cycle and nothing
        // else; once the check-in has opened the hatch the nine selectors refuse
        // together, which is how this loop learns the fight is over.
        let mut fired = false;
        for _ in 0..RETRY_BUDGET {
            let mark = ada.mark();
            ada.send_line("FIRE").await;
            match ada
                .expect_one_of_after(
                    mark,
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
                // The cycle, and the world's two transients: all three are a wait
                // and none of them is an answer about the shot.
                1..=3 => tokio::time::sleep(Duration::from_millis(500)).await,
                _ => break,
            }
        }
        if !fired {
            break;
        }
        shots += 1;
        // Read the cockpit's own count back once the mirror has caught up with
        // the shot — one pulse later, never in the same breath as the order.
        //
        // A `FIRE` sent against a frame one pulse behind is accepted by the
        // cockpit's pre-check and refused by the arena against its own
        // `ready_at_ms`, and then the count never moves: that is a round that went
        // nowhere, so the tally is put back and the loop tries again. It is the one
        // place this test has to know that the cockpit is a mirror and not the
        // truth.
        let want = LOAD - shots;
        let mut cockpit = true;
        let mut confirmed = false;
        for _ in 0..16 {
            match read_cannon_rounds(&mut ada).await {
                Some(rounds) if rounds == want => {
                    readings.push((shots, rounds));
                    confirmed = true;
                    break;
                }
                Some(_) => {}
                None => {
                    cockpit = false;
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        if !cockpit || strip_ansi(&ada.seen[fight_mark..]).contains(OVER) {
            break;
        }
        if !confirmed {
            shots -= 1;
        }
    }
    assert!(shots > 0, "the red tank never got a shot away");
    assert!(
        strip_ansi(&ada.seen[fight_mark..]).contains(OVER)
            || strip_ansi(&ada.seen[fight_mark..]).contains(OUTSIDE_COCKPIT),
        "{shots} rounds went into the blue tank and it is still fighting"
    );
    assert!(
        !readings.is_empty(),
        "the cockpit never showed a round count after a shot ({shots} fired):\n{}",
        tail(&strip_ansi(&ada.seen))
    );
    for (fired, rounds) in &readings {
        assert_eq!(
            fired + rounds,
            LOAD,
            "after {fired} shot(s) the cockpit showed {rounds} of {LOAD} rounds"
        );
    }

    // -----------------------------------------------------------------
    // (8) The end, as each of the four witnesses saw it.
    // -----------------------------------------------------------------
    ada.expect_after(fight_mark, OVER).await;
    ada.expect_after(fight_mark, "Its units are checked in.")
        .await;
    ada.expect_after(fight_mark, "The hatch of Red Pod 1 swings open.")
        .await;
    bo.expect(OVER).await;
    bo.expect("The hatch of Blue Pod 1 swings open.").await;
    cy.expect_after(bystander_mark, OVER).await;

    // Both pilots are back in their own team rooms, out of the cockpit.
    let seen = ask_whole(&mut ada, "look").await;
    assert!(seen.contains("Red Room"), "the winner walked out: {seen}");
    let seen = ask_whole(&mut bo, "look").await;
    assert!(seen.contains("Blue Room"), "the loser walked out: {seen}");
    say(&mut bo, "WEAPONSTATUS", OUTSIDE_COCKPIT).await;

    // The bystander stood in the lobby for the whole match and was shown the
    // centre's announcements and **nothing of the arena** (doc 16 §1's observer
    // rule).
    let bystander = strip_ansi(&cy.seen[bystander_mark..]);
    names_nothing(&bystander, red, blue, "the bystander's whole transcript");
    says_nothing_of_the_fight(&bystander, &contact, "the bystander's whole transcript");
    assert!(
        bystander.contains("Match 1 is running")
            && bystander.contains(OVER)
            && bystander.contains("Ada takes red slot 1"),
        "but the centre's own announcements did reach it:\n{bystander}"
    );

    // The loser's transcript names its killer nowhere. It names its **own** tank
    // — `STATUS` prints the platform a pilot is sitting in — and nothing of the
    // other side: not the red tank, not its dbref, not the shot, not the lock.
    // The cockpit carries no damage feed at all (see the module head), so what
    // the blue pilot was told about the rounds that killed it is the centre's own
    // result line and nothing else.
    let loser = strip_ansi(&bo.seen);
    never_names(
        &loser,
        &[
            RED_TANK.to_string(),
            format!("#{red}"),
            "Firing".to_string(),
            "Locked on".to_string(),
        ],
        "the loser's whole transcript",
    );
    assert!(
        loser.contains(OVER),
        "but it was told which side won:\n{}",
        tail(&loser)
    );

    // -----------------------------------------------------------------
    // (9) Staff: the wreck, the survivor, and the ammunition.
    // -----------------------------------------------------------------
    let wreck = ask_whole(&mut god, &format!("@unit/show #{blue}")).await;
    assert!(
        wreck.contains("section hull: wreck"),
        "the loser's tank came back a wreck (doc 16 §3.3): {wreck}"
    );
    let survivor = ask_whole(&mut god, &format!("@unit/show #{red}")).await;
    assert!(
        !survivor.contains("section hull: wreck"),
        "and the winner's did not: {survivor}"
    );
    let left = cell_current(&survivor, "component/cannon/ammo");

    // -----------------------------------------------------------------
    // (10) The black box.
    // -----------------------------------------------------------------
    let events = ledger_events(&server.store, 1).await;
    let order = [
        "match_started",
        "detected",
        "classified",
        "locked",
        "fired",
        "impact",
        "hit",
        "unit_destroyed",
        "match_ended",
    ];
    let mut previous = 0_usize;
    for kind in order {
        let at = first(&events, kind);
        assert!(
            at >= previous,
            "the ledger's `{kind}` came before the event that has to precede it: {:?}",
            kinds(&events)
        );
        previous = at;
    }
    let ended = events
        .iter()
        .find(|event| event.get("kind").and_then(Value::as_str) == Some("match_ended"))
        .expect("a match_ended line");
    assert_eq!(
        ended
            .pointer("/reason/elimination/winner")
            .and_then(Value::as_str),
        Some("red"),
        "the black box names the winner: {ended}"
    );
    let destroyed = events
        .iter()
        .find(|event| event.get("kind").and_then(Value::as_str) == Some("unit_destroyed"))
        .expect("a unit_destroyed line");
    assert_eq!(
        destroyed.get("unit").and_then(Value::as_u64),
        Some(u64::from(blue)),
        "and which tank went up: {destroyed}"
    );
    // Every hit is the red tank's, on the blue one, and carries the plan's cause.
    let hits: Vec<&Value> = events
        .iter()
        .filter(|event| event.get("kind").and_then(Value::as_str) == Some("hit"))
        .collect();
    assert!(
        !hits.is_empty(),
        "somebody hit something: {:?}",
        kinds(&events)
    );
    for hit in &hits {
        assert_eq!(
            hit.get("unit").and_then(Value::as_u64),
            Some(u64::from(red))
        );
        assert_eq!(
            hit.get("target").and_then(Value::as_u64),
            Some(u64::from(blue))
        );
        assert!(
            hit.get("cause")
                .and_then(Value::as_str)
                .is_some_and(|cause| cause.starts_with("shot:")),
            "a hit carries its mutation plan's cause: {hit}"
        );
    }
    // **Ammunition is conserved across a match** (§8's lens): what the black box
    // says left the barrel plus what the checked-in tank still carries is the
    // authored load. The wreck's own rounds burned with it, which is why the law
    // is stated for the tank that survived.
    let fired: Vec<&Value> = events
        .iter()
        .filter(|event| event.get("kind").and_then(Value::as_str) == Some("fired"))
        .filter(|event| event.get("unit").and_then(Value::as_u64) == Some(u64::from(red)))
        .collect();
    for shot in &fired {
        assert_eq!(
            shot.get("rounds").and_then(Value::as_i64),
            Some(1),
            "the cannon's burst is one round (`weapons/cannon_105.json`): {shot}"
        );
    }
    let fired_rounds = i64::try_from(fired.len()).expect("a shot count fits");
    assert!(fired_rounds > 0, "the black box records no shot at all");
    assert_eq!(
        fired_rounds + left,
        LOAD,
        "rounds fired ({fired_rounds}) plus rounds left ({left}) is the load"
    );
    // The test's own count is what the *cockpit* accepted; the ledger's is what the
    // arena took. They agree unless a `FIRE` sent against a frame one pulse behind
    // was accepted here and refused there, which is a wait the loop above can lose
    // and never a round that went anywhere.
    assert!(
        shots >= fired_rounds,
        "the black box recorded {fired_rounds} shots and the cockpit accepted only {shots}"
    );

    // -----------------------------------------------------------------
    // (11) The result document.
    // -----------------------------------------------------------------
    let result = server
        .store
        .module_json_get("tacsim", "tacsim:match:1")
        .await
        .expect("the store read")
        .expect("one result document per match (doc 16 §4)");
    assert_eq!(
        result.get("outcome").and_then(Value::as_str),
        Some("red"),
        "the result names the winner: {result}"
    );
    assert_eq!(
        result.get("reason").and_then(Value::as_str),
        Some("elimination"),
        "{result}"
    );
    assert_eq!(
        result.pointer("/ledger/log").and_then(Value::as_str),
        Some("match.1"),
        "{result}"
    );
    let digest = result
        .pointer("/ledger/digest")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("one digest per match: {result}"));
    assert_eq!(digest.len(), 64, "one sha-256 digest: {result}");
    let rows = result
        .get("units")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("a per-unit row each: {result}"));
    let row_of = |unit: u32| {
        rows.iter()
            .find(|row| row.get("unit").and_then(Value::as_u64) == Some(u64::from(unit)))
            .unwrap_or_else(|| panic!("no row for #{unit}: {result}"))
    };
    assert_eq!(
        row_of(blue).get("outcome").and_then(Value::as_str),
        Some("destroyed"),
        "{result}"
    );
    assert_eq!(
        row_of(red).get("outcome").and_then(Value::as_str),
        Some("intact"),
        "{result}"
    );
    assert_eq!(
        row_of(red).get("shots").and_then(Value::as_i64),
        Some(fired_rounds),
        "the winner's shot counter is the arena's own, and it is the black box's: {result}"
    );
    assert!(
        row_of(red)
            .get("hits")
            .and_then(Value::as_i64)
            .is_some_and(|hits| hits > 0 && hits <= fired_rounds),
        "and its hits are a subset of them: {result}"
    );
    let pilots = result
        .pointer("/teams/0/pilots")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("the roster carries its pilots: {result}"));
    assert_eq!(
        pilots
            .first()
            .and_then(|pair| pair.get("pilot"))
            .and_then(Value::as_u64),
        Some(u64::from(ADA.0)),
        "{result}"
    );

    // -----------------------------------------------------------------
    // (12) The money rule: nothing in the campaign economy moved.
    // -----------------------------------------------------------------
    let money_after = (
        balance(&mut god, ada_account).await,
        balance(&mut god, bo_account).await,
    );
    assert_eq!(
        money_after, money_before,
        "doc 16 §1's money rule: a match moves no instrument"
    );

    // -----------------------------------------------------------------
    // (13) The centre resets, and a second match can start.
    // -----------------------------------------------------------------
    let status = ask(&mut god, "SIM STATUS", "blue:").await;
    assert!(status.contains("ended"), "{status}");
    assert!(
        status.contains("red"),
        "the outcome is on the board: {status}"
    );
    say(&mut god, "@arena/reset", "centre is reset").await;
    say(&mut god, "@arena/reset", "already idle; nothing changed").await;
    say(&mut god, "@arena/status", "No match is running.").await;
    // Both pilots walk back to the lobby and claim again: doc 16 §1's *a second
    // match can start*.
    say(&mut ada, "lobby", "Simulator Lobby").await;
    say(&mut bo, "lobby", "Simulator Lobby").await;
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
}
