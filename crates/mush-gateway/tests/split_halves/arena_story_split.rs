//! **A1, through the split** (16b cut W.3, W4; doc 16 §1 "A1", §3.1's *the output
//! path differs*).
//!
//! `crates/mushd/tests/sim_lenses/arena_story.rs` is the in-process half of doc 16 §1's story
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
//! path. That is a shortcut for speed, not a requirement: since **16b cut F.4** the
//! oracle learns of a character in the world-task turn that creates it, so a
//! welcome-screen `create` through this same boundary can claim a slot at once —
//! which `crates/mush-gateway/tests/split_halves/front_door_split.rs` is there to prove, and
//! what is under test *here* is the match.
//!
//! # Why the two tanks start forty metres apart
//!
//! The approach — 2.4 km of authored muster, the sensor's 600 m, the contact that
//! appears only once the range closes — is the in-process twin's, and
//! `mushd/tests/sim_lenses/observer_rule.rs` owns it twice over. What is under test *here* is
//! the **boundary**, so staff re-poses the two slot-1 tanks four cells apart, nose
//! to nose, and the fight starts at once.

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

/// **The gateway's answer while the server half is not admitting connections
/// yet** (`ConnectErr::WorldGone`, `crates/mush-gateway/src/bridge_common.rs`).
///
/// This rig starts the two halves independently and then waits on ONE signal:
/// the gateway's `ssh_ready` oneshot, which fires when the SSH listener binds.
/// Nothing in it waits for the server half to admit a connection, and the
/// server's own oneshot says only that its IPC socket is bound. So the very
/// first `connect` can land in that window and be refused with this sentence,
/// and a rig that sent it once would then sit out its whole `STEP_TIMEOUT` and
/// fail a story it never got to tell. The first line is therefore re-sent for a
/// few seconds while this exact sentence is the answer — a rig fix, not a
/// product change; every assertion about the story itself is unchanged.
const WORLD_NOT_UP: &str = "Connection failed: the game is shutting down";
/// How long the first login waits for the server half, in 200 ms steps.
const ADMISSION_TRIES: usize = 25;

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
/// only the hull and the turret. Since **16b cut AP.2** a turret loss ends the
/// match too (a side that came in armed and has no gun left is out), so the
/// fight ends at the hull or at the turret by the seeded facet draw, and this
/// twin, like the other, asserts whichever section went.
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
/// The one line the twelve cockpit selectors refuse with once the check-in has
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
    cfg.tacsim.enabled = true;
    cfg.tacsim.arena.tempo.quiet_ms = 100;
    cfg.tacsim.arena.tempo.alert_ms = 100;
    cfg.tacsim.arena.tempo.engaged_ms = 100;
    // **16b cut I.3**: the validation refuses a substep longer than the engaged
    // period (a staff tempo floor could stop arena time under one).
    cfg.tacsim.arena.tempo.substep_max_ms = 100;
    // **16b cut Q.7**: this is the one arena test that boots through the host's
    // own `[tacsim]` mapping rather than building a `TacsimConfig` by hand, so it
    // is the one that meets the value validation — and the floor under
    // `summary_ms` is a thousand. Nothing here reads a `position_summary`; the
    // cadence was only ever "often enough to be a usable record".
    cfg.tacsim.arena.ledger.summary_ms = 1_000;
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

/// **Whether a line names a side as a word** (16b cut F.3).
///
/// The leak check forbids the substring `red` in a victim's own damage lines,
/// and `starts_with("The ")` lets in every line that opens that way — so a
/// sentence such as `The last match is cleared from the board.` would
/// false-fail on `cleared`. A boundary test forbids the side's own name and
/// nothing that merely spells it.
fn names_side(line: &str, side: &str) -> bool {
    line.match_indices(side).any(|(at, _)| {
        let before = line[..at].chars().next_back();
        let after = line[at.saturating_add(side.len())..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
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

    /// **`SIM REPORT`, read whole** (16b cut V.3) — the in-process twin's
    /// device. The report is a deferred store read on the server half, so an
    /// early read can outrun the result batch's commit and answer `still being
    /// written`; the wait asks again, and once the end line has crossed a
    /// sentinel read to its refusal brings in the rest.
    async fn sim_report(&mut self, line: &str) -> String {
        const ENDS: [&str; 3] = [
            "the match was aborted.",
            "the match ended: ",
            "the match was voided.",
        ];
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        loop {
            let mark = self.mark();
            self.send_line(line).await;
            let mut needles: Vec<&str> = ENDS.to_vec();
            needles.extend(["still being written", "already being read", COMMIT_PENDING]);
            if self.expect_one_of_after(mark, &needles).await < ENDS.len() {
                self.send_line(SENTINEL).await;
                self.expect_after(mark, HUH).await;
                return strip_ansi(&self.seen[mark..]);
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "`{line}` never read a whole report"
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
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
    let connected = format!("Connected as {character}");
    // See `WORLD_NOT_UP`: the rig has no barrier against the server half still
    // booting, so the first line waits it out rather than failing the story.
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

/// **The short designator in a `CONTACTS` listing** (16b cut Q.1): the first
/// word of a contact row, which is `X<n>` while the return is unresolved and
/// the team's letter with the target's centre slot once it has been told apart.
///
/// It replaces the sixteen-digit scraper this helper used to be. The digits are
/// not printed anywhere a pilot can see any more, and a designator is what
/// `LOCK`, `RANGE` and `BEARING` take.
fn designator_of(listing: &str) -> Option<String> {
    listing
        .lines()
        .find(|line| line.contains("bearing ") && line.contains("(rel "))
        .and_then(|line| line.split_whitespace().next())
        .map(str::to_owned)
}

/// The designator of the first contact row whose class column (the word after
/// the designator: `unknown`, then the resolved side) reads `class`.
fn designator_classed(listing: &str, class: &str) -> Option<String> {
    listing
        .lines()
        .filter(|line| line.contains("bearing ") && line.contains("(rel "))
        .find_map(|line| {
            let mut words = line.split_whitespace();
            let id = words.next()?;
            (words.next()? == class).then(|| id.to_owned())
        })
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
        if let Some(id) = designator_of(&seen) {
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

    // **16b cut Q.2**: the name the row reads once it has been told apart, which
    // is not `held` — `held` was scraped at the first detection and is the
    // provisional `X<n>` on a fast reader; a delayed reader can first observe
    // an already-classified row. Every sentence about the target's hull
    // is gated on classification, so the assertions below have to be against the
    // classified spelling or they would be about the wrong half of the match.
    // The row's own class column must read `blue`: the classification notice
    // (`Contact X1 identified: BA, blue.`) can land inside a `CONTACTS` answer
    // whose row is still the frame before it, `X1  unknown …`, so `blue`
    // anywhere in the answer named the provisional row (main's run
    // 40251385032 looked for `Hit on X1: ` against `Hit on BA: hull.`).
    let mut classified = String::new();
    for _ in 0..RETRY_BUDGET {
        let seen = ada.ask_whole("CONTACTS").await;
        if let Some(id) = designator_classed(&seen, "blue") {
            classified = id;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        !classified.is_empty(),
        "the contact never crossed the boundary as a classified row: first={held:?}, classified={classified:?}"
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
        // **16b cut Q.2**: the shooter's line names the section the round found
        // for a contact it has told apart, so the prefix is what crosses.
        winner.contains(&format!("Hit on {classified}: ")),
        "the shooter's outcome line never crossed the boundary:\n{winner}"
    );
    // **16b cut Q.2**: and so does the section the gunner watched come apart.
    // The blue tank is beaten by a hull loss or (16b cut AP.2) a turret loss —
    // the loser's own line says which, exactly one of the two, because the
    // match is over at the first — and that section is the certain one.
    let loser = strip_ansi(&bo.seen);
    let hull_went = loser.contains("The hull is destroyed: the tank is finished.");
    // 17b cut M3.6: the turret's line now ends with its core's words (was
    // "The turret is destroyed: its weapons are gone.").
    let turret_went =
        loser.contains("The turret is destroyed: its weapons are gone; its core is gone.");
    assert!(
        hull_went != turret_went,
        "exactly one of the hull and the turret went (hull {hull_went}, turret {turret_went}):\n{loser}"
    );
    eprintln!(
        "arena_story_split: the duel ended at the {}",
        if hull_went { "hull" } else { "turret" }
    );
    let section = if hull_went { "hull" } else { "turret" };
    assert!(
        winner.contains(&format!("{classified}'s {section} is destroyed.")),
        "the shooter's section-destroyed line never crossed the boundary:\n{winner}"
    );
    assert!(
        loser.contains("Hit on the "),
        "the victim's damage line never crossed the boundary:\n{loser}"
    );
    for line in loser
        .lines()
        .filter(|line| line.starts_with("Hit on the ") || line.starts_with("The "))
    {
        for leak in [held.as_str(), "bearing", "cannon", "°"] {
            assert!(
                !line.contains(leak),
                "a hit notice carried {leak:?} about the shooter: {line}"
            );
        }
        // 16b cut F.3: the side is forbidden as a **word**, so a future sentence
        // that happens to spell it (`cleared`) is not a false failure.
        assert!(
            !names_side(line, "red"),
            "a hit notice named the shooter's side: {line}"
        );
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

    // ---- the winner's debrief, through the boundary (16b cut V.3) -----
    //
    // The in-process twin reads the report document's facts against the store;
    // what this twin adds is that the deferred store read and the module's
    // completion work on the split server half, and that what crosses back is
    // the same public head and the pilot's own lines.
    let report = ada.sim_report("SIM REPORT").await;
    let flat = report.split_whitespace().collect::<Vec<_>>().join(" ");
    // 16b cut AP.2: the kill line is the hull's; a turret ending is the shot's
    // own clause instead.
    let finished = if hull_went {
        "destroyed by your round."
    } else {
        "'s turret destroyed"
    };
    for want in [
        "red won (blue eliminated)",
        "You flew RA: ",
        finished,
        "the match ended: blue eliminated.",
    ] {
        assert!(flat.contains(want), "{want:?} missing from:\n{report}");
    }
    let said = ada.ask("SIM REPORT 9", "not one of them").await;
    assert!(
        said.contains("The centre remembers its last 20 matches; match 9 is not one of them."),
        "{said}"
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
