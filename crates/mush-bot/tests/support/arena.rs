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
//! **The one cross-crate state-slot site** of the bot's tests is in [`boot`]: the
//! account oracle installed into its module-state slot after composition, as
//! every simulator lens does, because a simulator slot belongs to an ACCOUNT
//! (`crates/mush-contract-tests/state-slot-owners.toml` carries its row).
#![allow(dead_code)]

use std::{
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
    /// answer begins.
    pub async fn send(&mut self, line: &str) {
        self.mark = self.seen.len();
        self.session.send(line).await.expect("the line is sent");
    }

    /// Wait until one of `needles` has arrived since `from`; its index, or
    /// `None` at the deadline or when the channel closes.
    async fn until(&mut self, from: usize, needles: &[&str], timeout: Duration) -> Option<usize> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let tail = self.seen.get(from..).unwrap_or("");
            if let Some(index) = needles.iter().position(|needle| tail.contains(needle)) {
                return Some(index);
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
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => return None,
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
    /// panics on a refusal, so a caller can print one and carry on.
    pub async fn answer(&mut self, line: &str, needles: &[&str]) -> (Option<usize>, String) {
        let mut all: Vec<&str> = needles.to_vec();
        all.extend(TRANSIENTS);
        for _ in 0..RETRY_BUDGET {
            self.send(line).await;
            let from = self.mark;
            match self.until(from, &all, STEP_TIMEOUT).await {
                Some(index) if index < needles.len() => {
                    return (Some(index), self.seen.get(from..).unwrap_or("").to_owned());
                }
                Some(_) => tokio::time::sleep(Duration::from_millis(200)).await,
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
    /// still drains what it is sent, so nothing backs up behind it.
    pub async fn idle(&mut self, span: Duration) {
        let deadline = tokio::time::Instant::now() + span;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
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

/// **Re-pose a tank between matches** (cut AJ.4, the win-rate lane): as
/// [`pose`], but a refusal is the answer's text, not a panic, so a lane that
/// re-poses before every match can print it and carry on. `Ok` holds the
/// `opens at cell` answer.
pub async fn try_pose(god: &mut Client, spec: &str) -> Result<String, String> {
    match god
        .answer(&format!("@arena/pose {spec}"), &["opens at cell"])
        .await
    {
        (Some(_), text) => Ok(text),
        (None, text) => Err(text),
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
/// arena mode it bounds each match (a claim starts it again), which the
/// win-rate lane's three-minute matches need longer than 150 s.
pub fn bot_config_with(rig: &Rig, username: &str, max_seconds: u64, extra: &str) -> PathBuf {
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
settle_ms = 100
observation_ms = 1000

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
    let transcript = rig.dir.join("bot").join(format!("{name}.jsonl"));
    let report = rig.dir.join("bot").join(format!("{name}.json"));
    let mut command = tokio::process::Command::from(bot_command(rig, config, PILOT_PASSWORD));
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

/// **Wait for `needle` in `client`'s stream after byte `from` while the bot
/// flies** (cut AJ.4), at most `limit`: the byte just past it. A bot that ends
/// first fails the wait with its own output, so a blocker reads as itself and
/// not as a timeout.
pub async fn await_line(
    client: &mut Client,
    from: usize,
    needle: &str,
    flight: &mut Flight,
    limit: Duration,
) -> usize {
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        if client.watch(from, needle, Duration::from_secs(1)).await {
            let at = client
                .seen
                .get(from..)
                .and_then(|tail| tail.find(needle))
                .unwrap_or(0);
            return from + at + needle.len();
        }
        if flight.task.is_finished() {
            ended(flight, &format!("{needle:?}")).await;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "no {needle:?} inside {} s; since the mark:\n{}",
                limit.as_secs(),
                client.seen.get(from..).unwrap_or("")
            );
        }
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
