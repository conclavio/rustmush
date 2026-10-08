//! **The replay over a real fight, and the replay fixture set** (16b cut I.1;
//! 16c N9, Q37).
//!
//! The in-source tests of `match_::replay` pin the fold against a tape written
//! by hand. This one folds a tape the **arena** wrote: the shipped proving
//! ground, two stock tanks posed close through the shipped-scale rig
//! (`support::shipped_arena_seeded` at [`FIGHT_SEED`], 16b §0.4), the loser checked out dry so that only
//! one side shoots, run to an elimination — `tests/report.rs`'s fight. No
//! server: the fold is a pure function of the event list, which is exactly what
//! it is on the world side, where `arena_bridge::on_chunk` hands it the same
//! events a chunk at a time.
//!
//! What it proves is the picture's shape against the arena's own record: every
//! `position_summary` is one keyframe and keyframe 0 is the opening; every
//! `fired` is one shot carrying the outcome its `hit` or `miss` line gave it;
//! the kill is one wreck; the end is the `match_ended` line's; and the document
//! carries nothing but callsigns, cells and outcome words — no contact id, no
//! object number, no seed, no draw, no hash.
//!
//! It is also **the one blesser of the replay fixture set**
//! (`web/src/lib/fixtures/replay/replay.json` and `replay_window.json`), a
//! hand-built replay on the cockpit fixtures' yard (`support::yard`), which 16b
//! cut I.2's TUI and web compositors are written against. Re-bless with
//! `TACSIM_BLESS=1 cargo test -p tacsim --test replay`.

mod support;

use std::path::PathBuf;

use mush_core::ObjId;
use smol_str::SmolStr;
use support::{Muster, booted_world, ledger_events, run_until, shipped_arena_seeded};
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::state::{ArenaDefinition, EndReason, Team, Waypoints};
use tacsim::arena_task::MatchId;
use tacsim::match_::MatchOutcome;
use tacsim::match_::replay::{
    Keyframe, KeyframeUnit, Replay, ReplayFold, ReplayRoster, ReplayShot, ShotOutcome,
    replay_lines, replay_window,
};

/// How long the fight may take, arena milliseconds.
const BOUND_MS: i64 = 900_000;

/// Blue's opening cell: `tests/report.rs`'s, on the ford road fourteen cells
/// east of Red Tank 1's authored muster.
const BLUE_AT: (u16, u16) = (20, 97);

/// **The no-leak grep** over a replay document's text: the first thing it may
/// not carry — a sixteen-digit base-16 word (a contact id), an object mark, a
/// seed, a draw or a hash.
fn leak(text: &str) -> Option<&'static str> {
    let contact = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| {
            word.len() == 16
                && word
                    .chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        });
    if contact {
        return Some("a sixteen-digit word");
    }
    ["#", "seed", "draw", "hash"]
        .into_iter()
        .find(|word| text.contains(word))
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
fn i1_a_real_fight_folds_to_one_objective_picture() {
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

    // The fold, seeded exactly as `match_::start` seeds it: the arena's own
    // callsigns and sides, the check-out poses, the ground's grid and digest.
    let roster: Vec<(ObjId, tacsim::arena::state::Designator, Team)> = arena
        .callsigns()
        .into_iter()
        .filter_map(|(unit, designator)| arena.unit(unit).map(|held| (unit, designator, held.team)))
        .collect();
    let opening: Vec<(ObjId, mush_sim::data::Pose)> = roster
        .iter()
        .filter_map(|(unit, _, _)| arena.unit(*unit).map(|held| (*unit, held.pose.clone())))
        .collect();
    let (cell_cm, width, height) = (
        arena.definition.terrain.cell_cm(),
        arena.definition.terrain.width(),
        arena.definition.terrain.height(),
    );
    let digest = tacsim::arena_bridge::digest_text(&arena.definition.digest());
    let mut fold = ReplayFold::new(
        MatchId(1),
        &SmolStr::new(tacsim::match_::PROVING_GROUND),
        &digest,
        cell_cm,
        width,
        height,
        tacsim::config::ArenaConfig::default().ledger.summary_ms,
        &roster,
        &opening,
    );
    let cell_of = |pose: &mush_sim::data::Pose| {
        let cell = i64::from(cell_cm);
        (
            u16::try_from(pose.x_cm.div_euclid(cell)).unwrap_or(u16::MAX),
            u16::try_from(pose.y_cm.div_euclid(cell)).unwrap_or(u16::MAX),
        )
    };
    let opening_cells: Vec<(u16, u16)> = opening.iter().map(|(_, pose)| cell_of(pose)).collect();

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
    for event in &events {
        fold.absorb(event);
    }
    let replay = fold.finish(MatchOutcome::Blue, "elimination");
    let index_of = |unit: ObjId| {
        roster
            .iter()
            .position(|(held, _, _)| *held == unit)
            .and_then(|at| u16::try_from(at).ok())
            .expect("on the roster")
    };

    // Every summary is one keyframe, after the opening.
    let summaries: Vec<&LedgerEvent> = events
        .iter()
        .filter(|event| matches!(event.kind, EventKind::PositionSummary { .. }))
        .collect();
    assert!(!summaries.is_empty(), "the arena wrote no position summary");
    assert_eq!(replay.keyframes.len(), summaries.len() + 1);
    assert!(!replay.truncated, "a two-tank fight fits whole");
    let first = replay.keyframes.first().expect("keyframe 0");
    assert_eq!(first.t_ms, 0);
    assert_eq!(
        first
            .units
            .iter()
            .map(|slot| slot.map(|unit| (unit.x, unit.y)))
            .collect::<Vec<_>>(),
        opening_cells.iter().copied().map(Some).collect::<Vec<_>>(),
        "keyframe 0 is the opening"
    );
    for (keyframe, summary) in replay.keyframes.iter().skip(1).zip(&summaries) {
        assert_eq!(keyframe.t_ms, summary.t_ms);
        let EventKind::PositionSummary { units } = &summary.kind else {
            continue;
        };
        for (unit, x_cm, y_cm, _, heading) in units {
            let cell = i64::from(cell_cm);
            let slot = keyframe
                .units
                .get(usize::from(index_of(*unit)))
                .copied()
                .flatten()
                .expect("every unit is on the window");
            assert_eq!(
                (
                    i64::from(slot.x),
                    i64::from(slot.y),
                    u32::from(slot.heading_deg)
                ),
                (x_cm / cell, y_cm / cell, heading / 1_000)
            );
        }
    }

    // Every `fired` is one shot, with the outcome its line gave it.
    let fired: Vec<(i64, u64, ObjId, ObjId)> = events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Fired {
                unit,
                target,
                shot_seq,
                ..
            } => Some((event.t_ms, *shot_seq, *unit, *target)),
            _ => None,
        })
        .collect();
    assert!(!fired.is_empty(), "the winner fired nothing");
    assert_eq!(replay.shots.len(), fired.len());
    for (shot, (t_ms, seq, unit, target)) in replay.shots.iter().zip(&fired) {
        let outcome = events
            .iter()
            .find_map(|event| match &event.kind {
                EventKind::Hit { shot_seq, .. } if shot_seq == seq => Some(ShotOutcome::Hit),
                EventKind::Miss { shot_seq, .. } if shot_seq == seq => Some(ShotOutcome::Miss),
                _ => None,
            })
            .unwrap_or(ShotOutcome::Flying);
        assert_eq!(
            *shot,
            ReplayShot {
                t_ms: *t_ms,
                shooter: index_of(*unit),
                target: index_of(*target),
                outcome,
            }
        );
        assert_eq!(*unit, winner, "only the armed tank fired");
    }

    // The kill is one wreck; the end is the end line's.
    let killed = events
        .iter()
        .find(
            |event| matches!(&event.kind, EventKind::UnitDestroyed { unit, .. } if *unit == loser),
        )
        .expect("the loser was destroyed on the tape");
    assert_eq!(replay.destroyed, vec![(killed.t_ms, index_of(loser))]);
    let ended = events
        .iter()
        .find(|event| matches!(event.kind, EventKind::MatchEnded { .. }))
        .expect("the tape carries its end");
    assert_eq!(replay.end_ms, ended.t_ms);

    // **The last keyframe shows how the match ended** (the fixer's T1): the
    // arena writes a position summary only on its cadence and none at the end,
    // so the kill falls after the last keyframe unless it lands on a summary
    // boundary; the last keyframe of this whole replay answers to `end_ms`,
    // lists the wreck and draws it.
    let last = replay.keyframes.len() - 1;
    let last_t = replay.keyframes[last].t_ms;
    assert!(killed.t_ms > replay.keyframes[last - 1].t_ms);
    assert_eq!(replay.horizon(last), replay.end_ms.max(last_t));
    let loser_callsign = replay.roster[usize::from(index_of(loser))]
        .callsign
        .to_string();
    let (_, lines) = replay_lines(&replay, last, 0, &arena.definition.terrain);
    let wrecked = lines
        .iter()
        .find(|line| line.starts_with("Wrecked since"))
        .unwrap_or_else(|| panic!("no wreck on the last keyframe: {lines:?}"));
    assert!(
        wrecked.ends_with(&format!(": {loser_callsign}.")),
        "{wrecked}"
    );
    if killed.t_ms > last_t {
        assert!(wrecked.contains(", to the end at "), "{wrecked}");
    }
    let window = replay_window(
        &replay,
        last,
        index_of(loser),
        &arena.definition.terrain,
        &arena.definition.key_spots,
    );
    assert!(
        window["units"].as_array().is_some_and(|units| units
            .iter()
            .any(|unit| unit["id"] == loser_callsign.as_str()
                && unit["glyph"] == "%"
                && unit["name"] == "wreck")),
        "the last window does not draw the wreck: {window}"
    );

    // Callsigns, cells and outcome words; nothing else.
    let text = serde_json::to_string(&replay.to_document()).expect("the document encodes");
    assert_eq!(leak(&text), None, "{text}");
    // The roster names each unit by its callsign and nothing else.
    let document = replay.to_document();
    for entry in document["roster"].as_array().cloned().unwrap_or_default() {
        let mut keys: Vec<String> = entry
            .as_object()
            .map(|object| object.keys().cloned().collect())
            .unwrap_or_default();
        keys.sort();
        assert_eq!(keys, vec!["callsign", "team"], "{entry}");
    }
    assert_eq!(Replay::from_document(&replay.to_document()), Some(replay));
}

// ---------------------------------------------------------------------------
// The replay fixture set (16b cut I.1, for 16b cut I.2)
// ---------------------------------------------------------------------------

/// The env var that rewrites the fixtures instead of comparing against them.
const BLESS: &str = "TACSIM_BLESS";

/// The directory the replay fixtures live in, from this crate's manifest.
fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/src/lib/fixtures/replay")
}

/// Compare `actual` with the committed `name`, or rewrite it under [`BLESS`].
fn pinned(name: &str, committed: &str, actual: &str) {
    if std::env::var(BLESS).is_ok() {
        std::fs::create_dir_all(fixture_dir()).expect("the fixture directory");
        std::fs::write(fixture_dir().join(name), actual).expect("the fixture is writable");
        return;
    }
    assert_eq!(
        committed, actual,
        "web/src/lib/fixtures/replay/{name} is out of date; re-bless with \
         `{BLESS}=1 cargo test -p tacsim --test replay`"
    );
}

/// One payload, as a fixture file holds it: pretty JSON, a trailing newline.
fn text(value: &serde_json::Value) -> String {
    let mut out = serde_json::to_string_pretty(value).expect("a payload encodes");
    out.push('\n');
    out
}

/// A cell and a heading.
fn at(x: u16, y: u16, heading_deg: u16) -> Option<KeyframeUnit> {
    Some(KeyframeUnit { x, y, heading_deg })
}

/// **The fixture replay**: four units on the yard — `RA`, `RB` red and `BA`,
/// `BB` blue, in that roster order — over three keyframes at 0, 5,000 and
/// 10,000 ms; `RA` at `BA` a hit at 6,000, `BA` at `RA` a miss at 8,000; `BB`
/// wrecked at 7,000.
///
/// Keyframe 1 is the scene the window fixture draws: `RA` at (6, 4), so the
/// window clamps at the yard's south-west corner as `view_grid.txt`'s does;
/// `RB` at (2, 9); `BA` at (36, 14), on the window's east side; `BB` at (14, 7),
/// on The Roof, so the spot is placed, covered and named nowhere.
fn fixture_replay() -> Replay {
    let definition = ArenaDefinition {
        id: SmolStr::new(support::yard::SURFACE),
        terrain: support::yard::terrain(),
        entry_zones: Vec::new(),
        key_spots: support::yard::spots(),
        waypoints: Waypoints::default(),
        modes: Vec::new(),
    };
    let roster = |callsign: &str, team: Team| ReplayRoster {
        callsign: SmolStr::new(callsign),
        team,
    };
    Replay {
        match_id: MatchId(7),
        arena: SmolStr::new(support::yard::SURFACE),
        arena_digest: tacsim::arena_bridge::digest_text(&definition.digest()),
        cell_cm: 1_000,
        width: support::yard::WIDTH,
        height: support::yard::HEIGHT,
        summary_ms: 5_000,
        end_ms: 10_000,
        outcome: MatchOutcome::Abort,
        reason: SmolStr::new("abort"),
        roster: vec![
            roster("RA", Team::Red),
            roster("RB", Team::Red),
            roster("BA", Team::Blue),
            roster("BB", Team::Blue),
        ],
        keyframes: vec![
            Keyframe {
                t_ms: 0,
                units: vec![at(4, 4, 90), at(2, 7, 90), at(40, 14, 270), at(16, 7, 270)],
            },
            Keyframe {
                t_ms: 5_000,
                units: vec![at(6, 4, 90), at(2, 9, 0), at(36, 14, 270), at(14, 7, 225)],
            },
            Keyframe {
                t_ms: 10_000,
                units: vec![at(9, 4, 90), at(2, 11, 0), at(33, 14, 270), at(14, 7, 225)],
            },
        ],
        shots: vec![
            ReplayShot {
                t_ms: 6_000,
                shooter: 0,
                target: 2,
                outcome: ShotOutcome::Hit,
            },
            ReplayShot {
                t_ms: 8_000,
                shooter: 2,
                target: 0,
                outcome: ShotOutcome::Miss,
            },
        ],
        destroyed: vec![(7_000, 3)],
        truncated: false,
        cut: Vec::new(),
        mode: tacsim::arena::MatchMode::Skirmish,
    }
}

/// **The replay fixtures are what the builders produce** (16b cut I.1): the
/// document and keyframe 1's window, byte for byte.
#[test]
fn i1_the_replay_fixtures_are_what_the_builders_produce() {
    let replay = fixture_replay();
    let window = replay_window(
        &replay,
        1,
        0,
        &support::yard::terrain(),
        &support::yard::spots(),
    );
    pinned(
        "replay.json",
        include_str!("../../../web/src/lib/fixtures/replay/replay.json"),
        &text(&replay.to_document()),
    );
    pinned(
        "replay_window.json",
        include_str!("../../../web/src/lib/fixtures/replay/replay_window.json"),
        &text(&window),
    );
}

/// **What the fixture scene exercises**, asserted on the payloads rather than
/// left to a reader of the JSON: the south-west clamp, the marker order, the
/// covered spot named nowhere, `BB` still blue at keyframe 1 and a wreck at 2,
/// `you: null`, `retain: latest`, and a document that round-trips.
#[test]
fn i1_the_replay_fixture_scene_exercises_the_clamp_the_cover_and_the_wreck() {
    let replay = fixture_replay();
    let terrain = support::yard::terrain();
    let spots = support::yard::spots();
    let window = replay_window(&replay, 1, 0, &terrain, &spots);
    assert_eq!(window["origin_cell"], serde_json::json!([0, 0]));
    assert_eq!(window["schema"]["retain"], serde_json::json!("latest"));
    assert_eq!(window["you"], serde_json::Value::Null);
    let markers: Vec<(String, String)> = window["units"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|unit| {
            (
                unit["id"].as_str().unwrap_or_default().to_owned(),
                unit["glyph"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        markers,
        vec![
            ("A".to_owned(), "A".to_owned()),
            ("B".to_owned(), "B".to_owned()),
            ("RA".to_owned(), "o".to_owned()),
            ("RB".to_owned(), "o".to_owned()),
            ("BA".to_owned(), "*".to_owned()),
            ("BB".to_owned(), "*".to_owned()),
        ]
    );
    let legend = window["legend"].as_array().cloned().unwrap_or_default();
    assert!(legend.contains(&serde_json::json!(["A", "The Ford"])));
    assert!(
        !legend.iter().any(|entry| entry[1] == "The Roof"),
        "BB owns The Roof's cell"
    );
    assert!(!legend.contains(&serde_json::json!(["%", "wreck"])));
    let later = replay_window(&replay, 2, 0, &terrain, &spots);
    assert!(
        later["units"].as_array().is_some_and(|units| units
            .iter()
            .any(|unit| unit["id"] == "BB" && unit["glyph"] == "%" && unit["name"] == "wreck")),
        "{later}"
    );
    assert_eq!(
        Replay::from_document(&replay.to_document()),
        Some(replay.clone())
    );
    assert_eq!(leak(&text(&replay.to_document())), None);
}

/// **16b cut O.1 (16c N11; plan item 39)**: a replay finished as a Hold carries
/// the hold's words, and its document carries `"mode"` beside `outcome` and
/// `reason` — which a replay written before the cut has no key for, and reads
/// as a skirmish.
#[test]
fn o1_a_held_replay_carries_its_words_and_its_mode() {
    let mut replay = fixture_replay();
    replay.outcome = MatchOutcome::Red;
    replay.reason = SmolStr::new("hold");
    replay.mode = tacsim::arena::MatchMode::Hold;
    let document = replay.to_document();
    assert_eq!(document["outcome"], serde_json::json!("red"));
    assert_eq!(document["reason"], serde_json::json!("hold"));
    assert_eq!(document["mode"], serde_json::json!("hold"));
    assert_eq!(Replay::from_document(&document), Some(replay.clone()));
    let (header, _) = replay_lines(&replay, 0, 0, &support::yard::terrain());
    assert!(
        header.contains("red won (red held the key spot) [hold]"),
        "{header}"
    );
    // A document written before the cut: no key, a skirmish, no tag.
    let mut old = document;
    old.as_object_mut().expect("an object").remove("mode");
    let read = Replay::from_document(&old).expect("still a replay");
    assert_eq!(read.mode, tacsim::arena::MatchMode::Skirmish);
    let (header, _) = replay_lines(&read, 0, 0, &support::yard::terrain());
    assert!(!header.contains("[hold]"), "{header}");
}
