//! **The bot's win rate against the machine** (cut AJ.4, 16c Q-AJ1): one bot
//! process, `--arena red --matches 10 --headless` with no model, against one
//! blue machine at 1 v 1 on the AJ.1 rig.
//!
//! **Report-only.** The owner's target is seven wins in ten; a lower figure is
//! information for a later machine cut (16c Q-AJ5), not a failure. The test
//! asserts only that ten matches finished, and prints
//! `WIN RATE: {w} of 10 for red against one machine at 1 v 1 (…)` and one line
//! a match. It is `#[ignore]` (ten matches take up to forty minutes) and its
//! own target, so exactly one CI job owns it: `bot-win-rate` in
//! `.github/workflows/ci.yml`, run by hand (`workflow_dispatch`), never on push
//! or the weekly schedule. Run it locally with
//! `cargo test -p mush-bot --test arena_win_rate -- --ignored --nocapture`.
//!
//! **Re-posed before every match.** The shipped muster is 2 430 m apart against
//! a 600 m sight, and from the second match on the bot's claim puts the tanks
//! back at muster, so without a pose the figure would measure the search. God,
//! standing in the Simulator Lobby, poses `RA` and `BA` 500 m apart on the road
//! before the first match and again on each `Match {n} is over (` line: the end
//! consumed and cleared the last ones, the new ones survive the bot's re-claim a
//! second later, and the bot's `lead_start_grace_ms` of 5 s keeps its
//! `SIM START` behind them. A refused pose is printed and the lane carries on.
//!
//! Set `MUSH_BOT_WIN_RATE_KEEP` to a directory to keep the run's transcript and
//! report there for reading after (the rig's own directory is removed).
use std::time::Duration;

#[path = "support/arena.rs"]
mod arena_rig;

/// The matches the lane flies.
const MATCHES: usize = 10;
/// Each match's length before the centre calls it a timeout.
const MAX_DURATION_MS: u32 = 180_000;
/// The longest God waits for one match's end, boarding and startup included.
const MATCH_WAIT: Duration = Duration::from_secs(300);

/// The two poses, 500 m apart on the east-west road, facing each other.
const POSES: [&str; 2] = ["RA=100 97 90", "BA=150 97 270"];

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "report-only, about forty minutes: CI's manual bot-win-rate job runs it (16c Q-AJ1)"]
async fn arena_win_rate_over_ten_matches() {
    tokio::time::timeout(Duration::from_secs(40 * 60), fly_ten())
        .await
        .expect("ten matches inside 40 minutes");
}

async fn fly_ten() {
    let mut cfg = arena_rig::arena_config();
    cfg.match_.startup_ms = 2_000;
    cfg.match_.board_timeout_ms = 60_000;
    cfg.match_.max_duration_ms = MAX_DURATION_MS;
    let rig = arena_rig::boot(cfg, &[("pilot", "Pilot")]).await;

    let mut god =
        arena_rig::Client::login(&rig, arena_rig::GOD_ACCOUNT, arena_rig::GOD_PASSWORD, "God")
            .await;
    for spec in POSES {
        arena_rig::pose(&mut god, spec).await;
    }
    arena_rig::walk_to_lobby(&mut god).await;

    let config = arena_rig::bot_config_with(
        &rig,
        "pilot+Pilot",
        240,
        "\n[arena]\nlead_start_grace_ms = 5000\n",
    );
    let mut from = god.seen.len();
    let mut flight = arena_rig::launch(
        &rig,
        &config,
        "win-rate",
        &["--arena", "red", "--matches", "10"],
    );

    // Each match's end, then the poses for the next.
    for played in 1..=MATCHES {
        from = arena_rig::await_line(&mut god, from, " is over (", &mut flight, MATCH_WAIT).await;
        if played == MATCHES {
            break;
        }
        for spec in POSES {
            if let Err(answer) = arena_rig::try_pose(&mut god, spec).await {
                println!(
                    "the pose {spec} before match {} was refused: {}",
                    played + 1,
                    answer.trim()
                );
            }
        }
    }

    let landed = arena_rig::land(flight, Duration::from_secs(120)).await;
    if let Some(keep) = std::env::var_os("MUSH_BOT_WIN_RATE_KEEP") {
        let keep = std::path::PathBuf::from(keep);
        let _ = std::fs::create_dir_all(&keep);
        let _ = std::fs::write(keep.join("win-rate.json"), &landed.report_text);
        let _ = std::fs::write(
            keep.join("win-rate.jsonl"),
            landed
                .records
                .iter()
                .map(|record| format!("{record}\n"))
                .collect::<String>(),
        );
    }

    let arena = &landed.report["arena"];
    let count = |key: &str| arena[key].as_u64().unwrap_or(0);
    println!(
        "WIN RATE: {} of {MATCHES} for red against one machine at 1 v 1 (lost {}, drew {}, timed out {}, other {})",
        count("wins"),
        count("losses"),
        count("draws"),
        count("timeouts"),
        count("other")
    );
    let matches = landed.matches();
    for flown in matches {
        println!(
            "match {}: {}; {} shots, {} hits, {} kills, took {}; {} s",
            flown["match"],
            flown["result"].as_str().unwrap_or("?"),
            flown["shots"],
            flown["hits_scored"],
            flown["kills"],
            flown["hits_taken"],
            flown["duration_ms"].as_u64().unwrap_or(0) / 1_000
        );
    }
    assert_eq!(
        matches.len(),
        MATCHES,
        "ten matches finished: {}\nstderr:\n{}",
        landed.report_text,
        landed.stderr
    );
    god.close().await;
    drop(rig);
}
