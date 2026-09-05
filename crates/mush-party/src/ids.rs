//! Party identifiers (E6a, 08f §3.1).
//!
//! Two `u64` newtypes in the shipped `PropertyId`/`TenancyId` shape — monotonic,
//! never reused, `#[serde(transparent)]` so a document carries a bare number — and
//! they are deliberately *different kinds of thing*:
//!
//! - [`PartyId`] is **this crate's** id, minted by [`crate::PartyStore::alloc_party`].
//! - [`ContractRef`] is an **opaque handle the party crate never interprets**. It is
//!   the ruleset's `EconomicContract::id.0`, handed in at [`crate::lock`] and handed
//!   back at a render, and nothing in this crate ever resolves it, compares it
//!   against an economy, or asks whether it still exists. That is D-E6-5 as a type:
//!   the party is not a principal, and the contract never points back at the party,
//!   so the only thing a locked party knows about money is a number it cannot read.

use serde::{Deserialize, Serialize};

macro_rules! int_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub u64);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

int_id! {
    /// Monotonic party id; never reused.
    ///
    /// Renders **bare** (`3`), the shape the other E-series record ids render in: a
    /// party id is not a dbref, and printing it `#3` beside `#12` (an object) in the
    /// same line is exactly the confusion `+party`'s disambiguation rule exists to
    /// avoid. Input accepts both `3` and `#3` ([`parse_party_id`]).
    PartyId
}

int_id! {
    /// An opaque handle to the contract a party is locked against.
    ///
    /// The party crate **never** interprets it (D-E6-5): the party is not a principal
    /// and the contract never points back at the party. The ruleset supplies
    /// `EconomicContract::id.0` at [`crate::lock`] and reads it back off
    /// [`crate::PartyStatus::Locked`]; everything in between treats it as a number.
    ContractRef
}

/// Parse a party id written as `3` or `#3`.
///
/// Returns `None` for anything else — an empty string, a name, a negative number, a
/// value past `u64::MAX`. Callers render [`crate::NO_SUCH_PARTY`] for `None`, so a
/// malformed id and an unviewable party are indistinguishable.
#[must_use]
pub fn parse_party_id(raw: &str) -> Option<PartyId> {
    let s = raw.trim();
    let digits = s.strip_prefix('#').unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u64>().ok().map(PartyId)
}

#[cfg(test)]
mod tests {
    use super::{ContractRef, PartyId, parse_party_id};

    #[test]
    fn ids_are_transparent_ordered_and_bare() {
        assert!(PartyId(1) < PartyId(2));
        assert_eq!(serde_json::to_string(&PartyId(7)).expect("serializes"), "7");
        assert_eq!(
            serde_json::from_str::<ContractRef>("9").expect("parses"),
            ContractRef(9)
        );
        assert_eq!(PartyId(3).to_string(), "3");
    }

    #[test]
    fn party_ids_parse_bare_and_hashed_and_nothing_else() {
        assert_eq!(parse_party_id("3"), Some(PartyId(3)));
        assert_eq!(parse_party_id(" #12 "), Some(PartyId(12)));
        assert_eq!(parse_party_id("#"), None);
        assert_eq!(parse_party_id(""), None);
        assert_eq!(parse_party_id("-1"), None);
        assert_eq!(parse_party_id("3a"), None);
        assert_eq!(parse_party_id("Bob"), None);
        assert_eq!(parse_party_id("99999999999999999999999"), None);
    }
}
