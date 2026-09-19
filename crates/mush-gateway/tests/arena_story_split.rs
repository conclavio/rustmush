//! **A1, through the split** (16b cut W.3, W4; doc 16 §1 "A1", §3.1's *the output
//! path differs*).
//!
//! `crates/mushd/tests/arena_story.rs` is the in-process half of doc 16 §1's story
//! test. This is its **split** twin, and doc 16 §1 asks for it in so many words:
//!
//! > One test in `mushd/tests` (in-process) and its twin in `mush-server` through
//! > the gateway boundary; both must pass, because the output path differs (§3.1).
//!
//! Everything the story needs lives on the *server* half of a two-process deploy —
//! the arena task, the match book, the cockpit verbs, the ledger — while the SSH
//! front door, the session layer and the accounts live on the *gateway*. So every
//! line below is a command the gateway forwards and an answer it carries back, and
//! the two lines that matter most are the ones **nobody asked for**: the centre's
//! "Match 1 is running" and its "Match 1 is over (red). Its units are checked in.",
//! pushed back across the boundary by a task on the far side.
//!
//! # What this twin asserts, and what stays the `mushd` twin's
//!
//! It drives the **production composition** (`mush_server::serve_with_store`,
//! exactly as the deployed binary does) and can see only what the gateway can see:
//! two pilots and a bystander at three terminals. The ledger's event order, the
//! result document, the ammunition law and the money rule are claims about the
//! **store**, and they stay with the in-process twin, which can read it.
//!
//! # One store for both halves
//!
//! A real split points both processes at one Redis (the accounts the gateway
//! authenticates against are the accounts `mush_server::accounts_identity::hydrate`
//! builds the character → account-id oracle from), and `SIM TEAM` fails closed on a
//! character the oracle does not know. So this rig hands **one** `InMemoryStore` to
//! both halves and seeds it before either boots: the three accounts, and the world
//! their characters live in, written through `mush_server::persist`'s own boot-batch
//! path. That oracle is built **once, at boot** and never updated, which is why the
//! pilots are seeded rather than created at the welcome screen — the same reason the
//! in-process twin gives.
//!
//! # Why the two tanks start forty metres apart
//!
//! The approach — 2.4 km of authored muster, the sensor's 600 m, the contact that
//! appears only once the range closes — is the in-process twin's, and
//! `mushd/tests/observer_rule.rs` owns it twice over. What is under test *here* is
//! the **boundary**, so staff re-poses the two slot-1 tanks four cells apart, nose
//! to nose, and the fight starts at once.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use mush_core::{GOD, LIMBO, ObjId, ObjKind};
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore, WriteBatch};
use russh::ChannelMsg;
use tokio::sync::oneshot;

use mush_config::accounts::hash_password;
use mush_config::config::Config;
use mush_gateway::{GatewayOptions, LaunchedServer, ServerLauncher, ServerSource};

const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const GM_PASSWORD: &str = "gm-secret-1";
const PILOT_PASSWORD: &str = "pilot-secret-1";

/// The world's transient authoritative refusal: a commit is in flight.
const COMMIT_PENDING: &str = "please retry in a moment";
/// `BOARD`'s one transient: the control-lease mirror is a 1 Hz projection.
const CONTROL_UNKNOWN: &str = "control state unknown";
/// How many times a line is re-sent on a transient before the test gives up.
const RETRY_BUDGET: usize = 40;

/// The two pilots and the bystander, seeded into the shared store's world before
/// either half boots (see the module head), so their ids are known here.
const ADA: ObjId = ObjId(2);
const BO: ObjId = ObjId(3);
const CY: ObjId = ObjId(4);

/// Slot 1 of each side, as the shipped arena pack names them.
const RED_TANK: &str = "Red Tank 1";
const BLUE_TANK: &str = "Blue Tank 1";
/// The arena's muster room, as the shipped pack labels it.
const MUSTER: &str = "The Proving Ground";

/// Where staff puts the two tanks: four cells apart on the east-west road, nose
/// to nose, so the blue one presents its **front** at forty metres.
///
/// Frontal for the same reason the in-process twin is (see its module head): only
/// the hull's `destroy_unit` blows a `line_tank` up, and the front band reaches
/// only the hull and the turret — so this fight can end exactly one way, and the
/// twin tells the same story every run.
const RED_CELL: (i64, i64, u32) = (90, 97, 90_000);
const BLUE_CELL: (i64, i64, u32) = (94, 97, 270_000);

/// The cannon's authored load (`ammunition/he_105.json`).
const LOAD: i64 = 40;
/// How many shots the fight is allowed before the test calls it — a ceiling for
/// the tail of the front facet table's distribution (about ten hits through 400
/// cm of glacis and 200 of hull internals) and not a budget the ordinary run
/// comes near. See the in-process twin for the arithmetic.
const MAX_SHOTS: usize = 30;

/// The line every witness in the centre reads when the match is settled.
const OVER: &str = "is over (red)";
/// The parser's refusal, which is the sentinel's answer.
const HUH: &str = "Huh?";
/// The one line the nine cockpit selectors refuse with once the check-in has
/// opened the hatches.
const OUTSIDE_COCKPIT: &str = "You are not at the station of a deployed unit.";

// ---------------------------------------------------------------------------
// The split rig — `arena_custody_split.rs`', with one store for both halves
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("mush-story-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    dir
}

/// A fast arena on both halves: the same knobs the in-process twin sets, reached
/// here through `[tacsim]` rather than through a `TacsimConfig` literal, which is
/// itself worth proving — a split deploy has no other way to set them.
fn test_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg.ipc.exit_on_abandoned_supervision = false;
    cfg.tacsim.enabled = true;
    cfg.tacsim.arena.tempo.quiet_ms = 100;
    cfg.tacsim.arena.tempo.alert_ms = 100;
    cfg.tacsim.arena.tempo.engaged_ms = 100;
    cfg.tacsim.arena.ledger.summary_ms = 200;
    cfg.tacsim.match_.startup_ms = 300;
    cfg.tacsim.match_.board_timeout_ms = 120_000;
    cfg
}

/// The one store both halves are pointed at, seeded with the staff account, the
/// two pilots, the bystander, and the world their characters live in.
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

    // `load_or_seed_world` writes Limbo #0 and God #1 through on an empty store;
    // the three characters ride the same boot serial, so the server's own boot
    // finds a world that already has them.
    let mut world = mush_server::persist::load_or_seed_world(&*store)
        .await
        .expect("boot world");
    for (id, name) in [(ADA, "Ada"), (BO, "Bo"), (CY, "Cy")] {
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

async fn launch_thread_server(store: Arc<dyn ObjectStore>) -> Result<LaunchedServer, String> {
    let cfg = test_config("server");
    let (ipc_tx, ipc_rx) = oneshot::channel();
    let (exit_tx, exit_rx) = oneshot::channel();
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        let _ = rt.block_on(mush_server::serve_with_store(
            cfg,
            Some(ipc_tx),
            Some(store),
        ));
        let _ = exit_tx.send(());
    });
    let endpoint = tokio::time::timeout(Duration::from_secs(60), ipc_rx)
        .await
        .map_err(|_| "server ipc bind timed out".to_string())?
        .map_err(|_| "server dropped its ipc oneshot".to_string())?;
    Ok(LaunchedServer {
        endpoint,
        exited: Box::pin(async move {
            let _ = exit_rx.await;
        }),
    })
}

struct OneServer {
    store: Arc<dyn ObjectStore>,
}

impl ServerLauncher for OneServer {
    fn launch(&self) -> BoxFuture<'static, Result<LaunchedServer, String>> {
        Box::pin(launch_thread_server(Arc::clone(&self.store)))
    }
}

async fn start_split(store: Arc<dyn ObjectStore>) -> SocketAddr {
    let (ssh_tx, ssh_rx) = oneshot::channel();
    let gw_store = Arc::clone(&store);
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        let _ = rt.block_on(mush_gateway::serve(
            test_config("gateway"),
            GatewayOptions {
                source: ServerSource::Restartable(Box::new(OneServer { store })),
                store: Some(gw_store),
                ssh_ready: Some(ssh_tx),
                web_ready: None,
                restart_ready: None,
                siteban_refresh: None,
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

    /// **A whole answer, never half of one** — the in-process twin's device: a
    /// nonsense line after the real one, and the slice runs to *its* refusal, so a
    /// multi-line render is in the slice entire and nothing of it is still in
    /// flight.
    async fn ask_whole(&mut self, line: &str) -> String {
        for _ in 0..RETRY_BUDGET {
            let mark = self.mark();
            self.send_line(line).await;
            self.send_line(SENTINEL).await;
            // The sentinel is a parser refusal and needs no authority, but it rides
            // the same lane: a world mid-commit answers BOTH lines with the
            // transient and the `Huh?` never comes. That is a retry, not a failure.
            if self.expect_one_of_after(mark, &[HUH, COMMIT_PENDING]).await == 0 {
                return strip_ansi(&self.seen[mark..]);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        panic!("`{line}` never produced a whole answer")
    }
}

const SENTINEL: &str = "zzq-split-story-sentinel";

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

async fn login(
    addr: SocketAddr,
    account: &str,
    password: &str,
    character: &str,
) -> (russh::client::Handle<AcceptAll>, Term) {
    let (handle, mut term) = play_client(addr).await;
    term.send_line(&format!("connect {account} {password}"))
        .await;
    term.expect(&format!("Connected as {character}")).await;
    (handle, term)
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

/// The opaque contact id in a `CONTACTS` listing.
fn contact_id_of(listing: &str) -> Option<String> {
    listing
        .split(|c: char| !c.is_ascii_hexdigit())
        .find(|word| word.len() == 16 && u64::from_str_radix(word, 16).is_ok())
        .map(str::to_owned)
}

/// The cannon's rounds, off a `WEAPONSTATUS` listing.
fn cannon_rounds(listing: &str) -> Option<i64> {
    let needle = format!(" of {LOAD} round(s)");
    listing
        .lines()
        .rfind(|line| line.contains(&needle))
        .and_then(|line| {
            line.split(&needle)
                .next()?
                .split_whitespace()
                .next_back()?
                .parse::<i64>()
                .ok()
        })
}

/// Everything a surface outside the two cockpits must never say.
fn names_nothing(seen: &str, red: u32, blue: u32, what: &str) {
    for leak in [
        RED_TANK.to_string(),
        BLUE_TANK.to_string(),
        format!("#{red}"),
        format!("#{blue}"),
    ] {
        assert!(!seen.contains(&leak), "{what} leaked {leak:?}:\n{seen}");
    }
}

// ---------------------------------------------------------------------------
// The lens
// ---------------------------------------------------------------------------

/// **The whole story, across the IPC boundary.** See the module head for what this
/// twin can assert and what stays the in-process one's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_story_is_the_same_story_through_the_gateway() {
    let store = shared_store().await;
    let ssh = start_split(store).await;

    // ---- staff musters the two tanks, across the boundary --------------
    let (_g, mut god) = login(ssh, "boss", GM_PASSWORD, "God").await;
    god.say("@arena/status", "No match is running.").await;
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

    // ---- two pilots and a bystander walk in ----------------------------
    let (_a, mut ada) = login(ssh, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(ssh, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login(ssh, "cy", PILOT_PASSWORD, "Cy").await;
    for term in [&mut ada, &mut bo, &mut cy] {
        term.say("ooc", "Welcome Hall").await;
        // `SIM` is a verb before it is an exit, so the walk takes the exit's other
        // name. The in-process twin says the same thing about the same content.
        term.say("simulator", "Simulator Lobby").await;
    }
    let bystander_mark = cy.mark();

    // ---- selection, boarding, and the trigger --------------------------
    ada.say("SIM TEAM red", "takes red slot 1").await;
    bo.say("SIM TEAM blue", "takes blue slot 1").await;
    cy.expect_after(bystander_mark, "Ada takes red slot 1")
        .await;
    ada.say("SIM START", "is boarding").await;
    ada.say("red", "Red Room").await;
    bo.say("blue", "Blue Room").await;
    ada.say("BOARD", "climb into").await;
    bo.say("BOARD", "climb into").await;
    // The unsolicited line, pushed from the far side of the boundary.
    ada.expect("Good hunting").await;
    cy.expect_after(bystander_mark, "Good hunting").await;
    god.say("@arena/status", "running, tempo").await;

    // ---- the cockpit, across the boundary ------------------------------
    for term in [&mut ada, &mut bo] {
        term.say("STARTUP", "Startup sequence").await;
        for _ in 0..RETRY_BUDGET {
            if term.ask_whole("STATUS").await.contains("power online") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
    let mut held = String::new();
    for _ in 0..RETRY_BUDGET {
        let seen = ada.ask_whole("CONTACTS").await;
        if let Some(id) = contact_id_of(&seen) {
            held = id;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        !held.is_empty(),
        "the two tanks never detected each other across the boundary"
    );
    ada.say(&format!("LOCK {held}"), "Locking").await;
    let locked = {
        let mut seen = String::new();
        for _ in 0..RETRY_BUDGET {
            seen = ada.ask_whole("WEAPONSTATUS").await;
            if seen.contains("Locked on") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        seen
    };
    assert!(
        locked.contains(&held) && locked.contains("% to hit"),
        "the lock and its percentage crossed the boundary: {locked}"
    );

    // ---- the fight -----------------------------------------------------
    let fight_mark = ada.mark();
    let mut shots: i64 = 0;
    let mut readings: Vec<(i64, i64)> = Vec::new();
    for _ in 0..MAX_SHOTS {
        let mut fired = false;
        for _ in 0..RETRY_BUDGET {
            let mark = ada.mark();
            ada.send_line("FIRE").await;
            match ada
                .expect_one_of_after(
                    mark,
                    &[
                        "Firing cannon",
                        "Still cycling",
                        COMMIT_PENDING,
                        CONTROL_UNKNOWN,
                        OUTSIDE_COCKPIT,
                        OVER,
                    ],
                )
                .await
            {
                0 => {
                    fired = true;
                    break;
                }
                // The cycle, and the world's two transients: all three are a wait
                // and none of them is an answer about the shot.
                1..=3 => tokio::time::sleep(Duration::from_millis(500)).await,
                _ => break,
            }
        }
        if !fired {
            break;
        }
        shots += 1;
        // See the in-process twin: a `FIRE` accepted by the cockpit against a frame
        // one pulse behind and refused by the arena against its own `ready_at_ms` is
        // a round that went nowhere, and the tally is put back rather than counted.
        let want = LOAD - shots;
        let mut cockpit = true;
        let mut confirmed = false;
        for _ in 0..16 {
            let seen = ada.ask_whole("WEAPONSTATUS").await;
            match cannon_rounds(&seen) {
                Some(rounds) if rounds == want => {
                    readings.push((shots, rounds));
                    confirmed = true;
                    break;
                }
                Some(_) => {}
                None => {
                    assert!(
                        seen.contains(OUTSIDE_COCKPIT),
                        "a weapon table with no cannon row and no refusal:\n{seen}"
                    );
                    cockpit = false;
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        if !cockpit || strip_ansi(&ada.seen[fight_mark..]).contains(OVER) {
            break;
        }
        if !confirmed {
            shots -= 1;
        }
    }
    assert!(shots > 0, "the red tank never got a shot away");
    assert!(
        strip_ansi(&ada.seen[fight_mark..]).contains(OVER)
            || strip_ansi(&ada.seen[fight_mark..]).contains(OUTSIDE_COCKPIT),
        "{shots} rounds went into the blue tank and it is still fighting"
    );
    assert!(
        !readings.is_empty(),
        "the cockpit never showed a round count after a shot"
    );
    for (fired, rounds) in &readings {
        assert_eq!(
            fired + rounds,
            LOAD,
            "after {fired} shot(s) the cockpit showed {rounds} of {LOAD} rounds"
        );
    }

    // ---- the end, on every surface the split carries -------------------
    ada.expect_after(fight_mark, OVER).await;
    ada.expect_after(fight_mark, "Its units are checked in.")
        .await;
    ada.expect_after(fight_mark, "The hatch of Red Pod 1 swings open.")
        .await;
    bo.expect(OVER).await;
    bo.expect("The hatch of Blue Pod 1 swings open.").await;
    cy.expect_after(bystander_mark, OVER).await;

    assert!(
        ada.ask_whole("look").await.contains("Red Room"),
        "the winner walked out"
    );
    assert!(
        bo.ask_whole("look").await.contains("Blue Room"),
        "the loser walked out"
    );
    bo.say("WEAPONSTATUS", OUTSIDE_COCKPIT).await;

    // **16b cut F.2**: the pilot notices cross the boundary too, and split the
    // same way they do in process — the shooter is told the round landed, the
    // victim is told where it landed and nothing about who fired.
    let winner = strip_ansi(&ada.seen);
    assert!(
        winner.contains(&format!("Hit on {held}.")),
        "the shooter's outcome line never crossed the boundary:\n{winner}"
    );
    let loser = strip_ansi(&bo.seen);
    assert!(
        loser.contains("Hit on the "),
        "the victim's damage line never crossed the boundary:\n{loser}"
    );
    for line in loser
        .lines()
        .filter(|line| line.starts_with("Hit on the ") || line.starts_with("The "))
    {
        for leak in [held.as_str(), "red", "bearing", "cannon", "°"] {
            assert!(
                !line.contains(leak),
                "a hit notice carried {leak:?} about the shooter: {line}"
            );
        }
    }

    // The bystander stood in the lobby for the whole match: the announcements
    // crossed and nothing of the arena did.
    let bystander = strip_ansi(&cy.seen[bystander_mark..]);
    names_nothing(&bystander, red, blue, "the bystander's whole transcript");
    for leak in [
        held.as_str(),
        "Firing",
        "Locked on",
        "round(s)",
        "Hit on",
        "Miss on",
        "is destroyed",
        "armour",
    ] {
        assert!(
            !bystander.contains(leak),
            "the bystander was shown {leak:?} through the boundary:\n{bystander}"
        );
    }
    assert!(
        bystander.contains("Match 1 is running") && bystander.contains(OVER),
        "but the centre's own announcements did cross:\n{bystander}"
    );

    // ---- and the centre is idle again ----------------------------------
    //
    // Through `@arena/status` rather than `SIM STATUS`, because the staffer never
    // left Limbo and the lobby verbs are gated on the room (16b cut K.1) — which
    // is itself a refusal that had to cross the boundary to be read here.
    let status = god.ask("@arena/status", "Simulator centre: ended").await;
    assert!(
        status.contains("red"),
        "the outcome is on the board: {status}"
    );
    god.say("@arena/reset", "centre is reset").await;
    god.say("@arena/status", "No match is running.").await;
    ada.say("lobby", "Simulator Lobby").await;
    ada.say("SIM TEAM red", "takes red slot 1").await;

    drop((god, ada, bo, cy));
}
