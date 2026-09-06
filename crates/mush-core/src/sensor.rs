//! E7 cut 7 — the trusted sensor-policy ingress vocabulary (10b §3.7, 08i
//! answer 1, D-E7-36).
//!
//! **`mush-tactics` is the sole [`ContactState`](crate::contact::ContactState)
//! authority.** Cut 7 adds no second contact family and no second lifecycle.
//! What it adds is the vocabulary in which a *propagation policy* states, in
//! bounded deterministic terms, which of the six already-migrated contact
//! transitions is due — and the seam through which `mush-tactics` reads that
//! statement without ever depending on the crate that made it.
//!
//! The trait lives **here**, in `mush-core`, for one load-bearing reason: the
//! kernel's owner fence. `scheduled_action_ingress`
//! (`crate::schedule_source`) refuses a draft whose schedule owner is not the
//! fired action's owner (`ScheduleActionIngressError::OwnerMismatch`), and
//! `ScheduleSourceRegistryBuilder::register` refuses a registration whose
//! schedule family or action belongs to another owner. The six `contact_*`
//! actions are `mush-tactics`', so a `mush-propagation`-owned schedule source
//! **cannot fire them** — the observation ingress must be registered by
//! `mush-tactics`, under `mush-tactics`' owner. Putting `SensorPolicy` in core
//! is what lets propagation supply the policy through
//! `TacticsModule::with_sensor_policy` without `mush-tactics` gaining a
//! `mush-propagation` edge (the `IndustryModule::with_recipes` /
//! `SimulationModule::with_accounts` shape).
//!
//! Nothing in this file writes anything. An [`ObservationIntent`] is a *claim
//! that a transition is due*; the transition itself is committed only by the
//! registered tactics provider, inside an `ActionTxn`, against the one contact
//! family — so a policy that lies produces a refusal, never a contact.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::action::{ContactId, IdempotencyKeyText, PrincipalId, SubjectKey};
use crate::clock::ClockStamp;
use crate::mechanics::CanonicalId;
use crate::obj::ObjId;
use crate::world::World;

/// One pass's intent bound (10b §7). Deliberately the same figure as K's
/// `MAX_WINDOW_CAPACITY`: one window's worth of work, and no pass may exceed it.
pub const MAX_OBSERVATIONS_PER_PASS: usize = 256;

/// The most contacts one observer may hold (10b §7). A stored set over it is
/// **refused on load, never truncated**.
pub const MAX_CONTACTS_PER_OBSERVER: usize = 128;

/// How many ticks past `last_seen` a contact survives before the policy is due
/// to call it lost (10b §7).
pub const CONTACT_TTL: u64 = 30;

/// The most principals one observation intent may name as its opening audience.
/// Below K's `MAX_CONTACT_AUDIENCE` because an *opening* audience is the
/// observer itself plus whoever the policy is deliberately co-crediting.
pub const MAX_INTENT_AUDIENCE: usize = 8;

/// Who holds a contact (Q16A, D-E7-15).
///
/// **A platform or a station, never a side and never the world.** There is no
/// omniscient fused record anywhere, so there is deliberately no `Side` arm and
/// no `World` arm to write one with.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObserverRef {
    /// A whole platform — a ship, a vehicle, a person carrying a sensor.
    Platform { id: ObjId },
    /// One crew station on a host platform (cut 5's `ActiveFocus`/billet
    /// surface: the *focused* station is the live one).
    Station { host: ObjId, station: CanonicalId },
}

impl ObserverRef {
    /// The platform this observer is or sits on — the object a spatial
    /// resolver reads.
    #[must_use]
    pub const fn platform(&self) -> ObjId {
        match self {
            Self::Platform { id } => *id,
            Self::Station { host, .. } => *host,
        }
    }

    /// The principal a contact audience names for this observer.
    #[must_use]
    pub fn principal(&self) -> PrincipalId {
        PrincipalId::Object {
            id: self.platform(),
        }
    }

    /// The bounded, deterministic key fragment this observer contributes.
    #[must_use]
    pub fn key_fragment(&self) -> String {
        match self {
            Self::Platform { id } => format!("p{}", id.0),
            Self::Station { host, station } => format!("s{}.{}", host.0, station.as_str()),
        }
    }
}

impl fmt::Display for ObserverRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.key_fragment())
    }
}

/// Which of the six migrated contact transitions an intent names.
///
/// One variant per registered `mush-tactics` action. There is no seventh:
/// adding one would be a second lifecycle, which 09 §7 forbids.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationKind {
    Observe,
    Refresh,
    Share,
    Unshare,
    Loss,
    Reacquire,
}

impl ObservationKind {
    /// The registered `mush-tactics` action id this kind drives.
    #[must_use]
    pub const fn action_id(self) -> &'static str {
        match self {
            Self::Observe => "contact_observe",
            Self::Refresh => "contact_refresh",
            Self::Share => "contact_share",
            Self::Unshare => "contact_unshare",
            Self::Loss => "contact_loss",
            Self::Reacquire => "contact_reacquire",
        }
    }

    /// The short, stable name used in keys, schedule ids and family ids.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Refresh => "refresh",
            Self::Share => "share",
            Self::Unshare => "unshare",
            Self::Loss => "loss",
            Self::Reacquire => "reacquire",
        }
    }

    /// Every kind, in canonical order. Exhaustive by construction: a new arm
    /// that is not added here fails the exhaustiveness pin.
    #[must_use]
    pub const fn all() -> [Self; 6] {
        [
            Self::Observe,
            Self::Refresh,
            Self::Share,
            Self::Unshare,
            Self::Loss,
            Self::Reacquire,
        ]
    }
}

/// What a policy says is due, in the parameter vocabulary the migrated
/// providers already decode.
///
/// Every arm past `Observe` carries an `expected_revision`, because every
/// `ContactState` transition is revision-checked: a policy working from a stale
/// read is **refused by the kernel**, not silently applied.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObservationOp {
    Observe {
        /// The truth this contact maps to. A real unit is
        /// [`SubjectKey::Object`]; a FALSE contact is propagation's own
        /// authoritative phantom, a [`SubjectKey::Scoped`] that resolves to no
        /// unit through any projection.
        truth: SubjectKey,
        audience: Vec<PrincipalId>,
    },
    Refresh {
        contact: ContactId,
        expected_revision: u64,
    },
    Share {
        contact: ContactId,
        expected_revision: u64,
        to: ObserverRef,
    },
    Unshare {
        contact: ContactId,
        expected_revision: u64,
        from: ObserverRef,
    },
    Loss {
        contact: ContactId,
        expected_revision: u64,
        reason: CanonicalId,
    },
    Reacquire {
        contact: ContactId,
        expected_revision: u64,
    },
}

impl ObservationOp {
    /// Which transition this is.
    #[must_use]
    pub const fn kind(&self) -> ObservationKind {
        match self {
            Self::Observe { .. } => ObservationKind::Observe,
            Self::Refresh { .. } => ObservationKind::Refresh,
            Self::Share { .. } => ObservationKind::Share,
            Self::Unshare { .. } => ObservationKind::Unshare,
            Self::Loss { .. } => ObservationKind::Loss,
            Self::Reacquire { .. } => ObservationKind::Reacquire,
        }
    }

    /// The contact this op acts on, or `None` for the one arm that creates it.
    #[must_use]
    pub const fn contact(&self) -> Option<ContactId> {
        match self {
            Self::Observe { .. } => None,
            Self::Refresh { contact, .. }
            | Self::Share { contact, .. }
            | Self::Unshare { contact, .. }
            | Self::Loss { contact, .. }
            | Self::Reacquire { contact, .. } => Some(*contact),
        }
    }
}

/// One bounded, deterministic observation intent.
///
/// Construction is the only way to make one and it re-runs every invariant, so
/// a hand-built or stored intent past a cap cannot exist (§4 rule 1).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationIntent {
    observer: ObserverRef,
    op: ObservationOp,
    at: ClockStamp,
    key: IdempotencyKeyText,
}

/// The strict wire form of [`ObservationIntent`] (§4 rule 1).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationIntentWire {
    pub observer: ObserverRef,
    pub op: ObservationOp,
    pub at: ClockStamp,
    pub key: IdempotencyKeyText,
}

impl<'de> Deserialize<'de> for ObservationIntent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ObservationIntentWire::deserialize(deserializer)?;
        Self::new(wire.observer, wire.op, wire.at, wire.key).map_err(serde::de::Error::custom)
    }
}

impl ObservationIntent {
    /// The one constructor. The wire re-runs it (§4 rule 1).
    ///
    /// # Errors
    /// [`ObservationIntentError`] when the opening audience is empty or past
    /// [`MAX_INTENT_AUDIENCE`], when it is not strictly sorted (a duplicate
    /// principal is a silently smaller audience, which K's own
    /// `ContactState::validate` refuses too), or when the key does not name
    /// this intent's own kind.
    pub fn new(
        observer: ObserverRef,
        op: ObservationOp,
        at: ClockStamp,
        key: IdempotencyKeyText,
    ) -> Result<Self, ObservationIntentError> {
        if let ObservationOp::Observe { audience, .. } = &op {
            if audience.is_empty() || audience.len() > MAX_INTENT_AUDIENCE {
                return Err(ObservationIntentError::Audience {
                    actual: audience.len(),
                    max: MAX_INTENT_AUDIENCE,
                });
            }
            if audience.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(ObservationIntentError::NonCanonicalAudience);
            }
        }
        // The key names its own kind, so a `loss` intent can never be armed on
        // the `share` source's schedule family and vice versa: the arming half
        // and the firing half agree by construction, not by convention.
        let prefix = op.kind().as_str();
        if !key
            .as_str()
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
        {
            return Err(ObservationIntentError::KeyKindMismatch {
                expected: prefix,
                actual: key.as_str().to_owned(),
            });
        }
        Ok(Self {
            observer,
            op,
            at,
            key,
        })
    }

    /// Build the canonical key `<kind>/<observer>/<subject>/<tick>`.
    ///
    /// # Errors
    /// [`ObservationIntentError::Key`] past [`crate::action::MAX_IDEMPOTENCY_KEY_BYTES`].
    pub fn key_for(
        kind: ObservationKind,
        observer: &ObserverRef,
        subject: &str,
        tick: i64,
    ) -> Result<IdempotencyKeyText, ObservationIntentError> {
        let text = format!(
            "{}/{}/{}/{tick:x}",
            kind.as_str(),
            observer.key_fragment(),
            subject
        );
        IdempotencyKeyText::parse_canonical(text)
            .map_err(|error| ObservationIntentError::Key(error.to_string()))
    }

    /// Build the canonical key for a transition on an **existing** contact:
    /// `<kind>/<observer>/c<contact>` — **with no tick**.
    ///
    /// E7 cut 7 independent verify fold, D3(a). The identity of a
    /// `Refresh`/`Loss`/`Reacquire`/`Share`/`Unshare` is the *work*, which is
    /// `(kind, observer, contact)`; the tick lives in the intent's own
    /// [`Self::at`]. With the tick in the key, the same still-due transition one
    /// IC second later hashed to a **different** `ScheduleId`, so the ingress
    /// took its insert branch and stacked a second durable cursor beside the
    /// stuck one instead of replacing it. `Observe` keeps the tick, because its
    /// identity is the pair plus the sighting book's own record of it.
    ///
    /// The full contact hex is used, not a prefix: a 32-bit prefix collision
    /// between two contacts one observer holds silently dropped a transition.
    ///
    /// # Errors
    /// [`ObservationIntentError::Key`] past [`crate::action::MAX_IDEMPOTENCY_KEY_BYTES`],
    /// or when `kind` is [`ObservationKind::Observe`], which has no contact yet.
    pub fn key_for_state(
        kind: ObservationKind,
        observer: &ObserverRef,
        contact: ContactId,
    ) -> Result<IdempotencyKeyText, ObservationIntentError> {
        if kind == ObservationKind::Observe {
            return Err(ObservationIntentError::Key(
                "an observe intent has no contact to key over".to_owned(),
            ));
        }
        let text = format!(
            "{}/{}/c{}",
            kind.as_str(),
            observer.key_fragment(),
            contact.to_hex()
        );
        IdempotencyKeyText::parse_canonical(text)
            .map_err(|error| ObservationIntentError::Key(error.to_string()))
    }

    /// Who observes.
    #[must_use]
    pub const fn observer(&self) -> &ObserverRef {
        &self.observer
    }

    /// What is due.
    #[must_use]
    pub const fn op(&self) -> &ObservationOp {
        &self.op
    }

    /// Which transition.
    #[must_use]
    pub const fn kind(&self) -> ObservationKind {
        self.op.kind()
    }

    /// The IC stamp the transition is committed at.
    #[must_use]
    pub const fn at(&self) -> &ClockStamp {
        &self.at
    }

    /// The deterministic key: two passes over one context produce the same key
    /// for the same intent, so a duplicate arm is a HIT.
    #[must_use]
    pub const fn key(&self) -> &IdempotencyKeyText {
        &self.key
    }

    /// The exact parameter document the registered `mush-tactics` provider for
    /// [`Self::kind`] decodes.
    ///
    /// These shapes are `ContactState`'s own transitions (`checked_refresh`,
    /// `checked_share`, …), which are core's — so naming them here is not
    /// `mush-core` reaching into `mush-tactics`; it is both crates reading the
    /// same kernel state machine.
    #[must_use]
    pub fn parameters(&self) -> Value {
        match &self.op {
            ObservationOp::Observe { truth, audience } => json!({
                "truth_subject": truth,
                "audience": audience,
                "observed_at": self.at,
            }),
            ObservationOp::Refresh {
                contact,
                expected_revision,
            }
            | ObservationOp::Reacquire {
                contact,
                expected_revision,
            } => json!({
                "contact": contact,
                "expected_revision": expected_revision,
                "observed_at": self.at,
            }),
            ObservationOp::Share {
                contact,
                expected_revision,
                to,
            } => json!({
                "contact": contact,
                "expected_revision": expected_revision,
                "principal": to.principal(),
            }),
            ObservationOp::Unshare {
                contact,
                expected_revision,
                from,
            } => json!({
                "contact": contact,
                "expected_revision": expected_revision,
                "principal": from.principal(),
            }),
            ObservationOp::Loss {
                contact,
                expected_revision,
                reason,
            } => json!({
                "contact": contact,
                "expected_revision": expected_revision,
                "lost_at": self.at,
                "reason": reason,
            }),
        }
    }
}

/// An intent that will not construct.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ObservationIntentError {
    #[error("an observation's opening audience size {actual} is outside 1..={max}")]
    Audience { actual: usize, max: usize },
    #[error("an observation's opening audience must be strictly sorted and duplicate-free")]
    NonCanonicalAudience,
    #[error("an observation key must open with `{expected}/`, not `{actual}`")]
    KeyKindMismatch {
        expected: &'static str,
        actual: String,
    },
    #[error("an observation key is unusable: {0}")]
    Key(String),
    #[error("a pass produced {actual} intents; the bound is {max}")]
    PassBound { actual: usize, max: usize },
}

/// The most rows the leak-safe contact-ledger projection carries.
///
/// `MAX_CONTACTS_PER_OBSERVER` (128) × a small roster; past it the projection
/// **pages rather than growing**, and says so.
pub const MAX_CONTACT_LEDGER_ROWS: usize = 1_024;

/// One row of the **leak-safe** contact-ledger projection.
///
/// ## What is deliberately not here
///
/// `ContactState::truth_subject` — the whole point of the family. A row names
/// the *holder*, the opaque `ContactId` it already holds a `PerceivedRef` for,
/// and the counters that holder can already see. It carries **no `ObjId` of an
/// observed unit**, so a recursive leak scan over every player- and AI-facing
/// serialization of this type finds no undetected unit (doc 09 Gate 3).
///
/// The one exception is [`Self::phantom`], and it is not an exception at all:
/// a `SubjectKey::Scoped { owner, class: "phantom", .. }` subject was **minted
/// by the propagation policy itself**, so handing its own id back tells it
/// nothing it did not author, and it is what lets an expiring phantom issue
/// `Loss` to every holder (hazard axis 3). A real unit's `SubjectKey::Object`
/// never produces a `Some` here.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContactLedgerRow {
    /// Which principal holds it.
    pub holder: PrincipalId,
    /// The opaque identity that principal already holds a reference to.
    pub contact: ContactId,
    pub generation: u64,
    pub revision: u64,
    pub active: bool,
    pub observed_at: ClockStamp,
    /// The propagation-owned phantom this contact maps to, or `None` for
    /// **every** real subject.
    pub phantom: Option<IdempotencyKeyText>,
}

/// The world-visible, leak-safe projection of who holds which contact.
///
/// Published by the **owner** of the contact family (`mush-tactics`) from a
/// bounded scan, so a policy in another crate can name a `ContactId` without
/// ever holding a family handle — the owner fence stays exactly where it is,
/// and nothing outside `mush-tactics` can write a contact.
///
/// It lives in `mush-core` so `mush-propagation` reads it over the `mush-core`
/// edge alone.
#[derive(Clone, Debug, Default)]
pub struct ContactLedgerState {
    rows: Vec<ContactLedgerRow>,
    at: Option<ClockStamp>,
    paging: bool,
}

impl ContactLedgerState {
    /// Replace the projection with a completed bounded scan.
    ///
    /// Rows are sorted canonically, so two rebuilds over one unchanged family
    /// produce byte-identical output and a policy reading it is deterministic.
    ///
    /// **The window is fair per holder** (E7 cut 7 independent verify fold, O6).
    /// A flat `truncate` to [`MAX_CONTACT_LEDGER_ROWS`] over rows sorted by
    /// holder let ONE holder's volume starve every holder sorting after it: a
    /// contact past the cut was invisible in its own holder's `+contacts`, never
    /// refreshed and never lost, because the pass and the render both read this
    /// projection. No holder may therefore occupy more than
    /// [`MAX_CONTACTS_PER_OBSERVER`] rows — which is all any holder can see
    /// anyway — and only then does the global cap apply. Anything dropped sets
    /// [`Self::is_paging`], which the publisher pages on.
    pub fn install(&mut self, mut rows: Vec<ContactLedgerRow>, at: ClockStamp) {
        rows.sort_by(|left, right| {
            left.holder
                .cmp(&right.holder)
                .then_with(|| left.contact.cmp(&right.contact))
        });
        let mut kept: Vec<ContactLedgerRow> =
            Vec::with_capacity(rows.len().min(MAX_CONTACT_LEDGER_ROWS));
        let mut dropped = 0_usize;
        let mut run_holder: Option<PrincipalId> = None;
        let mut run = 0_usize;
        for row in rows {
            if run_holder.as_ref() != Some(&row.holder) {
                run_holder = Some(row.holder.clone());
                run = 0;
            }
            if run >= MAX_CONTACTS_PER_OBSERVER || kept.len() >= MAX_CONTACT_LEDGER_ROWS {
                dropped = dropped.saturating_add(1);
                continue;
            }
            run = run.saturating_add(1);
            kept.push(row);
        }
        self.paging = dropped > 0;
        self.rows = kept;
        self.at = Some(at);
    }

    /// Every row, in canonical order.
    #[must_use]
    pub fn rows(&self) -> &[ContactLedgerRow] {
        &self.rows
    }

    /// When the projection was last completed, or `None` before the first one.
    #[must_use]
    pub const fn at(&self) -> Option<&ClockStamp> {
        self.at.as_ref()
    }

    /// Whether the last rebuild found more contacts than the projection holds.
    #[must_use]
    pub const fn is_paging(&self) -> bool {
        self.paging
    }

    /// This observer's rows, at most [`MAX_CONTACTS_PER_OBSERVER`] of them.
    #[must_use]
    pub fn for_observer(&self, observer: &ObserverRef) -> Vec<&ContactLedgerRow> {
        let holder = observer.principal();
        self.rows
            .iter()
            .filter(|row| row.holder == holder)
            .take(MAX_CONTACTS_PER_OBSERVER)
            .collect()
    }
}

/// The bounded, deterministic observation policy `mush-tactics`' sensor ingress
/// reads.
///
/// Installed at composition through `TacticsModule::with_sensor_policy`; the
/// implementation is `mush-propagation`'s, and `mush-tactics` never learns which
/// crate wrote it. **Read-only**: the policy sees an immutable `&World` and
/// returns a claim. Two calls on one unchanged world must return the **same
/// ordered set** — a policy that is not deterministic makes a scheduled retry a
/// different act, which is the one thing the deterministic key exists to
/// prevent.
pub trait SensorPolicy: Send + Sync {
    /// Every intent due at `at`, in canonical order, at most
    /// [`MAX_OBSERVATIONS_PER_PASS`] of them.
    ///
    /// Implementations must bound themselves; [`bounded_pass`] is the shared
    /// guard the ingress applies on top, so an over-long set is **refused, not
    /// truncated**.
    fn due(&self, world: &World, at: &ClockStamp) -> Vec<ObservationIntent>;
}

/// Apply the pass bound to a policy's answer: over the cap is a refusal that
/// names the numbers, never a silent truncation (§4 rule 1, E4c V6).
///
/// # Errors
/// [`ObservationIntentError::PassBound`] past [`MAX_OBSERVATIONS_PER_PASS`].
pub fn bounded_pass(
    intents: Vec<ObservationIntent>,
) -> Result<Vec<ObservationIntent>, ObservationIntentError> {
    if intents.len() > MAX_OBSERVATIONS_PER_PASS {
        return Err(ObservationIntentError::PassBound {
            actual: intents.len(),
            max: MAX_OBSERVATIONS_PER_PASS,
        });
    }
    Ok(intents)
}

/// A newtype over `Option<std::sync::Arc<dyn SensorPolicy>>` so a module can
/// carry the composition-installed policy without an `Option` in its own
/// signature. `None` is an unwired composition and **arms nothing at all** —
/// fail closed, exactly as `AccountOracle::unwired` does.
#[derive(Clone, Default)]
pub struct SensorPolicyHandle(Option<std::sync::Arc<dyn SensorPolicy>>);

impl fmt::Debug for SensorPolicyHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SensorPolicyHandle")
            .field("wired", &self.0.is_some())
            .finish()
    }
}

impl SensorPolicyHandle {
    /// The wired policy.
    #[must_use]
    pub fn new(policy: std::sync::Arc<dyn SensorPolicy>) -> Self {
        Self(Some(policy))
    }

    /// No policy: every pass is empty.
    #[must_use]
    pub const fn unwired() -> Self {
        Self(None)
    }

    /// Whether a policy is installed at all — the composition probe.
    #[must_use]
    pub const fn is_wired(&self) -> bool {
        self.0.is_some()
    }

    /// The bounded due set, or an empty one when unwired.
    ///
    /// # Errors
    /// [`ObservationIntentError::PassBound`] when the policy answered past the
    /// per-pass cap.
    pub fn due(
        &self,
        world: &World,
        at: &ClockStamp,
    ) -> Result<Vec<ObservationIntent>, ObservationIntentError> {
        match &self.0 {
            Some(policy) => bounded_pass(policy.due(world, at)),
            None => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ClockRef;

    fn stamp(at: i64) -> ClockStamp {
        ClockStamp {
            clock: ClockRef::Utc,
            at,
        }
    }

    fn observer() -> ObserverRef {
        ObserverRef::Platform { id: ObjId(7) }
    }

    #[test]
    fn a_key_that_names_another_kind_is_refused() {
        let key = ObservationIntent::key_for(ObservationKind::Loss, &observer(), "o90", 3).unwrap();
        assert_eq!(key.as_str(), "loss/p7/o90/3");
        let error = ObservationIntent::new(
            observer(),
            ObservationOp::Observe {
                truth: SubjectKey::Object { id: ObjId(90) },
                audience: vec![PrincipalId::Object { id: ObjId(7) }],
            },
            stamp(1),
            key,
        )
        .expect_err("a loss key may not arm an observe intent");
        assert!(matches!(
            error,
            ObservationIntentError::KeyKindMismatch {
                expected: "observe",
                ..
            }
        ));
    }

    #[test]
    fn an_opening_audience_is_bounded_and_canonical() {
        let key =
            ObservationIntent::key_for(ObservationKind::Observe, &observer(), "o90", 3).unwrap();
        let duplicate = ObservationIntent::new(
            observer(),
            ObservationOp::Observe {
                truth: SubjectKey::Object { id: ObjId(90) },
                audience: vec![
                    PrincipalId::Object { id: ObjId(7) },
                    PrincipalId::Object { id: ObjId(7) },
                ],
            },
            stamp(1),
            key.clone(),
        )
        .expect_err("a duplicate principal is a silently smaller audience");
        assert_eq!(duplicate, ObservationIntentError::NonCanonicalAudience);
        let empty = ObservationIntent::new(
            observer(),
            ObservationOp::Observe {
                truth: SubjectKey::Object { id: ObjId(90) },
                audience: Vec::new(),
            },
            stamp(1),
            key,
        )
        .expect_err("an audience of nobody is not an observation");
        assert!(matches!(empty, ObservationIntentError::Audience { .. }));
    }

    #[test]
    fn every_kind_names_a_distinct_migrated_action_and_a_distinct_key_prefix() {
        let mut ids: Vec<&str> = ObservationKind::all()
            .iter()
            .map(|k| k.action_id())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 6, "six actions, one per kind, no aliasing");
        let mut names: Vec<&str> = ObservationKind::all().iter().map(|k| k.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 6);
    }

    #[test]
    fn a_pass_over_the_bound_refuses_rather_than_truncating() {
        let key =
            ObservationIntent::key_for(ObservationKind::Refresh, &observer(), "c1", 1).unwrap();
        let intent = ObservationIntent::new(
            observer(),
            ObservationOp::Refresh {
                contact: ContactId::from_bytes([1; 16]),
                expected_revision: 1,
            },
            stamp(1),
            key,
        )
        .unwrap();
        let over = vec![intent; MAX_OBSERVATIONS_PER_PASS + 1];
        let error = bounded_pass(over).expect_err("over the pass bound must refuse");
        assert_eq!(
            error,
            ObservationIntentError::PassBound {
                actual: MAX_OBSERVATIONS_PER_PASS + 1,
                max: MAX_OBSERVATIONS_PER_PASS,
            }
        );
    }

    #[test]
    fn an_unwired_policy_arms_nothing() {
        let handle = SensorPolicyHandle::unwired();
        assert!(!handle.is_wired());
        let world = World::new_minimal();
        assert!(handle.due(&world, &stamp(1)).unwrap().is_empty());
    }
}
