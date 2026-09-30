//! The pilot's picture of the match (cut AJ.1): every line a seated line-mode
//! pilot is told, folded into one structure.
//!
//! [`Picture::apply`] takes one classified [`Line`] and says whether it belongs
//! in the bot's journal; [`Picture::settle`] commits a reply whose last row has
//! gone quiet. Nothing here reads anything but the pilot's own session text, and
//! the numbers each rule uses come from the player help:
//!
//! * a `CONTACTS` reply is the platform's **whole** list ("`CONTACTS` is your
//!   platform's own picture and nothing else"), so a contact missing from the
//!   next reply is gone;
//! * a row's range is rounded to 50 m and held, its bearing is whole degrees,
//!   and it is where the contact **was last seen** ("A contact is drawn where it
//!   was last seen, not where it is now, and the age is how stale that is"), so
//!   every fix carries an uncertainty and a track needs real motion to show;
//! * the lock reads finer ("a fresh one reads to 10 m"), so a bare `RANGE` on
//!   the lock read just after its row refines that row's fix;
//! * every `FIRE` refusal reads `FIRE <weapon> refused. …` whichever half
//!   answered ("Every `FIRE` refusal names the weapon, whichever half
//!   answered"), so a refusal is paired with an outstanding `Firing` ack before
//!   it is allowed to un-count a shot — and one that answers a `FIRE` the
//!   runner reported ([`Picture::sent_fire`]) is that `FIRE`'s own pre-check,
//!   never paired.
//!
//! **The fire clock** (cut AK.1). [`Weapon::next_ready_ms`] is the one clock a
//! gun's readiness is read from; `None` means "no evidence yet: read the row".
//! Two facts set it, both read from the bot's own live transcript (cut AJ's
//! smoke), not from anything of the server's:
//!
//! * the seconds in `cycling (N s)` and `Still cycling: N s to go.` are
//!   **rounded up** — twenty cannon refusals 1.00 to 1.25 s after that gun's
//!   ack read `6 s to go` on a 6 s gun, where a truncated figure would say 5 —
//!   so `N s` is an upper bound, counted whole;
//! * the real cycle begins **up to a pulse after the ack** — fourteen refusals
//!   6.0 to 6.5 s after a 6 s gun's ack still read `1 s` — which is the help's
//!   "you and the arena are a pulse apart" (`help cockpit`, "Refusals").
//!
//! So the ack sets the clock to the ack plus the learned cycle plus a
//! per-weapon slack ([`Weapon::slack_ms`]) whose floor is one engaged pulse
//! ([`FIRE_SLACK_MIN_MS`]); a `cycling` row or a `Still cycling` refusal sets it
//! to its own reading time plus the whole seconds plus the slack; an accepted
//! `ready` row sets it to the row's own time (the row is the evidence); and an
//! arena refusal of an acked shot that is not `Still cycling` (the shot never
//! left) sets it to now. The cockpit's own pre-check refusal of a `FIRE` this
//! runner typed raises the slack; every ack lowers it toward the floor.
//!
//! **The observer rule, bot side.** The picture holds only designators the
//! stream named; it never invents a contact, a side or a section, and no line but
//! a contact row or a contact notice ever adds a contact.
use std::collections::{BTreeMap, VecDeque};

use serde::Serialize;

pub use super::lines::{HeldCause, Power, ResultWord, Scope};
use super::{
    Side,
    lines::{ContactRow, Line, WeaponRow, is_centre_room},
};

/// How long an open reply may sit after its last row before [`Picture::settle`]
/// commits it.
pub const SNAPSHOT_SETTLE_MS: u64 = 150;
/// Fixes kept per contact.
pub const MAX_FIXES: usize = 12;
/// Events kept in the ring.
pub const MAX_EVENTS: usize = 32;
/// Outstanding `Firing` acks kept per weapon.
pub const MAX_ACKS: usize = 8;
/// A `FIRE <w> refused.` pairs with an ack of `w` at most this old.
pub const ACK_PAIR_MS: u64 = 2_000;
/// A `ready` row read this soon after an ack may be the pre-shot frame.
pub const READY_AFTER_ACK_MS: u64 = 1_500;
/// A bare `RANGE` on the lock refines a row's fix read at most this long before.
pub const LOCK_FIX_MS: u64 = 400;
/// Tracks read fixes from this window only.
pub const TRACK_WINDOW_MS: u64 = 10_000;
/// A track needs fixes spanning at least this long…
pub const TRACK_SPAN_MS: u64 = 4_000;
/// …and at least this many of them…
pub const TRACK_MIN_FIXES: usize = 4;
/// …and a fitted speed of at least this, metres a second.
pub const TRACK_MIN_MPS: f64 = 3.0;
/// The widest line [`Picture::summary_lines`] writes (the terminal-width floor).
pub const SUMMARY_COLUMNS: usize = 120;
/// A weapon's slack before any ack or refusal has moved it (cut AK.1).
pub const FIRE_SLACK_START_MS: u64 = 500;
/// The slack grows this much on each `Still cycling` answer to a `FIRE` this
/// runner typed (the cockpit's own pre-check: the clock was early)…
pub const FIRE_SLACK_UP_MS: u64 = 250;
/// …and shrinks this much on each ack…
pub const FIRE_SLACK_DOWN_MS: u64 = 25;
/// …never below one engaged pulse. The real cycle begins up to a pulse after
/// the ack (the transcript's still-cycling refusals 6.0 to 6.5 s after a 6 s
/// gun's ack), and an additive loop that sits above its floor settles where
/// refusals are about DOWN / (UP + DOWN) of the `FIRE` lines (some 9 %), so
/// the floor itself must already cover the ack-to-pulse delay: the slack then
/// rests at the floor and rises only on a real miss.
pub const FIRE_SLACK_MIN_MS: u64 = 500;
/// …and never above this.
pub const FIRE_SLACK_MAX_MS: u64 = 1_500;
const _: () =
    assert!(FIRE_SLACK_MIN_MS <= FIRE_SLACK_START_MS && FIRE_SLACK_START_MS <= FIRE_SLACK_MAX_MS);

/// Where the pilot is in the simulator's life cycle.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Nothing read yet.
    #[default]
    Unknown,
    /// Logged in, outside the centre.
    Walking,
    /// In the lobby, a team room or a pod, holding no slot the lane knows of.
    InCentre,
    /// Holding a slot ([`Picture::claimed`]).
    Claimed { side: Side, index: u8 },
    /// The followed match is boarding.
    Boarding { match_id: u64 },
    /// In the pod, the followed match not yet running.
    Seated { match_id: u64 },
    /// The followed match is running.
    Running { match_id: u64 },
    /// The followed match is over.
    Over { match_id: u64, result: ResultWord },
}

impl Phase {
    fn words(&self) -> String {
        match self {
            Phase::Unknown => "unknown".into(),
            Phase::Walking => "walking".into(),
            Phase::InCentre => "in the centre".into(),
            Phase::Claimed { side, index } => format!("claimed {} {index}", side.as_str()),
            Phase::Boarding { match_id } => format!("boarding (match {match_id})"),
            Phase::Seated { match_id } => format!("seated (match {match_id})"),
            Phase::Running { match_id } => format!("running (match {match_id})"),
            Phase::Over { match_id, result } => {
                format!("over (match {match_id}, {})", result.as_str())
            }
        }
    }
}

/// One section of the own tank, as `STATUS` and the victim's lines print it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Section {
    pub condition: String,
    pub internals: Option<(i64, i64)>,
    /// Facet → `(armour left, armour max)`.
    pub facets: BTreeMap<String, (i64, i64)>,
}

/// The own tank.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Own {
    pub callsign: Option<String>,
    /// The tank's name, `STATUS`'s second line (`Red Tank 1`).
    pub name: Option<String>,
    /// The bracketed chassis, when it is not the pod's own.
    pub chassis: Option<String>,
    pub power: Power,
    /// Negative astern.
    pub speed_kph: i32,
    pub heading_deg: u32,
    /// Metres, +x east and +y north (`STATUS`'s `at {x} m east, {y} m north`).
    pub pos_m: Option<(f64, f64)>,
    /// When `pos_m` was read.
    pub pos_read_ms: u64,
    pub held: Option<HeldCause>,
    pub destroyed: bool,
    pub disabled: bool,
    pub immobile: bool,
    pub sections: BTreeMap<String, Section>,
    pub scope: Scope,
    /// `SPEED`'s own ceilings, from its usage line.
    pub max_ahead_kph: Option<i32>,
    pub max_astern_kph: Option<i32>,
    /// `(heat, max)` on a platform that carries heat.
    pub heat: Option<(u32, u32)>,
    pub overheated: bool,
    /// The receiver is under interference.
    pub jammed: bool,
    /// The jamming pod's switch, on a tank that carries one.
    pub emcon: Option<bool>,
}

/// One fix on a contact: where its plot put it, and how far to trust that.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Fix {
    /// When the plot was taken (the read less the printed age, less half a
    /// second for the rounding).
    pub at_ms: u64,
    pub pos_m: (f64, f64),
    pub sigma_m: f64,
    /// Refined by a bare `RANGE` on the lock.
    pub from_lock: bool,
}

/// A contact's fitted motion.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Track {
    /// Clockwise from north.
    pub course_deg: f64,
    pub speed_mps: f64,
}

/// One contact the platform holds, keyed by its current designator.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Contact {
    pub designator: String,
    /// The name it had before its last rename.
    pub was: Option<String>,
    /// `None` until a row or a notice resolves it.
    pub side: Option<Side>,
    pub bearing_deg: u32,
    pub rel_deg: u32,
    pub band: String,
    /// `None` for a contact known only from its `New contact` notice.
    pub range_m: Option<u32>,
    /// The plot's age when last read.
    pub age_ms: u64,
    /// When it was last read.
    pub read_ms: u64,
    pub fixes: VecDeque<Fix>,
    pub track: Option<Track>,
}

impl Contact {
    fn new(designator: &str, now_ms: u64) -> Contact {
        Contact {
            designator: designator.to_owned(),
            was: None,
            side: None,
            bearing_deg: 0,
            rel_deg: 0,
            band: String::new(),
            range_m: None,
            age_ms: 0,
            read_ms: now_ms,
            fixes: VecDeque::new(),
            track: None,
        }
    }

    fn push_fix(&mut self, fix: Fix) {
        self.fixes.push_back(fix);
        while self.fixes.len() > MAX_FIXES {
            self.fixes.pop_front();
        }
    }

    /// When the plot behind the last row was taken, by the rule every row fix
    /// uses.
    fn row_plot_ms(&self) -> u64 {
        self.read_ms.saturating_sub(self.age_ms.saturating_add(500))
    }
}

/// One weapon of the own tank.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Weapon {
    pub id: String,
    pub short: Option<String>,
    pub ready: bool,
    pub cycling_s: Option<u32>,
    pub rounds: u32,
    pub rounds_max: u32,
    pub band: Option<(u32, u32)>,
    pub percent: Option<u32>,
    /// When its row was last read; `0` for a weapon known only from an ack.
    pub read_ms: u64,
    /// The last `Firing` ack not un-counted by a paired refusal.
    pub fired_ms: Option<u64>,
    /// The shortest cycle learned from `cycling` rows (an upper bound).
    pub cycle_ms: Option<u64>,
    /// **The one readiness clock** (cut AK.1): ready at this time; `None` when
    /// nothing has set it yet, and then the row's own `ready` is read (see the
    /// module head).
    pub next_ready_ms: Option<u64>,
    /// What the clock adds to a cycle for the pulse between the pilot and the
    /// arena ([`FIRE_SLACK_START_MS`] for a weapon this picture made; see
    /// [`FIRE_SLACK_MIN_MS`]).
    pub slack_ms: u64,
    /// `Firing` acks not yet paired with a refusal, oldest first.
    pub acks: VecDeque<u64>,
    /// `Firing` acks since this weapon's last row was applied, less the ones a
    /// paired refusal un-counted: the rounds a row must be short of the last
    /// row's figure before it shows every one of them has left.
    pub shots_since_read: u32,
}

impl Weapon {
    /// Every weapon the picture holds is made here, so each starts with its
    /// slack at [`FIRE_SLACK_START_MS`] (`Default` gives 0).
    fn new(id: &str) -> Weapon {
        Weapon {
            id: id.to_owned(),
            slack_ms: FIRE_SLACK_START_MS,
            ..Weapon::default()
        }
    }

    /// Whether `name` (as a `FIRE` line, an ack or a refusal named it) is this
    /// weapon: its id or its short name.
    fn named(&self, name: &str) -> bool {
        self.id == name || self.short.as_deref() == Some(name)
    }
}

/// A named spot of the ground, as `RANGE`, `BEARING` and `STATUS` read it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Spot {
    pub name: String,
    pub range_m: Option<u32>,
    pub bearing_deg: Option<u32>,
    pub read_ms: u64,
    /// Where the spot lies, whole metres east and north (cut AJ.2): worked out
    /// only from a `STATUS` block that read the tank's own position and this
    /// spot as the nearest together, so its range and bearing were taken from
    /// the same place. A `RANGE` or `BEARING` answer changes one figure of the
    /// pair at a time and never moves it; the ground does not move either.
    pub plotted_m: Option<(i64, i64)>,
}

/// What the pilot's own shooting and the tank's own damage came to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Tally {
    /// `Firing` acks.
    pub shots: u32,
    /// Refusals paired with an ack: shots that never left.
    pub refused_shots: u32,
    /// Refusals with no ack to pair: the cockpit's own pre-check, or any other
    /// order refused.
    pub refused_orders: u32,
    pub hits_scored: u32,
    pub misses: u32,
    pub kills: u32,
    pub hits_taken: u32,
    pub sections_lost: Vec<String>,
}

impl Tally {
    /// The shots the report counts: acks less the refusals paired with them.
    pub fn fired(&self) -> u32 {
        self.shots.saturating_sub(self.refused_shots)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    ContactNew,
    Identified,
    Unidentified,
    Lost,
    LockBroken,
    Locked,
    HitScored,
    Missed,
    Kill,
    TargetSectionLost,
    HitTaken,
    SectionLost,
    OwnDestroyed,
    OwnDisabled,
    Held,
    PowerOnline,
    Refused,
    PhaseChanged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Event {
    pub at_ms: u64,
    pub kind: EventKind,
    pub text: String,
}

/// What one [`Picture::apply`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    /// Whether the line belongs in the journal (`false` for read replies, acks,
    /// `OUTSIDE_COCKPIT`, transients, and a lock line that tells nothing new).
    pub journal: bool,
    pub event: Option<Event>,
}

/// An open `CONTACTS` reply: rows with the time each was read.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ContactsReply {
    pub rows: Vec<(ContactRow, u64)>,
    pub last_ms: u64,
}

/// An open `WEAPONSTATUS` reply.
#[derive(Clone, Debug, Default, Serialize)]
pub struct WeaponsReply {
    /// A lock line came immediately before the first row.
    pub locked: bool,
    pub rows: Vec<(WeaponRow, u64)>,
    pub last_ms: u64,
}

/// An open `STATUS` block: its sections, staged until it closes.
#[derive(Clone, Debug, Default, Serialize)]
pub struct StatusBlock {
    pub sections: BTreeMap<String, Section>,
    pub last_ms: u64,
    /// The block's own position row, for the nearest spot's row after it.
    pub pos_m: Option<(f64, f64)>,
}

/// The replies being read now.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Open {
    pub contacts: Option<ContactsReply>,
    pub weapons: Option<WeaponsReply>,
    pub status: Option<StatusBlock>,
    /// The line before this one was a lock line.
    pub lock_line: bool,
    /// The section the next facet run belongs to.
    pub section: Option<String>,
    /// A `Back aboard …` block is being read: its `  Power {w}.` row counts.
    /// Set by the block's first line and cleared by the first line that is
    /// neither one of its rows (`Other`) nor that row.
    pub catchup: bool,
    /// Each `FIRE` the runner wrote ([`Picture::sent_fire`]): the weapon it
    /// named and when it went out, oldest first, while its own answer —
    /// `Firing {w}.` or the cockpit's pre-check `FIRE {w} refused. …` — has not
    /// been read. An answer takes the oldest mark **of its own weapon** (cut
    /// AK.1). A mark lapses after [`ACK_PAIR_MS`].
    pub fire_sent: VecDeque<(String, u64)>,
}

/// The pilot's picture of the match.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Picture {
    pub phase: Phase,
    /// The match this picture follows: the first `Match {n} is boarding` after
    /// [`Picture::claimed`]. `Seated`, `Running` and `Over` move only on lines
    /// about it.
    pub following: Option<u64>,
    pub room: Option<String>,
    pub side: Option<Side>,
    /// The character's own display name, from `Connected as`.
    pub me: Option<String>,
    /// In the cockpit of a tank.
    pub aboard: bool,
    /// Tanks taken back after a drop, this match.
    pub reclaims: u32,
    /// How long the last drop lasted.
    pub away_ms: u64,
    /// When `Power online.` first came this match (cut AJ.3: the commander's
    /// picture counts its seconds from it).
    pub power_ms: Option<u64>,
    pub own: Own,
    pub contacts: BTreeMap<String, Contact>,
    pub lock: Option<String>,
    pub lock_friendly: bool,
    pub lock_percent: Option<u32>,
    pub lock_read_ms: u64,
    pub weapons: Vec<Weapon>,
    pub spots: BTreeMap<char, Spot>,
    pub tally: Tally,
    pub events: VecDeque<Event>,
    /// Bumped on every change.
    pub revision: u64,
    /// The latest `now_ms` [`Picture::apply`] or [`Picture::settle`] was given.
    pub last_ms: u64,
    /// The replies being read now.
    pub open: Open,
}

fn is_status_form(line: &Line) -> bool {
    matches!(
        line,
        Line::StatusCallsign { .. }
            | Line::StatusHead(_)
            | Line::StatusPosition { .. }
            | Line::StatusNearestSpot { .. }
            | Line::StatusHeat { .. }
            | Line::StatusReceiver { .. }
            | Line::StatusEmcon { .. }
            | Line::StatusSection { .. }
            | Line::StatusFacets { .. }
    )
}

/// The bearing from `from` to `to`, degrees clockwise from north in `0..360`
/// (+x east, +y north).
pub fn bearing_deg(from: (f64, f64), to: (f64, f64)) -> f64 {
    (to.0 - from.0)
        .atan2(to.1 - from.1)
        .to_degrees()
        .rem_euclid(360.0)
}

/// Metres between two points.
pub fn distance_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

/// The raw least-squares fit over a contact's fixes inside the track window,
/// before any gate is applied.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fit {
    vx: f64,
    vy: f64,
    speed_mps: f64,
    fixes: usize,
    span_ms: u64,
    mean_sigma: f64,
}

/// The least-squares velocity over the fixes of the last [`TRACK_WINDOW_MS`],
/// weighted by each fix's `1/σ²`; `None` only when they do not span two
/// distinct times.
fn fit(fixes: &VecDeque<Fix>, now_ms: u64) -> Option<Fit> {
    let floor = now_ms.saturating_sub(TRACK_WINDOW_MS);
    let recent: Vec<&Fix> = fixes.iter().filter(|fix| fix.at_ms >= floor).collect();
    let first = recent.iter().map(|fix| fix.at_ms).min()?;
    let last = recent.iter().map(|fix| fix.at_ms).max()?;
    let span_ms = last - first;
    let (mut sw, mut st, mut sx, mut sy) = (0.0, 0.0, 0.0, 0.0);
    for fix in &recent {
        let w = 1.0 / fix.sigma_m.max(1.0).powi(2);
        let t = (fix.at_ms - first) as f64 / 1_000.0;
        sw += w;
        st += w * t;
        sx += w * fix.pos_m.0;
        sy += w * fix.pos_m.1;
    }
    let (tbar, xbar, ybar) = (st / sw, sx / sw, sy / sw);
    let (mut stt, mut stx, mut sty) = (0.0, 0.0, 0.0);
    for fix in &recent {
        let w = 1.0 / fix.sigma_m.max(1.0).powi(2);
        let dt = (fix.at_ms - first) as f64 / 1_000.0 - tbar;
        stt += w * dt * dt;
        stx += w * dt * (fix.pos_m.0 - xbar);
        sty += w * dt * (fix.pos_m.1 - ybar);
    }
    if stt <= f64::EPSILON {
        return None;
    }
    let (vx, vy) = (stx / stt, sty / stt);
    Some(Fit {
        vx,
        vy,
        speed_mps: vx.hypot(vy),
        fixes: recent.len(),
        span_ms,
        mean_sigma: recent.iter().map(|fix| fix.sigma_m).sum::<f64>() / recent.len() as f64,
    })
}

/// A contact's track: the [`fit`], published only when it is motion the fixes
/// can carry. Four gates, each able to refuse on its own: at least
/// [`TRACK_MIN_FIXES`] fixes, spanning at least [`TRACK_SPAN_MS`], a speed of at
/// least [`TRACK_MIN_MPS`], and a fitted displacement across the span of at
/// least three times the fixes' mean `σ` (so a held 50 m scope figure that
/// steps, or a stopped machine read from a moving tank, does not fit as motion).
/// With the fixes this picture makes (`σ` at least 10 m) and the window's
/// 10 s, the displacement gate already implies the speed floor; the floor is
/// kept as its own gate so a change of either constant cannot quietly drop it.
fn fit_track(fixes: &VecDeque<Fix>, now_ms: u64) -> Option<Track> {
    let fit = fit(fixes, now_ms)?;
    let span_s = fit.span_ms as f64 / 1_000.0;
    if fit.fixes < TRACK_MIN_FIXES
        || fit.span_ms < TRACK_SPAN_MS
        || !fit.speed_mps.is_finite()
        || fit.speed_mps < TRACK_MIN_MPS
        || fit.speed_mps * span_s < 3.0 * fit.mean_sigma
    {
        return None;
    }
    Some(Track {
        course_deg: fit.vx.atan2(fit.vy).to_degrees().rem_euclid(360.0),
        speed_mps: fit.speed_mps,
    })
}

fn clip(line: String) -> String {
    if line.chars().count() <= SUMMARY_COLUMNS {
        return line;
    }
    line.chars().take(SUMMARY_COLUMNS).collect()
}

impl Picture {
    /// Fold one line in. `now_ms` is the runner's session clock when the line
    /// was read.
    pub fn apply(&mut self, now_ms: u64, line: &Line) -> Applied {
        self.last_ms = self.last_ms.max(now_ms);
        // A reply ends at the first line that is not one of its rows.
        if !matches!(line, Line::ContactRow(_)) {
            self.commit_contacts(now_ms);
        }
        if !matches!(line, Line::WeaponRow(_)) {
            self.commit_weapons(now_ms);
        }
        if !is_status_form(line) {
            self.close_status();
        }
        if !matches!(line, Line::Other(_) | Line::CatchupPower { .. }) {
            self.open.catchup = false;
        }
        let lock_line_before = std::mem::replace(&mut self.open.lock_line, false);
        // A `STATUS` row is one only inside the block its `Callsign` line
        // opened: a row read anywhere else (a four-space facet run is what a
        // three-letter speaker's wrapped speech looks like) is someone else's
        // words, and it changes nothing.
        if is_status_form(line)
            && !matches!(line, Line::StatusCallsign { .. })
            && self.open.status.is_none()
        {
            return Applied {
                journal: true,
                event: None,
            };
        }
        let mut journal = true;
        let mut event = None;
        match line {
            // -- read replies ------------------------------------------------
            Line::ContactRow(row) => {
                journal = false;
                let open = self
                    .open
                    .contacts
                    .get_or_insert_with(ContactsReply::default);
                open.rows.push((row.clone(), now_ms));
                open.last_ms = now_ms;
            }
            Line::NoContacts => {
                journal = false;
                self.open.contacts = Some(ContactsReply::default());
                self.commit_contacts(now_ms);
            }
            Line::Lock {
                designator,
                friendly,
                percent,
            } => {
                // The one sentence two things print: `WEAPONSTATUS`'s head and the
                // arena's `Locked` notice. Its state is the same either way; it is
                // news, and journalled, only when it changes the lock.
                let news = self.lock.as_deref() != Some(designator.as_str())
                    || self.lock_friendly != *friendly;
                self.lock = Some(designator.clone());
                self.lock_friendly = *friendly;
                self.lock_percent = *percent;
                self.lock_read_ms = now_ms;
                self.open.lock_line = true;
                self.touch();
                journal = news;
                if news {
                    event = Some(self.event(
                        now_ms,
                        EventKind::Locked,
                        match percent {
                            Some(p) => format!("Locked on {designator}, {p}% to hit."),
                            None => format!("Locked on {designator}."),
                        },
                    ));
                }
            }
            Line::WeaponRow(row) => {
                journal = false;
                let open = self.open.weapons.get_or_insert_with(|| WeaponsReply {
                    locked: lock_line_before,
                    ..WeaponsReply::default()
                });
                open.rows.push((row.clone(), now_ms));
                open.last_ms = now_ms;
            }
            Line::NoWeapon => {
                journal = false;
                self.weapons.clear();
                self.touch();
            }
            Line::StatusCallsign { callsign } => {
                journal = false;
                self.close_status();
                self.own.callsign = Some(callsign.clone());
                // What the block does not print it no longer has.
                self.own.heat = None;
                self.own.overheated = false;
                self.own.jammed = false;
                self.own.emcon = None;
                self.open.status = Some(StatusBlock {
                    last_ms: now_ms,
                    ..StatusBlock::default()
                });
                self.open.section = None;
                self.touch();
            }
            Line::StatusHead(head) => {
                journal = false;
                self.own.name = Some(head.name.clone());
                self.own.chassis = head.chassis.clone();
                self.own.power = head.power;
                self.own.speed_kph = head.kph;
                self.own.heading_deg = head.heading_deg;
                self.own.held = head
                    .held
                    .then(|| self.own.held.unwrap_or(HeldCause::Unstated));
                self.own.destroyed = head.destroyed;
                self.status_row(now_ms);
            }
            Line::StatusPosition { x_m, y_m, scope } => {
                journal = false;
                self.own.pos_m = Some((*x_m as f64, *y_m as f64));
                self.own.pos_read_ms = now_ms;
                self.own.scope = *scope;
                if let Some(block) = self.open.status.as_mut() {
                    block.pos_m = self.own.pos_m;
                }
                self.status_row(now_ms);
            }
            Line::StatusNearestSpot {
                name,
                letter,
                range_m,
                bearing_deg,
            } => {
                journal = false;
                // Range and bearing from the block's own position: where the
                // spot lies. Without that row, the spot keeps what it had.
                let plotted_m = self
                    .open
                    .status
                    .as_ref()
                    .and_then(|block| block.pos_m)
                    .map(|(x, y)| {
                        let range = f64::from(*range_m);
                        let bearing = f64::from(*bearing_deg).to_radians();
                        (
                            (x + range * bearing.sin()).round() as i64,
                            (y + range * bearing.cos()).round() as i64,
                        )
                    })
                    .or_else(|| self.spots.get(letter).and_then(|spot| spot.plotted_m));
                self.spots.insert(
                    *letter,
                    Spot {
                        name: name.clone(),
                        range_m: Some(*range_m),
                        bearing_deg: Some(*bearing_deg),
                        read_ms: now_ms,
                        plotted_m,
                    },
                );
                self.status_row(now_ms);
            }
            Line::StatusHeat {
                heat,
                max,
                overheated,
            } => {
                journal = false;
                self.own.heat = Some((*heat, *max));
                self.own.overheated = *overheated;
                self.status_row(now_ms);
            }
            Line::StatusReceiver { .. } => {
                journal = false;
                self.own.jammed = true;
                self.status_row(now_ms);
            }
            Line::StatusEmcon { on } => {
                journal = false;
                self.own.emcon = Some(*on);
                self.status_row(now_ms);
            }
            Line::StatusSection {
                id,
                condition,
                internals,
            } => {
                journal = false;
                // Staged in the open block (the guard above holds one open).
                if let Some(block) = self.open.status.as_mut() {
                    block.sections.insert(
                        id.clone(),
                        Section {
                            condition: condition.clone(),
                            internals: *internals,
                            facets: BTreeMap::new(),
                        },
                    );
                }
                self.open.section = Some(id.clone());
                self.status_row(now_ms);
            }
            Line::StatusFacets { facets } => {
                journal = false;
                if let Some(id) = self.open.section.clone()
                    && let Some(section) = self
                        .open
                        .status
                        .as_mut()
                        .and_then(|block| block.sections.get_mut(&id))
                {
                    for (facet, left, max) in facets {
                        section.facets.insert(facet.clone(), (*left, *max));
                    }
                }
                self.status_row(now_ms);
            }
            Line::StatusUntold { name } => {
                journal = false;
                self.own.name = Some(name.clone());
                self.own.power = Power::Off;
                self.touch();
            }
            Line::NoFrameYet | Line::NoSuchContact => journal = false,
            Line::Range {
                designator,
                range_m,
                plotted_s,
            } => {
                journal = false;
                self.range_reply(now_ms, designator, *range_m, *plotted_s);
            }
            Line::Bearing {
                designator,
                bearing_deg,
                rel_deg,
                ..
            } => {
                journal = false;
                if let Some(contact) = self.contacts.get_mut(designator) {
                    contact.bearing_deg = *bearing_deg;
                    contact.rel_deg = *rel_deg;
                    self.touch();
                }
            }
            Line::SpotRange {
                name,
                letter,
                range_m,
            } => {
                journal = false;
                let spot = self.spot(*letter, name, now_ms);
                spot.range_m = Some(*range_m);
                self.touch();
            }
            Line::SpotBearing {
                name,
                letter,
                bearing_deg,
                ..
            } => {
                journal = false;
                let spot = self.spot(*letter, name, now_ms);
                spot.bearing_deg = Some(*bearing_deg);
                self.touch();
            }
            Line::NoLock | Line::NoLockOrName => {
                journal = false;
                self.clear_lock(now_ms);
            }
            Line::SpeedLimits {
                ahead_kph,
                astern_kph,
            } => {
                journal = false;
                self.own.max_ahead_kph = Some(*ahead_kph);
                self.own.max_astern_kph = Some(*astern_kph);
                self.touch();
            }
            Line::DestroyedNoOrders => {
                journal = false;
                self.own.destroyed = true;
                self.touch();
            }
            Line::EmconReading { on } => {
                journal = false;
                self.own.emcon = Some(*on);
                self.touch();
            }

            // -- acks --------------------------------------------------------
            Line::StartupBegun | Line::PowerStarting => {
                journal = false;
                self.own.power = Power::Starting;
                self.touch();
            }
            Line::PowerAlreadyOnline => {
                journal = false;
                self.own.power = Power::Online;
                self.touch();
            }
            Line::PowerNotOnline => {
                journal = false;
                self.own.power = Power::Off;
                self.touch();
            }
            Line::Immobile => {
                journal = false;
                self.own.immobile = true;
                self.touch();
            }
            Line::Steering { .. }
            | Line::Making { .. }
            | Line::Locking { .. }
            | Line::EmconAck { .. } => journal = false,
            Line::Firing { weapon } => {
                journal = false;
                // The ack is the answer to the oldest `FIRE` of this weapon
                // still waiting for one.
                self.take_fire_mark(now_ms, Some(weapon));
                self.tally.shots = self.tally.shots.saturating_add(1);
                let entry = self.weapon_mut(weapon);
                entry.fired_ms = Some(now_ms);
                entry.ready = false;
                // The one clock (cut AK.1): the ack lowers the slack toward its
                // floor, then the gun is ready one learned cycle and that slack
                // on — never sooner, whatever an earlier row left behind. With
                // no cycle learned yet, the row is read.
                entry.slack_ms = entry
                    .slack_ms
                    .saturating_sub(FIRE_SLACK_DOWN_MS)
                    .max(FIRE_SLACK_MIN_MS);
                let slack = entry.slack_ms;
                entry.next_ready_ms = entry
                    .cycle_ms
                    .map(|cycle| now_ms.saturating_add(cycle).saturating_add(slack));
                entry.shots_since_read = entry.shots_since_read.saturating_add(1);
                entry.acks.push_back(now_ms);
                while entry.acks.len() > MAX_ACKS {
                    entry.acks.pop_front();
                }
                self.touch();
            }

            // -- the unseated, reclaim and resume sentences ------------------
            Line::OutsideCockpit => {
                journal = false;
                self.set_aboard(false);
            }
            Line::AlreadyAboard
            | Line::NotStartedYet
            | Line::ClimbBack { .. }
            | Line::BackAtStation { .. } => self.set_aboard(true),
            Line::LeftForGood { .. } | Line::ClimbOut { .. } | Line::Resumed => {
                self.set_aboard(false);
            }
            Line::BackAboard { callsign, away_ms } => {
                self.reclaims = self.reclaims.saturating_add(1);
                self.away_ms = *away_ms;
                if self.own.callsign.is_none() {
                    self.own.callsign = Some(callsign.clone());
                }
                self.aboard = true;
                self.open.catchup = true;
                self.touch();
            }
            // The block's own power row, and only inside the block: the same
            // words two-space indented anywhere else are not the tank's.
            Line::CatchupPower { power } => {
                if std::mem::replace(&mut self.open.catchup, false) {
                    self.own.power = *power;
                    self.touch();
                }
            }
            Line::Doctrine { tank } => {
                if self.own.name.as_deref().is_none_or(|own| own == tank) {
                    self.set_aboard(false);
                }
            }
            Line::Connected { name, .. } => {
                self.me = Some(name.clone());
                if self.phase == Phase::Unknown {
                    event = self.set_phase(now_ms, Phase::Walking);
                }
                self.touch();
            }

            // -- notices -----------------------------------------------------
            Line::NewContact {
                designator,
                bearing_deg,
                band,
            } => {
                let contact = self
                    .contacts
                    .entry(designator.clone())
                    .or_insert_with(|| Contact::new(designator, now_ms));
                if let Some(bearing) = bearing_deg {
                    contact.bearing_deg = *bearing;
                }
                if let Some(band) = band {
                    contact.band = band.clone();
                }
                self.touch();
                event = Some(self.event(
                    now_ms,
                    EventKind::ContactNew,
                    format!("New contact {designator}."),
                ));
            }
            Line::Identified {
                was,
                designator,
                side,
            } => {
                self.rekey(now_ms, was, designator, Some(*side));
                event = Some(self.event(
                    now_ms,
                    EventKind::Identified,
                    format!("Contact {was} identified: {designator}, {}.", side.as_str()),
                ));
            }
            Line::Unidentified { was, designator } => {
                self.rekey(now_ms, was, designator, None);
                event = Some(self.event(
                    now_ms,
                    EventKind::Unidentified,
                    format!("{was} is unidentified again: {designator}."),
                ));
            }
            Line::LockBroken { designator } => {
                self.clear_lock(now_ms);
                event = Some(self.event(
                    now_ms,
                    EventKind::LockBroken,
                    format!("Lock on {designator} broken."),
                ));
            }
            Line::ContactLost { designator } => {
                self.contacts.remove(designator);
                if self.lock.as_deref() == Some(designator.as_str()) {
                    self.clear_lock(now_ms);
                }
                self.touch();
                event = Some(self.event(
                    now_ms,
                    EventKind::Lost,
                    format!("Contact {designator} lost."),
                ));
            }
            Line::HitScored {
                designator,
                section,
                ..
            } => {
                self.tally.hits_scored = self.tally.hits_scored.saturating_add(1);
                self.touch();
                let text = match (designator, section) {
                    (Some(d), Some(s)) => format!("Hit on {d}: {s}."),
                    (Some(d), None) => format!("Hit on {d}."),
                    (None, _) => "Hit.".to_owned(),
                };
                event = Some(self.event(now_ms, EventKind::HitScored, text));
            }
            Line::Missed { designator, words } => {
                self.tally.misses = self.tally.misses.saturating_add(1);
                self.touch();
                let text = match (designator, words) {
                    (Some(d), Some(w)) => format!("Miss on {d}: {w}."),
                    (None, Some(w)) => format!("Miss: {w}."),
                    _ => "Miss.".to_owned(),
                };
                event = Some(self.event(now_ms, EventKind::Missed, text));
            }
            Line::Killed { designator } => {
                self.tally.kills = self.tally.kills.saturating_add(1);
                self.touch();
                let text = match designator {
                    Some(d) => format!("{d} destroyed."),
                    None => "Target destroyed.".to_owned(),
                };
                event = Some(self.event(now_ms, EventKind::Kill, text));
            }
            Line::TargetSectionLost {
                designator,
                section,
            } => {
                event = Some(self.event(
                    now_ms,
                    EventKind::TargetSectionLost,
                    format!("{designator}'s {section} is destroyed."),
                ));
            }
            Line::HitTaken {
                section,
                through,
                facet,
                armour,
                internals,
                already_destroyed,
            } => {
                self.tally.hits_taken = self.tally.hits_taken.saturating_add(1);
                let plate = through.as_ref().unwrap_or(section).clone();
                if let Some(facet) = facet {
                    let cell = self
                        .own
                        .sections
                        .entry(plate)
                        .or_default()
                        .facets
                        .entry(facet.clone())
                        .or_insert((0, 0));
                    *cell = match armour {
                        Some(figures) => *figures,
                        None => (0, cell.1),
                    };
                }
                if let Some(inside) = internals {
                    self.own
                        .sections
                        .entry(section.clone())
                        .or_default()
                        .internals = Some(*inside);
                }
                if *already_destroyed {
                    self.own
                        .sections
                        .entry(section.clone())
                        .or_default()
                        .condition = "destroyed".to_owned();
                }
                self.touch();
                event = Some(self.event(
                    now_ms,
                    EventKind::HitTaken,
                    format!("Hit on the {section}."),
                ));
            }
            Line::SectionLost { section, cost } => {
                if !self.tally.sections_lost.contains(section) {
                    self.tally.sections_lost.push(section.clone());
                }
                self.own
                    .sections
                    .entry(section.clone())
                    .or_default()
                    .condition = "destroyed".to_owned();
                if cost
                    .as_deref()
                    .is_some_and(|cost| cost.contains("the tank cannot move"))
                {
                    self.own.immobile = true;
                }
                self.touch();
                event = Some(self.event(
                    now_ms,
                    EventKind::SectionLost,
                    format!("The {section} is destroyed."),
                ));
            }
            Line::OwnDestroyed => {
                self.own.destroyed = true;
                self.touch();
                event = Some(self.event(
                    now_ms,
                    EventKind::OwnDestroyed,
                    "Your tank is destroyed.".to_owned(),
                ));
            }
            Line::OwnDisabled => {
                self.own.disabled = true;
                self.touch();
                event = Some(self.event(
                    now_ms,
                    EventKind::OwnDisabled,
                    "Your tank can neither move nor shoot.".to_owned(),
                ));
            }
            Line::PowerOnline => {
                self.own.power = Power::Online;
                self.aboard = true;
                self.power_ms.get_or_insert(now_ms);
                self.touch();
                event = Some(self.event(now_ms, EventKind::PowerOnline, "Power online.".into()));
            }
            Line::Held { cause } => {
                self.own.held = Some(*cause);
                self.touch();
                let words = match cause {
                    HeldCause::Impassable => "the ground ahead will not take the tank",
                    HeldCause::Occupied => "something is in the way",
                    HeldCause::Edge => "this is the edge of the arena",
                    HeldCause::TooSteep => "the slope ahead is too steep for the tank",
                    HeldCause::Unstated => "the tank is held",
                };
                event = Some(self.event(now_ms, EventKind::Held, format!("Held: {words}.")));
            }
            Line::ReceiverDegraded | Line::ReceiverClear => {
                self.own.jammed = matches!(line, Line::ReceiverDegraded);
                self.touch();
            }
            Line::EmconSwitched { on, .. } => {
                self.own.emcon = Some(*on);
                self.touch();
            }
            Line::Refused {
                order,
                weapon,
                said,
                cycling_s,
            } => {
                self.refused(now_ms, order, weapon.as_deref(), said, *cycling_s);
                let text = match weapon {
                    Some(w) => format!("{order} {w} refused. {said}"),
                    None => format!("{order} refused. {said}"),
                };
                event = Some(self.event(now_ms, EventKind::Refused, text));
            }

            // -- the lobby and the centre ------------------------------------
            Line::Board { .. } => {
                if matches!(self.phase, Phase::Unknown | Phase::Walking) {
                    event = self.set_phase(now_ms, Phase::InCentre);
                }
            }
            Line::Boarding { match_id } => {
                if matches!(self.phase, Phase::Claimed { .. }) && self.following.is_none() {
                    self.following = Some(*match_id);
                    self.new_match();
                    event = self.set_phase(
                        now_ms,
                        Phase::Boarding {
                            match_id: *match_id,
                        },
                    );
                }
            }
            Line::ClimbIn { .. } => {
                self.set_aboard(true);
                if let Phase::Boarding { match_id } = self.phase {
                    event = self.set_phase(now_ms, Phase::Seated { match_id });
                }
            }
            Line::Running { match_id } => {
                if self.following == Some(*match_id)
                    && matches!(self.phase, Phase::Boarding { .. } | Phase::Seated { .. })
                {
                    event = self.set_phase(
                        now_ms,
                        Phase::Running {
                            match_id: *match_id,
                        },
                    );
                }
            }
            Line::Over { match_id, result } => {
                event = self.over(now_ms, *match_id, result.clone());
            }
            Line::Aborted { match_id, .. } | Line::CouldNotStart { match_id, .. } => {
                event = self.over(now_ms, *match_id, ResultWord::Abort);
            }
            Line::AllLeft { match_id } => {
                event = self.over(now_ms, *match_id, ResultWord::Timeout);
            }
            Line::CallsOff { .. } => {
                if let Phase::Boarding { match_id } | Phase::Seated { match_id } = self.phase {
                    event = self.over(now_ms, match_id, ResultWord::Abort);
                }
            }
            Line::HatchOpen { .. } => self.set_aboard(false),
            Line::Room(name) => {
                self.room = Some(name.clone());
                self.touch();
                let centre = is_centre_room(name);
                if centre && matches!(self.phase, Phase::Unknown | Phase::Walking) {
                    event = self.set_phase(now_ms, Phase::InCentre);
                } else if !centre && matches!(self.phase, Phase::Unknown | Phase::InCentre) {
                    event = self.set_phase(now_ms, Phase::Walking);
                }
            }
            Line::Transient(_) => journal = false,
            Line::NotInAPod
            | Line::OutsideTeamRoom
            | Line::HandingOver { .. }
            | Line::BoardSide { .. }
            | Line::AiFill { .. }
            | Line::BoardNext
            | Line::Claimed { .. }
            | Line::GaveUp { .. }
            | Line::ClimbsIn { .. }
            | Line::AbortsMatch { .. }
            | Line::BoardCleared
            | Line::AlreadyHold { .. }
            | Line::SideFull { .. }
            | Line::NotLead
            | Line::EmptySide { .. }
            | Line::WrongCentreState { .. }
            | Line::NoCentre
            | Line::LayerOff
            | Line::NobodyPlaying
            | Line::OutsideCentre
            | Line::NoSlot
            | Line::NothingToAbort
            | Line::Hint(_)
            | Line::Other(_) => {}
        }
        Applied { journal, event }
    }

    /// Commit any open reply whose last row is [`SNAPSHOT_SETTLE_MS`] old.
    pub fn settle(&mut self, now_ms: u64) {
        self.last_ms = self.last_ms.max(now_ms);
        let due = |last: u64| now_ms >= last.saturating_add(SNAPSHOT_SETTLE_MS);
        if self.open.contacts.as_ref().is_some_and(|o| due(o.last_ms)) {
            self.commit_contacts(now_ms);
        }
        if self.open.weapons.as_ref().is_some_and(|o| due(o.last_ms)) {
            self.commit_weapons(now_ms);
        }
        if self.open.status.as_ref().is_some_and(|o| due(o.last_ms)) {
            self.close_status();
        }
        if self.open.lock_line && now_ms >= self.lock_read_ms.saturating_add(SNAPSHOT_SETTLE_MS) {
            self.open.lock_line = false;
        }
        let mut changed = false;
        for contact in self.contacts.values_mut() {
            let track = fit_track(&contact.fixes, now_ms);
            if track != contact.track {
                contact.track = track;
                changed = true;
            }
        }
        if changed {
            self.touch();
        }
    }

    /// The lane holds `side` slot `index` (it read its own claim, or `You
    /// already hold`): the picture follows the next match that boards.
    pub fn claimed(&mut self, side: Side, index: u8) {
        self.side = Some(side);
        self.following = None;
        let _ = self.set_phase(self.last_ms, Phase::Claimed { side, index });
        self.touch();
    }

    /// The runner wrote a `FIRE` line (a re-send included) at `now_ms`.
    ///
    /// Every `FIRE` is answered at once by exactly one line of its own —
    /// `Firing {w}.`, or the cockpit's pre-check `FIRE {w} refused. …` — and the
    /// arena's later refusal of an acked shot says the very same `FIRE {w}
    /// refused. …` (a machine gun cycles in a second, so a second `FIRE` inside
    /// two seconds of an ack is routine). A refusal read while a mark is
    /// outstanding is that `FIRE`'s own answer, the pre-check, and never
    /// un-counts a shot that left; one read with no mark outstanding pairs with
    /// an ack, as before. A picture that is never told falls back to the
    /// pairing alone. A mark lapses after [`ACK_PAIR_MS`] (a `FIRE` answered by
    /// something else: a transient, `This unit carries no weapon.`).
    ///
    /// Marks are kept per weapon (cut AK.1): `weapon` is the name the runner
    /// typed, and an answer takes only a mark of its own weapon, so a cannon's
    /// refusal read while only the machine gun's `FIRE` is outstanding is the
    /// arena's refusal of the cannon's acked shot, not a pre-check.
    pub fn sent_fire(&mut self, now_ms: u64, weapon: &str) {
        self.last_ms = self.last_ms.max(now_ms);
        self.lapse_fire_sent(now_ms);
        self.open.fire_sent.push_back((weapon.to_owned(), now_ms));
        while self.open.fire_sent.len() > MAX_ACKS {
            self.open.fire_sent.pop_front();
        }
    }

    /// The own position at `at_ms`, dead-reckoned from the last `STATUS` read at
    /// the speed and heading read with it.
    pub fn own_pos_at(&self, at_ms: u64) -> Option<(f64, f64)> {
        let (x, y) = self.own.pos_m?;
        let dt = (at_ms as f64 - self.own.pos_read_ms as f64) / 1_000.0;
        let mps = f64::from(self.own.speed_kph) / 3.6;
        let heading = f64::from(self.own.heading_deg).to_radians();
        Some((x + mps * dt * heading.sin(), y + mps * dt * heading.cos()))
    }

    /// The picture in a few lines for a person (`/arena`), each at most
    /// [`SUMMARY_COLUMNS`] columns.
    pub fn summary_lines(&self, now_ms: u64) -> Vec<String> {
        let ago = |at: u64| format!("{:.1} s ago", now_ms.saturating_sub(at) as f64 / 1_000.0);
        let mut lines = Vec::new();
        lines.push(format!(
            "phase {}; room {}; {}; me {}; side {}",
            self.phase.words(),
            self.room.as_deref().unwrap_or("unknown"),
            if self.aboard { "aboard" } else { "not aboard" },
            self.me.as_deref().unwrap_or("unknown"),
            self.side.map_or("none", Side::as_str),
        ));
        let own = &self.own;
        let mut head = format!(
            "own {}: power {}, {} kph, heading {:03}°",
            own.callsign.as_deref().unwrap_or("?"),
            own.power.as_str(),
            own.speed_kph,
            own.heading_deg
        );
        if let Some((x, y)) = own.pos_m {
            head.push_str(&format!(
                ", at {x:.0} m east {y:.0} m north (read {})",
                ago(own.pos_read_ms)
            ));
        }
        for (flag, word) in [
            (own.held.is_some(), "HELD"),
            (own.immobile, "IMMOBILE"),
            (own.disabled, "DISABLED"),
            (own.destroyed, "DESTROYED"),
        ] {
            if flag {
                head.push_str(", ");
                head.push_str(word);
            }
        }
        lines.push(head);
        if !own.sections.is_empty() {
            let words: Vec<String> = own
                .sections
                .iter()
                .map(|(id, section)| format!("{id} {}", section.condition))
                .collect();
            lines.push(format!("sections: {}", words.join(", ")));
        }
        lines.push(match &self.lock {
            Some(lock) => format!(
                "lock {lock}{}{} (read {})",
                if self.lock_friendly {
                    " (FRIENDLY)"
                } else {
                    ""
                },
                self.lock_percent
                    .map_or(String::new(), |p| format!(", {p}% to hit")),
                ago(self.lock_read_ms)
            ),
            None => "no lock".to_owned(),
        });
        for weapon in &self.weapons {
            lines.push(format!(
                "weapon {}: {}, {} of {} round(s){}",
                weapon.id,
                if weapon.ready { "ready" } else { "not ready" },
                weapon.rounds,
                weapon.rounds_max,
                weapon.cycle_ms.map_or(String::new(), |c| format!(
                    ", cycle {:.1} s",
                    c as f64 / 1_000.0
                )),
            ));
        }
        if self.contacts.is_empty() {
            lines.push("no contacts".to_owned());
        }
        for contact in self.contacts.values() {
            lines.push(format!(
                "contact {} {} {:03}° {} {}, plot {} s old{}",
                contact.designator,
                contact.side.map_or("unknown", Side::as_str),
                contact.bearing_deg,
                contact
                    .range_m
                    .map_or("? m".to_owned(), |m| format!("{m} m")),
                contact.band,
                contact.age_ms / 1_000,
                contact
                    .track
                    .as_ref()
                    .map_or(String::new(), |track| format!(
                        ", track {:03.0}° {:.1} m/s",
                        track.course_deg, track.speed_mps
                    )),
            ));
        }
        for (letter, spot) in &self.spots {
            lines.push(format!(
                "spot {letter} {}: {} at {}",
                spot.name,
                spot.range_m.map_or("? m".to_owned(), |m| format!("{m} m")),
                spot.bearing_deg
                    .map_or("?°".to_owned(), |b| format!("{b:03}°"))
            ));
        }
        let tally = &self.tally;
        lines.push(format!(
            "tally: fired {} (acks {}, refused {}), orders refused {}, hits {}, misses {}, kills {}, \
             hits taken {}, sections lost {}",
            tally.fired(),
            tally.shots,
            tally.refused_shots,
            tally.refused_orders,
            tally.hits_scored,
            tally.misses,
            tally.kills,
            tally.hits_taken,
            if tally.sections_lost.is_empty() {
                "none".to_owned()
            } else {
                tally.sections_lost.join(" ")
            }
        ));
        lines.into_iter().map(clip).collect()
    }

    // -- internals -----------------------------------------------------------

    fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn event(&mut self, at_ms: u64, kind: EventKind, text: String) -> Event {
        let event = Event { at_ms, kind, text };
        self.events.push_back(event.clone());
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
        self.touch();
        event
    }

    fn set_phase(&mut self, now_ms: u64, phase: Phase) -> Option<Event> {
        if self.phase == phase {
            return None;
        }
        self.phase = phase;
        let text = format!("phase: {}", self.phase.words());
        Some(self.event(now_ms, EventKind::PhaseChanged, text))
    }

    fn set_aboard(&mut self, aboard: bool) {
        self.aboard = aboard;
        self.touch();
    }

    /// A followed match's end, and only the followed match's.
    fn over(&mut self, now_ms: u64, match_id: u64, result: ResultWord) -> Option<Event> {
        if self.following != Some(match_id) || matches!(self.phase, Phase::Over { .. }) {
            return None;
        }
        self.aboard = false;
        self.set_phase(now_ms, Phase::Over { match_id, result })
    }

    /// A new followed match starts from a clean tank, scope and tally.
    fn new_match(&mut self) {
        self.own = Own::default();
        self.contacts.clear();
        self.lock = None;
        self.lock_friendly = false;
        self.lock_percent = None;
        self.weapons.clear();
        self.spots.clear();
        self.tally = Tally::default();
        self.reclaims = 0;
        self.away_ms = 0;
        self.power_ms = None;
        self.aboard = false;
        self.open = Open::default();
        self.touch();
    }

    /// Drop the `FIRE` marks older than [`ACK_PAIR_MS`].
    fn lapse_fire_sent(&mut self, now_ms: u64) {
        while self
            .open
            .fire_sent
            .front()
            .is_some_and(|(_, at)| now_ms.saturating_sub(*at) > ACK_PAIR_MS)
        {
            self.open.fire_sent.pop_front();
        }
    }

    /// Take the oldest outstanding `FIRE` mark of `weapon` (its id or its short
    /// name, either way round), after lapsing the old ones; whether there was
    /// one. An answer that names no weapon takes the oldest mark of any.
    fn take_fire_mark(&mut self, now_ms: u64, weapon: Option<&str>) -> bool {
        self.lapse_fire_sent(now_ms);
        let index = match weapon {
            None => (!self.open.fire_sent.is_empty()).then_some(0),
            Some(name) => {
                let carried = self.weapons.iter().find(|w| w.named(name));
                self.open
                    .fire_sent
                    .iter()
                    .position(|(sent, _)| sent == name || carried.is_some_and(|w| w.named(sent)))
            }
        };
        index
            .and_then(|index| self.open.fire_sent.remove(index))
            .is_some()
    }

    fn clear_lock(&mut self, now_ms: u64) {
        self.lock = None;
        self.lock_friendly = false;
        self.lock_percent = None;
        self.lock_read_ms = now_ms;
        self.touch();
    }

    fn status_row(&mut self, now_ms: u64) {
        if let Some(block) = self.open.status.as_mut() {
            block.last_ms = now_ms;
        }
        self.touch();
    }

    fn close_status(&mut self) {
        if let Some(block) = self.open.status.take() {
            if !block.sections.is_empty() {
                self.own.sections = block.sections;
            }
            self.open.section = None;
            self.touch();
        }
    }

    fn spot(&mut self, letter: char, name: &str, now_ms: u64) -> &mut Spot {
        let spot = self.spots.entry(letter).or_insert_with(|| Spot {
            name: name.to_owned(),
            range_m: None,
            bearing_deg: None,
            read_ms: now_ms,
            plotted_m: None,
        });
        spot.name = name.to_owned();
        spot.read_ms = now_ms;
        spot
    }

    fn weapon_mut(&mut self, id: &str) -> &mut Weapon {
        let index = match self
            .weapons
            .iter()
            .position(|w| w.id == id || w.short.as_deref() == Some(id))
        {
            Some(index) => index,
            None => {
                self.weapons.push(Weapon::new(id));
                self.weapons.len() - 1
            }
        };
        &mut self.weapons[index]
    }

    /// A contact's rename (`identified`) or its un-naming (`unidentified again`):
    /// the fixes and the track move with it, and so does the lock.
    fn rekey(&mut self, now_ms: u64, was: &str, designator: &str, side: Option<Side>) {
        let mut contact = self
            .contacts
            .remove(was)
            .or_else(|| self.contacts.remove(designator))
            .unwrap_or_else(|| Contact::new(designator, now_ms));
        contact.designator = designator.to_owned();
        contact.was = Some(was.to_owned());
        contact.side = side;
        self.contacts.insert(designator.to_owned(), contact);
        if self.lock.as_deref() == Some(was) {
            self.lock = Some(designator.to_owned());
        }
        self.touch();
    }

    /// A fix from one contact row, or `None` before the first `STATUS`.
    fn row_fix(&self, row: &ContactRow, read_ms: u64) -> Option<Fix> {
        let origin = self.own_pos_at(read_ms)?;
        let range = f64::from(row.range_m);
        let bearing = f64::from(row.bearing_deg).to_radians();
        let reckoned_s = read_ms.saturating_sub(self.own.pos_read_ms) as f64 / 1_000.0;
        Some(Fix {
            at_ms: read_ms.saturating_sub(u64::from(row.age_s) * 1_000 + 500),
            pos_m: (
                origin.0 + range * bearing.sin(),
                origin.1 + range * bearing.cos(),
            ),
            sigma_m: 50.0 + range * 0.5_f64.to_radians() + 3.0 * reckoned_s,
            from_lock: false,
        })
    }

    fn commit_contacts(&mut self, now_ms: u64) {
        let Some(reply) = self.open.contacts.take() else {
            return;
        };
        let mut held = BTreeMap::new();
        for (row, read_ms) in reply.rows {
            let fix = self.row_fix(&row, read_ms);
            let mut contact = self
                .contacts
                .remove(&row.designator)
                .unwrap_or_else(|| Contact::new(&row.designator, read_ms));
            // The same plot read again has only aged: it is not a second fix.
            let same_plot = contact.range_m == Some(row.range_m)
                && contact.bearing_deg == row.bearing_deg
                && u64::from(row.age_s) * 1_000 > contact.age_ms;
            contact.side = row.side;
            contact.bearing_deg = row.bearing_deg;
            contact.rel_deg = row.rel_deg;
            contact.band = row.band.clone();
            contact.range_m = Some(row.range_m);
            contact.age_ms = u64::from(row.age_s) * 1_000;
            contact.read_ms = read_ms;
            if !same_plot && let Some(fix) = fix {
                contact.push_fix(fix);
            }
            contact.track = fit_track(&contact.fixes, now_ms);
            held.insert(row.designator.clone(), contact);
        }
        // A reply is the platform's whole list: what it does not name is gone.
        self.contacts = held;
        if let Some(lock) = &self.lock
            && !self.contacts.contains_key(lock)
        {
            self.clear_lock(now_ms);
        }
        self.touch();
    }

    fn commit_weapons(&mut self, now_ms: u64) {
        let Some(reply) = self.open.weapons.take() else {
            return;
        };
        let mut carried = Vec::new();
        for (row, read_ms) in reply.rows {
            let mut weapon = match self.weapons.iter().position(|w| w.id == row.id) {
                Some(index) => self.weapons.remove(index),
                None => Weapon::new(&row.id),
            };
            // A `ready` row this soon after the ack may be the pre-shot frame —
            // unless its rounds show every acked shot has left since the last
            // row (a machine gun cycles in a second, well inside the guard).
            let shots_left = weapon.read_ms > 0
                && row.rounds.saturating_add(weapon.shots_since_read) <= weapon.rounds;
            let stale_ready = row.ready
                && !shots_left
                && weapon
                    .fired_ms
                    .is_some_and(|fired| read_ms.saturating_sub(fired) < READY_AFTER_ACK_MS);
            if !stale_ready {
                weapon.shots_since_read = 0;
                weapon.short = row.short.clone();
                weapon.ready = row.ready;
                weapon.cycling_s = row.cycling_s;
                weapon.rounds = row.rounds;
                weapon.rounds_max = row.rounds_max;
                weapon.band = row.band;
                weapon.percent = row.percent;
                weapon.read_ms = read_ms;
                // The one clock (cut AK.1): an accepted `ready` row is the
                // evidence, ready from its own time (so a machine gun whose
                // row lands inside the guard, its rounds showing the shot has
                // left, is ready now, not at the next row); a `cycling` row's
                // seconds are rounded up, so they are counted whole, plus the
                // slack; any other row leaves nothing to count but the row.
                match row.cycling_s {
                    None if row.ready => weapon.next_ready_ms = Some(read_ms),
                    None => weapon.next_ready_ms = None,
                    Some(secs) => {
                        let left_ms = u64::from(secs) * 1_000;
                        weapon.next_ready_ms = Some(
                            read_ms
                                .saturating_add(left_ms)
                                .saturating_add(weapon.slack_ms),
                        );
                        if let Some(fired) = weapon.fired_ms {
                            let cycle = read_ms.saturating_sub(fired).saturating_add(left_ms);
                            weapon.cycle_ms =
                                Some(weapon.cycle_ms.map_or(cycle, |held| held.min(cycle)));
                        }
                    }
                }
            }
            carried.push(weapon);
        }
        self.weapons = carried;
        // No lock line in front of the rows means no lock.
        if !reply.locked {
            self.clear_lock(now_ms);
        }
        self.touch();
    }

    /// A `RANGE` answer: the contact's figure, and — for the lock read just
    /// after its row — a finer fix in that row's place.
    fn range_reply(&mut self, now_ms: u64, designator: &str, range_m: u32, plotted_s: Option<u32>) {
        let locked = self.lock.as_deref() == Some(designator);
        let origin_status = self.own.pos_read_ms;
        let Some(contact) = self.contacts.get(designator) else {
            return;
        };
        let refine = locked
            && now_ms.saturating_sub(contact.read_ms) <= LOCK_FIX_MS
            && contact
                .fixes
                .back()
                .is_some_and(|fix| !fix.from_lock && fix.at_ms == contact.row_plot_ms());
        let row_read = contact.read_ms;
        let bearing = f64::from(contact.bearing_deg).to_radians();
        let origin = self.own_pos_at(row_read);
        let Some(contact) = self.contacts.get_mut(designator) else {
            return;
        };
        if refine && let Some(origin) = origin {
            let range = f64::from(range_m);
            let reckoned_s = row_read.saturating_sub(origin_status) as f64 / 1_000.0;
            // As fine as the plot is fresh (`help cockpit`, "Seeing"): "a fresh
            // one reads to 10 m, a one-second plot to 20, and a plot a couple
            // of seconds old reads no better than the rest of the scope".
            let base = match plotted_s {
                None | Some(0) => 10.0,
                Some(1) => 20.0,
                Some(_) => 50.0,
            };
            let at_ms = contact.row_plot_ms();
            contact.fixes.pop_back();
            contact.push_fix(Fix {
                at_ms,
                pos_m: (
                    origin.0 + range * bearing.sin(),
                    origin.1 + range * bearing.cos(),
                ),
                sigma_m: base + range * 0.5_f64.to_radians() + 3.0 * reckoned_s,
                from_lock: true,
            });
            contact.track = fit_track(&contact.fixes, now_ms);
        }
        self.touch();
    }

    /// `ORDER[ weapon] refused. said` — paired with an outstanding `Firing` ack
    /// of that weapon, it is the arena's refusal of a shot that never left;
    /// unpaired, it is the cockpit's pre-check or another order refused.
    fn refused(
        &mut self,
        now_ms: u64,
        order: &str,
        weapon: Option<&str>,
        said: &str,
        cycling_s: Option<u32>,
    ) {
        let mut paired = false;
        // A `FIRE` of this weapon the runner wrote and has not yet seen
        // answered: this refusal is that answer, the cockpit's pre-check
        // ([`Picture::sent_fire`]).
        let answers_a_send = order == "FIRE" && self.take_fire_mark(now_ms, weapon);
        let carried = self
            .weapons
            .iter()
            .position(|w| weapon.is_some_and(|id| w.id == id));
        if order == "FIRE"
            && let Some(index) = carried
        {
            let entry = &mut self.weapons[index];
            while entry
                .acks
                .front()
                .is_some_and(|at| now_ms.saturating_sub(*at) > ACK_PAIR_MS)
            {
                entry.acks.pop_front();
            }
            if !answers_a_send && let Some(at) = entry.acks.pop_front() {
                paired = true;
                entry.shots_since_read = entry.shots_since_read.saturating_sub(1);
                if entry.fired_ms == Some(at) {
                    entry.fired_ms = None;
                }
            }
            if let Some(secs) = cycling_s {
                // The one clock (cut AK.1). The cockpit's own pre-check of a
                // `FIRE` this runner typed means the clock was early: the
                // slack grows first. The seconds are rounded up, so counted
                // whole.
                if answers_a_send {
                    entry.slack_ms = entry
                        .slack_ms
                        .saturating_add(FIRE_SLACK_UP_MS)
                        .min(FIRE_SLACK_MAX_MS);
                }
                entry.ready = false;
                entry.next_ready_ms = Some(
                    now_ms
                        .saturating_add(u64::from(secs) * 1_000)
                        .saturating_add(entry.slack_ms),
                );
            } else if paired {
                // The arena refused an acked shot for something other than its
                // cycle (`Too many orders at once`, `No lock: …`): the shot
                // never left and the cockpit's pre-check had passed it, so the
                // gun is ready now; the lane's own wait governs the retry.
                entry.next_ready_ms = Some(now_ms);
            }
        }
        if paired {
            self.tally.refused_shots = self.tally.refused_shots.saturating_add(1);
        } else {
            self.tally.refused_orders = self.tally.refused_orders.saturating_add(1);
        }
        if said == "Your tank cannot move." {
            self.own.immobile = true;
        }
        if said.starts_with("No lock:") {
            self.clear_lock(now_ms);
        }
        if said == "Power not online: STARTUP first." {
            self.own.power = Power::Off;
        }
        self.touch();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::classify;

    /// Apply each line at its time.
    fn feed(picture: &mut Picture, lines: &[(u64, &str)]) -> Vec<Applied> {
        lines
            .iter()
            .map(|(at, line)| picture.apply(*at, &classify(line)))
            .collect()
    }

    fn row_line(designator: &str, class: &str, bearing: u32, range: u32, age: u32) -> String {
        format!("{designator:<4}{class}  bearing {bearing:03}° (rel 000°)  far  {range} m  {age} s")
    }

    #[test]
    fn a_contacts_reply_is_the_whole_list() {
        let mut p = Picture::default();
        let applied = feed(
            &mut p,
            &[
                (0, "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s"),
                (
                    1,
                    "X2  unknown  bearing 315° (rel 270°)  effective  250 m  0 s",
                ),
            ],
        );
        assert!(applied.iter().all(|a| !a.journal));
        assert!(p.contacts.is_empty(), "held open until the reply ends");
        p.settle(100);
        assert!(
            p.contacts.is_empty(),
            "150 ms after the last row, not before"
        );
        p.settle(151);
        assert_eq!(p.contacts.keys().collect::<Vec<_>>(), ["BA", "X2"]);
        assert_eq!(p.contacts["BA"].side, Some(Side::Blue));
        assert_eq!(p.contacts["BA"].range_m, Some(450));
        assert_eq!(p.contacts["X2"].side, None);
        // The next reply drops what it does not name; a non-row line commits.
        feed(
            &mut p,
            &[
                (
                    1_000,
                    "X2  unknown  bearing 315° (rel 270°)  effective  250 m  1 s",
                ),
                (1_001, "Callsign RA."),
            ],
        );
        assert_eq!(p.contacts.keys().collect::<Vec<_>>(), ["X2"]);
        let applied = feed(&mut p, &[(2_000, "No contacts.")]);
        assert!(!applied[0].journal);
        assert!(p.contacts.is_empty());
    }

    #[test]
    fn a_weapons_reply_with_and_without_a_lock_and_a_bare_lock_notice() {
        let mut p = Picture::default();
        let applied = feed(
            &mut p,
            &[
                (0, "Locked on BA, 62% to hit."),
                (
                    1,
                    "cannon  ready  40 of 40 round(s)  band 1 of 2  62% to hit",
                ),
                (
                    2,
                    "machine_gun (mg)  ready  500 of 500 round(s)  21% to hit",
                ),
            ],
        );
        assert!(applied[1..].iter().all(|a| !a.journal));
        p.settle(200);
        assert_eq!(p.lock.as_deref(), Some("BA"));
        assert_eq!(p.lock_percent, Some(62));
        assert_eq!(p.weapons.len(), 2);
        assert_eq!(p.weapons[1].short.as_deref(), Some("mg"));
        // The same lock again (a poll reply) is not news.
        let applied = feed(
            &mut p,
            &[
                (1_000, "Locked on BA, 58% to hit."),
                (
                    1_001,
                    "cannon  ready  40 of 40 round(s)  band 1 of 2  58% to hit",
                ),
            ],
        );
        assert!(!applied[0].journal);
        p.settle(1_200);
        assert_eq!(p.lock_percent, Some(58));
        // A reply with no lock line in front means no lock.
        feed(&mut p, &[(2_000, "cannon  ready  40 of 40 round(s)")]);
        p.settle(2_200);
        assert_eq!(p.lock, None);
        assert_eq!(
            p.weapons.len(),
            1,
            "the machine gun is not carried any more"
        );
        // A bare lock line is the arena's notice: it sets the lock and is news.
        let applied = feed(
            &mut p,
            &[(3_000, "Locked on X2."), (3_050, "Power online.")],
        );
        assert!(applied[0].journal);
        assert_eq!(
            applied[0].event.as_ref().map(|e| e.kind),
            Some(EventKind::Locked)
        );
        assert_eq!(p.lock.as_deref(), Some("X2"));
        // `This unit carries no weapon.` empties the list.
        feed(&mut p, &[(4_000, "This unit carries no weapon.")]);
        assert!(p.weapons.is_empty());
    }

    #[test]
    fn a_rename_and_back_keep_the_fixes_and_the_lock() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Callsign RA."),
                (0, "Red Tank 1: power online, 0 kph, heading 000°"),
                (0, "  at 1000 m east, 975 m north; scope clear"),
                (10, "X2  unknown  bearing 090° (rel 090°)  far  450 m  0 s"),
                (20, "Locked on X2."),
            ],
        );
        assert_eq!(p.contacts["X2"].fixes.len(), 1);
        let applied = feed(&mut p, &[(500, "Contact X2 identified: BA, blue.")]);
        assert!(applied[0].journal);
        assert!(!p.contacts.contains_key("X2"));
        let ba = &p.contacts["BA"];
        assert_eq!(ba.was.as_deref(), Some("X2"));
        assert_eq!(ba.side, Some(Side::Blue));
        assert_eq!(ba.fixes.len(), 1);
        assert_eq!(p.lock.as_deref(), Some("BA"));
        feed(&mut p, &[(900, "BA is unidentified again: X2.")]);
        let x2 = &p.contacts["X2"];
        assert_eq!(x2.side, None);
        assert_eq!(x2.was.as_deref(), Some("BA"));
        assert_eq!(x2.fixes.len(), 1);
        assert_eq!(p.lock.as_deref(), Some("X2"));
        // `lost` removes the row and the lock with it.
        feed(&mut p, &[(1_000, "Contact X2 lost.")]);
        assert!(p.contacts.is_empty());
        assert_eq!(p.lock, None);
    }

    #[test]
    fn the_status_block() {
        let mut p = Picture::default();
        let applied = feed(
            &mut p,
            &[
                (0, "Callsign RB."),
                (
                    0,
                    "Red Tank 2 [biped_mk1]: power online, -12 kph, heading 270°, HELD, DESTROYED",
                ),
                (0, "  at -40 m east, 975 m north; scope holds a hostile"),
                (0, "  nearest spot: The Ford (A), 120 m at 045°"),
                (0, "  heat 100 of 100, OVERHEATED"),
                (0, "  receiver degraded: 15 of 40 through cover, 60% reach"),
                (0, "  emcon on: the pod radiates"),
                (0, "  hull  intact  internals 60/60"),
                (0, "    front 400/400  left 250/250"),
                (0, "    right 240/250  rear 150/150"),
                (0, "  left_track  destroyed  internals 0/24"),
                (0, "    left 0/120"),
                (1, "No contacts."),
            ],
        );
        assert!(applied.iter().all(|a| !a.journal));
        let own = &p.own;
        assert_eq!(own.callsign.as_deref(), Some("RB"));
        assert_eq!(own.name.as_deref(), Some("Red Tank 2"));
        assert_eq!(own.chassis.as_deref(), Some("biped_mk1"));
        assert_eq!(own.power, Power::Online);
        assert_eq!(own.speed_kph, -12);
        assert_eq!(own.heading_deg, 270);
        assert_eq!(own.held, Some(HeldCause::Unstated));
        assert!(own.destroyed);
        assert_eq!(own.pos_m, Some((-40.0, 975.0)));
        assert_eq!(own.scope, Scope::Hostile);
        assert_eq!(own.heat, Some((100, 100)));
        assert!(own.overheated && own.jammed);
        assert_eq!(own.emcon, Some(true));
        assert_eq!(own.sections["hull"].internals, Some((60, 60)));
        assert_eq!(
            own.sections["hull"].facets.len(),
            4,
            "the wrapped run joins its section"
        );
        assert_eq!(own.sections["hull"].facets["right"], (240, 250));
        assert_eq!(own.sections["left_track"].condition, "destroyed");
        assert_eq!(p.spots[&'A'].range_m, Some(120));
        // 120 m at 045 from (-40, 975): where The Ford lies.
        assert_eq!(p.spots[&'A'].plotted_m, Some((45, 1060)));
        // A head without HELD clears it; one with it keeps a known cause.
        feed(
            &mut p,
            &[
                (5_000, "Held: something is in the way."),
                (6_000, "Callsign RB."),
                (6_000, "Red Tank 2: power online, 0 kph, heading 270°, HELD"),
                (6_001, "Callsign RB."),
            ],
        );
        assert_eq!(p.own.held, Some(HeldCause::Occupied));
        assert!(!p.own.destroyed);
        feed(
            &mut p,
            &[(6_002, "Red Tank 2: power online, 0 kph, heading 270°")],
        );
        assert_eq!(p.own.held, None);
        // A block that has closed replaced the sections: the second block
        // printed none, so the first block's stay.
        p.settle(7_000);
        assert!(p.own.sections.contains_key("hull"));
    }

    #[test]
    fn dead_reckoning_and_the_plot_fix() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (1_000, "Callsign RA."),
                (1_000, "Red Tank 1: power online, 36 kph, heading 090°"),
                (1_000, "  at 1000 m east, 975 m north; scope clear"),
            ],
        );
        let (x, y) = p.own_pos_at(3_000).unwrap();
        assert!(
            (x - 1_020.0).abs() < 1e-6 && (y - 975.0).abs() < 1e-6,
            "{x} {y}"
        );
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Callsign RA."),
                (0, "Red Tank 1: power online, 0 kph, heading 000°"),
                (0, "  at 1000 m east, 975 m north; scope clear"),
                (1_000, "BA  blue  bearing 090° (rel 090°)  far  450 m  0 s"),
                (1_001, "Callsign RA."),
            ],
        );
        let fix = p.contacts["BA"].fixes.back().unwrap().clone();
        assert!((fix.pos_m.0 - 1_450.0).abs() < 1e-6 && (fix.pos_m.1 - 975.0).abs() < 1e-6);
        assert_eq!(fix.at_ms, 500);
        assert!(!fix.from_lock);
        let expected = 50.0 + 450.0 * 0.5_f64.to_radians() + 3.0;
        assert!((fix.sigma_m - expected).abs() < 1e-9);
        assert!((bearing_deg((0.0, 0.0), (10.0, 0.0)) - 90.0).abs() < 1e-9);
        assert!((bearing_deg((0.0, 0.0), (0.0, -10.0)) - 180.0).abs() < 1e-9);
        assert!((bearing_deg((0.0, 0.0), (-10.0, 0.0)) - 270.0).abs() < 1e-9);
        assert!((distance_m((0.0, 0.0), (3.0, 4.0)) - 5.0).abs() < 1e-9);
    }

    /// Own tank still at the origin, reading STATUS, CONTACTS and a bare RANGE
    /// on the lock every second while the target runs 10 m/s east.
    fn lock_fixed_run(fixes: u64, spacing_ms: u64) -> Picture {
        let mut p = Picture::default();
        feed(&mut p, &[(0, "Locked on BA.")]);
        for i in 0..fixes {
            let t = 10_000 + i * spacing_ms;
            let target = (300.0 + 10.0 * (t as f64 / 1_000.0 - 10.0), 400.0);
            let bearing = bearing_deg((0.0, 0.0), target).round() as u32 % 360;
            let range = distance_m((0.0, 0.0), target);
            let scope = ((range / 50.0).round() * 50.0) as u32;
            let lock = ((range / 10.0).round() * 10.0) as u32;
            let row = row_line("BA", "blue", bearing, scope, 0);
            let range_line = format!("Range to BA: {lock} m.");
            feed(
                &mut p,
                &[
                    (t, "Callsign RA."),
                    (t, "Red Tank 1: power online, 0 kph, heading 000°"),
                    (t, "  at 0 m east, 0 m north; scope holds a hostile"),
                    (t + 100, &row),
                    (t + 300, &range_line),
                ],
            );
        }
        p
    }

    #[test]
    fn a_moving_target_read_through_the_lock_gets_a_track() {
        let p = lock_fixed_run(7, 1_000);
        let ba = &p.contacts["BA"];
        assert!(ba.fixes.iter().all(|f| f.from_lock));
        let track = ba.track.as_ref().expect("a track");
        let course_off = (track.course_deg - 90.0).abs();
        assert!(course_off <= 10.0, "course {}", track.course_deg);
        assert!(
            (track.speed_mps - 10.0).abs() <= 2.0,
            "speed {}",
            track.speed_mps
        );
        // Three fixes are not enough, and neither are fixes spanning 3 s.
        assert_eq!(lock_fixed_run(3, 2_000).contacts["BA"].track, None);
        assert_eq!(lock_fixed_run(4, 1_000).contacts["BA"].track, None);
    }

    /// The game's scope, read the way the pilot reads it: whole degrees, 50 m
    /// steps held against the last figure printed.
    fn scope_range(true_m: f64, held: Option<u32>) -> u32 {
        if let Some(held) = held
            && (true_m - f64::from(held)).abs() <= 25.0 + 6.25
        {
            return held;
        }
        ((true_m / 50.0).round() * 50.0).max(50.0) as u32
    }

    /// What one stopped-target run came to.
    struct StoppedRun {
        /// Fixes the contact holds at the end.
        fixes: usize,
        /// Rows whose printed range was a held figure, not the nearest step.
        held_reads: usize,
        /// Times the printed range stepped to a new figure.
        steps: usize,
        /// Raw fits that cleared the count, span and speed gates, so the
        /// displacement gate alone kept them from being published.
        decided_by_displacement: usize,
        /// The fastest raw fit seen, metres a second.
        fastest_mps: f64,
    }

    /// A stopped `BA` at the origin, read from an own tank orbiting the point
    /// `centre_east_m` east of it at `radius` and 15 m/s (a pilot circling a
    /// target it has lost track of), `STATUS` every 2 s, a `CONTACTS` row every
    /// second, whole-degree bearings and the 50 m held scope figure, for 20 s.
    /// Asserts at every row that the contact has no track.
    fn stopped_target_run(centre_east_m: f64, radius: f64) -> StoppedRun {
        let mut p = Picture::default();
        let target = (0.0_f64, 0.0_f64);
        let omega = 15.0 / radius;
        let mut held = None;
        let mut run = StoppedRun {
            fixes: 0,
            held_reads: 0,
            steps: 0,
            decided_by_displacement: 0,
            fastest_mps: 0.0,
        };
        for step in 0..=40_u64 {
            let t = step * 500;
            let angle = omega * t as f64 / 1_000.0;
            let own = (centre_east_m + radius * angle.sin(), -radius * angle.cos());
            let heading = (angle.to_degrees() + 90.0).rem_euclid(360.0).round() as u32 % 360;
            if step % 4 == 0 {
                let lines = [
                    "Callsign RA.".to_owned(),
                    format!("Red Tank 1: power online, 54 kph, heading {heading:03}°"),
                    format!(
                        "  at {} m east, {} m north; scope holds a hostile",
                        own.0.trunc() as i64,
                        own.1.trunc() as i64
                    ),
                ];
                for line in &lines {
                    p.apply(t, &classify(line));
                }
            }
            if step % 2 == 0 {
                let true_m = distance_m(own, target);
                let range = scope_range(true_m, held);
                if range != ((true_m / 50.0).round() * 50.0).max(50.0) as u32 {
                    run.held_reads += 1;
                }
                if held.is_some_and(|was| was != range) {
                    run.steps += 1;
                }
                held = Some(range);
                let bearing = bearing_deg(own, target).round() as u32 % 360;
                p.apply(
                    t + 50,
                    &classify(&row_line("BA", "blue", bearing, range, 0)),
                );
                p.apply(t + 60, &classify("No such contact."));
                let ba = &p.contacts["BA"];
                assert_eq!(ba.track, None, "at {t} ms, centre {centre_east_m} m east");
                if let Some(raw) = fit(&ba.fixes, t + 60) {
                    run.fastest_mps = run.fastest_mps.max(raw.speed_mps);
                    if raw.fixes >= TRACK_MIN_FIXES
                        && raw.span_ms >= TRACK_SPAN_MS
                        && raw.speed_mps >= TRACK_MIN_MPS
                    {
                        run.decided_by_displacement += 1;
                    }
                }
            }
        }
        run.fixes = p.contacts["BA"].fixes.len();
        run
    }

    /// **The displacement gate is what keeps a stopped machine still** (review
    /// AJ-F8/AJ-R7). With the orbit off the target, the printed range swings
    /// across 50 m steps and holds between them, and the raw fit reads that as
    /// several metres a second — past the count, span and speed gates — yet no
    /// track is ever published.
    #[test]
    fn a_stopped_target_read_through_the_rounded_scope_never_gets_a_track() {
        let (mut held_reads, mut steps, mut decided) = (0, 0, 0);
        for centre_east_m in [30.0, 60.0, 90.0, 200.0] {
            let run = stopped_target_run(centre_east_m, 420.0);
            assert!(run.fixes >= TRACK_MIN_FIXES, "{centre_east_m}");
            held_reads += run.held_reads;
            steps += run.steps;
            decided += run.decided_by_displacement;
            assert!(
                run.fastest_mps > TRACK_MIN_MPS,
                "centre {centre_east_m} m east: the fastest raw fit was {} m/s",
                run.fastest_mps
            );
        }
        assert!(held_reads > 0, "the scope's hold never fired");
        assert!(steps > 0, "the printed range never stepped");
        assert!(
            decided > 0,
            "no raw fit reached the displacement gate: the run does not test it"
        );
        // Centred on the target, the true range never changes: the case the
        // gate is not needed for, kept as the plain control.
        let still = stopped_target_run(0.0, 420.0);
        assert_eq!(still.steps, 0);
        assert!(still.fixes >= TRACK_MIN_FIXES);
    }

    /// Fixes on a straight line: `n` of them `spacing_ms` apart, moving `mps`
    /// east, each with `sigma_m`, the last one at 20 s.
    fn line_of_fixes(n: u64, spacing_ms: u64, mps: f64, sigma_m: f64) -> VecDeque<Fix> {
        let end = 20_000;
        (0..n)
            .map(|i| {
                let at_ms = end - (n - 1 - i) * spacing_ms;
                Fix {
                    at_ms,
                    pos_m: (mps * at_ms as f64 / 1_000.0, 500.0),
                    sigma_m,
                    from_lock: true,
                }
            })
            .collect()
    }

    /// **Each gate of the track decides on its own** (review AJ-F8/AJ-R7): every
    /// refused case clears the other three gates, and its twin one step across
    /// the refusing gate is published.
    #[test]
    fn each_clause_of_the_track_gate_decides_on_its_own() {
        let now = 20_000;
        let published = |fixes: &VecDeque<Fix>| fit_track(fixes, now).is_some();
        // Count: three fixes over 4 s at 25 m/s (100 m against 3 × 10 m) are
        // refused; five over the same 4 s are published.
        let three = line_of_fixes(3, 2_000, 25.0, 10.0);
        let raw = fit(&three, now).unwrap();
        assert!(raw.span_ms >= TRACK_SPAN_MS && raw.speed_mps >= TRACK_MIN_MPS);
        assert!(raw.speed_mps * 4.0 >= 3.0 * raw.mean_sigma);
        assert!(!published(&three));
        assert!(published(&line_of_fixes(5, 1_000, 25.0, 10.0)));
        // Span: four fixes over 3 s at 25 m/s (75 m against 30 m) are refused;
        // over 4 s they are published.
        let short = line_of_fixes(4, 1_000, 25.0, 10.0);
        let raw = fit(&short, now).unwrap();
        assert!(raw.fixes >= TRACK_MIN_FIXES && raw.speed_mps >= TRACK_MIN_MPS);
        assert!(raw.speed_mps * 3.0 >= 3.0 * raw.mean_sigma);
        assert!(!published(&short));
        assert!(published(&line_of_fixes(5, 1_000, 25.0, 10.0)));
        // Displacement: 10 m/s over 6 s is 60 m, short of 3 × 55 m row fixes;
        // the same motion read through 15 m fixes (45 m) is published.
        let coarse = line_of_fixes(7, 1_000, 10.0, 55.0);
        let raw = fit(&coarse, now).unwrap();
        assert!(raw.fixes >= TRACK_MIN_FIXES && raw.span_ms >= TRACK_SPAN_MS);
        assert!(raw.speed_mps >= TRACK_MIN_MPS);
        assert!(!published(&coarse));
        assert!(published(&line_of_fixes(7, 1_000, 10.0, 15.0)));
        // Speed: 2 m/s over 6 s is 12 m, past 3 × 1 m, but under the floor;
        // 4 m/s is over it. (The picture's own fixes never read finer than
        // 10 m, which makes the displacement gate imply the floor; fixes this
        // fine are built by hand to show the floor refuses on its own.)
        let slow = line_of_fixes(7, 1_000, 2.0, 1.0);
        let raw = fit(&slow, now).unwrap();
        assert!(raw.fixes >= TRACK_MIN_FIXES && raw.span_ms >= TRACK_SPAN_MS);
        assert!(raw.speed_mps * 6.0 >= 3.0 * raw.mean_sigma);
        assert!(!published(&slow));
        assert!(published(&line_of_fixes(7, 1_000, 4.0, 1.0)));
        // The window: the same fixes read 11 s later have left it.
        assert_eq!(
            fit_track(&line_of_fixes(7, 1_000, 25.0, 10.0), 31_000),
            None
        );
    }

    #[test]
    fn the_tally_pairs_refusals_with_acks() {
        let mut p = Picture::default();
        // The cockpit's pre-check answers instead of an ack.
        feed(
            &mut p,
            &[(0, "FIRE cannon refused. No lock: LOCK a contact first.")],
        );
        assert_eq!(
            (p.tally.shots, p.tally.refused_orders, p.tally.refused_shots),
            (0, 1, 0)
        );
        // An ack, then the arena's refusal inside two seconds: nothing left.
        let applied = feed(
            &mut p,
            &[
                (1_000, "Firing cannon."),
                (1_500, "FIRE cannon refused. Still cycling: 4 s to go."),
            ],
        );
        assert!(!applied[0].journal && applied[1].journal);
        assert_eq!(p.tally.shots, 1);
        assert_eq!(p.tally.refused_shots, 1);
        assert_eq!(p.tally.fired(), 0);
        let cannon = p.weapons.iter().find(|w| w.id == "cannon").unwrap();
        // The ack lowered the slack, held at its floor; the arena's refusal of
        // the acked shot answers no mark of the runner's, so it does not raise
        // it. `4 s` is rounded up: counted whole, plus that slack.
        let slack = FIRE_SLACK_START_MS
            .saturating_sub(FIRE_SLACK_DOWN_MS)
            .max(FIRE_SLACK_MIN_MS);
        assert_eq!(cannon.slack_ms, slack);
        assert_eq!(cannon.next_ready_ms, Some(1_500 + 4_000 + slack));
        // Two refusals with nothing to pair never underflow.
        feed(
            &mut p,
            &[
                (
                    9_000,
                    "FIRE cannon refused. Empty: that weapon has nothing left to fire.",
                ),
                (
                    9_001,
                    "FIRE cannon refused. Empty: that weapon has nothing left to fire.",
                ),
            ],
        );
        assert_eq!(p.tally.refused_orders, 3);
        assert_eq!(p.tally.fired(), 0);
        // An ack older than two seconds is a shot that left.
        feed(
            &mut p,
            &[
                (10_000, "Firing cannon."),
                (12_500, "FIRE cannon refused. Still cycling: 1 s to go."),
            ],
        );
        assert_eq!(p.tally.fired(), 1);
        // Hits, misses, kills, hits taken and a lost section.
        feed(
            &mut p,
            &[
                (13_000, "Hit on BA: turret."),
                (13_001, "Miss."),
                (13_002, "BA destroyed."),
                (
                    13_003,
                    "Hit on the hull's front facet: 280 of 400 armour left.",
                ),
                (
                    13_004,
                    "Hit on the engine, through the hull's rear facet: armour gone, 20 of 40 inside.",
                ),
                (
                    13_005,
                    "The left_track is destroyed: the tank is slowed; the tank cannot move.",
                ),
            ],
        );
        let t = &p.tally;
        assert_eq!(
            (t.hits_scored, t.misses, t.kills, t.hits_taken),
            (1, 1, 1, 2)
        );
        assert_eq!(t.sections_lost, ["left_track"]);
        assert_eq!(p.own.sections["hull"].facets["front"], (280, 400));
        assert_eq!(p.own.sections["hull"].facets["rear"], (0, 0));
        assert_eq!(p.own.sections["engine"].internals, Some((20, 40)));
        assert_eq!(p.own.sections["left_track"].condition, "destroyed");
        assert!(p.own.immobile);
    }

    #[test]
    fn cycles_are_learned_from_cycling_rows_only() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Firing cannon."),
                (800, "cannon  ready  40 of 40 round(s)"),
                (900, "Callsign RA."),
            ],
        );
        let cannon = &p.weapons[0];
        assert!(
            !cannon.ready,
            "a ready row 800 ms after the ack is the pre-shot frame"
        );
        assert_eq!(cannon.cycle_ms, None);
        assert_eq!(cannon.next_ready_ms, None, "and it sets no clock");
        feed(
            &mut p,
            &[
                (1_000, "cannon  cycling (5 s)  39 of 40 round(s)"),
                (1_001, "Power online."),
            ],
        );
        let cannon = &p.weapons[0];
        assert_eq!(cannon.cycle_ms, Some(6_000));
        assert_eq!(cannon.rounds, 39);
        feed(
            &mut p,
            &[
                (7_000, "cannon  ready  39 of 40 round(s)"),
                (7_001, "Power online."),
            ],
        );
        let cannon = &p.weapons[0];
        assert!(cannon.ready);
        assert_eq!(
            cannon.next_ready_ms,
            Some(7_000),
            "a ready row: ready from its own time"
        );
        assert_eq!(cannon.cycle_ms, Some(6_000));
        // The next shot's ack sets the clock from the learned cycle; the
        // pre-shot frame read after it (rounds unchanged, inside the guard)
        // leaves that clock alone — the gun is not ready at that row.
        feed(&mut p, &[(8_000, "Firing cannon.")]);
        let due = p.weapons[0].next_ready_ms;
        assert_eq!(
            due,
            Some(8_000 + 6_000 + p.weapons[0].slack_ms),
            "the ack's clock"
        );
        feed(
            &mut p,
            &[
                (8_300, "cannon  ready  39 of 40 round(s)"),
                (8_301, "Power online."),
            ],
        );
        let cannon = &p.weapons[0];
        assert!(!cannon.ready);
        assert_eq!(
            cannon.next_ready_ms, due,
            "the pre-shot frame sets no clock"
        );
    }

    #[test]
    fn the_reclaim_sentences_flip_aboard() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[(
                0,
                "You climb into Red Pod 1. The panel wakes when every pilot is seated.",
            )],
        );
        assert!(p.aboard);
        let applied = feed(
            &mut p,
            &[(1, "You are not at the station of a deployed unit.")],
        );
        assert!(!applied[0].journal);
        assert!(!p.aboard);
        feed(&mut p, &[(2, "Back aboard RA after 2 m 14 s away.")]);
        assert!(p.aboard);
        assert_eq!(p.reclaims, 1);
        assert_eq!(p.away_ms, 134_000);
        feed(
            &mut p,
            &[(
                3,
                "The server restarted and the match resumed from its last save; what happened after that save did not count.",
            )],
        );
        assert!(!p.aboard);
        feed(&mut p, &[(4, "Power online.")]);
        assert!(p.aboard);
        feed(
            &mut p,
            &[(5, "Red Tank 1 is flying on doctrine; BOARD takes it back.")],
        );
        assert!(!p.aboard);
        feed(&mut p, &[(6, "You are already in your pod.")]);
        assert!(p.aboard);
    }

    #[test]
    fn only_the_followed_match_moves_the_phase() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Connected as Pilot (account pilot, 1 connection)."),
                (1, "Welcome Hall #3"),
                (2, "Simulator Lobby #9"),
            ],
        );
        assert_eq!(p.phase, Phase::InCentre);
        assert_eq!(p.me.as_deref(), Some("Pilot"));
        // Before a claim, another match's lines move nothing.
        feed(
            &mut p,
            &[(3, "Match 7 is boarding. Every pilot: BOARD your pod.")],
        );
        assert_eq!(p.phase, Phase::InCentre);
        p.claimed(Side::Red, 1);
        feed(
            &mut p,
            &[(4, "Match 8 is boarding. Every pilot: BOARD your pod.")],
        );
        assert_eq!(p.following, Some(8));
        assert_eq!(p.phase, Phase::Boarding { match_id: 8 });
        feed(
            &mut p,
            &[
                (5, "Match 9 is running. Good hunting."),
                (
                    6,
                    "You climb into Red Pod 1. The panel wakes when every pilot is seated.",
                ),
            ],
        );
        assert_eq!(p.phase, Phase::Seated { match_id: 8 });
        feed(&mut p, &[(7, "Match 8 is running. Good hunting.")]);
        assert_eq!(p.phase, Phase::Running { match_id: 8 });
        feed(
            &mut p,
            &[(8, "Match 9 is over (red). Its units are checked in.")],
        );
        assert_eq!(p.phase, Phase::Running { match_id: 8 });
        feed(
            &mut p,
            &[
                (9, "The hatch of Red Pod 1 swings open."),
                (10, "Red Room #12"),
                (11, "Match 8 is over (blue). Its units are checked in."),
            ],
        );
        assert_eq!(
            p.phase,
            Phase::Over {
                match_id: 8,
                result: ResultWord::Blue
            }
        );
        assert!(!p.aboard);
        let kinds: Vec<EventKind> = p.events.iter().map(|e| e.kind).collect();
        assert!(
            kinds
                .iter()
                .filter(|k| **k == EventKind::PhaseChanged)
                .count()
                >= 5
        );
    }

    #[test]
    fn no_line_but_a_contact_row_or_notice_adds_a_contact() {
        let lines = [
            "Locked on BA, 62% to hit.",
            "cannon  ready  40 of 40 round(s)",
            "This unit carries no weapon.",
            "Callsign RA.",
            "Red Tank 1: power online, 27 kph, heading 090°",
            "  at 10 m east, 20 m north; scope holds a hostile",
            "  hull  intact  internals 60/60",
            "    front 400/400",
            "Range to BA: 440 m.",
            "Bearing to BA: 087° (rel 357°).",
            "Range to The Ford (A): 430 m.",
            "No lock: LOCK a contact first, or name one.",
            "No such contact.",
            "That unit makes 54 kph ahead and 27 astern; SPEED -27 to 54.",
            "Startup sequence begun; power comes online shortly.",
            "Steering to 90°.",
            "Making 27 kph.",
            "Locking BA.",
            "Firing cannon.",
            "You are not at the station of a deployed unit.",
            "Back aboard RA after 2 m 14 s away.",
            "Red Tank 1 is flying on doctrine; BOARD takes it back.",
            "Connected as Pilot (account pilot, 1 connection).",
            "Lock on BA broken: contact gone.",
            "Contact BA lost.",
            "Hit on BA: turret.",
            "Miss on BA: the target moved.",
            "BA destroyed.",
            "BA's turret is destroyed.",
            "Hit on the hull's front facet: 280 of 400 armour left.",
            "The left_track is destroyed: the tank is slowed.",
            "Power online.",
            "Held: something is in the way.",
            "Receiver degraded: something is jamming the sight.",
            "FIRE cannon refused. Still cycling: 4 s to go.",
            "Simulator centre: running (match 8)",
            "  red: 1 (RA): Pilot, pod Red Pod 1, seated",
            "  blue: 1 (BA): Ada",
            "Pilot takes red slot 1. `SIM START` begins the match.",
            "Match 8 is boarding. Every pilot: BOARD your pod.",
            "Match 8 is running. Good hunting.",
            "Match 8 is over (red). Its units are checked in.",
            "Red Room #12",
            "Hint: LOCK <contact> holds the contact your panel just named.",
            "The world is still starting; please try again shortly.",
            "Ada says, \"BA is at 087, X2 behind it.\"",
        ];
        assert!(lines.len() >= 40);
        let mut p = Picture::default();
        for (i, line) in lines.iter().enumerate() {
            let read = classify(line);
            assert!(
                !matches!(
                    read,
                    Line::ContactRow(_)
                        | Line::NewContact { .. }
                        | Line::Identified { .. }
                        | Line::Unidentified { .. }
                ),
                "{line}"
            );
            p.apply(i as u64 * 10, &read);
        }
        p.settle(100_000);
        assert!(
            p.contacts.is_empty(),
            "{:?}",
            p.contacts.keys().collect::<Vec<_>>()
        );
    }

    /// **A refusal answering the runner's own `FIRE` never un-counts a shot
    /// that left** (the machine gun cycles in a second, so a second `FIRE`
    /// inside two seconds of an ack is routine and its pre-check says the very
    /// sentence the arena's refusal says).
    #[test]
    fn a_refusal_that_answers_a_sent_fire_is_the_pre_check() {
        let mut p = Picture::default();
        p.sent_fire(0, "machine_gun");
        feed(&mut p, &[(20, "Firing machine_gun.")]);
        p.sent_fire(1_000, "machine_gun");
        feed(
            &mut p,
            &[(1_020, "FIRE machine_gun refused. Still cycling: 1 s to go.")],
        );
        let t = &p.tally;
        assert_eq!(
            (t.shots, t.refused_shots, t.refused_orders, t.fired()),
            (1, 0, 1, 1)
        );
        let mg = &p.weapons[0];
        assert_eq!(mg.fired_ms, Some(20), "the cycle anchor is kept");
        assert_eq!(mg.acks.len(), 1);
        // The pre-check says the clock was early: the slack (lowered by the
        // ack, held at its floor) grows before the rounded-up second counts.
        let slack = (FIRE_SLACK_START_MS
            .saturating_sub(FIRE_SLACK_DOWN_MS)
            .max(FIRE_SLACK_MIN_MS)
            + FIRE_SLACK_UP_MS)
            .min(FIRE_SLACK_MAX_MS);
        assert_eq!(mg.next_ready_ms, Some(1_020 + 1_000 + slack));
        // With no `FIRE` outstanding, the same sentence is the arena's refusal
        // of the acked shot, as before.
        feed(
            &mut p,
            &[(1_500, "FIRE machine_gun refused. Still cycling: 1 s to go.")],
        );
        assert_eq!((p.tally.refused_shots, p.tally.fired()), (1, 0));
        // A mark lapses after two seconds: a `FIRE` answered by something else
        // (a transient, then re-sent) does not turn a later arena refusal into
        // a pre-check.
        p.sent_fire(3_000, "machine_gun");
        feed(
            &mut p,
            &[(
                3_010,
                "The world is committing another action; please retry in a moment.",
            )],
        );
        p.sent_fire(3_050, "machine_gun");
        feed(
            &mut p,
            &[
                (3_100, "Firing machine_gun."),
                (
                    5_060,
                    "FIRE machine_gun refused. Empty: that weapon has nothing left to fire.",
                ),
            ],
        );
        assert_eq!(
            (p.tally.shots, p.tally.refused_shots, p.tally.refused_orders),
            (2, 2, 1),
            "the ack took one mark, the other lapsed at 5 050: the refusal is the arena's"
        );
        p.sent_fire(10_000, "machine_gun");
        p.sent_fire(10_010, "machine_gun");
        feed(
            &mut p,
            &[(
                12_500,
                "FIRE machine_gun refused. No lock: LOCK a contact first.",
            )],
        );
        assert!(p.open.fire_sent.is_empty(), "both marks lapsed");
        assert_eq!(p.tally.refused_orders, 2);
    }

    /// **A `ready` row read inside the guard counts when its rounds show the
    /// shot has left** (the machine gun's one-second cycle ends well inside the
    /// 1.5 s guard); the pre-shot frame, rounds unchanged, is still ignored.
    #[test]
    fn a_ready_row_whose_rounds_show_the_shot_left_is_read() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (100, "machine_gun (mg)  ready  500 of 500 round(s)"),
                (101, "Power online."),
                (1_000, "Firing machine_gun."),
                (1_300, "machine_gun (mg)  ready  500 of 500 round(s)"),
                (1_301, "Power online."),
            ],
        );
        let mg = &p.weapons[0];
        assert!(!mg.ready, "the pre-shot frame is ignored");
        assert_eq!((mg.rounds, mg.shots_since_read), (500, 1));
        assert_eq!(
            mg.next_ready_ms, None,
            "nor does it set the clock (the ack left none: no cycle known)"
        );
        feed(
            &mut p,
            &[
                (2_200, "machine_gun (mg)  ready  499 of 500 round(s)"),
                (2_201, "Power online."),
            ],
        );
        let mg = &p.weapons[0];
        assert!(
            mg.ready,
            "1.2 s after the ack, one round gone: the cycle is over"
        );
        assert_eq!(
            (mg.rounds, mg.shots_since_read, mg.read_ms),
            (499, 0, 2_200)
        );
        // Two acks since the last row: one round gone is not both.
        feed(
            &mut p,
            &[
                (3_000, "Firing machine_gun."),
                (3_100, "Firing machine_gun."),
                (3_900, "machine_gun (mg)  ready  498 of 500 round(s)"),
                (3_901, "Power online."),
            ],
        );
        assert!(!p.weapons[0].ready);
        assert_eq!(p.weapons[0].rounds, 499);
        feed(
            &mut p,
            &[
                (4_000, "machine_gun (mg)  ready  497 of 500 round(s)"),
                (4_001, "Power online."),
            ],
        );
        assert!(p.weapons[0].ready);
        assert_eq!(p.weapons[0].rounds, 497);
    }

    /// The slack an ack leaves behind a weapon whose slack was `slack`.
    fn after_ack(slack: u64) -> u64 {
        slack
            .saturating_sub(FIRE_SLACK_DOWN_MS)
            .max(FIRE_SLACK_MIN_MS)
    }

    /// **An ack sets the one clock from the learned cycle** (cut AK.1): the
    /// ack plus the cycle plus the slack the ack leaves; with no cycle learned
    /// yet there is no clock, and the row is read.
    #[test]
    fn an_ack_sets_the_clock_from_the_learned_cycle() {
        let mut p = Picture::default();
        feed(&mut p, &[(0, "Firing cannon.")]);
        let cannon = &p.weapons[0];
        assert_eq!(cannon.cycle_ms, None);
        assert_eq!(cannon.next_ready_ms, None, "no cycle learned: read the row");
        assert_eq!(cannon.slack_ms, after_ack(FIRE_SLACK_START_MS));
        feed(
            &mut p,
            &[
                (1_000, "cannon  cycling (5 s)  39 of 40 round(s)"),
                (1_001, "Power online."),
            ],
        );
        assert_eq!(p.weapons[0].cycle_ms, Some(6_000));
        // A row left the clock in the past; the next ack moves it on, whatever
        // that row said.
        feed(
            &mut p,
            &[
                (7_000, "cannon  ready  39 of 40 round(s)"),
                (7_001, "Power online."),
            ],
        );
        assert_eq!(p.weapons[0].next_ready_ms, Some(7_000));
        feed(&mut p, &[(10_000, "Firing cannon.")]);
        let cannon = &p.weapons[0];
        let slack = after_ack(after_ack(FIRE_SLACK_START_MS));
        assert_eq!(cannon.slack_ms, slack);
        assert_eq!(cannon.next_ready_ms, Some(10_000 + 6_000 + slack));
        assert!(!cannon.ready);
    }

    /// **The seconds of a `cycling` row and of a `Still cycling` refusal are
    /// rounded up** (the bot's own transcript: `6 s to go` a whole second into
    /// a 6 s cycle), so they are counted whole, plus the slack — never less
    /// half a second.
    #[test]
    fn a_cycling_row_and_a_refusal_count_the_rounded_up_seconds() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (2_000, "cannon  cycling (4 s)  39 of 40 round(s)"),
                (2_001, "Power online."),
            ],
        );
        let slack = FIRE_SLACK_START_MS;
        assert_eq!(p.weapons[0].slack_ms, slack);
        assert_eq!(p.weapons[0].next_ready_ms, Some(2_000 + 4_000 + slack));
        // A refusal no `FIRE` of the runner's is waiting on: the seconds, the
        // slack unmoved.
        feed(
            &mut p,
            &[(5_000, "FIRE cannon refused. Still cycling: 3 s to go.")],
        );
        assert_eq!(p.weapons[0].slack_ms, slack);
        assert_eq!(p.weapons[0].next_ready_ms, Some(5_000 + 3_000 + slack));
        assert!(!p.weapons[0].ready);
    }

    /// **The slack rises on the cockpit's own refusal and falls on an ack**,
    /// never past [`FIRE_SLACK_MAX_MS`] or below [`FIRE_SLACK_MIN_MS`].
    #[test]
    fn the_slack_rises_on_its_own_refusal_and_falls_on_an_ack() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "cannon  ready  40 of 40 round(s)"),
                (1, "Power online."),
            ],
        );
        let mut at = 1_000;
        let mut expected = FIRE_SLACK_START_MS;
        let rises = (FIRE_SLACK_MAX_MS - FIRE_SLACK_MIN_MS) / FIRE_SLACK_UP_MS + 2;
        for _ in 0..rises {
            p.sent_fire(at, "cannon");
            feed(
                &mut p,
                &[(at + 10, "FIRE cannon refused. Still cycling: 2 s to go.")],
            );
            expected = (expected + FIRE_SLACK_UP_MS).min(FIRE_SLACK_MAX_MS);
            assert_eq!(p.weapons[0].slack_ms, expected);
            assert_eq!(p.weapons[0].next_ready_ms, Some(at + 10 + 2_000 + expected));
            at += 3_000;
        }
        assert_eq!(p.weapons[0].slack_ms, FIRE_SLACK_MAX_MS, "held at the top");
        let falls = (FIRE_SLACK_MAX_MS - FIRE_SLACK_MIN_MS) / FIRE_SLACK_DOWN_MS + 2;
        for _ in 0..falls {
            feed(&mut p, &[(at, "Firing cannon.")]);
            expected = after_ack(expected);
            assert_eq!(p.weapons[0].slack_ms, expected);
            at += 3_000;
        }
        assert_eq!(
            p.weapons[0].slack_ms, FIRE_SLACK_MIN_MS,
            "held at the floor"
        );
    }

    /// **`FIRE` marks are kept per weapon** (cut AK.1): a cannon refusal read
    /// while only the machine gun's `FIRE` is outstanding is not the cannon's
    /// pre-check but the arena's refusal of the cannon's acked shot.
    #[test]
    fn fire_marks_are_kept_per_weapon() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "cannon  ready  40 of 40 round(s)"),
                (1, "machine_gun (mg)  ready  500 of 500 round(s)"),
                (2, "Power online."),
                (1_000, "Firing cannon."),
            ],
        );
        p.sent_fire(1_100, "machine_gun");
        feed(
            &mut p,
            &[(1_200, "FIRE cannon refused. Still cycling: 5 s to go.")],
        );
        let t = &p.tally;
        assert_eq!(
            (t.shots, t.refused_shots, t.refused_orders),
            (1, 1, 0),
            "paired with the cannon's ack"
        );
        assert_eq!(p.open.fire_sent.len(), 1, "the machine gun's mark stands");
        assert_eq!(
            p.weapons[0].slack_ms,
            after_ack(FIRE_SLACK_START_MS),
            "the arena's refusal does not raise the slack"
        );
        // The machine gun's own answer takes its mark, by its short name too.
        feed(&mut p, &[(1_250, "Firing mg.")]);
        assert!(p.open.fire_sent.is_empty());
    }

    /// **A shot the arena refused for something other than its cycle leaves
    /// the gun ready now** (it never left; the lane's own wait governs the
    /// retry).
    #[test]
    fn a_shot_the_arena_refused_leaves_the_gun_ready() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Firing cannon."),
                (1_000, "cannon  cycling (5 s)  39 of 40 round(s)"),
                (1_001, "Power online."),
                (10_000, "Firing cannon."),
            ],
        );
        assert_eq!(p.weapons[0].cycle_ms, Some(6_000));
        assert!(
            p.weapons[0].next_ready_ms > Some(16_000),
            "a whole cycle on"
        );
        feed(
            &mut p,
            &[(
                10_300,
                "FIRE cannon refused. Too many orders at once; try that again.",
            )],
        );
        assert_eq!(p.tally.refused_shots, 1, "paired with the ack");
        assert_eq!(p.weapons[0].next_ready_ms, Some(10_300));
    }

    /// **A `ready` row accepted inside the guard is ready now**: the machine
    /// gun's row whose rounds show the shot has left is the evidence, so the
    /// clock is the row's own time.
    #[test]
    fn a_ready_row_inside_the_guard_is_ready_now() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (100, "machine_gun (mg)  ready  500 of 500 round(s)"),
                (101, "Power online."),
                (1_000, "Firing machine_gun."),
                (2_200, "machine_gun (mg)  ready  499 of 500 round(s)"),
                (2_201, "Power online."),
            ],
        );
        const { assert!(2_200 - 1_000 < READY_AFTER_ACK_MS, "inside the guard") };
        let mg = &p.weapons[0];
        assert!(mg.ready);
        assert_eq!(mg.next_ready_ms, Some(2_200));
    }

    /// **A lock's fix is as fine as its plot is fresh** (`help cockpit`,
    /// "Seeing"): the refined fix's base is 10 m with no `plotted` clause, 20 m
    /// a second old, and the scope's 50 m older than that.
    #[test]
    fn a_lock_fix_is_as_fine_as_its_plot_is_fresh() {
        for (range_line, base) in [
            ("Range to BA: 440 m.", 10.0),
            ("Range to BA: 440 m, plotted 1 s ago.", 20.0),
            ("Range to BA: 440 m, plotted 4 s ago.", 50.0),
        ] {
            let mut p = Picture::default();
            feed(
                &mut p,
                &[
                    (0, "Locked on BA."),
                    (10_000, "Callsign RA."),
                    (10_000, "Red Tank 1: power online, 0 kph, heading 000°"),
                    (10_000, "  at 0 m east, 0 m north; scope holds a hostile"),
                    (10_100, &row_line("BA", "blue", 90, 450, 0)),
                    (10_300, range_line),
                ],
            );
            let fix = p.contacts["BA"].fixes.back().expect("a fix").clone();
            assert!(fix.from_lock, "{range_line}");
            let expected = base + 440.0 * 0.5_f64.to_radians() + 3.0 * 0.1;
            assert!(
                (fix.sigma_m - expected).abs() < 1e-9,
                "{range_line}: {} against {expected}",
                fix.sigma_m
            );
        }
    }

    /// **The `journal` flag is a contract** AJ.2's journal and AJ.3's commander
    /// read: every read reply and every ack is `false`, every notice, lobby
    /// line, room and stray line `true`.
    #[test]
    fn the_journal_flag_for_every_kind_of_line() {
        let quiet = [
            "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s",
            "No contacts.",
            "cannon  ready  40 of 40 round(s)",
            "This unit carries no weapon.",
            "Callsign RA.",
            "The panel has told you nothing yet.",
            "Red Tank 1: power off. The panel has told you nothing yet.",
            "No such contact.",
            "Range to BA: 440 m.",
            "Range to BA: 450 m, plotted 4 s ago.",
            "Bearing to BA: 087° (rel 357°).",
            "Range to The Ford (A): 430 m.",
            "Bearing to The Ford (A): 270° (rel 180°).",
            "No lock: LOCK a contact first.",
            "No lock: LOCK a contact first, or name one.",
            "That unit makes 54 kph ahead and 27 astern; SPEED -27 to 54.",
            "Your tank is destroyed and takes no orders. EXIT leaves the pod.",
            "EMCON on: the pod radiates.",
            "Startup sequence begun; power comes online shortly.",
            "Power is already online.",
            "Power is still coming up; hold on.",
            "Power not online: STARTUP first.",
            "Your tank cannot move.",
            "Steering to 90°.",
            "Making 27 kph.",
            "Locking BA.",
            "EMCON coming on; the pod radiates from the next pulse.",
            "EMCON is already off.",
            "Firing cannon.",
            "You are not at the station of a deployed unit.",
            "The world is committing another action; please retry in a moment.",
            "The world is still starting; please try again shortly.",
            "That cockpit will not wake for you: control state unknown",
            "The arena is busy; try that again in a moment.",
        ];
        for line in quiet {
            let read = classify(line);
            assert!(
                !matches!(read, Line::Other(_)),
                "{line} must read as what it is"
            );
            let applied = Picture::default().apply(0, &read);
            assert!(!applied.journal, "{line} is journalled");
        }
        // A STATUS row inside its block.
        let mut p = Picture::default();
        let applied = feed(
            &mut p,
            &[
                (0, "Callsign RA."),
                (0, "Red Tank 1: power online, 0 kph, heading 000°"),
                (0, "  at 10 m east, 20 m north; scope clear"),
                (0, "  nearest spot: The Ford (A), 120 m at 045°"),
                (0, "  heat 10 of 100"),
                (0, "  receiver degraded: 15 of 40 through cover, 60% reach"),
                (0, "  emcon off: the pod is silent"),
                (0, "  hull  intact  internals 60/60"),
                (0, "    front 400/400"),
            ],
        );
        assert!(applied.iter().all(|a| !a.journal));
        let loud = [
            "New contact X2: bearing 315°, effective.",
            "Contact X2 identified: BA, blue.",
            "BA is unidentified again: X2.",
            "Lock on BA broken: contact gone.",
            "Contact BA lost.",
            "Locked on BA.",
            "Hit on BA: turret.",
            "Miss.",
            "BA destroyed.",
            "BA's turret is destroyed.",
            "Hit on the hull's front facet: 280 of 400 armour left.",
            "The left_track is destroyed.",
            "Power online.",
            "Held: something is in the way.",
            "Receiver degraded: something is jamming the sight.",
            "FIRE cannon refused. Still cycling: 4 s to go.",
            "Back aboard RA after 2 m 14 s away.",
            "You climb into Red Pod 1. The panel wakes when every pilot is seated.",
            "The hatch of Red Pod 1 swings open.",
            "Simulator centre: idle",
            "Pilot takes red slot 1. `SIM START` begins the match.",
            "Match 8 is boarding. Every pilot: BOARD your pod.",
            "Match 8 is running. Good hunting.",
            "Match 8 is over (red). Its units are checked in.",
            "The simulator centre is running; that is not something you can do to it now.",
            "Red Room #12",
            "Hint: STARTUP brings your power up.",
            "Connected as Pilot (account pilot, 1 connection).",
            "Ada says, \"hello\"",
        ];
        for line in loud {
            let applied = Picture::default().apply(0, &classify(line));
            assert!(applied.journal, "{line} is not journalled");
        }
        // A STATUS row with no block open is someone else's words.
        let applied = Picture::default().apply(0, &classify("    front 0/400"));
        assert!(applied.journal);
    }

    /// Claimed red 1 and following match 8, at `phase`.
    fn following_eight(phase: &str) -> Picture {
        let mut p = Picture::default();
        p.claimed(Side::Red, 1);
        feed(
            &mut p,
            &[(1, "Match 8 is boarding. Every pilot: BOARD your pod.")],
        );
        if phase != "boarding" {
            feed(
                &mut p,
                &[(
                    2,
                    "You climb into Red Pod 1. The panel wakes when every pilot is seated.",
                )],
            );
        }
        if phase == "running" {
            feed(&mut p, &[(3, "Match 8 is running. Good hunting.")]);
        }
        p
    }

    /// **Every way a followed match can end moves the phase**, and only for
    /// the followed match.
    #[test]
    fn a_followed_match_ends_every_way_it_can() {
        let ends: [(&str, ResultWord); 4] = [
            ("Match {n} is aborted: nobody boarded.", ResultWord::Abort),
            (
                "Match {n} could not start: nope. The pods are free again.",
                ResultWord::Abort,
            ),
            (
                "Match {n} ends: every pilot has left it.",
                ResultWord::Timeout,
            ),
            (
                "Match {n} is over (draw). Its units are checked in.",
                ResultWord::Draw,
            ),
        ];
        for start in ["boarding", "seated", "running"] {
            for (sentence, word) in &ends {
                let mut p = following_eight(start);
                let before = p.phase.clone();
                let (other, ours) = (sentence.replace("{n}", "9"), sentence.replace("{n}", "8"));
                feed(&mut p, &[(10, other.as_str())]);
                assert_eq!(p.phase, before, "{sentence} for match 9 from {start}");
                feed(&mut p, &[(11, ours.as_str())]);
                assert_eq!(
                    p.phase,
                    Phase::Over {
                        match_id: 8,
                        result: word.clone()
                    },
                    "{sentence} from {start}"
                );
                assert!(!p.aboard);
            }
            // `calls the match off` names no match: it ends the followed one
            // before it runs, and a running match only by its `is over` line.
            let mut p = following_eight(start);
            let before = p.phase.clone();
            feed(&mut p, &[(10, "Ada calls the match off.")]);
            if start == "running" {
                assert_eq!(p.phase, before);
            } else {
                assert_eq!(
                    p.phase,
                    Phase::Over {
                        match_id: 8,
                        result: ResultWord::Abort
                    },
                    "called off from {start}"
                );
            }
        }
    }

    /// **The tally is per match**: a new followed match starts from a clean
    /// tank, scope and tally, and follows the new id.
    #[test]
    fn a_new_followed_match_starts_clean() {
        let mut p = following_eight("running");
        feed(
            &mut p,
            &[
                (10, "Callsign RA."),
                (10, "Red Tank 1: power online, 0 kph, heading 000°"),
                (10, "  at 10 m east, 20 m north; scope clear"),
                (20, "BA  blue  bearing 087° (rel 032°)  far  450 m  2 s"),
                (30, "Locked on BA."),
                (40, "Firing cannon."),
                (50, "Hit on BA: turret."),
                (60, "Hit on the hull's front facet: 280 of 400 armour left."),
                (70, "Match 8 is over (red). Its units are checked in."),
            ],
        );
        assert_eq!(p.tally.shots, 1);
        assert!(!p.contacts.is_empty() && p.lock.is_some());
        p.claimed(Side::Red, 1);
        feed(
            &mut p,
            &[(100, "Match 9 is boarding. Every pilot: BOARD your pod.")],
        );
        assert_eq!(p.following, Some(9));
        assert_eq!(p.phase, Phase::Boarding { match_id: 9 });
        assert_eq!(p.tally, Tally::default());
        assert!(p.contacts.is_empty() && p.weapons.is_empty() && p.spots.is_empty());
        assert_eq!(p.lock, None);
        assert_eq!(p.own.callsign, None);
        assert_eq!(p.own.pos_m, None);
        assert!(p.own.sections.is_empty());
    }

    /// **Rows out of their block change nothing** (review of AJ.1): a facet
    /// run is what a three-letter speaker's wrapped speech looks like, and a
    /// catch-up block's power row means nothing outside the block.
    #[test]
    fn indented_rows_count_only_inside_their_block() {
        let mut p = Picture::default();
        feed(
            &mut p,
            &[
                (0, "Callsign RA."),
                (0, "Red Tank 1: power online, 0 kph, heading 000°"),
                (0, "  hull  intact  internals 60/60"),
                (0, "    front 400/400"),
                (1, "No contacts."),
            ],
        );
        let before = p.own.sections.clone();
        feed(
            &mut p,
            &[
                (10, "    front 0/400"),
                (11, "  hull  destroyed  internals 0/60"),
                (12, "  at 900 m east, 900 m north; scope holds a hostile"),
                (13, "Red Tank 1: power off, 0 kph, heading 000°, DESTROYED"),
            ],
        );
        assert_eq!(p.own.sections, before);
        assert_eq!(p.own.pos_m, None);
        assert!(!p.own.destroyed);
        assert_eq!(p.own.power, Power::Online);
        // The catch-up block's own row counts; the same row alone does not.
        feed(&mut p, &[(20, "  Power off.")]);
        assert_eq!(p.own.power, Power::Online);
        feed(
            &mut p,
            &[
                (30, "Back aboard RA after 45 s away."),
                (30, "  The machine had no state."),
                (30, "  Power starting."),
            ],
        );
        assert_eq!(p.own.power, Power::Starting);
        feed(&mut p, &[(40, "No contacts."), (41, "  Power online.")]);
        assert_eq!(p.own.power, Power::Starting);
    }

    #[test]
    fn the_summary_fits_the_terminal_and_the_picture_serialises() {
        let mut p = lock_fixed_run(7, 1_000);
        p.claimed(Side::Red, 1);
        feed(
            &mut p,
            &[
                (20_000, "Firing cannon."),
                (20_001, "Range to The Crossroads (B): 850 m."),
            ],
        );
        let lines = p.summary_lines(20_500);
        assert!(
            lines
                .iter()
                .all(|line| line.chars().count() <= SUMMARY_COLUMNS)
        );
        assert!(
            lines.iter().any(|line| line.starts_with("contact BA")),
            "{lines:#?}"
        );
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"B\""));
    }
}
