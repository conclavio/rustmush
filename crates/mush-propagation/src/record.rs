//! E7 cut 7 B2/B5 — propagation's own persisted records, all **exempt**
//! (§4.1's cut-7 row): the false-contact `Phantom`, the delivery `Envelope`,
//! the `ContactProvenance` side projection and the `Sighting` book that makes a
//! detection pass idempotent.
//!
//! **There is no contact record here, and there never will be** (08i answer 1,
//! D-E7-36). `ContactState` is K's, `mush-tactics` is its only lifecycle
//! authority, and this crate holds no handle on that family — a cross-owner key
//! would be refused by `ActionTxn::checked_key` even if it did.
//!
//! Every record is a bounded whole document at a deterministic key with a
//! `TryFrom` wire that re-runs its constructor (§4 rule 1), so a stored set over
//! any cap is **refused on load, never truncated**, and a corrupt document is
//! **skipped loudly with its subject marked** (rule 2).

use std::collections::BTreeMap;

use mush_core::{
    CanonicalId, ClockStamp, IdempotencyKeyText, ObjId, OwnerId, PrincipalId, SubjectKey,
};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use thiserror::Error;

use crate::NS;

/// 10b §7: the most relay hops one envelope may take.
pub const MAX_ENVELOPE_HOPS: u8 = 4;
/// 10b §7: how long an envelope lives, in IC days.
pub const ENVELOPE_TTL_DAYS: i64 = 30;
/// The same figure in IC seconds — the arithmetic the pass actually does.
pub const ENVELOPE_TTL_SECS: i64 = ENVELOPE_TTL_DAYS * 86_400;
/// The most bytes one order payload may carry. **Refused at issue, never
/// truncated at delivery.**
pub const MAX_ENVELOPE_PAYLOAD_BYTES: usize = 512;
/// The census bound on live envelopes.
pub const MAX_ENVELOPES: usize = 4_096;
/// The census bound on live phantoms.
pub const MAX_PHANTOMS: usize = 1_024;
/// The census bound on the provenance side projection.
pub const MAX_PROVENANCE: usize = 4_096;
/// The census bound on the sighting book.
pub const MAX_SIGHTINGS: usize = 4_096;
/// The most bytes a phantom id may carry.
pub const MAX_PHANTOM_ID_BYTES: usize = 48;
/// The `SubjectKey::Scoped` class every phantom is minted under. The same
/// constant `mush_tactics::PHANTOM_CLASS` names, asserted equal by a pin.
pub const PHANTOM_CLASS: &str = "phantom";

/// Why a contact is false.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FalseCause {
    /// A sensor artefact — a return with no cause at all.
    Artefact,
    /// A deliberate decoy someone launched.
    Decoy,
    /// A spoofed emission dressed as a platform.
    Spoof,
}

/// A **false contact's authoritative subject**, owned by propagation.
///
/// A phantom is what lets a `False` contact exist from the very first cut
/// without a truthless `ContactState` (which is K's v2 and out of E7): the
/// contact has a subject like any other, that subject is
/// `SubjectKey::Scoped { owner: "propagation", class: "phantom", id }`, and
/// **it resolves to no unit through any projection** — there is no
/// `SubjectKey::Object` anywhere in it, so nothing downstream can turn it into
/// one.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Phantom {
    id: IdempotencyKeyText,
    cause: FalseCause,
    created: ClockStamp,
    expires: ClockStamp,
    /// Which platforms were credited with the false contact. Bounded by K's own
    /// `MAX_CONTACT_AUDIENCE`, cited not redefined.
    holders: Vec<PrincipalId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PhantomWire {
    id: IdempotencyKeyText,
    cause: FalseCause,
    created: ClockStamp,
    expires: ClockStamp,
    holders: Vec<PrincipalId>,
}

impl<'de> Deserialize<'de> for Phantom {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = PhantomWire::deserialize(deserializer)?;
        Self::new(
            wire.id,
            wire.cause,
            wire.created,
            wire.expires,
            wire.holders,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl Phantom {
    /// The one constructor; the wire re-runs it.
    ///
    /// # Errors
    /// [`RecordError`] on an over-long id, an empty or over-large holder set, a
    /// non-canonical holder set, or an expiry that is not after creation.
    pub fn new(
        id: IdempotencyKeyText,
        cause: FalseCause,
        created: ClockStamp,
        expires: ClockStamp,
        mut holders: Vec<PrincipalId>,
    ) -> Result<Self, RecordError> {
        if id.as_str().len() > MAX_PHANTOM_ID_BYTES {
            return Err(RecordError::PhantomId {
                max: MAX_PHANTOM_ID_BYTES,
            });
        }
        holders.sort();
        let before = holders.len();
        holders.dedup();
        if before != holders.len() {
            return Err(RecordError::NonCanonicalHolders);
        }
        if holders.is_empty() || holders.len() > mush_core::MAX_CONTACT_AUDIENCE {
            return Err(RecordError::Holders {
                actual: holders.len(),
                max: mush_core::MAX_CONTACT_AUDIENCE,
            });
        }
        if expires.clock != created.clock || expires.at <= created.at {
            return Err(RecordError::PhantomExpiry);
        }
        Ok(Self {
            id,
            cause,
            created,
            expires,
            holders,
        })
    }

    /// The opaque id.
    #[must_use]
    pub const fn id(&self) -> &IdempotencyKeyText {
        &self.id
    }

    /// Why it is false.
    #[must_use]
    pub const fn cause(&self) -> FalseCause {
        self.cause
    }

    /// When it was minted.
    #[must_use]
    pub const fn created(&self) -> &ClockStamp {
        &self.created
    }

    /// When it stops being credible.
    #[must_use]
    pub const fn expires(&self) -> &ClockStamp {
        &self.expires
    }

    /// Who holds it.
    #[must_use]
    pub fn holders(&self) -> &[PrincipalId] {
        &self.holders
    }

    /// Whether it has expired at `at`. Incomparable clocks answer **`false`**:
    /// an unreadable comparison must not silently expire a live phantom.
    #[must_use]
    pub fn is_expired(&self, at: &ClockStamp) -> bool {
        at.clock == self.expires.clock && at.at >= self.expires.at
    }

    /// **The subject a phantom mints.**
    ///
    /// `SubjectKey::Scoped`, never `SubjectKey::Object` — there is no code path
    /// from a phantom to an object id, which is the structural half of the
    /// no-launder proof.
    #[must_use]
    pub fn subject(&self) -> SubjectKey {
        SubjectKey::Scoped {
            owner: owner(),
            class: CanonicalId::new(PHANTOM_CLASS).expect("static phantom class"),
            id: self.id.clone(),
        }
    }
}

/// Where an envelope is in its life (10b §3.7).
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeState {
    /// Issued, not yet handed to a relay.
    Queued,
    /// In flight. The issuer sees this until the ACK itself arrives.
    Transmitted,
    /// It reached the recipient. Only now may the contact it carries be shared.
    Delivered,
    /// The issuer received the acknowledgement — itself a delivery.
    Acknowledged,
    /// It ran out of hops, out of time, or its recipient went away.
    Expired,
    /// A newer revision to the same recipient was delivered first.
    Superseded,
}

impl EnvelopeState {
    /// Whether nothing further will happen to it.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Acknowledged | Self::Expired | Self::Superseded)
    }
}

/// One relay an envelope passed through.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HopRef {
    /// Who relayed it.
    pub relay: ObjId,
    /// When.
    pub at: ClockStamp,
}

/// A delivery envelope: an order, a report or an acknowledgement in flight.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    id: u64,
    issuer: ObjId,
    recipient: ObjId,
    payload: String,
    revision: u32,
    state: EnvelopeState,
    path: Vec<HopRef>,
    issued: ClockStamp,
    advanced: ClockStamp,
    /// The envelope this one acknowledges. **An acknowledgement is itself a
    /// delivery** (Q4A) — it is an ordinary envelope with this field set, and
    /// it must arrive before the issuer learns anything.
    ack_of: Option<u64>,
    /// The contact this envelope carries a REPORT of.
    ///
    /// `contact_share` is submitted **only after this envelope is
    /// `Delivered`** (10b §3.7, OPEN 4): a `Transmitted` envelope shares
    /// nothing, which is what makes "contacts are local; sharing is a delivery"
    /// true rather than aspirational. Propagation never writes the share — it
    /// states the intent, and `mush-tactics`' ingress commits it.
    shares: Option<mush_core::ContactId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvelopeWire {
    id: u64,
    issuer: ObjId,
    recipient: ObjId,
    payload: String,
    revision: u32,
    state: EnvelopeState,
    path: Vec<HopRef>,
    issued: ClockStamp,
    advanced: ClockStamp,
    #[serde(default)]
    ack_of: Option<u64>,
    #[serde(default)]
    shares: Option<mush_core::ContactId>,
}

impl<'de> Deserialize<'de> for Envelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = EnvelopeWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.id,
            wire.issuer,
            wire.recipient,
            wire.payload,
            wire.revision,
            wire.state,
            wire.path,
            wire.issued,
            wire.advanced,
            wire.ack_of,
            wire.shares,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl Envelope {
    /// Issue one. **The payload cap is checked here, at issue** — never
    /// truncated at delivery (the plan's own words).
    ///
    /// # Errors
    /// [`RecordError`] on an empty or over-long payload, a zero revision, or an
    /// issuer that is its own recipient.
    pub fn issue(
        id: u64,
        issuer: ObjId,
        recipient: ObjId,
        payload: String,
        revision: u32,
        issued: ClockStamp,
    ) -> Result<Self, RecordError> {
        Self::from_parts(
            id,
            issuer,
            recipient,
            payload,
            revision,
            EnvelopeState::Queued,
            Vec::new(),
            issued.clone(),
            issued,
            None,
            None,
        )
    }

    /// Issue a contact REPORT: an ordinary envelope that also carries a contact
    /// the recipient is to be given **once it arrives**.
    ///
    /// # Errors
    /// [`RecordError`] as [`Self::issue`].
    pub fn report(
        id: u64,
        issuer: ObjId,
        recipient: ObjId,
        contact: mush_core::ContactId,
        revision: u32,
        issued: ClockStamp,
    ) -> Result<Self, RecordError> {
        Self::from_parts(
            id,
            issuer,
            recipient,
            format!("report {}", contact.to_hex()),
            revision,
            EnvelopeState::Queued,
            Vec::new(),
            issued.clone(),
            issued,
            None,
            Some(contact),
        )
    }

    /// Issue the acknowledgement of `original` — an ordinary envelope back to
    /// the original's issuer, which must itself be delivered.
    ///
    /// # Errors
    /// [`RecordError`] as [`Self::issue`].
    pub fn acknowledge(id: u64, original: &Self, at: ClockStamp) -> Result<Self, RecordError> {
        Self::from_parts(
            id,
            original.recipient,
            original.issuer,
            format!("ack {}", original.id),
            original.revision,
            EnvelopeState::Queued,
            Vec::new(),
            at.clone(),
            at,
            Some(original.id),
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        id: u64,
        issuer: ObjId,
        recipient: ObjId,
        payload: String,
        revision: u32,
        state: EnvelopeState,
        path: Vec<HopRef>,
        issued: ClockStamp,
        advanced: ClockStamp,
        ack_of: Option<u64>,
        shares: Option<mush_core::ContactId>,
    ) -> Result<Self, RecordError> {
        if payload.is_empty() || payload.len() > MAX_ENVELOPE_PAYLOAD_BYTES {
            return Err(RecordError::Payload {
                actual: payload.len(),
                max: MAX_ENVELOPE_PAYLOAD_BYTES,
            });
        }
        if revision == 0 {
            return Err(RecordError::ZeroRevision);
        }
        if issuer == recipient {
            return Err(RecordError::SelfAddressed);
        }
        if path.len() > usize::from(MAX_ENVELOPE_HOPS) {
            return Err(RecordError::Hops {
                actual: path.len(),
                max: usize::from(MAX_ENVELOPE_HOPS),
            });
        }
        // A relayed report's age is monotonic: hop stamps never move backwards.
        if path
            .windows(2)
            .any(|pair| pair[0].at.clock != pair[1].at.clock || pair[0].at.at > pair[1].at.at)
        {
            return Err(RecordError::HopsRegressed);
        }
        Ok(Self {
            id,
            issuer,
            recipient,
            payload,
            revision,
            state,
            path,
            issued,
            advanced,
            ack_of,
            shares,
        })
    }

    /// The id.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }
    /// Who issued it.
    #[must_use]
    pub const fn issuer(&self) -> ObjId {
        self.issuer
    }
    /// Who it is addressed to.
    #[must_use]
    pub const fn recipient(&self) -> ObjId {
        self.recipient
    }
    /// The order text.
    #[must_use]
    pub fn payload(&self) -> &str {
        &self.payload
    }
    /// Which revision of the standing order this is.
    #[must_use]
    pub const fn revision(&self) -> u32 {
        self.revision
    }
    /// Where it is.
    #[must_use]
    pub const fn state(&self) -> EnvelopeState {
        self.state
    }
    /// Which relays it passed.
    #[must_use]
    pub fn path(&self) -> &[HopRef] {
        &self.path
    }
    /// When it was issued.
    #[must_use]
    pub const fn issued(&self) -> &ClockStamp {
        &self.issued
    }
    /// When it last moved.
    #[must_use]
    pub const fn advanced(&self) -> &ClockStamp {
        &self.advanced
    }
    /// The envelope this one acknowledges, when it is an acknowledgement.
    #[must_use]
    pub const fn ack_of(&self) -> Option<u64> {
        self.ack_of
    }
    /// The contact this envelope reports, when it reports one. Read only after
    /// [`EnvelopeState::Delivered`].
    #[must_use]
    pub const fn shares(&self) -> Option<mush_core::ContactId> {
        self.shares
    }
    /// Its age at `at`, in IC seconds. Incomparable clocks answer 0 rather than
    /// producing a nonsense age.
    #[must_use]
    pub fn age(&self, at: &ClockStamp) -> i64 {
        if at.clock == self.issued.clock {
            at.at.saturating_sub(self.issued.at).max(0)
        } else {
            0
        }
    }

    /// Take one relay hop.
    ///
    /// # Errors
    /// [`RecordError::Hops`] at [`MAX_ENVELOPE_HOPS`] — which is what makes a
    /// relay CYCLE (A → B → A) terminate rather than growing age without bound.
    pub fn relayed(&self, relay: ObjId, at: ClockStamp) -> Result<Self, RecordError> {
        let mut path = self.path.clone();
        path.push(HopRef {
            relay,
            at: at.clone(),
        });
        Self::from_parts(
            self.id,
            self.issuer,
            self.recipient,
            self.payload.clone(),
            self.revision,
            EnvelopeState::Transmitted,
            path,
            self.issued.clone(),
            at,
            self.ack_of,
            self.shares,
        )
    }

    /// Move to a new state. Terminal states are **idempotent**: re-applying one
    /// is a HIT that changes nothing, which is how expiry is idempotent and a
    /// duplicate ack is a HIT.
    #[must_use]
    pub fn in_state(&self, state: EnvelopeState, at: ClockStamp) -> Self {
        if self.state == state {
            return self.clone();
        }
        Self {
            state,
            advanced: at,
            ..self.clone()
        }
    }
}

/// How a contact was come by (10b §3.7's side projection).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Provenance {
    /// This observer's own sensors saw it.
    Local,
    /// Somebody reported it. **A stale relayed track is visibly stale and a
    /// false report is attributable to its transmitter.**
    Reported { source: ObjId, age: i64, via: u64 },
}

/// The confidence / provenance **side projection**, keyed by `ContactId`.
///
/// **NO LIFECYCLE AUTHORITY** (OPEN 2, taken as recommended). It never gates
/// legality, no prepared mechanic reads it, and no provider consults it: it is a
/// projection a viewer reads beside a contact so a relayed track is visibly
/// stale. §4.1's closing rule is the tell — the day anything gates on it, it
/// moves to authoritative first. **Flagged OPEN for K review.**
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContactProvenance {
    contact: mush_core::ContactId,
    band: CanonicalId,
    provenance: Provenance,
    generation: u64,
    recorded: ClockStamp,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContactProvenanceWire {
    contact: mush_core::ContactId,
    band: CanonicalId,
    provenance: Provenance,
    generation: u64,
    recorded: ClockStamp,
}

impl<'de> Deserialize<'de> for ContactProvenance {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ContactProvenanceWire::deserialize(deserializer)?;
        Self::new(
            wire.contact,
            wire.band,
            wire.provenance,
            wire.generation,
            wire.recorded,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl ContactProvenance {
    /// The one constructor; the wire re-runs it.
    ///
    /// # Errors
    /// [`RecordError::ZeroRevision`] on a zero generation, or
    /// [`RecordError::AgeRegressed`] on a reported age below zero — a negative
    /// age would render as fresher than local truth.
    pub fn new(
        contact: mush_core::ContactId,
        band: CanonicalId,
        provenance: Provenance,
        generation: u64,
        recorded: ClockStamp,
    ) -> Result<Self, RecordError> {
        if generation == 0 {
            return Err(RecordError::ZeroRevision);
        }
        if let Provenance::Reported { age, .. } = &provenance
            && *age < 0
        {
            return Err(RecordError::AgeRegressed);
        }
        Ok(Self {
            contact,
            band,
            provenance,
            generation,
            recorded,
        })
    }

    /// Which contact.
    #[must_use]
    pub const fn contact(&self) -> mush_core::ContactId {
        self.contact
    }
    /// Which sensor band.
    #[must_use]
    pub const fn band(&self) -> &CanonicalId {
        &self.band
    }
    /// How it was come by.
    #[must_use]
    pub const fn provenance(&self) -> &Provenance {
        &self.provenance
    }
    /// The contact generation this row describes.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    /// When it was recorded.
    #[must_use]
    pub const fn recorded(&self) -> &ClockStamp {
        &self.recorded
    }
}

/// One `(observer, subject)` pair propagation has already acted on.
///
/// The detection pass's idempotency key: without it, every pass would arm a
/// fresh `Observe` for a subject that is already a contact, and two passes over
/// one unchanged world would not produce the same set.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Sighting {
    /// The observing platform.
    pub observer: ObjId,
    /// The observed platform, or `None` for a phantom sighting.
    pub subject: Option<ObjId>,
    /// The phantom, when the sighting is false.
    pub phantom: Option<IdempotencyKeyText>,
    /// When it was first armed.
    pub first: ClockStamp,
    /// When it was last armed.
    pub last: ClockStamp,
}

impl Sighting {
    /// The document/book key for one sighting.
    #[must_use]
    pub fn key_of(
        observer: ObjId,
        subject: Option<ObjId>,
        phantom: Option<&IdempotencyKeyText>,
    ) -> String {
        match (subject, phantom) {
            (Some(subject), _) => format!("{}/o{}", observer.0, subject.0),
            (None, Some(phantom)) => format!("{}/f{}", observer.0, phantom.as_str()),
            (None, None) => format!("{}/none", observer.0),
        }
    }

    /// This sighting's key.
    #[must_use]
    pub fn key(&self) -> String {
        Self::key_of(self.observer, self.subject, self.phantom.as_ref())
    }
}

/// A record that will not construct or will not load.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RecordError {
    #[error("a phantom id is at most {max} bytes")]
    PhantomId { max: usize },
    #[error("a phantom's holder set size {actual} is outside 1..={max}")]
    Holders { actual: usize, max: usize },
    #[error("a phantom's holders must be strictly sorted and duplicate-free")]
    NonCanonicalHolders,
    #[error("a phantom must expire after it was created, on one clock generation")]
    PhantomExpiry,
    #[error("an order payload of {actual} bytes is outside 1..={max}")]
    Payload { actual: usize, max: usize },
    #[error("an envelope revision must be non-zero")]
    ZeroRevision,
    #[error("an envelope cannot be addressed to its own issuer")]
    SelfAddressed,
    #[error("an envelope took {actual} hops; the bound is {max}")]
    Hops { actual: usize, max: usize },
    #[error("an envelope's relay stamps moved backwards")]
    HopsRegressed,
    #[error("a reported contact's age cannot be negative")]
    AgeRegressed,
    #[error("that book is full: {actual} records against a bound of {max}")]
    Census { actual: usize, max: usize },
}

/// The propagation owner id.
#[must_use]
pub fn owner() -> OwnerId {
    OwnerId::new(NS).expect("static propagation owner id")
}

/// The `propagation.envelope` document key.
#[must_use]
pub fn envelope_key(id: u64) -> SmolStr {
    SmolStr::new(format!("envelope:{id}"))
}

/// The `propagation.phantom` document key.
#[must_use]
pub fn phantom_key(id: &IdempotencyKeyText) -> SmolStr {
    SmolStr::new(format!("phantom:{}", id.as_str()))
}

/// The `propagation.provenance` document key.
#[must_use]
pub fn provenance_key(contact: mush_core::ContactId) -> SmolStr {
    SmolStr::new(format!("provenance:{}", contact.to_hex()))
}

/// The sighting-book document key.
#[must_use]
pub fn sighting_key(key: &str) -> SmolStr {
    SmolStr::new(format!("sighting:{key}"))
}

/// The allocator document key.
pub const ALLOCATOR_KEY: &str = "propagation_meta";

/// The world-owned RAM mirror of every propagation record.
///
/// Persisted as the exempt `propagation.envelope` / `.phantom` / `.provenance`
/// documents and re-hydrated at boot; a document that will not decode is
/// **held, not deleted** (cut 6's R3 lesson: the record is the only place the
/// fact lives) and its subject is marked.
#[derive(Clone, Debug, Default)]
pub struct PropagationStore {
    envelopes: BTreeMap<u64, Envelope>,
    phantoms: BTreeMap<IdempotencyKeyText, Phantom>,
    provenance: BTreeMap<mush_core::ContactId, ContactProvenance>,
    sightings: BTreeMap<String, Sighting>,
    next_envelope: u64,
    next_phantom: u64,
    /// Declared interference by place (staff, or a later electronic-warfare
    /// result). **Delivery-affecting only**: R4's jammed unit still takes
    /// orders, and `resolve_detection` never reads this term.
    jamming: BTreeMap<ObjId, u32>,
    /// Documents that would not decode, kept verbatim so a later fix can read
    /// them and a boot never destroys the only copy.
    held: BTreeMap<SmolStr, serde_json::Value>,
}

impl PropagationStore {
    /// Every live envelope, in id order.
    pub fn envelopes(&self) -> impl Iterator<Item = &Envelope> {
        self.envelopes.values()
    }

    /// One envelope.
    #[must_use]
    pub fn envelope(&self, id: u64) -> Option<&Envelope> {
        self.envelopes.get(&id)
    }

    /// Insert or replace one envelope.
    ///
    /// # Errors
    /// [`RecordError::Census`] at [`MAX_ENVELOPES`] for a NEW id (replacing an
    /// existing one never grows the book, so it always succeeds).
    pub fn put_envelope(&mut self, envelope: Envelope) -> Result<(), RecordError> {
        if !self.envelopes.contains_key(&envelope.id) && self.envelopes.len() >= MAX_ENVELOPES {
            return Err(RecordError::Census {
                actual: self.envelopes.len(),
                max: MAX_ENVELOPES,
            });
        }
        self.envelopes.insert(envelope.id, envelope);
        Ok(())
    }

    /// The next envelope id, or `None` when the allocator is exhausted.
    pub fn alloc_envelope(&mut self) -> Option<u64> {
        self.next_envelope = self.next_envelope.checked_add(1)?;
        Some(self.next_envelope)
    }

    /// The next phantom ordinal, or `None` when exhausted.
    pub fn alloc_phantom(&mut self) -> Option<u64> {
        self.next_phantom = self.next_phantom.checked_add(1)?;
        Some(self.next_phantom)
    }

    /// Every live phantom, in id order.
    pub fn phantoms(&self) -> impl Iterator<Item = &Phantom> {
        self.phantoms.values()
    }

    /// One phantom.
    #[must_use]
    pub fn phantom(&self, id: &IdempotencyKeyText) -> Option<&Phantom> {
        self.phantoms.get(id)
    }

    /// Insert or replace one phantom.
    ///
    /// # Errors
    /// [`RecordError::Census`] at [`MAX_PHANTOMS`] for a NEW id.
    pub fn put_phantom(&mut self, phantom: Phantom) -> Result<(), RecordError> {
        if !self.phantoms.contains_key(&phantom.id) && self.phantoms.len() >= MAX_PHANTOMS {
            return Err(RecordError::Census {
                actual: self.phantoms.len(),
                max: MAX_PHANTOMS,
            });
        }
        self.phantoms.insert(phantom.id.clone(), phantom);
        Ok(())
    }

    /// Drop one phantom (its expiry has been issued to every holder).
    pub fn drop_phantom(&mut self, id: &IdempotencyKeyText) -> Option<Phantom> {
        self.phantoms.remove(id)
    }

    /// One contact's provenance row.
    #[must_use]
    pub fn provenance_of(&self, contact: mush_core::ContactId) -> Option<&ContactProvenance> {
        self.provenance.get(&contact)
    }

    /// Every provenance row.
    pub fn provenance(&self) -> impl Iterator<Item = &ContactProvenance> {
        self.provenance.values()
    }

    /// Record one contact's provenance.
    ///
    /// # Errors
    /// [`RecordError::Census`] at [`MAX_PROVENANCE`] for a NEW contact.
    pub fn put_provenance(&mut self, row: ContactProvenance) -> Result<(), RecordError> {
        if !self.provenance.contains_key(&row.contact) && self.provenance.len() >= MAX_PROVENANCE {
            return Err(RecordError::Census {
                actual: self.provenance.len(),
                max: MAX_PROVENANCE,
            });
        }
        self.provenance.insert(row.contact, row);
        Ok(())
    }

    /// One sighting.
    #[must_use]
    pub fn sighting(&self, key: &str) -> Option<&Sighting> {
        self.sightings.get(key)
    }

    /// Every sighting.
    pub fn sightings(&self) -> impl Iterator<Item = &Sighting> {
        self.sightings.values()
    }

    /// Record one sighting.
    ///
    /// # Errors
    /// [`RecordError::Census`] at [`MAX_SIGHTINGS`] for a NEW pair.
    pub fn put_sighting(&mut self, sighting: Sighting) -> Result<(), RecordError> {
        let key = sighting.key();
        if !self.sightings.contains_key(&key) && self.sightings.len() >= MAX_SIGHTINGS {
            return Err(RecordError::Census {
                actual: self.sightings.len(),
                max: MAX_SIGHTINGS,
            });
        }
        self.sightings.insert(key, sighting);
        Ok(())
    }

    /// Documents held because they would not decode.
    #[must_use]
    pub fn held(&self) -> &BTreeMap<SmolStr, serde_json::Value> {
        &self.held
    }

    /// Hold one undecodable document.
    pub fn hold(&mut self, key: SmolStr, value: serde_json::Value) {
        self.held.insert(key, value);
    }

    /// The declared jamming level at a place.
    #[must_use]
    pub fn jamming_at(&self, place: ObjId) -> u32 {
        self.jamming.get(&place).copied().unwrap_or(0)
    }

    /// Declare (or clear, at 0) the jamming level at a place.
    pub fn set_jamming(&mut self, place: ObjId, level: u32) {
        if level == 0 {
            self.jamming.remove(&place);
        } else {
            self.jamming.insert(place, level);
        }
    }

    /// The allocator document, persisted beside the records.
    #[must_use]
    pub fn allocator(&self) -> serde_json::Value {
        serde_json::json!({
            "next_envelope": self.next_envelope,
            "next_phantom": self.next_phantom,
            "jamming": self.jamming.iter().map(|(place, level)| serde_json::json!({
                "place": place.0, "level": level
            })).collect::<Vec<_>>(),
        })
    }

    /// Install a hydrated allocator.
    pub fn install_allocator(&mut self, value: &serde_json::Value) {
        self.next_envelope = value
            .get("next_envelope")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            .max(self.next_envelope);
        self.next_phantom = value
            .get("next_phantom")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            .max(self.next_phantom);
        if let Some(rows) = value.get("jamming").and_then(serde_json::Value::as_array) {
            for row in rows.iter().take(MAX_SIGHTINGS) {
                let (Some(place), Some(level)) = (
                    row.get("place").and_then(serde_json::Value::as_u64),
                    row.get("level").and_then(serde_json::Value::as_u64),
                ) else {
                    continue;
                };
                let (Ok(place), Ok(level)) = (u32::try_from(place), u32::try_from(level)) else {
                    continue;
                };
                self.set_jamming(ObjId(place), level);
            }
        }
    }

    /// Raise the allocators past whatever hydrated, so a restart never re-uses
    /// an id even if the allocator document itself was lost.
    pub fn reconcile_allocators(&mut self) {
        if let Some(highest) = self.envelopes.keys().next_back() {
            self.next_envelope = self.next_envelope.max(*highest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::ClockRef;

    fn stamp(at: i64) -> ClockStamp {
        ClockStamp {
            clock: ClockRef::Utc,
            at,
        }
    }

    fn phantom() -> Phantom {
        Phantom::new(
            IdempotencyKeyText::parse_canonical("ghost/1").unwrap(),
            FalseCause::Decoy,
            stamp(10),
            stamp(40),
            vec![PrincipalId::Object { id: ObjId(7) }],
        )
        .unwrap()
    }

    #[test]
    fn a_phantoms_subject_is_scoped_and_has_no_object_arm_at_all() {
        let subject = phantom().subject();
        let SubjectKey::Scoped { owner, class, id } = &subject else {
            panic!("a phantom subject is never an object");
        };
        assert_eq!(owner.as_str(), NS);
        assert_eq!(class.as_str(), PHANTOM_CLASS);
        assert_eq!(id.as_str(), "ghost/1");
        // The serialized form carries no object id anywhere.
        let text = serde_json::to_string(&subject).unwrap();
        assert!(!text.contains("\"object\""), "{text}");
    }

    #[test]
    fn an_over_long_payload_is_refused_at_issue_not_truncated_at_delivery() {
        let long = "x".repeat(MAX_ENVELOPE_PAYLOAD_BYTES + 1);
        let error = Envelope::issue(1, ObjId(1), ObjId(2), long, 1, stamp(0))
            .expect_err("an over-long payload must refuse");
        assert!(matches!(error, RecordError::Payload { .. }));
    }

    #[test]
    fn a_relay_cycle_terminates_at_the_hop_bound() {
        let mut envelope =
            Envelope::issue(1, ObjId(1), ObjId(2), "hold".into(), 1, stamp(0)).unwrap();
        for hop in 0..MAX_ENVELOPE_HOPS {
            // A → B → A → B …: the cycle is legal, the bound is what stops it.
            let relay = if hop % 2 == 0 { ObjId(3) } else { ObjId(4) };
            envelope = envelope
                .relayed(relay, stamp(i64::from(hop) + 1))
                .expect("within the bound");
        }
        let error = envelope
            .relayed(ObjId(3), stamp(99))
            .expect_err("the bound terminates the cycle");
        assert!(matches!(error, RecordError::Hops { .. }));
        assert_eq!(
            envelope.age(&stamp(99)),
            99,
            "age is monotonic, not unbounded"
        );
    }

    #[test]
    fn every_record_re_enters_through_its_constructor() {
        // A stored phantom with a duplicated holder set, an envelope past the
        // hop bound and a negative reported age are all refused ON LOAD.
        let mut value = serde_json::to_value(phantom()).unwrap();
        value["holders"] = serde_json::json!([
            { "kind": "object", "id": 7 },
            { "kind": "object", "id": 7 }
        ]);
        assert!(serde_json::from_value::<Phantom>(value).is_err());

        let envelope = Envelope::issue(1, ObjId(1), ObjId(2), "go".into(), 1, stamp(0)).unwrap();
        let mut value = serde_json::to_value(&envelope).unwrap();
        value["path"] = serde_json::json!(
            (0..=MAX_ENVELOPE_HOPS)
                .map(|n| serde_json::json!({
                    "relay": 3,
                    "at": { "clock": { "kind": "utc" }, "at": n }
                }))
                .collect::<Vec<_>>()
        );
        assert!(serde_json::from_value::<Envelope>(value).is_err());

        let row = ContactProvenance::new(
            mush_core::ContactId::from_bytes([2; 16]),
            CanonicalId::new("radar").unwrap(),
            Provenance::Reported {
                source: ObjId(9),
                age: 5,
                via: 1,
            },
            1,
            stamp(1),
        )
        .unwrap();
        let mut value = serde_json::to_value(&row).unwrap();
        value["provenance"]["age"] = serde_json::json!(-1);
        assert!(serde_json::from_value::<ContactProvenance>(value).is_err());
    }

    #[test]
    fn a_terminal_transition_is_idempotent() {
        let envelope = Envelope::issue(1, ObjId(1), ObjId(2), "go".into(), 1, stamp(0)).unwrap();
        let expired = envelope.in_state(EnvelopeState::Expired, stamp(5));
        let again = expired.in_state(EnvelopeState::Expired, stamp(9));
        assert_eq!(expired, again, "expiry is idempotent");
        assert_eq!(again.advanced().at, 5, "and it does not re-stamp");
    }
}
