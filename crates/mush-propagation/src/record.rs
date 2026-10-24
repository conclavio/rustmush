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
/// The most undecodable documents the boot holds in RAM (the fold's B7).
///
/// The four censuses bound the **books**; without this one a store carrying a
/// hundred thousand unreadable envelope documents booted with all of them held
/// verbatim, so the caps bounded nothing about the process. Past it the boot
/// **counts and pages** rather than retaining.
pub const MAX_HELD_DOCUMENTS: usize = 64;
/// The most lines the tick may queue for the next staff reader.
pub const MAX_PAGED_NOTICES: usize = 32;
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
    /// The allocator ordinal this phantom was minted under.
    ///
    /// Recorded on the record itself (fold B11) because the id is now a digest:
    /// a lost or older-schema allocator document would otherwise reset the
    /// ordinal to 0 while the `phantom:*` documents hydrate normally, and the
    /// next mint would re-issue a live phantom's identity. `#[serde(default)]`,
    /// so a pre-fold document still loads.
    ordinal: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PhantomWire {
    id: IdempotencyKeyText,
    cause: FalseCause,
    created: ClockStamp,
    expires: ClockStamp,
    holders: Vec<PrincipalId>,
    #[serde(default)]
    ordinal: u64,
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
        .map(|phantom| phantom.with_ordinal(wire.ordinal))
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
            ordinal: 0,
        })
    }

    /// The same phantom, carrying the allocator ordinal it was minted under.
    #[must_use]
    pub fn with_ordinal(mut self, ordinal: u64) -> Self {
        self.ordinal = ordinal;
        self
    }

    /// The allocator ordinal, or 0 for a pre-fold document.
    #[must_use]
    pub const fn ordinal(&self) -> u64 {
        self.ordinal
    }

    /// Mint the opaque id for a phantom.
    ///
    /// **A digest, not a dbref** (fold K13). The id was
    /// `<observer dbref>/<ordinal>`, and it appears verbatim in the document
    /// key, in the sighting book and in `ContactLedgerRow.phantom` — so anyone
    /// who could read a phantom id learned which platform it was minted for, and
    /// the phantom namespace was not independent of the real object space, which
    /// is the property the no-launder argument leans on.
    ///
    /// # Errors
    /// [`RecordError::PhantomId`] when the digest will not key.
    pub fn mint_id(
        observer: ObjId,
        ordinal: u64,
        created: &ClockStamp,
    ) -> Result<IdempotencyKeyText, RecordError> {
        let digest = mush_core::CanonicalHash::for_canonical_value(
            "propagation.phantom_id.v1",
            1,
            &serde_json::json!({
                "observer": observer.0,
                "ordinal": ordinal,
                "created": created,
            }),
            1024,
        )
        .map_err(|_| RecordError::PhantomId {
            max: MAX_PHANTOM_ID_BYTES,
        })?;
        IdempotencyKeyText::parse_canonical(format!("ph{}", &digest.to_hex()[..32])).map_err(|_| {
            RecordError::PhantomId {
                max: MAX_PHANTOM_ID_BYTES,
            }
        })
    }

    /// Re-anchor this phantom's stamps onto `clock`, keeping their IC scalars.
    ///
    /// Fold D5(a)(ii), `mush_sim::journey::reanchor`'s shape and argument: the
    /// IC scalar is continuous across a `WorldIc` generation boundary by
    /// construction, so moving the identity and keeping the scalar is exact.
    /// Without it a rate change froze every phantom: `is_expired` answered
    /// `false` for ever, no holder got its `Loss`, and the false contact became
    /// permanent knowledge.
    #[must_use]
    pub fn reanchored(&self, clock: &mush_core::ClockRef) -> Self {
        let mut next = self.clone();
        next.created.clock = clock.clone();
        next.expires.clock = clock.clone();
        next
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

/// Typed dispatch semantics. Only orders participate in revision supersession.
/// The mail/channel values reserve that boundary; their producers arrive in R6.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchKind {
    Order,
    Report,
    Acknowledgement,
    Mail,
    Channel,
}

/// A delivery envelope: an order, a report or an acknowledgement in flight.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    #[serde(skip_serializing_if = "Option::is_none")]
    representation: Option<mush_core::RelayRepresentation>,
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
    kind: DispatchKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    transit: Option<crate::routing::Transit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dispatch: Option<crate::dispatch::DispatchRef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvelopeWire {
    #[serde(default)]
    representation: Option<mush_core::RelayRepresentation>,
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
    #[serde(default)]
    kind: Option<DispatchKind>,
    #[serde(default)]
    transit: Option<crate::routing::Transit>,
    #[serde(default)]
    dispatch: Option<crate::dispatch::DispatchRef>,
}

impl<'de> Deserialize<'de> for Envelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = EnvelopeWire::deserialize(deserializer)?;
        let mut envelope = Self::from_parts(
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
            matches!(wire.kind, Some(DispatchKind::Mail | DispatchKind::Channel)),
        )
        .map_err(serde::de::Error::custom)?;
        envelope.representation = wire.representation;
        if let Some(kind) = wire.kind {
            if (kind == DispatchKind::Acknowledgement) != envelope.ack_of.is_some()
                || (kind == DispatchKind::Report) != envelope.shares.is_some()
            {
                return Err(serde::de::Error::custom(
                    "dispatch kind contradicts its payload reference",
                ));
            }
            envelope.kind = kind;
        }
        if let Some(reference) = wire.dispatch {
            if !reference.valid()
                || !matches!(envelope.kind, DispatchKind::Mail | DispatchKind::Channel)
            {
                return Err(serde::de::Error::custom("invalid dispatch reference"));
            }
            envelope.dispatch = Some(reference);
        } else if matches!(envelope.kind, DispatchKind::Mail | DispatchKind::Channel) {
            return Err(serde::de::Error::custom(
                "message dispatch reference missing",
            ));
        }
        if let Some(transit) = wire.transit {
            transit.validate().map_err(serde::de::Error::custom)?;
            if transit
                .flight
                .as_ref()
                .is_some_and(|f| f.started.clock != envelope.issued.clock)
                || transit
                    .queued_at
                    .as_ref()
                    .is_some_and(|at| at.clock != envelope.issued.clock)
                || transit
                    .history
                    .iter()
                    .any(|h| h.arrived.clock != envelope.issued.clock)
                || transit
                    .legs
                    .iter()
                    .skip(transit.next)
                    .filter(|l| l.pair.is_some())
                    .count()
                    + envelope.path.len()
                    > crate::routing::MAX_LINK_HOPS
            {
                return Err(serde::de::Error::custom(
                    "itinerary contradicts envelope clock or hop budget",
                ));
            }
            if transit
                .service
                .as_ref()
                .is_some_and(|t| t.units != crate::service::units(&envelope))
            {
                return Err(serde::de::Error::custom(
                    "service units contradict content accounting",
                ));
            }
            envelope.transit = Some(transit);
        }
        Ok(envelope)
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
            false,
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
            false,
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
            false,
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
        allow_self: bool,
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
        if issuer == recipient && !allow_self {
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
            kind: if ack_of.is_some() {
                DispatchKind::Acknowledgement
            } else if shares.is_some() {
                DispatchKind::Report
            } else {
                DispatchKind::Order
            },
            representation: None,
            transit: None,
            dispatch: None,
        })
    }

    /// A source-owned message reference; self-mail is a legitimate local route.
    pub fn dispatch(
        id: u64,
        issuer: ObjId,
        recipient: ObjId,
        kind: DispatchKind,
        reference: crate::dispatch::DispatchRef,
        issued: ClockStamp,
    ) -> Result<Self, RecordError> {
        if !reference.valid() || !matches!(kind, DispatchKind::Mail | DispatchKind::Channel) {
            return Err(RecordError::DispatchReference);
        }
        let mut envelope = Self::from_parts(
            id,
            issuer,
            recipient,
            reference.key.clone(),
            1,
            EnvelopeState::Queued,
            Vec::new(),
            issued.clone(),
            issued,
            None,
            None,
            true,
        )?;
        envelope.kind = kind;
        envelope.dispatch = Some(reference);
        Ok(envelope)
    }

    #[must_use]
    pub fn representation(&self) -> Option<mush_core::RelayRepresentation> {
        self.representation
    }
    pub fn with_representation(mut self, r: Option<mush_core::RelayRepresentation>) -> Self {
        self.representation = r;
        self
    }
    pub fn dispatch_ref(&self) -> Option<&crate::dispatch::DispatchRef> {
        self.dispatch.as_ref()
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

    #[must_use]
    pub const fn kind(&self) -> DispatchKind {
        self.kind
    }

    #[must_use]
    pub fn transit(&self) -> Option<&crate::routing::Transit> {
        self.transit.as_ref()
    }

    #[must_use]
    pub fn with_transit(&self, transit: crate::routing::Transit) -> Self {
        let mut next = self.clone();
        next.transit = Some(transit);
        next
    }
    /// Its age at `at`, in IC seconds. Incomparable clocks answer 0 rather than
    /// producing a nonsense age; use [`Self::checked_age`] where the difference
    /// matters, because 0 renders as **fresher than the truth**.
    #[must_use]
    pub fn age(&self, at: &ClockStamp) -> i64 {
        self.checked_age(at).unwrap_or(0)
    }

    /// Its age at `at`, or `None` when the two stamps are incomparable.
    ///
    /// Fold D5(a)(i): a render must never present an incomparable stamp as
    /// fresher than the truth, so the one view that shows an age asks this and
    /// prints `stale` for `None`.
    #[must_use]
    pub fn checked_age(&self, at: &ClockStamp) -> Option<i64> {
        (at.clock == self.issued.clock).then(|| at.at.saturating_sub(self.issued.at).max(0))
    }

    /// Re-anchor every stamp onto `clock`, keeping the IC scalars (fold D5).
    ///
    /// The scalars are continuous across a `WorldIc` generation boundary by
    /// construction, so this is exact; without it an envelope's `age` answered 0
    /// for ever after a rate change, it never reached `ENVELOPE_TTL_SECS`, and
    /// the whole delivery book stopped ageing out.
    #[must_use]
    pub fn reanchored(&self, clock: &mush_core::ClockRef) -> Self {
        let mut next = self.clone();
        next.issued.clock = clock.clone();
        next.advanced.clock = clock.clone();
        for hop in &mut next.path {
            hop.at.clock = clock.clone();
        }
        if let Some(transit) = &mut next.transit {
            transit.reanchor(clock);
        }
        next
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
            self.dispatch.is_some(),
        )
        .map(|mut next| {
            next.kind = self.kind;
            next.transit = self.transit.clone();
            next.dispatch = self.dispatch.clone();
            next
        })
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
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Sighting {
    /// The observing platform.
    observer: ObjId,
    /// The observed platform, or `None` for a phantom sighting.
    subject: Option<ObjId>,
    /// The phantom, when the sighting is false.
    phantom: Option<IdempotencyKeyText>,
    /// When it was first armed.
    first: ClockStamp,
    /// When it was last armed.
    last: ClockStamp,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SightingWire {
    observer: ObjId,
    subject: Option<ObjId>,
    phantom: Option<IdempotencyKeyText>,
    first: ClockStamp,
    last: ClockStamp,
}

impl<'de> Deserialize<'de> for Sighting {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = SightingWire::deserialize(deserializer)?;
        Self::new(
            wire.observer,
            wire.subject,
            wire.phantom,
            wire.first,
            wire.last,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl Sighting {
    /// The one constructor; the wire re-runs it (§4 rule 1, fold B10).
    ///
    /// Before the fold `Sighting` was the one exempt record with public fields,
    /// a derived `Deserialize` and no constructor at all: a document with
    /// `subject: null, phantom: null` loaded and folded every such row for one
    /// observer onto a single key, and `first`/`last` were checked neither for
    /// monotonicity nor for one clock generation.
    ///
    /// # Errors
    /// [`RecordError`] when the sighting names neither a subject nor a phantom,
    /// or when `last` precedes `first` or sits on another clock generation.
    pub fn new(
        observer: ObjId,
        subject: Option<ObjId>,
        phantom: Option<IdempotencyKeyText>,
        first: ClockStamp,
        last: ClockStamp,
    ) -> Result<Self, RecordError> {
        if subject.is_none() && phantom.is_none() {
            return Err(RecordError::SubjectlessSighting);
        }
        if last.clock != first.clock || last.at < first.at {
            return Err(RecordError::SightingRegressed);
        }
        Ok(Self {
            observer,
            subject,
            phantom,
            first,
            last,
        })
    }

    /// The observing platform.
    #[must_use]
    pub const fn observer(&self) -> ObjId {
        self.observer
    }
    /// The observed platform, when the sighting is of a real one.
    #[must_use]
    pub const fn subject(&self) -> Option<ObjId> {
        self.subject
    }
    /// The phantom, when the sighting is false.
    #[must_use]
    pub const fn phantom(&self) -> Option<&IdempotencyKeyText> {
        self.phantom.as_ref()
    }
    /// When it was first armed.
    #[must_use]
    pub const fn first(&self) -> &ClockStamp {
        &self.first
    }
    /// When it was last armed.
    #[must_use]
    pub const fn last(&self) -> &ClockStamp {
        &self.last
    }

    /// The same sighting, re-stamped as seen again at `at` (fold D4(c)).
    ///
    /// # Errors
    /// [`RecordError::SightingRegressed`] when `at` precedes `first` or sits on
    /// another clock generation.
    pub fn seen_again(&self, at: &ClockStamp) -> Result<Self, RecordError> {
        Self::new(
            self.observer,
            self.subject,
            self.phantom.clone(),
            self.first.clone(),
            at.clone(),
        )
    }

    /// How stale this sighting is at `at`, or `None` when incomparable — which
    /// the ageing pass treats as "retire it", so a rate change cannot freeze the
    /// book (fold D5).
    #[must_use]
    pub fn staleness(&self, at: &ClockStamp) -> Option<i64> {
        (at.clock == self.last.clock).then(|| at.at.saturating_sub(self.last.at).max(0))
    }

    /// The document/book key for one sighting.
    ///
    /// **A domain-separated digest, not the pair in plaintext** (fold K13). The
    /// key used to be `<observer>/o<subject>`, persisted as
    /// `sighting:<observer>/o<subject>` — the one place outside the owner-fenced
    /// contact family where a real observer→subject truth mapping was written
    /// down, readable from a key listing or a backup by anyone with read access
    /// to the namespace.
    #[must_use]
    pub fn key_of(
        observer: ObjId,
        subject: Option<ObjId>,
        phantom: Option<&IdempotencyKeyText>,
    ) -> String {
        let value = serde_json::json!({
            "observer": observer.0,
            "subject": subject.map(|id| id.0),
            "phantom": phantom.map(IdempotencyKeyText::as_str),
        });
        mush_core::CanonicalHash::for_canonical_value("propagation.sighting.v1", 1, &value, 1024)
            .map_or_else(
                |_| {
                    // Unreachable for a three-field document; a deterministic,
                    // non-identifying fallback rather than a panic in a pass.
                    format!("x{:016x}", fold_digest(observer, subject, phantom))
                },
                |digest| digest.to_hex()[..32].to_owned(),
            )
    }

    /// This sighting's key.
    #[must_use]
    pub fn key(&self) -> String {
        Self::key_of(self.observer, self.subject, self.phantom.as_ref())
    }
}

/// A tiny deterministic FNV-1a fallback for [`Sighting::key_of`]'s error arm.
/// It carries no readable dbref either.
fn fold_digest(
    observer: ObjId,
    subject: Option<ObjId>,
    phantom: Option<&IdempotencyKeyText>,
) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut eat = |byte: u8| {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for byte in observer.0.to_le_bytes() {
        eat(byte);
    }
    for byte in subject.map_or(0_u32, |id| id.0).to_le_bytes() {
        eat(byte);
    }
    for byte in phantom
        .map(IdempotencyKeyText::as_str)
        .unwrap_or("")
        .bytes()
    {
        eat(byte);
    }
    hash
}

/// A record that will not construct or will not load.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RecordError {
    #[error("invalid message dispatch reference")]
    DispatchReference,
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
    #[error("a sighting must name a subject or a phantom")]
    SubjectlessSighting,
    #[error("a sighting's stamps moved backwards or changed clock generation")]
    SightingRegressed,
    #[error("that phantom id already names a different phantom")]
    PhantomIdReused,
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
    /// Where the next delivery pass resumes (fold B3).
    ///
    /// Without it the pass restarted at the lowest envelope id on every run, so
    /// a permanently jammed head starved every envelope behind it — never
    /// relayed, never delivered, and never even aged out, because the TTL check
    /// lives inside the loop body — while the page line said "run @prop/pass
    /// again to continue", which was false.
    next_envelope_scan: u64,
    /// Where the next detection pass resumes: `(observer, subject)`.
    ///
    /// Fold K7(a): the roster truncation and the per-observer subject cap were
    /// both deterministic and stateless, so the same low-`ObjId` 32 were
    /// examined on every pass for ever and the 33rd placed object was never a
    /// subject. The rotation is what makes the bound a page rather than a wall.
    next_detection_scan: Option<(ObjId, ObjId)>,
    /// How many documents the boot dropped past [`MAX_HELD_DOCUMENTS`].
    held_overflow: usize,
    /// Lines the tick and the boot page to the next staff reader.
    notices: Vec<String>,
    /// Declared interference by place (staff, or a later electronic-warfare
    /// result). **Delivery-affecting only**: R4's jammed unit still takes
    /// orders, and `resolve_detection` never reads this term.
    jamming: BTreeMap<ObjId, u32>,
    /// Documents that would not decode, kept verbatim so a later fix can read
    /// them and a boot never destroys the only copy.
    held: BTreeMap<SmolStr, serde_json::Value>,
    /// Shared pair bandwidth, committed atomically with envelope transitions.
    pub(crate) transport: crate::routing::TransportState,
}

impl PropagationStore {
    /// Internal read-only service/transport projection; public player views must
    /// go through the actor-scoped service board and arrived receipts.
    pub fn relay_transport(&self) -> &crate::routing::TransportState {
        &self.transport
    }

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

    /// Drop one envelope (terminal and past its TTL — the fold's D7 reaper).
    pub fn drop_envelope(&mut self, id: u64) -> Option<Envelope> {
        self.envelopes.remove(&id)
    }

    /// Where the next delivery pass resumes.
    #[must_use]
    pub const fn envelope_scan_cursor(&self) -> u64 {
        self.next_envelope_scan
    }

    /// Record where the delivery pass stopped, so the next one starts there.
    pub const fn set_envelope_scan_cursor(&mut self, cursor: u64) {
        self.next_envelope_scan = cursor;
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
    /// [`RecordError::Census`] at [`MAX_PHANTOMS`] for a NEW id, or
    /// [`RecordError::PhantomIdReused`] when the incoming id already names a
    /// **different** phantom (fold B11): replacing one silently destroyed the
    /// live phantom's cause, expiry and holder set, so its holders never
    /// received the expiry `Loss` and their false contacts became permanent.
    pub fn put_phantom(&mut self, phantom: Phantom) -> Result<(), RecordError> {
        match self.phantoms.get(&phantom.id) {
            // The IC scalar, not the clock identity: a re-anchor (fold D5) moves
            // the generation and keeps the scalar, and must not read as a
            // different phantom.
            Some(existing) if existing.created().at != phantom.created().at => {
                return Err(RecordError::PhantomIdReused);
            }
            Some(_) => {}
            None => {
                if self.phantoms.len() >= MAX_PHANTOMS {
                    return Err(RecordError::Census {
                        actual: self.phantoms.len(),
                        max: MAX_PHANTOMS,
                    });
                }
            }
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

    /// Drop one provenance row (its contact is gone from the ledger).
    pub fn drop_provenance(&mut self, contact: mush_core::ContactId) -> Option<ContactProvenance> {
        self.provenance.remove(&contact)
    }

    /// Drop one sighting (older than `CONTACT_TTL`, or un-ageable).
    pub fn drop_sighting(&mut self, key: &str) -> Option<Sighting> {
        self.sightings.remove(key)
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

    /// Hold one undecodable document, up to [`MAX_HELD_DOCUMENTS`].
    ///
    /// Returns `true` when the document was retained. Past the cap the boot
    /// **counts and pages** rather than retaining (fold B7); the document itself
    /// is untouched in the store, so nothing is destroyed either way.
    pub fn hold(&mut self, key: SmolStr, value: serde_json::Value) -> bool {
        if !self.held.contains_key(&key) && self.held.len() >= MAX_HELD_DOCUMENTS {
            self.held_overflow = self.held_overflow.saturating_add(1);
            return false;
        }
        self.held.insert(key, value);
        true
    }

    /// How many undecodable documents were past the held cap.
    #[must_use]
    pub const fn held_overflow(&self) -> usize {
        self.held_overflow
    }

    /// Record that the boot dropped `count` documents past the held cap.
    pub fn record_held_overflow(&mut self, count: usize) {
        self.held_overflow = self.held_overflow.saturating_add(count);
    }

    /// Page one line to the next staff reader.
    ///
    /// Fold B3/B7: the Coarse tick has no actor to talk to, so before the fold
    /// its bound, its anomalies and its boot-held count went to `tracing::warn!`
    /// and nobody was ever told. `mush-propagation`'s edge set is
    /// `{mush-core, mush-sim}`, which does not include the economy escalation
    /// crate, so the page lands here — a bounded durable queue `@prop/pass` and
    /// `@prop/held` both drain — rather than in a log.
    pub fn page(&mut self, line: impl Into<String>) {
        if self.notices.len() >= MAX_PAGED_NOTICES {
            self.notices.remove(0);
        }
        self.notices.push(line.into());
    }

    /// Everything paged since the last drain.
    #[must_use]
    pub fn notices(&self) -> &[String] {
        &self.notices
    }

    /// Drain the paged lines.
    pub fn take_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notices)
    }

    /// The declared jamming level at a place.
    #[must_use]
    pub fn jamming_at(&self, place: ObjId) -> u32 {
        self.jamming.get(&place).copied().unwrap_or(0)
    }

    /// A snapshot of the declared interference, for one pass to read.
    ///
    /// Fold B16: the delivery pass used to deep-clone the WHOLE store — up to
    /// three books of 4_096 records each plus the unbounded held map — solely so
    /// `context_between` could read a jamming level. This clone is O(jammed
    /// places), and there are usually none.
    #[must_use]
    pub fn jamming_map(&self) -> BTreeMap<ObjId, u32> {
        self.jamming.clone()
    }

    /// Where the next detection pass resumes.
    #[must_use]
    pub const fn detection_cursor(&self) -> Option<(ObjId, ObjId)> {
        self.next_detection_scan
    }

    /// Record where the detection pass stopped; `None` restarts the rotation.
    pub const fn set_detection_cursor(&mut self, cursor: Option<(ObjId, ObjId)>) {
        self.next_detection_scan = cursor;
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
            "next_envelope_scan": self.next_envelope_scan,
            "next_detection_scan": self.next_detection_scan
                .map(|(observer, subject)| serde_json::json!({
                    "observer": observer.0, "subject": subject.0
                })),
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
        self.next_envelope_scan = value
            .get("next_envelope_scan")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            .max(self.next_envelope_scan);
        if let Some(cursor) = value.get("next_detection_scan").filter(|v| v.is_object())
            && let (Some(observer), Some(subject)) = (
                cursor.get("observer").and_then(serde_json::Value::as_u64),
                cursor.get("subject").and_then(serde_json::Value::as_u64),
            )
            && let (Ok(observer), Ok(subject)) = (u32::try_from(observer), u32::try_from(subject))
        {
            self.next_detection_scan = Some((ObjId(observer), ObjId(subject)));
        }
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
    ///
    /// **Both allocators** (fold B11). Before the fold only the envelope
    /// allocator was raised, so a missing, truncated or older-schema allocator
    /// document reset the phantom ordinal to 0 while the `phantom:*` documents
    /// hydrated normally, and the next mint re-issued a live phantom's identity.
    pub fn reconcile_allocators(&mut self) {
        if let Some(highest) = self.envelopes.keys().next_back() {
            self.next_envelope = self.next_envelope.max(*highest);
        }
        if let Some(highest) = self.phantoms.values().map(Phantom::ordinal).max() {
            self.next_phantom = self.next_phantom.max(highest);
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
