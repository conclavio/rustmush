//! The market's own id types (E6c).
//!
//! Every one is a never-reused `u64` behind a newtype, minted by a saturating
//! allocator whose floor is derived from every persisted document's **parsed key**
//! (E6b verify fold R8's rule): a key that parses is a claim on an id whether or not
//! its body does, so a corrupt document plus a lost allocator can never re-mint an id
//! a live record already carries.
//!
//! [`MarketId`] is the exception and is deliberately a **name**, not a number: a
//! market is a venue ("seattle", "the corridor"), it is declared by a vendor rather
//! than allocated, and the safe public projection is keyed on `(catalog, market)`.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// The longest venue name a market may carry.
pub const MAX_MARKET_ID_BYTES: usize = 32;

macro_rules! numeric_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash,
        )]
        pub struct $name(pub u64);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

numeric_id!(
    /// A listing's id (`market:listing:{id}`).
    ListingId
);
numeric_id!(
    /// A quote's id (`market:quote:{id}`).
    QuoteId
);
numeric_id!(
    /// A registered vendor's id (`market:vendor:{id}`).
    VendorId
);
numeric_id!(
    /// A registered NPC market maker's id (`market:source:{id}`).
    MarketSourceId
);

/// A **venue** — the second half of the projection's `(catalog, market)` key.
///
/// A name rather than an allocated id on purpose: the projection publishes per
/// market, a vendor declares which one it trades in, and two servers' "seattle" are
/// the same market by construction. Lower-case ASCII, digits, `.`, `-` and `_`, at
/// most [`MAX_MARKET_ID_BYTES`] bytes, so it is a safe document-key component.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash)]
#[serde(try_from = "SmolStr", into = "SmolStr")]
pub struct MarketId(SmolStr);

impl MarketId {
    /// Parse and normalize a venue name; `None` if it is empty, over-long, or holds
    /// anything but `[a-z0-9._-]` once lower-cased.
    #[must_use]
    pub fn new(s: &str) -> Option<MarketId> {
        let s = s.trim().to_ascii_lowercase();
        if s.is_empty() || s.len() > MAX_MARKET_ID_BYTES {
            return None;
        }
        if !s.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_')
        }) {
            return None;
        }
        Some(MarketId(SmolStr::new(s)))
    }

    /// The venue name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// The lowest key a `(CatalogRef, MarketId)` range can start at.
    ///
    /// Deliberately **not** a venue: [`MarketId::new`] refuses an empty name, so this
    /// value can never name a real market and exists only so a walk over the pair
    /// index can be a `range` scoped to one catalogue rather than a walk of the whole
    /// map (independent verify fold, D8).
    #[must_use]
    pub(crate) fn floor() -> MarketId {
        MarketId(SmolStr::new_static(""))
    }
}

impl std::fmt::Display for MarketId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl TryFrom<SmolStr> for MarketId {
    type Error = &'static str;

    fn try_from(s: SmolStr) -> Result<MarketId, &'static str> {
        MarketId::new(s.as_str()).ok_or("a market id is 1..=32 bytes of [a-z0-9._-]")
    }
}

impl From<MarketId> for SmolStr {
    fn from(m: MarketId) -> SmolStr {
        m.0
    }
}

/// Parse a `#n` / `n` id out of a command argument.
#[must_use]
pub fn parse_u64(s: &str) -> Option<u64> {
    s.trim().trim_start_matches('#').parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_market_id_normalizes_and_refuses_junk() {
        assert_eq!(
            MarketId::new(" Seattle ").map(|m| m.to_string()).as_deref(),
            Some("seattle")
        );
        assert!(MarketId::new("").is_none());
        assert!(MarketId::new("has space").is_none());
        assert!(MarketId::new(&"x".repeat(MAX_MARKET_ID_BYTES + 1)).is_none());
    }

    #[test]
    fn a_market_id_round_trips_through_its_wire() {
        let m = MarketId::new("corridor").expect("valid");
        let v = serde_json::to_value(&m).expect("serializes");
        assert_eq!(serde_json::from_value::<MarketId>(v).ok(), Some(m));
        assert!(serde_json::from_value::<MarketId>(serde_json::json!("NOT OK")).is_err());
    }
}
