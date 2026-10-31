//! **The TUI cockpit, through the split** (16b cut B.3, `T-43`; doc 16 §5, §3.1's
//! *the output path differs*).
//!
//! `crates/mushd/tests/sim_lenses/observer_rule.rs` drives `@tui` onto screen 0 in the
//! all-in-one binary. Production is the split: the SSH front door, the session
//! layer and the TUI live on the *gateway*, while the arena, the cockpit verbs and
//! the two feeds the cockpit screen reads live on the *server*. So the one seam
//! that exists only here is the query path — the TUI's entry asks for
//! `arena.cockpit`, the frame it is answered opens screen 0, and that screen's own
//! `arena.map` ask is PULLED across the IPC boundary through the gateway's `query`
//! path (nothing ever pushes the ground). A dark map on this test is that seam
//! broken, and it is what this file exists for.
//!
//! The rig is `arena_story_split.rs`' (one store for both halves, the accounts and
//! the characters seeded before either boots; `crates/mush-gateway/tests` has no
//! shared support module, so every file here carries its own), and the tanks are
//! re-posed four cells apart as the story twin does, so the match is under way at
//! once. What the pilot's screen may say is the observer rule's, and the leak scan
//! is `observer_rule.rs`' own two helpers.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_core::{GOD, LIMBO, ObjId, ObjKind};
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore, WriteBatch};
use russh::ChannelMsg;
use tokio::sync::oneshot;

use mush_config::accounts::hash_password;
use mush_config::config::Config;
use mush_gateway::GatewayOptions;

use super::harness::ServerUnit;

const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const GM_PASSWORD: &str = "gm-secret-1";
const PILOT_PASSWORD: &str = "pilot-secret-1";

/// The world's transient authoritative refusal: a commit is in flight.
const COMMIT_PENDING: &str = "please retry in a moment";
/// `BOARD`'s one transient: the control-lease mirror is a 1 Hz projection.
const CONTROL_UNKNOWN: &str = "control state unknown";
/// How many times a line is re-sent on a transient before the test gives up.
const RETRY_BUDGET: usize = 40;

/// The gateway's answer while the server half is not admitting connections yet
/// (see `arena_story_split.rs`' `WORLD_NOT_UP`: a rig fix, not a product change).
const WORLD_NOT_UP: &str = "Connection failed: the game is shutting down";
/// How long the first login waits for the server half, in 200 ms steps.
const ADMISSION_TRIES: usize = 25;

/// The two pilots, seeded into the shared store's world before either half boots.
const ADA: ObjId = ObjId(2);
const BO: ObjId = ObjId(3);

/// Slot 1 of each side, as the shipped arena pack names them.
const RED_TANK: &str = "Red Tank 1";
const BLUE_TANK: &str = "Blue Tank 1";
/// The arena's muster room, as the shipped pack labels it.
const MUSTER: &str = "The Proving Ground";

/// Where staff puts the two tanks: four cells apart on the east-west road, nose
/// to nose — the story twin's poses.
const RED_CELL: (i64, i64, u32) = (90, 97, 90_000);
const BLUE_CELL: (i64, i64, u32) = (94, 97, 270_000);

// ---------------------------------------------------------------------------
// The split rig — `arena_story_split.rs`', one store for both halves. B.4 may
// reuse it for its attach test.
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("mush-tui-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    dir
}

/// A fast arena on both halves, set through `[tacsim]` exactly as the story twin
/// sets it.
fn test_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg.tacsim.enabled = true;
    cfg.tacsim.arena.tempo.quiet_ms = 100;
    cfg.tacsim.arena.tempo.alert_ms = 100;
    cfg.tacsim.arena.tempo.engaged_ms = 100;
    // 16b cut I.3: the validation refuses a substep longer than the engaged
    // period, as the story twin's comment says.
    cfg.tacsim.arena.tempo.substep_max_ms = 100;
    cfg.tacsim.arena.ledger.summary_ms = 1_000;
    cfg.tacsim.match_.startup_ms = 300;
    cfg.tacsim.match_.board_timeout_ms = 120_000;
    cfg
}

/// The one store both halves are pointed at, seeded with the staff account, the
/// two pilots and the world their characters live in.
async fn shared_store() -> Arc<dyn ObjectStore> {
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
    let mut world = mush_server::persist::load_or_seed_world(&*store)
        .await
        .expect("boot world");
    for (id, name) in [(ADA, "Ada"), (BO, "Bo")] {
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
    let dirty = world.take_dirty();
    let batch: WriteBatch = mush_server::persist::batch_from_dirty(&world, dirty, 0, true);
    store.write(batch).await.expect("the character seed batch");
    store
}

/// The server unit over `store` (`harness.rs`): a real server on its own
/// thread + runtime, which the gateway dials. Nothing here restarts it; the
/// unit handle may be dropped (the server runs on until the process ends).
fn unit(store: Arc<dyn ObjectStore>) -> ServerUnit {
    ServerUnit::new(store, || test_config("server"))
}

async fn start_split(store: Arc<dyn ObjectStore>) -> SocketAddr {
    let unit = unit(Arc::clone(&store));
    unit.start().await;
    let (ssh_tx, ssh_rx) = oneshot::channel();
    let gw_store = Arc::clone(&store);
    let mut cfg = test_config("gateway");
    unit.aim(&mut cfg);
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        let _ = rt.block_on(mush_gateway::serve(
            cfg,
            GatewayOptions {
                store: Some(gw_store),
                ssh_ready: Some(ssh_tx),
                web_ready: None,
                siteban_refresh: None,
                give_up_after: None,
                generation: None,
            },
        ));
    });
    tokio::time::timeout(Duration::from_secs(60), ssh_rx)
        .await
        .expect("gateway bound SSH")
        .expect("ssh addr")
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

struct Term {
    channel: russh::Channel<russh::client::Msg>,
    seen: String,
}

impl Term {
    fn mark(&self) -> usize {
        self.seen.len()
    }

    async fn expect_after(&mut self, mark: usize, needle: &str) {
        self.expect_one_of_after(mark, &[needle]).await;
    }

    async fn expect(&mut self, needle: &str) {
        self.expect_after(0, needle).await;
    }

    async fn send_line(&mut self, line: &str) {
        let bytes = format!("{line}\r").into_bytes();
        self.channel.data(&bytes[..]).await.expect("send line");
    }

    async fn expect_one_of_after(&mut self, mark: usize, needles: &[&str]) -> usize {
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        loop {
            let seen = strip_ansi(&self.seen[mark..]);
            if let Some(index) = needles.iter().position(|needle| seen.contains(needle)) {
                return index;
            }
            let msg = tokio::time::timeout_at(deadline, self.channel.wait())
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "timeout waiting for any of {needles:?}; seen: {:?}",
                        self.seen
                    )
                });
            match msg {
                Some(ChannelMsg::Data { data }) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Some(_) => {}
                None => panic!(
                    "channel closed waiting for any of {needles:?}; seen: {:?}",
                    self.seen
                ),
            }
        }
    }

    /// Run one line and wait for `needle`, retrying the two transients. The needle
    /// must not appear in the line's own echo.
    async fn say(&mut self, line: &str, needle: &str) {
        assert!(
            !line.contains(needle),
            "`{needle}` appears in `{line}`'s own echo; pick a needle from the ANSWER"
        );
        for _ in 0..RETRY_BUDGET {
            let mark = self.mark();
            self.send_line(line).await;
            match self
                .expect_one_of_after(mark, &[needle, COMMIT_PENDING, CONTROL_UNKNOWN])
                .await
            {
                0 => return,
                _ => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
        panic!("`{line}` was refused as transient {RETRY_BUDGET} times running");
    }

    async fn ask(&mut self, line: &str, needle: &str) -> String {
        let mark = self.mark();
        self.say(line, needle).await;
        strip_ansi(&self.seen[mark..])
    }
}

/// `ssh play@addr` with a PTY of `cols` by `rows` (16b cut AB.1: a client that
/// opens `@tui` asks for at least the 120-column floor, or it reads one
/// sentence instead of a screen; [`login`] is the line client's 80 by 24).
async fn play_client_sized(
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

async fn login(
    addr: SocketAddr,
    account: &str,
    password: &str,
    character: &str,
) -> (russh::client::Handle<AcceptAll>, Term) {
    login_sized(addr, account, password, character, 80, 24).await
}

/// [`login`] on a PTY of `cols` by `rows`.
async fn login_sized(
    addr: SocketAddr,
    account: &str,
    password: &str,
    character: &str,
    cols: u32,
    rows: u32,
) -> (russh::client::Handle<AcceptAll>, Term) {
    let (handle, mut term) = play_client_sized(addr, cols, rows).await;
    let connected = format!("Connected as {character}");
    for _ in 0..ADMISSION_TRIES {
        let mark = term.mark();
        term.send_line(&format!("connect {account} {password}"))
            .await;
        if term
            .expect_one_of_after(mark, &[&connected, WORLD_NOT_UP])
            .await
            == 0
        {
            return (handle, term);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("the server half never admitted `{account}`");
}

/// The dbref `@find` printed beside `name`, as `<name>(#N)`.
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

// ---------------------------------------------------------------------------
// The TUI witness — `observer_rule.rs`' own helpers
// ---------------------------------------------------------------------------

/// TUI text with every space taken out: the TUI paints cells, so it writes each
/// word at its own cursor position and the spaces between them are never sent.
fn squeezed(raw: &str) -> String {
    strip_ansi(raw)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// Everything the TUI must never say about a unit an arena holds.
fn tui_names_nothing(seen: &str, red: u32, blue: u32) {
    for leak in [
        "RedTank1".to_string(),
        "BlueTank1".to_string(),
        format!("#{red}"),
        format!("#{blue}"),
    ] {
        assert!(!seen.contains(&leak), "the TUI leaked {leak:?}:\n{seen}");
    }
}

/// **Nothing a session is sent carries a sixteen-digit contact id** (16b cut
/// Q.1). Read with its spaces still in it: the scraper splits on every
/// non-hex-digit character, and a squeezed screen puts words hard together.
fn no_sixteen_digit_word(seen: &str, what: &str) {
    for word in seen.split(|c: char| !c.is_ascii_hexdigit()) {
        assert!(
            word.len() != 16 || u64::from_str_radix(word, 16).is_err(),
            "{what} carried a sixteen-digit contact id ({word}):\n{seen}"
        );
    }
}

// ---------------------------------------------------------------------------
// The lens
// ---------------------------------------------------------------------------

/// **Screen 0 over the production split.** A pilot powers up in line mode, opens
/// `@tui`, and the cockpit screen opens itself on her own status — then the map
/// pane composes the ground that was PULLED across the IPC boundary. Nothing on
/// the screen names a tank, and `@notui` hands her back a working line client.
///
/// Ada's terminal is 120 by 34, the floor (16b cut AB.1, 16c `Q-AB1`): the
/// floor is the size this test proves the production TUI paints the cockpit
/// at, across the split. Narrower, the screen would be one sentence. God and
/// Bo stay in line mode at 80 by 24.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_split_gateway_paints_the_cockpit_on_screen_zero() {
    let store = shared_store().await;
    let ssh = start_split(store).await;

    // ---- staff re-poses the two slot-1 tanks, as the story twin does -------
    let (_g, mut god) = login(ssh, "boss", GM_PASSWORD, "God").await;
    let listing = god.ask(&format!("@find {RED_TANK}"), "END OF LIST").await;
    let red = dbref_of(&listing, RED_TANK);
    let listing = god.ask(&format!("@find {BLUE_TANK}"), "END OF LIST").await;
    let blue = dbref_of(&listing, BLUE_TANK);
    let listing = god.ask("@domain", &format!("host {MUSTER}")).await;
    let domain = domain_of(&listing, MUSTER);
    for (unit, (x, y, heading)) in [(red, RED_CELL), (blue, BLUE_CELL)] {
        let pose = format!(
            "{{\"x_cm\":{},\"y_cm\":{},\"heading_mdeg\":{heading}}}",
            x * 1_000 + 500,
            y * 1_000 + 500
        );
        god.say(
            &format!("@spatial/place/domain #{unit}={domain},{pose}"),
            "Placed",
        )
        .await;
    }

    // ---- two pilots, a team each, the match, the seats ----------------------
    let (_a, mut ada) = login_sized(ssh, "ada", PILOT_PASSWORD, "Ada", 120, 34).await;
    let (_b, mut bo) = login(ssh, "bo", PILOT_PASSWORD, "Bo").await;
    for term in [&mut ada, &mut bo] {
        term.say("ooc", "Welcome Hall").await;
        term.say("simulator", "Simulator Lobby").await;
    }
    ada.say("SIM TEAM red", "takes red slot 1").await;
    bo.say("SIM TEAM blue", "takes blue slot 1").await;
    ada.say("SIM START", "is boarding").await;
    ada.say("red", "Red Room").await;
    bo.say("blue", "Blue Room").await;
    ada.say("BOARD", "climb into").await;
    bo.say("BOARD", "climb into").await;
    ada.expect("Good hunting").await;

    // ---- her power comes up in LINE mode ------------------------------------
    let powering = ada.mark();
    ada.say("STARTUP", "Startup sequence").await;
    ada.expect_after(powering, "Power online.").await;

    // ---- `@tui`: the cockpit screen, across the boundary --------------------
    let opened = ada.mark();
    ada.send_line("@tui").await;
    ada.expect_after(opened, "rustmush").await;
    // `Callsign` is the status pane's first row, and it paints on the cockpit
    // screen alone — which the frame the entry query was answered opens.
    ada.expect_after(opened, "Callsign").await;
    // **The pulled ground.** `arena.map` is answered only to a query the TUI
    // sends itself, through the gateway's `query` path to the server half; the
    // heading line replaces the pane's `Waiting for the arena map...`, and
    // nothing else on this screen prints a capital `Map`. It arrives after the
    // frame, so the whole status pane has been read by then.
    ada.expect_after(opened, "Map").await;
    let screen = squeezed(&ada.seen[opened..]);
    // Doc 32 cut CF.0 (Q-CF-11): red's letter, then a drawn unit letter.
    assert!(
        screen
            .split("CallsignR")
            .nth(1)
            .and_then(|rest| rest.chars().next())
            .is_some_and(|c| tacsim::arena::PARTY_LETTERS.contains(&(c as u8))),
        "her own tank's callsign crossed the boundary:\n{screen}"
    );
    assert!(
        screen.contains("Nolock"),
        "the status pane painted whole:\n{screen}"
    );
    assert!(
        screen.contains("Map\u{2014}heading") || screen.contains("Map-heading"),
        "the map pane composed the pulled ground:\n{screen}"
    );
    // The observer rule over everything the full screen painted.
    tui_names_nothing(&screen, red, blue);
    no_sixteen_digit_word(&strip_ansi(&ada.seen[opened..]), "the split TUI cockpit");

    // ---- `@notui`: a working line client again ------------------------------
    ada.send_line("").await; // the box, empty: the focused map pane has no row
    ada.send_line("@notui").await;
    ada.say("STATUS", "Callsign R").await;

    drop((god, ada, bo));
}
