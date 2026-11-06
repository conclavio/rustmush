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
// Doc 40 cut FR.7: the round's story at both sizes, over tacsim's own two grounds.
#[path = "mining_cohort/fr7_round.rs"]
mod fr7_round;
#[path = "../../tacsim/tests/support/field/grounds_rig.rs"]
mod grounds_rig;
#[path = "mining_cohort/in11_matter.rs"]
mod in11_matter;
// Doc 41 cut RS.9d: story (a) whole at both sizes, the tender and the bay.
#[path = "mining_cohort/rs9_story_a.rs"]
mod rs9_story_a;
// Doc 41 cut RS.9c: story (b) whole at both sizes, three players.
#[path = "mining_cohort/rs9_story_b.rs"]
mod rs9_story_b;
// Doc 41 cut RS.9b: story (e) whole at both sizes, the bot the buyer.
#[path = "mining_cohort/rs9_story_e.rs"]
mod rs9_story_e;
/// The one name `grounds_rig` reads from tacsim's `driving_rig`: Ahuna Flank's port entry.
mod driving_rig {
    pub(crate) const AHUNA: &str = "ahuna_port";
}
/// The one name `grounds_rig` reads from tacsim's `field_raiders_rig`: Ahuna's content raider.
mod field_raiders_rig {
    pub(crate) const SKULKS: &str = "tacsim.raider.ahuna_skulks";
}

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

/// **Doc 40 cut FR.0's smoke: the bot on Urvara Reach** (the first 512 site, its pockets numbered
/// to `P1024`). The canary's drive at `--mine urvara`: the bot's own lane kits up at Ceres, flies
/// to Urvara, launches and scans, over its own SSH session in line mode. Then, on the same session
/// and through the bot's own closed vocabularies ([`in_vocabulary`] and
/// `mush_bot::mine::lane::field::in_vocabulary`, both asserted before each line goes), it drives
/// rig 1 to `G1000`, the first four-digit pocket east of the pad (the pad's own pocket is `G993`),
/// reads the arrival with the bot's own address parser, and types `+field/void`, read by the bot's
/// own `void_answer` as nothing to void (no field ran). `#[ignore]`d, run by name:
///
/// `cargo test -j 8 -p mush-bot --test mining_cohort -- --ignored fr0_ --nocapture`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "doc 40 cut FR.0's Urvara smoke: a hop and a launch, minutes; run by name"]
async fn fr0_the_bot_drives_to_a_four_digit_pocket_on_urvara_and_types_void() {
    tokio::time::timeout(Duration::from_secs(30 * 60), urvara_smoke())
        .await
        .expect("the Urvara smoke finishes inside 30 minutes");
}

async fn urvara_smoke() {
    use mush_bot::mine::lane::field;
    let started = std::time::Instant::now();
    let miner = Miner {
        account: "urvara",
        character: "Urvara",
        site: Site::Urvara,
    };
    let rig = mine::boot(&[miner]).await;
    let mut god = arena::Client::login(&rig, arena::GOD_ACCOUNT, arena::GOD_PASSWORD, "God").await;
    let mut client =
        arena::Client::login(&rig, miner.account, arena::PILOT_PASSWORD, miner.character).await;
    let hull = hull_name("urvara+Urvara").expect("the hull name");
    let mut lane = Lane::new(
        Site::Urvara,
        Settings {
            hull,
            epoch_unix_ms: mine::unix_now_ms(),
        },
        miner.character.to_owned(),
    );
    // The canary's two phases around the rate lift.
    let lift = mine::rate_lift(mine::LIFTED_RATE);
    let drive_started = std::time::Instant::now();
    let bound = Duration::from_secs(28 * 60);
    let enrolled_now = |lane: &Lane| lane.step_line() == Some("+coop/refit prospector");
    mine::drive_lane_since(
        &mut client,
        &mut lane,
        drive_started,
        |lane| enrolled_now(lane) || lane.reached(Milestone::Scanned),
        bound,
    )
    .await;
    if let (Some(rate), true) = (lift, enrolled_now(&lane)) {
        mine::lift_rate(&mut god, rate).await;
    }
    let (wall, _) = mine::drive_lane_since(
        &mut client,
        &mut lane,
        drive_started,
        |lane| lane.reached(Milestone::Scanned),
        bound,
    )
    .await;
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
    println!("urvara: scanned on Urvara Reach after {wall} s");
    // The lane filed its sounding with the scan; a drive typed while the rig sounds is refused
    // ("Rig 1 is sounding"), so the sounding is heard out first. The lane is not ticked again, so
    // it never chooses a pocket of its own.
    let from = client
        .seen
        .rfind("Rig 1 starts its sounding")
        .unwrap_or(client.seen.len());
    assert!(
        client
            .watch(from, lines::SOUNDED, Duration::from_secs(5 * 60))
            .await,
        "the lane's sounding is heard:\n{}",
        client.seen.get(from..).unwrap_or("")
    );

    // The four-digit drive, in the bot's own words.
    let addr = "G1000";
    assert!(lines::is_addr(addr), "{addr} is an address");
    let drive = format!("+tv/1 drive {addr} go");
    assert!(in_vocabulary(&drive), "{drive}");
    let from = client.seen.len();
    client.send(&drive).await;
    let arrival = format!("{}{addr}", lines::ARRIVED);
    let arrived = client
        .watch(from, &arrival, Duration::from_secs(10 * 60))
        .await;
    let since = client.seen.get(from..).unwrap_or("").to_owned();
    assert!(
        arrived && lines::has_addr(&lines::normalise(&since), lines::ARRIVED, addr),
        "rig 1 arrives at {addr}; since the drive:\n{since}"
    );
    println!(
        "urvara: +{} s rig 1 arrives at {addr}",
        started.elapsed().as_secs()
    );

    // `+field/void`, in the captain's field vocabulary: nothing to void, no field ran.
    assert!(field::in_vocabulary("+field/void"));
    let (answered, text) = client
        .answer("+field/void", &[lines::NOTHING_TO_VOID, lines::VOIDED])
        .await;
    assert_eq!(
        (answered, lines::void_answer(&lines::normalise(&text))),
        (Some(0), Some(false)),
        "+field/void answers nothing to void:\n{text}"
    );
    println!(
        "urvara: +{} s +field/void answered: {}",
        started.elapsed().as_secs(),
        lines::NOTHING_TO_VOID
    );
    client.close().await;
    god.close().await;
    drop(rig);
}

/// **Doc 41 cut RS.0's smoke: the salvage loop's words over SSH**, on Urvara Reach (512) and on
/// Ahuna Flank (256). The bot's own lane kits up at Ceres, flies to the site and launches rig 1,
/// over its own SSH session in line mode; then, on the same session and through the captain's
/// closed vocabulary (`mush_bot::mine::lane::field::in_vocabulary`, asserted before each line
/// goes), it types story (b)'s `+salvage/claim`, `+salvage/contest <hull>=recovery` and `+tv/1
/// scrap`, and the bot's own `salvage::answer` reads each answer: no custody names the hull it
/// names (nothing was recovered) and no remains lie by the rig. `#[ignore]`d, run by name:
///
/// `cargo test -j 8 -p mush-bot --test mining_cohort -- --ignored rs0_ --nocapture`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "doc 41 cut RS.0's Urvara smoke: a hop and a launch, minutes; run by name"]
async fn rs0_the_bot_types_the_salvage_words_on_urvara() {
    tokio::time::timeout(
        Duration::from_secs(30 * 60),
        salvage_smoke(Miner {
            account: "rsurvara",
            character: "Rsurvara",
            site: Site::Urvara,
        }),
    )
    .await
    .expect("the Urvara salvage smoke finishes inside 30 minutes");
}

/// The same smoke on Ahuna Flank (256). `#[ignore]`d, run by name (`rs0_`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "doc 41 cut RS.0's Ahuna smoke: a hop and a launch, minutes; run by name"]
async fn rs0_the_bot_types_the_salvage_words_on_ahuna() {
    tokio::time::timeout(
        Duration::from_secs(30 * 60),
        salvage_smoke(Miner {
            account: "rsahuna",
            character: "Rsahuna",
            site: Site::Ahuna,
        }),
    )
    .await
    .expect("the Ahuna salvage smoke finishes inside 30 minutes");
}

async fn salvage_smoke(miner: Miner) {
    use mush_bot::mine::lane::{field, salvage};
    let started = std::time::Instant::now();
    let rig = mine::boot(&[miner]).await;
    let mut god = arena::Client::login(&rig, arena::GOD_ACCOUNT, arena::GOD_PASSWORD, "God").await;
    let mut client =
        arena::Client::login(&rig, miner.account, arena::PILOT_PASSWORD, miner.character).await;
    let hull = hull_name(&format!("{}+{}", miner.account, miner.character)).expect("the hull name");
    let mut lane = Lane::new(
        miner.site,
        Settings {
            hull: hull.clone(),
            epoch_unix_ms: mine::unix_now_ms(),
        },
        miner.character.to_owned(),
    );
    let lift = mine::rate_lift(mine::LIFTED_RATE);
    let drive_started = std::time::Instant::now();
    let bound = Duration::from_secs(28 * 60);
    let enrolled_now = |lane: &Lane| lane.step_line() == Some("+coop/refit prospector");
    mine::drive_lane_since(
        &mut client,
        &mut lane,
        drive_started,
        |lane| enrolled_now(lane) || lane.reached(Milestone::Launched),
        bound,
    )
    .await;
    if let (Some(rate), true) = (lift, enrolled_now(&lane)) {
        mine::lift_rate(&mut god, rate).await;
    }
    let (wall, _) = mine::drive_lane_since(
        &mut client,
        &mut lane,
        drive_started,
        |lane| lane.reached(Milestone::Launched),
        bound,
    )
    .await;
    let report = lane.report();
    let seen = client.seen.clone();
    let beat = lane.beat().as_str();
    let fail = |why: &str| canary_failed(&seen, &report, beat, why);
    if let Some(result) = lane.result() {
        fail(&format!("the lane ended: {}", result.outcome));
    }
    for milestone in [Milestone::ArrivedSite, Milestone::Launched] {
        if !lane.reached(milestone) {
            fail(&format!("{milestone:?} was not reached in {wall} s"));
        }
    }
    let site = miner.site.word();
    println!("{site}: rig 1 launched after {wall} s");

    // Story (b)'s three lines, in the bot's own words, each answer read by the bot's reader.
    let claim = format!("+salvage/claim {hull}");
    let contest = format!("+salvage/contest {hull}=recovery");
    let not_in_custody = tacsim::surface::claim::NOT_IN_CUSTODY.replace("{hull}", &hull);
    let no_remains = tacsim::surface::claim::NO_REMAINS.replace("{n}", "1");
    for (line, needle, want) in [
        (
            claim.as_str(),
            not_in_custody.as_str(),
            salvage::Answer::NotInCustody,
        ),
        (
            contest.as_str(),
            not_in_custody.as_str(),
            salvage::Answer::NotInCustody,
        ),
        (
            "+tv/1 scrap",
            no_remains.as_str(),
            salvage::Answer::NoRemains,
        ),
    ] {
        assert!(field::in_vocabulary(line), "{line}");
        assert!(salvage::in_vocabulary(line), "{line}");
        let (answered, text) = client.answer(line, &[needle]).await;
        let read = text.lines().find_map(salvage::answer);
        assert_eq!(
            (answered, read),
            (Some(0), Some(want)),
            "{line} answers {want:?}:
{text}"
        );
        println!(
            "{site}: +{} s {line} answered: {needle}",
            started.elapsed().as_secs()
        );
    }
    client.close().await;
    god.close().await;
    drop(rig);
}

/// **Doc 39 cut IN.5: the first production story, the bot over SSH at 256** (Ahuna Flank). The
/// mining lane plays its loop — kit-up, the hop, the mine, home, the hold sold into the Exchange's
/// ore bid — and then the bot types the production run's half, every line in its closed
/// vocabulary and every answer read by its own reader ([`mush_bot::mine::lane::works`]): at Ceres
/// it buys concentrate at the Exchange's ask and a tonne of carbon ore at Freeport Works', bunkers and sails to
/// Stickney Yards by Luna and Deimos (the corridor's outbound leg is interdicted at Kirkwood by the
/// SF track's picket, whose words the bot does not have), sells the concentrate into Kasei's bid, watches Kasei's alloy ask restock by the
/// refine's tonnes, buys five tonnes of alloys there, books `fittings` at the works, delivers the
/// alloys and the carbon from its hold, collects five tonnes of fittings once the arm has pooled
/// them, sails home and sells them into the Co-op's bid at 600. God lifts the world rate after
/// the enrolment (the canary's lift) and to 3,600 for the production half (`ic_slice_loop`'s
/// rate: the way out is some 27 IC days, the way home 11.6). `#[ignore]`d, run by name (`in5_`),
/// the dispatch / by-hand tier (doc 43 TS.2): about 30 minutes a site on conclav.dev.
///
/// `cargo test -j 8 -p mush-bot --test mining_cohort -- --ignored in5_ --nocapture --test-threads 1`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "doc 39 cut IN.5's production story at 256: a mining loop, two passages and the works; run by name"]
async fn in5_the_bot_mines_refines_and_sells_fittings_from_ahuna() {
    tokio::time::timeout(
        Duration::from_secs(60 * 60),
        production_story(Miner {
            account: "in5ahuna",
            character: "Anvil",
            site: Site::Ahuna,
        }),
    )
    .await
    .expect("the Ahuna production story finishes inside an hour");
}

/// The same story with the ground half at Urvara Reach (512). `#[ignore]`d, run by name (`in5_`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "doc 39 cut IN.5's production story at 512: a mining loop, two passages and the works; run by name"]
async fn in5_the_bot_mines_refines_and_sells_fittings_from_urvara() {
    tokio::time::timeout(
        Duration::from_secs(60 * 60),
        production_story(Miner {
            account: "in5urvara",
            character: "Bellows",
            site: Site::Urvara,
        }),
    )
    .await
    .expect("the Urvara production story finishes inside an hour");
}

/// **Doc 39 cut IN.11: the matter story, the bot over SSH at 256** (Ahuna Flank): the mining
/// loop, then ore to parts to a prospector, a part sold into a bid and another fitted, the census
/// unchanged across the whole story ([`in11_matter`]). `#[ignore]`d, run by name (`in11_`), the
/// dispatch / by-hand tier (doc 43 TS.2).
///
/// `cargo test -j 8 -p mush-bot --test mining_cohort -- --ignored in11_ --nocapture --test-threads 1`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "doc 39 cut IN.11's matter story at 256: a mining loop, three passages, the works, the yard and the bay; run by name"]
async fn in11_the_bot_mines_and_builds_a_prospector_from_ahuna() {
    tokio::time::timeout(
        Duration::from_secs(90 * 60),
        in11_matter::matter_story(Miner {
            account: "in11ahuna",
            character: "Assay",
            site: Site::Ahuna,
        }),
    )
    .await
    .expect("the Ahuna matter story finishes inside ninety minutes");
}

/// The same story with the ground half at Urvara Reach (512). `#[ignore]`d, run by name (`in11_`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "doc 39 cut IN.11's matter story at 512: a mining loop, three passages, the works, the yard and the bay; run by name"]
async fn in11_the_bot_mines_and_builds_a_prospector_from_urvara() {
    tokio::time::timeout(
        Duration::from_secs(90 * 60),
        in11_matter::matter_story(Miner {
            account: "in11urvara",
            character: "Crucible",
            site: Site::Urvara,
        }),
    )
    .await
    .expect("the Urvara matter story finishes inside ninety minutes");
}

/// **Doc 41 cut RS.9b: story (e) whole, the bot over SSH at 256** (Ahuna Flank): a raider's
/// wreck swept, restored and sold by Tycho Mutual, the bot the buyer, the matter census held
/// (`rs9_story_e`). `#[ignore]`d, run by name:
///
/// `cargo test -j 8 -p mush-bot --test mining_cohort -- --ignored rs9_ --nocapture --test-threads 1`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "doc 41 cut RS.9b's story (e) at 256: 56 IC days at rate 3600, the sweep, the restoration and the sale; run by name"]
async fn rs9_story_e_the_bot_buys_a_swept_wreck_from_ahuna() {
    tokio::time::timeout(
        Duration::from_secs(90 * 60),
        rs9_story_e::story_e(
            Miner {
                account: "rs9eahuna",
                character: "Rseahuna",
                site: Site::Ahuna,
            },
            &grounds_rig::AHUNA_FLANK,
        ),
    )
    .await
    .expect("the Ahuna story (e) finishes inside ninety minutes");
}

/// The same story at Urvara Reach (512). `#[ignore]`d, run by name (`rs9_`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "doc 41 cut RS.9b's story (e) at 512: 56 IC days at rate 3600, the sweep, the restoration and the sale; run by name"]
async fn rs9_story_e_the_bot_buys_a_swept_wreck_from_urvara() {
    tokio::time::timeout(
        Duration::from_secs(90 * 60),
        rs9_story_e::story_e(
            Miner {
                account: "rs9eurvara",
                character: "Rseurvara",
                site: Site::Urvara,
            },
            &grounds_rig::URVARA_REACH,
        ),
    )
    .await
    .expect("the Urvara story (e) finishes inside ninety minutes");
}

/// **Doc 41 cut RS.9d: story (a) whole, the bot over SSH at 256** (Ahuna Flank): the tender and
/// the bay on the site and under way (`rs9_story_a`). `#[ignore]`d, run by name (`rs9_`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "doc 41 cut RS.9d's story (a) at 256: the kit-up, Stickney by Luna and Deimos, the site; run by name"]
async fn rs9_story_a_the_tender_and_the_bay_work_on_ahuna() {
    tokio::time::timeout(
        Duration::from_secs(100 * 60),
        rs9_story_a::story_a(
            Miner {
                account: "rs9aahuna",
                character: "Raahuna",
                site: Site::Ahuna,
            },
            &grounds_rig::AHUNA_FLANK,
        ),
    )
    .await
    .expect("the Ahuna story (a) finishes inside 100 minutes");
}

/// The same story at Urvara Reach (512). `#[ignore]`d, run by name (`rs9_`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "doc 41 cut RS.9d's story (a) at 512: the kit-up, Stickney by Luna and Deimos, the site; run by name"]
async fn rs9_story_a_the_tender_and_the_bay_work_on_urvara() {
    tokio::time::timeout(
        Duration::from_secs(100 * 60),
        rs9_story_a::story_a(
            Miner {
                account: "rs9aurvara",
                character: "Raurvara",
                site: Site::Urvara,
            },
            &grounds_rig::URVARA_REACH,
        ),
    )
    .await
    .expect("the Urvara story (a) finishes inside 100 minutes");
}

/// **Doc 41 cut RS.9c: story (b) whole, the bot over SSH at 256** (Ahuna Flank): the contested
/// salvage, three players (`rs9_story_b`). `#[ignore]`d, run by name (`rs9_`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "doc 41 cut RS.9c's story (b) at 256: two kit-ups and hops, the claims and contests, the fine's 28 IC days; run by name"]
async fn rs9_story_b_the_claims_are_contested_on_ahuna() {
    tokio::time::timeout(
        Duration::from_secs(100 * 60),
        rs9_story_b::story_b(
            rs9_story_b::Cast {
                t: Miner {
                    account: "rs9bta",
                    character: "Tahuna",
                    site: Site::Ahuna,
                },
                o: Miner {
                    account: "rs9boa",
                    character: "Oahuna",
                    site: Site::Ahuna,
                },
                l: Miner {
                    account: "rs9bla",
                    character: "Lahuna",
                    site: Site::Ahuna,
                },
            },
            &grounds_rig::AHUNA_FLANK,
        ),
    )
    .await
    .expect("the Ahuna story (b) finishes inside 100 minutes");
}

/// The same story at Urvara Reach (512). `#[ignore]`d, run by name (`rs9_`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "doc 41 cut RS.9c's story (b) at 512: two kit-ups and hops, the claims and contests, the fine's 28 IC days; run by name"]
async fn rs9_story_b_the_claims_are_contested_on_urvara() {
    tokio::time::timeout(
        Duration::from_secs(100 * 60),
        rs9_story_b::story_b(
            rs9_story_b::Cast {
                t: Miner {
                    account: "rs9btu",
                    character: "Turvara",
                    site: Site::Urvara,
                },
                o: Miner {
                    account: "rs9bou",
                    character: "Ourvara",
                    site: Site::Urvara,
                },
                l: Miner {
                    account: "rs9blu",
                    character: "Lurvara",
                    site: Site::Urvara,
                },
            },
            &grounds_rig::URVARA_REACH,
        ),
    )
    .await
    .expect("the Urvara story (b) finishes inside 100 minutes");
}

/// The production half's world rate: an IC hour a real second.
const PRODUCTION_RATE: u32 = 3_600;

/// One line of the production run, held to the bot's vocabulary and answered by one of
/// `needles` (sent again two seconds later on a `retry` needle or a transient, at most sixty
/// times); the needle's index and the text since the send. The needles are matched on the text
/// with its whitespace normalised ([`lines::normalise`]): the game wraps a long sentence at the
/// session's width, anywhere between two words.
async fn typed(
    client: &mut arena::Client,
    line: &str,
    needles: &[&str],
    retry: &[&str],
) -> (usize, String) {
    use mush_bot::mine::lane::works;
    assert!(
        in_vocabulary(line) || works::in_vocabulary(line),
        "`{line}` is not in the bot's vocabulary"
    );
    let found = |text: &str, wanted: &[&str]| {
        let text = lines::normalise(text);
        wanted
            .iter()
            .position(|needle| text.contains(&lines::normalise(needle)))
    };
    for _ in 0..60 {
        let from = client.seen.len();
        if let Err(closed) = client.try_send(line).await {
            panic!("`{line}`: {closed}");
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let answer = loop {
            client.pump(Duration::from_millis(200)).await;
            let text = client.seen.get(from..).unwrap_or("").to_owned();
            if let Some(i) = found(&text, needles) {
                break Some((i, text));
            }
            if found(&text, retry).is_some() || found(&text, &arena::TRANSIENTS).is_some() {
                break None;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "`{line}` answered none of {needles:?} or {retry:?}:\n{text}"
            );
        };
        match answer {
            Some(answer) => return answer,
            None => tokio::time::sleep(Duration::from_secs(2)).await,
        }
    }
    panic!("`{line}` kept answering {retry:?}");
}

/// **The story's float** (a staff seed, outside the bot's vocabulary): God opens a system
/// source, the pilot reads its own account's id off `@account/list`, and God mints it 50,000.00cr.
async fn float(god: &mut arena::Client, client: &mut arena::Client) {
    let id = |text: &str, after: &str| {
        text.split(after)
            .nth(1)
            .and_then(|rest| {
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                digits.parse::<u64>().ok()
            })
            .unwrap_or_else(|| panic!("no account id after {after:?}:\n{text}"))
    };
    let opened = god
        .say(
            "@account/system in5_float=tacsim.credit",
            "Opened account #",
        )
        .await;
    let source = id(&opened, "Opened account #");
    let listed = client.say("@account/list", " tacsim.credit = ").await;
    let row = listed
        .lines()
        .find(|line| line.trim_start().starts_with('#') && line.contains(" tacsim.credit = "))
        .unwrap_or_else(|| panic!("no tacsim.credit account in the pilot's list:\n{listed}"));
    let pilot = id(row, "#");
    god.say(&format!("@econ/mint {source}={pilot} 5000000"), "Minted")
        .await;
}

/// `+market/list`'s rows, read by the bot's own parser.
async fn board_rows(client: &mut arena::Client) -> Vec<lines::Listing> {
    let from = client.seen.len();
    typed(client, "+market/list", &[" × "], &[]).await;
    client.idle(Duration::from_secs(2)).await;
    let text = client.seen.get(from..).unwrap_or("").to_owned();
    lines::listings(&text.lines().map(str::to_owned).collect::<Vec<_>>())
}

/// Quote and accept `qty` of listing `n`.
async fn buy(client: &mut arena::Client, n: u64, qty: u64) {
    let (_, text) = typed(
        client,
        &format!("+market/quote {n}={qty}"),
        &[lines::QUOTED],
        &[lines::QUOTE_STALE],
    )
    .await;
    let quote = lines::quote_number(&text).expect("the quote's number");
    typed(
        client,
        &format!("+market/accept {quote}"),
        &[lines::AGREED],
        &[],
    )
    .await;
}

/// Sell the whole of a good into bid `n`; the receipt.
async fn sell_all(client: &mut arena::Client, n: u64) -> lines::SaleRead {
    let (_, text) = typed(
        client,
        &format!("+market/sell {n}=all"),
        &[" when the goods are in"],
        &[lines::GOODS_WAIT, lines::GOODS_LEAVING, lines::SELL_BUSY],
    )
    .await;
    lines::sale_receipt(&text).expect("the sale's receipt")
}

/// **Bunker for the passage to `port`** (the mining lane's rule: the plan's burn and crew-days
/// with a margin), buying any shortfall at the port's reaction mass and provisions asks.
async fn bunker(client: &mut arena::Client, port: &str) {
    for _ in 0..4 {
        let from = client.seen.len();
        typed(client, &format!("+ship/plan {port}"), &["Stores "], &[]).await;
        client.idle(Duration::from_secs(1)).await;
        let text = client.seen.get(from..).unwrap_or("").to_owned();
        let raw: Vec<String> = text.lines().map(str::to_owned).collect();
        let plan = lines::plan_figures(&raw).expect("the plan's figures");
        let (want_tank, want_locker) = (plan.burn_t + 10, plan.crew_days + 5);
        if plan.tank_t >= want_tank && plan.locker >= want_locker {
            return;
        }
        let rows = board_rows(client).await;
        if plan.tank_t < want_tank {
            let fuel = lines::ask_for(&rows, "reaction mass").expect("a reaction mass ask");
            buy(client, fuel, want_tank - plan.tank_t).await;
        }
        if plan.locker < want_locker {
            let food = lines::ask_for(&rows, "provisions").expect("a provisions ask");
            buy(client, food, want_locker - plan.locker).await;
        }
        client.idle(Duration::from_secs(5)).await;
    }
    panic!("the bunkers for {port} never covered the plan");
}

/// Board at the gallery, file the passage to `port` once every bought good is aboard and every
/// sold one has left (the hand-overs settle on the market's sweep), and wait for the hull to dock
/// there; then ashore.
async fn sail(client: &mut arena::Client, hull: &str, port: &str) {
    typed(client, "+ship/board", &[lines::BOARDED], &[]).await;
    let from = client.seen.len();
    typed(
        client,
        &format!("+ship/undock {port}"),
        &[lines::CLEARANCE],
        &[lines::GOODS_WAIT, lines::GOODS_LEAVING],
    )
    .await;
    if !client
        .watch(from, &lines::docks(hull), Duration::from_secs(20 * 60))
        .await
    {
        panic!(
            "{hull} did not dock at {port} inside 20 minutes; the client's last 4 KiB:\n{}",
            mush_bot::tail(&client.seen, 4 * 1024)
        );
    }
    typed(client, "+ship/leave", &[lines::ASHORE], &[]).await;
}

#[allow(clippy::too_many_lines)] // the story's beats, in order
async fn production_story(miner: Miner) {
    use mush_bot::mine::lane::works;
    let started = std::time::Instant::now();
    let rig = mine::boot(&[miner]).await;
    let mut god = arena::Client::login(&rig, arena::GOD_ACCOUNT, arena::GOD_PASSWORD, "God").await;
    let mut client =
        arena::Client::login(&rig, miner.account, arena::PILOT_PASSWORD, miner.character).await;
    let hull = hull_name(&format!("{}+{}", miner.account, miner.character)).expect("the hull name");
    let site = miner.site.word();
    let stamp = |what: &str| println!("in5 {site}: +{} s {what}", started.elapsed().as_secs());

    // ---- The mining loop: the lane's own, to the hold sold at the Exchange ----
    let mut lane = Lane::new(
        miner.site,
        Settings {
            hull: hull.clone(),
            epoch_unix_ms: mine::unix_now_ms(),
        },
        miner.character.to_owned(),
    );
    let lift = mine::rate_lift(mine::LIFTED_RATE);
    let drive_started = std::time::Instant::now();
    let bound = Duration::from_secs(40 * 60);
    let enrolled_now = |lane: &Lane| lane.step_line() == Some("+coop/refit prospector");
    mine::drive_lane_since(
        &mut client,
        &mut lane,
        drive_started,
        |lane| enrolled_now(lane) || lane.reached(Milestone::Launched),
        bound,
    )
    .await;
    if let (Some(rate), true) = (lift, enrolled_now(&lane)) {
        mine::lift_rate(&mut god, rate).await;
    }
    let (wall, _) = mine::drive_lane_since(
        &mut client,
        &mut lane,
        drive_started,
        |lane| lane.reached(Milestone::Sold),
        bound,
    )
    .await;
    if !lane.reached(Milestone::Sold) {
        let report = lane.report();
        canary_failed(
            &client.seen,
            &report,
            lane.beat().as_str(),
            &format!("the hold was not sold in {wall} s"),
        );
    }
    stamp("the mining loop sold its hold at the Exchange");

    // ---- The production half, an IC hour a real second ----
    if lift != Some(PRODUCTION_RATE) {
        mine::lift_rate(&mut god, PRODUCTION_RATE).await;
    }
    // The seed (doc 43 rule 2.3 #4), as the in-process story's `@econ/mint`: the mining loop's
    // earnings do not bunker the way out by Luna and Deimos and the way home, so God mints the
    // pilot a working float from a system source of the story's own.
    float(&mut god, &mut client).await;
    stamp("God minted the pilot a 50,000.00cr float");
    // Ceres: the concentrate at the Exchange's ask, the carbon at Freeport Works', the bunkers,
    // the passage.
    let rows = board_rows(&mut client).await;
    let concentrate = works::ask_row(&rows, "metal concentrate").expect("the concentrate ask");
    let carbon = works::ask_row(&rows, "carbon ore").expect("Freeport Works' carbon ask");
    let (concentrate, carbon) = (concentrate.number, carbon.number);
    buy(&mut client, concentrate, 20).await;
    buy(&mut client, carbon, 1).await;
    bunker(&mut client, "luna").await;
    for (word, room) in [
        ("concourse", lines::ROOM_CONCOURSE),
        ("arrivals", lines::ROOM_ARRIVALS),
        ("lobby", lines::ROOM_LOBBY),
        ("gallery", lines::ROOM_GALLERY),
    ] {
        typed(&mut client, word, &[room], &[]).await;
    }
    stamp(
        "bought 20 t of concentrate and 1 t of carbon ore; sailing for Stickney by Luna and Deimos",
    );
    // The way out by Luna and Deimos: the Ceres–Stickney corridor's outbound leg is interdicted at
    // Kirkwood by the SF track's customs picket, whose words (heave-to, `+helm/submit`, `+tac/pdc
    // auto`, a jink) the bot does not have (doc 39 cut IN.5's note).
    sail(&mut client, &hull, "luna").await;
    stamp("docked at Aitken Highport");
    bunker(&mut client, "deimos").await;
    sail(&mut client, &hull, "deimos").await;
    stamp("docked at Voltaire Anchorage");
    bunker(&mut client, "stickney").await;
    sail(&mut client, &hull, "stickney").await;
    stamp("docked at Stickney Yards");

    // Stickney: the concentrate into Kasei's bid, and the refine's restock of its alloy ask.
    let rows = board_rows(&mut client).await;
    let alloys_before = works::ask_row(&rows, "structural alloy").and_then(works::row_quantity);
    let kasei = works::bid_row(&rows, "metal concentrate").expect("Kasei's concentrate bid");
    let sale = sell_all(&mut client, kasei.number).await;
    assert_eq!(sale.qty_t, 20, "{sale:?}");
    stamp(&format!("sold 20 t of concentrate to Kasei: {}", sale.net));
    let mut alloys = None;
    for _ in 0..60 {
        let rows = board_rows(&mut client).await;
        let ask = works::ask_row(&rows, "structural alloy");
        if let (Some(now), Some(before)) = (ask.and_then(works::row_quantity), alloys_before)
            && now >= before + 16
        {
            alloys = ask.map(|row| row.number);
            stamp(&format!("Kasei's alloy ask restocked {before} -> {now} t"));
            break;
        }
        client.idle(Duration::from_secs(5)).await;
    }
    let alloys = alloys.expect("Kasei's alloy ask restocks by the refine's sixteen tonnes");

    // The works: five tonnes of alloys bought, the fittings booked, the bill delivered.
    buy(&mut client, alloys, 5).await;
    let (_, quote) = typed(&mut client, "+works/quote fittings", &[works::FEE], &[]).await;
    let fee = works::quote_fee(&quote).expect("the quote's fee");
    stamp(&format!("+works/quote fittings: fee {fee}"));
    let (_, booking) = typed(&mut client, "+works/order fittings", &[works::BOOKED], &[]).await;
    let order = works::booked(&booking).expect("the order's number");
    let from = client.seen.len();
    for (good, units) in [("structural_alloys", 5), ("carbon_ore", 1)] {
        let (_, text) = typed(
            &mut client,
            &format!("+works/deliver {order}={good} {units}"),
            &[works::INTO_ESCROW],
            &["is still settling.", "That hold carries "],
        )
        .await;
        assert!(works::supplied(&text, order), "{text}");
    }
    let delivered = format!("Order {order}{}", works::DELIVERED);
    assert!(
        client
            .watch(from, &delivered, Duration::from_secs(5 * 60))
            .await,
        "order {order} is delivered at the works"
    );
    let (_, text) = typed(
        &mut client,
        &format!("+works/collect {order}"),
        &[works::INTO_HOLD],
        &[works::SETTLING],
    )
    .await;
    assert!(works::collected(&text, order), "{text}");
    stamp(&format!("order {order}: five tonnes of fittings collected"));

    // Home: the fittings into the Co-op's bid at 600.
    bunker(&mut client, "ceres").await;
    sail(&mut client, &hull, "ceres").await;
    stamp("docked at Ceres");
    let rows = board_rows(&mut client).await;
    let coop = works::bid_row(&rows, "yard fittings").expect("the Co-op's fittings bid");
    assert!(coop.text.contains("600.00cr"), "{}", coop.text);
    let sale = sell_all(&mut client, coop.number).await;
    assert_eq!(sale.qty_t, 5, "{sale:?}");
    assert_eq!(sale.gross, "3000.00cr", "{sale:?}");
    stamp(&format!(
        "sold five tonnes of fittings to the Co-op: gross {}, net {}",
        sale.gross, sale.net
    ));

    // The matter pin and the drift check: no readiness line is pending, missing or flagged but
    // two the story's own rate brings: the rate line itself (play runs at 24), and Occator's
    // house rig stopped under a claim after the month of IC time the lifted rate runs.
    let (index, report) = god
        .answer("@tacsim/port status", &[mine::READY, mine::NOT_READY])
        .await;
    assert!(
        index.is_some(),
        "the readiness report's last line:\n{report}"
    );
    let text = lines::normalise(&report);
    let flagged = ["[flagged]", "[pending]", "[missing]"]
        .iter()
        .flat_map(|tag| text.match_indices(tag).map(|(at, _)| at))
        .map(|at| {
            let entry = &text[..at];
            entry[entry.rfind("] ").map_or(0, |end| end + 2)..].trim()
        })
        .filter(|entry| {
            !entry.starts_with("world rate: ")
                && !(entry.contains("occator_port dispatch occator: ")
                    && entry.contains(" stopped: under claim."))
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert!(flagged.is_empty(), "{flagged:#?}\n{report}");
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

/// Both mining flags reject unsupported sites and competing control lanes before
/// connecting. Each admits a headless run, and the help presents the alternatives.
#[test]
fn the_mine_flag_refuses_what_it_cannot_play() {
    let session = "--mine cannot be combined with --script, --paused, --managed-floor, \
                   --replay, --studio, --harness or a campaign session";
    for flag in ["--mine", "--mine-detachment"] {
        for (args, sentence) in [
            (&[flag, "vesta"][..], mush_bot::mine::MINE_SITE_USAGE),
            (&[flag][..], mush_bot::mine::MINE_SITE_USAGE),
            (
                &[flag, "ahuna", "--arena", "red"][..],
                "--mine and --arena are two lanes; choose one",
            ),
            (
                &[flag, "ahuna", "--goal", "x"][..],
                "--mine plays now and --goal lets the model decide; choose one",
            ),
            (&[flag, "ahuna", "--paused"][..], session),
            // The runner's own refusal: neither lane admits REPL control.
            (
                &[flag, "ahuna"][..],
                "the mining pilot plays headless; add --headless",
            ),
        ] {
            let (code, stdout, stderr) = cli(args);
            assert_eq!(code, Some(1), "{args:?}: {stderr}");
            assert_eq!(stderr.trim(), format!("mush-bot: {sentence}"), "{args:?}");
            assert!(!stdout.contains("[bot/connect]"), "{args:?}: {stdout}");
        }
        // Reject both duplicate selections and mixing solo with detachment,
        // regardless of flag order, before any connection attempt.
        for second in ["--mine", "--mine-detachment"] {
            let args = [flag, "ahuna", second, "ahuna", "--headless"];
            let (code, stdout, stderr) = cli(&args);
            assert_eq!(code, Some(1), "{args:?}: {stderr}");
            assert_eq!(
                stderr.trim(),
                "mush-bot: choose --mine or --mine-detachment once",
                "{args:?}"
            );
            assert!(!stdout.contains("[bot/connect]"), "{args:?}: {stdout}");
        }
        // Admitted with no goal or model: port 1 refuses the SSH connection,
        // so the run ends on connection failure, never a mining-lane refusal.
        let (code, stdout, stderr) = cli(&[flag, "ahuna", "--headless"]);
        assert_eq!(code, Some(1), "{flag}: {stdout}\n{stderr}");
        assert!(
            stdout.contains("[bot/connect] pilot+Pilot@127.0.0.1:1"),
            "{flag} reached the connection:\n{stdout}\n{stderr}"
        );
        assert!(
            !stderr.contains("headless") && !stderr.contains("--mine"),
            "{flag} has no refusal: {stderr}"
        );
    }
    let (code, usage, _) = cli(&["--help"]);
    assert_eq!(code, Some(0));
    assert!(
        usage.contains("[--mine <site> | --mine-detachment <site>]"),
        "{usage}"
    );
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

/// The shipped fixture raider at Ahuna Flank (`content/field/raiders.json`, doc 32 cut CF.6b).
const SKULKS: &str = "tacsim.raider.ahuna_skulks";

/// The last lines the bot's session decoded, for a failure's message: each transcript record's
/// text, the last 60.
fn transcript_tail(flight: &arena::Flight) -> String {
    let log = std::fs::read_to_string(&flight.transcript).unwrap_or_default();
    let lines: Vec<String> = log
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|record| record["data"].as_str().map(str::to_owned))
        .collect();
    lines[lines.len().saturating_sub(60)..].concat()
}

/// The bot's transcript once it holds `needle`, polled every half second up to `limit`.
async fn transcript_holds(flight: &arena::Flight, needle: &str, limit: Duration) -> bool {
    let started = std::time::Instant::now();
    while started.elapsed() < limit {
        let log = std::fs::read_to_string(&flight.transcript).unwrap_or_default();
        if log.contains(needle) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

/// The bot's transcript once it holds every one of `needles`, polled every half second.
async fn all_hold(flights: &[(&arena::Flight, &str)], limit: Duration) -> bool {
    let started = std::time::Instant::now();
    while started.elapsed() < limit {
        if flights.iter().all(|(flight, needle)| {
            std::fs::read_to_string(&flight.transcript).is_ok_and(|log| log.contains(needle))
        }) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

/// **The captain and its escort in an IC field** (doc 32 G-4, Q-CF-47's bot run): two actual
/// `mush-bot` executables over public SSH on the production root, at Ahuna Flank.
///
/// - **The captain** (`--mine-field ahuna`) hires Rook Adeyemi under a combat contract for rig
///   2 and rides rig 1's cab, working its block on open ground.
/// - **The escort** (`--field-escort ahuna --escort-for Fieldcap`) leases Occator's armed rig
///   through CF.6c's player path (`+tv/lease escort go`, the loan), sails its own Dray-P,
///   boards the cab, launches to the ramp and waits there.
///
/// **Harness setup** (the fixture opponent's, never a bot's; doc 32 §16):
/// - the raider (CF.6b) is seeded and launched from its carrier at boot, so it waits on the site.
///   A staff world write is held behind every authoritative action a live world has in flight,
///   and these bots always have one;
/// - once the captain's run has cut its first cycle and the escort stands at its ramp, the
///   raider's NPC leader declares (`+field/mark <octant>=hostile`), typed as that NPC straight
///   into the world task (`Rig::act_as`), as tacsim's field stories type it, because no player
///   path reaches an NPC's own act;
/// - once the captain has withdrawn Rook and the escort is making for its ramp, staff withdraw
///   the raider up its own ramp.
///
/// **The escort** reads its ramp cell (`VIEW BRIEF`) and its party (`+field`), sets aside its
/// own party, its captain's (paged it, `Captain: party A.`) and every party its `LOCK` reads
/// `(FRIENDLY)` (its own Dray-P), takes the one left for the raider, pages the captain
/// `Raider: party D, contact DQ.`, marks that party, and fires its gun until a round lands, then drives to the ramp, takes CF.5's
/// exit (`+tv/1 recover go`), waits out the field and sails home.
///
/// **The captain** takes the raider from the escort's page (a field of several parties gives
/// line text no other way to tell them apart), marks it Hostile, orders Rook ENGAGE on it and
/// reads the WILCO, holds, orders WITHDRAW and reads the reply, and holds until the field
/// settles. Then it drives rig 1 home and closes the roster. It resumes the fill plan on rig 1,
/// launched again on its own (the field is closed, so its own party confuses nothing): one run at
/// a pocket of its own reading. Its plain lift meets the bay gate's refusal for rig 1, and it
/// lifts again with `leave=1`. Home, `+tv` has rig 1 out of the bay with its ore, and staff's
/// `@spatial` still finds it on the site's ground. Then it sells.
///
/// **A director raid instead** (doc 36 cut GC.11): with `FIELD_RAID=director` the harness seeds
/// and wears the raider as above and launches, declares and withdraws nothing.
/// `[tacsim.field].raid_director` is on, and the raid
/// director sends the content's raider once the site holds three flights and a rig holds ore (the
/// captain's first cycle, with its two rigs and the escort's out): it lands, its leader declares,
/// and it turns for home on its own triggers (Q-36-55). The bots run as before. `FIELD_IDLE_S`
/// sets `[tacsim.field].idle_demote_s` for the run.
///
/// No model: the run is deterministic.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines)] // one story, two lanes, in order
async fn a_captain_and_its_escort_fight_the_raider_in_an_ic_field() {
    let director = std::env::var("FIELD_RAID").is_ok_and(|raid| raid == "director");
    tokio::time::timeout(Duration::from_secs(35 * 60), async {
        let captain = Miner {
            account: "fieldcap",
            character: "Fieldcap",
            site: Site::Ahuna,
        };
        let escort = Miner {
            account: "fieldesc",
            character: "Fieldesc",
            site: Site::Ahuna,
        };
        // The raider waits on the site, seeded and launched from its carrier at boot, its hulls
        // worn as CF.6b's own gate wears one.
        // Doc 36 cut GC.11: under `FIELD_RAID=director` nobody seeds it; the director does.
        let (rig, staff) = mine::boot_with(
            &[captain, escort],
            |cfg| {
                cfg.tacsim.field.disengage_s = 30;
                cfg.tacsim.field.raid_director = director;
                // `FIELD_IDLE_S` shortens the idle bound (and so a director raid's idle trigger).
                if let Some(idle) = std::env::var("FIELD_IDLE_S")
                    .ok()
                    .and_then(|idle| idle.parse().ok())
                {
                    cfg.tacsim.field.idle_demote_s = idle;
                }
            },
            |engine| {
                // Under the director the raider is seeded and worn as in the staff run, and never
                // launched: the director lands, launches, declares and withdraws it.
                if director {
                    let mut said = vec![mine::staff_line(
                        engine,
                        &format!("@field/raiders {SKULKS}"),
                    )];
                    said.extend(mine::wear_raiders(engine));
                    return said;
                }
                let mut said = vec![
                    mine::staff_line(engine, &format!("@field/raiders {SKULKS}")),
                    mine::staff_line(engine, &format!("@field/raiders/launch {SKULKS}")),
                ];
                said.extend(mine::wear_raiders(engine));
                said
            },
        )
        .await;
        println!(
            "field rig: {}\nfield staff: {staff:?}; director raid: {director}",
            rig.dir.display()
        );
        let leader = (!director).then(|| {
            assert_eq!(staff.len(), 4, "two raider hulls worn: {staff:?}");
            assert!(staff[1].contains("hulls on the ground"), "{staff:?}");
            staff[0]
                .split_once("led by ")
                .and_then(|(_, rest)| rest.split_once("(#"))
                .and_then(|(_, rest)| rest.split_once(')'))
                .and_then(|(n, _)| n.parse::<u32>().ok())
                .map(mush_core::ObjId)
                .unwrap_or_else(|| panic!("the raider's leader: {staff:?}"))
        });
        // One config file a bot: the arena rig's one `bot/arena.toml` would be overwritten by the
        // second before the first child read it.
        let at = |name: &str| rig.dir.join(name).join("bot.toml");
        let config = arena::bot_config_agent_at(
            &rig,
            &at("captain"),
            "fieldcap+Fieldcap",
            1800,
            250,
            250,
            "",
        );
        let flight = arena::launch(
            &rig,
            &config,
            "field",
            &["--mine-field", "ahuna", "--escorted-by", "Fieldesc"],
        );
        let escort_config = arena::bot_config_agent_at(
            &rig,
            &at("escort"),
            "fieldesc+Fieldesc",
            1800,
            250,
            250,
            "",
        );
        let escort_flight = arena::launch(
            &rig,
            &escort_config,
            "escort",
            &["--field-escort", "ahuna", "--escort-for", "Fieldcap"],
        );
        let mut god =
            arena::Client::login(&rig, arena::GOD_ACCOUNT, arena::GOD_PASSWORD, "God").await;
        assert!(
            all_hold(
                &[
                    (&flight, "+coop/refit prospector"),
                    (&escort_flight, "+coop/refit prospector")
                ],
                Duration::from_secs(600)
            )
            .await,
            "both bots enrol; the captain's ends:\n{}\nthe escort's ends:\n{}",
            transcript_tail(&flight),
            transcript_tail(&escort_flight)
        );
        if let Some(rate) = mine::rate_lift(mine::LIFTED_RATE) {
            mine::lift_rate(&mut god, rate).await;
        }
        // The raider declares once the captain's run has cut its first cycle (so the hold has
        // ore to sell however soon the field stops it) and the escort has driven off its ramp.
        assert!(
            all_hold(
                &[
                    (&flight, "Rig 1, cycle 1 of"),
                    (&escort_flight, "Rig 1 arrives at ")
                ],
                Duration::from_secs(1200)
            )
            .await,
            "the captain's run and the escort's launch; the captain's ends:\n{}\nthe escort's \
             ends:\n{}",
            transcript_tail(&flight),
            transcript_tail(&escort_flight)
        );
        let _ = god.drain();
        let mut opened = false;
        if let Some(leader) = leader {
            // The leader declares on the first bearing that names a flight on the ground.
            for octant in ["W", "NW", "SW", "N", "S", "NE", "SE", "E"] {
                assert!(
                    rig.act_as(leader, &format!("+field/mark {octant}=hostile"))
                        .await
                );
                if transcript_holds(&flight, "is open on Ahuna Flank", Duration::from_secs(8)).await
                {
                    opened = true;
                    break;
                }
            }
        } else {
            // The director's raid lands and its leader declares on its own.
            opened =
                transcript_holds(&flight, "is open on Ahuna Flank", Duration::from_secs(900)).await;
        }
        assert!(
            opened,
            "the raider's mark opened no field; the captain's transcript ends:\n{}\nthe escort's \
             ends:\n{}",
            transcript_tail(&flight),
            transcript_tail(&escort_flight)
        );
        // The raider leaves once the captain has withdrawn Rook and the escort its own hull.
        assert!(
            all_hold(
                &[
                    (&flight, " 1 WITHDRAW"),
                    (&escort_flight, "+tv/1 recover go")
                ],
                Duration::from_secs(900)
            )
            .await,
            "both withdraw; the captain's ends:\n{}\nthe escort's ends:\n{}",
            transcript_tail(&flight),
            transcript_tail(&escort_flight)
        );
        // A director raid turns for home on its own triggers (Q-36-55).
        if !director {
            god.send(&format!("@field/raiders/withdraw {SKULKS}")).await;
            god.pump(Duration::from_secs(3)).await;
        }

        let landed = arena::land(flight, Duration::from_secs(25 * 60)).await;
        let landed_escort = arena::land(escort_flight, Duration::from_secs(10 * 60)).await;

        // -- The escort -------------------------------------------------------------------
        let report = &landed_escort.report["mine"];
        assert_eq!(
            report["success"], true,
            "{}\n{}\n{}",
            landed_escort.report_text, landed_escort.stdout, landed_escort.stderr
        );
        let escort = &report["escort"];
        assert!(
            escort["leased"]
                .as_str()
                .is_some_and(|line| line.contains("OWP-ES-")),
            "the armed rig, leased the player's way: {report}"
        );
        for fact in ["launched", "at_ramp", "in_bay", "settled"] {
            assert_eq!(escort[fact], true, "{fact}: {report}");
        }
        assert!(
            escort["hits"].as_u64().is_some_and(|n| n > 0),
            "lands fire: {report}"
        );
        assert!(
            escort["exit_asked"]
                .as_str()
                .is_some_and(|line| line.contains("makes for the ramp")),
            "withdraws at its ramp: {report}"
        );
        assert_eq!(
            escort["captain_party"], landed.report["mine"]["field"]["own_party"],
            "the escort took its captain's word: {report}"
        );
        assert!(
            escort["paged"]
                .as_str()
                .is_some_and(|line| line.starts_with("Raider: party ")),
            "{report}"
        );
        let escort_sends: Vec<_> = landed_escort
            .records
            .iter()
            .filter(|r| r["kind"] == "mine-send")
            .filter_map(|r| r["data"]["line"].as_str())
            .collect();
        assert!(
            escort_sends
                .iter()
                .all(|l| mush_bot::mine::lane::escort::in_vocabulary(l)),
            "{escort_sends:?}"
        );
        for verb in [
            "+tv/lease escort go",
            "+tv/1 launch go",
            "LOCK ",
            "FIRE ",
            "DRIVE ",
            "+tv/1 recover go",
        ] {
            assert!(
                escort_sends.iter().any(|l| l.starts_with(verb)),
                "{verb}: {escort_sends:?}"
            );
        }

        // -- The captain ------------------------------------------------------------------
        let report = &landed.report["mine"];
        assert_eq!(
            report["success"], true,
            "{}\n{}\n{}",
            landed.report_text, landed.stdout, landed.stderr
        );
        let field = &report["field"];
        for fact in ["engage_wilco", "settled", "resumed"] {
            assert_eq!(field[fact], true, "{fact}: {report}");
        }
        assert!(
            field["marked"]
                .as_str()
                .is_some_and(|ack| ack.contains("Hostile")),
            "{report}"
        );
        assert_eq!(
            field["raider_party"], escort["raider_party"],
            "the captain takes the escort's raider: {report}"
        );
        assert_ne!(field["own_party"], field["raider_party"], "{report}");
        // The fill plan resumed on rig 1, launched again on its own; the departure met the bay
        // gate's refusal and left it, and it is still on the site once the captain is home.
        assert!(
            field["refill_run"]
                .as_str()
                .is_some_and(|line| line.contains("Rig 1, cycle")),
            "{report}"
        );
        assert!(
            field["lift_refused"]
                .as_str()
                .is_some_and(|line| line.contains("Rig 1 is out on Ahuna Flank")),
            "{report}"
        );
        assert_eq!(field["left_behind"], "1", "{report}");
        assert!(
            field["left_home"].as_str().is_some_and(|line| {
                line.starts_with("Rig 1: ") && line.contains("not in the Fieldcap Lode's bay")
            }),
            "{report}"
        );
        assert_eq!(report["detachment"]["closed"], true, "{report}");
        // Staff's read of the world: rig 1 still stands on the site's ground after the lift.
        let rig1 = report["detachment"]["units"][0]
            .as_u64()
            .unwrap_or_else(|| panic!("rig 1's unit: {report}"));
        god.send(&format!("@spatial #{rig1}")).await;
        let placed = god
            .expect("effective domain", Duration::from_secs(30))
            .await;
        assert!(!placed.contains("unplaced"), "{placed}");
        let sends: Vec<_> = landed
            .records
            .iter()
            .filter(|r| r["kind"] == "mine-send")
            .filter_map(|r| r["data"]["line"].as_str())
            .collect();
        assert!(
            sends
                .iter()
                .all(|l| mush_bot::mine::lane::field::in_vocabulary(l)),
            "{sends:?}"
        );
        for verb in [
            "+field/mark ",
            " 1 ENGAGE ",
            " 1 WITHDRAW",
            "+sortie/activate ",
            "+tv/1 launch go",
            "+ship/lift ceres leave=1",
        ] {
            assert!(sends.iter().any(|l| l.contains(verb)), "{verb}: {sends:?}");
        }
        println!(
            "field captain: mark {}, raider {}, sold {} t; escort: {} shots, {} hits, exit {}",
            field["marked"],
            field["raider"],
            report["sold_t"],
            escort["shots"],
            escort["hits"],
            escort["exit_asked"]
        );
        god.close().await;
    })
    .await
    .expect("the captain and its escort finish within 35 minutes");
}
