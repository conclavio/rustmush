//! **The custody lens, through the split** (16b cut T.3; 16b §7 "Verify
//! lenses"; doc 16 §3.1 "Custody", §8 "Restart mid-match").
//!
//! `crates/mushd/tests/sim_lenses/arena_custody.rs` is the in-process half of this lens: it
//! runs a real match over real SSH and asserts the whole custody round trip. This
//! is its **split** twin, and it exists because everything cut T.3 added lives on
//! the *server* half of a two-process deploy — the arena tasks, the match book,
//! the `deployed` markers, the boot reconciliation — while the SSH front door
//! lives on the *gateway*. A verb that works in the all-in-one and not across the
//! boundary would be a verb nobody could use in production.
//!
//! # What this twin can assert, and what it cannot
//!
//! It drives the **production composition** — `mush_server::serve_with_store`,
//! exactly as the deployed binary does — so it cannot swap a module in. 16b cut
//! C.2 is what made that enough: the composition now carries doc 16 §3.3's stock
//! tank in its unit catalog and puts the arena's ground and its twelve tanks in
//! the world at a first boot, so this file runs the **whole round trip** on the
//! units the content made, beside the half that is a property of the *split*:
//!
//! * the whole `@arena` lifecycle surface is live across the boundary — the read,
//!   the refusals, and the fact that each one comes back as itself and not as a
//!   broken pipe;
//! * a server restart **while a match is running** leaves the arena surface live
//!   and — since 16b cut AF.3 — the match running too, resumed from its last
//!   snapshot: doc 16 §8's "restart mid-match" seen from the side the gateway
//!   can see, and the store's side of it (the markers kept, no result document
//!   until the match ends the ordinary way);
//! * the custody admission error is the same named sentence here as everywhere
//!   else;
//! * (16b cut AF.3) two pilots who really boarded, whose sessions survive the
//!   restart on the gateway, read that the match resumed and that their tanks
//!   fly on their minds (on doctrine until 17b cut M5.3), and take them back
//!   with `BOARD` — the split's whole point;
//! * (16b cut U.4, 16c `B-37`) a pilot in a pod when the server stops during
//!   `Boarding` — nothing checked out, nothing to resume — is let out of the pod
//!   by the boot and reads the hatch line from the rebooted world's first tick.
//!
//! What stays the `mushd` twin's is the rest of the claims about the **store**:
//! the byte-identical record around an idle unit, the driven unit's final
//! placement, the injected panic. This file sees what the gateway can see, and
//! (16b cut M.1, `T-42`; 16b cut AF.3) reads the server half's store for the
//! restart's own facts: both units' `deployed` markers and the arena's snapshot
//! written before the crash, the markers still there after it and no result
//! document, and the ordinary result once the resumed match is aborted.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use mush_core::{GOD, LIMBO, ObjId, ObjKind};
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore, WriteBatch};
use russh::ChannelMsg;
use tokio::sync::oneshot;

use mush_config::accounts::hash_password;
use mush_config::config::Config;
use mush_gateway::{GatewayOptions, LaunchedServer, RestartTrigger, ServerLauncher, ServerSource};

const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const GM_PASSWORD: &str = "gm-secret-1";
/// The world's transient authoritative refusal (`mush_server`'s
/// `RefusalReason::CommitPending`): a commit is in flight, so try again.
const COMMIT_PENDING: &str = "please retry in a moment";
/// How many times a line is re-sent on that refusal before the test gives up.
const RETRY_BUDGET: usize = 8;
/// **16b cut U.4**: the pilots' password, for the sibling that seats two of them.
const PILOT_PASSWORD: &str = "pilot-secret-1";
/// `BOARD`'s one transient: the control-lease mirror is a 1 Hz projection.
const CONTROL_UNKNOWN: &str = "control state unknown";
/// The two pilots the sibling seeds, in the order `load_or_seed_world` hands out
/// ids after Limbo #0 and God #1.
const ADA: ObjId = ObjId(2);
const BO: ObjId = ObjId(3);

/// **The gateway's answer while the server half is not admitting connections
/// yet** (`ConnectErr::WorldGone`, `crates/mush-gateway/src/bridge_common.rs`).
///
/// This rig starts the two halves independently and then waits on ONE signal:
/// the gateway's `ssh_ready` oneshot, which fires when the SSH listener binds.
/// Nothing in it waits for the server half to admit a connection. So the very
/// first `connect` can land in that window and be refused with this sentence,
/// and a rig that sent it once would then sit out its whole `STEP_TIMEOUT`. The
/// first line is therefore re-sent for a few seconds while this exact sentence
/// is the answer — a rig fix, not a product change, and nothing the lens
/// asserts about custody is weakened.
const WORLD_NOT_UP: &str = "Connection failed: the game is shutting down";
/// How long the first login waits for the server half, in 200 ms steps.
const ADMISSION_TRIES: usize = 25;

// ---------------------------------------------------------------------------
// The split rig — `restart_survival.rs`'s, because `crates/mush-gateway/tests`
// has no shared support module and every file here carries its own.
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("mush-arena-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    dir
}

fn test_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg.ipc.exit_on_abandoned_supervision = false;
    // 16b cut C.2: the layer on, which is what puts the game's own boot packs
    // in the content module — the proving ground's ground and its twelve stock
    // tanks. It is the one knob this file sets beyond the split rig's own,
    // because without it there is no arena for a match to be fought on.
    cfg.tacsim.enabled = true;
    cfg
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
    let endpoint = tokio::time::timeout(Duration::from_secs(30), ipc_rx)
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

/// A server that can be made to drop its IPC boundary on its own — the crash /
/// auto-restart path (D1a), reached without the `RestartTrigger`. This is the
/// "kill the server child" of doc 16 §8's restart-mid-match test.
async fn launch_thread_server_abortable(
    store: Arc<dyn ObjectStore>,
    crash: oneshot::Receiver<()>,
) -> Result<LaunchedServer, String> {
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
        rt.block_on(async move {
            tokio::select! {
                _ = mush_server::serve_with_store(cfg, Some(ipc_tx), Some(store)) => {}
                _ = crash => {}
            }
        });
        let _ = exit_tx.send(());
    });
    let endpoint = tokio::time::timeout(Duration::from_secs(30), ipc_rx)
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

/// The first boot is crashable; every boot after it is a healthy server.
struct CrashOnceLauncher {
    store: Arc<dyn ObjectStore>,
    launches: AtomicUsize,
    crash: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl ServerLauncher for CrashOnceLauncher {
    fn launch(&self) -> BoxFuture<'static, Result<LaunchedServer, String>> {
        let n = self.launches.fetch_add(1, Ordering::SeqCst);
        let store = Arc::clone(&self.store);
        if n == 0 {
            let (crash_tx, crash_rx) = oneshot::channel();
            if let Ok(mut slot) = self.crash.lock() {
                *slot = Some(crash_tx);
            }
            Box::pin(launch_thread_server_abortable(store, crash_rx))
        } else {
            Box::pin(launch_thread_server(store))
        }
    }
}

async fn seeded_gw_store() -> Arc<dyn ObjectStore> {
    let gw_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    gw_store
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
        .expect("the account");
    gw_store
}

async fn start_split(launcher: Box<dyn ServerLauncher>) -> (SocketAddr, RestartTrigger) {
    start_split_with(seeded_gw_store().await, launcher).await
}

/// **The one store both halves are pointed at** (16b cut U.4), as
/// `arena_story_split.rs` seeds it: the staff account, the two pilots, and the
/// world their characters live in, written through `mush_server::persist`'s own
/// boot-batch path. A real split points both processes at one Redis, and `SIM
/// TEAM` fails closed on a character the account oracle does not know.
async fn shared_store() -> Arc<dyn ObjectStore> {
    let store = seeded_gw_store().await;
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

/// [`start_split`] over a given gateway store.
async fn start_split_with(
    gw_store: Arc<dyn ObjectStore>,
    launcher: Box<dyn ServerLauncher>,
) -> (SocketAddr, RestartTrigger) {
    let (ssh_tx, ssh_rx) = oneshot::channel();
    let (restart_tx, restart_rx) = oneshot::channel::<RestartTrigger>();
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
                source: ServerSource::Restartable(launcher),
                store: Some(gw_store),
                ssh_ready: Some(ssh_tx),
                web_ready: None,
                restart_ready: Some(restart_tx),
                siteban_refresh: None,
            },
        ));
    });
    let ssh_addr = tokio::time::timeout(Duration::from_secs(30), ssh_rx)
        .await
        .expect("gateway bound SSH")
        .expect("ssh addr");
    let trigger = tokio::time::timeout(Duration::from_secs(30), restart_rx)
        .await
        .expect("supervisor handed back a restart trigger")
        .expect("restart trigger");
    (ssh_addr, trigger)
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

    async fn expect(&mut self, needle: &str) {
        self.expect_after(0, needle).await;
    }

    async fn send_line(&mut self, line: &str) {
        let bytes = format!("{line}\r").into_bytes();
        self.channel.data(&bytes[..]).await.expect("send line");
    }

    /// Log in, waiting out a server half that is still booting. See
    /// [`WORLD_NOT_UP`] for why this one line retries and no other does.
    async fn connect_when_admitted(&mut self, account: &str, password: &str, connected: &str) {
        for _ in 0..ADMISSION_TRIES {
            let mark = self.mark();
            self.send_line(&format!("connect {account} {password}"))
                .await;
            if self
                .expect_one_of_after(mark, &[connected, WORLD_NOT_UP])
                .await
                == 0
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        panic!("the server half never admitted `{account}`");
    }

    /// Run one line and wait for `needle` on the same span, retrying while the
    /// world answers the **transient** authoritative refusal.
    ///
    /// `@arena`'s switches are on the authoritative lane (they check units out
    /// and hand an arena custody), and that lane answers "the world is
    /// committing another action; please retry in a moment" when a commit is in
    /// flight. That is a real answer a real client retries, not a failure, so a
    /// test that drives the lane retries it too.
    ///
    /// The needle must not appear in the line's own echo, or it would match
    /// before the world had even seen the command.
    async fn say(&mut self, line: &str, needle: &str) {
        assert!(
            !line.contains(needle),
            "`{needle}` appears in `{line}`'s own echo; pick a needle from the ANSWER"
        );
        for _ in 0..RETRY_BUDGET {
            let mark = self.mark();
            self.send_line(line).await;
            let which = self
                .expect_one_of_after(mark, &[needle, COMMIT_PENDING])
                .await;
            if which == 0 {
                return;
            }
        }
        panic!("`{line}` was refused as commit-pending {RETRY_BUDGET} times running");
    }

    /// Wait for whichever of `needles` arrives first, answering its index.
    /// Run one line, wait for `needle`, and hand back everything the world said
    /// since — the one place this lens has to *read* an answer rather than
    /// assert on it, because the arena's twelve tanks are made by a content pack
    /// and their dbrefs are the pack's to choose (16b cut C.2).
    async fn ask(&mut self, line: &str, needle: &str) -> String {
        let mark = self.mark();
        self.say(line, needle).await;
        strip_ansi(&self.seen[mark..])
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
}

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

/// **Wait until `check` answers `true`**, polling every 200 ms, or fail naming
/// `label` (16b cut M.1). `crates/mushd/tests/support::wait_until`'s shape (cut L,
/// `T-41`): `crates/mush-gateway/tests` has no shared support module, so this file
/// carries its own copy, as it carries the rest of its rig.
async fn wait_until(bound: Duration, label: &str, mut check: impl AsyncFnMut() -> bool) {
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

/// Whether `unit`'s authoritative record carries a `deployed` marker, as the
/// server half's store holds it.
async fn deployed(store: &Arc<dyn ObjectStore>, unit: u32) -> bool {
    store
        .module_json_get("tactics", &format!("unit:{unit}"))
        .await
        .expect("the store read")
        .is_some_and(|document| document.get("deployed").is_some())
}

/// The match id in `Match N is running on ...`.
fn running_match_id(answer: &str) -> u64 {
    answer
        .lines()
        .find_map(|line| {
            let rest = line.split_once("Match ")?.1;
            let (id, tail) = rest.split_once(' ')?;
            if tail.starts_with("is running on") {
                id.parse::<u64>().ok()
            } else {
                None
            }
        })
        .unwrap_or_else(|| {
            panic!(
                "no running match id in:
{answer}"
            )
        })
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

// ---------------------------------------------------------------------------
// The lens
// ---------------------------------------------------------------------------

/// Whether the server half's store holds `tacsim:snapshot:<match_id>` — the
/// arena's own snapshot of a running match (16b cut AF.2), which a boot resumes
/// from (16b cut AF.3).
async fn snapshot_on_disk(store: &Arc<dyn ObjectStore>, match_id: u64) -> bool {
    store
        .module_json_get("tacsim", &format!("tacsim:snapshot:{match_id}"))
        .await
        .expect("the store read")
        .is_some()
}

/// `text` with every run of whitespace one space: the line client wraps at the
/// terminal's width, and the resume line is longer than eighty columns.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How many times `needle` appears in what `term` received since `mark`, both
/// flattened.
fn count_since(term: &Term, mark: usize, needle: &str) -> usize {
    flat(&strip_ansi(&term.seen[mark..]))
        .matches(flat(needle).as_str())
        .count()
}

/// The arena lifecycle surface is live across the boundary, and a restart
/// **while a match is running** resumes it.
///
/// **16b cut M.1** (`T-42`) made the crash come while the match is `Running`,
/// once both units' `deployed` markers are in the store. **16b cut AF.3**: and
/// once the arena's snapshot is too, because since that cut a server-half
/// restart resumes a running match from it. So afterwards the surviving staff
/// session reads `@arena/status` with the match running and its `resumed:`
/// line, the store holds no result document for it, the admission site still
/// refuses its unit, and `@arena/abort` ends it the ordinary way — an `abort`
/// result, and neither unit left in custody — after which the lifecycle
/// refusals still answer as themselves and the unit is movable again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_arena_surface_survives_the_split_and_a_server_restart() {
    let server_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let crash: Arc<Mutex<Option<oneshot::Sender<()>>>> = Arc::new(Mutex::new(None));
    let (ssh_addr, _trigger) = start_split(Box::new(CrashOnceLauncher {
        store: Arc::clone(&server_store),
        launches: AtomicUsize::new(0),
        crash: Arc::clone(&crash),
    }))
    .await;

    let (_handle, mut god) = play_client(ssh_addr).await;
    god.connect_when_admitted(
        "boss",
        GM_PASSWORD,
        "Connected as God (account boss, 1 connection).",
    )
    .await;
    god.expect("Limbo #0").await;

    // --- (1) the read crosses the boundary -------------------------------
    god.say("@arena/status", "No match is running.").await;

    // --- (2) so do the lifecycle switches, as themselves ------------------
    //
    // Each refusal is the arena's own sentence arriving intact through the IPC
    // boundary, which is the thing a split can break and an all-in-one cannot.
    god.say("@arena/start", "Usage:").await;
    god.say("@arena/abort", "No match is running.").await;
    god.say("@arena/intent #9999=startup", "not in a running match")
        .await;
    god.say(
        "@arena/start tacsim.arena.proving_ground=#9998/#9999",
        "is not a tactical unit",
    )
    .await;
    // And an unknown arena is refused by name rather than by silence.
    god.say("@arena/start nowhere=#9998/#9999", "no arena definition")
        .await;

    // --- (2b) the whole round trip, on the units the content made ---------
    let red = dbref_of(
        &god.ask("@find Red Tank 1", "END OF LIST").await,
        "Red Tank 1",
    );
    let blue = dbref_of(
        &god.ask("@find Blue Tank 1", "END OF LIST").await,
        "Blue Tank 1",
    );
    let started = god
        .ask(
            &format!("@arena/start tacsim.arena.proving_ground=#{red}/#{blue}"),
            "is running on",
        )
        .await;
    let match_id = running_match_id(&started);
    god.say("@arena/status", "running, tempo").await;
    god.say(&format!("@arena/intent #{red}=startup"), "Ordered")
        .await;
    // And an admission site refuses while the arena holds it — the custody
    // error, crossing the boundary as itself.
    god.say(
        &format!("@spatial/unplace #{red}"),
        "deployed in arena match",
    )
    .await;
    // Both markers and the arena's snapshot durable before the crash, or the
    // reboot has nothing to resume from.
    wait_until(
        STEP_TIMEOUT,
        "both custody markers and the snapshot to land",
        async || {
            deployed(&server_store, red).await
                && deployed(&server_store, blue).await
                && snapshot_on_disk(&server_store, match_id).await
        },
    )
    .await;

    // --- (3) make the server EXIT ON ITS OWN, mid-match --------------------
    let before_crash = god.mark();
    let crash_tx = crash
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
        .expect("the first boot published its crash sender");
    let _ = crash_tx.send(());

    god.expect_after(before_crash, "restarting").await;
    god.expect_after(before_crash, "Reconnected").await;
    god.expect_after(before_crash, "Limbo #0").await;

    // --- (4) the match resumed -------------------------------------------
    god.say("@arena/status", "resumed: from a snapshot at")
        .await;
    let status = god.ask("@arena/status", "last snapshot: at").await;
    assert!(
        status.contains(&format!("match {match_id}: running")),
        "{status}"
    );
    let key = format!("tacsim:match:{match_id}");
    assert!(
        server_store
            .module_json_get("tacsim", &key)
            .await
            .expect("the store read")
            .is_none(),
        "a resumed match writes no result"
    );
    assert!(deployed(&server_store, red).await, "the red marker stays");
    assert!(deployed(&server_store, blue).await, "the blue marker stays");
    god.say(
        &format!("@spatial/unplace #{red}"),
        "deployed in arena match",
    )
    .await;

    // --- (5) and ends the ordinary way ------------------------------------
    god.say(&format!("@arena/abort {match_id}"), "is stopping")
        .await;
    wait_until(STEP_TIMEOUT, "the abort's result document", async || {
        server_store
            .module_json_get("tacsim", &key)
            .await
            .expect("the store read")
            .is_some_and(|result| {
                result.get("reason").and_then(serde_json::Value::as_str) == Some("abort")
            })
    })
    .await;
    wait_until(STEP_TIMEOUT, "both markers to go", async || {
        !deployed(&server_store, red).await && !deployed(&server_store, blue).await
    })
    .await;
    god.say("@arena/abort", "No match is running.").await;
    // The lifecycle refusals still come back across the rebooted boundary as
    // themselves.
    god.say(
        "@arena/start tacsim.arena.proving_ground=#9998/#9999",
        "is not a tactical unit",
    )
    .await;
    // Last, because it takes the tank off the map: the admission site that
    // refused while the resumed arena held the unit lets it go once the abort
    // checked it in.
    let answer = god
        .ask(&format!("@spatial/unplace #{red}"), "Unplaced")
        .await;
    assert!(
        !answer.contains("deployed in arena match"),
        "a unit the abort checked in is still refused as deployed: {answer}"
    );

    drop(god);
}

/// Two pilots claim, start and walk to their team rooms on `store`'s split; the
/// ones `board` names (0 Ada, 1 Bo) `BOARD`, retried on the control-lease
/// mirror's transient.
async fn claim_and_board(ada: &mut Term, bo: &mut Term, board: &[usize]) {
    for term in [&mut *ada, &mut *bo] {
        term.say("ooc", "Welcome Hall").await;
        // `SIM` is a verb before it is an exit, so the walk takes the exit's
        // other name, as `arena_story_split.rs` does.
        term.say("simulator", "Simulator Lobby").await;
    }
    ada.say("SIM TEAM red", "takes red slot 1").await;
    bo.say("SIM TEAM blue", "takes blue slot 1").await;
    ada.say("SIM START", "is boarding").await;
    ada.say("red", "Red Room").await;
    bo.say("blue", "Blue Room").await;
    for (n, term) in [&mut *ada, &mut *bo].into_iter().enumerate() {
        if !board.contains(&n) {
            continue;
        }
        // The control-lease mirror is a 1 Hz projection, and a `BOARD` before
        // its first read is refused as the transient it is.
        wait_until(STEP_TIMEOUT, "a pilot to board", async || {
            let mark = term.mark();
            term.send_line("BOARD").await;
            term.expect_one_of_after(mark, &["climb into", CONTROL_UNKNOWN, COMMIT_PENDING])
                .await
                == 0
        })
        .await;
    }
}

/// The split with a crashable first boot, over one shared store.
async fn crashable_split(
    store: &Arc<dyn ObjectStore>,
) -> (SocketAddr, Arc<Mutex<Option<oneshot::Sender<()>>>>) {
    let crash: Arc<Mutex<Option<oneshot::Sender<()>>>> = Arc::new(Mutex::new(None));
    let (ssh_addr, _trigger) = start_split_with(
        Arc::clone(store),
        Box::new(CrashOnceLauncher {
            store: Arc::clone(store),
            launches: AtomicUsize::new(0),
            crash: Arc::clone(&crash),
        }),
    )
    .await;
    (ssh_addr, crash)
}

/// Make the first boot's server half exit on its own.
fn crash_now(crash: &Arc<Mutex<Option<oneshot::Sender<()>>>>) {
    let crash_tx = crash
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
        .expect("the first boot published its crash sender");
    let _ = crash_tx.send(());
}

/// **A pilot in a pod across a server restart boards the resumed match** (16b
/// cut AF.3; the split's whole point): the gateway holds the connections while
/// the server half restarts, and from AF.3 the match they were flying survives
/// it too.
///
/// Two pilots really `BOARD`; once `Good hunting` is read and both markers and
/// the arena's snapshot are durable, the server half exits on its own. Each
/// surviving session reads `Reconnected`, then the resume line and Y.3's line
/// (`… is flying on its mind; …`) exactly once; `BOARD` (retried while the tank
/// is still being handed over) takes the tank back where it stands and the
/// first frame says `Back aboard`; the markers are still there; `STATUS`
/// answers a frame.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pilot_in_a_pod_across_a_server_restart_boards_the_resumed_match() {
    let store = shared_store().await;
    let (ssh_addr, crash) = crashable_split(&store).await;
    let (_g, mut god) = play_client(ssh_addr).await;
    god.connect_when_admitted("boss", GM_PASSWORD, "Connected as God")
        .await;
    let red = dbref_of(
        &god.ask("@find Red Tank 1", "END OF LIST").await,
        "Red Tank 1",
    );
    let blue = dbref_of(
        &god.ask("@find Blue Tank 1", "END OF LIST").await,
        "Blue Tank 1",
    );
    let (_a, mut ada) = play_client(ssh_addr).await;
    ada.connect_when_admitted("ada", PILOT_PASSWORD, "Connected as Ada")
        .await;
    let (_b, mut bo) = play_client(ssh_addr).await;
    bo.connect_when_admitted("bo", PILOT_PASSWORD, "Connected as Bo")
        .await;
    claim_and_board(&mut ada, &mut bo, &[0, 1]).await;
    ada.expect("Good hunting").await;
    wait_until(
        STEP_TIMEOUT,
        "both custody markers and the snapshot to land",
        async || {
            deployed(&store, red).await
                && deployed(&store, blue).await
                && snapshot_on_disk(&store, 1).await
        },
    )
    .await;

    let (ada_mark, bo_mark) = (ada.mark(), bo.mark());
    crash_now(&crash);

    let resumed = tacsim::match_::RESUME_LINE;
    for (term, mark, tank) in [
        (&mut ada, ada_mark, "Red Tank 1"),
        (&mut bo, bo_mark, "Blue Tank 1"),
    ] {
        // Y.3's line (`… is flying on doctrine; …` until 17b cut M5.3, one
        // word for every sealed mind since).
        let on_its_mind = format!("{tank} is flying on its mind; BOARD takes it back.");
        term.expect_after(mark, "Reconnected").await;
        term.expect_after(mark, &on_its_mind).await;
        // Two more boarding ticks: nothing is said again.
        tokio::time::sleep(Duration::from_millis(2_500)).await;
        term.say("zzz-sentinel", "Huh?").await;
        assert_eq!(count_since(term, mark, resumed), 1, "the resume line, once");
        assert_eq!(
            count_since(term, mark, &on_its_mind),
            1,
            "the reconnect line, once"
        );
        let seen = flat(&strip_ansi(&term.seen[mark..]));
        assert!(
            seen.find(flat(resumed).as_str()) < seen.find(flat(&on_its_mind).as_str()),
            "the resume line first: {seen}"
        );
    }
    for term in [&mut ada, &mut bo] {
        let mark = term.mark();
        wait_until(STEP_TIMEOUT, "the tank to come back", async || {
            let at = term.mark();
            term.send_line("BOARD").await;
            term.expect_one_of_after(
                at,
                &[
                    "yours again where it stands",
                    "still being handed over",
                    CONTROL_UNKNOWN,
                    COMMIT_PENDING,
                ],
            )
            .await
                == 0
        })
        .await;
        term.expect_after(mark, "Back aboard").await;
    }
    assert!(deployed(&store, red).await, "the red marker stays");
    assert!(deployed(&store, blue).await, "the blue marker stays");
    ada.say("STATUS", "power").await;
    bo.say("STATUS", "power").await;
}

/// **A pilot in a pod across a crash during `Boarding` reads the hatch line**
/// (16b cut U.4, 16c `B-37`, kept by 16b cut AF.3): nothing was checked out yet
/// — no marker, no snapshot, nothing to resume — so the boot opens the pod and
/// gives the billet back, and because boot output reaches nobody the boarding
/// tick says the hatch line to the pilot whose session the gateway re-attached.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pilot_in_a_pod_across_a_boarding_crash_reads_the_hatch_line() {
    let store = shared_store().await;
    let (ssh_addr, crash) = crashable_split(&store).await;
    let (_g, mut god) = play_client(ssh_addr).await;
    god.connect_when_admitted("boss", GM_PASSWORD, "Connected as God")
        .await;
    let red = dbref_of(
        &god.ask("@find Red Tank 1", "END OF LIST").await,
        "Red Tank 1",
    );
    let blue = dbref_of(
        &god.ask("@find Blue Tank 1", "END OF LIST").await,
        "Blue Tank 1",
    );
    let (_a, mut ada) = play_client(ssh_addr).await;
    ada.connect_when_admitted("ada", PILOT_PASSWORD, "Connected as Ada")
        .await;
    let (_b, mut bo) = play_client(ssh_addr).await;
    bo.connect_when_admitted("bo", PILOT_PASSWORD, "Connected as Bo")
        .await;
    // Ada boards; Bo does not, so the match is still `Boarding`.
    claim_and_board(&mut ada, &mut bo, &[0]).await;
    assert!(!deployed(&store, red).await && !deployed(&store, blue).await);

    let ada_mark = ada.mark();
    crash_now(&crash);

    ada.expect_after(ada_mark, "Reconnected").await;
    ada.expect_after(ada_mark, "The hatch of").await;
    ada.expect_after(ada_mark, "swings open.").await;
    assert!(!deployed(&store, red).await, "no red marker");
    assert!(!deployed(&store, blue).await, "no blue marker");
    ada.say("STATUS", "You are not at the station of a deployed unit.")
        .await;
}
