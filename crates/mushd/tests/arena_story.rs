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
//! | Ada | red pilot | the whole cockpit: contacts, the lock and its percentage, the rounds going down, the kill (or, since 16b cut AP.2, the turret going), the team room afterwards |
//! | Bo | blue pilot | the cockpit on the other side: that it is told of every round that hit it and of its hull (or its turret) going, and **still nothing** that identifies or locates its killer |
//! | Cy | a bystander standing in the lobby for the whole match | the centre's own announcements, and **nothing of the arena** |
//! | God | staff | `@arena/status` running then reset, `@unit/show` on the beaten tank, the ledger, the result document, the money |
//!
//! # What the cockpit shows each pilot about the shooting
//!
//! Doc 16 §5 lists *armor and internals per section as current over max* in the
//! status block and *`Notice` events for transitions and refusals* beside it. Cut
//! W.3 shipped neither — the frame carried no damage at all, so neither pilot was
//! told about a hit and this file recorded the gap honestly — and **16b cut F.2
//! closed it**. So the story now asserts what each side is told, and the split is
//! the point:
//!
//! * **Ada**, the shooter, is told that each round landed, named by the opaque
//!   contact her own platform holds, which section of the target went, and —
//!   when that was the hull — that the target was destroyed. She is
//!   told nothing of which section she struck or what the target had left: a
//!   gunner sees the round land, not the target's internals.
//! * **Bo**, the victim, is told that it was hit, which of *its own* sections and
//!   facets took it and what is left there, and that its hull (or, since 16b
//!   cut AP.2, its turret) is gone. It is told
//!   nothing whatever about Ada — no object number, no name, no contact id, no
//!   bearing, no side, no weapon — because where a round came from is a thing only
//!   a sensor may tell.
//!
//! "The loser sees its tank beaten" is therefore asserted in the cockpit as
//! well as on the surfaces that always carried it: the centre's own "Match 1 is
//! over (red)" line, the hatch that opens, the result document's row for the blue
//! tank — `destroyed` when the hull went, `intact` when the turret went (16b cut
//! AP.2: a disarmed tank still drives, and its outcome word is untouched) — and
//! `@unit/show`'s `section hull: wreck` on the checked-in tank, or
//! `section turret: destroyed` when the turret went first (see below).
//!
//! # Why the two accounts are seeded before the world task spawns
//!
//! For speed, and nothing else. `mush_server::accounts_identity::hydrate` builds
//! the character → account map at boot and, since **16b cut F.4**, the world task
//! keeps it current: a player who types `create` at the welcome screen can claim a
//! simulator slot straight away, which
//! `crates/mushd/tests/front_door.rs` proves over real SSH and
//! `crates/mush-gateway/tests/front_door_split.rs` proves through the split. This
//! lens seeds its two accounts in the store before the engine is built because
//! that is several seconds cheaper than registering them through the welcome
//! screen, and what it is here to assert is the match, not the front door. The
//! pilots still arrive through the front door, build their sheets with
//! `+chargen`, and **walk** `ooc` → `simulator` → `red`/`blue` rather than being
//! teleported.
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
//! only the hull and the turret, and before **16b cut AP.2** a turret loss
//! disabled nothing, so a frontal duel could end exactly one way: the blue tank
//! blew up. It costs about twice the rounds (400 cm of glacis against 150 cm of
//! engine deck).
//!
//! **Since AP.2 the duel ends at the hull or at the turret.** A side that came
//! in armed and has no gun left is out (16c `B-87`, `Q57`; doc 16 §3.8), and Bo's
//! side is one tank whose one gun is in its turret (350 armour over 150
//! internals, `remove_weapons`). Which section the k-th hit finds is the seeded
//! facet draw's, keyed on the shot number, so which section goes first is a
//! matter of which shots hit — timing — and nothing in a stock duel can make the
//! hull go first. So the test reads which ending it got off the loser's own
//! transcript (exactly one of the two sentences: the match is over at the first
//! of them) and asserts the elimination and **whichever section went** on every
//! surface — the two cockpits, `@unit/show`, the black box, the result document
//! and the debrief. The end is detected by `OVER` either way, so the fight's
//! flow does not change.
//!
//! # Why the two tanks are re-posed before `SIM START`
//!
//! The shipped `arenas/proving_ground.json` musters red at cell column 6 and blue at
//! column 249, both on row 97, the east-west road (16b cut F.4): 2.4 km apart
//! against a `visual_mk1` that reaches 600 m, which is a
//! minute of wall time spent proving that a tank can cross a map. So staff re-poses
//! the two slot-1 tanks **62 cells apart** on the east-west road — just outside the
//! sensor's own 60 cells at every signature factor, so the first `CONTACTS` is
//! honestly empty and the detection this test waits for is earned by driving.
//! `arena_def::opening_pose` (16b cut T.3) is what carries the placement into the
//! match. `observer_rule.rs` uses the same device for the same reason.

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

use support::{Term, names_side, play_client, scratch_dir, strip_ansi};

const GM_PASSWORD: &str = "gm-secret-1";
const PILOT_PASSWORD: &str = "pilot-secret-1";

/// The world's transient authoritative refusal: a commit is in flight.
const COMMIT_PENDING: &str = "please retry in a moment";
/// `BOARD`'s one transient: the control-lease mirror is a 1 Hz projection.
const CONTROL_UNKNOWN: &str = "control state unknown";
/// How many times a line is re-sent on a transient before the test gives up.
const RETRY_BUDGET: usize = 40;

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
/// The loser's own line when its hull goes (`FrameNotice::SectionLost` with the
/// hull's `destroy_unit`).
const HULL_LOST: &str = "The hull is destroyed: the tank is finished.";
/// **16b cut AP.2**: the loser's own line when its turret goes
/// (`FrameNotice::SectionLost` with the turret's `remove_weapons`, in
/// `consequence_words`' words) — which, Bo's side having no other gun, ends the
/// match at that hit. **17b cut M3.6**: and the turret's `destroy_core`, the
/// line core it carries (was "The turret is destroyed: its weapons are gone.").
const TURRET_LOST: &str = "The turret is destroyed: its weapons are gone; its core is gone.";
/// What `CONTACTS` says when the platform holds nothing.
const NO_CONTACTS: &str = "No contacts.";
/// The parser's refusal, which is the sentinel's answer.
const HUH: &str = "Huh?";
/// The one line the eleven cockpit selectors refuse with once the pilot is no
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
    mods.extend(modules::sheet_modules(onboarding, &Default::default()));
    mods.push(modules::tacsim_module_with(tacsim_config(), true));
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
        // Always consume this attempt's sentinel before retrying. A delayed
        // refusal must not terminate the next command's reply early.
        term.expect_after(mark, HUH).await;
        let reply = strip_ansi(&term.seen[mark..]);
        if !reply.contains(COMMIT_PENDING) && !reply.contains(CONTROL_UNKNOWN) {
            return reply;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("`{line}` never produced a whole answer")
}

/// **`SIM REPORT`, read whole** (16b cut V.3) — `observer_rule.rs`' device.
///
/// The report document rides the result's batch and the store's read lane is not
/// ordered against its write lane, so an early read answers `still being
/// written`; the wait asks again rather than sleeping. Once the report's end line
/// has arrived, a sentinel read to its refusal brings in the rest.
async fn sim_report(term: &mut Term, line: &str) -> String {
    const ENDS: [&str; 3] = [
        "the match was aborted.",
        "the match ended: ",
        "the match was voided.",
    ];
    let mut report = String::new();
    wait_until(STEP_TIMEOUT, "the report to be read whole", async || {
        let mark = term.mark();
        term.send_line(line).await;
        let mut needles: Vec<&str> = ENDS.to_vec();
        needles.extend(["still being written", "already being read", COMMIT_PENDING]);
        if term.expect_one_of_after(mark, &needles).await >= ENDS.len() {
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
/// breath after the last shot is answered by the eleven selectors' one refusal
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
/// string in chunk order and splitting it — **16b cut AF.3**: in the order the
/// tape counts (`tacsim::arena_bridge::tape_in_order`), so a resumed match's
/// abandoned tail is never read as the match. Behaviour-neutral on a tape no
/// restart forked, which is every tape this story writes.
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

/// **The black box's raw bytes** (16b cut M.1, `B-48`): every `lines` string of
/// every kept chunk, in chunk order, concatenated exactly — the same walk
/// [`ledger_events`] makes, with no parse, no trim and no newline added. Its
/// SHA-256 is what the result document's digest must be (16b cut AF.3: over
/// the tape as `tape_in_order` counts it, which is the bytes that left the
/// arena whether or not a resume forked the log).
async fn ledger_bytes(store: &Arc<dyn ObjectStore>, match_id: u64) -> String {
    let kept = kept_tape(store, match_id).await;
    let mut bytes = String::new();
    for value in kept {
        let mut lines: Vec<String> = Vec::new();
        collect_lines(&value, &mut lines);
        for blob in lines {
            bytes.push_str(&blob);
        }
    }
    bytes
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
/// Point a tank down the road and open it up.
async fn drive(term: &mut Term, heading: u32) {
    wait_until(STEP_TIMEOUT, "the tank to accept its heading", async || {
        let mark = term.mark();
        term.send_line(&format!("HEADING {heading}")).await;
        term.expect_one_of_after(
            mark,
            &[
                &format!("Steering to {heading}"),
                "Power not online",
                // 16b cut F.1: the same refusal while the startup is running.
                "Power is still coming up",
                COMMIT_PENDING,
                CONTROL_UNKNOWN,
            ],
        )
        .await
            == 0
    })
    .await;
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
        // 16b cut F.2: a pilot's notices are addressed to one actor and reach
        // nobody else, on any surface.
        "Hit on",
        "Miss on",
        "is destroyed",
        "armour",
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
    // Doc 18 cut IC3: the two port domains stand beside it (was "No domains."),
    // and the muster's is still absent. Read WHOLE (to the sentinel), so the
    // absence holds whatever order the domain ids come in.
    let seen = ask_whole(&mut god, "@domain").await;
    for place in ["Stickney Yards", "Ceres Freeport"] {
        assert!(
            seen.contains(&format!("host {place} presents in Visiting Berth Gallery")),
            "{place}:\n{seen}"
        );
    }
    assert!(
        !seen.contains("host The Proving Ground"),
        "`@domain` still lists the muster:\n{seen}"
    );

    // -----------------------------------------------------------------
    // (5) Drive toward each other until the sensor pass finds something.
    // -----------------------------------------------------------------
    drive(&mut ada, 90).await;
    drive(&mut bo, 270).await;

    let mut held = String::new();
    wait_until(
        STEP_TIMEOUT,
        "the driving tanks to detect one another",
        async || {
            let mark = ada.mark();
            ada.send_line("CONTACTS").await;
            if ada
                .expect_one_of_after(
                    mark,
                    // **16b cut Q.1**: `(rel ` and not `bearing`, because the
                    // acquisition notice now says `New contact X1: bearing 090°,
                    // effective.` and would satisfy a `bearing` needle before the
                    // `CONTACTS` row this loop is waiting for arrives. Only a row
                    // carries the relative bearing.
                    &["(rel ", NO_CONTACTS, COMMIT_PENDING, CONTROL_UNKNOWN],
                )
                .await
                == 0
            {
                held = strip_ansi(&ada.seen[mark..]);
                return true;
            }
            false
        },
    )
    .await;
    assert!(
        !held.is_empty(),
        "the two tanks never detected each other:\n{}",
        tail(&strip_ansi(&ada.seen))
    );
    let contact = designator_of(&held)
        .unwrap_or_else(|| panic!("a contact designator in:\n{held}"))
        .to_owned();
    names_nothing(&held, red, blue, "the red pilot's held `CONTACTS`");

    // The notice came first, unasked, while she was driving — doc 16 §3.6's
    // `acquired` — and then, after `sensors.classify_after_ms`, the side.
    assert!(
        strip_ansi(&ada.seen).contains(&format!("New contact {contact}")),
        "the acquisition notice reached the pilot:\n{held}"
    );
    // The *row* has to be re-read until it is renamed, not merely until the
    // answer says `blue`: the `identified` notice arrives on its own, and an
    // answer window that caught it may still carry the `X<n>` row rendered
    // before it (`observer_rule.rs`' Q.1 wait, for the same race).
    let mut classified = String::new();
    wait_until(
        STEP_TIMEOUT,
        "the `CONTACTS` row to read the classified name",
        async || {
            classified = ask_whole(&mut ada, "CONTACTS").await;
            classified.contains("blue")
                && designator_of(&classified).is_some_and(|row| row != contact)
        },
    )
    .await;
    names_nothing(&classified, red, blue, "the classified `CONTACTS`");
    assert!(
        strip_ansi(&ada.seen).contains(&format!("Contact {contact} identified")),
        "and the classification notice did too:\n{classified}"
    );
    // **16b cut Q.1**: the rename. Once the return has been told apart it answers
    // to the target's public callsign, and that is the name every line about it
    // says from here on; the provisional number stays typable, which is what the
    // `LOCK {contact}` below exercises.
    let named = designator_of(&classified)
        .unwrap_or_else(|| panic!("a contact designator in:\n{classified}"));
    assert_ne!(
        named, contact,
        "the classified row still reads as the provisional number:\n{classified}"
    );

    // -----------------------------------------------------------------
    // (6) Close and halt, nose to nose.
    // -----------------------------------------------------------------
    wait_until(
        STEP_TIMEOUT,
        "the tanks to close to the chosen fight range",
        async || easting_m(&mut ada).await >= STOP_AT_M,
    )
    .await;
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
    // The asynchronous lock notice can arrive beside an older weapon reply.
    // Wait for the command's weapon telemetry, not that out-of-band notice.
    let locked = poll_whole(&mut ada, "WEAPONSTATUS", "band 1 of 2").await;
    assert!(locked.contains(&named), "the lock is the contact: {locked}");
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
        // else; once the check-in has opened the hatch the eleven selectors refuse
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

    // **16b cut F.2**: the loser was told about the rounds that killed it. It
    // names its **own** tank — `STATUS` prints the platform a pilot is sitting in,
    // and a hit names the section and facet the round found — and nothing of the
    // other side: not the red tank, not its dbref, not the shot, not the lock.
    let loser = strip_ansi(&bo.seen);
    assert!(
        loser.contains("Hit on the "),
        "the loser was told nothing about being shot:\n{}",
        tail(&loser)
    );
    // **16b cut AP.2**: the duel ends at the hull or at the turret (the module
    // head's "Why the fight is frontal"), and the loser's own transcript says
    // which — exactly one of the two, because the match is over at the first.
    let hull_went = loser.contains(HULL_LOST);
    let turret_went = loser.contains(TURRET_LOST);
    assert!(
        hull_went != turret_went,
        "exactly one of the hull and the turret went (hull {hull_went}, turret {turret_went}):\n{}",
        tail(&loser)
    );
    eprintln!(
        "arena_story: the duel ended at the {}",
        if hull_went { "hull" } else { "turret" }
    );
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
    // Line by line, on the sentences this cut added: nothing in them identifies or
    // locates the shooter.
    for line in loser.lines().filter(|line| {
        line.starts_with("Hit on the ")
            || line.starts_with("The hull is")
            || line.starts_with("The turret is")
    }) {
        for leak in [contact.as_str(), "bearing", "cannon", "°"] {
            assert!(
                !line.contains(leak),
                "a hit notice carried {leak:?} about the shooter: {line}"
            );
        }
        // 16b cut F.3: the side is forbidden as a **word**, so a future sentence
        // that happens to spell it (`cleared`) is not a false failure.
        assert!(
            !names_side(line, "red"),
            "a hit notice named the shooter's side: {line}"
        );
    }
    assert!(
        loser.contains(OVER),
        "but it was told which side won:\n{}",
        tail(&loser)
    );

    // The winner saw her rounds land and saw the kill (or the turret go), and
    // saw nothing of what the blue tank had left: the two halves of the same shot never cross.
    let winner = strip_ansi(&ada.seen);
    assert!(
        // 16b cut Q.1: by the shot the target has been told apart, so the
        // shooter's line names its callsign. **16b cut Q.2**: and the contact is
        // classified, so the line also names the section the round found — which
        // section is the facet draw's, so the prefix is what is pinned here and
        // `sim_weapons.rs` is where the whole sentence's shape is.
        winner.contains(&format!("Hit on {named}: ")),
        "the shooter was never told a round landed:\n{}",
        tail(&winner)
    );
    // **16b cut Q.2**: and told when a section of the target came apart — the
    // owner's ask of 2026-09-19. The blue tank is beaten by a hull loss or (16b
    // cut AP.2) a turret loss, so that section is the one the shooter is certain
    // to watch go.
    let section = if hull_went { "hull" } else { "turret" };
    assert!(
        winner.contains(&format!("{named}'s {section} is destroyed.")),
        "the shooter was never told a section of the target went:\n{}",
        tail(&winner)
    );
    // The gunner's line and the victim's are not the same sentence: the victim's
    // says what it cost the tank, and the shooter's cannot.
    assert!(
        !winner.contains("The hull is destroyed") && !winner.contains("The turret is destroyed"),
        "the shooter was shown the victim's own consequence list:\n{}",
        tail(&winner)
    );
    // The kill line is the hull's alone: a disarmed tank still drives.
    assert_eq!(
        winner.contains(&format!("{named} destroyed.")),
        hull_went,
        "the target is destroyed exactly when its hull went:\n{}",
        tail(&winner)
    );
    assert!(
        !winner.contains("Hit on the "),
        "the shooter was shown the target's own damage:\n{}",
        tail(&winner)
    );

    // -----------------------------------------------------------------
    // (9) Staff: the wreck, the survivor, and the ammunition.
    // -----------------------------------------------------------------
    let wreck = ask_whole(&mut god, &format!("@unit/show #{blue}")).await;
    // 16b cut AP.2: a wreck, or its turret destroyed and its hull whole.
    let beaten = if hull_went {
        wreck.contains("section hull: wreck")
    } else {
        wreck.contains("section turret: destroyed") && !wreck.contains("section hull: wreck")
    };
    assert!(
        beaten,
        "the loser's tank came back a wreck or disarmed (doc 16 §3.3, §3.8): {wreck}"
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
        // 16b cut AP.2: the section that went — the hull, whose loss also writes
        // `unit_destroyed` (asserted below), or the turret — precedes the end.
        "section_destroyed",
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
    // **17b cut M3.6** (doc 17 §13): the real rig's tape opens with the
    // sorties — one `mind_sealed` per tank, in unit-id order, right after
    // `match_started`, at 0 under the opening hash, each naming its pilot as
    // the warrant principal — and closes them: the beaten tank's at its
    // destruction (`destroyed`, right after `unit_destroyed`) when its hull
    // went, every sortie still open (`match_end`, in unit-id order) right
    // before `match_ended`, which is the tape's last line.
    let kind_at = |at: usize| {
        events
            .get(at)
            .and_then(|event| event.get("kind"))
            .and_then(Value::as_str)
    };
    let (low, high) = (red.min(blue), red.max(blue));
    assert_eq!(kind_at(0), Some("match_started"), "{:?}", kinds(&events));
    for (at, unit) in [(1_usize, low), (2, high)] {
        let sealed = &events[at];
        assert_eq!(kind_at(at), Some("mind_sealed"), "{:?}", kinds(&events));
        assert_eq!(
            sealed.get("unit").and_then(Value::as_u64),
            Some(u64::from(unit)),
            "{sealed}"
        );
        assert_eq!(sealed.get("t_ms").and_then(Value::as_i64), Some(0));
        assert_eq!(sealed.get("hash"), events[0].get("hash"), "{sealed}");
        assert_eq!(
            sealed.get("mind").and_then(Value::as_str),
            Some("Standard Doctrine")
        );
        assert_eq!(
            sealed.pointer("/principal/kind").and_then(Value::as_str),
            Some("character"),
            "a seated pilot is the warrant principal: {sealed}"
        );
    }
    assert_eq!(
        kinds(&events)
            .iter()
            .filter(|kind| **kind == "mind_sealed")
            .count(),
        2,
        "one seal line per tank"
    );
    let last = events.len() - 1;
    assert_eq!(
        kind_at(last),
        Some("match_ended"),
        "the end is the last line"
    );
    let closes: Vec<&Value> = events
        .iter()
        .filter(|event| event.get("kind").and_then(Value::as_str) == Some("sortie_closed"))
        .collect();
    assert_eq!(closes.len(), 2, "one close per sortie: {closes:?}");
    let at_end: Vec<u32> = if hull_went {
        let gone = first(&events, "unit_destroyed");
        let close = &events[gone + 1];
        assert_eq!(kind_at(gone + 1), Some("sortie_closed"), "{close}");
        assert_eq!(close.get("word").and_then(Value::as_str), Some("destroyed"));
        assert_eq!(
            close.get("unit").and_then(Value::as_u64),
            Some(u64::from(blue))
        );
        vec![red]
    } else {
        vec![low, high]
    };
    for (k, unit) in at_end.iter().enumerate() {
        let at = last - at_end.len() + k;
        let close = &events[at];
        assert_eq!(kind_at(at), Some("sortie_closed"), "{:?}", kinds(&events));
        assert_eq!(close.get("word").and_then(Value::as_str), Some("match_end"));
        assert_eq!(
            close.get("unit").and_then(Value::as_u64),
            Some(u64::from(*unit))
        );
        assert_eq!(close.get("hash"), events[last].get("hash"), "{close}");
        assert_eq!(close.get("t_ms"), events[last].get("t_ms"), "{close}");
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
    let lost = events
        .iter()
        .find(|event| {
            event.get("kind").and_then(Value::as_str) == Some("section_destroyed")
                && event.get("section").and_then(Value::as_str) == Some(section)
        })
        .unwrap_or_else(|| {
            panic!(
                "a section_destroyed line for the {section}: {:?}",
                kinds(&events)
            )
        });
    assert_eq!(
        lost.get("unit").and_then(Value::as_u64),
        Some(u64::from(blue)),
        "and which tank lost it: {lost}"
    );
    // 17b cut M3.6 (D-UM-34): a lost turret carries the line core with it, so
    // its line names the shipped `destroy_core` after `remove_weapons`.
    if !hull_went {
        assert_eq!(
            lost.get("consequences"),
            Some(&serde_json::json!(["remove_weapons", "destroy_core"])),
            "{lost}"
        );
    }
    let destroyed = events
        .iter()
        .find(|event| event.get("kind").and_then(Value::as_str) == Some("unit_destroyed"));
    if hull_went {
        let destroyed = destroyed.expect("a unit_destroyed line");
        assert_eq!(
            destroyed.get("unit").and_then(Value::as_u64),
            Some(u64::from(blue)),
            "and which tank went up: {destroyed}"
        );
        // The hull's own line comes first, then the tank going up, then the
        // end — the order the old list pinned with `unit_destroyed` in it.
        assert!(
            first(&events, "section_destroyed") <= first(&events, "unit_destroyed"),
            "the tank went up before its hull was destroyed: {:?}",
            kinds(&events)
        );
        assert!(
            first(&events, "unit_destroyed") <= first(&events, "match_ended"),
            "the tank went up before the end: {:?}",
            kinds(&events)
        );
    } else {
        assert!(
            destroyed.is_none(),
            "a disarmed tank is not destroyed: {:?}",
            kinds(&events)
        );
    }
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
    // **16b cut M.1** (`B-48`): and it is the digest of the log. Nothing asserted
    // this before, and under backpressure the result could name bytes the log
    // never got; now the arena task waits for every chunk before its `Ended` and
    // reports the digest of what left it.
    let logged = {
        use sha2::{Digest as _, Sha256};
        use std::fmt::Write as _;
        let raw = ledger_bytes(&server.store, 1).await;
        let mut hex = String::with_capacity(64);
        for byte in Sha256::digest(raw.as_bytes()) {
            let _ = write!(hex, "{byte:02x}");
        }
        hex
    };
    assert_eq!(
        digest, logged,
        "the result's digest is SHA-256 over the log's own lines, in chunk order"
    );
    // **16b cut M.3** (16c `T-07`, and `T-12`'s proof on the reachable path):
    // the result carries the seed the arena drew with, and the record is enough
    // to re-derive a draw. The label is read back off the line itself — a
    // `classified` line carries no pass sequence of its own, so its draw's label
    // string is the one place it is written — and the value must be the seed's.
    let seed = result
        .get("seed")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("the result carries the seed: {result}"));
    let rederive = |line: &Value, kind: &str| {
        let draws = line
            .get("draws")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("a `{kind}` line with its draws: {line}"));
        let (text, value) = draws
            .iter()
            .filter_map(|entry| Some((entry.get(0)?.as_str()?.to_owned(), entry.get(1)?.as_u64()?)))
            .find(|(text, _)| text.starts_with(kind))
            .unwrap_or_else(|| panic!("a `{kind}:` draw on {line}"));
        let label = tacsim::arena::rng::DrawLabel::decode(&text)
            .unwrap_or_else(|| panic!("the label decodes: {text}"));
        assert_eq!(label.encode(), text);
        assert_eq!(
            tacsim::arena::rng::draw(seed, &label),
            value,
            "the result's seed re-derives `{text}` off the line: the record's seed is not \
             the one the arena drew with"
        );
    };
    let classified = events
        .iter()
        .find(|event| {
            event.get("kind").and_then(Value::as_str) == Some("classified")
                && event
                    .get("draws")
                    .and_then(Value::as_array)
                    .is_some_and(|draws| !draws.is_empty())
        })
        .unwrap_or_else(|| {
            panic!(
                "a classification by a platform's own sight: {:?}",
                kinds(&events)
            )
        });
    rederive(classified, "classify");
    // And the section a round found: the `hit` line's second draw.
    let hit = events
        .iter()
        .find(|event| event.get("kind").and_then(Value::as_str) == Some("hit"))
        .unwrap_or_else(|| panic!("a hit line: {:?}", kinds(&events)));
    rederive(hit, "facet");
    assert_eq!(
        hit.get("draws")
            .and_then(Value::as_array)
            .map(|draws| draws.len()),
        Some(2),
        "a hit carries its shot draw and its facet draw: {hit}"
    );
    let rows = result
        .get("units")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("a per-unit row each: {result}"));
    let row_of = |unit: u32| {
        rows.iter()
            .find(|row| row.get("unit").and_then(Value::as_u64) == Some(u64::from(unit)))
            .unwrap_or_else(|| panic!("no row for #{unit}: {result}"))
    };
    // 16b cut AP.2: `UnitOutcome` is untouched, so a tank that lost its turret
    // and still drives comes back `intact` on its row — the match's end is the
    // side's, and the record's `reason` above says it.
    assert_eq!(
        row_of(blue).get("outcome").and_then(Value::as_str),
        Some(if hull_went { "destroyed" } else { "intact" }),
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
    // **16b cut V.2**: every unit row and every pilot row names its unit by the
    // callsign the arena resolved at check-out — red slot 1 `RA`, blue slot 1
    // `BA` — and the ledger object counts the tape's chunks.
    assert_eq!(
        row_of(red).get("callsign").and_then(Value::as_str),
        Some("RA"),
        "{result}"
    );
    assert_eq!(
        row_of(blue).get("callsign").and_then(Value::as_str),
        Some("BA"),
        "{result}"
    );
    for (team, pilot, callsign) in [("red", ADA, "RA"), ("blue", BO, "BA")] {
        let row = result
            .get("teams")
            .and_then(Value::as_array)
            .and_then(|teams| {
                teams
                    .iter()
                    .find(|roster| roster.get("team").and_then(Value::as_str) == Some(team))
            })
            .and_then(|roster| roster.get("pilots").and_then(Value::as_array))
            .and_then(|rows| rows.first())
            .unwrap_or_else(|| panic!("the {team} roster carries its pilot: {result}"));
        assert_eq!(
            row.get("pilot").and_then(Value::as_u64),
            Some(u64::from(pilot.0)),
            "{result}"
        );
        assert_eq!(
            row.get("callsign").and_then(Value::as_str),
            Some(callsign),
            "{result}"
        );
    }
    assert!(
        result
            .pointer("/ledger/chunks")
            .and_then(Value::as_u64)
            .is_some_and(|chunks| chunks >= 1),
        "the result counts the tape it names: {result}"
    );
    // **16b cut V.3**: and it names the report documents written with it — one
    // per pilot's tank, in id order.
    let mut flown = vec![u64::from(red), u64::from(blue)];
    flown.sort_unstable();
    assert_eq!(
        result.get("reports").cloned(),
        Some(serde_json::json!(flown)),
        "{result}"
    );

    // -----------------------------------------------------------------
    // (11b) The winner's debrief (16b cut V.3; 16c N2, Q37).
    // -----------------------------------------------------------------
    // Ada reads her own match: the public outcome at its head, her own shots,
    // and the kill (or the turret) her own round made — and a match the centre
    // never held is refused in one sentence.
    let report = sim_report(&mut ada, "SIM REPORT").await;
    let flat = report.split_whitespace().collect::<Vec<_>>().join(" ");
    // 16b cut AP.2: the kill line is the hull's; a turret ending is the shot's
    // own clause instead.
    let finished = if hull_went {
        "destroyed by your round.".to_owned()
    } else {
        "'s turret destroyed".to_owned()
    };
    for want in [
        "red won (blue eliminated)",
        "You flew RA: ",
        "cannon at ",
        finished.as_str(),
        "the match ended: blue eliminated.",
    ] {
        assert!(flat.contains(want), "{want:?} missing from:\n{report}");
    }
    let said = ask(&mut ada, "SIM REPORT 9", "not one of them").await;
    assert!(
        said.contains("The centre remembers its last 20 matches; match 9 is not one of them."),
        "{said}"
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
