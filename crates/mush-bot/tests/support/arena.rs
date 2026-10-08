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
//! **The lanes** (cut AK.3; generalised in cut AN.1) — the posed win-rate
//! lane, the muster lane and the live commander lane — fly through one
//! function here, [`fly_lane`]: the same rig, the plan's poses before every
//! match (the posed lanes' two, 500 m apart, or none for the muster lane,
//! which flies the shipped match from the muster), the plan's cap, one
//! deadline for the whole flight, and a result ([`Posed`]) that always holds
//! what there is to print and keep, however the flight ended.
//!
//! **The pooled measure** (cut AO.3): every win-rate figure is printed with a
//! 95 per cent Wilson interval ([`interval_line`]); the `KNOBS` line is the
//! value flown, read from the shipped defaults ([`knobs_flown`]) and kept
//! beside each kept run ([`keep_knobs`]); and kept runs are read together
//! ([`reading_of`], [`pool_lines`]), refusing runs flown with different
//! knobs.
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
/// from the verb's own refusal sentences as text. **The first is the shape**:
/// since the game's cut AP.4 every refusal opens `POSE refused. `, the
/// cockpit's `{ORDER} refused. …` form, so that one needle alone tells a
/// refusal from the `opens at cell` answer. The other eight are kept, each
/// for the refusal it names, so a game that dropped the prefix would still
/// be read and a test can still say which refusal it was: an unknown callsign
/// (`There is no slot {X} on this centre.`, the verb's first refusal; cut
/// AM.3), the centre running, a cell off the window, a cell that is
/// impassable or has no way out, a cell already another tank's opening cell,
/// a heading out of range, a malformed line, and a server without the arena.
pub const POSE_REFUSALS: [&str; 9] = [
    "POSE refused. ",
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
// Cut AK.3 and cut AN.1: the lanes (the posed win-rate lane, the muster lane
// and the live commander lane).
// ---------------------------------------------------------------------------

/// **Each posed match's length** before the centre calls it a timeout (cut
/// AK.3, 16c Q-AK3): 420 s, so a fight the pilot is winning is finished
/// rather than cut off (on AJ's 180 s cap five of ten timed out, every one
/// one-sided). A timeout is its own column, never a loss.
pub const MAX_DURATION_MS: u32 = 420_000;
/// `[agent] max_seconds` in a posed lane's config: in arena mode it bounds
/// each match (a claim starts it again), so it sits above the cap.
pub const MAX_SECONDS: u64 = 540;
/// The longest God waits for one posed match's end, boarding and startup
/// included: [`match_wait`] of [`MAX_DURATION_MS`].
pub const MATCH_WAIT: Duration = Duration::from_secs(600);
/// **Each muster match's length** before the centre calls it a timeout (cut
/// AN.1): 600 s. From the muster, 2 430 m apart against a 600 m sight, the
/// opening alone can take minutes before anything is in sight.
pub const MUSTER_DURATION_MS: u32 = 600_000;
/// `[agent] max_seconds` in the muster lane's config: above its cap, as
/// [`MAX_SECONDS`] is above the posed one.
pub const MUSTER_MAX_SECONDS: u64 = 720;

/// **The longest God waits for one match's end** on a cap of
/// `max_duration_ms` (cut AN.1): the cap and 180 s for boarding and startup.
/// 600 s for the posed cap (so [`MATCH_WAIT`] is the posed lanes' wait), 780 s
/// for the muster's.
pub fn match_wait(max_duration_ms: u32) -> Duration {
    Duration::from_millis(u64::from(max_duration_ms)) + Duration::from_secs(180)
}
/// The two poses, 500 m apart on the east-west road, facing each other.
pub const POSES: [&str; 2] = ["RA=100 97 90", "BA=150 97 270"];
/// The variable naming a directory a lane keeps its transcript
/// (`{name}.jsonl`) and report (`{name}.json`) in (the rig's own directory
/// is removed).
pub const KEEP_ENV: &str = "MUSH_BOT_WIN_RATE_KEEP";
/// The variable that picks the engagement the win-rate lanes fly (cut AL.3;
/// moved into the rig in cut AN.1, where the posed and the muster lane both
/// read it).
pub const ENGAGEMENT_ENV: &str = "MUSH_BOT_WIN_RATE_ENGAGEMENT";

/// **The engagement a win-rate lane flies** (cut AL.3), from
/// [`ENGAGEMENT_ENV`]'s value: unset or `hold` → `hold`, `orbit` → `orbit`,
/// and anything else a panic, before anything is launched.
pub fn engagement(value: Option<&str>) -> &'static str {
    match value {
        None | Some("hold") => "hold",
        Some("orbit") => "orbit",
        Some(other) => panic!("MUSH_BOT_WIN_RATE_ENGAGEMENT is hold or orbit, not {other}"),
    }
}

/// The variable naming the pilot's knobs a win-rate lane turns off (cut
/// AN.2): a comma list of `opening`, `reapproach`, (cut AN.3) `follow`,
/// `lattice` or `all`.
pub const KNOBS_OFF_ENV: &str = "MUSH_BOT_WIN_RATE_OFF";

/// The knobs a lane can turn off, in the `KNOBS` line's order (cut AN.3).
pub const KNOBS: [&str; 4] = ["opening", "reapproach", "follow", "lattice"];

/// **The knobs a lane turns off** (cut AN.2), from [`KNOBS_OFF_ENV`]'s value:
/// unset (or naming nothing) turns none off; `opening`, `reapproach`, (cut
/// AN.3) `follow` and `lattice` that knob, `all` all four; any other word is
/// a panic, before anything is launched.
pub fn knobs_off(value: Option<&str>) -> BTreeSet<&'static str> {
    let mut off = BTreeSet::new();
    for word in value
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|word| !word.is_empty())
    {
        match word {
            "all" => off.extend(KNOBS),
            other => match KNOBS.iter().find(|knob| **knob == other) {
                Some(knob) => {
                    off.insert(*knob);
                }
                None => panic!(
                    "MUSH_BOT_WIN_RATE_OFF names opening, reapproach, follow, lattice or all, not \
                     {other}"
                ),
            },
        }
    }
    off
}

/// **One knob as a flight flies it** (cut AO.3): its word in [`KNOBS`], the
/// `[arena]` line that turns it off, whether the shipped default has it on,
/// whether the flight names it off, and whether this flight has it on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlownKnob {
    pub knob: &'static str,
    pub off_line: &'static str,
    pub default_on: bool,
    pub named: bool,
    pub on: bool,
}

/// **The knobs as flown** (cut AO.3), in [`KNOBS`]' order: each knob's
/// shipped default read from `defaults` (`opening` on when it is `wait`;
/// `reapproach`, `follow_broken` and `lattice` as they stand), less what
/// `off` names. A knob whose default is already off is flown off whether
/// named or not. The one helper both [`knobs_line_from`] and [`extra_from`]
/// read, so the `KNOBS` line and the config a lane writes never disagree.
pub fn knobs_flown(
    defaults: &mush_bot::arena::settings::Settings,
    off: &BTreeSet<&str>,
) -> [FlownKnob; 4] {
    use mush_bot::arena::settings::Opening;
    let knob = |knob: &'static str, off_line: &'static str, default_on: bool| FlownKnob {
        knob,
        off_line,
        default_on,
        named: off.contains(knob),
        on: default_on && !off.contains(knob),
    };
    [
        knob(
            "opening",
            "opening = \"search\"\n",
            defaults.opening == Opening::Wait,
        ),
        knob("reapproach", "reapproach = false\n", defaults.reapproach),
        knob("follow", "follow_broken = false\n", defaults.follow_broken),
        knob("lattice", "lattice = false\n", defaults.lattice),
    ]
}

/// **The bot's extra config** for the engagement `e` (cut AL.3): the lanes'
/// 5 s start grace, and `[arena] engagement`; and (cut AN.2) `opening =
/// "search"` and `reapproach = false`, (cut AN.3) `follow_broken = false`
/// and `lattice = false` for the knobs `off` names, in that order. With none
/// named it is byte for byte what it was. [`extra_from`] on the shipped
/// defaults (`Settings::default()`, cut AO.3).
pub fn extra(e: &str, off: &BTreeSet<&str>) -> String {
    extra_from(&mush_bot::arena::settings::Settings::default(), e, off)
}

/// [`extra`] on the defaults of the caller's (cut AO.3), through
/// [`knobs_flown`]: each named knob's line turning it off, as before,
/// whatever its default. A knob whose default is already off is written off
/// too when named, which changes nothing (the bot's own default flies it
/// off), and gets no line when not named (it is flown off all the same).
/// The bytes depend only on what is named, so they are what [`extra`] wrote
/// before whatever the defaults.
pub fn extra_from(
    defaults: &mush_bot::arena::settings::Settings,
    e: &str,
    off: &BTreeSet<&str>,
) -> String {
    let mut extra = format!("\n[arena]\nlead_start_grace_ms = 5000\nengagement = \"{e}\"\n");
    for flown in knobs_flown(defaults, off) {
        if flown.named {
            extra.push_str(flown.off_line);
        }
    }
    extra
}

/// **The `KNOBS` line** both win-rate lanes print after `ENGAGEMENT` (cut
/// AN.2; cut AN.3): `KNOBS: opening {wait|search}, reapproach {on|off},
/// follow {on|off}, lattice {on|off}`. Cut AO.3: the value **flown**, the
/// shipped default (`Settings::default()`) less what `off` names
/// ([`knobs_line_from`]).
pub fn knobs_line(off: &BTreeSet<&str>) -> String {
    knobs_line_from(&mush_bot::arena::settings::Settings::default(), off)
}

/// [`knobs_line`] on the defaults of the caller's (cut AO.3), through
/// [`knobs_flown`]. While every default is on this is byte for byte what
/// [`knobs_line`] printed before.
pub fn knobs_line_from(
    defaults: &mush_bot::arena::settings::Settings,
    off: &BTreeSet<&str>,
) -> String {
    let [opening, reapproach, follow, lattice] = knobs_flown(defaults, off);
    let on_off = |flown: FlownKnob| if flown.on { "on" } else { "off" };
    format!(
        "KNOBS: opening {}, reapproach {}, follow {}, lattice {}",
        if opening.on { "wait" } else { "search" },
        on_off(reapproach),
        on_off(follow),
        on_off(lattice)
    )
}

/// The file a lane keeps its `KNOBS` line in, beside its transcript and
/// report (cut AO.3): `{stem}.knobs`.
pub fn knobs_file(stem: &str) -> String {
    format!("{stem}.knobs")
}

/// **Keep the `KNOBS` line** (cut AO.3): when [`KEEP_ENV`] names a
/// directory, write `line` and a newline to [`knobs_file`]`(stem)` there,
/// so a pool of kept runs ([`pool_lines`]) can refuse runs flown with
/// different knobs. Nothing is written, and nothing panics, otherwise.
pub fn keep_knobs(stem: &str, line: &str) {
    if let Some(keep) = std::env::var_os(KEEP_ENV) {
        let keep = PathBuf::from(keep);
        let _ = std::fs::create_dir_all(&keep);
        let _ = std::fs::write(keep.join(knobs_file(stem)), format!("{line}\n"));
    }
}

/// What a lane flies (cut AK.3's posed plan, generalised in cut AN.1): the
/// flight's name (and the kept files' stem), how many matches, the whole
/// flight's deadline measured from the launch, the config's extra tables,
/// variables for the child ([`launch_env`]), the poses God types before the
/// first match and after each end, the centre's cap on a match, and the
/// bot's `[agent] max_seconds`.
pub struct LanePlan<'a> {
    pub name: &'a str,
    pub matches: usize,
    pub deadline: Duration,
    pub extra: &'a str,
    pub env: &'a [(&'a str, &'a str)],
    /// `@arena/pose` specs: `&POSES` for the posed lanes, `&[]` for the
    /// muster lane (nothing typed between matches).
    pub poses: &'a [&'a str],
    /// `match.max_duration_ms` on the rig: [`MAX_DURATION_MS`] posed,
    /// [`MUSTER_DURATION_MS`] from the muster.
    pub max_duration_ms: u32,
    /// `[agent] max_seconds`: [`MAX_SECONDS`] posed, [`MUSTER_MAX_SECONDS`]
    /// from the muster.
    pub max_seconds: u64,
}

/// **What a lane's flight left**, however it ended (cut AK.3). A flight with
/// no poses never has a refused one.
pub struct Posed {
    /// What the bot left ([`try_land`]).
    pub partial: Partial,
    /// God's own `Match {n} is over ({word})` lines since the launch, by id.
    pub ends: BTreeMap<u64, String>,
    /// The match ends God waited for and heard.
    pub heard: usize,
    /// Why the flight ended before its last match's end, when it did:
    /// `the bot ended`, `no match end inside {n} s` (600 posed, 780 from
    /// the muster), `the {m}-minute deadline` or `the watching session
    /// closed`.
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

/// **A lane's results** for red (cut AK.3): won, lost, drew, timed out,
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

    /// **Every match counted** (cut AO.3): won, lost, drew, timed out and
    /// other together, the `n` of [`interval_line`].
    pub fn total(&self) -> u64 {
        self.won + self.lost + self.drew + self.timed_out + self.other
    }

    /// Two outcomes' columns summed (cut AO.3; the pool's figure).
    pub fn plus(self, other: Outcome) -> Outcome {
        Outcome {
            won: self.won + other.won,
            lost: self.lost + other.lost,
            drew: self.drew + other.drew,
            timed_out: self.timed_out + other.timed_out,
            other: self.other + other.other,
        }
    }
}

/// **A 95 per cent Wilson interval** on `wins` of `n` (cut AO.3), in whole
/// per cent: with `z` = 1.96, `p = wins / n`, `d = 1 + z² / n`, `c = p + z² /
/// (2n)` and `s = z × √(p(1 − p) / n + z² / (4n²))`, the bounds are `(c − s) /
/// d` and `(c + s) / d`, each clamped to `[0, 1]` and rounded half away from
/// zero. `None` for `n = 0`. Ten matches cannot tell 7 of 10 from 5 of 10
/// (40 to 89 per cent against 24 to 76), so every win-rate figure is printed
/// with it and no difference is called while two intervals overlap.
pub fn wilson_95(wins: u64, n: u64) -> Option<(u32, u32)> {
    const Z: f64 = 1.96;
    if n == 0 {
        return None;
    }
    // Counts of matches: far inside f64's exact integers.
    let n = n as f64;
    let p = wins as f64 / n;
    let z2 = Z * Z;
    let d = 1.0 + z2 / n;
    let c = p + z2 / (2.0 * n);
    let s = Z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
    let per_cent = |bound: f64| ((bound / d).clamp(0.0, 1.0) * 100.0).round() as u32;
    Some((per_cent(c - s), per_cent(c + s)))
}

/// **The `WIN RATE INTERVAL` line** (cut AO.3), printed straight after every
/// `WIN RATE` line: `WIN RATE INTERVAL: {w} of {n}, 95 per cent Wilson {lo}
/// to {hi} per cent` ([`wilson_95`]), or `WIN RATE INTERVAL: no matches`.
pub fn interval_line(wins: u64, n: u64) -> String {
    match wilson_95(wins, n) {
        Some((lo, hi)) => {
            format!("WIN RATE INTERVAL: {wins} of {n}, 95 per cent Wilson {lo} to {hi} per cent")
        }
        None => "WIN RATE INTERVAL: no matches".to_owned(),
    }
}

/// **A lane's two figure lines** (cut AO.3): `outcome`'s
/// [`Outcome::line`] under `head` of `of`, then its [`interval_line`] over
/// [`Outcome::total`]. Both win-rate lanes print these.
pub fn win_rate_lines(outcome: &Outcome, head: &str, of: usize) -> [String; 2] {
    [
        outcome.line(head, of),
        interval_line(outcome.won, outcome.total()),
    ]
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

/// **Why a lane's flight ended early** (cut AK.3), from how its last wait
/// ended: `the bot ended`; at a wait whose `limit` was cut short of
/// `match_wait` (the flight's whole wait for one match, [`match_wait`] of its
/// cap; cut AN.1) by the flight's own `deadline`, `the {m}-minute deadline`;
/// `the watching session closed` when God's own session did (cut AM.3);
/// otherwise `no match end inside {s} s`, `s` being `match_wait`'s seconds.
pub fn early_why(
    ended: Ended,
    limit: Duration,
    deadline: Duration,
    match_wait: Duration,
) -> String {
    match ended {
        Ended::Bot => "the bot ended".to_owned(),
        Ended::Closed => "the watching session closed".to_owned(),
        Ended::Timeout if limit < match_wait => {
            format!("the {}-minute deadline", deadline.as_secs() / 60)
        }
        Ended::Timeout => format!("no match end inside {} s", match_wait.as_secs()),
    }
}

/// **Fly a lane** (cut AK.3's posed flight, generalised in cut AN.1; every
/// lane calls it): boot the rig with the AI fill and the plan's cap
/// ([`LanePlan::max_duration_ms`]); God types the plan's poses (the posed
/// lanes' `RA` and `BA`, [`POSES`]; none from the muster) and walks to the
/// Simulator Lobby; the bot flies `--arena red --matches {n}` on the plan's
/// `[agent] max_seconds` and extra tables; and after each `Match {n} is over
/// (` God hears, God types the poses again for the next (the end consumed and
/// cleared the last ones, the new ones survive the bot's re-claim a second
/// later, and the lanes' `lead_start_grace_ms` of 5 s keeps its `SIM START`
/// behind them). With no poses God types nothing between matches: the bot's
/// own claim puts the tanks back at muster from the second match on, and the
/// world is fresh for the first.
///
/// **One deadline, measured from the launch**: each wait's limit is the
/// lesser of [`match_wait`] of the plan's cap and what is left of it, so
/// running out ends the loop like any other early end. The loop never panics: a bot that ends, a
/// match that never ends, the deadline and (cut AM.3) God's own session
/// closing each end it, remembered in [`Posed::early`], and a pose on a closed
/// session is refused, not a panic ([`try_pose`]). Then the bot is landed ([`try_land`], 120 s), and when
/// [`KEEP_ENV`] names a directory its transcript (raw) and report (when there
/// is one) are copied there **before** anything is printed or asserted.
/// A refused pose is printed and recorded ([`Posed::refused`]), and the
/// flight carries on.
pub async fn fly_lane(plan: &LanePlan<'_>) -> Posed {
    let mut cfg = arena_config();
    cfg.match_.startup_ms = 2_000;
    cfg.match_.board_timeout_ms = 60_000;
    cfg.match_.max_duration_ms = plan.max_duration_ms;
    let rig = boot(cfg, &[("pilot", "Pilot")]).await;
    let wait = match_wait(plan.max_duration_ms);

    let mut god = Client::login(&rig, GOD_ACCOUNT, GOD_PASSWORD, "God").await;
    let mut refused = Vec::new();
    for spec in plan.poses {
        if let Err(answer) = try_pose(&mut god, spec).await {
            println!(
                "the pose {spec} before the first match was refused: {}",
                answer.trim()
            );
            refused.push(None);
        }
    }
    walk_to_lobby(&mut god).await;

    let config = bot_config_with(&rig, "pilot+Pilot", plan.max_seconds, plan.extra);
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
        let limit = left.min(wait);
        match try_await_line(&mut god, from, " is over (", &mut flight, limit).await {
            Ok(past) => {
                heard += 1;
                let ended = god.seen.get(..past - " is over (".len()).and_then(ended_id);
                from = past;
                if heard == plan.matches {
                    break;
                }
                for spec in plan.poses {
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
                early = Some(early_why(why, limit, plan.deadline, wait));
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
// Cut AL.3: the lead's reading of a lane's flight (cut AN.1: and who saw whom
// first, the opening, the front's share).
// ---------------------------------------------------------------------------

/// **What a lane's transcript and report say** (cut AL.3), as the lead reads
/// a run: what was flown, the guns' cadence and what the facet shown cost;
/// and (cut AN.1) who saw whom first, how long the opening took and how much
/// of the punishment the front took. Printed by every lane
/// ([`Reading::lines`]).
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
    /// **Each match's first times** (cut AN.1), in the report's order:
    /// `(first_contact_ms, first_hit_taken_ms, first_hit_scored_ms)`, each
    /// milliseconds after the match's power online, a missing key (or a
    /// `null`) read as `None`; `None` when there is no report.
    pub firsts: Option<Vec<FirstTimes>>,
    /// **What the pilot's own rules did** (cut AN.2): the transcript's
    /// `arena` records that begin `Opening: holding near our entry`,
    /// `Opening: stepping off the road`, `Firing from the halt at `, `Opening
    /// the range on ` and `Out of ` (only with `'s sight at ` in it: the
    /// lobby's `Out of the tank; boarding again.` is an `arena` record too),
    /// and (cut AN.3) `Following `, `Waiting for ` and `Firing on the arena's
    /// lattice`, and (cut AO.1) `Opening: moving in `, `No room behind: `, `No
    /// room behind for the return: ` and `Could not open the range on `, and
    /// (cut AO.2) those containing `is off the lattice clock` (the bench),
    /// counted in that order ([`PilotRules`]).
    pub rules: PilotRules,
}

/// **The pilot rules' counts** (cut AN.2), as [`Reading::rules`] reads them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PilotRules {
    pub opening: usize,
    pub off_the_road: usize,
    pub halt: usize,
    pub reapproach: usize,
    pub approach_wait: usize,
    /// Cut AN.3: broken machines followed, returns waited for, and matches
    /// whose guns went on the arena's lattice.
    pub follow: usize,
    pub return_wait: usize,
    pub lattice: usize,
    /// Cut AO.1: the opening's moves in for room, the re-approach's and the
    /// return's back-offs held where the ground behind ran out, and the
    /// re-approaches given up at their bound.
    pub inward: usize,
    pub room_holds: usize,
    pub return_room_holds: usize,
    pub gave_up: usize,
    /// Cut AO.2: the lattice clock's benches (`The {w} is off the lattice
    /// clock for {s} s: refused twice.`).
    pub benched: usize,
}

impl PilotRules {
    /// Count one `arena` record's text.
    fn count(&mut self, text: &str) {
        if text.starts_with("Opening: holding near our entry") {
            self.opening += 1;
        } else if text.starts_with("Opening: stepping off the road") {
            self.off_the_road += 1;
        } else if text.starts_with("Firing from the halt at ") {
            self.halt += 1;
        } else if text.starts_with("Opening the range on ") {
            self.reapproach += 1;
        } else if text.starts_with("Out of ") && text.contains("'s sight at ") {
            self.approach_wait += 1;
        } else if text.starts_with("Following ") {
            self.follow += 1;
        } else if text.starts_with("Waiting for ") {
            self.return_wait += 1;
        } else if text.starts_with("Firing on the arena's lattice") {
            self.lattice += 1;
        } else if text.starts_with("Opening: moving in ") {
            self.inward += 1;
        } else if text.starts_with("No room behind: ") {
            self.room_holds += 1;
        } else if text.starts_with("No room behind for the return: ") {
            self.return_room_holds += 1;
        } else if text.starts_with("Could not open the range on ") {
            self.gave_up += 1;
        } else if text.contains("is off the lattice clock") {
            self.benched += 1;
        }
    }

    /// `PILOT RULES: opening {a}, off the road {b}, halt {c}, reapproach
    /// {d}, approach wait {e}, follow {f}, return wait {g}, lattice {h}` and
    /// (cut AO.1) `, inward {i}, room holds {k}, return room holds {r}, gave
    /// up {u}` and (cut AO.2) `, benched {n}`.
    pub fn line(&self) -> String {
        format!(
            "PILOT RULES: opening {}, off the road {}, halt {}, reapproach {}, approach wait {}, \
             follow {}, return wait {}, lattice {}, inward {}, room holds {}, return room holds \
             {}, gave up {}, benched {}",
            self.opening,
            self.off_the_road,
            self.halt,
            self.reapproach,
            self.approach_wait,
            self.follow,
            self.return_wait,
            self.lattice,
            self.inward,
            self.room_holds,
            self.return_room_holds,
            self.gave_up,
            self.benched
        )
    }
}

/// One match's `(first_contact_ms, first_hit_taken_ms, first_hit_scored_ms)`
/// (cut AN.1).
pub type FirstTimes = (Option<u64>, Option<u64>, Option<u64>);

/// **Read a lane's flight** (cut AL.3): `transcript` is the raw JSON-lines
/// transcript (a torn line, the last one above all, is skipped), `report` the
/// bot's report when it left one. Cut AO.3: [`reading_of`] over this one run,
/// its lines byte for byte what they were.
pub fn reading(transcript: &str, report: Option<&serde_json::Value>) -> Reading {
    reading_of(&[(transcript, report)])
}

/// How far apart [`reading_of`] sets two kept runs' clocks (cut AO.3), in
/// milliseconds: 10¹², about 31 years, so every gap that spans two runs is
/// far past `FIRE_GAP_WINDOW_MS` and is dropped.
pub const POOL_RUN_OFFSET_MS: u64 = 1_000_000_000_000;

/// **Read several kept runs as one** (cut AO.3; the pool lane's reading):
/// each `(transcript, report)` read as [`reading`] reads one, run `k`'s times
/// (`lane_ms` of its sends, `elapsed_ms` of its observations) offset by `k ×`
/// [`POOL_RUN_OFFSET_MS`], so no gap spans two runs. The stances, picture
/// records, `FIRE` lines, still-cycling counts and [`PilotRules`] are summed;
/// the facets are summed (`None` only when no run has a report); the first
/// times are concatenated in run order (`None` only when no run has a
/// report).
pub fn reading_of(runs: &[(&str, Option<&serde_json::Value>)]) -> Reading {
    let mut reading = Reading::default();
    let mut sends: Vec<(u64, String)> = Vec::new();
    let mut acks: Vec<(u64, String)> = Vec::new();
    let mut facets: Option<BTreeMap<String, u32>> = None;
    let mut firsts: Option<Vec<FirstTimes>> = None;
    for (k, (transcript, report)) in runs.iter().enumerate() {
        let offset = u64::try_from(k)
            .unwrap_or(u64::MAX)
            .saturating_mul(POOL_RUN_OFFSET_MS);
        read_run(&mut reading, &mut sends, &mut acks, transcript, offset);
        if let Some(report) = report {
            add_facets(facets.get_or_insert_with(BTreeMap::new), report);
            firsts
                .get_or_insert_with(Vec::new)
                .extend(first_times(report));
        }
    }
    reading.gaps =
        mush_bot::arena::report::fire_gaps(sends.iter().map(|(at, line)| (*at, line.as_str())));
    reading.shots =
        mush_bot::arena::report::shot_gaps(acks.iter().map(|(at, line)| (*at, line.as_str())));
    reading.facets = facets;
    reading.firsts = firsts;
    reading
}

/// One run's transcript into `reading`'s counts, and its sends and `Firing`
/// acks (each at its time plus `offset`) into `sends` and `acks`.
fn read_run(
    reading: &mut Reading,
    sends: &mut Vec<(u64, String)>,
    acks: &mut Vec<(u64, String)>,
    transcript: &str,
    offset: u64,
) {
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
                sends.push((at.saturating_add(offset), sent.to_owned()));
            }
            Some("arena") => {
                if let Some(text) = record["data"].as_str() {
                    reading.rules.count(text);
                }
            }
            Some("observation") => {
                let (Some(at), Some(text)) =
                    (record["elapsed_ms"].as_u64(), record["data"].as_str())
                else {
                    continue;
                };
                for line in text.lines().map(str::trim) {
                    if line.starts_with("Firing ") {
                        acks.push((at.saturating_add(offset), line.to_owned()));
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
}

/// A report's `arena.matches[*].hits_taken_by_facet` added into `facets`
/// (a zero or unreadable count adds nothing).
fn add_facets(facets: &mut BTreeMap<String, u32>, report: &serde_json::Value) {
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
}

/// A report's matches' first times, in the report's order.
fn first_times(report: &serde_json::Value) -> Vec<FirstTimes> {
    report["arena"]["matches"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|flown| {
            (
                flown["first_contact_ms"].as_u64(),
                flown["first_hit_taken_ms"].as_u64(),
                flown["first_hit_scored_ms"].as_u64(),
            )
        })
        .collect()
}

/// **Percentiles in whole seconds** (cut AN.1): the 10th, 50th and 90th of
/// `ms`, each the sorted values' element at `floor(q × (len − 1))` (the rule
/// of `mush_bot::arena::report::GapStats`), as milliseconds / 1 000, floored;
/// `None` for no value.
fn seconds_p10_p50_p90(mut ms: Vec<u64>) -> Option<(u64, u64, u64)> {
    ms.sort_unstable();
    let last = ms.len().checked_sub(1)?;
    let at = |tenths: usize| ms.get(last * tenths / 10).copied().unwrap_or(0) / 1_000;
    Some((at(1), at(5), at(9)))
}

/// **The `SEEN FIRST` line** (cut AN.1) over each match's first times.
///
/// **Why the lead.** The bot cannot know when the machine saw it. The machine
/// closes to about 420 m before it shoots (`help simulator`) and both sights
/// reach past that, so a contact before the first hit taken is nearly certain
/// and a count of it alone saturates. What the bot can measure is the lead:
/// the seconds from its own first contact to its first hit taken. A long lead
/// is the bot seeing first; a short one is the machine finding it first.
///
/// `contact before hit` is a match whose first contact came before its first
/// hit taken (or with none taken), `hit before contact` one whose first hit
/// taken came at or before its first contact (or with no contact), `neither`
/// one with neither; the lead's percentiles are over the matches with both
/// and the contact first (`lead none` with none); `shot first` is a match
/// whose first hit scored came before its first hit taken (or with none
/// taken), `shot at first` one whose first hit taken came at or before its
/// first hit scored (or with none scored). A match with neither hit is in
/// neither of the last two counts.
fn seen_first_line(firsts: &[FirstTimes]) -> String {
    let (mut before, mut after, mut neither, mut first, mut at_first) = (0, 0, 0, 0, 0);
    let mut leads = Vec::new();
    for &(contact, taken, scored) in firsts {
        match (contact, taken) {
            (Some(c), Some(h)) if c < h => {
                before += 1;
                leads.push(h - c);
            }
            (Some(_), None) => before += 1,
            (_, Some(_)) => after += 1,
            (None, None) => neither += 1,
        }
        match (scored, taken) {
            (Some(s), Some(h)) if s < h => first += 1,
            (Some(_), None) => first += 1,
            (_, Some(_)) => at_first += 1,
            (None, None) => {}
        }
    }
    let k = leads.len();
    let lead = match seconds_p10_p50_p90(leads) {
        Some((p10, p50, p90)) => format!("lead p10 {p10} / p50 {p50} / p90 {p90} s over {k}"),
        None => "lead none".to_owned(),
    };
    format!(
        "SEEN FIRST: contact before hit {before}, hit before contact {after}, neither {neither} of {} matches; {lead}; shot first {first}, shot at first {at_first}",
        firsts.len()
    )
}

/// **The `OPENING` line** (cut AN.1): how long from power online to the
/// first contact (p10 / p50 / p90 over the matches that had one) and to the
/// first hit taken (p50 over the matches that took one), in whole seconds.
fn opening_line(firsts: &[FirstTimes]) -> String {
    let contacts: Vec<u64> = firsts.iter().filter_map(|f| f.0).collect();
    let taken: Vec<u64> = firsts.iter().filter_map(|f| f.1).collect();
    let k = contacts.len();
    let j = taken.len();
    let Some((p10, p50, p90)) = seconds_p10_p50_p90(contacts) else {
        return format!("OPENING: no contact in {} matches", firsts.len());
    };
    let tail = match seconds_p10_p50_p90(taken) {
        Some((_, hit, _)) => format!("first hit taken p50 {hit} s over {j}"),
        None => "first hit taken none".to_owned(),
    };
    format!(
        "OPENING: first contact p10 {p10} / p50 {p50} / p90 {p90} s after power online over {k} matches; {tail}"
    )
}

/// **One muster match's line** (cut AN.1), from a report's
/// `arena.matches[*]`: `match {n}: {result}; {shots} shots, {hits} hits,
/// {kills} kills, took {t}; {s} s; first contact {c}, first hit taken {h},
/// first hit scored {x}`, the posed lane's match line with the match's three
/// first times, each `{whole seconds} s` after power online or `none`.
pub fn muster_match_line(flown: &serde_json::Value) -> String {
    let time = |key: &str| {
        flown[key]
            .as_u64()
            .map_or_else(|| "none".to_owned(), |ms| format!("{} s", ms / 1_000))
    };
    format!(
        "match {}: {}; {} shots, {} hits, {} kills, took {}; {} s; first contact {}, first hit taken {}, first hit scored {}",
        flown["match"],
        flown["result"].as_str().unwrap_or("?"),
        flown["shots"],
        flown["hits_scored"],
        flown["kills"],
        flown["hits_taken"],
        flown["duration_ms"].as_u64().unwrap_or(0) / 1_000,
        time("first_contact_ms"),
        time("first_hit_taken_ms"),
        time("first_hit_scored_ms"),
    )
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
    ///   `HITS TAKEN BY FACET: no report` when there is no report;
    /// * (cut AN.1) `SEEN FIRST: contact before hit {a}, hit before contact
    ///   {b}, neither {c} of {n} matches; lead p10 {x} / p50 {y} / p90 {z} s
    ///   over {k}; shot first {s}, shot at first {t}` (`lead none` with no
    ///   lead): who saw whom first, read through the lead
    ///   ([`seen_first_line`]);
    /// * (cut AN.1) `OPENING: first contact p10 {a} / p50 {b} / p90 {c} s
    ///   after power online over {k} matches; first hit taken p50 {d} s over
    ///   {j}` (`first hit taken none` with none taken; `OPENING: no contact
    ///   in {n} matches` with no contact at all), percentiles by index in
    ///   whole seconds ([`opening_line`]);
    /// * (cut AN.1) `FRONT SHARE: {f} of {h} facet hits taken on a front
    ///   facet`, `h` the facet hits taken in all and `f` those on a key
    ///   ending ` front`;
    /// * (cut AN.2) `PILOT RULES: opening {a}, off the road {b}, halt {c},
    ///   reapproach {d}, approach wait {e}` and (cut AN.3) `, follow {f},
    ///   return wait {g}, lattice {h}` and (cut AO.1) `, inward {i}, room
    ///   holds {k}, return room holds {r}, gave up {u}` and (cut AO.2) `,
    ///   benched {n}` ([`PilotRules::line`]), from the transcript alone (zeros with no
    ///   `arena` record).
    ///
    /// With no report the last three read `SEEN FIRST: no report`,
    /// `OPENING: no report` and `FRONT SHARE: no report`. None of them names
    /// anything of the other side.
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
        match &self.firsts {
            None => {
                lines.push("SEEN FIRST: no report".to_owned());
                lines.push("OPENING: no report".to_owned());
            }
            Some(firsts) => {
                lines.push(seen_first_line(firsts));
                lines.push(opening_line(firsts));
            }
        }
        lines.push(match &self.facets {
            None => "FRONT SHARE: no report".to_owned(),
            Some(facets) => {
                let all: u64 = facets.values().map(|n| u64::from(*n)).sum();
                let front: u64 = facets
                    .iter()
                    .filter(|(key, _)| key.ends_with(" front"))
                    .map(|(_, n)| u64::from(*n))
                    .sum();
                format!("FRONT SHARE: {front} of {all} facet hits taken on a front facet")
            }
        });
        lines.push(self.rules.line());
        lines
    }
}

// ---------------------------------------------------------------------------
// Cut AO.3: the pool (kept runs read together into one figure and one
// reading).
// ---------------------------------------------------------------------------

/// The variable naming the kept directories a pool reads (cut AO.3), split
/// as `std::env::split_paths` splits a path list (`;` on Windows, `:`
/// elsewhere).
pub const POOL_ENV: &str = "MUSH_BOT_WIN_RATE_POOL";
/// The variable setting the kept files' stem a pool reads (cut AO.3):
/// [`POOL_STEM`] by default; `win-rate` pools posed runs.
pub const POOL_NAME_ENV: &str = "MUSH_BOT_WIN_RATE_POOL_NAME";
/// The muster lane's stem, the pool's default (cut AO.3).
pub const POOL_STEM: &str = "win-rate-muster";
/// What the pool lane prints, and passes on, with [`POOL_ENV`] unset.
pub const POOL_UNSET: &str = "POOL: MUSH_BOT_WIN_RATE_POOL is unset; nothing to pool.";
/// A kept run's knobs when it left no [`knobs_file`].
pub const KNOBS_UNKNOWN: &str = "KNOBS: unknown";

/// **One kept run** (cut AO.3): its directory and what it holds of
/// `{stem}.jsonl` (the raw transcript), `{stem}.json` (whether the file is
/// there, and the report when it reads as JSON) and `{stem}.knobs` (its
/// first line, when not empty).
#[derive(Clone, Debug, Default)]
pub struct KeptRun {
    pub dir: PathBuf,
    pub transcript: Option<String>,
    pub report_file: bool,
    pub report: Option<serde_json::Value>,
    pub knobs: Option<String>,
}

impl KeptRun {
    /// The run's `KNOBS` line, or [`KNOBS_UNKNOWN`].
    pub fn knobs_line(&self) -> String {
        self.knobs
            .clone()
            .unwrap_or_else(|| KNOBS_UNKNOWN.to_owned())
    }
}

/// **Read the kept runs** (cut AO.3) in `dirs`, each by `stem`. Panics,
/// naming every such directory, when one holds neither `{stem}.jsonl` nor
/// `{stem}.json`: a pool never silently reads a run that is not there.
pub fn read_kept(dirs: &[PathBuf], stem: &str) -> Vec<KeptRun> {
    let runs: Vec<KeptRun> = dirs
        .iter()
        .map(|dir| {
            let report_text = std::fs::read_to_string(dir.join(format!("{stem}.json"))).ok();
            KeptRun {
                dir: dir.clone(),
                transcript: std::fs::read_to_string(dir.join(format!("{stem}.jsonl"))).ok(),
                report_file: report_text.is_some(),
                report: report_text
                    .as_deref()
                    .and_then(|text| serde_json::from_str(text).ok()),
                knobs: std::fs::read_to_string(dir.join(knobs_file(stem)))
                    .ok()
                    .and_then(|text| text.lines().next().map(|line| line.trim().to_owned()))
                    .filter(|line| !line.is_empty()),
            }
        })
        .collect();
    let empty: Vec<String> = runs
        .iter()
        .filter(|run| run.transcript.is_none() && !run.report_file)
        .map(|run| run.dir.display().to_string())
        .collect();
    assert!(
        empty.is_empty(),
        "POOL: no {stem}.jsonl and no {stem}.json in {}",
        empty.join("; ")
    );
    runs
}

/// **Refuse runs flown with different knobs** (cut AO.3): panics, naming
/// both runs' directories and lines, when two runs' known `KNOBS` lines
/// differ. A run with no knobs file ([`KNOBS_UNKNOWN`]) is not compared.
pub fn check_knobs(runs: &[KeptRun]) {
    let mut known = runs
        .iter()
        .filter_map(|run| run.knobs.as_ref().map(|line| (run, line)));
    let Some((first, line)) = known.next() else {
        return;
    };
    if let Some((other, theirs)) = known.find(|(_, theirs)| *theirs != line) {
        panic!(
            "POOL: the kept runs were flown with different knobs: {} has `{line}`, {} has `{theirs}`",
            first.dir.display(),
            other.dir.display()
        );
    }
}

/// The pool's figure head for `stem` (cut AO.3): `WIN RATE FROM THE MUSTER,
/// POOLED` for the muster lane's runs ([`POOL_STEM`]), `WIN RATE, POOLED`
/// for any other (the posed lane's `win-rate`).
pub fn pool_head(stem: &str) -> &'static str {
    if stem == POOL_STEM {
        "WIN RATE FROM THE MUSTER, POOLED"
    } else {
        "WIN RATE, POOLED"
    }
}

/// **The pool's lines** (cut AO.3), in order: `POOLED: {k} runs`; for each
/// run `RUN {i}: {w} of {n}, 95 per cent Wilson {lo} to {hi} per cent` (from
/// its report; `RUN {i}: no matches` for a report of none) or `RUN {i}: no
/// report`, then `RUN {i}: {its KNOBS line}` or `RUN {i}: KNOBS: unknown`;
/// the summed [`Outcome::line`] under [`pool_head`] of the summed
/// [`Outcome::total`]; its [`interval_line`]; and [`reading_of`]'s lines from
/// `FIRE GAP` on (its `ENGAGEMENT` head left out). [`check_knobs`] runs
/// first, so a refused pool prints nothing.
pub fn pool_lines(runs: &[KeptRun], stem: &str) -> Vec<String> {
    check_knobs(runs);
    let mut lines = vec![format!("POOLED: {} runs", runs.len())];
    let mut pooled = Outcome::default();
    for (i, run) in runs.iter().enumerate() {
        let i = i + 1;
        match &run.report {
            Some(report) => {
                let outcome = Outcome::from_report(report);
                pooled = pooled.plus(outcome);
                let interval = interval_line(outcome.won, outcome.total());
                let figure = interval
                    .strip_prefix("WIN RATE INTERVAL: ")
                    .unwrap_or(&interval);
                lines.push(format!("RUN {i}: {figure}"));
            }
            None => lines.push(format!("RUN {i}: no report")),
        }
        lines.push(format!("RUN {i}: {}", run.knobs_line()));
    }
    let of = usize::try_from(pooled.total()).unwrap_or(usize::MAX);
    lines.extend(win_rate_lines(&pooled, pool_head(stem), of));
    let kept: Vec<(&str, Option<&serde_json::Value>)> = runs
        .iter()
        .map(|run| (run.transcript.as_deref().unwrap_or(""), run.report.as_ref()))
        .collect();
    lines.extend(reading_of(&kept).lines("pooled").into_iter().skip(1));
    lines
}
