//! **The stock card's shape** (17b cut M1b.1; doc 17 §11 "The card", D-UM-43,
//! D-UM-44).
//!
//! The card is the parameter block a stock mind reads: Standard Doctrine's
//! combat schema, in [`crate::config::ReflexConfig`]'s own units where a term
//! maps onto today's machine, so that the **Line** preset on its defaults
//! ([`line()`]) IS today's doctrine — the regression anchor. It lives on every
//! mind's row ([`crate::arena::mind::MindState::card`]), is handed to every
//! think ([`crate::arena::mind::ThinkJob::card`]), is folded into the state hash
//! inside item 3c and is carried by the snapshot from format 7 — the shape lands
//! here, in the one cut that moves the format, so the sub-cuts that read it move
//! nothing carried.
//!
//! This cut builds the **shape and the anchor only**: `set`, `CardError`,
//! `as_reflex`, `preset`, `name_of` and `read_back` are 17b cut M1b.5's. Since
//! 17b cut M1b.2 `NativeStockMind` reads the card's fire discipline alone —
//! `min_odds_permille`, `aspect` and `reserve_pct`, the terms its slots are
//! armed on (`crate::arena::stock_mind::arming_for`); `NoMind` ignores it. The
//! eleven terms nothing reads before 17b cut M2 are carried and marked `[M2]`
//! (17b's Q-M3).

use serde::{Deserialize, Serialize};

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
}
