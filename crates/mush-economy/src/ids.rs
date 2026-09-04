//! Economy identifiers (E2a).
//!
//! - [`AccountId`] and [`TransactionId`] are monotonic `u64` allocator ids, **never
//!   reused** (the `FactionId`/`OrgId` pattern; the allocator + `meta` resume live
//!   in the E2b module). `TransactionId` is additionally the journal's stable
//!   reference — a reversal points at the id it inverts.
//! - [`InstrumentId`] is a canonical, validated `SmolStr` (the `CapabilityId`
//!   pattern): a namespaced code like `sr.nuyen` or `tacsim.cbill`, registered by a
//!   trusted module. The engine only compares instrument ids; there is **no
//!   implicit exchange** between two of them.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

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
    /// Monotonic account id; never reused.
    AccountId
}
int_id! {
    /// Monotonic transaction id; never reused. The journal's stable reference — a
    /// reversal names the id it inverts, and the idempotency index maps to it.
    TransactionId
}
int_id! {
    /// Monotonic vault id; never reused (E3a). A vault is a custody container bound
    /// to a world object (an ordinary container or a `mush-sim` interior); the
    /// allocator + `meta` resume live in the E3b module. Assets themselves are keyed
    /// by `ObjId`, which is never recycled, so they need no id of their own.
    VaultId
}
int_id! {
    /// Monotonic approval-request id; never reused (E3a). A request binds a
    /// `plan_hash` to the approvals collected for it; at most ONE live (pending)
    /// request exists per hash.
    ApprovalRequestId
}
int_id! {
    /// Monotonic hold id; never reused (E4a). A hold is the economy's reservation
    /// record (doc 09 C8); its floor is re-derived **from the journal's hold
    /// effects**, never from surviving `hold:{id}` docs, so a garbage-collected
    /// terminal hold can never be re-minted under a stale `Contract.escrow` id.
    HoldId
}
int_id! {
    /// Monotonic obligation id; never reused (E4a). Obligation *state* is a
    /// doc-authority container (`obligation:{id}`); its money is the journal.
    ObligationId
}
int_id! {
    /// Monotonic contract id; never reused (E4a). Contract *state* is a
    /// doc-authority container (`contract:{id}`); its money is the journal.
    ContractId
}
int_id! {
    /// Monotonic cadence-clock id; never reused (E4a, D-E4-2). A named
    /// campaign/domain clock the economy owns and advances from its own source with
    /// bounded catch-up; the pre-K4 stand-in for doc 09's
    /// `ClockRef::Registered { owner, id, generation }`.
    CadenceId
}
int_id! {
    /// Monotonic escalation id; never reused (E4a, §4.6). The graded ladder's
    /// level-2 record — a subject suspended pending a human decision names the id
    /// that suspended it. The record itself is E4b1's; the id is minted here so
    /// [`crate::SuspendReason`] can carry it.
    EscalationId
}

/// A registered instrument code: a canonical, validated `SmolStr` — a namespaced
/// currency/commodity code (`sr.nuyen`, `tacsim.cbill`, `wod.resources`). Trim +
/// lowercase over `[a-z0-9_.-]`; the engine only ever compares these for equality.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "SmolStr", into = "SmolStr")]
pub struct InstrumentId(SmolStr);

impl InstrumentId {
    /// Validate + canonicalize (trim + lowercase); `None` if empty or containing a
    /// character outside `[a-z0-9_.-]`.
    #[must_use]
    pub fn new(s: impl AsRef<str>) -> Option<InstrumentId> {
        let t = s.as_ref().trim().to_ascii_lowercase();
        if t.is_empty()
            || !t.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.' || b == b'-'
            })
        {
            return None;
        }
        Some(InstrumentId(SmolStr::new(t)))
    }
    /// The canonical id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<SmolStr> for InstrumentId {
    type Error = String;
    fn try_from(s: SmolStr) -> Result<InstrumentId, String> {
        InstrumentId::new(&s).ok_or_else(|| format!("invalid InstrumentId {s:?}"))
    }
}
impl From<InstrumentId> for SmolStr {
    fn from(v: InstrumentId) -> SmolStr {
        v.0
    }
}
impl std::fmt::Display for InstrumentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::fmt::Debug for InstrumentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "InstrumentId({:?})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instrument_id_canonicalizes() {
        let a = InstrumentId::new("  SR.Nuyen ").unwrap();
        assert_eq!(a.as_str(), "sr.nuyen");
        assert_eq!(serde_json::to_string(&a).unwrap(), "\"sr.nuyen\"");
        assert!(InstrumentId::new("bad space").is_none());
        assert!(serde_json::from_str::<InstrumentId>("\"bad space\"").is_err());
    }

    #[test]
    fn int_ids_transparent_and_ordered() {
        assert!(AccountId(1) < AccountId(2));
        assert_eq!(serde_json::to_string(&TransactionId(7)).unwrap(), "7");
        assert_eq!(
            serde_json::from_str::<AccountId>("9").unwrap(),
            AccountId(9)
        );
    }
}
