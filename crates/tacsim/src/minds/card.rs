//! Read-only canonical card access, shared by every authored revision.
//! One generic getter and the stock compatibility getters; no per-source engine.

use super::Registrar;
use crate::arena::runtime_card::RuntimeCard;
use crate::arena::schema::CardValue;
use mush_mind::rhai::{Array, Dynamic, ImmutableString};

/// The exact hydrated card selected for this frozen think.
#[derive(Clone, Debug)]
pub struct Card(pub RuntimeCard);

impl Card {
    /// Generic closed-schema access. Unknown names read unit; no fallback value.
    pub fn value(&self, field: &str) -> Dynamic {
        if field.len() > 64 {
            return Dynamic::UNIT;
        }
        match self.0.get(field) {
            Some(CardValue::Integer(value)) => Dynamic::from_int(*value),
            Some(CardValue::Boolean(value)) => Dynamic::from_bool(*value),
            Some(CardValue::Word(value)) => Dynamic::from(ImmutableString::from(value.as_str())),
            Some(CardValue::Set(values)) => Dynamic::from_array(
                values
                    .iter()
                    .map(|word| Dynamic::from(ImmutableString::from(word.as_str())))
                    .collect::<Array>(),
            ),
            None => Dynamic::UNIT,
        }
    }
    /// Compatibility is explicit: only a complete stock-shaped card has these
    /// aliases. An unrelated authored schema must use its declared field names.
    fn stock_value(&self, field: &str) -> Dynamic {
        if self.0.stock().is_none() {
            return Dynamic::UNIT;
        }
        self.value(if field == "band_permille" {
            "engage_band_permille"
        } else {
            field
        })
    }
}

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

/// Register one immutable type regardless of source/schema/profile count.
pub(crate) fn register(api: &mut Registrar<'_>) {
    custom_type!(api, Card, "Card");
    getter!(api, "Card", "stance", |c: &mut Card| {
        c.stock_value("stance")
    });
    getter!(api, "Card", "band_permille", |c: &mut Card| {
        c.stock_value("band_permille")
    });
    getter!(api, "Card", "close_m", |c: &mut Card| {
        c.stock_value("close_m")
    });
    getter!(api, "Card", "orbit_sense", |c: &mut Card| {
        c.stock_value("orbit_sense")
    });
    getter!(api, "Card", "turn_into_hits", |c: &mut Card| {
        c.stock_value("turn_into_hits")
    });
    getter!(api, "Card", "turn_s", |c: &mut Card| {
        c.stock_value("turn_s")
    });
    getter!(api, "Card", "screen_bearing", |c: &mut Card| {
        c.stock_value("screen_bearing")
    });
    getter!(api, "Card", "screen_arc", |c: &mut Card| {
        c.stock_value("screen_arc")
    });
    getter!(api, "Card", "opening", |c: &mut Card| {
        c.stock_value("opening")
    });
    getter!(api, "Card", "min_odds_permille", |c: &mut Card| {
        c.stock_value("min_odds_permille")
    });
    getter!(api, "Card", "aspect", |c: &mut Card| {
        c.stock_value("aspect")
    });
    getter!(api, "Card", "reserve_pct", |c: &mut Card| {
        c.stock_value("reserve_pct")
    });
    getter!(api, "Card", "reserve_odds_pct", |c: &mut Card| {
        c.stock_value("reserve_odds_pct")
    });
    getter!(api, "Card", "prefer", |c: &mut Card| {
        c.stock_value("prefer")
    });
    getter!(api, "Card", "weight_range", |c: &mut Card| {
        c.stock_value("weight_range")
    });
    getter!(api, "Card", "weight_facet", |c: &mut Card| {
        c.stock_value("weight_facet")
    });
    getter!(api, "Card", "weight_health", |c: &mut Card| {
        c.stock_value("weight_health")
    });
    getter!(api, "Card", "switch_m", |c: &mut Card| {
        c.stock_value("switch_m")
    });
    getter!(api, "Card", "withdraw_permille", |c: &mut Card| {
        c.stock_value("withdraw_permille")
    });
    getter!(api, "Card", "stand_min_permille", |c: &mut Card| {
        c.stock_value("stand_min_permille")
    });
    getter!(api, "Card", "rally_ms", |c: &mut Card| {
        c.stock_value("rally_ms")
    });
    getter!(api, "Card", "search_ms", |c: &mut Card| {
        c.stock_value("search_ms")
    });
    getter!(api, "Card", "hunt_after_ms", |c: &mut Card| {
        c.stock_value("hunt_after_ms")
    });
    getter!(api, "Card", "stop_unseen", |c: &mut Card| {
        c.stock_value("stop_unseen")
    });
    method!(
        api,
        "Card",
        "get",
        |c: &mut Card, field: ImmutableString| {
            super::fees::charge(super::fees::FEE_READ);
            c.value(field.as_str())
        }
    );
}
#[cfg(test)]
mod tests {
    use mush_mind::memory::{Value, decode};

    use super::super::fixture::{context_on, think_with, view};
    use super::super::registered_names;
    use super::*;
    use crate::arena::card::StockCard;
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
        assert_eq!(
            getters,
            CARD_FIELDS
                .iter()
                .copied()
                .chain(["get"])
                .collect::<Vec<_>>()
        );
        let stored = serde_json::to_value(line(&cfg)).expect("a card serialises");
        assert_eq!(stored.as_object().map(serde_json::Map::len), Some(24));
    }
}
