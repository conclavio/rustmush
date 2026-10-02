//! **The bot's win rate from the muster**: one bot process, `--arena red
//! --matches 10 --headless` with no model, against one blue machine at 1 v 1
//! on the arena rig, **flying the shipped match**: no pose, both tanks at the
//! muster about 2.4 km apart, on a 600 s cap (`arena_rig::MUSTER_DURATION_MS`).
//!
//! **The figure of record.** The match a person plays starts at the muster, so
//! this lane's `WIN RATE FROM THE MUSTER` is the bot's figure from now on. The
//! posed lane (`arena_win_rate`, 500 m apart on the road, every match a duel
//! from its first second) stays as the duel benchmark; both fly through the
//! rig's `fly_lane`, this one with no poses, so nothing is typed between
//! matches: the bot's own claim puts the tanks back at muster from the second
//! match on, and the world is fresh for the first.
//!
//! **Report-only, and ignored.** It asserts only that ten matches finished. Ten
//! matches of up to 600 s take up to about two hours, so it is `#[ignore]` and
//! its own target, and exactly one CI job owns it: `bot-muster-rate` in
//! `.github/workflows/ci.yml`, run by hand (`workflow_dispatch`), never on push
//! or on the weekly schedule. Run it locally with
//!
//! ```text
//! cargo test -p mush-bot --test arena_muster_rate -- --ignored --nocapture
//! ```
//!
//! and set `MUSH_BOT_WIN_RATE_KEEP` to a directory to keep the run's
//! transcript (`win-rate-muster.jsonl`) and report (`win-rate-muster.json`)
//! there for reading after (the rig's own directory is removed).
//! `MUSH_BOT_WIN_RATE_ENGAGEMENT` (`hold`, the default, or `orbit`) picks the
//! pilot's engagement as it does for the posed lane, and (cut AN.2; cut
//! AN.3) `MUSH_BOT_WIN_RATE_OFF` turns the pilot's own knobs off for an
//! ablation as it does there.
//!
//! **What it prints**, in this order: the reading's `ENGAGEMENT` line; (cut
//! AN.2; cut AN.3) `KNOBS: opening {wait|search}, reapproach {on|off}, follow
//! {on|off}, lattice {on|off}`; `WIN
//! RATE FROM THE MUSTER: {w} of 10 for red against one machine at 1 v 1 (…)`;
//! one line a match with its three first times (the rig's
//! `muster_match_line`: `first contact`, `first hit taken` and `first hit
//! scored`, each whole seconds after that match's power online, or `none`);
//! the rest of the reading (`FIRE GAP`, `SHOT GAP`, `STILL CYCLING`, `HITS
//! TAKEN BY FACET`, `SEEN FIRST`, `OPENING`, `FRONT SHARE`, and cut AN.2's
//! `PILOT RULES`, with cut AN.3's follow, return wait and lattice counts and
//! cut AO.1's inward, room holds, return room holds and gave up, and cut
//! AO.2's benched);
//! then `NO REPORT`
//! and `EARLY END` when they apply. Nothing between the launch and the last
//! line panics. The offline [`the_muster_lane_reads_its_match_lines`] (not
//! ignored) proves the lines on hand-written matches.
use std::time::Duration;

#[path = "support/arena.rs"]
mod arena_rig;

/// The matches the lane flies.
const MATCHES: usize = 10;
/// The whole flight's deadline, from the launch: ten matches of up to 600 s
/// with boarding and startup are about 110 minutes, and the rest is room for
/// slow boards.
const DEADLINE: Duration = Duration::from_secs(140 * 60);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "report-only, up to about two hours: CI's manual bot-muster-rate job runs it"]
async fn arena_win_rate_from_the_muster() {
    let value =
        std::env::var_os(arena_rig::ENGAGEMENT_ENV).map(|v| v.to_string_lossy().into_owned());
    let flying = arena_rig::engagement(value.as_deref());
    // Cut AN.2: the pilot's knobs this flight turns off, refused before the
    // launch when the variable names anything else.
    let off_value =
        std::env::var_os(arena_rig::KNOBS_OFF_ENV).map(|v| v.to_string_lossy().into_owned());
    let off = arena_rig::knobs_off(off_value.as_deref());
    let extra = arena_rig::extra(flying, &off);
    let flown = arena_rig::fly_lane(&arena_rig::LanePlan {
        name: "win-rate-muster",
        matches: MATCHES,
        deadline: DEADLINE,
        extra: &extra,
        env: &[],
        poses: &[],
        max_duration_ms: arena_rig::MUSTER_DURATION_MS,
        max_seconds: arena_rig::MUSTER_MAX_SECONDS,
    })
    .await;

    // The reading is of what was kept; nothing in it panics.
    let reading = arena_rig::reading(
        &flown.partial.transcript_text,
        flown.partial.report.as_ref(),
    );
    let lines = reading.lines(flying);
    if let Some(head) = lines.first() {
        println!("{head}");
    }
    println!("{}", arena_rig::knobs_line(&off));
    println!(
        "{}",
        flown.outcome().line("WIN RATE FROM THE MUSTER", MATCHES)
    );
    for each in flown.flown() {
        println!("{}", arena_rig::muster_match_line(each));
    }
    for line in lines.iter().skip(1) {
        println!("{line}");
    }
    if flown.partial.report.is_none() {
        println!("{}", arena_rig::NO_REPORT);
    }
    if let Some(line) = flown.early_line() {
        println!("{line}");
    }
    assert_eq!(
        flown.flown().len(),
        MATCHES,
        "ten matches finished: {}\nstderr:\n{}",
        flown
            .partial
            .report_text
            .as_deref()
            .unwrap_or("there was no report"),
        flown.partial.stderr
    );
}

/// **The muster lane's lines, offline** (cut AN.1): no rig, no bot, no model.
/// The match line with all three first times and with none; the results line
/// under its own head; `SEEN FIRST` on hand-written matches (a contact with
/// no hit taken, a hit taken before the contact, a tie at the same
/// millisecond read as hit before contact, one with neither, and two with
/// known leads) and the lead's percentiles over eleven leads; `OPENING`'s
/// percentiles, and its two short forms; and the early end of a muster wait,
/// which names 780 s.
#[test]
fn the_muster_lane_reads_its_match_lines() {
    use arena_rig::{Ended, Outcome};
    use serde_json::json;

    // (1) One match line, with its three times and with none of them.
    assert_eq!(
        arena_rig::muster_match_line(&json!({
            "match": 3, "result": "red", "shots": 14, "hits_scored": 9, "kills": 1,
            "hits_taken": 4, "duration_ms": 312_400, "first_contact_ms": 95_300,
            "first_hit_taken_ms": 131_999, "first_hit_scored_ms": 120_000
        })),
        "match 3: red; 14 shots, 9 hits, 1 kills, took 4; 312 s; first contact 95 s, first hit taken 131 s, first hit scored 120 s"
    );
    assert_eq!(
        arena_rig::muster_match_line(&json!({
            "match": 4, "result": "timeout", "shots": 0, "hits_scored": 0, "kills": 0,
            "hits_taken": 0, "duration_ms": 600_000, "first_contact_ms": null
        })),
        "match 4: timeout; 0 shots, 0 hits, 0 kills, took 0; 600 s; first contact none, first hit taken none, first hit scored none"
    );

    // (2) The results line under the lane's head.
    let outcome = Outcome {
        won: 3,
        lost: 4,
        drew: 0,
        timed_out: 2,
        other: 1,
    };
    assert_eq!(
        outcome.line("WIN RATE FROM THE MUSTER", MATCHES),
        "WIN RATE FROM THE MUSTER: 3 of 10 for red against one machine at 1 v 1 (lost 4, drew 0, timed out 2, other 1)"
    );

    // (3) `SEEN FIRST` and `OPENING` on six hand-written matches.
    let report = json!({"arena": {"matches": [
        // A contact and no hit taken (one scored): contact before hit, shot
        // first.
        {"match": 1, "first_contact_ms": 20_000, "first_hit_scored_ms": 30_000},
        // A hit taken before the contact: hit before contact, shot at first.
        {"match": 2, "first_contact_ms": 40_000, "first_hit_taken_ms": 35_000},
        // A tie at the same millisecond: hit before contact; taken before
        // scored, shot at first.
        {"match": 3, "first_contact_ms": 50_000, "first_hit_taken_ms": 50_000,
         "first_hit_scored_ms": 52_000},
        // Neither: in neither of the last two counts either.
        {"match": 4, "first_contact_ms": null, "first_hit_taken_ms": null,
         "first_hit_scored_ms": null},
        // Two known leads, 15.5 s and 4.9 s.
        {"match": 5, "first_contact_ms": 10_000, "first_hit_taken_ms": 25_500,
         "first_hit_scored_ms": 20_000},
        {"match": 6, "first_contact_ms": 60_000, "first_hit_taken_ms": 64_900}
    ]}});
    let lines = arena_rig::reading("", Some(&report)).lines("hold");
    let line = |head: &str| {
        lines
            .iter()
            .find(|line| line.starts_with(head))
            .cloned()
            .unwrap_or_default()
    };
    // Leads 4 900 and 15 500: floor(q × 1) is index 0 for every q, so each
    // percentile is the shorter, 4 s.
    assert_eq!(
        line("SEEN FIRST:"),
        "SEEN FIRST: contact before hit 3, hit before contact 2, neither 1 of 6 matches; lead p10 4 / p50 4 / p90 4 s over 2; shot first 2, shot at first 3"
    );
    // Contacts 10, 20, 40, 50 and 60 s: indices 0, 2 and 3. Hits taken 25.5,
    // 35, 50 and 64.9 s: index 1.
    assert_eq!(
        line("OPENING:"),
        "OPENING: first contact p10 10 / p50 40 / p90 50 s after power online over 5 matches; first hit taken p50 35 s over 4"
    );
    assert_eq!(
        line("FRONT SHARE:"),
        "FRONT SHARE: 0 of 0 facet hits taken on a front facet"
    );

    // (4) The lead's percentiles over eleven leads, 1 s to 11 s (each
    // contact at 10 s, each first hit 1 000 × n ms and a little later):
    // indices 1, 5 and 9.
    let leads: Vec<serde_json::Value> = (1..=11u64)
        .map(|n| {
            json!({"match": n, "first_contact_ms": 10_000,
                   "first_hit_taken_ms": 10_000 + n * 1_000 + 400})
        })
        .collect();
    let report = json!({"arena": {"matches": leads}});
    let lines = arena_rig::reading("", Some(&report)).lines("hold");
    assert!(
        lines.contains(
            &"SEEN FIRST: contact before hit 11, hit before contact 0, neither 0 of 11 matches; lead p10 2 / p50 6 / p90 10 s over 11; shot first 0, shot at first 11"
                .to_owned()
        ),
        "{lines:?}"
    );

    // (5) `OPENING`'s short forms: no contact at all, and contacts with no
    // hit taken (and so no lead).
    let report = json!({"arena": {"matches": [{"match": 1}, {"match": 2}]}});
    let lines = arena_rig::reading("", Some(&report)).lines("hold");
    assert!(
        lines.contains(&"OPENING: no contact in 2 matches".to_owned()),
        "{lines:?}"
    );
    assert!(
        lines.contains(
            &"SEEN FIRST: contact before hit 0, hit before contact 0, neither 2 of 2 matches; lead none; shot first 0, shot at first 0"
                .to_owned()
        ),
        "{lines:?}"
    );
    let report = json!({"arena": {"matches": [
        {"match": 1, "first_contact_ms": 70_000},
        {"match": 2, "first_contact_ms": 90_000}
    ]}});
    let lines = arena_rig::reading("", Some(&report)).lines("hold");
    assert!(
        lines.contains(
            &"OPENING: first contact p10 70 / p50 70 / p90 70 s after power online over 2 matches; first hit taken none"
                .to_owned()
        ),
        "{lines:?}"
    );

    // (6) A muster wait names its own 780 s; the posed wait is 600 s; a wait
    // the deadline cut short is the deadline's.
    let wait = arena_rig::match_wait(arena_rig::MUSTER_DURATION_MS);
    assert_eq!(wait, Duration::from_secs(780));
    assert_eq!(
        arena_rig::match_wait(arena_rig::MAX_DURATION_MS),
        arena_rig::MATCH_WAIT
    );
    assert_eq!(
        arena_rig::early_why(Ended::Timeout, wait, DEADLINE, wait),
        "no match end inside 780 s"
    );
    assert_eq!(
        arena_rig::early_why(Ended::Timeout, Duration::from_secs(1), DEADLINE, wait),
        "the 140-minute deadline"
    );
}
