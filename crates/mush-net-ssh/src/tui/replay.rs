//! **The replay on screen 0** (16b cut I.2; 16c N9, Q37): a finished match
//! drawn on the cockpit screen's own panes, one keyframe at a time.
//!
//! `SIM REPLAY <match>` (16b cut I.1) pushes a `grid` window on
//! [`REPLAY_FEED`], and that push is what opens the picture (`TuiApp`'s
//! `follow_the_replay`): the app asks the parameter-less `arena.replay` query
//! ONCE for the whole document of the match the viewer's cursor names, and
//! `arena.map` under the cockpit's own latch for the ground it was fought on.
//! From then on every step is the client's own, over the document it holds —
//! `,` and `.` step, `u` moves the focus, `x` closes — and nothing is sent.
//!
//! **The layout is the cockpit's.** Screen 0 keeps its `Screen` (the same panes,
//! slots, sizes and degrade order 16b cut AB.1 built); what changes is what each
//! pane draws while the held `arena.cockpit` frame is not seated and a picture
//! is open ([`super::render::RenderCtx::picture`]):
//!
//! * **map** — the ground windowed on the focus unit at the keyframe, the named
//!   spots first, then every roster unit that stood in the window, in roster
//!   order: `o` red, `*` blue, `%` a wreck — the one marker order every
//!   compositor follows, a later marker overwriting an earlier one. Its heading
//!   is `Replay 12 — 1:30 (19/55)`, its legend the units that own their cell
//!   (`o RA  * BA  % BB`) and then the spots that own theirs (`A The Ford`);
//! * **status** — six sentences, each at or under the Left pane's 32 columns so
//!   none wraps at the floor;
//! * **damage** — the wrecks recorded so far, newest first;
//! * **weapons** — the shots fired since the previous keyframe, newest first;
//! * **contacts** — every roster unit at the keyframe, selectable; `Enter` on a
//!   row moves the focus to it;
//! * **notices** — unchanged.
//!
//! **Both sides are shown, and the pane says so** (Q37): a replay is of a
//! finished match and names every tank by the callsign the arena gave it — no
//! object number, no pilot and no contact id reach the document, so none can
//! reach this screen. **The TUI draws nothing between keyframes**: positions
//! are the arena's own every `summary_ms`, and the status pane says that too.
//! (The web draws between them, and says so there.)
//!
//! A wreck, a shot and the damage pane's list all run to the keyframe's
//! **horizon** (16b cut I.1's `Replay::horizon`): the keyframe's own `t_ms`,
//! except the last keyframe of a replay whose keyframes were not cut, which
//! answers for everything to `end_ms` — the arena writes no summary at the end,
//! so the killing shot of an elimination falls after the last keyframe and this
//! is where it is shown.
//!
//! # The live picture (16b cut I.4)
//!
//! `@arena/watch <match>` makes a staffer a watcher of a RUNNING match, and the
//! world pushes them the whole field on `arena.observer` at most once an arena
//! second (`tacsim::match_::observer::observer_frame`). The first such keyframe
//! opens a **live** picture on this same layout (`Picture::live`), closing a
//! replay that was open; each later one replaces what is drawn. The panes read
//! the pushed picture ([`Live`]) instead of a document:
//!
//! * **map** — the units as a replay draws them (`o` red, `*` blue, `%` a
//!   wreck; a unit with no cell is not drawn), headed `Watching 12 — 1:30
//!   (live)`, centred on the focus unit;
//! * **status** — `Match 12  engaged  floor 5000` (`floor` only while a staff
//!   floor stands), `Live, at most once a second.`, `In flight: 2` and
//!   `@arena/watch off stops it.`, each at or under 32 columns;
//! * **damage** and **weapons** — `Nothing yet.`;
//! * **contacts** — `RA   red   (120, 97) 090° #301 machine wreck`, the
//!   object number on purpose (a staff surface), `pilot` for a pilot's tank,
//!   then the FIRST of `wreck`, `held` and `starting` that applies; one space
//!   between fields and one state word so a realistic row — a three-digit
//!   cell, a four-digit object number, `wreck` or `held` — is whole in the
//!   46-column contacts pane at the 120-column floor, and a row wider still
//!   (`starting`, for the ten seconds of a start-up) is cut at the pane's edge
//!   with an ellipsis rather than wrapped: one line a unit, always.
//!
//! **Nothing steps and nothing closes it but the server**: `,` `.` and `x` are
//! not in the live picture's key table (the picture is what watching is, and a
//! local close would be undone by the next keyframe a second later); `u` moves
//! the focus and `p` pulls. `watching: false` — the watch ended, the match
//! ended, or a server-half restart forgot it — closes the picture and marks the
//! match dismissed.

use std::collections::HashMap;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

use super::panels::{ARENA_MAP_FEED, OBSERVER_FEED, REPLAY_FEED};
use super::render::{
    COCKPIT_MAP_FALLBACK, MAP_CHROME_ROWS, NOTHING_YET, PaneRow, PaneRows, RenderCtx,
    WAITING_FOR_THE_MAP, cell_num, cell_pair, clean, degree, dim_span_owned, fit_legend, one_char,
    put_marker, seated, truncate_chars,
};

/// **The replay open on screen 0** (16b cut I.2): which match, the keyframe the
/// panes draw, the roster unit the map is centred on — and `live`, which 16b
/// cut I.4's watched picture sets and a replay never does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Picture {
    /// The match the replay is of (the pushed grid's `match`).
    pub match_id: i64,
    /// The keyframe the panes draw, 0-based; clamped to the document's.
    pub at: usize,
    /// The roster index of the unit the map window is centred on.
    pub focus: u16,
    /// Whether this is a live picture (16b cut I.4's); `false` for a replay.
    pub live: bool,
}

/// The `schema.shape` of the `arena.replay` query's answer — the whole
/// document, or `available: false` (16b cut I.1's `REPLAY_DOCUMENT_SHAPE`).
pub const REPLAY_DOCUMENT_SHAPE: &str = "replay";

/// The `schema.shape` of the window `SIM REPLAY` pushes — the trigger that
/// opens a picture.
pub const REPLAY_GRID_SHAPE: &str = "grid";

/// What the map pane says while a picture is open and its document has not
/// arrived (16b cut I.2): the document is pulled, never pushed.
pub const WAITING_FOR_THE_REPLAY: &str = "Waiting for the replay...";

/// A red unit's marker (16b cut I.1's `RED_GLYPH`).
pub const RED_GLYPH: char = 'o';
/// A blue unit's marker (16b cut I.1's `BLUE_GLYPH`).
pub const BLUE_GLYPH: char = '*';
/// A wreck's marker, whichever side it was (16b cut I.1's `WRECK_GLYPH`).
pub const WRECK_GLYPH: char = '%';

/// What the map pane says while a live picture is open and no picture of its
/// match is held (16b cut I.4) — a moment after a re-hydrate forgot it.
pub const WAITING_FOR_THE_PICTURE: &str = "Waiting for the picture...";

/// The live status pane's second sentence (16b cut I.4).
pub const LIVE_CADENCE: &str = "Live, at most once a second.";

/// The live status pane's last sentence (16b cut I.4).
pub const LIVE_STOP: &str = "@arena/watch off stops it.";

/// **How a match ended, in the status pane's words** (16b cut I.2): the rows
/// of `tacsim::match_::record::outcome_table` — an outcome word and a reason
/// word, exactly as a result and a replay document carry them — each with its
/// words abbreviated so `Match 1234  blue won (red elim.)` stays inside the
/// Left pane's 32 columns. `tacsim` is not a dependency of this crate (nor a
/// dev-dependency), so the rows are copied here rather than read, and no test
/// pins them against `history_lines`' words; a pair not here prints its
/// outcome word alone, as the record's own fallback prints the raw pair.
const OUTCOME_WORDS: &[(&str, &str, &str)] = &[
    ("red", "elimination", "red won (blue elim.)"),
    ("blue", "elimination", "blue won (red elim.)"),
    ("red", "conceded", "red won (blue conc.)"),
    ("blue", "conceded", "blue won (red conc.)"),
    // 16b cut O.1 (16c N11): a hold names its winner.
    ("red", "hold", "red won (held spot)"),
    ("blue", "hold", "blue won (held spot)"),
    ("draw", "spent", "draw (no ammo)"),
    ("draw", "elimination", "draw (both elim.)"),
    ("timeout", "unattended", "timed out (empty)"),
    ("timeout", "timeout", "timed out (time)"),
    ("abort", "abort", "aborted"),
    ("abort", "restart", "aborted (restart)"),
    ("void", "void", "void"),
    ("void", "ledger_overflow", "void (tape full)"),
];

/// The status pane's words for an outcome and reason pair (the `OUTCOME_WORDS` table).
#[must_use]
pub fn outcome_short(outcome: &str, reason: &str) -> String {
    OUTCOME_WORDS
        .iter()
        .find(|(o, r, _)| *o == outcome && *r == reason)
        .map_or_else(
            || truncate_chars(&clean(outcome), 16),
            |(_, _, words)| (*words).to_string(),
        )
}

/// **The ` [hold]` tag for a replay document's mode word** (16b cut O.1; plan
/// item 39): ` [{mode}]` for a word that is neither empty nor `skirmish`, so a
/// match fought under another rule is never read as a plain fight; the empty
/// string otherwise — which is every replay written before the cut, whose
/// document has no `mode` at all.
#[must_use]
pub fn mode_tag(mode: &str) -> String {
    if mode.is_empty() || mode == "skirmish" {
        String::new()
    } else {
        format!(" [{mode}]")
    }
}

/// **The replay map's heading** (16b cut I.2; the tag 16b cut O.1's): `Replay
/// 12 — 1:30 (19/55)`, `-` for the dash without UTF-8, and the mode's ` [hold]`
/// tag after it ([`mode_tag`]). At its widest — a four-digit match, a
/// sixty-minute clock, three-digit keyframes and the tag — it is 36 columns,
/// inside the replay map's 39. (The status pane's first row carries the outcome
/// words and is 32 columns at its widest already, so the tag rides here.)
#[must_use]
pub fn replay_heading(
    match_id: i64,
    t_ms: i64,
    at: usize,
    count: usize,
    mode: &str,
    utf8: bool,
) -> String {
    let sep = if utf8 { "\u{2014}" } else { "-" };
    format!(
        "Replay {match_id} {sep} {} ({}/{count}){}",
        clock_text(t_ms),
        at.saturating_add(1),
        mode_tag(mode)
    )
}

/// An arena millisecond as `m:ss` — `tacsim::match_::lobby::clock_text`, the
/// clock every replay line reads.
#[must_use]
pub fn clock_text(ms: i64) -> String {
    let total_s = ms.max(0) / 1_000;
    format!("{}:{:02}", total_s / 60, total_s % 60)
}

/// The keyframe cadence in words: `5 s`, or `100 ms` for a cadence that is not
/// whole seconds (line mode's `every_words`).
fn every_words(summary_ms: i64) -> String {
    if summary_ms > 0 && summary_ms % 1_000 == 0 {
        format!("{} s", summary_ms / 1_000)
    } else {
        format!("{} ms", summary_ms.max(0))
    }
}

/// One roster unit as the panes print it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosterUnit {
    /// The callsign, cleaned and cut to eight characters (a designator's
    /// length).
    pub callsign: String,
    /// Whether it flew blue; anything else reads red, as the server's own
    /// reader keeps an unreadable side.
    pub blue: bool,
}

impl RosterUnit {
    /// `red` or `blue`.
    #[must_use]
    pub fn side(&self) -> &'static str {
        if self.blue { "blue" } else { "red" }
    }

    /// The marker it is drawn as while it stands.
    #[must_use]
    pub fn glyph(&self) -> char {
        if self.blue { BLUE_GLYPH } else { RED_GLYPH }
    }
}

/// **A read of one `arena.replay` document** (16b cut I.1's `Replay::
/// to_document` under `schema.shape == "replay"`), total over what the wire
/// might carry: a missing or malformed key reads empty, a keyframe slot that is
/// not three numbers reads as not in the keyframe, and nothing here indexes.
#[derive(Clone, Copy, Debug)]
pub struct Document<'a> {
    doc: &'a Value,
    match_id: i64,
}

impl<'a> Document<'a> {
    /// The document in `value`, or `None` for anything else — the pushed
    /// grid, `available: false`, or a payload with no match.
    #[must_use]
    pub fn of(value: &'a Value) -> Option<Document<'a>> {
        if value.pointer("/schema/shape").and_then(Value::as_str) != Some(REPLAY_DOCUMENT_SHAPE)
            || value.get("available").and_then(Value::as_bool) == Some(false)
        {
            return None;
        }
        let match_id = value.get("match").and_then(Value::as_i64)?;
        Some(Document {
            doc: value,
            match_id,
        })
    }

    /// The match it is a replay of.
    #[must_use]
    pub fn match_id(&self) -> i64 {
        self.match_id
    }

    fn int(&self, key: &str) -> i64 {
        self.doc.get(key).and_then(Value::as_i64).unwrap_or(0)
    }

    fn list(&self, key: &str) -> &'a [Value] {
        self.doc
            .get(key)
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice)
    }

    /// Every roster unit, in roster order.
    #[must_use]
    pub fn roster(&self) -> Vec<RosterUnit> {
        self.list("roster")
            .iter()
            .map(|entry| RosterUnit {
                callsign: truncate_chars(
                    &clean(entry.get("callsign").and_then(Value::as_str).unwrap_or("?")),
                    8,
                ),
                blue: entry.get("team").and_then(Value::as_str) == Some("blue"),
            })
            .collect()
    }

    /// The roster index of `callsign`, exactly as [`Self::roster`] prints it.
    #[must_use]
    pub fn roster_index(&self, callsign: &str) -> Option<u16> {
        self.roster()
            .iter()
            .position(|unit| unit.callsign == callsign)
            .and_then(|at| u16::try_from(at).ok())
    }

    /// **The rule the match was fought under** (16b cut O.1): the document's
    /// `mode` word, cleaned and cut to sixteen characters, or `skirmish` where
    /// the key is absent — every replay written before the cut.
    #[must_use]
    pub fn mode(&self) -> String {
        self.doc.get("mode").and_then(Value::as_str).map_or_else(
            || "skirmish".to_string(),
            |word| truncate_chars(&clean(word), 16),
        )
    }

    /// How many keyframes it holds.
    #[must_use]
    pub fn keyframe_count(&self) -> usize {
        self.list("keyframes").len()
    }

    /// Keyframe `at`'s arena millisecond (0 past the end).
    #[must_use]
    pub fn keyframe_t(&self, at: usize) -> i64 {
        self.list("keyframes")
            .get(at)
            .and_then(|kf| kf.get(0))
            .and_then(Value::as_i64)
            .unwrap_or(0)
    }

    /// Where roster unit `unit` stood at keyframe `at` — `(x, y, heading)` —
    /// or `None` when it is not in that keyframe.
    #[must_use]
    pub fn slot(&self, at: usize, unit: usize) -> Option<(usize, usize, u64)> {
        let row = self
            .list("keyframes")
            .get(at)?
            .get(1)?
            .as_array()?
            .get(unit)?
            .as_array()?;
        let num = |i: usize| row.get(i).and_then(Value::as_u64);
        let x = usize::try_from(num(0)?).ok()?;
        let y = usize::try_from(num(1)?).ok()?;
        Some((x, y, num(2)? % 360))
    }

    /// The arena millisecond the match ended at.
    #[must_use]
    pub fn end_ms(&self) -> i64 {
        self.int("end_ms")
    }

    /// The `cut` words, in the document's order.
    #[must_use]
    pub fn cuts(&self) -> Vec<&'a str> {
        self.list("cut").iter().filter_map(Value::as_str).collect()
    }

    /// Whether anything was left out.
    #[must_use]
    pub fn truncated(&self) -> bool {
        self.doc.get("truncated").and_then(Value::as_bool) == Some(true)
    }

    /// **The arena millisecond keyframe `at` answers for** (16b cut I.1's
    /// `Replay::horizon`): its own `t_ms`, except the LAST keyframe of a
    /// replay whose keyframes were not cut, which answers to `end_ms`. A
    /// `truncated` document naming no cause reads as cut at its last keyframe,
    /// as the server's own reader reads it.
    #[must_use]
    pub fn horizon(&self, at: usize) -> i64 {
        let t_ms = self.keyframe_t(at);
        let last = at.saturating_add(1) == self.keyframe_count();
        let cut_at_keyframes =
            self.cuts().contains(&"keyframes") || (self.truncated() && self.cuts().is_empty());
        if last && !cut_at_keyframes {
            t_ms.max(self.end_ms())
        } else {
            t_ms
        }
    }

    /// The wrecks, `(t_ms, roster index)`, in tape order.
    #[must_use]
    pub fn destroyed(&self) -> Vec<(i64, usize)> {
        self.list("destroyed")
            .iter()
            .filter_map(|entry| {
                let t = entry.get(0)?.as_i64()?;
                let at = usize::try_from(entry.get(1)?.as_u64()?).ok()?;
                Some((t, at))
            })
            .collect()
    }

    /// Whether roster unit `unit` was a wreck by arena millisecond `t_ms`.
    #[must_use]
    pub fn wrecked_by(&self, unit: usize, t_ms: i64) -> bool {
        self.destroyed()
            .iter()
            .any(|(when, at)| *at == unit && *when <= t_ms)
    }

    /// The shots, `(t_ms, shooter, target, outcome word)`, in firing order.
    #[must_use]
    pub fn shots(&self) -> Vec<(i64, usize, usize, &'a str)> {
        self.list("shots")
            .iter()
            .filter_map(|entry| {
                let t = entry.get(0)?.as_i64()?;
                let shooter = usize::try_from(entry.get(1)?.as_u64()?).ok()?;
                let target = usize::try_from(entry.get(2)?.as_u64()?).ok()?;
                let word = entry.get(3)?.as_str()?;
                Some((t, shooter, target, word))
            })
            .collect()
    }

    /// The document's `summary_ms`.
    #[must_use]
    pub fn summary_ms(&self) -> i64 {
        self.int("summary_ms")
    }
}

/// **One unit of a live picture** (16b cut I.4): a positional row of the
/// `arena.observer` payload, `[callsign, team letter, x | null, y | null,
/// heading, object number, flags]`, read totally — a malformed row reads what it
/// can and `?` for a callsign.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveUnit {
    /// The callsign, cleaned and cut to eight characters.
    pub callsign: String,
    /// Whether it is blue (`B`); anything else reads red.
    pub blue: bool,
    /// Its cell, or `None` off the window.
    pub cell: Option<(usize, usize)>,
    /// Its heading, whole degrees.
    pub heading: u64,
    /// Its object number.
    pub id: u64,
    /// The flags, `w p s m h` in order.
    pub flags: String,
}

impl LiveUnit {
    /// `red` or `blue`.
    #[must_use]
    pub fn side(&self) -> &'static str {
        if self.blue { "blue" } else { "red" }
    }

    /// Whether it is a wreck (`w`).
    #[must_use]
    pub fn wreck(&self) -> bool {
        self.flags.contains('w')
    }

    /// The marker it is drawn as: `%` a wreck, else its side's.
    #[must_use]
    pub fn glyph(&self) -> char {
        if self.wreck() {
            WRECK_GLYPH
        } else if self.blue {
            BLUE_GLYPH
        } else {
            RED_GLYPH
        }
    }

    /// The contacts row's words after the object number: `machine` or
    /// `pilot`, then the FIRST of `wreck`, `held` and `starting` that applies
    /// — one state word, in that order of weight: a wreck is no subject of any
    /// order, so its hold and its power say nothing more, and a tank held
    /// against another is one already moving under power. One word keeps the
    /// row inside the contacts pane at the 120-column floor (see the live
    /// contacts pane's renderer, `live_contacts_rows`).
    #[must_use]
    pub fn words(&self) -> String {
        let controller = if self.flags.contains('m') {
            "machine"
        } else {
            "pilot"
        };
        [('w', "wreck"), ('h', "held"), ('s', "starting")]
            .into_iter()
            .find(|(flag, _)| self.flags.contains(*flag))
            .map_or_else(
                || controller.to_string(),
                |(_, state)| format!("{controller} {state}"),
            )
    }
}

/// **A read of one `arena.observer` picture** (16b cut I.4), total over what
/// the wire might carry. `watching: false` and a payload with no match are not
/// a picture.
#[derive(Clone, Copy, Debug)]
pub struct Live<'a> {
    doc: &'a Value,
    match_id: i64,
}

impl<'a> Live<'a> {
    /// The picture in `value`, or `None` for anything else.
    #[must_use]
    pub fn of(value: &'a Value) -> Option<Live<'a>> {
        if value.get("watching").and_then(Value::as_bool) == Some(false) {
            return None;
        }
        let match_id = value.get("match").and_then(Value::as_i64)?;
        Some(Live {
            doc: value,
            match_id,
        })
    }

    /// The match it is a picture of.
    #[must_use]
    pub fn match_id(&self) -> i64 {
        self.match_id
    }

    /// The arena millisecond it was taken at.
    #[must_use]
    pub fn now_ms(&self) -> i64 {
        self.doc.get("now_ms").and_then(Value::as_i64).unwrap_or(0)
    }

    /// The tempo word, cleaned and cut.
    #[must_use]
    pub fn tempo(&self) -> String {
        truncate_chars(
            &clean(self.doc.get("tempo").and_then(Value::as_str).unwrap_or("?")),
            8,
        )
    }

    /// The staff tempo floor, milliseconds, while one stands.
    #[must_use]
    pub fn floor_ms(&self) -> Option<u64> {
        self.doc.get("floor_ms").and_then(Value::as_u64)
    }

    /// Rounds in flight.
    #[must_use]
    pub fn in_flight(&self) -> u64 {
        self.doc
            .get("in_flight")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    }

    /// Every unit, in the payload's (unit-id) order.
    #[must_use]
    pub fn units(&self) -> Vec<LiveUnit> {
        self.doc
            .get("units")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .map(|row| {
                let at = |i: usize| row.get(i);
                let num = |i: usize| at(i).and_then(Value::as_u64);
                let cell = match (num(2), num(3)) {
                    (Some(x), Some(y)) => usize::try_from(x).ok().zip(usize::try_from(y).ok()),
                    _ => None,
                };
                LiveUnit {
                    callsign: truncate_chars(
                        &clean(at(0).and_then(Value::as_str).unwrap_or("?")),
                        8,
                    ),
                    blue: at(1).and_then(Value::as_str) == Some("B"),
                    cell,
                    heading: num(4).unwrap_or(0) % 360,
                    id: num(5).unwrap_or(0),
                    flags: truncate_chars(&clean(at(6).and_then(Value::as_str).unwrap_or("")), 8),
                }
            })
            .collect()
    }

    /// The index of `callsign` among the units, exactly as [`Self::units`]
    /// prints it.
    #[must_use]
    pub fn unit_index(&self, callsign: &str) -> Option<u16> {
        self.units()
            .iter()
            .position(|unit| unit.callsign == callsign)
            .and_then(|at| u16::try_from(at).ok())
    }
}

/// **The live picture held for the open live picture's match** (16b cut I.4),
/// the condition every live pane is drawn on.
#[must_use]
pub fn held_live<'a>(ctx: &RenderCtx<'a>) -> Option<(&'a Picture, Live<'a>)> {
    let picture = ctx.picture.filter(|picture| picture.live)?;
    let live = ctx
        .store
        .get(OBSERVER_FEED)
        .and_then(Live::of)
        .filter(|live| live.match_id() == picture.match_id)?;
    Some((picture, live))
}

/// Whether the open picture is a live one.
fn is_live(ctx: &RenderCtx<'_>) -> bool {
    ctx.picture.is_some_and(|picture| picture.live)
}

/// The keyframe the panes draw: the picture's own, clamped to the document.
fn at_of(picture: &Picture, doc: &Document<'_>) -> usize {
    picture.at.min(doc.keyframe_count().saturating_sub(1))
}

/// **The picture and its document**, when the store holds the replay document
/// of the open picture's match — the condition every replay pane is drawn on.
#[must_use]
pub fn held<'a>(ctx: &RenderCtx<'a>) -> Option<(&'a Picture, Document<'a>)> {
    let picture = ctx.picture?;
    let doc = ctx
        .store
        .get(REPLAY_FEED)
        .and_then(Document::of)
        .filter(|doc| doc.match_id() == picture.match_id)?;
    Some((picture, doc))
}

/// A roster unit's callsign, `?` past the roster.
fn callsign_of(roster: &[RosterUnit], at: usize) -> &str {
    roster.get(at).map_or("?", |unit| unit.callsign.as_str())
}

/// The shot's outcome in a pane's words: `in flight` for one still flying.
fn shot_words(word: &str) -> &str {
    match word {
        "flying" => "in flight",
        "hit" => "hit",
        "miss" => "miss",
        _ => "?",
    }
}

/// One unit as the picture's map draws it: its marker, its callsign, and its
/// cell (`None`: not drawn).
type MapUnit = (char, String, Option<(usize, usize)>);

/// Who wrote a replay map cell last: a spot by its index in the map's
/// `spots`, or a roster unit by its roster index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Owner {
    Spot(usize),
    Unit(usize),
}

/// **The map pane of a replay** (16b cut I.2), or `None` when no picture is
/// open.
///
/// The composition is the cockpit map's own (`render_cockpit_map_rows`), with
/// the replay's units where the frame's contacts were:
///
/// 1. the window is `mush_ansi::viewport(map width, map height, the FOCUS
///    unit's cell at the keyframe, grid width, grid height)` — the arena's
///    centre when the focus is not in the keyframe — so the grid is the pane's
///    `(width, height − MAP_CHROME_ROWS)`, never eighty; with no pane it is
///    `COCKPIT_MAP_FALLBACK`;
/// 2. the terrain is `mush_ansi::window_rows` of the pulled `arena.map`, which
///    must be the ground of the picture's match (else `Waiting for the arena
///    map...`);
/// 3. the markers go down in ONE order: the map's named spots in authored
///    order, as their letters; then every roster unit in the keyframe, in
///    roster order — `o` red, `*` blue, `%` a unit wrecked by the keyframe's
///    horizon — a later marker overwriting an earlier one; a unit not in the
///    keyframe, or off the window, is not drawn;
/// 4. the rows print north-up.
///
/// The rows are the heading (`Replay 12 — 1:30 (19/55)`, bold; `-` without
/// UTF-8), the grid, and the legend (`fit_legend`, the owner rule: the units
/// that own their cell in roster order as `o RA  * BA  % BB`, then the spots
/// that own theirs as `A The Ford`), shedding the chrome exactly as the
/// cockpit's map does on a short pane. This is the TUI half of the parity:
/// at a `(39, 22)` pane over the committed fixtures its twenty grid rows are
/// `web/src/lib/fixtures/replay/replay_grid.txt`, byte for byte.
#[must_use]
pub fn map_rows(ctx: &RenderCtx<'_>) -> Option<PaneRows> {
    let picture = ctx.picture?;
    if picture.live {
        return Some(live_map_rows(ctx, picture));
    }
    let Some((_, doc)) = held(ctx) else {
        return Some(PaneRows::one(WAITING_FOR_THE_REPLAY));
    };
    let Some(map) = ground_of(ctx, picture) else {
        return Some(PaneRows::one(WAITING_FOR_THE_MAP));
    };
    let at = at_of(picture, &doc);
    let horizon = doc.horizon(at);
    let units: Vec<MapUnit> = doc
        .roster()
        .iter()
        .enumerate()
        .map(|(index, unit)| {
            let glyph = if doc.wrecked_by(index, horizon) {
                WRECK_GLYPH
            } else {
                unit.glyph()
            };
            (
                glyph,
                unit.callsign.clone(),
                doc.slot(at, index).map(|(x, y, _)| (x, y)),
            )
        })
        .collect();
    let focus = doc
        .slot(at, usize::from(picture.focus))
        .map(|(x, y, _)| (x, y));
    let heading = replay_heading(
        picture.match_id,
        doc.keyframe_t(at),
        at,
        doc.keyframe_count(),
        &doc.mode(),
        ctx.utf8,
    );
    Some(compose_map(ctx, map, &heading, focus, &units))
}

/// **The map pane of a live picture** (16b cut I.4): [`compose_map`] over the
/// held picture's units — `o` red, `*` blue, `%` a wreck, a unit with no cell
/// not drawn — centred on the focus unit and headed `Watching 12 — 1:30
/// (live)`.
fn live_map_rows(ctx: &RenderCtx<'_>, picture: &Picture) -> PaneRows {
    let Some((_, live)) = held_live(ctx) else {
        return PaneRows::one(WAITING_FOR_THE_PICTURE);
    };
    let Some(map) = ground_of(ctx, picture) else {
        return PaneRows::one(WAITING_FOR_THE_MAP);
    };
    let units: Vec<MapUnit> = live
        .units()
        .into_iter()
        .map(|unit| (unit.glyph(), unit.callsign.clone(), unit.cell))
        .collect();
    let focus = units
        .get(usize::from(picture.focus))
        .and_then(|(_, _, cell)| *cell);
    let sep = if ctx.utf8 { "\u{2014}" } else { "-" };
    let heading = format!(
        "Watching {} {sep} {} (live)",
        picture.match_id,
        clock_text(live.now_ms())
    );
    compose_map(ctx, map, &heading, focus, &units)
}

/// The pulled ground of the picture's match, when the store holds it.
fn ground_of<'a>(ctx: &RenderCtx<'a>, picture: &Picture) -> Option<&'a Value> {
    ctx.store.get(ARENA_MAP_FEED).filter(|map| {
        seated(map) && map.get("match").and_then(Value::as_i64) == Some(picture.match_id)
    })
}

/// **The picture's map, composed** (16b cut I.2; shared with 16b cut I.4's
/// live picture): the window on `focus`, the terrain, the spots, then `units`
/// — `(marker, callsign, cell)` in drawing order, a unit with no cell not
/// drawn — the heading, and the owner-rule legend. See [`map_rows`].
fn compose_map(
    ctx: &RenderCtx<'_>,
    map: &Value,
    heading_text: &str,
    focus: Option<(usize, usize)>,
    units: &[MapUnit],
) -> PaneRows {
    let width = cell_num(map, "width");
    let height = cell_num(map, "height");
    let empty = Vec::new();
    let rows = map.get("rows").and_then(Value::as_array).unwrap_or(&empty);
    let (grid_w, grid_h, heading, with_legend) = match ctx.pane {
        Some((w, h)) if h > MAP_CHROME_ROWS + 1 => (w, h - MAP_CHROME_ROWS, true, true),
        Some((w, h)) if h == MAP_CHROME_ROWS + 1 => (w, h - 1, false, true),
        Some((w, h)) => (w, h.max(1), false, false),
        None => (COCKPIT_MAP_FALLBACK.0, COCKPIT_MAP_FALLBACK.1, true, true),
    };
    let (x0, y0, w, h) = mush_ansi::viewport(
        width,
        height,
        focus,
        usize::from(grid_w),
        usize::from(grid_h),
    );
    let mut grid = mush_ansi::window_rows(rows, width, height, x0, y0, w, h);
    // Which marker wrote each window cell LAST: a lookup only, never iterated,
    // so the legend's order is the drawing's and nothing else.
    let mut owner: HashMap<(usize, usize), Owner> = HashMap::new();
    let mut spots_landed: Vec<(usize, char, String, (usize, usize))> = Vec::new();
    let empty_spots = Vec::new();
    let spots = map
        .get("spots")
        .and_then(Value::as_array)
        .unwrap_or(&empty_spots);
    for (index, spot) in spots.iter().enumerate() {
        let Some(letter) = spot
            .get("letter")
            .and_then(Value::as_str)
            .and_then(one_char)
        else {
            continue;
        };
        let Some(cell) = cell_pair(spot.get("cell")) else {
            continue;
        };
        if let Some(cell_at) = put_marker(&mut grid, (x0, y0), cell, letter) {
            owner.insert(cell_at, Owner::Spot(index));
            let name = clean(spot.get("name").and_then(Value::as_str).unwrap_or(""));
            spots_landed.push((index, letter, name, cell_at));
        }
    }
    let mut landed: Vec<(usize, char, (usize, usize))> = Vec::new();
    for (index, (glyph, _, cell)) in units.iter().enumerate() {
        let Some(cell) = *cell else {
            continue;
        };
        if let Some(cell_at) = put_marker(&mut grid, (x0, y0), cell, *glyph) {
            owner.insert(cell_at, Owner::Unit(index));
            landed.push((index, *glyph, cell_at));
        }
    }
    let mut legend: Vec<String> = landed
        .iter()
        .filter(|(index, _, cell_at)| owner.get(cell_at) == Some(&Owner::Unit(*index)))
        .map(|(index, glyph, _)| {
            let callsign = units.get(*index).map_or("?", |(_, name, _)| name.as_str());
            format!("{glyph} {callsign}")
        })
        .collect();
    legend.extend(
        spots_landed
            .iter()
            .filter(|(index, _, _, cell_at)| owner.get(cell_at) == Some(&Owner::Spot(*index)))
            .map(|(_, letter, name, _)| format!("{letter} {name}")),
    );
    let mut out = Vec::with_capacity(h + 2);
    if heading {
        out.push(PaneRow::plain(Line::from(Span::styled(
            heading_text.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ))));
    }
    for y in (0..h).rev() {
        let row: String = grid.get(y).map(|r| r.iter().collect()).unwrap_or_default();
        out.push(PaneRow::plain(Line::from(Span::raw(row))));
    }
    if with_legend {
        let width = ctx.pane.map(|(w, _)| usize::from(w));
        out.push(PaneRow::plain(Line::from(dim_span_owned(&fit_legend(
            &legend, width,
        )))));
    }
    PaneRows { rows: out }
}

/// **The status pane of a replay** (16b cut I.2): what match, which keyframe,
/// around whom, what the positions are, whose they are and what was cut — each
/// sentence at or under the Left pane's 32 columns, so none wraps at the floor:
///
/// ```text
/// Match 12  red won (blue elim.)
/// Keyframe 19/55  1:30 of 4:32
/// Focus RA  (u next)
/// Arena positions, every 5 s.
/// Both sides shown: match over.
/// Cut: first 61:20 kept.
/// ```
///
/// The last is one line per `cut` word, only when the replay was cut: `Cut:
/// first 61:20 kept.` (the keyframes, or a `truncated` document naming no
/// cause), `Cut: shots to 12:34 kept.`, `Cut: a page of tape lost.` and `Cut:
/// first 24 units kept.`. `None` when no picture is open or its document is not
/// held.
#[must_use]
pub fn status_rows(ctx: &RenderCtx<'_>) -> Option<PaneRows> {
    if is_live(ctx) {
        return live_status_rows(ctx);
    }
    let (picture, doc) = held(ctx)?;
    let at = at_of(picture, &doc);
    let roster = doc.roster();
    let outcome = doc.doc.get("outcome").and_then(Value::as_str).unwrap_or("");
    let reason = doc.doc.get("reason").and_then(Value::as_str).unwrap_or("");
    let mut lines = vec![
        format!(
            "Match {}  {}",
            picture.match_id,
            outcome_short(outcome, reason)
        ),
        format!(
            "Keyframe {}/{}  {} of {}",
            at.saturating_add(1),
            doc.keyframe_count(),
            clock_text(doc.keyframe_t(at)),
            clock_text(doc.end_ms())
        ),
        format!(
            "Focus {}  (u next)",
            callsign_of(&roster, usize::from(picture.focus))
        ),
        format!("Arena positions, every {}.", every_words(doc.summary_ms())),
        "Both sides shown: match over.".to_string(),
    ];
    let cuts = doc.cuts();
    if doc.truncated() && cuts.is_empty() {
        lines.push(first_kept(&doc));
    }
    for cut in cuts {
        lines.push(match cut {
            "keyframes" => first_kept(&doc),
            "shots" => format!(
                "Cut: shots to {} kept.",
                clock_text(doc.shots().last().map_or(0, |shot| shot.0))
            ),
            "page" => "Cut: a page of tape lost.".to_string(),
            "roster" => format!("Cut: first {} units kept.", roster.len()),
            _ => continue,
        });
    }
    let mut rows: Vec<PaneRow> = lines
        .into_iter()
        .map(|line| PaneRow::plain(Line::from(Span::raw(line))))
        .collect();
    if let Some(first) = rows.first_mut() {
        first.line = Line::from(Span::styled(
            first.text(),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    }
    Some(PaneRows { rows })
}

/// **The status pane of a live picture** (16b cut I.4), four sentences at or
/// under the Left pane's 32 columns:
///
/// ```text
/// Match 12  engaged  floor 5000
/// Live, at most once a second.
/// In flight: 2
/// @arena/watch off stops it.
/// ```
///
/// `floor` only while a staff floor stands. `None` when no picture of the live
/// match is held.
fn live_status_rows(ctx: &RenderCtx<'_>) -> Option<PaneRows> {
    let (picture, live) = held_live(ctx)?;
    let mut head = format!("Match {}  {}", picture.match_id, live.tempo());
    if let Some(floor) = live.floor_ms() {
        head.push_str(&format!("  floor {floor}"));
    }
    let mut rows = vec![PaneRow::plain(Line::from(Span::styled(
        head,
        Style::default().add_modifier(Modifier::BOLD),
    )))];
    for line in [
        LIVE_CADENCE.to_string(),
        format!("In flight: {}", live.in_flight()),
        LIVE_STOP.to_string(),
    ] {
        rows.push(PaneRow::plain(Line::from(Span::raw(line))));
    }
    Some(PaneRows { rows })
}

/// `Cut: first 61:20 kept.` — the replay holds its keyframes to the last one's
/// time.
fn first_kept(doc: &Document<'_>) -> String {
    let last = doc.keyframe_count().saturating_sub(1);
    format!("Cut: first {} kept.", clock_text(doc.keyframe_t(last)))
}

/// **The damage pane of a replay** (16b cut I.2): `1:25  BB wrecked` for every
/// wreck recorded by the keyframe's horizon, NEWEST first; `Nothing yet.` when
/// there is none. `None` when no picture's document is held.
#[must_use]
pub fn damage_rows(ctx: &RenderCtx<'_>) -> Option<PaneRows> {
    // 16b cut I.4: a live picture carries no damage.
    if is_live(ctx) {
        return held_live(ctx).map(|_| PaneRows::one(NOTHING_YET));
    }
    let (picture, doc) = held(ctx)?;
    let horizon = doc.horizon(at_of(picture, &doc));
    let roster = doc.roster();
    let mut wrecks: Vec<(i64, usize)> = doc
        .destroyed()
        .into_iter()
        .filter(|(when, _)| *when <= horizon)
        .collect();
    if wrecks.is_empty() {
        return Some(PaneRows::one(NOTHING_YET));
    }
    wrecks.reverse();
    Some(PaneRows {
        rows: wrecks
            .iter()
            .map(|(when, at)| {
                PaneRow::plain(Line::from(Span::raw(format!(
                    "{}  {} wrecked",
                    clock_text(*when),
                    callsign_of(&roster, *at)
                ))))
            })
            .collect(),
    })
}

/// **The weapons pane of a replay** (16b cut I.2): the shots fired after the
/// previous keyframe's time and at or before this keyframe's horizon, NEWEST
/// first, as `1:27  RA at BA: hit` (`in flight` for one still flying when the
/// tape ended); `Nothing yet.` when none. `None` when no picture's document is
/// held.
#[must_use]
pub fn weapons_rows(ctx: &RenderCtx<'_>) -> Option<PaneRows> {
    // 16b cut I.4: nor any shot but the count in flight.
    if is_live(ctx) {
        return held_live(ctx).map(|_| PaneRows::one(NOTHING_YET));
    }
    let (picture, doc) = held(ctx)?;
    let at = at_of(picture, &doc);
    let horizon = doc.horizon(at);
    let since = at.checked_sub(1).map(|before| doc.keyframe_t(before));
    let roster = doc.roster();
    let mut shots: Vec<(i64, usize, usize, &str)> = doc
        .shots()
        .into_iter()
        .filter(|(t, ..)| since.is_none_or(|since| *t > since) && *t <= horizon)
        .collect();
    if shots.is_empty() {
        return Some(PaneRows::one(NOTHING_YET));
    }
    shots.reverse();
    Some(PaneRows {
        rows: shots
            .iter()
            .map(|(t, shooter, target, word)| {
                PaneRow::plain(Line::from(Span::raw(format!(
                    "{}  {} at {}: {}",
                    clock_text(*t),
                    callsign_of(&roster, *shooter),
                    callsign_of(&roster, *target),
                    shot_words(word)
                ))))
            })
            .collect(),
    })
}

/// **The contacts pane of a replay** (16b cut I.2): every roster unit at the
/// keyframe, in roster order —
///
/// ```text
/// RA   red   (120, 97)  090°
/// BB   blue  wreck
/// BA   blue  off the window
/// ```
///
/// — its cell and heading, `wreck` for a unit wrecked by the horizon, `off the
/// window` for one the keyframe does not hold. Every row is SELECTABLE with the
/// callsign as its key and NO prefill: `Enter` on it moves the picture's focus
/// (a client action, `TuiApp`'s), never opens the box with a line. `None` when
/// no picture's document is held.
#[must_use]
pub fn contacts_rows(ctx: &RenderCtx<'_>) -> Option<PaneRows> {
    if is_live(ctx) {
        return live_contacts_rows(ctx);
    }
    let (picture, doc) = held(ctx)?;
    let at = at_of(picture, &doc);
    let horizon = doc.horizon(at);
    let rows = doc
        .roster()
        .iter()
        .enumerate()
        .map(|(index, unit)| {
            let rest = if doc.wrecked_by(index, horizon) {
                "wreck".to_string()
            } else {
                match doc.slot(at, index) {
                    Some((x, y, heading)) => {
                        format!("({x}, {y})  {heading:03}{}", degree(ctx.utf8))
                    }
                    None => "off the window".to_string(),
                }
            };
            PaneRow {
                line: Line::from(vec![
                    Span::styled(
                        format!("{:<4} ", unit.callsign),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(format!("{:<5} {rest}", unit.side())),
                ]),
                prefill: None,
                row_key: Some(unit.callsign.clone()),
                selectable: true,
            }
        })
        .collect();
    Some(PaneRows { rows })
}

/// **The contacts pane of a live picture** (16b cut I.4): every unit, in the
/// picture's order —
///
/// ```text
/// RA   red   (120, 97) 090° #1234 machine wreck
/// BA   blue  off the window #302 pilot held
/// ```
///
/// — its cell and heading (or `off the window`), its object number (a staff
/// surface), `machine` or `pilot`, then one state word ([`LiveUnit::words`]).
/// **One line a unit at the 120-column floor** (the owner's width ruling): the
/// contacts pane is 46 columns inside there, and with one space between fields
/// and one state word a row of a three-digit cell and a four-digit object
/// number is whole as `machine`, `pilot`, `wreck` or `held`; a row wider than
/// the pane it is drawn into — `starting` on such a row, which lasts the
/// `startup_ms` of a start-up (ten seconds by default), or a five-digit number
/// — is cut at the pane's edge with an ellipsis instead of wrapping, which
/// would spend two lines on it and halve the units the pane shows. Where the pane's width is not known (the input paths ask
/// what the rows ARE) the row is whole. Every row is selectable with the
/// callsign as its key and no prefill: `Enter` moves the focus. `None` when no
/// picture is held.
fn live_contacts_rows(ctx: &RenderCtx<'_>) -> Option<PaneRows> {
    let (_, live) = held_live(ctx)?;
    let width = ctx.pane.map(|(width, _)| usize::from(width));
    let rows = live
        .units()
        .into_iter()
        .map(|unit| {
            let place = match unit.cell {
                Some((x, y)) => format!("({x}, {y}) {:03}{}", unit.heading, degree(ctx.utf8)),
                None => "off the window".to_string(),
            };
            let head = format!("{:<4} ", unit.callsign);
            let mut rest = format!("{:<5} {place} #{} {}", unit.side(), unit.id, unit.words());
            if let Some(width) = width {
                let room = width.saturating_sub(head.chars().count());
                if rest.chars().count() > room {
                    let ellipsis = if ctx.utf8 { "\u{2026}" } else { "..." };
                    let keep = room.saturating_sub(ellipsis.chars().count());
                    rest = format!("{}{ellipsis}", truncate_chars(&rest, keep));
                }
            }
            PaneRow {
                line: Line::from(vec![
                    Span::styled(head, Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(rest),
                ]),
                prefill: None,
                row_key: Some(unit.callsign.clone()),
                selectable: true,
            }
        })
        .collect();
    Some(PaneRows { rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **16b cut I.4**: a live picture's rows read totally — a null cell is off
    /// the window — and the flags make the controller word and the one state
    /// word that weighs most. (What the panes print is pinned as rendered below.)
    #[test]
    fn i4_a_live_picture_reads_its_rows_and_its_words() {
        let data = serde_json::json!({
            "schema": { "id": "arena.observer", "version": 1, "retain": "latest" },
            "match": 12, "now_ms": 90_000, "tempo": "engaged", "floor_ms": null,
            "in_flight": 2,
            "units": [["RA", "R", 120, 97, 90, 301, "pm"], ["BA", "B", null, null, 270, 302, "wsh"]],
        });
        let live = Live::of(&data).expect("a picture");
        assert_eq!(live.match_id(), 12);
        let units = live.units();
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].cell, Some((120, 97)));
        assert_eq!(units[0].glyph(), RED_GLYPH);
        assert_eq!(units[0].words(), "machine");
        assert_eq!(units[1].cell, None);
        assert_eq!(units[1].glyph(), WRECK_GLYPH);
        assert_eq!(units[1].words(), "pilot wreck");
        assert_eq!(live.unit_index("BA"), Some(1));
        assert!(Live::of(&serde_json::json!({ "watching": false })).is_none());
        // One state word, the first that applies in the order of weight.
        let words = |flags: &str| {
            LiveUnit {
                callsign: "RA".into(),
                blue: false,
                cell: None,
                heading: 0,
                id: 1,
                flags: flags.into(),
            }
            .words()
        };
        assert_eq!(words(""), "pilot");
        assert_eq!(words("m"), "machine");
        assert_eq!(words("sm"), "machine starting");
        assert_eq!(words("phm"), "machine held");
        assert_eq!(words("wsh"), "pilot wreck");
    }

    /// The live panes as `TuiApp::draw_pane` renders them, into a pane of
    /// `pane` columns: every row's text.
    fn live_pane_text(
        data: &Value,
        render: fn(&RenderCtx<'_>) -> Option<PaneRows>,
        pane: Option<(u16, u16)>,
    ) -> Vec<String> {
        let theme = mush_ansi::AnsiTheme::classic();
        let mut store = super::super::panels::PanelStore::new();
        store.apply_panel(
            &mush_proto::SmolStr::new_static(OBSERVER_FEED),
            mush_proto::PanelFrameKind::Keyframe,
            data,
            0,
        );
        let picture = Picture {
            match_id: data["match"].as_i64().unwrap_or(0),
            at: 0,
            focus: 0,
            live: true,
        };
        let ctx = RenderCtx {
            theme: &theme,
            depth: mush_proto::ColorDepth::Ansi16,
            utf8: true,
            store: &store,
            selected_channel: None,
            pane,
            picture: Some(&picture),
        };
        render(&ctx)
            .expect("the live picture is held")
            .rows
            .iter()
            .map(|row| {
                row.line
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            })
            .collect()
    }

    /// **16b cut I.4 (`T3`)**: the status pane AS RENDERED, at the widest head
    /// it can print — a four-digit match, the longest tempo word, a ten-second
    /// floor, a `u32` of rounds in flight — is four sentences, each inside the
    /// Left pane's 32 columns.
    #[test]
    fn i4_the_live_status_rows_fit_the_left_pane_at_their_widest() {
        let data = serde_json::json!({
            "schema": { "id": "arena.observer", "version": 1, "retain": "latest" },
            "match": 9999, "now_ms": 3_600_000, "tempo": "engaged", "floor_ms": 10_000,
            "in_flight": u32::MAX, "units": [],
        });
        let rows = live_pane_text(&data, status_rows, Some((32, 20)));
        assert_eq!(
            rows,
            [
                "Match 9999  engaged  floor 10000".to_string(),
                LIVE_CADENCE.to_string(),
                format!("In flight: {}", u32::MAX),
                LIVE_STOP.to_string(),
            ]
        );
        for row in &rows {
            assert!(row.chars().count() <= 32, "{row:?}");
        }
    }

    /// **16b cut I.4 (`R3`)**: a contacts row of realistic figures — the 256-cell
    /// ground's three-digit cells, a four-digit object number — is WHOLE in the
    /// 46 columns the contacts pane has at the 120-column floor as `wreck`,
    /// `held` or no state word; a wider one (`starting` there, or a five-digit
    /// number) is cut at the pane's edge with an ellipsis and never wraps; where
    /// the width is not known the row is whole.
    #[test]
    fn i4_a_live_contacts_row_is_one_line_in_the_pane_at_the_floor() {
        let data = serde_json::json!({
            "schema": { "id": "arena.observer", "version": 1, "retain": "latest" },
            "match": 12, "now_ms": 90_000, "tempo": "engaged", "floor_ms": null,
            "in_flight": 0,
            "units": [
                ["RA", "R", 255, 255, 359, 1234, "sm"],
                ["RB", "R", 255, 255, 90, 1235, "pwm"],
                ["BA", "B", 255, 199, 270, 9876, "phm"],
                ["BB", "B", null, null, 0, 9877, "s"],
                ["BC", "B", 255, 255, 359, 12_345, "s"],
                ["BD", "B", 255, 255, 359, 1236, "pm"],
            ],
        });
        let rows = live_pane_text(&data, contacts_rows, Some((46, 20)));
        assert_eq!(
            rows,
            [
                "RA   red   (255, 255) 359\u{b0} #1234 machine star\u{2026}",
                "RB   red   (255, 255) 090\u{b0} #1235 machine wreck",
                "BA   blue  (255, 199) 270\u{b0} #9876 machine held",
                "BB   blue  off the window #9877 pilot starting",
                "BC   blue  (255, 255) 359\u{b0} #12345 pilot start\u{2026}",
                "BD   blue  (255, 255) 359\u{b0} #1236 machine",
            ]
        );
        for row in &rows {
            assert!(row.chars().count() <= 46, "{row:?}");
        }
        let whole = live_pane_text(&data, contacts_rows, None);
        assert_eq!(
            whole.get(4).map(String::as_str),
            Some("BC   blue  (255, 255) 359\u{b0} #12345 pilot starting")
        );
    }

    /// Every status sentence the table can print, at the longest match id a
    /// four-digit centre reaches, sits inside the Left pane's 32 columns.
    #[test]
    fn every_outcome_fits_the_left_pane_at_four_digits() {
        for (outcome, reason, _) in OUTCOME_WORDS {
            let line = format!("Match 9999  {}", outcome_short(outcome, reason));
            assert!(line.chars().count() <= 32, "{line:?}");
        }
        assert_eq!(outcome_short("red", "elimination"), "red won (blue elim.)");
        assert_eq!(outcome_short("abort", "abort"), "aborted");
        // 16b cut O.1: a hold's words.
        assert_eq!(outcome_short("red", "hold"), "red won (held spot)");
        assert_eq!(outcome_short("blue", "hold"), "blue won (held spot)");
        // A pair not in the table prints its outcome word alone.
        assert_eq!(outcome_short("draw", "stalemate"), "draw");
        for line in [
            "Keyframe 736/736  61:20 of 60:00",
            "Arena positions, every 100 ms.",
            "Both sides shown: match over.",
            "Cut: first 61:20 kept.",
            "Cut: shots to 59:59 kept.",
            "Cut: a page of tape lost.",
            "Cut: first 24 units kept.",
            "Focus ABCDEFGH  (u next)",
        ] {
            assert!(line.chars().count() <= 32, "{line:?}");
        }
    }

    /// **16b cut O.1 (plan item 39)**: the replay map's heading carries ` [hold]`
    /// for a match fought under the Hold rule, nothing for a skirmish or for a
    /// document with no `mode` (every replay before the cut), and stays inside
    /// the replay map's 39 columns at its widest.
    #[test]
    fn o1_the_replay_heading_carries_the_mode_tag() {
        assert_eq!(
            replay_heading(12, 90_000, 18, 55, "hold", true),
            "Replay 12 \u{2014} 1:30 (19/55) [hold]"
        );
        assert_eq!(
            replay_heading(12, 90_000, 18, 55, "hold", false),
            "Replay 12 - 1:30 (19/55) [hold]"
        );
        assert_eq!(
            replay_heading(12, 90_000, 18, 55, "skirmish", true),
            "Replay 12 \u{2014} 1:30 (19/55)"
        );
        assert_eq!(mode_tag(""), "");
        assert_eq!(mode_tag("skirmish"), "");
        assert_eq!(mode_tag("hold"), " [hold]");
        let widest = replay_heading(9_999, 3_680_000, 735, 736, "hold", true);
        assert!(widest.chars().count() <= 39, "{widest:?}");
        // The decoder: an absent key reads a skirmish, a present one its word.
        let mut doc = serde_json::json!({
            "schema": { "shape": "replay" }, "match": 12, "outcome": "red", "reason": "hold",
        });
        assert_eq!(
            Document::of(&doc).map(|doc| doc.mode()).as_deref(),
            Some("skirmish")
        );
        doc["mode"] = serde_json::json!("hold");
        assert_eq!(
            Document::of(&doc).map(|doc| doc.mode()).as_deref(),
            Some("hold")
        );
    }

    #[test]
    fn the_clock_and_the_cadence_read_as_line_mode_reads_them() {
        assert_eq!(clock_text(90_000), "1:30");
        assert_eq!(clock_text(3_680_000), "61:20");
        assert_eq!(clock_text(-5), "0:00");
        assert_eq!(every_words(5_000), "5 s");
        assert_eq!(every_words(250), "250 ms");
    }

    /// The horizon: a keyframe's own time, except the last of a whole replay,
    /// which runs to the end — and a `truncated` document with no cause reads
    /// as cut at its keyframes.
    #[test]
    fn the_last_keyframe_of_a_whole_replay_answers_to_the_end() {
        let doc = |cut: Value, truncated: bool| {
            serde_json::json!({
                "schema": { "id": "arena.replay", "version": 1, "shape": "replay" },
                "match": 3, "end_ms": 12_000, "truncated": truncated, "cut": cut,
                "roster": [{ "callsign": "RA", "team": "red" }],
                "keyframes": [[0, [[1, 1, 0]]], [10_000, [null]]],
            })
        };
        let whole = doc(serde_json::json!([]), false);
        let read = Document::of(&whole).expect("a document");
        assert_eq!(read.horizon(0), 0);
        assert_eq!(read.horizon(1), 12_000);
        assert_eq!(read.slot(1, 0), None);
        assert_eq!(read.slot(0, 0), Some((1, 1, 0)));
        let sealed = doc(serde_json::json!(["keyframes"]), true);
        assert_eq!(Document::of(&sealed).map(|d| d.horizon(1)), Some(10_000));
        let unnamed = doc(serde_json::json!([]), true);
        assert_eq!(Document::of(&unnamed).map(|d| d.horizon(1)), Some(10_000));
        // Neither the grid nor `available: false` is a document.
        assert!(
            Document::of(&serde_json::json!({
                "schema": { "shape": "grid" }, "match": 3
            }))
            .is_none()
        );
        assert!(
            Document::of(&serde_json::json!({
                "schema": { "shape": "replay" }, "available": false, "reason": "x"
            }))
            .is_none()
        );
    }
}
