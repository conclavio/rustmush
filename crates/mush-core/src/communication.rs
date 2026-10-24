//! Small owner-neutral contracts for bounded IC communication handoff.
use serde::{Deserialize, Serialize};

/// An explicitly selected organization and the scope that authorized speaking.
/// The actor remains the source object's identity; this is never a payer alias.
/// IDs are zero-based. The owner adapter checks existence, scope and live authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayRepresentation {
    pub organization: u64,
    pub scope: u64,
}

/// Producers receive a typed refusal before creating durable source state.
/// Capacity refusals are retryable after pending traffic settles, not a promise
/// that the next clock tick will free a slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportRefusal {
    Unavailable,
    Authority,
    Principal,
    Producer,
    ControlReserve,
    Global,
}
impl TransportRefusal {
    pub fn message(self) -> &'static str {
        match self {
            Self::Unavailable => "IC transport is unavailable; retry when service returns.",
            Self::Authority => {
                "Communication representation is no longer authorized; select personal or obtain relay.speak authority."
            }
            Self::Principal => {
                "Your communication allowance is full; retry after pending traffic settles."
            }
            Self::Producer => {
                "This communication producer's allowance is full; retry after pending traffic settles."
            }
            Self::ControlReserve => {
                "Communication capacity is reserved for acknowledgements; retry after pending traffic settles."
            }
            Self::Global => "Transport pending queue is full; retry after pending traffic settles.",
        }
    }
    pub fn retryable(self) -> bool {
        self != Self::Authority
    }
}
impl std::fmt::Display for TransportRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}
impl std::error::Error for TransportRefusal {}
