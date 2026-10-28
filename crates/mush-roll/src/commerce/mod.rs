//! Pure commercial identities and independent temporal/capacity facts (M7.1a).
//!
//! None of these values proves allocation, ownership, certification, a qualifying
//! parent grant or permission to act. They contain no holder, source bytes or
//! mutable owner. Later typed plans must resolve their exact committed records.
//! In particular, temporal classification is not an admission decision.
//! Input owners must still bound the complete encoded document before decoding;
//! scalar visitor limits do not bound a JSON parser's work on an entire document.

use std::num::{NonZeroU16, NonZeroU32, NonZeroU64};

use mush_core::{ClockRef, ClockStamp, ObjId, mechanics::DefinitionDigest};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{Revision, RevisionRef};

/// Doc 17h §3 M8c (B1): the typed holder of a mind licence, a character or an
/// organisation through its acting character. Defined in the economy (the
/// lowest crate naming an organisation principal) and re-exported here.
pub use mush_economy::Holder;

pub(crate) mod follow;
/// Version of these source-free value envelopes, independent of M6 workflow JSON.
pub(crate) mod line_update;
pub const VALUE_FORMAT: u16 = 1;
/// Explicit representational ceiling for a purchased seat count. New-work and
/// open-claim storage reservations remain separate, lower operational limits.
/// This ceiling never applies to an owner's dispatch authority.
pub const MAX_PAID_SEATS: u16 = 1_024;
/// Compact audit term bound, in world-IC seconds (90 days): every paid term, its
/// renewals, and the personal and dependent owner terms.
pub const MAX_TERM_SECONDS: i64 = 90 * 24 * 60 * 60;
/// The owner-term bound, in world-IC seconds (365 days). 17g C0: a House-filed
/// mind's free owner licence runs 365 IC days until House owner attestation
/// exists. Only owner filing terms, the attested owner terms (M8c: the House
/// attestation's 365 days) and [`TermWindow`] itself use it; a paid
/// record still holds its window to [`MAX_TERM_SECONDS`].
pub const MAX_OWNER_TERM_SECONDS: i64 = 365 * 24 * 60 * 60;
/// Compact Grace bound, in world-IC seconds (3 days).
pub const MAX_GRACE_SECONDS: i64 = 3 * 24 * 60 * 60;

/// Closed, source-free validation failures; no error identifies another holder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ValueError {
    #[error("expected a nonzero canonical commerce identifier")]
    Identifier,
    #[error("unsupported commerce value version")]
    Version,
    #[error("invalid immutable revision identity")]
    Revision,
    #[error("expected a versioned world-IC clock stamp")]
    Clock,
    #[error("paid seat count is outside the supported bound")]
    Seats,
    #[error("invalid or excessive world-IC term interval")]
    Term,
}

macro_rules! identity {
    ($name:ident, $raw:ty, $nonzero:ty, $digits:literal, $prefix:literal, $doc:literal) => {
        #[doc = $doc]
        /// This is a reference value, not an allocation or authority token.
        /// Decimal-string JSON preserves every bit across ordinary clients.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name($nonzero);

        impl $name {
            pub fn new(value: $raw) -> Result<Self, ValueError> {
                <$nonzero>::new(value)
                    .map(Self)
                    .ok_or(ValueError::Identifier)
            }

            #[must_use]
            pub const fn get(self) -> $raw {
                self.0.get()
            }

            /// Stable, domain-prefixed key. The owning typed plan must still
            /// prove existence and cannot use this value to reuse an allocation.
            #[must_use]
            pub fn key(self) -> String {
                format!(concat!($prefix, "/{:016x}"), self.get())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.get().to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct IdVisitor;
                impl serde::de::Visitor<'_> for IdVisitor {
                    type Value = $name;

                    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(
                            f,
                            "a nonzero canonical decimal identifier of at most {} bytes",
                            $digits
                        )
                    }

                    fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Self::Value, E> {
                        if text.is_empty()
                            || text.len() > $digits
                            || text.starts_with('0')
                            || !text.bytes().all(|b| b.is_ascii_digit())
                        {
                            return Err(E::custom(ValueError::Identifier));
                        }
                        let raw = text
                            .parse::<$raw>()
                            .map_err(|_| E::custom(ValueError::Identifier))?;
                        $name::new(raw).map_err(E::custom)
                    }
                }
                deserializer.deserialize_str(IdVisitor)
            }
        }
    };
}

identity!(
    CharterId,
    u32,
    NonZeroU32,
    10,
    "charter",
    "The reserved charter world object's identity; there is no second charter allocator."
);
identity!(
    LineId,
    u64,
    NonZeroU64,
    20,
    "line",
    "A compatible charter line's stable identifier."
);
identity!(
    FiledRevisionId,
    u64,
    NonZeroU64,
    20,
    "filed",
    "A filed revision record's stable identifier."
);
identity!(
    LicenceId,
    u64,
    NonZeroU64,
    20,
    "licence",
    "A licence instrument's stable identifier."
);
identity!(
    TermGeneration,
    u64,
    NonZeroU64,
    20,
    "term",
    "A licence-local nonzero term generation."
);

impl CharterId {
    /// Reference an already-reserved world object. This does not prove its
    /// reservation, object shape, title or current holder.
    pub fn from_object(object: ObjId) -> Result<Self, ValueError> {
        Self::new(object.0)
    }

    #[must_use]
    pub const fn object(self) -> ObjId {
        ObjId(self.get())
    }
}

/// Exact immutable revision and source identities, without executable bytes.
/// Decoding establishes structure only; it does not verify retained source or
/// grant access. The existing revision/source owners remain authoritative.
/// Source-free does not mean public: future endpoint disclosure still needs
/// its current actor/record ACL, including for the embedded character identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RevisionIdentityWire")]
pub struct RevisionIdentity {
    format: u16,
    revision: RevisionRef,
    source: DefinitionDigest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevisionIdentityWire {
    format: u16,
    revision: RevisionRef,
    source: DefinitionDigest,
}

impl RevisionIdentity {
    /// Capture already-immutable owner data after structural validation. Source
    /// verification and current actor access are deliberately separate duties.
    pub fn capture(revision: &Revision) -> Result<Self, ValueError> {
        revision.validate().map_err(|_| ValueError::Revision)?;
        Self::try_from(RevisionIdentityWire {
            format: VALUE_FORMAT,
            revision: revision.reference(),
            source: revision.source(),
        })
    }

    #[must_use]
    pub const fn revision(self) -> RevisionRef {
        self.revision
    }

    #[must_use]
    pub const fn source(self) -> DefinitionDigest {
        self.source
    }

    /// Structural equality with this exact retained record. A matching result
    /// is neither an ACL decision nor proof that source storage is readable.
    pub fn matches(self, revision: &Revision) -> Result<bool, ValueError> {
        Ok(self == Self::capture(revision)?)
    }
}

impl TryFrom<RevisionIdentityWire> for RevisionIdentity {
    type Error = ValueError;

    fn try_from(wire: RevisionIdentityWire) -> Result<Self, Self::Error> {
        if wire.format != VALUE_FORMAT {
            return Err(ValueError::Version);
        }
        if wire.revision.key.mind == 0 || wire.revision.key.number == 0 {
            return Err(ValueError::Revision);
        }
        Ok(Self {
            format: wire.format,
            revision: wire.revision,
            source: wire.source,
        })
    }
}

/// A purchased seat count. It is not an open-claim counter or a reservation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct PaidSeats(NonZeroU16);

impl PaidSeats {
    pub fn new(value: u16) -> Result<Self, ValueError> {
        if value > MAX_PAID_SEATS {
            return Err(ValueError::Seats);
        }
        NonZeroU16::new(value).map(Self).ok_or(ValueError::Seats)
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0.get()
    }
}

impl TryFrom<u16> for PaidSeats {
    type Error = ValueError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<PaidSeats> for u16 {
    fn from(value: PaidSeats) -> Self {
        value.get()
    }
}

/// Distinct licence capacity facts. Owned dispatch has no purchased count;
/// actual hull title/dispatch authority and bounded open-claim capacity must
/// both be checked by the later owner-specific admission service.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Capacity {
    PaidSeats { seats: PaidSeats },
    // Keep an empty struct variant: serde's internally-tagged unit visitor
    // ignores extra fields even when the enum denies unknown fields.
    OwnedDispatch {},
}

/// An absolute world-IC second, not a UTC second or a term generation.
/// Timeline generations belong to operation guards, not to the date value:
/// a continuous world rate change must not restart an existing term.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorldIcSeconds(i64);

impl WorldIcSeconds {
    /// Convert a server clock stamp. This checks its clock kind, not whether
    /// that generation is still current; a later action guard must prove that.
    pub fn from_stamp(stamp: &ClockStamp) -> Result<Self, ValueError> {
        match &stamp.clock {
            ClockRef::WorldIc { generation } if *generation != 0 => Ok(Self(stamp.at)),
            _ => Err(ValueError::Clock),
        }
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// Temporal facts only. Recall, current title, dependency and pending transfer
/// remain independent; even `Active` is not execution/source/sale permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermPhase {
    NotEffective,
    Active,
    Grace,
    Lapsed,
}

/// A bounded eligibility interval and its actual issue instant. The independent
/// eligibility origin prevents a late Grace issue from disguising an excessive
/// original term. A parent-bound constructor must later prove where inherited
/// dates came from; this value alone proves no parent grant.
///
/// One type carries owner and paid windows alike, so its own bound is the longer
/// owner bound ([`MAX_OWNER_TERM_SECONDS`]); a paid record checks its window
/// against [`MAX_TERM_SECONDS`] through [`TermWindow::term_seconds`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "TermWindowWire")]
pub struct TermWindow {
    format: u16,
    eligibility_start: WorldIcSeconds,
    issued_at: WorldIcSeconds,
    term_end: WorldIcSeconds,
    grace_end: WorldIcSeconds,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TermWindowWire {
    format: u16,
    eligibility_start: WorldIcSeconds,
    issued_at: WorldIcSeconds,
    term_end: WorldIcSeconds,
    grace_end: WorldIcSeconds,
}

impl TermWindow {
    pub fn new(
        eligibility_start: WorldIcSeconds,
        issued_at: WorldIcSeconds,
        term_end: WorldIcSeconds,
        grace_end: WorldIcSeconds,
    ) -> Result<Self, ValueError> {
        let term = term_end
            .0
            .checked_sub(eligibility_start.0)
            .ok_or(ValueError::Term)?;
        let grace = grace_end
            .0
            .checked_sub(term_end.0)
            .ok_or(ValueError::Term)?;
        if !(1..=MAX_OWNER_TERM_SECONDS).contains(&term)
            || !(0..=MAX_GRACE_SECONDS).contains(&grace)
            || issued_at >= grace_end
        {
            return Err(ValueError::Term);
        }
        Ok(Self {
            format: VALUE_FORMAT,
            eligibility_start,
            issued_at,
            term_end,
            grace_end,
        })
    }

    /// The term's length, eligibility start to term end; positive and within
    /// [`MAX_OWNER_TERM_SECONDS`] by construction.
    #[must_use]
    pub const fn term_seconds(self) -> i64 {
        self.term_end.0 - self.eligibility_start.0
    }

    #[must_use]
    pub const fn eligibility_start(self) -> WorldIcSeconds {
        self.eligibility_start
    }
    #[must_use]
    pub const fn issued_at(self) -> WorldIcSeconds {
        self.issued_at
    }
    #[must_use]
    pub const fn term_end(self) -> WorldIcSeconds {
        self.term_end
    }
    #[must_use]
    pub const fn grace_end(self) -> WorldIcSeconds {
        self.grace_end
    }

    #[must_use]
    pub fn phase_at(self, now: WorldIcSeconds) -> TermPhase {
        if now < self.issued_at || now < self.eligibility_start {
            TermPhase::NotEffective
        } else if now < self.term_end {
            TermPhase::Active
        } else if now < self.grace_end {
            TermPhase::Grace
        } else {
            TermPhase::Lapsed
        }
    }
}

impl TryFrom<TermWindowWire> for TermWindow {
    type Error = ValueError;

    fn try_from(wire: TermWindowWire) -> Result<Self, Self::Error> {
        if wire.format != VALUE_FORMAT {
            return Err(ValueError::Version);
        }
        Self::new(
            wire.eligibility_start,
            wire.issued_at,
            wire.term_end,
            wire.grace_end,
        )
    }
}

pub(crate) mod audit;
pub mod authored_access;
pub(crate) mod authored_fork;
pub mod authored_history;
pub(crate) mod authored_retirement;
pub(crate) mod charter_pending;
mod service_guards;
pub use charter_pending::PendingCharterTransferConfig;
pub mod charter_transfer;
pub mod command_actions;
pub mod command_reader;
pub mod facts;
pub(crate) mod owner_attestation;
pub(crate) mod owner_qualification;
pub mod paid_resale;
pub mod registry_recall;
pub mod registry_revocation;
pub(crate) mod withdrawal;
pub use authored_retirement::entry::AuthoredRetirementSelection;
pub(crate) mod bootstrap;
pub mod coverage;
mod dependent_filing;
mod dependent_owner;
pub mod dependent_source;
pub mod entitlement;
pub mod families;
pub mod filed;
pub mod filing;
pub mod fork_protocol;
pub mod holding_reader;
pub mod house_catalogue;
pub mod ic_claims;
pub mod marked_rating;
pub mod module;
pub mod org_holder;
pub mod owner_holdings;
pub mod paid;
mod paid_audit;
pub mod paid_holdings;
pub(crate) mod paid_roots;
pub mod paid_source_reader;
pub(crate) mod parent_grant;
pub(crate) mod parent_selector;
pub(crate) mod participant;
pub mod personal_filed_reader;
pub mod personal_paid_holdings;
pub mod proceeds;
pub mod purchase;
pub mod purchase_writer;
pub mod rating;
pub(crate) mod rating_roots;
pub(crate) mod read;
pub mod records;
pub mod renewal;
pub mod rights;
pub(crate) mod roots;
pub mod sale_reader;
pub mod seal_marks;
pub mod usage;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod publication_tests;

#[cfg(test)]
mod bootstrap_tests;

#[cfg(test)]
mod filing_tests;

pub mod house_purchase;

pub mod house_renewal;

pub mod conduct;
