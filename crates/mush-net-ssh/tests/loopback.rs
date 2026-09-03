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
use mush_proto::{NarrativeKind, ObjRef, OutputEvent, PanelFrameKind, RichText, SmolStr, WhoEntry};
use russh::ChannelMsg;
use tokio::sync::mpsc;

const STEP_TIMEOUT: Duration = Duration::from_secs(15);

// ---- mock game ----------------------------------------------------------

#[derive(Default)]
struct MockHooks {
    /// account (lowercase) -> (password, characters)
    accounts: Mutex<HashMap<String, (String, Vec<CharSummary>)>>,
    /// account (lowercase) -> registered OpenSSH public-key lines (M19).
    ssh_keys: Mutex<HashMap<String, Vec<String>>>,
    /// Output queues by conn id (dropping one closes that connection).
    txs: Mutex<HashMap<u64, mpsc::Sender<Arc<OutputEvent>>>>,
    lines: Mutex<Vec<(u64, String)>>,
    disconnects: Mutex<Vec<u64>>,
    /// (conn id, character key) for each character actually connected — lets a
    /// test prove which character `account+char` direct-entered.
    connected_chars: Mutex<Vec<(u64, u64)>>,
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

    /// Register an OpenSSH public-key line on an account (the stage-1 store
    /// method's effect, here in the mock).
    fn register_key(&self, name: &str, openssh: &str) {
        self.ssh_keys
            .lock()
            .unwrap()
            .entry(name.to_ascii_lowercase())
            .or_default()
            .push(openssh.to_owned());
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

    fn auth_account_key<'a>(
        &'a self,
        account: &'a str,
        openssh_key: &'a str,
    ) -> BoxFuture<'a, AuthResult> {
        Box::pin(async move {
            use russh::keys::ssh_key::PublicKey;
            let key = account.to_ascii_lowercase();
            // Parse the offered key; compare by key material, never raw string.
            let Ok(offered) = PublicKey::from_openssh(openssh_key) else {
                return AuthResult::BadCreds;
            };
            let accounts = self.accounts.lock().unwrap();
            let Some((_, chars)) = accounts.get(&key) else {
                return AuthResult::BadCreds;
            };
            let stored = self.ssh_keys.lock().unwrap();
            let matched = stored.get(&key).is_some_and(|lines| {
                lines.iter().any(|line| {
                    PublicKey::from_openssh(line)
                        .is_ok_and(|pk| pk.key_data() == offered.key_data())
                })
            });
            if matched {
                AuthResult::Ok {
                    account: key.clone(),
                    characters: chars.clone(),
                }
            } else {
                AuthResult::BadCreds
            }
        })
    }

    fn connect_character<'a>(
        &'a self,
        conn: NewConn,
        account: &'a str,
        character: u64,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let tx = conn.tx.clone();
            self.connected_chars
                .lock()
                .unwrap()
                .push((conn.id, character));
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

    fn query(&self, conn: u64, name: String) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let Some(tx) = self.tx_of(conn) else {
                return;
            };
            // The TUI widgets submit read-only queries; answer each with the
            // Reply/Panel a real binary would (via the world's query handlers).
            let ev = match name.as_str() {
                "who" => OutputEvent::Who(vec![WhoEntry {
                    name: "Alice".into(),
                    alias: None,
                    idle_s: 0,
                    conn_s: 42,
                    location: None,
                    doing: None,
                }]),
                "mail.list" => OutputEvent::Panel {
                    feed: SmolStr::new_static("mail"),
                    kind: PanelFrameKind::Keyframe,
                    data: serde_json::json!({
                        "folder": "inbox",
                        "entries": [
                            { "index": 1, "status": "unread",
                              "from": { "name": "Bento" }, "subject": "Welcome" }
                        ],
                    }),
                },
                "intel.list" => OutputEvent::Panel {
                    feed: SmolStr::new_static("intel"),
                    kind: PanelFrameKind::Keyframe,
                    data: serde_json::json!({
                        "count": 1,
                        "records": [
                            { "subject": "Mallory", "kind": "spoke",
                              "detail": "plotting", "provenance": "seen by Snitch",
                              "room": "Bar", "ts": 0 }
                        ],
                    }),
                },
                _ => return,
            };
            let _ = tx.send(Arc::new(ev)).await;
        })
    }

    fn disconnected(&self, conn: u64) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.disconnects.lock().unwrap().push(conn);
            self.txs.lock().unwrap().remove(&conn);
        })
    }

    fn create_character<'a>(
        &'a self,
        account: &'a str,
        name: &'a str,
    ) -> BoxFuture<'a, Result<CharSummary, String>> {
        Box::pin(async move {
            let key = account.to_ascii_lowercase();
            let mut accounts = self.accounts.lock().unwrap();
            let Some((_, chars)) = accounts.get_mut(&key) else {
                return Err("No such account.".to_owned());
            };
            // Mirrors the binary's [game].max_characters cap (2 here).
            if chars.len() >= 2 {
                return Err("That account already has the maximum number of characters.".to_owned());
            }
            let char_key = 100 + self.next_char_key.fetch_add(1, Ordering::Relaxed) as u64;
            let c = CharSummary {
                key: char_key,
                name: name.to_owned(),
            };
            chars.push(c.clone());
            Ok(c)
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

    /// Send raw bytes (TUI hotkeys, e.g. Tab to cycle the secondary pane).
    async fn send_bytes(&mut self, bytes: &[u8]) {
        self.channel.data(bytes).await.expect("send bytes");
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

/// The M2 character-select screen: multiple characters show a numbered
/// menu (with the newchar hint), picking by number connects, `newchar`
/// creates-and-connects, the per-account cap refuses politely, and an
/// account with zero characters lands on the same screen instead of a
/// dead end.
#[tokio::test]
async fn char_select_menu_newchar_and_cap() {
    let hooks = Arc::new(MockHooks::default());
    hooks.seed_account(
        "duo",
        "pw123",
        vec![
            CharSummary {
                key: 7,
                name: "Zoe".to_owned(),
            },
            CharSummary {
                key: 8,
                name: "Max".to_owned(),
            },
        ],
    );
    hooks.seed_account("fresh", "pw123", vec![]);
    let addr = start_server("charselect", Arc::clone(&hooks)).await;
    let config = Arc::new(russh::client::Config::default());

    // Two characters: numbered menu, pick #2.
    let mut handle = russh::client::connect(Arc::clone(&config), addr, AcceptAll)
        .await
        .expect("connect");
    assert!(
        handle
            .authenticate_password("duo", "pw123")
            .await
            .expect("auth")
            .success()
    );
    let mut term = Term {
        channel: open_shell_on(&handle).await,
        seen: String::new(),
    };
    term.expect("Characters on duo:").await;
    term.expect("1) Zoe").await;
    term.expect("2) Max").await;
    term.expect("newchar <Name>").await;
    term.send_line("2").await;
    term.expect("Welcome, duo!").await;

    // Cap: the mock allows 2 characters; a third is refused and the menu
    // comes back (the select screen survives the error).
    let mut handle2 = russh::client::connect(Arc::clone(&config), addr, AcceptAll)
        .await
        .expect("connect");
    assert!(
        handle2
            .authenticate_password("duo", "pw123")
            .await
            .expect("auth")
            .success()
    );
    let mut term2 = Term {
        channel: open_shell_on(&handle2).await,
        seen: String::new(),
    };
    term2.expect("Characters on duo:").await;
    term2.send_line("newchar Trinity").await;
    term2.expect("maximum number of characters").await;
    term2
        .expect("Enter a number, newchar <Name>, or QUIT:")
        .await;
    term2.send_line("Zoe").await; // picking by name still works
    term2.expect("Welcome, duo!").await;

    // Zero characters: the select screen offers newchar instead of a dead
    // end, and creating auto-connects.
    let mut handle3 = russh::client::connect(Arc::clone(&config), addr, AcceptAll)
        .await
        .expect("connect");
    assert!(
        handle3
            .authenticate_password("fresh", "pw123")
            .await
            .expect("auth")
            .success()
    );
    let mut term3 = Term {
        channel: open_shell_on(&handle3).await,
        seen: String::new(),
    };
    term3.expect("has no characters yet").await;
    term3.send_line("newchar").await;
    term3.expect("Usage: newchar <Name>").await;
    term3.send_line("newchar Neo").await;
    term3.expect("Character Neo created.").await;
    term3.expect("Welcome, fresh!").await;

    for h in [handle, handle2, handle3] {
        let _ = h.disconnect(russh::Disconnect::ByApplication, "", "").await;
    }
}

// ---- public-key auth fixtures (M19) -------------------------------------

/// A fixed ed25519 OpenSSH **private** key (test-only; generated once with
/// `ssh-keygen -t ed25519`). Embedding a fixed pair keeps the loopback test
/// deterministic — no RNG, no filesystem. The matching public line is
/// derived at runtime via [`priv_key`]`.public_key()`.
const TEST_PRIV: &str = "\
-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACAimbLmJ8+bw424vILUEi+XY4bI9f/KJQPsh5DzC6fyogAAAKANWMViDVjF
YgAAAAtzc2gtZWQyNTUxOQAAACAimbLmJ8+bw424vILUEi+XY4bI9f/KJQPsh5DzC6fyog
AAAEAxj4SkgEcYjlGJjsCyLFctOcsJ/hm6ZWXlqQgTZw5esSKZsuYnz5vDjbi8gtQSL5dj
hsj1/8olA+yHkPMLp/KiAAAAFmxvb3BiYWNrLXRlc3RAcnVzdG11c2gBAgMEBQYH
-----END OPENSSH PRIVATE KEY-----
";

/// A second, unrelated ed25519 private key — never registered, so it must
/// fall through public-key auth to the password fallback.
const WRONG_PRIV: &str = "\
-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACDIklxIsFZ/Zadggeq52MwqDO/a/eFZXR/6b4VgHPcERwAAAJi9ZbfhvWW3
4QAAAAtzc2gtZWQyNTUxOQAAACDIklxIsFZ/Zadggeq52MwqDO/a/eFZXR/6b4VgHPcERw
AAAEAlMRUdks8bgtY+gCvj5eL0OLVEDFKWe3JfaP4GX7rTlsiSXEiwVn9lp2CB6rnYzCoM
79r94VldH/pvhWAc9wRHAAAADndyb25nQHJ1c3RtdXNoAQIDBAUGBw==
-----END OPENSSH PRIVATE KEY-----
";

fn priv_key(pem: &str) -> russh::keys::PrivateKey {
    russh::keys::PrivateKey::from_openssh(pem).expect("parse test private key")
}

/// The OpenSSH public-key line ("type base64 comment") for a private key —
/// what the account registers via `@sshkey/add` / the stage-1 store method.
fn public_line(pem: &str) -> String {
    priv_key(pem)
        .public_key()
        .to_openssh()
        .expect("render public key")
}

/// Wrap a private key for russh's `authenticate_publickey` (ed25519 ignores
/// the hash-alg argument).
fn signing_key(pem: &str) -> russh::keys::PrivateKeyWithHashAlg {
    russh::keys::PrivateKeyWithHashAlg::new(Arc::new(priv_key(pem)), None)
}

/// A stored ed25519 key authenticates and lands in game with no password:
/// `ssh -i key nick@game` → auto-connect (single character).
#[tokio::test]
async fn pubkey_auth_lands_in_game() {
    let hooks = Arc::new(MockHooks::default());
    hooks.seed_account(
        "nick",
        "pw123",
        vec![CharSummary {
            key: 7,
            name: "Nick".to_owned(),
        }],
    );
    hooks.register_key("nick", &public_line(TEST_PRIV));
    let addr = start_server("pubkey", Arc::clone(&hooks)).await;

    let config = Arc::new(russh::client::Config::default());
    let mut handle = russh::client::connect(config, addr, AcceptAll)
        .await
        .expect("connect");
    let auth = handle
        .authenticate_publickey("nick", signing_key(TEST_PRIV))
        .await
        .expect("auth attempt");
    assert!(auth.success(), "a registered key must authenticate");
    // Not one failure was counted on the happy path.
    assert_eq!(hooks.auth_failures.load(Ordering::Relaxed), 0);

    let mut term = Term {
        channel: open_shell_on(&handle).await,
        seen: String::new(),
    };
    // Single character → auto-connect, straight into the game.
    term.expect("Welcome, nick!").await;
    term.send_line("say hi").await;
    term.expect("Nick says, \"hi\"").await;

    term.send_line("QUIT").await;
    term.expect_close().await;
    let _ = handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
}

/// Regression (M19 deploy): a real OpenSSH client probes with `none` FIRST to
/// discover the method list, so the reject MUST advertise `publickey` — else
/// `ssh -i key <account>@host` never offers the key and drops to password.
/// (The other pubkey tests offer the key directly, bypassing this discovery
/// step, so they cannot catch a missing advertisement.) Also asserts that an
/// unregistered-but-owned key offer is NOT counted as an auth failure (so an
/// ssh-agent with several keys cannot self-trip the tempban).
#[tokio::test]
async fn auth_none_advertises_publickey_and_key_miss_is_not_counted() {
    let hooks = Arc::new(MockHooks::default());
    hooks.seed_account(
        "nick",
        "pw123",
        vec![CharSummary {
            key: 7,
            name: "Nick".to_owned(),
        }],
    );
    hooks.register_key("nick", &public_line(TEST_PRIV));
    let addr = start_server("pk-discovery", Arc::clone(&hooks)).await;

    let config = Arc::new(russh::client::Config::default());
    let mut handle = russh::client::connect(config, addr, AcceptAll)
        .await
        .expect("connect");

    // The none-probe an OpenSSH client does first: the server must offer
    // publickey (so the client goes on to sign its key) plus the password
    // fallback.
    let none = handle.authenticate_none("nick").await.expect("auth-none");
    match none {
        russh::client::AuthResult::Failure {
            remaining_methods, ..
        } => {
            assert!(
                remaining_methods.contains(&russh::MethodKind::PublicKey),
                "publickey must be advertised after the none-probe (else ssh -i never offers the key)"
            );
            assert!(
                remaining_methods.contains(&russh::MethodKind::Password),
                "password fallback must remain advertised"
            );
        }
        other => panic!("expected a Failure with a method list, got {other:?}"),
    }

    // A wrong key the client provably owns is not ban-worthy.
    let wrong = handle
        .authenticate_publickey("nick", signing_key(WRONG_PRIV))
        .await
        .expect("auth attempt");
    assert!(
        !wrong.success(),
        "an unregistered key must not authenticate"
    );
    assert_eq!(
        hooks.auth_failures.load(Ordering::Relaxed),
        0,
        "an unregistered-but-owned key offer must NOT count toward the tempban"
    );

    // The registered key still authenticates on the same connection.
    let ok = handle
        .authenticate_publickey("nick", signing_key(TEST_PRIV))
        .await
        .expect("auth attempt");
    assert!(ok.success(), "the registered key authenticates");
    let _ = handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
}

/// `ssh -i key account+char@game` direct-enters the named character (the
/// zero-step login), skipping the character-select screen — and only ever a
/// character the authenticated account owns.
#[tokio::test]
async fn pubkey_plus_char_direct_enters_owned_character() {
    let hooks = Arc::new(MockHooks::default());
    hooks.seed_account(
        "duo",
        "pw123",
        vec![
            CharSummary {
                key: 7,
                name: "Zoe".to_owned(),
            },
            CharSummary {
                key: 8,
                name: "Max".to_owned(),
            },
        ],
    );
    hooks.register_key("duo", &public_line(TEST_PRIV));
    let addr = start_server("pubkey-plus", Arc::clone(&hooks)).await;

    let config = Arc::new(russh::client::Config::default());
    let mut handle = russh::client::connect(config, addr, AcceptAll)
        .await
        .expect("connect");
    let auth = handle
        .authenticate_publickey("duo+Max", signing_key(TEST_PRIV))
        .await
        .expect("auth attempt");
    assert!(auth.success());

    let mut term = Term {
        channel: open_shell_on(&handle).await,
        seen: String::new(),
    };
    // Direct-enter: the game greeting, never the numbered select menu.
    term.expect("Welcome, duo!").await;
    assert!(
        !strip_ansi(&term.seen).contains("Characters on duo:"),
        "a matching +char must skip the character-select screen"
    );
    // Prove it entered Max (key 8), not Zoe.
    let connected = hooks.connected_chars.lock().unwrap().clone();
    assert_eq!(
        connected.iter().map(|(_, k)| *k).collect::<Vec<_>>(),
        vec![8],
        "the +char must direct-enter the named owned character (Max=8)"
    );

    term.send_line("QUIT").await;
    term.expect_close().await;
    let _ = handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
}

/// A `+char` that the account does NOT own (typo, or a cross-account name)
/// falls to *this account's* character-select screen — never auto-enters and
/// never reveals another account (invariant S2).
#[tokio::test]
async fn pubkey_plus_unowned_char_falls_to_char_select() {
    let hooks = Arc::new(MockHooks::default());
    hooks.seed_account(
        "duo",
        "pw123",
        vec![
            CharSummary {
                key: 7,
                name: "Zoe".to_owned(),
            },
            CharSummary {
                key: 8,
                name: "Max".to_owned(),
            },
        ],
    );
    hooks.register_key("duo", &public_line(TEST_PRIV));
    let addr = start_server("pubkey-unowned", Arc::clone(&hooks)).await;

    let config = Arc::new(russh::client::Config::default());
    let mut handle = russh::client::connect(config, addr, AcceptAll)
        .await
        .expect("connect");
    // "Ghost" belongs to no one on this account.
    let auth = handle
        .authenticate_publickey("duo+Ghost", signing_key(TEST_PRIV))
        .await
        .expect("auth attempt");
    assert!(auth.success());

    let mut term = Term {
        channel: open_shell_on(&handle).await,
        seen: String::new(),
    };
    // Lands on this account's own select screen; no character auto-entered.
    term.expect("Characters on duo:").await;
    term.expect("1) Zoe").await;
    term.expect("2) Max").await;
    assert!(
        hooks.connected_chars.lock().unwrap().is_empty(),
        "an unowned +char must not connect any character"
    );

    // Picking an owned character from the menu still works.
    term.send_line("1").await;
    term.expect("Welcome, duo!").await;

    let _ = handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
}

/// A wrong (unregistered) key does not grant access and does not kill the
/// fallback: public-key auth fails, then password auth on the same handle
/// succeeds (invariant S1 — a key miss rejects toward password, never a hard
/// reject).
#[tokio::test]
async fn wrong_pubkey_falls_back_to_password() {
    let hooks = Arc::new(MockHooks::default());
    hooks.seed_account(
        "nick",
        "pw123",
        vec![CharSummary {
            key: 7,
            name: "Nick".to_owned(),
        }],
    );
    // Only the correct key is registered.
    hooks.register_key("nick", &public_line(TEST_PRIV));
    let addr = start_server("pubkey-wrong", Arc::clone(&hooks)).await;

    let config = Arc::new(russh::client::Config::default());
    let mut handle = russh::client::connect(config, addr, AcceptAll)
        .await
        .expect("connect");

    // The wrong key is refused (russh verified its signature; our lookup finds
    // no match) and the client is steered toward password.
    let auth = handle
        .authenticate_publickey("nick", signing_key(WRONG_PRIV))
        .await
        .expect("auth attempt");
    assert!(!auth.success(), "an unregistered key must not authenticate");

    // Password fallback on the same handle still works.
    let auth = handle
        .authenticate_password("nick", "pw123")
        .await
        .expect("auth attempt");
    assert!(auth.success(), "password fallback must survive a key miss");

    let mut term = Term {
        channel: open_shell_on(&handle).await,
        seen: String::new(),
    };
    term.expect("Welcome, nick!").await;

    term.send_line("QUIT").await;
    term.expect_close().await;
    let _ = handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
}

// ---- account-username password auth (M2) --------------------------------

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

/// The M15 TUI plumbing end-to-end: `@tui` enters the full-screen ratatui
/// frame (WHO rail + main frame + input line), a command typed inside it
/// reaches the world and its output lands in the scrollback, and `@notui`
/// restores byte-for-byte line mode (a subsequent `say` renders the classic
/// way again).
#[tokio::test]
async fn tui_enter_shows_frame_then_notui_restores_line_mode() {
    let hooks = Arc::new(MockHooks::default());
    let addr = start_server("tui", Arc::clone(&hooks)).await;

    let config = Arc::new(russh::client::Config::default());
    let mut handle = russh::client::connect(config, addr, AcceptAll)
        .await
        .expect("connect");
    assert!(
        handle
            .authenticate_none("play")
            .await
            .expect("auth-none")
            .success()
    );
    let mut term = Term {
        channel: open_shell_on(&handle).await,
        seen: String::new(),
    };
    term.expect("Welcome to rustmush!").await;
    term.send_line("create nick s3cretpw").await;
    term.expect("Welcome, nick!").await;

    // Enter the TUI: the frame carries the main-frame title, the WHO rail,
    // and (after the auto-submitted `who` query replies) a listed player.
    term.send_line("@tui").await;
    term.expect("rustmush").await; // main-frame block title
    term.expect("WHO").await; // rail block title
    term.expect("Alice").await; // WHO rail entry from the query reply
    // Entering the TUI switches to the alternate screen and hides the cursor.
    assert!(
        term.seen.contains("\u{1b}[?1049h"),
        "expected the alternate-screen enter sequence"
    );

    // A command typed at the TUI input line reaches the world; its narrative
    // reply is applied to the scrollback and repainted.
    term.send_line("say hello").await;
    term.expect("hello").await;
    assert!(
        hooks
            .lines
            .lock()
            .unwrap()
            .iter()
            .any(|(_, l)| l == "say hello"),
        "the TUI input line must forward commands to the world"
    );

    // Tab cycles the switchable secondary pane. First press opens the @mail
    // widget, which submits the `mail.list` query; its Panel keyframe reply
    // repaints the pane with the listed message.
    term.send_bytes(&[0x09]).await;
    term.expect("mail").await; // @mail pane title
    term.expect("Bento").await; // sender from the mail.list reply

    // Second press opens the +intel widget (submits `intel.list`); its reply
    // repaints the pane with the filed record.
    term.send_bytes(&[0x09]).await;
    term.expect("intel").await; // +intel pane title
    term.expect("Mallory").await; // subject from the intel.list reply

    // The widget queries reached the world through the read-only query seam.
    assert!(
        hooks
            .lines
            .lock()
            .unwrap()
            .iter()
            .all(|(_, l)| l != "mail.list" && l != "intel.list"),
        "widget refreshes must go through hooks.query, never hooks.line"
    );

    // `@notui` leaves the alternate screen (restoring the cursor) and
    // restores line mode; a fresh `say` renders the classic way again.
    let before_notui = term.seen.len();
    term.send_line("@notui").await;
    term.send_line("say back").await;
    term.expect("Nick says, \"back\"").await;
    assert!(
        term.seen[before_notui..].contains("\u{1b}[?1049l"),
        "expected the alternate-screen leave sequence after @notui"
    );

    term.send_line("QUIT").await;
    term.expect_close().await;
    let _ = handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
}
