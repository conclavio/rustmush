//! **The card as a mind reads it** (17b cut M2.3; doc 17 §7 "Parameters", §11
//! "The card"; D-UM-27, D-UM-28, D-UM-43; 17b §1's departure (3)).
//!
//! `Card` is the hull's stock card (`crate::arena::card::StockCard`) handed to
//! every think as its second argument — read-only, never folded into the AST
//! (D-UM-27: one AST per `(source digest, MIND_ABI)`, so three hundred hulls
//! on three hundred differently tuned cards compile once). One getter per
//! **stored** field, in **stored integer units**, named after doc 17 §11's
//! slider where the units agree and after the stored figure where they do not
//! (`band_permille` for `band_pct`, `withdraw_permille` for `break_off_pct`,
//! `stand_min_permille` for `fight_home_pct`, `rally_ms` for `rally_s`, …): a
//! half-percent slider cannot cross as an INT percent, and D-UM-28 forbids a
//! FLOAT. No derived figure — `fight_home_pct`'s halves, a preset's name,
//! the overlay `StockCard::as_reflex` builds — reaches the script; a mind
//! derives what it needs from the stored figures, as T1 does. The enums cross
//! as their snake-case words, the words the card's serde form and the
//! `@mind/set` read-back use ([`CARD_FIELDS`]'s test pins them against
//! serde's).

use std::sync::Arc;

use mush_mind::rhai::Dynamic;

use super::Registrar;
use super::api::{int, word};
use crate::arena::card::{AspectWord, Opening, OrbitSense, Prefer, Stance, StockCard};

/// **The card** (`Card`): the think's own copy of the hull's card.
#[derive(Clone, Debug)]
pub struct Card(pub Arc<StockCard>);

/// **The 24 getters, in `StockCard`'s field order** — pinned against the
/// struct by `the_card_type_reads_the_24_stored_fields`.
pub const CARD_FIELDS: [&str; 24] = [
    "stance",
    "band_permille",
    "close_m",
    "orbit_sense",
    "turn_into_hits",
    "turn_s",
    "screen_bearing",
    "screen_arc",
    "opening",
    "min_odds_permille",
    "aspect",
    "reserve_pct",
    "reserve_odds_pct",
    "prefer",
    "weight_range",
    "weight_facet",
    "weight_health",
    "switch_m",
    "withdraw_permille",
    "stand_min_permille",
    "rally_ms",
    "search_ms",
    "hunt_after_ms",
    "stop_unseen",
];

/// A stance's word.
const fn stance_word(stance: Stance) -> &'static str {
    match stance {
        Stance::Hold => "hold",
        Stance::Close => "close",
        Stance::Orbit => "orbit",
        Stance::Hunt => "hunt",
        Stance::Screen => "screen",
    }
}

/// An orbit sense's word.
const fn orbit_word(sense: OrbitSense) -> &'static str {
    match sense {
        OrbitSense::Cw => "cw",
        OrbitSense::Ccw => "ccw",
        OrbitSense::Auto => "auto",
    }
}

/// An opening's word.
const fn opening_word(opening: Opening) -> &'static str {
    match opening {
        Opening::Wait => "wait",
        Opening::Search => "search",
        Opening::Advance => "advance",
    }
}

/// An aspect's word: `any`, `flank` or `rear`.
const fn aspect_word(aspect: AspectWord) -> &'static str {
    match aspect {
        AspectWord::Any => "any",
        AspectWord::Flank => "flank",
        AspectWord::Rear => "rear",
    }
}

/// A preference's word.
const fn prefer_word(prefer: Prefer) -> &'static str {
    match prefer {
        Prefer::Nearest => "nearest",
        Prefer::Exposed => "exposed",
        Prefer::Hurt => "hurt",
        Prefer::Balanced => "balanced",
        Prefer::BestOdds => "best_odds",
    }
}

/// **The card's type and its 24 getters.**
pub(crate) fn register(api: &mut Registrar<'_>) {
    custom_type!(api, Card, "Card");
    getter!(api, "Card", "stance", |c: &mut Card| {
        word(stance_word(c.0.stance))
    });
    getter!(api, "Card", "band_permille", |c: &mut Card| {
        int(c.0.engage_band_permille)
    });
    getter!(api, "Card", "close_m", |c: &mut Card| { int(c.0.close_m) });
    getter!(api, "Card", "orbit_sense", |c: &mut Card| {
        word(orbit_word(c.0.orbit_sense))
    });
    getter!(api, "Card", "turn_into_hits", |c: &mut Card| {
        Dynamic::from_bool(c.0.turn_into_hits)
    });
    getter!(api, "Card", "turn_s", |c: &mut Card| { int(c.0.turn_s) });
    getter!(api, "Card", "screen_bearing", |c: &mut Card| {
        int(c.0.screen_bearing)
    });
    getter!(api, "Card", "screen_arc", |c: &mut Card| {
        int(c.0.screen_arc)
    });
    getter!(api, "Card", "opening", |c: &mut Card| {
        word(opening_word(c.0.opening))
    });
    getter!(api, "Card", "min_odds_permille", |c: &mut Card| {
        int(c.0.min_odds_permille)
    });
    getter!(api, "Card", "aspect", |c: &mut Card| {
        word(aspect_word(c.0.aspect))
    });
    getter!(api, "Card", "reserve_pct", |c: &mut Card| {
        int(c.0.reserve_pct)
    });
    getter!(api, "Card", "reserve_odds_pct", |c: &mut Card| {
        int(c.0.reserve_odds_pct)
    });
    getter!(api, "Card", "prefer", |c: &mut Card| {
        word(prefer_word(c.0.prefer))
    });
    getter!(api, "Card", "weight_range", |c: &mut Card| {
        int(c.0.weight_range)
    });
    getter!(api, "Card", "weight_facet", |c: &mut Card| {
        int(c.0.weight_facet)
    });
    getter!(api, "Card", "weight_health", |c: &mut Card| {
        int(c.0.weight_health)
    });
    getter!(api, "Card", "switch_m", |c: &mut Card| {
        int(c.0.switch_m)
    });
    getter!(api, "Card", "withdraw_permille", |c: &mut Card| {
        int(c.0.withdraw_permille)
    });
    getter!(api, "Card", "stand_min_permille", |c: &mut Card| {
        int(c.0.stand_min_permille)
    });
    getter!(api, "Card", "rally_ms", |c: &mut Card| {
        int(c.0.rally_ms)
    });
    getter!(api, "Card", "search_ms", |c: &mut Card| {
        int(c.0.search_ms)
    });
    getter!(api, "Card", "hunt_after_ms", |c: &mut Card| {
        int(c.0.hunt_after_ms)
    });
    getter!(api, "Card", "stop_unseen", |c: &mut Card| {
        Dynamic::from_bool(c.0.stop_unseen)
    });
}

#[cfg(test)]
mod tests {
    use mush_mind::memory::{Value, decode};

    use super::super::fixture::{context_on, think_with, view};
    use super::super::registered_names;
    use super::*;
    use crate::arena::card::{PRESETS, line, preset};
    use crate::config::ReflexConfig;

    /// A word as serde writes it — the card's own serde form.
    fn serde_word<T: serde::Serialize>(value: &T) -> String {
        serde_json::to_value(value)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default()
    }

    /// What one getter must read off `card`, in the memory's shapes (a word
    /// as its string).
    fn expected(card: &StockCard) -> Vec<(&'static str, Value)> {
        let n = |value: u32| Value::Int(i64::from(value));
        vec![
            ("stance", Value::Str(serde_word(&card.stance))),
            ("band_permille", n(card.engage_band_permille)),
            ("close_m", n(card.close_m)),
            ("orbit_sense", Value::Str(serde_word(&card.orbit_sense))),
            ("turn_into_hits", Value::Bool(card.turn_into_hits)),
            ("turn_s", n(card.turn_s)),
            ("screen_bearing", n(card.screen_bearing)),
            ("screen_arc", n(card.screen_arc)),
            ("opening", Value::Str(serde_word(&card.opening))),
            ("min_odds_permille", n(u32::from(card.min_odds_permille))),
            ("aspect", Value::Str(serde_word(&card.aspect))),
            ("reserve_pct", n(card.reserve_pct)),
            ("reserve_odds_pct", n(card.reserve_odds_pct)),
            ("prefer", Value::Str(serde_word(&card.prefer))),
            ("weight_range", n(card.weight_range)),
            ("weight_facet", n(card.weight_facet)),
            ("weight_health", n(card.weight_health)),
            ("switch_m", n(card.switch_m)),
            ("withdraw_permille", n(card.withdraw_permille)),
            ("stand_min_permille", n(card.stand_min_permille)),
            ("rally_ms", n(card.rally_ms)),
            ("search_ms", n(card.search_ms)),
            ("hunt_after_ms", n(card.hunt_after_ms)),
            ("stop_unseen", Value::Bool(card.stop_unseen)),
        ]
    }

    /// 17b §2's M2.3 test 7: **the card type reads the 24 stored fields** on
    /// Line and on every preset — a script copies every getter into its
    /// memory and the memory equals the struct field for field, the enums as
    /// serde's words.
    #[test]
    fn the_card_type_reads_the_24_stored_fields() {
        let cfg = ReflexConfig::default();
        let copy: String = CARD_FIELDS
            .iter()
            .map(|field| format!("this.{field} = c.{field};"))
            .collect();
        let source = format!("fn think(v, c) {{ {copy} }}");
        let mut cards = vec![("Line", line(&cfg))];
        for name in PRESETS {
            cards.push((name, preset(name, &cfg).expect("a preset")));
        }
        for (name, card) in cards {
            let ran = think_with(&source, context_on(view()), card.clone(), &[]);
            let outcome = ran
                .result
                .unwrap_or_else(|fault| panic!("{name}: {fault:?}"));
            let memory = decode(&outcome.memory).expect("canonical memory");
            assert_eq!(memory.len(), 24, "{name}");
            for (field, value) in expected(&card) {
                assert_eq!(memory.get(field), Some(&value), "{name}: {field}");
            }
        }
        // The getters are exactly the 24, in the struct's order.
        let getters: Vec<String> = registered_names()
            .into_iter()
            .filter_map(|name| name.strip_prefix("Card.").map(str::to_owned))
            .collect();
        assert_eq!(getters, CARD_FIELDS.to_vec());
        let stored = serde_json::to_value(line(&cfg)).expect("a card serialises");
        assert_eq!(stored.as_object().map(serde_json::Map::len), Some(24));
    }

    /// Every enum word is serde's, for every value.
    #[test]
    fn the_card_words_are_serdes() {
        for stance in [
            Stance::Hold,
            Stance::Close,
            Stance::Orbit,
            Stance::Hunt,
            Stance::Screen,
        ] {
            assert_eq!(stance_word(stance), serde_word(&stance));
        }
        for sense in [OrbitSense::Cw, OrbitSense::Ccw, OrbitSense::Auto] {
            assert_eq!(orbit_word(sense), serde_word(&sense));
        }
        for opening in [Opening::Wait, Opening::Search, Opening::Advance] {
            assert_eq!(opening_word(opening), serde_word(&opening));
        }
        for aspect in [AspectWord::Any, AspectWord::Flank, AspectWord::Rear] {
            assert_eq!(aspect_word(aspect), serde_word(&aspect));
        }
        for prefer in [
            Prefer::Nearest,
            Prefer::Exposed,
            Prefer::Hurt,
            Prefer::Balanced,
            Prefer::BestOdds,
        ] {
            assert_eq!(prefer_word(prefer), serde_word(&prefer));
        }
    }
}
