//! **The arena rig** (cut AJ.1): the production composition with the simulator
//! centre open, booted inside the bot's own test binary, and a line client that
//! is the bot's own `mush_bot::transport::Session`.
//!
//! Game-crate imports here belong to the server under test and nothing else,
//! exactly as `tests/e2e.rs`'s fixture's do: the bot's parser, picture and
//! decisions never see a game type. It is modelled on
//! `crates/mushd/tests/sim_solo.rs` (`boot_on`, `tacsim_config`) and on
//! `tests/e2e.rs`'s `Fixture` (the scratch directory, the host-key pin read off
//! the listener's own `ssh_host_ed25519`).
//!
//! **Why the client logs in through the front door.** `Session::connect` reads a
//! password only from an environment variable, and setting one in-process is
//! `unsafe` under edition 2024 (the workspace denies `unsafe_code`); and
//! `Session::send` refuses a `connect` line, as it must. So the in-process client
//! authenticates as `play` with no password — the same front door, the same dumb
//! 120 by 40 PTY — and writes `connect <account> <password>` straight onto the
//! session's public channel, as `sim_solo.rs`'s `login` does over its own client.
//! The real bot binary still logs in at the SSH layer, with the password on the
//! child's environment only ([`bot_command`]).
//!
//! **The posed lanes** (cut AK.3) — the win-rate lane and the live commander
//! lane — fly through one function here, [`fly_posed`]: the same rig, the same
//! two poses before every match, the same 420 s cap, one deadline for the
//! whole flight, and a result ([`Posed`]) that always holds what there is to
//! print and keep, however the flight ended.
//!
//! **The one cross-crate state-slot site** of the bot's tests is in [`boot`]: the
//! account oracle installed into its module-state slot after composition, as
//! every simulator lens does, because a simulator slot belongs to an ACCOUNT
//! (`crates/mush-contract-tests/state-slot-owners.toml` carries its row).
#![allow(dead_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use mush_bot::{config::Ssh, terminal::Decoder, transport::Session};
use mush_core::{Engine, GOD, LIMBO, ObjKind, module::GameModule};
use mush_llm::{LlmProvider, LlmService, MockProvider};
use mush_session::{SessionConfig, SessionManager};
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use mushd::{
    accounts::{Accounts, hash_password},
    ipguard::IpGuard,
    modules,
    persist::load_or_seed_world,
    ssh_bridge::{SshBridge, SshListener, start_listener},
    staff::{StaffQueue, register_server_commands},
    world_task::{self, PersistHandles, ServerDeps},
};
use russh::ChannelMsg;

/// The staff account (`boss`, character `God`).
pub const GOD_ACCOUNT: &str = "boss";
pub const GOD_PASSWORD: &str = "arena-boss-secret-1";
/// Every pilot account's password.
pub const PILOT_PASSWORD: &str = "arena-pilot-secret-1";
/// The one variable a bot child reads its password from ([`bot_command`]).
pub const PASSWORD_ENV: &str = "MUSH_BOT_ARENA_PASSWORD";
/// The front door's banner.
pub const BANNER: &str = "Welcome to the simulator.";
/// How long any one wait for a server answer may take.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(20);

/// The transients a line is sent again after (the four `Line::Transient`
/// sentences, by a needle each).
pub const TRANSIENTS: [&str; 4] = [
    "please retry in a moment",
    "please try again shortly",
    "control state unknown",
    "The arena is busy; try that again in a moment.",
];
/// How many times [`Client::say`] sends a line refused as transient.
pub const RETRY_BUDGET: usize = 40;

/// A booted server: its SSH address, its host key's pin, its scratch directory
/// (removed on drop) and its store.
pub struct Rig {
    pub addr: SocketAddr,
    pub pin: String,
    pub dir: PathBuf,
    pub store: Arc<dyn ObjectStore>,
    _listener: SshListener,
    _handles: PersistHandles,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Rig {
    /// The bot's own SSH settings for this server: `username` at the pinned host
    /// key, no password, no key.
    pub fn ssh(&self, username: &str) -> Ssh {
        Ssh {
            host: "127.0.0.1".into(),
            port: self.addr.port(),
            username: username.into(),
            password_env: None,
            identity_file: None,
            host_key_sha256: Some(self.pin.clone()),
            ..Ssh::default()
        }
    }
}

/// **The preset the arena lanes run on**: the layer on, the AI fill on (so one
/// pilot is a match), the shipped tempo, a one-second startup, a thirty-second
/// boarding window, the validated floor of a match's length (a minute) and the
/// floor of the unattended close.
pub fn arena_config() -> tacsim::TacsimConfig {
    let mut cfg = tacsim::TacsimConfig {
        enabled: true,
        ..tacsim::TacsimConfig::default()
    };
    cfg.match_.ai_fill = true;
    cfg.match_.startup_ms = 1_000;
    cfg.match_.board_timeout_ms = 30_000;
    cfg.match_.max_duration_ms = 60_000;
    cfg.match_.unattended_ms = 10_000;
    cfg
}

fn scratch_dir() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "mush-bot-arena-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("the rig's scratch directory");
    dir
}

/// The production module list, in production order, with the starter area on.
fn module_list(
    cfg: tacsim::TacsimConfig,
    onboarding: &mush_core::OnboardingConfig,
    llm: &Arc<LlmService>,
) -> Vec<Box<dyn GameModule>> {
    let mut mods = modules::server_modules();
    mods.push(modules::worldgen_module(Arc::clone(llm)));
    mods.push(modules::npc_module(modules::npc_agent_adapter()));
    mods.extend(modules::sheet_modules(onboarding, &Default::default()));
    mods.push(modules::tacsim_module(cfg));
    mods.push(modules::content_module_with(true, true).expect("the content module composes"));
    mods
}

/// **Boot the server under test**: an in-memory store holding `boss` (God) and
/// one account per `(account, character)` pilot, every pilot a player in Limbo
/// made before the world task spawns and before the account oracle is hydrated,
/// and the SSH listener with [`BANNER`].
pub async fn boot(cfg: tacsim::TacsimConfig, pilots: &[(&str, &str)]) -> Rig {
    let dir = scratch_dir();
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    store
        .create_account(AccountRec {
            id: None,
            name: GOD_ACCOUNT.to_owned(),
            argon2_hash: hash_password(GOD_PASSWORD).expect("a hash"),
            created: 0,
            characters: vec![CharacterRef {
                id: GOD,
                name: "God".to_owned(),
            }],
            ssh_keys: vec![],
            discord: None,
        })
        .await
        .expect("the staff account");

    let mut world = load_or_seed_world(&*store).await.expect("boot world");
    for (n, (account, character)) in pilots.iter().enumerate() {
        let id = world
            .create(ObjKind::Player, character, GOD, Some(LIMBO))
            .expect("a pilot");
        store
            .create_account(AccountRec {
                id: None,
                name: (*account).to_owned(),
                argon2_hash: hash_password(PILOT_PASSWORD).expect("a hash"),
                created: i64::try_from(n).unwrap_or(0) + 1,
                characters: vec![CharacterRef {
                    id,
                    name: (*character).to_owned(),
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
    // The server's own generation service, globally disabled: nothing in an
    // arena lane asks it for anything, and the bot never talks to it.
    let mock: Arc<dyn LlmProvider> = Arc::new(MockProvider::new("openrouter"));
    let llm = Arc::new(
        LlmService::builder()
            .provider("openrouter", mock)
            .global_enabled(false)
            .build(),
    );

    let mut engine = Engine::with_modules(world, registry, module_list(cfg, &onboarding, &llm))
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
            ..ServerDeps::in_memory()
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
        BANNER.to_owned(),
        bridge,
    )
    .await
    .expect("ssh listener");
    let key = russh::keys::load_secret_key(dir.join("ssh_host_ed25519"), None)
        .expect("the listener's host key");
    let pin = key
        .public_key()
        .fingerprint(russh::keys::HashAlg::Sha256)
        .to_string();
    Rig {
        addr: listener.addr(),
        pin,
        dir,
        store,
        _listener: listener,
        _handles: handles,
    }
}

/// What [`Client::try_send`] and [`Client::answer`] say once the session has
/// closed (cut AM.3).
pub const SESSION_CLOSED: &str = "the session is closed: the channel has closed";

/// **An in-process line client**: the bot's own SSH session, and everything it
/// has been sent, decoded by the bot's own `terminal::Decoder`.
pub struct Client {
    session: Session,
    decoder: Decoder,
    /// Every decoded byte since the connection opened.
    pub seen: String,
    /// Where the last [`Client::send`] began.
    mark: usize,
    /// Where the last [`Client::drain`] stopped.
    drained: usize,
    /// **The session has closed** (cut AM.3): set by [`Client::until`] (and so
    /// [`Client::pump`]) when the channel answers nothing more, `Close` or
    /// `Eof`. While it is set no wait waits: `until` answers `None` at once,
    /// [`Client::idle`] returns at once and [`Client::try_send`] refuses, so a
    /// lane whose watcher has gone ends its wait instead of spinning to its
    /// limit.
    pub closed: bool,
}

impl Client {
    /// Connect through the front door (`play`, no password), write the
    /// `connect` line straight onto the channel, and wait for `Connected as
    /// {character}`.
    pub async fn login(rig: &Rig, account: &str, password: &str, character: &str) -> Client {
        let session = Session::connect(&rig.ssh("play"))
            .await
            .expect("the front door opens");
        let mut client = Client {
            session,
            decoder: Decoder::default(),
            seen: String::new(),
            mark: 0,
            drained: 0,
            closed: false,
        };
        client.expect(BANNER, STEP_TIMEOUT).await;
        client.mark = client.seen.len();
        client
            .session
            .channel
            .data(format!("connect {account} {password}\r").as_bytes())
            .await
            .expect("the connect line is written");
        client
            .expect(&format!("Connected as {character}"), STEP_TIMEOUT)
            .await;
        client
    }

    /// Send one line through the bot's own `Session::send` and mark where its
    /// answer begins. Panics when the line cannot be sent: for the set-ups
    /// that want a dead session to fail the test; a lane that must carry on
    /// uses [`Client::try_send`].
    pub async fn send(&mut self, line: &str) {
        self.mark = self.seen.len();
        self.session.send(line).await.expect("the line is sent");
    }

    /// **Send one line, and never panic** (cut AM.3): as [`Client::send`], but
    /// a closed session ([`Client::closed`]) or a line that cannot be sent is
    /// `Err("the session is closed: {error}")`.
    pub async fn try_send(&mut self, line: &str) -> Result<(), String> {
        self.mark = self.seen.len();
        if self.closed {
            return Err(SESSION_CLOSED.to_owned());
        }
        self.session
            .send(line)
            .await
            .map_err(|error| format!("the session is closed: {error}"))
    }

    /// **Type `QUIT`** (cut AM.3), the server's own disconnect, straight onto
    /// the channel as [`Client::login`] writes its `connect` line: the bot's
    /// own `Session::send` refuses the session verbs, as it must.
    pub async fn quit(&mut self) {
        self.mark = self.seen.len();
        self.session
            .channel
            .data(&b"QUIT\r"[..])
            .await
            .expect("the QUIT line is written");
    }

    /// Wait until one of `needles` has arrived since `from`; its index, or
    /// `None` at the deadline or when the channel closes. A needle already
    /// read still counts after the close; otherwise a closed session answers
    /// `None` at once (cut AM.3).
    async fn until(&mut self, from: usize, needles: &[&str], timeout: Duration) -> Option<usize> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let tail = self.seen.get(from..).unwrap_or("");
            if let Some(index) = needles.iter().position(|needle| tail.contains(needle)) {
                return Some(index);
            }
            if self.closed {
                return None;
            }
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                return None;
            }
            match tokio::time::timeout(left, self.session.channel.wait()).await {
                Ok(Some(ChannelMsg::Data { data }))
                | Ok(Some(ChannelMsg::ExtendedData { data, .. })) => {
                    let text = self.decoder.feed(&data);
                    self.seen.push_str(&text);
                }
                // The server closed the session: nothing more will come.
                Ok(None) | Ok(Some(ChannelMsg::Close)) | Ok(Some(ChannelMsg::Eof)) => {
                    self.closed = true;
                    return None;
                }
                Ok(Some(_)) => {}
                Err(_) => return None,
            }
        }
    }

    /// Wait for `needle` after the mark and answer everything decoded since it.
    pub async fn expect(&mut self, needle: &str, timeout: Duration) -> String {
        let from = self.mark;
        if self.until(from, &[needle], timeout).await.is_none() {
            panic!(
                "timed out waiting for {needle:?}; since the mark:\n{}",
                self.seen.get(from..).unwrap_or("")
            );
        }
        self.seen.get(from..).unwrap_or("").to_owned()
    }

    /// Wait up to `timeout` for anything more, and keep it.
    pub async fn pump(&mut self, timeout: Duration) {
        let from = self.seen.len();
        let _ = self.until(from, &["\n"], timeout).await;
    }

    /// **Send `line` and wait for `needle` in its answer**, sending it again
    /// after 200 ms whenever the answer is one of the four transients, at most
    /// [`RETRY_BUDGET`] times (`sim_solo.rs`' `say`). Answers everything decoded
    /// since the last send.
    pub async fn say(&mut self, line: &str, needle: &str) -> String {
        assert!(
            !line.contains(needle),
            "`{needle}` appears in `{line}`'s own echo; pick a needle from the answer"
        );
        let mut needles = vec![needle];
        needles.extend(TRANSIENTS);
        for _ in 0..RETRY_BUDGET {
            self.send(line).await;
            let from = self.mark;
            match self.until(from, &needles, STEP_TIMEOUT).await {
                Some(0) => return self.seen.get(from..).unwrap_or("").to_owned(),
                Some(_) => tokio::time::sleep(Duration::from_millis(200)).await,
                None => panic!(
                    "`{line}` never answered {needle:?}; since it was sent:\n{}",
                    self.seen.get(from..).unwrap_or("")
                ),
            }
        }
        panic!("`{line}` was refused as transient {RETRY_BUDGET} times running");
    }

    /// **Send `line` and wait for any of `needles`** (cut AJ.4), sending it
    /// again after 200 ms on a transient as [`Client::say`] does: the index of
    /// the needle that came and everything decoded since the last send, or
    /// `None` and that text when nothing came inside [`STEP_TIMEOUT`]. Never
    /// panics on a refusal, so a caller can print one and carry on; nor (cut
    /// AM.3) on a closed session, which answers `None` and
    /// [`Client::try_send`]'s `the session is closed: …` at once — whether it
    /// was seen closed before the send or closed while its answer was awaited
    /// (then with what came before the close after it).
    pub async fn answer(&mut self, line: &str, needles: &[&str]) -> (Option<usize>, String) {
        let mut all: Vec<&str> = needles.to_vec();
        all.extend(TRANSIENTS);
        for _ in 0..RETRY_BUDGET {
            if let Err(closed) = self.try_send(line).await {
                return (None, closed);
            }
            let from = self.mark;
            match self.until(from, &all, STEP_TIMEOUT).await {
                Some(index) if index < needles.len() => {
                    return (Some(index), self.seen.get(from..).unwrap_or("").to_owned());
                }
                Some(_) => tokio::time::sleep(Duration::from_millis(200)).await,
                None if self.closed => {
                    return (
                        None,
                        format!(
                            "{SESSION_CLOSED}; since `{line}` was sent:\n{}",
                            self.seen.get(from..).unwrap_or("")
                        ),
                    );
                }
                None => return (None, self.seen.get(from..).unwrap_or("").to_owned()),
            }
        }
        (
            None,
            format!("`{line}` was refused as transient {RETRY_BUDGET} times running"),
        )
    }

    /// **Wait for `needle` anywhere after byte `from`** of [`Client::seen`]
    /// (cut AJ.4), reading as it goes: whether it came inside `timeout`. A
    /// long watch is a loop of short ones, so the caller can look at a child
    /// process between them.
    pub async fn watch(&mut self, from: usize, needle: &str, timeout: Duration) -> bool {
        self.until(from, &[needle], timeout).await.is_some()
    }

    /// **Keep reading for `span`** (cut AJ.4): a client that sits in a pod
    /// still drains what it is sent, so nothing backs up behind it. A closed
    /// session has nothing to read, so it returns at once (cut AM.3).
    pub async fn idle(&mut self, span: Duration) {
        let deadline = tokio::time::Instant::now() + span;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() || self.closed {
                return;
            }
            self.pump(left).await;
        }
    }

    /// Everything decoded since the last drain.
    pub fn drain(&mut self) -> String {
        let text = self.seen.get(self.drained..).unwrap_or("").to_owned();
        self.drained = self.seen.len();
        text
    }

    pub async fn close(self) {
        self.session.close().await;
    }
}

/// **Staff re-pose a tank for the next match** (cut AJ.2): `@arena/pose
/// {callsign}={x} {y} {heading}` typed by God's client, waited for its answer.
/// It is refused only while a match runs, it survives a player's claim (which
/// puts the tanks back at muster but keeps the openings), and the match that
/// uses it clears it. The stock muster is 2 430 m apart against a 600 m sight,
/// so every live arena lane poses the two tanks inside each other's sight.
pub async fn pose(god: &mut Client, spec: &str) {
    god.say(&format!("@arena/pose {spec}"), "opens at cell")
        .await;
}

/// The answers `@arena/pose` refuses with, one needle each (cut AK.3), read
/// from the verb's own refusal sentences as text: an unknown callsign (`There
/// is no slot {X} on this centre.`, the verb's first refusal; cut AM.3), the
/// centre running, a cell off the window, a cell that is impassable or has no
/// way out, a cell already another tank's opening cell, a heading out of
/// range, a malformed line, and a server without the arena.
pub const POSE_REFUSALS: [&str; 8] = [
    "There is no slot ",
    "The simulator centre is running; set opening cells",
    "is off the arena window.",
    "is impassable, or has no way out.",
    "opening cell.",
    "Heading is 0 to 359.",
    "Usage: ",
    "The arena is not available on this server.",
];

/// **Re-pose a tank between matches** (cut AJ.4, the win-rate lane): as
/// [`pose`], but a refusal is the answer's text, not a panic, so a lane that
/// re-poses before every match can print it and carry on. `Ok` holds the
/// `opens at cell` answer. **A refusal answers at once** (cut AK.3): any of
/// [`POSE_REFUSALS`] ends the wait as `Err` with the answer's text, so a
/// refused pose never costs the whole [`STEP_TIMEOUT`]; no answer inside it is
/// an `Err` too, and so (cut AM.3) is a closed session, at once and without a
/// panic ([`Client::try_send`]).
pub async fn try_pose(god: &mut Client, spec: &str) -> Result<String, String> {
    let mut needles = vec!["opens at cell"];
    needles.extend(POSE_REFUSALS);
    match god.answer(&format!("@arena/pose {spec}"), &needles).await {
        (Some(0), text) => Ok(text),
        (_, text) => Err(text),
    }
}

/// **Walk a line client from Limbo to the Simulator Lobby** (cut AJ.4): `ooc`
/// to the Welcome Hall, `simulator` to the lobby, as a person types it. God
/// stands there to hear the centre's announcements; the human wingman claims
/// from there.
pub async fn walk_to_lobby(client: &mut Client) {
    client.say("ooc", "Welcome Hall").await;
    client.say("simulator", "Simulator Lobby").await;
}

/// **The bot's config for this rig** (cut AJ.2): `[ssh]` at the pinned host key
/// as `username` with the password in [`PASSWORD_ENV`], no `[model]`, memory
/// and costs session-only, the run bounded at 150 s, then `extra` (more
/// tables). Written under the bot's own working directory.
pub fn bot_config(rig: &Rig, username: &str, extra: &str) -> PathBuf {
    bot_config_with(rig, username, 150, extra)
}

/// [`bot_config`] with `[agent] max_seconds` of the caller's (cut AJ.4): in
/// arena mode it bounds each match (a claim starts it again), and the
/// win-rate lane's seven-minute matches ([`MAX_DURATION_MS`]) need longer than
/// 150 s.
pub fn bot_config_with(rig: &Rig, username: &str, max_seconds: u64, extra: &str) -> PathBuf {
    bot_config_agent(rig, username, max_seconds, 100, 1_000, extra)
}

/// [`bot_config_with`] with the `[agent]` table's `settle_ms` and
/// `observation_ms` of the caller's too (cut AM.2): how long a command's output
/// must be quiet, and at most how long it is collected, before the command is
/// finished. They cannot come through `extra`: TOML refuses a second `[agent]`
/// table.
pub fn bot_config_agent(
    rig: &Rig,
    username: &str,
    max_seconds: u64,
    settle_ms: u64,
    observation_ms: u64,
    extra: &str,
) -> PathBuf {
    let home = rig.dir.join("bot");
    std::fs::create_dir_all(&home).expect("the bot's working directory");
    let config = format!(
        r#"[ssh]
host = "127.0.0.1"
port = {}
username = "{username}"
password_env = "{PASSWORD_ENV}"
host_key_sha256 = "{}"

[agent]
max_seconds = {max_seconds}
settle_ms = {settle_ms}
observation_ms = {observation_ms}

[memory]
persistent = false

[costs]
persistent = false
{extra}
"#,
        rig.addr.port(),
        rig.pin
    );
    let path = home.join("arena.toml");
    std::fs::write(&path, config).expect("the bot's config");
    path
}

/// **The bot binary as a child of this rig** (for AJ.2 and AJ.4): `--config
/// config`, its working directory a directory of its own under the rig's scratch
/// directory holding an **empty `.env`** (the bot's `dotenvy::dotenv()` walks up
/// from its working directory and stops at the first `.env` it finds, so no
/// `.env` in any parent — the repository's, the home directory's the temporary
/// directory sits under, a container's `/.env` — can put a removed key back, as
/// `tests/studio.rs` does), every model key and the bot's default password
/// variable removed, and the one password variable the rig's configs name
/// ([`PASSWORD_ENV`]) set on the child only.
pub fn bot_command(rig: &Rig, config: &Path, password: &str) -> std::process::Command {
    bot_command_in(&rig.dir.join("bot"), config, password)
}

/// **A bot child in flight** (cut AJ.4): its output collected on a task of its
/// own from the moment it starts, so its pipes never fill while the test drives
/// God and the human wingman beside it; where its transcript and report go.
pub struct Flight {
    pub task: tokio::task::JoinHandle<std::io::Result<std::process::Output>>,
    pub transcript: PathBuf,
    pub report: PathBuf,
}

/// **What a bot child left** (cut AJ.4): its exit code, its stdout and stderr,
/// its JSON report (whole and as text) and every transcript record.
pub struct Landed {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub report: serde_json::Value,
    pub report_text: String,
    pub records: Vec<serde_json::Value>,
}

impl Landed {
    /// Every line the lane typed, in order: `(line, purpose, lane_ms)`.
    pub fn sends(&self) -> Vec<(String, String, u64)> {
        self.records
            .iter()
            .filter(|record| record["kind"] == "arena-send")
            .map(|record| {
                (
                    record["data"]["line"].as_str().expect("a line").to_owned(),
                    record["data"]["purpose"]
                        .as_str()
                        .expect("a purpose")
                        .to_owned(),
                    record["data"]["lane_ms"].as_u64().expect("a lane stamp"),
                )
            })
            .collect()
    }

    /// The text of every `kind` event, in order (`arena`, `send`, …).
    pub fn events(&self, kind: &str) -> Vec<String> {
        self.records
            .iter()
            .filter(|record| record["kind"] == kind)
            .filter_map(|record| record["data"].as_str().map(str::to_owned))
            .collect()
    }

    /// The report's `arena.matches`, or a panic that shows the run.
    pub fn matches(&self) -> &Vec<serde_json::Value> {
        self.report["arena"]["matches"]
            .as_array()
            .unwrap_or_else(|| {
                panic!(
                    "the arena block: {}\nstdout:\n{}\nstderr:\n{}",
                    self.report_text, self.stdout, self.stderr
                )
            })
    }
}

/// **Start the bot binary against this rig** (cut AJ.4): `--config config`,
/// `args`, `--headless`, and a transcript and report named `{name}` under the
/// bot's working directory, through [`bot_command`] (the empty `.env`, no model
/// key, the password on the child only).
pub fn launch(rig: &Rig, config: &Path, name: &str, args: &[&str]) -> Flight {
    launch_env(rig, config, name, args, &[])
}

/// [`launch`] with variables of the caller's on the child (cut AK.3): each
/// `env` pair is set **after** [`bot_command`]'s removals, so it wins over
/// them. The live commander lane passes its model key this way and nothing
/// else does; the value is never printed.
pub fn launch_env(
    rig: &Rig,
    config: &Path,
    name: &str,
    args: &[&str],
    env: &[(&str, &str)],
) -> Flight {
    let transcript = rig.dir.join("bot").join(format!("{name}.jsonl"));
    let report = rig.dir.join("bot").join(format!("{name}.json"));
    let mut child = bot_command(rig, config, PILOT_PASSWORD);
    for (key, value) in env {
        child.env(key, value);
    }
    let mut command = tokio::process::Command::from(child);
    command
        .args(args)
        .arg("--headless")
        .arg("--transcript")
        .arg(&transcript)
        .arg("--report")
        .arg(&report)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let child = command.spawn().expect("the bot starts");
    Flight {
        task: tokio::spawn(child.wait_with_output()),
        transcript,
        report,
    }
}

/// **Start the bot binary at its prompt** (cut AM.2): as [`launch_env`], but
/// without `--headless`, so it reads its control lines (`/send …`, `/resume`,
/// `/quit`) from stdin, which is piped and handed back — taken from the child
/// before its output is waited for, which would close it.
pub fn launch_interactive(
    rig: &Rig,
    config: &Path,
    name: &str,
    args: &[&str],
) -> (Flight, tokio::process::ChildStdin) {
    let transcript = rig.dir.join("bot").join(format!("{name}.jsonl"));
    let report = rig.dir.join("bot").join(format!("{name}.json"));
    let mut command = tokio::process::Command::from(bot_command(rig, config, PILOT_PASSWORD));
    command
        .args(args)
        .arg("--transcript")
        .arg(&transcript)
        .arg("--report")
        .arg(&report)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().expect("the bot starts");
    let stdin = child.stdin.take().expect("the bot's stdin");
    (
        Flight {
            task: tokio::spawn(child.wait_with_output()),
            transcript,
            report,
        },
        stdin,
    )
}

/// **Wait for the bot to finish**, at most `bound`, and read what it left.
pub async fn land(flight: Flight, bound: Duration) -> Landed {
    let output = tokio::time::timeout(bound, flight.task)
        .await
        .unwrap_or_else(|_| panic!("the bot finishes inside {} s", bound.as_secs()))
        .expect("the bot's task")
        .expect("the bot runs");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let report_text = std::fs::read_to_string(&flight.report).unwrap_or_else(|error| {
        panic!("the report ({error}); stdout:\n{stdout}\nstderr:\n{stderr}")
    });
    let report = serde_json::from_str(&report_text).expect("the report is JSON");
    let records = std::fs::read_to_string(&flight.transcript)
        .expect("the transcript")
        .lines()
        .map(|line| serde_json::from_str(line).expect("a transcript record"))
        .collect();
    Landed {
        code: output.status.code(),
        stdout,
        stderr,
        report,
        report_text,
        records,
    }
}

/// **What a bot child left, whatever happened** (cut AK.3): as [`Landed`], but
/// nothing is required. `code`, `stdout` and `stderr` are empty (`None`) when
/// the child was stopped at [`try_land`]'s bound; `report` is `None` when there
/// is no report or it does not read as JSON; the transcript is its raw text,
/// a torn last line and all.
pub struct Partial {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub transcript_text: String,
    pub report_text: Option<String>,
    pub report: Option<serde_json::Value>,
}

/// **Wait for the bot to finish, and never panic** (cut AK.3): at most
/// `bound`; at the bound the flight's task is aborted, which drops the child
/// (`kill_on_drop`, so it dies), and what exists on disk is read. A lane that
/// must print its figure and keep its transcript whatever went wrong lands
/// here; [`land`] stays for the tests that want a whole run or a failure.
pub async fn try_land(flight: Flight, bound: Duration) -> Partial {
    let Flight {
        mut task,
        transcript,
        report,
    } = flight;
    let (code, stdout, stderr) = match tokio::time::timeout(bound, &mut task).await {
        Ok(Ok(Ok(output))) => (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ),
        Ok(Ok(Err(error))) => (None, String::new(), format!("the bot did not run: {error}")),
        Ok(Err(error)) => (None, String::new(), format!("the bot's task: {error}")),
        Err(_) => {
            task.abort();
            let _ = task.await;
            (None, String::new(), String::new())
        }
    };
    let transcript_text = std::fs::read_to_string(&transcript).unwrap_or_default();
    let report_text = std::fs::read_to_string(&report).ok();
    let report = report_text
        .as_deref()
        .and_then(|text| serde_json::from_str(text).ok());
    Partial {
        code,
        stdout,
        stderr,
        transcript_text,
        report_text,
        report,
    }
}

/// Why a wait for a line while the bot flies ended without it (cut AK.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// The bot's own task finished first.
    Bot,
    /// The wait's limit came first.
    Timeout,
    /// The watching session closed (cut AM.3): nothing more can be heard.
    Closed,
}

/// **Wait for `needle` in `client`'s stream after byte `from` while the bot
/// flies, and never panic** (cut AK.3), at most `limit`: the byte just past
/// it, `Err(Ended::Bot)` when the bot's task finished first,
/// `Err(Ended::Timeout)` at the limit, or (cut AM.3) `Err(Ended::Closed)` as
/// soon as the watching client's session has closed — looked at before the
/// bot's task, so a closed watcher is never read as the bot ending. The
/// flight's task is only looked at, never consumed, so the caller can still
/// land it. A bot that finished is given one short last look, so a line it
/// saw as it exited is not lost to the race between its exit and God's own
/// copy of the announcement.
pub async fn try_await_line(
    client: &mut Client,
    from: usize,
    needle: &str,
    flight: &mut Flight,
    limit: Duration,
) -> Result<usize, Ended> {
    let past = |client: &Client| {
        let at = client
            .seen
            .get(from..)
            .and_then(|tail| tail.find(needle))
            .unwrap_or(0);
        from + at + needle.len()
    };
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if client
            .watch(from, needle, left.min(Duration::from_secs(1)))
            .await
        {
            return Ok(past(client));
        }
        if client.closed {
            return Err(Ended::Closed);
        }
        if flight.task.is_finished() {
            if client.watch(from, needle, Duration::from_secs(2)).await {
                return Ok(past(client));
            }
            return Err(Ended::Bot);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Ended::Timeout);
        }
    }
}

/// **Wait for `needle` in `client`'s stream after byte `from` while the bot
/// flies** (cut AJ.4), at most `limit`: the byte just past it. A bot that ends
/// first fails the wait with its own output, so a blocker reads as itself and
/// not as a timeout. [`try_await_line`] with its three ends made panics.
pub async fn await_line(
    client: &mut Client,
    from: usize,
    needle: &str,
    flight: &mut Flight,
    limit: Duration,
) -> usize {
    match try_await_line(client, from, needle, flight, limit).await {
        Ok(past) => past,
        Err(Ended::Bot) => ended(flight, &format!("{needle:?}")).await,
        Err(Ended::Closed) => panic!(
            "the watching session closed before {needle:?}; since the mark:\n{}",
            client.seen.get(from..).unwrap_or("")
        ),
        Err(Ended::Timeout) => panic!(
            "no {needle:?} inside {} s; since the mark:\n{}",
            limit.as_secs(),
            client.seen.get(from..).unwrap_or("")
        ),
    }
}

impl Flight {
    /// **Whether the bot has typed a `FIRE` yet** (cut AJ.4), read from its
    /// transcript as it is written (one flushed record a line; a torn last
    /// line is skipped until it is whole).
    pub fn fired(&self) -> bool {
        let Ok(text) = std::fs::read_to_string(&self.transcript) else {
            return false;
        };
        text.split_inclusive('\n')
            .filter(|line| line.ends_with('\n'))
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .any(|record| {
                record["kind"] == "arena-send"
                    && record["data"]["line"]
                        .as_str()
                        .is_some_and(|line| line.starts_with("FIRE "))
            })
    }
}

/// **Keep `client` reading until the bot has fired** (cut AJ.4), at most
/// `limit`: whether it did. A test that calls a match off once the proof it
/// needs is in never races the machine for the match's own end (the first
/// `FIRE` comes about five seconds after `is running`; the machine has ended a
/// posed 1 v 1 in about thirty). A bot that ends first fails the wait with its
/// own output.
pub async fn await_fire(client: &mut Client, flight: &mut Flight, limit: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        if flight.fired() {
            return true;
        }
        if flight.task.is_finished() {
            ended(flight, "its first FIRE").await;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        client.idle(Duration::from_millis(250)).await;
    }
}

/// **The match is still running**, as `client` heard it after byte `from`
/// (cut AJ.4): a test about to call a match off fails here, saying so, when
/// the centre already announced its end, rather than on the abort's refusal.
pub fn still_running(client: &Client, from: usize) {
    let since = client.seen.get(from..).unwrap_or("");
    assert!(
        !since.contains(" is over ("),
        "the match ended before the test called it off:\n{since}"
    );
}

/// The bot ended before `what`: fail with its own output.
async fn ended(flight: &mut Flight, what: &str) -> ! {
    let output = (&mut flight.task).await;
    let (stdout, stderr) = match &output {
        Ok(Ok(output)) => (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ),
        other => (format!("{other:?}"), String::new()),
    };
    panic!("the bot ended before {what}; stdout:\n{stdout}\nstderr:\n{stderr}");
}

/// [`bot_command`] in a working directory of the caller's (cut AJ.2): the same
/// empty `.env`, the same removed keys and the same one password variable, for
/// a child that never reaches a server (the command line's own refusals).
pub fn bot_command_in(home: &Path, config: &Path, password: &str) -> std::process::Command {
    std::fs::create_dir_all(home).expect("the bot's working directory");
    std::fs::write(home.join(".env"), "").expect("the bot's empty .env");
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_mush-bot"));
    command
        .current_dir(home)
        .arg("--config")
        .arg(config)
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("MUSH_BOT_PASSWORD")
        .env(PASSWORD_ENV, password)
        .stdin(std::process::Stdio::null());
    command
}

// ---------------------------------------------------------------------------
// Cut AK.3: the posed lanes (the win-rate lane and the live commander lane).
// ---------------------------------------------------------------------------

/// **Each posed match's length** before the centre calls it a timeout (cut
/// AK.3, 16c Q-AK3): 420 s, so a fight the pilot is winning is finished
/// rather than cut off (on AJ's 180 s cap five of ten timed out, every one
/// one-sided). A timeout is its own column, never a loss.
pub const MAX_DURATION_MS: u32 = 420_000;
/// `[agent] max_seconds` in a posed lane's config: in arena mode it bounds
/// each match (a claim starts it again), so it sits above the cap.
pub const MAX_SECONDS: u64 = 540;
/// The longest God waits for one match's end, boarding and startup included.
pub const MATCH_WAIT: Duration = Duration::from_secs(600);
/// The two poses, 500 m apart on the east-west road, facing each other.
pub const POSES: [&str; 2] = ["RA=100 97 90", "BA=150 97 270"];
/// The variable naming a directory a posed lane keeps its transcript
/// (`{name}.jsonl`) and report (`{name}.json`) in (the rig's own directory
/// is removed).
pub const KEEP_ENV: &str = "MUSH_BOT_WIN_RATE_KEEP";

/// What a posed lane flies: the flight's name (and the kept files' stem), how
/// many matches, the whole flight's deadline measured from the launch, the
/// config's extra tables, and variables for the child ([`launch_env`]).
pub struct PosedPlan<'a> {
    pub name: &'a str,
    pub matches: usize,
    pub deadline: Duration,
    pub extra: &'a str,
    pub env: &'a [(&'a str, &'a str)],
}

/// **What a posed flight left**, however it ended (cut AK.3).
pub struct Posed {
    /// What the bot left ([`try_land`]).
    pub partial: Partial,
    /// God's own `Match {n} is over ({word})` lines since the launch, by id.
    pub ends: BTreeMap<u64, String>,
    /// The match ends God waited for and heard.
    pub heard: usize,
    /// Why the flight ended before its last match's end, when it did:
    /// `the bot ended`, `no match end inside 600 s`, `the {m}-minute
    /// deadline` or `the watching session closed`.
    pub early: Option<String>,
    /// Each refused pose: `None` before the launch, `Some(n)` after God heard
    /// match `n` end.
    pub refused: Vec<Option<u64>>,
}

impl Posed {
    /// The report's `arena.matches`, in the order flown; none without a report.
    pub fn flown(&self) -> &[serde_json::Value] {
        self.partial
            .report
            .as_ref()
            .and_then(|report| report["arena"]["matches"].as_array())
            .map_or(&[], Vec::as_slice)
    }

    /// **The matches flown from muster**, keyed by id rather than by flying
    /// order: a pose refused before the launch flags the first match flown,
    /// and one refused after match `n` ended flags the first match flown whose
    /// id is greater than `n` (the one those poses were for).
    pub fn unposed(&self) -> BTreeSet<u64> {
        let ids: Vec<u64> = self
            .flown()
            .iter()
            .filter_map(|flown| flown["match"].as_u64())
            .collect();
        self.refused
            .iter()
            .filter_map(|after| match after {
                None => ids.first().copied(),
                Some(n) => ids.iter().copied().find(|id| id > n),
            })
            .collect()
    }

    /// `UNPOSED: {u} of {n} match(es) flew from muster.`, when any did.
    pub fn unposed_line(&self) -> Option<String> {
        let unposed = self.unposed().len();
        (unposed > 0).then(|| {
            format!(
                "UNPOSED: {unposed} of {} match(es) flew from muster.",
                self.flown().len()
            )
        })
    }

    /// `EARLY END: {why} after {n} match(es).`, when the flight ended early.
    pub fn early_line(&self) -> Option<String> {
        self.early
            .as_ref()
            .map(|why| format!("EARLY END: {why} after {} match(es).", self.heard))
    }

    /// The results, from the report when there is one, else from God's own
    /// match-end lines.
    pub fn outcome(&self) -> Outcome {
        match &self.partial.report {
            Some(report) => Outcome::from_report(report),
            None => Outcome::from_lobby(&self.ends),
        }
    }
}

/// The line printed under the results line when the bot left no report.
pub const NO_REPORT: &str = "NO REPORT: the figure is from the lobby's match-end lines.";

/// **A posed lane's results** for red (cut AK.3): won, lost, drew, timed out,
/// and anything else.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub won: u64,
    pub lost: u64,
    pub drew: u64,
    pub timed_out: u64,
    pub other: u64,
}

impl Outcome {
    /// From the report's `arena` block (`wins`, `losses`, `draws`,
    /// `timeouts`, `other`).
    pub fn from_report(report: &serde_json::Value) -> Outcome {
        let count = |key: &str| report["arena"][key].as_u64().unwrap_or(0);
        Outcome {
            won: count("wins"),
            lost: count("losses"),
            drew: count("draws"),
            timed_out: count("timeouts"),
            other: count("other"),
        }
    }

    /// From God's `Match {n} is over ({word})` lines: `red` a win, `blue` a
    /// loss, `draw` a draw, `timeout` a timeout, anything else other.
    pub fn from_lobby(ends: &BTreeMap<u64, String>) -> Outcome {
        let mut outcome = Outcome::default();
        for word in ends.values() {
            match word.as_str() {
                "red" => outcome.won += 1,
                "blue" => outcome.lost += 1,
                "draw" => outcome.drew += 1,
                "timeout" => outcome.timed_out += 1,
                _ => outcome.other += 1,
            }
        }
        outcome
    }

    /// `{head}: {w} of {of} for red against one machine at 1 v 1 (lost {l},
    /// drew {d}, timed out {t}, other {o})` — AJ's `WIN RATE` format exactly.
    pub fn line(&self, head: &str, of: usize) -> String {
        format!(
            "{head}: {} of {of} for red against one machine at 1 v 1 (lost {}, drew {}, timed out {}, other {})",
            self.won, self.lost, self.drew, self.timed_out, self.other
        )
    }
}

/// The match id of a `Match {n} is over (` line whose needle begins at the end
/// of `before`: the digits after the last `Match ` in it.
fn ended_id(before: &str) -> Option<u64> {
    let at = before.rfind("Match ")?;
    before.get(at + "Match ".len()..)?.trim().parse().ok()
}

/// **Every `Match {n} is over ({word})` line in `text`**, by id: the centre's
/// own announcement, as God heard it in the lobby.
pub fn match_ends(text: &str) -> BTreeMap<u64, String> {
    const NEEDLE: &str = " is over (";
    let mut ends = BTreeMap::new();
    for (at, _) in text.match_indices(NEEDLE) {
        let Some(id) = text.get(..at).and_then(ended_id) else {
            continue;
        };
        let Some(word) = text
            .get(at + NEEDLE.len()..)
            .and_then(|tail| tail.split_once(')'))
            .map(|(word, _)| word)
        else {
            continue;
        };
        if !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric()) {
            ends.insert(id, word.to_owned());
        }
    }
    ends
}

/// **Why a posed flight ended early** (cut AK.3), from how its last wait
/// ended: `the bot ended`; at a wait whose `limit` was cut short of
/// [`MATCH_WAIT`] by the flight's own `deadline`, `the {m}-minute deadline`;
/// `the watching session closed` when God's own session did (cut AM.3);
/// otherwise `no match end inside 600 s`.
pub fn early_why(ended: Ended, limit: Duration, deadline: Duration) -> String {
    match ended {
        Ended::Bot => "the bot ended".to_owned(),
        Ended::Closed => "the watching session closed".to_owned(),
        Ended::Timeout if limit < MATCH_WAIT => {
            format!("the {}-minute deadline", deadline.as_secs() / 60)
        }
        Ended::Timeout => format!("no match end inside {} s", MATCH_WAIT.as_secs()),
    }
}

/// **Fly a posed lane** (cut AK.3; both posed lanes call it): boot the rig
/// with the AI fill and a [`MAX_DURATION_MS`] cap; God poses `RA` and `BA`
/// ([`POSES`]) and walks to the Simulator Lobby; the bot flies `--arena red
/// --matches {n}` on `[agent] max_seconds` [`MAX_SECONDS`] and the plan's
/// extra tables; and after each `Match {n} is over (` God hears, God poses
/// both tanks again for the next (the end consumed and cleared the last ones,
/// the new ones survive the bot's re-claim a second later, and the lanes'
/// `lead_start_grace_ms` of 5 s keeps its `SIM START` behind them).
///
/// **One deadline, measured from the launch**: each wait's limit is the
/// lesser of [`MATCH_WAIT`] and what is left of it, so running out ends the
/// loop like any other early end. The loop never panics: a bot that ends, a
/// match that never ends, the deadline and (cut AM.3) God's own session
/// closing each end it, remembered in [`Posed::early`], and a pose on a closed
/// session is refused, not a panic ([`try_pose`]). Then the bot is landed ([`try_land`], 120 s), and when
/// [`KEEP_ENV`] names a directory its transcript (raw) and report (when there
/// is one) are copied there **before** anything is printed or asserted.
/// A refused pose is printed and recorded ([`Posed::refused`]), and the
/// flight carries on.
pub async fn fly_posed(plan: &PosedPlan<'_>) -> Posed {
    let mut cfg = arena_config();
    cfg.match_.startup_ms = 2_000;
    cfg.match_.board_timeout_ms = 60_000;
    cfg.match_.max_duration_ms = MAX_DURATION_MS;
    let rig = boot(cfg, &[("pilot", "Pilot")]).await;

    let mut god = Client::login(&rig, GOD_ACCOUNT, GOD_PASSWORD, "God").await;
    let mut refused = Vec::new();
    for spec in POSES {
        if let Err(answer) = try_pose(&mut god, spec).await {
            println!(
                "the pose {spec} before the first match was refused: {}",
                answer.trim()
            );
            refused.push(None);
        }
    }
    walk_to_lobby(&mut god).await;

    let config = bot_config_with(&rig, "pilot+Pilot", MAX_SECONDS, plan.extra);
    let launched = god.seen.len();
    let mut from = launched;
    let count = plan.matches.to_string();
    let start = tokio::time::Instant::now();
    let mut flight = launch_env(
        &rig,
        &config,
        plan.name,
        &["--arena", "red", "--matches", &count],
        plan.env,
    );

    // Each match's end, then the poses for the next.
    let mut heard = 0;
    let mut early = None;
    while heard < plan.matches {
        let left = plan.deadline.saturating_sub(start.elapsed());
        let limit = left.min(MATCH_WAIT);
        match try_await_line(&mut god, from, " is over (", &mut flight, limit).await {
            Ok(past) => {
                heard += 1;
                let ended = god.seen.get(..past - " is over (".len()).and_then(ended_id);
                from = past;
                if heard == plan.matches {
                    break;
                }
                for spec in POSES {
                    if let Err(answer) = try_pose(&mut god, spec).await {
                        match ended {
                            Some(n) => {
                                println!(
                                    "the pose {spec} after match {n} was refused: {}",
                                    answer.trim()
                                );
                                refused.push(Some(n));
                            }
                            // The centre's line always names its match; one
                            // that did not cannot key a flag.
                            None => println!(
                                "the pose {spec} after an unnumbered match end was refused: {}",
                                answer.trim()
                            ),
                        }
                    }
                }
            }
            Err(why) => {
                early = Some(early_why(why, limit, plan.deadline));
                break;
            }
        }
    }

    let partial = try_land(flight, Duration::from_secs(120)).await;
    if let Some(keep) = std::env::var_os(KEEP_ENV) {
        let keep = PathBuf::from(keep);
        let _ = std::fs::create_dir_all(&keep);
        let _ = std::fs::write(
            keep.join(format!("{}.jsonl", plan.name)),
            &partial.transcript_text,
        );
        if let Some(text) = &partial.report_text {
            let _ = std::fs::write(keep.join(format!("{}.json", plan.name)), text);
        }
    }
    let ends = match_ends(god.seen.get(launched..).unwrap_or(""));
    god.close().await;
    drop(rig);
    Posed {
        partial,
        ends,
        heard,
        early,
        refused,
    }
}

// ---------------------------------------------------------------------------
// Cut AL.3: the lead's reading of a posed flight.
// ---------------------------------------------------------------------------

/// **What a posed flight's transcript and report say** (cut AL.3), as the lead
/// reads a run: what was flown, the guns' cadence and what the facet shown
/// cost. Printed by both posed lanes ([`Reading::lines`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    /// The `arena-picture` records, counted by their `data.doctrine.stance`…
    pub stances: BTreeMap<String, usize>,
    /// …of this many `arena-picture` records.
    pub pictures: usize,
    /// The lead's count ([`mush_bot::arena::report::fire_gaps`]) over the
    /// `arena-send` records' `(data.lane_ms, data.line)`.
    pub gaps: BTreeMap<String, mush_bot::arena::report::GapStats>,
    /// The same over the shots that left
    /// ([`mush_bot::arena::report::shot_gaps`]): each observation line
    /// `Firing {w}.` at its record's `elapsed_ms`.
    pub shots: BTreeMap<String, mush_bot::arena::report::GapStats>,
    /// The `arena-send` lines starting `FIRE `, by the word after it.
    pub fire_lines: BTreeMap<String, usize>,
    /// The observation lines `FIRE {w} refused. Still cycling…`, by `{w}`.
    pub still_cycling: BTreeMap<String, usize>,
    /// The report's `arena.matches[*].hits_taken_by_facet`, summed; `None`
    /// when there is no report.
    pub facets: Option<BTreeMap<String, u32>>,
}

/// **Read a posed flight** (cut AL.3): `transcript` is the raw JSON-lines
/// transcript (a torn line, the last one above all, is skipped), `report` the
/// bot's report when it left one.
pub fn reading(transcript: &str, report: Option<&serde_json::Value>) -> Reading {
    let mut reading = Reading::default();
    let mut sends: Vec<(u64, String)> = Vec::new();
    let mut acks: Vec<(u64, String)> = Vec::new();
    for line in transcript.lines() {
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match record["kind"].as_str() {
            Some("arena-picture") => {
                reading.pictures += 1;
                if let Some(stance) = record["data"]["doctrine"]["stance"].as_str() {
                    *reading.stances.entry(stance.to_owned()).or_default() += 1;
                }
            }
            Some("arena-send") => {
                let (Some(at), Some(sent)) = (
                    record["data"]["lane_ms"].as_u64(),
                    record["data"]["line"].as_str(),
                ) else {
                    continue;
                };
                if let Some(weapon) = sent
                    .strip_prefix("FIRE ")
                    .and_then(|rest| rest.split_whitespace().next())
                {
                    *reading.fire_lines.entry(weapon.to_owned()).or_default() += 1;
                }
                sends.push((at, sent.to_owned()));
            }
            Some("observation") => {
                let (Some(at), Some(text)) =
                    (record["elapsed_ms"].as_u64(), record["data"].as_str())
                else {
                    continue;
                };
                for line in text.lines().map(str::trim) {
                    if line.starts_with("Firing ") {
                        acks.push((at, line.to_owned()));
                    }
                    if let Some((weapon, said)) = line
                        .strip_prefix("FIRE ")
                        .and_then(|rest| rest.split_once(' '))
                        && said.starts_with("refused. Still cycling")
                    {
                        *reading.still_cycling.entry(weapon.to_owned()).or_default() += 1;
                    }
                }
            }
            _ => {}
        }
    }
    reading.gaps =
        mush_bot::arena::report::fire_gaps(sends.iter().map(|(at, line)| (*at, line.as_str())));
    reading.shots =
        mush_bot::arena::report::shot_gaps(acks.iter().map(|(at, line)| (*at, line.as_str())));
    reading.facets = report.map(|report| {
        let mut facets: BTreeMap<String, u32> = BTreeMap::new();
        let matches = report["arena"]["matches"]
            .as_array()
            .map_or(&[][..], Vec::as_slice);
        for flown in matches {
            let Some(split) = flown["hits_taken_by_facet"].as_object() else {
                continue;
            };
            for (key, count) in split {
                let count = count
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(0);
                if count == 0 {
                    continue;
                }
                let total = facets.entry(key.clone()).or_default();
                *total = total.saturating_add(count);
            }
        }
        facets
    });
    reading
}

/// `{key} {n}, …` by count, highest first, then by key.
fn by_count<K: std::fmt::Display + Ord, N: std::fmt::Display + Ord + Copy>(
    counts: &BTreeMap<K, N>,
) -> String {
    let mut pairs: Vec<(&K, N)> = counts.iter().map(|(k, n)| (k, *n)).collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    pairs
        .iter()
        .map(|(k, n)| format!("{k} {n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl Reading {
    /// **The reading lines** (cut AL.3), in this order:
    ///
    /// * `ENGAGEMENT: {e}; flown {stance} {k}, {stance} {k} of {n} picture
    ///   records` — the engagement the lane was given, then what its picture
    ///   records flew, by count, highest first, then by word (`flown nothing`
    ///   when no record carried a stance);
    /// * `FIRE GAP {w}: p10 {a} / p50 {b} / p90 {c} ms over {n} gaps`, one per
    ///   weapon by id (the lead's count);
    /// * `SHOT GAP {w}: p10 {a} / p50 {b} / p90 {c} ms over {n} gaps`, one per
    ///   weapon by id (the shots that left);
    /// * `STILL CYCLING {w}: {k} of {n} FIRE lines`, one per weapon by id;
    /// * `HITS TAKEN BY FACET: {key} {n}, …` by count, highest first, then by
    ///   key; `HITS TAKEN BY FACET: none` when the report holds no facet hit,
    ///   `HITS TAKEN BY FACET: no report` when there is no report.
    pub fn lines(&self, engagement: &str) -> Vec<String> {
        let flown = if self.stances.is_empty() {
            "nothing".to_owned()
        } else {
            by_count(&self.stances)
        };
        let mut lines = vec![format!(
            "ENGAGEMENT: {engagement}; flown {flown} of {} picture records",
            self.pictures
        )];
        for (head, gaps) in [("FIRE GAP", &self.gaps), ("SHOT GAP", &self.shots)] {
            for (weapon, g) in gaps {
                lines.push(format!(
                    "{head} {weapon}: p10 {} / p50 {} / p90 {} ms over {} gaps",
                    g.p10_ms, g.p50_ms, g.p90_ms, g.gaps
                ));
            }
        }
        let weapons: BTreeSet<&String> = self
            .fire_lines
            .keys()
            .chain(self.still_cycling.keys())
            .collect();
        for weapon in weapons {
            lines.push(format!(
                "STILL CYCLING {weapon}: {} of {} FIRE lines",
                self.still_cycling.get(weapon).copied().unwrap_or(0),
                self.fire_lines.get(weapon).copied().unwrap_or(0)
            ));
        }
        lines.push(match &self.facets {
            None => "HITS TAKEN BY FACET: no report".to_owned(),
            Some(facets) if facets.is_empty() => "HITS TAKEN BY FACET: none".to_owned(),
            Some(facets) => format!("HITS TAKEN BY FACET: {}", by_count(facets)),
        });
        lines
    }
}
