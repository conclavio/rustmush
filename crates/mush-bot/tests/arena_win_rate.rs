//! **The bot's win rate against the machine** (cut AJ.4, 16c Q-AJ1; re-measured
//! in cut AK.3): one bot process, `--arena red --matches 10 --headless` with no
//! model, against one blue machine at 1 v 1 on the AJ.1 rig.
//!
//! **Report-only.** The owner's target is seven wins in ten; a lower figure is
//! information for a later machine cut (16c Q-AJ5, Q-AK5), not a failure. The
//! test asserts only that ten matches finished, and prints
//! `WIN RATE: {w} of 10 for red against one machine at 1 v 1 (…)` and one line
//! a match. It is `#[ignore]` (ten matches take about seventy-five minutes) and
//! its own target, so exactly one CI job owns it: `bot-win-rate` in
//! `.github/workflows/ci.yml`, run by hand (`workflow_dispatch`), never on push
//! or the weekly schedule. Run it locally with
//! `cargo test -p mush-bot --test arena_win_rate -- --ignored --nocapture`.
//!
//! **420 s a match** (cut AK.3, 16c Q-AK3; `arena_rig::MAX_DURATION_MS`): on
//! AJ's 180 s cap five of ten matches timed out, every one of them one-sided,
//! so the cap cut off fights the pilot was winning. A timeout is its own
//! column, never a loss. `[agent] max_seconds` is 540 (it bounds each match),
//! God waits at most 600 s for one match's end, and the whole flight has one
//! deadline, [`DEADLINE`], measured from the launch: each wait's limit is the
//! lesser of the two, so running out ends the flight like any other early end.
//!
//! **Re-posed before every match.** The shipped muster is 2 430 m apart against
//! a 600 m sight, and from the second match on the bot's claim puts the tanks
//! back at muster, so without a pose the figure would measure the search. God,
//! standing in the Simulator Lobby, poses `RA` and `BA` 500 m apart on the road
//! before the first match and again on each `Match {n} is over (` line (the
//! rig's `fly_posed`, which the live commander lane flies too). **A refused
//! pose answers at once and flags its match by id**: the first match flown
//! whose id is greater than the `n` of the end the poses followed (the first
//! match flown, for a pose refused before the launch). Its line ends
//! ` (unposed)`, and `UNPOSED: {u} of {n} match(es) flew from muster.` follows
//! the match lines.
//!
//! **It always reports.** Nothing between the launch and the last line panics:
//! the transcript (raw) and the report are kept first, then the figure is
//! printed from the report, or — when the bot left none — from God's own
//! `Match {n} is over ({word})` lines, with `NO REPORT: the figure is from the
//! lobby's match-end lines.` under it; a flight that ended early says
//! `EARLY END: {the bot ended | no match end inside 600 s | the 100-minute
//! deadline} after {n} match(es).`; and only then is the count of finished
//! matches asserted. Those reads run only on the run that goes wrong, so the
//! offline `the_lane_reads_the_lobby_and_flags_a_match_by_its_id` (not
//! ignored) proves them on hand-written lobby text and a hand-written report.
//!
//! Set `MUSH_BOT_WIN_RATE_KEEP` to a directory to keep the run's transcript
//! (`win-rate.jsonl`) and report (`win-rate.json`) there for reading after (the
//! rig's own directory is removed).
use std::time::Duration;

#[path = "support/arena.rs"]
mod arena_rig;

/// The matches the lane flies.
const MATCHES: usize = 10;
/// The whole flight's deadline, from the launch: ten matches of 420 s with
/// boarding and startup are about 76 minutes, and the rest is room for slow
/// boards.
const DEADLINE: Duration = Duration::from_secs(100 * 60);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "report-only, about seventy-five minutes: CI's manual bot-win-rate job runs it (16c Q-AJ1, Q-AK3)"]
async fn arena_win_rate_over_ten_matches() {
    let posed = arena_rig::fly_posed(&arena_rig::PosedPlan {
        name: "win-rate",
        matches: MATCHES,
        deadline: DEADLINE,
        extra: "\n[arena]\nlead_start_grace_ms = 5000\n",
        env: &[],
    })
    .await;

    println!("{}", posed.outcome().line("WIN RATE", MATCHES));
    let unposed = posed.unposed();
    for flown in posed.flown() {
        let flagged = flown["match"]
            .as_u64()
            .is_some_and(|id| unposed.contains(&id));
        println!(
            "match {}: {}; {} shots, {} hits, {} kills, took {}; {} s{}",
            flown["match"],
            flown["result"].as_str().unwrap_or("?"),
            flown["shots"],
            flown["hits_scored"],
            flown["kills"],
            flown["hits_taken"],
            flown["duration_ms"].as_u64().unwrap_or(0) / 1_000,
            if flagged { " (unposed)" } else { "" }
        );
    }
    if let Some(line) = posed.unposed_line() {
        println!("{line}");
    }
    if posed.partial.report.is_none() {
        println!("{}", arena_rig::NO_REPORT);
    }
    if let Some(line) = posed.early_line() {
        println!("{line}");
    }
    assert_eq!(
        posed.flown().len(),
        MATCHES,
        "ten matches finished: {}\nstderr:\n{}",
        posed
            .partial
            .report_text
            .as_deref()
            .unwrap_or("there was no report"),
        posed.partial.stderr
    );
}

/// **The lane's reporting, offline** (cut AK.3): the paths a lane takes only on
/// the run that goes wrong — no report, a refused pose, an early end — are pure
/// reads of the lobby's text and the report, so they are proved here with no
/// rig, no bot and no model, rather than waiting for the one bad run.
#[test]
fn the_lane_reads_the_lobby_and_flags_a_match_by_its_id() {
    use std::collections::BTreeMap;

    use arena_rig::{Ended, Outcome, Partial, Posed};
    use serde_json::json;

    // (1) The lobby's match-end lines, by id: the centre's own sentence, with
    // its line breaks and a two-digit id; a sentence that names no match, and
    // one torn before its word closes, are not match ends.
    let lobby = "Match 1 is over (red). Its units are checked in.\r\n\
                 RA opens at cell (100, 97) heading 90 for the next match.\r\n\
                 The match is over (red), the board says.\r\n\
                 Match 12 is over (timeout). Its units are checked in.\r\n\
                 Match 3 is over (blue). Its units are checked in.\r\n\
                 Match 4 is over (draw). Its units are checked in.\r\n\
                 Match 5 is over (abort). Its units are checked in.\r\n\
                 Match 6 is over (";
    let ends = arena_rig::match_ends(lobby);
    let want: BTreeMap<u64, String> = [
        (1, "red"),
        (3, "blue"),
        (4, "draw"),
        (5, "abort"),
        (12, "timeout"),
    ]
    .into_iter()
    .map(|(id, word)| (id, word.to_owned()))
    .collect();
    assert_eq!(ends, want);
    let lobby_outcome = Outcome::from_lobby(&ends);
    assert_eq!(
        lobby_outcome,
        Outcome {
            won: 1,
            lost: 1,
            drew: 1,
            timed_out: 1,
            other: 1,
        }
    );
    assert_eq!(
        lobby_outcome.line("WIN RATE", MATCHES),
        "WIN RATE: 1 of 10 for red against one machine at 1 v 1 (lost 1, drew 1, timed out 1, other 1)"
    );

    // (2) A flight whose report holds matches 1, 2 and 5 (3 and 4 never flew
    // for this bot), with a pose refused before the launch and one refused
    // after match 3 ended: the flags go to match 1 and to match 5, the first
    // flown whose id is past 3 — by id, not by flying order (the third match
    // flown, or the fourth, would be a positional slip).
    let report = json!({"arena": {
        "wins": 2, "losses": 0, "draws": 0, "timeouts": 1, "other": 0,
        "matches": [{"match": 1}, {"match": 2}, {"match": 5}]
    }});
    let partial = |report: Option<serde_json::Value>| Partial {
        code: Some(0),
        stdout: String::new(),
        stderr: String::new(),
        transcript_text: String::new(),
        report_text: report.as_ref().map(ToString::to_string),
        report,
    };
    let posed = Posed {
        partial: partial(Some(report)),
        ends: ends.clone(),
        heard: 3,
        early: Some(arena_rig::early_why(
            Ended::Timeout,
            Duration::from_secs(90),
            DEADLINE,
        )),
        refused: vec![None, Some(3)],
    };
    assert_eq!(posed.unposed(), [1, 5].into_iter().collect());
    assert_eq!(
        posed.unposed_line().as_deref(),
        Some("UNPOSED: 2 of 3 match(es) flew from muster.")
    );
    assert_eq!(
        posed.early_line().as_deref(),
        Some("EARLY END: the 100-minute deadline after 3 match(es).")
    );
    // With a report, the figure is the report's, not the lobby's.
    assert_eq!(
        posed.outcome(),
        Outcome {
            won: 2,
            timed_out: 1,
            ..Outcome::default()
        }
    );

    // (3) With no report there is nothing to flag, and the figure is the
    // lobby's; a flight that was not refused and ended on time says neither.
    let bare = Posed {
        partial: partial(None),
        ends,
        heard: 10,
        early: None,
        refused: vec![Some(9)],
    };
    assert!(bare.flown().is_empty());
    assert!(bare.unposed().is_empty());
    assert_eq!(bare.unposed_line(), None);
    assert_eq!(bare.early_line(), None);
    assert_eq!(bare.outcome(), lobby_outcome);

    // (4) The three early ends, each by its own cause: a wait the deadline cut
    // short of the whole match wait is the deadline's; a full wait is the
    // match's; the bot's own end is the bot's whatever the limit.
    assert_eq!(
        arena_rig::early_why(Ended::Timeout, arena_rig::MATCH_WAIT, DEADLINE),
        "no match end inside 600 s"
    );
    assert_eq!(
        arena_rig::early_why(Ended::Timeout, Duration::from_secs(1), DEADLINE),
        "the 100-minute deadline"
    );
    assert_eq!(
        arena_rig::early_why(Ended::Bot, Duration::from_secs(1), DEADLINE),
        "the bot ended"
    );
}
