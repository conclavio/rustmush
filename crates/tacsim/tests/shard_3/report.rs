//! **The debrief over a real fight** (16b cut V.3; 16c N2, Q37).
//!
//! The in-source tests of `match_::report` pin every line against a tape written
//! by hand. This one folds a tape the **arena** wrote: the shipped proving
//! ground, two stock tanks posed close through the shipped-scale rig
//! (`support::shipped_arena_seeded` at [`FIGHT_SEED`], 16b §0.4), the loser checked out dry so that only
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

use crate::support;

use std::collections::BTreeSet;

use mush_core::ObjId;
use smol_str::SmolStr;
use support::{Muster, booted_world, ledger_events, run_until, shipped_arena_seeded};
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::state::{Arena, EndReason, Team};
use tacsim::match_::HistoryRow;
use tacsim::match_::report::{
    Fold, HandOver, OwnMind, Page, Reader, Report, ReportRules, Roster, fold, header_lines, stamp,
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
/// unit's side, no pilots (the fold's tally never reads one; `Fold::pilot`
/// does, for the world's spans, which these tests tell the fold directly).
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
        "seed": FIGHT_SEED,
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

/// **The match seed this fight runs on** (17b cut M4.5a): re-derived from
/// [`support::SEED`] — see the test's doc for why.
const FIGHT_SEED: u64 = 2_352;

/// **17b cut M4.5a**: both tanks are unit minds flown by Standard Doctrine on
/// the native stock mind ([`Muster::mind`], what production fields since 17b
/// cut M4.3), where they were the reflex controller; every claim is kept as
/// written and the bound is unchanged. **The match seed is re-derived from
/// [`support::SEED`] (2,350) to 2,352**: a stock mind fires its cannon and its
/// coaxial from two slots at one millisecond where the reflex alternated them,
/// so the `hit`/`burst`/facet draws keyed on `shot_seq` fall differently, and
/// at 2,350 the dry tank's turret goes first (`remove_weapons`,
/// `destroy_core` at 26,008 ms) — an elimination by disarming (16b cut AP.2)
/// with no wreck, against which the wreck claims below cannot be asserted.
/// 2,352 is a seed from 2,350 whose fight ends in the loser's wreck (the
/// porter measured 8 of the 20 seeds 2,350 to 2,369 doing so); the fight, the
/// musters and the bound are otherwise the original's.
#[test]
fn v3_a_real_fight_folds_to_each_pilots_own_projection() {
    let world = booted_world();
    let mut arena = shipped_arena_seeded(
        &world,
        &[
            &[Muster::mind(Team::Red, 1).dry()],
            &[Muster::mind(Team::Blue, 1).at(BLUE_AT)],
        ],
        FIGHT_SEED,
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
            .any(|line| line.starts_with(&format!("seed {FIGHT_SEED}; "))),
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

/// **16b cut O.2** (16c N12): the pod's switch folds to the switching pilot's
/// own timeline and to nobody else's, and the staff fold spells it as a field
/// with the position as a bare word.
#[test]
fn o2_the_pods_switch_is_the_pilots_own_line_and_a_staff_field() {
    let red = ObjId(101);
    let blue = ObjId(201);
    let mut roster = Roster::new();
    roster.insert(red, Some(Team::Red), SmolStr::new("RA"), None);
    roster.insert(blue, Some(Team::Blue), SmolStr::new("BA"), None);
    let event = |seq: u64, t_ms: i64, unit: ObjId, on: bool| LedgerEvent {
        seq,
        t_ms,
        kind: EventKind::Emcon { unit, on },
        draws: Vec::new(),
        hash: 0,
    };
    let tape = vec![
        event(1, 10_000, red, false),
        event(2, 12_000, blue, false),
        event(3, 30_000, red, true),
    ];
    let own = fold(&tape, &roster, Reader::Pilot { unit: red }, RULES);
    assert_eq!(
        own.lines,
        vec![
            "  0:10  you switched EMCON off.".to_owned(),
            "  0:30  you switched EMCON on.".to_owned(),
        ],
        "the pilot's own switches, and not the other side's"
    );
    let theirs = fold(&tape, &roster, Reader::Pilot { unit: blue }, RULES);
    assert_eq!(
        theirs.lines,
        vec!["  0:12  you switched EMCON off.".to_owned()]
    );
    let staff = fold(&tape, &roster, Reader::Staff, RULES);
    let fields: Vec<String> = staff
        .lines
        .iter()
        .map(|line| {
            line.split_once("  emcon ")
                .map_or(String::new(), |(_, rest)| rest.to_owned())
        })
        .collect();
    assert_eq!(
        fields,
        vec![
            "unit=RA(#101) off".to_owned(),
            "unit=BA(#201) off".to_owned(),
            "unit=RA(#101) on".to_owned(),
        ],
        "{:?}",
        staff.lines
    );
    for line in own.lines.iter().chain(theirs.lines.iter()) {
        assert_eq!(leak(line), None, "{line}");
    }
}

/// **16b cut O.3 (16c N2, N13)**: the pilot's own line carries the hit rate —
/// `(42%)` for 6 hits in 14 shots, rounded down; `(0%)` when shots were fired
/// and none landed; no bracket at all when nothing was fired — and a range's
/// header carries the ` [range]` tag after the outcome words. The staff rows
/// are unchanged.
#[test]
fn o3_the_pilot_line_carries_the_hit_rate_and_the_header_tags_a_range() {
    let (ra, rb, rc) = (ObjId(101), ObjId(102), ObjId(103));
    let (ba, bb) = (ObjId(201), ObjId(202));
    let document = serde_json::json!({
        "record": "tacsim.match",
        "record_version": 1,
        "match": 4,
        "arena": "tacsim.arena.proving_ground",
        "outcome": "red",
        "reason": "elimination",
        "mode": "range",
        "teams": [
            { "team": "red", "units": [ra.0, rb.0, rc.0], "pilots": [] },
            { "team": "blue", "units": [ba.0, bb.0], "pilots": [] },
        ],
        "units": [
            { "unit": ra.0, "callsign": "RA", "outcome": "intact", "shots": 14, "hits": 6 },
            { "unit": rb.0, "callsign": "RB", "outcome": "intact", "shots": 0, "hits": 0 },
            { "unit": rc.0, "callsign": "RC", "outcome": "intact", "shots": 3, "hits": 0 },
            { "unit": ba.0, "callsign": "BA", "outcome": "destroyed", "shots": 0, "hits": 0 },
            { "unit": bb.0, "callsign": "BB", "outcome": "destroyed", "shots": 0, "hits": 0 },
        ],
        "ledger": { "log": "match.4", "digest": "ab", "chunks": 1 },
        "seed": 0,
        "started_ms": 1_790_258_000_000_i64,
        "ended_ms": 1_790_258_520_000_i64,
        "arena_ms": 272_000,
        "reports": [],
        "replay": false,
    });
    let row = HistoryRow::from_document(&document).expect("a row");
    assert_eq!(row.mode, Some(tacsim::arena::MatchMode::Range));
    let pilot = |unit: ObjId| {
        header_lines(
            &row,
            "The Proving Ground",
            Reader::Pilot { unit },
            None,
            None,
        )
    };
    let header = pilot(ra);
    assert_eq!(
        header,
        vec![
            "Match 4 on The Proving Ground, 2026-09-24 14:02 UTC: red won (blue eliminated) \
             [range], 4 m 32 s."
                .to_owned(),
            "Red lost 0 of 3; blue lost 2 of 2.".to_owned(),
            "You flew RA: intact; 14 shots, 6 hits (42%).".to_owned(),
        ]
    );
    assert_eq!(
        pilot(rb).get(2).map(String::as_str),
        Some("You flew RB: intact; 0 shots, 0 hits."),
        "no shot, no rate"
    );
    assert_eq!(
        pilot(rc).get(2).map(String::as_str),
        Some("You flew RC: intact; 3 shots, 0 hits (0%)."),
        "shots and no hit is nought per cent"
    );
    // The staff rows carry no rate.
    let staff = header_lines(
        &row,
        "The Proving Ground",
        Reader::Staff,
        None,
        Some((
            &Page {
                first: 0,
                last: 0,
                lines: 0,
                unreadable: 0,
            },
            &Roster::from_row(&row),
        )),
    );
    assert!(
        staff
            .iter()
            .any(|line| line.contains("RA") && line.ends_with("14 shots, 6 hits.")),
        "{staff:?}"
    );
    assert!(staff.iter().all(|line| !line.contains('%')), "{staff:?}");
}

// ---------------------------------------------------------------------------
// 16b cut AF.3 (16c N16) — the truncating reader
// ---------------------------------------------------------------------------

/// One arena chunk entry, `{"chunk": n, "lines": ...}`, with a tag in its lines
/// so the test can tell two writes of one sequence apart.
fn chunk_entry(n: u64, tag: &str) -> serde_json::Value {
    serde_json::json!({ "chunk": n, "bytes": tag.len(), "lines": tag })
}

/// One resume marker, as `tacsim::match_::resume_pending` appends it.
fn marker_entry(from: u64, at_ms: i64) -> serde_json::Value {
    serde_json::json!({ "resumed": { "from_chunk": from, "at_ms": at_ms, "restart_ms": 7_000 } })
}

/// The `lines` tags of a kept tape, in order.
fn tags(kept: &[serde_json::Value]) -> Vec<String> {
    kept.iter()
        .filter_map(|entry| entry.get("lines").and_then(serde_json::Value::as_str))
        .map(str::to_owned)
        .collect()
}

/// **The tape reader truncates at the marker** (16b cut AF.3; the brief's test
/// 3): a pure proof of `arena_bridge::tape_in_order` and of the pager's use of
/// it.
///
/// - `[1, 2, 3, 4, 5, marker(from 2), 3']` keeps `[1, 2, 3']` — the old
///   server's 4 and 5 are abandoned however many there were, which a
///   last-write rule alone would have kept;
/// - a marker at the end of what the old server wrote (`from 3` after 3)
///   discards nothing;
/// - two markers each retire their own abandoned tail;
/// - an unforked tape comes back unchanged;
/// - the pager's sentence is rendered from the marker verbatim and is at most
///   120 columns at a six-digit chunk and eight-digit milliseconds;
/// - a staff page holding `[1 .. N, abandoned, marker, N + 1']` folds with no
///   gap: the fold is reset by nothing and the truncated tape is contiguous.
#[test]
fn af3_the_tape_reader_truncates_at_the_marker() {
    use tacsim::arena_bridge::{resume_marker_line, tape_in_order};

    // (1) The abandoned tail, whatever its length.
    let tape = vec![
        chunk_entry(1, "1"),
        chunk_entry(2, "2"),
        chunk_entry(3, "3"),
        chunk_entry(4, "4"),
        chunk_entry(5, "5"),
        marker_entry(2, 10_000),
        chunk_entry(3, "3'"),
    ];
    let (kept, markers) = tape_in_order(&tape);
    assert_eq!(tags(&kept), ["1", "2", "3'"]);
    assert_eq!(markers, vec![marker_entry(2, 10_000)]);

    // (2) A marker at the tail's own end abandons nothing.
    let tape = vec![
        chunk_entry(1, "1"),
        chunk_entry(2, "2"),
        chunk_entry(3, "3"),
        marker_entry(3, 15_000),
        chunk_entry(4, "4"),
        chunk_entry(5, "5"),
    ];
    let (kept, markers) = tape_in_order(&tape);
    assert_eq!(tags(&kept), ["1", "2", "3", "4", "5"]);
    assert_eq!(markers.len(), 1);

    // (3) Two resumes, two abandoned tails.
    let tape = vec![
        chunk_entry(1, "1"),
        chunk_entry(2, "2"),
        chunk_entry(3, "3"),
        marker_entry(1, 5_000),
        chunk_entry(2, "2'"),
        chunk_entry(3, "3'"),
        chunk_entry(4, "4'"),
        marker_entry(3, 15_000),
        chunk_entry(4, "4''"),
    ];
    let (kept, markers) = tape_in_order(&tape);
    assert_eq!(tags(&kept), ["1", "2'", "3'", "4''"]);
    assert_eq!(
        markers,
        vec![marker_entry(1, 5_000), marker_entry(3, 15_000)]
    );

    // (4) An unforked tape is itself.
    let tape: Vec<serde_json::Value> = (1..=6).map(|n| chunk_entry(n, &n.to_string())).collect();
    let (kept, markers) = tape_in_order(&tape);
    assert_eq!(kept, tape);
    assert!(markers.is_empty());

    // (5) The pager's sentence, and its worst case against the floor.
    assert_eq!(
        resume_marker_line(&marker_entry(2, 10_000)).as_deref(),
        Some(
            "-- resumed at chunk 2 (10000 ms of arena time); the old server's chunks after 2 \
             are abandoned --"
        )
    );
    assert_eq!(resume_marker_line(&chunk_entry(2, "2")), None);
    let worst = resume_marker_line(&marker_entry(999_999, 99_999_999)).expect("a marker line");
    assert!(
        worst.chars().count() <= 120,
        "{} columns: {worst}",
        worst.chars().count()
    );
    assert_eq!(worst.chars().count(), 109, "{worst}");

    // (6) A staff page over a forked tape folds with no gap. Chunks 0 and 1
    // hold events 1..=4; the old server's chunk 2 holds 5..=6 and is
    // abandoned; the resumed chunk 2' holds 5..=6 again.
    let unit = ObjId(101);
    let events = |from: u64, at_ms: i64| -> String {
        (0..2)
            .map(|n| {
                let held = LedgerEvent {
                    seq: from + n,
                    t_ms: at_ms + i64::try_from(n).unwrap_or(0) * 1_000,
                    kind: EventKind::PowerOnline { unit },
                    draws: Vec::new(),
                    hash: 0,
                };
                serde_json::to_string(&held).expect("an event encodes")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let doc = |n: u64, entries: Vec<(u64, serde_json::Value)>| {
        (
            mush_core::chunk_key("match.12", n),
            serde_json::json!({
                "chunk": n,
                "sealed": true,
                "entries": entries
                    .into_iter()
                    .map(|(seq, value)| serde_json::json!({ "seq": seq, "value": value }))
                    .collect::<Vec<_>>(),
            }),
        )
    };
    let arena_chunk = |n: u64, lines: String| serde_json::json!({ "chunk": n, "lines": lines });
    let docs = vec![
        doc(0, vec![(0, arena_chunk(1, events(1, 1_000)))]),
        doc(1, vec![(1, arena_chunk(2, events(3, 3_000)))]),
        doc(
            2,
            vec![
                (2, arena_chunk(3, events(5, 5_000))),
                (3, marker_entry(2, 4_500)),
                (4, arena_chunk(3, events(5, 5_500))),
            ],
        ),
    ];
    let mut roster = Roster::new();
    roster.insert(unit, Some(Team::Red), SmolStr::new("RA"), None);
    let folded =
        tacsim::match_::report::fold_page(&docs, "match.12", 0, &roster).expect("a valid page");
    assert!(!folded.report.gap, "{:?}", folded.report.lines);
    let marker_at = folded
        .report
        .lines
        .iter()
        .position(|line| line.starts_with("-- resumed at chunk 2 (4500 ms"))
        .unwrap_or_else(|| panic!("the marker line is printed: {:?}", folded.report.lines));
    assert_eq!(
        marker_at, 4,
        "after the four kept events and before the resumed two: {:?}",
        folded.report.lines
    );
    assert_eq!(folded.report.lines.len(), 7, "{:?}", folded.report.lines);
    assert_eq!(folded.page.lines, 6, "the abandoned two are not folded");
}

// ---------------------------------------------------------------------------
// 17b cut M5.6: the `Your mind` block over the real fight
// ---------------------------------------------------------------------------

/// **The real fight, as `v3_a_real_fight_folds_to_each_pilots_own_projection`
/// runs it** (17b cut M5.6): the same rig, musters, seed and bound — the armed
/// blue tank eliminates the dry red one — answering the arena, its tape, the
/// roster, the winner and the loser.
fn the_fight() -> (Arena, Vec<LedgerEvent>, Roster, ObjId, ObjId) {
    let world = booted_world();
    let mut arena = shipped_arena_seeded(
        &world,
        &[
            &[Muster::mind(Team::Red, 1).dry()],
            &[Muster::mind(Team::Blue, 1).at(BLUE_AT)],
        ],
        FIGHT_SEED,
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
        "the armed tank did not eliminate the dry one inside {BOUND_MS} ms: {:?}",
        arena.ended
    );
    let events = ledger_events(&mut arena, &mut bytes);
    let roster = roster_of(&arena);
    (arena, events, roster, winner, loser)
}

/// One moment the world tells a fold of: a release (`Some(by)`) or a
/// reclaim (`None`), at an arena millisecond.
type Moment = (i64, Option<HandOver>);

/// **`unit`'s pilot fold as the world feeds it** (17b cut M5.6): seeded with
/// `own` as `match_::start` seeds it, and each of `moments` told before the
/// first event at or after it — the world marks the slot ahead of the chunks
/// that carry the tape after it.
fn folded(
    events: &[LedgerEvent],
    roster: &Roster,
    unit: ObjId,
    own: OwnMind,
    moments: &[Moment],
) -> Report {
    let mut fold = Fold::new(roster.clone(), Reader::Pilot { unit }, RULES).with_own_mind(own);
    let tell = |fold: &mut Fold, (at_ms, by): Moment| match by {
        Some(by) => fold.released(at_ms, by),
        None => fold.reclaimed(at_ms),
    };
    let mut moments = moments.iter().copied().peekable();
    for event in events {
        while let Some(moment) = moments.next_if(|(at_ms, _)| *at_ms <= event.t_ms) {
            tell(&mut fold, moment);
        }
        fold.absorb(event);
    }
    for moment in moments {
        tell(&mut fold, moment);
    }
    fold.finish()
}

/// Standard Doctrine at `profiles` (empty is Line alone), as the check-out
/// sealed it.
fn doctrine(chose: bool, profiles: &[&str]) -> OwnMind {
    OwnMind {
        chose,
        mind: SmolStr::new_static("Standard Doctrine"),
        profiles: profiles.iter().map(|name| SmolStr::new(*name)).collect(),
        substituted: false,
    }
}

/// The block's lines, or none.
fn block_of(report: &Report) -> Vec<String> {
    report
        .mind
        .as_ref()
        .map(|block| block.lines.clone())
        .unwrap_or_default()
}

/// Everything a report says to its reader, the block included.
fn whole(report: &Report) -> Vec<String> {
    report
        .lines
        .iter()
        .chain(report.tail.iter())
        .cloned()
        .chain(block_of(report))
        .collect()
}

/// **17b cut M5.6** (the owner's OQ-9 (ii); the design note's §2.4 fog table:
/// `Your mind` is the pilot's whose hull carries the mind, alone): over the
/// arena's own tape, each pilot's report carries their own `Your mind` block
/// and no other — the winner, who chose Sniper and Line and climbed out
/// mid-fight, reads theirs and nothing of the loser's Hunter; the loser, who
/// chose Hunter and was never released, reads theirs and nothing of the
/// winner's; an unchosen pilot never released reads none; and the staff page
/// of the same tape carries no block. Every block line is under 120 columns
/// and passes the report's no-leak grep.
#[test]
fn m5_the_pilots_report_carries_their_own_mind_block_and_no_other() {
    let (_arena, events, roster, winner, loser) = the_fight();
    let fired_at: Vec<i64> = events
        .iter()
        .filter(|event| matches!(&event.kind, EventKind::Fired { unit, .. } if *unit == winner))
        .map(|event| event.t_ms)
        .collect();
    assert!(fired_at.len() >= 2, "the winner fired {fired_at:?}");
    let exit_at = fired_at[fired_at.len() / 2];

    let shooter = folded(
        &events,
        &roster,
        winner,
        doctrine(true, &["Sniper", "Line"]),
        &[(exit_at, Some(HandOver::ClimbedOut))],
    );
    let mine = block_of(&shooter);
    assert_eq!(
        mine.first().map(String::as_str),
        Some("Your mind: Standard Doctrine (Sniper, Line).")
    );
    assert_eq!(
        mine.get(1),
        Some(&format!(
            "  It flew your tank from {}, when you climbed out, to the end.",
            stamp(exit_at)
        ))
    );
    assert!(
        mine.get(2)
            .is_some_and(|line| line.starts_with("  Its shots: ")),
        "{mine:#?}"
    );

    let target = folded(&events, &roster, loser, doctrine(true, &["Hunter"]), &[]);
    let theirs = block_of(&target);
    assert_eq!(
        theirs.first().map(String::as_str),
        Some("Your mind: Standard Doctrine (Hunter).")
    );
    assert_eq!(
        theirs.get(1).map(String::as_str),
        Some("  It never flew: you flew the tank the whole match.")
    );
    // Seed 2,352's tape carries no drop to the ROM on the loser's hull (it is
    // destroyed without one), so the block is its two lines; a drop is `m5_the_drop_reads_its_cause_word_
    // alone`'s (17b cut M5.6's review, R29: the branch pinned, not either).
    let dropped = events.iter().find_map(|event| match &event.kind {
        EventKind::MindDropped { unit, .. } if *unit == loser => Some(event.t_ms),
        _ => None,
    });
    assert_eq!(dropped, None, "the loser drops to the ROM on seed 2,352");
    assert_eq!(theirs.len(), 2, "{theirs:#?}");

    // No other: neither reads the other's mind, an unchosen pilot never
    // released reads none, and the staff page carries no block.
    for line in whole(&shooter) {
        assert!(
            !line.contains("Hunter"),
            "the loser's mind reached the winner: {line}"
        );
    }
    for line in whole(&target) {
        assert!(
            !line.contains("Sniper"),
            "the winner's mind reached the loser: {line}"
        );
        assert!(
            !line.contains("Its shots"),
            "the winner's shots reached the loser: {line}"
        );
    }
    let unchosen = folded(&events, &roster, loser, doctrine(false, &[]), &[]);
    assert_eq!(unchosen.mind, None);
    assert_eq!(
        unchosen,
        fold(&events, &roster, Reader::Pilot { unit: loser }, RULES)
    );
    let staff = fold(&events, &roster, Reader::Staff, RULES);
    assert_eq!(staff.mind, None);
    assert!(staff.lines.iter().all(|line| !line.contains("Your mind")));

    for line in mine.iter().chain(theirs.iter()) {
        assert!(line.chars().count() < 120, "{line}");
        assert_eq!(leak(line), None, "{line}");
    }
}

/// **17b cut M5.6's review** (R1/R23; the owner's OQ-9 (ii)), on the arena's
/// own tape: the mind flies no wreck. Seed 2,352's loser is destroyed by the
/// winner; a release the world tells at or after that moment — an `EXIT` from
/// the wreck the cockpit itself advises, or a drop — is no span: an unchosen
/// pilot reads no block and the report is the one with no mind, a chosen one
/// reads that it never flew; and so whether the world's call reaches the fold
/// before the chunk that carries the destruction (the tape lags the world) or
/// after it. A span running when the hull is destroyed ends at its stamp.
#[test]
fn m5_the_mind_flies_no_wreck_on_the_arenas_own_tape() {
    let (_arena, events, roster, _winner, loser) = the_fight();
    let wrecked_at = events
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::UnitDestroyed { unit, .. } if *unit == loser => Some(event.t_ms),
            _ => None,
        })
        .expect("the winner destroys the loser");
    let base = fold(&events, &roster, Reader::Pilot { unit: loser }, RULES);
    for at_ms in [wrecked_at, wrecked_at + 5_000] {
        for by in [HandOver::ClimbedOut, HandOver::Dropped] {
            let moment = [(at_ms, Some(by))];
            let unchosen = folded(&events, &roster, loser, doctrine(false, &[]), &moment);
            assert_eq!(unchosen, base, "{by:?} at {at_ms} ms, in tape order");
            let mut ahead = Fold::new(roster.clone(), Reader::Pilot { unit: loser }, RULES)
                .with_own_mind(doctrine(false, &[]));
            ahead.released(at_ms, by);
            for event in &events {
                ahead.absorb(event);
            }
            assert_eq!(
                ahead.finish(),
                base,
                "{by:?} at {at_ms} ms, ahead of the tape"
            );
            let chosen = folded(
                &events,
                &roster,
                loser,
                doctrine(true, &["Hunter"]),
                &moment,
            );
            assert_eq!(
                block_of(&chosen),
                vec![
                    "Your mind: Standard Doctrine (Hunter).",
                    "  It never flew: you flew the tank the whole match.",
                ],
                "{by:?} at {at_ms} ms"
            );
        }
    }
    let from = wrecked_at / 2;
    let flown = folded(
        &events,
        &roster,
        loser,
        doctrine(false, &[]),
        &[(from, Some(HandOver::ClimbedOut))],
    );
    assert_eq!(
        block_of(&flown).get(1),
        Some(&format!(
            "  It flew your tank from {}, when you climbed out, to {}.",
            stamp(from),
            stamp(wrecked_at)
        ))
    );
}

/// **17b cut M5.6** (the design note's M5.6 item 1; the adversarial review's
/// finding 5): the mind's shots are the hull's `fired` lines inside a span,
/// with their hits and the kills they made — the shots and hits counted here
/// off the arena's own tape, independently of the fold, and the kills by the
/// fold's own rule restated (the hull's last completed shot before the
/// `unit_destroyed` was a span's shot that hit that unit) — and **the pilot's own lines are
/// unchanged**: the report's timeline, tail, rounds, kills and gap, and the
/// header's `You flew …` and `You fired …` lines, are those of the same match
/// folded with no mind at all (the code path every report took before this
/// cut), for a span the pilot took back and for one that ran to the end.
#[test]
fn m5_the_minds_shots_are_counted_in_its_block_and_the_pilots_lines_are_unchanged() {
    let (arena, events, roster, winner, loser) = the_fight();
    let base = fold(&events, &roster, Reader::Pilot { unit: winner }, RULES);
    // The winner's own counts on seed 2,352, as literals (17b cut M5.6's
    // review, R5): what the fold with no mind reads on this tape. The fold's
    // counting code is `5f505c8`'s (`report.rs` unchanged from `5f505c8` to
    // M5.5's `354e0ae8`; M5.6 adds the mind's calls beside it), so these are
    // the counts that commit's fold gives this tape.
    assert_eq!(
        base.rounds,
        std::collections::BTreeMap::from([
            (SmolStr::new("cannon"), 4),
            (SmolStr::new("machine_gun"), 230)
        ])
    );
    assert_eq!(base.kills, 1);
    let row = history_row(
        &arena,
        winner,
        loser,
        [(ObjId(9_001), winner), (ObjId(9_002), loser)],
    );
    let header = |report: &Report| {
        header_lines(
            &row,
            "The Proving Ground",
            Reader::Pilot { unit: winner },
            Some(report),
            None,
        )
    };
    let fired: Vec<(i64, u64, String)> = events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Fired {
                unit,
                shot_seq,
                weapon,
                ..
            } if *unit == winner => Some((event.t_ms, *shot_seq, weapon.to_string())),
            _ => None,
        })
        .collect();
    assert!(fired.len() >= 3, "the winner fired {fired:?}");
    let from = fired[1].0;
    let last = fired.last().map_or(from, |(t_ms, ..)| *t_ms);
    assert!(from < last, "the winner's shots span no time: {fired:?}");

    for to in [Some(last), None] {
        let mut moments: Vec<Moment> = vec![(from, Some(HandOver::Dropped))];
        moments.extend(to.map(|to| (to, None)));
        let flown = folded(&events, &roster, winner, doctrine(false, &[]), &moments);

        // The pilot's own lines: as the same match folded with no mind.
        assert_eq!(
            Report {
                mind: None,
                ..flown.clone()
            },
            base
        );
        assert_eq!(header(&flown), header(&base));

        // The block's count, off the tape: the shots inside the span, the hits
        // among them, and a kill whose last outcome before it was one of the
        // span's shots hitting the unit destroyed.
        let inside = |t_ms: i64| from <= t_ms && to.is_none_or(|to| t_ms < to);
        let ours: Vec<&(i64, u64, String)> =
            fired.iter().filter(|(t_ms, ..)| inside(*t_ms)).collect();
        assert!(
            !ours.is_empty(),
            "the span holds none of the winner's shots"
        );
        let mut per_weapon: std::collections::BTreeMap<&str, u32> =
            std::collections::BTreeMap::new();
        for (_, _, weapon) in &ours {
            *per_weapon.entry(weapon.as_str()).or_default() += 1;
        }
        let seqs: BTreeSet<u64> = ours.iter().map(|(_, seq, _)| *seq).collect();
        let winners: BTreeSet<u64> = fired.iter().map(|(_, seq, _)| *seq).collect();
        let hits = events
            .iter()
            .filter(|event| {
                matches!(
                    &event.kind,
                    EventKind::Hit { shot_seq, .. } if seqs.contains(shot_seq)
                )
            })
            .count();
        let mut last_outcome: Option<(u64, Option<ObjId>)> = None;
        let mut kills = 0_u32;
        for event in &events {
            match &event.kind {
                EventKind::Hit {
                    shot_seq, target, ..
                } if winners.contains(shot_seq) => last_outcome = Some((*shot_seq, Some(*target))),
                EventKind::Miss { shot_seq, .. } if winners.contains(shot_seq) => {
                    last_outcome = Some((*shot_seq, None));
                }
                EventKind::UnitDestroyed { unit, by }
                    if *by == Some(winner)
                        && last_outcome.is_some_and(|(seq, target)| {
                            seqs.contains(&seq) && target == Some(*unit)
                        }) =>
                {
                    kills += 1;
                }
                _ => {}
            }
        }
        let shots: u32 = per_weapon.values().sum();
        let each: Vec<String> = per_weapon
            .iter()
            .map(|(weapon, n)| format!("{weapon} {n}"))
            .collect();
        let weapons = match each.split_last() {
            Some((only, [])) => only.clone(),
            Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
            None => unreachable!("the span holds a shot"),
        };
        let outcome = match (shots, hits) {
            (1, 0) => "no hit".to_owned(),
            (1, _) => "a hit".to_owned(),
            (_, 0) => "none of them hits".to_owned(),
            (_, 1) => "1 of them a hit".to_owned(),
            (_, hits) => format!("{hits} of them hits"),
        };
        let destroyed = match kills {
            0 => "nothing".to_owned(),
            1 => "1 unit".to_owned(),
            kills => format!("{kills} units"),
        };
        let span = match to {
            Some(to) => format!(
                "  It flew your tank from {} to {}, while your line was down.",
                stamp(from),
                stamp(to)
            ),
            None => format!(
                "  It flew your tank from {} to the end, while your line was down.",
                stamp(from)
            ),
        };
        let said = vec![
            "Your mind: Standard Doctrine (Line).".to_owned(),
            span,
            format!("  Its shots: {weapons}, {outcome}; it destroyed {destroyed}."),
        ];
        assert!(
            !events.iter().any(|event| matches!(
                &event.kind,
                EventKind::MindDropped { unit, .. } if *unit == winner
            )),
            "the winner never drops to the ROM in this fight"
        );
        assert_eq!(block_of(&flown), said, "span to {to:?}");
        if to.is_none() {
            assert_eq!(kills, base.kills, "the killing shot was the mind's");
        }
    }
}
