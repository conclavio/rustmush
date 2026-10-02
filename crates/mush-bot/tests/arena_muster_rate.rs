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
//! there for reading after (the rig's own directory is removed), and (cut
//! AO.3) its `KNOBS` line with a newline (`win-rate-muster.knobs`).
//! `MUSH_BOT_WIN_RATE_ENGAGEMENT` (`hold`, the default, or `orbit`) picks the
//! pilot's engagement as it does for the posed lane, and (cut AN.2; cut
//! AN.3) `MUSH_BOT_WIN_RATE_OFF` names the pilot's own knobs to fly off for
//! an ablation as it does there (cut AO.3: a knob shipped off is flown off
//! whether named or not).
//!
//! **What it prints**, in this order: the reading's `ENGAGEMENT` line; (cut
//! AN.2; cut AN.3) `KNOBS: opening {wait|search}, reapproach {on|off}, follow
//! {on|off}, lattice {on|off}` (cut AO.3: the value flown, the shipped
//! default less what the variable names); `WIN
//! RATE FROM THE MUSTER: {w} of 10 for red against one machine at 1 v 1 (…)`
//! and straight after it (cut AO.3) `WIN RATE INTERVAL: {w} of {n}, 95 per
//! cent Wilson {lo} to {hi} per cent` (`n` the matches counted);
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
//!
//! **The pool** (cut AO.3): ten matches cannot tell 7 of 10 from 5 of 10, so
//! a default is chosen on kept runs read together. The ignored
//! [`arena_win_rate_pooled`] reads the directories `MUSH_BOT_WIN_RATE_POOL`
//! names (a path list) into one figure, its interval and one reading, and
//! refuses runs flown with different knobs:
//!
//! ```text
//! MUSH_BOT_WIN_RATE_POOL="d1;d2;d3" cargo test -p mush-bot --test arena_muster_rate arena_win_rate_pooled -- --ignored --nocapture
//! ```
//!
//! The offline [`the_interval_and_the_pool_read_their_lines`] (not ignored)
//! proves the interval, the pooled reading and the knobs check.
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
    // Cut AO.3: the knobs as flown, kept beside the transcript and report
    // (when the run is kept) so a pool can refuse runs flown differently.
    let knobs = arena_rig::knobs_line(&off);
    arena_rig::keep_knobs(arena_rig::POOL_STEM, &knobs);
    let flown = arena_rig::fly_lane(&arena_rig::LanePlan {
        name: arena_rig::POOL_STEM,
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
    println!("{knobs}");
    for line in arena_rig::win_rate_lines(&flown.outcome(), "WIN RATE FROM THE MUSTER", MATCHES) {
        println!("{line}");
    }
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

/// **The pool** (cut AO.3): kept runs read together into one figure and one
/// reading. `MUSH_BOT_WIN_RATE_POOL` names the kept directories (split as
/// `std::env::split_paths` splits a path list: `;` on Windows, `:`
/// elsewhere), each a `MUSH_BOT_WIN_RATE_KEEP` of a lane run, holding
/// `{stem}.jsonl`, `{stem}.json` when the bot left one and `{stem}.knobs`
/// when the lane wrote one; `MUSH_BOT_WIN_RATE_POOL_NAME` sets the stem
/// (`win-rate-muster` by default; `win-rate` pools posed runs). Unset, it
/// prints `POOL: MUSH_BOT_WIN_RATE_POOL is unset; nothing to pool.` and
/// passes. It flies nothing: it reads files.
///
/// It prints `POOLED: {k} runs`; for each run `RUN {i}: {w} of {n}, 95 per
/// cent Wilson {lo} to {hi} per cent` (or `RUN {i}: no report`) and `RUN {i}:
/// {its KNOBS line}` (or `RUN {i}: KNOBS: unknown`); `WIN RATE FROM THE
/// MUSTER, POOLED: …` over the summed results and its `WIN RATE INTERVAL`;
/// and the pooled reading from `FIRE GAP` on (the rig's `pool_lines`). It
/// panics, before printing anything, when a directory holds neither the
/// transcript nor the report, or when two runs' known `KNOBS` lines differ.
#[test]
#[ignore = "report-only, reads kept runs: set MUSH_BOT_WIN_RATE_POOL to the kept directories"]
fn arena_win_rate_pooled() {
    let Some(dirs) = std::env::var_os(arena_rig::POOL_ENV) else {
        println!("{}", arena_rig::POOL_UNSET);
        return;
    };
    let dirs: Vec<std::path::PathBuf> = std::env::split_paths(&dirs)
        .filter(|dir| !dir.as_os_str().is_empty())
        .collect();
    let stem = std::env::var(arena_rig::POOL_NAME_ENV)
        .ok()
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| arena_rig::POOL_STEM.to_owned());
    let runs = arena_rig::read_kept(&dirs, &stem);
    // Every check panics inside `pool_lines`, before a line is printed.
    for line in arena_rig::pool_lines(&runs, &stem) {
        println!("{line}");
    }
}

/// **The interval and the pool, offline** (cut AO.3): no rig, no bot, no
/// model. The eight pins of `wilson_95` and both forms of `interval_line`;
/// `reading_of` over two hand-written runs, each with a machine-gun `FIRE`
/// pair 1 500 ms apart (two gaps in all and never one across the runs, the
/// outcome summed through `Outcome::total`, the first times concatenated and
/// `PILOT RULES` summed); one run's `reading_of` equal to `reading`; and the
/// pool's knobs check through `read_kept` and `pool_lines`, the functions the
/// pool lane calls: equal lines pass, unequal lines panic naming both, a
/// missing knobs file reads `KNOBS: unknown`, and a directory with neither
/// the transcript nor the report panics naming it.
#[test]
fn the_interval_and_the_pool_read_their_lines() {
    use arena_rig::{KeptRun, Outcome, interval_line, pool_lines, read_kept, reading, reading_of};
    use serde_json::json;

    // (1) The pins.
    for (wins, n, want) in [
        (7, 10, (40, 89)),
        (0, 10, (0, 28)),
        (10, 10, (72, 100)),
        (15, 30, (33, 67)),
        (20, 30, (49, 81)),
        (14, 30, (30, 64)),
        (8, 10, (49, 94)),
        (2, 10, (6, 51)),
    ] {
        assert_eq!(arena_rig::wilson_95(wins, n), Some(want), "{wins} of {n}");
    }
    assert_eq!(arena_rig::wilson_95(0, 0), None);
    assert_eq!(
        interval_line(7, 10),
        "WIN RATE INTERVAL: 7 of 10, 95 per cent Wilson 40 to 89 per cent"
    );
    assert_eq!(interval_line(0, 0), "WIN RATE INTERVAL: no matches");

    // (2) Two runs, each with a machine-gun pair 1 500 ms apart at the same
    // clock readings (so a gap across the runs would be 0 ms without the
    // offset), a lattice sentence, a re-approach and a bench.
    let run = |at: u64| {
        [
            json!({"kind": "arena-send", "elapsed_ms": at,
                   "data": {"lane_ms": at, "line": "FIRE machine_gun", "purpose": "order"}}),
            json!({"kind": "observation", "elapsed_ms": at + 3, "data": "Firing machine_gun.\r\n"}),
            json!({"kind": "arena-send", "elapsed_ms": at + 1_500,
                   "data": {"lane_ms": at + 1_500, "line": "FIRE machine_gun", "purpose": "order"}}),
            json!({"kind": "observation", "elapsed_ms": at + 1_503, "data": "Firing machine_gun.\r\n"}),
            json!({"kind": "arena", "elapsed_ms": 1,
                   "data": "Opening the range on BA: 10 hits taken to 4 scored in 20 s; backing off bow on."}),
            json!({"kind": "arena", "elapsed_ms": 1,
                   "data": "The machine_gun is off the lattice clock for 30 s: refused twice."}),
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
    };
    let one = run(10_000);
    let two = run(10_000);
    let report_one = json!({"arena": {"wins": 2, "losses": 1, "draws": 0, "timeouts": 0, "other": 0,
    "matches": [
        {"match": 1, "first_contact_ms": 150_000, "first_hit_taken_ms": 163_000,
         "hits_taken_by_facet": {"hull front": 2}},
        {"match": 2, "first_contact_ms": 155_000},
        {"match": 3}
    ]}});
    let report_two = json!({"arena": {"wins": 1, "losses": 0, "draws": 0, "timeouts": 1, "other": 0,
    "matches": [
        {"match": 1, "first_contact_ms": 158_000, "first_hit_taken_ms": 170_000,
         "hits_taken_by_facet": {"hull front": 3, "turret front": 1}},
        {"match": 2}
    ]}});
    let pooled = reading_of(&[(&one, Some(&report_one)), (&two, Some(&report_two))]);
    let gaps = pooled.gaps.get("machine_gun").expect("machine-gun gaps");
    assert_eq!(
        (gaps.gaps, gaps.p10_ms, gaps.p50_ms, gaps.p90_ms),
        (2, 1_500, 1_500, 1_500),
        "two gaps, never one across the runs"
    );
    let shots = pooled.shots.get("machine_gun").expect("machine-gun shots");
    assert_eq!((shots.gaps, shots.p50_ms), (2, 1_500));
    assert_eq!(pooled.fire_lines.get("machine_gun"), Some(&4));
    assert_eq!(
        pooled.firsts,
        Some(vec![
            (Some(150_000), Some(163_000), None),
            (Some(155_000), None, None),
            (None, None, None),
            (Some(158_000), Some(170_000), None),
            (None, None, None),
        ]),
        "first times concatenated in run order"
    );
    assert_eq!(
        pooled.rules.line(),
        "PILOT RULES: opening 0, off the road 0, halt 0, reapproach 2, approach wait 0, follow 0, \
         return wait 0, lattice 0, inward 0, room holds 0, return room holds 0, gave up 0, benched 2"
    );
    let outcome = Outcome::from_report(&report_one).plus(Outcome::from_report(&report_two));
    assert_eq!(outcome.total(), 5);
    assert_eq!(
        arena_rig::win_rate_lines(&outcome, "WIN RATE FROM THE MUSTER, POOLED", 5),
        [
            "WIN RATE FROM THE MUSTER, POOLED: 3 of 5 for red against one machine at 1 v 1 (lost 1, drew 0, timed out 1, other 0)",
            "WIN RATE INTERVAL: 3 of 5, 95 per cent Wilson 23 to 88 per cent",
        ]
    );
    // Facets summed; with no report anywhere, none.
    let lines = pooled.lines("hold");
    assert!(
        lines.contains(&"HITS TAKEN BY FACET: hull front 5, turret front 1".to_owned()),
        "{lines:?}"
    );
    let bare = reading_of(&[(&one, None), (&two, None)]);
    assert_eq!((bare.facets, bare.firsts), (None, None));
    let half = reading_of(&[(&one, None), (&two, Some(&report_two))]);
    assert_eq!(half.firsts.map(|firsts| firsts.len()), Some(2));

    // (3) One run's `reading_of` is `reading`.
    assert_eq!(
        reading_of(&[(&one, Some(&report_one))]),
        reading(&one, Some(&report_one))
    );
    assert_eq!(
        reading_of(&[(&one, Some(&report_one))]).lines("hold"),
        reading(&one, Some(&report_one)).lines("hold")
    );

    // (4) The knobs check, through the functions the pool lane calls.
    let base = std::env::temp_dir().join(format!("mush-bot-pool-{}", std::process::id()));
    let keep = |name: &str, knobs: Option<&str>, report: Option<&serde_json::Value>| {
        let dir = base.join(name);
        std::fs::create_dir_all(&dir).expect("a kept directory");
        std::fs::write(dir.join("win-rate-muster.jsonl"), &one).expect("a transcript");
        if let Some(report) = report {
            std::fs::write(dir.join("win-rate-muster.json"), report.to_string()).expect("a report");
        }
        if let Some(knobs) = knobs {
            std::fs::write(dir.join("win-rate-muster.knobs"), format!("{knobs}\n"))
                .expect("a knobs file");
        }
        dir
    };
    let on = "KNOBS: opening wait, reapproach on, follow on, lattice on";
    let off = "KNOBS: opening wait, reapproach off, follow on, lattice on";
    let a = keep("a", Some(on), Some(&report_one));
    let b = keep("b", Some(on), Some(&report_two));
    let c = keep("c", None, None);
    let d = keep("d", Some(off), Some(&report_two));
    // Equal lines, and a run with no knobs file: pooled.
    let runs = read_kept(&[a.clone(), b.clone(), c.clone()], "win-rate-muster");
    let lines = pool_lines(&runs, "win-rate-muster");
    assert_eq!(
        lines.get(..9).map(<[String]>::to_vec),
        Some(
            [
                "POOLED: 3 runs",
                "RUN 1: 2 of 3, 95 per cent Wilson 21 to 94 per cent",
                &format!("RUN 1: {on}"),
                "RUN 2: 1 of 2, 95 per cent Wilson 9 to 91 per cent",
                &format!("RUN 2: {on}"),
                "RUN 3: no report",
                "RUN 3: KNOBS: unknown",
                "WIN RATE FROM THE MUSTER, POOLED: 3 of 5 for red against one machine at 1 v 1 (lost 1, drew 0, timed out 1, other 0)",
                "WIN RATE INTERVAL: 3 of 5, 95 per cent Wilson 23 to 88 per cent",
            ]
            .map(str::to_owned)
            .to_vec()
        ),
        "{lines:?}"
    );
    // The pooled reading follows from `FIRE GAP` on: three runs' gaps.
    assert_eq!(
        lines.get(9).map(String::as_str),
        Some("FIRE GAP machine_gun: p10 1500 / p50 1500 / p90 1500 ms over 3 gaps")
    );
    assert!(lines.iter().all(|line| !line.starts_with("ENGAGEMENT")));
    assert_eq!(
        lines.last().map(String::as_str),
        Some(
            "PILOT RULES: opening 0, off the road 0, halt 0, reapproach 3, approach wait 0, \
             follow 0, return wait 0, lattice 0, inward 0, room holds 0, return room holds 0, \
             gave up 0, benched 3"
        )
    );
    // The posed lane's stem reads its own head.
    assert_eq!(arena_rig::pool_head("win-rate"), "WIN RATE, POOLED");
    // Unequal lines: refused, naming both runs and both lines.
    let refused = std::panic::catch_unwind(|| {
        pool_lines(
            &read_kept(&[a.clone(), c.clone(), d.clone()], "win-rate-muster"),
            "win-rate-muster",
        )
    })
    .expect_err("refused");
    let said = refused
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert!(
        said.starts_with("POOL: the kept runs were flown with different knobs: ")
            && said.contains(&a.display().to_string())
            && said.contains(&d.display().to_string())
            && said.contains(on)
            && said.contains(off),
        "{said}"
    );
    // A directory with neither the transcript nor the report: refused,
    // naming it.
    let empty = base.join("empty");
    std::fs::create_dir_all(&empty).expect("an empty directory");
    let refused =
        std::panic::catch_unwind(|| read_kept(&[a.clone(), empty.clone()], "win-rate-muster"))
            .expect_err("refused");
    let said = refused
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        said,
        format!(
            "POOL: no win-rate-muster.jsonl and no win-rate-muster.json in {}",
            empty.display()
        )
    );
    // A run kept by hand holds only what it holds.
    let bare = KeptRun {
        dir: c.clone(),
        ..KeptRun::default()
    };
    assert_eq!(bare.knobs_line(), "KNOBS: unknown");
    let _ = std::fs::remove_dir_all(&base);
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
