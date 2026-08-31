//! The M1 exit criterion as one scripted end-to-end journey over real SSH.
//!
//! Boots the whole server (world task, session layer, store-backed
//! accounts, IP guard, SSH listener) on an ephemeral port with a scratch
//! data dir —
//! production wiring except for port and paths — then drives three russh
//! clients through the classic session:
//!
//! - God (a seeded account linked to `#1`) connects via the welcome
//!   screen's `connect`, runs `@slowtest` (typed async re-entry, with a
//!   `think` interleaved to prove the loop never blocked) and `@dig`s a
//!   room with exits both ways.
//! - Alice and Bob `create` accounts in-band (argon2 + world round trip),
//!   land in Limbo together, `say` to each other, and Bob walks east and
//!   back through the dug exits while Alice watches the announcements.
//! - `WHO` lists all three with well-formed idle/conn columns and a conn
//!   time that has really advanced (>= the 3 s the slowtest step took).
//! - Alice QUITs cleanly (farewell + close + room announcement); Bob drops
//!   his transport without QUIT (link-dead announcement), reconnects
//!   within the grace ("has reconnected."), and can still talk.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_core::{GOD, LIMBO, ObjKind};
use mush_session::{SessionConfig, SessionManager};
use russh::ChannelMsg;

use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore, SshKeyRec};
use mushd::accounts::{Accounts, hash_password};
use mushd::ipguard::IpGuard;
use mushd::ssh_bridge::{SshBridge, start_listener};
use mushd::world_task;

const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const GM_PASSWORD: &str = "gm-secret-1";

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mushd-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Boot the full production wiring on an ephemeral port. The `boss` account
/// is seeded and linked to God (`#1`) so a staff connection can drive
/// `@dig` and `@slowtest` over the same SSH front door as everyone else.
async fn boot_server(dir: &std::path::Path) -> SocketAddr {
    std::fs::create_dir_all(dir).unwrap();
    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(120),
        ..SessionConfig::default()
    }));
    let (world, _join) = world_task::spawn(world_task::boot_engine(), Arc::clone(&sessions));
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
    let accounts = Arc::new(Accounts::new(store, world.clone(), 3));
    let bridge = Arc::new(SshBridge::new(
        accounts,
        Arc::clone(&sessions),
        world,
        Arc::new(IpGuard::new()),
    ));
    start_listener(
        "127.0.0.1:0".parse().unwrap(),
        dir,
        "Welcome to the e2e MUSH!".to_string(),
        bridge,
    )
    .await
    .expect("listener")
    .addr()
}

// ---- public-key auth e2e (M19) ------------------------------------------

/// A fixed ed25519 private key (test-only). Its public half is registered on
/// the seeded account; the client authenticates with the private key.
const TEST_PRIV: &str = "\
-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACAimbLmJ8+bw424vILUEi+XY4bI9f/KJQPsh5DzC6fyogAAAKANWMViDVjF
YgAAAAtzc2gtZWQyNTUxOQAAACAimbLmJ8+bw424vILUEi+XY4bI9f/KJQPsh5DzC6fyog
AAAEAxj4SkgEcYjlGJjsCyLFctOcsJ/hm6ZWXlqQgTZw5esSKZsuYnz5vDjbi8gtQSL5dj
hsj1/8olA+yHkPMLp/KiAAAAFmxvb3BiYWNrLXRlc3RAcnVzdG11c2gBAgMEBQYH
-----END OPENSSH PRIVATE KEY-----
";

/// A second, unregistered ed25519 private key — must fall to password.
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

fn public_line(pem: &str) -> String {
    priv_key(pem)
        .public_key()
        .to_openssh()
        .expect("render public key")
}

fn signing_key(pem: &str) -> russh::keys::PrivateKeyWithHashAlg {
    russh::keys::PrivateKeyWithHashAlg::new(Arc::new(priv_key(pem)), None)
}

/// Boot full production wiring with a `duo` account that owns two real
/// characters (Nick, Ada). `seed_key` controls whether `TEST_PRIV`'s public key
/// is registered up front (stage-2 path) or the account starts keyless so the
/// in-game `@sshkey` command can add one; `with_staff` wires the staff-op
/// executor (via `spawn_persist`) that the `@sshkey`/`@newpassword` commands
/// need — the stage-2 pubkey journey drives no server commands, so it uses the
/// leaner `spawn`.
async fn boot_duo_server(dir: &std::path::Path, seed_key: bool, with_staff: bool) -> SocketAddr {
    std::fs::create_dir_all(dir).unwrap();
    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(120),
        ..SessionConfig::default()
    }));
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());

    // Build the engine and its two characters. When the staff executor is
    // wanted, the registry also carries the server-side commands (@sshkey).
    let mut world = mush_core::World::new_minimal();
    world.set_now(mushd::unix_now_s());
    let staff = Arc::new(mushd::staff::StaffQueue::new());
    let mut registry = mushd::world_task::base_registry();
    if with_staff {
        mushd::staff::register_server_commands(&mut registry, &staff).unwrap();
    }
    let mut engine = mush_core::Engine::new(world, registry);
    let nick = engine
        .world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    let ada = engine
        .world
        .create(ObjKind::Player, "Ada", GOD, Some(LIMBO))
        .unwrap();

    store
        .create_account(AccountRec {
            name: "duo".to_string(),
            argon2_hash: hash_password(GM_PASSWORD).unwrap(),
            created: 0,
            characters: vec![
                CharacterRef {
                    id: nick,
                    name: "Nick".to_string(),
                },
                CharacterRef {
                    id: ada,
                    name: "Ada".to_string(),
                },
            ],
            ssh_keys: vec![],
        })
        .await
        .unwrap();
    if seed_key {
        // Register the public key via the stage-1 store method (the same write
        // `@sshkey/add` performs), proving the gateway reads it fresh per auth.
        let publine = public_line(TEST_PRIV);
        let key = SshKeyRec {
            openssh: publine.clone(),
            fingerprint: russh::keys::ssh_key::PublicKey::from_openssh(&publine)
                .unwrap()
                .fingerprint(russh::keys::HashAlg::Sha256)
                .to_string(),
            added: 0,
        };
        assert!(store.set_account_ssh_keys("duo", &[key]).await.unwrap());
    }

    let world = if with_staff {
        mushd::world_task::spawn_persist(
            engine,
            Arc::clone(&sessions),
            mushd::world_task::ServerDeps {
                metrics: Arc::new(mush_net_web::Metrics::new()),
                store: Arc::clone(&store),
                guard: Arc::new(IpGuard::new()),
                staff,
                llm: None,
            },
        )
        .handle
    } else {
        world_task::spawn(engine, Arc::clone(&sessions)).0
    };
    let accounts = Arc::new(Accounts::new(store, world.clone(), 3));
    let bridge = Arc::new(SshBridge::new(
        accounts,
        Arc::clone(&sessions),
        world,
        Arc::new(IpGuard::new()),
    ));
    start_listener(
        "127.0.0.1:0".parse().unwrap(),
        dir,
        "Welcome to the e2e MUSH!".to_string(),
        bridge,
    )
    .await
    .expect("listener")
    .addr()
}

/// The stage-2 harness: `duo` with `TEST_PRIV`'s public key already registered.
async fn boot_pubkey_server(dir: &std::path::Path) -> SocketAddr {
    boot_duo_server(dir, true, false).await
}

/// Open a PTY shell on an already-authenticated handle.
async fn shell_term(handle: &russh::client::Handle<AcceptAll>) -> Term {
    let channel = handle.channel_open_session().await.expect("open channel");
    channel
        .request_pty(true, "xterm-256color", 80, 24, 0, 0, &[])
        .await
        .expect("pty");
    channel.request_shell(true).await.expect("shell");
    Term {
        channel,
        seen: String::new(),
    }
}

/// The M19 public-key journey over the REAL bridge (gateway `verify_pubkey`
/// + server `auth_publickey`/`_offered` + `+char` parse):
///
/// - `ssh -i key duo+Ada@game` authenticates by key alone and direct-enters
///   the named owned character (skips char-select).
/// - `ssh -i key duo@game` (no `+char`) authenticates by key and lands on the
///   two-character select screen.
/// - `ssh -i wrongkey duo@game` fails public-key auth and falls back to the
///   account password, which succeeds.
/// - `ssh -i key duo+Ghost@game` (a character the account does not own) falls
///   to this account's own char-select — never enters another account.
#[tokio::test(flavor = "multi_thread")]
async fn m19_pubkey_and_plus_char_journey() {
    let dir = scratch_dir("pubkey");
    let addr = boot_pubkey_server(&dir).await;
    let config = Arc::new(russh::client::Config::default());

    // --- key + `+Ada`: zero-step direct-enter of the named owned char. ----
    let mut h1 = russh::client::connect(Arc::clone(&config), addr, AcceptAll)
        .await
        .expect("connect");
    assert!(
        h1.authenticate_publickey("duo+Ada", signing_key(TEST_PRIV))
            .await
            .expect("auth")
            .success(),
        "a registered key must authenticate"
    );
    let mut t1 = shell_term(&h1).await;
    t1.expect("Connected as Ada (account duo, 1 connection).")
        .await;
    t1.expect("Limbo(#0)").await;

    // --- key, no `+char`: lands on the two-character select screen. -------
    let mut h2 = russh::client::connect(Arc::clone(&config), addr, AcceptAll)
        .await
        .expect("connect");
    assert!(
        h2.authenticate_publickey("duo", signing_key(TEST_PRIV))
            .await
            .expect("auth")
            .success()
    );
    let mut t2 = shell_term(&h2).await;
    t2.expect("Characters on duo:").await;
    t2.expect("Nick").await;
    t2.expect("Ada").await;

    // --- wrong key: public-key auth fails, password fallback succeeds. ----
    let mut h3 = russh::client::connect(Arc::clone(&config), addr, AcceptAll)
        .await
        .expect("connect");
    assert!(
        !h3.authenticate_publickey("duo", signing_key(WRONG_PRIV))
            .await
            .expect("auth")
            .success(),
        "an unregistered key must not authenticate"
    );
    assert!(
        h3.authenticate_password("duo", GM_PASSWORD)
            .await
            .expect("auth")
            .success(),
        "password fallback must survive a key miss"
    );
    let mut t3 = shell_term(&h3).await;
    t3.expect("Characters on duo:").await;

    // --- key + unowned `+Ghost`: falls to this account's own char-select. -
    let mut h4 = russh::client::connect(Arc::clone(&config), addr, AcceptAll)
        .await
        .expect("connect");
    assert!(
        h4.authenticate_publickey("duo+Ghost", signing_key(TEST_PRIV))
            .await
            .expect("auth")
            .success()
    );
    let mut t4 = shell_term(&h4).await;
    t4.expect("Characters on duo:").await;
    t4.expect("Nick").await;
    // Picking an owned character from the menu still connects.
    t4.send_line("Nick").await;
    t4.expect("Connected as Nick (account duo, 1 connection).")
        .await;

    for h in [h1, h2, h3, h4] {
        let _ = h.disconnect(russh::Disconnect::ByApplication, "", "").await;
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Log in by password (landing on the char-select) and connect the named
/// owned character, returning its live terminal.
async fn password_into_char(
    addr: SocketAddr,
    config: &Arc<russh::client::Config>,
    account: &str,
    password: &str,
    character: &str,
) -> (russh::client::Handle<AcceptAll>, Term) {
    let mut handle = russh::client::connect(Arc::clone(config), addr, AcceptAll)
        .await
        .expect("connect");
    assert!(
        handle
            .authenticate_password(account, password)
            .await
            .expect("auth")
            .success(),
        "password login must succeed"
    );
    let mut term = shell_term(&handle).await;
    term.expect(&format!("Characters on {account}:")).await;
    term.send_line(character).await;
    term.expect(&format!("Connected as {character}")).await;
    (handle, term)
}

/// The M19 section-D journey: the in-game `@sshkey` command drives real SSH
/// public-key auth end to end.
///
/// - `duo` starts with NO registered keys; a password login as Nick runs
///   `@sshkey/list` (empty), then `@sshkey/add <public line>`.
/// - A FRESH `ssh -i key duo+Ada@game` now authenticates by that key alone and
///   direct-enters Ada — proving the gateway read the freshly written key (S4)
///   and that `@sshkey` reached the caller's own account (S5).
/// - `@sshkey/list` shows the fingerprint + comment; `@sshkey/del 1` removes it.
/// - A FRESH key connection then FAILS public-key auth (falls back to password).
#[tokio::test(flavor = "multi_thread")]
async fn m19_sshkey_command_registers_key_e2e() {
    let dir = scratch_dir("sshkey-cmd");
    let addr = boot_duo_server(&dir, false, true).await;
    let config = Arc::new(russh::client::Config::default());

    // --- password login as Nick; the account has no keys yet. -------------
    let (nick_handle, mut nick) =
        password_into_char(addr, &config, "duo", GM_PASSWORD, "Nick").await;
    let m = nick.mark();
    nick.send_line("@sshkey/list").await;
    nick.expect_after(m, "no registered SSH keys").await;

    // --- add the public key in-game (ack-gated confirmation). -------------
    let publine = public_line(TEST_PRIV);
    let m = nick.mark();
    nick.send_line(&format!("@sshkey/add {publine}")).await;
    nick.expect_after(m, "key SHA256:").await;
    nick.expect_after(m, "added").await;

    // --- a FRESH key connection now authenticates (S4) and direct-enters. -
    let mut hk = russh::client::connect(Arc::clone(&config), addr, AcceptAll)
        .await
        .expect("connect");
    assert!(
        hk.authenticate_publickey("duo+Ada", signing_key(TEST_PRIV))
            .await
            .expect("auth")
            .success(),
        "the just-added key must authenticate on the next connection"
    );
    let mut tk = shell_term(&hk).await;
    tk.expect("Connected as Ada (account duo, 1 connection).").await;

    // --- list shows the fingerprint + comment; then delete it. ------------
    let m = nick.mark();
    nick.send_line("@sshkey/list").await;
    nick.expect_after(m, "SHA256:").await;
    nick.expect_after(m, "loopback-test@rustmush").await;

    let m = nick.mark();
    nick.send_line("@sshkey/del 1").await;
    nick.expect_after(m, "removed key SHA256:").await;

    // --- after deletion a FRESH key connection FAILS public-key auth. -----
    let mut hg = russh::client::connect(Arc::clone(&config), addr, AcceptAll)
        .await
        .expect("connect");
    assert!(
        !hg.authenticate_publickey("duo", signing_key(TEST_PRIV))
            .await
            .expect("auth")
            .success(),
        "a removed key must no longer authenticate"
    );
    // Password fallback still works after the key miss.
    assert!(
        hg.authenticate_password("duo", GM_PASSWORD)
            .await
            .expect("auth")
            .success()
    );

    for h in [nick_handle, hk, hg] {
        let _ = h.disconnect(russh::Disconnect::ByApplication, "", "").await;
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The M20 `@newchar` journey over the REAL bridge: a player creates a
/// second (well, third) character on their own account in-game, and a FRESH
/// connection can reach it both at the character-select screen and directly
/// via `<account>+<NewName>`.
///
/// `duo` starts with Nick + Ada (2 chars) and `TEST_PRIV`'s public key
/// registered; the cap is 3 (the harness default). A password login as Nick:
///
/// - `@newchar 9bad` (starts with a digit) and `@newchar Nick` (already held)
///   are refused up front — no character is created (N2).
/// - `@newchar Casey` creates the third character (ack-gated confirmation, N3).
/// - `@newchar Dana` now hits the cap of 3 and is refused (N2) — the caller is
///   still Nick throughout (no mid-session switch, N4).
/// - A FRESH password login lands on a char-select that now lists Casey, and
///   picking it connects (N4, char-select).
/// - A FRESH `ssh -i key duo+Casey@game` authenticates by key and direct-enters
///   Casey (N4, `<account>+<name>` login) — proving the account link is live.
#[tokio::test(flavor = "multi_thread")]
async fn m20_newchar_creates_playable_second_character_e2e() {
    let dir = scratch_dir("newchar");
    // seed_key = true (register TEST_PRIV so duo+Casey works), with_staff = true
    // (wire the staff-op executor @newchar needs); cap is the harness default 3.
    let addr = boot_duo_server(&dir, true, true).await;
    let config = Arc::new(russh::client::Config::default());

    // --- password login as Nick; the account has Nick + Ada (2 of 3). -----
    let (nick_handle, mut nick) =
        password_into_char(addr, &config, "duo", GM_PASSWORD, "Nick").await;

    // --- a bad name is refused up front; nothing is created (N2). ---------
    let m = nick.mark();
    nick.send_line("@newchar 9bad").await;
    nick.expect_after(m, "character names start with a letter").await;

    // --- a name already held is refused up front (N2). --------------------
    let m = nick.mark();
    nick.send_line("@newchar Ada").await;
    nick.expect_after(m, "already taken").await;

    // --- @newchar Casey: the third character is created (ack-gated, N3). --
    let m = nick.mark();
    nick.send_line("@newchar Casey").await;
    nick.expect_after(m, "created Casey").await;
    nick.expect_after(m, "duo+Casey").await;

    // --- @newchar Dana now hits the cap of 3 and is refused (N2). ---------
    let m = nick.mark();
    nick.send_line("@newchar Dana").await;
    nick.expect_after(m, "maximum of 3 characters").await;

    // The caller was never switched mid-session (N4): Nick is still in play.
    let m = nick.mark();
    nick.send_line("say still nick").await;
    nick.expect_after(m, "Nick says, \"still nick\"").await;

    // --- a FRESH password login lands on a char-select listing Casey and
    //     can connect it (N4, char-select). --------------------------------
    let (casey_handle, mut casey) =
        password_into_char(addr, &config, "duo", GM_PASSWORD, "Casey").await;
    casey.expect("Connected as Casey").await;
    casey.expect("Limbo(#0)").await;

    // --- a FRESH `ssh -i key duo+Casey@game` direct-enters Casey (N4,
    //     `<account>+<name>` login) — the account link is live. ------------
    let mut hk = russh::client::connect(Arc::clone(&config), addr, AcceptAll)
        .await
        .expect("connect");
    assert!(
        hk.authenticate_publickey("duo+Casey", signing_key(TEST_PRIV))
            .await
            .expect("auth")
            .success(),
        "the newly created character must be reachable via <account>+<name> pubkey login"
    );
    let mut tk = shell_term(&hk).await;
    tk.expect("Connected as Casey").await;
    tk.expect("Limbo(#0)").await;

    for h in [nick_handle, casey_handle, hk] {
        let _ = h.disconnect(russh::Disconnect::ByApplication, "", "").await;
    }
    let _ = std::fs::remove_dir_all(&dir);
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
    /// Byte offset of "now" in the raw stream, for [`Term::expect_after`]
    /// when the needle may already occur earlier in the scrollback.
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

    async fn send_line(&mut self, line: &str) {
        let bytes = format!("{line}\r").into_bytes();
        self.channel.data(&bytes[..]).await.expect("send line");
    }

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

/// Parse a rendered WHO duration token (`0s`, `12m`, `3h`, `2d`) to seconds.
fn duration_secs(tok: &str) -> Option<u64> {
    let (num, unit) = tok.split_at(tok.len().checked_sub(1)?);
    let n: u64 = num.parse().ok()?;
    match unit {
        "s" => Some(n),
        "m" => Some(n * 60),
        "h" => Some(n * 3600),
        "d" => Some(n * 86_400),
        _ => None,
    }
}

/// Find `name`'s WHO row in the rendered block and return (idle, conn) in
/// seconds.
fn who_row(block: &str, name: &str) -> (u64, u64) {
    for line in block.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.first() == Some(&name) {
            let idle = duration_secs(toks.get(1).copied().unwrap_or(""))
                .unwrap_or_else(|| panic!("bad idle column in WHO row {line:?}"));
            let conn = duration_secs(toks.get(2).copied().unwrap_or(""))
                .unwrap_or_else(|| panic!("bad conn column in WHO row {line:?}"));
            return (idle, conn);
        }
    }
    panic!("no WHO row for {name:?} in:\n{block}");
}

#[tokio::test(flavor = "multi_thread")]
async fn m1_exit_criterion_full_journey() {
    let dir = scratch_dir("journey");
    let addr = boot_server(&dir).await;

    // --- God connects through the same front door as everyone else. ------
    let (god_handle, mut god) = play_client(addr).await;
    god.expect("Welcome to the e2e MUSH!").await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God (account boss, 1 connection).")
        .await;
    god.expect("Limbo(#0)").await;

    // --- Alice and Bob create accounts and land in the shared room. ------
    let (alice_handle, mut alice) = play_client(addr).await;
    alice.expect("connect <account> <password>").await;
    alice.send_line("create Alice secret123").await;
    alice.expect("Account created.").await;
    alice
        .expect("Connected as Alice (account Alice, 1 connection).")
        .await;
    alice.expect("Limbo(#0)").await;
    god.expect("Alice has connected.").await;

    let (_bob_handle, mut bob) = play_client(addr).await;
    bob.send_line("create Bob hunter42").await;
    bob.expect("Connected as Bob (account Bob, 1 connection).")
        .await;
    bob.expect("Limbo(#0)").await;
    alice.expect("Bob has connected.").await;

    // --- say fans out room-wide (speaker included, third person). --------
    alice.send_line("say Hello from the other side").await;
    bob.expect("Alice says, \"Hello from the other side\"")
        .await;
    alice
        .expect("Alice says, \"Hello from the other side\"")
        .await;

    // --- @slowtest: the typed async re-entry seam, over real SSH. --------
    // The completion lands ~3s later; a think sent right behind it must
    // come back FIRST — the world loop never blocked on the fake call.
    god.send_line("@slowtest").await;
    god.expect("Thinking...").await;
    god.send_line("think interleaved-42").await;
    god.expect("interleaved-42").await;
    god.expect("SLOWTEST: fake async call").await;
    let stripped = strip_ansi(&god.seen);
    let think_at = stripped.find("interleaved-42").expect("think output");
    let slow_at = stripped.find("SLOWTEST:").expect("slowtest output");
    assert!(
        think_at < slow_at,
        "the interleaved think must land before the slowtest completion"
    );

    // --- God digs a room with exits both ways; Bob walks them. -----------
    god.send_line("@dig Vault = east;e, west;w").await;
    god.expect("Vault created with room number").await;
    god.expect("Opened exit east(").await;
    god.expect("Opened exit west(").await;

    bob.send_line("east").await;
    bob.expect("Vault(#").await;
    alice.expect("Bob goes east.").await;
    let bob_back = bob.mark();
    bob.send_line("west").await;
    bob.expect_after(bob_back, "Limbo(#0)").await;
    alice.expect("Bob has arrived.").await;

    // --- WHO: all three, with real idle/conn columns. --------------------
    let who_mark = bob.mark();
    bob.send_line("WHO").await;
    bob.expect_after(who_mark, "3 players connected.").await;
    let block = strip_ansi(&bob.seen[who_mark..]);
    let (_alice_idle, alice_conn) = who_row(&block, "Alice");
    let (bob_idle, _bob_conn) = who_row(&block, "Bob");
    who_row(&block, "God");
    assert!(
        alice_conn >= 3,
        "Alice connected before the 3s slowtest; her conn time must show it (got {alice_conn}s)"
    );
    assert!(bob_idle <= 60, "Bob just typed; idle must be fresh");

    // --- Alice QUITs cleanly. ---------------------------------------------
    alice.send_line("QUIT").await;
    alice.expect("Goodbye.").await;
    alice.expect_close().await;
    bob.expect("Alice has disconnected.").await;
    let _ = alice_handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;

    // --- Bob drops his link without QUIT: link-dead, then reconnect. ------
    let god_linkdead = god.mark();
    let _ = _bob_handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
    drop(bob);
    drop(_bob_handle);
    god.expect_after(god_linkdead, "Bob has gone link-dead.")
        .await;

    // Reconnect within the grace: announced back, session alive again.
    let god_rejoin = god.mark();
    let (bob2_handle, mut bob2) = play_client(addr).await;
    bob2.send_line("connect Bob hunter42").await;
    bob2.expect("Connected as Bob (account Bob, 1 connection).")
        .await;
    bob2.expect("Limbo(#0)").await;
    god.expect_after(god_rejoin, "Bob has reconnected.").await;

    // The revived session still routes both ways.
    bob2.send_line("say back online").await;
    god.expect_after(god_rejoin, "Bob says, \"back online\"")
        .await;

    let _ = god_handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
    let _ = bob2_handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
    let _ = std::fs::remove_dir_all(&dir);
}
