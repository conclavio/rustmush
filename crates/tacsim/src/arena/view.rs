//! **The view** (17b cut M1a.2; doc 17 §3 "One formatter", D-UM-4) — the pure
//! projections of a [`Frame`] that a pilot's `CONTACTS`, the panel rows and —
//! from 17b cut M1a.5 — a mind's `MindView` all read through, so the sentence a
//! pilot reads and the figure a mind reads are one function and cannot drift.
//!
//! Doc 17 §3's parity rule is that a mind reads exactly what the cockpit
//! keyframe carries. Until this cut the row formatter lived in
//! `crate::match_::cockpit`, the world half's module, where nothing inside the
//! arena could call it; a mind reading its rows through a copy would have been
//! the second formatter doc 17 forbids. So [`frame_contacts_held`],
//! [`contact_band`], [`ContactRow`] and [`own_scope`] moved here, with what they
//! call ([`fired_by_default`], `class_word`, `relative_mdeg`,
//! [`held_range_m`]), the lock reads ([`frame_lock`], [`lock_designator`]) and
//! the four `VIEW` glyphs, bodies and doc comments verbatim.
//! `crate::match_::cockpit` re-exports every one of them under its old path,
//! so `CONTACTS`, the panel rows, the bridge and the bot's contract test name
//! the one function they always named. The range steps
//! ([`CONTACT_RANGE_STEP_M`], `lock_range_step_m`) already lived in
//! [`crate::arena::perception`] and stay there; the **sentences** built on
//! these rows — the brief's `contact_sentence`, `lock_line`, `notice_line`,
//! `watch_lines` — stay in the cockpit, because a sentence is the world half's.
//!
//! **Pure over a [`Frame`] and a `BTreeMap<ContactId, u64>` range memory**: no
//! world, no clock, no store, no task. It is a file of the arena library, so
//! `crates/mush-contract-tests/tests/arena_library.rs` scans it by
//! construction.
//!
//! **One rounding, two memories.** A scope row's range is held against the
//! figure its reader was last given ([`held_range_m`], 16b cut B.4, `T-60`).
//! The pilot's memory lives on the world mirror
//! ([`CockpitFrame::ranges`](crate::arena_bridge::CockpitFrame::ranges)) and
//! settles once per frame sent ([`settle_ranges`]); a mind's twin
//! (`MindState.ranges`, 17b cut M1a.5) settles once per think. The rounding is
//! this one function; the cadence of its memory differs by design.
//!
//! `crates/tacsim/tests/cockpit.rs::m1a2_the_rows_render_as_before` pins the
//! move: the fixture scene's four `CONTACTS` rows against literals read before
//! it, and the brief's contact sentences against `view_brief.txt`.
//!
//! **17b cut M1a.3** (doc 17 §3, D-UM-5) grew the row by two columns — the
//! aspect's side word ([`ContactRow::aspect`]) and the main gun's odds
//! ([`ContactRow::odds`], an [`OddsCell`]) — so the `CONTACTS` literals of that
//! test are now the rows with their tails; the brief's sentence did not move.

use std::collections::BTreeMap;

use smol_str::SmolStr;

use crate::arena::perception::CONTACT_RANGE_STEP_M;
use crate::arena::state::{Classification, ContactId, Designator};
use crate::arena::step::Frame;
use crate::arena::weapons::{DEFAULT_WEAPON, FrameWeapon};

/// Moved from `match_::cockpit` by 17b cut M1a.2.
///
/// The glyph the pilot's own platform draws with.
pub const YOU_GLYPH: &str = "@";

/// Moved from `match_::cockpit` by 17b cut M1a.2.
///
/// **The glyph an unresolved contact draws with** on `VIEW` (16b cut S.2; doc 16
/// §3.6: *a marker per contact*). A return, not a tank: the pilot knows something
/// is there and nothing else.
pub const UNKNOWN_GLYPH: &str = "?";

/// Moved from `match_::cockpit` by 17b cut M1a.2.
///
/// A contact resolved to the platform's **own** side (doc 16 §3.6: *teammates
/// appear when shared*).
pub const FRIEND_GLYPH: &str = "+";

/// Moved from `match_::cockpit` by 17b cut M1a.2.
///
/// A contact resolved to the **other** side.
pub const HOSTILE_GLYPH: &str = "x";

/// Moved from `match_::cockpit` by 17b cut M1a.2.
///
/// **The contacts the platform holds**, as a pilot is told about them (doc 16
/// §3.6 "What a pilot sees").
///
/// **16b cut S.1 filled the arena side** — the sensor pass mints the ids and
/// mirrors them into [`Frame::contacts`] — and **this is cut S.2's read of it**:
/// the row key unchanged and never printed, the short designator 16b cut Q.1 gave
/// a pilot to read and type, the class as `unknown` or as the side the platform
/// has resolved, the bearing in whole degrees both absolute and relative to the
/// platform's own heading, the range as one of [`contact_band`]'s four words
/// (16b cut D.1: the pilot's word, thirds of the lesser of the platform's own
/// sight and the fired weapon's reach), and the age in whole seconds.
///
/// Three things a pilot is deliberately **not** given, and each is the observer
/// rule on this surface: the target's object number, its name, and its team while
/// the contact is still `unknown`. The row carries the contact's last-seen
/// **cell** because that is what `VIEW` draws a marker in, which is the map's own
/// resolution and no finer.
///
/// **16b cut F.3** changed what the numbers are *of*. Every figure here —
/// bearing, range, band, cell — is read off the **plot** the arena recorded at
/// the sighting that last refreshed the contact, so a row is what the platform
/// knows and never where the target now is; and `age_ms` is the age of that
/// sighting, which is how a pilot tells a live track from a dying one. It also
/// adds the range in metres ([`ContactRow::range_m`]), because a band alone told
/// a pilot nothing at all.
///
/// **16b cut B.4** (`T-60`): this is [`frame_contacts_held`] with no memory — every
/// row rounded afresh. The fixtures and every pure reading of a lone frame use it;
/// the world's read verbs and the bridge pass the mirror's held ranges.
#[must_use]
pub fn frame_contacts(frame: &Frame) -> Vec<ContactRow> {
    frame_contacts_held(frame, &BTreeMap::new())
}

/// Moved from `match_::cockpit` by 17b cut M1a.2.
///
/// **The contacts the platform holds, each range held against the last figure the
/// pilot was told** (16b cut B.4, `T-60`).
///
/// [`frame_contacts`]' rows, with each row's `range_m` rounded through
/// [`held_range_m`] against `ranges` — the figure the same row carried on the
/// previous frame the pilot was sent ([`settle_ranges`], stored on the bridge's
/// mirror as [`CockpitFrame::ranges`]). A row the map does not name rounds plainly.
///
/// [`CockpitFrame::ranges`]: crate::arena_bridge::CockpitFrame::ranges
#[must_use]
pub fn frame_contacts_held(frame: &Frame, ranges: &BTreeMap<ContactId, u64>) -> Vec<ContactRow> {
    // 16b cut F.3: the bands of the weapon a bare `FIRE` would fire — the weapon
    // the band word is advice about. Resolved once for the list.
    let default = fired_by_default(&frame.weapons);
    let bands: &[u32] = default.map_or(&[], |weapon| &weapon.bands_cm);
    frame
        .contacts
        .iter()
        .map(|held| ContactRow {
            id: held.id,
            // 16b cut Q.1: the short name and the number behind it, both as the
            // arena resolved them off the live contact.
            designator: held.designator,
            alias: held.alias,
            class: SmolStr::new(class_word(held.class)),
            bearing_deg: held.bearing_mdeg / 1_000,
            relative_deg: relative_mdeg(frame.pose.heading_mdeg, held.bearing_mdeg) / 1_000,
            // 16b cut D.1: thirds of the lesser of the sight and the weapon's
            // reach. 16b cut E.1: and the only band word there is.
            band: SmolStr::new(contact_band(held.range_cm, bands, frame.sight_cm)),
            // 16b cut B.4 (`T-60`): held against the last figure the pilot was
            // told, so a plot sitting near a 50 m edge does not flip every pulse.
            range_m: held_range_m(
                held.range_cm,
                CONTACT_RANGE_STEP_M,
                ranges.get(&held.id).copied(),
            ),
            age_ms: held.age_ms,
            cell: held.cell,
            friendly: held.relation.is_friendly(),
            relation: held.relation,
            // 17b cut M1a.3 (doc 17 §3): the side word of the face the plot was
            // showing, and the main gun's odds off `frame.odds` — the entry for
            // this row and the weapon a bare `FIRE` would fire, which
            // `Arena::frame_odds` puts first.
            aspect: SmolStr::new(held.aspect.side_word()),
            odds: match default {
                None => OddsCell::Unarmed,
                Some(weapon) => frame
                    .odds
                    .iter()
                    .find(|odds| odds.contact == held.id && odds.weapon == weapon.id)
                    .map_or(OddsCell::Dash, |odds| {
                        OddsCell::Odds(odds.weapon.clone(), odds.p_permille)
                    }),
            },
        })
        .collect()
}

/// **The odds cell a `CONTACTS` row ends in** (17b cut M1a.3; doc 17 §3).
///
/// Three shapes, because a dash and nothing are different statements: a dash
/// is an armed hull's fire control not resolving the contact (an unclassified
/// return, or one past the nearest `odds_rows`), and nothing is a hull with no
/// gun to resolve with — *an unarmed hull's row ends at the aspect*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OddsCell {
    /// The hull carries no weapon: the row prints nothing after the aspect.
    Unarmed,
    /// An armed hull's row with no odds: the row prints `-`.
    Dash,
    /// The main gun's chance: its id and the per mille, printed as
    /// `{weapon} {percent}%`.
    Odds(SmolStr, u32),
}

/// Moved from `match_::cockpit` by 17b cut M1a.2.
///
/// A contact's range in metres, rounded to the nearest `step` (16b cut F.3;
/// 16b cut Q.1 made the step an argument). Half a step rounds up.
///
/// The step is the caller's because there are two: [`CONTACT_RANGE_STEP_M`] for
/// a row on the scope, [`LOCK_RANGE_STEP_M`] for the one contact the platform is
/// locked on.
///
/// **The answer is never nought** (16b cut D.1). Rounding to the nearest step put
/// everything inside half a step at `0 m` — under 25 m on a scope row, under 5 m
/// on a lock — and `0 m` is the one distance a contact cannot be at: a platform
/// that close is being rammed. It floors at one step, so the coarsest thing the
/// row can say about a contact on top of you is *one step away*, which is what a
/// coarse reading is entitled to say.
///
/// [`LOCK_RANGE_STEP_M`]: crate::arena::perception::LOCK_RANGE_STEP_M
#[must_use]
pub fn range_m_rounded(range_cm: u64, step_m: u64) -> u64 {
    let step = step_m.max(1);
    let metres = range_cm / 100;
    metres
        .saturating_add(step / 2)
        .saturating_div(step)
        .saturating_mul(step)
        .max(step)
}

/// Moved from `match_::cockpit` by 17b cut M1a.2.
///
/// **A scope row's range, held against the figure the pilot was last told** (16b
/// cut B.4, `T-60`), metres.
///
/// [`range_m_rounded`] alone is pure, so a plot sitting at 425 m — a tank holding
/// station, or two tanks circling — read `450 m` and `400 m` on alternate pulses,
/// and every panel row and every `CONTACTS` flickered with it. With `held` the
/// figure the same row was last published at, the row keeps that figure while the
/// plot stays within five eighths of a step of it — **six metres of hysteresis each
/// side of the edge at the 50 m step** (an edge at 425 m is crossed at 431.25 m
/// going out and at 418.75 m coming back) — and rounds afresh once it leaves. A
/// row with no memory (`None`) rounds plainly, so the first figure a pilot reads
/// about a new contact is [`range_m_rounded`]'s.
///
/// Integers only, as every stored figure is: the tolerance is
/// `step_m × 100 × 5 / 8` centimetres.
#[must_use]
pub fn held_range_m(range_cm: u64, step_m: u64, held: Option<u64>) -> u64 {
    if let Some(m) = held {
        let tolerance_cm = step_m.saturating_mul(100).saturating_mul(5) / 8;
        if range_cm.abs_diff(m.saturating_mul(100)) <= tolerance_cm {
            return m;
        }
    }
    range_m_rounded(range_cm, step_m)
}

/// Moved from `match_::cockpit` by 17b cut M1a.2.
///
/// **The range memory the next frame is held against** (16b cut B.4, `T-60`):
/// every contact on `frame`, mapped to the figure its row publishes now
/// ([`held_range_m`] against `previous`). A contact that is gone from the frame
/// is gone from the map, so a re-acquired id starts afresh.
///
/// The bridge calls it once per frame per pilot and stores the answer on the
/// mirror ([`CockpitFrame::ranges`]); it is a fact about what the pilot has been
/// told, never about where anything is.
///
/// [`CockpitFrame::ranges`]: crate::arena_bridge::CockpitFrame::ranges
#[must_use]
pub fn settle_ranges(
    frame: &Frame,
    previous: &BTreeMap<ContactId, u64>,
) -> BTreeMap<ContactId, u64> {
    frame
        .contacts
        .iter()
        .map(|held| {
            (
                held.id,
                held_range_m(
                    held.range_cm,
                    CONTACT_RANGE_STEP_M,
                    previous.get(&held.id).copied(),
                ),
            )
        })
        .collect()
}

/// Moved from `match_::cockpit` by 17b cut M1a.2.
///
/// **What this platform's own scope holds** (16b cut D.1; doc 16 §5, §1).
///
/// `STATUS`'s third line ended `; the arena is running {tempo}` — the arena's one
/// **global** word, raised when anybody anywhere on the map holds a contact. A
/// pilot sitting in a wreck with an empty scope read `engaged`, which is a fact
/// about somebody else's fight and exactly the shape of thing doc 16 §1 says a
/// platform has to earn. The three words are read off this platform's own contact
/// list and nothing else:
///
/// * `hostile` — it holds a contact classified to another team;
/// * `unidentified` — it holds a contact that is not a classified friend;
/// * `clear` — it holds nothing, or nothing but classified friends.
///
/// The arena's `tempo` stays on the frame, because a panel's own timers read the
/// cadence off it.
#[must_use]
pub fn own_scope(frame: &Frame) -> &'static str {
    let mut unidentified = false;
    for held in &frame.contacts {
        // Doc 32 cut CF.0: by relation. A neutral is held, not a threat, and
        // not a friend: it reads as unidentified does.
        match held.relation {
            crate::arena::party::Relation::Own | crate::arena::party::Relation::Friend => {}
            crate::arena::party::Relation::Hostile => return "hostile",
            crate::arena::party::Relation::Neutral | crate::arena::party::Relation::Unknown => {
                unidentified = true;
            }
        }
    }
    if unidentified {
        "unidentified"
    } else {
        "clear"
    }
}

/// Moved from `match_::cockpit` by 17b cut M1a.2.
///
/// The word a class renders as: `unknown`, or the **side** the platform has
/// resolved (doc 16 §3.6: *class or `unknown`*).
///
/// Not `Classification::as_str`, which answers the *state* (`classified`): what a
/// pilot wants off a resolved return is whose it is.
#[must_use]
pub(crate) fn class_word(class: Classification) -> &'static str {
    match class.party() {
        Some(team) => team.as_str(),
        None => class.as_str(),
    }
}

/// Moved from `match_::cockpit` by 17b cut M1a.2.
///
/// A bearing read off the platform's own nose: the absolute bearing less the
/// platform's heading, wrapped into `0..360_000` millidegrees.
#[must_use]
pub(crate) fn relative_mdeg(heading_mdeg: u32, bearing_mdeg: u32) -> u32 {
    let turn = i64::from(bearing_mdeg) - i64::from(heading_mdeg);
    u32::try_from(turn.rem_euclid(360_000)).unwrap_or(0)
}

/// Moved from `match_::cockpit` by 17b cut M1a.2.
///
/// **The range band** (16b cut D.1; doc 16 §3.6's *range band*, read against what
/// this platform can actually see).
///
/// **There is one band word, and this is it** (16b cut E.1, 16c `T-81`). Cut D.1
/// left a second — `range_band`, thirds of the *weapon's* innermost band, which
/// the commander's brief and an order's ceiling read — so a lead's `close` was
/// 500 m and a pilot's was 200 m. Cut E.1 deleted it; this function is what the
/// contact row, the acquisition notice, the squad brief and
/// [`crate::deliberation::order::RangeBand::ceiling_cm`] (its inverse, pinned by
/// a test) all say.
///
/// `CONTACTS` could only ever print two of its four words. The stock cannon's
/// bands are 1.5 km and 3 km, so a word banded on the gun alone put `close` at
/// 500 m and `effective` at 1,500 m — while the shipped sight is 60 cells of
/// 1,000 cm, 600 m, and one mesh hop adds 300 m. The furthest plot any shipped
/// platform can hold is 900 m, so from acquisition to the kill a contact read
/// `effective` and then `close`, and `far` and `beyond` could not print at all.
///
/// So the word is banded against **the lesser of the sight and the weapon's last
/// band**, which on every shipped platform is the sight. That makes it **scope
/// advice**: how far across your own sight the contact is — `close` the nearest
/// third, `effective` the middle third, `far` the outer third and anything a
/// teammate is passing you from beyond it. It is a deliberate change of what the
/// word *answers*, not a side effect: a contact at 450 m, which the cannon makes
/// comfortably, reads `far`. What the gun makes of the distance is the lock's
/// `% to hit` and `WEAPONSTATUS`, and `docs/help/cockpit.md` says so in the
/// pilot's own terms.
///
/// The rule, in order:
///
/// * no bands at all, or `range_cm` past the last band: `beyond` — still literally
///   true, because nothing the platform fires reaches it, **and the honest word
///   for a platform that carries nothing that fires**;
/// * otherwise `limit` is `min(last band, sight_cm)`, and the last band alone when
///   `sight_cm` is `0` (a platform with no sensor, which holds no contacts anyway);
/// * `range_cm × 3 ≤ limit`: `close`; else `range_cm × 3 ≤ limit × 2`:
///   `effective`; else `far`.
///
/// On the stock tank that is `close` to 200 m, `effective` to 400 m, `far` beyond
/// — and a mesh plot past the platform's own sight is `far`.
#[must_use]
pub fn contact_band(range_cm: u64, bands_cm: &[u32], sight_cm: u32) -> &'static str {
    let Some(last) = bands_cm.last() else {
        return "beyond";
    };
    let last = u64::from(*last);
    if range_cm > last {
        return "beyond";
    }
    let sight = u64::from(sight_cm);
    let limit = if sight == 0 { last } else { last.min(sight) };
    if range_cm.saturating_mul(3) <= limit {
        "close"
    } else if range_cm.saturating_mul(3) <= limit.saturating_mul(2) {
        "effective"
    } else {
        "far"
    }
}

/// **The metres [`contact_band`]'s three words cut at, for one weapon on one
/// frame** (17b cut M1b.4; D-UM-18's `band = close|effective|far` term), as
/// inclusive `(nearest, furthest)` pairs in whole metres — `close`,
/// `effective`, `far` — or `None` for a weapon with no bands, which no band
/// word describes.
///
/// The same limit `contact_band` reads — the lesser of the weapon's last band
/// and the sight (`sight_cm`; the last band alone at `0`) — cut in thirds on
/// the metre figure a slot's range term is tested against (the ROUNDED range,
/// [`range_m_rounded`]): `r` is `close` while `3r ≤ limit`, `effective` while
/// `3r ≤ 2 × limit`, and `far` from there to the weapon's last band, past which
/// `contact_band` says `beyond`. So the stock tank's 600 m sight and the
/// cannon's 3,000 m reach give `(0, 200)`, `(201, 400)`, `(401, 3000)`. A
/// pilot's `FIRE … WHEN band = effective` is compiled through this to FIXED
/// metres off the frame it was typed on.
#[must_use]
pub fn band_bounds_m(sight_cm: u32, bands_cm: &[u32]) -> Option<[(u32, u32); 3]> {
    let last = u64::from(*bands_cm.last()?);
    let sight = u64::from(sight_cm);
    let limit = if sight == 0 { last } else { last.min(sight) };
    let metres = |cm: u64| u32::try_from(cm).unwrap_or(u32::MAX);
    let close = metres(limit / 300);
    let effective = metres(limit.saturating_mul(2) / 300);
    let reach = metres(last / 100);
    Some([
        (0, close),
        (close.saturating_add(1), effective),
        (effective.saturating_add(1), reach),
    ])
}

/// Moved from `match_::cockpit` by 17b cut M1a.2.
///
/// One contact, as a pilot is told about it (doc 16 §3.6 "What a pilot sees").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactRow {
    /// The opaque arena id. **16b cut Q.1**: the stable row key a panel
    /// re-identifies a row by, never printed and never typed — see
    /// [`ContactRow::designator`] and [`resolve_designator`].
    ///
    /// [`resolve_designator`]: crate::match_::cockpit::resolve_designator
    pub id: ContactId,
    /// **The short name the pilot reads and types** (16b cut Q.1): `BA` while the
    /// contact is classified now, `X2` while it is not.
    pub designator: Designator,
    /// **The provisional number this row's target still answers to** (16b cut
    /// Q.1), where the row is naming it by slot and a number was issued. It is
    /// what makes `LOCK X2` keep working after the rename.
    pub alias: Option<u16>,
    /// `unknown`, or the side the platform has resolved.
    pub class: SmolStr,
    /// Bearing, degrees clockwise from north.
    pub bearing_deg: u32,
    /// The same bearing off the platform's own nose, degrees clockwise (16b cut
    /// S.2): `0` is dead ahead.
    pub relative_deg: u32,
    /// The range band's label ([`contact_band`], which since 16b cut E.1 is the
    /// one band word a pilot, a commander and an order all read).
    pub band: SmolStr,
    /// **The range to the plot, metres**, rounded to [`CONTACT_RANGE_STEP_M`]
    /// (16b cut F.3) — see [`range_m_rounded`] for why a passive sight reports one
    /// at all.
    pub range_m: u64,
    /// **How stale the plot is**, milliseconds of arena time (16b cut F.3): now
    /// less the sighting that last refreshed the contact, this platform's own or
    /// the sharer's. Cut S.2 printed the hold time since first detection, which
    /// carried no staleness signal at all.
    pub age_ms: i64,
    /// The cell it was last seen in, for `VIEW`'s marker (16b cut S.2).
    pub cell: Option<(u16, u16)>,
    /// Whether the resolved side is the platform's own — what decides the glyph
    /// `VIEW` draws it with. `false` while the contact is `unknown`.
    pub friendly: bool,
    /// **The platform's party's relation to it** (doc 32 Q-CF-12): what the
    /// clients colour it by. `friendly` is own or friend.
    pub relation: crate::arena::party::Relation,
    /// **The side word of the face the contact was showing** (17b cut M1a.3; doc
    /// 17 §3): `front`, `side` or `rear` —
    /// [`crate::arena::perception::Aspect::side_word`] of the frame's facet.
    pub aspect: SmolStr,
    /// **The main gun's odds against this row** (17b cut M1a.3; doc 17 §3), or
    /// the dash, or nothing on an unarmed hull — see [`OddsCell`].
    pub odds: OddsCell,
}

impl ContactRow {
    /// The line `CONTACTS` prints for this row.
    ///
    /// **16b cut F.3** put the range in it, between the band and the age, and
    /// **16b cut Q.1** put the short designator where the sixteen digits were,
    /// left-aligned in a column of four so `X2` and `BA` line up with `RAA`:
    /// `BA  blue  bearing 090° (rel 000°)  close  300 m  1 s`.
    ///
    /// **17b cut M1a.3** (doc 17 §3) added two columns after the age: the
    /// aspect's side word, then on an armed hull the main gun's odds or a dash —
    /// `BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  side  cannon 62%`,
    /// `…  2 s  side  -` — and on an unarmed hull nothing more: `…  2 s  side`.
    #[must_use]
    pub fn render(&self) -> String {
        // Rendered first: `Designator`'s own `Display` writes straight to the
        // formatter, so a width on it would be ignored.
        let named = self.designator.to_string();
        let odds = match &self.odds {
            OddsCell::Unarmed => String::new(),
            OddsCell::Dash => String::from("  -"),
            OddsCell::Odds(weapon, permille) => format!("  {weapon} {}%", permille / 10),
        };
        format!(
            "{named:<4}{}  bearing {:03}° (rel {:03}°)  {}  {} m  {} s  {}{odds}",
            self.class,
            self.bearing_deg,
            self.relative_deg,
            self.band,
            self.range_m,
            self.age_ms / 1_000,
            self.aspect,
        )
    }

    /// The glyph `VIEW` draws this contact with: [`UNKNOWN_GLYPH`] while it is
    /// unresolved, then [`FRIEND_GLYPH`] or [`HOSTILE_GLYPH`].
    #[must_use]
    pub fn glyph(&self) -> &'static str {
        // Doc 32 cut CF.0 (Q-CF-12): by relation; a neutral reads as an
        // unknown does, one slot for the two.
        match self.relation {
            crate::arena::party::Relation::Own | crate::arena::party::Relation::Friend => {
                FRIEND_GLYPH
            }
            crate::arena::party::Relation::Hostile => HOSTILE_GLYPH,
            crate::arena::party::Relation::Neutral | crate::arena::party::Relation::Unknown => {
                UNKNOWN_GLYPH
            }
        }
    }
}

/// Moved from `match_::cockpit` by 17b cut M1a.2.
///
/// **The lock the platform holds**, as the frame carries it (16b cut W.1 filled
/// it): always one of [`frame_contacts`]'s rows, because the arena clears a lock the
/// moment its contact stops being held.
#[must_use]
pub fn frame_lock(frame: &Frame) -> Option<ContactId> {
    frame.lock
}

/// Moved from `match_::cockpit` by 17b cut M1a.2.
///
/// **The designator of the contact the platform is locked on** (16b cut Q.1), or
/// `None` with no lock.
///
/// Read off the frame's own contact list rather than carried beside the lock, so
/// the name in `WEAPONSTATUS`'s lock line, in `RANGE`'s answer and in the row
/// `CONTACTS` prints is one string resolved once.
#[must_use]
pub fn lock_designator(frame: &Frame) -> Option<Designator> {
    let contact = frame_lock(frame)?;
    frame
        .contacts
        .iter()
        .find(|held| held.id == contact)
        .map(|held| held.designator)
}

/// Moved from `match_::cockpit` by 17b cut M1a.2.
///
/// **The weapon a bare `FIRE` would fire** (16b cut F.2): [`DEFAULT_WEAPON`], else
/// the first the platform carries — the same choice `choose_weapon` makes for an
/// empty argument, which is what makes `LOCK`'s percentage the percentage of the
/// shot the pilot is about to take.
#[must_use]
pub fn fired_by_default(weapons: &[FrameWeapon]) -> Option<&FrameWeapon> {
    weapons
        .iter()
        .find(|weapon| weapon.id.as_str() == DEFAULT_WEAPON)
        .or_else(|| weapons.first())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::perception::LOCK_RANGE_STEP_M;

    /// One row, as `match_::cockpit`'s tests build it: the fields `render` reads.
    fn row(id: ContactId, designator: Designator, alias: Option<u16>) -> ContactRow {
        ContactRow {
            id,
            designator,
            alias,
            class: SmolStr::new_static("blue"),
            bearing_deg: 87,
            relative_deg: 32,
            band: SmolStr::new_static("effective"),
            range_m: 450,
            age_ms: 2_000,
            cell: Some((4, 4)),
            friendly: false,
            aspect: SmolStr::new_static("side"),
            odds: OddsCell::Odds(SmolStr::new_static("cannon"), 624),
        }
    }

    // Moved from match_::cockpit by 17b cut M1a.2; proves what it proved.
    /// The row `CONTACTS` prints, and the column the designator sits in.
    ///
    /// **17b cut M1a.3**: with the aspect and the odds after the age.
    #[test]
    fn a_contact_row_renders_its_designator_in_a_column_of_four() {
        let slot = Designator::Slot {
            team: crate::arena::state::PartyId::BLUE,
            slot: 1,
        };
        assert_eq!(
            row(ContactId(7), slot, Some(2)).render(),
            "BA  blue  bearing 087° (rel 032°)  effective  450 m  2 s  side  cannon 62%"
        );
        let mut unknown = row(ContactId(7), Designator::Provisional(12), None);
        unknown.class = SmolStr::new_static("unknown");
        unknown.odds = OddsCell::Dash;
        assert_eq!(
            unknown.render(),
            "X12 unknown  bearing 087° (rel 032°)  effective  450 m  2 s  side  -"
        );
    }

    /// **17b cut M1a.3** (doc 17 §3): the row's three tails — doc 17's own pin
    /// with the odds, an armed hull's dash, and an unarmed hull's row that ends
    /// at the aspect.
    #[test]
    fn m1a3_the_row_ends_in_the_aspect_and_the_main_guns_odds() {
        let slot = Designator::Slot {
            team: crate::arena::state::PartyId::BLUE,
            slot: 1,
        };
        let mut far = row(ContactId(7), slot, None);
        far.band = SmolStr::new_static("far");
        assert_eq!(
            far.render(),
            "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  side  cannon 62%"
        );
        far.odds = OddsCell::Dash;
        assert_eq!(
            far.render(),
            "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  side  -"
        );
        far.odds = OddsCell::Unarmed;
        assert_eq!(
            far.render(),
            "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  side"
        );
        // The percent is the whole per cent, rounded down as every percent is.
        far.odds = OddsCell::Odds(SmolStr::new_static("mg"), 9);
        far.aspect = SmolStr::new_static("rear");
        assert!(far.render().ends_with("  2 s  rear  mg 0%"));
    }

    // Moved from match_::cockpit by 17b cut M1a.2; proves what it proved.
    /// **16b cut Q.1**: two steps, one rounding rule. A lock earns ten metres and
    /// the rest of the scope fifty; half a step rounds up either way.
    #[test]
    fn a_range_rounds_to_its_own_step_and_a_lock_earns_the_finer_one() {
        assert_eq!(CONTACT_RANGE_STEP_M, 50);
        assert_eq!(LOCK_RANGE_STEP_M, 10);
        // 434 m: 450 at fifty, 430 at ten.
        assert_eq!(range_m_rounded(43_400, CONTACT_RANGE_STEP_M), 450);
        assert_eq!(range_m_rounded(43_400, LOCK_RANGE_STEP_M), 430);
        // Half a step up.
        assert_eq!(range_m_rounded(2_500, CONTACT_RANGE_STEP_M), 50);
        assert_eq!(range_m_rounded(500, LOCK_RANGE_STEP_M), 10);
        // A step of zero is read as one rather than dividing by it.
        assert_eq!(range_m_rounded(43_400, 0), 434);
    }

    // Moved from match_::cockpit by 17b cut M1a.2; proves what it proved.
    /// **16b cut B.4** (`T-60`): a scope row's range holds against the figure
    /// the pilot was last told while the plot stays within five eighths of a
    /// step of it — six metres of hysteresis each side of a 50 m edge — and
    /// rounds afresh once it leaves; with no memory it rounds plainly.
    #[test]
    fn a_held_range_keeps_its_figure_across_a_fifty_metre_edge() {
        let step = CONTACT_RANGE_STEP_M;
        // 424 m, no memory: 400, as `range_m_rounded` says.
        assert_eq!(held_range_m(42_400, step, None), 400);
        assert_eq!(held_range_m(43_100, step, None), 450, "plain rounding");
        // 431 m with 400 held: inside 431.25, so it holds.
        assert_eq!(held_range_m(43_100, step, Some(400)), 400);
        // Exactly at the tolerance holds; one centimetre past it does not.
        assert_eq!(held_range_m(43_125, step, Some(400)), 400);
        assert_eq!(held_range_m(43_126, step, Some(400)), 450);
        // 432 m: past it, so it rounds afresh to 450.
        assert_eq!(held_range_m(43_200, step, Some(400)), 450);
        // Coming back: 418 m from 450 is past the tolerance, and rounds to 400;
        // 419 m still holds 450.
        assert_eq!(held_range_m(41_800, step, Some(450)), 400);
        assert_eq!(held_range_m(41_900, step, Some(450)), 450);
        // A held figure far from the plot is simply replaced.
        assert_eq!(held_range_m(12_000, step, Some(450)), 100);
    }
}
