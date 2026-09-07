//! Identities (E7 cut 9). Two allocators, both floored from the store's own meta
//! document at boot, both transparent over `u64` so a document key and a rendered
//! id are the same string.

use std::fmt;

use serde::{Deserialize, Serialize};

/// One declared operation (10b §3.9, D-E7-18). The terms and the runtime are
/// separately versioned halves of **one** document under this id.
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct OperationId(pub u64);

/// One formation node in an order of battle (T8).
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct FormationId(pub u64);

macro_rules! id_display {
    ($t:ty, $prefix:literal) => {
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefix, self.0)
            }
        }

        impl $t {
            /// Parse a rendered id, with or without its sigil. `None` for anything
            /// that is not a bare non-zero decimal.
            #[must_use]
            pub fn parse(text: &str) -> Option<Self> {
                let raw = text.trim();
                let raw = raw.strip_prefix($prefix).unwrap_or(raw);
                let raw = raw.strip_prefix('#').unwrap_or(raw);
                let n: u64 = raw.parse().ok()?;
                (n != 0).then_some(Self(n))
            }
        }
    };
}

id_display!(OperationId, "op");
id_display!(FormationId, "fm");

/// The document key one operation's whole record lives at (rule 3: deterministic).
#[must_use]
pub fn operation_key(id: OperationId) -> smol_str::SmolStr {
    smol_str::SmolStr::new(format!("op:{}", id.0))
}

/// The document key one formation node lives at.
#[must_use]
pub fn formation_key(id: FormationId) -> smol_str::SmolStr {
    smol_str::SmolStr::new(format!("formation:{}", id.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e7c9_b2_ids_render_and_parse_round_trip_and_reject_zero() {
        assert_eq!(OperationId(7).to_string(), "op7");
        assert_eq!(FormationId(9).to_string(), "fm9");
        for text in ["op7", "7", "#7", " op7 "] {
            assert_eq!(OperationId::parse(text), Some(OperationId(7)), "{text}");
        }
        assert_eq!(OperationId::parse("0"), None);
        assert_eq!(OperationId::parse("op0"), None);
        assert_eq!(OperationId::parse("nope"), None);
        assert_eq!(operation_key(OperationId(7)), "op:7");
        assert_eq!(formation_key(FormationId(9)), "formation:9");
    }
}
