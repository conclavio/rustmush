//! Loopback integration: a real russh client drives a full session against
//! the server on an ephemeral port, with a mock [`SshHooks`] standing in
//! for the game binary.
//!
//! Covers the M1 exit-path: `ssh play@…` (auth-none) → welcome banner →
//! bad credentials counted → `create` → auto-connect → world output
//! rendered through the pump → `say` echo → `QUIT` closes the transport
//! and fires the disconnect hook. A second test takes the account-username
//! password route with auto-connect.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use mush_net_ssh::{AuthResult, CharSummary, CreateErr, NewConn, SshConfig, SshHooks, SshServer};
use mush_proto::{NarrativeKind, ObjRef, OutputEvent, RichText};
use russh::ChannelMsg;
use tokio::sync::mpsc;

const STEP_TIMEOUT: Duration = Duration::from_secs(15);

// ---- mock game ----------------------------------------------------------

#[derive(Default)]
struct MockHooks {
    /// account (lowercase) -> (password, characters)
    accounts: Mutex<HashMap<String, (String, Vec<CharSummary>)>>,
    /// Output queues by conn id (dropping one closes that connection).
    txs: Mutex<HashMap<u64, mpsc::Sender<Arc<OutputEvent>>>>,
    lines: Mutex<Vec<(u64, String)>>,
    disconnects: Mutex<Vec<u64>>,
    auth_failures: AtomicUsize,
    next_char_key: AtomicUsize,
}

impl MockHooks {
    fn seed_account(&self, name: &str, password: &str, chars: Vec<CharSummary>) {
        self.accounts
            .lock()
            .unwrap()
            .insert(name.to_ascii_lowercase(), (password.to_owned(), chars));
    }

    fn tx_of(&self, conn: u64) -> Option<mpsc::Sender<Arc<OutputEvent>>> {
        self.txs.lock().unwrap().get(&conn).cloned()
    }
}

impl SshHooks for MockHooks {
    fn auth_account<'a>(
        &'a self,
        account: &'a str,
        password: &'a str,
    ) -> BoxFuture<'a, AuthResult> {
        Box::pin(async move {
            let key = account.to_ascii_lowercase();
            let accounts = self.accounts.lock().unwrap();
            match accounts.get(&key) {
                Some((pw, chars)) if pw == password => AuthResult::Ok {
                    account: key.clone(),
                    characters: chars.clone(),
                },
                _ => AuthResult::BadCreds,
            }
        })
    }

    fn create_account<'a>(
        &'a self,
        account: &'a str,
        password: &'a str,
    ) -> BoxFuture<'a, Result<(), CreateErr>> {
        Box::pin(async move {
            let key = account.to_ascii_lowercase();
            let mut accounts = self.accounts.lock().unwrap();
            if accounts.contains_key(&key) {
                return Err(CreateErr::Taken);
            }
            if password.len() < 4 {
                return Err(CreateErr::Weak);
            }
            let char_key = 100 + self.next_char_key.fetch_add(1, Ordering::Relaxed) as u64;
            // M1: creating an account creates a same-named character.
            let mut name = key.clone();
            if let Some(first) = name.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            accounts.insert(
                key,
                (
                    password.to_owned(),
                    vec![CharSummary {
                        key: char_key,
                        name,
                    }],
                ),
            );
            Ok(())
        })
    }

    fn connect_character<'a>(
        &'a self,
        conn: NewConn,
        account: &'a str,
        _character: u64,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let tx = conn.tx.clone();
            self.txs.lock().unwrap().insert(conn.id, conn.tx);
            let ev = Arc::new(OutputEvent::Sys(RichText::plain(format!(
                "Welcome, {account}!"
            ))));
            tx.send(ev).await.map_err(|_| "queue closed".to_owned())?;
            Ok(())
        })
    }

    fn line(&self, conn: u64, line: String) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.lines.lock().unwrap().push((conn, line.clone()));
            if line.eq_ignore_ascii_case("quit") {
                // The "world" detaches the connection: dropping the sender
                // makes the pump close the transport.
                self.txs.lock().unwrap().remove(&conn);
            } else if let Some(said) = line.strip_prefix("say ")
                && let Some(tx) = self.tx_of(conn)
            {
                let ev = Arc::new(OutputEvent::Narrative {
                    kind: NarrativeKind::Say,
                    actor: Some(ObjRef::new("#10", "Nick")),
                    text: RichText::plain(said),
                });
                let _ = tx.send(ev).await;
            }
        })
    }

    fn disconnected(&self, conn: u64) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.disconnects.lock().unwrap().push(conn);
            self.txs.lock().unwrap().remove(&conn);
        })
    }

    fn is_ip_blocked(&self, _addr: IpAddr) -> bool {
        // Mirrors the binary's IpGuard policy: 5 failures arm a tempban.
        self.auth_failures.load(Ordering::Relaxed) >= 5
    }

    fn auth_failed(&self, _addr: IpAddr) {
        self.auth_failures.fetch_add(1, Ordering::Relaxed);
    }
}

// ---- client-side plumbing ----------------------------------------------

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

struct Term {
    channel: russh::Channel<russh::client::Msg>,
    seen: String,
}

/// Drop ANSI CSI sequences (`ESC [ … final`) so assertions match the text
/// a player reads, not the styling.
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

impl Term {
    /// Read channel data until `needle` shows up in everything seen so far
    /// (ANSI styling stripped).
    async fn expect(&mut self, needle: &str) {
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        while !strip_ansi(&self.seen).contains(needle) {
            let msg = tokio::time::timeout_at(deadline, self.channel.wait())
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "timeout waiting for {needle:?}; seen so far: {:?}",
                        self.seen
                    )
                });
            match msg {
                Some(ChannelMsg::Data { data }) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Some(_) => {}
                None => panic!(
                    "channel closed while waiting for {needle:?}; seen: {:?}",
                    self.seen
                ),
            }
        }
    }

    async fn send_line(&mut self, line: &str) {
        let bytes = format!("{line}\r").into_bytes();
        self.channel.data(&bytes[..]).await.expect("send line");
    }

    /// Wait for the server to close the channel/connection.
    async fn expect_close(&mut self) {
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        loop {
            let msg = tokio::time::timeout_at(deadline, self.channel.wait())
                .await
                .expect("timeout waiting for close");
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

fn unique_key_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "mush-net-ssh-loopback-{tag}-{}-{}",
        std::process::id(),
        getrandom_u64()
    ))
}

fn getrandom_u64() -> u64 {
    // Distinct temp paths per run without another dev-dependency.
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

async fn start_server(tag: &str, hooks: Arc<MockHooks>) -> std::net::SocketAddr {
    let cfg = SshConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        host_key_path: unique_key_path(tag).join("host_ed25519"),
        banner: "Welcome to rustmush!\nA MUSH over SSH.".to_owned(),
    };
    let bound = SshServer::bind(cfg, hooks).await.expect("bind");
    let addr = bound.local_addr();
    tokio::spawn(async move {
        let _ = bound.run().await;
    });
    addr
}

async fn open_shell_on(
    handle: &russh::client::Handle<AcceptAll>,
) -> russh::Channel<russh::client::Msg> {
    let channel = handle.channel_open_session().await.expect("open channel");
    channel
        .request_pty(true, "xterm-256color", 80, 24, 0, 0, &[])
        .await
        .expect("pty");
    channel.request_shell(true).await.expect("shell");
    channel
}

// ---- the tests ----------------------------------------------------------

#[tokio::test]
async fn play_flow_create_connect_say_quit() {
    let hooks = Arc::new(MockHooks::default());
    let addr = start_server("play", Arc::clone(&hooks)).await;

    let (handle, mut term) = {
        let config = Arc::new(russh::client::Config::default());
        let mut handle = russh::client::connect(config, addr, AcceptAll)
            .await
            .expect("connect");
        let auth = handle.authenticate_none("play").await.expect("auth-none");
        assert!(auth.success(), "play must be accepted with no auth");
        let channel = open_shell_on(&handle).await;
        (
            handle,
            Term {
                channel,
                seen: String::new(),
            },
        )
    };

    // Classic welcome screen.
    term.expect("Welcome to rustmush!").await;
    term.expect("connect <account> <password>").await;

    // Bad in-band credentials are counted against the IP.
    term.send_line("connect ghost nope").await;
    term.expect("password is wrong").await;
    assert_eq!(hooks.auth_failures.load(Ordering::Relaxed), 1);

    // Unknown verbs get help, not silence.
    term.send_line("frobnicate").await;
    term.expect("Unknown command \"frobnicate\"").await;

    // Create makes account + same-named character and auto-connects.
    term.send_line("create nick s3cretpw").await;
    term.expect("Account created.").await;
    term.expect("Welcome, nick!").await;

    // In-game lines reach the hooks; world output comes back rendered.
    term.send_line("say hi").await;
    term.expect("Nick says, \"hi\"").await;

    // QUIT: the "world" drops the queue; the server closes the transport.
    term.send_line("QUIT").await;
    term.expect_close().await;

    let lines = hooks.lines.lock().unwrap().clone();
    let texts: Vec<&str> = lines.iter().map(|(_, l)| l.as_str()).collect();
    assert!(texts.contains(&"say hi"), "lines seen: {texts:?}");
    assert!(texts.contains(&"QUIT"), "lines seen: {texts:?}");

    // The disconnect hook fires once the transport is torn down.
    let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
    loop {
        if !hooks.disconnects.lock().unwrap().is_empty() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "disconnected hook never fired"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let _ = handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
}

/// The in-band brute-force gate: once the hooks report the address blocked
/// (5 failures), the *already open* welcome screen refuses further
/// `connect` attempts and closes — the tempban is not just for new TCP
/// connections.
#[tokio::test]
async fn in_band_lockout_closes_open_welcome_screen() {
    let hooks = Arc::new(MockHooks::default());
    let addr = start_server("lockout", Arc::clone(&hooks)).await;

    let config = Arc::new(russh::client::Config::default());
    let mut handle = russh::client::connect(config, addr, AcceptAll)
        .await
        .expect("connect");
    let auth = handle.authenticate_none("play").await.expect("auth-none");
    assert!(auth.success());
    let channel = open_shell_on(&handle).await;
    let mut term = Term {
        channel,
        seen: String::new(),
    };
    term.expect("Welcome to rustmush!").await;

    // Five bad guesses ride the pre-auth flood burst (burst 5).
    for _ in 0..5 {
        term.send_line("connect victim nope").await;
    }
    // Wait until all five failures registered server-side.
    let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
    while hooks.auth_failures.load(Ordering::Relaxed) < 5 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "5 in-band failures never registered"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    // Let the flood bucket refill one token, then guess again: the blocked
    // address is refused outright and the connection closes.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    term.send_line("connect victim nope").await;
    term.expect("Too many failed logins from your address")
        .await;
    term.expect_close().await;
    assert_eq!(
        hooks.auth_failures.load(Ordering::Relaxed),
        5,
        "the blocked attempt must never reach the auth backend"
    );

    let _ = handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
}

#[tokio::test]
async fn account_username_password_auth_auto_connects() {
    let hooks = Arc::new(MockHooks::default());
    hooks.seed_account(
        "zoe",
        "pw123",
        vec![CharSummary {
            key: 7,
            name: "Zoe".to_owned(),
        }],
    );
    let addr = start_server("account", Arc::clone(&hooks)).await;

    let config = Arc::new(russh::client::Config::default());
    let mut handle = russh::client::connect(config, addr, AcceptAll)
        .await
        .expect("connect");

    // Wrong password is refused and counted.
    let auth = handle
        .authenticate_password("zoe", "wrong")
        .await
        .expect("auth attempt");
    assert!(!auth.success());
    assert_eq!(hooks.auth_failures.load(Ordering::Relaxed), 1);

    // Right password: in (russh requires a fresh attempt on same handle).
    let auth = handle
        .authenticate_password("zoe", "pw123")
        .await
        .expect("auth attempt");
    assert!(auth.success());

    let channel = open_shell_on(&handle).await;
    let mut term = Term {
        channel,
        seen: String::new(),
    };
    // Exactly one character: auto-connect, no select screen.
    term.expect("Welcome, zoe!").await;
    term.send_line("say hello").await;
    term.expect("Nick says, \"hello\"").await;

    term.send_line("QUIT").await;
    term.expect_close().await;
    let _ = handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
}
