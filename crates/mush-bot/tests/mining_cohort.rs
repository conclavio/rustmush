//! **The mining bot cohort** (doc 20 cut SM.12c-bot; doc 20 §13's SM.12 row,
//! its bot clause: "the bot cohort completes the loop in line mode", doc 14 P10
//! "Text is sufficient"; doc 20a §5.3.12).
//!
//! * **The canary** ([`the_mining_root_runs_under_the_bots_serde`], every
//!   build): the production root booted inside this test binary, where
//!   `serde_json` carries `arbitrary_precision`, and the bot's own mining lane
//!   driven in-process over the bot's own SSH session through the kit-up, the
//!   bunkers (two Draws through the market's numeric wires), the board, the
//!   hop, the witness map, the launch and one scan — so the lane's parsers are
//!   held to the live game in every build.
//! * **The cohort** ([`four_bots_mine_and_sell_over_ssh_in_line_mode`], run by
//!   hand and named in `ci.yml`, the owner's O-28 default (i)): four real
//!   `mush-bot` binaries, two at Ahuna Flank and two at Kerwan Floor, each
//!   site's second launched after the first's scan, each playing the whole
//!   loop with `--mine <site>`; it asserts every bot's receipts, its
//!   vocabulary, the observer rule on its witness map and the heard path
//!   between the two bots at each site, and prints one `mining_cohort:` line a
//!   bot, one a site and the run's totals.
//! * **The command line** ([`the_mine_flag_refuses_what_it_cannot_play`]):
//!   `--mine`'s refusals, before any connection, and the run it admits.
#[path = "support/arena.rs"]
mod arena;
#[path = "support/mine.rs"]
mod mine;

use std::{collections::BTreeSet, time::Duration};

use mine::Miner;
use mush_bot::mine::{
    Family, Lane, Milestone, Settings, Site, hull_name, in_vocabulary,
    lines::{self, GRID_COLUMNS, GRID_GLYPHS, ORE_GLYPHS, SITE_VIEW_CELLS},
};

/// The heard windows' slack on every edge (doc 20a §5.3.12).
const SLACK_MS: u64 = 5_000;

/// The four miners: two at Ahuna Flank, two at Kerwan Floor (both open
/// ground, neither worked by Occator's drones).
const MINERS: [Miner; 4] = [
    Miner {
        account: "minera",
        character: "Ardent",
        site: Site::Ahuna,
    },
    Miner {
        account: "minerb",
        character: "Bellwether",
        site: Site::Kerwan,
    },
    Miner {
        account: "minerc",
        character: "Cobalt",
        site: Site::Ahuna,
    },
    Miner {
        account: "minerd",
        character: "Dunmore",
        site: Site::Kerwan,
    },
];

// ---------------------------------------------------------------------------
// The canary
// ---------------------------------------------------------------------------

/// **The mining root runs under the bot's serde** (doc 20a §5.3.12's canary;
/// every build): God reads the rate and geology lines `[ready]`, then the
/// bot's lane, in-process over the bot's own SSH session, plays from Limbo
/// through its first scan at Ahuna Flank. Once the pilot has enrolled God lifts
/// the world rate to [`mine::LIFTED_RATE`] 168 (lead ruling L1's pattern; the
/// testing-rate trial, `docs/testing-rate.md`: the hop, the launch and the scan
/// at a seventh of their rate-24 walls, 363 s against 528); `MINE_RATE`
/// overrides it ([`mine::rate_lift`]), and `MINE_RATE=24` is the rate-24 run.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_mining_root_runs_under_the_bots_serde() {
    tokio::time::timeout(Duration::from_secs(20 * 60), canary())
        .await
        .expect("the canary finishes inside 20 minutes");
}

async fn canary() {
    let started = std::time::Instant::now();
    let miner = Miner {
        account: "canary",
        character: "Canary",
        site: Site::Ahuna,
    };
    let rig = mine::boot(&[miner]).await;
    let mut god = arena::Client::login(&rig, arena::GOD_ACCOUNT, arena::GOD_PASSWORD, "God").await;
    let (_, report) = god
        .answer("@tacsim/port status", &[mine::READY, mine::NOT_READY])
        .await;
    assert!(
        mine::rate_and_geology_ready(&report),
        "the rate and geology lines read ready at boot:\n{report}\nwanted: {} and {} [ready]",
        mine::RATE_READY,
        mine::geology_line()
    );

    let mut client =
        arena::Client::login(&rig, miner.account, arena::PILOT_PASSWORD, miner.character).await;
    let hull = hull_name("canary+Canary").expect("the canary's hull name");
    let mut lane = Lane::new(
        Site::Ahuna,
        Settings {
            hull,
            epoch_unix_ms: mine::unix_now_ms(),
        },
        miner.character.to_owned(),
    );
    // The drive, in two phases around the one staff act. Phase one runs from Limbo
    // to the enrolment — the lane's next line is the refit once `+coop/enrol` is
    // answered — under the boot rate the Rate line gates on.
    let lift = mine::rate_lift(mine::LIFTED_RATE);
    let drive_started = std::time::Instant::now();
    let bound = Duration::from_secs(19 * 60);
    let enrolled_now = |lane: &Lane| lane.step_line() == Some("+coop/refit prospector");
    let (_, mut sent) = mine::drive_lane_since(
        &mut client,
        &mut lane,
        drive_started,
        |lane| enrolled_now(lane) || lane.reached(Milestone::Scanned),
        bound,
    )
    .await;
    let enrolled = enrolled_now(&lane).then(|| started.elapsed().as_secs());
    // The staff act: God opens a new world-time generation at the lifted rate
    // (`LIFTED_RATE`, or `MINE_RATE`; none at 24) after the enrolment and before
    // the refit — lead ruling L1's pattern, as `ic_slice_loop.rs` sets 3600. The
    // lane's own cadences stay the shipped ones, sized to 24; what the lift buys
    // is game clock.
    if let (Some(rate), Some(_)) = (lift, enrolled) {
        let reply = mine::lift_rate(&mut god, rate).await;
        println!(
            "canary: +{} s the world rate lifted to {rate} after the enrolment:\n{}",
            drive_started.elapsed().as_secs(),
            reply.trim_end()
        );
    }
    // Phase two: the refit through the first scan.
    let (wall, rest) = mine::drive_lane_since(
        &mut client,
        &mut lane,
        drive_started,
        |lane| lane.reached(Milestone::Scanned),
        bound,
    )
    .await;
    sent.extend(rest);
    let report = lane.report();
    let seen = client.seen.clone();
    let beat = lane.beat().as_str();
    let fail = |why: &str| canary_failed(&seen, &report, beat, why);
    if let Some(result) = lane.result() {
        fail(&format!("the lane ended: {}", result.outcome));
    }
    for milestone in [
        Milestone::ArrivedSite,
        Milestone::Launched,
        Milestone::Scanned,
    ] {
        if !lane.reached(milestone) {
            fail(&format!("{milestone:?} was not reached in {wall} s"));
        }
    }
    // The witness print, read live (`site_map_lines` is the game's own and
    // `pub(crate)`): 32 rows, each at most 64 columns of grid glyphs.
    println!("canary: the witness rows:");
    for row in &report.witness {
        println!("canary: |{row}|");
    }
    if report.witness.len() != SITE_VIEW_CELLS
        || report.witness.iter().any(|row| {
            row.chars().count() > GRID_COLUMNS || !row.chars().all(|c| GRID_GLYPHS.contains(c))
        })
        || report.witness.iter().all(String::is_empty)
    {
        fail("the witness print is not 32 grid rows of the site view");
    }
    // The boards, read live: both asks the bunkers bought from, and both ore
    // bids the sale will read (`listing_line` is `pub(crate)`).
    let rows = lines::listings(&report.listings);
    let bid_buying =
        |family: Family| lines::bid_row(&rows, family).is_some_and(|row| !row.not_buying);
    if lines::ask_for(&rows, "reaction mass").is_none()
        || lines::ask_for(&rows, "provisions").is_none()
        || !bid_buying(Family::Metals)
        || !bid_buying(Family::Volatiles)
    {
        fail(&format!(
            "the listings do not read both asks and both buying bids: {:#?}",
            report.listings
        ));
    }
    // The Draws, played live: the bunkers bought through the market's
    // numeric wires (`QtyWire` under `arbitrary_precision`, the hazard this
    // canary exists for), so at least one `+market/quote` and one
    // `+market/accept` went in the Bunkers beat and the market answered
    // "Agreed." — a first plan that already met its want would buy nothing
    // and test nothing.
    let bunkers = |prefix: &str| {
        sent.iter()
            .filter(|send| {
                send.beat == mush_bot::mine::Beat::Bunkers && send.line.starts_with(prefix)
            })
            .count()
    };
    let (quotes, accepts) = (bunkers("+market/quote "), bunkers("+market/accept "));
    if quotes == 0 || accepts == 0 || !seen.contains("Agreed. Contract #") {
        fail(&format!(
            "no Draw was played in the bunkers: {quotes} quotes, {accepts} acceptances, \
             \"Agreed.\" {}",
            if seen.contains("Agreed. Contract #") {
                "read"
            } else {
                "never read"
            }
        ));
    }
    println!("canary: bunkers {quotes} quotes, {accepts} acceptances");
    match report.plan {
        Some(plan)
            if plan.burn_t > 0
                && plan.tank_t >= plan.burn_t
                && plan.home_burn_t.is_some()
                && plan.home_days.is_some() => {}
        other => fail(&format!("the last bunker plan does not read: {other:?}")),
    }
    if report.hull != "Canary Lode" {
        fail(&format!("the hull is {:?}", report.hull));
    }
    let arrived = report
        .milestone(Milestone::ArrivedSite)
        .map_or(0, |at| at.lane_ms / 1_000);
    println!(
        "canary: wall {} s at world rate {}, enrolment open after {} s, arrived after {arrived} s, lines {}, bytes {}",
        started.elapsed().as_secs(),
        lift.map_or_else(
            || mine::BOOT_RATE.to_string(),
            |rate| format!("{} then {rate} after the enrolment", mine::BOOT_RATE)
        ),
        enrolled.map_or_else(|| "?".to_owned(), |s| s.to_string()),
        report.lines_sent,
        report.bytes_received
    );
    client.close().await;
    god.close().await;
    drop(rig);
}

/// **The canary failed**: the client's last 4 KiB and the lane's report
/// printed, then a panic naming the beat. An engine-under-feature finding,
/// reported and never worked around (revision 2's F5).
fn canary_failed(seen: &str, report: &mush_bot::mine::Report, beat: &str, why: &str) -> ! {
    println!(
        "canary: the client's last 4 KiB:\n{}",
        mush_bot::tail(seen, 4 * 1024)
    );
    println!(
        "canary: the lane's report:\n{}",
        serde_json::to_string_pretty(report).unwrap_or_default()
    );
    panic!(
        "the canary failed in beat {beat}: {why} (an engine-under-feature finding: report it, \
         never work around it)"
    );
}

// ---------------------------------------------------------------------------
// The command line
// ---------------------------------------------------------------------------

/// Run the binary in a scratch home through the rig's own command builder
/// (`arena_e2e.rs`'s `cli`); its exit code, stdout and stderr.
fn cli(args: &[&str]) -> (Option<i32>, String, String) {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "mush-bot-mine-cli-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let config = dir.join("bot.toml");
    std::fs::write(
        &config,
        "[ssh]\nhost = \"127.0.0.1\"\nport = 1\nusername = \"pilot+Pilot\"\n\
         password_env = \"MUSH_BOT_ARENA_PASSWORD\"\n\
         [memory]\npersistent = false\n[costs]\npersistent = false\n",
    )
    .expect("a config");
    let output = arena::bot_command_in(&dir, &config, "unused")
        .args(args)
        .output()
        .expect("the binary runs");
    let _ = std::fs::remove_dir_all(&dir);
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// **`--mine`'s refusals** (doc 20 cut SM.12c-bot, B5): an unknown site, a
/// missing one, the arena, a goal, a session that sends its own lines and a
/// run that is not headless — each before any connection — then the run the
/// flag admits (`--mine ahuna --headless` reaches the connection: the
/// runner's headless admission), and the usage's new line.
#[test]
fn the_mine_flag_refuses_what_it_cannot_play() {
    let session = "--mine cannot be combined with --script, --paused, --managed-floor, \
                   --replay, --studio, --harness or a campaign session";
    for (args, sentence) in [
        (&["--mine", "vesta"][..], mush_bot::mine::MINE_SITE_USAGE),
        (&["--mine"][..], mush_bot::mine::MINE_SITE_USAGE),
        (
            &["--mine", "ahuna", "--arena", "red"][..],
            "--mine and --arena are two lanes; choose one",
        ),
        (
            &["--mine", "ahuna", "--goal", "x"][..],
            "--mine plays now and --goal lets the model decide; choose one",
        ),
        (&["--mine", "ahuna", "--paused"][..], session),
        // The runner's own refusal: the lane is headless (no REPL control or
        // `/arena` can reach a run while it plays).
        (
            &["--mine", "ahuna"][..],
            "the mining pilot plays headless; add --headless",
        ),
    ] {
        let (code, _, stderr) = cli(args);
        assert_eq!(code, Some(1), "{args:?}: {stderr}");
        assert_eq!(stderr.trim(), format!("mush-bot: {sentence}"), "{args:?}");
    }
    // **Admitted**: `--mine ahuna --headless` passes every refusal, the
    // runner's headless check included (`--mine` is a way to play headless
    // with no goal and no model), and goes on to connect — port 1 refuses
    // it, so the run ends on the connection, never on a refusal.
    let (code, stdout, stderr) = cli(&["--mine", "ahuna", "--headless"]);
    assert_eq!(code, Some(1), "{stdout}\n{stderr}");
    assert!(
        stdout.contains("[bot/connect] pilot+Pilot@127.0.0.1:1"),
        "it reached the connection:\n{stdout}\n{stderr}"
    );
    assert!(
        !stderr.contains("headless") && !stderr.contains("--mine"),
        "no refusal: {stderr}"
    );
    let (code, usage, _) = cli(&["--help"]);
    assert_eq!(code, Some(0));
    assert!(usage.contains("[--mine <site>]"), "{usage}");
}

// ---------------------------------------------------------------------------
// The cohort
// ---------------------------------------------------------------------------

/// One bot's flight, landed and read.
struct Flown {
    miner: Miner,
    order: usize,
    launched_at: u64,
    partial: arena::Partial,
    report: Option<mush_bot::mine::Report>,
    records: Vec<serde_json::Value>,
}

impl Flown {
    fn sends(&self) -> Vec<String> {
        self.records
            .iter()
            .filter(|record| record["kind"] == "mine-send")
            .filter_map(|record| record["data"]["line"].as_str().map(str::to_owned))
            .collect()
    }

    fn print(&self) {
        println!(
            "mining_cohort: === bot {} report:\n{}",
            self.miner.account,
            self.partial.report_text.as_deref().unwrap_or("(no report)")
        );
        println!(
            "mining_cohort: === bot {} last 60 records:",
            self.miner.account
        );
        let skip = self.records.len().saturating_sub(60);
        for record in &self.records[skip..] {
            println!("{record}");
        }
        if !self.partial.stderr.is_empty() {
            println!(
                "mining_cohort: === bot {} stderr:\n{}",
                self.miner.account, self.partial.stderr
            );
        }
    }
}

/// The block a stake receipt names (`… records your claim on block {b} at …`).
fn staked_block(report: &mush_bot::mine::Report) -> String {
    if report.mined_open_ground {
        return "open".to_owned();
    }
    report
        .receipts
        .stake
        .as_deref()
        .and_then(|line| line.split("claim on block ").nth(1))
        .and_then(|rest| rest.chars().next())
        .map_or_else(|| "none".to_owned(), |c| c.to_string())
}

/// Whether `t` falls in `[from, to]` with the slack on both edges.
fn within(t: u64, from: u64, to: u64) -> bool {
    t.saturating_add(SLACK_MS) >= from && t <= to.saturating_add(SLACK_MS)
}

/// **Four real bots mine and sell over SSH in line mode** (doc 20 §13's SM.12
/// row, the bot clause; doc 14 P10; doc 20a §5.3.12). Run by hand: named in
/// `ci.yml`, run by no job.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "the mining bot cohort: four real bots over SSH, about 35-45 minutes with the readiness wait, run by hand (doc 20 cut SM.12c-bot; named in ci.yml)"]
async fn four_bots_mine_and_sell_over_ssh_in_line_mode() {
    tokio::time::timeout(Duration::from_secs(75 * 60), cohort())
        .await
        .expect("the cohort finishes inside 75 minutes");
}

async fn cohort() {
    let started = std::time::Instant::now();
    // 1. The rig and readiness.
    let rig = mine::boot(&MINERS).await;
    let mut god = arena::Client::login(&rig, arena::GOD_ACCOUNT, arena::GOD_PASSWORD, "God").await;
    let ready_s = mine::await_ready(&mut god, Duration::from_secs(20 * 60)).await;
    println!("mining_cohort: ready after {ready_s} s");

    // 2. The launch order: each site's first at once, its second after the
    // first's scan.
    let configs: Vec<_> = MINERS
        .iter()
        .map(|miner| mine::miner_config(&rig, miner, 2_700))
        .collect();
    let distinct: BTreeSet<_> = configs.iter().collect();
    assert_eq!(distinct.len(), MINERS.len(), "one config file a miner");
    let launch = |i: usize| {
        let miner = &MINERS[i];
        arena::launch(
            &rig,
            &configs[i],
            miner.account,
            &["--mine", miner.site.word()],
        )
    };
    let first_launch = mine::unix_now_ms();
    let first_started = std::time::Instant::now();
    let mut flights: Vec<Option<arena::Flight>> = vec![None, None, None, None];
    let mut launched_at = [0_u64; 4];
    let mut order = [0_usize; 4];
    for (n, i) in [0, 1].into_iter().enumerate() {
        launched_at[i] = mine::unix_now_ms();
        order[i] = n + 1;
        flights[i] = Some(launch(i));
    }
    let mut next_order = 3;
    // minerc after minera's scan, minerd after minerb's.
    let mut waiting: Vec<(usize, usize)> = vec![(0, 2), (1, 3)];
    while !waiting.is_empty() {
        god.idle(Duration::from_secs(1)).await;
        let mut still = Vec::new();
        for (first, second) in waiting {
            let flight = flights[first].as_ref().expect("the first flies");
            if mine::milestone_at(flight, "scanned").is_some() {
                launched_at[second] = mine::unix_now_ms();
                order[second] = next_order;
                next_order += 1;
                flights[second] = Some(launch(second));
            } else if flight.task.is_finished() {
                let partial = arena::try_land(
                    flights[first].take().expect("the flight"),
                    Duration::from_secs(5),
                )
                .await;
                panic!(
                    "bot {} ended before its scan:\nstdout:\n{}\nstderr:\n{}\ntranscript:\n{}",
                    MINERS[first].account,
                    mush_bot::tail(&partial.stdout, 8_000),
                    partial.stderr,
                    mush_bot::tail(&partial.transcript_text, 8_000)
                );
            } else if first_started.elapsed() > Duration::from_secs(30 * 60) {
                panic!(
                    "bot {} did not scan within 30 minutes",
                    MINERS[first].account
                );
            } else {
                still.push((first, second));
            }
        }
        waiting = still;
    }

    // 3. Land all four inside 45 minutes from the first launch.
    let bound = Duration::from_secs(45 * 60);
    let mut flown = Vec::new();
    for i in 0..MINERS.len() {
        let left = bound
            .saturating_sub(first_started.elapsed())
            .max(Duration::from_secs(5));
        let partial = arena::try_land(flights[i].take().expect("a flight"), left).await;
        let report = partial
            .report
            .as_ref()
            .and_then(|report| report.get("mine").cloned())
            .and_then(|mine| serde_json::from_value::<mush_bot::mine::Report>(mine).ok());
        let records = partial
            .transcript_text
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .collect();
        flown.push(Flown {
            miner: MINERS[i],
            order: order[i],
            launched_at: launched_at[i],
            partial,
            report,
            records,
        });
    }
    let last_sale = flown
        .iter()
        .filter_map(|f| {
            f.report
                .as_ref()?
                .milestone(Milestone::Sold)
                .map(|m| m.at_unix_ms)
        })
        .max()
        .unwrap_or(first_launch);

    // 4. Per bot.
    let mut failures: Vec<String> = Vec::new();
    let heard_set: BTreeSet<String> = lines::heard_sentences().into_iter().collect();
    for f in &flown {
        let who = f.miner.account;
        let runner_success = f
            .partial
            .report
            .as_ref()
            .and_then(|r| r["success"].as_bool())
            .unwrap_or(false);
        let Some(report) = &f.report else {
            failures.push(format!("{who}: no mine report"));
            continue;
        };
        if !runner_success || !report.success {
            failures.push(format!(
                "{who}: not a success: {}",
                report.failure.as_deref().unwrap_or("(no failure named)")
            ));
        }
        if report.hull != format!("{} Lode", f.miner.character) || report.me != f.miner.character {
            failures.push(format!("{who}: played as {} ({})", report.me, report.hull));
        }
        if report.receipts.stake.is_none() && !report.mined_open_ground {
            failures.push(format!("{who}: no stake receipt and not open ground"));
        }
        if report.receipts.unloaded.is_none() {
            failures.push(format!("{who}: no unload receipt"));
        }
        if !report
            .receipts
            .sale
            .as_ref()
            .is_some_and(|sale| sale.qty_t > 0)
        {
            failures.push(format!("{who}: no sale receipt"));
        }
        for line in f.sends() {
            if !in_vocabulary(&line) || line.starts_with('@') {
                failures.push(format!("{who}: typed `{line}`"));
            }
        }
        // The observer rule: the bot's rig was in the bay, so any ore glyph
        // on its witness map is someone else's reading (SM.4b D12).
        if report.witness.is_empty() {
            failures.push(format!("{who}: no witness map"));
        }
        for row in &report.witness {
            if row.chars().any(|c| ORE_GLYPHS.contains(&c)) {
                failures.push(format!("{who}: an ore glyph on the witness map: |{row}|"));
            }
        }
        for heard in &report.heard {
            if !lines::heard_line_is_bare(&heard.line) || !heard_set.contains(&heard.sentence) {
                failures.push(format!(
                    "{who}: a heard line that is not bare: {:?}",
                    heard.line
                ));
            }
        }
    }
    // The ordering: each site's second witnessed after its first scanned.
    let report_of = |i: usize| flown[i].report.as_ref();
    let mut site_lines = Vec::new();
    for (a, b) in [(0, 2), (1, 3)] {
        let (Some(ra), Some(rb)) = (report_of(a), report_of(b)) else {
            failures.push(format!(
                "site {}: a report is missing",
                MINERS[a].site.name()
            ));
            continue;
        };
        let scanned = ra.milestone(Milestone::Scanned).map(|m| m.at_unix_ms);
        let witness = rb
            .beat(mush_bot::mine::Beat::Witness)
            .map(|s| s.start_unix_ms);
        if !matches!((scanned, witness), (Some(s), Some(w)) if w > s) {
            failures.push(format!(
                "{}: its witness ({witness:?}) did not start after {}'s scan ({scanned:?})",
                MINERS[b].account, MINERS[a].account
            ));
        }
        // 5. The heard path between A and B.
        let a_arrived = ra.milestone(Milestone::ArrivedSite).map(|m| m.at_unix_ms);
        let b_arrived = rb.milestone(Milestone::ArrivedSite).map(|m| m.at_unix_ms);
        let a_home = ra.beat(mush_bot::mine::Beat::Home).map(|s| s.start_unix_ms);
        let b_home = rb.beat(mush_bot::mine::Beat::Home).map(|s| s.start_unix_ms);
        let b1 = rb.sounding(1);
        let a2 = ra.sounding(2).filter(|span| span.skipped.is_none());
        let heard1 = b1.is_some_and(|span| {
            let to = span.done_unix_ms.unwrap_or(span.sent_unix_ms);
            ra.heard
                .iter()
                .any(|h| within(h.at_unix_ms, span.sent_unix_ms, to))
        });
        let heard2 = match (a2, b_arrived) {
            (Some(span), Some(arrived)) => {
                let to = span.done_unix_ms.unwrap_or(span.sent_unix_ms);
                rb.heard
                    .iter()
                    .any(|h| h.at_unix_ms >= arrived && within(h.at_unix_ms, span.sent_unix_ms, to))
            }
            _ => false,
        };
        let b1_done = b1.and_then(|span| span.done_unix_ms);
        let overlap1 = match (a_arrived, b1_done, a_home) {
            (Some(arrived), Some(done), Some(home)) => within(done, arrived, home),
            _ => false,
        };
        let a2_done = a2.and_then(|span| span.done_unix_ms);
        let overlap2 = match (a2_done, b_arrived, b_home) {
            (Some(done), Some(arrived), Some(home)) => within(done, arrived, home),
            _ => false,
        };
        let verdict = if heard1 || heard2 {
            "pass".to_owned()
        } else if !overlap1 && !overlap2 {
            "heard path not exercised: no overlap".to_owned()
        } else {
            format!("heard path open but silent: overlap1 {overlap1} overlap2 {overlap2}")
        };
        if verdict != "pass" {
            failures.push(format!("site {}: {verdict}", MINERS[a].site.name()));
        }
        if let Some(arrived) = b_arrived
            && let Some(early) = rb.heard.iter().find(|h| h.at_unix_ms < arrived)
        {
            failures.push(format!(
                "{}: heard {:?} before its own arrival",
                MINERS[b].account, early.sentence
            ));
        }
        site_lines.push(format!(
            "mining_cohort: site {} heard1 {heard1} heard2 {heard2} overlap1 {overlap1} overlap2 {overlap2} verdict {verdict}",
            MINERS[a].site.word()
        ));
    }

    // 6. The figures.
    for f in &flown {
        let Some(report) = &f.report else {
            println!(
                "mining_cohort: bot {} site {} order {} launched_at {} (no report)",
                f.miner.account,
                f.miner.site.word(),
                f.order,
                f.launched_at
            );
            continue;
        };
        let loop_min = match (
            report.beats.first().map(|s| s.start_unix_ms),
            report.milestone(Milestone::Sold).map(|m| m.at_unix_ms),
        ) {
            (Some(from), Some(to)) => format!("{:.1}", to.saturating_sub(from) as f64 / 60_000.0),
            _ => "?".to_owned(),
        };
        println!(
            "mining_cohort: bot {} site {} order {} launched_at {} loop_min {loop_min} lines {} bytes {} family {} mined {} t sold {} t net {} stake {} refused {} switched {} heard {} witness_rows {}",
            f.miner.account,
            f.miner.site.word(),
            f.order,
            f.launched_at,
            report.lines_sent,
            report.bytes_received,
            report.family.as_deref().unwrap_or("?"),
            report.mined_t,
            report.sold_t,
            report
                .receipts
                .sale
                .as_ref()
                .map_or("?", |sale| sale.net.as_str()),
            staked_block(report),
            report.stakes_refused,
            report.family_switched,
            report.heard.len(),
            report.witness.len()
        );
    }
    for line in &site_lines {
        println!("{line}");
    }
    println!(
        "mining_cohort: ready {ready_s} s, first launch to last sale {:.1} min, total {:.1} min",
        last_sale.saturating_sub(first_launch) as f64 / 60_000.0,
        started.elapsed().as_secs_f64() / 60.0
    );
    if !failures.is_empty() {
        for f in &flown {
            f.print();
        }
        panic!("the mining cohort failed:\n{}", failures.join("\n"));
    }
    god.close().await;
    drop(rig);
}

/// The actual standalone executable plays the entire detachment loop over public SSH.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bot_hires_launches_mines_recovers_and_sells_with_a_detachment() {
    tokio::time::timeout(Duration::from_secs(22 * 60), async {
        let miner = Miner {
            account: "expedition",
            character: "Expedition",
            site: Site::Ahuna,
        };
        let rig = mine::boot(&[miner]).await;
        println!("detachment rig: {}", rig.dir.display());
        let config = arena::bot_config_agent(&rig, "expedition+Expedition", 1200, 250, 250, "");
        let flight = arena::launch(&rig, &config, "detachment", &["--mine-detachment", "ahuna"]);
        let mut god =
            arena::Client::login(&rig, arena::GOD_ACCOUNT, arena::GOD_PASSWORD, "God").await;
        // Only the server fixture changes the clock, after the captain enrols at rate 24.
        let started = std::time::Instant::now();
        loop {
            let log = std::fs::read_to_string(&flight.transcript).unwrap_or_default();
            if log.contains("+coop/refit prospector") {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(600),
                "bot never enrolled: {log}"
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        if let Some(rate) = mine::rate_lift(mine::LIFTED_RATE) {
            mine::lift_rate(&mut god, rate).await;
        }
        let landed = arena::land(flight, Duration::from_secs(20 * 60)).await;
        let report = &landed.report["mine"];
        assert_eq!(
            report["success"], true,
            "{}\n{}\n{}",
            landed.report_text, landed.stdout, landed.stderr
        );
        assert!(report["sold_t"].as_u64().is_some_and(|n| n > 0), "{report}");
        assert_eq!(
            report["plan"]["crew_days"], 2,
            "the planner includes Ilya and the captain before bunkering: {report}"
        );
        let evidence = &report["detachment"];
        for fact in [
            "chat_received",
            "wilco_received",
            "returned",
            "unseated",
            "closed",
        ] {
            assert_eq!(evidence[fact], true, "{fact}: {report}");
        }
        assert!(
            evidence["contract"]
                .as_str()
                .is_some_and(|s| s.contains("Contract recorded"))
        );
        assert_eq!(evidence["units"].as_array().map(Vec::len), Some(2));
        let sends: Vec<_> = landed
            .records
            .iter()
            .filter(|r| r["kind"] == "mine-send")
            .filter_map(|r| r["data"]["line"].as_str())
            .collect();
        assert!(
            sends
                .iter()
                .all(|l| mush_bot::mine::lane::detachment::in_vocabulary(l)),
            "{sends:?}"
        );
        assert!(
            !sends.contains(&"+tv/1 launch go"),
            "detachment must use real DT flights"
        );
        println!("detachment expedition sold {} tonnes", report["sold_t"]);
    })
    .await
    .expect("the detachment expedition finishes within 22 minutes");
}
