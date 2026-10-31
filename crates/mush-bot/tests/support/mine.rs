//! **The mining rig** (doc 20 cut SM.12c-bot): the production root —
//! `mushd::boot::boot_all_in_one`, the deployed all-in-one's own boot —
//! booted **inside the bot's own test binary**, where `serde_json` carries
//! `arbitrary_precision` (no mining root ran under it before this cut), with
//! `sm_story`'s config shape (memory store, world rate 24, the sky at rate 0)
//! and the fixed geology key; the world task and the SSH listener as `mushd`'s
//! `serve` runs them (`crates/mushd/src/main.rs:383-475`); and the arena rig's
//! [`Client`](super::arena::Client), [`launch`](super::arena::launch) and
//! [`try_land`](super::arena::try_land) through
//! [`Rig::serving`](super::arena::Rig::serving).
//!
//! Game-crate imports here serve the server under test and nothing else,
//! exactly as `support/arena.rs`'s do: the bot's mining lane never sees a game
//! type. The accounts are made **after** the boot and before the world task
//! spawns — `boot_all_in_one`'s memory backend makes its store inside the boot
//! — each character linked through `Engine::link_account_character`, as
//! `sm_story`'s `pilot` links one (`crates/mush-server/tests/commerce_composition/sm_story.rs:562`).
//! Nothing here reaches a module-state slot, so the bot rig adds no
//! `state-slot-owners.toml` row.
//!
//! **The rate lift** (the testing-rate trial, `docs/testing-rate.md`): the rig boots
//! at the play rate because the production Rate line gates `+coop/enrol` at exactly
//! `PLAY_RATE` (`readiness.rs:37`, `coop.rs:880`); once a pilot has enrolled nothing
//! on the mining loop reads that line again (a departure to a site asks
//! `port_ready`, the site's own lines), so a rig may have God open a faster
//! world-time generation — `ic_slice_loop.rs`'s pattern (lead ruling L1) — through
//! [`rate_lift`] and [`lift_rate`]. Each rig names its own default rate: the canary
//! lifts to [`LIFTED_RATE`] 168 (the owner's call of 2026-10-27: 165 s off its 528-s
//! wall, the game clock being the only thing the lift buys); the cohort plays at
//! [`BOOT_RATE`] 24 throughout, its four pilots enrolling at different times. The
//! `MINE_RATE` environment variable overrides any rig's default — `MINE_RATE=24` is
//! the rate-24 run, the world as the deploy envs seed it.
#![allow(dead_code)]

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use mush_bot::mine::{Lane, LaneEvent, Site};
use mush_core::{AccountId, GOD, LIMBO, ObjKind};
use mush_session::{SessionConfig, SessionManager};
use mush_store::{AccountRec, CharacterRef, ObjectStore};
use mushd::{
    accounts::{Accounts, hash_password},
    config::Config,
    controls::GatewayControls,
    ipguard::IpGuard,
    quarantine::QuarantineSpool,
    ssh_bridge::{SshBridge, start_listener},
    world_task::{ServerDeps, spawn_persist_with_decision_workers},
};

use super::arena;

/// `sm_story`'s `STORY_KEY` (`:64`): the tacsim suites' `KEY_1` bytes, so the
/// ground is the one SM.7's and SM.8's tests measured.
pub const KEY_1: [u8; 32] = *b"rustmush-sm1-geology-test-key-01";
/// The readiness report's last line when every binding is in place.
pub const READY: &str = "Ready: every binding above is in place.";
/// The readiness report's last line otherwise.
pub const NOT_READY: &str = "Not ready: ";
/// The rate line, ready (`readiness.rs:388`, at `PLAY_RATE` 24).
pub const RATE_READY: &str = "world rate: 24 IC seconds per real second. [ready]";
/// The world rate the rig boots at: `PLAY_RATE`, the one rate the production Rate
/// line reads `[ready]` at on a world that is not a test world, so `+coop/enrol` is
/// open (`readiness.rs:384`).
pub const BOOT_RATE: u32 = 24;
/// The world rate the canary lifts to after the pilot's enrolment: an IC week per
/// real hour, the owner's figure. The hop, the launch and the scan run at a seventh
/// of their rate-24 walls; the per-fire budgets, the market's sweep and the lane's
/// own cadences do not move (`docs/testing-rate.md` §4).
pub const LIFTED_RATE: u32 = 168;

/// One miner: its gateway account, its character and the site its bot mines.
#[derive(Clone, Copy, Debug)]
pub struct Miner {
    pub account: &'static str,
    pub character: &'static str,
    pub site: Site,
}

/// **Write a fixed geology key into `dir`** — a local copy of `sm_story`'s
/// `write_fixed_key` (`:159-187`, as of `7c886408`, the 0700 fix), unix
/// permissions and all: CI runs on ubuntu-latest and the durable key store
/// refuses a wider file or directory (`crates/mush-config/src/durable_keys.rs:572-610`);
/// a test target cannot import another's helper. `geology-key-0000000001.hex`
/// holds the 32 bytes as 64 lowercase hex digits and a newline, and
/// `geology-ring-0000000001.json` names it current; both 0600 and the
/// directory 0700 on unix.
pub fn write_fixed_key(dir: &Path, key: [u8; 32]) {
    use std::fmt::Write as _;
    std::fs::create_dir_all(dir).expect("the key directory");
    let mut hex = String::with_capacity(65);
    for byte in key {
        let _ = write!(hex, "{byte:02x}");
    }
    hex.push('\n');
    let key_path = dir.join("geology-key-0000000001.hex");
    let ring_path = dir.join("geology-ring-0000000001.json");
    std::fs::write(&key_path, hex.as_bytes()).expect("the key file");
    std::fs::write(
        &ring_path,
        br#"{"format_version":1,"current":1,"retained":[]}"#,
    )
    .expect("the ring manifest");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        for path in [&key_path, &ring_path] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .expect("private key files");
        }
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .expect("a private key directory");
    }
}

/// **`sm_story`'s `config`** (`:191-204`) with this rig's directories: the
/// memory store, `data_dir`, IPC on `127.0.0.1:0`, the repository's
/// `docs/help`, `[tacsim].enabled`, no pack or seed paths, `[game].ic_ratio =
/// 24`, the sky at rate 0, `[tacsim.geology].key_dir`. `[llm]` stays at its
/// disabled default.
pub fn config(data_dir: &Path, key_dir: &Path) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = data_dir.to_path_buf();
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg.server.help_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/help");
    cfg.tacsim.enabled = true;
    cfg.tacsim.pack_paths = Vec::new();
    cfg.tacsim.seed_paths = Vec::new();
    cfg.game.ic_ratio = BOOT_RATE;
    cfg.tacsim.time.ephemeris.rate = 0;
    cfg.tacsim.geology.key_dir = Some(key_dir.to_path_buf());
    cfg
}

/// **The geology readiness line the fixed key reads** (`sm_story`'s
/// `geology_line`, `:958`): the key's fingerprint, its first twelve lowercase
/// hex digits, and the reading rule's version.
pub fn geology_line() -> String {
    let secret = mush_core::GeologySecret::new(1, KEY_1).expect("the fixed key");
    let hex: String = secret
        .fingerprint()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!(
        "geology: key v1, fingerprint {}, pinned; reading rule v{}.",
        &hex[..12],
        tacsim::surface::READING_RULE_VERSION
    )
}

/// A scratch directory of the rig's own (removed by the [`arena::Rig`]'s drop).
fn scratch_dir() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "mush-bot-mine-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the rig's scratch directory");
    dir
}

/// One gateway account holding one character.
async fn account(
    store: &Arc<dyn ObjectStore>,
    name: &str,
    password: &str,
    created: i64,
    character: mush_core::ObjId,
    character_name: &str,
) -> AccountId {
    let id = AccountId::mint(name, created).expect("an account id");
    store
        .create_account(AccountRec {
            id: Some(id),
            name: name.to_owned(),
            argon2_hash: hash_password(password).expect("a hash"),
            created,
            characters: vec![CharacterRef {
                id: character,
                name: character_name.to_owned(),
            }],
            ssh_keys: Vec::new(),
            discord: None,
        })
        .await
        .expect("the account is written");
    id
}

/// **Boot the server under test**: a scratch dir (`mush-bot-mine-{pid}-{n}`
/// under the temp dir); the fixed key written; `QuarantineSpool::open(data/
/// quarantine)`; `boot_all_in_one`; then, before the world task spawns, the
/// staff account `boss` ([`arena::GOD_ACCOUNT`], character God) and one
/// account per miner ([`arena::PILOT_PASSWORD`]), each miner a player in Limbo
/// made in the booted world, each account written to the boot's own store and
/// each character linked through `Engine::link_account_character` (asserted);
/// then one `IpGuard` shared by the world task's deps and the SSH bridge, the
/// world task through `ServerDeps::production` and
/// `spawn_persist_with_decision_workers`, the store-backed accounts, the
/// bridge, and the listener on `127.0.0.1:0` with [`arena::BANNER`].
pub async fn boot(miners: &[Miner]) -> arena::Rig {
    let dir = scratch_dir();
    let key_dir = dir.join("geology-keys");
    let data_dir = dir.join("data");
    std::fs::create_dir_all(&data_dir).expect("the data directory");
    write_fixed_key(&key_dir, KEY_1);
    let cfg = config(&data_dir, &key_dir);
    let mut spool =
        QuarantineSpool::open(data_dir.join("quarantine")).expect("the quarantine spool");
    let (opened, mut booted) = mushd::boot::boot_all_in_one(&cfg, &mut spool)
        .await
        .unwrap_or_else(|error| panic!("the production root boots: {error}"));
    let store = Arc::clone(&opened.store);

    let boss = account(
        &store,
        arena::GOD_ACCOUNT,
        arena::GOD_PASSWORD,
        1,
        GOD,
        "God",
    )
    .await;
    assert!(
        booted.engine.link_account_character(GOD, boss),
        "the oracle takes God"
    );
    for (n, miner) in miners.iter().enumerate() {
        let id = booted
            .engine
            .world
            .create(ObjKind::Player, miner.character, GOD, Some(LIMBO))
            .expect("a miner");
        let created = i64::try_from(n).unwrap_or(0) + 2;
        let linked = account(
            &store,
            miner.account,
            arena::PILOT_PASSWORD,
            created,
            id,
            miner.character,
        )
        .await;
        assert!(
            booted.engine.link_account_character(id, linked),
            "the oracle takes {}",
            miner.character
        );
    }

    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(120),
        ..SessionConfig::default()
    }));
    let guard = Arc::new(IpGuard::new());
    let deps = ServerDeps::production(
        &booted,
        Arc::clone(&store),
        Arc::clone(&guard),
        ServerDeps::in_memory().metrics,
        spool,
        Vec::new(),
    );
    let workers = booted.decision_workers;
    let checkpoint_s = booted.world_clock_checkpoint_s;
    let handles = spawn_persist_with_decision_workers(
        booted.engine,
        Arc::clone(&sessions),
        deps,
        workers,
        checkpoint_s,
        Arc::new(GatewayControls::default()),
    )
    .unwrap_or_else(|error| panic!("the world task spawns: {error}"));
    let accounts = Arc::new(Accounts::new(Arc::clone(&store), handles.handle.clone(), 6));
    let bridge = Arc::new(SshBridge::new(
        accounts,
        Arc::clone(&sessions),
        handles.handle.clone(),
        guard,
    ));
    let listener = start_listener(
        "127.0.0.1:0".parse().expect("a loopback address"),
        &dir,
        arena::BANNER.to_owned(),
        bridge,
    )
    .await
    .expect("ssh listener");
    arena::Rig::serving(listener, handles, dir, store)
}

/// **The rate and geology lines of a readiness report read `[ready]`**: the
/// rate line holding [`RATE_READY`] and the line `{geology_line()} [ready]`.
pub fn rate_and_geology_ready(report: &str) -> bool {
    let geology = format!("{} [ready]", geology_line());
    report.contains(RATE_READY) && report.lines().any(|line| line.trim_end() == geology)
}

/// **Staff: wait until the whole readiness report reads Ready** (`sm_story`'s
/// `stock`, `:548`, in real time): `@tacsim/port status` through
/// `Client::answer(…, &[READY, NOT_READY])` every 10 s, at most `bound`; then
/// assert the report's rate line ends "world rate: 24 IC seconds per real
/// second. [ready]" and it holds `{geology_line()} [ready]`. Returns the
/// seconds it took.
pub async fn await_ready(god: &mut arena::Client, bound: Duration) -> u64 {
    let started = Instant::now();
    let mut last = String::new();
    while started.elapsed() < bound {
        let (index, report) = god.answer("@tacsim/port status", &[READY, NOT_READY]).await;
        if index == Some(0) {
            assert!(
                rate_and_geology_ready(&report),
                "the rate and geology lines read ready:\n{report}\nwanted: {RATE_READY} and {} [ready]",
                geology_line()
            );
            return started.elapsed().as_secs();
        }
        last = report;
        god.idle(Duration::from_secs(10)).await;
    }
    panic!(
        "the readiness report did not read Ready within {} s:\n{last}",
        bound.as_secs()
    );
}

/// **The world rate a rig lifts to after the pilot's enrolment** (the testing-rate
/// trial, `docs/testing-rate.md`): the `MINE_RATE` environment variable when it is set
/// and not empty, else the rig's own `default` ([`LIFTED_RATE`] for the canary,
/// [`BOOT_RATE`] for the cohort). [`BOOT_RATE`] means no lift — the rig plays at 24
/// throughout and no staff act runs — so `MINE_RATE=24` is the rate-24 run of any rig;
/// any other rate inside the kernel's band
/// (`mush_core::MIN_WORLD_TIME_RATE..=MAX_WORLD_TIME_RATE`) is the one God opens with
/// [`lift_rate`] once the pilot has enrolled. A `MINE_RATE` that is not such a rate
/// panics: a mistyped run must not play quietly at the default.
#[must_use]
pub fn rate_lift(default: u32) -> Option<u32> {
    let rate: u32 = match std::env::var("MINE_RATE") {
        Ok(raw) if !raw.trim().is_empty() => {
            let raw = raw.trim();
            raw.parse().unwrap_or_else(|_| {
                panic!("MINE_RATE is not a rate in IC seconds per real second: {raw:?}")
            })
        }
        _ => default,
    };
    assert!(
        (mush_core::MIN_WORLD_TIME_RATE..=mush_core::MAX_WORLD_TIME_RATE).contains(&rate),
        "the lifted rate {rate} is outside the kernel's band {}..={}",
        mush_core::MIN_WORLD_TIME_RATE,
        mush_core::MAX_WORLD_TIME_RATE
    );
    (rate != BOOT_RATE).then_some(rate)
}

/// **Staff: open a new world-time generation at `rate`** — `@tacsim/time rate=<n>` as
/// God, `ic_slice_loop.rs`'s `set_rate` over SSH — asserted on the verb's own reply,
/// so a refused rate (outside the band, the live rate re-typed, a non-staff actor)
/// fails here, named, and not three beats later as a hop that never arrives. Returns
/// the reply, the subsystems' declared catch-up policies included.
pub async fn lift_rate(god: &mut arena::Client, rate: u32) -> String {
    let opened = format!("opens at {rate} IC seconds per real second");
    let (index, reply) = god
        .answer(
            &format!("@tacsim/time rate={rate}"),
            &[
                opened.as_str(),
                "The world time rate must be between",
                "The world already runs at",
                "Only staff change",
                "The rate was not changed",
            ],
        )
        .await;
    assert_eq!(
        index,
        Some(0),
        "God opens a world-time generation at {rate} IC seconds per real second:\n{reply}"
    );
    reply
}

/// **The bot's config, one file a miner** (D22):
/// `arena::bot_config_agent_at` at `bot/{account}.toml`, the username
/// `{account}+{character}`, `[agent] max_seconds`, settle 100 ms and an
/// observation window of 1 s — its path, distinct per account, so two bots
/// launched together never read each other's username (the arena's
/// `bot_config_agent` writes the one `bot/arena.toml`, which a second call
/// overwrites before the first child may have read it).
pub fn miner_config(rig: &arena::Rig, miner: &Miner, max_seconds: u64) -> PathBuf {
    arena::bot_config_agent_at(
        rig,
        &rig.dir.join("bot").join(format!("{}.toml", miner.account)),
        &format!("{}+{}", miner.account, miner.character),
        max_seconds,
        100,
        1_000,
        "",
    )
}

/// **A transcript's milestone stamp** (`Flight::fired`'s reader): the
/// `at_unix_ms` of the first `mine-milestone` record named `name`, read from
/// the transcript as it is written (one flushed record a line; a torn last
/// line is skipped until it is whole).
pub fn milestone_at(flight: &arena::Flight, name: &str) -> Option<u64> {
    let text = std::fs::read_to_string(&flight.transcript).ok()?;
    text.split_inclusive('\n')
        .filter(|line| line.ends_with('\n'))
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|record| record["kind"] == "mine-milestone" && record["data"]["name"] == name)
        .and_then(|record| record["data"]["at_unix_ms"].as_u64())
}

/// Unix milliseconds now.
pub fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// **Drive the lane in-process over `client`** (the canary's driver): every
/// 50 ms `client.pump`, `client.drain` fed to `lane.feed(now_ms)`,
/// `lane.tick(now_ms)`'s lines sent with `client.try_send`, `lane.events()`
/// printed as `canary:` lines; until `until(&lane)`, a result, or `bound`. The
/// lane's clock is unix milliseconds less its settings' epoch, so its report
/// stamps are real. Returns the elapsed real seconds and every line the lane
/// typed, in order (so the canary can show what it played, not only where it
/// got to).
pub async fn drive_lane(
    client: &mut arena::Client,
    lane: &mut Lane,
    until: impl Fn(&Lane) -> bool,
    bound: Duration,
) -> (u64, Vec<mush_bot::mine::Send>) {
    drive_lane_since(client, lane, Instant::now(), until, bound).await
}

/// [`drive_lane`] on a clock the caller holds: `bound` and the `canary: +N s` stamps
/// count from `started`, so a drive split around a staff act ([`lift_rate`]) prints
/// one run of stamps and keeps one budget. Returns the seconds since `started`.
pub async fn drive_lane_since(
    client: &mut arena::Client,
    lane: &mut Lane,
    started: Instant,
    until: impl Fn(&Lane) -> bool,
    bound: Duration,
) -> (u64, Vec<mush_bot::mine::Send>) {
    let mut sent = Vec::new();
    let epoch = lane.settings().epoch_unix_ms;
    while started.elapsed() < bound && lane.result().is_none() && !until(lane) {
        client.pump(Duration::from_millis(50)).await;
        let now = unix_now_ms().saturating_sub(epoch);
        let text = client.drain();
        if !text.is_empty() {
            lane.feed(now, &text);
        }
        for send in lane.tick(now) {
            println!(
                "canary: +{} s send {}",
                started.elapsed().as_secs(),
                send.line
            );
            if let Err(error) = client.try_send(&send.line).await {
                println!("canary: {error}");
            }
            sent.push(send);
        }
        for event in lane.events() {
            let text = match event {
                LaneEvent::Said(text) => text,
                other => format!("{other:?}"),
            };
            println!("canary: +{} s {text}", started.elapsed().as_secs());
        }
    }
    (started.elapsed().as_secs(), sent)
}
