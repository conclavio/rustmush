//! **The debrief over a real fight** (16b cut V.3; 16c N2, Q37).
//!
//! The in-source tests of `match_::report` pin every line against a tape written
//! by hand. This one folds a tape the **arena** wrote: the shipped proving
//! ground, two stock reflex tanks posed close through the shipped-scale rig
//! (`support::shipped_arena`, 16b §0.4), the loser checked out dry so that only
//! one side shoots, run to an elimination. No server: the fold is a pure function
//! of the event list, which is exactly what it is on the world side, where
//! `arena_bridge::on_chunk` hands it the same events a chunk at a time.
//!
//! What it proves is the projection's shape against the arena's own record,
//! rather than against a tape a test author imagined: every shot the winner
//! fired is one line of its report (or counted as dropped); every round on the
//! loser is one `hit on` line of the loser's; neither report names a unit its
//! own platform had not told apart; and no pilot line carries a contact id, an
//! object number, a draw, the seed, a range or a damage figure — while the staff
//! fold of the same events carries all of them.

mod support;

use std::collections::BTreeSet;

use mush_core::ObjId;
use smol_str::SmolStr;
use support::{Muster, booted_world, ledger_events, run_until, shipped_arena};
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::state::{Arena, EndReason, Team};
use tacsim::match_::HistoryRow;
use tacsim::match_::report::{
    Page, Reader, Report, ReportRules, Roster, fold, header_lines, stamp,
};

/// How long the fight may take, arena milliseconds.
const BOUND_MS: i64 = 900_000;

/// Blue's opening cell: on the ford road, fourteen cells east of Red Tank 1's
/// authored muster at `(6, 97)` — close enough that the fight is found at once.
const BLUE_AT: (u16, u16) = (20, 97);

/// The rules the rig's arena runs under (`SensorsConfig::default()`).
const RULES: ReportRules = ReportRules {
    designate_on_detection: false,
};

/// **The no-leak grep**: the first thing a pilot line may not carry.
fn leak(line: &str) -> Option<&'static str> {
    let contact = line
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| {
            word.len() == 16
                && word
                    .chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        });
    if contact {
        return Some("a contact id");
    }
    ["#", "seed", "hash", "draw", " m:", "damage"]
        .into_iter()
        .find(|word| line.contains(word))
}

/// The roster the world's record would hold: the arena's own callsigns, each
/// unit's side, no pilots (the fold never reads one).
fn roster_of(arena: &Arena) -> Roster {
    let mut roster = Roster::new();
    for (unit, designator) in arena.callsigns() {
        let team = arena.unit(unit).map(|held| held.team);
        roster.insert(unit, team, SmolStr::new(designator.to_string()), None);
    }
    roster
}

/// A result-document-shaped row for the header, written the way
/// `record::write_result` shapes one (its builder is private to the crate), and
/// read by V.2's own `HistoryRow::from_document`.
fn history_row(
    arena: &Arena,
    winner: ObjId,
    loser: ObjId,
    pilots: [(ObjId, ObjId); 2],
) -> HistoryRow {
    let callsign = |unit: ObjId| {
        arena
            .callsigns()
            .into_iter()
            .find(|(held, _)| *held == unit)
            .map(|(_, designator)| designator.to_string())
            .unwrap_or_default()
    };
    let team = |unit: ObjId| arena.unit(unit).map_or("red", |held| held.team.as_str());
    let document = serde_json::json!({
        "record": "tacsim.match",
        "record_version": 1,
        "match": 1,
        "arena": "tacsim.arena.proving_ground",
        "outcome": team(winner),
        "reason": "elimination",
        "teams": [
            { "team": team(loser), "units": [loser.0],
              "pilots": [{ "pilot": pilots[1].0.0, "unit": loser.0, "callsign": callsign(loser) }] },
            { "team": team(winner), "units": [winner.0],
              "pilots": [{ "pilot": pilots[0].0.0, "unit": winner.0, "callsign": callsign(winner) }] },
        ],
        "units": [
            { "unit": loser.0, "callsign": callsign(loser), "outcome": "destroyed", "shots": 0, "hits": 0 },
            { "unit": winner.0, "callsign": callsign(winner), "outcome": "intact", "shots": 1, "hits": 1 },
        ],
        "ledger": { "log": "match.1", "digest": "ab12cd34ef", "chunks": 3 },
        "seed": support::SEED,
        "started_ms": 1_790_258_000_000_i64,
        "ended_ms": 1_790_258_520_000_i64,
        "arena_ms": arena.now_ms,
        "reports": [loser.0, winner.0],
        // 16b cut I.1: the replay document rode the result's batch too.
        "replay": true,
    });
    let row = HistoryRow::from_document(&document).expect("a result-document-shaped row reads");
    assert!(
        row.replay,
        "16b cut I.1: `HistoryRow.replay` reads the flag"
    );
    // And a result written before the cut, with no flag at all, reads `false`.
    let mut before = document;
    before
        .as_object_mut()
        .expect("a result document is an object")
        .remove("replay");
    assert_eq!(
        HistoryRow::from_document(&before).map(|row| row.replay),
        Some(false),
        "an absent flag is a result written before replays existed"
    );
    row
}

/// The seconds a pilot line is stamped at (`  m:ss  …`), or `None`.
fn stamp_secs(line: &str) -> Option<i64> {
    let stamp = line.split_whitespace().next()?;
    let (minutes, seconds) = stamp.split_once(':')?;
    Some(minutes.parse::<i64>().ok()? * 60 + seconds.parse::<i64>().ok()?)
}

/// **Every callsign in `report` other than `own` is one `reader`'s platform had
/// classified on the tape.** A line other than a shot line is held to its own
/// stamp; a shot line is stamped at its `fired` and named when its outcome
/// landed, so it is held to the tape (the classification may fall in between).
fn names_only_what_was_told_apart(
    report: &Report,
    reader: ObjId,
    own: &str,
    roster: &Roster,
    events: &[LedgerEvent],
    weapons: &BTreeSet<String>,
) {
    let classified: Vec<(i64, ObjId)> = events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Classified { viewer, target, .. } if *viewer == reader => {
                Some((event.t_ms, *target))
            }
            _ => None,
        })
        .collect();
    for line in report.lines.iter().chain(report.tail.iter()) {
        let shot = weapons
            .iter()
            .any(|weapon| line.contains(&format!("{weapon} at ")));
        let by_ms = if shot {
            i64::MAX
        } else {
            stamp_secs(line).map_or(i64::MAX, |secs| (secs + 1) * 1_000)
        };
        for (unit, held) in roster.iter() {
            let callsign = held.callsign.as_str();
            if callsign.is_empty() || callsign == own {
                continue;
            }
            let named = line
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|word| word == callsign);
            if named {
                assert!(
                    classified
                        .iter()
                        .any(|(at, target)| *target == unit && *at < by_ms),
                    "{own}'s report names {callsign}, which its platform had not told apart \
                     by then: {line}"
                );
            }
        }
    }
}

#[test]
fn v3_a_real_fight_folds_to_each_pilots_own_projection() {
    let world = booted_world();
    let mut arena = shipped_arena(
        &world,
        &[
            &[Muster::reflex(Team::Red, 1).dry()],
            &[Muster::reflex(Team::Blue, 1).at(BLUE_AT)],
        ],
    );
    let loser = support::tank(&world, Team::Red, 1);
    let winner = support::tank(&world, Team::Blue, 1);
    let mut bytes = run_until(&mut arena, BOUND_MS, 1_000, |_, _| {});
    assert!(
        matches!(
            arena.ended.map(|end| end.reason),
            Some(EndReason::Elimination {
                winner: Some(Team::Blue)
            })
        ),
        "the armed tank did not eliminate the dry one inside {BOUND_MS} ms: {:?} at {} ms",
        arena.ended,
        arena.now_ms
    );
    let events = ledger_events(&mut arena, &mut bytes);
    let roster = roster_of(&arena);
    let callsign = |unit: ObjId| roster.callsign(unit).unwrap_or_default().to_owned();
    let (winner_cs, loser_cs) = (callsign(winner), callsign(loser));
    assert!(!winner_cs.is_empty() && !loser_cs.is_empty());

    // The tape: the winner fired, the loser did not.
    let fired: Vec<&LedgerEvent> = events
        .iter()
        .filter(|event| matches!(&event.kind, EventKind::Fired { unit, .. } if *unit == winner))
        .collect();
    assert!(!fired.is_empty(), "the winner fired nothing on the tape");
    assert!(
        !events
            .iter()
            .any(|event| matches!(&event.kind, EventKind::Fired { unit, .. } if *unit == loser)),
        "the dry tank fired"
    );
    let weapons: BTreeSet<String> = fired
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Fired { weapon, .. } => Some(weapon.to_string()),
            _ => None,
        })
        .collect();
    let struck = events
        .iter()
        .filter(|event| matches!(&event.kind, EventKind::Hit { target, .. } if *target == loser))
        .count();

    // The winner's own report: one line a shot (or counted as dropped).
    let shooter = fold(&events, &roster, Reader::Pilot { unit: winner }, RULES);
    let shot_lines = shooter
        .lines
        .iter()
        .filter(|line| {
            weapons
                .iter()
                .any(|weapon| line.contains(&format!("{weapon} at ")))
        })
        .count();
    if shooter.truncated == 0 {
        assert_eq!(shot_lines, fired.len(), "{:#?}", shooter.lines);
    } else {
        assert!(shot_lines + shooter.truncated >= fired.len());
        assert!(shot_lines <= fired.len());
    }
    assert!(!shooter.gap, "the rig's tape is whole");
    assert_eq!(shooter.kills, 1, "{:#?}", shooter.lines);
    assert_eq!(
        shooter.rounds.values().copied().map(u64::from).sum::<u64>(),
        fired
            .iter()
            .map(|event| match &event.kind {
                EventKind::Fired { rounds, .. } => u64::from(*rounds),
                _ => 0,
            })
            .sum::<u64>()
    );
    assert!(
        shooter
            .tail
            .iter()
            .any(|line| line.ends_with(&format!("the match ended: {} eliminated.", "red"))),
        "{:?}",
        shooter.tail
    );

    // The loser's: the hits on its own tank, one line each; no shot, no shooter.
    let target = fold(&events, &roster, Reader::Pilot { unit: loser }, RULES);
    let hit_lines = target
        .lines
        .iter()
        .filter(|line| line.contains("  hit on "))
        .count();
    if target.truncated == 0 {
        assert_eq!(hit_lines, struck, "{:#?}", target.lines);
    }
    assert!(struck > 0, "the loser was never struck");
    assert!(
        target
            .tail
            .iter()
            .any(|line| line.ends_with("your tank destroyed.")),
        "{:?}",
        target.tail
    );
    for line in target.lines.iter().chain(target.tail.iter()) {
        assert!(
            !line.contains(" at "),
            "a shot line reached the target: {line}"
        );
        assert!(
            line.split(|c: char| !c.is_ascii_alphanumeric())
                .all(|word| word != winner_cs),
            "the shooter's callsign reached the target: {line}"
        );
    }
    // Its header names the winner only in the public words: the side, never
    // the callsign.
    let pilots = [(ObjId(9_001), winner), (ObjId(9_002), loser)];
    let row = history_row(&arena, winner, loser, pilots);
    let header = header_lines(
        &row,
        "The Proving Ground",
        Reader::Pilot { unit: loser },
        Some(&target),
        None,
    );
    assert!(
        header
            .iter()
            .any(|line| line == &format!("You flew {loser_cs}: destroyed; 0 shots, 0 hits.")),
        "{header:?}"
    );
    assert!(
        header.iter().any(|line| line == "You fired nothing."),
        "{header:?}"
    );
    assert!(
        header.iter().all(|line| line
            .split(|c: char| !c.is_ascii_alphanumeric())
            .all(|word| word != winner_cs)),
        "{header:?}"
    );

    // Each reader names only what its own platform told apart, and no pilot
    // line leaks.
    names_only_what_was_told_apart(&shooter, winner, &winner_cs, &roster, &events, &weapons);
    names_only_what_was_told_apart(&target, loser, &loser_cs, &roster, &events, &weapons);
    for line in shooter
        .lines
        .iter()
        .chain(shooter.tail.iter())
        .chain(target.lines.iter())
        .chain(target.tail.iter())
    {
        assert_eq!(leak(line), None, "{line}");
    }
    // The stamp a pilot reads is the arena's own clock.
    assert_eq!(stamp(arena.now_ms), stamp(shooter.last_ms));

    // Staff read the whole of the same events: object numbers, a draw, and the
    // seed on the header.
    let staff = fold(&events, &roster, Reader::Staff, RULES);
    let text = staff.lines.join("\n");
    assert!(text.contains(&format!("#{}", winner.0)), "{text}");
    assert!(
        text.contains("[hit:") || text.contains("[burst:") || text.contains("[classify:"),
        "no draw on the staff page"
    );
    let mut named = roster.clone();
    named.name_pilots(|pilot| format!("Pilot {}", pilot.0));
    let page = Page {
        first: 0,
        last: 0,
        lines: events.len(),
        unreadable: 0,
    };
    let staff_header = header_lines(
        &row,
        "The Proving Ground",
        Reader::Staff,
        None,
        Some((&page, &named)),
    );
    assert!(
        staff_header
            .iter()
            .any(|line| line.starts_with(&format!("seed {}; ", support::SEED))),
        "{staff_header:?}"
    );
    // A pilot's header never carries it.
    assert!(header.iter().all(|line| !line.contains("seed")));
}

/// **16b cut O.1 (16c N11)**: a Hold's end, as a pilot's report and the
/// header say it — the pilot's end line reads `the match ended: red held the
/// key spot.` off the tape's own `match_ended { hold }` line, and the header
/// carries the ` [hold]` tag after the outcome words.
#[test]
fn o1_a_hold_ends_the_pilots_report_and_tags_the_header() {
    let red = ObjId(101);
    let blue = ObjId(201);
    let mut roster = Roster::new();
    roster.insert(red, Some(Team::Red), SmolStr::new("RA"), None);
    roster.insert(blue, Some(Team::Blue), SmolStr::new("BA"), None);
    let end = LedgerEvent {
        seq: 1,
        t_ms: 120_250,
        kind: EventKind::MatchEnded {
            reason: EndReason::Hold { winner: Team::Red },
        },
        draws: Vec::new(),
        hash: 0,
    };
    for unit in [red, blue] {
        let report = fold(
            std::slice::from_ref(&end),
            &roster,
            Reader::Pilot { unit },
            RULES,
        );
        assert_eq!(
            report.tail,
            vec!["  2:00  the match ended: red held the key spot.".to_owned()],
            "{unit}"
        );
    }
    let document = serde_json::json!({
        "record": "tacsim.match",
        "record_version": 1,
        "match": 3,
        "arena": "tacsim.arena.proving_ground",
        "outcome": "red",
        "reason": "hold",
        "mode": "hold",
        "teams": [
            { "team": "red", "units": [red.0], "pilots": [] },
            { "team": "blue", "units": [blue.0], "pilots": [] },
        ],
        "units": [
            { "unit": red.0, "callsign": "RA", "outcome": "intact", "shots": 0, "hits": 0 },
            { "unit": blue.0, "callsign": "BA", "outcome": "intact", "shots": 0, "hits": 0 },
        ],
        "ledger": { "log": "match.3", "digest": "ab", "chunks": 1 },
        "seed": 0,
        "started_ms": 1_790_258_000_000_i64,
        "ended_ms": 1_790_258_520_000_i64,
        "arena_ms": 120_250,
        "reports": [],
        "replay": false,
    });
    let row = HistoryRow::from_document(&document).expect("a row");
    let header = header_lines(
        &row,
        "The Proving Ground",
        Reader::Pilot { unit: red },
        None,
        None,
    );
    assert_eq!(
        header.first().map(String::as_str),
        Some(
            "Match 3 on The Proving Ground, 2026-09-24 14:02 UTC: red won (red held the key \
             spot) [hold], 2 m 0 s."
        )
    );
    // A skirmish's header carries no tag, and neither does a row that does not
    // say its mode.
    let mut plain = document.clone();
    plain["mode"] = serde_json::json!("skirmish");
    let row = HistoryRow::from_document(&plain).expect("a row");
    let header = header_lines(&row, "The Proving Ground", Reader::Staff, None, None);
    assert!(
        header.first().is_some_and(|line| !line.contains('[')),
        "{header:?}"
    );
}
