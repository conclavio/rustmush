//! Shared helpers for mushd integration tests: an SSH "player terminal"
//! over a real russh client, a driver for the ACTUAL `mushd` binary as a
//! child process (durability tests kill it hard), and small utilities
//! (ANSI stripping, Redis logical-db selection, id parsing).
//!
//! Compiled per test binary that declares `mod support;`, so not every item
//! is used everywhere.
#![allow(dead_code)]

use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::ChannelMsg;

/// Generous per-step timeout: CI boxes can stall on argon2 + process spawn.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(60);

/// Wait for an observable condition, with one deadline covering the checks as
/// well as the polling interval. A slow SSH reply cannot multiply the bound.
pub async fn wait_until(bound: Duration, label: &str, mut check: impl AsyncFnMut() -> bool) {
    tokio::time::timeout(bound, async {
        loop {
            if check().await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{label} did not happen within {bound:?}"));
}

/// The running arena's own `clock m:ss`, as printed by `@arena/status`.
/// Use this for tests whose premise needs elapsed simulation time.
pub fn arena_clock_seconds(status: &str) -> Option<u64> {
    let clock = status.split_once(", clock ")?.1.split(',').next()?;
    let (minutes, seconds) = clock.split_once(':')?;
    Some(minutes.parse::<u64>().ok()? * 60 + seconds.parse::<u64>().ok()?)
}

/// The durable cockpit billet written by boarding and removed by EXIT/check-in.
/// A grace expiry releases the pilot's arena binding while retaining this
/// billet until check-in; use the staff board to observe that release.
pub async fn holds_a_cockpit(store: &Arc<dyn mush_store::ObjectStore>, unit: u32) -> bool {
    store
        .module_json_get("sim", &format!("assignment:{unit}"))
        .await
        .expect("the assignment read")
        .is_some_and(|value| value.pointer("/seats/cockpit").is_some())
}

/// Wait for the staff board to acknowledge a pilot's arena binding released by
/// an `EXIT` — the slot's row ending `, left`, which since 16b cut Y.3 is a
/// deliberate leave's alone (a drop reads `, flown by its mind`, `, flown by
/// doctrine` until 17b cut M5.3; see [`wait_arena_pilot_out`]).
pub async fn wait_arena_pilot_left(term: &mut Term, callsign: &str, pilot: &str) {
    wait_arena_pilot_out(term, callsign, pilot, "left").await;
}

/// **Wait for the staff board to say `word` about one pilot's slot** (16b cut
/// Y.3): the row `({callsign}): {pilot}, …` ending `, {word}`.
///
/// `word` is the board's own: `flown by its mind` for a drop (the linkdead
/// grace run out, or a `QUIT`: the tank is its sealed mind's and `BOARD` takes
/// it back; `flown by doctrine` until 17b cut M5.3), `left` for an `EXIT`
/// (one-way), `seated` for a pilot in the cockpit — so the same wait reads a
/// reclaim landing, too.
pub async fn wait_arena_pilot_out(term: &mut Term, callsign: &str, pilot: &str, word: &str) {
    let slot = format!("({callsign}): {pilot},");
    let tail = format!(", {word}");
    wait_until(
        STEP_TIMEOUT,
        "the pilot's slot to read as the board's word",
        async || {
            let mark = term.mark();
            term.send_line("@arena/status").await;
            if term
                .expect_one_of_after(
                    mark,
                    &[
                        "ledger writes awaiting",
                        "please retry in a moment",
                        "control state unknown",
                    ],
                )
                .await
                != 0
            {
                return false;
            }
            strip_ansi(&term.seen[mark..])
                .lines()
                .filter(|line| {
                    let line = line.trim_start();
                    line.starts_with("red:") || line.starts_with("blue:")
                })
                .flat_map(|line| line.split("; "))
                .any(|entry| entry.contains(&slot) && entry.trim_end().ends_with(tail.as_str()))
        },
    )
    .await;
}

/// Wait for simulation time rather than assuming that a wall-clock sleep
/// schedules enough arena pulses on a loaded test worker.
pub async fn wait_arena_seconds(term: &mut Term, seconds: u64) {
    let mut initial = None;
    wait_until(STEP_TIMEOUT, "arena time to advance", async || {
        let mark = term.mark();
        term.send_line("@arena/status").await;
        if term
            .expect_one_of_after(
                mark,
                &[
                    "ledger writes awaiting",
                    "please retry in a moment",
                    "control state unknown",
                ],
            )
            .await
            != 0
        {
            return false;
        }
        let now = arena_clock_seconds(&strip_ansi(&term.seen[mark..]))
            .expect("@arena/status reports the arena clock");
        let start = *initial.get_or_insert(now);
        now >= start + seconds
    })
    .await;
}

/// A unique scratch directory per call (removed best-effort by the caller).
pub fn scratch_dir(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "mushd-test-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Drop ANSI CSI sequences so assertions match what a player (or an
/// operator reading logs) actually reads.
pub fn strip_ansi(s: &str) -> String {
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

/// Test-only SSH client handler: trust any host key.
pub struct AcceptAll;

impl russh::client::Handler for AcceptAll {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// One player's terminal: an SSH channel plus everything it has printed.
pub struct Term {
    channel: russh::Channel<russh::client::Msg>,
    pub seen: String,
}

impl Term {
    /// Byte offset of "now" in the raw stream, for [`Term::expect_after`]
    /// when the needle may already occur earlier in the scrollback.
    pub fn mark(&self) -> usize {
        self.seen.len()
    }

    pub async fn expect(&mut self, needle: &str) {
        self.expect_after(0, needle).await;
    }

    pub async fn expect_after(&mut self, mark: usize, needle: &str) {
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

    /// Wait for `needle` in the post-mark output read the way a full-screen
    /// paint has to be read: ANSI dropped AND every whitespace character
    /// squeezed out, so `needle` is written without spaces (`Nolock`).
    ///
    /// The `@tui` screen paints only the cells that change, so a run of blanks
    /// that was blank before goes out as a cursor jump: `No lock` can reach the
    /// wire as `No`, a jump, `lock`, which [`Term::expect_after`] never sees.
    /// Squeezed, the two spellings are one.
    pub async fn expect_squeezed_after(&mut self, mark: usize, needle: &str) {
        let squeezed = |raw: &str| -> String {
            strip_ansi(raw)
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect()
        };
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        while !squeezed(&self.seen[mark..]).contains(needle) {
            let msg = tokio::time::timeout_at(deadline, self.channel.wait())
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "timeout waiting for {needle:?} squeezed; seen: {:?}",
                        self.seen
                    )
                });
            match msg {
                Some(ChannelMsg::Data { data }) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Some(_) => {}
                None => panic!(
                    "channel closed waiting for {needle:?} squeezed; seen: {:?}",
                    self.seen
                ),
            }
        }
    }

    /// Wait for one of several post-mark strings and return its index.
    pub async fn expect_one_of_after(&mut self, mark: usize, needles: &[&str]) -> usize {
        assert!(!needles.is_empty(), "at least one needle is required");
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        loop {
            let output = strip_ansi(&self.seen[mark..]);
            if let Some((index, _)) = needles
                .iter()
                .enumerate()
                .find(|(_, needle)| output.contains(**needle))
            {
                return index;
            }
            let msg = tokio::time::timeout_at(deadline, self.channel.wait())
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "timeout waiting for one of {needles:?}; seen: {:?}",
                        self.seen,
                    )
                });
            match msg {
                Some(ChannelMsg::Data { data }) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Some(_) => {}
                None => panic!(
                    "channel closed waiting for one of {needles:?}; seen: {:?}",
                    self.seen,
                ),
            }
        }
    }

    pub async fn send_line(&mut self, line: &str) {
        let bytes = format!("{line}\r").into_bytes();
        self.channel.data(&bytes[..]).await.expect("send line");
    }

    /// Resize the PTY to `cols` by `rows` — the SSH window-change a terminal
    /// sends when its window is resized (16b cut I.2). In `@tui` the server
    /// clears and repaints every cell at the new size, which is how a test
    /// reads a WHOLE frame: an ordinary paint writes only the cells that
    /// changed.
    pub async fn resize(&mut self, cols: u32, rows: u32) {
        self.channel
            .window_change(cols, rows, 0, 0)
            .await
            .expect("window change");
    }

    /// Await the channel closing (server-side boot/quit), collecting any
    /// final output into `seen` on the way.
    pub async fn expect_close(&mut self) {
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        loop {
            let msg = tokio::time::timeout_at(deadline, self.channel.wait())
                .await
                .unwrap_or_else(|_| panic!("timeout waiting for close; seen: {:?}", self.seen));
            match msg {
                None | Some(ChannelMsg::Close) => return,
                Some(ChannelMsg::Data { data }) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Some(_) => {}
            }
        }
    }
}

fn transaction_after(output: &str, mark: usize) -> String {
    let output = strip_ansi(&output[mark..]);
    let at = output
        .rfind("Transaction:")
        .unwrap_or_else(|| panic!("missing transaction reference in {output:?}"));
    output[at + "Transaction:".len()..]
        .trim_start()
        .chars()
        .take_while(|character| !character.is_whitespace())
        .collect()
}

/// Submit an authoritative line action and follow its durable transaction to
/// completion. The all-in-one bridge returns ownership before the writer has
/// necessarily installed the post-image, so the stable `action/status` path is
/// the ordering fence for subsequent commands in a journey.
pub async fn submit_action(term: &mut Term, line: &str, expected: &str) {
    let action_mark = term.mark();
    term.send_line(line).await;
    let response = term
        .expect_one_of_after(action_mark, &[expected, "Action accepted. Transaction:"])
        .await;
    if response == 0 {
        return;
    }

    let transaction = transaction_after(&term.seen, action_mark);
    for _ in 0..100 {
        let status_mark = term.mark();
        term.send_line(&format!("action/status {transaction}"))
            .await;
        let status = term
            .expect_one_of_after(
                status_mark,
                &[
                    expected,
                    "Action is still pending.",
                    "That transaction needs recovery.",
                ],
            )
            .await;
        if status == 0 {
            return;
        }
        assert_ne!(status, 2, "authoritative action requires recovery");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("authoritative action remained pending: {transaction}");
}

/// `ssh play@addr` with a PTY + shell, landing on the welcome screen. The PTY
/// is 80 by 24: LINE mode stays at eighty, and every expectation the mushd
/// tests make of a line client's wrap is made at that width.
pub async fn play_client(addr: SocketAddr) -> (russh::client::Handle<AcceptAll>, Term) {
    play_client_sized(addr, 80, 24).await
}

/// [`play_client`] with a PTY of `cols` by `rows` (16b cut AB.1): a session that
/// opens `@tui` asks for at least the 120-column floor, or its screen is one
/// sentence.
pub async fn play_client_sized(
    addr: SocketAddr,
    cols: u32,
    rows: u32,
) -> (russh::client::Handle<AcceptAll>, Term) {
    let config = Arc::new(russh::client::Config::default());
    let mut handle = russh::client::connect(config, addr, AcceptAll)
        .await
        .expect("connect");
    let auth = handle.authenticate_none("play").await.expect("auth-none");
    assert!(auth.success(), "play must be accepted with no auth");
    let channel = handle.channel_open_session().await.expect("open channel");
    channel
        .request_pty(true, "xterm-256color", cols, rows, 0, 0, &[])
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

/// Rewrite a Redis URL to a specific logical database, dropping any db (or
/// path) the base URL carried. Gated tests each claim their own db so the
/// test binaries cargo runs in parallel never wipe each other's keys:
/// mush-store's suite uses the URL as given (db 0); mushd claims dbs 12-15.
pub fn redis_url_with_db(base: &str, db: u32) -> String {
    let (scheme, rest) = base.split_once("://").unwrap_or(("redis", base));
    let authority = rest.split(['/', '?']).next().unwrap_or(rest);
    format!("{scheme}://{authority}/{db}")
}

/// The last `{prefix}<digits>` in `hay` (already ANSI-stripped), parsed.
/// Used to pull object ids out of `@dig`/`@create` output.
pub fn parse_id_after(hay: &str, prefix: &str) -> u32 {
    let at = hay
        .rfind(prefix)
        .unwrap_or_else(|| panic!("no {prefix:?} in output:\n{hay}"));
    let digits: String = hay[at + prefix.len()..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits
        .parse()
        .unwrap_or_else(|_| panic!("no id digits after {prefix:?} in output:\n{hay}"))
}

/// Write a minimal `mushd.toml` into `dir` (data dir inside it) with the
/// given `[store]` section body; returns the config path.
pub fn write_config(dir: &Path, store_section: &str) -> PathBuf {
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let path = dir.join("mushd.toml");
    // TOML basic strings treat backslash as an escape; forward slashes work
    // fine on Windows.
    let data_str = data.display().to_string().replace('\\', "/");
    let toml = format!(
        "[server]\n\
         name = \"durability rig\"\n\
         data_dir = \"{data_str}\"\n\
         \n\
         [store]\n\
         {store_section}\n\
         \n\
         [ssh]\n\
         listen = \"127.0.0.1:0\"\n\
         \n\
         # Ephemeral web port too: parallel test children must not fight\n\
         # over the default 8080.\n\
         [web]\n\
         listen = \"127.0.0.1:0\"\n"
    );
    std::fs::write(&path, toml).unwrap();
    path
}

/// Boot the full production wiring — world task, session layer,
/// store-backed accounts, IP guard, SSH listener — with the real
/// [`server_modules`](mushd::modules::server_modules) list (including sim and
/// tactics) booted exactly as `main.rs` does
/// (`Engine::with_modules` → `boot_modules`), on an ephemeral SSH port with
/// an in-memory store. A `boss` account linked to God (`#1`) is seeded so a
/// staff connection can drive builder commands over the same front door as
/// everyone else. Returns the bound SSH address.
///
/// The M5 scripting + weather e2e journeys share this; it is the minimal
/// standard-module server (no per-test producer module — those boot their
/// own rig, cf. `modules_e2e`).
pub async fn boot_module_server(dir: &Path, server_name: &str, boss_password: &str) -> SocketAddr {
    use mush_session::{SessionConfig, SessionManager};

    std::fs::create_dir_all(dir).unwrap();
    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(120),
        ..SessionConfig::default()
    }));
    let store: Arc<dyn mush_store::ObjectStore> = Arc::new(mush_store::InMemoryStore::new());
    store
        .create_account(mush_store::AccountRec {
            id: None,
            name: "boss".to_string(),
            argon2_hash: mushd::accounts::hash_password(boss_password).unwrap(),
            created: 0,
            characters: vec![mush_store::CharacterRef {
                id: mush_core::GOD,
                name: "God".to_string(),
            }],
            ssh_keys: vec![],
            discord: None,
        })
        .await
        .unwrap();

    // main.rs boot order: base registry + staff commands + help topics, then
    // Engine::with_modules + boot_modules, then the world task.
    let staff = Arc::new(mushd::staff::StaffQueue::new());
    let mut registry = mushd::world_task::base_registry();
    mushd::staff::register_server_commands(&mut registry, &staff).unwrap();
    let help_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/help"));
    let _ = mushd::help::load_help_topics(&mut registry, help_dir);

    let mut world = mush_core::World::new_minimal();
    world.set_now(mushd::unix_now_s());
    // S1/S2b: the content module composes LAST, as main.rs boots it, so the
    // starter lot (the OOC Area and the Staff lot) is applied at first boot and
    // the weather journey has a place to key on.
    let mut mods = mushd::modules::server_modules();
    mods.push(mushd::modules::content_module(true).unwrap());
    let mut engine = mush_core::Engine::with_modules(world, registry, mods).unwrap();
    mushd::modules::boot_modules(&mut engine, &*store)
        .await
        .unwrap();

    let handles = mushd::world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        mushd::world_task::ServerDeps {
            metrics: std::sync::Arc::new(mush_net_web::Metrics::new()),
            store: Arc::clone(&store),
            guard: Arc::new(mushd::ipguard::IpGuard::new()),
            staff,
            llm: None,
            quarantine_spool: mushd::quarantine::QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
        },
    );
    let world = handles.handle;
    let accounts = Arc::new(mushd::accounts::Accounts::new(
        Arc::clone(&store),
        world.clone(),
        3,
    ));
    let bridge = Arc::new(mushd::ssh_bridge::SshBridge::new(
        accounts,
        Arc::clone(&sessions),
        world,
        Arc::new(mushd::ipguard::IpGuard::new()),
    ));
    mushd::ssh_bridge::start_listener(
        "127.0.0.1:0".parse().unwrap(),
        dir,
        server_name.to_string(),
        bridge,
    )
    .await
    .expect("listener")
    .addr()
}

/// Boot the full production wiring exactly like [`boot_module_server`], but
/// with the M7 worldgen module registered against the supplied `llm` handle
/// and that same handle installed on the world task's
/// [`ServerDeps`](mushd::world_task::ServerDeps) — so a `@gen`'s deferred
/// `Effect::ModuleAsync` is driven off-loop through `run_worldgen_async`
/// against `llm` and re-enters as `WorldCommand::ModuleAsyncDone`, the real
/// server's async path. Tests inject a `MockProvider`-backed service (no
/// network, no key) and keep their own `Arc` to it to assert on the requests
/// it received. Returns the bound SSH address.
pub async fn boot_worldgen_server(
    dir: &Path,
    server_name: &str,
    boss_password: &str,
    llm: Arc<mush_llm::LlmService>,
) -> SocketAddr {
    use mush_session::{SessionConfig, SessionManager};

    std::fs::create_dir_all(dir).unwrap();
    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(120),
        ..SessionConfig::default()
    }));
    let store: Arc<dyn mush_store::ObjectStore> = Arc::new(mush_store::InMemoryStore::new());
    store
        .create_account(mush_store::AccountRec {
            id: None,
            name: "boss".to_string(),
            argon2_hash: mushd::accounts::hash_password(boss_password).unwrap(),
            created: 0,
            characters: vec![mush_store::CharacterRef {
                id: mush_core::GOD,
                name: "God".to_string(),
            }],
            ssh_keys: vec![],
            discord: None,
        })
        .await
        .unwrap();

    let staff = Arc::new(mushd::staff::StaffQueue::new());
    let mut registry = mushd::world_task::base_registry();
    mushd::staff::register_server_commands(&mut registry, &staff).unwrap();

    let mut world = mush_core::World::new_minimal();
    world.set_now(mushd::unix_now_s());
    // main.rs boot order: built-ins, then worldgen (with the shared service).
    let mut mods = mushd::modules::server_modules();
    mods.push(mushd::modules::worldgen_module(Arc::clone(&llm)));
    let mut engine = mush_core::Engine::with_modules(world, registry, mods).unwrap();
    mushd::modules::boot_modules(&mut engine, &*store)
        .await
        .unwrap();

    let handles = mushd::world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        mushd::world_task::ServerDeps {
            metrics: std::sync::Arc::new(mush_net_web::Metrics::new()),
            store: Arc::clone(&store),
            guard: Arc::new(mushd::ipguard::IpGuard::new()),
            staff,
            llm: Some(llm),
            quarantine_spool: mushd::quarantine::QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
        },
    );
    let world = handles.handle;
    let accounts = Arc::new(mushd::accounts::Accounts::new(
        Arc::clone(&store),
        world.clone(),
        3,
    ));
    let bridge = Arc::new(mushd::ssh_bridge::SshBridge::new(
        accounts,
        Arc::clone(&sessions),
        world,
        Arc::new(mushd::ipguard::IpGuard::new()),
    ));
    mushd::ssh_bridge::start_listener(
        "127.0.0.1:0".parse().unwrap(),
        dir,
        server_name.to_string(),
        bridge,
    )
    .await
    .expect("listener")
    .addr()
}

/// `create <name> <pw>` a fresh account+character in-band and land in Limbo.
/// Returns the client handle (kept alive by the caller) and its terminal.
pub async fn login_create(
    addr: SocketAddr,
    name: &str,
    pw: &str,
) -> (russh::client::Handle<AcceptAll>, Term) {
    let (handle, mut term) = play_client(addr).await;
    term.send_line(&format!("create {name} {pw}")).await;
    term.expect(&format!("Connected as {name}")).await;
    term.expect("Limbo #0").await;
    (handle, term)
}

/// Seed a `boss` account linked to God (`#1`) via the one-time
/// `accounts.json` import the server runs at boot — works identically for
/// the Redis and in-memory backends, since the child imports it itself.
pub fn seed_boss_account(data_dir: &Path, password: &str) {
    std::fs::create_dir_all(data_dir).unwrap();
    let json = serde_json::json!({
        "version": 1,
        "accounts": [{
            "name": "boss",
            "hash": mushd::accounts::hash_password(password).expect("hash"),
            "characters": [{ "id": 1, "name": "God" }],
        }],
    });
    std::fs::write(data_dir.join("accounts.json"), json.to_string()).unwrap();
}

/// The ACTUAL `mushd` binary as a child process, with its stdout/stderr
/// captured line by line so tests can await log lines (bound address,
/// drain/BGSAVE messages) and kill it hard mid-play.
pub struct ChildServer {
    child: Child,
    log: Arc<Mutex<String>>,
}

impl ChildServer {
    /// Spawn `mushd --config <config>`.
    pub fn spawn(config: &Path) -> ChildServer {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mushd"))
            .arg("--config")
            .arg(config)
            // The harness waits on INFO-level readiness/durability lines.
            // Never inherit a developer/CI `RUST_LOG=warn` that suppresses
            // those protocol markers and turns a healthy child into a timeout.
            .env("RUST_LOG", "info")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn mushd");
        let log = Arc::new(Mutex::new(String::new()));
        for stream in [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let log = Arc::clone(&log);
            std::thread::spawn(move || {
                let reader = BufReader::new(stream);
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    let mut log = log.lock().expect("log lock");
                    log.push_str(&line);
                    log.push('\n');
                }
            });
        }
        ChildServer { child, log }
    }

    /// Everything the child has logged so far (ANSI-stripped).
    pub fn log_snapshot(&self) -> String {
        strip_ansi(&self.log.lock().expect("log lock"))
    }

    /// Wait (poll) until `needle` appears in the child's log.
    pub async fn await_log(&self, needle: &str) {
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        loop {
            if self.log_snapshot().contains(needle) {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timeout waiting for {needle:?} in child log:\n{}",
                self.log_snapshot()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// The child's bound SSH address, read from its own startup log line
    /// (`... up; press Ctrl-C to shut down ssh=127.0.0.1:PORT`).
    pub async fn ssh_addr(&self) -> SocketAddr {
        self.await_log("ssh=").await;
        let log = self.log_snapshot();
        let at = log.rfind("ssh=").expect("just awaited");
        let token: String = log[at + 4..]
            .chars()
            .take_while(|c| !c.is_whitespace())
            .collect();
        token
            .parse()
            .unwrap_or_else(|_| panic!("unparseable ssh address {token:?} in child log:\n{log}"))
    }

    /// kill -9 equivalent: SIGKILL on Unix, `TerminateProcess` on Windows.
    /// No shutdown hook, no drain, no BGSAVE — the process just stops.
    pub fn kill_hard(&mut self) {
        self.child.kill().expect("kill child");
        let _ = self.child.wait();
    }

    /// Wait for the child to exit on its own (clean-shutdown path).
    pub async fn wait_exit(&mut self, timeout: Duration) -> ExitStatus {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                return status;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "child did not exit in {timeout:?}; log:\n{}",
                self.log_snapshot()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

impl Drop for ChildServer {
    fn drop(&mut self) {
        // Never leave a stray server running after a failed test.
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// **Whether a line names a side as a word** (16b cut F.3).
///
/// The observer-rule leak checks forbid the substring `red` in a victim's own
/// damage lines, and `starts_with("The ")` lets in every line that opens that
/// way — so a sentence such as `The last match is cleared from the board.` would
/// false-fail on `cleared`. A boundary test forbids exactly what the leak check
/// meant to forbid, the side's own name, and nothing that merely spells it.
///
/// A match counts when neither neighbour is alphanumeric, so `red`, `(red)` and
/// `red's` are leaks and `cleared`, `predicted` and `redoubt` are not.
#[must_use]
pub fn names_side(line: &str, side: &str) -> bool {
    line.match_indices(side).any(|(at, _)| {
        let before = line[..at].chars().next_back();
        let after = line[at.saturating_add(side.len())..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// **The eight bearing words `VIEW BRIEF` walks, in compass order** (16b cut
/// AG.1): `tacsim::match_::cockpit::BRIEF_BEARINGS`' words, re-stated because
/// these tests read a terminal and not the crate.
pub const BRIEF_BEARING_WORDS: [&str; 8] = [
    "North",
    "North-east",
    "East",
    "South-east",
    "South",
    "South-west",
    "West",
    "North-west",
];

/// Whether a line opens a brief bearing sentence: one of the eight words, then a
/// colon (so `North Barn` is not `North`).
fn opens_bearing_sentence(line: &str) -> bool {
    BRIEF_BEARING_WORDS.iter().any(|word| {
        line.strip_prefix(word)
            .is_some_and(|rest| rest.starts_with(':'))
    })
}

/// Whether a line opens a brief contact sentence (16b cut AG.1), by hand and
/// with no regex: one or two ASCII capitals, then any ASCII digits (`BA`, `X3`,
/// `X12`), then `, `, then the class word (`unknown`, `red` or `blue`), then
/// `: bearing `.
#[must_use]
pub fn opens_contact_sentence(line: &str) -> bool {
    let capitals = line.chars().take_while(char::is_ascii_uppercase).count();
    if !(1..=2).contains(&capitals) {
        return false;
    }
    let rest = &line[capitals..];
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    let Some(rest) = rest[digits..].strip_prefix(", ") else {
        return false;
    };
    ["unknown", "red", "blue"].iter().any(|class| {
        rest.strip_prefix(class)
            .is_some_and(|tail| tail.starts_with(": bearing "))
    })
}

/// The brief's own empty-contacts sentence (16b cut AP.4, 16c `N20`;
/// `cockpit::NO_CONTACTS_IN_WINDOW`), never `CONTACTS`' `No contacts.`.
pub const BRIEF_NO_CONTACTS: &str = "No contacts in the window.";

/// The brief's fixed last line (16b cut AP.4; `cockpit::BRIEF_END_LINE`).
pub const BRIEF_END: &str = "End of brief.";

/// Whether a line opens one of `VIEW BRIEF`'s sentences (16b cut AG.1). The
/// old empty sentence `No contacts.` still opens one, so a brief that said it
/// fails [`assert_brief_shape`] by name rather than dropping it as a stray.
fn opens_brief_sentence(line: &str) -> bool {
    line.starts_with("Window around ")
        || opens_bearing_sentence(line)
        || line == BRIEF_NO_CONTACTS
        || line == "No contacts."
        || opens_contact_sentence(line)
        || line.contains("beyond this window")
        || line.starts_with("In the window:")
        || line.starts_with("Off the window:")
        || line == BRIEF_END
}

/// Whether a trimmed terminal line is the reader's own traffic rather than
/// anything the world said: the blank line, the typed echo (`VIEW BRIEF`), a
/// sentinel line (every file's sentinel is a `zzq-…` word) and the parser's
/// `Huh?` answering it.
fn is_brief_traffic(line: &str) -> bool {
    line.is_empty()
        || line.eq_ignore_ascii_case("VIEW BRIEF")
        || line.starts_with("zzq-")
        || line == "Huh?"
}

/// The one walk [`brief_sentences`] and [`brief_strays`] share: the sentences,
/// and every line it dropped with the number of sentences opened before it.
fn brief_walk(seen: &str) -> (Vec<String>, Vec<(usize, String)>) {
    let mut sentences: Vec<String> = Vec::new();
    let mut dropped: Vec<(usize, String)> = Vec::new();
    for raw in seen.lines() {
        let line = raw.trim();
        if is_brief_traffic(line) {
            continue;
        }
        if opens_brief_sentence(line) {
            sentences.push(line.to_owned());
            continue;
        }
        match sentences.last_mut() {
            Some(current) if !current.ends_with('.') => {
                current.push(' ');
                current.push_str(line);
            }
            _ => dropped.push((sentences.len(), line.to_owned())),
        }
    }
    (sentences, dropped)
}

/// **`VIEW BRIEF`'s own sentences, picked out of a whole answer by shape**
/// (16b cut AG.1), with the terminal's wraps undone.
///
/// `seen` is a slice of what a terminal printed around a `VIEW BRIEF` — an
/// `ask_whole` answer, or the tail of one from the brief's header on. Each line
/// is trimmed; the typed echo (`VIEW BRIEF`), a sentinel line (every file's
/// sentinel is a `zzq-…` word) and the parser's `Huh?` are dropped. A line that
/// **opens a brief sentence** — `Window around `, a bearing word and a colon,
/// `No contacts in the window.`, a contact sentence (one or two capitals,
/// optional digits, `, `, a class word, `: bearing `), the beyond-window line
/// (`beyond this window`), `In the window:`, `Off the window:` or the closing
/// `End of brief.` — starts a new sentence. A
/// line that opens none is a wrapped continuation of the current sentence while
/// that sentence does not yet end with `.` (the PTY is 80 by 24 and the session
/// wraps at spaces), joined with one space; otherwise it is dropped — an arena
/// notice that landed before the brief or after it (E.3's `Held: something is
/// in the way.` while the two tanks close head-on on row 97, an acquisition
/// notice, a damage line), or `VIEW`'s own `legend:` line ahead of the header
/// in `sim_cockpit`. What is dropped from *inside* the brief is
/// [`brief_strays`]' to report, never a silent skip.
#[must_use]
pub fn brief_sentences(seen: &str) -> Vec<String> {
    brief_walk(seen).0
}

/// **The lines [`brief_sentences`] dropped from inside the brief** (16b cut
/// AG.1): every line that opened no sentence and continued none, and sits after
/// the brief's first sentence and before its last. The world is the one writer
/// and says a verb's lines in one batch (`say_and_frame`), while an arena
/// notice is a batch of its own drained on another turn of the world task, so
/// a notice reaches the terminal before the brief or after it and never between
/// two of its lines. A line dropped inside the brief is therefore a brief line
/// whose opener a change mis-worded — a malformed contact sentence, a
/// re-worded spot line — and every reader asserts this list is empty.
///
/// Since 16b cut AP.4 a reader knows where its brief ends: `End of brief.` is
/// the brief's own last line, which [`assert_brief_shape`] requires, so a
/// mis-worded line before it is inside the brief and reported here.
#[must_use]
pub fn brief_strays(seen: &str) -> Vec<String> {
    let (sentences, dropped) = brief_walk(seen);
    dropped
        .into_iter()
        .filter(|(opened, _)| *opened >= 1 && *opened < sentences.len())
        .map(|(_, line)| line)
        .collect()
}

/// **A brief has its shape, in its order** (16b cut AG.1; `brief_lines`'
/// order): the header first and once; then the eight bearing sentences in
/// compass order; then either the one `No contacts in the window.` or one or
/// more contact sentences, never both (and never `CONTACTS`' `No contacts.`,
/// 16b cut AP.4); then at most one beyond-window line, at most one `In the
/// window:` and at most one `Off the window:`, in that order; then `End of
/// brief.` (AP.4), last; and nothing after. Every sentence
/// [`brief_sentences`] opened is placed by this walk, so a sentence out of its
/// place, doubled, or the empty sentence beside a contact fails here. It reads
/// sentences, so a line that opened none is [`brief_strays`]' to catch.
/// Panics naming `what`.
pub fn assert_brief_shape(sentences: &[String], what: &str) {
    let mut rest = sentences.iter().map(String::as_str).peekable();
    assert!(
        rest.next()
            .is_some_and(|header| header.starts_with("Window around ")),
        "{what}: the brief opens with its header:\n{sentences:#?}"
    );
    for word in BRIEF_BEARING_WORDS {
        assert!(
            rest.next().is_some_and(|sentence| {
                sentence
                    .strip_prefix(word)
                    .is_some_and(|tail| tail.starts_with(':'))
            }),
            "{what}: the {word} sentence, in compass order after the header:\n{sentences:#?}"
        );
    }
    if rest
        .next_if(|sentence| *sentence == BRIEF_NO_CONTACTS)
        .is_none()
    {
        let mut contacts = 0_usize;
        while rest
            .next_if(|sentence| opens_contact_sentence(sentence))
            .is_some()
        {
            contacts += 1;
        }
        assert!(
            contacts >= 1,
            "{what}: `{BRIEF_NO_CONTACTS}` or a contact sentence after the bearings:\n{sentences:#?}"
        );
    }
    let _beyond = rest.next_if(|sentence| sentence.contains("beyond this window"));
    let _in_window = rest.next_if(|sentence| sentence.starts_with("In the window:"));
    let _off_window = rest.next_if(|sentence| sentence.starts_with("Off the window:"));
    let closed = rest.next_if(|sentence| *sentence == BRIEF_END).is_some();
    let left: Vec<&str> = rest.collect();
    assert!(
        closed && left.is_empty(),
        "{what}: sentences out of the brief's order (a contact beside `{BRIEF_NO_CONTACTS}`, \
         a line doubled or out of place, `{BRIEF_END}` missing or not last): \
         {left:#?}\n{sentences:#?}"
    );
}

/// Everything `seen` holds, with the terminal's wraps undone: one space for
/// every run of whitespace, so a sentence longer than the eighty columns reads
/// back whole. (Moved here from `sim_solo.rs` by 16b cut AH.1, which reads the
/// first-time hints — four of the five are wider than the test PTY — in two
/// files.)
#[must_use]
pub fn unwrapped(seen: &str) -> String {
    seen.split_whitespace().collect::<Vec<&str>>().join(" ")
}

/// **Every first-time hint a transcript read, in order** (16b cut AH.1; 16c
/// N18): `seen` is stripped of ANSI and unwrapped, split at every `Hint: `, and
/// every piece after the first must open with the rest of one of
/// `tacsim::match_::tutorial::HINT_LINES` — a hint the terminal printed whole,
/// which is the only kind there is. Answers the matched lines verbatim.
///
/// Panics on a piece that opens with none: a `Hint: ` that is not one of the
/// five fixed sentences is a hint that interpolated something, or a sentence
/// the table does not hold.
#[must_use]
pub fn hints_read(seen: &str) -> Vec<String> {
    let flat = unwrapped(&strip_ansi(seen));
    let prefix = tacsim::match_::tutorial::HINT_PREFIX;
    flat.split(prefix)
        .skip(1)
        .map(|piece| {
            tacsim::match_::tutorial::HINT_LINES
                .iter()
                .find(|line| piece.starts_with(line.trim_start_matches(prefix)))
                .map(|line| (*line).to_owned())
                .unwrap_or_else(|| panic!("a `{prefix}` that is none of the five: {piece:?}"))
        })
        .collect()
}

/// **A slice with the first-time hints taken out** (16b cut AH.1): unwrapped,
/// then every occurrence of each of the five sentences removed — for a
/// byte-exact compare of two answers whose window a hint's occasion can
/// straddle. Never a wider needle: the answers themselves are compared whole.
#[must_use]
pub fn without_hints(slice: &str) -> String {
    let mut flat = unwrapped(slice);
    for line in tacsim::match_::tutorial::HINT_LINES {
        flat = flat.replace(line, "");
    }
    unwrapped(&flat)
}
