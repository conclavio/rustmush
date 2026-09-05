//! The M9 headline demo as one scripted end-to-end journey over real SSH:
//! **new player signs up → guided chargen → approved → plays**.
//!
//! Boots the whole server (world task, session layer, store-backed accounts,
//! IP guard, the built-in modules + worldgen + npc + the WoD ruleset with
//! `chargen_required` on) on an ephemeral port — production wiring except for
//! port, paths, and an in-memory store — then drives real russh clients:
//!
//! - A brand-new player `create`s an account over the welcome screen, is
//!   placed in the restricted chargen area, and is **dropped into the guided
//!   wizard** on connect (the transport-agnostic `Connected` hook — no SSH
//!   code path special-cases it).
//! - They build a full vampire through the wizard's steps and `submit`, which
//!   files a `wod-chargen` approval job and flips the sheet to Pending.
//! - While Pending they are **blocked** from a live-grid command (`+roll`) by
//!   the onboarding gate.
//! - Staff (God) `+job/approve` the job; the gate lifts, the character is
//!   **moved to the live start room** and re-rendered, and the same `+roll`
//!   now works.
//!
//! A second journey proves the back-compat default (`chargen_required = false`):
//! a new player is **not** forced into a wizard, lands in Limbo, and the
//! classic M6 `+chargen` opens normally.
//!
//! No real LLM is ever called: the worldgen/npc modules are wired to a
//! `MockProvider`-backed [`LlmService`] with the global kill switch engaged,
//! and nothing in the onboarding flow reaches the LLM anyway.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_core::{Engine, GOD, ObjKind};
use mush_llm::{LlmProvider, LlmService, MockProvider};
use mush_session::{SessionConfig, SessionManager};
use russh::ChannelMsg;

use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use mushd::accounts::{Accounts, hash_password};
use mushd::ipguard::IpGuard;
use mushd::modules;
use mushd::persist::load_or_seed_world;
use mushd::ssh_bridge::{SshBridge, SshListener, start_listener};
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::world_task::{self, PersistHandles, ServerDeps};

const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const GM_PASSWORD: &str = "gm-secret-1";

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mushd-onboarding-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A running server plus the handles that keep it alive for the test's life.
struct Server {
    addr: SocketAddr,
    dir: PathBuf,
    _listener: SshListener,
    _handles: PersistHandles,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Boot the full production module stack over SSH with `chargen_required` set
/// as requested. The world is seeded fresh (Limbo `#0`, God `#1`), then two
/// rooms are created — a chargen area (`#2`) and the live start room (`#3`) —
/// and the WoD onboarding config points at them. The `boss` account is linked
/// to God so staff can approve over the same front door.
async fn boot(tag: &str, chargen_required: bool) -> Server {
    let dir = scratch_dir(tag);
    std::fs::create_dir_all(&dir).unwrap();

    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    store
        .create_account(AccountRec {
            name: "boss".to_string(),
            argon2_hash: hash_password(GM_PASSWORD).unwrap(),
            created: 0,
            characters: vec![CharacterRef {
                id: GOD,
                name: "God".to_string(),
            }],
            ssh_keys: vec![],
        })
        .await
        .unwrap();

    let mut world = load_or_seed_world(&*store).await.expect("boot world");
    // The chargen area (#2) and the live start room (#3). Created directly on
    // the loaded world before the task spawns — RAM is all the onboarding
    // seams read (this is an in-memory test; the gated persistence twin covers
    // durability).
    let chargen_area = world
        .create(ObjKind::Room, "Chargen Nexus", GOD, None)
        .expect("chargen area");
    let commons = world
        .create(ObjKind::Room, "The Commons", GOD, None)
        .expect("live start room");
    assert_eq!(
        chargen_area,
        mush_core::ObjId(2),
        "chargen-area id assumption"
    );
    assert_eq!(commons, mush_core::ObjId(3), "start-room id assumption");

    let onboarding = ruleset_wod::OnboardingConfig {
        chargen_required,
        start_room: commons,
        start_room_unapproved: chargen_area,
    };

    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).unwrap();

    // A MockProvider-backed LLM with the global kill switch engaged: the
    // worldgen/npc modules register cleanly and never make a real call.
    let mock: Arc<dyn LlmProvider> = Arc::new(MockProvider::new("openrouter"));
    let llm = Arc::new(
        LlmService::builder()
            .provider("openrouter", mock)
            .global_enabled(false)
            .build(),
    );

    let mut mods = modules::server_modules();
    mods.push(modules::worldgen_module(Arc::clone(&llm)));
    let grounding = modules::npc_agent_adapter(&["wod".to_string()])
        .map(|a| a as Arc<dyn mush_npc::AgentGrounding>);
    mods.push(modules::npc_module(grounding));
    mods.extend(
        modules::ruleset_modules(
            &["wod".to_string()],
            &onboarding,
            &ruleset_wod::WodEconomyConfig::default(),
            &ruleset_shadowrun::SrEconomyConfig::default(),
        )
        .expect("wod ruleset"),
    );

    let mut engine = Engine::with_modules(world, registry, mods).expect("modules");
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
        "127.0.0.1:0".parse().unwrap(),
        &dir,
        "Welcome to the onboarding MUSH!".to_string(),
        bridge,
    )
    .await
    .expect("ssh listener");
    let addr = listener.addr();

    Server {
        addr,
        dir,
        _listener: listener,
        _handles: handles,
    }
}

struct AcceptAll;

impl russh::client::Handler for AcceptAll {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// Drop ANSI CSI sequences so assertions match what a player reads.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for f in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&f) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// One player's terminal: an SSH channel plus everything it has printed.
struct Term {
    channel: russh::Channel<russh::client::Msg>,
    seen: String,
}

impl Term {
    fn mark(&self) -> usize {
        self.seen.len()
    }

    async fn expect(&mut self, needle: &str) {
        self.expect_after(0, needle).await;
    }

    async fn expect_after(&mut self, mark: usize, needle: &str) {
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        while !strip_ansi(&self.seen[mark..]).contains(needle) {
            let msg = tokio::time::timeout_at(deadline, self.channel.wait())
                .await
                .unwrap_or_else(|_| {
                    panic!("timeout waiting for {needle:?}; seen: {:?}", self.seen)
                });
            match msg {
                Some(ChannelMsg::Data { data }) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Some(_) => {}
                None => panic!(
                    "channel closed waiting for {needle:?}; seen: {:?}",
                    self.seen
                ),
            }
        }
    }

    /// Assert `needle` has NOT appeared since `mark` after the pipeline has
    /// drained — a round-trip barrier line proves the command was processed.
    fn not_seen_since(&self, mark: usize, needle: &str) -> bool {
        !strip_ansi(&self.seen[mark..]).contains(needle)
    }

    async fn send_line(&mut self, line: &str) {
        let bytes = format!("{line}\r").into_bytes();
        self.channel.data(&bytes[..]).await.expect("send line");
    }
}

/// `ssh play@addr` with a PTY + shell, landing on the welcome screen.
async fn play_client(addr: SocketAddr) -> (russh::client::Handle<AcceptAll>, Term) {
    let config = Arc::new(russh::client::Config::default());
    let mut handle = russh::client::connect(config, addr, AcceptAll)
        .await
        .expect("connect");
    let auth = handle.authenticate_none("play").await.expect("auth-none");
    assert!(auth.success(), "play must be accepted with no auth");
    let channel = handle.channel_open_session().await.expect("open channel");
    channel
        .request_pty(true, "xterm-256color", 80, 24, 0, 0, &[])
        .await
        .expect("pty");
    channel.request_shell(true).await.expect("shell");
    (
        handle,
        Term {
            channel,
            seen: String::new(),
        },
    )
}

/// Drive the guided wizard from the Template step through `submit`, building a
/// balanced vampire (the same allocation the ruleset's in-process onboarding
/// test uses). Returns the wizard-reported `wod-chargen` job id, parsed from
/// the submit confirmation.
async fn send_and_expect(t: &mut Term, line: &str, needle: &str) {
    let mark = t.mark();
    t.send_line(line).await;
    t.expect_after(mark, needle).await;
}

async fn wizard_vampire_and_submit(t: &mut Term) -> u64 {
    // Template.
    send_and_expect(t, "vampire", "Template set to vampire").await;

    // Concept + attributes + abilities + backgrounds (balanced).
    for line in [
        "next",
        "concept=Jaded Detective",
        "next",
        "physical=7 social=5 mental=3",
        "next",
        "strength=4",
        "dexterity=4",
        "stamina=2",
        "charisma=3",
        "manipulation=3",
        "appearance=2",
        "perception=2",
        "intelligence=2",
        "wits=2",
        "next",
        "talents=13 skills=9 knowledges=5",
        "next",
        "brawl=3",
        "alertness=3",
        "subterfuge=3",
        "intimidation=3",
        "streetwise=1",
        "firearms=3",
        "drive=3",
        "stealth=3",
        "investigation=3",
        "occult=2",
        "next",
        "Resources=3",
        "Allies=2",
    ] {
        let mark = t.mark();
        t.send_line(line).await;
        let acknowledgement = match line {
            "next" => "--- ".to_string(),
            "concept=Jaded Detective" => "concept set.".to_string(),
            "physical=7 social=5 mental=3" => "Attribute priorities set.".to_string(),
            "talents=13 skills=9 knowledges=5" => "Ability priorities set.".to_string(),
            "Resources=3" | "Allies=2" => {
                let (name, value) = line.split_once('=').expect("background=value");
                format!("Background {name} = {value}.")
            }
            _ => {
                let (field, value) = line.split_once('=').expect("field=value");
                format!("{field} = {value}.")
            }
        };
        t.expect_after(mark, &acknowledgement).await;
    }
    // Splat: clan + a 3-dot Discipline pool (the budget names the pool).
    send_and_expect(t, "next", "--- Template Traits ---").await;
    send_and_expect(t, "clan=brujah", "clan set.").await;
    send_and_expect(t, "Celerity=2", "Celerity = 2.").await;
    send_and_expect(t, "Potence=1", "Potence = 1.").await;

    // Virtues.
    send_and_expect(t, "next", "--- Virtues ---").await;
    send_and_expect(t, "conscience=4", "conscience = 4.").await;
    send_and_expect(t, "self_control=3", "self_control = 3.").await;
    send_and_expect(t, "courage=3", "courage = 3.").await;
    // Freebies (balanced — nothing to spend) → Review → submit.
    send_and_expect(t, "next", "--- Freebies ---").await;
    send_and_expect(t, "next", "--- Review & Submit ---").await;
    send_and_expect(t, "submit", "Submitted for approval as job").await;

    // Parse the job id out of the confirmation line.
    let hay = strip_ansi(&t.seen);
    let idx = hay
        .find("Submitted for approval as job ")
        .expect("submit confirmation present");
    let tail = &hay[idx + "Submitted for approval as job ".len()..];
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().expect("job id parses")
}

#[tokio::test(flavor = "multi_thread")]
async fn new_player_signs_up_guided_chargen_approved_plays() {
    let server = boot("gated", true).await;
    let addr = server.addr;

    // --- Staff (God) connects through the same welcome screen. -----------
    let (god_handle, mut god) = play_client(addr).await;
    god.expect("Welcome to the onboarding MUSH!").await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God").await;

    // --- A brand-new player creates an account and is dropped into the ---
    // --- guided wizard (the connect-time hook, over real SSH). -----------
    let (nick_handle, mut nick) = play_client(addr).await;
    nick.expect("connect <account> <password>").await;
    nick.send_line("create Nick secret123").await;
    nick.expect("Account created.").await;
    nick.expect("Connected as Nick").await;
    // Dropped straight into the guided wizard by the connect-time hook. The
    // initial auto-look is correctly suppressed (it would otherwise be fed to
    // the wizard as a bogus template line); the wizard's own prompt stands.
    nick.expect("Guided Character Creation").await;

    // --- Build a vampire through the wizard and submit. -------------------
    let job = wizard_vampire_and_submit(&mut nick).await;

    // --- Gate holds: a live-grid roll is refused while Pending. -----------
    let blocked_mark = nick.mark();
    nick.send_line("+roll strength").await;
    nick.expect_after(blocked_mark, "must finish creating")
        .await;
    // The refusal was the gate's, not a dice result.
    assert!(
        nick.not_seen_since(blocked_mark, "Nick rolls"),
        "a gated +roll must not produce a roll result"
    );

    // --- Staff approves the chargen job. ---------------------------------
    let god_mark = god.mark();
    god.send_line(&format!("+job/approve {job}=Welcome aboard."))
        .await;
    god.expect_after(god_mark, "approved").await;

    // --- Gate lifts: Nick is moved to the live start room, re-rendered, --
    // --- and the same roll now works. ------------------------------------
    nick.expect("approved").await; // "...has been approved — welcome to the game!"
    nick.expect("The Commons").await; // re-rendered in the live start room
    let play_mark = nick.mark();
    nick.send_line("+roll strength").await;
    nick.expect_after(play_mark, "Nick rolls").await;
    assert!(
        nick.not_seen_since(play_mark, "must finish creating"),
        "an approved character's +roll must not be gated"
    );

    let _ = god_handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
    let _ = nick_handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chargen_not_required_leaves_classic_flow_intact() {
    // The back-compat default: no gate, no forced wizard, classic +chargen.
    let server = boot("ungated", false).await;
    let addr = server.addr;

    let (mort_handle, mut mort) = play_client(addr).await;
    mort.expect("connect <account> <password>").await;
    mort.send_line("create Morty pass1234").await;
    mort.expect("Account created.").await;
    mort.expect("Connected as Morty").await;
    // Lands in Limbo (the seeded start room), NOT the chargen area — the
    // onboarding spawn observer is inert when chargen_required is off.
    mort.expect("Limbo(#0)").await;

    // No wizard was forced: an ordinary command runs immediately, and the
    // classic M6 +chargen opens a session rather than being gated.
    let mark = mort.mark();
    mort.send_line("say hello world").await;
    mort.expect_after(mark, "Morty says, \"hello world\"").await;
    assert!(
        mort.not_seen_since(mark, "Guided Character Creation"),
        "no wizard is forced when chargen_required is off"
    );

    let chargen_mark = mort.mark();
    mort.send_line("+chargen").await;
    mort.expect_after(chargen_mark, "Pick a splat").await;
    assert!(
        mort.not_seen_since(chargen_mark, "must finish creating"),
        "+chargen is never gated when chargen_required is off"
    );
    // The classic template surface works: pick mortal.
    mort.send_line("+chargen/template mortal").await;
    mort.expect_after(chargen_mark, "mortal").await;

    let _ = mort_handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
}
