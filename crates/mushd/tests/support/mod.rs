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

    pub async fn send_line(&mut self, line: &str) {
        let bytes = format!("{line}\r").into_bytes();
        self.channel.data(&bytes[..]).await.expect("send line");
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

/// `ssh play@addr` with a PTY + shell, landing on the welcome screen.
pub async fn play_client(addr: SocketAddr) -> (russh::client::Handle<AcceptAll>, Term) {
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

/// Rewrite a Redis URL to a specific logical database, dropping any db (or
/// path) the base URL carried. Gated tests each claim their own db so the
/// test binaries cargo runs in parallel never wipe each other's keys:
/// mush-store's suite uses the URL as given (db 0); mushd claims dbs 12-14.
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
