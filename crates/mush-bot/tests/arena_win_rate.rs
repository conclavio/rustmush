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
//! rig's `fly_lane`, which the live commander lane flies too). **A refused
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
//! deadline | the watching session closed} after {n} match(es).`; and only
//! then is the count of finished
//! matches asserted. Those reads run only on the run that goes wrong, so the
//! offline `the_lane_reads_the_lobby_and_flags_a_match_by_its_id` (not
//! ignored) proves them on hand-written lobby text and a hand-written report.
//!
//! Set `MUSH_BOT_WIN_RATE_KEEP` to a directory to keep the run's transcript
//! (`win-rate.jsonl`) and report (`win-rate.json`) there for reading after (the
//! rig's own directory is removed), and (cut AO.3) its `KNOBS` line with a
//! newline (`win-rate.knobs`); `arena_muster_rate`'s pool lane reads kept
//! posed runs together with `MUSH_BOT_WIN_RATE_POOL_NAME=win-rate`.
//!
//! **The interval** (cut AO.3): straight after the `WIN RATE` line it prints
//! `WIN RATE INTERVAL: {w} of {n}, 95 per cent Wilson {lo} to {hi} per cent`
//! (`n` the matches counted; `WIN RATE INTERVAL: no matches` with none):
//! ten matches cannot tell 7 of 10 from 5 of 10, so no difference is called
//! while two intervals overlap. The offline `the_lanes_print_the_interval`
//! proves both lanes' lines.
//!
//! **The duel benchmark** (cut AN.1): posed 500 m apart, every match here is a
//! duel from the first second. The match a person plays starts at the shipped
//! muster, and its figure of record is the sibling lane `arena_muster_rate`
//! (CI's `bot-muster-rate`), flown through the same rig (`fly_lane`).
//!
//! **The engagement** (cut AL.3): `MUSH_BOT_WIN_RATE_ENGAGEMENT` picks the
//! pilot's own engagement, written into the bot's config as `[arena]
//! engagement` (the rig's `extra`, cut AN.1): unset or `hold` flies the hold
//! (the default), `orbit` the orbit (the comparison run); anything else panics
//! before the launch with `MUSH_BOT_WIN_RATE_ENGAGEMENT is hold or orbit, not
//! {value}`.
//! No code is edited between the runs.
//!
//! **The knobs** (cut AN.2; cut AN.3): `MUSH_BOT_WIN_RATE_OFF` is a comma
//! list of `opening`, `reapproach`, `follow`, `lattice` or `all` (all four),
//! the pilot's own knobs to turn off for an ablation (written into the bot's
//! config as `[arena] opening = "search"`, `reapproach = false`,
//! `follow_broken = false` and `lattice = false`); anything else panics
//! before the launch with `MUSH_BOT_WIN_RATE_OFF names opening, reapproach,
//! follow, lattice or all, not {word}`. Both win-rate lanes print `KNOBS:
//! opening {wait|search}, reapproach {on|off}, follow {on|off}, lattice
//! {on|off}` straight after the `ENGAGEMENT` line. Cut AO.3: the variable
//! names knobs to fly off, and the line is the value flown: the shipped
//! default (`Settings::default()`) less what it names, so a knob shipped off
//! is flown off, and printed `off`, whether named or not (the offline
//! `the_knobs_line_is_the_flown_value`).
//!
//! **The lead's reading** (cut AL.3; the rig's `reading` over the kept
//! transcript and the report): first `ENGAGEMENT: {e}; flown {stance} {k}, …
//! of {n} picture records` (the stances the picture records flew), then the
//! `WIN RATE` line and the match lines as before, then `FIRE GAP {w}: …` (the
//! lead's count of the `FIRE` lines), `SHOT GAP {w}: …` (the shots that left),
//! `STILL CYCLING {w}: {k} of {n} FIRE lines`, `HITS TAKEN BY FACET: …` and
//! (cut AN.1) `SEEN FIRST: …`, `OPENING: …` and `FRONT SHARE: …`, (cut AN.2)
//! `PILOT RULES: opening {a}, off the road {b}, halt {c}, reapproach {d},
//! approach wait {e}`, (cut AN.3) `, follow {f}, return wait {g}, lattice
//! {h}`, (cut AO.1) `, inward {i}, room holds {k}, return room holds {r},
//! gave up {u}` and (cut AO.2) `, benched {n}` (what the pilot's own rules
//! did, counted from its `arena` sentences; `benched` counts the lattice
//! clock's benches), and only then `UNPOSED`, `NO REPORT` and `EARLY END`. The offline
//! `the_reading_lines_are_the_leads_count` (not ignored) proves the reading on
//! a hand-written transcript and report.
//!
//! **The mind lane** (17b cut M1b.5; doc 17 §22's M1b gate):
//! `the_bot_against_a_conditioned_t1_over_thirty_pooled_matches`, also
//! `#[ignore]` and report-only, flies fifteen matches as red and fifteen as
//! blue, posed and capped as above, each against one fill hull flown by
//! Standard Doctrine on Line — the shipped fill, a conditioned T1 since 17b
//! cut M4.3 (the lab knob `fill_controller` and the rig's
//! `arena_config_with_fill` are gone with it) — and prints
//! one line a match, then `MIND RATE: {w} of {n} for the bot against a
//! conditioned T1 (red …, blue …, draws …, timeouts …)` and `MIND RATE
//! INTERVAL: {w} of {n}, 95 per cent Wilson {lo} to {hi} per cent`.
//! `MUSH_BOT_MIND_RATE_MATCHES` flies another count a side and
//! `MUSH_BOT_MIND_RATE_PARALLEL=1` flies the two flights side by side; the
//! offline `the_mind_rate_lines_print_from_hand_written_figures` proves the
//! two lines.
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
    arena_rig::keep_knobs("win-rate", &knobs);
    let posed = arena_rig::fly_lane(&arena_rig::LanePlan {
        name: "win-rate",
        matches: MATCHES,
        deadline: DEADLINE,
        extra: &extra,
        env: &[],
        poses: &arena_rig::POSES,
        max_duration_ms: arena_rig::MAX_DURATION_MS,
        max_seconds: arena_rig::MAX_SECONDS,
    })
    .await;

    // The reading is of what was kept; nothing in it panics.
    let reading = arena_rig::reading(
        &posed.partial.transcript_text,
        posed.partial.report.as_ref(),
    );
    let lines = reading.lines(flying);
    if let Some(head) = lines.first() {
        println!("{head}");
    }
    println!("{knobs}");
    for line in arena_rig::win_rate_lines(&posed.outcome(), "WIN RATE", MATCHES) {
        println!("{line}");
    }
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
    for line in lines.iter().skip(1) {
        println!("{line}");
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
            arena_rig::MATCH_WAIT,
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

    // (4) The four early ends, each by its own cause: a wait the deadline cut
    // short of the whole match wait is the deadline's; a full wait is the
    // match's; the bot's own end is the bot's whatever the limit; and (cut
    // AM.3) God's own session closing is the watcher's, whatever the limit.
    assert_eq!(
        arena_rig::early_why(
            Ended::Timeout,
            arena_rig::MATCH_WAIT,
            DEADLINE,
            arena_rig::MATCH_WAIT
        ),
        "no match end inside 600 s"
    );
    assert_eq!(
        arena_rig::early_why(
            Ended::Timeout,
            Duration::from_secs(1),
            DEADLINE,
            arena_rig::MATCH_WAIT
        ),
        "the 100-minute deadline"
    );
    assert_eq!(
        arena_rig::early_why(
            Ended::Bot,
            Duration::from_secs(1),
            DEADLINE,
            arena_rig::MATCH_WAIT
        ),
        "the bot ended"
    );
    for limit in [Duration::from_secs(1), arena_rig::MATCH_WAIT] {
        assert_eq!(
            arena_rig::early_why(Ended::Closed, limit, DEADLINE, arena_rig::MATCH_WAIT),
            "the watching session closed"
        );
    }
    let closed = Posed {
        partial: partial(None),
        ends: BTreeMap::new(),
        heard: 4,
        early: Some(arena_rig::early_why(
            Ended::Closed,
            Duration::from_secs(90),
            DEADLINE,
            arena_rig::MATCH_WAIT,
        )),
        refused: Vec::new(),
    };
    assert_eq!(
        closed.early_line().as_deref(),
        Some("EARLY END: the watching session closed after 4 match(es).")
    );
}

/// **The lead's reading, offline** (cut AL.3): a hand-written transcript
/// fragment — picture records of three stances, `FIRE` sends of both guns
/// (one gap over the 20 s window, one refused line tried again), the
/// `Firing` acks and two `Still cycling` answers inside observation records
/// (with an echo of a `FIRE` beside them), and a torn last line — and a
/// hand-written report with two matches' facet maps give exactly the reading's
/// line shapes; a report whose facet maps are empty gives `none`, and no
/// report `no report`. Cut AN.1: the two matches carry their first times (one
/// whose contact came 12 s before its first hit taken, one hit before any
/// contact), read into `SEEN FIRST`, `OPENING` and `FRONT SHARE`. The
/// engagement knob (the rig's) reads `hold` and `orbit` and refuses anything
/// else, and the bot's extra config names it.
#[test]
fn the_reading_lines_are_the_leads_count() {
    use serde_json::json;

    let picture = |stance: &str| {
        json!({"kind": "arena-picture", "elapsed_ms": 1,
        "data": {"doctrine": {"stance": stance}}})
    };
    let send = |at: u64, line: &str| {
        json!({"kind": "arena-send", "elapsed_ms": at,
        "data": {"lane_ms": at, "line": line, "purpose": "order"}})
    };
    let seen = |at: u64, text: &str| {
        json!({"kind": "observation", "elapsed_ms": at,
        "data": text})
    };
    let mut records = vec![
        picture("hold"),
        picture("hunt"),
        picture("hold"),
        picture("orbit"),
        picture("hold"),
        // The cannon: 6 400 ms, a refused line 200 ms before its retry, then
        // a gap over the window (left out).
        send(1_000, "FIRE cannon"),
        seen(1_003, "FIRE cannon\r\nFiring cannon.\r\n"),
        send(7_400, "FIRE cannon"),
        seen(7_403, "FIRE cannon refused. Still cycling: 1 s to go.\r\n"),
        send(7_600, "FIRE cannon"),
        seen(7_603, "Firing cannon.\r\nHit on BA: turret.\r\n"),
        send(14_000, "FIRE cannon"),
        seen(14_003, "Firing cannon.\r\n"),
        send(40_000, "FIRE cannon"),
        seen(40_003, "Firing cannon.\r\n"),
        // The machine gun: 1 200, a refused line and its retry 250 ms on,
        // 1 050 and 1 200.
        send(1_100, "FIRE machine_gun"),
        seen(1_103, "Firing machine_gun.\r\n"),
        send(2_300, "FIRE machine_gun"),
        seen(
            2_303,
            "FIRE machine_gun refused. Still cycling: 1 s to go.\r\n",
        ),
        send(2_550, "FIRE machine_gun"),
        seen(2_553, "Firing machine_gun.\r\n"),
        send(3_600, "FIRE machine_gun"),
        seen(3_603, "Firing machine_gun.\r\n"),
        send(4_800, "FIRE machine_gun"),
        seen(4_803, "Firing machine_gun.\r\n"),
        send(4_900, "CONTACTS"),
    ]
    .into_iter()
    .map(|record| record.to_string())
    .collect::<Vec<_>>()
    .join("\n");
    // A torn last line: the transcript as a killed child leaves it.
    records.push_str("\n{\"kind\":\"arena-send\",\"data\":{\"lane_ms\":50000,\"line\":\"FIRE can");
    let report = json!({"arena": {"matches": [
        {"match": 1, "hits_taken_by_facet": {"hull left": 3, "hull front": 1},
         "first_contact_ms": 41_500, "first_hit_taken_ms": 53_900,
         "first_hit_scored_ms": 50_200},
        {"match": 2, "hits_taken_by_facet":
            {"hull left": 2, "turret rear": 1, "right_track right": 1},
         "first_contact_ms": 30_000, "first_hit_taken_ms": 29_000,
         "first_hit_scored_ms": null}
    ]}});
    let reading = arena_rig::reading(&records, Some(&report));
    assert_eq!(reading.pictures, 5);
    assert_eq!(
        reading.lines("hold"),
        [
            "ENGAGEMENT: hold; flown hold 3, hunt 1, orbit 1 of 5 picture records",
            "FIRE GAP cannon: p10 200 / p50 6400 / p90 6400 ms over 3 gaps",
            "FIRE GAP machine_gun: p10 250 / p50 1050 / p90 1200 ms over 4 gaps",
            "SHOT GAP cannon: p10 6400 / p50 6400 / p90 6400 ms over 2 gaps",
            "SHOT GAP machine_gun: p10 1050 / p50 1200 / p90 1200 ms over 3 gaps",
            "STILL CYCLING cannon: 1 of 5 FIRE lines",
            "STILL CYCLING machine_gun: 1 of 5 FIRE lines",
            "HITS TAKEN BY FACET: hull left 5, hull front 1, right_track right 1, turret rear 1",
            "SEEN FIRST: contact before hit 1, hit before contact 1, neither 0 of 2 matches; lead p10 12 / p50 12 / p90 12 s over 1; shot first 1, shot at first 1",
            "OPENING: first contact p10 30 / p50 30 / p90 30 s after power online over 2 matches; first hit taken p50 29 s over 2",
            "FRONT SHARE: 1 of 8 facet hits taken on a front facet",
            "PILOT RULES: opening 0, off the road 0, halt 0, reapproach 0, approach wait 0, follow 0, return wait 0, lattice 0, inward 0, room holds 0, return room holds 0, gave up 0, benched 0",
        ]
    );
    // Empty facet maps: none. No report: no report. The engagement is the
    // lane's word.
    let empty =
        json!({"arena": {"matches": [{"match": 1, "hits_taken_by_facet": {}}, {"match": 2}]}});
    let lines = arena_rig::reading(&records, Some(&empty)).lines("orbit");
    assert_eq!(
        lines.first().map(String::as_str),
        Some("ENGAGEMENT: orbit; flown hold 3, hunt 1, orbit 1 of 5 picture records")
    );
    assert_eq!(
        lines.get(7).map(String::as_str),
        Some("HITS TAKEN BY FACET: none")
    );
    assert_eq!(
        lines.get(10).map(String::as_str),
        Some("FRONT SHARE: 0 of 0 facet hits taken on a front facet")
    );
    let lines = arena_rig::reading(&records, None).lines("hold");
    assert_eq!(
        lines.get(7).map(String::as_str),
        Some("HITS TAKEN BY FACET: no report")
    );
    assert_eq!(
        lines.get(10).map(String::as_str),
        Some("FRONT SHARE: no report")
    );
    assert_eq!(
        lines.last().map(String::as_str),
        Some(
            "PILOT RULES: opening 0, off the road 0, halt 0, reapproach 0, approach wait 0, follow 0, return wait 0, lattice 0, inward 0, room holds 0, return room holds 0, gave up 0, benched 0"
        )
    );
    assert_eq!(lines.len(), 12);
    // Nothing read: one line for what was flown, one for the facets, and the
    // three of who saw whom first, the opening and the front.
    assert_eq!(
        arena_rig::reading("", None).lines("hold"),
        [
            "ENGAGEMENT: hold; flown nothing of 0 picture records",
            "HITS TAKEN BY FACET: no report",
            "SEEN FIRST: no report",
            "OPENING: no report",
            "FRONT SHARE: no report",
            "PILOT RULES: opening 0, off the road 0, halt 0, reapproach 0, approach wait 0, follow 0, return wait 0, lattice 0, inward 0, room holds 0, return room holds 0, gave up 0, benched 0",
        ]
    );

    // The knob, and the config it writes (the rig's, cut AN.1).
    use arena_rig::{engagement, extra};
    assert_eq!(engagement(None), "hold");
    assert_eq!(engagement(Some("hold")), "hold");
    assert_eq!(engagement(Some("orbit")), "orbit");
    let refused = std::panic::catch_unwind(|| engagement(Some("circle"))).expect_err("refused");
    assert_eq!(
        refused.downcast_ref::<String>().map(String::as_str),
        Some("MUSH_BOT_WIN_RATE_ENGAGEMENT is hold or orbit, not circle")
    );
    assert_eq!(
        extra("orbit", &std::collections::BTreeSet::new()),
        "\n[arena]\nlead_start_grace_ms = 5000\nengagement = \"orbit\"\n"
    );
}

/// **The knob variable, offline** (cut AN.2; cut AN.3): `MUSH_BOT_WIN_RATE_OFF`
/// reads nothing off when unset, the knobs it names, all four for `all`, and
/// refuses any other word with its sentence; the bot's extra config is byte
/// for byte what it was with nothing off and names only the knobs turned off;
/// the `KNOBS` line says them; and the reading's `PILOT RULES` counts a
/// hand-written transcript's `arena` sentences — two of each rule, and (cut
/// AO.2) two of the lattice clock's benches — and counts the lobby's `Out of
/// the tank; boarding again.` nowhere.
#[test]
fn the_knob_variable_reads_and_refuses() {
    use std::collections::BTreeSet;

    use arena_rig::{extra, knobs_line, knobs_off};
    use serde_json::json;

    assert!(knobs_off(None).is_empty());
    assert!(knobs_off(Some("")).is_empty());
    let both: BTreeSet<&str> = ["opening", "reapproach"].into_iter().collect();
    assert_eq!(knobs_off(Some("opening,reapproach")), both);
    // Cut AN.3: `all` is all four.
    let all: BTreeSet<&str> = ["opening", "reapproach", "follow", "lattice"]
        .into_iter()
        .collect();
    assert_eq!(knobs_off(Some("all")), all);
    assert_eq!(knobs_off(Some("opening, reapproach,follow,lattice")), all);
    assert_eq!(
        knobs_off(Some("opening")),
        ["opening"].into_iter().collect::<BTreeSet<_>>()
    );
    for knob in ["follow", "lattice"] {
        assert_eq!(
            knobs_off(Some(knob)),
            [knob].into_iter().collect::<BTreeSet<_>>()
        );
    }
    let refused = std::panic::catch_unwind(|| knobs_off(Some("halt"))).expect_err("refused");
    assert_eq!(
        refused.downcast_ref::<String>().map(String::as_str),
        Some("MUSH_BOT_WIN_RATE_OFF names opening, reapproach, follow, lattice or all, not halt")
    );
    assert!(std::panic::catch_unwind(|| knobs_off(Some("opening,follow_broken"))).is_err());
    // Nothing off: today's bytes.
    let none = BTreeSet::new();
    assert_eq!(
        extra("hold", &none),
        "\n[arena]\nlead_start_grace_ms = 5000\nengagement = \"hold\"\n"
    );
    assert_eq!(
        extra("hold", &knobs_off(Some("opening"))),
        "\n[arena]\nlead_start_grace_ms = 5000\nengagement = \"hold\"\nopening = \"search\"\n"
    );
    assert_eq!(
        extra("orbit", &both),
        "\n[arena]\nlead_start_grace_ms = 5000\nengagement = \"orbit\"\nopening = \
         \"search\"\nreapproach = false\n"
    );
    assert_eq!(
        extra("hold", &all),
        "\n[arena]\nlead_start_grace_ms = 5000\nengagement = \"hold\"\nopening = \
         \"search\"\nreapproach = false\nfollow_broken = false\nlattice = false\n"
    );
    assert_eq!(
        extra("hold", &knobs_off(Some("lattice"))),
        "\n[arena]\nlead_start_grace_ms = 5000\nengagement = \"hold\"\nlattice = false\n"
    );
    // The extra tables read as the pilot's settings.
    let read: mush_bot::arena::settings::Settings = toml::from_str(
        extra("hold", &all)
            .trim_start()
            .trim_start_matches("[arena]\n"),
    )
    .expect("the [arena] table");
    assert_eq!(read.knobs(), mush_bot::arena::pilot::Knobs::off());
    assert!(!read.lattice);
    // The `KNOBS` line, the value flown (cut AO.3: the re-approach ships
    // off, so it reads `off` whether named or not).
    assert_eq!(
        knobs_line(&none),
        "KNOBS: opening wait, reapproach off, follow on, lattice on"
    );
    assert_eq!(
        knobs_line(&knobs_off(Some("reapproach"))),
        "KNOBS: opening wait, reapproach off, follow on, lattice on"
    );
    assert_eq!(
        knobs_line(&both),
        "KNOBS: opening search, reapproach off, follow on, lattice on"
    );
    assert_eq!(
        knobs_line(&knobs_off(Some("follow,lattice"))),
        "KNOBS: opening wait, reapproach off, follow off, lattice off"
    );
    assert_eq!(
        knobs_line(&all),
        "KNOBS: opening search, reapproach off, follow off, lattice off"
    );
    // `PILOT RULES` over a hand-written transcript.
    let said = |text: &str| json!({"kind": "arena", "elapsed_ms": 1, "data": text}).to_string();
    let transcript = [
        "Opening: holding near our entry, facing 091° toward the other side's entry.",
        "Opening: holding near our entry, facing 090° toward the other side's entry.",
        "Opening: stepping off the road to the north (35 m).",
        "Opening: stepping off the road to the south (35 m).",
        "Firing from the halt at BA, 590 m off.",
        "Firing from the halt at BB, 600 m off.",
        "Opening the range on BA: 10 hits taken to 4 scored in 20 s; backing off bow on.",
        "Opening the range on BA: 9 hits taken to 2 scored in 20 s; backing off bow on.",
        "Out of BA's sight at 650 m from its last plot; waiting for it to come on.",
        "Out of BA's sight at 652 m from its last plot; waiting for it to come on.",
        "Following BA home: it is hurt and driving away; holding 350 m behind it.",
        "Following BB home: it is hurt and driving away; holding 350 m behind it.",
        "Waiting for BA to come back, 650 m from its last plot.",
        "Waiting for BB to come back, 655 m from its last plot.",
        "Firing on the arena's lattice: phase 5 ms, round trip 40 ms.",
        "Firing on the arena's lattice: phase 245 ms, round trip 38 ms.",
        "Out of the tank; boarding again.",
        "Not following BA into its entry zone; backing off out of its sight.",
        "No return from BA in 90 s; searching.",
        "The machine_gun is off the lattice clock for 30 s: refused twice.",
        "The cannon is off the lattice clock for 30 s: refused twice.",
        "Opening over: BA is in sight after 95 s.",
        "The halt is over: BA is at the band.",
        "Opening: moving in 235 m toward the other side's entry: 65 m of ground behind us, 290 m wanted.",
        "Opening: moving in 41 m toward the other side's entry: 259 m of ground behind us, 290 m wanted.",
        "No room behind: waiting 410 m from BA's last plot.",
        "No room behind: waiting for BB.",
        "No room behind for the return: waiting 520 m from BA's last plot.",
        "No room behind for the return: waiting for BB.",
        "Could not open the range on BA in 60 s; holding.",
        "Could not open the range on BB in 60 s; holding.",
        "Opening: no way in toward the other side's entry; 60 m of ground behind us.",
    ]
    .iter()
    .map(|text| said(text))
    .collect::<Vec<_>>()
    .join("\n");
    let reading = arena_rig::reading(&transcript, None);
    assert_eq!(
        reading.lines("hold").last().map(String::as_str),
        Some(
            "PILOT RULES: opening 2, off the road 2, halt 2, reapproach 2, approach wait 2, \
             follow 2, return wait 2, lattice 2, inward 2, room holds 2, return room holds 2, \
             gave up 2, benched 2"
        )
    );
    // No transcript: zeros.
    assert_eq!(
        arena_rig::reading("", None)
            .lines("hold")
            .last()
            .map(String::as_str),
        Some(
            "PILOT RULES: opening 0, off the road 0, halt 0, reapproach 0, approach wait 0, follow 0, return wait 0, lattice 0, inward 0, room holds 0, return room holds 0, gave up 0, benched 0"
        )
    );
}

/// **The lanes print the interval** (cut AO.3): for a hand-written report,
/// the two figure lines each lane prints, through the helpers the lanes call
/// (`Outcome::from_report`, then the rig's `win_rate_lines`, which is the
/// `WIN RATE` line and `interval_line` over `Outcome::total`): the posed
/// lane's under `WIN RATE`, the muster lane's under `WIN RATE FROM THE
/// MUSTER`, each with the same `WIN RATE INTERVAL` straight after it; and a
/// report of no matches reads `no matches`.
#[test]
fn the_lanes_print_the_interval() {
    use arena_rig::{Outcome, win_rate_lines};
    use serde_json::json;

    let report = json!({"arena": {"wins": 7, "losses": 2, "draws": 0, "timeouts": 1, "other": 0,
        "matches": []}});
    let outcome = Outcome::from_report(&report);
    assert_eq!(outcome.total(), 10);
    assert_eq!(
        win_rate_lines(&outcome, "WIN RATE", MATCHES),
        [
            "WIN RATE: 7 of 10 for red against one machine at 1 v 1 (lost 2, drew 0, timed out 1, other 0)",
            "WIN RATE INTERVAL: 7 of 10, 95 per cent Wilson 40 to 89 per cent",
        ]
    );
    assert_eq!(
        win_rate_lines(&outcome, "WIN RATE FROM THE MUSTER", MATCHES),
        [
            "WIN RATE FROM THE MUSTER: 7 of 10 for red against one machine at 1 v 1 (lost 2, drew 0, timed out 1, other 0)",
            "WIN RATE INTERVAL: 7 of 10, 95 per cent Wilson 40 to 89 per cent",
        ]
    );
    // A short flight is counted over what it flew, not over ten.
    let short = Outcome::from_report(&json!({"arena": {"wins": 2, "losses": 1}}));
    assert_eq!(
        win_rate_lines(&short, "WIN RATE", MATCHES)[1],
        "WIN RATE INTERVAL: 2 of 3, 95 per cent Wilson 21 to 94 per cent"
    );
    let none = Outcome::from_report(&json!({"arena": {}}));
    assert_eq!(
        win_rate_lines(&none, "WIN RATE", MATCHES)[1],
        "WIN RATE INTERVAL: no matches"
    );
}

/// **The `KNOBS` line is the value flown** (cut AO.3): through the helpers
/// that take the defaults (`knobs_line_from`, `extra_from`), a `Settings`
/// whose `reapproach` is off reads `reapproach off` with nothing named, and
/// the extra config writes nothing for it (the bot's own default flies it
/// off); named, it is accepted and written off as before, which changes
/// nothing; a knob still on by default is written off when named. With every
/// default on both are byte for byte what the lanes printed and wrote
/// before, and the public `knobs_line` and `extra` are the helpers on
/// `Settings::default()`.
#[test]
fn the_knobs_line_is_the_flown_value() {
    use std::collections::BTreeSet;

    use arena_rig::{extra, extra_from, knobs_line, knobs_line_from, knobs_off};
    use mush_bot::arena::settings::Settings;

    let shipped_off = Settings {
        reapproach: false,
        ..Settings::default()
    };
    let none = BTreeSet::new();
    let head = "\n[arena]\nlead_start_grace_ms = 5000\nengagement = \"hold\"\n";
    assert_eq!(
        knobs_line_from(&shipped_off, &none),
        "KNOBS: opening wait, reapproach off, follow on, lattice on"
    );
    assert_eq!(extra_from(&shipped_off, "hold", &none), head);
    // The bot reads that config with the knob off all the same.
    let read: Settings = toml::from_str(
        extra_from(&shipped_off, "hold", &none)
            .trim_start()
            .trim_start_matches("[arena]\n"),
    )
    .expect("the [arena] table");
    assert_eq!(read.reapproach, shipped_off.reapproach);
    // Named while already off: accepted, written off as any named knob is,
    // and nothing changes (the line and what the bot reads are the same).
    let reapproach = knobs_off(Some("reapproach"));
    assert_eq!(
        knobs_line_from(&shipped_off, &reapproach),
        "KNOBS: opening wait, reapproach off, follow on, lattice on"
    );
    assert_eq!(
        extra_from(&shipped_off, "hold", &reapproach),
        format!("{head}reapproach = false\n")
    );
    // A knob still on by default is written off when named.
    let lattice = knobs_off(Some("lattice"));
    assert_eq!(
        knobs_line_from(&shipped_off, &lattice),
        "KNOBS: opening wait, reapproach off, follow on, lattice off"
    );
    assert_eq!(
        extra_from(&shipped_off, "hold", &lattice),
        format!("{head}lattice = false\n")
    );
    // The extra config and the line agree with what the bot then reads.
    let read: Settings = toml::from_str(
        extra_from(&shipped_off, "hold", &lattice)
            .trim_start()
            .trim_start_matches("[arena]\n"),
    )
    .expect("the [arena] table");
    assert!(!read.lattice && !read.reapproach && read.follow_broken);
    // Every default on (as the bot shipped before the pool): the bytes the
    // lanes printed and wrote before.
    let all_on = Settings {
        opening: mush_bot::arena::settings::Opening::Wait,
        reapproach: true,
        follow_broken: true,
        lattice: true,
        ..Settings::default()
    };
    // All five of the old `KNOBS` pins, each named knob read off: a knob on
    // by default and named prints `off` (the line is never the default alone).
    for (named, line) in [
        (
            None,
            "KNOBS: opening wait, reapproach on, follow on, lattice on",
        ),
        (
            Some("reapproach"),
            "KNOBS: opening wait, reapproach off, follow on, lattice on",
        ),
        (
            Some("opening,reapproach"),
            "KNOBS: opening search, reapproach off, follow on, lattice on",
        ),
        (
            Some("follow,lattice"),
            "KNOBS: opening wait, reapproach on, follow off, lattice off",
        ),
        (
            Some("all"),
            "KNOBS: opening search, reapproach off, follow off, lattice off",
        ),
    ] {
        assert_eq!(
            knobs_line_from(&all_on, &knobs_off(named)),
            line,
            "{named:?}"
        );
    }
    assert_eq!(extra_from(&all_on, "hold", &none), head);
    assert_eq!(
        extra_from(&all_on, "hold", &knobs_off(Some("all"))),
        format!(
            "{head}opening = \"search\"\nreapproach = false\nfollow_broken = false\nlattice = \
             false\n"
        )
    );
    // The public functions are the helpers on the shipped defaults.
    let shipped = Settings::default();
    for named in [
        None,
        Some("opening"),
        Some("reapproach"),
        Some("follow,lattice"),
        Some("all"),
    ] {
        let off = knobs_off(named);
        assert_eq!(
            knobs_line_from(&shipped, &off),
            knobs_line(&off),
            "{named:?}"
        );
        assert_eq!(
            extra_from(&shipped, "hold", &off),
            extra("hold", &off),
            "{named:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 17b cut M1b.5: the bot against a conditioned T1, pooled over thirty matches
// ---------------------------------------------------------------------------

/// The matches each flight of the mind lane flies (17b cut M1b.5): fifteen
/// as red and fifteen as blue, thirty pooled.
const MIND_RATE_MATCHES: usize = 15;
/// `MUSH_BOT_MIND_RATE_MATCHES`: a count to fly a side in place of fifteen.
const MIND_RATE_MATCHES_ENV: &str = "MUSH_BOT_MIND_RATE_MATCHES";
/// `MUSH_BOT_MIND_RATE_PARALLEL=1`: fly the two flights side by side, each on
/// its own rig, instead of one after the other.
const MIND_RATE_PARALLEL_ENV: &str = "MUSH_BOT_MIND_RATE_PARALLEL";

/// **One flight of the mind lane** (17b cut M1b.5): [`arena_rig::fly_lane`]'s
/// steps — the rig booted with the AI fill and the plan's cap, God's poses
/// (`RA` and `BA` 500 m apart, [`arena_rig::POSES`]) before the first match
/// and after each `Match {n} is over (`, one deadline from the launch, the bot
/// landed and its transcript and report kept under `MUSH_BOT_WIN_RATE_KEEP` —
/// on [`arena_rig::arena_config`] and for `side` (17b cut M4.3: the shipped
/// fill IS the conditioned T1, so the lane keeps its two-sided shape on the
/// one config). The rig's `fly_lane` flies red on the same fill and is not
/// moved; this is its shape for either side.
async fn fly_mind_side(side: &'static str, matches: usize, extra: &str) -> arena_rig::Posed {
    let mut cfg = arena_rig::arena_config();
    cfg.match_.startup_ms = 2_000;
    cfg.match_.board_timeout_ms = 60_000;
    cfg.match_.max_duration_ms = arena_rig::MAX_DURATION_MS;
    let rig = arena_rig::boot(cfg, &[("pilot", "Pilot")]).await;
    let wait = arena_rig::match_wait(arena_rig::MAX_DURATION_MS);
    // Fifteen matches of 420 s with boarding and startup, and room.
    let deadline = Duration::from_secs(
        u64::try_from(matches)
            .unwrap_or(15)
            .saturating_mul(10 * 60)
            .max(10 * 60),
    );
    let name = format!("mind-rate-{side}");

    let mut god =
        arena_rig::Client::login(&rig, arena_rig::GOD_ACCOUNT, arena_rig::GOD_PASSWORD, "God")
            .await;
    let mut refused = Vec::new();
    for spec in arena_rig::POSES {
        if let Err(answer) = arena_rig::try_pose(&mut god, spec).await {
            println!(
                "{side}: the pose {spec} before the first match was refused: {}",
                answer.trim()
            );
            refused.push(None);
        }
    }
    arena_rig::walk_to_lobby(&mut god).await;

    let config = arena_rig::bot_config_with(&rig, "pilot+Pilot", arena_rig::MAX_SECONDS, extra);
    let launched = god.seen.len();
    let mut from = launched;
    let count = matches.to_string();
    let start = tokio::time::Instant::now();
    let mut flight = arena_rig::launch_env(
        &rig,
        &config,
        &name,
        &["--arena", side, "--matches", &count],
        &[],
    );
    let mut heard = 0;
    let mut early = None;
    while heard < matches {
        let left = deadline.saturating_sub(start.elapsed());
        let limit = left.min(wait);
        match arena_rig::try_await_line(&mut god, from, " is over (", &mut flight, limit).await {
            Ok(past) => {
                heard += 1;
                let ended = god
                    .seen
                    .get(..past - " is over (".len())
                    .and_then(|before| before.rfind("Match ").map(|at| (before, at)))
                    .and_then(|(before, at)| before.get(at + "Match ".len()..))
                    .and_then(|id| id.trim().parse::<u64>().ok());
                from = past;
                if heard == matches {
                    break;
                }
                for spec in arena_rig::POSES {
                    if let Err(answer) = arena_rig::try_pose(&mut god, spec).await {
                        println!(
                            "{side}: the pose {spec} after match {ended:?} was refused: {}",
                            answer.trim()
                        );
                        if let Some(n) = ended {
                            refused.push(Some(n));
                        }
                    }
                }
            }
            Err(why) => {
                early = Some(arena_rig::early_why(why, limit, deadline, wait));
                break;
            }
        }
    }
    let partial = arena_rig::try_land(flight, Duration::from_secs(120)).await;
    if let Some(keep) = std::env::var_os(arena_rig::KEEP_ENV) {
        let keep = std::path::PathBuf::from(keep);
        let _ = std::fs::create_dir_all(&keep);
        let _ = std::fs::write(keep.join(format!("{name}.jsonl")), &partial.transcript_text);
        if let Some(text) = &partial.report_text {
            let _ = std::fs::write(keep.join(format!("{name}.json")), text);
        }
    }
    let ends = arena_rig::match_ends(god.seen.get(launched..).unwrap_or(""));
    god.close().await;
    drop(rig);
    arena_rig::Posed {
        partial,
        ends,
        heard,
        early,
        refused,
    }
}

/// **A flight's results from the bot's own side** (17b cut M1b.5): the
/// report's own `wins`/`losses` (the bot's, whichever side it flew), or with
/// no report the lobby's match-end words read for `side` — `blue` a win for a
/// blue flight.
fn mind_outcome(posed: &arena_rig::Posed, side: &str) -> arena_rig::Outcome {
    let outcome = posed.outcome();
    if posed.partial.report.is_none() && side == "blue" {
        arena_rig::Outcome {
            won: outcome.lost,
            lost: outcome.won,
            ..outcome
        }
    } else {
        outcome
    }
}

/// **The mind lane's two report lines** (17b cut M1b.5): `MIND RATE: {w} of
/// {n} for the bot against a conditioned T1 (red {wr} of {nr}, blue {wb} of
/// {nb}, draws {d}, timeouts {t})` over both flights pooled, then `MIND RATE
/// INTERVAL: {w} of {n}, 95 per cent Wilson {lo} to {hi} per cent` through
/// the rig's [`arena_rig::wilson_95`] (`MIND RATE INTERVAL: no matches` with
/// none). `n` counts every match each flight finished, its draws, timeouts
/// and others included.
fn mind_rate_lines(red: &arena_rig::Outcome, blue: &arena_rig::Outcome) -> [String; 2] {
    let pooled = red.plus(*blue);
    let (w, n) = (pooled.won, pooled.total());
    let interval = match arena_rig::wilson_95(w, n) {
        Some((lo, hi)) => {
            format!("MIND RATE INTERVAL: {w} of {n}, 95 per cent Wilson {lo} to {hi} per cent")
        }
        None => "MIND RATE INTERVAL: no matches".to_owned(),
    };
    [
        format!(
            "MIND RATE: {w} of {n} for the bot against a conditioned T1 (red {} of {}, blue {} of \
             {}, draws {}, timeouts {})",
            red.won,
            red.total(),
            blue.won,
            blue.total(),
            pooled.drew,
            pooled.timed_out
        ),
        interval,
    ]
}

/// **The bot against a conditioned T1, pooled over thirty matches** (17b cut
/// M1b.5; doc 17 §22's M1b gate: *a 30-match pooled measure of `mush-bot`
/// against a conditioned T1 reported with Wilson intervals*).
///
/// Two flights of fifteen in this file's own shape — the rig, the poses 500 m
/// apart before every match, 420 s a match, the knobs line, the keep
/// directory — `--arena red --matches 15` and `--arena blue --matches 15`,
/// each against ONE fill hull flown by the shipped fill — Standard Doctrine
/// on a unit mind sealed at `[tacsim.minds].fill_class` since 17b cut M4.3,
/// arming its two slots at its think. Never a
/// second lane in the engine and never the bot's doctrine ported: the bot
/// reads its opponent off its SSH session as it reads any machine. The second
/// flight runs on a second rig, after the first (`MUSH_BOT_MIND_RATE_PARALLEL=1`
/// side by side); `MUSH_BOT_MIND_RATE_MATCHES` flies another count a side.
///
/// **Report only**: one line a match, then the two `MIND RATE` lines
/// ([`mind_rate_lines`]); the test asserts only that every match finished.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "report-only, about two to four hours: the M1b gate's pooled measure (17b cut M1b.5)"]
async fn the_bot_against_a_conditioned_t1_over_thirty_pooled_matches() {
    let matches = std::env::var(MIND_RATE_MATCHES_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(MIND_RATE_MATCHES);
    let parallel = std::env::var(MIND_RATE_PARALLEL_ENV).is_ok_and(|value| value.trim() == "1");
    let none = std::collections::BTreeSet::new();
    let extra = arena_rig::extra(arena_rig::engagement(None), &none);
    let knobs = arena_rig::knobs_line(&none);
    arena_rig::keep_knobs("mind-rate", &knobs);
    println!("{knobs}");
    let (red, blue) = if parallel {
        tokio::join!(
            fly_mind_side("red", matches, &extra),
            fly_mind_side("blue", matches, &extra)
        )
    } else {
        let red = fly_mind_side("red", matches, &extra).await;
        let blue = fly_mind_side("blue", matches, &extra).await;
        (red, blue)
    };
    for (side, posed) in [("red", &red), ("blue", &blue)] {
        let unposed = posed.unposed();
        for flown in posed.flown() {
            let flagged = flown["match"]
                .as_u64()
                .is_some_and(|id| unposed.contains(&id));
            println!(
                "{side} match {}: {}; {} shots, {} hits, {} kills, took {}; {} s{}",
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
    }
    for line in mind_rate_lines(&mind_outcome(&red, "red"), &mind_outcome(&blue, "blue")) {
        println!("{line}");
    }
    for (side, posed) in [("red", &red), ("blue", &blue)] {
        if let Some(line) = posed.unposed_line() {
            println!("{side}: {line}");
        }
        if posed.partial.report.is_none() {
            println!("{side}: {}", arena_rig::NO_REPORT);
        }
        if let Some(line) = posed.early_line() {
            println!("{side}: {line}");
        }
    }
    for (side, posed) in [("red", &red), ("blue", &blue)] {
        assert_eq!(
            posed.flown().len(),
            matches,
            "{side}: every match finished: {}\nstderr:\n{}",
            posed
                .partial
                .report_text
                .as_deref()
                .unwrap_or("there was no report"),
            posed.partial.stderr
        );
    }
}

/// **The mind lane's two lines, offline** (17b cut M1b.5): from hand-written
/// figures, the pooled `MIND RATE` line with each side's count, the draws and
/// the timeouts, and the `MIND RATE INTERVAL` through the rig's Wilson helper;
/// nothing flown reads `no matches`; and a blue flight with no report reads
/// the lobby's `blue` as its win.
#[test]
fn the_mind_rate_lines_print_from_hand_written_figures() {
    use arena_rig::Outcome;

    let red = Outcome {
        won: 6,
        lost: 6,
        drew: 1,
        timed_out: 2,
        other: 0,
    };
    let blue = Outcome {
        won: 9,
        lost: 4,
        drew: 0,
        timed_out: 1,
        other: 1,
    };
    assert_eq!(
        mind_rate_lines(&red, &blue),
        [
            "MIND RATE: 15 of 30 for the bot against a conditioned T1 (red 6 of 15, blue 9 of 15, \
             draws 1, timeouts 3)",
            "MIND RATE INTERVAL: 15 of 30, 95 per cent Wilson 33 to 67 per cent",
        ]
    );
    assert_eq!(
        mind_rate_lines(&Outcome::default(), &Outcome::default()),
        [
            "MIND RATE: 0 of 0 for the bot against a conditioned T1 (red 0 of 0, blue 0 of 0, \
             draws 0, timeouts 0)",
            "MIND RATE INTERVAL: no matches",
        ]
    );
    let ends: std::collections::BTreeMap<u64, String> = [(1, "blue"), (2, "red"), (3, "blue")]
        .into_iter()
        .map(|(id, word)| (id, word.to_owned()))
        .collect();
    let posed = arena_rig::Posed {
        partial: arena_rig::Partial {
            code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            transcript_text: String::new(),
            report_text: None,
            report: None,
        },
        ends,
        heard: 3,
        early: None,
        refused: Vec::new(),
    };
    assert_eq!(
        mind_outcome(&posed, "blue"),
        Outcome {
            won: 2,
            lost: 1,
            ..Outcome::default()
        }
    );
    assert_eq!(
        mind_outcome(&posed, "red"),
        Outcome {
            won: 1,
            lost: 2,
            ..Outcome::default()
        }
    );
}
