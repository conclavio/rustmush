//! **The stock card** (17b cut M1b.1 the shape, 17b cut M1b.5 the setters,
//! the presets and the overlay; doc 17 §11 "The card", D-UM-43, D-UM-44).
//!
//! The card is the parameter block a stock mind reads: Standard Doctrine's
//! combat schema, in [`crate::config::ReflexConfig`]'s own units where a term
//! maps onto today's machine, so that the **Line** preset on its defaults
//! ([`line()`]) IS today's doctrine — the regression anchor. It lives on every
//! mind's row ([`crate::arena::mind::MindState::card`]), is handed to every
//! think ([`crate::arena::mind::ThinkJob::card`]), is folded into the state hash
//! inside item 3c and is carried by the snapshot from format 7 — the shape
//! landed in 17b cut M1b.1, the one cut that moved the format, so the sub-cuts
//! that read it move nothing carried.
//!
//! **17b cut M1b.5** gives it the rest of doc 17 §11:
//!
//! * [`StockCard::set`] — one slider by its name, in the slider's own units,
//!   **refused, never clamped** (D-UM-43): a value outside its range or off its
//!   step is a [`CardError::Range`] (or, for `fight_home_pct`'s halves, a
//!   [`CardError::Halves`]) naming the range and the step, a word outside its
//!   choices a [`CardError::Choice`] naming them, and an unknown name
//!   [`CardError::NoSuchSlider`]. The Compact floor is NOT a slider's range:
//!   `min_odds` is 10 to 95 here, and the floor (`[tacsim.minds]`, captured
//!   into each arena) is its caller's check — `@mind/live`'s off the match's
//!   mirror, and the arena's own at the card's landing
//!   (`Arena::apply_mind_cards`);
//! * [`preset`] and [`PRESETS`] — Standard Doctrine's six named cards over
//!   [`line()`], exactly doc 17 §11's lists and nothing more (D-UM-44);
//! * [`StockCard::name_of`] — the preset a card is, or `Sniper, edited
//!   (min_odds 70)` naming the first slider it differs from the nearest preset
//!   by;
//! * [`StockCard::as_reflex`] — **the overlay**: the doctrine with the card's
//!   mapped terms written over it, which is what `NativeStockMind` thinks on
//!   (`line(cfg).as_reflex(cfg) == cfg`: the anchor, by construction);
//! * [`StockCard::read_back`] — the five section lines `@mind/live` prints, one
//!   place, with the fire-discipline section inert and its reason on a hull
//!   with no weapons station (doc 17 §11: shown inert, never refused);
//! * [`CardChange`] — what `@mind/live` offers the arena: a change, never a
//!   card.
//!
//! **What T1 reads, and the eleven it does not** (17b's Q-M3). Through
//! [`StockCard::as_reflex`] `NativeStockMind` reads the six mapped terms
//! (`band_pct`, `break_off_pct`, `fight_home_pct`, `rally_s`, `search_s`,
//! `hunt_after_s`) and `prefer`'s weights; through
//! `crate::arena::stock_mind::arming_for` the fire discipline (`min_odds`,
//! `aspect`, `reserve_pct`). The other eleven — `stance`, `close_m`,
//! `orbit_sense`, `turn_into_hits`, `turn_s`, `screen_bearing`, `screen_arc`,
//! `opening`, `reserve_odds`, `switch_m`, `stop_unseen` — have no arm in
//! today's machine to drive: each needs a behaviour (an orbit, a screen of the
//! card's own bearing, a reserve spent at its own odds, a lock switch, an
//! opening) that 17b cut M2's Rhai Standard Doctrine writes. They are carried,
//! validated by `set` and hashed, so a card set today means the same thing
//! when M2 reads it, and [`StockCard::read_back`] marks each `[M2]` so nobody
//! mistakes a carried slider for a live one. `NoMind` ignores the card.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::conditions::ASPECTS_ANY;
use crate::arena::weapons::FrameWeapon;
use crate::config::ReflexConfig;

/// **The posture** (doc 17 §11 *Posture*; the bot's words plus the `ORDER`
/// objective's). `[M2]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stance {
    /// Hold where it stands (the default).
    Hold,
    /// Close to `close_m`.
    Close,
    /// Orbit the target at the engage band.
    Orbit,
    /// Go and look.
    Hunt,
    /// Screen a bearing.
    Screen,
}

impl Stance {
    /// The code byte the state hash folds.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Stance::Hold => 0,
            Stance::Close => 1,
            Stance::Orbit => 2,
            Stance::Hunt => 3,
            Stance::Screen => 4,
        }
    }
}

/// **Which way an orbit turns** (doc 17 §11). `[M2]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrbitSense {
    /// Clockwise.
    Cw,
    /// Counter-clockwise.
    Ccw,
    /// Whichever the ground suggests (the default).
    Auto,
}

impl OrbitSense {
    /// The code byte the state hash folds.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            OrbitSense::Cw => 0,
            OrbitSense::Ccw => 1,
            OrbitSense::Auto => 2,
        }
    }
}

/// **The opening** (doc 17 §11; the bot's muster opening). `[M2]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Opening {
    /// Wait at the muster (the default).
    Wait,
    /// Search.
    Search,
    /// Advance.
    Advance,
}

impl Opening {
    /// The code byte the state hash folds.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Opening::Wait => 0,
            Opening::Search => 1,
            Opening::Advance => 2,
        }
    }
}

/// **The aspect a slot fires at, as the card words it** (doc 17 §11 *Fire
/// discipline*).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AspectWord {
    /// Any aspect (the default).
    Any,
    /// A flank: left or right.
    Flank,
    /// The rear.
    Rear,
}

impl AspectWord {
    /// The code byte the state hash folds.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            AspectWord::Any => 0,
            AspectWord::Flank => 1,
            AspectWord::Rear => 2,
        }
    }

    /// **The slot mask this word arms** ([`crate::arena::conditions`]'s bits:
    /// `0` front, `1` left, `2` right, `3` rear).
    #[must_use]
    pub const fn mask(&self) -> u8 {
        match self {
            AspectWord::Any => ASPECTS_ANY,
            AspectWord::Flank => 0b0110,
            AspectWord::Rear => 0b1000,
        }
    }
}

/// **What a mind prefers when it chooses a lock** (doc 17 §11 *Targeting*),
/// each a labelled setting of the three weights; `best_odds` weighs the odds
/// column and is **not** the anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Prefer {
    /// The nearest.
    Nearest,
    /// The most exposed.
    Exposed,
    /// The most hurt.
    Hurt,
    /// The base weights (the default).
    Balanced,
    /// The best odds.
    BestOdds,
}

impl Prefer {
    /// The code byte the state hash folds.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Prefer::Nearest => 0,
            Prefer::Exposed => 1,
            Prefer::Hurt => 2,
            Prefer::Balanced => 3,
            Prefer::BestOdds => 4,
        }
    }
}

/// **Standard Doctrine's stock card** (doc 17 §11; D-UM-43), twenty-two terms
/// in four sections plus search, each in its stated unit. `Default` is
/// deliberately not derived: the card's defaults are the arena's own doctrine
/// ([`line()`]), never a second copy of its numbers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StockCard {
    // ---- posture ----
    /// The posture. `[M2]`.
    pub stance: Stance,
    /// The engage band, per mille of the weapon's reach — the card's `band_pct`
    /// × 10, `ReflexConfig::engage_band_permille`.
    pub engage_band_permille: u32,
    /// How close `close` drives, metres. `[M2]`.
    pub close_m: u32,
    /// Which way an orbit turns. `[M2]`.
    pub orbit_sense: OrbitSense,
    /// Whether the hull turns its front into hits. `[M2]`.
    pub turn_into_hits: bool,
    /// How long it holds the turn, seconds. `[M2]`.
    pub turn_s: u32,
    /// The bearing a screen faces, degrees. `[M2]`.
    pub screen_bearing: u32,
    /// The screen's arc, degrees. `[M2]`.
    pub screen_arc: u32,
    /// The opening. `[M2]`.
    pub opening: Opening,
    // ---- fire discipline (a weapons station only) ----
    /// The least chance a slot fires at, per mille — the card's `min_odds` ×
    /// 10; the Compact floor's 100 by default.
    pub min_odds_permille: u16,
    /// The aspect a slot fires at.
    pub aspect: AspectWord,
    /// The reserve a slot keeps, per cent of the load.
    pub reserve_pct: u32,
    /// The odds a reserve round is spent at, per cent. `[M2]`.
    pub reserve_odds_pct: u32,
    // ---- targeting ----
    /// What the mind prefers in a lock.
    pub prefer: Prefer,
    /// The range weight — `ReflexConfig::weight_range`, carried so `Balanced`
    /// round-trips.
    pub weight_range: u32,
    /// The facet weight — `ReflexConfig::weight_facet`.
    pub weight_facet: u32,
    /// The health weight — `ReflexConfig::weight_health`.
    pub weight_health: u32,
    /// How much nearer another hostile must be to take the lock, metres.
    /// `[M2]`.
    pub switch_m: u32,
    // ---- survival ----
    /// The health it breaks off at, per mille — the card's `break_off_pct` ×
    /// 10, `ReflexConfig::withdraw_permille`.
    pub withdraw_permille: u32,
    /// The floor under an en-route stand, per mille — the card's
    /// `fight_home_pct` × 10, in halves, `ReflexConfig::stand_min_permille`.
    pub stand_min_permille: u32,
    /// The lull before a withdrawn hull rallies, milliseconds — `rally_s` ×
    /// 1,000; `0` never rallies. `ReflexConfig::rally_ms`.
    pub rally_ms: u32,
    // ---- search ----
    /// How long it drives to a lost plot, milliseconds —
    /// `ReflexConfig::search_ms`.
    pub search_ms: u32,
    /// How long it waits before it hunts, milliseconds —
    /// `ReflexConfig::hunt_after_ms`.
    pub hunt_after_ms: u32,
    /// Whether it stops when it has seen nobody. `[M2]`.
    pub stop_unseen: bool,
}

/// **The Line preset** (doc 17 §11, D-UM-44): the arena's own `cfg` read
/// VERBATIM for every mapped term — `engage_band_permille`,
/// `withdraw_permille`, `stand_min_permille`, `rally_ms`, `search_ms`,
/// `hunt_after_ms` and the three weights — and section 11's defaults for the
/// rest. On a default config it is today's machine.
#[must_use]
pub fn line(cfg: &ReflexConfig) -> StockCard {
    StockCard {
        stance: Stance::Hold,
        engage_band_permille: cfg.engage_band_permille,
        close_m: 200,
        orbit_sense: OrbitSense::Auto,
        turn_into_hits: true,
        turn_s: 4,
        screen_bearing: 0,
        screen_arc: 120,
        opening: Opening::Wait,
        min_odds_permille: 100,
        aspect: AspectWord::Any,
        reserve_pct: 0,
        reserve_odds_pct: 75,
        prefer: Prefer::Balanced,
        weight_range: cfg.weight_range,
        weight_facet: cfg.weight_facet,
        weight_health: cfg.weight_health,
        switch_m: 100,
        withdraw_permille: cfg.withdraw_permille,
        stand_min_permille: cfg.stand_min_permille,
        rally_ms: cfg.rally_ms,
        search_ms: cfg.search_ms,
        hunt_after_ms: cfg.hunt_after_ms,
        stop_unseen: true,
    }
}

/// **Whether the card's fire-discipline section is inert on this hull** (doc
/// 17 §11: *a card section that drives a station the hull lacks is shown inert
/// with the reason*): a hull with no weapons.
#[must_use]
pub fn fire_section_inert(weapons: &[FrameWeapon]) -> bool {
    weapons.is_empty()
}

// ---------------------------------------------------------------------------
// 17b cut M1b.5: the setters, the presets, the overlay and the read-back
// ---------------------------------------------------------------------------

/// **Why a slider refused a value** (17b cut M1b.5; D-UM-43: refused, never
/// clamped). Each sentence names the slider and what it takes, in the
/// slider's own units; the caller adds what it was given.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CardError {
    /// A number outside the slider's range or off its step.
    #[error("{slider} is {low} to {high} in steps of {step}")]
    Range {
        /// The slider's name.
        slider: &'static str,
        /// The least value it takes.
        low: u32,
        /// The greatest.
        high: u32,
        /// The step between two values it takes.
        step: u32,
    },
    /// **A number outside a slider that steps in halves** — `fight_home_pct`
    /// alone (doc 17 §11: *the slider steps in halves*), whose step no whole
    /// number can name.
    #[error("{slider} is {low} to {high} in halves")]
    Halves {
        /// The slider's name.
        slider: &'static str,
        /// The least value it takes.
        low: u32,
        /// The greatest.
        high: u32,
    },
    /// A word outside the slider's choices.
    #[error("{slider} is {choices}")]
    Choice {
        /// The slider's name.
        slider: &'static str,
        /// Its choices, as a sentence lists them (`hold, close, orbit, hunt or
        /// screen`).
        choices: &'static str,
    },
    /// No slider has that name.
    #[error("no slider named {0}")]
    NoSuchSlider(String),
    /// **The doctrine pair broken** (17b cut M2.2, the lead's ruling (b)): a
    /// `break_off_pct` or `fight_home_pct` that would leave the card's
    /// `stand_min_permille` above its `withdraw_permille` — a hull that
    /// withdraws before it would stand and fight at home. Refused, never
    /// clamped; the two values as the card prints them (`17.5`, `10`).
    #[error(
        "{slider} would put fight_home_pct {fight_home} above break_off_pct {break_off}; \
         fight_home_pct is at most break_off_pct"
    )]
    Pair {
        /// The slider being set — `break_off_pct` or `fight_home_pct` — or
        /// the preset that made the card (`Rearguard`, [`StockCard::pair`]).
        slider: &'static str,
        /// `fight_home_pct` as it would read.
        fight_home: String,
        /// `break_off_pct` as it would read.
        break_off: String,
    },
}

/// **A change to a mind's card, as `@mind/live` offers it** (17b cut M1b.5;
/// doc 17 §22's M1b, D-UM-43): a change, not a card — the arena applies it
/// to ITS OWN row at the next pulse's start (`Arena::stage_card`, then
/// `Arena::apply_mind_cards`), so a world-side copy can never be written over
/// a card the arena has since changed, refused or resumed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum CardChange {
    /// One of [`PRESETS`], case-folded.
    Preset(SmolStr),
    /// One slider set by [`StockCard::set`].
    Set {
        /// The slider's name.
        slider: SmolStr,
        /// The value, in the slider's own units.
        value: SmolStr,
    },
}

/// **Standard Doctrine's six combat presets** (doc 17 §11, D-UM-44), in the
/// order doc 17 lists them — the order [`StockCard::name_of`] breaks a tie
/// in. [`preset`] reads each name case-folded.
pub const PRESETS: [&str; 6] = [
    "Line",
    "Sentry",
    "Sniper",
    "Skirmisher",
    "Hunter",
    "Rearguard",
];

/// **A preset by name** (doc 17 §11, D-UM-44), case-folded, over the arena's
/// own [`line()`] — so a preset changes exactly the sliders doc 17 lists for it
/// and every other term is Line's on this doctrine. A preset is a named card
/// and nothing more: it never carries a value a slider cannot show. `None`
/// for a name that is not one of [`PRESETS`].
#[must_use]
pub fn preset(name: &str, cfg: &ReflexConfig) -> Option<StockCard> {
    let mut card = line(cfg);
    match name.trim().to_ascii_lowercase().as_str() {
        "line" => {}
        // Screen, min_odds 55, opening wait, hunt_after 0, rally 0.
        "sentry" => {
            card.stance = Stance::Screen;
            card.min_odds_permille = 550;
            card.opening = Opening::Wait;
            card.hunt_after_ms = 0;
            card.rally_ms = 0;
        }
        // Hold, band 100, min_odds 65, reserve 30, stop_unseen on.
        "sniper" => {
            card.stance = Stance::Hold;
            card.engage_band_permille = 1_000;
            card.min_odds_permille = 650;
            card.reserve_pct = 30;
            card.stop_unseen = true;
        }
        // Orbit, band 85, aspect flank, min_odds 35.
        "skirmisher" => {
            card.stance = Stance::Orbit;
            card.engage_band_permille = 850;
            card.aspect = AspectWord::Flank;
            card.min_odds_permille = 350;
        }
        // Hunt, band 60, search 60, hunt_after 45, min_odds 30.
        "hunter" => {
            card.stance = Stance::Hunt;
            card.engage_band_permille = 600;
            card.search_ms = 60_000;
            card.hunt_after_ms = 45_000;
            card.min_odds_permille = 300;
        }
        // Hold, break_off 70, rally 0, min_odds 50.
        "rearguard" => {
            card.stance = Stance::Hold;
            card.withdraw_permille = 700;
            card.rally_ms = 0;
            card.min_odds_permille = 500;
        }
        _ => return None,
    }
    Some(card)
}

/// **A preset's code**: its index in [`PRESETS`], `word` case-folded — what a
/// [`ProfileSet`] holds — or `None` for a word that names none.
#[must_use]
pub fn preset_code(word: &str) -> Option<u8> {
    let word = word.trim();
    PRESETS
        .iter()
        .position(|name| name.eq_ignore_ascii_case(word))
        .and_then(|at| u8::try_from(at).ok())
}

/// **The profiles a hull is sealed with** (17b cut M5.2; doc 17 §11, §12;
/// the owner's OQ-3 of 2026-10-10: a profile is one of Standard Doctrine's
/// six [`PRESETS`], a slot seals up to N at the choice, N the class row's
/// `profiles` at the mind's sealed class — Class I's two — and no row holds
/// more than four, `[tacsim.minds].classes.*.profiles` being 1 to 4): each
/// code an index into [`PRESETS`], in the order they were chosen, the first
/// the profile that flies at the release.
///
/// Kept on [`crate::arena::mind::SealRef::profiles`], derived from the seal's
/// own names in [`crate::arena::mind::MindSeal::seal_ref`]; a per-match
/// constant, carried and **never folded**, as the rest of the reference is.
/// `Copy`, so `SealRef` stays `Copy` — which holds only because the profiles
/// 17b cut M5.2 seals are preset codes (17b cut M6's authored profiles change
/// that). Serialised
/// as the array of its codes (`[2, 0]` is Sniper then Line); the codes past
/// `len` are kept `0`, so two equal sets compare equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileSet {
    /// How many profiles the set holds, 1 to [`ProfileSet::MOST`].
    pub len: u8,
    /// The codes, the first `len` of them read.
    pub codes: [u8; 4],
}

impl ProfileSet {
    /// **The Line card alone** — what every seal whose `profiles` record is
    /// empty holds, every default seal, and what `SealRef::none` reads.
    pub const LINE: ProfileSet = ProfileSet {
        len: 1,
        codes: [0; 4],
    };

    /// The most profiles a set holds — the greatest a class row may store.
    pub const MOST: usize = 4;

    /// **The set a seal's names make** — [`ProfileSet::LINE`] for none (a seal
    /// whose set is the Line card alone writes no names), else each name's
    /// [`preset_code`] in order; `None` for a name that is no preset, or more
    /// than [`ProfileSet::MOST`].
    #[must_use]
    pub fn of_names<S: AsRef<str>>(names: &[S]) -> Option<ProfileSet> {
        if names.is_empty() {
            return Some(ProfileSet::LINE);
        }
        if names.len() > ProfileSet::MOST {
            return None;
        }
        let mut codes = [0_u8; 4];
        for (slot, name) in codes.iter_mut().zip(names) {
            *slot = preset_code(name.as_ref())?;
        }
        Some(ProfileSet {
            len: u8::try_from(names.len()).ok()?,
            codes,
        })
    }

    /// The codes held, in order.
    #[must_use]
    pub fn codes(&self) -> &[u8] {
        self.codes
            .get(..usize::from(self.len))
            .unwrap_or(&self.codes)
    }

    /// **The preset at `index`** in the set (`0` the first), or `None` past
    /// its end.
    #[must_use]
    pub fn name(&self, index: u8) -> Option<&'static str> {
        self.codes()
            .get(usize::from(index))
            .and_then(|code| PRESETS.get(usize::from(*code)))
            .copied()
    }

    /// **The card the profile at `index` flies** (17b cut M5.2; its review):
    /// its preset over `reflex`, or the Line card for an index past the set —
    /// the one rule a mind row laid on a hull's active profile
    /// (`Arena::active_card`) and the check-out's mirror cards (the first,
    /// `crate::arena_bridge::open_match`) share.
    #[must_use]
    pub fn card(&self, index: u8, reflex: &ReflexConfig) -> StockCard {
        self.name(index)
            .and_then(|name| preset(name, reflex))
            .unwrap_or_else(|| line(reflex))
    }

    /// The presets held, in order — what the keyframe's `mind` object lists
    /// as `profiles` (17b cut M5.4, `crate::match_::cockpit::mind_json`).
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.codes()
            .iter()
            .filter_map(|code| PRESETS.get(usize::from(*code)).copied())
    }
}

impl Serialize for ProfileSet {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.codes().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ProfileSet {
    /// The array of codes: 1 to [`ProfileSet::MOST`] of them, each an index
    /// into [`PRESETS`] — anything else is refused, never clamped.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let codes = Vec::<u8>::deserialize(deserializer)?;
        if codes.is_empty() || codes.len() > ProfileSet::MOST {
            return Err(serde::de::Error::invalid_length(
                codes.len(),
                &"1 to 4 profile codes",
            ));
        }
        let mut set = ProfileSet {
            len: 0,
            codes: [0; 4],
        };
        for (slot, code) in set.codes.iter_mut().zip(&codes) {
            if usize::from(*code) >= PRESETS.len() {
                return Err(serde::de::Error::invalid_value(
                    serde::de::Unexpected::Unsigned(u64::from(*code)),
                    &"a preset's code, 0 to 5",
                ));
            }
            *slot = *code;
        }
        set.len = u8::try_from(codes.len()).map_err(serde::de::Error::custom)?;
        Ok(set)
    }
}

/// **Which of its sealed profiles a hull's mind will fly, and when it last
/// switched** (17b cut M5.2; doc 17 §11, §12, D-UM-44): `index` into the
/// hull's [`crate::arena::mind::SealRef::profiles`] (`0`, the first, at
/// check-out) and the arena millisecond of the last switch (`None` until the
/// first) — the stamp D-UM-44's "at most once per 10 s" is checked against.
///
/// On [`crate::arena::state::UnitState::card_profile`] (`UnitState.profile`
/// is the hull's `UnitProfile`), laid at [`ActiveProfile::FIRST`] by
/// `Arena::new`; carried by the unit's derive, and folded into the state hash
/// as item 1's sub-item 20 **only when it is not [`ActiveProfile::FIRST`]**
/// (`crate::arena::hash::state_hash`), so a match that switches nothing folds
/// the bytes it always did. Both fields decide a later card and a refusal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveProfile {
    /// The profile flown: an index into the hull's sealed set.
    pub index: u8,
    /// The arena millisecond of the last switch, or `None`.
    pub switched_ms: Option<i64>,
}

impl ActiveProfile {
    /// **The first profile, never switched** — every hull at check-out.
    pub const FIRST: ActiveProfile = ActiveProfile {
        index: 0,
        switched_ms: None,
    };

    /// Whether this is [`ActiveProfile::FIRST`]: the first profile and no
    /// stamp — a switch back to the first still carries its stamp, so it is
    /// not.
    #[must_use]
    pub const fn is_first(&self) -> bool {
        self.index == 0 && self.switched_ms.is_none()
    }
}

/// **The card's twenty-one sliders**, in doc 17 §11's order — the order
/// [`StockCard::name_of`] names the first difference in and
/// [`StockCard::read_back`] prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slider {
    Stance,
    BandPct,
    CloseM,
    OrbitSense,
    TurnIntoHits,
    TurnS,
    ScreenBearing,
    ScreenArc,
    Opening,
    MinOdds,
    Aspect,
    ReservePct,
    ReserveOdds,
    Prefer,
    SwitchM,
    BreakOffPct,
    FightHomePct,
    RallyS,
    SearchS,
    HuntAfterS,
    StopUnseen,
}

/// Every slider, in order.
const SLIDERS: [Slider; 21] = [
    Slider::Stance,
    Slider::BandPct,
    Slider::CloseM,
    Slider::OrbitSense,
    Slider::TurnIntoHits,
    Slider::TurnS,
    Slider::ScreenBearing,
    Slider::ScreenArc,
    Slider::Opening,
    Slider::MinOdds,
    Slider::Aspect,
    Slider::ReservePct,
    Slider::ReserveOdds,
    Slider::Prefer,
    Slider::SwitchM,
    Slider::BreakOffPct,
    Slider::FightHomePct,
    Slider::RallyS,
    Slider::SearchS,
    Slider::HuntAfterS,
    Slider::StopUnseen,
];

impl Slider {
    /// The name `@mind/live` takes.
    const fn name(self) -> &'static str {
        match self {
            Slider::Stance => "stance",
            Slider::BandPct => "band_pct",
            Slider::CloseM => "close_m",
            Slider::OrbitSense => "orbit_sense",
            Slider::TurnIntoHits => "turn_into_hits",
            Slider::TurnS => "turn_s",
            Slider::ScreenBearing => "screen_bearing",
            Slider::ScreenArc => "screen_arc",
            Slider::Opening => "opening",
            Slider::MinOdds => "min_odds",
            Slider::Aspect => "aspect",
            Slider::ReservePct => "reserve_pct",
            Slider::ReserveOdds => "reserve_odds",
            Slider::Prefer => "prefer",
            Slider::SwitchM => "switch_m",
            Slider::BreakOffPct => "break_off_pct",
            Slider::FightHomePct => "fight_home_pct",
            Slider::RallyS => "rally_s",
            Slider::SearchS => "search_s",
            Slider::HuntAfterS => "hunt_after_s",
            Slider::StopUnseen => "stop_unseen",
        }
    }

    /// The slider a name names, case-folded.
    fn named(name: &str) -> Option<Slider> {
        let folded = name.trim().to_ascii_lowercase();
        SLIDERS
            .iter()
            .copied()
            .find(|slider| slider.name() == folded)
    }
}

/// **Whether `name` is a slider's name** (case-folded) — what `@mind/live`
/// asks to tell `foo 1` (no such slider or preset) from a slider's own
/// refusal.
#[must_use]
pub fn is_slider(name: &str) -> bool {
    Slider::named(name).is_some()
}

/// `value` as a whole number in `low..=high` on `step` from `low`, else the
/// slider's [`CardError::Range`].
fn stepped(slider: Slider, value: &str, low: u32, high: u32, step: u32) -> Result<u32, CardError> {
    let refused = CardError::Range {
        slider: slider.name(),
        low,
        high,
        step,
    };
    let text = value.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(refused);
    }
    let number: u32 = text.parse().map_err(|_| refused.clone())?;
    if number < low || number > high || !(number - low).is_multiple_of(step.max(1)) {
        return Err(refused);
    }
    Ok(number)
}

/// `value` as one of `words` (case-folded), else the slider's
/// [`CardError::Choice`] listing `choices`.
fn chosen<T: Copy>(
    slider: Slider,
    value: &str,
    words: &[(&str, T)],
    choices: &'static str,
) -> Result<T, CardError> {
    let folded = value.trim().to_ascii_lowercase();
    words
        .iter()
        .find(|(word, _)| *word == folded)
        .map(|(_, choice)| *choice)
        .ok_or(CardError::Choice {
            slider: slider.name(),
            choices,
        })
}

/// `on` or `off`.
fn toggle(slider: Slider, value: &str) -> Result<bool, CardError> {
    chosen(slider, value, &[("on", true), ("off", false)], "on or off")
}

/// **`value` in halves of a per cent, as per mille** — `fight_home_pct`'s
/// `17.5` is `175` (the slider steps in halves, doc 17 §11): a whole number,
/// or one with a fraction of `.0` or `.5` (`.00`, `.50` too), from `0` to
/// `100`.
fn halves(slider: Slider, value: &str) -> Result<u32, CardError> {
    let refused = CardError::Halves {
        slider: slider.name(),
        low: 0,
        high: 100,
    };
    let text = value.trim();
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty() || !whole.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(refused);
    }
    let half = match fraction {
        "" if !text.ends_with('.') => 0,
        "0" | "00" => 0,
        "5" | "50" => 5,
        _ => return Err(refused),
    };
    let permille = whole
        .parse::<u32>()
        .ok()
        .and_then(|whole| whole.checked_mul(10))
        .and_then(|tenths| tenths.checked_add(half))
        .ok_or_else(|| refused.clone())?;
    if permille > 1_000 {
        return Err(refused);
    }
    Ok(permille)
}

/// **`value ÷ scale`, printed** — whole when it divides, else with its
/// fraction's digits and no trailing zero (`175 ÷ 10` is `17.5`; `scale` is
/// a power of ten).
fn decimal(value: u64, scale: u64) -> String {
    let scale = scale.max(1);
    let whole = value / scale;
    let rest = value % scale;
    if rest == 0 {
        return whole.to_string();
    }
    let digits = scale.to_string().len().saturating_sub(1);
    let fraction = format!("{rest:0digits$}");
    format!("{whole}.{}", fraction.trim_end_matches('0'))
}

/// An enum's snake-case word, as serde writes it.
fn word<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// `on` or `off`.
const fn on_off(flag: bool) -> &'static str {
    if flag { "on" } else { "off" }
}

/// The stance words, in [`Stance`]'s order.
const STANCES: [(&str, Stance); 5] = [
    ("hold", Stance::Hold),
    ("close", Stance::Close),
    ("orbit", Stance::Orbit),
    ("hunt", Stance::Hunt),
    ("screen", Stance::Screen),
];

/// The weights `prefer nearest` writes over the doctrine, `(range, facet,
/// health)` (doc 17 §11 *Targeting*: each choice a labelled setting of the
/// three): the range weighed twice over, the facet a half.
const NEAREST_WEIGHTS: (u32, u32, u32) = (200, 20, 30);
/// `exposed`: the facet weighed three times over.
const EXPOSED_WEIGHTS: (u32, u32, u32) = (100, 120, 30);
/// `hurt`: the health term weighed four times over.
const HURT_WEIGHTS: (u32, u32, u32) = (100, 40, 120);

impl StockCard {
    /// **Set one slider** (17b cut M1b.5; doc 17 §11, D-UM-43), by its name and
    /// in its own units — `band_pct 85`, `fight_home_pct 17.5`, `prefer
    /// exposed`, `stop_unseen off` — both case-folded.
    ///
    /// Every range and step is doc 17 §11's: `band_pct` 30 to 100 in steps of
    /// 5; `close_m` 100 to 400 in steps of 50; `orbit_sense` cw, ccw or auto;
    /// `turn_into_hits` on or off; `turn_s` 2 to 10; `screen_bearing` 0 to 359;
    /// `screen_arc` 60 to 180; `opening` wait, search or advance; `min_odds` 10
    /// to 95 in steps of 5 (the Compact floor is the caller's check, and the
    /// arena's at the card's landing); `aspect` any, flank or rear;
    /// `reserve_pct` 0 to 50 in steps of 5; `reserve_odds` 50 to 95;
    /// `prefer` nearest, exposed, hurt, balanced or best_odds; `switch_m` 50 to
    /// 300; `break_off_pct` 10 to 80; `fight_home_pct` 0 to 100 in halves;
    /// `rally_s` and `search_s` 0 to 120; `hunt_after_s` 0 to 300;
    /// `stop_unseen` on or off; `stance` hold, close, orbit, hunt or screen.
    ///
    /// **17b cut M2.2, the lead's ruling (b)**: `break_off_pct` and
    /// `fight_home_pct` are a PAIR — the new value is taken on a copy of the
    /// card and refused [`CardError::Pair`] when the copy would hold
    /// `stand_min_permille > withdraw_permille` (`fight_home_pct` above
    /// `break_off_pct`; equal is lawful).
    ///
    /// # Errors
    ///
    /// The [`CardError`] naming the range, the step, the choices or the
    /// pair; the card is untouched. **Never clamped.**
    pub fn set(&mut self, slider: &str, value: &str) -> Result<(), CardError> {
        let Some(which) = Slider::named(slider) else {
            return Err(CardError::NoSuchSlider(slider.to_owned()));
        };
        match which {
            Slider::Stance => {
                self.stance = chosen(which, value, &STANCES, "hold, close, orbit, hunt or screen")?;
            }
            Slider::BandPct => {
                self.engage_band_permille = stepped(which, value, 30, 100, 5)?.saturating_mul(10);
            }
            Slider::CloseM => self.close_m = stepped(which, value, 100, 400, 50)?,
            Slider::OrbitSense => {
                self.orbit_sense = chosen(
                    which,
                    value,
                    &[
                        ("cw", OrbitSense::Cw),
                        ("ccw", OrbitSense::Ccw),
                        ("auto", OrbitSense::Auto),
                    ],
                    "cw, ccw or auto",
                )?;
            }
            Slider::TurnIntoHits => self.turn_into_hits = toggle(which, value)?,
            Slider::TurnS => self.turn_s = stepped(which, value, 2, 10, 1)?,
            Slider::ScreenBearing => self.screen_bearing = stepped(which, value, 0, 359, 1)?,
            Slider::ScreenArc => self.screen_arc = stepped(which, value, 60, 180, 1)?,
            Slider::Opening => {
                self.opening = chosen(
                    which,
                    value,
                    &[
                        ("wait", Opening::Wait),
                        ("search", Opening::Search),
                        ("advance", Opening::Advance),
                    ],
                    "wait, search or advance",
                )?;
            }
            Slider::MinOdds => {
                let percent = stepped(which, value, 10, 95, 5)?;
                self.min_odds_permille = u16::try_from(percent.saturating_mul(10)).unwrap_or(950);
            }
            Slider::Aspect => {
                self.aspect = chosen(
                    which,
                    value,
                    &[
                        ("any", AspectWord::Any),
                        ("flank", AspectWord::Flank),
                        ("rear", AspectWord::Rear),
                    ],
                    "any, flank or rear",
                )?;
            }
            Slider::ReservePct => self.reserve_pct = stepped(which, value, 0, 50, 5)?,
            Slider::ReserveOdds => self.reserve_odds_pct = stepped(which, value, 50, 95, 1)?,
            Slider::Prefer => {
                self.prefer = chosen(
                    which,
                    value,
                    &[
                        ("nearest", Prefer::Nearest),
                        ("exposed", Prefer::Exposed),
                        ("hurt", Prefer::Hurt),
                        ("balanced", Prefer::Balanced),
                        ("best_odds", Prefer::BestOdds),
                    ],
                    "nearest, exposed, hurt, balanced or best_odds",
                )?;
            }
            Slider::SwitchM => self.switch_m = stepped(which, value, 50, 300, 1)?,
            Slider::BreakOffPct => {
                let mut next = self.clone();
                next.withdraw_permille = stepped(which, value, 10, 80, 1)?.saturating_mul(10);
                *self = next.paired(which)?;
            }
            Slider::FightHomePct => {
                let mut next = self.clone();
                next.stand_min_permille = halves(which, value)?;
                *self = next.paired(which)?;
            }
            Slider::RallyS => {
                self.rally_ms = stepped(which, value, 0, 120, 1)?.saturating_mul(1_000);
            }
            Slider::SearchS => {
                self.search_ms = stepped(which, value, 0, 120, 1)?.saturating_mul(1_000);
            }
            Slider::HuntAfterS => {
                self.hunt_after_ms = stepped(which, value, 0, 300, 1)?.saturating_mul(1_000);
            }
            Slider::StopUnseen => self.stop_unseen = toggle(which, value)?,
        }
        Ok(())
    }

    /// **The card, if its doctrine pair holds** (17b cut M2.2, ruling (b)):
    /// [`StockCard::pair`] naming `slider`.
    fn paired(self, slider: Slider) -> Result<StockCard, CardError> {
        self.pair(slider.name())?;
        Ok(self)
    }

    /// **Whether the doctrine pair holds** (17b cut M2.2, the lead's ruling
    /// (b)): `stand_min_permille <= withdraw_permille` — `fight_home_pct` at
    /// most `break_off_pct`, equal lawful — else [`CardError::Pair`] naming
    /// `named` (the slider being set, or the preset that made the card) and
    /// both values in their own units. [`StockCard::set`] reads it for the
    /// two sliders; the preset paths (`@mind/live`'s and the arena's
    /// `Arena::apply_mind_cards`) read it for a [`preset`], because
    /// *Rearguard* writes `break_off_pct` 70 over a doctrine whose
    /// `fight_home_pct` the config may set as high as its own `break_off_pct`
    /// (`[tacsim.arena.reflex]` admits 750 under 800): refused by name, never
    /// clamped.
    ///
    /// # Errors
    ///
    /// [`CardError::Pair`] when `fight_home_pct` reads above `break_off_pct`.
    pub fn pair(&self, named: &'static str) -> Result<(), CardError> {
        if self.stand_min_permille > self.withdraw_permille {
            return Err(CardError::Pair {
                slider: named,
                fight_home: self.reading(Slider::FightHomePct),
                break_off: self.reading(Slider::BreakOffPct),
            });
        }
        Ok(())
    }

    /// **One slider's reading, in its own units** — what `set` takes back
    /// (`17.5`, `best_odds`, `on`).
    fn reading(&self, slider: Slider) -> String {
        match slider {
            Slider::Stance => word(&self.stance),
            Slider::BandPct => decimal(u64::from(self.engage_band_permille), 10),
            Slider::CloseM => self.close_m.to_string(),
            Slider::OrbitSense => word(&self.orbit_sense),
            Slider::TurnIntoHits => on_off(self.turn_into_hits).to_owned(),
            Slider::TurnS => self.turn_s.to_string(),
            Slider::ScreenBearing => self.screen_bearing.to_string(),
            Slider::ScreenArc => self.screen_arc.to_string(),
            Slider::Opening => word(&self.opening),
            Slider::MinOdds => decimal(u64::from(self.min_odds_permille), 10),
            Slider::Aspect => word(&self.aspect),
            Slider::ReservePct => self.reserve_pct.to_string(),
            Slider::ReserveOdds => self.reserve_odds_pct.to_string(),
            Slider::Prefer => word(&self.prefer),
            Slider::SwitchM => self.switch_m.to_string(),
            Slider::BreakOffPct => decimal(u64::from(self.withdraw_permille), 10),
            Slider::FightHomePct => decimal(u64::from(self.stand_min_permille), 10),
            Slider::RallyS => decimal(u64::from(self.rally_ms), 1_000),
            Slider::SearchS => decimal(u64::from(self.search_ms), 1_000),
            Slider::HuntAfterS => decimal(u64::from(self.hunt_after_ms), 1_000),
            Slider::StopUnseen => on_off(self.stop_unseen).to_owned(),
        }
    }

    /// **The sliders this card reads otherwise than `before`**, by name, in
    /// doc 17 §11's order — what a `mind_card` line lists as `changed`.
    #[must_use]
    pub fn changed_from(&self, before: &StockCard) -> Vec<&'static str> {
        SLIDERS
            .iter()
            .filter(|slider| self.reading(**slider) != before.reading(**slider))
            .map(|slider| slider.name())
            .collect()
    }

    /// **The overlay** (17b cut M1b.5; doc 17 §11, D-UM-43): `base` with the
    /// card's mapped terms written over it — the doctrine `NativeStockMind`
    /// thinks on.
    ///
    /// `engage_band_permille`, `withdraw_permille`, `stand_min_permille`,
    /// `search_ms` and `hunt_after_ms` verbatim (`0` already turns the search
    /// and the hunt off in the doctrine's own reading); `rally_ms` verbatim
    /// except **`0`, which is written as `u32::MAX`**: the card's `rally_s 0`
    /// never rallies, where the doctrine would read `0` as "at once" (the stock
    /// mind's `t − since >= rally_ms` is an `i64` compare against
    /// `i64::from(u32::MAX)`, about 49 days of arena time, which no match
    /// reaches); the weights by `prefer` — `balanced` the card's own carried
    /// weights (Line's are the doctrine's, so it round-trips), `nearest`
    /// 200/20/30, `exposed` 100/120/30, `hurt` 100/40/120, and `best_odds` the
    /// balanced triple (the odds term the utility score does not weigh until
    /// 17b cut M2, declared). Every other term of `base` — the cadence, the path
    /// budget — is the match's and no card's.
    ///
    /// **`line(cfg).as_reflex(cfg) == cfg` for every `cfg` whose `rally_ms`
    /// is not `0`** (which `ReflexConfig`'s own validation guarantees: it is
    /// at or above `decide_ms`) — the regression anchor, pinned for the
    /// default and a non-default doctrine.
    #[must_use]
    pub fn as_reflex(&self, base: &ReflexConfig) -> ReflexConfig {
        let carried = (self.weight_range, self.weight_facet, self.weight_health);
        let (weight_range, weight_facet, weight_health) = match self.prefer {
            Prefer::Balanced | Prefer::BestOdds => carried,
            Prefer::Nearest => NEAREST_WEIGHTS,
            Prefer::Exposed => EXPOSED_WEIGHTS,
            Prefer::Hurt => HURT_WEIGHTS,
        };
        ReflexConfig {
            engage_band_permille: self.engage_band_permille,
            withdraw_permille: self.withdraw_permille,
            stand_min_permille: self.stand_min_permille,
            rally_ms: if self.rally_ms == 0 {
                u32::MAX
            } else {
                self.rally_ms
            },
            search_ms: self.search_ms,
            hunt_after_ms: self.hunt_after_ms,
            weight_range,
            weight_facet,
            weight_health,
            ..base.clone()
        }
    }

    /// **The card's name** (doc 17 §11: *the card then says `Line, edited
    /// (min_odds 60)` until it matches one again*): the preset's capitalised
    /// name when every slider reads as one of [`PRESETS`] on this doctrine;
    /// else `{Preset}, edited ({slider} {value})` against the NEAREST preset —
    /// the fewest sliders apart, a tie to the earlier in [`PRESETS`] — naming
    /// the first slider (doc 17 §11's order) it differs by, at the card's own
    /// value.
    #[must_use]
    pub fn name_of(&self, cfg: &ReflexConfig) -> String {
        let mine: Vec<String> = SLIDERS.iter().map(|slider| self.reading(*slider)).collect();
        let mut nearest: Option<(usize, &str, Option<Slider>)> = None;
        for name in PRESETS {
            let Some(card) = preset(name, cfg) else {
                continue;
            };
            let apart: Vec<Slider> = SLIDERS
                .iter()
                .zip(&mine)
                .filter(|(slider, reading)| card.reading(**slider) != **reading)
                .map(|(slider, _)| *slider)
                .collect();
            if nearest.is_none_or(|(count, ..)| apart.len() < count) {
                nearest = Some((apart.len(), name, apart.first().copied()));
            }
        }
        match nearest {
            Some((_, name, Some(slider))) => {
                format!(
                    "{name}, edited ({} {})",
                    slider.name(),
                    self.reading(slider)
                )
            }
            Some((_, name, None)) => name.to_owned(),
            None => "Line".to_owned(),
        }
    }

    /// **The card, read back** (17b cut M1b.5; doc 17 §11) — the five section
    /// lines `@mind/live` prints under its head, one place: posture, fire
    /// discipline, targeting, survival and search, each slider in its own
    /// units, the eleven T1 does not read until 17b cut M2 marked `[M2]` (this
    /// module's head). The band is printed in per cent alone: the metres it
    /// makes depend on the hull's sight and its loaded weapon, which the
    /// registry's mirror does not hold. `prefer`'s weights are the ones
    /// [`StockCard::as_reflex`] writes over `cfg`. On a hull with no weapons
    /// station (`armed` false) the fire-discipline line is the inert sentence
    /// and nothing else — shown, never refused (doc 17 §11).
    #[must_use]
    pub fn read_back(&self, cfg: &ReflexConfig, armed: bool) -> Vec<String> {
        let reflex = self.as_reflex(cfg);
        let line = |label: &str, body: String| format!("  {label:<17}{body}");
        let fire = if armed {
            format!(
                "min_odds {}%  aspect {}  reserve {}%  reserve_odds {}% [M2]  ident identified",
                self.reading(Slider::MinOdds),
                self.reading(Slider::Aspect),
                self.reserve_pct,
                self.reserve_odds_pct,
            )
        } else {
            "no weapons station on this hull".to_owned()
        };
        vec![
            line(
                "posture",
                format!(
                    "stance {} [M2]  band {}%  close {} m [M2]  orbit {} [M2]  turn into hits \
                     {} [M2] turn {} s [M2]  screen {:03}\u{b0} arc {}\u{b0} [M2]  opening {} \
                     [M2]",
                    self.reading(Slider::Stance),
                    self.reading(Slider::BandPct),
                    self.close_m,
                    self.reading(Slider::OrbitSense),
                    on_off(self.turn_into_hits),
                    self.turn_s,
                    self.screen_bearing,
                    self.screen_arc,
                    self.reading(Slider::Opening),
                ),
            ),
            line("fire discipline", fire),
            line(
                "targeting",
                format!(
                    "prefer {} ({}/{}/{})  switch {} m [M2]",
                    self.reading(Slider::Prefer),
                    reflex.weight_range,
                    reflex.weight_facet,
                    reflex.weight_health,
                    self.switch_m,
                ),
            ),
            line(
                "survival",
                format!(
                    "break_off {}%  fight_home {}%  rally {} s",
                    self.reading(Slider::BreakOffPct),
                    self.reading(Slider::FightHomePct),
                    self.reading(Slider::RallyS),
                ),
            ),
            line(
                "search",
                format!(
                    "search {} s  hunt_after {} s  stop_unseen {} [M2]",
                    self.reading(Slider::SearchS),
                    self.reading(Slider::HuntAfterS),
                    on_off(self.stop_unseen),
                ),
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **`line(cfg)` reads a non-default doctrine verbatim** and takes section
    /// 11's defaults for the rest.
    #[test]
    fn m1b1_line_reads_the_arenas_own_doctrine_verbatim() {
        let cfg = ReflexConfig {
            withdraw_permille: 410,
            weight_range: 70,
            weight_facet: 55,
            weight_health: 25,
            engage_band_permille: 850,
            rally_ms: 7_000,
            stand_min_permille: 205,
            search_ms: 30_000,
            hunt_after_ms: 45_000,
            ..ReflexConfig::default()
        };
        let card = line(&cfg);
        assert_eq!(
            (
                card.engage_band_permille,
                card.withdraw_permille,
                card.stand_min_permille,
                card.rally_ms,
                card.search_ms,
                card.hunt_after_ms,
                card.weight_range,
                card.weight_facet,
                card.weight_health,
            ),
            (850, 410, 205, 7_000, 30_000, 45_000, 70, 55, 25)
        );
        assert_eq!(card.stance, Stance::Hold);
        assert_eq!(card.close_m, 200);
        assert_eq!(card.orbit_sense, OrbitSense::Auto);
        assert!(card.turn_into_hits);
        assert_eq!(card.turn_s, 4);
        assert_eq!((card.screen_bearing, card.screen_arc), (0, 120));
        assert_eq!(card.opening, Opening::Wait);
        assert_eq!(card.min_odds_permille, 100);
        assert_eq!(card.aspect, AspectWord::Any);
        assert_eq!((card.reserve_pct, card.reserve_odds_pct), (0, 75));
        assert_eq!(card.prefer, Prefer::Balanced);
        assert_eq!(card.switch_m, 100);
        assert!(card.stop_unseen);
        // On the default doctrine it is the default doctrine's numbers.
        let stock = line(&ReflexConfig::default());
        assert_eq!(
            (stock.engage_band_permille, stock.withdraw_permille),
            (700, 350)
        );
    }

    /// **`AspectWord::mask`** arms the bits `crate::arena::conditions` reads.
    #[test]
    fn m1b1_an_aspect_word_arms_its_mask() {
        use crate::arena::conditions::{aspect_in, aspect_mask};
        use crate::arena::perception::Aspect;
        assert_eq!(AspectWord::Any.mask(), ASPECTS_ANY);
        assert_eq!(
            AspectWord::Flank.mask(),
            aspect_mask(&[Aspect::Left, Aspect::Right])
        );
        assert_eq!(AspectWord::Rear.mask(), aspect_mask(&[Aspect::Rear]));
        assert!(!aspect_in(AspectWord::Flank.mask(), Aspect::Front));
        assert!(aspect_in(AspectWord::Flank.mask(), Aspect::Left));
        assert!(fire_section_inert(&[]));
    }

    /// The card round-trips through serde as the snapshot carries it.
    #[test]
    fn m1b1_the_card_round_trips() {
        let card = line(&ReflexConfig::default());
        let text = serde_json::to_string(&card).expect("a card serialises");
        let back: StockCard = serde_json::from_str(&text).expect("and reads back");
        assert_eq!(back, card);
        assert!(text.contains("\"stance\":\"hold\""), "{text}");
        assert!(text.contains("\"prefer\":\"balanced\""), "{text}");
    }

    // ---- 17b cut M1b.5 ----

    /// A non-default doctrine every mapped term of which differs from the
    /// default's.
    fn other_doctrine() -> ReflexConfig {
        ReflexConfig {
            decide_ms: 500,
            repath_ms: 0,
            withdraw_permille: 410,
            weight_range: 70,
            weight_facet: 55,
            weight_health: 25,
            engage_band_permille: 850,
            rally_ms: 7_000,
            stand_min_permille: 205,
            search_ms: 30_000,
            hunt_after_ms: 45_000,
            ..ReflexConfig::default()
        }
    }

    /// **Every slider refuses outside its range and off its step by name, and
    /// never clamps** (doc 17 §11, D-UM-43): for each numeric slider the value
    /// under its low end, over its high end and (where it steps) between two
    /// steps is refused with the slider's one sentence, and the card is
    /// untouched; both ends are taken.
    #[test]
    fn m1b5_every_slider_refuses_out_of_range_and_off_step_by_name() {
        let cfg = ReflexConfig::default();
        let line = line(&cfg);
        // (slider, low, high, step, a value off the step or None)
        let numbers: [(&str, u32, u32, u32, Option<&str>); 13] = [
            ("band_pct", 30, 100, 5, Some("72")),
            ("close_m", 100, 400, 50, Some("125")),
            ("turn_s", 2, 10, 1, None),
            ("screen_bearing", 0, 359, 1, None),
            ("screen_arc", 60, 180, 1, None),
            ("min_odds", 10, 95, 5, Some("12")),
            ("reserve_pct", 0, 50, 5, Some("7")),
            ("reserve_odds", 50, 95, 1, None),
            ("switch_m", 50, 300, 1, None),
            ("break_off_pct", 10, 80, 1, None),
            ("rally_s", 0, 120, 1, None),
            ("search_s", 0, 120, 1, None),
            ("hunt_after_s", 0, 300, 1, None),
        ];
        for (slider, low, high, step, off) in numbers {
            let sentence = format!("{slider} is {low} to {high} in steps of {step}");
            let mut wrong: Vec<String> =
                vec![(high + 1).to_string(), "x".to_owned(), "-1".to_owned()];
            if low > 0 {
                wrong.push((low - 1).to_string());
            }
            wrong.extend(off.map(str::to_owned));
            for value in wrong {
                let mut card = line.clone();
                let refused = card.set(slider, &value).expect_err(&value);
                assert_eq!(refused.to_string(), sentence, "{slider} {value}");
                assert!(matches!(refused, CardError::Range { .. }));
                assert_eq!(card, line, "{slider} {value}: never clamped");
            }
            for value in [low, high] {
                // 17b cut M2.2, ruling (b): `break_off_pct`'s low end is
                // under Line's `fight_home_pct` 17.5 — the pair's refusal,
                // pinned in `m2_the_doctrine_pair_refuses_by_name` — so the
                // range's ends are taken with fight_home at nought.
                let mut card = line.clone();
                if slider == "break_off_pct" {
                    assert_eq!(card.set("fight_home_pct", "0"), Ok(()));
                }
                assert_eq!(
                    card.set(slider, &value.to_string()),
                    Ok(()),
                    "{slider} {value}"
                );
            }
        }
        // The halves.
        for value in ["100.5", "17.25", "17.", "-1", "x", "101"] {
            let mut card = line.clone();
            assert_eq!(
                card.set("fight_home_pct", value)
                    .map_err(|error| error.to_string()),
                Err("fight_home_pct is 0 to 100 in halves".to_owned()),
                "{value}"
            );
            assert_eq!(card, line);
        }
        // 17b cut M2.2, ruling (b): taken under `break_off_pct` 80, the
        // pair's widest; `fight_home_pct` 100 is in the slider's range and
        // above every `break_off_pct`, so the pair refuses it on any card.
        let wide = {
            let mut card = line.clone();
            assert_eq!(card.set("break_off_pct", "80"), Ok(()));
            card
        };
        for (value, permille) in [
            ("17.5", 175),
            ("0", 0),
            ("80", 800),
            ("42.0", 420),
            ("3.50", 35),
        ] {
            let mut card = wide.clone();
            assert_eq!(card.set("fight_home_pct", value), Ok(()), "{value}");
            assert_eq!(card.stand_min_permille, permille, "{value}");
        }
        let mut card = wide.clone();
        assert!(matches!(
            card.set("fight_home_pct", "100"),
            Err(CardError::Pair { .. })
        ));
        assert_eq!(card, wide);
        // The choices and the toggles.
        for (slider, choices) in [
            ("stance", "hold, close, orbit, hunt or screen"),
            ("orbit_sense", "cw, ccw or auto"),
            ("opening", "wait, search or advance"),
            ("aspect", "any, flank or rear"),
            ("prefer", "nearest, exposed, hurt, balanced or best_odds"),
            ("turn_into_hits", "on or off"),
            ("stop_unseen", "on or off"),
        ] {
            let mut card = line.clone();
            let refused = card.set(slider, "fast").expect_err(slider);
            assert_eq!(refused.to_string(), format!("{slider} is {choices}"));
            assert_eq!(card, line);
        }
        let mut card = line.clone();
        assert_eq!(
            card.set("foo", "1"),
            Err(CardError::NoSuchSlider("foo".to_owned()))
        );
        assert_eq!(
            CardError::NoSuchSlider("foo".to_owned()).to_string(),
            "no slider named foo"
        );
        // `ident` is a reading, not a slider (doc 17 §11: there is no `any`).
        assert!(matches!(
            card.set("ident", "any"),
            Err(CardError::NoSuchSlider(_))
        ));
        assert!(is_slider("MIN_ODDS") && !is_slider("sniper"));
        // Case-folded, units read: the stored terms.
        let mut card = line.clone();
        for (slider, value) in [
            ("BAND_PCT", "85"),
            ("min_odds", "70"),
            ("break_off_pct", "60"),
            ("rally_s", "45"),
            ("search_s", "0"),
            ("hunt_after_s", "300"),
            ("Stance", "ORBIT"),
            ("stop_unseen", "off"),
            ("turn_into_hits", "off"),
        ] {
            assert_eq!(card.set(slider, value), Ok(()), "{slider} {value}");
        }
        assert_eq!(
            (
                card.engage_band_permille,
                card.min_odds_permille,
                card.withdraw_permille,
                card.rally_ms,
                card.search_ms,
                card.hunt_after_ms,
                card.stance,
                card.stop_unseen,
                card.turn_into_hits,
            ),
            (
                850,
                700,
                600,
                45_000,
                0,
                300_000,
                Stance::Orbit,
                false,
                false
            )
        );
    }

    /// **The doctrine pair refuses by name** (17b cut M2.2, the lead's ruling
    /// (b)): on Line (`break_off_pct` 35, `fight_home_pct` 17.5) a
    /// `break_off_pct 10` is refused naming `break_off_pct` with both values
    /// as the card prints them, and a `fight_home_pct 40` naming
    /// `fight_home_pct`; the card untouched either way; `fight_home_pct 35`
    /// (equal) is lawful, and so is `break_off_pct 20` then `fight_home_pct
    /// 20`; every preset and `line(cfg)` holds the pair on the default
    /// doctrine, on `other_doctrine` and on `tests/minds.rs`' non-default
    /// doctrine (`withdraw_permille` 700, `stand_min_permille` 300); and on a
    /// doctrine the config admits with `fight_home` 75 under `break_off` 80,
    /// Rearguard (`break_off_pct` 70) is refused `Pair` by its own name.
    #[test]
    fn m2_the_doctrine_pair_refuses_by_name() {
        let cfg = ReflexConfig::default();
        let line = line(&cfg);
        assert_eq!(
            (line.withdraw_permille, line.stand_min_permille),
            (350, 175),
            "Line's pair on the default doctrine"
        );
        let mut card = line.clone();
        let refused = card.set("break_off_pct", "10").expect_err("under the pair");
        assert_eq!(
            refused,
            CardError::Pair {
                slider: "break_off_pct",
                fight_home: "17.5".to_owned(),
                break_off: "10".to_owned(),
            }
        );
        assert_eq!(
            refused.to_string(),
            "break_off_pct would put fight_home_pct 17.5 above break_off_pct 10; \
             fight_home_pct is at most break_off_pct"
        );
        assert_eq!(card, line, "untouched");
        let refused = card.set("fight_home_pct", "40").expect_err("over the pair");
        assert_eq!(
            refused,
            CardError::Pair {
                slider: "fight_home_pct",
                fight_home: "40".to_owned(),
                break_off: "35".to_owned(),
            }
        );
        assert_eq!(
            refused.to_string(),
            "fight_home_pct would put fight_home_pct 40 above break_off_pct 35; \
             fight_home_pct is at most break_off_pct"
        );
        assert_eq!(card, line, "untouched");
        assert_eq!(card.set("fight_home_pct", "35"), Ok(()), "equal is lawful");
        assert_eq!(card.stand_min_permille, 350);
        let mut card = line.clone();
        assert_eq!(card.set("break_off_pct", "20"), Ok(()));
        assert_eq!(card.set("fight_home_pct", "20"), Ok(()));
        assert_eq!(
            (card.withdraw_permille, card.stand_min_permille),
            (200, 200)
        );
        let non_default = ReflexConfig {
            withdraw_permille: 700,
            stand_min_permille: 300,
            ..other_doctrine()
        };
        for cfg in [ReflexConfig::default(), other_doctrine(), non_default] {
            let line = super::line(&cfg);
            assert!(
                line.stand_min_permille <= line.withdraw_permille,
                "line on {cfg:?}"
            );
            for name in PRESETS {
                let card = preset(name, &cfg).expect(name);
                assert!(
                    card.stand_min_permille <= card.withdraw_permille,
                    "{name} on {cfg:?}"
                );
                assert_eq!(card.pair(name), Ok(()), "{name} on {cfg:?}");
            }
        }
        // A doctrine the config admits (`fight_home` 75 under `break_off`
        // 80) whose `fight_home_pct` reads above Rearguard's `break_off_pct`
        // 70: Line and the five presets that leave the pair alone hold it,
        // and Rearguard's card is refused by its own name — never clamped.
        let high = ReflexConfig {
            withdraw_permille: 800,
            stand_min_permille: 750,
            ..ReflexConfig::default()
        };
        assert_eq!(
            crate::config::ArenaConfig {
                reflex: high.clone(),
                ..crate::config::ArenaConfig::default()
            }
            .validate(),
            Ok(()),
            "the config admits the pair"
        );
        assert_eq!(super::line(&high).pair("Line"), Ok(()));
        for name in PRESETS {
            let card = preset(name, &high).expect(name);
            if name == "Rearguard" {
                let refused = card.pair(name).expect_err("Rearguard breaks the pair");
                assert_eq!(
                    refused,
                    CardError::Pair {
                        slider: "Rearguard",
                        fight_home: "75".to_owned(),
                        break_off: "70".to_owned(),
                    }
                );
                assert_eq!(
                    refused.to_string(),
                    "Rearguard would put fight_home_pct 75 above break_off_pct 70; \
                     fight_home_pct is at most break_off_pct"
                );
            } else {
                assert_eq!(card.pair(name), Ok(()), "{name} on the high doctrine");
            }
        }
    }

    /// **`rally_s 0` never rallies** — the overlay writes `u32::MAX` where the
    /// doctrine would read `0` as at once — and a rally the card names is the
    /// doctrine's milliseconds.
    #[test]
    fn m1b5_rally_nought_is_never_and_the_overlay_saturates() {
        let cfg = ReflexConfig::default();
        let mut card = line(&cfg);
        assert_eq!(card.set("rally_s", "0"), Ok(()));
        assert_eq!(card.rally_ms, 0);
        assert_eq!(card.as_reflex(&cfg).rally_ms, u32::MAX);
        // The stock mind's compare: `t − since >= rally_ms`, in `i64`.
        let since = 0_i64;
        let t_ms = i64::from(u32::MAX) - 1;
        assert!(t_ms.saturating_sub(since) < i64::from(card.as_reflex(&cfg).rally_ms));
        assert_eq!(card.set("rally_s", "120"), Ok(()));
        assert_eq!(card.as_reflex(&cfg).rally_ms, 120_000);
    }

    /// **`prefer` round-trips and writes its labelled weights** (doc 17 §11
    /// *Targeting*): every word sets and reads back as itself; `balanced` and
    /// `best_odds` write the card's carried weights (the doctrine's, on Line),
    /// `nearest`, `exposed` and `hurt` their triples.
    #[test]
    fn m1b5_prefer_round_trips_and_writes_its_weights() {
        let cfg = other_doctrine();
        for (word, weights) in [
            ("nearest", (200, 20, 30)),
            ("exposed", (100, 120, 30)),
            ("hurt", (100, 40, 120)),
            ("balanced", (70, 55, 25)),
            ("best_odds", (70, 55, 25)),
        ] {
            let mut card = line(&cfg);
            assert_eq!(card.set("prefer", word), Ok(()));
            assert_eq!(card.reading(Slider::Prefer), word);
            let reflex = card.as_reflex(&cfg);
            assert_eq!(
                (
                    reflex.weight_range,
                    reflex.weight_facet,
                    reflex.weight_health
                ),
                weights,
                "{word}"
            );
            // The carried weights are untouched: `balanced` comes back.
            assert_eq!(card.set("prefer", "balanced"), Ok(()));
            assert_eq!(card, line(&cfg), "{word} round-trips");
        }
    }

    /// **Each preset is doc 17 §11's list over Line, and nothing more**
    /// (D-UM-44): the sliders each names, and every other slider Line's.
    #[test]
    fn m1b5_each_preset_is_doc_seventeens_list_and_nothing_more() {
        let cfg = ReflexConfig::default();
        let line = line(&cfg);
        let differs = |card: &StockCard| -> Vec<(&'static str, String)> {
            SLIDERS
                .iter()
                .filter(|slider| card.reading(**slider) != line.reading(**slider))
                .map(|slider| (slider.name(), card.reading(*slider)))
                .collect()
        };
        let named = |pairs: &[(&'static str, &str)]| -> Vec<(&'static str, String)> {
            pairs
                .iter()
                .map(|(slider, value)| (*slider, (*value).to_owned()))
                .collect()
        };
        assert_eq!(preset("LINE", &cfg), Some(line.clone()));
        // Sentry: screen, min_odds 55, opening wait (Line's), hunt_after 0, rally 0.
        let sentry = preset("sentry", &cfg).expect("sentry");
        assert_eq!(sentry.opening, Opening::Wait);
        assert_eq!(
            differs(&sentry),
            named(&[
                ("stance", "screen"),
                ("min_odds", "55"),
                ("rally_s", "0"),
                ("hunt_after_s", "0"),
            ])
        );
        // Sniper: hold (Line's), band 100, min_odds 65, reserve 30, stop_unseen on (Line's).
        let sniper = preset("Sniper", &cfg).expect("sniper");
        assert_eq!((sniper.stance, sniper.stop_unseen), (Stance::Hold, true));
        assert_eq!(
            differs(&sniper),
            named(&[
                ("band_pct", "100"),
                ("min_odds", "65"),
                ("reserve_pct", "30")
            ])
        );
        assert_eq!(
            differs(&preset("skirmisher", &cfg).expect("skirmisher")),
            named(&[
                ("stance", "orbit"),
                ("band_pct", "85"),
                ("min_odds", "35"),
                ("aspect", "flank"),
            ])
        );
        // Hunter: hunt, band 60, search 60 (Line's), hunt_after 45, min_odds 30.
        let hunter = preset("hunter", &cfg).expect("hunter");
        assert_eq!(hunter.search_ms, 60_000);
        assert_eq!(
            differs(&hunter),
            named(&[
                ("stance", "hunt"),
                ("band_pct", "60"),
                ("min_odds", "30"),
                ("hunt_after_s", "45"),
            ])
        );
        // Rearguard: hold (Line's), break_off 70, rally 0, min_odds 50.
        assert_eq!(
            differs(&preset("rearguard", &cfg).expect("rearguard")),
            named(&[
                ("min_odds", "50"),
                ("break_off_pct", "70"),
                ("rally_s", "0")
            ])
        );
        assert_eq!(preset("berserker", &cfg), None);
        assert_eq!(PRESETS.len(), 6);
        // Every preset is lawful under every floor the bound allows (10 to
        // 100 per mille) and every slider of it reads back through `set`.
        for name in PRESETS {
            let card = preset(name, &cfg).expect(name);
            assert!(card.min_odds_permille >= 100, "{name}");
            let mut rebuilt = line.clone();
            for slider in SLIDERS {
                assert_eq!(
                    rebuilt.set(slider.name(), &card.reading(slider)),
                    Ok(()),
                    "{name} {}",
                    slider.name()
                );
            }
            assert_eq!(rebuilt, card, "{name} is sliders and nothing more");
        }
    }

    /// **`name_of`** (doc 17 §11): each preset by its capitalised name, on the
    /// default and a non-default doctrine; an edited Line and an edited
    /// Sniper by the nearest preset and the first slider apart; a card that
    /// matches a preset again by that preset's name.
    #[test]
    fn m1b5_name_of_names_the_preset_or_the_first_edit_off_the_nearest() {
        for cfg in [ReflexConfig::default(), other_doctrine()] {
            for name in PRESETS {
                assert_eq!(preset(name, &cfg).expect(name).name_of(&cfg), name);
            }
            let mut card = line(&cfg);
            assert_eq!(card.set("min_odds", "60"), Ok(()));
            assert_eq!(card.name_of(&cfg), "Line, edited (min_odds 60)");
            let mut sniper = preset("sniper", &cfg).expect("sniper");
            assert_eq!(sniper.set("min_odds", "70"), Ok(()));
            assert_eq!(sniper.name_of(&cfg), "Sniper, edited (min_odds 70)");
            assert_eq!(sniper.set("min_odds", "65"), Ok(()));
            assert_eq!(sniper.name_of(&cfg), "Sniper");
        }
        // Nearest by count: a Rearguard with Line's rally is one slider from
        // Rearguard and two from Line.
        let cfg = ReflexConfig::default();
        let mut card = preset("rearguard", &cfg).expect("rearguard");
        assert_eq!(card.set("rally_s", "20"), Ok(()));
        assert_eq!(card.name_of(&cfg), "Rearguard, edited (rally_s 20)");
        // A tie goes to the earlier preset: band 100 and min_odds 40 is two
        // from Line (band, min_odds) and two from Sniper (min_odds, reserve).
        let mut card = line(&cfg);
        assert_eq!(card.set("band_pct", "100"), Ok(()));
        assert_eq!(card.set("min_odds", "40"), Ok(()));
        assert_eq!(card.name_of(&cfg), "Line, edited (band_pct 100)");
    }

    /// **The overlay** (D-UM-43): `line(cfg).as_reflex(cfg) == cfg` for the
    /// default and a non-default doctrine — the anchor — and each preset
    /// writes its mapped terms and nothing else of the doctrine.
    #[test]
    fn m1b5_line_overlays_to_its_own_doctrine_and_each_preset_to_its_terms() {
        for cfg in [ReflexConfig::default(), other_doctrine()] {
            assert_eq!(line(&cfg).as_reflex(&cfg), cfg);
            for name in PRESETS {
                let card = preset(name, &cfg).expect(name);
                let reflex = card.as_reflex(&cfg);
                // The match's own terms never move.
                assert_eq!(
                    (
                        reflex.decide_ms,
                        reflex.path_max_cells,
                        reflex.max_path_queries,
                        reflex.repath_ms
                    ),
                    (
                        cfg.decide_ms,
                        cfg.path_max_cells,
                        cfg.max_path_queries,
                        cfg.repath_ms
                    ),
                    "{name}"
                );
                assert_eq!(reflex.engage_band_permille, card.engage_band_permille);
                assert_eq!(reflex.withdraw_permille, card.withdraw_permille);
                assert_eq!(reflex.stand_min_permille, card.stand_min_permille);
                assert_eq!(reflex.search_ms, card.search_ms);
                assert_eq!(reflex.hunt_after_ms, card.hunt_after_ms);
                assert_eq!(
                    reflex.rally_ms,
                    if card.rally_ms == 0 {
                        u32::MAX
                    } else {
                        card.rally_ms
                    }
                );
            }
        }
        let cfg = ReflexConfig::default();
        let at = |name: &str| preset(name, &cfg).expect(name).as_reflex(&cfg);
        assert_eq!(
            (
                at("sentry").rally_ms,
                at("sentry").hunt_after_ms,
                at("sniper").engage_band_permille,
                at("skirmisher").engage_band_permille,
                at("hunter").hunt_after_ms,
                at("hunter").engage_band_permille,
                at("rearguard").withdraw_permille,
                at("rearguard").rally_ms,
            ),
            (u32::MAX, 0, 1_000, 850, 45_000, 600, 700, u32::MAX)
        );
    }

    /// **The read-back** (doc 17 §11): five section lines, the eleven unread
    /// sliders marked `[M2]`, and on an unarmed hull the fire-discipline line
    /// is the inert sentence alone.
    #[test]
    fn m1b5_the_read_back_marks_the_unread_sliders_and_an_unarmed_hull_inert() {
        let cfg = ReflexConfig::default();
        let card = line(&cfg);
        assert_eq!(
            card.read_back(&cfg, true),
            vec![
                "  posture          stance hold [M2]  band 70%  close 200 m [M2]  orbit auto \
                 [M2]  turn into hits on [M2] turn 4 s [M2]  screen 000\u{b0} arc 120\u{b0} [M2]  \
                 opening wait [M2]"
                    .to_owned(),
                "  fire discipline  min_odds 10%  aspect any  reserve 0%  reserve_odds 75% [M2]  \
                 ident identified"
                    .to_owned(),
                "  targeting        prefer balanced (100/40/30)  switch 100 m [M2]".to_owned(),
                "  survival         break_off 35%  fight_home 17.5%  rally 20 s".to_owned(),
                "  search           search 60 s  hunt_after 90 s  stop_unseen on [M2]".to_owned(),
            ]
        );
        let unarmed = card.read_back(&cfg, false);
        assert_eq!(
            unarmed.get(1).map(String::as_str),
            Some("  fire discipline  no weapons station on this hull")
        );
        // Ten `[M2]` marks for the eleven unread sliders on an armed hull
        // (`screen_bearing` and `screen_arc` share the screen's one), nine on
        // an unarmed one (its `reserve_odds` is inside the inert section).
        let marks = |lines: &[String]| {
            lines
                .iter()
                .map(|line| line.matches("[M2]").count())
                .sum::<usize>()
        };
        assert_eq!(marks(&card.read_back(&cfg, true)), 10);
        assert_eq!(marks(&unarmed), 9);
        let sniper = preset("sniper", &cfg).expect("sniper");
        let lines = sniper.read_back(&cfg, true);
        assert!(
            lines.iter().any(|line| line.contains("band 100%")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("min_odds 65%  aspect any  reserve 30%")),
            "{lines:?}"
        );
        let mut hurt = line(&cfg);
        assert_eq!(hurt.set("prefer", "hurt"), Ok(()));
        assert!(
            hurt.read_back(&cfg, true)
                .iter()
                .any(|line| line.contains("prefer hurt (100/40/120)"))
        );
    }

    /// **17b cut M5.2: the sealed set and the active profile** (doc 17 §11,
    /// §12; the owner's OQ-3): a set made of a seal's names holds each
    /// preset's code in order, case-folded, and [`ProfileSet::LINE`] for none;
    /// a name that is no preset, or a fifth, makes none. It is written as the
    /// array of its codes and read back only as one to four codes each a
    /// preset's. [`ActiveProfile::FIRST`] is the default and the only value
    /// [`ActiveProfile::is_first`] holds for — a switch back to the first
    /// keeps its stamp.
    #[test]
    fn m5_a_profile_set_is_its_preset_codes_and_the_first_profile_is_the_default() {
        assert_eq!(ProfileSet::of_names::<&str>(&[]), Some(ProfileSet::LINE));
        let set = ProfileSet::of_names(&["sniper", "LINE"]).expect("two presets");
        assert_eq!(
            set,
            ProfileSet {
                len: 2,
                codes: [2, 0, 0, 0],
            }
        );
        assert_eq!(set.codes(), &[2, 0]);
        assert_eq!(set.names().collect::<Vec<_>>(), vec!["Sniper", "Line"]);
        assert_eq!(
            (set.name(0), set.name(1), set.name(2)),
            (Some("Sniper"), Some("Line"), None)
        );
        assert_eq!(ProfileSet::LINE.names().collect::<Vec<_>>(), vec!["Line"]);
        assert_eq!(ProfileSet::of_names(&["Line", "Brawler"]), None);
        assert_eq!(
            ProfileSet::of_names(&["Line", "Sentry", "Sniper", "Hunter", "Rearguard"]),
            None
        );
        let four = ProfileSet::of_names(&["Rearguard", "Hunter", "Skirmisher", "Sentry"])
            .expect("four presets");
        for set in [ProfileSet::LINE, set, four] {
            let wire = serde_json::to_value(set).expect("a set writes");
            assert_eq!(
                wire,
                serde_json::json!(set.codes()),
                "the array of its codes"
            );
            assert_eq!(serde_json::from_value::<ProfileSet>(wire).ok(), Some(set));
        }
        for refused in [
            serde_json::json!([]),
            serde_json::json!([0, 1, 2, 3, 4]),
            serde_json::json!([6]),
            serde_json::json!({"len": 1, "codes": [0, 0, 0, 0]}),
        ] {
            assert!(
                serde_json::from_value::<ProfileSet>(refused.clone()).is_err(),
                "{refused}"
            );
        }
        assert_eq!(ActiveProfile::default(), ActiveProfile::FIRST);
        assert!(ActiveProfile::FIRST.is_first());
        for profile in [
            ActiveProfile {
                index: 1,
                switched_ms: Some(5_000),
            },
            ActiveProfile {
                index: 0,
                switched_ms: Some(5_000),
            },
            ActiveProfile {
                index: 1,
                switched_ms: None,
            },
        ] {
            assert!(!profile.is_first(), "{profile:?}");
            let wire = serde_json::to_value(profile).expect("it writes");
            assert_eq!(
                serde_json::from_value::<ActiveProfile>(wire).ok(),
                Some(profile)
            );
        }
        assert_eq!(preset_code("rEaRgUaRd"), Some(5));
        assert_eq!(preset_code("Brawler"), None);
    }

    /// **17b cut M5.6's review** (R17, R19): `help minds`' preset rows name
    /// every term each preset flies otherwise than Line, at the card's own
    /// reading in its own words — on the shipped doctrine, and on one whose
    /// every mapped term differs from every preset's, so a term a preset writes
    /// at Line's shipped value (Hunter's `search_s 60`) is named too: an
    /// operator's doctrine moves Line's and never the preset's. The eleven
    /// carried inert (this module's head) are not the rows' to name. And the
    /// figures the page quotes for Line as shipped are the default doctrine's
    /// and Line's own.
    #[test]
    fn m5_help_minds_names_every_term_a_preset_flies() {
        const INERT: [Slider; 11] = [
            Slider::Stance,
            Slider::CloseM,
            Slider::OrbitSense,
            Slider::TurnIntoHits,
            Slider::TurnS,
            Slider::ScreenBearing,
            Slider::ScreenArc,
            Slider::Opening,
            Slider::ReserveOdds,
            Slider::SwitchM,
            Slider::StopUnseen,
        ];
        let help = include_str!("../../../../docs/help/minds.md");
        let lines: Vec<&str> = help.lines().collect();
        // A row: its first line opens `  <Preset> `, and the lines under it
        // indented past the name column carry on.
        let row = |preset: &str| -> String {
            let at = lines
                .iter()
                .position(|line| line.starts_with(&format!("  {preset} ")))
                .unwrap_or_else(|| panic!("help minds has no row for {preset}"));
            let mut text = lines.get(at).copied().unwrap_or_default().to_owned();
            for line in lines.iter().skip(at + 1) {
                if !line.starts_with("                ") {
                    break;
                }
                text.push(' ');
                text.push_str(line.trim());
            }
            text
        };
        // `said` as words of the row: never `min_odds 5` inside `min_odds 55`.
        let names = |text: &str, said: &str| {
            text.match_indices(said).any(|(at, _)| {
                text.get(at + said.len()..)
                    .and_then(|rest| rest.chars().next())
                    .is_none_or(|next| !next.is_ascii_alphanumeric())
            })
        };
        let shipped = ReflexConfig::default();
        let moved = ReflexConfig {
            engage_band_permille: 750,
            withdraw_permille: 400,
            stand_min_permille: 200,
            rally_ms: 25_000,
            search_ms: 50_000,
            hunt_after_ms: 100_000,
            ..ReflexConfig::default()
        };
        for cfg in [&shipped, &moved] {
            let base = line(cfg);
            for name in PRESETS.iter().skip(1) {
                let card = preset(name, cfg).unwrap_or_else(|| panic!("{name}"));
                let text = row(name);
                for slider in SLIDERS {
                    if INERT.contains(&slider) || card.reading(slider) == base.reading(slider) {
                        continue;
                    }
                    let said = format!("{} {}", slider.name(), card.reading(slider));
                    assert!(
                        names(&text, &said),
                        "{name}'s row does not name {said}: {text}"
                    );
                }
            }
        }
        // Line, as the page quotes it shipped: closes to 70% of its reach,
        // fires at 10% or better on any face, keeps no rounds back, breaks off
        // under 35%, back after 20 quiet seconds, a minute's search, a minute
        // and a half before it goes looking.
        let line_card = line(&shipped);
        for (slider, reading) in [
            (Slider::BandPct, "70"),
            (Slider::MinOdds, "10"),
            (Slider::Aspect, "any"),
            (Slider::ReservePct, "0"),
            (Slider::BreakOffPct, "35"),
            (Slider::RallyS, "20"),
            (Slider::SearchS, "60"),
            (Slider::HuntAfterS, "90"),
        ] {
            assert_eq!(line_card.reading(slider), reading, "{}", slider.name());
        }
        assert!(help.contains("Line is the Registry's doctrine as this server runs it"));
    }
}
